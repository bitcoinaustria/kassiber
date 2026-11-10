//! `AF_UNIX` stream sockets as CPython's `socket` module drives them for the
//! operator broker (`kassiber/operator/protocol.py`).
//!
//! std binds and listens in one call with a backlog of 128 and has no
//! connect timeout or peer credentials, so the socket calls are made here.
//! Every descriptor is created close-on-exec, as CPython's are.

use std::ffi::OsStr;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
pub use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::{Duration, Instant};

/// What the kernel reports about the process on the other end.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerCredentials {
    pub uid: u32,
    /// The peer's pid when the platform reports one above zero.
    pub pid: Option<u32>,
}

/// The real uid of this process (`os.getuid()`).
pub fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

fn last_error() -> io::Error {
    io::Error::last_os_error()
}

fn cvt(result: libc::c_int) -> io::Result<libc::c_int> {
    if result == -1 {
        Err(last_error())
    } else {
        Ok(result)
    }
}

fn address(path: &Path) -> io::Result<(libc::sockaddr_un, libc::socklen_t)> {
    // SAFETY: sockaddr_un is plain old data; all-zero is a valid value.
    let mut address: libc::sockaddr_un = unsafe { zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = OsStr::as_bytes(path.as_os_str());
    if bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "embedded null character in path",
        ));
    }
    // CPython: "AF_UNIX path too long" when the path fills sun_path.
    if bytes.len() >= address.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "AF_UNIX path too long",
        ));
    }
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }
    let length = size_of::<libc::sa_family_t>() + bytes.len() + 1;
    Ok((address, length as libc::socklen_t))
}

fn stream_socket() -> io::Result<OwnedFd> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // SAFETY: plain socket(2) call; the result is checked before use.
        let fd =
            cvt(unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) })?;
        // SAFETY: fd is a fresh descriptor this function owns.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        // SAFETY: plain socket(2) call; the result is checked before use.
        let fd = cvt(unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) })?;
        // SAFETY: fd is a fresh descriptor this function owns.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        set_cloexec(owned.as_raw_fd())?;
        Ok(owned)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn set_cloexec(fd: RawFd) -> io::Result<()> {
    // SAFETY: fd is a live descriptor owned by the caller.
    let flags = cvt(unsafe { libc::fcntl(fd, libc::F_GETFD) })?;
    // SAFETY: as above; only FD_CLOEXEC is added.
    cvt(unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) })?;
    Ok(())
}

fn set_nonblocking(fd: RawFd, on: bool) -> io::Result<()> {
    // SAFETY: fd is a live descriptor owned by the caller.
    let flags = cvt(unsafe { libc::fcntl(fd, libc::F_GETFL) })?;
    let flags = if on {
        flags | libc::O_NONBLOCK
    } else {
        flags & !libc::O_NONBLOCK
    };
    // SAFETY: as above.
    cvt(unsafe { libc::fcntl(fd, libc::F_SETFL, flags) })?;
    Ok(())
}

/// `socket.bind(path)`, `os.chmod(path, mode)`, `socket.listen(backlog)`,
/// in Python's order. A failure after binding leaves the socket file for
/// the caller to judge, as Python does.
pub fn bind_listener(path: &Path, mode: u32, backlog: i32) -> io::Result<UnixListener> {
    let socket = stream_socket()?;
    let (address, length) = address(path)?;
    // SAFETY: address is a valid sockaddr_un of `length` bytes.
    cvt(unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            length,
        )
    })?;
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode))?;
    // SAFETY: the socket is bound and owned here.
    cvt(unsafe { libc::listen(socket.as_raw_fd(), backlog) })?;
    Ok(UnixListener::from(socket))
}

/// `socket.settimeout(timeout); socket.connect(path)` with CPython's
/// semantics: the connect runs non-blocking, a connect that is in progress
/// (or interrupted) waits for writability until the timeout, and any other
/// failure (including `EAGAIN` from a full backlog) is reported at once.
/// The stream comes back blocking, without I/O timeouts.
pub fn connect(path: &Path, timeout: Option<Duration>) -> io::Result<UnixStream> {
    let socket = stream_socket()?;
    let (address, length) = address(path)?;
    let fd = socket.as_raw_fd();
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    if timeout.is_some() {
        set_nonblocking(fd, true)?;
    }
    // SAFETY: address is a valid sockaddr_un of `length` bytes.
    let result =
        unsafe { libc::connect(fd, (&address as *const libc::sockaddr_un).cast(), length) };
    if result == -1 {
        let error = last_error();
        let waits = matches!(error.raw_os_error(), Some(libc::EINPROGRESS | libc::EINTR));
        match deadline {
            Some(deadline) if waits => wait_connected(fd, Some(deadline))?,
            // Blocking connect: CPython waits out an interrupted connect.
            None if error.raw_os_error() == Some(libc::EINTR) => wait_connected(fd, None)?,
            _ => return Err(error),
        }
    }
    if timeout.is_some() {
        set_nonblocking(fd, false)?;
    }
    Ok(UnixStream::from(socket))
}

fn wait_connected(fd: RawFd, deadline: Option<Instant>) -> io::Result<()> {
    loop {
        let millis = match deadline {
            None => -1,
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "timed out"));
                }
                remaining.as_millis().clamp(1, libc::c_int::MAX as u128) as libc::c_int
            }
        };
        let mut entry = libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        // SAFETY: one valid pollfd entry.
        let ready = unsafe { libc::poll(&mut entry, 1, millis) };
        if ready == -1 {
            let error = last_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error);
        }
        if ready == 0 {
            continue;
        }
        let mut code: libc::c_int = 0;
        let mut size = size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: code and size describe a writable c_int.
        cvt(unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut code as *mut libc::c_int).cast(),
                &mut size,
            )
        })?;
        return if code == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(code))
        };
    }
}

/// The peer's uid and pid as the kernel recorded them at connect time.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn peer_credentials(stream: &UnixStream) -> io::Result<PeerCredentials> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut size = size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: credentials and size describe a writable ucred.
    cvt(unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut size,
        )
    })?;
    Ok(PeerCredentials {
        uid: credentials.uid,
        pid: u32::try_from(credentials.pid).ok().filter(|pid| *pid > 0),
    })
}

/// The peer's uid and pid as the kernel recorded them at connect time.
#[cfg(target_os = "macos")]
pub fn peer_credentials(stream: &UnixStream) -> io::Result<PeerCredentials> {
    // <sys/un.h>: SOL_LOCAL, LOCAL_PEERPID (as protocol.py spells them).
    const SOL_LOCAL: libc::c_int = 0;
    const LOCAL_PEERPID: libc::c_int = 0x002;
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    // SAFETY: both outputs are writable and the descriptor is live.
    cvt(unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) })?;
    let mut pid: libc::pid_t = 0;
    let mut size = size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: pid and size describe a writable pid_t.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            SOL_LOCAL,
            LOCAL_PEERPID,
            (&mut pid as *mut libc::pid_t).cast(),
            &mut size,
        )
    };
    // Python treats a failed LOCAL_PEERPID as an unknown pid, not an error.
    let pid = if result == 0 { pid } else { 0 };
    Ok(PeerCredentials {
        uid,
        pid: u32::try_from(pid).ok().filter(|pid| *pid > 0),
    })
}

/// No supported peer-credential primitive on this platform.
#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
pub fn peer_credentials(_stream: &UnixStream) -> io::Result<PeerCredentials> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no local peer-credential primitive",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};

    fn scratch() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("ipc-").tempdir().unwrap()
    }

    #[test]
    fn bound_listener_has_the_requested_mode_and_round_trips() {
        let dir = scratch();
        let path = dir.path().join("s.sock");
        let listener = bind_listener(&path, 0o600, 32).unwrap();
        let info = std::fs::symlink_metadata(&path).unwrap();
        assert!(info.file_type().is_socket());
        assert_eq!(info.permissions().mode() & 0o777, 0o600);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let credentials = peer_credentials(&stream).unwrap();
            let mut buffer = [0u8; 4];
            stream.read_exact(&mut buffer).unwrap();
            stream.write_all(b"pong").unwrap();
            (credentials, buffer)
        });
        let mut client = connect(&path, Some(Duration::from_secs(2))).unwrap();
        let server_side = peer_credentials(&client).unwrap();
        client.write_all(b"ping").unwrap();
        let mut reply = [0u8; 4];
        client.read_exact(&mut reply).unwrap();
        let (client_side, request) = server.join().unwrap();
        assert_eq!(&reply, b"pong");
        assert_eq!(&request, b"ping");
        assert_eq!(client_side.uid, current_uid());
        assert_eq!(server_side.uid, current_uid());
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        assert_eq!(client_side.pid, Some(std::process::id()));
    }

    #[test]
    fn connect_without_a_listener_is_refused_or_missing() {
        let dir = scratch();
        let missing = connect(
            &dir.path().join("absent.sock"),
            Some(Duration::from_secs(1)),
        )
        .unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        let path = dir.path().join("stale.sock");
        drop(bind_listener(&path, 0o600, 1).unwrap());
        let refused = connect(&path, Some(Duration::from_secs(1))).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::ConnectionRefused);
    }

    #[test]
    fn overlong_paths_are_refused_before_any_socket_call() {
        let long = Path::new("/").join("x".repeat(200));
        let error = connect(&long, None).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "AF_UNIX path too long");
    }

    #[test]
    fn descriptors_are_close_on_exec() {
        let dir = scratch();
        let path = dir.path().join("c.sock");
        let listener = bind_listener(&path, 0o600, 1).unwrap();
        let stream = connect(&path, Some(Duration::from_secs(1))).unwrap();
        for fd in [listener.as_raw_fd(), stream.as_raw_fd()] {
            // SAFETY: both descriptors are live for this test.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            assert_ne!(flags & libc::FD_CLOEXEC, 0);
        }
        // SAFETY: as above.
        let status = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(
            status & libc::O_NONBLOCK,
            0,
            "connect restores blocking mode"
        );
    }
}

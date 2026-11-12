//! The broker's endpoint: where it lives, who may create it, and how a
//! listener and a client reach it (`kassiber/operator/protocol.py`).
//!
//! POSIX: `<passwd home>/.kassiber/run/operator-v1.sock` in a 0700
//! directory, elected through `operator-v1.start.lock`. Windows:
//! `\\.\pipe\kassiber-operator-<sha256(SID)[:24]>`, elected through
//! `FILE_FLAG_FIRST_PIPE_INSTANCE`. `HOME`, `XDG_RUNTIME_DIR` and the
//! working directory never move the endpoint; the gated test override does
//! only in builds that are not frozen.

use crate::error::{app, Error, Result};
use crate::protocol::{Channel, FrameChannel, Transport};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SOCKET_FILENAME: &str = "operator-v1.sock";
pub const STARTUP_LOCK_FILENAME: &str = "operator-v1.start.lock";
pub const RUNTIME_OVERRIDE_ENV: &str = "KASSIBER_OPERATOR_RUNTIME_DIR";
pub const TEST_RUNTIME_OVERRIDE_ENV: &str = "KASSIBER_OPERATOR_ALLOW_TEST_RUNTIME_DIR";
pub const DEFAULT_WINDOWS_IO_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_UNIX_SERVER_IO_TIMEOUT: Duration = Duration::from_secs(30);
/// `_remove_stale_socket` probes an existing endpoint this long.
pub const STALE_PROBE_TIMEOUT: Duration = Duration::from_millis(200);
pub const LISTEN_BACKLOG: i32 = 32;

/// What decides the runtime directory: the account's passwd home, the
/// override variables, and whether this build is frozen. Tests construct it
/// directly instead of patching `pwd.getpwuid` and `os.environ`.
#[derive(Clone, Debug)]
pub struct Runtime {
    pub account_home: Option<PathBuf>,
    pub override_dir: Option<OsString>,
    pub override_gate: Option<OsString>,
    pub frozen: bool,
}

impl Runtime {
    /// This process: the passwd entry for the real uid, and the environment.
    pub fn from_process() -> Self {
        Runtime {
            account_home: account_home(),
            override_dir: std::env::var_os(RUNTIME_OVERRIDE_ENV),
            override_gate: std::env::var_os(TEST_RUNTIME_OVERRIDE_ENV),
            frozen: crate::build::FROZEN,
        }
    }

    fn override_admitted(&self) -> Option<&OsString> {
        let gate = self.override_gate.as_deref() == Some(std::ffi::OsStr::new("1"));
        self.override_dir
            .as_ref()
            .filter(|dir| !dir.is_empty() && gate && !self.frozen)
    }

    /// `operator_runtime_dir()`: the private directory, created if missing,
    /// resolved.
    pub fn runtime_dir(&self) -> Result<PathBuf> {
        let selected = if let Some(dir) = self.override_admitted() {
            kassiber_paths::Context::from_env()
                .and_then(|context| context.expand_user(Path::new(dir)))
                .map_err(|_| {
                    Error::App(app(
                        "operator_runtime_unavailable",
                        "operator runtime directory is unavailable",
                    ))
                })?
        } else {
            let Some(home) = &self.account_home else {
                return Err(Error::App(app(
                    "operator_runtime_unavailable",
                    "the operator account home is unavailable",
                )));
            };
            if !home.is_absolute() {
                return Err(Error::App(app(
                    "unsafe_operator_runtime_directory",
                    "the operator account home is unsafe",
                )));
            }
            home.join(".kassiber").join("run")
        };
        ensure_private_directory(&selected)?;
        Ok(std::fs::canonicalize(&selected)?)
    }

    /// `operator_endpoint()`.
    pub fn endpoint(&self) -> Result<Endpoint> {
        #[cfg(windows)]
        {
            Ok(Endpoint::Pipe(windows_pipe_name(
                &kassiber_ipc::windows::current_sid()?,
            )))
        }
        #[cfg(unix)]
        {
            Ok(Endpoint::Socket(self.runtime_dir()?.join(SOCKET_FILENAME)))
        }
    }
}

/// Where the broker listens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Endpoint {
    Socket(PathBuf),
    Pipe(String),
}

/// `\\.\pipe\kassiber-operator-<sha256(sid)[:24]>`.
pub fn windows_pipe_name(sid: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest: String = Sha256::digest(sid.as_bytes())
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!(r"\\.\pipe\kassiber-operator-{digest}")
}

#[cfg(unix)]
fn account_home() -> Option<PathBuf> {
    nix::unistd::User::from_uid(nix::unistd::getuid())
        .ok()
        .flatten()
        .map(|user| user.dir)
}

#[cfg(windows)]
fn account_home() -> Option<PathBuf> {
    None
}

fn unavailable() -> Error {
    Error::App(
        app(
            "operator_runtime_unavailable",
            "operator runtime directory is unavailable",
        )
        .retryable(true),
    )
}

fn unsafe_directory(message: &str) -> Error {
    Error::App(app("unsafe_operator_runtime_directory", message))
}

/// `Path.mkdir(mode=0o700, parents=True, exist_ok=True)`: missing parents
/// get the default mode, only the leaf 0700; an existing non-directory fails.
fn make_directory(path: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().filter(|parent| *parent != path);
            let Some(parent) = parent else {
                return Err(error);
            };
            match std::fs::create_dir_all(parent) {
                Ok(()) => {}
                Err(error) if parent.is_dir() => drop(error),
                Err(error) => return Err(error),
            }
            match builder.create(path) {
                Ok(()) => Ok(()),
                Err(_) if path.is_dir() => Ok(()),
                Err(error) => Err(error),
            }
        }
        Err(_) if path.is_dir() => Ok(()),
        Err(error) => Err(error),
    }
}

/// `_ensure_private_directory`: no symlink, a directory, this user's, and
/// (POSIX) no group or other permission bits.
pub fn ensure_private_directory(path: &Path) -> Result<()> {
    if std::fs::symlink_metadata(path).is_ok_and(|info| info.file_type().is_symlink()) {
        return Err(unsafe_directory(
            "operator runtime directory may not be a symlink",
        ));
    }
    make_directory(path).map_err(|_| unavailable())?;
    let info = std::fs::metadata(path).map_err(|_| unavailable())?;
    if !info.is_dir() {
        return Err(unsafe_directory("operator runtime path is not a directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if info.uid() != kassiber_ipc::unix::current_uid() {
            return Err(unsafe_directory(
                "operator runtime directory is owned by another OS user",
            ));
        }
        if info.mode() & 0o077 != 0 {
            return Err(Error::App(
                app(
                    "unsafe_operator_runtime_directory",
                    "operator runtime directory permissions are too broad",
                )
                .with_hint("Restrict the directory to mode 0700 and retry."),
            ));
        }
    }
    Ok(())
}

/// The peer must be this OS user (`_verify_unix_peer`); returns its pid.
#[cfg(unix)]
pub fn verify_unix_peer(
    credentials: io::Result<kassiber_ipc::unix::PeerCredentials>,
    expected_uid: u32,
) -> Result<Option<u32>> {
    let credentials = match credentials {
        Ok(credentials) => credentials,
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            return Err(Error::App(app(
                "operator_peer_credentials_unavailable",
                "this platform lacks a supported local peer-credential primitive",
            )))
        }
        Err(error) => return Err(Error::Io(error)),
    };
    if credentials.uid != expected_uid {
        return Err(Error::App(app(
            "operator_peer_rejected",
            "operator IPC peer is a different OS user",
        )));
    }
    Ok(credentials.pid)
}

/// `socket.settimeout(t)` semantics over a Unix stream: each receive waits
/// up to `t`; a whole `send_all` must finish within `t`.
#[cfg(unix)]
pub struct SocketTransport {
    stream: Option<kassiber_ipc::unix::UnixStream>,
    timeout: Option<Duration>,
}

#[cfg(unix)]
impl SocketTransport {
    pub fn new(
        stream: kassiber_ipc::unix::UnixStream,
        timeout: Option<Duration>,
    ) -> io::Result<Self> {
        stream.set_read_timeout(timeout)?;
        Ok(SocketTransport {
            stream: Some(stream),
            timeout,
        })
    }

    fn stream(&mut self) -> io::Result<&mut kassiber_ipc::unix::UnixStream> {
        self.stream
            .as_mut()
            .ok_or_else(|| io::Error::from_raw_os_error(libc_ebadf()))
    }
}

#[cfg(unix)]
fn libc_ebadf() -> i32 {
    9
}

#[cfg(unix)]
fn timed_out(error: io::Error) -> io::Error {
    if matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ) {
        io::Error::new(io::ErrorKind::TimedOut, "timed out")
    } else {
        error
    }
}

#[cfg(unix)]
impl Transport for SocketTransport {
    fn recv_exact(&mut self, size: usize) -> io::Result<Vec<u8>> {
        use std::io::Read;
        let stream = self.stream()?;
        let mut chunks = vec![0u8; size];
        let mut filled = 0;
        while filled < size {
            match stream.read(&mut chunks[filled..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "operator IPC connection closed",
                    ))
                }
                Ok(read) => filled += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(timed_out(error)),
            }
        }
        Ok(chunks)
    }

    fn send_all(&mut self, payload: &[u8]) -> io::Result<()> {
        use std::io::Write;
        use std::time::Instant;
        let deadline = self.timeout.map(|timeout| Instant::now() + timeout);
        let stream = self.stream()?;
        let mut offset = 0;
        while offset < payload.len() {
            if let Some(deadline) = deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "timed out"));
                }
                stream.set_write_timeout(Some(remaining))?;
            }
            match stream.write(&payload[offset..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(written) => offset += written,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(timed_out(error)),
            }
        }
        Ok(())
    }

    fn close(&mut self) {
        self.stream = None;
    }
}

/// The Windows pipe end as a transport.
#[cfg(windows)]
pub struct PipeTransport(kassiber_ipc::windows::pipe::PipeStream);

#[cfg(windows)]
impl From<kassiber_ipc::windows::pipe::PipeStream> for PipeTransport {
    fn from(stream: kassiber_ipc::windows::pipe::PipeStream) -> Self {
        PipeTransport(stream)
    }
}

#[cfg(windows)]
impl Transport for PipeTransport {
    fn recv_exact(&mut self, size: usize) -> io::Result<Vec<u8>> {
        self.0.recv_exact(size)
    }
    fn send_all(&mut self, payload: &[u8]) -> io::Result<()> {
        self.0.send_all(payload)
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// `connect(timeout=..., io_timeout=...)`: reach the endpoint and prove the
/// server is this OS user before any frame is sent.
pub fn connect(
    runtime: &Runtime,
    timeout: Duration,
    io_timeout: Option<Duration>,
) -> Result<Box<dyn FrameChannel>> {
    match runtime.endpoint()? {
        #[cfg(unix)]
        Endpoint::Socket(path) => {
            let stream = kassiber_ipc::unix::connect(&path, Some(timeout))?;
            verify_unix_peer(
                kassiber_ipc::unix::peer_credentials(&stream),
                kassiber_ipc::unix::current_uid(),
            )?;
            Ok(Box::new(Channel::new(
                SocketTransport::new(stream, io_timeout)?,
                None,
            )))
        }
        #[cfg(windows)]
        Endpoint::Pipe(name) => {
            let (stream, server_sid) = kassiber_ipc::windows::pipe::connect(
                &name,
                timeout,
                Some(io_timeout.unwrap_or(DEFAULT_WINDOWS_IO_TIMEOUT)),
            )?;
            if server_sid != kassiber_ipc::windows::current_sid()? {
                drop(stream);
                return Err(Error::App(app(
                    "operator_peer_rejected",
                    "operator named-pipe server SID did not match",
                )));
            }
            Ok(Box::new(Channel::new(PipeTransport(stream), None)))
        }
        #[allow(unreachable_patterns)]
        _ => Err(Error::Io(io::Error::from(io::ErrorKind::Unsupported))),
    }
}

#[cfg(unix)]
mod listener {
    use super::*;
    use std::fs::File;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};

    /// `_acquire_startup_lock`: the election every POSIX listener runs before
    /// touching the endpoint.
    pub fn acquire_startup_lock(path: &Path) -> Result<File> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| {
                Error::App(
                    app(
                        "operator_runtime_unavailable",
                        "the operator startup lock is unavailable",
                    )
                    .retryable(true),
                )
            })?;
        let info = file.metadata()?;
        if !info.is_file() {
            return Err(Error::App(app(
                "unsafe_operator_endpoint",
                "the operator startup lock is unsafe",
            )));
        }
        if info.uid() != kassiber_ipc::unix::current_uid() {
            return Err(Error::App(app(
                "unsafe_operator_endpoint",
                "the operator startup lock belongs to another OS user",
            )));
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        match file.try_lock() {
            Ok(()) => Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => Err(Error::App(app(
                "operator_broker_running",
                "the operator broker is already starting or running",
            ))),
            Err(std::fs::TryLockError::Error(error)) => Err(Error::Io(error)),
        }
    }

    /// Whether `_remove_stale_socket` may unlink the endpoint: nobody
    /// answered the probe because nobody listens or the file vanished.
    pub fn stale_probe_allows_unlink(probe: &io::Result<kassiber_ipc::unix::UnixStream>) -> bool {
        matches!(
            probe,
            Err(error) if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            )
        )
    }

    /// `_remove_stale_socket`: only this user's socket that refuses a probe.
    pub fn remove_stale_socket(endpoint: &Path) -> Result<()> {
        let info = std::fs::symlink_metadata(endpoint)?;
        if !info.file_type().is_socket() {
            return Err(Error::App(app(
                "unsafe_operator_endpoint",
                "operator endpoint is not a socket",
            )));
        }
        if info.uid() != kassiber_ipc::unix::current_uid() {
            return Err(Error::App(app(
                "unsafe_operator_endpoint",
                "operator endpoint is owned by another OS user",
            )));
        }
        let probe = kassiber_ipc::unix::connect(endpoint, Some(STALE_PROBE_TIMEOUT));
        if stale_probe_allows_unlink(&probe) {
            std::fs::remove_file(endpoint)?;
            return Ok(());
        }
        drop(probe?);
        Err(Error::App(app(
            "operator_broker_running",
            "the operator broker is already running",
        )))
    }

    /// The bound endpoint. Closing unlinks it only while it is still the
    /// inode this listener bound.
    pub struct UnixBrokerListener {
        pub endpoint: PathBuf,
        socket: Option<kassiber_ipc::unix::UnixListener>,
        startup_lock: Option<File>,
        identity: Option<(u64, u64)>,
    }

    impl UnixBrokerListener {
        pub fn bind(endpoint: PathBuf, startup_lock: File) -> Result<Self> {
            let socket = kassiber_ipc::unix::bind_listener(&endpoint, 0o600, LISTEN_BACKLOG)?;
            let info = std::fs::symlink_metadata(&endpoint)?;
            Ok(UnixBrokerListener {
                endpoint,
                socket: Some(socket),
                startup_lock: Some(startup_lock),
                identity: Some((info.dev(), info.ino())),
            })
        }

        /// One same-user client with a 30 s per-operation deadline.
        pub fn accept(&self) -> Result<Box<dyn FrameChannel>> {
            let socket = self
                .socket
                .as_ref()
                .ok_or_else(|| io::Error::from_raw_os_error(libc_ebadf()))?;
            let (stream, _) = socket.accept()?;
            let pid = verify_unix_peer(
                kassiber_ipc::unix::peer_credentials(&stream),
                kassiber_ipc::unix::current_uid(),
            )?;
            Ok(Box::new(Channel::new(
                SocketTransport::new(stream, Some(DEFAULT_UNIX_SERVER_IO_TIMEOUT))?,
                pid,
            )))
        }

        /// The listening socket, for waiting with `poll`.
        pub fn as_fd(&self) -> Option<std::os::fd::BorrowedFd<'_>> {
            use std::os::fd::AsFd;
            self.socket.as_ref().map(|socket| socket.as_fd())
        }

        pub fn close(&mut self) -> Result<()> {
            if self.socket.is_none() {
                return Ok(());
            }
            self.socket = None;
            let result = match std::fs::symlink_metadata(&self.endpoint) {
                Ok(info) if self.identity == Some((info.dev(), info.ino())) => {
                    match std::fs::remove_file(&self.endpoint) {
                        Err(error) if error.kind() != io::ErrorKind::NotFound => {
                            Err(Error::Io(error))
                        }
                        _ => Ok(()),
                    }
                }
                Ok(_) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(Error::Io(error)),
            };
            self.startup_lock = None;
            result
        }
    }

    impl Drop for UnixBrokerListener {
        fn drop(&mut self) {
            let _ = self.close();
        }
    }

    /// `listen()` on POSIX: win the startup lock, clear a stale socket,
    /// bind.
    pub fn listen(runtime: &Runtime) -> Result<UnixBrokerListener> {
        let Endpoint::Socket(endpoint) = runtime.endpoint()? else {
            unreachable!("POSIX endpoints are sockets");
        };
        let parent = endpoint.parent().unwrap_or(Path::new("/"));
        let lock = acquire_startup_lock(&parent.join(STARTUP_LOCK_FILENAME))?;
        if endpoint.exists() || endpoint.is_symlink() {
            remove_stale_socket(&endpoint)?;
        }
        UnixBrokerListener::bind(endpoint, lock)
    }
}

#[cfg(unix)]
pub use listener::{
    acquire_startup_lock, listen, remove_stale_socket, stale_probe_allows_unlink,
    UnixBrokerListener,
};

#[cfg(windows)]
mod listener {
    use super::*;
    use kassiber_ipc::windows::pipe::{CreateError, PipeListener};

    /// The per-user pipe; the first instance won the election.
    pub struct WindowsBrokerListener {
        pipe: PipeListener,
        sid: String,
    }

    impl WindowsBrokerListener {
        pub fn accept(&self) -> Result<Box<dyn FrameChannel>> {
            let accepted = self
                .pipe
                .accept(&self.sid, Some(DEFAULT_WINDOWS_IO_TIMEOUT))?;
            if accepted.sid != self.sid {
                accepted.stream.reject();
                return Err(Error::App(app(
                    "operator_peer_rejected",
                    "operator named-pipe client SID did not match",
                )));
            }
            Ok(Box::new(Channel::new(
                PipeTransport(accepted.stream),
                accepted.pid,
            )))
        }

        /// Stop accepting; a blocked [`WindowsBrokerListener::accept`]
        /// returns an error. Callable from another thread.
        pub fn close(&self) -> Result<()> {
            self.pipe.close();
            Ok(())
        }
    }

    /// `listen()` on Windows.
    pub fn listen(runtime: &Runtime) -> Result<WindowsBrokerListener> {
        let Endpoint::Pipe(name) = runtime.endpoint()? else {
            unreachable!("Windows endpoints are pipes");
        };
        let sid = kassiber_ipc::windows::current_sid()?;
        match PipeListener::bind(&name, &sid) {
            Ok(pipe) => Ok(WindowsBrokerListener { pipe, sid }),
            Err(CreateError::AlreadyRunning) => Err(Error::App(app(
                "operator_broker_running",
                "the operator broker is already starting or running",
            ))),
            Err(CreateError::Io(error)) => Err(Error::Io(error)),
        }
    }
}

#[cfg(windows)]
pub use listener::{listen, WindowsBrokerListener};

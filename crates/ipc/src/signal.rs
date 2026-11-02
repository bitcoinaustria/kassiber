//! Termination signals delivered to a flag.
//!
//! The Python broker installs `signal.signal(SIGTERM/SIGINT, stop)` and the
//! handler only asks the server to stop. Here the handler sets a process-wide
//! flag and writes one byte to a non-blocking pipe, both async-signal-safe,
//! so a server blocked in `poll` on its listener and the pipe wakes up.
//!
//! Windows: Python installs no handler there (`os.name != "nt"`), and a
//! detached broker has no console to receive control events, so
//! [`install_stop_signals`] answers a flag that is never set.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

/// The installed handlers' state. Cheap to copy; all copies share the flag.
#[derive(Clone, Copy, Debug)]
pub struct StopSignal {
    #[cfg(unix)]
    wake: std::os::fd::RawFd,
}

impl StopSignal {
    /// Whether SIGTERM or SIGINT arrived since installation.
    pub fn is_set(&self) -> bool {
        STOP_REQUESTED.load(Ordering::SeqCst)
    }

    /// A descriptor that turns readable once a signal arrived, for `poll`.
    /// It stays open for the life of the process.
    #[cfg(unix)]
    pub fn wake_fd(&self) -> std::os::fd::BorrowedFd<'static> {
        // SAFETY: the read end is leaked at installation and never closed.
        unsafe { std::os::fd::BorrowedFd::borrow_raw(self.wake) }
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::fd::{AsRawFd, IntoRawFd};
    use std::sync::atomic::AtomicI32;
    use std::sync::OnceLock;

    static WAKE_WRITER: AtomicI32 = AtomicI32::new(-1);
    static INSTALLED: OnceLock<io::Result<StopSignal>> = OnceLock::new();

    extern "C" fn handle(_signal: libc::c_int) {
        STOP_REQUESTED.store(true, Ordering::SeqCst);
        let fd = WAKE_WRITER.load(Ordering::SeqCst);
        if fd >= 0 {
            let byte = 1u8;
            // SAFETY: write(2) is async-signal-safe; the descriptor is a
            // leaked non-blocking pipe end, so a full pipe just drops it.
            unsafe {
                libc::write(fd, (&byte as *const u8).cast(), 1);
            }
        }
    }

    fn install() -> io::Result<StopSignal> {
        let (reader, writer) = io::pipe()?;
        // SAFETY: writer is a live descriptor owned here.
        let flags = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) };
        if flags == -1
            // SAFETY: as above; only O_NONBLOCK is added.
            || unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                == -1
        {
            return Err(io::Error::last_os_error());
        }
        let wake = reader.into_raw_fd();
        WAKE_WRITER.store(writer.into_raw_fd(), Ordering::SeqCst);
        for signal in [libc::SIGTERM, libc::SIGINT] {
            // SAFETY: sigaction is plain data; the handler only touches
            // atomics and write(2). SA_RESTART keeps unrelated calls going.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = handle as *const () as libc::sighandler_t;
                action.sa_flags = libc::SA_RESTART;
                libc::sigemptyset(&mut action.sa_mask);
                if libc::sigaction(signal, &action, std::ptr::null_mut()) == -1 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        Ok(StopSignal { wake })
    }

    pub(super) fn install_once() -> io::Result<StopSignal> {
        match INSTALLED.get_or_init(install) {
            Ok(signal) => Ok(*signal),
            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }
}

/// Route SIGTERM and SIGINT to the stop flag for the rest of the process.
/// Later calls return the same state.
pub fn install_stop_signals() -> io::Result<StopSignal> {
    #[cfg(unix)]
    {
        imp::install_once()
    }
    #[cfg(windows)]
    {
        Ok(StopSignal {})
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sigterm_sets_the_flag_and_wakes_the_pipe() {
        let stop = install_stop_signals().unwrap();
        assert!(!stop.is_set());
        // SAFETY: raise(3) delivers to this thread; the handler is installed.
        assert_eq!(unsafe { libc::raise(libc::SIGTERM) }, 0);
        assert!(stop.is_set());
        assert!(crate::poll::wait_readable(stop.wake_fd(), Some(Duration::from_secs(1))).unwrap());
        assert_eq!(
            install_stop_signals()
                .unwrap()
                .wake_fd()
                .as_raw_fd_for_test(),
            stop.wake
        );
    }

    trait RawForTest {
        fn as_raw_fd_for_test(&self) -> std::os::fd::RawFd;
    }
    impl RawForTest for std::os::fd::BorrowedFd<'_> {
        fn as_raw_fd_for_test(&self) -> std::os::fd::RawFd {
            std::os::fd::AsRawFd::as_raw_fd(self)
        }
    }
}

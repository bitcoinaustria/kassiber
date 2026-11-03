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

/// Ctrl-C while a brokered CLI command waits for its operation: Python's
/// `KeyboardInterrupt` around `client.wait`. Only SIGINT (a console
/// `CTRL_C_EVENT` on Windows) is caught; SIGTERM and Ctrl-Break keep
/// their default and end the process, as they do for Python.
#[derive(Clone, Copy, Debug)]
pub struct Interrupt {
    #[cfg(unix)]
    wake: std::os::fd::RawFd,
}

static INTERRUPT_REQUESTED: AtomicBool = AtomicBool::new(false);

impl Interrupt {
    /// Whether Ctrl-C arrived since installation.
    pub fn is_set(&self) -> bool {
        INTERRUPT_REQUESTED.load(Ordering::SeqCst)
    }

    /// Sleep for `timeout`, or less when Ctrl-C arrives; answers
    /// [`Interrupt::is_set`].
    pub fn sleep(&self, timeout: std::time::Duration) -> bool {
        if self.is_set() {
            return true;
        }
        #[cfg(unix)]
        {
            // SAFETY: the read end is leaked at installation and never closed.
            let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(self.wake) };
            if crate::poll::wait_readable(fd, Some(timeout)).is_err() {
                std::thread::sleep(timeout);
            }
        }
        #[cfg(windows)]
        {
            let deadline = std::time::Instant::now() + timeout;
            while !self.is_set() {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    break;
                }
                std::thread::sleep(left.min(std::time::Duration::from_millis(20)));
            }
        }
        self.is_set()
    }
}

#[cfg(unix)]
mod interrupt_imp {
    use super::*;
    use std::os::fd::{AsRawFd, IntoRawFd};
    use std::sync::atomic::AtomicI32;
    use std::sync::OnceLock;

    static WAKE_WRITER: AtomicI32 = AtomicI32::new(-1);
    static INSTALLED: OnceLock<io::Result<Interrupt>> = OnceLock::new();

    extern "C" fn handle(_signal: libc::c_int) {
        INTERRUPT_REQUESTED.store(true, Ordering::SeqCst);
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

    fn install() -> io::Result<Interrupt> {
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
        // SAFETY: sigaction is plain data; the handler only touches atomics
        // and write(2). SA_RESTART keeps unrelated calls going.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = handle as *const () as libc::sighandler_t;
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            if libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut()) == -1 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Interrupt { wake })
    }

    pub(super) fn install_once() -> io::Result<Interrupt> {
        match INSTALLED.get_or_init(install) {
            Ok(interrupt) => Ok(*interrupt),
            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }
}

#[cfg(windows)]
mod interrupt_imp {
    use super::*;
    use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_C_EVENT};

    unsafe extern "system" fn handle(event: u32) -> windows_sys::core::BOOL {
        if event == CTRL_C_EVENT {
            INTERRUPT_REQUESTED.store(true, Ordering::SeqCst);
            1
        } else {
            0
        }
    }

    pub(super) fn install_once() -> io::Result<Interrupt> {
        static INSTALLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        // SAFETY: the handler only stores to an atomic; it stays registered
        // for the life of the process.
        let installed =
            *INSTALLED.get_or_init(|| unsafe { SetConsoleCtrlHandler(Some(handle), 1) } != 0);
        if installed {
            Ok(Interrupt {})
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

/// Route Ctrl-C to the interrupt flag for the rest of the process. Later
/// calls return the same state.
pub fn install_interrupt() -> io::Result<Interrupt> {
    interrupt_imp::install_once()
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

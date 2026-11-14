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

    /// Answer whether Ctrl-C arrived and forget it, so the next one counts
    /// on its own: Python raises one `KeyboardInterrupt` per SIGINT, and
    /// `kassiber chat` handles them one at a time.
    pub fn take(&self) -> bool {
        let arrived = INTERRUPT_REQUESTED.swap(false, Ordering::SeqCst);
        #[cfg(unix)]
        {
            // SAFETY: the read end is leaked at installation and never closed.
            let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(self.wake) };
            let mut byte = 0u8;
            while crate::poll::wait_readable(fd, Some(std::time::Duration::ZERO)).unwrap_or(false) {
                // SAFETY: one byte into a local buffer from a readable pipe.
                if unsafe { libc::read(self.wake, (&mut byte as *mut u8).cast(), 1) } != 1 {
                    break;
                }
            }
        }
        arrived
    }

    /// Wait until `fd` is readable (or at end of file) or Ctrl-C arrives:
    /// `true` when it is readable, `false` when Ctrl-C came first. A
    /// terminal read cannot be interrupted any other way, because the
    /// handler restarts system calls.
    #[cfg(unix)]
    pub fn wait_readable(&self, fd: std::os::fd::BorrowedFd<'_>) -> io::Result<bool> {
        use std::os::fd::AsRawFd;
        loop {
            if self.is_set() {
                return Ok(false);
            }
            let mut entries = [
                libc::pollfd {
                    fd: fd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.wake,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // SAFETY: two valid pollfd entries for live descriptors.
            let ready = unsafe { libc::poll(entries.as_mut_ptr(), 2, -1) };
            if ready == -1 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(error);
            }
            if self.is_set() {
                return Ok(false);
            }
            if entries[0].revents != 0 {
                return Ok(true);
            }
        }
    }
}

/// Python's uncaught `KeyboardInterrupt`: the process ends by SIGINT (on
/// Windows with `STATUS_CONTROL_C_EXIT`), so a shell sees an interrupted
/// command rather than an ordinary failure.
pub fn exit_by_interrupt() -> ! {
    #[cfg(unix)]
    {
        // SAFETY: restoring the default disposition and raising the signal
        // at this thread; the default action ends the process.
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::raise(libc::SIGINT);
        }
        std::process::exit(130)
    }
    #[cfg(windows)]
    {
        std::process::exit(0xC000_013Au32 as i32)
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

/// The provider broker's termination signals (plan 23): SIGTERM, SIGINT
/// and SIGHUP each write their number to a private pipe that
/// [`TerminationSignals::wait`] reads, so one thread can stop the broker's
/// children and remove its directories as ordinary code, then exit.
///
/// Handlers rather than a blocked mask with `sigwait`: a child inherits the
/// blocked mask across exec (std does not reset it), so every provider CLI
/// would then ignore the daemon's SIGTERM, while a handler resets to the
/// default action in the exec'd child.
#[cfg(unix)]
#[derive(Clone, Copy, Debug)]
pub struct TerminationSignals {
    reader: std::os::fd::RawFd,
}

#[cfg(unix)]
impl TerminationSignals {
    /// Block until one of the signals arrives and answer its number.
    pub fn wait(&self) -> io::Result<i32> {
        loop {
            let mut byte = 0u8;
            // SAFETY: one byte into a local buffer from the leaked, blocking
            // read end of the pipe.
            let read = unsafe { libc::read(self.reader, (&mut byte as *mut u8).cast(), 1) };
            match read {
                1 => return Ok(i32::from(byte)),
                0 => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
                _ => {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::EINTR) {
                        return Err(error);
                    }
                }
            }
        }
    }
}

#[cfg(unix)]
mod termination_imp {
    use super::*;
    use std::os::fd::{AsRawFd, IntoRawFd};
    use std::sync::atomic::AtomicI32;
    use std::sync::OnceLock;

    static WRITER: AtomicI32 = AtomicI32::new(-1);
    static INSTALLED: OnceLock<io::Result<TerminationSignals>> = OnceLock::new();

    extern "C" fn handle(signal: libc::c_int) {
        let fd = WRITER.load(Ordering::SeqCst);
        if fd >= 0 {
            let byte = u8::try_from(signal).unwrap_or(0);
            // SAFETY: write(2) is async-signal-safe; the descriptor is a
            // leaked non-blocking pipe end, so a full pipe just drops it.
            unsafe {
                libc::write(fd, (&byte as *const u8).cast(), 1);
            }
        }
    }

    fn install() -> io::Result<TerminationSignals> {
        // std creates both ends close-on-exec: children never hold them.
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
        let reader = reader.into_raw_fd();
        WRITER.store(writer.into_raw_fd(), Ordering::SeqCst);
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            // SAFETY: sigaction is plain data; the handler only reads an
            // atomic and calls write(2). SA_RESTART keeps other calls going.
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
        Ok(TerminationSignals { reader })
    }

    pub(super) fn install_once() -> io::Result<TerminationSignals> {
        match INSTALLED.get_or_init(install) {
            Ok(signals) => Ok(*signals),
            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }
}

/// Route SIGTERM, SIGINT and SIGHUP to [`TerminationSignals`] for the rest
/// of the process. Later calls return the same state.
#[cfg(unix)]
pub fn install_termination_signals() -> io::Result<TerminationSignals> {
    termination_imp::install_once()
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

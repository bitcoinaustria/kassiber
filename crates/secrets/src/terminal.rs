use std::io;

#[cfg(unix)]
mod native {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, MutexGuard,
    };

    static INTERRUPTED: AtomicBool = AtomicBool::new(false);
    static OWNER: Mutex<()> = Mutex::new(());

    extern "C" fn interrupt(_: libc::c_int) {
        INTERRUPTED.store(true, Ordering::SeqCst);
    }

    pub struct ReviewInput {
        previous: libc::sigaction,
        flags: libc::c_int,
        _owner: MutexGuard<'static, ()>,
    }

    impl ReviewInput {
        pub fn install() -> io::Result<Option<Self>> {
            if unsafe { libc::isatty(libc::STDIN_FILENO) } != 1 {
                return Ok(None);
            }
            let owner = OWNER
                .try_lock()
                .map_err(|_| io::Error::other("terminal review is already active"))?;
            let flags = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_GETFL) };
            if flags < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut previous = std::mem::MaybeUninit::<libc::sigaction>::uninit();
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = interrupt as *const () as usize;
            unsafe {
                libc::sigemptyset(&mut action.sa_mask);
            }
            INTERRUPTED.store(false, Ordering::SeqCst);
            if unsafe { libc::sigaction(libc::SIGINT, &action, previous.as_mut_ptr()) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let result = Self {
                previous: unsafe { previous.assume_init() },
                flags,
                _owner: owner,
            };
            if unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_SETFL, flags | libc::O_NONBLOCK) }
                != 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Some(result))
        }

        pub fn read(&mut self, reader: &mut dyn io::Read, buffer: &mut [u8]) -> io::Result<usize> {
            loop {
                if INTERRUPTED.load(Ordering::SeqCst) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                match reader.read(buffer) {
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => (),
                    result => return result,
                }
                let mut descriptor = libc::pollfd {
                    fd: libc::STDIN_FILENO,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let ready = unsafe { libc::poll(&mut descriptor, 1, 50) };
                if ready < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                if ready == 0 {
                    continue;
                }
                if INTERRUPTED.load(Ordering::SeqCst) {
                    return Err(io::ErrorKind::Interrupted.into());
                }
            }
        }
    }

    impl Drop for ReviewInput {
        fn drop(&mut self) {
            unsafe {
                libc::fcntl(libc::STDIN_FILENO, libc::F_SETFL, self.flags);
                libc::sigaction(libc::SIGINT, &self.previous, std::ptr::null_mut());
            }
        }
    }
}

#[cfg(unix)]
pub use native::ReviewInput;

#[cfg(windows)]
mod windows {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, MutexGuard,
    };
    static INTERRUPTED: AtomicBool = AtomicBool::new(false);
    static OWNER: Mutex<()> = Mutex::new(());

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(kind: u32) -> *mut std::ffi::c_void;
        fn GetConsoleMode(handle: *mut std::ffi::c_void, mode: *mut u32) -> i32;
        fn SetConsoleCtrlHandler(handler: Option<extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }

    extern "system" fn interrupt(event: u32) -> i32 {
        if event != 0 {
            return 0;
        }
        INTERRUPTED.store(true, Ordering::SeqCst);
        1
    }

    pub struct ReviewInput {
        _owner: MutexGuard<'static, ()>,
    }
    impl ReviewInput {
        pub fn install() -> io::Result<Option<Self>> {
            let mut mode = 0;
            if unsafe { GetConsoleMode(GetStdHandle((-10i32) as u32), &mut mode) } == 0 {
                return Ok(None);
            }
            let owner = OWNER
                .try_lock()
                .map_err(|_| io::Error::other("terminal review is already active"))?;
            INTERRUPTED.store(false, Ordering::SeqCst);
            if unsafe { SetConsoleCtrlHandler(Some(interrupt), 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Some(Self { _owner: owner }))
        }
        pub fn read(&mut self, reader: &mut dyn io::Read, buffer: &mut [u8]) -> io::Result<usize> {
            if INTERRUPTED.load(Ordering::SeqCst) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let result = reader.read(buffer);
            if INTERRUPTED.load(Ordering::SeqCst) {
                Err(io::ErrorKind::Interrupted.into())
            } else {
                result
            }
        }
    }
    impl Drop for ReviewInput {
        fn drop(&mut self) {
            unsafe {
                SetConsoleCtrlHandler(Some(interrupt), 0);
            }
        }
    }
}

#[cfg(windows)]
pub use windows::ReviewInput;

#[cfg(not(any(unix, windows)))]
pub struct ReviewInput;

#[cfg(not(any(unix, windows)))]
impl ReviewInput {
    pub fn install() -> io::Result<Option<Self>> {
        Ok(None)
    }
    pub fn read(&mut self, _: &mut dyn io::Read, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

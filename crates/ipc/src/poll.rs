//! Waiting for a pipe or socket to become readable, with a timeout.
//!
//! "Readable" includes end of file: a read after a `true` answer returns
//! data or zero bytes without blocking. The broker uses it to bound waits
//! on helper and child pipes (`select.select` in Python).

use std::io;
use std::time::Duration;

/// Whether `fd` became readable (or reached end of file) within `timeout`;
/// `None` waits indefinitely. `false` means the timeout passed.
#[cfg(unix)]
pub fn wait_readable(
    fd: std::os::fd::BorrowedFd<'_>,
    timeout: Option<Duration>,
) -> io::Result<bool> {
    use std::os::fd::AsRawFd;
    use std::time::Instant;
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
        let millis = match deadline {
            None => -1,
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                // Round up so a sub-millisecond remainder still polls once.
                remaining
                    .as_micros()
                    .div_ceil(1000)
                    .min(libc::c_int::MAX as u128) as libc::c_int
            }
        };
        let mut entry = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd entry for a borrowed live descriptor.
        let ready = unsafe { libc::poll(&mut entry, 1, millis) };
        if ready == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error);
        }
        if ready > 0 {
            if entry.revents & libc::POLLNVAL != 0 {
                return Err(io::Error::from_raw_os_error(libc::EBADF));
            }
            return Ok(true);
        }
        if millis == 0 || deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Ok(false);
        }
    }
}

/// Whether the pipe `handle` has data or a closed writer within `timeout`.
/// Anonymous pipes cannot be waited on, so this peeks every 10 ms.
#[cfg(windows)]
pub fn wait_readable(
    handle: std::os::windows::io::BorrowedHandle<'_>,
    timeout: Option<Duration>,
) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use std::time::Instant;
    use windows_sys::Win32::Foundation::ERROR_BROKEN_PIPE;
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
        let mut available = 0u32;
        // SAFETY: a borrowed live pipe handle; only the byte count is read.
        let ok = unsafe {
            PeekNamedPipe(
                handle.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                return Ok(true);
            }
            return Err(error);
        }
        if available > 0 {
            return Ok(true);
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::AsFd;
    use std::time::Instant;

    #[test]
    fn times_out_then_sees_data_and_end_of_file() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        let started = Instant::now();
        assert!(!wait_readable(reader.as_fd(), Some(Duration::from_millis(50))).unwrap());
        assert!(started.elapsed() >= Duration::from_millis(50));
        writer.write_all(b"R").unwrap();
        assert!(wait_readable(reader.as_fd(), Some(Duration::from_secs(1))).unwrap());
        let (reader, writer) = std::io::pipe().unwrap();
        drop(writer);
        assert!(wait_readable(reader.as_fd(), Some(Duration::ZERO)).unwrap());
    }
}

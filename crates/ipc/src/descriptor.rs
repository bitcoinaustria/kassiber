//! A caller-supplied descriptor number taken over for reading, as
//! `os.fdopen(fd, "rb", closefd=True)` takes it: reads go straight to the
//! OS (an invalid number is an I/O error, not undefined behaviour) and the
//! number is closed exactly once when dropped. On Windows the number is a
//! C runtime descriptor, as Python's is.

use std::io::{self, Read};

/// An adopted descriptor; dropping it closes the number.
pub struct Descriptor(i32);

impl Descriptor {
    /// Adopt `fd`. The caller gives up the number; nothing checks it here.
    pub fn adopt(fd: i32) -> Self {
        Descriptor(fd)
    }
}

#[cfg(windows)]
unsafe extern "C" {
    fn _read(fd: i32, buffer: *mut u8, count: u32) -> i32;
    fn _close(fd: i32) -> i32;
}

impl Read for Descriptor {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            loop {
                // SAFETY: the OS validates the number; the buffer is writable
                // for its length.
                let count = unsafe { libc::read(self.0, buffer.as_mut_ptr().cast(), buffer.len()) };
                if count >= 0 {
                    return Ok(count as usize);
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
        #[cfg(windows)]
        {
            // SAFETY: the CRT validates the number; the count fits in u32.
            let count = unsafe {
                _read(
                    self.0,
                    buffer.as_mut_ptr(),
                    buffer.len().min(u32::MAX as usize) as u32,
                )
            };
            if count < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(count as usize)
            }
        }
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: close accepts any number; this object consumes it once.
        unsafe {
            #[cfg(unix)]
            libc::close(self.0);
            #[cfg(windows)]
            _close(self.0);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::IntoRawFd;

    #[test]
    fn reads_then_closes_and_reports_bad_numbers() {
        let (reader, mut writer) = io::pipe().unwrap();
        writer.write_all(b"secret\n").unwrap();
        drop(writer);
        let fd = reader.into_raw_fd();
        let mut text = Vec::new();
        Descriptor::adopt(fd).read_to_end(&mut text).unwrap();
        assert_eq!(text, b"secret\n");
        // -1 is never open, so reading and closing it cannot touch others.
        let mut invalid = Descriptor::adopt(-1);
        assert_eq!(
            invalid.read(&mut [0u8; 4]).unwrap_err().raw_os_error(),
            Some(libc::EBADF)
        );
    }
}

//! Moving a stdio protocol onto private descriptors (`_claim_stdio` in
//! `kassiber/mcp/cli.py`).
//!
//! `kassiber mcp serve` speaks JSON-RPC on stdin and stdout. Before anything
//! else runs, the protocol moves to duplicates and the standard streams
//! point at the null device: nothing else in the process can read protocol
//! input or corrupt protocol output, and nothing reaches a stderr the client
//! may persist.

use std::fs::File;
use std::io::{self, Write};

/// The protocol's private input and output.
pub struct Claimed {
    pub input: File,
    pub output: File,
}

/// Duplicate stdin and stdout for the protocol, then point descriptors 0, 1
/// and 2 (standard handles on Windows) at the null device.
pub fn claim() -> io::Result<Claimed> {
    let _ = io::stdout().flush();
    imp::claim()
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};

    fn duplicate(fd: RawFd) -> io::Result<File> {
        // SAFETY: fcntl(F_DUPFD_CLOEXEC) only creates a new descriptor; the
        // result is owned by the returned File and nothing else.
        let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
        if copy == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `copy` is a fresh descriptor no other owner holds.
        Ok(unsafe { File::from_raw_fd(copy) })
    }

    pub(super) fn claim() -> io::Result<Claimed> {
        let input = duplicate(libc::STDIN_FILENO)?;
        let output = duplicate(libc::STDOUT_FILENO)?;
        let null = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")?;
        for target in [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO] {
            // SAFETY: dup2 onto the standard descriptor numbers replaces
            // what they referred to; `null` stays owned and is closed when
            // it drops, the standard numbers keep their own references.
            if unsafe { libc::dup2(null.as_raw_fd(), target) } == -1 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Claimed { input, output })
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::io::{FromRawHandle, IntoRawHandle};
    use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, SetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    fn duplicate(which: STD_HANDLE) -> io::Result<File> {
        // SAFETY: GetStdHandle only reads the process's handle table.
        let handle = unsafe { GetStdHandle(which) };
        if handle.is_null() || handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let mut copy: HANDLE = std::ptr::null_mut();
        // SAFETY: duplicates a live handle of this process into a new,
        // non-inheritable one owned by the returned File.
        let ok = unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                handle,
                GetCurrentProcess(),
                &mut copy,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `copy` is a fresh handle no other owner holds.
        Ok(unsafe { File::from_raw_handle(copy) })
    }

    pub(super) fn claim() -> io::Result<Claimed> {
        let input = duplicate(STD_INPUT_HANDLE)?;
        let output = duplicate(STD_OUTPUT_HANDLE)?;
        let null = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("NUL")?;
        // The standard handles keep the null device for the life of the
        // process, so its handle is never closed.
        let null = null.into_raw_handle();
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: SetStdHandle stores a live handle this process keeps.
            if unsafe { SetStdHandle(which, null) } == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Claimed { input, output })
    }
}

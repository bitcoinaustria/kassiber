//! Windows named pipes, token SIDs and handle-list launches.
//!
//! The pipe code follows the Windows half of `kassiber/operator/protocol.py`
//! call for call: a protected DACL granting only the current SID,
//! `FILE_FLAG_FIRST_PIPE_INSTANCE` for the startup election,
//! `PIPE_REJECT_REMOTE_CLIENTS`, overlapped I/O bounded by deadlines with
//! `CancelIoEx`, and a buffer kept alive until a cancelled operation
//! drains. Comparing SIDs is the caller's policy; this module reports them.

pub mod pipe;
pub mod process;
mod sid;

pub use sid::{current_sid, process_sid};

use std::io;
use windows_sys::Win32::Foundation::HANDLE;

/// A raw handle that may cross threads. Ownership is tracked by the code
/// holding it, as Python tracks its `wintypes.HANDLE` values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RawHandle(pub(crate) HANDLE);
// SAFETY: a HANDLE is an opaque kernel object index usable from any thread.
unsafe impl Send for RawHandle {}
// SAFETY: as above; this wrapper exposes no interior mutation.
unsafe impl Sync for RawHandle {}

pub(crate) fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    text.encode_wide().chain(Some(0)).collect()
}

pub(crate) fn last_error(message: &str) -> io::Error {
    let error = io::Error::last_os_error();
    io::Error::new(error.kind(), format!("{message}: {error}"))
}

//! The user SID of a process token, as `S-1-5-...` text.

use super::last_error;
use std::io;
use std::mem::size_of;
use std::ptr;
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

fn token_sid(token: HANDLE) -> io::Result<String> {
    let mut needed = 0u32;
    // SAFETY: the size query writes only `needed`.
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut needed);
    }
    let mut storage = vec![0usize; (needed as usize).div_ceil(size_of::<usize>()).max(1)];
    // SAFETY: the aligned buffer holds at least `needed` bytes.
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            storage.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(last_error("GetTokenInformation failed"));
    }
    // SAFETY: a successful TokenUser query starts with TOKEN_USER.
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    let mut text = ptr::null_mut();
    // SAFETY: the SID lives inside `storage` for this call.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
        return Err(last_error("ConvertSidToStringSidW failed"));
    }
    let mut length = 0;
    // SAFETY: the allocation is NUL-terminated.
    unsafe {
        while *text.add(length) != 0 {
            length += 1;
        }
    }
    // SAFETY: `length` UTF-16 units precede the terminator found above.
    let result = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "SID text is not UTF-16"));
    // SAFETY: ConvertSidToStringSidW allocates with LocalAlloc.
    unsafe {
        LocalFree(text.cast());
    }
    result
}

fn process_token_sid(process: HANDLE, message: &str) -> io::Result<String> {
    let mut token = ptr::null_mut();
    // SAFETY: `process` is live for the call; the token handle is closed below.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error(message));
    }
    let result = token_sid(token);
    // SAFETY: OpenProcessToken returned this handle to us.
    unsafe {
        CloseHandle(token);
    }
    result
}

/// The current process's user SID.
pub fn current_sid() -> io::Result<String> {
    // SAFETY: the pseudo-handle needs no closing.
    process_token_sid(unsafe { GetCurrentProcess() }, "OpenProcessToken failed")
}

/// The user SID of process `pid`, opened with limited query rights.
pub fn process_sid(pid: u32) -> io::Result<String> {
    // SAFETY: plain OpenProcess; the handle is checked and closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(last_error("OpenProcess for named-pipe peer failed"));
    }
    let result = process_token_sid(process, "OpenProcessToken for named-pipe peer failed");
    // SAFETY: OpenProcess returned this handle to us.
    unsafe {
        CloseHandle(process);
    }
    result
}

//! Narrow Win32 ownership/identity adapter. Every OS allocation and handle is
//! released locally; no caller-controlled handle is dereferenced.
use crate::{Error, ErrorKind, Result};
use std::{
    fs::File,
    mem::size_of,
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree},
    Security::{
        Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT},
        CheckTokenMembership, GetTokenInformation, IsWellKnownSid, TokenUser,
        WinBuiltinAdministratorsSid, OWNER_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
    System::Threading::{GetCurrentProcess, OpenProcessToken},
    UI::Shell::SHGetFolderPathW,
};
fn error() -> Error {
    Error::new(
        ErrorKind::UnsafeProjectOwner,
        "Windows project ownership could not be verified",
    )
}
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
pub fn local_appdata() -> Result<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    // SAFETY: writable UTF-16 buffer is larger than MAX_PATH; no borrowed handles.
    if unsafe {
        SHGetFolderPathW(
            ptr::null_mut(),
            0x001c,
            ptr::null_mut(),
            0,
            buffer.as_mut_ptr(),
        )
    } != 0
    {
        return Err(error());
    }
    let len = buffer.iter().position(|c| *c == 0).ok_or_else(error)?;
    Ok(PathBuf::from(
        String::from_utf16(&buffer[..len]).map_err(|_| error())?,
    ))
}
fn sid_text(sid: *mut core::ffi::c_void) -> Result<String> {
    let mut text = ptr::null_mut();
    // SAFETY: callers supply a live SID from an OS-owned descriptor/token.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(error());
    }
    let result = (|| {
        let mut len = 0;
        // SAFETY: ConvertSidToStringSidW returns a terminated allocated string.
        unsafe {
            while *text.add(len) != 0 {
                len += 1;
            }
        }
        // SAFETY: the preceding scan found the valid allocation's terminator.
        String::from_utf16(unsafe { std::slice::from_raw_parts(text, len) }).map_err(|_| error())
    })();
    // SAFETY: released with the allocator documented by ConvertSidToStringSidW.
    unsafe {
        LocalFree(text.cast());
    }
    result
}
pub fn current_sid() -> Result<String> {
    let mut token = ptr::null_mut();
    // SAFETY: current process pseudo-handle is valid; output is initialized by OS.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(error());
    }
    let result = (|| {
        let mut size = 0;
        // SAFETY: initial null-buffer call requests the required byte count.
        unsafe {
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut size);
        }
        let mut storage = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        // SAFETY: aligned buffer covers the returned required byte count.
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                storage.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        } == 0
        {
            return Err(error());
        }
        // SAFETY: successful TokenUser result begins with TOKEN_USER.
        let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
        sid_text(user.User.Sid)
    })();
    // SAFETY: OpenProcessToken returned this owned handle.
    unsafe {
        CloseHandle(token);
    }
    result
}
pub fn owned_by_current_user(path: &Path) -> Result<bool> {
    let mut owner = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: terminated path and output pointers are valid; descriptor retained
    // until all uses of its owner SID have finished.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide(path).as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(error());
    }
    let result = (|| {
        if sid_text(owner)? == current_sid()? {
            return Ok(true);
        }
        let mut member = 0;
        // SAFETY: owner is a live SID in descriptor; null token means this user.
        Ok(unsafe {
            IsWellKnownSid(owner, WinBuiltinAdministratorsSid) != 0
                && CheckTokenMembership(ptr::null_mut(), owner, &mut member) != 0
                && member != 0
        })
    })();
    // SAFETY: GetNamedSecurityInfoW allocates with LocalAlloc.
    unsafe {
        LocalFree(descriptor.cast());
    }
    result
}
fn info(path: &Path) -> Result<BY_HANDLE_FILE_INFORMATION> {
    let file = File::open(path).map_err(Error::io)?;
    let mut info = std::mem::MaybeUninit::uninit();
    // SAFETY: file handle lives through the call; output initialized on success.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(error());
    }
    // SAFETY: the API succeeded.
    Ok(unsafe { info.assume_init() })
}
pub fn file_identity(path: &Path) -> Result<String> {
    let i = info(path)?;
    Ok(format!(
        "file:{}:{}",
        i.dwVolumeSerialNumber,
        ((i.nFileIndexHigh as u64) << 32) | i.nFileIndexLow as u64
    ))
}
/// The file's hard-link count (`st_nlink`).
pub fn link_count(path: &Path) -> Result<u32> {
    Ok(info(path)?.nNumberOfLinks)
}

/// Rust canonicalize returns verbatim paths; Python realpath removes the
/// kernel's added prefix before using the path in its ownership digest.
pub fn python_resolved_path(path: &Path) -> PathBuf {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let prefix: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
    if wide.starts_with(&prefix) {
        let mut result: Vec<u16> = "\\\\".encode_utf16().collect();
        result.extend(&wide[prefix.len()..]);
        return PathBuf::from(OsString::from_wide(&result));
    }
    let prefix: Vec<u16> = "\\\\?\\".encode_utf16().collect();
    if wide.starts_with(&prefix) {
        return PathBuf::from(OsString::from_wide(&wide[prefix.len()..]));
    }
    path.to_path_buf()
}

//! Explicit storage publication, separate from read-only path resolution.
use std::{io, path::Path};

/// Rename atomically without replacing any existing file or directory.
/// No copy/delete or check-then-overwriting-rename fallback is permitted.
pub fn rename_noreplace(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use nix::fcntl::{renameat2, RenameFlags, AT_FDCWD};
        renameat2(
            AT_FDCWD,
            source,
            AT_FDCWD,
            target,
            RenameFlags::RENAME_NOREPLACE,
        )
        .map_err(io::Error::from)
    }
    #[cfg(target_os = "macos")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source = CString::new(source.as_os_str().as_bytes())?;
        let target = CString::new(target.as_os_str().as_bytes())?;
        // SAFETY: both paths are live NUL-terminated strings. RENAME_EXCL
        // refuses every pre-existing target, including dangling symlinks.
        if unsafe {
            nix::libc::renamex_np(source.as_ptr(), target.as_ptr(), nix::libc::RENAME_EXCL)
        } == 0
        {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        if source[..source.len() - 1].contains(&0) || target[..target.len() - 1].contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains NUL",
            ));
        }
        // SAFETY: live UTF-16 paths. Omitting MOVEFILE_REPLACE_EXISTING keeps
        // publication exclusive; WRITE_THROUGH requests durable completion.
        if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) } != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (source, target);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "exclusive rename is unavailable",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_never_replaces_an_existing_copy() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&target, b"retained").unwrap();
        assert!(rename_noreplace(&source, &target).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), b"new");
        assert_eq!(std::fs::read(&target).unwrap(), b"retained");
        std::fs::remove_file(&target).unwrap();
        rename_noreplace(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }
}

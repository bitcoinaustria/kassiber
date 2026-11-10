//! Small durable filesystem operations. No environment-driven fault injection.
use crate::{Error, ErrorKind, Result};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

/// Roots the user or OS chooses (a data root and its state root, the app-data
/// directory, home) are resolved like Python's `Path.resolve(strict=False)`:
/// links above them, such as macOS `/var` -> `/private/var` or a linked home,
/// are the user's layout. Return `path` on its resolved root with the
/// Kassiber-managed tail kept lexical, so `refuse_symlinks` still refuses
/// every link from the root down.
pub(crate) fn under(root: &Path, path: &Path) -> Result<PathBuf> {
    let tail = path.strip_prefix(root).map_err(|_| {
        Error::new(
            ErrorKind::UnsafeConfiguration,
            "storage path is outside its root",
        )
    })?;
    let resolved = kassiber_paths::canonicalize_allow_missing(root)?;
    Ok(if tail.as_os_str().is_empty() {
        resolved
    } else {
        resolved.join(tail)
    })
}

/// `under` the parent directory: a database in its data root, or a file whose
/// directory the user named.
pub(crate) fn beside(path: &Path) -> Result<PathBuf> {
    under(path.parent().unwrap_or(Path::new("")), path)
}

pub(crate) fn refuse_symlinks(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(Error::new(
                    ErrorKind::UnsafeConfiguration,
                    "storage path contains a symlink",
                ));
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(Error::io(e)),
        }
    }
    Ok(())
}

pub(crate) fn directory(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    refuse_symlinks(path)?;
    if path.is_dir() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        directory(parent)?;
    }
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(Error::io)?;
    sync_directory(path.parent().unwrap_or(Path::new(".")))
}

pub(crate) fn create_new(path: &Path) -> Result<File> {
    refuse_symlinks(path)?;
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(Error::io)
}

/// Flush an already written file by path. Windows' `FlushFileBuffers` needs a
/// handle with write access (Python's `durable_sync.sync_file` likewise
/// opens read-write there); elsewhere a read-only handle is enough.
pub(crate) fn sync_file(path: &Path) -> Result<()> {
    sync_options()
        .open(path)
        .and_then(|f| f.sync_all())
        .map_err(Error::io)
}

fn sync_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(cfg!(windows));
    options
}

pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(not(windows))]
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(Error::io)?;
    #[cfg(windows)]
    let _ = path; // Python's directory-fsync contract is a no-op on Windows.
    Ok(())
}

/// Observer labels identify durable boundaries, and are never read from env.
pub(crate) fn atomic_write(
    path: &Path,
    bytes: &[u8],
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    atomic_write_with_permissions(path, bytes, None, observer)
}
pub(crate) fn atomic_write_with_permissions(
    path: &Path,
    bytes: &[u8],
    permissions: Option<fs::Permissions>,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().unwrap_or(Path::new("."));
    directory(parent)?;
    refuse_symlinks(path)?;
    if path.exists() && !path.is_file() {
        return Err(Error::new(
            ErrorKind::UnsafeConfiguration,
            "destination is not a regular file",
        ));
    }
    let temporary = parent.join(format!(
        ".kassiber-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = create_new(&temporary)?;
    let result = (|| {
        if let Some(permissions) = permissions {
            file.set_permissions(permissions).map_err(Error::io)?;
        }
        file.write_all(bytes).map_err(Error::io)?;
        observer("file.before_sync")?;
        file.sync_all().map_err(Error::io)?;
        observer("file.after_sync")?;
        refuse_symlinks(path)?;
        observer("file.before_replace")?;
        fs::rename(&temporary, path).map_err(Error::io)?;
        observer("file.after_replace")?;
        sync_directory(parent)?;
        observer("file.after_directory_sync")
    })();
    drop(file);
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(all(test, unix))]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// A macOS-style `var` -> `private-var` alias inside a disposable root.
    pub(crate) fn aliased(root: &Path) -> (PathBuf, PathBuf) {
        let real = root.join("private-var");
        fs::create_dir(&real).unwrap();
        symlink("private-var", root.join("var")).unwrap();
        (root.join("var"), fs::canonicalize(real).unwrap())
    }

    #[cfg(unix)]
    #[test]
    fn file_syncs_need_no_write_access_off_windows() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("read-only");
        fs::write(&path, b"x").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        // A write-access open would be refused here (unless run as root).
        sync_file(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"x");
    }

    #[test]
    fn roots_resolve_while_links_below_them_still_refuse() {
        let temporary = tempfile::tempdir().unwrap();
        let (alias, real) = aliased(temporary.path());
        let root = alias.join("state");
        let settings = root.join("config/settings.json");
        let resolved = under(&root, &settings).unwrap();
        assert_eq!(resolved, real.join("state/config/settings.json"));
        refuse_symlinks(&resolved).unwrap();
        directory(resolved.parent().unwrap()).unwrap();
        assert_eq!(beside(&settings).unwrap(), resolved);
        assert_eq!(under(&root, &root).unwrap(), real.join("state"));
        // The unresolved path keeps the prior refusal for links above a root.
        assert_eq!(
            refuse_symlinks(&settings).unwrap_err().kind,
            ErrorKind::UnsafeConfiguration
        );
        fs::rename(real.join("state/config"), real.join("state/elsewhere")).unwrap();
        symlink("elsewhere", real.join("state/config")).unwrap();
        let error = refuse_symlinks(&under(&root, &settings).unwrap()).unwrap_err();
        assert_eq!(
            (error.kind, error.message),
            (
                ErrorKind::UnsafeConfiguration,
                "storage path contains a symlink"
            )
        );
        assert_eq!(
            under(&alias.join("other"), &settings).unwrap_err().kind,
            ErrorKind::UnsafeConfiguration
        );
    }
}

//! Small durable filesystem operations. No environment-driven fault injection.
use crate::{Error, ErrorKind, Result};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

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

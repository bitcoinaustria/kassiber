//! Pinned backend certificates: one owner-only PEM file per backend under the
//! book's data root. Writing a pin is the last step of a confirmed trust
//! decision; nothing here fetches, compares or decides.
use kassiber_envelope::AppError;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The data-root directory holding pinned certificates.
pub const DIRECTORY: &str = "certificates";

/// `<data root>/certificates/<readable name>-<hash>.pem`. Backend names are
/// free text, so the readable part keeps `[a-z0-9-]` only and a hash of the
/// full name keeps two names that reduce alike apart.
pub fn pin_path(data_root: &Path, backend: &str) -> PathBuf {
    let name = backend.trim().to_lowercase();
    let readable: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(40)
        .collect();
    let digest = Sha256::digest(name.as_bytes());
    let hash: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    data_root
        .join(DIRECTORY)
        .join(format!("{}-{hash}.pem", readable.trim_matches('-')))
}

fn failed() -> AppError {
    AppError::new(
        "backend_certificate_write_failed",
        "Could not save the trusted certificate",
    )
    .with_hint("Check that the book's data folder is writable, then trust the certificate again.")
}

/// Replace the backend's pin with `pem`: owner-only directory (0700) and file
/// (0600), written to a temporary file and renamed into place, so a reader
/// never sees a partial certificate. Symlinked targets are refused.
pub fn write_pin(data_root: &Path, backend: &str, pem: &str) -> Result<PathBuf, AppError> {
    let path = pin_path(data_root, backend);
    let directory = path.parent().expect("pin paths have a directory");
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory).map_err(|_| failed())?;
    for target in [directory, path.as_path()] {
        if fs::symlink_metadata(target).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(failed());
        }
    }
    let temporary = directory.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("pin"),
        std::process::id()
    ));
    let _ = fs::remove_file(&temporary);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&temporary).and_then(|mut file| {
        file.write_all(pem.as_bytes())?;
        file.sync_all()
    });
    if written.is_err() || fs::rename(&temporary, &path).is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(failed());
    }
    #[cfg(not(windows))]
    let _ = fs::File::open(directory).and_then(|d| d.sync_all());
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_owner_only_per_backend_and_replace_atomically() {
        let root = tempfile::tempdir().unwrap();
        let first = write_pin(root.path(), "Home Node", "one\n").unwrap();
        assert_eq!(first, pin_path(root.path(), "home node"));
        assert!(first.starts_with(root.path().join(DIRECTORY)));
        let name = first.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("home-node-") && name.ends_with(".pem"),
            "{name}"
        );
        assert_ne!(pin_path(root.path(), "a/b"), pin_path(root.path(), "a b"));
        assert_eq!(write_pin(root.path(), "home node", "two\n").unwrap(), first);
        assert_eq!(fs::read_to_string(&first).unwrap(), "two\n");
        assert_eq!(fs::read_dir(first.parent().unwrap()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&first), 0o600);
            assert_eq!(mode(first.parent().unwrap()), 0o700);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_pin_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = root.path().join("elsewhere.pem");
        fs::write(&elsewhere, "keep\n").unwrap();
        let path = pin_path(root.path(), "home");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
        assert_eq!(
            write_pin(root.path(), "home", "new\n").unwrap_err().code,
            "backend_certificate_write_failed"
        );
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "keep\n");
    }
}

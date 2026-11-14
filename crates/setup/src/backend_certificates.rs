//! Pinned backend certificates: immutable owner-only PEM files under the
//! book's data root, each named by the full SHA-256 of the certificate it
//! holds. A pin file is written before the backend row points at it and is
//! never rewritten, so a failed trust leaves the previous pin and row intact;
//! nothing here fetches, compares or decides.
use kassiber_envelope::AppError;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The data-root directory holding pinned certificates.
pub const DIRECTORY: &str = "certificates";

/// `<data root>/certificates/<sha256 of the DER, hex>.pem`. Content-addressed,
/// so two backends can never share a file unless they pin the same leaf.
pub fn pin_path(data_root: &Path, der: &[u8]) -> PathBuf {
    let digest: String = Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    data_root.join(DIRECTORY).join(format!("{digest}.pem"))
}

/// Whether `path` names a file in the managed pin directory of `data_root`.
pub fn is_managed(data_root: &Path, path: &Path) -> bool {
    path.parent() == Some(data_root.join(DIRECTORY).as_path())
        && path.extension().is_some_and(|ext| ext == "pem")
}

fn failed() -> AppError {
    AppError::new(
        "backend_certificate_write_failed",
        "Could not save the trusted certificate",
    )
    .with_hint("Check that the book's data folder is writable and owned by you, then trust the certificate again.")
}

/// The pin directory, created owner-only. An existing one must be a real
/// directory owned by this user; its mode is tightened to 0700.
fn directory(data_root: &Path) -> Result<PathBuf, AppError> {
    let directory = data_root.join(DIRECTORY);
    match fs::symlink_metadata(&directory) {
        Ok(meta) => {
            if !meta.file_type().is_dir() {
                return Err(failed());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                if meta.uid() != kassiber_paths::current_uid() {
                    return Err(failed());
                }
                if meta.mode() & 0o777 != 0o700 {
                    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                        .map_err(|_| failed())?;
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&directory).map_err(|_| failed())?;
        }
        Err(_) => return Err(failed()),
    }
    Ok(directory)
}

/// A pin file this call wrote, or one that already held the same certificate.
#[derive(Debug)]
pub struct Pin {
    pub path: PathBuf,
    /// This call created the file; remove it if the trust does not commit.
    pub created: bool,
}

/// Write `pem` (the encoding of `der`) as its content-addressed pin, 0600,
/// through a temporary file and a rename. An existing pin of the same
/// certificate is reused and never rewritten; one with other contents, or a
/// symlink in its place, is refused.
pub fn write_pin(data_root: &Path, der: &[u8], pem: &str) -> Result<Pin, AppError> {
    let directory = directory(data_root)?;
    let path = pin_path(data_root, der);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_file() => {
            return if fs::read(&path).map_err(|_| failed())? == pem.as_bytes() {
                Ok(Pin {
                    path,
                    created: false,
                })
            } else {
                Err(failed())
            };
        }
        Ok(_) => return Err(failed()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(failed()),
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
    let _ = fs::File::open(&directory).and_then(|d| d.sync_all());
    Ok(Pin {
        path,
        created: true,
    })
}

/// Remove a managed pin file no backend refers to any more. Best effort:
/// a leftover file trusts nothing on its own.
pub fn remove_unreferenced(data_root: &Path, path: &Path, referenced: bool) {
    if !referenced && is_managed(data_root, path) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_content_addressed_owner_only_and_never_rewritten() {
        let root = tempfile::tempdir().unwrap();
        let first = write_pin(root.path(), b"der-one", "one\n").unwrap();
        assert!(first.created);
        assert_eq!(first.path, pin_path(root.path(), b"der-one"));
        assert!(is_managed(root.path(), &first.path));
        let name = first.path.file_name().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 64 + ".pem".len(), "{name}");
        // The same certificate again reuses the file.
        let again = write_pin(root.path(), b"der-one", "one\n").unwrap();
        assert!(!again.created);
        assert_eq!(again.path, first.path);
        // A different certificate never replaces it.
        let second = write_pin(root.path(), b"der-two", "two\n").unwrap();
        assert_ne!(second.path, first.path);
        assert_eq!(fs::read_to_string(&first.path).unwrap(), "one\n");
        assert_eq!(fs::read_to_string(&second.path).unwrap(), "two\n");
        // Other contents under a pin's name are refused, not overwritten.
        assert_eq!(
            write_pin(root.path(), b"der-one", "forged\n")
                .unwrap_err()
                .code,
            "backend_certificate_write_failed"
        );
        assert_eq!(fs::read_to_string(&first.path).unwrap(), "one\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&first.path), 0o600);
            assert_eq!(mode(first.path.parent().unwrap()), 0o700);
        }
        remove_unreferenced(root.path(), &second.path, true);
        assert!(second.path.exists());
        remove_unreferenced(root.path(), &second.path, false);
        assert!(!second.path.exists());
        // Never a file outside the pin directory.
        let outside = root.path().join("keep.pem");
        fs::write(&outside, "keep").unwrap();
        remove_unreferenced(root.path(), &outside, false);
        assert!(outside.exists());
    }

    #[test]
    fn backend_names_that_reduce_alike_get_their_own_pins() {
        // These two names once shared `homehome…home-0b71770f.pem`, so
        // trusting one overwrote the other's pin. Pins no longer depend on
        // the backend name at all.
        let root = tempfile::tempdir().unwrap();
        let (a, b) = (
            format!("{}4842", "home".repeat(10)),
            format!("{}63481", "home".repeat(10)),
        );
        let first = write_pin(root.path(), a.as_bytes(), "a\n").unwrap();
        let second = write_pin(root.path(), b.as_bytes(), "b\n").unwrap();
        assert_ne!(first.path, second.path);
        assert_eq!(fs::read_to_string(&first.path).unwrap(), "a\n");
        assert_eq!(fs::read_to_string(&second.path).unwrap(), "b\n");
    }

    #[cfg(unix)]
    #[test]
    fn an_unsafe_directory_is_tightened_and_a_symlinked_pin_refused() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join(DIRECTORY);
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o777)).unwrap();
        write_pin(root.path(), b"der", "pem\n").unwrap();
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let elsewhere = root.path().join("elsewhere.pem");
        fs::write(&elsewhere, "keep\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, pin_path(root.path(), b"other")).unwrap();
        assert_eq!(
            write_pin(root.path(), b"other", "new\n").unwrap_err().code,
            "backend_certificate_write_failed"
        );
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "keep\n");
        // A symlinked pin directory is refused too.
        let other = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(root.path(), other.path().join(DIRECTORY)).unwrap();
        assert!(write_pin(other.path(), b"x", "x\n").is_err());
    }
}

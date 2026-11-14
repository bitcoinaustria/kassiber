//! Per-turn working directories (`working-directory.ts`) and the random
//! names the broker hands out.

use std::io;
use std::path::{Path, PathBuf};

use crate::cleanup::track_directory;
use crate::executables::fs_failure;
use crate::protocol::{BrokerFailure, ProviderId};

/// The variable naming the daemon's per-broker temporary root.
pub const BROKER_TMPDIR_ENV: &str = "KASSIBER_AI_BROKER_TMPDIR";

/// `brokerTempRoot()`: the daemon gives each broker a temporary root it owns
/// and removes once the broker has exited, however it exited. Working
/// directories go there so a killed broker leaves nothing behind. The tool
/// bridge stays in the system temporary directory: its socket path must
/// stay short enough for macOS.
pub fn broker_temp_root() -> PathBuf {
    match std::env::var_os(BROKER_TMPDIR_ENV) {
        Some(root) if !root.is_empty() && Path::new(&root).is_absolute() => PathBuf::from(root),
        _ => std::env::temp_dir(),
    }
}

/// Random bytes from the operating system.
pub fn random_bytes<const N: usize>() -> Result<[u8; N], BrokerFailure> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes)
        .map_err(|_| BrokerFailure::new("Kassiber could not read system randomness."))?;
    Ok(bytes)
}

/// `crypto.randomUUID()`: a version 4 UUID.
pub fn random_uuid() -> Result<String, BrokerFailure> {
    let mut bytes = random_bytes::<16>()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

const TEMP_LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// `mkdtemp(<parent>/<prefix>)`: a new private (0700) directory named by the
/// prefix and six random letters.
pub fn make_temp_dir(parent: &Path, prefix: &str) -> Result<PathBuf, BrokerFailure> {
    let template = parent.join(format!("{prefix}XXXXXX"));
    for _ in 0..100 {
        let random = random_bytes::<6>()?;
        let suffix: String = random
            .iter()
            .map(|byte| TEMP_LETTERS[usize::from(*byte) % TEMP_LETTERS.len()] as char)
            .collect();
        let path = parent.join(format!("{prefix}{suffix}"));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(fs_failure("mkdtemp", &template, &error)),
        }
    }
    Err(fs_failure(
        "mkdtemp",
        &template,
        &io::Error::from(io::ErrorKind::AlreadyExists),
    ))
}

/// `withWorkingDirectory(provider, run)`: a fresh empty directory under the
/// broker's temporary root for `run`, tracked for termination and removed
/// afterwards. A failed removal fails the turn, as `rm` rejecting in a
/// `finally` does.
pub fn with_working_directory<T>(
    provider: ProviderId,
    run: impl FnOnce(&Path) -> Result<T, BrokerFailure>,
) -> Result<T, BrokerFailure> {
    let cwd = make_temp_dir(&broker_temp_root(), &format!("kassiber-ai-{provider}-"))?;
    let tracked = track_directory(&cwd);
    let result = run(&cwd);
    let removed = match std::fs::remove_dir_all(&cwd) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            Err(fs_failure("rm", &cwd, &error))
        }
        _ => Ok(()),
    };
    tracked.untrack();
    removed?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_are_version_four_and_unique() {
        let first = random_uuid().unwrap();
        let second = random_uuid().unwrap();
        assert_ne!(first, second);
        assert_eq!(first.len(), 36);
        assert_eq!(&first[14..15], "4");
        assert!(matches!(&first[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn temp_dirs_are_private_and_named_like_mkdtemp() {
        let parent = tempfile::tempdir().unwrap();
        let dir = make_temp_dir(parent.path(), "kassiber-ai-codex-").unwrap();
        let name = dir.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("kassiber-ai-codex-") && name.len() == 24,
            "{name}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        let missing = make_temp_dir(&parent.path().join("absent"), "x-").unwrap_err();
        assert!(missing.message().starts_with("ENOENT: "), "{missing}");
        assert!(missing.message().ends_with("absent/x-XXXXXX'"), "{missing}");
    }

    #[test]
    fn uses_a_fresh_empty_working_directory_and_removes_it_afterward() {
        // broker.test.ts: "uses a fresh empty working directory and removes
        // it afterward".
        let first = with_working_directory(ProviderId::Codex, |cwd| {
            assert!(cwd.is_dir());
            assert_eq!(std::fs::read_dir(cwd).unwrap().count(), 0);
            Ok(cwd.to_path_buf())
        })
        .unwrap();
        assert!(!first.exists());
        let second = with_working_directory(ProviderId::Codex, |cwd| {
            std::fs::write(cwd.join("left-behind"), "x").unwrap();
            Ok(cwd.to_path_buf())
        })
        .unwrap();
        assert_ne!(first, second);
        assert!(!second.exists());
        // A failed turn still removes its directory.
        let mut seen = PathBuf::new();
        let failed = with_working_directory::<()>(ProviderId::Claude, |cwd| {
            seen = cwd.to_path_buf();
            Err(BrokerFailure::new("boom"))
        });
        assert_eq!(failed.unwrap_err().message(), "boom");
        assert!(!seen.as_os_str().is_empty() && !seen.exists());
    }
}

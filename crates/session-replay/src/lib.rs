//! Recorded product processes and their replay.
//!
//! `session-shim` stands in for a product executable during a recording run:
//! it starts the real binary, passes every byte through, and writes one
//! session per process: argv, working directory, the product environment,
//! stdin with the number of stdout lines printed before each chunk, stdout,
//! stderr, signals, the exit status, and the state roots the process named,
//! captured before it starts and after it exits. `session-replay` restores a
//! session's starting state at a fresh path, runs the same executable on the
//! same inputs, and compares what it printed and the state it left.
//!
//! Recordings are made once, where the Rust binary and the Python reference
//! were compared and agreed, so each session is an oracle the Python suite
//! accepted (docs/plan/22-python-removal.md).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod compare;
pub mod dbdump;
pub mod replay;

pub const SESSION_FORMAT: &str = "kassiber-process-session/1";

/// One stdin chunk, with how many stdout lines the process had printed when
/// it arrived; `data` is base64, `None` marks the end of stdin.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StdinChunk {
    pub after_lines: u64,
    pub data: Option<String>,
}

/// A signal the shim forwarded, with the stdout line count at that moment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignalEvent {
    pub after_lines: u64,
    pub signal: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    Dir {
        path: String,
        mode: u32,
    },
    File {
        path: String,
        mode: u32,
        blob: String,
        size: u64,
    },
    Symlink {
        path: String,
        target: String,
    },
}

/// A state root as the process saw it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Root {
    pub path: String,
    /// Whether the root was a single file rather than a directory.
    pub file: bool,
    pub before: Vec<Entry>,
    pub after: Vec<Entry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Exit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub format: String,
    /// The executable's name in the shim map (`kassiber`, an example name).
    pub binary: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    /// `PYTEST_CURRENT_TEST` of the recording test, without the phase suffix.
    pub test: Option<String>,
    /// Wall-clock span of the recorded run, epoch microseconds.
    pub started_us: i64,
    pub ended_us: i64,
    pub stdin: Vec<StdinChunk>,
    pub stdout: String,
    pub stderr: String,
    pub signals: Vec<SignalEvent>,
    pub exit: Exit,
    pub roots: Vec<Root>,
    /// The repository root of the recording checkout, mapped to the replay's.
    pub repo: String,
}

/// Content-addressed file storage under `<dir>/blobs/ab/<sha256>`.
#[derive(Clone, Debug)]
pub struct Blobs {
    dir: PathBuf,
}

impl Blobs {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.join("blobs"),
        }
    }

    fn path(&self, hash: &str) -> PathBuf {
        self.dir.join(&hash[..2]).join(hash)
    }

    pub fn put(&self, bytes: &[u8]) -> io::Result<String> {
        let hash = hex(&Sha256::digest(bytes));
        let path = self.path(&hash);
        if !path.exists() {
            fs::create_dir_all(path.parent().unwrap())?;
            let tmp = path.with_extension(format!("tmp{}", std::process::id()));
            fs::write(&tmp, bytes)?;
            fs::rename(&tmp, &path)?;
        }
        Ok(hash)
    }

    pub fn get(&self, hash: &str) -> io::Result<Vec<u8>> {
        fs::read(self.path(hash))
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Every entry under `root`, paths relative to it, sorted. Sockets and
/// pipes are skipped: a process cannot be restored with them.
pub fn snapshot(root: &Path, blobs: &Blobs) -> io::Result<Vec<Entry>> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let mut entries = Vec::new();
    let meta = fs::symlink_metadata(root)?;
    if meta.is_file() {
        let bytes = fs::read(root)?;
        #[cfg(unix)]
        let mode = meta.permissions().mode() & 0o7777;
        #[cfg(not(unix))]
        let mode = 0o644;
        entries.push(Entry::File {
            path: String::new(),
            mode,
            size: bytes.len() as u64,
            blob: blobs.put(&bytes)?,
        });
        return Ok(entries);
    }
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let dir = root.join(&relative);
        let mut children: Vec<_> = match fs::read_dir(&dir) {
            Ok(read) => read.filter_map(|e| e.ok()).collect(),
            // Removed while we looked (a daemon cleaning up); keep the rest.
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        children.sort_by_key(|e| e.file_name());
        for child in children {
            let rel = relative.join(child.file_name());
            let name = rel.to_string_lossy().replace('\\', "/");
            let Ok(meta) = fs::symlink_metadata(child.path()) else {
                continue;
            };
            #[cfg(unix)]
            let mode = meta.permissions().mode() & 0o7777;
            #[cfg(not(unix))]
            let mode = 0o644;
            if meta.file_type().is_symlink() {
                let target = fs::read_link(child.path())?.to_string_lossy().into_owned();
                entries.push(Entry::Symlink { path: name, target });
            } else if meta.is_dir() {
                entries.push(Entry::Dir { path: name, mode });
                pending.push(rel);
            } else if meta.is_file() {
                let Ok(bytes) = fs::read(child.path()) else {
                    continue;
                };
                entries.push(Entry::File {
                    path: name,
                    mode,
                    size: bytes.len() as u64,
                    blob: blobs.put(&bytes)?,
                });
            }
        }
    }
    entries.sort_by(|a, b| entry_path(a).cmp(entry_path(b)));
    Ok(entries)
}

pub fn entry_path(entry: &Entry) -> &str {
    match entry {
        Entry::Dir { path, .. } | Entry::File { path, .. } | Entry::Symlink { path, .. } => path,
    }
}

/// Write `entries` under `root` (which must not exist yet).
pub fn restore(root: &Path, file: bool, entries: &[Entry], blobs: &Blobs) -> io::Result<()> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    if file {
        if let Some(Entry::File { blob, mode, .. }) = entries.first() {
            fs::create_dir_all(root.parent().unwrap())?;
            fs::write(root, blobs.get(blob)?)?;
            #[cfg(unix)]
            fs::set_permissions(root, fs::Permissions::from_mode(*mode))?;
        }
        return Ok(());
    }
    fs::create_dir_all(root)?;
    let mut dirs = Vec::new();
    for entry in entries {
        match entry {
            Entry::Dir { path, mode } => {
                fs::create_dir_all(root.join(path))?;
                dirs.push((root.join(path), *mode));
            }
            Entry::File {
                path, mode, blob, ..
            } => {
                let target = root.join(path);
                fs::create_dir_all(target.parent().unwrap())?;
                fs::write(&target, blobs.get(blob)?)?;
                #[cfg(unix)]
                fs::set_permissions(&target, fs::Permissions::from_mode(*mode))?;
            }
            Entry::Symlink { path, target } => {
                let link = root.join(path);
                fs::create_dir_all(link.parent().unwrap())?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(target, &link)?;
            }
        }
    }
    // Directory modes last, deepest first, so a read-only dir can be filled.
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (dir, mode) in dirs {
        #[cfg(unix)]
        fs::set_permissions(&dir, fs::Permissions::from_mode(mode))?;
        #[cfg(not(unix))]
        let _ = (dir, mode);
    }
    Ok(())
}

/// The environment a session keeps: what the product reads, never a value
/// whose name says it is a secret.
pub fn keeps_env(name: &str) -> bool {
    const NAMES: &[&str] = &[
        "HOME",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
        "APPIMAGE",
        "APPDIR",
        "TZ",
        "LANG",
        "LANGUAGE",
        "LC_ALL",
        "LC_CTYPE",
        "COLUMNS",
        "LINES",
        "TERM",
        "NO_COLOR",
        "FORCE_COLOR",
        "TMPDIR",
        "PYTHONHASHSEED",
    ];
    let product = name.starts_with("KASSIBER_") || name.starts_with("SATBOOKS_");
    let recorder = name.starts_with("KASSIBER_SESSION_");
    // Test switches name test fakes (`..._SECRET_STORE`), never secrets.
    let test_switch = name.starts_with("KASSIBER_TEST_");
    (product && !recorder && (test_switch || !secret_name(name))) || NAMES.contains(&name)
}

/// `PASSWORD`, `TOKEN`, `KEY` and similar as a `_`-separated name segment.
pub fn secret_name(name: &str) -> bool {
    const SEGMENTS: &[&str] = &[
        "TOKEN",
        "TOKENS",
        "PASSWORD",
        "PASSWD",
        "SECRET",
        "SECRETS",
        "KEY",
        "APIKEY",
        "PASSPHRASE",
        "CREDENTIAL",
        "CREDENTIALS",
        "COOKIE",
        "PRIVATE",
        "AUTH",
        "AUTHORIZATION",
        "SEED",
        "XPUB",
        "XPRV",
        "MNEMONIC",
    ];
    name.split('_').any(|segment| SEGMENTS.contains(&segment))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_restore_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir_all(root.join("a/empty")).unwrap();
        fs::write(root.join("a/f.txt"), b"one").unwrap();
        fs::write(root.join("g.db"), b"two").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("g.db", root.join("link")).unwrap();
        let blobs = Blobs::new(&dir.path().join("store"));
        let entries = snapshot(&root, &blobs).unwrap();
        let copy = dir.path().join("copy");
        restore(&copy, false, &entries, &blobs).unwrap();
        assert_eq!(snapshot(&copy, &blobs).unwrap(), entries);
        assert_eq!(fs::read(copy.join("a/f.txt")).unwrap(), b"one");
        assert!(copy.join("a/empty").is_dir());
    }

    #[test]
    fn secret_variables_are_never_kept() {
        assert!(keeps_env("KASSIBER_DATA_ROOT"));
        assert!(!keeps_env("KASSIBER_DB_PASSPHRASE"));
        assert!(!keeps_env("KASSIBER_API_KEY"));
        assert!(!keeps_env("KASSIBER_SESSION_RECORD_DIR"));
        assert!(!keeps_env("GITHUB_TOKEN"));
        assert!(keeps_env("HOME"));
    }
}

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
pub mod encrypted;
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
        /// An encrypted book, stored decrypted (`blob` is the plaintext).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        encryption: Option<encrypted::Encryption>,
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
    /// The process's network tape (`kassiber-net`'s test-observer tape): the
    /// bytes every connection read, served again on replay.
    #[serde(default)]
    pub net_tape: Option<String>,
    /// Inherited input descriptors the argv names (`--db-passphrase-fd 3`):
    /// descriptor number to base64 content. Test passphrases only.
    #[serde(default)]
    pub fds: BTreeMap<i32, String>,
}

/// Passphrases a session hands its process: inherited descriptors and the
/// passphrase fields of stdin requests.
pub fn passphrase_candidates(fds: &[Vec<u8>], stdin: &[u8]) -> Vec<String> {
    use std::sync::OnceLock;
    static FIELD: OnceLock<regex::Regex> = OnceLock::new();
    let field = FIELD.get_or_init(|| {
        regex::Regex::new(r#""(?:new_)?passphrase(?:_secret)?"\s*:\s*"((?:[^"\\]|\\.)*)""#).unwrap()
    });
    let mut found = Vec::new();
    for bytes in fds {
        let text = String::from_utf8_lossy(bytes);
        let text = text.strip_suffix('\n').unwrap_or(&text).to_owned();
        if !text.is_empty() && !found.contains(&text) {
            found.push(text);
        }
    }
    for caps in field.captures_iter(&String::from_utf8_lossy(stdin)) {
        let value: String = serde_json::from_str(&format!("\"{}\"", &caps[1])).unwrap_or_default();
        if !value.is_empty() && !found.contains(&value) {
            found.push(value);
        }
    }
    found
}

/// Replace each encrypted book in `entries` by its plaintext, stored with
/// the passphrase that opened it (decision B); the ciphertext stays
/// referenced until a selection drops it.
pub fn decrypt_books(entries: &mut [Entry], candidates: &[String], blobs: &Blobs, scratch: &Path) {
    for entry in entries.iter_mut() {
        let Entry::File {
            blob, encryption, ..
        } = entry
        else {
            continue;
        };
        if encryption.is_some() {
            continue;
        }
        let Ok(bytes) = blobs.get(blob) else { continue };
        if let Some((plaintext, mut found)) = encrypted::decrypt(&bytes, candidates, scratch) {
            if let Ok(plain) = blobs.put(&plaintext) {
                found.ciphertext = Some(blob.clone());
                *blob = plain;
                *encryption = Some(found);
            }
        }
    }
}

/// The descriptor numbers argv hands over with a `--...-fd N` (or
/// `--...-fd=N`) option.
pub fn descriptor_args(argv: &[String]) -> Vec<i32> {
    let mut fds = Vec::new();
    for (index, arg) in argv.iter().enumerate() {
        let value = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") && flag.ends_with("-fd") => {
                Some(value.to_owned())
            }
            _ if arg.starts_with("--") && arg.ends_with("-fd") => argv.get(index + 1).cloned(),
            _ => None,
        };
        if let Some(fd) = value
            .and_then(|v| v.parse::<i32>().ok())
            .filter(|fd| *fd > 2)
        {
            fds.push(fd);
        }
    }
    fds
}

/// A pipe already holding `bytes` (closed for writing); returns its read
/// end, for a child to receive at a given descriptor number.
#[cfg(unix)]
#[allow(unsafe_code)] // pipe(2) and the write end's ownership
pub fn filled_pipe(bytes: &[u8]) -> io::Result<i32> {
    use std::io::Write;
    use std::os::fd::FromRawFd;
    let mut ends = [0i32; 2];
    // SAFETY: `ends` has room for the two descriptors pipe(2) writes.
    if unsafe { libc::pipe(ends.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the write end was just created and is owned only here.
    let mut writer = unsafe { fs::File::from_raw_fd(ends[1]) };
    writer.write_all(bytes)?;
    drop(writer);
    Ok(ends[0])
}

/// Give a child each `(target, source)` descriptor before it executes.
#[cfg(unix)]
#[allow(unsafe_code)] // dup2(2) in the forked child
pub fn hand_over(command: &mut std::process::Command, descriptors: Vec<(i32, i32)>) {
    use std::os::unix::process::CommandExt;
    if descriptors.is_empty() {
        return;
    }
    // SAFETY: only async-signal-safe dup2 runs between fork and exec.
    unsafe {
        command.pre_exec(move || {
            for (target, source) in &descriptors {
                if libc::dup2(*source, *target) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

/// Content-addressed file storage under `<dir>/blobs/ab/<sha256>`.
#[derive(Clone, Debug)]
pub struct Blobs {
    dir: PathBuf,
}

/// Blobs this large are stored as pages; the SQLCipher and SQLite page size.
pub const PAGED_MIN: usize = 64 * 1024;
const PAGE: usize = 4096;

impl Blobs {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.join("blobs"),
        }
    }

    fn path(&self, hash: &str) -> PathBuf {
        self.dir.join(&hash[..2]).join(hash)
    }

    /// Store `bytes` under their sha256. Small blobs are one gzip file;
    /// files of `PAGED_MIN` bytes or more (books) are split into 4 KiB pages,
    /// each stored once by its own hash, plus a list of page hashes, since
    /// most of a book's pages recur across sessions.
    pub fn put(&self, bytes: &[u8]) -> io::Result<String> {
        let hash = hex(&Sha256::digest(bytes));
        if bytes.len() < PAGED_MIN {
            self.put_raw(&hash, bytes)?;
            return Ok(hash);
        }
        let list = self.path(&hash).with_extension("pages");
        if !list.exists() {
            let mut hashes = Vec::with_capacity(bytes.len() / PAGE * 32);
            for page in bytes.chunks(PAGE) {
                let digest = Sha256::digest(page);
                self.put_raw(&hex(&digest), page)?;
                hashes.extend_from_slice(&digest);
            }
            self.write_compressed(&list, &hashes)?;
        }
        Ok(hash)
    }

    fn put_raw(&self, hash: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.path(hash).with_extension("gz");
        if path.exists() || self.path(hash).exists() {
            return Ok(());
        }
        self.write_compressed(&path, bytes)
    }

    fn write_compressed(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        use flate2::write::GzEncoder;
        fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::new(9));
        io::Write::write_all(&mut encoder, bytes)?;
        fs::write(&tmp, encoder.finish()?)?;
        fs::rename(&tmp, path)
    }

    fn read_compressed(path: &Path) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        io::Read::read_to_end(
            &mut flate2::read::GzDecoder::new(fs::File::open(path)?),
            &mut out,
        )?;
        Ok(out)
    }

    pub fn get(&self, hash: &str) -> io::Result<Vec<u8>> {
        let list = self.path(hash).with_extension("pages");
        if list.exists() {
            let hashes = Self::read_compressed(&list)?;
            let mut out = Vec::with_capacity(hashes.len() / 32 * PAGE);
            for digest in hashes.chunks(32) {
                out.extend(self.get(&hex(digest))?);
            }
            return Ok(out);
        }
        let compressed = self.path(hash).with_extension("gz");
        if compressed.exists() {
            return Self::read_compressed(&compressed);
        }
        fs::read(self.path(hash))
    }

    /// The blob files a stored blob consists of (itself, or its page list and
    /// pages), for copying a selection of sessions into a smaller store.
    pub fn files(&self, hash: &str) -> io::Result<Vec<PathBuf>> {
        let list = self.path(hash).with_extension("pages");
        if list.exists() {
            let mut files = vec![list.clone()];
            for digest in Self::read_compressed(&list)?.chunks(32) {
                files.push(self.path(&hex(digest)).with_extension("gz"));
            }
            return Ok(files);
        }
        let compressed = self.path(hash).with_extension("gz");
        Ok(vec![if compressed.exists() {
            compressed
        } else {
            self.path(hash)
        }])
    }

    /// The stored (compressed) size of a blob.
    pub fn stored_size(&self, hash: &str) -> u64 {
        let compressed = self.path(hash).with_extension("gz");
        fs::metadata(&compressed)
            .or_else(|_| fs::metadata(self.path(hash)))
            .map(|m| m.len())
            .unwrap_or(0)
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
            encryption: None,
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
                    encryption: None,
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
    fn large_blobs_are_stored_as_shared_pages() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = Blobs::new(dir.path());
        let mut first = vec![7u8; PAGED_MIN + 100];
        let a = blobs.put(&first).unwrap();
        first[5] = 8;
        let b = blobs.put(&first).unwrap();
        assert_ne!(a, b);
        assert_eq!(blobs.get(&b).unwrap(), first);
        // The two share every page but the first.
        let files_a = blobs.files(&a).unwrap();
        let files_b = blobs.files(&b).unwrap();
        let shared = files_a.iter().filter(|f| files_b.contains(f)).count();
        assert!(shared >= files_a.len() - 2, "{shared} of {}", files_a.len());
        assert_eq!(blobs.get(&blobs.put(b"small").unwrap()).unwrap(), b"small");
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

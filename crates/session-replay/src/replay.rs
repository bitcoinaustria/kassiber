//! Run one recorded session against the current binary and compare.

use sha2::Digest;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;

use crate::compare::{identities, pin_inputs, pin_shared_uuid5, Canon, Window};
use crate::{dbdump, entry_path, restore, Blobs, Entry, Session};

/// How long a replayed process may run, and wait for the output a recorded
/// input followed.
pub const PROCESS_TIMEOUT: Duration = Duration::from_secs(120);

/// Where recordings keep their state roots: `record.sh` runs the suite in a
/// mount namespace with a build directory bound here.
pub const NAMESPACE_ROOT: &str = "/mnt/";

/// Bind the scratch copy at /mnt as root of a new user namespace, then drop
/// back to the caller's uid and gid in a nested one and run the process.
/// The tools come as absolute paths: the replayed environment's PATH is the
/// session's, which may hide them on purpose.
const NAMESPACE_SCRIPT: &str =
    "\"$5\" --bind \"$1\" /mnt && cd \"$2\" && u=$3 && g=$4 && un=$6 && shift 6 && \
     exec \"$un\" --user --map-user=\"$u\" --map-group=\"$g\" \"$@\"";

/// `name` on the runner's own PATH.
fn tool(name: &str) -> Result<PathBuf, String> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("{name} is not on PATH"))
}

/// Where a recorded root lives on disk during a replay.
fn disk_path(scratch: &Path, index: usize, recorded: &str, namespaced: bool) -> PathBuf {
    if namespaced {
        return scratch
            .join("mnt")
            .join(recorded.trim_start_matches(NAMESPACE_ROOT));
    }
    let name = Path::new(recorded)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    scratch.join(format!("r{index}")).join(name)
}

/// How far before a recorded process its test may have written state.
pub const RECORDING_LOOKBACK_US: i64 = 600_000_000;

pub struct Outcome {
    pub differences: Vec<String>,
    /// What this run printed and left, for re-recording a session after a
    /// reviewed change (`session-replay replay --update`). Output is in the
    /// recorded paths; `after` blobs live in `store`.
    pub replayed: Replayed,
}

pub struct Replayed {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit: crate::Exit,
    pub after: Vec<Vec<Entry>>,
    pub store: PathBuf,
    /// When this run's process started and ended (µs since the epoch): a
    /// re-recorded session takes them as its window, so the clock readings
    /// it now holds are recognized as generated.
    pub started_us: i64,
    pub ended_us: i64,
}

/// Path substitutions, longest first: recorded text to replay text.
#[derive(Clone, Debug, Default)]
pub struct Paths(Vec<(String, String)>);

impl Paths {
    pub fn add(&mut self, recorded: &str, replayed: &str) {
        if !recorded.is_empty() && recorded != replayed {
            self.0.push((recorded.to_owned(), replayed.to_owned()));
            self.0.sort_by_key(|p| std::cmp::Reverse(p.0.len()));
        }
    }

    pub fn forward(&self, text: &str) -> String {
        self.0
            .iter()
            .fold(text.to_owned(), |t, (a, b)| t.replace(a, b))
    }

    pub fn back(&self, text: &str) -> String {
        let mut back: Vec<_> = self.0.iter().map(|(a, b)| (b.clone(), a.clone())).collect();
        back.sort_by_key(|p| std::cmp::Reverse(p.0.len()));
        back.iter()
            .fold(text.to_owned(), |t, (a, b)| t.replace(a, b))
    }
}

fn now_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

fn local_offset_us() -> i64 {
    // The recording machine's offset is unknown; tests run in UTC or in the
    // local zone of the same machine, and TZ is recorded with the session.
    0
}

/// A state tree as comparable artifacts: path to canonical text.
/// The members of a ZIP archive (XLSX and ZIP exports), in archive order;
/// `None` for anything else.
fn zip_members(bytes: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    if !bytes.starts_with(b"PK\x03\x04") {
        return None;
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let mut members = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let mut member = archive.by_index(index).ok()?;
        let mut body = Vec::new();
        std::io::Read::read_to_end(&mut member, &mut body).ok()?;
        members.push((member.name().ok()?.to_string(), body));
    }
    Some(members)
}

fn artifacts(
    root: &Path,
    entries: &[Entry],
    blobs: &Blobs,
    canon: &mut Canon<'_>,
    paths: Option<&Paths>,
) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in entries {
        let name = entry_path(entry).to_owned();
        if ignored(&name) {
            continue;
        }
        let text = match entry {
            Entry::Dir { mode, .. } => format!("dir {mode:o}"),
            Entry::Symlink { target, .. } => format!("symlink {}", canon.rewrite(target, true)),
            Entry::File {
                blob,
                mode,
                encryption,
                ..
            } => {
                let bytes = blobs.get(blob).unwrap_or_default();
                let sealed = encryption
                    .as_ref()
                    .map(|e| format!("encrypted book, {}-byte pages\n", e.page_size))
                    .unwrap_or_default();
                let body = if dbdump::is_sqlite(&bytes) {
                    // A disposable copy with its write-ahead log beside it.
                    let file = root.join(".dump.sqlite");
                    let wal = root.join(".dump.sqlite-wal");
                    let _ = fs::write(&file, &bytes);
                    let _ = fs::remove_file(&wal);
                    let _ = fs::remove_file(root.join(".dump.sqlite-shm"));
                    let wal_name = format!("{}-wal", entry_path(entry));
                    if let Some(Entry::File { blob, .. }) =
                        entries.iter().find(|e| entry_path(e) == wal_name)
                    {
                        let _ = fs::write(&wal, blobs.get(blob).unwrap_or_default());
                    }
                    let dumped = dbdump::dump(&file);
                    for leftover in [&file, &wal, &root.join(".dump.sqlite-shm")] {
                        let _ = fs::remove_file(leftover);
                    }
                    match dumped {
                        Ok(blocks) => blocks
                            .into_iter()
                            .map(|(title, rows)| {
                                let rows: Vec<String> =
                                    rows.iter().map(|r| back(paths, r)).collect();
                                format!("{title}\n{}", canon.rows(&rows).join("\n"))
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                        Err(error) => format!("database that does not open: {error}"),
                    }
                } else if let Ok(text) = std::str::from_utf8(&bytes) {
                    canon.rewrite(&back(paths, text), true)
                } else if let Some(members) = zip_members(&bytes) {
                    // An XLSX or ZIP export: each member compares as text
                    // (with the usual rules, so its creation time is a clock
                    // reading) or, if binary, by its bytes.
                    members
                        .into_iter()
                        .map(|(name, body)| match String::from_utf8(body) {
                            Ok(text) => format!(
                                "member {name}\n{}",
                                canon.rewrite(&back(paths, &text), true)
                            ),
                            Err(error) => format!(
                                "member {name}: binary {}",
                                crate::hex(&sha2::Sha256::digest(error.as_bytes()))
                            ),
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    // Binary content (an encrypted book, an archive): its size.
                    format!("binary {} bytes", bytes.len())
                };
                format!("file {mode:o}\n{sealed}{body}")
            }
        };
        // Names that carry generated values (an attachment's uuid, an
        // export's timestamp) compare by shape; same-shaped files as a set.
        let key = canon.rewrite(&back(paths, &name), false);
        out.entry(key).or_default().push(text);
    }
    for texts in out.values_mut() {
        texts.sort();
    }
    out
}

fn back(paths: Option<&Paths>, text: &str) -> String {
    paths
        .map(|p| p.back(text))
        .unwrap_or_else(|| text.to_owned())
}

/// Every passphrase a session hands over or recorded a book under.
fn session_passphrases(session: &Session, blobs: &Blobs) -> Vec<String> {
    let fds: Vec<Vec<u8>> = session
        .fds
        .values()
        .map(|d| {
            base64::engine::general_purpose::STANDARD
                .decode(d)
                .unwrap_or_default()
        })
        .collect();
    let stdin: Vec<u8> = session
        .stdin
        .iter()
        .filter_map(|c| c.data.as_ref())
        .flat_map(|d| {
            base64::engine::general_purpose::STANDARD
                .decode(d)
                .unwrap_or_default()
        })
        .collect();
    let mut found = crate::passphrase_candidates(&fds, &stdin);
    for root in &session.roots {
        for entry in root.before.iter().chain(&root.after) {
            if let Entry::File {
                encryption: Some(e),
                ..
            } = entry
            {
                if !found.contains(&e.passphrase) {
                    found.push(e.passphrase.clone());
                }
            }
        }
    }
    let _ = blobs;
    found
}

/// Encrypt the restored books again (decision B): with their original
/// ciphertext where the recording kept it, otherwise through the product's
/// encryption, verified (`encrypted::encrypt`). A failure fails the replay.
fn encrypt_books(
    root: &Path,
    file: bool,
    entries: &[Entry],
    blobs: &Blobs,
    scratch: &Path,
) -> Result<(), String> {
    for entry in entries {
        let Entry::File {
            path,
            blob,
            encryption: Some(encryption),
            ..
        } = entry
        else {
            continue;
        };
        let target = if file {
            root.to_path_buf()
        } else {
            root.join(path)
        };
        let bytes = match encryption
            .ciphertext
            .as_ref()
            .and_then(|c| blobs.get(c).ok())
        {
            Some(ciphertext) => ciphertext,
            None => {
                let plaintext = blobs.get(blob).map_err(|e| format!("{path}: {e}"))?;
                crate::encrypted::encrypt(&plaintext, encryption, scratch)
                    .map_err(|e| format!("{path}: {e}"))?
            }
        };
        fs::write(&target, bytes).map_err(|e| format!("{path}: {e}"))?;
    }
    let _ = fs::remove_dir_all(scratch);
    Ok(())
}

/// Files that are never compared: SQLite's transient journals and lock files.
fn ignored(name: &str) -> bool {
    let base = name.rsplit('/').next().unwrap_or(name);
    name.ends_with("-wal")
        || name.ends_with("-shm")
        || name.ends_with("-journal")
        || name.ends_with(".lock")
        // Ownership locks name the live process (pid, generation) and the
        // locked path; they are runtime state, never a result.
        || base.starts_with(".operator-owner.lock")
        || base.contains(".lock.")
}

fn pinned_inputs(session: &Session, blobs: &Blobs) -> HashSet<String> {
    let mut pinned = HashSet::new();
    for arg in &session.argv {
        pin_inputs(arg, &mut pinned);
    }
    for value in session.env.values() {
        pin_inputs(value, &mut pinned);
    }
    // Stdin identities the process printed first are echoes of generated
    // values (a test reading an id back), not inputs.
    let printed: HashSet<String> = identities(&String::from_utf8_lossy(
        &blobs.get(&session.stdout).unwrap_or_default(),
    ))
    .into_iter()
    .collect();
    for chunk in &session.stdin {
        if let Some(data) = &chunk.data {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .unwrap_or_default();
            let mut found = HashSet::new();
            pin_inputs(&String::from_utf8_lossy(&bytes), &mut found);
            pinned.extend(found.into_iter().filter(|token| !printed.contains(token)));
        }
    }
    for root in &session.roots {
        for entry in &root.before {
            if let Entry::File { blob, .. } = entry {
                let bytes = blobs.get(blob).unwrap_or_default();
                if dbdump::is_sqlite(&bytes) {
                    let dir =
                        std::env::temp_dir().join(format!("kassiber-pin-{}", std::process::id()));
                    let _ = fs::create_dir_all(&dir);
                    let file = dir.join("pin.sqlite");
                    let _ = fs::write(&file, &bytes);
                    if let Ok(blocks) = dbdump::dump(&file) {
                        for (_, rows) in blocks {
                            for row in rows {
                                pin_inputs(&row, &mut pinned);
                            }
                        }
                    }
                    let _ = fs::remove_dir_all(&dir);
                } else {
                    pin_inputs(&String::from_utf8_lossy(&bytes), &mut pinned);
                }
            }
        }
    }
    pinned
}

/// Restore, run, and compare one session. `binaries` maps a session's
/// binary name to the executable to run; `scratch` is an empty directory.
pub fn replay(
    session: &Session,
    blobs: &Blobs,
    binaries: &BTreeMap<String, PathBuf>,
    repo: &Path,
    scratch: &Path,
    profile: Option<&Path>,
) -> Result<Outcome, String> {
    let binary = binaries
        .get(&session.binary)
        .ok_or_else(|| format!("no binary for {}", session.binary))?;
    let mut paths = Paths::default();
    // Sessions recorded under NAMESPACE_ROOT replay at the very same paths:
    // the process runs in a user and mount namespace with that directory
    // bound to this replay's scratch copy, so path-derived values (keychain
    // service ids, book hashes) match. Other roots are relocated.
    let mounted = scratch.join("mnt");
    let namespaced = session
        .roots
        .iter()
        .all(|r| r.path.starts_with(NAMESPACE_ROOT));
    for (index, root) in session.roots.iter().enumerate() {
        let on_disk = disk_path(scratch, index, &root.path, namespaced);
        if !namespaced {
            paths.add(&root.path, &on_disk.to_string_lossy());
        }
        restore(&on_disk, root.file, &root.before, blobs)
            .map_err(|e| format!("restore {}: {e}", root.path))?;
        encrypt_books(
            &on_disk,
            root.file,
            &root.before,
            blobs,
            &scratch.join("encrypt"),
        )?;
    }
    paths.add(&session.repo, &repo.to_string_lossy());
    if !namespaced {
        for (index, root) in session.roots.iter().enumerate() {
            relocate(&disk_path(scratch, index, &root.path, false), &paths)?;
        }
    }
    let tmp = scratch.join("tmp");
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    if namespaced {
        for dir in ["bt", "tmp"] {
            fs::create_dir_all(mounted.join(dir)).map_err(|e| e.to_string())?;
        }
    } else if let Some(recorded_tmp) = session.env.get("TMPDIR") {
        paths.add(recorded_tmp, &tmp.to_string_lossy());
    }
    let cwd = paths.forward(&session.cwd);
    let cwd = if Path::new(&cwd).is_dir() || cwd.starts_with(NAMESPACE_ROOT) {
        cwd
    } else {
        scratch.to_string_lossy().into_owned()
    };

    let mut command = if namespaced {
        #[cfg(unix)]
        let (uid, gid) = {
            use std::os::unix::fs::MetadataExt;
            let meta = fs::metadata(scratch).map_err(|e| e.to_string())?;
            (meta.uid(), meta.gid())
        };
        #[cfg(not(unix))]
        let (uid, gid) = (0, 0);
        let unshare = tool("unshare")?;
        let mut command = Command::new(&unshare);
        command.arg("-Urm").arg(tool("sh")?);
        command.args(["-c", NAMESPACE_SCRIPT, "sh"]);
        command
            .arg(&mounted)
            .arg(&cwd)
            .arg(uid.to_string())
            .arg(gid.to_string())
            .arg(tool("mount")?)
            .arg(&unshare);
        // unshare cannot set argv[0]; `session-replay exec-as` does.
        if let Some(argv0) = &session.argv0 {
            let tool = std::env::current_exe().map_err(|e| e.to_string())?;
            command.arg(tool).arg("exec-as").arg(argv0);
        }
        command.arg(binary);
        command
    } else {
        let mut command = Command::new(binary);
        command.current_dir(&cwd);
        #[cfg(unix)]
        if let Some(argv0) = &session.argv0 {
            use std::os::unix::process::CommandExt;
            command.arg0(argv0);
        }
        command
    };
    command.args(session.argv.iter().map(|a| paths.forward(a)));
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    // The runner's own I/O shim (the tests' fast-io wrapper), never the
    // session's.
    // Every session replays on the clock shim beside the session-replay
    // binary: one recorded on the shared SQLite clock replays on it, and
    // every one sees the wall calendar of its recording (realtime is the
    // recorded start plus real elapsed time), so "today" and certificate
    // validity do not move with the date of the replay.
    let clock = std::env::current_exe().ok().and_then(|exe| {
        exe.parent()
            .map(|dir| dir.join("libkassiber_clock_shim.so"))
    });
    if let Some(clock) = &clock {
        if !clock.is_file() {
            return Err(format!(
                "{} is missing; build kassiber-clock-shim",
                clock.display()
            ));
        }
    }
    let preload: Vec<String> = clock
        .iter()
        .map(|c| c.to_string_lossy().into_owned())
        .chain(std::env::var("LD_PRELOAD").ok().filter(|p| !p.is_empty()))
        .collect();
    if !preload.is_empty() {
        command.env("LD_PRELOAD", preload.join(":"));
    }
    command.env("HOME", scratch.join("home"));
    command.env(
        "KASSIBER_REPLAY_REALTIME_BASE_US",
        session.started_us.to_string(),
    );
    for (key, value) in &session.env {
        if key == "PATH" {
            // The recorded search path, minus directories this host lacks
            // (the recording account's tool installs); directories of the
            // session's own roots map to the replay's.
            let entries: Vec<String> = value
                .split(':')
                .map(|entry| paths.forward(entry))
                .filter(|entry| {
                    !entry.is_empty()
                        && ((namespaced && entry.starts_with(NAMESPACE_ROOT))
                            || Path::new(entry).is_dir())
                })
                .collect();
            if !entries.is_empty() {
                command.env("PATH", entries.join(":"));
            }
            continue;
        }
        let value = paths.forward(value);
        // A recorded path that is not part of the session (the recording
        // account's config or runtime dirs) is never handed to a replay.
        let inside = value.starts_with(NAMESPACE_ROOT) && namespaced;
        let outside = Path::new(&value).is_absolute()
            && !inside
            && !value.starts_with(&*scratch.to_string_lossy())
            && !value.starts_with(&*repo.to_string_lossy());
        if outside {
            command.env(key, scratch.join("env").join(key));
        } else {
            command.env(key, value);
        }
    }
    if !session.env.contains_key("TMPDIR") {
        command.env("TMPDIR", &tmp);
    }
    // A coverage-instrumented binary writes its counters here.
    if let Some(profile) = profile {
        command.env("LLVM_PROFILE_FILE", profile.join("%p-%m.profraw"));
    }
    // Connections replay from the recorded tape; with none recorded, an
    // empty tape refuses every connection instead of reaching the network.
    let tape = scratch.join("net.tape");
    let recorded_tape = match &session.net_tape {
        Some(blob) => blobs.get(blob).map_err(|e| e.to_string())?,
        None => Vec::new(),
    };
    fs::write(&tape, recorded_tape).map_err(|e| e.to_string())?;
    command.env(
        "KASSIBER_TEST_NET_TAPE",
        format!("replay:{}", tape.display()),
    );
    // Ownership locks rendezvous outside the state roots; a test-harness
    // binary keeps them in this replay's own directory instead.
    if !session.env.contains_key("KASSIBER_RUST_TEST_OWNER_ROOT") {
        command.env("KASSIBER_RUST_TEST_OWNER_ROOT", scratch.join("owners"));
    }
    // The recorded passphrase pipes, at the same descriptor numbers.
    let mut handed = Vec::new();
    for (fd, data) in &session.fds {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap_or_default();
        handed.push((*fd, crate::filled_pipe(&bytes).map_err(|e| e.to_string())?));
    }
    crate::hand_over(&mut command, handed);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let started = now_us();
    let mut child = command
        .spawn()
        .map_err(|e| format!("start {}: {e}", binary.display()))?;
    let lines = Arc::new(AtomicU64::new(0));
    let printed = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let mut stdout = child.stdout.take().unwrap();
    let counter = Arc::clone(&lines);
    let shared = Arc::clone(&printed);
    let closed_after = session.stdout_closed_after;
    let out = std::thread::spawn(move || {
        let mut buffer = vec![0u8; 65536];
        while let Ok(n) = stdout.read(&mut buffer) {
            if n == 0 {
                break;
            }
            // Where the recorded reader closed the pipe, this one does too:
            // the rest was never read.
            if let Some(limit) = closed_after {
                let mut kept = shared.lock().unwrap();
                let room = limit.saturating_sub(kept.len() as u64) as usize;
                kept.extend_from_slice(&buffer[..n.min(room)]);
                if n >= room {
                    break;
                }
                drop(kept);
                counter.fetch_add(
                    buffer[..n].iter().filter(|b| **b == b'\n').count() as u64,
                    Ordering::SeqCst,
                );
                continue;
            }
            shared.lock().unwrap().extend_from_slice(&buffer[..n]);
            counter.fetch_add(
                buffer[..n].iter().filter(|b| **b == b'\n').count() as u64,
                Ordering::SeqCst,
            );
        }
    });
    let recorded_lines: Vec<String> =
        String::from_utf8_lossy(&blobs.get(&session.stdout).unwrap_or_default())
            .lines()
            .map(str::to_owned)
            .collect();
    let mut stderr = child.stderr.take().unwrap();
    let err = std::thread::spawn(move || {
        let mut all = Vec::new();
        let _ = stderr.read_to_end(&mut all);
        all
    });
    let mut stdin = child.stdin.take();
    let pid = child.id() as i32;
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    let mut differences = Vec::new();
    let wait_for = |count: u64| -> bool {
        while lines.load(Ordering::SeqCst) < count {
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        true
    };
    let mut events: Vec<(u64, usize, bool)> = Vec::new();
    for (i, chunk) in session.stdin.iter().enumerate() {
        events.push((chunk.after_lines, i, false));
    }
    for (i, signal) in session.signals.iter().enumerate() {
        events.push((signal.after_lines, i, true));
    }
    events.sort_by_key(|(after, index, signal)| (*after, *signal, *index));
    for (after, index, is_signal) in events {
        if !wait_for(after) {
            differences.push(format!(
                "timed out waiting for {after} stdout lines before an input"
            ));
            break;
        }
        if is_signal {
            // SAFETY-free: the process is our child; `kill` is a plain command.
            let _ = Command::new("kill")
                .arg(format!("-{}", session.signals[index].signal))
                .arg(pid.to_string())
                .status();
            continue;
        }
        match &session.stdin[index].data {
            Some(data) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .unwrap_or_default();
                let mut text = paths.forward(&String::from_utf8_lossy(&bytes));
                // Ids the recorded run printed and the test sent back: send
                // the ids this run printed at the same positions instead.
                let replayed = String::from_utf8_lossy(&printed.lock().unwrap()).into_owned();
                for (recorded, now) in recorded_lines
                    .iter()
                    .zip(replayed.lines())
                    .take(after as usize)
                {
                    let (a, b) = (identities(recorded), identities(now));
                    if a.len() == b.len() {
                        for (from, to) in a.iter().zip(&b) {
                            if from != to {
                                text = text.replace(from.as_str(), to);
                            }
                        }
                    }
                }
                if let Some(pipe) = stdin.as_mut() {
                    let _ = pipe.write_all(text.as_bytes()).and_then(|_| pipe.flush());
                }
            }
            None => drop(stdin.take()),
        }
    }
    drop(stdin.take());
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() > deadline => {
                let _ = child.kill();
                differences.push("the replay timed out".into());
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => return Err(error.to_string()),
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let ended = now_us();
    let _ = out.join();
    let stdout = std::mem::take(&mut *printed.lock().unwrap());
    let stderr = err.join().unwrap_or_default();

    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    if status.code() != session.exit.code || signal != session.exit.signal {
        differences.push(format!(
            "exit: recorded {:?}/{:?}, replayed {:?}/{:?}",
            session.exit.code,
            session.exit.signal,
            status.code(),
            signal
        ));
    }

    let mut pinned = pinned_inputs(session, blobs);
    pin_shared_uuid5(
        &String::from_utf8_lossy(&blobs.get(&session.stdout).unwrap_or_default()),
        &paths.back(&String::from_utf8_lossy(&stdout)),
        &mut pinned,
    );
    // The recorded state was written during the recording test, which began
    // before this process; its readings count as generated on both sides.
    let recorded_window = Window {
        start: session.started_us - RECORDING_LOOKBACK_US,
        end: session.ended_us,
        local_offset_us: local_offset_us(),
        also: session.rerecorded_us.clone(),
    };
    let replay_window = Window {
        start: started,
        end: ended,
        local_offset_us: local_offset_us(),
        also: std::iter::once((recorded_window.start, recorded_window.end))
            .chain(session.rerecorded_us.iter().copied())
            .collect(),
    };
    // Both sides are normalized with the same spans: a reading restored
    // from the starting state appears on both, and a rule that matched it on
    // one side only (the replay window's hour offsets line up with any
    // instant whose minutes and seconds match the moment the replay runs)
    // would make equal state differ.
    let mut left = Canon::new(replay_window.clone(), &pinned);
    let mut right = Canon::new(replay_window, &pinned);
    let recorded_out =
        String::from_utf8_lossy(&blobs.get(&session.stdout).unwrap_or_default()).into_owned();
    let recorded_err =
        String::from_utf8_lossy(&blobs.get(&session.stderr).unwrap_or_default()).into_owned();
    let mut replay_out = paths.back(&String::from_utf8_lossy(&stdout));
    let replay_err = paths.back(&String::from_utf8_lossy(&stderr));
    // Concurrent daemon requests interleave their records freely.
    let daemon = session.argv.iter().any(|a| a == "daemon");
    let recorded_out = if daemon {
        crate::compare::group_by_request(&recorded_out)
    } else {
        recorded_out
    };
    if daemon {
        replay_out = crate::compare::group_by_request(&replay_out);
    }
    let (a, b) = (
        left.rewrite(&recorded_out, true),
        right.rewrite(&replay_out, true),
    );
    if a != b {
        differences.push(first_difference("stdout", &a, &b));
    }
    let (a, b) = (
        left.rewrite(&recorded_err, true),
        right.rewrite(&replay_err, true),
    );
    if a != b {
        differences.push(first_difference("stderr", &a, &b));
    }
    let scan_blobs = Blobs::new(&scratch.join("after-store"));
    let passphrases = session_passphrases(session, blobs);
    let mut all_after = Vec::new();
    for (index, root) in session.roots.iter().enumerate() {
        let new = disk_path(scratch, index, &root.path, namespaced);
        let mut after = if new.exists() {
            crate::snapshot(&new, &scan_blobs).map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        // Encrypted books this run left compare decrypted, like the recording.
        crate::decrypt_books(
            &mut after,
            &passphrases,
            &scan_blobs,
            &scratch.join("decrypt"),
        );
        let dump_dir = scratch.join("dump");
        fs::create_dir_all(&dump_dir).map_err(|e| e.to_string())?;
        let recorded = artifacts(&dump_dir, &root.after, blobs, &mut left, None);
        let replayed = artifacts(&dump_dir, &after, &scan_blobs, &mut right, Some(&paths));
        all_after.push(after.clone());
        for name in recorded
            .keys()
            .chain(replayed.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            match (recorded.get(name), replayed.get(name)) {
                (Some(a), Some(b)) if a == b => {}
                (Some(a), Some(b)) => differences.push(first_difference(
                    &format!("{}/{name}", root.path),
                    &a.join("\n--\n"),
                    &b.join("\n--\n"),
                )),
                (Some(_), None) => {
                    differences.push(format!("{}/{name}: missing after replay", root.path))
                }
                (None, Some(_)) => {
                    differences.push(format!("{}/{name}: only after replay", root.path))
                }
                (None, None) => {}
            }
        }
    }
    Ok(Outcome {
        differences,
        replayed: Replayed {
            stdout: paths.back(&String::from_utf8_lossy(&stdout)).into_bytes(),
            stderr: paths.back(&String::from_utf8_lossy(&stderr)).into_bytes(),
            exit: crate::Exit {
                code: status.code(),
                signal,
            },
            after: all_after,
            store: scratch.join("after-store"),
            started_us: started,
            ended_us: ended,
        },
    })
}

/// Rewrite recorded paths inside a restored state: text files by
/// substitution, SQLite books in every text column that holds one.
fn relocate(root: &Path, paths: &Paths) -> Result<(), String> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            for entry in fs::read_dir(&path).map_err(|e| e.to_string())?.flatten() {
                pending.push(entry.path());
            }
        } else if meta.is_file() {
            files.push(path);
        }
    }
    for file in files {
        let bytes = fs::read(&file).map_err(|e| e.to_string())?;
        if dbdump::is_sqlite(&bytes) {
            relocate_database(&file, paths)?;
        } else if let Ok(text) = std::str::from_utf8(&bytes) {
            let moved = paths.forward(text);
            if moved != text {
                fs::write(&file, moved).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn relocate_database(file: &Path, paths: &Paths) -> Result<(), String> {
    use rusqlite::Connection;
    let conn = Connection::open(file).map_err(|e| e.to_string())?;
    // Triggers must not see the rewrite (audit rows, append-only refusals):
    // drop them, rewrite, and recreate them from their recorded SQL.
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| e.to_string())?;
    let triggers: Vec<(String, String)> = conn
        .prepare("SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND sql IS NOT NULL")
        .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect())
        .map_err(|e| e.to_string())?;
    for (name, _) in &triggers {
        conn.execute_batch(&format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\"")))
            .map_err(|e| e.to_string())?;
    }
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
        .map_err(|e| e.to_string())?;
    for table in tables {
        let quoted = table.replace('"', "\"\"");
        let columns: Vec<String> = conn
            .prepare(&format!(
                "SELECT name FROM pragma_table_info('{}')",
                table.replace('\'', "''")
            ))
            .and_then(|mut s| s.query_map([], |r| r.get(0))?.collect())
            .map_err(|e| e.to_string())?;
        for column in columns {
            let col = column.replace('"', "\"\"");
            for (old, new) in &paths.0 {
                let _ = conn.execute(
                    &format!(
                        "UPDATE \"{quoted}\" SET \"{col}\" = replace(\"{col}\", ?1, ?2) \
                         WHERE typeof(\"{col}\") = 'text' AND instr(\"{col}\", ?1) > 0"
                    ),
                    rusqlite::params![old, new],
                );
            }
        }
    }
    for (_, sql) in &triggers {
        conn.execute_batch(sql).map_err(|e| e.to_string())?;
    }
    conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
    Ok(())
}

fn first_difference(label: &str, a: &str, b: &str) -> String {
    let mut at = a
        .bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    while !a.is_char_boundary(at) {
        at -= 1;
    }
    let start = a[..at]
        .char_indices()
        .rev()
        .nth(80)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let clip = |s: &str| {
        let start = (start..=s.len())
            .find(|i| s.is_char_boundary(*i))
            .unwrap_or(s.len());
        let end = (at + 160).min(s.len());
        let end = (end..=s.len())
            .find(|i| s.is_char_boundary(*i))
            .unwrap_or(s.len());
        s[start..end].replace('\n', "\\n")
    };
    format!(
        "{label}: differs at byte {at}\n    recorded: {}\n    replayed: {}",
        clip(a),
        clip(b)
    )
}

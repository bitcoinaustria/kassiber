//! Run one recorded session against the current binary and compare.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;

use crate::compare::{identities, pin_inputs, Canon, Window};
use crate::{dbdump, entry_path, restore, Blobs, Entry, Session};

/// How long a replayed process may run, and wait for the output a recorded
/// input followed.
pub const PROCESS_TIMEOUT: Duration = Duration::from_secs(120);

/// Where recordings keep their state roots: `record.sh` runs the suite in a
/// mount namespace with a build directory bound here.
pub const NAMESPACE_ROOT: &str = "/mnt/";

/// Bind the scratch copy at /mnt as root of a new user namespace, then drop
/// back to the caller's uid and gid in a nested one and run the process.
const NAMESPACE_SCRIPT: &str =
    "mount --bind \"$1\" /mnt && cd \"$2\" && u=$3 && g=$4 && shift 4 && \
     exec unshare --user --map-user=\"$u\" --map-group=\"$g\" \"$@\"";

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
            Entry::File { blob, mode, .. } => {
                let bytes = blobs.get(blob).unwrap_or_default();
                let body = if dbdump::is_sqlite(&bytes) {
                    let file = root.join(".dump.sqlite");
                    let _ = fs::write(&file, &bytes);
                    let dumped = dbdump::dump(&file);
                    let _ = fs::remove_file(&file);
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
                } else {
                    // Binary content (an encrypted book, an archive): its size.
                    format!("binary {} bytes", bytes.len())
                };
                format!("file {mode:o}\n{body}")
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

/// Files that are never compared: SQLite's transient journals and lock files.
fn ignored(name: &str) -> bool {
    let base = name.rsplit('/').next().unwrap_or(name);
    name.ends_with("-wal")
        || name.ends_with("-shm")
        || name.ends_with("-journal")
        || name.ends_with(".lock")
        // Ownership locks name the live process (pid, generation); they are
        // runtime state, never a result.
        || base.starts_with(".operator-owner.lock")
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
        let mut command = Command::new("unshare");
        command.args(["-Urm", "sh", "-c", NAMESPACE_SCRIPT, "sh"]);
        command
            .arg(&mounted)
            .arg(&cwd)
            .arg(uid.to_string())
            .arg(gid.to_string())
            .arg(binary);
        command
    } else {
        let mut command = Command::new(binary);
        command.current_dir(&cwd);
        command
    };
    command.args(session.argv.iter().map(|a| paths.forward(a)));
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    command.env("HOME", scratch.join("home"));
    for (key, value) in &session.env {
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
    // Ownership locks rendezvous outside the state roots; a test-harness
    // binary keeps them in this replay's own directory instead.
    if !session.env.contains_key("KASSIBER_RUST_TEST_OWNER_ROOT") {
        command.env("KASSIBER_RUST_TEST_OWNER_ROOT", scratch.join("owners"));
    }
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
    let out = std::thread::spawn(move || {
        let mut buffer = vec![0u8; 65536];
        while let Ok(n) = stdout.read(&mut buffer) {
            if n == 0 {
                break;
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

    let pinned = pinned_inputs(session, blobs);
    // The recorded state was written during the recording test, which began
    // before this process; its readings count as generated on both sides.
    let recorded_window = Window {
        start: session.started_us - RECORDING_LOOKBACK_US,
        end: session.ended_us,
        local_offset_us: local_offset_us(),
        also: None,
    };
    let replay_window = Window {
        start: started,
        end: ended,
        local_offset_us: local_offset_us(),
        also: Some((recorded_window.start, recorded_window.end)),
    };
    let mut left = Canon::new(recorded_window, &pinned);
    let mut right = Canon::new(replay_window, &pinned);
    let recorded_out =
        String::from_utf8_lossy(&blobs.get(&session.stdout).unwrap_or_default()).into_owned();
    let recorded_err =
        String::from_utf8_lossy(&blobs.get(&session.stderr).unwrap_or_default()).into_owned();
    let replay_out = paths.back(&String::from_utf8_lossy(&stdout));
    let replay_err = paths.back(&String::from_utf8_lossy(&stderr));
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
    for (index, root) in session.roots.iter().enumerate() {
        let new = disk_path(scratch, index, &root.path, namespaced);
        let after = if new.exists() {
            crate::snapshot(&new, &scan_blobs).map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        let dump_dir = scratch.join("dump");
        fs::create_dir_all(&dump_dir).map_err(|e| e.to_string())?;
        let recorded = artifacts(&dump_dir, &root.after, blobs, &mut left, None);
        let replayed = artifacts(&dump_dir, &after, &scan_blobs, &mut right, Some(&paths));
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
    Ok(Outcome { differences })
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

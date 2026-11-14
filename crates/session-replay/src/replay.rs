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

/// The recording test that held a wallet's first history request in its
/// loopback server while it sent a freshness control (cancel, revoke,
/// pause, offline). The tape answers at once, so without the same hold
/// whether the control lands before the job's next check is a race with
/// how fast the fetch runs.
const HELD_FETCH_TEST: &str = "tests/test_rust_freshness_lifecycle.py::\
     test_live_freshness_controls_remain_available_during_fetch[";

/// Its inputs: the sync, a status read, a competing sync, then the control.
const HELD_CONTROL_INPUT: usize = 3;

/// The request a session's recording held, as tape-hold bytes.
fn held_request(session: &Session) -> Option<&'static str> {
    if !session.test.as_deref()?.starts_with(HELD_FETCH_TEST) {
        return None;
    }
    Some(
        match session
            .env
            .get("KASSIBER_BACKEND_FIXTURE_KIND")
            .map(String::as_str)
        {
            Some("electrum") => "blockchain.scripthash.get_history",
            _ => "/scripthash/",
        },
    )
}

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
    /// The state this run left, as on disk: still in the replay's paths.
    /// [`Replayed::recorded_after`] maps it back to the recorded ones.
    pub after: Vec<Vec<Entry>>,
    pub store: PathBuf,
    /// What this run sent at each recorded input (in the recorded paths):
    /// the recorded text with this run's printed ids (document tokens, job
    /// ids) in place of the recording's. `None` where it sent nothing (an
    /// EOF, or an input the run never reached).
    pub stdin: Vec<Option<String>>,
    /// The clock spans this run's readings came from (µs since the epoch):
    /// its real start and end, and the virtual spans the clock shim showed
    /// the process. A re-recorded session keeps them as windows, so the
    /// clock readings it now holds are recognized as generated.
    pub spans: Vec<(i64, i64)>,
    /// The comparison's path mapping (recorded to replayed).
    pub paths: Paths,
}

impl Replayed {
    /// The state this run left, in the recorded paths: every text file,
    /// SQLite text value and file name the comparison maps back through
    /// [`Paths::back`] is rewritten the same way, so a session re-recorded
    /// from another checkout (or temp dir, or account) replays clean from
    /// any. New blobs go into `store`; `work` is scratch space.
    pub fn recorded_after(&self, work: &Path) -> Result<Vec<Vec<Entry>>, String> {
        let store = Blobs::new(&self.store);
        self.after
            .iter()
            .map(|entries| recorded_entries(entries, &store, &self.paths, work))
            .collect()
    }
}

/// `session` as `replayed` left it, for `session-replay replay --update`:
/// its output, exit, the inputs it sent (with the ids it printed and read
/// back), its clock spans and the state it left, blobs copied into the
/// recording's `blobs`. Everything is in the recorded paths, mapped back
/// the way the comparison maps a replay, so the session replays clean
/// from any checkout. `work` is scratch space.
pub fn rerecorded(
    mut session: Session,
    blobs: &Blobs,
    replayed: &Replayed,
    work: &Path,
) -> Result<Session, String> {
    let after = replayed.recorded_after(work)?;
    let scan = Blobs::new(&replayed.store);
    let copy = |hash: &str| -> Result<String, String> {
        let bytes = scan.get(hash).map_err(|e| e.to_string())?;
        blobs.put(&bytes).map_err(|e| e.to_string())
    };
    session.stdout = blobs.put(&replayed.stdout).map_err(|e| e.to_string())?;
    session.stderr = blobs.put(&replayed.stderr).map_err(|e| e.to_string())?;
    session.exit = replayed.exit.clone();
    // Ids the replay printed and the test sent back (a document token)
    // must match the output recorded above: a later replay maps the ids of
    // recorded output lines onto its own, so a stale one would go out as is.
    for (chunk, sent) in session.stdin.iter_mut().zip(&replayed.stdin) {
        if let (Some(data), Some(sent)) = (chunk.data.as_mut(), sent) {
            *data = base64::engine::general_purpose::STANDARD.encode(sent);
        }
    }
    for span in &replayed.spans {
        if !session.rerecorded_us.contains(span) {
            session.rerecorded_us.push(*span);
        }
    }
    for (root, mut after) in session.roots.iter_mut().zip(after) {
        // A book's ciphertext stays only where the recording kept one
        // (`pack --keep-ciphertext`): nothing compares it, and the replay's
        // is a megabyte of pages no other session shares.
        let kept: HashSet<&str> = root
            .after
            .iter()
            .filter_map(|e| match e {
                Entry::File {
                    path,
                    encryption: Some(e),
                    ..
                } if e.ciphertext.is_some() => Some(path.as_str()),
                _ => None,
            })
            .collect();
        for entry in &mut after {
            if let Entry::File {
                path,
                blob,
                encryption,
                ..
            } = entry
            {
                *blob = copy(blob)?;
                if let Some(e) = encryption {
                    e.ciphertext = match &e.ciphertext {
                        Some(ciphertext) if kept.contains(path.as_str()) => Some(copy(ciphertext)?),
                        _ => None,
                    };
                }
            }
        }
        root.after = after;
    }
    Ok(session)
}

/// One root's entries with replayed paths mapped back (see
/// [`Replayed::recorded_after`]).
fn recorded_entries(
    entries: &[Entry],
    store: &Blobs,
    paths: &Paths,
    work: &Path,
) -> Result<Vec<Entry>, String> {
    let reverse = paths.reversed();
    let blob_of = |name: &str| {
        entries.iter().find_map(|e| match e {
            Entry::File { path, blob, .. } if path == name => store.get(blob).ok(),
            _ => None,
        })
    };
    // Journals of books rewritten below: the rewrite checkpoints them in.
    let mut absorbed = HashSet::new();
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut entry = entry.clone();
        if let Entry::File {
            path,
            blob,
            size,
            encryption,
            ..
        } = &mut entry
        {
            let bytes = store.get(blob).map_err(|e| format!("{path}: {e}"))?;
            let rewritten = if dbdump::is_sqlite(&bytes) {
                let wal = blob_of(&format!("{path}-wal")).unwrap_or_default();
                if reverse.named_in(&bytes) || reverse.named_in(&wal) {
                    fs::create_dir_all(work).map_err(|e| e.to_string())?;
                    let file = work.join("book.sqlite");
                    let mut leftovers = Vec::new();
                    for suffix in ["", "-wal", "-shm", "-journal"] {
                        let mut name = file.clone().into_os_string();
                        name.push(suffix);
                        let _ = fs::remove_file(&name);
                        leftovers.push(PathBuf::from(name));
                    }
                    fs::write(&file, &bytes).map_err(|e| e.to_string())?;
                    if !wal.is_empty() {
                        fs::write(&leftovers[1], &wal).map_err(|e| e.to_string())?;
                    }
                    relocate_database(&file, &reverse).map_err(|e| format!("{path}: {e}"))?;
                    let moved = fs::read(&file).map_err(|e| e.to_string())?;
                    for leftover in &leftovers {
                        let _ = fs::remove_file(leftover);
                    }
                    for suffix in ["-wal", "-shm", "-journal"] {
                        absorbed.insert(format!("{path}{suffix}"));
                    }
                    Some(moved)
                } else {
                    None
                }
            } else if let Some(members) = zip_members(&bytes) {
                // Before the text branch: an archive of plain text members
                // can be valid UTF-8, and substituting in its bytes would
                // break its sizes and checksums. The comparison maps text
                // members back; a rebuilt archive would not keep the
                // product's bytes, so refuse instead.
                if let Some((name, _)) = members.iter().find(|(_, body)| reverse.named_in(body)) {
                    return Err(format!(
                        "{path}: archive member {name} names a replay path; \
                         re-recording cannot map it back"
                    ));
                }
                None
            } else if let Ok(text) = std::str::from_utf8(&bytes) {
                let moved = paths.back(text);
                (moved != text).then(|| moved.into_bytes())
            } else {
                None
            };
            if let Some(moved) = rewritten {
                *blob = store.put(&moved).map_err(|e| e.to_string())?;
                *size = moved.len() as u64;
                // The kept ciphertext held the replay's paths; without it a
                // restore encrypts the mapped plaintext again.
                if let Some(encryption) = encryption {
                    encryption.ciphertext = None;
                }
            }
        }
        let replayed_name = entry_path(&entry).to_owned();
        match &mut entry {
            Entry::Dir { path, .. } | Entry::File { path, .. } | Entry::Symlink { path, .. } => {
                *path = paths.back(path);
            }
        }
        out.push((replayed_name, entry));
    }
    Ok(out
        .into_iter()
        .filter(|(name, _)| !absorbed.contains(name))
        .map(|(_, entry)| entry)
        .collect())
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

    /// Whether `bytes` hold any recorded path.
    pub fn named_in(&self, bytes: &[u8]) -> bool {
        self.0.iter().any(|(recorded, _)| {
            let needle = recorded.as_bytes();
            bytes.windows(needle.len()).any(|w| w == needle)
        })
    }

    /// Replayed text to recorded text: the mapping the comparison applies
    /// to everything a replay printed or left.
    pub fn back(&self, text: &str) -> String {
        self.reversed().forward(text)
    }

    /// The inverse substitutions (replayed text to recorded), longest first.
    pub fn reversed(&self) -> Paths {
        let mut back: Vec<_> = self.0.iter().map(|(a, b)| (b.clone(), a.clone())).collect();
        back.sort_by_key(|p| std::cmp::Reverse(p.0.len()));
        Paths(back)
    }
}

/// The wall-clock spans the replayed process saw through the clock shim:
/// its realtime base (the shared SQLite clock's, or the recorded start) plus
/// the replay's own elapsed time. A slow machine stretches them past the
/// recording's end, so they count as generated in their own right.
fn virtual_spans(session: &Session, elapsed_us: i64) -> Vec<(i64, i64)> {
    let mut bases = vec![session.started_us];
    if let Some(base) = session
        .env
        .get("KASSIBER_TEST_CLOCK_BASE_US")
        .and_then(|v| v.parse::<i64>().ok())
    {
        bases.push(base);
    }
    bases
        .into_iter()
        .map(|base| (base, base + elapsed_us.max(0)))
        .collect()
}

/// The clock spans a replay that ran from `started` to `ended` (µs since
/// the epoch) compares under: its own run, the recording's (the recorded
/// state was written during the recording test, which began before the
/// process, so its readings count as generated on both sides), the virtual
/// spans the clock shim showed the process, and every re-recording's.
fn comparison_window(session: &Session, started: i64, ended: i64) -> Window {
    Window {
        start: started,
        end: ended,
        local_offset_us: local_offset_us(),
        also: std::iter::once((session.started_us - RECORDING_LOOKBACK_US, session.ended_us))
            .chain(virtual_spans(session, ended - started))
            .chain(session.rerecorded_us.iter().copied())
            .collect(),
    }
}

/// The account this replay runs as, from the password database.
fn current_user() -> Option<String> {
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    passwd
        .lines()
        .find_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            (fields.len() > 2 && fields[2].parse::<u32>().ok() == Some(uid))
                .then(|| fields[0].to_owned())
        })
        .or_else(|| std::env::var("USER").ok())
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
                } else if let Some(members) = zip_members(&bytes) {
                    // An XLSX or ZIP export, even one whose bytes are valid
                    // UTF-8 (stored text members): each member compares as
                    // text (with the usual rules, so its creation time is a
                    // clock reading) or, if binary, by its bytes.
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
                } else if let Ok(text) = std::str::from_utf8(&bytes) {
                    canon.rewrite(&back(paths, text), true)
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
    // `~user` names the recording account (a test that passes a path
    // through the user's home): the replay's account stands in.
    if let (Some(recorded), Some(current)) = (
        session
            .env
            .get("HOME")
            .and_then(|home| Path::new(home).file_name())
            .map(|name| name.to_string_lossy().into_owned()),
        current_user(),
    ) {
        if recorded != current {
            paths.add(&format!("~{recorded}/"), &format!("~{current}/"));
        }
    }
    // Restored state still names the recording checkout (a book's custom CA
    // bundle in the repository's fixtures); point it at this one. Namespaced
    // roots keep their own /mnt paths, so only the repository moves there.
    for (index, root) in session.roots.iter().enumerate() {
        relocate(&disk_path(scratch, index, &root.path, namespaced), &paths)?;
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
    // A recording whose loopback server held one request in flight gets the
    // tape's hold on that request, released below once its control answered.
    let hold = match held_request(session) {
        Some(pattern) => {
            let dir = scratch.join("hold");
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            fs::write(dir.join("arm"), pattern).map_err(|e| e.to_string())?;
            command.env("KASSIBER_TEST_NET_TAPE_HOLD", &dir);
            Some(dir)
        }
        None => None,
    };
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
    let handed_sources = handed.clone();
    crate::hand_over(&mut command, handed);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let started = now_us();
    let spawned = command.spawn();
    // The child has its copies; the parent's pipes would only leak.
    crate::close_sources(&handed_sources);
    let mut child = spawned.map_err(|e| format!("start {}: {e}", binary.display()))?;
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
    let mut sent: Vec<Option<String>> = vec![None; session.stdin.len()];
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
        let control = hold.as_ref().filter(|_| index == HELD_CONTROL_INPUT);
        if let Some(dir) = control {
            // The test sent its control while the request was held.
            while !dir.join("held").exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
            if !dir.join("held").exists() {
                differences.push("the held request never came".into());
                break;
            }
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
                sent[index] = Some(paths.back(&text));
            }
            None => drop(stdin.take()),
        }
        if let Some(dir) = control {
            // ...and let the request finish once the control's answer printed.
            if !wait_for(after + 1) {
                differences.push("timed out waiting for the held control's answer".into());
                break;
            }
            fs::write(dir.join("release"), b"").map_err(|e| e.to_string())?;
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
    let replay_window = comparison_window(session, started, ended);
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
            stdin: sent,
            spans: std::iter::once((started, ended))
                .chain(virtual_spans(session, ended - started))
                .collect(),
            paths,
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
        // Rewriting a book in WAL mode checkpoints it, which removes its
        // -wal and -shm files before the walk reaches them.
        let bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        if dbdump::is_sqlite(&bytes) {
            // Opening a book checkpoints its WAL when it closes, which would
            // erase a pending journal a session records on purpose: only a
            // book (or its WAL) that names a recorded path is opened.
            let mut wal = file.clone().into_os_string();
            wal.push("-wal");
            let wal = fs::read(&wal).unwrap_or_default();
            if paths.named_in(&bytes) || paths.named_in(&wal) {
                relocate_database(&file, paths)?;
            }
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

#[cfg(test)]
mod relocation_tests {
    use super::Paths;

    #[test]
    fn only_bytes_naming_a_recorded_path_are_relocated() {
        let mut paths = Paths::default();
        paths.add("/home/rec/checkout", "/home/runner/work/checkout");
        assert!(paths.named_in(b"ca=/home/rec/checkout/crates/net/tests/fixtures/ca.pem"));
        assert!(!paths.named_in(b"SQLite format 3\0 no paths here"));
        // An identical checkout path adds no mapping, so nothing is touched.
        let mut same = Paths::default();
        same.add("/srv/checkout", "/srv/checkout");
        assert!(!same.named_in(b"/srv/checkout/x"));
    }
}

#[cfg(test)]
mod rerecord_tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;

    use base64::Engine;

    use super::{artifacts, comparison_window, rerecorded, Paths, Replayed};
    use crate::compare::Canon;
    use crate::{dbdump, entry_path, Blobs, Entry, Session};

    const RECORDED_REPO: &str = "/home/rec/.t3/worktrees/kassiber/t3-recorded";
    const REPLAY_REPO: &str = "/home/runner/verify/r8-gate";
    const RECORDED_TMP: &str = "/home/rec/tmp/pytest-7";
    const REPLAY_TMP: &str = "/home/runner/verify/r8-gate/target/session-replay-scratch/1-w0-0/tmp";
    /// The recording's process: 2026-11-11T02:54:46Z for 21.6 s.
    const STARTED_US: i64 = 1_794_365_686_167_661;
    const ENDED_US: i64 = 1_794_365_707_798_928;
    const OLD_TOKEN: &str = "1d0a3a9e-36f4-4c3e-9a51-2b8f0f7d1c11";
    const NEW_TOKEN: &str = "6f0e2b7c-8a1d-4f5e-b3c2-9d4a7e1f0b22";

    fn b64(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    fn file(path: &str, blobs: &Blobs, bytes: &[u8]) -> Entry {
        Entry::File {
            path: path.to_owned(),
            mode: 0o644,
            blob: blobs.put(bytes).unwrap(),
            size: bytes.len() as u64,
            encryption: None,
        }
    }

    /// A book in WAL mode whose only row (naming `source`) still sits in
    /// its write-ahead log, as a daemon leaves it: the book and log bytes.
    fn wal_book(dir: &Path, source: &str) -> (Vec<u8>, Vec<u8>) {
        let path = dir.join("book.sqlite3");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;
             CREATE TABLE wallets (id TEXT, config_json TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO wallets VALUES ('a', ?1)",
            [format!(
                "{{\"source_file\": \"{source}/relative/bull.csv\"}}"
            )],
        )
        .unwrap();
        let book = fs::read(&path).unwrap();
        let wal = fs::read(dir.join("book.sqlite3-wal")).unwrap();
        drop(conn);
        (book, wal)
    }

    fn session(repo: &str, root: &str) -> Session {
        serde_json::from_value(serde_json::json!({
            "format": crate::SESSION_FORMAT,
            "binary": "kassiber",
            "argv": ["--machine", "daemon"],
            "cwd": repo,
            "env": {"HOME": "/home/rec", "TMPDIR": RECORDED_TMP},
            "test": null,
            "started_us": STARTED_US,
            "ended_us": ENDED_US,
            "stdin": [
                {"after_lines": 1, "data": b64(&format!(
                    "{{\"kind\": \"preview\", \"args\": {{\"document_token\": \"{OLD_TOKEN}\", \
                     \"source_file\": \"{repo}/receipt.png\"}}}}\n"))},
                {"after_lines": 2, "data": null},
            ],
            "stdout": "",
            "stderr": "",
            "signals": [],
            "exit": {"code": 0, "signal": null},
            "roots": [{"path": root, "file": false, "before": [], "after": []}],
            "repo": repo,
        }))
        .unwrap()
    }

    /// A replay of a session recorded in one checkout, run from another (a
    /// verification checkout) and re-recorded because it differed.
    struct Rerecord {
        dir: tempfile::TempDir,
        recording: Blobs,
        scan: Blobs,
        paths: Paths,
        replayed: Replayed,
        session: Session,
        sent: String,
        virtual_end: i64,
    }

    fn rerecord() -> Rerecord {
        let dir = tempfile::tempdir().unwrap();
        let recording = Blobs::new(&dir.path().join("recording"));
        let store = dir.path().join("after-store");
        let scan = Blobs::new(&store);
        let mut paths = Paths::default();
        paths.add(RECORDED_REPO, REPLAY_REPO);
        paths.add(RECORDED_TMP, REPLAY_TMP);
        paths.add("~rec/", "~runner/");

        // What the replay left, as on disk: in the replaying paths.
        let traceback = format!(
            "Traceback (most recent call last):\n  File \"{REPLAY_REPO}/kassiber/daemon.py\", \
             line 21223, in run\nupload {REPLAY_TMP}/kassiber-chat-uploads-1/paste.csv\n\
             notes ~runner/notes.txt\n"
        );
        let books = dir.path().join("books");
        fs::create_dir_all(&books).unwrap();
        let (book, wal) = wal_book(&books, REPLAY_REPO);
        let after = vec![
            Entry::Dir {
                path: "data".into(),
                mode: 0o755,
            },
            file("data/kassiber.sqlite3", &scan, &book),
            file("data/kassiber.sqlite3-wal", &scan, &wal),
            file("python-stderr.txt", &scan, traceback.as_bytes()),
            file("receipt.png", &scan, b"\x89PNG\r\n\x1a\n binary"),
        ];
        let sent = format!(
            "{{\"kind\": \"preview\", \"args\": {{\"document_token\": \"{NEW_TOKEN}\", \
             \"source_file\": \"{RECORDED_REPO}/receipt.png\"}}}}\n"
        );
        // The re-recording run took 43.3 s on the clock shim's virtual
        // clock, from the recorded start.
        let virtual_end = STARTED_US + 43_310_000;
        let replayed = Replayed {
            stdout: format!("{{\"document_token\": \"{NEW_TOKEN}\"}}\n").into_bytes(),
            stderr: Vec::new(),
            exit: crate::Exit {
                code: Some(0),
                signal: None,
            },
            after: vec![after],
            store,
            stdin: vec![Some(sent.clone()), None],
            spans: vec![
                (1_794_635_190_043_740, 1_794_635_233_351_737),
                (STARTED_US, virtual_end),
            ],
            paths: paths.clone(),
        };
        let root = "/mnt/bt/popen-gw4/test_bull_wallet_create_daemon0";
        let work = dir.path().join("work");
        let session = rerecorded(session(RECORDED_REPO, root), &recording, &replayed, &work)
            .expect("re-record");
        Rerecord {
            dir,
            recording,
            scan,
            paths,
            replayed,
            session,
            sent,
            virtual_end,
        }
    }

    /// The state a re-record stores is what the recording checkout would
    /// have left: no replaying checkout, temp dir or account in any file,
    /// book or name, so a later replay from any checkout compares clean.
    #[test]
    fn a_rerecord_from_another_checkout_stores_state_in_the_recorded_paths() {
        let run = rerecord();
        let stored = &run.session.roots[0].after;
        let book_text = |bytes: &[u8]| {
            let copy = run.dir.path().join("dump.sqlite3");
            fs::write(&copy, bytes).unwrap();
            if let Some(Entry::File { blob, .. }) = stored
                .iter()
                .find(|e| entry_path(e) == "data/kassiber.sqlite3-wal")
            {
                fs::write(
                    run.dir.path().join("dump.sqlite3-wal"),
                    run.recording.get(blob).unwrap(),
                )
                .unwrap();
            }
            let rows = dbdump::dump(&copy).unwrap();
            let _ = fs::remove_file(&copy);
            format!("{rows:?}")
        };
        for entry in stored {
            let Entry::File { path, blob, .. } = entry else {
                continue;
            };
            if path.ends_with("-wal") {
                continue;
            }
            let bytes = run.recording.get(blob).unwrap();
            let text = if dbdump::is_sqlite(&bytes) {
                book_text(&bytes)
            } else {
                String::from_utf8_lossy(&bytes).into_owned()
            };
            for replay_path in [REPLAY_REPO, "~runner/"] {
                assert!(
                    !text.contains(replay_path),
                    "{path} kept {replay_path}: {text}"
                );
            }
            if dbdump::is_sqlite(&bytes) {
                assert!(
                    text.contains(&format!("{RECORDED_REPO}/relative/bull.csv")),
                    "{text}"
                );
            }
        }
        let stored_text = |name: &str| {
            stored
                .iter()
                .find_map(|e| match e {
                    Entry::File { path, blob, .. } if path == name => {
                        Some(String::from_utf8(run.recording.get(blob).unwrap()).unwrap())
                    }
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(
            stored_text("python-stderr.txt"),
            format!(
                "Traceback (most recent call last):\n  File \"{RECORDED_REPO}/kassiber/daemon.py\", \
                 line 21223, in run\nupload {RECORDED_TMP}/kassiber-chat-uploads-1/paste.csv\n\
                 notes ~rec/notes.txt\n"
            )
        );
        // The book's log is folded into the mapped book, not kept beside it
        // holding the replay's paths.
        let names: Vec<&str> = stored.iter().map(entry_path).collect();
        assert!(!names.contains(&"data/kassiber.sqlite3-wal"), "{names:?}");

        // ...and the comparison finds it equal to what the replay left.
        let pinned = HashSet::new();
        let window = comparison_window(&run.session, run.virtual_end, run.virtual_end + 1);
        let dumps = run.dir.path().join("dumps");
        fs::create_dir_all(&dumps).unwrap();
        let recorded_state = artifacts(
            &dumps,
            stored,
            &run.recording,
            &mut Canon::new(window.clone(), &pinned),
            None,
        );
        let replayed_state = artifacts(
            &dumps,
            &run.replayed.after[0],
            &run.scan,
            &mut Canon::new(window, &pinned),
            Some(&run.paths),
        );
        assert_eq!(recorded_state, replayed_state);
    }

    /// An id the replay printed and the test sent back (a document token)
    /// is stored as this run sent it: the stored output holds this run's
    /// token, and a later replay maps recorded output ids onto its own, so
    /// a request still carrying the recording's token would go out stale.
    #[test]
    fn a_rerecord_stores_the_inputs_the_replay_sent() {
        let run = rerecord();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(run.session.stdin[0].data.as_deref().unwrap())
            .unwrap();
        let decoded = String::from_utf8(decoded).unwrap();
        assert!(decoded.contains(NEW_TOKEN), "{decoded}");
        assert_eq!(decoded, run.sent);
        assert_eq!(run.session.stdin[1].data, None, "EOF stays EOF");
    }

    /// A reading the re-recording's process took on the clock shim's
    /// virtual clock after the recording's own end is generated for a
    /// later, faster replay.
    #[test]
    fn a_rerecord_keeps_the_virtual_clock_span_it_ran_in() {
        let run = rerecord();
        let fast = comparison_window(&run.session, STARTED_US, STARTED_US + 5_000_000);
        let pinned = HashSet::new();
        let mut canon = Canon::new(fast, &pinned);
        // 2026-11-11T02:55:28Z: 20 s after the recording ended, 1 s before
        // the re-recording did on the virtual clock.
        let late = "{\"finished_at\":\"2026-11-11T02:55:28Z\"}";
        assert_eq!(canon.rewrite(late, true), "{\"finished_at\":\"<time>\"}");
    }

    /// A re-recorded book keeps its ciphertext only where the recording
    /// kept one (`pack --keep-ciphertext`); nothing compares it.
    #[test]
    fn a_rerecord_keeps_ciphertext_only_where_the_recording_did() {
        let dir = tempfile::tempdir().unwrap();
        let recording = Blobs::new(&dir.path().join("recording"));
        let store = dir.path().join("after-store");
        let scan = Blobs::new(&store);
        let book = |blobs: &Blobs, path: &str, ciphertext: Option<&[u8]>| Entry::File {
            path: path.to_owned(),
            mode: 0o644,
            blob: blobs.put(b"plaintext pages").unwrap(),
            size: 15,
            encryption: Some(crate::encrypted::Encryption {
                passphrase: "synthetic".into(),
                ciphertext: ciphertext.map(|c| blobs.put(c).unwrap()),
                page_size: 4096,
                page_count: 1,
            }),
        };
        let mut recorded = session(RECORDED_REPO, "/mnt/tmp/r");
        recorded.roots[0].after = vec![
            book(&recording, "kept.sqlite3", Some(b"recorded ciphertext")),
            book(&recording, "plain.sqlite3", None),
        ];
        let replayed = Replayed {
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit: crate::Exit {
                code: Some(0),
                signal: None,
            },
            after: vec![vec![
                book(&scan, "kept.sqlite3", Some(b"replayed ciphertext")),
                book(&scan, "plain.sqlite3", Some(b"replayed ciphertext 2")),
            ]],
            store,
            stdin: Vec::new(),
            spans: Vec::new(),
            paths: Paths::default(),
        };
        let session =
            rerecorded(recorded, &recording, &replayed, &dir.path().join("work")).unwrap();
        let ciphertext = |name: &str| {
            session.roots[0].after.iter().find_map(|e| match e {
                Entry::File {
                    path,
                    encryption: Some(e),
                    ..
                } if path == name => Some(e.ciphertext.clone()),
                _ => None,
            })
        };
        let kept = ciphertext("kept.sqlite3").unwrap().expect("kept");
        assert_eq!(recording.get(&kept).unwrap(), b"replayed ciphertext");
        assert_eq!(ciphertext("plain.sqlite3"), Some(None));
    }

    /// An archive with `body` as its one member `sheet.xml`, stored
    /// (uncompressed) or deflated.
    fn archive(body: &str, stored: bool) -> Vec<u8> {
        let options = zip::write::SimpleFileOptions::default();
        let options = if stored {
            // Mode 0o720 stores its attributes as `D0 81`, a valid UTF-8
            // pair (0o644's `A4 81` is not).
            options
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o720)
        } else {
            options.compression_method(zip::CompressionMethod::Deflated)
        };
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        archive.start_file("sheet.xml", options).unwrap();
        std::io::Write::write_all(&mut archive, body.as_bytes()).unwrap();
        archive.finish().unwrap().into_inner()
    }

    /// A ZIP export the replay left naming its paths cannot be mapped back
    /// without rebuilding the product's archive: re-recording refuses. That
    /// holds for an archive whose bytes happen to be valid UTF-8 (stored
    /// text members): substituting in it as text would leave its sizes and
    /// checksums wrong, a corrupt archive in the recording.
    #[test]
    fn a_rerecord_refuses_an_archive_naming_replay_paths() {
        let deflated = archive(&format!("{REPLAY_REPO}/a.csv"), false);
        // A stored archive whose checksum and other header bytes are valid
        // UTF-8 as well.
        let utf8 = (0..10_000)
            .map(|n| archive(&format!("{REPLAY_REPO}/a.csv {n}"), true))
            .find(|bytes| std::str::from_utf8(bytes).is_ok())
            .expect("a stored archive that is valid UTF-8");
        for bytes in [deflated, utf8] {
            let dir = tempfile::tempdir().unwrap();
            let store = dir.path().join("after-store");
            let scan = Blobs::new(&store);
            let mut paths = Paths::default();
            paths.add(RECORDED_REPO, REPLAY_REPO);
            let after = vec![file("exports/report.xlsx", &scan, &bytes)];
            // The comparison reads it member by member, mapped back.
            let pinned = HashSet::new();
            let window = comparison_window(&session(RECORDED_REPO, "/mnt/r"), 0, 1);
            let dumps = dir.path().join("dumps");
            fs::create_dir_all(&dumps).unwrap();
            let compared = artifacts(
                &dumps,
                &after,
                &scan,
                &mut Canon::new(window, &pinned),
                Some(&paths),
            );
            let text = compared.values().flatten().cloned().collect::<String>();
            assert!(text.contains("member sheet.xml"), "{text}");
            assert!(text.contains(&format!("{RECORDED_REPO}/a.csv")), "{text}");
            let replayed = Replayed {
                stdout: Vec::new(),
                stderr: Vec::new(),
                exit: crate::Exit {
                    code: Some(0),
                    signal: None,
                },
                after: vec![after],
                store,
                stdin: Vec::new(),
                spans: Vec::new(),
                paths,
            };
            let error = replayed
                .recorded_after(&dir.path().join("work"))
                .unwrap_err();
            assert!(error.contains("exports/report.xlsx"), "{error}");
        }
    }
}

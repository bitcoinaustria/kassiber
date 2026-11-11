//! Replay recorded process sessions against the current binaries.
//!
//!     session-replay replay DIR --bin NAME=PATH ... [--examples DIR] [--only TEXT] [--jobs N]
//!                           [--keep] [--report FILE [--resume]] [--coverage DIR]
//!                           [--update]  (re-record the sessions that differ)
//!     session-replay list DIR
//!     session-replay pack SRC DST --sessions FILE [--keep-ciphertext FILE]
//!     session-replay decrypt REC --allowlist FILE [--also PASSPHRASE...]
//!     session-replay archive PACK OUT     (deterministic tars for test data)
//!     session-replay extract OUT DIR
//!
//! DIR holds `sessions/*.json` and `blobs/` as `session-shim` wrote them. A
//! session replays from its recorded starting state at a fresh path under
//! `target/session-replay-scratch`, and must print the same output (with
//! generated values mapped consistently, see `compare`) and leave the same
//! state. `--only` selects sessions whose test id or file name contains TEXT.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use kassiber_session_replay::replay::replay;
use kassiber_session_replay::{Blobs, Session};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

/// A session file, plain or gzipped (`pack` writes `.json.gz`).
fn read_session(path: &Path) -> Result<Session, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let text = if path.to_string_lossy().ends_with(".gz") {
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&bytes[..]), &mut out)
            .map_err(|e| e.to_string())?;
        out
    } else {
        bytes
    };
    serde_json::from_slice(&text).map_err(|e| e.to_string())
}

fn sessions(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(dir.join("sessions"))
        .map(|read| read.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    found.retain(|p| {
        let name = p.to_string_lossy();
        name.ends_with(".json") || name.ends_with(".json.gz")
    });
    found.sort();
    found
}

/// Merge a session's raw counters into one sparse profile.
fn merge_profile(raw: &Path, out: &Path) {
    let files: Vec<PathBuf> = fs::read_dir(raw)
        .map(|read| read.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    if files.is_empty() {
        return;
    }
    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tool = std::env::var("LLVM_PROFDATA").unwrap_or_else(|_| "llvm-profdata".into());
    let _ = std::process::Command::new(tool)
        .args(["merge", "-sparse", "-o"])
        .arg(out)
        .args(&files)
        .status();
}

/// `pack SRC DST --sessions FILE [--keep-ciphertext FILE]`: copy the
/// listed sessions and only the blob files they use into DST. A kept
/// encrypted book's original ciphertext stays only for sessions on the
/// keep list; the others replay from the re-encrypted plaintext.
fn pack(args: &[String]) -> ExitCode {
    let (Some(src), Some(dst)) = (args.first(), args.get(1)) else {
        eprintln!("usage: session-replay pack SRC DST --sessions FILE [--keep-ciphertext FILE]");
        return ExitCode::from(2);
    };
    let list = |flag: &str| -> Vec<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .and_then(|f| fs::read_to_string(f).ok())
            .map(|t| {
                t.lines()
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let (sessions, keep) = (list("--sessions"), list("--keep-ciphertext"));
    let (src, dst) = (PathBuf::from(src), PathBuf::from(dst));
    let source = Blobs::new(&src);
    if fs::create_dir_all(dst.join("sessions")).is_err() {
        eprintln!("cannot create {}", dst.display());
        return ExitCode::from(2);
    }
    let mut copied = std::collections::BTreeSet::new();
    let (mut bytes, mut files) = (0u64, 0usize);
    for name in &sessions {
        let Ok(text) = fs::read_to_string(src.join("sessions").join(name)) else {
            eprintln!("no session {name}");
            return ExitCode::from(2);
        };
        let Ok(mut session) = serde_json::from_str::<Session>(&text) else {
            eprintln!("unreadable session {name}");
            return ExitCode::from(2);
        };
        let keep_ciphertext = keep.contains(name);
        let mut hashes = vec![session.stdout.clone(), session.stderr.clone()];
        hashes.extend(session.net_tape.clone());
        for root in &mut session.roots {
            for entry in root.before.iter_mut().chain(root.after.iter_mut()) {
                if let kassiber_session_replay::Entry::File {
                    blob, encryption, ..
                } = entry
                {
                    hashes.push(blob.clone());
                    if let Some(e) = encryption {
                        if !keep_ciphertext {
                            e.ciphertext = None;
                        }
                        hashes.extend(e.ciphertext.clone());
                    }
                }
            }
        }
        for hash in hashes {
            for file in source.files(&hash).unwrap_or_default() {
                let Ok(relative) = file.strip_prefix(&src) else {
                    continue;
                };
                if copied.insert(relative.to_path_buf()) {
                    let target = dst.join(relative);
                    let _ = fs::create_dir_all(target.parent().unwrap());
                    if fs::copy(&file, &target).is_ok() {
                        bytes += fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                        files += 1;
                    }
                }
            }
        }
        let out = serde_json::to_vec(&session).unwrap_or_default();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(9));
        let _ = std::io::Write::write_all(&mut encoder, &out);
        let packed = encoder.finish().unwrap_or_default();
        bytes += packed.len() as u64;
        let _ = fs::write(dst.join("sessions").join(format!("{name}.gz")), packed);
    }
    println!(
        "{} sessions, {files} blob files, {:.1} MB",
        sessions.len(),
        bytes as f64 / 1e6
    );
    ExitCode::SUCCESS
}

/// `decrypt REC --allowlist FILE`: decrypt, in place, the encrypted books a
/// session could not open itself (it never received their passphrase, such
/// as a refused read of a locked book), with the synthetic passphrases other
/// sessions of the recording used. Every passphrase tried must be on the
/// allowlist. Each distinct ciphertext is tried once.
fn decrypt(args: &[String]) -> ExitCode {
    use kassiber_session_replay::encrypted::{maybe_encrypted, Allowlist, Encryption};
    use kassiber_session_replay::Entry;
    use std::collections::{BTreeSet, HashMap};
    let Some(rec) = args.first().map(PathBuf::from) else {
        eprintln!("usage: session-replay decrypt REC --allowlist FILE [--jobs N] [--also P...]");
        return ExitCode::from(2);
    };
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let Some(allowlist) = flag("--allowlist").map(|f| Allowlist::load(Path::new(f))) else {
        eprintln!("--allowlist FILE is required");
        return ExitCode::from(2);
    };
    let jobs: usize = flag("--jobs").and_then(|j| j.parse().ok()).unwrap_or(8);
    let blobs = Blobs::new(&rec);
    let files = sessions(&rec);
    // The passphrases the recording already opened books with, overall and
    // per test module (tried first: a module keys its books one way).
    let module = |session: &Session| {
        session
            .test
            .as_deref()
            .unwrap_or("")
            .split("::")
            .next()
            .unwrap_or("")
            .to_owned()
    };
    let mut global: Vec<String> = Vec::new();
    let mut by_module: HashMap<String, Vec<String>> = HashMap::new();
    let mut wanted: BTreeMap<String, String> = BTreeMap::new();
    for file in &files {
        let Ok(session) = read_session(file) else {
            eprintln!("unreadable session {}", file.display());
            return ExitCode::from(2);
        };
        let name = module(&session);
        for root in &session.roots {
            for entry in root.before.iter().chain(&root.after) {
                if let Entry::File {
                    blob,
                    encryption,
                    size,
                    ..
                } = entry
                {
                    match encryption {
                        Some(e) => {
                            for list in [&mut global, by_module.entry(name.clone()).or_default()] {
                                if !list.contains(&e.passphrase) {
                                    list.push(e.passphrase.clone());
                                }
                            }
                        }
                        None if *size >= 4096 && *size % 1024 == 0 => {
                            wanted.entry(blob.clone()).or_insert_with(|| name.clone());
                        }
                        None => {}
                    }
                }
            }
        }
    }
    for extra in args.iter().skip_while(|a| *a != "--also").skip(1) {
        if !global.contains(extra) {
            global.push(extra.clone());
        }
    }
    global.retain(|p| allowlist.allows(p));
    let scratch = repo().join("target").join("session-replay-decrypt");
    let work: Vec<(String, Vec<String>)> = wanted
        .into_iter()
        .map(|(blob, name)| {
            let mut order: Vec<String> = by_module
                .get(&name)
                .into_iter()
                .flatten()
                .filter(|p| allowlist.allows(p))
                .cloned()
                .collect();
            let rest: Vec<String> = global
                .iter()
                .filter(|p| !order.contains(p))
                .cloned()
                .collect();
            order.extend(rest);
            (blob, order)
        })
        .collect();
    let next = Arc::new(AtomicUsize::new(0));
    let found = Arc::new(Mutex::new(BTreeMap::<String, (String, Encryption)>::new()));
    let work = Arc::new(work);
    let threads: Vec<_> = (0..jobs.max(1))
        .map(|worker| {
            let (next, found, work, blobs) = (
                Arc::clone(&next),
                Arc::clone(&found),
                Arc::clone(&work),
                blobs.clone(),
            );
            let scratch = scratch.join(format!("w{worker}"));
            std::thread::spawn(move || {
                while let Some((blob, order)) = work.get(next.fetch_add(1, Ordering::SeqCst)) {
                    let Ok(bytes) = blobs.get(blob) else { continue };
                    if !maybe_encrypted(&bytes) {
                        continue;
                    }
                    if let Some((plaintext, mut encryption)) =
                        kassiber_session_replay::encrypted::decrypt(&bytes, order, &scratch)
                    {
                        encryption.ciphertext = Some(blob.clone());
                        if let Ok(plain) = blobs.put(&plaintext) {
                            found
                                .lock()
                                .unwrap()
                                .insert(blob.clone(), (plain, encryption));
                        }
                    }
                }
            })
        })
        .collect();
    for thread in threads {
        let _ = thread.join();
    }
    let found = found.lock().unwrap().clone();
    let mut rewritten = 0usize;
    let mut touched = BTreeSet::new();
    for file in &files {
        let Ok(mut session) = read_session(file) else {
            continue;
        };
        let mut changed = false;
        for root in &mut session.roots {
            for entry in root.before.iter_mut().chain(root.after.iter_mut()) {
                if let Entry::File {
                    blob, encryption, ..
                } = entry
                {
                    if encryption.is_none() {
                        if let Some((plain, e)) = found.get(blob.as_str()) {
                            touched.insert(blob.clone());
                            *blob = plain.clone();
                            *encryption = Some(e.clone());
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            if let Err(error) = write_session(file, &session) {
                eprintln!("cannot rewrite {}: {error}", file.display());
                return ExitCode::from(2);
            }
            rewritten += 1;
        }
    }
    let _ = fs::remove_dir_all(&scratch);
    println!(
        "{} passphrases tried; {} books opened; {rewritten} sessions rewritten",
        global.len(),
        touched.len()
    );
    ExitCode::SUCCESS
}

/// Replace a session file atomically, keeping its compression.
fn write_session(file: &Path, session: &Session) -> Result<(), String> {
    let bytes = serde_json::to_vec(session).map_err(|e| e.to_string())?;
    let bytes = if file.to_string_lossy().ends_with(".gz") {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(9));
        std::io::Write::write_all(&mut encoder, &bytes).map_err(|e| e.to_string())?;
        encoder.finish().map_err(|e| e.to_string())?
    } else {
        bytes
    };
    let tmp = file.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    fs::rename(&tmp, file).map_err(|e| e.to_string())
}

/// Write what a replay did into its session: output, exit and the state it
/// left (blobs copied into the recording's store).
fn rerecord(
    file: &Path,
    blobs: &Blobs,
    replayed: &kassiber_session_replay::replay::Replayed,
) -> Result<(), String> {
    use kassiber_session_replay::Entry;
    let mut session = read_session(file)?;
    let scan = Blobs::new(&replayed.store);
    let copy = |hash: &str| -> Result<String, String> {
        let bytes = scan.get(hash).map_err(|e| e.to_string())?;
        blobs.put(&bytes).map_err(|e| e.to_string())
    };
    session.stdout = blobs.put(&replayed.stdout).map_err(|e| e.to_string())?;
    session.stderr = blobs.put(&replayed.stderr).map_err(|e| e.to_string())?;
    session.exit = replayed.exit.clone();
    session
        .rerecorded_us
        .push((replayed.started_us, replayed.ended_us));
    for (root, after) in session.roots.iter_mut().zip(&replayed.after) {
        let mut after = after.clone();
        for entry in &mut after {
            if let Entry::File {
                blob, encryption, ..
            } = entry
            {
                *blob = copy(blob)?;
                if let Some(e) = encryption {
                    if let Some(ciphertext) = &e.ciphertext {
                        e.ciphertext = Some(copy(ciphertext)?);
                    }
                }
            }
        }
        root.after = after;
    }
    let bytes = serde_json::to_vec(&session).map_err(|e| e.to_string())?;
    if file.to_string_lossy().ends_with(".gz") {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(9));
        std::io::Write::write_all(&mut encoder, &bytes).map_err(|e| e.to_string())?;
        fs::write(file, encoder.finish().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    } else {
        fs::write(file, bytes).map_err(|e| e.to_string())
    }
}

/// Archives a recording travels in on the test-data branch: one gzip
/// stream of every session (`sessions.tar.gz`, sorted by test so similar
/// sessions sit together) and a few gzip streams of their blobs
/// (`blobs-NN.tar.gz`, uncompressed parts in first-use order), so the
/// compression sees across sessions and pages. Times, owners and gzip
/// headers are fixed: an unchanged recording archives to the same bytes.
const SHARD_RAW_BYTES: u64 = 256 << 20;

fn gzip_tar(path: &Path, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
    let file = fs::File::create(path).map_err(|e| e.to_string())?;
    let encoder = flate2::GzBuilder::new()
        .mtime(0)
        .write(file, flate2::Compression::best());
    let mut builder = tar::Builder::new(encoder);
    builder.mode(tar::HeaderMode::Deterministic);
    for (name, bytes) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder
            .append_data(&mut header, name, bytes.as_slice())
            .map_err(|e| e.to_string())?;
    }
    builder
        .into_inner()
        .and_then(|encoder| encoder.finish())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `archive PACK OUT`: a pack directory as the archives above.
fn archive(args: &[String]) -> ExitCode {
    use kassiber_session_replay::Entry;
    let (Some(src), Some(out)) = (args.first(), args.get(1)) else {
        eprintln!("usage: session-replay archive PACK OUT");
        return ExitCode::from(2);
    };
    let (src, out) = (PathBuf::from(src), PathBuf::from(out));
    let blobs = Blobs::new(&src);
    let mut loaded: Vec<(String, String, Session)> = Vec::new();
    for file in sessions(&src) {
        let Ok(session) = read_session(&file) else {
            eprintln!("unreadable session {}", file.display());
            return ExitCode::from(2);
        };
        let name = file
            .file_name()
            .unwrap()
            .to_string_lossy()
            .trim_end_matches(".gz")
            .to_owned();
        loaded.push((session.test.clone().unwrap_or_default(), name, session));
    }
    loaded.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    if fs::create_dir_all(&out).is_err() {
        eprintln!("cannot create {}", out.display());
        return ExitCode::from(2);
    }
    let mut session_entries = Vec::new();
    let mut order: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (_, name, session) in &loaded {
        let Ok(bytes) = serde_json::to_vec(session) else {
            return ExitCode::from(2);
        };
        session_entries.push((format!("sessions/{name}"), bytes));
        let mut hashes = vec![session.stdout.clone(), session.stderr.clone()];
        hashes.extend(session.net_tape.clone());
        for root in &session.roots {
            for entry in root.before.iter().chain(&root.after) {
                if let Entry::File {
                    blob, encryption, ..
                } = entry
                {
                    hashes.push(blob.clone());
                    if let Some(ciphertext) = encryption.as_ref().and_then(|e| e.ciphertext.clone())
                    {
                        hashes.push(ciphertext);
                    }
                }
            }
        }
        for hash in hashes {
            if seen.insert(hash.clone()) {
                order.push(hash);
            }
        }
    }
    if let Err(error) = gzip_tar(&out.join("sessions.tar.gz"), &session_entries) {
        eprintln!("cannot write sessions.tar.gz: {error}");
        return ExitCode::from(2);
    }
    drop(session_entries);
    let mut shard: Vec<(String, Vec<u8>)> = Vec::new();
    let (mut shard_bytes, mut shard_index) = (0u64, 0usize);
    let mut parts_seen = std::collections::HashSet::new();
    let flush = |shard: &mut Vec<(String, Vec<u8>)>, index: &mut usize| -> Result<(), String> {
        if shard.is_empty() {
            return Ok(());
        }
        gzip_tar(&out.join(format!("blobs-{:02}.tar.gz", *index)), shard)?;
        shard.clear();
        *index += 1;
        Ok(())
    };
    for hash in &order {
        let Ok(parts) = blobs.raw_parts(hash) else {
            eprintln!("missing blob {hash}");
            return ExitCode::from(2);
        };
        for (name, bytes) in parts {
            if !parts_seen.insert(name.clone()) {
                continue;
            }
            shard_bytes += bytes.len() as u64;
            shard.push((format!("blobs/{name}"), bytes));
        }
        if shard_bytes >= SHARD_RAW_BYTES {
            if let Err(error) = flush(&mut shard, &mut shard_index) {
                eprintln!("cannot write a blob archive: {error}");
                return ExitCode::from(2);
            }
            shard_bytes = 0;
        }
    }
    if let Err(error) = flush(&mut shard, &mut shard_index) {
        eprintln!("cannot write a blob archive: {error}");
        return ExitCode::from(2);
    }
    println!(
        "{} sessions, {} blob parts in {} archives under {}",
        loaded.len(),
        parts_seen.len(),
        shard_index + 1,
        out.display()
    );
    ExitCode::SUCCESS
}

/// `extract OUT DIR`: the archives `archive` wrote, as a recording directory.
fn extract(args: &[String]) -> ExitCode {
    let (Some(src), Some(dst)) = (args.first(), args.get(1)) else {
        eprintln!("usage: session-replay extract ARCHIVES DIR");
        return ExitCode::from(2);
    };
    let (src, dst) = (PathBuf::from(src), PathBuf::from(dst));
    let mut names: Vec<PathBuf> = fs::read_dir(&src)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".tar.gz"))
        .collect();
    names.sort();
    if names.is_empty() {
        eprintln!("no archives in {}", src.display());
        return ExitCode::from(2);
    }
    let blobs = Blobs::new(&dst);
    if fs::create_dir_all(dst.join("sessions")).is_err() {
        return ExitCode::from(2);
    }
    for name in names {
        let Ok(file) = fs::File::open(&name) else {
            return ExitCode::from(2);
        };
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
        let Ok(entries) = archive.entries() else {
            eprintln!("cannot read {}", name.display());
            return ExitCode::from(2);
        };
        for entry in entries {
            let Ok(mut entry) = entry else {
                eprintln!("cannot read {}", name.display());
                return ExitCode::from(2);
            };
            let Ok(path) = entry.path().map(|p| p.to_string_lossy().into_owned()) else {
                return ExitCode::from(2);
            };
            let mut bytes = Vec::new();
            if std::io::Read::read_to_end(&mut entry, &mut bytes).is_err() {
                return ExitCode::from(2);
            }
            // Only the two kinds of names `archive` writes; nothing else,
            // and no path that could leave DIR.
            let written = match path.split_once('/') {
                Some(("sessions", file))
                    if !file.contains('/') && !file.starts_with('.') && file.ends_with(".json") =>
                {
                    fs::write(dst.join("sessions").join(file), &bytes).map_err(|e| e.to_string())
                }
                Some(("blobs", part))
                    if !part.contains('/')
                        && part.trim_end_matches(".pages").len() == 64
                        && part
                            .trim_end_matches(".pages")
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit()) =>
                {
                    blobs.put_part(part, &bytes).map_err(|e| e.to_string())
                }
                _ => Err(format!("unexpected entry {path}")),
            };
            if let Err(error) = written {
                eprintln!("{}: {error}", name.display());
                return ExitCode::from(2);
            }
        }
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        eprintln!("usage: session-replay replay|list DIR ...");
        return ExitCode::from(2);
    };
    if command == "decrypt" {
        return decrypt(&args[1..]);
    }
    if command == "archive" {
        return archive(&args[1..]);
    }
    if command == "extract" {
        return extract(&args[1..]);
    }
    if command == "pack" {
        return pack(&args[1..]);
    }
    let Some(dir) = args.get(1).map(PathBuf::from) else {
        eprintln!("usage: session-replay {command} DIR ...");
        return ExitCode::from(2);
    };
    let mut binaries = BTreeMap::new();
    let mut only = Vec::new();
    let mut jobs = 4usize;
    let mut keep = false;
    let mut update = false;
    let mut resume = false;
    let mut report: Option<PathBuf> = None;
    let mut coverage: Option<PathBuf> = None;
    let mut rest = args[2..].iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--bin" => {
                let Some((name, path)) = rest.next().and_then(|v| v.split_once('=')) else {
                    eprintln!("--bin NAME=PATH");
                    return ExitCode::from(2);
                };
                binaries.insert(name.to_owned(), PathBuf::from(path));
            }
            "--only" => only.extend(rest.next().cloned()),
            "--jobs" => jobs = rest.next().and_then(|v| v.parse().ok()).unwrap_or(4),
            "--keep" => keep = true,
            // Re-record each session that differs from what this run did:
            // only after the difference is reviewed as intended.
            "--update" => update = true,
            // Skip the sessions the --report file already lists, and append.
            "--resume" => resume = true,
            "--report" => report = rest.next().map(PathBuf::from),
            "--coverage" => coverage = rest.next().map(PathBuf::from),
            // Every executable in DIR, by file name (cargo examples).
            "--examples" => {
                let Some(dir) = rest.next() else {
                    eprintln!("--examples DIR");
                    return ExitCode::from(2);
                };
                for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if path.is_file() && !name.contains('.') {
                        binaries.entry(name).or_insert(path);
                    }
                }
            }
            other => {
                eprintln!("unknown argument {other}");
                return ExitCode::from(2);
            }
        }
    }
    // Recordings are reviewed changes, never rewritten by a CI run (as
    // KASSIBER_BLESS and KASSIBER_GOLDEN_WRITE are refused there).
    if update && std::env::var_os("CI").is_some() {
        eprintln!("--update is refused under CI");
        return ExitCode::from(2);
    }
    let files: Vec<PathBuf> = sessions(&dir)
        .into_iter()
        .filter(|f| {
            only.is_empty() || {
                let name = f.to_string_lossy();
                let test = read_session(f)
                    .ok()
                    .and_then(|s| s.test)
                    .unwrap_or_default();
                only.iter()
                    .any(|o| name.contains(o.as_str()) || test.contains(o.as_str()))
            }
        })
        .collect();
    if command == "list" {
        for file in &files {
            let session: Session = read_session(file).unwrap();
            println!(
                "{}\t{}\t{}",
                file.file_name().unwrap().to_string_lossy(),
                session.binary,
                session.test.unwrap_or_default()
            );
        }
        return ExitCode::SUCCESS;
    }
    if command != "replay" {
        eprintln!("unknown command {command}");
        return ExitCode::from(2);
    }
    if files.is_empty() {
        eprintln!("no sessions selected under {}", dir.display());
        return ExitCode::from(2);
    }
    let repo = repo();
    let target = repo.join("target").canonicalize().expect("target/");
    let scratch_root = target.join("session-replay-scratch");
    let blobs = Blobs::new(&dir);
    let next = Arc::new(AtomicUsize::new(0));
    let failures = Arc::new(Mutex::new(Vec::<(String, Vec<String>)>::new()));
    let outcomes = Arc::new(Mutex::new(Vec::<String>::new()));
    // Each outcome is appended as it completes, so an interrupted run keeps
    // what it finished.
    let done: std::collections::HashSet<String> = match (&report, resume) {
        (Some(path), true) => fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split('\t').next())
            .map(str::to_owned)
            .collect(),
        _ => Default::default(),
    };
    let files: Vec<PathBuf> = files
        .into_iter()
        .filter(|f| {
            let name = f.file_name().unwrap().to_string_lossy();
            !done.contains(name.trim_end_matches(".gz"))
        })
        .collect();
    let report_file = Arc::new(Mutex::new(report.as_ref().and_then(|path| {
        fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(resume)
            .truncate(!resume)
            .open(path)
            .ok()
    })));
    let files = Arc::new(files);
    let binaries = Arc::new(binaries);
    let workers: Vec<_> = (0..jobs.max(1))
        .map(|worker| {
            let outcomes = Arc::clone(&outcomes);
            let report_file = Arc::clone(&report_file);
            let coverage = coverage.clone();
            let (next, failures, files, binaries, blobs, repo, scratch_root) = (
                Arc::clone(&next),
                Arc::clone(&failures),
                Arc::clone(&files),
                Arc::clone(&binaries),
                blobs.clone(),
                repo.clone(),
                scratch_root.clone(),
            );
            std::thread::spawn(move || loop {
                let index = next.fetch_add(1, Ordering::SeqCst);
                let Some(file) = files.get(index) else { break };
                let name = file
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .trim_end_matches(".gz")
                    .to_owned();
                // Per run, so two runs side by side never share a directory.
                let scratch =
                    scratch_root.join(format!("{}-w{worker}-{index}", std::process::id()));
                let _ = fs::remove_dir_all(&scratch);
                let _ = fs::create_dir_all(&scratch);
                let result = read_session(file).and_then(|session| {
                    let profile = coverage.as_ref().map(|_| scratch.join("profile"));
                    if let Some(profile) = &profile {
                        let _ = fs::create_dir_all(profile);
                    }
                    replay(
                        &session,
                        &blobs,
                        &binaries,
                        &repo,
                        &scratch,
                        profile.as_deref(),
                    )
                });
                if let Some(out) = &coverage {
                    merge_profile(
                        &scratch.join("profile"),
                        &out.join(name.replace(".json", ".profdata")),
                    );
                }
                let problems = match result {
                    Ok(outcome) => {
                        if update && !outcome.differences.is_empty() {
                            if let Err(error) = rerecord(file, &blobs, &outcome.replayed) {
                                eprintln!("could not re-record {name}: {error}");
                            }
                        }
                        outcome.differences
                    }
                    Err(error) => vec![format!("could not replay: {error}")],
                };
                if !keep && scratch.starts_with(&scratch_root) {
                    let _ = fs::remove_dir_all(&scratch);
                }
                let first = problems
                    .first()
                    .map(|p| p.lines().next().unwrap_or("").to_owned())
                    .unwrap_or_default();
                let line = format!(
                    "{name}\t{}\t{}",
                    if problems.is_empty() { "ok" } else { "differs" },
                    first.replace('\t', " ")
                );
                if let Some(file) = report_file.lock().unwrap().as_mut() {
                    use std::io::Write;
                    let _ = writeln!(file, "{line}");
                    let _ = file.flush();
                }
                outcomes.lock().unwrap().push(line);
                if !problems.is_empty() {
                    failures.lock().unwrap().push((name, problems));
                }
            })
        })
        .collect();
    for worker in workers {
        let _ = worker.join();
    }

    let mut failures = failures.lock().unwrap().clone();
    failures.sort();
    for (name, problems) in &failures {
        println!("FAIL {name}");
        for problem in problems.iter().take(5) {
            println!("  {problem}");
        }
    }
    println!(
        "{} sessions replayed, {} differ",
        files.len(),
        failures.len()
    );
    if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

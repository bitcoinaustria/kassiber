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
    use kassiber_session_replay::encrypted::{maybe_encrypted, Allowlist};
    use kassiber_session_replay::Entry;
    let Some(rec) = args.first().map(PathBuf::from) else {
        eprintln!("usage: session-replay decrypt REC --allowlist FILE");
        return ExitCode::from(2);
    };
    let Some(allowlist) = args
        .iter()
        .position(|a| a == "--allowlist")
        .and_then(|i| args.get(i + 1))
        .map(|f| Allowlist::load(Path::new(f)))
    else {
        eprintln!("--allowlist FILE is required");
        return ExitCode::from(2);
    };
    let blobs = Blobs::new(&rec);
    let files = sessions(&rec);
    // The passphrases the recording already opened books with.
    let mut candidates: Vec<String> = Vec::new();
    for file in &files {
        let Ok(session) = read_session(file) else {
            continue;
        };
        for root in &session.roots {
            for entry in root.before.iter().chain(&root.after) {
                if let Entry::File {
                    encryption: Some(e),
                    ..
                } = entry
                {
                    if !candidates.contains(&e.passphrase) {
                        candidates.push(e.passphrase.clone());
                    }
                }
            }
        }
    }
    candidates.extend(args.iter().skip_while(|a| *a != "--also").skip(1).cloned());
    candidates.retain(|p| allowlist.allows(p));
    let scratch = repo().join("target").join("session-replay-decrypt");
    let mut known: BTreeMap<
        String,
        Option<(String, kassiber_session_replay::encrypted::Encryption)>,
    > = BTreeMap::new();
    let (mut opened, mut rewritten) = (0usize, 0usize);
    for file in &files {
        let Ok(mut session) = read_session(file) else {
            continue;
        };
        let mut changed = false;
        for root in &mut session.roots {
            for entry in root.before.iter_mut().chain(root.after.iter_mut()) {
                let Entry::File {
                    blob, encryption, ..
                } = entry
                else {
                    continue;
                };
                if encryption.is_some() {
                    continue;
                }
                let result = known.entry(blob.clone()).or_insert_with(|| {
                    let bytes = blobs.get(blob).ok()?;
                    if !maybe_encrypted(&bytes) {
                        return None;
                    }
                    let (plaintext, mut found) =
                        kassiber_session_replay::encrypted::decrypt(&bytes, &candidates, &scratch)?;
                    found.ciphertext = Some(blob.clone());
                    opened += 1;
                    Some((blobs.put(&plaintext).ok()?, found))
                });
                if let Some((plain, found)) = result.clone() {
                    *blob = plain;
                    *encryption = Some(found);
                    changed = true;
                }
            }
        }
        if changed {
            let bytes = serde_json::to_vec(&session).unwrap_or_default();
            let written = if file.to_string_lossy().ends_with(".gz") {
                let mut encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(9));
                std::io::Write::write_all(&mut encoder, &bytes)
                    .and_then(|_| encoder.finish())
                    .and_then(|packed| fs::write(file, packed))
            } else {
                fs::write(file, bytes)
            };
            if written.is_err() {
                eprintln!("cannot rewrite {}", file.display());
                return ExitCode::from(2);
            }
            rewritten += 1;
        }
    }
    println!(
        "{} passphrases tried; {opened} books opened; {rewritten} sessions rewritten",
        candidates.len()
    );
    ExitCode::SUCCESS
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

/// The shard a pack file goes to: `sessions.tar` for the sessions, and
/// `blobs-<first hex digit>.tar` for the blobs, so the test-data lock pins
/// 17 files whatever the number of recordings.
fn shard(relative: &Path) -> String {
    let mut parts = relative.components();
    match parts
        .next()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
    {
        Some(top) if top == "blobs" => {
            let prefix = parts
                .next()
                .map(|c| {
                    c.as_os_str()
                        .to_string_lossy()
                        .chars()
                        .next()
                        .unwrap_or('0')
                })
                .unwrap_or('0');
            format!("blobs-{prefix}.tar")
        }
        _ => "sessions.tar".to_owned(),
    }
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, base, out);
        } else if let Ok(relative) = path.strip_prefix(base) {
            out.push(relative.to_path_buf());
        }
    }
}

/// `archive PACK OUT`: a pack directory as deterministic tars (sorted
/// names, zero times and owners, mode 0644), so an unchanged recording
/// archives to identical bytes.
fn archive(args: &[String]) -> ExitCode {
    let (Some(src), Some(out)) = (args.first(), args.get(1)) else {
        eprintln!("usage: session-replay archive PACK OUT");
        return ExitCode::from(2);
    };
    let (src, out) = (PathBuf::from(src), PathBuf::from(out));
    let mut files = Vec::new();
    walk(&src, &src, &mut files);
    files.sort();
    let mut shards: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for file in files {
        shards.entry(shard(&file)).or_default().push(file);
    }
    if fs::create_dir_all(&out).is_err() {
        eprintln!("cannot create {}", out.display());
        return ExitCode::from(2);
    }
    for (name, files) in &shards {
        let mut builder = tar::Builder::new(Vec::new());
        builder.mode(tar::HeaderMode::Deterministic);
        for file in files {
            let Ok(bytes) = fs::read(src.join(file)) else {
                eprintln!("cannot read {}", file.display());
                return ExitCode::from(2);
            };
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(0);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();
            if builder
                .append_data(&mut header, file, bytes.as_slice())
                .is_err()
            {
                eprintln!("cannot archive {}", file.display());
                return ExitCode::from(2);
            }
        }
        let Ok(bytes) = builder.into_inner() else {
            return ExitCode::from(2);
        };
        if fs::write(out.join(name), bytes).is_err() {
            eprintln!("cannot write {name}");
            return ExitCode::from(2);
        }
    }
    println!("{} archives in {}", shards.len(), out.display());
    ExitCode::SUCCESS
}

/// `extract OUT DIR`: the archives `archive` wrote, back as a pack directory.
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
        .filter(|p| p.extension().is_some_and(|e| e == "tar"))
        .collect();
    names.sort();
    if names.is_empty() {
        eprintln!("no archives in {}", src.display());
        return ExitCode::from(2);
    }
    for name in names {
        let Ok(file) = fs::File::open(&name) else {
            return ExitCode::from(2);
        };
        let mut archive = tar::Archive::new(file);
        // `unpack` refuses entries that would land outside DST.
        if let Err(error) = archive.unpack(&dst) {
            eprintln!("cannot extract {}: {error}", name.display());
            return ExitCode::from(2);
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

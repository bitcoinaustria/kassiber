//! Replay recorded process sessions against the current binaries.
//!
//!     session-replay replay DIR --bin NAME=PATH ... [--examples DIR] [--only TEXT] [--jobs N]
//!                           [--keep] [--report FILE] [--coverage DIR]
//!     session-replay list DIR
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

fn sessions(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(dir.join("sessions"))
        .map(|read| read.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    found.retain(|p| p.extension().is_some_and(|e| e == "json"));
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        eprintln!("usage: session-replay replay|list DIR ...");
        return ExitCode::from(2);
    };
    let Some(dir) = args.get(1).map(PathBuf::from) else {
        eprintln!("usage: session-replay {command} DIR ...");
        return ExitCode::from(2);
    };
    let mut binaries = BTreeMap::new();
    let mut only = Vec::new();
    let mut jobs = 4usize;
    let mut keep = false;
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
                let test = fs::read_to_string(f)
                    .ok()
                    .and_then(|t| serde_json::from_str::<Session>(&t).ok())
                    .and_then(|s| s.test)
                    .unwrap_or_default();
                only.iter()
                    .any(|o| name.contains(o.as_str()) || test.contains(o.as_str()))
            }
        })
        .collect();
    if command == "list" {
        for file in &files {
            let session: Session =
                serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
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
    let report_file = Arc::new(Mutex::new(
        report.as_ref().and_then(|path| fs::File::create(path).ok()),
    ));
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
                let name = file.file_name().unwrap().to_string_lossy().into_owned();
                let scratch = scratch_root.join(format!("w{worker}-{index}"));
                let _ = fs::remove_dir_all(&scratch);
                let _ = fs::create_dir_all(&scratch);
                let result = fs::read_to_string(file)
                    .map_err(|e| e.to_string())
                    .and_then(|t| serde_json::from_str::<Session>(&t).map_err(|e| e.to_string()))
                    .and_then(|session| {
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
                    Ok(outcome) => outcome.differences,
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

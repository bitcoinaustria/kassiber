//! The journal reproducibility check through the real binary: the CLI leaf,
//! the daemon kind, its narrowed arguments, and that it writes nothing
//! (docs/reference/replay-bundle.md). The desk arms the test egress guard,
//! so any connection attempt fails the run.

#![cfg(feature = "test-harness")]

mod support;

use kassiber_pyjson::Value;
use support::{kind_of, path, text, Daemon, Desk};

fn json(output: &std::process::Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    kassiber_pytext::loads(stdout.trim()).unwrap_or_else(|_| {
        panic!(
            "stdout {stdout:?} stderr {:?}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn data<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    let mut full = vec!["data"];
    full.extend_from_slice(keys);
    path(value, &full).and_then(text)
}

fn error_code(value: &Value) -> Option<&str> {
    path(value, &["error", "code"]).and_then(text)
}

/// Every file under `root`, without SQLite's transient sidecars.
fn files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.to_string_lossy().into_owned();
            if path.is_dir() {
                stack.push(path);
            } else if !name.ends_with("-shm")
                && !name.ends_with("-wal")
                && !name.ends_with("-journal")
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn cli_checks_the_journals_without_writing() {
    let desk = Desk::from_book("journals_verify_cli", "paired-fee-processed");
    let base = desk.base_path();
    // The first command of a desk bootstraps its settings; compare around a
    // second run, by path and by the book's bytes.
    let first = json(&desk.run(&["journals", "verify"], b""));
    assert_eq!(data(&first, &["status"]), Some("identical"), "{first:?}");
    let contents = |paths: &[std::path::PathBuf]| -> Vec<Vec<u8>> {
        paths.iter().map(|p| std::fs::read(p).unwrap()).collect()
    };
    let before = files(&base);
    let bytes = contents(&before);
    let report = json(&desk.run(&["journals", "verify"], b""));
    assert_eq!(data(&report, &["status"]), Some("identical"), "{report:?}");
    let after = files(&base);
    assert_eq!(after, before, "the check wrote a file");
    assert!(contents(&after) == bytes, "the check changed a file");
}

#[test]
fn daemon_checks_the_journals_with_only_a_limit() {
    let desk = Desk::from_book("journals_verify_daemon", "paired-fee-processed");
    let mut daemon = Daemon::spawn(&desk, &[]);
    daemon.require_kind("ui.journals.verify");
    let report = daemon.step("verify", "ui.journals.verify", Some(r#"{"limit": 5}"#));
    assert_eq!(kind_of(&report), "ui.journals.verify");
    assert_eq!(data(&report, &["status"]), Some("identical"), "{report:?}");
    let scoped = daemon.step(
        "scoped",
        "ui.journals.verify",
        Some(r#"{"profile": "other"}"#),
    );
    assert_eq!(error_code(&scoped), Some("validation"));
    daemon.exit_cleanly();
}

//! Replay bundles through the real binary: the CLI leaves, the daemon kind,
//! consent, and that nothing but the chosen bundle reaches disk
//! (docs/reference/replay-bundle.md). The desk arms the test egress guard,
//! so any connection attempt fails the run.

#![cfg(feature = "test-harness")]

mod support;

use kassiber_pyjson::Value;
use support::{path, text, Daemon, Desk};

const PASSPHRASE: &str = "cli-replay-passphrase";

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
fn cli_exports_and_verifies_a_bundle_without_other_writes() {
    let desk = Desk::from_book("replay_bundle_cli", "paired-fee-processed");
    // The first command bootstraps the desk's settings; the book reproduces.
    let report = json(&desk.run(&["journals", "verify"], b""));
    assert_eq!(data(&report, &["status"]), Some("identical"), "{report:?}");

    let base = desk.base_path();
    let bundle = base.join("support.kassiber-replay");
    let bundle_arg = bundle.to_string_lossy().into_owned();
    let before = files(&base);

    let refused = json(&desk.run(
        &[
            "diagnostics",
            "replay-bundle",
            "export",
            "--file",
            &bundle_arg,
        ],
        b"",
    ));
    assert_eq!(error_code(&refused), Some("confirmation_required"));
    let refused = json(&desk.run(
        &[
            "diagnostics",
            "replay-bundle",
            "export",
            "--file",
            &bundle_arg,
            "--confirm",
        ],
        b"",
    ));
    assert_eq!(error_code(&refused), Some("interaction_required"));
    assert_eq!(files(&base), before, "a refused export wrote a file");

    let exported = json(&desk.run(
        &[
            "diagnostics",
            "replay-bundle",
            "export",
            "--file",
            &bundle_arg,
            "--confirm",
            "--bundle-passphrase-fd",
            "0",
        ],
        PASSPHRASE.as_bytes(),
    ));
    assert_eq!(
        data(&exported, &["file"]),
        Some(bundle_arg.as_str()),
        "{exported:?}"
    );
    assert_eq!(
        data(&exported, &["manifest", "local_check", "status"]),
        Some("identical")
    );
    let mut expected = before.clone();
    expected.push(bundle.clone());
    expected.sort();
    assert_eq!(files(&base), expected, "export wrote more than the bundle");

    let again = json(&desk.run(
        &[
            "diagnostics",
            "replay-bundle",
            "export",
            "--file",
            &bundle_arg,
            "--confirm",
            "--bundle-passphrase-fd",
            "0",
        ],
        PASSPHRASE.as_bytes(),
    ));
    assert_eq!(error_code(&again), Some("file_exists"));

    // Verify reads no book: point it at an empty data root.
    let verified = desk
        .raw_command(&[
            "--machine",
            "--data-root",
            &base.join("no-book").to_string_lossy(),
            "diagnostics",
            "replay-bundle",
            "verify",
            &bundle_arg,
            "--bundle-passphrase-fd",
            "0",
        ])
        .spawn()
        .map(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(PASSPHRASE.as_bytes())
                .unwrap();
            child.wait_with_output().unwrap()
        })
        .unwrap();
    let verified = json(&verified);
    assert_eq!(
        data(&verified, &["status"]),
        Some("identical"),
        "{verified:?}"
    );
    assert!(!base.join("no-book").exists(), "verify created a data root");
}

#[test]
fn daemon_exports_only_with_consent_into_managed_exports() {
    let desk = Desk::from_book("replay_bundle_daemon", "paired-fee-processed");
    let mut daemon = Daemon::spawn(&desk, &[]);
    daemon.require_kind("ui.diagnostics.replay_bundle.export");
    // The first request opens the book (and its owner locks); it reproduces.
    let report = daemon.step("verify", "ui.journals.verify", None);
    assert_eq!(data(&report, &["status"]), Some("identical"), "{report:?}");

    let base = desk.base_path();
    let before = files(&base);
    let refused = daemon.step(
        "unconfirmed",
        "ui.diagnostics.replay_bundle.export",
        Some(&format!(r#"{{"passphrase": "{PASSPHRASE}"}}"#)),
    );
    assert_eq!(error_code(&refused), Some("confirmation_required"));
    let pathed = daemon.step(
        "pathed",
        "ui.diagnostics.replay_bundle.export",
        Some(&format!(
            r#"{{"passphrase": "{PASSPHRASE}", "confirm": true, "file": "/tmp/x.kassiber-replay"}}"#
        )),
    );
    assert_eq!(error_code(&pathed), Some("validation"));
    assert_eq!(files(&base), before);

    let exported = daemon.step(
        "export",
        "ui.diagnostics.replay_bundle.export",
        Some(&format!(
            r#"{{"passphrase": "{PASSPHRASE}", "confirm": true}}"#
        )),
    );
    let file = data(&exported, &["file"]).expect("bundle file").to_owned();
    assert!(file.contains("/exports/diagnostics/replay-"), "{file}");
    assert!(file.ends_with(".kassiber-replay"));
    let after = files(&base);
    let added: Vec<_> = after.iter().filter(|p| !before.contains(p)).collect();
    assert_eq!(added.len(), 1, "{added:?}");
    let sealed = std::fs::read(&file).unwrap();
    let report = kassiber_replay::verify(&sealed, PASSPHRASE, 5).unwrap();
    assert_eq!(report["status"], "identical");
    daemon.exit_cleanly();
}

//! Book settings through the real binary: the CLI's `profiles settings` and
//! `profiles configure`, the daemon's `ui.profiles.settings` and widened
//! `ui.profiles.update`, a dry run that writes nothing, and a generic book's
//! own tax year reaching the stored tax summary
//! (docs/reference/tax.md#book-settings). The desk arms the test egress
//! guard, so any connection attempt fails the run.

#![cfg(feature = "test-harness")]

mod support;

use kassiber_pyjson::Value;
use support::{kind_of, list, path, text, Daemon, Desk};

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

fn version(desk: &Desk) -> i64 {
    desk.connection()
        .query_row(
            "SELECT journal_input_version FROM profiles WHERE id='p'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn cli_previews_applies_and_the_engine_cuts_the_books_tax_year() {
    let mut desk = Desk::from_book("book_settings_cli", "paired-fee-processed");
    desk.journal_env();
    let settings = json(&desk.run(&["profiles", "settings"], b""));
    assert_eq!(kind_of(&settings), "profiles.settings");
    assert_eq!(data(&settings, &["tax_year_start"]), Some("01-01"));
    assert_eq!(data(&settings, &["timezone"]), Some("UTC"));
    let before = version(&desk);
    // A dry run reports the change and writes nothing.
    let preview = json(&desk.run(
        &[
            "profiles",
            "configure",
            "--tax-year-start",
            "april",
            "--dry-run",
        ],
        b"",
    ));
    assert_eq!(kind_of(&preview), "profiles.configure");
    assert_eq!(
        path(&preview, &["data", "dry_run"]),
        Some(&Value::Bool(true)),
        "{preview:?}"
    );
    let changes = list(&preview, &["data", "changes"]);
    assert_eq!(changes.len(), 1, "{preview:?}");
    assert_eq!(path(&changes[0], &["to"]).and_then(text), Some("04-01"));
    assert_eq!(
        path(&preview, &["data", "journals", "reprocess"]),
        Some(&Value::Bool(true))
    );
    assert_eq!(version(&desk), before);
    let unchanged = json(&desk.run(&["profiles", "settings"], b""));
    assert_eq!(data(&unchanged, &["tax_year_start"]), Some("01-01"));
    // Applied: the journals are stale, then the engine cuts 1 April years.
    let applied = json(&desk.run(&["profiles", "configure", "--tax-year-start", "april"], b""));
    assert_eq!(
        data(&applied, &["tax_year_start"]),
        Some("04-01"),
        "{applied:?}"
    );
    assert_eq!(version(&desk), before + 1);
    let processed = json(&desk.run(&["journals", "process"], b""));
    assert!(path(&processed, &["error"]).is_none(), "{processed:?}");
    let years: std::collections::BTreeSet<i64> = desk
        .connection()
        .prepare("SELECT year FROM journal_tax_summary WHERE profile_id='p'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    // The March 2024 sale and interest fall in the year from 1 April 2023,
    // the April sale in the year from 1 April 2024.
    assert_eq!(years, [2023, 2024].into_iter().collect());
    // A locked country refuses its year, with the typed code.
    let refused = json(&desk.run(
        &[
            "profiles",
            "configure",
            "--tax-country",
            "at",
            "--gains-algorithm",
            "FIFO",
            "--tax-year-start",
            "07-01",
        ],
        b"",
    ));
    assert_eq!(
        path(&refused, &["error", "code"]).and_then(text),
        Some("tax_year_start_locked"),
        "{refused:?}"
    );
}

#[test]
fn daemon_reads_and_previews_any_books_settings() {
    let desk = Desk::from_book("book_settings_daemon", "paired-fee-processed");
    let mut daemon = Daemon::spawn(&desk, &[]);
    daemon.require_kind("ui.profiles.settings");
    let settings = daemon.step(
        "settings",
        "ui.profiles.settings",
        Some(r#"{"profile_id": "p"}"#),
    );
    assert_eq!(kind_of(&settings), "ui.profiles.settings");
    assert_eq!(data(&settings, &["tax_year_editable"]), None);
    assert_eq!(
        path(&settings, &["data", "tax_year_editable"]),
        Some(&Value::Bool(true))
    );
    let before = version(&desk);
    let preview = daemon.step(
        "preview",
        "ui.profiles.update",
        Some(
            r#"{"profile_id": "p", "fiat_currency": "CHF", "timezone": "Europe/Zurich", "require_coarse_review": true, "dry_run": true}"#,
        ),
    );
    assert_eq!(
        path(&preview, &["data", "dry_run"]),
        Some(&Value::Bool(true)),
        "{preview:?}"
    );
    assert!(path(&preview, &["data", "price_currency_holds"]).is_some());
    assert_eq!(version(&desk), before);
    let refused = daemon.step(
        "refused",
        "ui.profiles.update",
        Some(r#"{"profile_id": "p", "tax_year_start": "02-29"}"#),
    );
    assert_eq!(
        path(&refused, &["error", "code"]).and_then(text),
        Some("tax_year_start_invalid")
    );
    daemon.exit_cleanly();
}

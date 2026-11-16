use super::*;
use crate::file_import::{execute, Format, Request};

const NOW: &str = "2026-07-15T10:00:00Z";

fn book() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let creation: Json =
        serde_json::from_str(include_str!("../../../store/fixtures/creation.json")).unwrap();
    for sql in creation["statements"].as_array().unwrap() {
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    conn.execute_batch(
        "INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01');
         INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Book','2025-01-01');",
    )
    .unwrap();
    conn
}

fn wallet(conn: &Connection, id: &str, kind: &str, config: &str) {
    conn.execute(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'ws','p',?,?,?,'2025-01-01')",
        params![id, id, kind, config],
    )
    .unwrap();
}

fn transaction(conn: &Connection, wallet: &str, id: &str, at: &str) {
    conn.execute(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES(?,'ws','p',?,?,?,?,'inbound','BTC',100000000,0,'2025-01-01')",
        params![id, wallet, id, id, at],
    )
    .unwrap();
}

fn source(conn: &Connection, wallet: &str, period_end: Option<&str>) -> Source {
    sources(
        conn,
        "p",
        &Options {
            period_end,
            wallet: Some(wallet),
            ..Options::default()
        },
        NOW,
    )
    .unwrap()
    .remove(0)
}

struct TempFile(std::path::PathBuf);
impl TempFile {
    fn new(name: &str, content: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "kassiber-coverage-{}-{:?}-{name}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, content).unwrap();
        Self(path)
    }
    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn import(conn: &Connection, wallet: &str, file: &TempFile) -> kassiber_pyjson::Value {
    execute(
        conn,
        Request {
            profile_id: "p".into(),
            wallet: wallet.into(),
            file: file.path().into(),
            format: Format::Csv,
            mode: None,
            companion: None,
            timezone: None,
        },
        None,
    )
    .unwrap()
}

#[test]
fn live_sources_are_current_through_their_last_successful_sync() {
    let conn = book();
    wallet(
        &conn,
        "cold",
        "descriptor",
        r#"{"descriptor":"wpkh(x)","last_synced_at":"2026-07-15T08:00:00Z"}"#,
    );
    transaction(&conn, "cold", "t1", "2024-03-01T00:00:00Z");
    let today = source(&conn, "cold", None);
    assert_eq!(today.source_mode, "live");
    assert_eq!(today.freshness_state, "current");
    assert_eq!(today.covered_from.as_deref(), Some("2024-03-01T00:00:00Z"));
    assert_eq!(
        today.covered_through.as_deref(),
        Some("2026-07-15T08:00:00Z")
    );
    // A past period end is covered too; a later one is not.
    assert_eq!(
        source(&conn, "cold", Some("2025-12-31")).freshness_state,
        "current"
    );
    let later = source(&conn, "cold", Some("2026-12-31"));
    assert_eq!((later.freshness_state, later.action), ("behind", "sync"));

    wallet(&conn, "fresh", "descriptor", r#"{"descriptor":"wpkh(y)"}"#);
    let never = source(&conn, "fresh", None);
    assert_eq!(
        (never.freshness_state, never.reason),
        ("never_imported", "never_synced")
    );
    assert!(never.needs_action());
}

#[test]
fn a_failed_sync_outranks_coverage() {
    let conn = book();
    wallet(
        &conn,
        "cold",
        "descriptor",
        r#"{"descriptor":"wpkh(x)","last_synced_at":"2026-07-15T08:00:00Z"}"#,
    );
    conn.execute_batch(
        "INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,last_success_at,last_error_at,last_error_code,updated_at)
         VALUES('p','onchain_wallet:cold','onchain_wallet','cold','error','2026-07-15T08:00:00Z','2026-07-15T09:00:00Z','backend_unreachable','x')",
    )
    .unwrap();
    let failed = source(&conn, "cold", None);
    assert_eq!(failed.freshness_state, "sync_failed");
    assert_eq!(
        failed.last_sync_error_code.as_deref(),
        Some("backend_unreachable")
    );
    // A later success clears it.
    conn.execute_batch(
        "UPDATE freshness_source_states SET status='fresh', last_success_at='2026-07-15T09:30:00Z'",
    )
    .unwrap();
    assert_eq!(source(&conn, "cold", None).freshness_state, "current");
}

#[test]
fn file_sources_cover_the_rows_their_files_held_not_the_import_time() {
    let conn = book();
    wallet(&conn, "csv", "custom", r#"{"source_format":"csv"}"#);
    let empty = source(&conn, "csv", None);
    assert_eq!(
        (empty.source_mode, empty.freshness_state, empty.action),
        ("file", "never_imported", "import_newer_file")
    );

    let first = TempFile::new(
        "first.csv",
        "date,txid,direction,asset,amount,fee\n2025-02-01,a,in,BTC,1,0\n2025-06-30,b,in,BTC,1,0\n",
    );
    import(&conn, "csv", &first);
    let partial = source(&conn, "csv", Some("2025-12-31"));
    assert_eq!(partial.freshness_state, "behind");
    assert_eq!(partial.reason, "file_ends_before_period_end");
    assert_eq!(
        partial
            .covered_through
            .as_deref()
            .and_then(day_of)
            .as_deref(),
        Some("2025-06-30")
    );
    assert_eq!(
        source(&conn, "csv", Some("2025-06-30")).freshness_state,
        "current"
    );

    // A newer export that repeats old rows: the range counts every row read.
    let second = TempFile::new(
        "second.csv",
        "date,txid,direction,asset,amount,fee\n2025-02-01,a,in,BTC,1,0\n2025-06-30,b,in,BTC,1,0\n2025-12-31,c,in,BTC,1,0\n",
    );
    import(&conn, "csv", &second);
    let (first_row, last_row, read, sha): (String, String, i64, String) = conn
        .query_row(
            "SELECT first_row_at,last_row_at,rows_read,file_sha256 FROM import_batches ORDER BY import_sequence DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(day_of(&first_row).as_deref(), Some("2025-02-01"));
    assert_eq!(day_of(&last_row).as_deref(), Some("2025-12-31"));
    assert_eq!(read, 3);
    assert_eq!(
        Some(sha),
        crate::file_import::file_snapshot(second.path()).sha256
    );
    let full = source(&conn, "csv", Some("2025-12-31"));
    assert_eq!((full.freshness_state, full.import_count), ("current", 2));
    assert_eq!(
        (full.last_run_rows_read, full.last_run_rows_inserted),
        (Some(3), Some(1))
    );
    assert_eq!(
        full.covered_from.as_deref().and_then(day_of).as_deref(),
        Some("2025-02-01")
    );

    // Importing the identical file again adds no rows but still records the
    // run (finding 8 of the review): what the file held, and that it was seen.
    let again = import(&conn, "csv", &second);
    assert_eq!(
        kassiber_envelope::get(&again, "imported"),
        Some(&Value::from(0_i64))
    );
    assert_eq!(kassiber_envelope::get(&again, "import_batch_id"), None);
    let rerun = source(&conn, "csv", None);
    assert_eq!(rerun.import_count, 3);
    assert_eq!(
        (rerun.last_run_rows_read, rerun.last_run_rows_inserted),
        (Some(3), Some(0))
    );
}

/// A run as an older import recorded it, or with the given file range.
fn batch(conn: &Connection, wallet: &str, id: &str, at: &str, range: Option<(&str, &str)>) {
    conn.execute(
        "INSERT INTO import_batches(id,workspace_id,profile_id,wallet_id,source_format,imported_at,rows_inserted,first_row_at,last_row_at,rows_read) \
         VALUES(?,'ws','p',?,'csv',?,0,?,?,?)",
        params![
            id,
            wallet,
            at,
            range.map(|r| r.0),
            range.map(|r| r.1),
            range.map(|_| 1_i64)
        ],
    )
    .unwrap();
    // The rows a file held are in the book, created by this run: a range
    // rests on them.
    if let Some((first, last)) = range {
        for (row, at) in [("first", first), ("last", last)] {
            let row = format!("{id}-{row}");
            transaction(conn, wallet, &row, at);
            member(conn, id, &row);
        }
    }
}

fn member(conn: &Connection, batch: &str, transaction: &str) {
    conn.execute(
        "INSERT INTO import_batch_transactions(batch_id,transaction_id) VALUES(?,?)",
        params![batch, transaction],
    )
    .unwrap();
}

/// A dynamic key of the coverage payload (present after the review fixes).
fn field(source: &Source, path: &[&str]) -> Value {
    path.iter()
        .try_fold(source.to_value(), |value, key| {
            kassiber_envelope::get(&value, key).cloned()
        })
        .unwrap_or(Value::Null)
}

#[test]
fn a_remembered_file_that_changed_since_its_import_is_flagged_from_metadata() {
    let conn = book();
    let file = TempFile::new(
        "remembered.csv",
        "date,txid,direction,asset,amount,fee\n2025-02-01,a,in,BTC,1,0\n",
    );
    wallet(
        &conn,
        "csv",
        "custom",
        &serde_json::json!({"source_format": "csv", "source_file": file.path()}).to_string(),
    );
    import(&conn, "csv", &file);
    // The import is recorded after the file was written.
    conn.execute_batch("UPDATE import_batches SET imported_at='9999-01-01T00:00:00Z'")
        .unwrap();
    assert_eq!(source(&conn, "csv", None).source_file_changed, Some(false));
    // A different size than the import recorded is a newer export, even
    // before its time says so.
    std::fs::write(
        &file.0,
        "date,txid,direction,asset,amount,fee\n2025-02-01,a,in,BTC,1,0\n2025-03-01,b,in,BTC,1,0\n",
    )
    .unwrap();
    assert_eq!(source(&conn, "csv", None).source_file_changed, Some(true));
    // A symlink is not followed.
    #[cfg(unix)]
    {
        let link = std::env::temp_dir().join(format!(
            "kassiber-coverage-{}-{:?}-link.csv",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&file.0, &link).unwrap();
        conn.execute(
            "UPDATE wallets SET config_json=? WHERE id='csv'",
            [serde_json::json!({"source_format": "csv", "source_file": link}).to_string()],
        )
        .unwrap();
        assert_eq!(source(&conn, "csv", None).source_file_changed, None);
        let _ = std::fs::remove_file(&link);
    }
}

/// Finding 3: a display read never opens the remembered file, so a FIFO in
/// its place cannot block it.
#[cfg(unix)]
#[test]
fn a_fifo_in_place_of_the_remembered_file_never_blocks_a_read() {
    let fifo = std::env::temp_dir().join(format!(
        "kassiber-coverage-{}-{:?}-fifo",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&fifo);
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    let path = fifo.to_str().unwrap().to_owned();
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let conn = book();
        wallet(
            &conn,
            "csv",
            "custom",
            &serde_json::json!({"source_format": "csv", "source_file": path}).to_string(),
        );
        batch(
            &conn,
            "csv",
            "b1",
            "2000-01-01T00:00:00Z",
            Some(("2025-01-01", "2025-01-02")),
        );
        let changed = source(&conn, "csv", None).source_file_changed;
        // An import's snapshot of a FIFO records nothing and returns at once.
        let snapshot = crate::file_import::file_snapshot(&path);
        let _ = done.send((changed, snapshot.sha256, snapshot.size));
    });
    let outcome = finished.recv_timeout(std::time::Duration::from_secs(10));
    let _ = std::fs::remove_file(&fifo);
    assert_eq!(outcome.expect("blocked on the FIFO"), (None, None, None));
}

/// Finding 3: hashing during an import is bounded; a larger file keeps only
/// its size, and the read stops at the limit.
#[test]
fn import_snapshots_hash_only_up_to_their_limit() {
    let small = TempFile::new("small.csv", "abc");
    let snapshot = crate::file_import::file_snapshot_bounded(small.path(), 3);
    assert_eq!(snapshot.size, Some(3));
    assert_eq!(
        snapshot.sha256.as_deref(),
        Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );
    let large = TempFile::new("large.csv", "");
    std::fs::File::options()
        .write(true)
        .open(&large.0)
        .unwrap()
        .set_len(64 * 1024 * 1024)
        .unwrap();
    let snapshot = crate::file_import::file_snapshot_bounded(large.path(), 1024);
    assert_eq!(
        (snapshot.size, snapshot.sha256),
        (Some(64 * 1024 * 1024), None)
    );
}

/// Finding 8: a CSV whose rows an API wallet already holds still makes it a
/// hybrid source, through the run the re-import records.
#[test]
fn a_csv_import_that_adds_nothing_still_counts_as_csv_evidence() {
    let conn = book();
    wallet(&conn, "kraken", "kraken", r#"{"backend":"kraken"}"#);
    let csv = TempFile::new(
        "kraken.csv",
        "date,txid,direction,asset,amount,fee\n2024-02-01,k1,in,BTC,1,0\n",
    );
    import(&conn, "kraken", &csv);
    import(&conn, "kraken", &csv);
    let hybrid = source(&conn, "kraken", None);
    assert_eq!((hybrid.source_mode, hybrid.import_count), ("hybrid", 2));
    assert_eq!(
        hybrid
            .file_last_row_at
            .as_deref()
            .and_then(day_of)
            .as_deref(),
        Some("2024-02-01")
    );
}

/// Finding 5: a generic-ledger or custom wallet with a remembered file is a
/// file snapshot and goes behind; one with neither stays manual.
#[test]
fn ledgers_with_a_remembered_file_are_file_sources() {
    let conn = book();
    wallet(
        &conn,
        "ledger",
        "custom",
        r#"{"source_format":"generic_ledger","source_file":"/books/ledger.xlsx"}"#,
    );
    batch(
        &conn,
        "ledger",
        "b1",
        "2025-04-01T00:00:00Z",
        Some(("2025-01-01T00:00:00Z", "2025-03-01T00:00:00Z")),
    );
    let ledger = source(&conn, "ledger", Some("2025-12-31"));
    assert_eq!(ledger.completeness_class, "manual");
    assert_eq!(
        (ledger.source_mode, ledger.freshness_state, ledger.action),
        ("file", "behind", "import_newer_file")
    );
    wallet(
        &conn,
        "custom",
        "custom",
        r#"{"source_file":"/exports/other.csv"}"#,
    );
    assert_eq!(source(&conn, "custom", None).source_mode, "file");
    wallet(&conn, "hand", "custom", "{}");
    transaction(&conn, "hand", "t1", "2025-03-01T00:00:00Z");
    assert_eq!(source(&conn, "hand", None).source_mode, "manual");
}

/// Finding 7: a file's reach comes from its import runs; a transaction added
/// by hand later does not extend it. Old runs without a recorded range count
/// only the rows they created.
#[test]
fn transactions_outside_import_runs_never_extend_file_coverage() {
    let conn = book();
    wallet(&conn, "csv", "custom", r#"{"source_format":"csv"}"#);
    batch(
        &conn,
        "csv",
        "b1",
        "2025-04-01T00:00:00Z",
        Some(("2025-01-10T00:00:00Z", "2025-03-01T00:00:00Z")),
    );
    transaction(&conn, "csv", "manual-later", "2025-11-20T00:00:00Z");
    let file = source(&conn, "csv", Some("2025-12-31"));
    assert_eq!(
        file.covered_through.as_deref(),
        Some("2025-03-01T00:00:00Z")
    );
    assert_eq!(file.freshness_state, "behind");

    wallet(&conn, "old", "custom", r#"{"source_format":"csv"}"#);
    transaction(&conn, "old", "o1", "2024-02-01T00:00:00Z");
    transaction(&conn, "old", "o2", "2024-05-01T00:00:00Z");
    transaction(&conn, "old", "hand", "2025-09-01T00:00:00Z");
    batch(&conn, "old", "legacy", "2024-06-01T00:00:00Z", None);
    member(&conn, "legacy", "o1");
    member(&conn, "legacy", "o2");
    let legacy = source(&conn, "old", Some("2025-12-31"));
    assert_eq!(
        (
            legacy.covered_from.as_deref(),
            legacy.covered_through.as_deref()
        ),
        (Some("2024-02-01T00:00:00Z"), Some("2024-05-01T00:00:00Z"))
    );
}

#[test]
fn manual_ledgers_have_no_currency_and_summaries_count_states() {
    let conn = book();
    wallet(&conn, "ledger", "custom", "{}");
    transaction(&conn, "ledger", "t1", "2025-03-01T00:00:00Z");
    wallet(&conn, "cold", "descriptor", r#"{"descriptor":"wpkh(x)"}"#);
    let manual = source(&conn, "ledger", None);
    assert_eq!(
        (manual.source_mode, manual.freshness_state),
        ("manual", "unknown")
    );
    assert!(!manual.needs_action());
    let report = report(&conn, "p", &Options::default(), NOW).unwrap();
    let summary = kassiber_envelope::get(&report, "summary").unwrap();
    assert_eq!(
        kassiber_envelope::get(summary, "needs_action"),
        Some(&Value::from(1_i64))
    );
    assert_eq!(
        kassiber_envelope::get(&report, "period_end"),
        Some(&s("2026-07-15"))
    );
    let bad = report_error(&conn);
    assert!(bad.contains("period_end"), "{bad}");
}

fn report_error(conn: &Connection) -> String {
    report(
        conn,
        "p",
        &Options {
            period_end: Some("last year"),
            ..Options::default()
        },
        NOW,
    )
    .unwrap_err()
    .message
}

#[test]
fn days_are_validated() {
    assert_eq!(
        day_of("2024-02-29T10:00:00Z").as_deref(),
        Some("2024-02-29")
    );
    assert_eq!(day_of("2023-02-29"), None);
    assert_eq!(day_of("2025-13-01"), None);
    assert_eq!(day_of("bad"), None);
}

/// Egress regression (docs/reference/privacy-and-security.md): background
/// freshness never plans an exchange API source, synced or not, hybrid or
/// not. Only the user's explicit Sync now or title-bar refresh reaches one.
#[test]
fn freshness_planning_never_includes_exchange_api_sources() {
    let conn = book();
    for kind in crate::file_import::exchange_api::KINDS {
        wallet(
            &conn,
            &format!("{kind}-api"),
            kind,
            &format!(r#"{{"backend":"{kind}","last_synced_at":"2026-01-01T00:00:00Z"}}"#),
        );
        wallet(
            &conn,
            &format!("{kind}-hybrid"),
            kind,
            &format!(r#"{{"backend":"{kind}","source_file":"/x.csv","source_format":"csv"}}"#),
        );
    }
    for wallet in [None, Some("kraken-api".to_owned())] {
        let specs = crate::freshness::sources::build(
            &conn,
            "p",
            &crate::freshness::sources::Options {
                wallet,
                rates: false,
                journals: false,
                auto_pair: false,
                force_full: true,
            },
        )
        .unwrap();
        assert!(specs.is_empty(), "{specs:?}");
    }
}

/// The overview's light read skips only the scan-bound row walk.
#[test]
fn skipping_scan_bounds_changes_only_the_warnings() {
    let conn = book();
    wallet(&conn, "csv", "custom", r#"{"source_format":"csv"}"#);
    transaction(&conn, "csv", "t1", "2025-03-01T00:00:00Z");
    wallet(
        &conn,
        "sp",
        "silent-payment",
        r#"{"sp_descriptor":"d","sp_scan_start_date":"2024-05-01","last_synced_at":"2026-07-15T08:00:00Z"}"#,
    );
    wallet(&conn, "kraken", "kraken", r#"{"backend":"kraken"}"#);
    let full = sources(&conn, "p", &Options::default(), NOW).unwrap();
    let light = sources(
        &conn,
        "p",
        &Options {
            skip_scan_bounds: true,
            ..Options::default()
        },
        NOW,
    )
    .unwrap();
    assert_eq!(full.len(), light.len());
    for (full, light) in full.into_iter().zip(light) {
        assert!(light.scan_warnings.is_empty());
        assert_eq!(
            Source {
                scan_warnings: vec![],
                ..full
            },
            light
        );
    }
}

/// Second review, finding 1: a re-import that only re-observed rows keeps
/// no coverage once the rows it re-observed are rolled back.
#[test]
fn a_rollback_takes_the_coverage_of_re_imports_with_it() {
    let conn = book();
    wallet(&conn, "csv", "custom", r#"{"source_format":"csv"}"#);
    let csv = TempFile::new(
        "rolled.csv",
        "date,txid,direction,asset,amount,fee\n2025-02-01,a,in,BTC,1,0\n2025-12-31,b,in,BTC,1,0\n",
    );
    import(&conn, "csv", &csv);
    import(&conn, "csv", &csv);
    assert_eq!(
        source(&conn, "csv", Some("2025-12-31")).freshness_state,
        "current"
    );
    // What `imports rollback` removes: the original run and its rows.
    conn.execute_batch(
        "DELETE FROM transactions WHERE id IN (SELECT transaction_id FROM import_batch_transactions);
         DELETE FROM import_batches WHERE rows_inserted > 0;",
    )
    .unwrap();
    let after = source(&conn, "csv", Some("2025-12-31"));
    assert_eq!(after.transaction_count, 0);
    assert_eq!(after.covered_through, None);
    assert_ne!(after.freshness_state, "current");
    assert_eq!(after.freshness_state, "never_imported");
}

/// Second review, finding 1: a book reset removes every row a run read, so
/// no run keeps coverage, even before the reset clears the ranges.
#[test]
fn a_reset_book_keeps_no_file_coverage() {
    let conn = book();
    wallet(&conn, "csv", "custom", r#"{"source_format":"csv"}"#);
    let csv = TempFile::new(
        "reset.csv",
        "date,txid,direction,asset,amount,fee\n2025-02-01,a,in,BTC,1,0\n2025-12-31,b,in,BTC,1,0\n",
    );
    import(&conn, "csv", &csv);
    // What "Reset book data" removes.
    conn.execute_batch("DELETE FROM transactions WHERE profile_id='p'")
        .unwrap();
    let reset = source(&conn, "csv", Some("2025-12-31"));
    assert_eq!(
        (reset.freshness_state, reset.covered_through),
        ("never_imported", None)
    );
}

/// Second review, finding 1: rows a range rests on bound it; a smaller
/// re-import after a reset covers only what it held.
#[test]
fn surviving_rows_bound_an_older_range() {
    let conn = book();
    wallet(&conn, "csv", "custom", r#"{"source_format":"csv"}"#);
    batch(
        &conn,
        "csv",
        "old",
        "2025-04-01T00:00:00Z",
        Some(("2025-01-01T00:00:00Z", "2025-12-31T00:00:00Z")),
    );
    conn.execute_batch("DELETE FROM transactions WHERE id='old-last'")
        .unwrap();
    transaction(&conn, "csv", "kept", "2025-03-01T00:00:00Z");
    member(&conn, "old", "kept");
    let bounded = source(&conn, "csv", Some("2025-12-31"));
    assert_eq!(
        bounded.covered_through.as_deref(),
        Some("2025-03-01T00:00:00Z")
    );
    assert_eq!(bounded.freshness_state, "behind");
}

/// Second review, finding 4: the metadata signal is only advisory; an
/// explicit check hashes the remembered file and compares it with the last
/// import. Display reads never run it.
#[test]
fn an_explicit_check_compares_the_file_contents() {
    let conn = book();
    let file = TempFile::new(
        "checked.csv",
        "date,txid,direction,asset,amount,fee\n2025-02-01,a,in,BTC,1,0\n",
    );
    wallet(
        &conn,
        "csv",
        "custom",
        &serde_json::json!({"source_format": "csv", "source_file": file.path()}).to_string(),
    );
    import(&conn, "csv", &file);
    let check = |conn: &Connection| {
        sources(
            conn,
            "p",
            &Options {
                wallet: Some("csv"),
                check_file: true,
                ..Options::default()
            },
            NOW,
        )
        .unwrap()
        .remove(0)
        .source_file_check
    };
    assert_eq!(source(&conn, "csv", None).source_file_check, None);
    assert_eq!(check(&conn), Some("same"));
    std::fs::write(
        &file.0,
        "date,txid,direction,asset,amount,fee\n2025-02-01,a,in,BTC,1,0\n2025-03-01,b,in,BTC,1,0\n",
    )
    .unwrap();
    assert_eq!(check(&conn), Some("different"));
    std::fs::remove_file(&file.0).unwrap();
    assert_eq!(check(&conn), Some("unknown"));
    // Only for one named source.
    let all = sources(
        &conn,
        "p",
        &Options {
            check_file: true,
            ..Options::default()
        },
        NOW,
    )
    .unwrap();
    assert_eq!(all[0].source_file_check, None);
}

/// Third review, finding 1 (the reviewer's repro): a re-import of the same
/// rows in a different file (another hash) only matched rows the first run
/// created. Rolling the first run back removes them; a manual December row
/// must not keep the second run's range alive. Rows from other sources never
/// support a file range.
#[test]
fn rows_from_other_sources_never_support_a_file_range() {
    let conn = book();
    wallet(&conn, "csv", "custom", r#"{"source_format":"csv"}"#);
    let a = TempFile::new(
        "first.csv",
        "date,txid,direction,asset,amount,fee\n2025-02-01,a,in,BTC,1,0\n2025-12-31,b,in,BTC,1,0\n",
    );
    // Same rows, other bytes: another hash, every row matched.
    let b = TempFile::new(
        "second.csv",
        "date,txid,direction,asset,amount,fee\n2025-02-01,a,in,BTC,1,0\n2025-12-31,b,in,BTC,1,0\n\n",
    );
    import(&conn, "csv", &a);
    import(&conn, "csv", &b);
    let hashes: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT file_sha256) FROM import_batches",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hashes, 2);
    assert_eq!(
        source(&conn, "csv", Some("2025-12-31")).freshness_state,
        "current"
    );
    // What `imports rollback` removes for the first run: its rows and the
    // run. The second run has another hash, so its range stays stored.
    conn.execute_batch(
        "DELETE FROM transactions WHERE id IN (SELECT transaction_id FROM import_batch_transactions);
         DELETE FROM import_batches WHERE rows_inserted > 0;",
    )
    .unwrap();
    // A row the user entered by hand in December.
    transaction(&conn, "csv", "manual-dec", "2025-12-20T00:00:00Z");
    let after = source(&conn, "csv", Some("2025-12-31"));
    assert_eq!(after.covered_through, None);
    assert_ne!(after.freshness_state, "current");
}

/// Third review, finding 3: the check compares with the newest run only.
/// A newest run without a hash (too large to fingerprint) leaves it unknown
/// even when an older run's hash matches the file.
#[test]
fn the_check_compares_with_the_newest_run_only() {
    let conn = book();
    let file = TempFile::new(
        "newest.csv",
        "date,txid,direction,asset,amount,fee\n2025-02-01,a,in,BTC,1,0\n",
    );
    wallet(
        &conn,
        "csv",
        "custom",
        &serde_json::json!({"source_format": "csv", "source_file": file.path()}).to_string(),
    );
    import(&conn, "csv", &file);
    import(&conn, "csv", &file);
    // The newest run recorded no hash (as for a file over the limit).
    conn.execute_batch(
        "UPDATE import_batches SET file_sha256 = NULL
          WHERE import_sequence = (SELECT MAX(import_sequence) FROM import_batches)",
    )
    .unwrap();
    let check = sources(
        &conn,
        "p",
        &Options {
            wallet: Some("csv"),
            check_file: true,
            ..Options::default()
        },
        NOW,
    )
    .unwrap()
    .remove(0)
    .source_file_check;
    assert_eq!(check, Some("unknown"));
}

/// Third review, finding 4: the payload says whether the wallet remembers
/// its file (never the path), so the check is offered for every such source.
#[test]
fn the_payload_says_whether_a_file_is_remembered() {
    let conn = book();
    let file = TempFile::new("remembered.csv", "date,txid,direction,asset,amount,fee\n");
    wallet(
        &conn,
        "kept",
        "custom",
        &serde_json::json!({"source_format": "csv", "source_file": file.path()}).to_string(),
    );
    wallet(&conn, "plain", "custom", r#"{"source_format":"csv"}"#);
    let kept = source(&conn, "kept", None);
    assert_eq!(
        field(&kept, &["file", "source_file_remembered"]),
        Value::Bool(true)
    );
    assert!(
        !kassiber_pyjson::dumps(&kept.to_value(), &Default::default())
            .unwrap()
            .contains(file.path())
    );
    // Absent, not false, so payloads of other sources keep their bytes.
    let plain = source(&conn, "plain", None).to_value();
    assert_eq!(
        kassiber_envelope::get(
            kassiber_envelope::get(&plain, "file").unwrap(),
            "source_file_remembered"
        ),
        None
    );
}

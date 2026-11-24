//! Source coverage kinds and the exchange attach path through the real
//! daemon (docs/reference/desktop.md#sources-live-sync-and-file-snapshots).

#![cfg(feature = "test-harness")]

mod support;

use kassiber_envelope::get;
use kassiber_pyjson::Value;
use support::{kind_of, path, text, Desk};

fn data(record: &Value) -> &Value {
    assert_ne!(kind_of(record), "error", "{record:?}");
    get(record, "data").expect("data")
}

fn source<'a>(report: &'a Value, wallet: &str) -> &'a Value {
    let Some(Value::List(sources)) = get(report, "sources") else {
        panic!("sources: {report:?}")
    };
    sources
        .iter()
        .find(|source| path(source, &["wallet_id"]).and_then(text) == Some(wallet))
        .unwrap_or_else(|| panic!("{wallet} in {report:?}"))
}

#[test]
fn coverage_reads_are_period_relative_and_refuse_unknown_arguments() {
    let desk = Desk::new("desktop_sources_unrecorded");
    let today = desk.ask("ui.sources.coverage", "{}");
    let report = data(&today);
    // The seeded address wallet was never synced.
    let cold = source(report, "cold");
    assert_eq!(path(cold, &["source_mode"]).and_then(text), Some("live"));
    assert_eq!(
        path(cold, &["freshness_state"]).and_then(text),
        Some("never_imported")
    );
    assert_eq!(get(report, "period_end_default"), Some(&Value::Bool(true)));

    let past = desk.ask(
        "ui.sources.coverage",
        r#"{"period_end": "2024-12-31", "wallet": "Exchange"}"#,
    );
    let report = data(&past);
    assert_eq!(
        path(report, &["period_end"]).and_then(text),
        Some("2024-12-31")
    );
    let Some(Value::List(sources)) = get(report, "sources") else {
        panic!()
    };
    assert_eq!(sources.len(), 1);

    let refused = desk.ask("ui.sources.coverage", r#"{"bogus": 1}"#);
    assert_eq!(kind_of(&refused), "error");
    assert_eq!(
        path(&refused, &["error", "code"]).and_then(text),
        Some("validation")
    );
    let bad_day = desk.ask("ui.sources.coverage", r#"{"period_end": "last year"}"#);
    assert_eq!(kind_of(&bad_day), "error");
}

#[test]
fn an_attestation_clears_the_warning_and_nothing_else() {
    let desk = Desk::new("desktop_sources_unrecorded");
    // A CSV run covered the exchange wallet through 30 June 2024; the user
    // states it was closed on that day, which closes the gap after it. The
    // rows that file held are in the book, created by that run: its range
    // rests on them.
    desk.change(
        "INSERT INTO import_batches(id,workspace_id,profile_id,wallet_id,source_format,imported_at,rows_inserted,first_row_at,last_row_at,rows_read)
         VALUES('csv-run','w','p','exchange','csv','2024-07-01T00:00:00Z',1,'2024-01-01T00:00:00Z','2024-06-30T12:00:00Z',2);
         INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at)
         VALUES('csv-first','w','p','exchange','csv-first','csv-first','2024-01-01T00:00:00Z','inbound','BTC',100000000,0,'2024-07-01T00:00:00Z'),
               ('csv-last','w','p','exchange','csv-last','csv-last','2024-06-30T12:00:00Z','inbound','BTC',100000000,0,'2024-07-01T00:00:00Z');
         INSERT INTO import_batch_transactions(batch_id,transaction_id) VALUES('csv-run','csv-first'),('csv-run','csv-last');",
    );
    let before = desk.connection().query_row(
        "SELECT COUNT(*), (SELECT last_processed_input_version FROM profiles WHERE id='p') FROM transactions",
        [],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)),
    );
    let set = desk.ask(
        "ui.sources.attest_coverage",
        r#"{"wallet": "Exchange", "kind": "closed_on", "date": "2024-06-30", "note": "account closed"}"#,
    );
    let result = data(&set);
    assert_eq!(
        path(result, &["source", "freshness_state"]).and_then(text),
        Some("current")
    );
    assert_eq!(
        path(result, &["source", "attestation", "kind"]).and_then(text),
        Some("closed_on")
    );
    let after = desk.connection().query_row(
        "SELECT COUNT(*), (SELECT last_processed_input_version FROM profiles WHERE id='p') FROM transactions",
        [],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)),
    );
    assert_eq!(before.unwrap(), after.unwrap());

    let future = desk.ask(
        "ui.sources.attest_coverage",
        r#"{"wallet": "Exchange", "kind": "closed_on", "date": "9999-01-01"}"#,
    );
    assert_eq!(kind_of(&future), "error");
    let both = desk.ask(
        "ui.sources.attest_coverage",
        r#"{"wallet": "Exchange", "kind": "closed_on", "date": "2024-06-30", "clear": true}"#,
    );
    assert_eq!(kind_of(&both), "error");
    let cleared = desk.ask(
        "ui.sources.attest_coverage",
        r#"{"wallet": "Exchange", "clear": true}"#,
    );
    assert_eq!(
        path(data(&cleared), &["source", "attestation"]),
        Some(&Value::Null)
    );
}

#[test]
fn the_overview_status_follows_coverage() {
    let desk = Desk::new("desktop_sources_unrecorded");
    desk.change(
        "UPDATE wallets SET config_json='{\"network\":\"regtest\",\"addresses\":[\"x\"],\"last_synced_at\":\"2099-01-01T00:00:00Z\"}' WHERE id='cold';",
    );
    let overview = desk.ask("ui.overview.snapshot", "{}");
    let Some(Value::List(connections)) = get(data(&overview), "connections") else {
        panic!()
    };
    let status = |id: &str| {
        let connection = connections
            .iter()
            .find(|c| path(c, &["id"]).and_then(text) == Some(id))
            .unwrap();
        (
            path(connection, &["status"])
                .and_then(text)
                .unwrap()
                .to_owned(),
            path(connection, &["coverage", "freshness_state"])
                .and_then(text)
                .unwrap()
                .to_owned(),
        )
    };
    assert_eq!(status("cold"), ("synced".into(), "current".into()));
    // A file-style exchange wallet with rows that end long ago is behind,
    // not "synced" just because it holds transactions.
    let (exchange_status, exchange_state) = status("exchange");
    assert_ne!(exchange_status, "synced", "{exchange_state}");
}

#[test]
fn an_exchange_key_attaches_to_the_csv_wallet_of_the_same_exchange() {
    let desk = Desk::new("desktop_sources_unrecorded");
    // The wallet holds a CSV import run, as a desktop file import leaves it.
    desk.change(
        "INSERT INTO import_batches(id,workspace_id,profile_id,wallet_id,source_format,source_filename,imported_at,rows_inserted,first_row_at,last_row_at,rows_read)
         VALUES('csv-run','w','p','exchange','generic_ledger','kraken.csv','2025-03-01T00:00:00Z',1,'2024-01-01T00:00:00Z','2024-12-31T00:00:00Z',1);",
    );
    let conflict = desk.ask(
        "ui.connections.exchange.create",
        r#"{"exchange": "kraken", "label": "Exchange", "api_key": "synthetic-key", "api_secret": "c3ludGhldGlj"}"#,
    );
    assert_eq!(kind_of(&conflict), "error");
    assert_eq!(
        path(&conflict, &["error", "details", "attachable_wallet_id"]).and_then(text),
        Some("exchange")
    );
    // Another exchange's wallet cannot take the key.
    let wrong = desk.ask(
        "ui.connections.exchange.create",
        r#"{"exchange": "coinbase", "wallet": "exchange", "api_key": "synthetic-key", "api_secret": "c3ludGhldGlj"}"#,
    );
    assert_eq!(kind_of(&wrong), "error");
    let attached = desk.ask(
        "ui.connections.exchange.create",
        r#"{"exchange": "kraken", "wallet": "exchange", "api_key": "synthetic-key", "api_secret": "c3ludGhldGlj"}"#,
    );
    let result = data(&attached);
    assert_eq!(get(result, "attached"), Some(&Value::Bool(true)));
    assert_eq!(
        path(result, &["wallet", "id"]).and_then(text),
        Some("exchange")
    );
    let backend: String = desk
        .connection()
        .query_row(
            "SELECT json_extract(config_json,'$.backend') FROM wallets WHERE id='exchange'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(backend, "kraken");
    let again = desk.ask(
        "ui.connections.exchange.create",
        r#"{"exchange": "kraken", "wallet": "exchange", "api_key": "synthetic-key", "api_secret": "c3ludGhldGlj"}"#,
    );
    assert_eq!(
        path(&again, &["error", "code"]).and_then(text),
        Some("conflict")
    );
    // No sync has run: the persisted status says so, after a fresh daemon.
    let status = desk.ask(
        "ui.connections.exchange.status",
        r#"{"wallet": "exchange"}"#,
    );
    let Some(Value::List(rows)) = get(data(&status), "connections") else {
        panic!()
    };
    assert_eq!(get(&rows[0], "last_sync"), Some(&Value::Null));
    let coverage = desk.ask("ui.sources.coverage", r#"{"wallet": "exchange"}"#);
    assert_eq!(
        path(source(data(&coverage), "exchange"), &["source_mode"]).and_then(text),
        Some("hybrid")
    );
}

#[test]
fn the_cli_reads_and_attests_like_the_daemon() {
    let desk = Desk::new("desktop_sources_unrecorded");
    let output = desk.run(
        &[
            "--machine",
            "wallets",
            "coverage",
            "--period-end",
            "2024-12-31",
        ],
        b"",
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let envelope = kassiber_pytext::loads(&stdout).unwrap();
    assert_eq!(kind_of(&envelope), "wallets.coverage", "{stdout}");
    assert_eq!(
        path(&envelope, &["data", "period_end"]).and_then(text),
        Some("2024-12-31")
    );
    let output = desk.run(
        &[
            "--machine",
            "wallets",
            "attest-coverage",
            "--wallet",
            "Exchange",
            "--kind",
            "no_activity_after",
            "--date",
            "2024-12-31",
        ],
        b"",
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let envelope = kassiber_pytext::loads(&stdout).unwrap();
    assert_eq!(
        path(&envelope, &["data", "source", "attestation", "kind"]).and_then(text),
        Some("no_activity_after"),
        "{stdout}"
    );
    let output = desk.run(
        &[
            "--machine",
            "wallets",
            "attest-coverage",
            "--wallet",
            "Exchange",
        ],
        b"",
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("needs a kind and a date"), "{stdout}");
}

/// Review finding 6: a CLI sync that names no `--wallet` records its outcome
/// on the provider wallet it ran for. Egress is off in tests, so the sync
/// fails before any request, and only the error's class is stored.
#[test]
fn a_cli_sync_without_a_wallet_records_its_outcome() {
    let desk = Desk::new("desktop_sources_unrecorded");
    // The default sync target is the wallet labelled after the exchange.
    desk.change("UPDATE wallets SET label='Kraken' WHERE id='exchange';");
    let created = desk.ask(
        "ui.connections.exchange.create",
        r#"{"exchange": "kraken", "wallet": "exchange", "api_key": "synthetic-key", "api_secret": "c3ludGhldGlj"}"#,
    );
    let wallet = path(data(&created), &["wallet", "id"])
        .and_then(text)
        .unwrap()
        .to_owned();
    let output = desk.run(
        &["--machine", "wallets", "sync-kraken", "--backend", "kraken"],
        b"",
    );
    assert!(!output.status.success());
    let status = desk.ask(
        "ui.connections.exchange.status",
        &format!(r#"{{"wallet": "{wallet}"}}"#),
    );
    let Some(Value::List(rows)) = get(data(&status), "connections") else {
        panic!()
    };
    let last = get(&rows[0], "last_sync").cloned().unwrap_or(Value::Null);
    assert_eq!(
        path(&last, &["status"]).and_then(text),
        Some("error"),
        "{last:?}"
    );
    assert_eq!(
        path(&last, &["error", "class"]).and_then(text),
        Some("offline"),
        "{last:?}"
    );
    let stored: String = desk
        .connection()
        .query_row(
            "SELECT config_json FROM wallets WHERE id=?",
            [&wallet],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!stored.contains("message"), "{stored}");
}

/// Review finding 1: the title-bar refresh names its book on every exchange
/// request; the daemon refuses one meant for another book before any work.
#[test]
fn exchange_requests_for_another_book_are_refused() {
    let desk = Desk::new("desktop_sources_unrecorded");
    let same = desk.ask(
        "ui.connections.exchange.status",
        r#"{"expected_scope": {"workspace_id": "w", "profile_id": "p"}}"#,
    );
    assert_ne!(kind_of(&same), "error", "{same:?}");
    for kind in [
        "ui.connections.exchange.status",
        "ui.connections.exchange.sync",
    ] {
        let other = desk.ask(
            kind,
            r#"{"wallet": "exchange", "expected_scope": {"workspace_id": "w", "profile_id": "foreign"}}"#,
        );
        assert_eq!(
            path(&other, &["error", "code"]).and_then(text),
            Some("stale_context"),
            "{kind}: {other:?}"
        );
    }
}

//! The journal reproducibility check against frozen processed books.
use kassiber_journals::reproduce;
use kassiber_journals::service::ReplayGuard;
use kassiber_store::{rusqlite::Connection, Book, OpenOptions};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// A fresh book holding a frozen book's rows (the black-box tests' books).
fn book(name: &str) -> (PathBuf, Book) {
    let root = std::env::temp_dir().join(format!(
        "kassiber-reproduce-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    drop(Book::create(&root, None).unwrap());
    let rows = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kassiber/tests/data/books")
            .join(format!("{name}.sql")),
    )
    .unwrap();
    {
        let book = Book::open_migrating(&root, OpenOptions::default(), true).unwrap();
        let conn = book.connection();
        conn.set_db_config(
            kassiber_store::rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
            false,
        )
        .unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF; BEGIN")
            .unwrap();
        let names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_stat%'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for name in names {
            conn.execute_batch(&format!("DELETE FROM \"{name}\""))
                .unwrap();
        }
        conn.execute_batch(&rows).unwrap();
        conn.execute_batch("COMMIT").unwrap();
    }
    let book = Book::open_migrating(&root, OpenOptions::default(), true).unwrap();
    (root, book)
}

fn check(conn: &Connection, limit: usize) -> Value {
    reproduce::check(conn, None, None, limit, &kassiber_transfers::SystemSources).unwrap()
}

/// Process the book for real, so its stored journals are this build's.
fn process(conn: &Connection) {
    kassiber_journals::service::process(conn, None, None, &mut |_| {})
        .unwrap()
        .after_reply()
        .unwrap();
}

#[test]
fn a_freshly_processed_book_reproduces_without_writing_to_the_book() {
    let (root, book) = book("plain-unchanged");
    let conn = book.connection();
    process(conn);
    let guard = ReplayGuard::capture(conn);
    let changes = conn.total_changes();
    let report = check(conn, 20);
    assert_eq!(report["status"], "identical", "{report:#}");
    assert_eq!(report["differing"], 0);
    assert!(report["tables"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["table"] == "journal_entries" && t["stored"].as_u64() > Some(0)));
    assert_eq!(conn.total_changes(), changes);
    assert!(guard.unchanged(conn), "the check wrote to the book");
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_tampered_stored_entry_is_named_with_its_columns_wallet_and_year() {
    let (root, book) = book("plain-unchanged");
    let conn = book.connection();
    process(conn);
    let (id, transaction, wallet, year): (String, String, String, String) = conn
        .query_row(
            "SELECT id,transaction_id,wallet_id,substr(occurred_at,1,4) FROM journal_entries ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    conn.execute(
        "UPDATE journal_entries SET fiat_value_exact='12345.67' WHERE id=?",
        [&id],
    )
    .unwrap();
    conn.execute("DELETE FROM journal_wallet_holdings", [])
        .unwrap();
    let report = check(conn, 1);
    assert_eq!(report["status"], "different", "{report:#}");
    assert_eq!(report["deterministic"], true);
    assert!(report["differing"].as_u64().unwrap() >= 2);
    assert_eq!(report["truncated"], true);
    assert_eq!(report["differences"].as_array().unwrap().len(), 1);
    let first = &report["differences"][0];
    assert_eq!(first["table"], "journal_entries");
    assert_eq!(first["change"], "changed");
    assert_eq!(first["key"], id.as_str());
    assert_eq!(first["transaction_id"], transaction.as_str());
    assert_eq!(first["wallet_id"], wallet.as_str());
    assert_eq!(first["year"], year.as_str());
    assert_eq!(first["columns"][0]["column"], "fiat_value_exact");
    assert_eq!(first["columns"][0]["stored"], "12345.67");
    assert_eq!(
        report["by_wallet"][&wallet],
        json!(report["differing"].as_u64().unwrap())
    );
    let holdings = report["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["table"] == "journal_wallet_holdings")
        .unwrap();
    assert_eq!(holdings["stored"], 0);
    assert!(holdings["differing"].as_u64().unwrap() > 0);
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_inputs_report_stale_instead_of_a_diff() {
    let (root, book) = book("plain-unchanged");
    let conn = book.connection();
    process(conn);
    conn.execute_batch("UPDATE profiles SET journal_input_version=journal_input_version+1")
        .unwrap();
    let report = check(conn, 20);
    assert_eq!(report["status"], "stale", "{report:#}");
    assert_eq!(
        report["reason"],
        "journal inputs changed since last processing"
    );
    assert!(report.get("differences").is_none());
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
}

/// The frozen books' journals were written by the Python reference; the
/// native pipeline reproduces every compared row of them, custody decisions,
/// quantity postings and blocked books included.
#[test]
fn journals_the_reference_wrote_reproduce_natively() {
    for name in [
        "paired-fee-processed",
        "paired-short-processed",
        "plain-blocked-processed",
        "gaps-10-processed",
    ] {
        let (root, book) = book(name);
        let report = check(book.connection(), 3);
        assert_eq!(report["status"], "identical", "{name}: {report:#}");
        drop(book);
        std::fs::remove_dir_all(root).unwrap();
    }
}

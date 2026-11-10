#[path = "support/source_funds_cases.rs"]
mod support;

use kassiber_envelope::{get, s, set};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::Connection, Book};
use support::{dump, encode, outcome, text, value};

fn fixture() -> Value {
    kassiber_pytext::loads(include_str!("data/source_funds_cases.json")).unwrap()
}

fn seed(conn: &Connection, fixture: &Value) {
    conn.execute_batch("BEGIN").unwrap();
    conn.execute_batch(text(fixture, "base_sql")).unwrap();
    conn.execute_batch("COMMIT").unwrap();
}

const MASKED: &str = "no such savepoint: source_funds_case_snapshot";

#[test]
fn source_funds_case_snapshot_pins_json_hash_receipt_and_all_112_tables() {
    let fixture = fixture();
    let Value::List(cases) = value(&fixture, "cases") else {
        panic!()
    };
    assert_eq!(cases.len(), 37);
    let scratch = tempfile::tempdir().unwrap();
    let template = Book::create(&scratch.path().join("template"), None).unwrap();
    seed(template.connection(), &fixture);
    for case in cases {
        let mut conn = Connection::open_in_memory().unwrap();
        kassiber_store::rusqlite::backup::Backup::new(template.connection(), &mut conn)
            .unwrap()
            .run_to_completion(256, std::time::Duration::ZERO, None)
            .unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        let conn = &conn;
        conn.execute_batch(text(case, "sql")).unwrap();
        let mut expected_tables = dump(conn);
        let Value::Dict(written) = value(case, "tables") else {
            panic!()
        };
        for (key, rows) in written {
            let kassiber_pyjson::Key::Str(key) = key else {
                panic!()
            };
            set(&mut expected_tables, key.as_str().unwrap(), rows.clone());
        }
        // A trigger's RAISE(ROLLBACK) ends the transaction, so Python's
        // savepoint cleanup fails and masks the refusal
        // (tests/rust_port/repro_source_funds_save_rollback_masks_refusal.py);
        // Rust reports the refusal itself. The book is unchanged either way.
        let mut wanted = encode(value(case, "expected"));
        if text(case, "name").ends_with("-ROLLBACK") {
            assert!(wanted.contains(MASKED), "{}", text(case, "name"));
            wanted = wanted.replace(MASKED, "synthetic late refusal");
        }
        assert_eq!(
            encode(&outcome(conn, case)),
            wanted,
            "{}",
            text(case, "name")
        );
        assert_eq!(
            encode(&dump(conn)),
            encode(&expected_tables),
            "{}",
            text(case, "name")
        );
        assert!(
            conn.is_autocommit(),
            "{} leaked its transaction",
            text(case, "name")
        );
    }
}

#[test]
fn source_funds_case_snapshot_preserves_outer_transaction_and_late_rollback() {
    let fixture = fixture();
    let Value::List(cases) = value(&fixture, "cases") else {
        panic!()
    };
    let scratch = tempfile::tempdir().unwrap();
    for encrypted in [false, true] {
        for name in ["frozen-report", "snapshot-id-collision"] {
            let case = cases
                .iter()
                .find(|case| text(case, "name") == name)
                .unwrap();
            let root = scratch.path().join(format!("{encrypted}-{name}"));
            let book =
                Book::create(&root, encrypted.then_some("synthetic-case-passphrase")).unwrap();
            let conn = book.connection();
            seed(conn, &fixture);
            conn.execute_batch(text(case, "sql")).unwrap();
            let committed = dump(conn);
            conn.execute_batch("BEGIN IMMEDIATE; INSERT INTO settings(key,value) VALUES('outer-pending','retained')").unwrap();
            let pending = dump(conn);
            let result = outcome(conn, case);
            assert_eq!(encode(&result), encode(value(case, "expected")));
            assert!(!conn.is_autocommit());
            assert_eq!(
                conn.query_row(
                    "SELECT value FROM settings WHERE key='outer-pending'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
                "retained"
            );
            if get(&result, "error").is_some() {
                assert_eq!(encode(&dump(conn)), encode(&pending));
            } else {
                assert_eq!(
                    conn.query_row(
                        "SELECT count(*) FROM source_funds_cases WHERE id='new-case'",
                        [],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                    1
                );
            }
            conn.execute_batch("ROLLBACK").unwrap();
            assert_eq!(encode(&dump(conn)), encode(&committed));
        }
    }
}

#[test]
fn source_funds_case_snapshot_rolls_back_commit_busy_on_both_stores() {
    let fixture = fixture();
    let Value::List(cases) = value(&fixture, "cases") else {
        panic!()
    };
    let scratch = tempfile::tempdir().unwrap();
    for encrypted in [false, true] {
        let root = scratch.path().join(encrypted.to_string());
        let passphrase = encrypted.then_some("synthetic-case-passphrase");
        let book = Book::create(&root, passphrase).unwrap();
        let conn = book.connection();
        seed(conn, &fixture);
        conn.pragma_update(None, "journal_mode", "DELETE").unwrap();
        conn.busy_timeout(std::time::Duration::ZERO).unwrap();
        let database: String = conn
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let observer = Connection::open(database).unwrap();
        if let Some(passphrase) = passphrase {
            observer.pragma_update(None, "key", passphrase).unwrap();
        }
        observer
            .execute_batch("BEGIN;SELECT * FROM source_funds_cases")
            .unwrap();
        let before = dump(conn);
        let fixed = support::Fixed::new(&cases[0]);
        let error = support::save(conn, value(&cases[0], "args"), &fixed).unwrap_err();
        assert_eq!(error.code, "database_busy");
        assert!(error.retryable);
        assert_eq!(
            fixed.calls.into_inner(),
            vec![s("uuid"), s("uuid"), s("now")]
        );
        assert!(conn.is_autocommit());
        assert_eq!(encode(&dump(conn)), encode(&before));
        observer.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            encode(&outcome(conn, &cases[0])),
            encode(value(&cases[0], "expected"))
        );
    }
}

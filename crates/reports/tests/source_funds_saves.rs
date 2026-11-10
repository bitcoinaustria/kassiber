#[path = "support/source_funds_mutations.rs"]
mod support;

use kassiber_envelope::{get, s, set, AppError};
use kassiber_pyjson::Value;
use kassiber_setup::Sources;
use kassiber_store::{rusqlite::Connection, Book};
use support::{dump, encode, outcome, text, value};

fn fixture() -> Value {
    kassiber_pytext::loads(include_str!("data/source_funds_saves.json")).unwrap()
}
fn seed(conn: &Connection, fixture: &Value) {
    conn.execute_batch(text(fixture, "base_sql")).unwrap();
}
fn named(fixture: &Value, name: &str) -> Value {
    let Value::List(cases) = value(fixture, "cases") else {
        panic!()
    };
    cases
        .iter()
        .find(|case| text(case, "name") == name)
        .unwrap()
        .clone()
}

/// Python's cleanup error after a refusal that ended the transaction.
const MASKED: &str = "no such savepoint: source_funds_case_snapshot";

#[test]
fn source_funds_saves_pin_reports_diagrams_fingerprints_receipts_and_all_112_tables() {
    let fixture = fixture();
    let Value::List(cases) = value(&fixture, "cases") else {
        panic!()
    };
    assert!(cases.len() >= 70);
    let root = tempfile::tempdir().unwrap();
    let template = Book::create(&root.path().join("template"), None).unwrap();
    seed(template.connection(), &fixture);
    for case in cases {
        let mut conn = Connection::open_in_memory().unwrap();
        kassiber_store::rusqlite::backup::Backup::new(template.connection(), &mut conn)
            .unwrap()
            .run_to_completion(256, std::time::Duration::ZERO, None)
            .unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        conn.execute_batch(text(case, "sql")).unwrap();
        let mut expected = dump(&conn);
        let Value::Dict(written) = value(case, "tables") else {
            panic!()
        };
        for (key, rows) in written {
            let kassiber_pyjson::Key::Str(key) = key else {
                panic!()
            };
            set(&mut expected, key.as_str().unwrap(), rows.clone());
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
            encode(&outcome(&conn, case)),
            wanted,
            "{}",
            text(case, "name")
        );
        assert_eq!(
            encode(&dump(&conn)),
            encode(&expected),
            "{}",
            text(case, "name")
        );
        assert!(
            conn.is_autocommit(),
            "{} leaked a transaction",
            text(case, "name")
        );
    }
}

#[test]
fn source_funds_reviewed_saves_refuse_outer_transactions_and_keep_their_work() {
    let fixture = fixture();
    let case = named(&fixture, "reviewed-current");
    let root = tempfile::tempdir().unwrap();
    for encrypted in [false, true] {
        let book = Book::create(
            &root.path().join(encrypted.to_string()),
            encrypted.then_some("synthetic-saves-passphrase"),
        )
        .unwrap();
        let conn = book.connection();
        seed(conn, &fixture);
        conn.execute_batch(text(&case, "sql")).unwrap();
        conn.execute_batch(
            "BEGIN;INSERT INTO settings(key,value) VALUES('outer-pending','retained')",
        )
        .unwrap();
        let before = dump(conn);
        let result = outcome(conn, &case);
        assert_eq!(
            text(value(&result, "error"), "message"),
            "A reviewed source-funds save requires an idle database transaction"
        );
        assert_eq!(encode(&dump(conn)), encode(&before));
        assert!(!conn.is_autocommit());
        conn.execute_batch("ROLLBACK").unwrap();
    }
}

struct Locked<'a> {
    fixed: support::Fixed<'a>,
    other: &'a Connection,
}
impl Sources for Locked<'_> {
    fn uuid(&self, conn: &Connection) -> Result<String, AppError> {
        let error = self
            .other
            .execute(
                "UPDATE attachments SET sha256='racing edit' WHERE id='a-a'",
                [],
            )
            .unwrap_err();
        assert!(matches!(
            error.sqlite_error_code(),
            Some(kassiber_store::rusqlite::ErrorCode::DatabaseBusy)
        ));
        self.fixed.uuid(conn)
    }
    fn now(&self, conn: &Connection) -> Result<String, AppError> {
        self.fixed.now(conn)
    }
}

#[test]
fn source_funds_saves_hold_writer_admission_through_report_freezing_and_late_rollback() {
    let fixture = fixture();
    let root = tempfile::tempdir().unwrap();
    for encrypted in [false, true] {
        for name in ["ready", "reviewed-current"] {
            let case = named(&fixture, name);
            let book = Book::create(
                &root.path().join(format!("{encrypted}-{name}")),
                encrypted.then_some("synthetic-saves-passphrase"),
            )
            .unwrap();
            let conn = book.connection();
            seed(conn, &fixture);
            conn.execute_batch(text(&case, "sql")).unwrap();
            conn.execute_batch("PRAGMA journal_mode=DELETE").unwrap();
            conn.busy_timeout(std::time::Duration::ZERO).unwrap();
            let path: String = conn
                .query_row(
                    "SELECT file FROM pragma_database_list WHERE name='main'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let other = Connection::open(path).unwrap();
            if encrypted {
                other
                    .pragma_update(None, "key", "synthetic-saves-passphrase")
                    .unwrap();
            }
            other.busy_timeout(std::time::Duration::ZERO).unwrap();
            let sources = Locked {
                fixed: support::Fixed::new(&case),
                other: &other,
            };
            let result = kassiber_reports::execute_with_sources(
                conn,
                text(&case, "command"),
                value(&case, "args"),
                &sources,
            )
            .unwrap();
            assert!(get(&result, "case").is_some());
            assert_eq!(
                other
                    .query_row("SELECT sha256 FROM attachments WHERE id='a-a'", [], |r| {
                        r.get::<_, String>(0)
                    })
                    .unwrap(),
                "hash"
            );
            let mut second = case.clone();
            set(
                &mut second,
                "ids",
                Value::List(vec![s("second-case"), s("second-snapshot")]),
            );
            let before = dump(conn);
            other
                .execute_batch("BEGIN;SELECT * FROM attachments")
                .unwrap();
            let result = outcome(conn, &second);
            assert_eq!(
                text(value(&result, "error"), "message"),
                "the database is busy in another Kassiber process"
            );
            assert!(conn.is_autocommit());
            assert_eq!(encode(&dump(conn)), encode(&before));
            other.execute_batch("ROLLBACK").unwrap();
        }
    }
}

#[path = "support/source_funds_mutations.rs"]
mod support;
use kassiber_envelope::{get, set};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::Connection, Book};
use support::{dump, encode, outcome, text, value};

fn fixture() -> Value {
    kassiber_pytext::loads(include_str!("data/source_funds_batch.json")).unwrap()
}
fn seed(conn: &Connection, fixture: &Value) {
    conn.execute_batch("BEGIN").unwrap();
    conn.execute_batch(text(fixture, "base_sql")).unwrap();
    conn.execute_batch("COMMIT").unwrap();
}

#[test]
fn source_funds_batch_pins_eligibility_envelopes_and_all_112_tables() {
    let fixture = fixture();
    let Value::List(cases) = value(&fixture, "cases") else {
        panic!()
    };
    assert!(cases.len() >= 90);
    let scratch = tempfile::tempdir().unwrap();
    let template = Book::create(&scratch.path().join("template"), None).unwrap();
    seed(template.connection(), &fixture);
    for case in cases
        .iter()
        .filter(|case| !text(case, "name").ends_with("-deferred"))
    {
        let mut conn = Connection::open_in_memory().unwrap();
        kassiber_store::rusqlite::backup::Backup::new(template.connection(), &mut conn)
            .unwrap()
            .run_to_completion(256, std::time::Duration::ZERO, None)
            .unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        conn.execute_batch(text(case, "sql")).unwrap();
        let mut expected = dump(&conn);
        let Value::Dict(tables) = value(case, "tables") else {
            panic!()
        };
        for (key, rows) in tables {
            let kassiber_pyjson::Key::Str(key) = key else {
                panic!()
            };
            let Value::List(mut rows) = rows.clone() else {
                panic!()
            };
            rows.sort_by_key(encode);
            set(&mut expected, key.as_str().unwrap(), Value::List(rows));
        }
        assert_eq!(
            encode(&outcome(&conn, case)),
            encode(value(case, "expected")),
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
            "{} leaked transaction",
            text(case, "name")
        );
    }
}

#[test]
fn source_funds_batch_preserves_outer_work_and_rolls_back_partial_writes() {
    let fixture = fixture();
    let Value::List(cases) = value(&fixture, "cases") else {
        panic!()
    };
    let scratch = tempfile::tempdir().unwrap();
    for encrypted in [false, true] {
        for name in ["two-current", "two-dry-run", "partial-ABORT"] {
            let case = cases.iter().find(|c| text(c, "name") == name).unwrap();
            let book = Book::create(
                &scratch.path().join(format!("{encrypted}-{name}")),
                encrypted.then_some("synthetic-source-passphrase"),
            )
            .unwrap();
            let conn = book.connection();
            seed(conn, &fixture);
            conn.execute_batch(text(case, "sql")).unwrap();
            let committed = dump(conn);
            conn.execute_batch("BEGIN IMMEDIATE; INSERT INTO settings(key,value) VALUES('outer-pending','retained')").unwrap();
            let pending = dump(conn);
            let actual = outcome(conn, case);
            assert_eq!(encode(&actual), encode(value(case, "expected")), "{name}");
            assert!(!conn.is_autocommit());
            assert_eq!(
                conn.query_row(
                    "SELECT value FROM settings WHERE key='outer-pending'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "retained"
            );
            if get(&actual, "error").is_some() || name == "two-dry-run" {
                assert_eq!(encode(&dump(conn)), encode(&pending))
            }
            conn.execute_batch("ROLLBACK").unwrap();
            assert_eq!(encode(&dump(conn)), encode(&committed));
        }
    }
}

#[test]
fn source_funds_batch_rolls_back_busy_deferred_and_projection_failures_on_both_stores() {
    let fixture = fixture();
    let Value::List(cases) = value(&fixture, "cases") else {
        panic!()
    };
    let scratch = tempfile::tempdir().unwrap();
    for encrypted in [false, true] {
        for failure in ["busy", "deferred", "projection"] {
            let passphrase = encrypted.then_some("synthetic-source-passphrase");
            let book = Book::create(
                &scratch.path().join(format!("{encrypted}-{failure}")),
                passphrase,
            )
            .unwrap();
            let conn = book.connection();
            seed(conn, &fixture);
            let case = cases
                .iter()
                .find(|c| {
                    text(c, "name")
                        == if failure == "deferred" {
                            "commit-deferred"
                        } else {
                            "two-current"
                        }
                })
                .unwrap();
            if failure != "deferred" {
                conn.execute_batch(text(case, "sql")).unwrap();
            }
            let before = dump(conn);
            if failure == "deferred" {
                conn.execute_batch(text(case, "sql")).unwrap();
            }
            let observer = if failure == "busy" {
                conn.pragma_update(None, "journal_mode", "DELETE").unwrap();
                conn.busy_timeout(std::time::Duration::ZERO).unwrap();
                let database: String = conn
                    .query_row(
                        "SELECT file FROM pragma_database_list WHERE name='main'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                let other = Connection::open(database).unwrap();
                if let Some(passphrase) = passphrase {
                    other.pragma_update(None, "key", passphrase).unwrap()
                }
                other
                    .execute_batch("BEGIN; SELECT * FROM source_funds_links")
                    .unwrap();
                Some(other)
            } else {
                None
            };
            if failure == "projection" {
                conn.execute_batch("CREATE TRIGGER refuse_projection AFTER UPDATE ON source_funds_links WHEN NEW.id='l-second' BEGIN UPDATE source_funds_links SET allocation_amount='synthetic malformed amount' WHERE id='l-second'; END;").unwrap()
            }
            let actual = outcome(conn, case);
            assert_eq!(
                text(value(&actual, "error"), "code"),
                match failure {
                    "busy" => "database_busy",
                    // A malformed stored amount is dec()'s AppError since 0e8864e8b.
                    "projection" => "app_error",
                    _ => "internal_error",
                }
            );
            assert!(conn.is_autocommit());
            if failure == "deferred" {
                conn.execute_batch("DROP TRIGGER refuse_commit; DROP TABLE commit_failure")
                    .unwrap();
            }
            assert_eq!(encode(&dump(conn)), encode(&before));
            if let Some(observer) = observer {
                observer.execute_batch("ROLLBACK").unwrap()
            }
        }
    }
}

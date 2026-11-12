#[path = "support/source_funds_mutations.rs"]
mod support;
use kassiber_envelope::{get, set};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::Connection, Book};
use support::{dump, encode, outcome, text, value};

fn fixture() -> Value {
    kassiber_pytext::loads(include_str!("data/source_funds_links.json")).unwrap()
}
fn seed(conn: &Connection, fixture: &Value) {
    conn.execute_batch("BEGIN").unwrap();
    conn.execute_batch(text(fixture, "base_sql")).unwrap();
    conn.execute_batch("COMMIT").unwrap();
}

#[test]
fn source_funds_links_pin_validation_envelopes_and_all_112_tables() {
    let fixture = fixture();
    let Value::List(cases) = value(&fixture, "cases") else {
        panic!()
    };
    assert!(cases.len() >= 300);
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
fn source_funds_projection_failures_roll_back_all_112_tables_on_both_stores() {
    let fixture = fixture();
    let scratch = tempfile::tempdir().unwrap();
    for encrypted in [false, true] {
        for (family, table, column, identity, argument) in [
            ("sources", "source_funds_sources", "amount", "s-z", "source"),
            (
                "links",
                "source_funds_links",
                "allocation_amount",
                "l-z",
                "link",
            ),
        ] {
            let book = Book::create(
                &scratch
                    .path()
                    .join(format!("projection-{encrypted}-{family}")),
                encrypted.then_some("synthetic-source-passphrase"),
            )
            .unwrap();
            let conn = book.connection();
            seed(conn, &fixture);
            conn.execute(
                &format!("UPDATE {table} SET {column}=? WHERE id=?"),
                ["synthetic malformed amount", identity],
            )
            .unwrap();
            let before = dump(conn);
            let case = kassiber_envelope::dict([
                (
                    "command",
                    kassiber_envelope::s(format!("ui.source_funds.{family}.attach")),
                ),
                (
                    "args",
                    kassiber_envelope::dict([
                        (argument, kassiber_envelope::s(identity)),
                        ("attachment_id", kassiber_envelope::s("a-a")),
                    ]),
                ),
            ]);
            let result = outcome(conn, &case);
            // A malformed stored amount is dec()'s AppError since 0e8864e8b.
            assert_eq!(text(value(&result, "error"), "code"), "app_error");
            assert_eq!(encode(&dump(conn)), encode(&before), "{encrypted}-{family}");
            assert!(conn.is_autocommit());
        }
    }
}

#[test]
fn source_funds_links_preserve_outer_work_and_roll_back_late_failures_on_both_stores() {
    let fixture = fixture();
    let Value::List(cases) = value(&fixture, "cases") else {
        panic!()
    };
    let scratch = tempfile::tempdir().unwrap();
    for encrypted in [false, true] {
        for name in [
            "create-attachments",
            "desktop-review",
            "desktop-attach",
            "create-source_funds_links-ABORT",
            "create-source_funds_link_attachments-ABORT",
            "review-source_funds_links-ABORT",
            "attach-source_funds_link_attachments-ABORT",
            "create-attachment_ids-2",
        ] {
            let case = cases
                .iter()
                .find(|case| text(case, "name") == name)
                .unwrap();
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
            assert_eq!(
                encode(&actual),
                encode(if encrypted {
                    get(case, "expected_cipher").unwrap_or(value(case, "expected"))
                } else {
                    value(case, "expected")
                }),
                "{name}"
            );
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
            if get(&actual, "error").is_some() {
                assert_eq!(encode(&dump(conn)), encode(&pending));
            }
            conn.execute_batch("ROLLBACK").unwrap();
            assert_eq!(encode(&dump(conn)), encode(&committed));
        }
    }
}

#[test]
fn source_funds_links_roll_back_busy_and_deferred_commits_on_both_stores() {
    let fixture = fixture();
    let Value::List(cases) = value(&fixture, "cases") else {
        panic!()
    };
    let scratch = tempfile::tempdir().unwrap();
    for encrypted in [false, true] {
        let passphrase = encrypted.then_some("synthetic-source-passphrase");
        for action in ["create", "review", "attach"] {
            for deferred in [false, true] {
                let book = Book::create(
                    &scratch
                        .path()
                        .join(format!("{encrypted}-{action}-{deferred}")),
                    passphrase,
                )
                .unwrap();
                let conn = book.connection();
                seed(conn, &fixture);
                let before = dump(conn);
                let case = cases
                    .iter()
                    .find(|case| {
                        text(case, "name")
                            == if deferred {
                                format!(
                                    "{action}-{}-deferred",
                                    if action == "attach" {
                                        "source_funds_link_attachments"
                                    } else {
                                        "source_funds_links"
                                    }
                                )
                            } else {
                                format!("desktop-{action}")
                            }
                    })
                    .unwrap();
                let observer = if deferred {
                    conn.execute_batch(text(case, "sql")).unwrap();
                    None
                } else {
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
                        .execute_batch("BEGIN; SELECT * FROM source_funds_links")
                        .unwrap();
                    Some(observer)
                };
                let actual = outcome(conn, case);
                let error = value(&actual, "error");
                assert_eq!(
                    text(error, "code"),
                    if deferred {
                        "internal_error"
                    } else {
                        "database_busy"
                    }
                );
                assert!(conn.is_autocommit());
                if deferred {
                    conn.execute_batch("DROP TRIGGER refuse_commit; DROP TABLE commit_failure")
                        .unwrap();
                }
                assert_eq!(encode(&dump(conn)), encode(&before));
                if let Some(observer) = observer {
                    observer.execute_batch("ROLLBACK").unwrap();
                }
            }
        }
    }
}

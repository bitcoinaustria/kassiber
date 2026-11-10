#[path = "support/source_funds_mutations.rs"]
mod support;
use kassiber_envelope::{get, set};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::Connection, Book};
use support::{dump, encode, outcome, text, value};

fn cases() -> Vec<Value> {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("data/source_funds_passes.json")).unwrap()
    else {
        panic!()
    };
    assert!(cases.len() > 200);
    cases
}

fn replay(witnesses: bool) {
    let root = tempfile::tempdir().unwrap();
    let template = Book::create(&root.path().join("template"), None).unwrap();
    for case in cases() {
        if witnesses
            && !text(&case, "name").starts_with("ui.")
            && ![
                "suggest-provider-default",
                "suggest-cap-rollback",
                "suggest-late-statement",
                "suggest-deferred",
                "assemble-provider-default-1",
                "assemble-provider-default-2",
                "assemble-provider-default-99",
                "assemble-provider-default-0",
                "assemble-provider-default--1",
                "assemble-structural-current-1",
                "assemble-structural-current-8",
                "assemble-attestation",
                "assemble-frozen-0",
                "assemble-frozen-1",
                "assemble-frozen-review",
                "assemble-late-2",
                "assemble-late-3",
                "assemble-review-abort",
            ]
            .contains(&text(&case, "name"))
        {
            continue;
        }

        let mut conn = Connection::open_in_memory().unwrap();
        kassiber_store::rusqlite::backup::Backup::new(template.connection(), &mut conn)
            .unwrap()
            .run_to_completion(256, std::time::Duration::ZERO, None)
            .unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        conn.execute_batch(text(&case, "sql")).unwrap();
        let mut expected = dump(&conn);
        if let Value::Dict(tables) = value(&case, "tables") {
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
        }
        assert_eq!(
            encode(&outcome(&conn, &case)),
            encode(value(&case, "expected")),
            "{}",
            text(&case, "name")
        );
        assert_eq!(
            encode(&dump(&conn)),
            encode(&expected),
            "{}",
            text(&case, "name")
        );
        assert!(conn.is_autocommit(), "{}", text(&case, "name"));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM transaction_pairs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn source_funds_passes_pin_persisted_envelopes_and_all_112_tables() {
    replay(false);
}

#[test]
fn source_funds_passes_mutation_witnesses() {
    replay(true);
}

#[test]
fn source_funds_passes_keep_committed_stages_and_rollback_late_failures_on_both_stores() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let evidence = root.path().join("unopened.pdf");
    std::fs::write(&evidence, b"synthetic evidence\0\xff").unwrap();
    let bytes = std::fs::read(&evidence).unwrap();
    let hash = Sha256::digest(&bytes);
    let cases = cases();
    for encrypted in [false, true] {
        for name in ["assemble-late-2", "assemble-late-3"] {
            let case = cases.iter().find(|c| text(c, "name") == name).unwrap();
            let book = Book::create(
                &root.path().join(format!("{encrypted}-{name}")),
                encrypted.then_some("synthetic-passes-passphrase"),
            )
            .unwrap();
            let conn = book.connection();
            conn.execute_batch(text(case, "sql")).unwrap();
            let mut expected = dump(conn);
            if let Value::Dict(tables) = value(case, "tables") {
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
            }
            assert!(get(&outcome(conn, case), "error").is_some());
            assert_eq!(encode(&dump(conn)), encode(&expected), "{encrypted}-{name}");
            assert!(conn.is_autocommit());
            assert_eq!(std::fs::read(&evidence).unwrap(), bytes);
            assert_eq!(Sha256::digest(std::fs::read(&evidence).unwrap()), hash);
        }
    }
}

#[test]
fn source_funds_passes_commit_caller_work_and_preserve_preview_rollback() {
    let cases = cases();
    let root = tempfile::tempdir().unwrap();
    for encrypted in [false, true] {
        let book = Book::create(
            &root.path().join(format!("{encrypted}")),
            encrypted.then_some("synthetic-passes-passphrase"),
        )
        .unwrap();
        let conn = book.connection();
        let case = cases
            .iter()
            .find(|c| text(c, "name") == "suggest-provider-default")
            .unwrap();
        conn.execute_batch(text(case, "sql")).unwrap();
        conn.execute_batch(
            "BEGIN; INSERT INTO settings(key,value) VALUES('caller-work','retained')",
        )
        .unwrap();
        assert!(get(&outcome(conn, case), "ok").is_some());
        assert!(conn.is_autocommit());
        assert_eq!(
            conn.query_row(
                "SELECT value FROM settings WHERE key='caller-work'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "retained"
        );
        let before = dump(conn);
        conn.execute_batch(
            "BEGIN; INSERT INTO settings(key,value) VALUES('preview-work','pending')",
        )
        .unwrap();
        let pending = dump(conn);
        let mut preview = case.clone();
        let mut args = value(case, "args").clone();
        set(&mut args, "dry_run", Value::Bool(true));
        set(&mut preview, "args", args);
        assert!(get(&outcome(conn, &preview), "ok").is_some());
        assert!(!conn.is_autocommit());
        assert_eq!(encode(&dump(conn)), encode(&pending));
        conn.execute_batch("ROLLBACK").unwrap();
        assert_eq!(encode(&dump(conn)), encode(&before));
    }
}

#[test]
fn source_funds_passes_rollback_busy_commits_on_both_stores() {
    let root = tempfile::tempdir().unwrap();
    let cases = cases();
    let case = cases
        .iter()
        .find(|c| text(c, "name") == "suggest-provider-default")
        .unwrap();
    for encrypted in [false, true] {
        let passphrase = encrypted.then_some("synthetic-passes-passphrase");
        let book = Book::create(&root.path().join(format!("{encrypted}")), passphrase).unwrap();
        let conn = book.connection();
        conn.execute_batch(text(case, "sql")).unwrap();
        conn.pragma_update(None, "journal_mode", "DELETE").unwrap();
        conn.busy_timeout(std::time::Duration::ZERO).unwrap();
        let path: String = conn
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let other = Connection::open(path).unwrap();
        if let Some(passphrase) = passphrase {
            other.pragma_update(None, "key", passphrase).unwrap();
        }
        other.execute_batch("BEGIN").unwrap();
        other
            .query_row("SELECT COUNT(*) FROM source_funds_links", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
        let before = dump(conn);
        let result = outcome(conn, case);
        assert!(get(&result, "error").is_some());
        assert_eq!(text(value(&result, "error"), "code"), "database_busy");
        assert_eq!(encode(&dump(conn)), encode(&before));
        assert!(conn.is_autocommit());
        other.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn source_funds_passes_rollback_response_projection_without_undoing_prior_commits() {
    let cases = cases();
    let root = tempfile::tempdir().unwrap();
    for encrypted in [false, true] {
        for (name, event) in [
            ("suggest-provider-default", "INSERT"),
            ("assemble-structural-current-1", "UPDATE"),
        ] {
            let passphrase = encrypted.then_some("synthetic-passes-passphrase");
            let case = cases.iter().find(|c| text(c, "name") == name).unwrap();
            let book = Book::create(
                &root.path().join(format!("{encrypted}-{event}")),
                passphrase,
            )
            .unwrap();
            let conn = book.connection();
            conn.execute_batch(text(case, "sql")).unwrap();
            let expected = if event == "INSERT" {
                dump(conn)
            } else {
                let control = Book::create(
                    &root.path().join(format!("{encrypted}-control")),
                    passphrase,
                )
                .unwrap();
                control
                    .connection()
                    .execute_batch(text(case, "sql"))
                    .unwrap();
                let mut suggest = case.clone();
                set(
                    &mut suggest,
                    "command",
                    kassiber_envelope::s("source-funds.suggest"),
                );
                assert!(get(&outcome(control.connection(), &suggest), "ok").is_some());
                let mut expected = dump(conn);
                set(
                    &mut expected,
                    "source_funds_links",
                    value(&dump(control.connection()), "source_funds_links").clone(),
                );
                expected
            };
            conn.execute_batch(&format!("CREATE TRIGGER corrupt_response AFTER {event} ON source_funds_links BEGIN UPDATE source_funds_links SET allocation_amount='bad' WHERE id=NEW.id; END;")).unwrap();
            assert!(get(&outcome(conn, case), "error").is_some());
            assert_eq!(encode(&dump(conn)), encode(&expected));
            assert!(conn.is_autocommit());
        }
    }
}

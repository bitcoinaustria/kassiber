#![cfg(feature = "test-harness")]
use kassiber_envelope::{dict, get, py_str, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::{types::Value as SqlValue, Connection};
use kassiber_transfers::{review_cases, Scope};
use std::collections::BTreeMap;

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap()
}
fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(values) => values,
        _ => panic!("expected list"),
    }
}
fn bytes(value: &Value) -> String {
    dumps(value, &Options::default()).unwrap()
}
fn fixture() -> Value {
    kassiber_pytext::loads(include_str!("review_cases.json")).unwrap()
}
fn seed(fixture: &Value, scenario: &Value, encrypted: bool) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    if encrypted {
        conn.pragma_update(None, "key", "synthetic-case-key")
            .unwrap();
    }
    conn.execute_batch("PRAGMA foreign_keys=OFF; PRAGMA temp_store=MEMORY")
        .unwrap();
    for statements in [
        field(fixture, "schema"),
        field(scenario, "sql"),
        field(fixture, "objects"),
    ] {
        for statement in list(statements) {
            conn.execute_batch(&py_str(statement)).unwrap();
        }
    }
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    assert!(conn
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
    conn
}
fn dump(conn: &Connection) -> BTreeMap<String, Vec<Vec<SqlValue>>> {
    let tables: Vec<String> = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    tables
        .into_iter()
        .map(|table| {
            let mut statement = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
            let count = statement.column_count();
            let mut rows: Vec<Vec<SqlValue>> = statement
                .query_map([], |row| (0..count).map(|index| row.get(index)).collect())
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            rows.sort_by_key(|row| format!("{row:?}"));
            (table, rows)
        })
        .collect()
}

#[test]
fn review_cases_match_python_pages_freshness_pair_evidence_and_receipts_without_writes() {
    assert_cases(false);
}

#[test]
fn review_cases_preserve_sqlcipher_snapshots_and_caller_transactions() {
    assert_cases(true);
}

fn assert_cases(encrypted: bool) {
    let fixture = fixture();
    let mut successes = 0;
    let mut refusals = 0;
    let mut pairs = 0;
    for scenario in list(field(&fixture, "scenarios")) {
        let name = py_str(field(scenario, "name"));
        let conn = seed(&fixture, scenario, encrypted);
        let before = dump(&conn);
        let changes = conn.total_changes();
        for transaction in [None, Some("BEGIN"), Some("BEGIN IMMEDIATE")] {
            if let Some(transaction) = transaction {
                conn.execute_batch(transaction).unwrap();
            }
            for case in list(field(scenario, "cases")) {
                let mut calls = 0usize;
                let actual = review_cases::inspect(
                    &conn,
                    &Scope {
                        workspace_id: "w",
                        profile_id: "p",
                    },
                    field(case, "args"),
                    |connection, profile| {
                        calls += 1;
                        assert!(!connection.is_autocommit());
                        assert_eq!(field(profile, "id"), &s("p"));
                        Ok(list(field(scenario, "quarantines")).to_vec())
                    },
                );
                assert_eq!(Value::from(calls), *field(case, "build_calls"), "{name}");
                if let Some(expected) = get(case, "result") {
                    let actual = actual.unwrap_or_else(|error| {
                        panic!("{name} {:?}: {error:?}", field(case, "args"))
                    });
                    pairs += list(field(&actual, "cases"))
                        .iter()
                        .filter(|case| get(case, "pair").is_some())
                        .count();
                    assert_eq!(
                        bytes(&actual),
                        bytes(expected),
                        "{name} {:?}",
                        field(case, "args")
                    );
                    successes += 1;
                } else {
                    let error = actual.unwrap_err();
                    assert_eq!(
                        bytes(&dict([
                            ("code", s(error.code)),
                            ("message", s(error.message)),
                            ("hint", error.hint.map(s).unwrap_or(Value::Null)),
                            ("details", error.details.unwrap_or(Value::Null))
                        ])),
                        bytes(field(case, "error")),
                        "{name} {:?}",
                        field(case, "args")
                    );
                    refusals += 1;
                }
                assert_eq!(conn.total_changes(), changes, "{name}");
                assert_eq!(conn.is_autocommit(), transaction.is_none(), "{name}");
            }
            assert_eq!(dump(&conn), before, "{name}");
            if transaction.is_some() {
                conn.execute_batch("ROLLBACK").unwrap();
            }
        }
    }
    assert!(successes > 100 && refusals > 100 && pairs > 0);
}

#[test]
fn review_waiting_roots_match_python_chains_cycles_groups_and_ambiguous_timestamps() {
    let fixture = fixture();
    let rows = |value: &Value| -> BTreeMap<String, Value> {
        match value {
            Value::Dict(entries) => entries
                .iter()
                .map(|(key, value)| match key {
                    kassiber_pyjson::Key::Str(key) => {
                        (key.as_str().unwrap().to_string(), value.clone())
                    }
                    _ => panic!("string key"),
                })
                .collect(),
            _ => panic!("object"),
        }
    };
    for case in list(field(&fixture, "waiting")) {
        let actual =
            review_cases::waiting_roots(&rows(field(case, "rows")), &rows(field(case, "details")));
        let actual = dict(actual.into_iter().map(|(key, value)| (key, s(value))));
        let options = Options::new().sort_keys(true);
        assert_eq!(
            dumps(&actual, &options).unwrap(),
            dumps(field(case, "expected"), &options).unwrap(),
            "{:?}",
            field(case, "name")
        );
    }
}

#[test]
fn review_pair_fingerprints_bind_allocations_and_observations_but_ignore_note_only_revisions() {
    let fixture = fixture();
    let mut fingerprints = BTreeMap::new();
    let mut differing = 0;
    let mut same = 0;
    let mut reverse = 0;
    let mut cross_asset = 0;
    for scenario in list(field(&fixture, "scenarios")) {
        let conn = seed(&fixture, scenario, false);
        for reading in list(field(scenario, "pair_readings")) {
            let pair = field(reading, "pair");
            let actual =
                review_cases::pair_evidence(&conn, &py_str(field(reading, "transaction_id")), pair)
                    .unwrap();
            assert_eq!(
                bytes(&actual),
                bytes(field(reading, "expected")),
                "{:?}",
                field(scenario, "name")
            );
            match get(&actual, "pair_txids_differ") {
                Some(Value::Bool(true)) => differing += 1,
                Some(Value::Bool(false)) => same += 1,
                None => cross_asset += 1,
                _ => panic!("boolean hint"),
            }
            if get(&actual, "pair_receipt_before_spend") == Some(&Value::Bool(true)) {
                reverse += 1;
            }
            fingerprints.insert(
                py_str(field(scenario, "name")),
                field(&actual, "pair_fingerprint").clone(),
            );
        }
    }
    assert_eq!(fingerprints["pair"], fingerprints["pair-note"]);
    assert_ne!(fingerprints["pair"], fingerprints["pair-allocation"]);
    assert!(differing > 0 && same > 0 && reverse > 0 && cross_asset > 0);
}

#[test]
fn review_cases_roll_back_only_owned_reads_after_early_and_late_refusals() {
    let fixture = fixture();
    let scenario = &list(field(&fixture, "scenarios"))[0];
    for encrypted in [false, true] {
        let conn = seed(&fixture, scenario, encrypted);
        for owns in [true, false] {
            if !owns {
                conn.execute_batch("BEGIN IMMEDIATE; UPDATE transactions SET note='pending caller edit' WHERE id='a'").unwrap();
            }
            let before = dump(&conn);
            let changes = conn.total_changes();
            let error = review_cases::inspect(
                &conn,
                &Scope {
                    workspace_id: "w",
                    profile_id: "p",
                },
                &dict::<&str>([]),
                |_, _| Err(AppError::new("injected_build", "late build refusal")),
            )
            .unwrap_err();
            assert_eq!(error.code, "injected_build");
            assert_eq!(conn.is_autocommit(), owns);
            assert_eq!(conn.total_changes(), changes);
            assert_eq!(dump(&conn), before);
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = review_cases::inspect(
                    &conn,
                    &Scope {
                        workspace_id: "w",
                        profile_id: "p",
                    },
                    &dict::<&str>([]),
                    |_, _| panic!("injected build unwind"),
                );
            }));
            assert!(panic.is_err());
            assert_eq!(conn.is_autocommit(), owns);
            assert_eq!(dump(&conn), before);
            let error = review_cases::inspect(
                &conn,
                &Scope {
                    workspace_id: "foreign",
                    profile_id: "p",
                },
                &dict::<&str>([]),
                |_, _| panic!("foreign profile cannot build"),
            )
            .unwrap_err();
            assert_eq!(error.code, "not_found");
            assert_eq!(conn.is_autocommit(), owns);
            if !owns {
                conn.execute_batch("ROLLBACK").unwrap();
                assert_ne!(dump(&conn), before, "caller edit remained rollbackable");
            }
        }
    }
}

#[test]
fn review_case_cursor_and_freshness_share_the_build_snapshot_during_concurrent_writes() {
    let fixture = fixture();
    let scenario = &list(field(&fixture, "scenarios"))[0];
    let memory = seed(&fixture, scenario, false);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic-review.sqlite");
    let mut conn = Connection::open(&path).unwrap();
    kassiber_store::rusqlite::backup::Backup::new(&memory, &mut conn)
        .unwrap()
        .run_to_completion(1000, std::time::Duration::ZERO, None)
        .unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    let peer = Connection::open(&path).unwrap();
    let before_version: i64 = conn
        .query_row(
            "SELECT journal_input_version FROM profiles WHERE id='p'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let before_note: Option<String> = conn
        .query_row("SELECT note FROM transactions WHERE id='a'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let page=review_cases::inspect(&conn,&Scope{workspace_id:"w",profile_id:"p"},&dict([("limit",1.into())]),|source,_| {
        assert!(std::ptr::eq(source,&conn));
        peer.execute_batch("BEGIN IMMEDIATE; UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id='p'; UPDATE transactions SET note='concurrent edit' WHERE id='a'; COMMIT").unwrap();
        let note:Option<String>=source.query_row("SELECT note FROM transactions WHERE id='a'",[],|row|row.get(0)).unwrap();
        assert_eq!(note,before_note);
        Ok(list(field(scenario,"quarantines")).to_vec())
    }).unwrap();
    assert_eq!(field(&page, "input_version"), &Value::from(before_version));
    assert_eq!(
        field(field(&page, "freshness"), "journal_input_version"),
        &Value::from(before_version)
    );
    assert!(conn.is_autocommit());
    let current: i64 = conn
        .query_row(
            "SELECT journal_input_version FROM profiles WHERE id='p'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(current, before_version + 1);
    let error = review_cases::inspect(
        &conn,
        &Scope {
            workspace_id: "w",
            profile_id: "p",
        },
        &dict([("cursor", field(&page, "next_cursor").clone())]),
        |_, _| panic!("stale cursor must refuse before building"),
    )
    .unwrap_err();
    assert_eq!(error.code, "review_cursor_stale");
    assert!(conn.is_autocommit());
}

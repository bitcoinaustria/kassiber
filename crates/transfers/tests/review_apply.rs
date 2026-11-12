#![cfg(feature = "test-harness")]
use kassiber_envelope::{dict, get, py_str, s, set, AppError};
use kassiber_journals::review_workflow::{self, Apply, Error};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{functions::FunctionFlags, types::ValueRef, Connection};
use kassiber_transfers::{review_artifact, review_operations, Scope, Sources};
use std::collections::BTreeMap;

const CLOCK: &str = "2026-10-31T12:00:00Z";
/// Rust's identities in draw order, with one draw optionally replaced.
#[derive(Default)]
struct Draws {
    ids: std::cell::RefCell<Vec<String>>,
    replace: Option<(usize, String)>,
}
impl Sources for Draws {
    fn uuid(&self, conn: &Connection) -> Result<String, AppError> {
        let index = self.ids.borrow().len();
        let id = match &self.replace {
            Some((at, id)) if *at == index => id.clone(),
            _ => kassiber_transfers::SystemSources.uuid(conn)?,
        };
        self.ids.borrow_mut().push(id.clone());
        Ok(id)
    }
    fn now(&self, _: &Connection) -> Result<String, AppError> {
        Ok(CLOCK.into())
    }
}
/// A pair term's pair_id is a fresh uuid4 that round 8's component authoring
/// never persists, and a dismissal later undone leaves no row either, so no
/// row identity names them, yet components derive their ids and hashes from
/// the pair id. The draw that made Rust's id, if one differs from Python's,
/// and Python's id for it.
fn pair_seed(actual: &Value, case: &Value, draws: &Draws) -> Option<(usize, String)> {
    let operations = |v: &Value| {
        get(v, "result")
            .and_then(|r| get(r, "operations"))
            .map(list)
            .unwrap_or(&[])
            .to_vec()
    };
    let pair = |op: &Value| {
        let result = get(op, "result")?;
        get(result, "pair_id")
            .or_else(|| get(result, "dismissal_id"))
            .map(py_str)
    };
    operations(actual)
        .iter()
        .zip(operations(case))
        .find_map(|(have, want)| {
            let (have, want) = (pair(have)?, pair(&want)?);
            let at = draws.ids.borrow().iter().position(|id| *id == have)?;
            (have != want).then_some((at, want))
        })
}
struct Clock;
impl Sources for Clock {
    fn uuid(&self, conn: &Connection) -> Result<String, AppError> {
        kassiber_transfers::SystemSources.uuid(conn)
    }
    fn now(&self, _: &Connection) -> Result<String, AppError> {
        Ok(CLOCK.into())
    }
}
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or(&Value::Null)
}
fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(rows) => rows,
        _ => panic!("fixture list"),
    }
}
fn scope() -> Scope<'static> {
    Scope {
        workspace_id: "w",
        profile_id: "p",
    }
}
fn fixtures() -> &'static Value {
    static FIXTURE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    FIXTURE.get_or_init(|| kassiber_pytext::loads(include_str!("review_apply.json")).unwrap())
}
fn cases(fixture: &Value) -> &[Value] {
    list(field(fixture, "cases"))
}
fn first(fixture: &Value) -> &Value {
    cases(fixture)
        .iter()
        .find(|row| {
            field(row, "name") == &s("mixed-metadata") && field(row, "source") == &s("user")
        })
        .unwrap()
}
fn seed(case: &Value, encrypted: bool) -> Connection {
    seed_connection(Connection::open_in_memory().unwrap(), case, encrypted)
}
fn seed_connection(conn: Connection, case: &Value, encrypted: bool) -> Connection {
    if encrypted {
        conn.pragma_update(None, "key", "synthetic-apply-source")
            .unwrap();
    }
    conn.execute_batch("PRAGMA foreign_keys=OFF; PRAGMA temp_store=MEMORY; PRAGMA cache_size=73; PRAGMA cache_spill=OFF").unwrap();
    let fixture = fixtures();
    for statements in [
        field(fixture, "schema"),
        field(case, "base_sql"),
        field(fixture, "objects"),
        field(case, "setup"),
    ] {
        for statement in list(statements) {
            conn.execute_batch(&py_str(statement)).unwrap();
        }
    }
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    conn.create_scalar_function("strftime", 2, FunctionFlags::SQLITE_UTF8, |_| {
        Ok(CLOCK.to_owned())
    })
    .unwrap();
    conn
}
fn snapshot(conn: &Connection) -> Value {
    let tables=conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|row|row.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    dict(tables.into_iter().map(|table| {
        let query = format!("SELECT * FROM \"{table}\"");
        let mut statement = conn
            .prepare(&format!("{query} ORDER BY rowid"))
            .or_else(|_| conn.prepare(&query))
            .unwrap();
        let names = statement
            .column_names()
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        let rows = statement
            .query_map([], |row| {
                Ok(dict(names.iter().enumerate().map(|(i, name)| {
                    let value = match row.get_ref(i).unwrap() {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(n) => n.into(),
                        ValueRef::Real(n) => Value::Float(n),
                        ValueRef::Text(raw) => s(std::str::from_utf8(raw).unwrap()),
                        ValueRef::Blob(raw) => dict([(
                            "$sqlite_blob",
                            s(raw
                                .iter()
                                .map(|byte| format!("{byte:02x}"))
                                .collect::<String>()),
                        )]),
                    };
                    (name.as_str(), value)
                })))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        (table, Value::List(rows))
    }))
}
fn apply(
    conn: &Connection,
    case: &Value,
    artifact: &Value,
    key: &str,
    commit: bool,
) -> Result<Value, Error> {
    apply_with(conn, case, artifact, key, commit, &Clock)
}
fn apply_with(
    conn: &Connection,
    case: &Value,
    artifact: &Value,
    key: &str,
    commit: bool,
    sources: &dyn Sources,
) -> Result<Value, Error> {
    review_workflow::apply(
        conn,
        &scope(),
        Apply {
            artifact,
            idempotency_key: &s(key),
            authored_source: &py_str(field(case, "source")),
            commit,
        },
        CLOCK,
        sources,
    )
}
fn outcome(result: Result<Value, Error>) -> Value {
    match result {
        Ok(result) => dict([("result", result)]),
        Err(Error::Service(error)) => dict([(
            "error",
            dict([("code", s(error.code)), ("message", s(error.message))]),
        )]),
        Err(Error::Build(error)) => dict([(
            "error",
            dict([
                (
                    "code",
                    s(error.code.clone().unwrap_or("internal_error".into())),
                ),
                ("message", s(error.message.clone())),
            ]),
        )]),
    }
}
fn normalized(value: &Value, mapping: &BTreeMap<String, String>) -> Value {
    let mut wire = review_operations::canonical(value).unwrap();
    for (from, to) in mapping {
        wire = wire.replace(from, to);
    }
    kassiber_pytext::loads(&wire).unwrap()
}
fn settings(conn: &Connection) {
    for (name, expected) in [("cache_size", 73), ("cache_spill", 0), ("synchronous", 2)] {
        assert_eq!(
            conn.pragma_query_value(None, name, |row| row.get::<_, i64>(0))
                .unwrap(),
            expected,
            "{name}"
        );
    }
}
fn check_complete_rows(raw: bool) {
    for case in cases(fixtures())
        .iter()
        .filter(|case| py_str(field(case, "name")).starts_with("raw-") == raw)
    {
        for encrypted in [false, true] {
            let mut conn = seed(case, encrypted);
            let draws = Draws::default();
            let mut actual = outcome(apply_with(
                &conn,
                case,
                field(case, "artifact"),
                "review-apply",
                true,
                &draws,
            ));
            // Replay with Python's pair id at the draw that seeded Rust's.
            if let Some(replace) = pair_seed(&actual, case, &draws) {
                conn = seed(case, encrypted);
                let draws = Draws {
                    replace: Some(replace),
                    ..Default::default()
                };
                actual = outcome(apply_with(
                    &conn,
                    case,
                    field(case, "artifact"),
                    "review-apply",
                    true,
                    &draws,
                ));
            }
            assert_eq!(
                get(&actual, "error").is_some(),
                get(case, "error").is_some(),
                "{} source={} encrypted={encrypted}: {}",
                py_str(field(case, "name")),
                py_str(field(case, "source")),
                review_operations::canonical(&actual).unwrap()
            );
            let rows = snapshot(&conn);
            let mut mapping = BTreeMap::new();
            for identity in list(field(case, "identities")) {
                let table = py_str(field(identity, "table"));
                let index = py_str(field(identity, "index")).parse::<usize>().unwrap();
                let from = py_str(field(&list(field(&rows, &table))[index], "id"));
                let to = py_str(field(identity, "id"));
                assert!(mapping.insert(from, to).is_none(), "identity bijection");
            }
            let wanted = if let Some(result) = get(case, "result") {
                dict([("result", result.clone())])
            } else {
                dict([("error", field(case, "error").clone())])
            };
            assert!(
                review_artifact::equal(&normalized(&actual, &mapping), &wanted),
                "{} source={} encrypted={encrypted}\nactual={}\nwanted={}",
                py_str(field(case, "name")),
                py_str(field(case, "source")),
                review_operations::canonical(&normalized(&actual, &mapping)).unwrap(),
                review_operations::canonical(&wanted).unwrap()
            );
            let rows = normalized(&rows, &mapping);
            for (key, expected) in match field(case, "expected") {
                Value::Dict(rows) => rows,
                _ => panic!(),
            } {
                let table = match key {
                    kassiber_pyjson::Key::Str(key) => key.as_str().unwrap(),
                    _ => panic!(),
                };
                assert!(
                    review_artifact::equal(field(&rows, table), expected),
                    "{} source={} encrypted={encrypted} table={table}\nactual={}\nwanted={}",
                    py_str(field(case, "name")),
                    py_str(field(case, "source")),
                    review_operations::canonical(field(&rows, table)).unwrap(),
                    review_operations::canonical(expected).unwrap()
                );
            }
            assert!(conn.is_autocommit());
            settings(&conn);
        }
    }
}

#[test]
fn review_apply_matches_python_receipts_authorship_and_complete_database_rows() {
    check_complete_rows(false);
}

#[test]
fn review_apply_preserves_raw_loader_storage_classes_and_receipts() {
    check_complete_rows(true);
}

#[test]
fn review_apply_rejects_unsupported_metadata_authors_even_for_noops() {
    let fixture = fixtures();
    let case = cases(fixture)
        .iter()
        .find(|case| {
            field(case, "name") == &s("noop-author-surrogate")
                && field(case, "source") == &s("desktop")
        })
        .unwrap();
    for encrypted in [false, true] {
        let conn = seed(case, encrypted);
        let before = snapshot(&conn);
        let error = apply(
            &conn,
            case,
            field(case, "artifact"),
            "invalid-noop-author",
            true,
        )
        .unwrap_err();
        let Error::Service(error) = error else {
            panic!("metadata source refusal")
        };
        assert_eq!(error.code, "validation");
        assert_eq!(error.message, "transaction edit source is not supported");
        assert_eq!(snapshot(&conn), before);
        assert!(conn.is_autocommit());
    }
}
#[test]
fn review_apply_historical_retries_precede_freshness_and_conflicting_keys() {
    let fixture = fixtures();
    let case = first(fixture);
    for encrypted in [false, true] {
        let conn = seed(case, encrypted);
        let artifact = field(case, "artifact");
        let receipt = apply(&conn, case, artifact, "retry", true).unwrap();
        conn.execute_batch("UPDATE profiles SET journal_input_version=99,tax_country='unsupported'; UPDATE transactions SET amount=amount+1").unwrap();
        let before = snapshot(&conn);
        conn.execute_batch("SAVEPOINT expected_reservation; UPDATE profiles SET journal_input_version=journal_input_version WHERE id='p'").unwrap();
        let reserved = snapshot(&conn);
        conn.execute_batch("ROLLBACK TO expected_reservation; RELEASE expected_reservation")
            .unwrap();
        let retry = apply(&conn, case, artifact, "retry", true).unwrap();
        assert!(review_artifact::equal(&retry, &receipt));
        assert_eq!(snapshot(&conn), reserved);
        let mut other = artifact.clone();
        let mut operations = field(&other, "operations").clone();
        if let Value::List(operations) = &mut operations {
            set(&mut operations[0], "reason", s("Another reviewed proposal"));
        }
        set(&mut other, "operations", operations);
        redigest(&mut other);
        let error = apply(&conn, case, &other, "retry", true).unwrap_err();
        assert!(matches!(error,Error::Service(error) if error.code=="review_idempotency_conflict"));
        assert_eq!(snapshot(&conn), reserved);
        assert!(conn.is_autocommit());
        settings(&conn);
        assert_ne!(before, reserved);
    }
}
fn redigest(artifact: &mut Value) {
    let payload = dict(
        [
            "schema_version",
            "workspace_id",
            "profile_id",
            "base_input_version",
            "operations",
            "before",
            "after",
            "delta",
        ]
        .into_iter()
        .map(|key| (key, field(artifact, key).clone())),
    );
    set(
        artifact,
        "digest",
        s(review_operations::digest(&payload).unwrap()),
    );
}
#[test]
fn review_apply_refuses_changed_before_and_after_effects_and_rolls_back() {
    let fixture = fixtures();
    let case = first(fixture);
    for encrypted in [false, true] {
        for phase in ["before", "after"] {
            let conn = seed(case, encrypted);
            let mut artifact = field(case, "artifact").clone();
            let mut effects = field(&artifact, phase).clone();
            set(&mut effects, "report_ready", Value::Bool(phase == "before"));
            set(&mut artifact, phase, effects);
            redigest(&mut artifact);
            let before = snapshot(&conn);
            let error = apply(&conn, case, &artifact, "stale", true).unwrap_err();
            assert!(
                matches!(error,Error::Service(error) if error.code=="review_plan_stale" && error.message==if phase=="before"{"Review evidence changed; create a fresh preview"}else{"Review effects changed; create a fresh preview"})
            );
            assert_eq!(snapshot(&conn), before);
            assert!(conn.is_autocommit());
            settings(&conn);
        }
    }
}
#[test]
fn review_apply_success_and_late_failures_preserve_caller_transactions_and_cache() {
    let fixture = fixtures();
    let case = first(fixture);
    let failures=[
        "CREATE TEMP TRIGGER refuse_review BEFORE INSERT ON transaction_edit_events WHEN NEW.transaction_id='b' BEGIN SELECT RAISE(ABORT,'second metadata refusal'); END",
        "CREATE TEMP TRIGGER refuse_review BEFORE INSERT ON journal_wallet_holdings BEGIN SELECT RAISE(ABORT,'late journal refusal'); END",
        "CREATE TEMP TRIGGER refuse_review BEFORE UPDATE OF last_processed_at ON profiles BEGIN SELECT RAISE(ABORT,'freshness refusal'); END",
        "CREATE TEMP TRIGGER refuse_review BEFORE INSERT ON review_workflow_receipts BEGIN SELECT RAISE(ABORT,'receipt refusal'); END",
    ];
    for encrypted in [false, true] {
        for caller in ["own", "read", "write"] {
            for failure in std::iter::once(None).chain(failures.iter().map(Some)) {
                let conn = seed(case, encrypted);
                let committed = snapshot(&conn);
                if caller == "read" {
                    conn.execute_batch("BEGIN; SELECT * FROM profiles").unwrap();
                }
                if caller == "write" {
                    conn.execute_batch(
                        "BEGIN IMMEDIATE; UPDATE transactions SET note='caller-owned' WHERE id='a'",
                    )
                    .unwrap();
                }
                let before = snapshot(&conn);
                if let Some(failure) = failure {
                    conn.execute_batch(failure).unwrap();
                }
                let result = apply(&conn, case, field(case, "artifact"), "pending", false);
                if failure.is_some() {
                    assert!(result.is_err());
                    assert_eq!(snapshot(&conn), before);
                    assert_eq!(conn.is_autocommit(), caller == "own");
                } else {
                    assert!(result.is_ok(), "{result:?}");
                    assert!(!conn.is_autocommit());
                    assert_eq!(
                        conn.query_row("SELECT count(*) FROM review_workflow_receipts", [], |r| {
                            r.get::<_, i64>(0)
                        })
                        .unwrap(),
                        1
                    );
                }
                settings(&conn);
                if !conn.is_autocommit() {
                    conn.execute_batch("ROLLBACK").unwrap();
                }
                assert_eq!(snapshot(&conn), committed);
            }
        }
    }
}
#[test]
fn review_apply_commit_failure_rolls_back_after_receipt_and_savepoint_release() {
    let fixture = fixtures();
    let case = first(fixture);
    for encrypted in [false, true] {
        let conn = seed(case, encrypted);
        conn.execute_batch("CREATE TABLE deferred_parent(id TEXT PRIMARY KEY); CREATE TABLE deferred_child(id TEXT REFERENCES deferred_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TEMP TRIGGER fail_commit AFTER INSERT ON review_workflow_receipts BEGIN INSERT INTO deferred_child VALUES('missing'); END").unwrap();
        let before = snapshot(&conn);
        let error = apply(&conn, case, field(case, "artifact"), "commit-fail", true).unwrap_err();
        assert!(matches!(error, Error::Service(_)));
        assert!(conn.is_autocommit());
        assert_eq!(snapshot(&conn), before);
        settings(&conn);
    }
}
#[test]
fn review_apply_reserves_writer_before_historical_lookup_for_deferred_callers() {
    use kassiber_store::rusqlite::TransactionState;
    let fixture = fixtures();
    let case = first(fixture);
    for encrypted in [false, true] {
        let conn = seed(case, encrypted);
        apply(&conn, case, field(case, "artifact"), "writer", true).unwrap();
        conn.execute_batch("BEGIN; SELECT * FROM profiles").unwrap();
        assert_eq!(
            conn.transaction_state(Some("main")).unwrap(),
            TransactionState::Read
        );
        apply(&conn, case, field(case, "artifact"), "writer", false).unwrap();
        assert_eq!(
            conn.transaction_state(Some("main")).unwrap(),
            TransactionState::Write
        );
        assert!(!conn.is_autocommit());
        settings(&conn);
        conn.execute_batch("ROLLBACK").unwrap();
    }
}
#[test]
fn review_apply_admission_and_version_checks_precede_domain_writes() {
    let fixture = fixtures();
    let case = first(fixture);
    for encrypted in [false, true] {
        for kind in ["digest", "version"] {
            let conn = seed(case, encrypted);
            let mut artifact = field(case, "artifact").clone();
            if kind == "digest" {
                set(&mut artifact, "digest", s("wrong"));
                conn.execute_batch("CREATE TEMP TRIGGER refuse_reservation BEFORE UPDATE ON profiles BEGIN SELECT RAISE(ABORT,'writer used before admission'); END").unwrap();
            } else {
                conn.execute_batch("UPDATE profiles SET journal_input_version=99 WHERE id='p'")
                    .unwrap();
            }
            let before = snapshot(&conn);
            let error = apply(&conn, case, &artifact, "invalid", true).unwrap_err();
            assert!(
                matches!(error,Error::Service(error) if error.code==if kind=="digest"{"review_artifact_invalid"}else{"custody_review_plan_stale"})
            );
            assert_eq!(snapshot(&conn), before);
            assert!(conn.is_autocommit());
            settings(&conn);
        }
    }
}
#[test]
fn review_apply_filed_impacts_resolve_only_after_report_ready_storage_and_roll_back_late() {
    let fixture = fixtures();
    for name in ["mixed-metadata-filed", "pair-filed"] {
        let case = cases(fixture)
            .iter()
            .find(|row| field(row, "name") == &s(name) && field(row, "source") == &s("user"))
            .unwrap();
        for encrypted in [false, true] {
            for late_failure in [false, true] {
                let conn = seed(case, encrypted);
                let before = snapshot(&conn);
                if late_failure {
                    conn.execute_batch("CREATE TEMP TRIGGER refuse_receipt BEFORE INSERT ON review_workflow_receipts BEGIN SELECT RAISE(ABORT,'after filed resolution'); END").unwrap();
                }
                let result = apply(&conn, case, field(case, "artifact"), "filed", true);
                if late_failure {
                    assert!(result.is_err());
                    assert_eq!(snapshot(&conn), before);
                } else {
                    let receipt = result.unwrap();
                    let count = conn
                        .query_row(
                            "SELECT count(*) FROM custody_filed_report_impact_resolutions",
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                        .unwrap();
                    assert_eq!(count, if name == "mixed-metadata-filed" { 2 } else { 0 });
                    assert_eq!(
                        field(field(&receipt, "verification"), "report_ready"),
                        &Value::Bool(name == "mixed-metadata-filed")
                    );
                }
                assert!(conn.is_autocommit());
                settings(&conn);
            }
        }
    }
}
#[test]
fn review_apply_receipt_survives_reopening_both_database_bindings() {
    let case = first(fixtures());
    let scratch =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/r5-evidence/tmp");
    std::fs::create_dir_all(&scratch).unwrap();
    let directory = tempfile::Builder::new()
        .prefix("review-reopen-")
        .tempdir_in(scratch)
        .unwrap();
    for encrypted in [false, true] {
        let path = directory.path().join(format!("review-{encrypted}.sqlite3"));
        let conn = seed_connection(Connection::open(&path).unwrap(), case, encrypted);
        let receipt = apply(&conn, case, field(case, "artifact"), "durable", true).unwrap();
        let stored = snapshot(&conn);
        drop(conn);
        let reopened = Connection::open(&path).unwrap();
        if encrypted {
            reopened
                .pragma_update(None, "key", "synthetic-apply-source")
                .unwrap();
        }
        assert_eq!(snapshot(&reopened), stored);
        let historical = kassiber_transfers::execute(
            &reopened,
            &scope(),
            "review.receipt",
            &dict([("idempotency_key", s("durable"))]),
        )
        .unwrap();
        assert!(review_artifact::equal(&historical, &receipt));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.starts_with(b"SQLite format 3\0"), !encrypted);
    }
}

#[test]
fn review_apply_receipts_keep_python_transport_key_order() {
    let case = first(fixtures());
    let conn = seed(case, false);
    let receipt = apply(&conn, case, field(case, "artifact"), "order", true).unwrap();
    let Value::Dict(entries) = &receipt else {
        panic!("receipt object")
    };
    let keys: Vec<String> = entries.iter().map(|(key, _)| format!("{key:?}")).collect();
    let expected = [
        "id",
        "schema_version",
        "workspace_id",
        "profile_id",
        "idempotency_key",
        "artifact_digest",
        "base_input_version",
        "result_input_version",
        "status",
        "operations",
        "transaction_ids",
        "before",
        "delta",
        "proposed_operations",
        "verification",
        "authored_source",
        "created_at",
    ];
    assert_eq!(
        keys,
        expected
            .iter()
            .map(|key| format!("{:?}", kassiber_pyjson::Key::Str((*key).into())))
            .collect::<Vec<_>>()
    );
    let Value::Dict(verification) = field(&receipt, "verification") else {
        panic!("verification object")
    };
    assert_eq!(
        format!("{:?}", verification.last().unwrap().0),
        format!("{:?}", kassiber_pyjson::Key::Str("processed_at".into()))
    );
}

#[test]
fn review_execute_routes_plans_commits_applies_and_reads_receipts_as_authored() {
    let case = first(fixtures());
    let conn = seed(case, false);
    let artifact = field(case, "artifact");
    let run = |operation: &str, args: Value| {
        review_workflow::execute(&conn, &scope(), operation, &args, "cli", &Clock)
    };
    let planned = run(
        "review.plan",
        dict([
            ("operations", field(artifact, "operations").clone()),
            (
                "expected_input_version",
                field(artifact, "base_input_version").clone(),
            ),
        ]),
    )
    .unwrap();
    assert!(review_artifact::equal(&planned, artifact));
    let page = run("review.cases", dict::<&str>([])).unwrap();
    assert_eq!(
        field(&page, "input_version"),
        field(artifact, "base_input_version")
    );
    let before = snapshot(&conn);
    let receipt = run(
        "review.apply",
        dict([
            ("artifact", artifact.clone()),
            ("idempotency_key", s("execute")),
        ]),
    )
    .unwrap();
    assert!(conn.is_autocommit(), "the service boundary commits");
    assert_ne!(snapshot(&conn), before);
    assert_eq!(field(&receipt, "authored_source"), &s("cli"));
    for selector in [
        dict([("idempotency_key", s("execute"))]),
        dict([("receipt_id", field(&receipt, "id").clone())]),
    ] {
        let stored = run("review.receipt", selector).unwrap();
        assert!(review_artifact::equal(&stored, &receipt));
    }
    let Err(Error::Service(error)) = run("review.export", dict::<&str>([])) else {
        panic!("an unknown review operation refuses");
    };
    assert_eq!(error.message, "Unsupported review command");
}

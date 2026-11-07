#![cfg(feature = "test-harness")]
use kassiber_envelope::{dict, get, py_str, s, AppError};
use kassiber_journals::review_workflow::{self, Error};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{types::Value as Sql, Connection};
use kassiber_transfers::{review_artifact, review_operations::canonical, Scope, Sources};
use std::collections::BTreeMap;

const CLOCK: &str = "2026-10-31T12:00:00Z";
struct Clock;
impl Sources for Clock {
    fn uuid(&self, _: &Connection) -> Result<String, AppError> {
        // Distinct ids: a plan's tax preview stores rows it then rolls back.
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Ok(format!(
            "aaaaaaaa-aaaa-4aaa-8aaa-{:012x}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
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
        _ => panic!("expected list"),
    }
}
fn scope() -> Scope<'static> {
    Scope {
        workspace_id: "w",
        profile_id: "p",
    }
}
fn plans() -> Value {
    kassiber_pytext::loads(include_str!("review_plans.json")).unwrap()
}
fn seed(case: &Value, encrypted: bool) -> Connection {
    let fixture = kassiber_pytext::loads(include_str!("review_cases.json")).unwrap();
    let base = get(case, "base_sql").unwrap_or_else(|| {
        field(
            list(field(&fixture, "scenarios"))
                .iter()
                .find(|row| field(row, "name") == field(case, "base"))
                .unwrap(),
            "sql",
        )
    });
    let conn = Connection::open_in_memory().unwrap();
    if encrypted {
        conn.pragma_update(None, "key", "synthetic-plan-source")
            .unwrap();
    }
    conn.execute_batch("PRAGMA foreign_keys=OFF; PRAGMA temp_store=MEMORY")
        .unwrap();
    for statements in [
        field(&fixture, "schema"),
        base,
        field(&fixture, "objects"),
        field(case, "setup"),
    ] {
        for statement in list(statements) {
            conn.execute_batch(&py_str(statement)).unwrap();
        }
    }
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    conn
}
fn dump(conn: &Connection) -> BTreeMap<String, Vec<Vec<Sql>>> {
    let tables = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    tables
        .into_iter()
        .map(|table| {
            let mut statement = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
            let count = statement.column_count();
            let mut rows = statement
                .query_map([], |row| {
                    (0..count)
                        .map(|index| row.get(index))
                        .collect::<Result<Vec<Sql>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.sort_by_key(|row| format!("{row:?}"));
            (table, rows)
        })
        .collect()
}
fn outcome(result: Result<Value, Error>) -> Value {
    let error = match result {
        Ok(result) => return dict([("result", result)]),
        Err(Error::Service(error)) => dict([
            ("code", s(error.code)),
            ("message", s(error.message)),
            ("hint", error.hint.map(s).unwrap_or(Value::Null)),
            ("details", error.details.unwrap_or(Value::Null)),
        ]),
        Err(Error::Build(error)) => dict([
            (
                "code",
                s(error.code.clone().unwrap_or("internal_error".into())),
            ),
            ("message", s(error.message.clone())),
            ("hint", error.hint.clone().map(s).unwrap_or(Value::Null)),
            (
                "details",
                kassiber_custody::codec::from_json(&error.details).unwrap(),
            ),
        ]),
    };
    dict([("error", error)])
}
fn wanted(case: &Value) -> Value {
    match get(case, "result") {
        Some(result) => dict([("result", result.clone())]),
        None => dict([("error", field(case, "error").clone())]),
    }
}
fn check_group(group: &str) {
    for case in list(&plans())
        .iter()
        .filter(|case| field(case, "group") == &s(group))
    {
        for encrypted in [false, true] {
            for read in [false, true] {
                let conn = seed(case, encrypted);
                if read {
                    conn.execute_batch("BEGIN; SELECT * FROM profiles").unwrap();
                }
                let before = dump(&conn);
                let changes = conn.total_changes();
                let actual = outcome(review_workflow::plan(
                    &conn,
                    &scope(),
                    field(case, "operations"),
                    field(case, "version"),
                    CLOCK,
                    &Clock,
                ));
                assert_eq!(
                    canonical(&actual).unwrap(),
                    canonical(&wanted(case)).unwrap(),
                    "{} encrypted={encrypted} read={read}",
                    py_str(field(case, "name"))
                );
                assert_eq!(conn.total_changes(), changes);
                assert_eq!(dump(&conn), before);
                assert_eq!(conn.is_autocommit(), !read);
            }
        }
    }
}

#[test]
fn review_plans_match_complete_python_mixed_metadata_effects_and_refusals() {
    check_group("metadata");
}

#[test]
fn review_plans_preserve_raw_loader_scalars_and_downstream_serialization_refusals() {
    check_group("raw");
}

#[test]
fn review_plans_preserve_authoring_refusals_and_metadata_noop_ordering() {
    check_group("authoring");
}

#[test]
fn review_plans_match_python_component_and_metadata_order() {
    check_group("components");
}

#[test]
fn review_plans_preserve_confirmed_sibling_unpair_baselines() {
    check_group("unpairs");
}

#[test]
fn review_plans_refresh_state_and_fingerprints_after_intervening_operations() {
    check_group("refresh");
}

#[test]
fn review_plans_carry_python_per_year_tax_figures_and_delta() {
    check_group("tax");
}

#[test]
fn review_plans_match_python_term_operations_and_coarse_acceptance() {
    check_group("terms");
}

#[test]
fn review_validation_recomputes_complete_artifacts_and_current_policy_and_evidence() {
    for case in list(&plans()) {
        for validation in list(field(case, "validations")) {
            for encrypted in [false, true] {
                let conn = seed(case, encrypted);
                if let Some(setup) = get(validation, "setup") {
                    conn.execute_batch(&py_str(setup)).unwrap();
                }
                let before = dump(&conn);
                let changes = conn.total_changes();
                let actual = outcome(review_workflow::validate(
                    &conn,
                    &scope(),
                    field(validation, "artifact"),
                    CLOCK,
                    &Clock,
                ));
                assert!(
                    review_artifact::equal(&actual, &wanted(validation)),
                    "{}\n{}\n{}",
                    py_str(field(case, "name")),
                    canonical(&actual).unwrap(),
                    canonical(&wanted(validation)).unwrap()
                );
                assert_eq!(conn.total_changes(), changes);
                assert_eq!(dump(&conn), before);
                assert!(conn.is_autocommit());
            }
        }
    }
}

#[test]
fn review_plans_refuse_pending_writes_and_preserve_caller_transaction() {
    let fixture = plans();
    let case = &list(&fixture)[0];
    for encrypted in [false, true] {
        let conn = seed(case, encrypted);
        conn.execute_batch(
            "BEGIN IMMEDIATE; UPDATE transactions SET note='caller-owned' WHERE id='a'",
        )
        .unwrap();
        let before = dump(&conn);
        let changes = conn.total_changes();
        let error = review_workflow::plan(
            &conn,
            &scope(),
            field(case, "operations"),
            field(case, "version"),
            CLOCK,
            &Clock,
        )
        .unwrap_err();
        assert!(
            matches!(error, Error::Build(error) if error.code.as_deref()==Some("review_transaction_active"))
        );
        assert_eq!(dump(&conn), before);
        assert_eq!(conn.total_changes(), changes);
        assert!(!conn.is_autocommit());
        conn.execute_batch("ROLLBACK").unwrap();
    }
}

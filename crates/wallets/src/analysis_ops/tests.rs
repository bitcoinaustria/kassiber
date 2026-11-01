use super::*;

fn run(group: &str) {
    run_corpus(include_str!("../../tests/data/analysis_ops.json"), group);
}

pub(super) fn run_corpus(corpus: &str, group: &str) {
    let Value::List(cases) = kassiber_pytext::loads(corpus).unwrap() else {
        panic!("fixture corpus")
    };
    let mut selected = 0;
    for case in cases
        .iter()
        .filter(|c| field(c, "group").as_str() == Some(group))
    {
        selected += 1;
        let conn = Connection::open_in_memory().unwrap();
        if truthy(field(case, "encrypted")) {
            conn.execute_batch(fixture::WATCH_KEY).unwrap();
        }
        conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        conn.execute_batch(include_str!(
            "../../../reports/tests/data/privacy_hygiene_seed.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(field(case, "sql").as_str().unwrap_or(""))
            .unwrap();
        let actual = fixture::evaluate(case, &conn).unwrap();
        let Value::List(actual) = actual else {
            panic!("result list")
        };
        let Value::List(expected) = field(case, "expected") else {
            panic!("expected list")
        };
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            for key in ["result", "in_transaction"] {
                assert_eq!(
                    canonical(field(actual, key)).unwrap(),
                    canonical(field(expected, key)).unwrap(),
                    "{}: {key}",
                    field(case, "name").as_str().unwrap()
                );
            }
            let actual = field(actual, "state");
            let expected = field(expected, "state");
            for key in ["table_count", "schema"] {
                assert_eq!(
                    field(actual, key),
                    field(expected, key),
                    "{}: {key}",
                    field(case, "name").as_str().unwrap()
                );
            }
            let Value::Dict(tables) = field(expected, "tables") else {
                panic!("tables")
            };
            for (key, hash) in tables {
                let Key::Str(key) = key else {
                    panic!("table name")
                };
                assert_eq!(
                    get(field(actual, "tables"), key.as_str().unwrap()),
                    Some(hash),
                    "{}: {:?}",
                    field(case, "name").as_str().unwrap(),
                    key
                );
            }
        }
    }
    assert!(selected > 0, "empty group: {group}");
}

#[test]
fn analysis_labels_match_python_bytes_and_all_112_tables() {
    run("labels");
}
#[test]
fn analysis_imports_match_python_atomic_history_and_all_112_tables() {
    run("imports");
}
#[test]
fn analysis_cases_match_python_bytes_integrity_and_all_112_tables() {
    run("cases");
}
#[test]
fn analysis_dataset_lifecycle_matches_python_and_all_112_tables() {
    run("datasets");
}
#[test]
fn analysis_operations_preserve_caller_transactions_and_rollback() {
    run("rollback");
}

#[test]
fn analysis_watches_match_python_bytes_events_and_all_112_tables() {
    run_corpus(
        include_str!("../../tests/data/analysis_watches.json"),
        "watches",
    );
}

#[test]
fn analysis_history_failure_rolls_back_label_and_import() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../../reports/tests/data/privacy_hygiene_seed.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!("../../../reports/tests/data/book.sql"))
        .unwrap();
    conn.execute_batch("CREATE TRIGGER reject_analysis_history BEFORE INSERT ON chain_analysis_label_history BEGIN SELECT RAISE(ABORT,'injected history failure'); END;").unwrap();
    let before = crate::analysis_index::fixture::tables(&conn);
    let args = kassiber_pytext::loads(r#"{"subject":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","chain":"bitcoin","network":"main","label":"Claim","category":"other","source":"local","confidence":"unverified"}"#).unwrap();
    let scope = Scope {
        workspace_id: "ws",
        profile_id: "p",
    };
    assert!(execute(&conn, &scope, "labels.upsert", &args).is_err());
    assert_eq!(crate::analysis_index::fixture::tables(&conn), before);
    assert!(execute(
        &conn,
        &scope,
        "labels.import",
        &dict([("items", Value::List(vec![args]))])
    )
    .is_err());
    assert_eq!(crate::analysis_index::fixture::tables(&conn), before);
    assert!(conn.is_autocommit());
}

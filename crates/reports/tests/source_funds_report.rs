use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::{rusqlite::Connection, Book};

fn dump(conn: &Connection) -> Vec<(String, Vec<String>)> {
    let names: Vec<String> = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
        .query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(names.len(), 113);
    names
        .into_iter()
        .map(|name| {
            let mut statement = conn
                .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
                .unwrap();
            let columns = statement.column_count();
            let mut records: Vec<_> = statement
                .query_map([], |row| {
                    Ok((0..columns)
                        .map(|column| format!("{:?}", row.get_ref(column).unwrap()))
                        .collect::<Vec<_>>())
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            records.sort();
            (
                name,
                records
                    .into_iter()
                    .map(|record| format!("{record:?}"))
                    .collect(),
            )
        })
        .collect()
}

fn outcome(result: Result<Value, AppError>) -> Value {
    match result {
        Ok(data) => dict([("ok", data)]),
        Err(error) => dict([(
            "error",
            dict([
                ("code", s(error.code)),
                ("message", s(error.message)),
                ("hint", error.hint.map(s).unwrap_or(Value::Null)),
                ("details", error.details.unwrap_or(Value::Null)),
                ("retryable", Value::Bool(error.retryable)),
            ]),
        )]),
    }
}

#[test]
fn source_funds_report_preserves_complete_bytes_lineage_diagrams_and_all_tables() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/source_funds_report.json")).unwrap();
    assert!(cases.len() >= 200);
    for passphrase in [None, Some("synthetic-source-funds-report-passphrase")] {
        let root = tempfile::tempdir().unwrap();
        let book = Book::create(&root.path().join("data"), passphrase).unwrap();
        let conn = book.connection();
        for case in &cases {
            conn.execute_batch("SAVEPOINT report_case").unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let before = dump(conn);
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let actual = outcome(kassiber_reports::execute(
                conn,
                case["command"].as_str().unwrap(),
                &args,
            ));
            let actual = dumps(&actual, &Options::new().compact()).unwrap();
            let variants = case["expected_variants"].as_array().unwrap();
            if !variants
                .iter()
                .any(|expected| expected.as_str() == Some(&actual))
            {
                let expected = variants[0].as_str().unwrap();
                let index = actual
                    .bytes()
                    .zip(expected.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(actual.len().min(expected.len()));
                panic!(
                    "{} at byte {}: actual {:?}, expected {:?}",
                    case["name"],
                    index,
                    &actual[index.saturating_sub(80)..actual.len().min(index + 240)],
                    &expected[index.saturating_sub(80)..expected.len().min(index + 240)]
                );
            }
            assert_eq!(dump(conn), before, "{}", case["name"]);
            conn.execute_batch("ROLLBACK TO report_case;RELEASE report_case")
                .unwrap();
        }
    }
}

#[test]
fn source_funds_structural_lineage_preserves_ancestors_authority_and_all_tables() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/source_funds_lineage.json")).unwrap();
    for passphrase in [None, Some("synthetic-lineage-passphrase")] {
        let root = tempfile::tempdir().unwrap();
        let book = Book::create(&root.path().join("data"), passphrase).unwrap();
        let before = dump(book.connection());
        for case in &cases {
            let rows = case["rows"].as_array().unwrap();
            let owned = case["owned"].as_array().unwrap();
            let actual = kassiber_custody::source_funds::structural_pairs(rows, owned);
            let actual: Vec<_> = actual
                .into_iter()
                .map(|((from, to), (amount, gross))| serde_json::json!([from, to, amount, gross]))
                .collect();
            assert_eq!(
                serde_json::to_string(&actual).unwrap(),
                serde_json::to_string(&case["expected"]).unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(dump(book.connection()), before, "{}", case["name"]);
        }
        assert_eq!(
            kassiber_custody::source_funds::weighted(
                &[i64::MAX as i128 * 1000, i64::MAX as i128 * 1000],
                i64::MAX as i128
            ),
            vec![4611686018427387904, 4611686018427387903]
        );
    }
}

#[test]
fn source_funds_report_binds_inputs_to_book_and_refuses_case_mutation() {
    let left = tempfile::tempdir().unwrap();
    let right = tempfile::tempdir().unwrap();
    let left = Book::create(&left.path().join("data"), None).unwrap();
    let right = Book::create(&right.path().join("data"), None).unwrap();
    for book in [&left, &right] {
        book.connection()
            .execute_batch(include_str!("data/book.sql"))
            .unwrap();
    }
    let args = kassiber_pytext::loads(r#"{"target_transaction":"in","save_case":true}"#).unwrap();
    let before = dump(left.connection());
    let right_before = dump(right.connection());
    let (_, profile) = kassiber_setup::references::scope(left.connection(), &args).unwrap();
    let inputs =
        kassiber_reports::source_funds_report::load_inputs(left.connection(), &profile).unwrap();
    let (workspace, profile) =
        kassiber_setup::references::scope(right.connection(), &args).unwrap();
    let error = kassiber_reports::source_funds_report::build(
        right.connection(),
        &workspace,
        &profile,
        &args,
        Some(&inputs),
    )
    .unwrap_err();
    assert_eq!(error.code, "validation");
    assert_eq!(
        error.message,
        "Source-funds report inputs belong to a different book"
    );
    let error = kassiber_reports::source_funds_report::execute(
        left.connection(),
        "reports.source-funds",
        &args,
    )
    .unwrap_err();
    assert_eq!(error.code, "not_implemented");
    assert_eq!(dump(left.connection()), before);
    assert_eq!(dump(right.connection()), right_before);
}

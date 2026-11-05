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

fn replay(witnesses: bool) {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/source_funds_coverage.json")).unwrap();
    assert!(cases.len() >= 80);
    for passphrase in [None, Some("synthetic-coverage-passphrase")] {
        let root = tempfile::tempdir().unwrap();
        let book = Book::create(&root.path().join("data"), passphrase).unwrap();
        let conn = book.connection();
        for case in &cases {
            let name = case["name"].as_str().unwrap();
            if witnesses
                && ![
                    "fully-traced",
                    "attested",
                    "unattributed",
                    "mixed-fiat_purchase-missing_history",
                    "mixed-fiat_purchase-unknown",
                    "mixed-opening_balance_attestation-unknown",
                    "sqlite-error",
                    "report-explorer-bitcoin-main-https://[broken/api",
                    "unlinked-malformed-schema",
                    "capped-malformed-schema",
                    "cli-cap-1",
                    "desktop-cap-50001",
                    "desktop-depth-True",
                    "default-production-cap",
                    "wallet-label-order",
                    "excluded-inbound",
                    "desktop-depth-33",
                    "desktop-unknown-fields",
                    "wallet-label-case",
                    "multiple-assets",
                    "unbounded-positive-total",
                    "unbounded-negative-total",
                    "fractions",
                    "negative",
                    "text-cap-1",
                    "report-branch-depth-8",
                    "rejected-only",
                    "suggested",
                    "cli-profile-Other",
                ]
                .contains(&name)
            {
                continue;
            }
            conn.execute_batch("SAVEPOINT coverage_case").unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let before = dump(conn);
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let actual = outcome(kassiber_reports::execute(
                conn,
                case["command"].as_str().unwrap(),
                &args,
            ));
            let actual = dumps(&actual, &Options::new().compact()).unwrap();
            let expected = case["expected_json"].as_str().unwrap();
            if actual != expected {
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
            conn.execute_batch("ROLLBACK TO coverage_case;RELEASE coverage_case")
                .unwrap();
        }
    }
}

#[test]
fn source_funds_coverage_preserves_buckets_limits_error_families_and_all_tables() {
    replay(false);
}

#[test]
fn source_funds_coverage_mutation_witnesses() {
    replay(true);
}

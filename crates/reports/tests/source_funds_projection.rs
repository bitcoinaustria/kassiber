use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_reports::{source_funds_disclosure as disclosure, source_funds_projection};
use kassiber_store::{rusqlite::Connection, Book};

fn value<'a>(row: &'a Value, key: &str) -> &'a Value {
    get(row, key).unwrap_or(&Value::Null)
}
fn text(row: &Value, key: &str) -> String {
    kassiber_envelope::py_str(value(row, key))
}
fn project(operation: &str, request: &Value) -> Result<Value, AppError> {
    Ok(match operation {
        "shape" => {
            let mut report = request.clone();
            source_funds_projection::add_shape(&mut report)?;
            report
        }
        "transaction" => disclosure::transaction_node(
            value(request, "row"),
            &text(request, "mode"),
            value(request, "required"),
            kassiber_envelope::truthy(value(request, "is_target")),
            value(request, "overrides"),
        )?,
        "source" => disclosure::source_node(
            value(request, "row"),
            &text(request, "mode"),
            value(request, "required"),
        )?,
        "attachment" => disclosure::attachment(value(request, "row"), &text(request, "mode"))?,
        "options" => disclosure::report_options(request)?,
        "context" => disclosure::report_context(request),
        "reveal" => s(disclosure::reveal_mode(request)?),
        "purpose" => s(disclosure::report_purpose(request)?),
        "edge_explanation" => {
            let Value::List(endpoints) = value(request, "endpoint_ids") else {
                panic!("endpoint ids")
            };
            disclosure::edge_explanation(
                value(request, "explanation"),
                &text(request, "mode"),
                &endpoints
                    .iter()
                    .map(kassiber_envelope::py_str)
                    .collect::<Vec<_>>(),
                value(request, "overrides"),
            )
        }
        _ => panic!("unknown projection {operation}"),
    })
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

#[test]
fn source_funds_projection_preserves_full_bytes_disclosure_flow_and_all_tables() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/source_funds_projection.json")).unwrap();
    assert!(cases.len() >= 400);
    for passphrase in [None, Some("synthetic-source-funds-projection-passphrase")] {
        let root = tempfile::tempdir().unwrap();
        let book = Book::create(&root.path().join("data"), passphrase).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        for case in &cases {
            let before = dump(conn);
            let request = kassiber_pytext::loads(case["input_json"].as_str().unwrap()).unwrap();
            let actual = outcome(project(case["operation"].as_str().unwrap(), &request));
            assert_eq!(
                dumps(&actual, &Options::new().compact()).unwrap(),
                case["expected_json"].as_str().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(dump(conn), before, "{}", case["name"]);
        }
    }
}

#[test]
fn source_funds_projection_orders_reference_ties_deterministically() {
    let mut report = kassiber_pytext::loads(r#"{"target":{"id":"target","required_amount":1,"required_amount_msat":100000000000,"asset":"BTC"},"graph":{"nodes":[{"id":"target","node_type":"transaction","label":"Target"},{"id":"a","node_type":"transaction","label":"same","occurred_at":"2024-01-01"},{"id":"b","node_type":"transaction","label":"same","occurred_at":"2024-01-01"}],"edges":[{"id":"a","from":"a","to":"target","link_type":"self_transfer"},{"id":"b","from":"b","to":"target","link_type":"self_transfer"}]},"allocations":{},"source_mix":[],"explain_gates":{"blockers":[],"warnings":[]}}"#).unwrap();
    source_funds_projection::add_shape(&mut report).unwrap();
    let first = report.clone();
    source_funds_projection::add_shape(&mut report).unwrap();
    assert_eq!(report, first);
    let untied = dumps(&report, &Options::new().compact()).unwrap().replacen(
        r#""label":"same""#,
        r#""label":"distinct""#,
        1,
    );
    let mut untied = kassiber_pytext::loads(&untied).unwrap();
    source_funds_projection::add_shape(&mut untied).unwrap();
}

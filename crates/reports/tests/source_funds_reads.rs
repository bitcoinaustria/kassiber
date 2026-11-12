use kassiber_envelope::{dict, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::Book;

#[test]
fn source_funds_stored_reads_pin_editor_disclosure_scope_order_and_pagination() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/source_funds_reads.json")).unwrap();
    assert!(cases.len() >= 180);
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    for case in cases {
        conn.execute_batch("SAVEPOINT sof_case").unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
        let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
        let command = case["command"].as_str().unwrap();
        let result = kassiber_reports::execute(conn, command, &args);
        let actual = match result {
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
        };
        let options = Options::new().sort_keys(true).compact();
        assert_eq!(
            dumps(&actual, &options).unwrap(),
            case["expected_json"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        conn.execute_batch("ROLLBACK TO sof_case;RELEASE sof_case")
            .unwrap();
    }
}

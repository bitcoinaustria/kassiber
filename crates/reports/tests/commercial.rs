use kassiber_envelope::{dict, s};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::Connection, Book};

struct NoSources;
impl kassiber_setup::Sources for NoSources {
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        panic!("commercial export requested entropy")
    }
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        panic!("commercial export requested a clock")
    }
}
fn encoded(value: &Value) -> serde_json::Value {
    serde_json::from_str(&kassiber_pyjson::dumps(value, &Default::default()).unwrap()).unwrap()
}
#[test]
fn reviewed_commercial_subledger_python_vectors() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/commercial.json")).unwrap();
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    for case in cases {
        conn.execute_batch("SAVEPOINT fixture").unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
        let before = conn.total_changes();
        let mut args = kassiber_pytext::loads(&case["args"].to_string()).unwrap();
        let expected = &case["expected"];
        let result = kassiber_reports::execute(conn, "reports.commercial-subledger", &args);
        match result {
            Ok(rows) => {
                assert_eq!(encoded(&rows), expected["ok"], "{}", case["name"]);
                if let Value::Dict(fields) = &mut args {
                    fields.push((
                        "file".into(),
                        s(root
                            .path()
                            .join("export.csv")
                            .to_string_lossy()
                            .into_owned()),
                    ));
                }
                let result = kassiber_reports::export_request::execute(
                    conn,
                    "reports.export-commercial-subledger-csv",
                    &args,
                    root.path(),
                    &NoSources,
                )
                .unwrap();
                let actual = kassiber_pyjson::dumps(&result, &Default::default())
                    .unwrap()
                    .replace(root.path().to_str().unwrap(), "<root>");
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&actual).unwrap(),
                    expected["exported"],
                    "{}",
                    case["name"]
                );
                assert_eq!(
                    std::fs::read(root.path().join("export.csv")).unwrap(),
                    expected["csv"].as_str().unwrap().as_bytes(),
                    "{}",
                    case["name"]
                );
            }
            Err(e) => {
                let actual = dict([
                    ("code", s(e.code)),
                    ("message", s(e.message)),
                    ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                    ("details", e.details.unwrap_or(Value::Null)),
                    ("retryable", Value::Bool(e.retryable)),
                ]);
                assert_eq!(encoded(&actual), expected["error"], "{}", case["name"]);
            }
        }
        assert_eq!(
            conn.total_changes(),
            before,
            "commercial evidence changed the book"
        );
        conn.execute_batch("ROLLBACK TO fixture; RELEASE fixture")
            .unwrap();
    }
}

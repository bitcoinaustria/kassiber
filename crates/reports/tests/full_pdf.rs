//! The full text PDF: Python's report model and its PDF 1.4 bytes.
use kassiber_envelope::{dict, s};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::Connection, Book};

struct Fixed;
impl kassiber_setup::Sources for Fixed {
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok("2026-01-02T03:04:05Z".into())
    }
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        panic!("PDF preparation requested an identity")
    }
}

fn error(e: kassiber_envelope::AppError) -> serde_json::Value {
    let value = dict([(
        "error",
        dict([
            ("code", s(e.code)),
            ("message", s(e.message)),
            ("hint", e.hint.map(s).unwrap_or(Value::Null)),
            ("details", e.details.unwrap_or(Value::Null)),
            ("retryable", Value::Bool(e.retryable)),
        ]),
    )]);
    serde_json::from_str(&kassiber_pyjson::dumps(&value, &Default::default()).unwrap()).unwrap()
}

#[test]
fn full_pdf_model_and_bytes_match_python() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/full_pdf.json")).unwrap();
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    let mut pages = std::collections::BTreeSet::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        conn.execute_batch("SAVEPOINT oracle_fixture").unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
        let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
        let expected = &case["expected"];
        match kassiber_reports::artifact_csv::prepare(conn, "reports.export-pdf", &args, &Fixed) {
            Ok(prepared) => {
                let ok = &expected["ok"];
                assert!(ok.is_object(), "{name}: Python failed with {expected}");
                let [file] = prepared.files.as_slice() else {
                    panic!("{name}: one PDF")
                };
                let pdf: Vec<u8> = ok["pdf"]
                    .as_str()
                    .unwrap()
                    .chars()
                    .map(|c| c as u8)
                    .collect();
                assert!(file.bytes == pdf, "{name}: PDF bytes differ");
                let metadata =
                    kassiber_pyjson::dumps(&prepared.metadata, &Default::default()).unwrap();
                let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
                assert_eq!(metadata["pages"], ok["pages"], "{name}");
                assert_eq!(metadata["title"], ok["title"], "{name}");
                let args = prepared_args(&args);
                let (workspace, profile) = kassiber_setup::references::scope(conn, &args).unwrap();
                let (title, lines) =
                    kassiber_reports::full_pdf::model(conn, &workspace, &profile, &args, &Fixed)
                        .unwrap();
                assert_eq!(title, ok["title"].as_str().unwrap(), "{name}");
                let expected_lines: Vec<&str> = ok["lines"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|l| l.as_str().unwrap())
                    .collect();
                assert_eq!(lines, expected_lines, "{name}: report model");
                pages.insert(ok["pages"].as_u64().unwrap());
            }
            Err(e) => assert_eq!(error(e), *expected, "{name}"),
        }
        conn.execute_batch("ROLLBACK TO oracle_fixture; RELEASE oracle_fixture")
            .unwrap();
    }
    // The fixtures reach single and multi-page documents.
    assert!(pages.len() >= 3, "{pages:?}");
}

/// The history limit after the export's own `int()` admission.
fn prepared_args(args: &Value) -> Value {
    let mut args = args.clone();
    if let Value::Dict(fields) = &mut args {
        for (k, v) in fields.iter_mut() {
            if *k == "history_limit".into() {
                *v = match &*v {
                    Value::Null => Value::Null,
                    Value::Bool(b) => Value::Int(i64::from(*b).into()),
                    Value::Float(f) => Value::Int((f.trunc() as i64).into()),
                    Value::Str(raw) => {
                        Value::Int(kassiber_pytext::parse_int(raw.as_str().unwrap()).unwrap())
                    }
                    other => other.clone(),
                };
            }
        }
    }
    args
}

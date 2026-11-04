//! The full report in the shared design: Python's report model, refusals
//! and a deterministic rendering.
use kassiber_envelope::{dict, s};
use kassiber_pyjson::Value;
use kassiber_reports::report_design;
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
fn full_report_model_matches_python_and_renders_deterministically() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/full_pdf.json")).unwrap();
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    let mut rendered = 0;
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
                let limited = prepared_args(&args);
                let (workspace, profile) =
                    kassiber_setup::references::scope(conn, &limited).unwrap();
                let model =
                    kassiber_reports::full_pdf::model(conn, &workspace, &profile, &limited, &Fixed)
                        .unwrap();
                let document = kassiber_reports::report_design::full(&model);
                let model: serde_json::Value = serde_json::from_str(
                    &kassiber_pyjson::dumps(&model, &Default::default()).unwrap(),
                )
                .unwrap();
                assert_eq!(model, ok["model"], "{name}: report model");
                assert_eq!(
                    disposal_groups(&document),
                    python_groups(&ok["model"]),
                    "{name}: disposals per wallet and asset"
                );
                let [file] = prepared.files.as_slice() else {
                    panic!("{name}: one PDF")
                };
                assert!(file.bytes.starts_with(b"%PDF-"), "{name}");
                let again = kassiber_reports::artifact_csv::prepare(
                    conn,
                    "reports.export-pdf",
                    &args,
                    &Fixed,
                )
                .unwrap();
                assert!(
                    again.files[0].bytes == file.bytes,
                    "{name}: PDF bytes repeat"
                );
                let metadata =
                    kassiber_pyjson::dumps(&prepared.metadata, &Default::default()).unwrap();
                let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
                assert_eq!(metadata["title"], ok["title"], "{name}");
                rendered += 1;
            }
            Err(e) => assert_eq!(error(e), *expected, "{name}"),
        }
        conn.execute_batch("ROLLBACK TO oracle_fixture; RELEASE oracle_fixture")
            .unwrap();
    }
    assert!(rendered >= 40, "{rendered}");
}

/// Python's capital-gain buckets: first-seen (wallet, asset) order and row
/// counts.
fn python_groups(model: &serde_json::Value) -> Vec<(String, String, i64)> {
    let mut groups: Vec<(String, String, i64)> = Vec::new();
    for row in model["capital"].as_array().unwrap() {
        let key = (
            row["wallet"].as_str().unwrap(),
            row["asset"].as_str().unwrap(),
        );
        match groups
            .iter_mut()
            .find(|g| (g.0.as_str(), g.1.as_str()) == key)
        {
            Some(group) => group.2 += 1,
            None => groups.push((key.0.into(), key.1.into(), 1)),
        }
    }
    groups
}
fn disposal_groups(document: &report_design::Document) -> Vec<(String, String, i64)> {
    use report_design::{Block, Cell};
    let rows = document
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .find_map(|b| match b {
            Block::Table(t) if t.id == "disposals_by_wallet" => Some(t.rows.clone()),
            _ => None,
        })
        .unwrap_or_default();
    rows.iter()
        .map(|row| match (&row[0], &row[1], &row[2]) {
            (Cell::Text(w), Cell::Text(a), Cell::Count(n)) => (w.clone(), a.clone(), *n),
            other => panic!("{other:?}"),
        })
        .collect()
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

//! The source-of-funds report in the shared design: Python's saved-case
//! model and refusals, the flow diagram and a deterministic rendering.
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
fn source_funds_model_matches_python_and_renders_deterministically() {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("data/source_funds_pdf.json")).unwrap();
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    let command = "reports.export-source-funds-pdf";
    let mut rendered = 0;
    for case in vectors["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        conn.execute_batch("SAVEPOINT oracle_fixture").unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        conn.execute_batch(vectors["base_sql"].as_str().unwrap())
            .unwrap();
        conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
        let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
        let expected = &case["expected"];
        match kassiber_reports::artifact_csv::prepare(conn, command, &args, &Fixed) {
            Ok(prepared) => {
                let ok = &expected["ok"];
                assert!(ok.is_object(), "{name}: Python failed with {expected}");
                let model = kassiber_reports::source_funds_pdf::model(conn, &args, &Fixed).unwrap();
                let document = report_design::source_funds(&model);
                let model: serde_json::Value = serde_json::from_str(
                    &kassiber_pyjson::dumps(&model, &Default::default()).unwrap(),
                )
                .unwrap();
                assert_eq!(model, ok["model"], "{name}: report model");
                check_flow(&document, &ok["model"]["report"], name);
                let [file] = prepared.files.as_slice() else {
                    panic!("{name}: one PDF")
                };
                assert!(file.bytes.starts_with(b"%PDF-"), "{name}");
                let again =
                    kassiber_reports::artifact_csv::prepare(conn, command, &args, &Fixed).unwrap();
                assert!(
                    again.files[0].bytes == file.bytes,
                    "{name}: PDF bytes repeat"
                );
                rendered += 1;
            }
            Err(e) => assert_eq!(error(e), *expected, "{name}"),
        }
        conn.execute_batch("ROLLBACK TO oracle_fixture; RELEASE oracle_fixture")
            .unwrap();
    }
    assert!(rendered >= 7, "{rendered}");
}

/// The diagram draws Python's visible levels: the same columns, and every
/// edge whose ends are drawn.
fn check_flow(document: &report_design::Document, report: &serde_json::Value, name: &str) {
    let flow = document
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .find_map(|b| match b {
            report_design::Block::Flow(f) => Some(f.clone()),
            _ => None,
        });
    let levels = report["simplified_flow"]["levels"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let Some(flow) = flow else {
        assert!(levels.is_empty(), "{name}: no diagram");
        return;
    };
    let detailed = report["report_options"]["diagram_detail"] == "detailed";
    let max_levels = if detailed { 16 } else { 6 };
    assert_eq!(
        flow.columns.len(),
        levels.len().min(max_levels),
        "{name}: columns"
    );
    assert_eq!(
        flow.links.len(),
        report["simplified_flow"]["edges"]
            .as_array()
            .map_or(0, |e| e.len()),
        "{name}: links"
    );
}

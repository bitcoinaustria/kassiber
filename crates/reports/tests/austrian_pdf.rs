//! The Austrian E 1kv report in the shared design: Python's report model,
//! refusals, the section column and a deterministic rendering.
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
fn austrian_model_matches_python_and_renders_deterministically() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/austrian_pdf.json")).unwrap();
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
        let command = "reports.export-austrian-e1kv-pdf";
        match kassiber_reports::artifact_csv::prepare(conn, command, &args, &Fixed) {
            Ok(prepared) => {
                let ok = &expected["ok"];
                assert!(ok.is_object(), "{name}: Python failed with {expected}");
                let (workspace, profile) = kassiber_setup::references::scope(conn, &args).unwrap();
                let model = kassiber_reports::austrian_pdf::model(
                    conn, &workspace, &profile, &args, &Fixed,
                )
                .unwrap();
                let document = report_design::austrian(&model);
                let model: serde_json::Value = serde_json::from_str(
                    &kassiber_pyjson::dumps(&model, &Default::default()).unwrap(),
                )
                .unwrap();
                assert_eq!(model, ok["model"], "{name}: report model");
                check_sections(&document, &ok["model"], name);
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
                let metadata =
                    kassiber_pyjson::dumps(&prepared.metadata, &Default::default()).unwrap();
                let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
                assert_eq!(metadata["title"], ok["title"], "{name}");
                assert_eq!(
                    metadata["transactions"].as_u64().unwrap() as usize,
                    ok["model"]["transactions"].as_array().unwrap().len(),
                    "{name}"
                );
                rendered += 1;
            }
            Err(e) => assert_eq!(error(e), *expected, "{name}"),
        }
        conn.execute_batch("ROLLBACK TO oracle_fixture; RELEASE oracle_fixture")
            .unwrap();
    }
    assert!(rendered >= 30, "{rendered}");
}

/// Every detail row of every Austrian section appears once, in section
/// order, in Disposals or Income with its section number; the Kennzahl page
/// indexes all sections.
fn check_sections(document: &report_design::Document, model: &serde_json::Value, name: &str) {
    use report_design::{Block, Cell};
    let table = |id: &str| {
        document
            .sections
            .iter()
            .flat_map(|s| &s.blocks)
            .find_map(|b| match b {
                Block::Table(t) if t.id == id => Some(t.clone()),
                _ => None,
            })
    };
    let report = &model["report"];
    let mut disposals = Vec::new();
    let mut income = Vec::new();
    for id in report["section_order"].as_array().unwrap() {
        let id = id.as_str().unwrap();
        for row in report["sections"][id]["detail_rows"].as_array().unwrap() {
            let entry = (id.to_owned(), row["date"].as_str().unwrap().to_owned());
            if row["entry_type"] == "income" {
                income.push(entry);
            } else {
                disposals.push(entry);
            }
        }
    }
    let rows = |id: &str| -> Vec<(String, String)> {
        table(id)
            .map(|t| {
                t.rows
                    .iter()
                    .map(|r| match (&r[0], &r[1]) {
                        (Cell::Text(s), Cell::Date(d)) => (s.clone(), d.clone()),
                        other => panic!("{other:?}"),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    assert_eq!(rows("disposals"), disposals, "{name}: disposals");
    assert_eq!(rows("income"), income, "{name}: income");
    let index = table("e1kv_sections").unwrap();
    assert_eq!(
        index.rows.len(),
        report["section_order"].as_array().unwrap().len(),
        "{name}"
    );
}

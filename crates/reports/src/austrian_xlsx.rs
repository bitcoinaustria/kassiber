//! The Austrian E 1kv workbook in the shared report design: the same
//! document as the PDF over the same model, as sheets.
use super::*;
use artifact_csv::{list, File, Prepared};
use kassiber_xlsx::Document;

fn project(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<(Value, Document)> {
    // The export transport holds admission through publication and completion.
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    require_context(conn, &profile, false)?;
    let model = austrian_pdf::model(conn, &workspace, &profile, args, sources)?;
    let document = report_design::xlsx_document(&report_design::austrian(&model));
    Ok((value(&model, "report").clone(), document))
}

pub(super) fn prepare(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let (report, document) = project(conn, args, sources)?;
    let bytes = kassiber_xlsx::render(&document)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    Ok(Prepared {
        files: vec![File {
            name: "report.xlsx".into(),
            sheet: String::new(),
            bytes,
            rows: list(value(&report, "rows")).len(),
        }],
        metadata: dict([
            ("form", value(&report, "form").clone()),
            ("tax_year", value(&report, "tax_year").clone()),
            (
                "sheets",
                Value::List(
                    document
                        .sheets
                        .iter()
                        .map(|sheet| s(sheet.name.as_str()))
                        .collect(),
                ),
            ),
            (
                "rows",
                Value::Int(list(value(&report, "rows")).len().into()),
            ),
            (
                "summary_rows",
                Value::Int(list(value(&report, "summary_rows")).len().into()),
            ),
        ]),
        completion: Some(export_completion::Completion {
            kind: "austrian-e1kv.xlsx".into(),
            year: Some(value(&report, "tax_year").clone()),
            end_year: None,
            scope: Value::Null,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixed;
    impl kassiber_setup::Sources for Fixed {
        fn now(&self, _: &Connection) -> Result<String> {
            Ok("2026-01-02T03:04:05Z".into())
        }
        fn uuid(&self, _: &Connection) -> Result<String> {
            Ok("00000000-0000-4000-8000-000000000001".into())
        }
    }
    // Resolve styles and compare operations independently of write order and
    // format indices. Openpyxl process tests additionally compare rendered cells.
    /// Python's refusals, and for every accepted case the shared design's
    /// sheets over the Austrian model (pinned to Python in
    /// tests/austrian_pdf.rs).
    #[test]
    fn austrian_xlsx_refusals_match_python_and_sheets_follow_the_design() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/austrian_xlsx.json")).unwrap();
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        for case in cases.as_array().unwrap() {
            conn.execute_batch("SAVEPOINT xlsx_vector").unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let result = project(conn, &args, &Fixed);
            if case["expected"].get("error").is_some() {
                let error = result.unwrap_err();
                assert_eq!(
                    error.code,
                    case["expected"]["error"]["code"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
                assert_eq!(
                    error.message,
                    case["expected"]["error"]["message"].as_str().unwrap(),
                    "{}",
                    case["name"]
                );
            } else {
                let (_, document) = result.unwrap();
                let names: Vec<&str> = document.sheets.iter().map(|s| s.name.as_str()).collect();
                assert_eq!(names[0], "summary", "{}", case["name"]);
                for sheet in ["e1kv_kennzahlen", "e1kv_sections", "assumptions"] {
                    assert!(names.contains(&sheet), "{}: {sheet}", case["name"]);
                }
                assert!(kassiber_xlsx::render(&document).is_ok(), "{}", case["name"]);
            }
            conn.execute_batch("ROLLBACK TO xlsx_vector; RELEASE xlsx_vector")
                .unwrap();
        }
    }
    #[test]
    fn austrian_xlsx_export_hashes_actual_bytes_and_saves_without_filing() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        conn.execute_batch("UPDATE profiles SET tax_country='at' WHERE id='p'")
            .unwrap();
        let target = root.path().join("annual.xlsx");
        let result = export::export(
            conn,
            "reports.export-austrian-e1kv-xlsx",
            &dict([("year", Value::Int(2025.into()))]),
            &target,
            &Fixed,
        )
        .unwrap();
        assert!(std::fs::read(&target).unwrap().starts_with(b"PK"));
        let snapshot = value(&result, "report_snapshot");
        assert_eq!(text(snapshot, "report_state"), "saved");
        assert_eq!(text(snapshot, "report_kind"), "austrian-e1kv.xlsx");
        assert_eq!(int(snapshot, "period_start_year"), 2025);
        assert_eq!(int(snapshot, "period_end_year"), 2025);
        assert_eq!(
            text(snapshot, "content_sha256"),
            export_completion::content_sha256(&[target]).unwrap()
        );
        assert_eq!(
            rows(
                conn,
                "SELECT id FROM filed_report_snapshots WHERE report_state='filed'",
                []
            )
            .unwrap()
            .len(),
            0
        );
    }
    #[test]
    fn austrian_xlsx_managed_request_preserves_format_and_freshness() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        conn.execute_batch("UPDATE profiles SET tax_country='at' WHERE id='p'")
            .unwrap();
        let args = dict([("year", Value::Int(2025.into()))]);
        let result = export_request::execute(
            conn,
            "ui.reports.export_austrian_e1kv_xlsx",
            &args,
            root.path(),
            &Fixed,
        )
        .unwrap();
        assert_eq!(
            text(&result, "filename"),
            "kassiber-austrian-e1kv-2025-20260102-030405.xlsx"
        );
        assert_eq!(text(&result, "format"), "xlsx");
        assert_eq!(text(&result, "scope"), "austrian_e1kv");
        assert!(std::fs::read(text(&result, "file"))
            .unwrap()
            .starts_with(b"PK"));
        conn.execute_batch("INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,blocking_reports,updated_at) VALUES('p','source','wallet','Cold','error',1,'fixed')").unwrap();
        assert_eq!(
            export_request::execute(
                conn,
                "ui.reports.export_austrian_e1kv_xlsx",
                &args,
                root.path(),
                &Fixed
            )
            .unwrap_err()
            .code,
            "report_freshness_blocked"
        );
        assert_eq!(
            std::fs::read_dir(root.path().join("reports"))
                .unwrap()
                .count(),
            1
        );
    }
}

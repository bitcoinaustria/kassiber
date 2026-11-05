use super::*;
use artifact_csv::{File, Prepared};
use kassiber_xlsx::Document;

/// The full report workbook: the shared design's sheets for the full
/// report's model, then the verification appendix when asked for.
fn project(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<(Prepared, Document)> {
    let mut args = args.clone();
    let limit = artifact_csv::history_limit(value(&args, "history_limit"))?;
    replace(&mut args, "history_limit", limit);
    let (workspace, profile) = kassiber_setup::references::scope(conn, &args)?;
    require_context(conn, &profile, false)?;
    let model = full_pdf::model(conn, &workspace, &profile, &args, sources)?;
    let design = report_design::full(&model);
    let mut document = report_design::xlsx_document(&design);
    let wallet = if truthy(value(&args, "wallet")) {
        Some(kassiber_wallets::resolve(
            conn,
            &text(&profile, "id"),
            &text(&args, "wallet"),
        )?)
    } else {
        None
    };
    let verify = get(&args, "verify").is_none_or(truthy);
    if verify {
        verification_xlsx::append(
            &mut document,
            &profile,
            wallet.as_ref().map(|w| w.label.as_str()),
            &text(&model, "generated_at"),
            &verification::project(conn, &profile)?,
        )?;
    }
    // Python's title names the report's scope; the workbook's own title is
    // the design's.
    let scope = match &wallet {
        Some(wallet) => wallet.label.clone(),
        None => text(&profile, "label"),
    };
    let rows: usize = design
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .map(|b| match b {
            report_design::Block::Table(t) => t.rows.len(),
            _ => 0,
        })
        .sum();
    let prepared = Prepared {
        files: vec![File {
            name: "report.xlsx".into(),
            sheet: String::new(),
            bytes: Vec::new(),
            rows,
        }],
        metadata: dict([
            ("title", s(format!("Kassiber Report - {scope}"))),
            (
                "wallet",
                if truthy(value(&args, "wallet")) {
                    value(&args, "wallet").clone()
                } else {
                    s("")
                },
            ),
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
            ("verified", Value::Bool(verify)),
            ("rows", Value::Int(rows.into())),
        ]),
        completion: Some(crate::export_completion::Completion {
            kind: "full-report.xlsx".into(),
            year: None,
            end_year: None,
            scope: match &wallet {
                Some(wallet) => dict([("wallet_ids", Value::List(vec![s(wallet.id.as_str())]))]),
                None => Value::Null,
            },
        }),
    };
    Ok((prepared, document))
}

pub(super) fn prepare(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let (mut prepared, document) = project(conn, args, sources)?;
    let bytes = kassiber_xlsx::render(&document)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    prepared.files = vec![File {
        name: "report.xlsx".into(),
        sheet: String::new(),
        bytes,
        rows: prepared.files[0].rows,
    }];
    Ok(prepared)
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
    fn operations(doc: &Document) -> Vec<(String, Vec<String>)> {
        doc.sheets
            .iter()
            .map(|sheet| {
                let mut operations: Vec<String> = sheet
                    .operations
                    .iter()
                    .map(|op| {
                        let mut json = serde_json::to_value(op).unwrap();
                        if let Some(index) = json.get("style").and_then(|n| n.as_u64()) {
                            json["style"] =
                                serde_json::to_value(&doc.formats[index as usize]).unwrap();
                        }
                        serde_json::to_string(&json).unwrap()
                    })
                    .collect();
                operations.sort();
                (sheet.name.clone(), operations)
            })
            .collect()
    }
    fn normalize_numbers(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Number(number)
                if number.as_u64().is_none() && number.as_i64().is_none() =>
            {
                *number = serde_json::Number::from_f64(number.as_f64().unwrap()).unwrap();
            }
            serde_json::Value::Array(values) => values.iter_mut().for_each(normalize_numbers),
            serde_json::Value::Object(fields) => fields.values_mut().for_each(normalize_numbers),
            _ => (),
        }
    }
    #[test]
    fn full_xlsx_errors_and_verification_appendix_match_python() {
        let mut cases: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/full_xlsx.json")).unwrap();
        normalize_numbers(&mut cases);
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
                // The report sheets are the shared design over the full
                // report's model (pinned to Python in tests/full_pdf.rs);
                // the verification appendix stays Python's, formulas and all.
                let (_, actual) = result.unwrap();
                let mut expected: Document =
                    serde_json::from_value(case["expected"]["document"].clone()).unwrap();
                // In the shared design's palette.
                expected.formats = expected
                    .formats
                    .into_iter()
                    .map(verification_xlsx::shared_style)
                    .collect();
                let appendix = [
                    "Verify",
                    "Acquisitions",
                    "Disposals",
                    "Control",
                    "Evidence",
                    "Quarantined",
                ];
                let pick = |doc: &Document| {
                    operations(doc)
                        .into_iter()
                        .filter(|(name, _)| appendix.contains(&name.as_str()))
                        .collect::<Vec<_>>()
                };
                let actual_sheets: Vec<String> =
                    actual.sheets.iter().map(|s| s.name.clone()).collect();
                assert_eq!(actual_sheets[0], "summary", "{}", case["name"]);
                let actual = pick(&actual);
                let expected = pick(&expected);
                assert_eq!(actual.len(), expected.len(), "{}", case["name"]);
                for ((name, actual), (wanted, expected)) in actual.iter().zip(&expected) {
                    assert_eq!(name, wanted, "{}", case["name"]);
                    assert_eq!(actual.len(), expected.len(), "{} {name}", case["name"]);
                    for (actual, expected) in actual.iter().zip(expected) {
                        assert_eq!(actual, expected, "{} {name}", case["name"]);
                    }
                }
            }
            conn.execute_batch("ROLLBACK TO xlsx_vector; RELEASE xlsx_vector")
                .unwrap();
        }
    }
    #[test]
    fn full_xlsx_export_hashes_actual_bytes_and_saves_without_filing() {
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
            "reports.export-xlsx",
            &dict::<&str>([]),
            &target,
            &Fixed,
        )
        .unwrap();
        assert!(std::fs::read(&target).unwrap().starts_with(b"PK"));
        let snapshot = value(&result, "report_snapshot");
        assert_eq!(text(snapshot, "report_state"), "saved");
        assert_eq!(text(snapshot, "report_kind"), "full-report.xlsx");
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
    fn full_xlsx_managed_verify_argument_is_boolean() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        for verify in [Value::Null, Value::Int(1.into()), s("false")] {
            let error = export_request::execute(
                conn,
                "ui.reports.export_xlsx",
                &dict([("verify", verify)]),
                root.path(),
                &Fixed,
            )
            .unwrap_err();
            assert_eq!(error.code, "validation");
            assert_eq!(
                error.message,
                "ui.reports.export_xlsx verify must be a boolean"
            );
        }
        assert!(!root.path().join("reports").exists());
    }

    #[test]
    fn full_xlsx_managed_request_preserves_format_and_freshness() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        conn.execute_batch("UPDATE profiles SET tax_country='at' WHERE id='p'")
            .unwrap();
        let args = dict::<&str>([]);
        let result =
            export_request::execute(conn, "ui.reports.export_xlsx", &args, root.path(), &Fixed)
                .unwrap();
        assert_eq!(
            text(&result, "filename"),
            "kassiber-report-20260102-030405.xlsx"
        );
        assert_eq!(text(&result, "format"), "xlsx");
        assert_eq!(text(&result, "scope"), "report");
        assert!(std::fs::read(text(&result, "file"))
            .unwrap()
            .starts_with(b"PK"));
        conn.execute_batch("INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,blocking_reports,updated_at) VALUES('p','source','wallet','Cold','error',1,'fixed')").unwrap();
        assert_eq!(
            export_request::execute(conn, "ui.reports.export_xlsx", &args, root.path(), &Fixed)
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

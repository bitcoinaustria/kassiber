//! The Austrian E 1kv CSV bundle in the shared design's machine format: the
//! Austrian document's tables, one CSV each, and `manifest.json`.
use super::*;
use artifact_csv::{list, File, Prepared};

pub(super) fn prepare(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let model = austrian_pdf::model(conn, workspace, profile, args, sources)?;
    let report = value(&model, "report");
    let design = report_design::austrian(&model);
    let rows: Vec<(&str, usize)> = design
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .filter_map(|b| match b {
            report_design::Block::Table(t) => Some((t.id, t.rows.len())),
            _ => None,
        })
        .collect();
    let files: Vec<File> = report_design::csv(&design)?
        .into_iter()
        .map(|(name, bytes)| {
            let id = name.trim_end_matches(".csv").to_owned();
            let count = rows
                .iter()
                .find(|(table, _)| *table == id)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            File {
                sheet: id,
                name,
                bytes,
                rows: count,
            }
        })
        .collect();
    Ok(Prepared {
        metadata: dict([
            ("form", value(report, "form").clone()),
            ("tax_year", value(report, "tax_year").clone()),
            (
                "sheets",
                Value::List(files.iter().map(|f| s(f.sheet.as_str())).collect()),
            ),
            ("rows", Value::Int(list(value(report, "rows")).len().into())),
            (
                "summary_rows",
                Value::Int(list(value(report, "summary_rows")).len().into()),
            ),
        ]),
        files,
        completion: Some(crate::export_completion::Completion {
            kind: "austrian-e1kv.csv-bundle".into(),
            year: Some(value(report, "tax_year").clone()),
            end_year: None,
            scope: Value::Null,
        }),
    })
}

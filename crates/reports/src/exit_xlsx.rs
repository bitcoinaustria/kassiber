//! The exit-tax workbook in the shared report design: the same document as
//! the PDF over the same exit-tax report, as sheets.
use super::*;
use artifact_csv::{list, File, Prepared};
use kassiber_xlsx::Document;

pub(super) fn project(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<(Value, Document)> {
    let model = exit_pdf::model(conn, args, sources)?;
    let report = value(&model, "report");
    let document = report_design::xlsx_document(&report_design::exit_tax(&model));
    let metadata = dict([
        ("scope", s("exit_tax")),
        ("format", s("xlsx")),
        ("departure_date", value(report, "departureDate").clone()),
        ("destination", value(report, "destination").clone()),
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
        ("rows", Value::Int(list(value(report, "lots")).len().into())),
    ]);
    Ok((metadata, document))
}

pub(super) fn prepare(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let (metadata, document) = project(conn, args, sources)?;
    let bytes = kassiber_xlsx::render(&document)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    Ok(Prepared {
        files: vec![File {
            name: "report.xlsx".into(),
            sheet: "".into(),
            bytes,
            rows: int(&metadata, "rows") as usize,
        }],
        metadata,
        // Python registers no saved marker for the exit-tax estimate.
        completion: None,
    })
}

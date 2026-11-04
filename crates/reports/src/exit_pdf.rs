//! The exit-tax estimate as the hand-written text PDF (`export_exit_tax_pdf_report`).
use super::*;
use artifact_csv::{File, Prepared};

pub(super) fn prepare(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    require_context(conn, &profile, false)?;
    let report = exit_tax::report(conn, &workspace, &profile, args, sources)?;
    let lines = exit_tax::lines(&report)?;
    let title = format!(
        "Exit-tax estimate — {} ({})",
        text(&report, "profile"),
        text(&report, "departureDate")
    );
    let rendered = kassiber_pdf::text::render(&title, &lines);
    Ok(Prepared {
        files: vec![File {
            name: "report.pdf".into(),
            sheet: String::new(),
            bytes: rendered.bytes,
            rows: 0,
        }],
        metadata: dict([
            ("pages", Value::Int(rendered.pages.into())),
            ("title", s(title)),
            ("scope", s("exit_tax")),
            ("format", s("pdf")),
            ("departure_date", value(&report, "departureDate").clone()),
            ("destination", value(&report, "destination").clone()),
        ]),
        // Python registers no saved marker for the exit-tax estimate.
        completion: None,
    })
}

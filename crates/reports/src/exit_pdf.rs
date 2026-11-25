//! The exit-tax estimate PDF (`export_exit_tax_pdf_report`) in the shared
//! report design, over the exit-tax report Python's writer formats.
use super::*;
use artifact_csv::{File, Prepared};

/// The exit-tax report and the export clock the design renders.
pub fn model(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    let draft = draft_mode(args);
    let reasons = require_context_admit(conn, &profile, false, draft)?;
    let report = exit_tax::report(conn, &workspace, &profile, args, sources)?;
    let mut out = dict([("report", report), ("generated_at", s(sources.now(conn)?))]);
    if draft {
        replace(
            &mut out,
            "draft_reasons",
            Value::List(reasons.into_iter().map(s).collect()),
        );
    }
    Ok(out)
}

pub(super) fn prepare(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let draft = draft_mode(args);
    let model = model(conn, args, sources)?;
    let mut design = report_design::exit_tax(&model);
    if privacy_mode(args) {
        let (_, profile) = kassiber_setup::references::scope(conn, args)?;
        design.apply_privacy_mode_for(&wealth::wallet_custody_map(conn, &text(&profile, "id"))?);
    }
    if draft {
        let reasons: Vec<String> = artifact_csv::list(value(&model, "draft_reasons"))
            .iter()
            .map(py_str)
            .collect();
        design.apply_draft(&reasons);
    }
    let (bytes, pages) = report_design::pdf(&design)?;
    let report = value(&model, "report");
    let title = format!(
        "Exit-tax estimate — {} ({})",
        text(report, "profile"),
        text(report, "departureDate")
    );
    Ok(Prepared {
        files: vec![File {
            name: "report.pdf".into(),
            sheet: String::new(),
            bytes,
            rows: 0,
        }],
        metadata: dict([
            ("pages", Value::Int(pages.into())),
            ("title", s(title)),
            ("scope", s("exit_tax")),
            ("format", s("pdf")),
            ("departure_date", value(report, "departureDate").clone()),
            ("destination", value(report, "destination").clone()),
        ]),
        // Python registers no saved marker for the exit-tax estimate.
        completion: None,
    })
}

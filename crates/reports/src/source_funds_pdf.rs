//! The source-of-funds PDF (`export_pdf` in Python's source-funds core) in
//! the shared report design. Only a saved, exportable case snapshot renders.
use super::*;
use artifact_csv::{File, Prepared};
use sha2::{Digest, Sha256};

/// `_snapshot_hash`: SHA-256 of the snapshot's sorted, compact JSON.
pub(super) fn snapshot_hash(snapshot: &Value) -> Result<String> {
    let compact = kassiber_pyjson::dumps(
        &kassiber_envelope::json_ready(snapshot.clone()),
        &kassiber_pyjson::Options::new().sort_keys(true).compact(),
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(compact.as_bytes())))
}

/// `load_case_snapshot`: the saved case's snapshot, its stored hash checked.
fn load_case(conn: &Connection, profile: &Value, case: &str) -> Result<Value> {
    let found = rows(
        conn,
        "SELECT snapshot_json,snapshot_hash FROM source_funds_cases WHERE profile_id=? AND id=?",
        [&text(profile, "id"), case],
    )?;
    let Some(row) = found.first() else {
        return Err(AppError::new(
            "not_found",
            format!("Source-funds case '{case}' not found"),
        ));
    };
    let snapshot = kassiber_pytext::loads(&text(row, "snapshot_json"))
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    if snapshot_hash(&snapshot)? != text(row, "snapshot_hash") {
        return Err(AppError::new(
            "snapshot_integrity_error",
            "Source-funds case snapshot hash does not match stored payload",
        ));
    }
    Ok(snapshot)
}

/// The saved case, admitted for export the way Python admits it, with its
/// hash and the export clock.
pub fn model(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    admit(conn, args, sources, false)
}

/// `model` for the PDF or, with `bundle`, the bundle, whose refusals name it.
pub(super) fn admit(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
    bundle: bool,
) -> Result<Value> {
    let case = value(args, "case");
    if !truthy(case) {
        return Err(AppError::new(
            "validation",
            format!(
                "export-source-funds-{} requires --case from a saved source-funds preview",
                if bundle { "bundle" } else { "pdf" }
            ),
        )
        .with_hint(format!(
            "Run `reports source-funds --save-case ...` first, review the disclosure preview, then export that case id{}.",
            if bundle { " as a bundle" } else { "" }
        )));
    }
    let (_, profile) = kassiber_setup::references::scope(conn, args)?;
    let report = load_case(conn, &profile, &py_str(case))?;
    if artifact_csv::list(value(&report, "source_mix"))
        .iter()
        .any(|row| matches!(value(row, "asset"), Value::Null))
    {
        return Err(AppError::new(
            "source_funds_snapshot_outdated",
            "This source-funds case was saved before source assets and gross upstream demand were recorded",
        )
        .with_hint("Re-run the report for this target and save the case again, then export the new case."));
    }
    let gates = value(&report, "explain_gates");
    if !truthy(value(gates, "exportable")) {
        return Err(AppError::new(
            "export_blocked",
            format!(
                "Source-of-funds {} export is blocked by unresolved review gates",
                if bundle { "bundle" } else { "PDF" }
            ),
        )
        .with_hint("Run `reports source-funds --machine ...` and resolve every explain_gates.blockers item.")
        .with_details(dict([("blockers", value(gates, "blockers").clone())])));
    }
    let hash = snapshot_hash(&report)?;
    Ok(dict([
        ("report", report),
        ("snapshot_hash", s(hash)),
        ("generated_at", s(sources.now(conn)?)),
    ]))
}

pub(super) fn prepare(
    conn: &Connection,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    let model = model(conn, args, sources)?;
    let document = report_design::source_funds(&model);
    let (bytes, pages) = report_design::pdf(&document)?;
    let report = value(&model, "report");
    let context = value(report, "report_context");
    let title = match text(context, "report_title") {
        t if t.is_empty() || t == "None" => "Source of Funds Report".to_owned(),
        t => t,
    };
    let purpose = match get(value(report, "purpose"), "type") {
        Some(v) => v.clone(),
        None => s("existing_transaction"),
    };
    Ok(Prepared {
        files: vec![File {
            name: "source-funds.pdf".into(),
            sheet: String::new(),
            bytes,
            rows: 0,
        }],
        metadata: dict([
            ("pages", Value::Int(pages.into())),
            ("title", s(format!("Kassiber {title}"))),
            ("renderer", s("typst")),
            ("scope", s("source_funds")),
            ("format", s("pdf")),
            ("snapshot_hash", value(&model, "snapshot_hash").clone()),
            ("reveal_mode", value(report, "reveal_mode").clone()),
            ("purpose", purpose),
            (
                "target_transaction_id",
                value(value(report, "target"), "transaction_id").clone(),
            ),
            ("exportable", Value::Bool(true)),
        ]),
        // Python registers no saved marker for a source-of-funds export.
        completion: None,
    })
}

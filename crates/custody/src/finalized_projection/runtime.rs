//! Raw-stage finalized-projection adapter; no persistence authority.
use crate::{codec, graph::stored, interpretation, snapshot::Snapshot};
use kassiber_pyjson::Value as Py;
use serde_json::{json, Map, Value};

fn exact(value: &Py) -> Result<Value, String> {
    let converted = codec::to_json(value);
    if codec::dumps(value, true)? != codec::dumps(&codec::from_json(&converted)?, true)? {
        return Err("ValueError".into());
    }
    Ok(converted)
}

fn pools(value: &Py) -> Result<Value, String> {
    let mut value = exact(value)?;
    for pair in value.as_array_mut().ok_or("TypeError")? {
        pair[0].as_object_mut().ok_or("TypeError")?.remove("$type");
    }
    Ok(value)
}

/// Capture the original row population before rates and interpreter admission.
pub fn confirmation_rows(snapshot: &Snapshot) -> Result<Value, String> {
    let mut rows = Map::new();
    for raw in &snapshot.transactions {
        let mut row = json!(raw);
        row["_closed_confirmation_observation"] = interpretation::capture_confirmation(&row)?;
        let id = if codec::truth(&row["id"]) {
            codec::pystr(&row["id"])
        } else {
            String::new()
        };
        rows.insert(id, row);
    }
    Ok(Value::Object(rows))
}

/// Assemble Python compiler operands from previously computed Rust stages.
/// Interpreter diagnostics bypass serde, including grouping and sorting.
pub fn stage_with_rows(
    snapshot: &Snapshot,
    rows: &Value,
    quantity: &Py,
    interpreter: &Py,
) -> Result<Py, String> {
    let projection = stored::get(quantity, "projection");
    let eligibility = stored::get(quantity, "tax_eligibility");
    let eligible = stored::list(stored::get(eligibility, "eligible_decisions"))
        .iter()
        .map(|d| exact(stored::get(d, "source")))
        .collect::<Result<Vec<_>, _>>()?;
    let canonical = stored::get(quantity, "canonical_input");
    let mut blocked_anchors = Vec::new();
    for issue in stored::list(stored::get(quantity, "issues")) {
        let component = match stored::get(issue, "reason") {
            Py::Str(reason) => reason
                .code_points()
                .take("custody_component_".len())
                .eq("custody_component_".chars().map(u32::from)),
            _ => return Err("AttributeError".into()),
        };
        if component
            || matches!(stored::get(issue, "issue_type"), Py::Str(kind) if matches!(kind.as_str(), Some("component_claim_compile_failed" | "quantity_claim_bundle_invalid")))
        {
            blocked_anchors.extend(
                stored::list(stored::get(issue, "transaction_ids"))
                    .iter()
                    .map(exact)
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
    }
    // Interpreter-blocked quantity issues are deliberately ignored by Python's
    // compiler. Their complete lossless records remain in interpreter quarantines.
    let issues = stored::list(stored::get(quantity, "issues"))
        .iter()
        .filter(|i| stored::get(i, "issue_type") != &Py::Str("custody_interpreter_blocked".into()))
        .map(exact)
        .collect::<Result<Vec<_>, _>>()?;
    let mut non_events = exact(stored::get(interpreter, "non_event_transaction_ids"))?
        .as_array()
        .ok_or("TypeError")?
        .clone();
    non_events.extend(
        interpretation::channel_non_event_ids(snapshot)?
            .into_iter()
            .map(Value::String),
    );
    let input = json!({
        "op":"finalize", "profile":snapshot.profile, "rows":rows,
        "observations":exact(stored::get(projection,"observations"))?,
        "decisions":exact(stored::get(projection,"decisions"))?,
        "eligible":eligible,
        "events":exact(stored::get(canonical,"events"))?,
        "pool_barriers":pools(stored::get(eligibility,"pool_barriers"))?,
        "pool_barrier_roots":pools(stored::get(eligibility,"pool_barrier_roots"))?,
        "issues":issues, "blocked_anchor_ids":blocked_anchors,
        "blocked_transaction_ids":exact(stored::get(interpreter,"blocked_transaction_ids"))?,
        "non_event_transaction_ids":non_events,
        "payouts":exact(stored::get(quantity,"reviewed_direct_payouts"))?,
        "reviewed":exact(stored::get(quantity,"reviewed_conversion_pairs"))?
    });
    super::compose_lossless(
        &input,
        stored::list(stored::get(interpreter, "quarantines")),
    )
    .map_err(|error| {
        if error == "AppError" {
            "validation".into()
        } else {
            error
        }
    })
}

pub fn stage(snapshot: &Snapshot, quantity: &Py, interpreter: &Py) -> Result<Py, String> {
    stage_with_rows(
        snapshot,
        &confirmation_rows(snapshot)?,
        quantity,
        interpreter,
    )
}

//! The Python result's insertion order is a transport contract, including joins.
use super::*;
use kassiber_envelope::{dict, set};
use kassiber_pyjson::Value as Py;

fn project(value: &Value, keys: &[&str]) -> Result<Py, Refusal> {
    let map = value
        .as_object()
        .ok_or_else(|| Refusal::new("RuntimeError", "expected journal result mapping"))?;
    if map.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err(Refusal::new("RuntimeError", "unknown journal result field"));
    }
    Ok(dict(
        keys.iter()
            .filter_map(|key| map.get(*key).map(|value| (*key, value)))
            .map(|(key, value)| Ok((key, codec::from_json(value)?)))
            .collect::<Result<Vec<_>, Refusal>>()?,
    ))
}

pub(super) fn payload(value: &Value) -> Result<Py, Refusal> {
    let mut result = project(
        value,
        &[
            "profile",
            "entries_created",
            "quarantined",
            "transfers_detected",
            "cross_asset_pairs",
            "auto_priced",
            "processed_transactions",
            "processed_at",
            "custody_quantity",
            "direct_swap_payouts",
            "warnings",
            "custody_component_blockers",
            "source_overlap_repair",
            "source_overlap_warning",
            "custody_pair_evidence_repairs",
            "filed_report_impacts_resolved",
        ],
    )?;
    let quantity = project(
        &value["custody_quantity"],
        &[
            "postings",
            "issues",
            "balances",
            "decisions",
            "economic_relations",
            "differences",
            "blocked",
            "blocked_from",
        ],
    )?;
    set(&mut result, "custody_quantity", quantity);
    if let Some(warnings) = value.get("warnings") {
        let warnings = crate::array(warnings)
            .iter()
            .map(|warning| project(warning, &["code", "label", "wallet_ids", "message"]))
            .collect::<Result<Vec<_>, _>>()?;
        set(&mut result, "warnings", Py::List(warnings));
    }
    if let Some(blockers) = value.get("custody_component_blockers") {
        let blockers = crate::array(blockers)
            .iter()
            .map(|blocker| {
                project(
                    blocker,
                    &["component_id", "lineage_id", "revision", "issue_codes"],
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        set(
            &mut result,
            "custody_component_blockers",
            Py::List(blockers),
        );
    }
    let sources = prebuild::source_overlap_result_fields(
        value.get("source_overlap_repair"),
        value.get("source_overlap_warning"),
    )?;
    if let Py::Dict(fields) = sources {
        for (key, value) in fields {
            if let kassiber_pyjson::Key::Str(key) = key {
                set(
                    &mut result,
                    key.as_str().expect("source result field"),
                    value,
                );
            }
        }
    }
    Ok(result)
}

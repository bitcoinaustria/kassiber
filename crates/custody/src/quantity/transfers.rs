//! Finalized ownership edges remain visible independently of tax readiness.
use super::{array, domain, exact, number};
use crate::codec;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Private intermediate seam, not raw snapshot admission. Eligibility contains
/// complete decisions: matching only a claim id would grant the wrong slice.
pub fn canonical_internal_transfer_rows(
    projection: &Value,
    eligible: &[Value],
    refs: &Value,
) -> Result<Value, String> {
    let observations: BTreeMap<_, _> = array(&projection["observations"])
        .iter()
        .map(|o| (codec::text(&o["quantity_hash"]), o))
        .collect();
    let mut rows = Vec::new();
    for decision in array(&projection["decisions"]) {
        if !matches!(
            decision["state"].as_str(),
            Some("internal_verified" | "internal_reviewed")
        ) || decision["target"].is_null()
        {
            continue;
        }
        let source = observations
            .get(&codec::text(&decision["source"]["observation_hash"]))
            .ok_or("KeyError")?;
        let target = observations
            .get(&codec::text(&decision["target"]["observation_hash"]))
            .ok_or("KeyError")?;
        let label = |o: &Value| {
            refs.get(codec::text(&o["wallet_id"]))
                .and_then(|r| r.get("label"))
                .unwrap_or(&o["wallet_id"])
                .clone()
        };
        let amount =
            exact(&decision["source"]["end_msat"])? - exact(&decision["source"]["start_msat"])?;
        let scope = domain(*source);
        let mut row = json!({
            "out_transaction_id":source["anchor_transaction_id"],
            "in_transaction_id":target["anchor_transaction_id"],
            "occurred_at":source["occurred_at"],"asset":source["asset"],
            "amount_msat":number(&amount),
            "from_wallet_id":source["wallet_id"],"from_wallet":label(source),
            "to_wallet_id":target["wallet_id"],"to_wallet":label(target),
            "custody_state":decision["state"],
            "basis_state":if eligible.contains(decision) {"eligible"} else {"blocked_by_prior_custody_basis"},
            "evidence_reason":decision["reason"],"network":scope["network"],"rail":scope["rail"]
        });
        for key in ["atomic_bundle_id", "component_id"] {
            if codec::truth(&decision[key]) {
                row[key] = decision[key].clone();
            }
        }
        rows.push((
            (
                codec::text(&row["occurred_at"]),
                codec::text(&row["out_transaction_id"]),
                codec::text(&row["in_transaction_id"]),
                amount,
            ),
            row,
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Value::Array(rows.into_iter().map(|(_, row)| row).collect()))
}

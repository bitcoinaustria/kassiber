//! Composed comparison seam through privacy, raw holds and dependency closure.
//! Channels, native HTLC production, loans and reconciliation remain unported.
use super::{
    array,
    holds::{self, Ids},
    pairs, pipeline, privacy, recorded, samourai, Context,
};
use crate::{codec, snapshot::Snapshot};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn ids(items: &Value) -> Ids {
    array(items).iter().map(codec::text).collect()
}
fn blocked(items: &[Value]) -> Ids {
    items
        .iter()
        .map(|item| codec::text(&item["transaction_id"]))
        .collect()
}

pub(super) fn compile(snapshot: &Snapshot, context: &Context) -> Result<Value, String> {
    let run = || {
        let samourai = samourai::compile(snapshot, context)?;
        let rows: Vec<_> = snapshot.transactions.iter().map(|row| json!(row)).collect();
        let has_chain = array(&context.canonical["events"])
            .iter()
            .any(|event| event["event_key"]["native_namespace"] == "chain");
        let has_hashes = rows
            .iter()
            .any(|row| !row["payment_hash"].is_null() && row["payment_hash"] != "");
        let auto = if has_chain || has_hashes {
            recorded::detect(&rows)?.0
        } else {
            Vec::new()
        };
        let prepared = pipeline::prepare_with_pairs(snapshot, context, rows, auto)?;
        let rows = &prepared.rows;
        let wallets: BTreeMap<_, _> = snapshot
            .wallets
            .iter()
            .map(|wallet| (codec::pystr(&json!(wallet)["id"]), json!(wallet)))
            .collect();
        let derived = &prepared.candidates;
        let privacy = privacy::compute(
            rows,
            &prepared.recorded,
            derived,
            context,
            &wallets,
            &ids(&samourai["candidate_ids"]),
            &Ids::new(),
        )?;
        let derivation_blocked = blocked(&prepared.derivation_quarantines);
        let mut touched = derivation_blocked.clone();
        touched.extend(derived.iter().flat_map(holds::anchors));
        let mut candidates: Vec<_> = prepared
            .recorded
            .iter()
            .filter(|pair| holds::anchors(pair).is_disjoint(&touched))
            .cloned()
            .collect();
        candidates.extend(array(&samourai["pairs"]).iter().cloned());
        candidates.extend(derived.iter().cloned());
        let mut other_blocked = derivation_blocked;
        other_blocked.extend(ids(&privacy["blocked_transaction_ids"]));
        other_blocked.extend(ids(&samourai["blocked_transaction_ids"]));
        let mut resolved = context.excluded.clone();
        resolved.extend(ids(&samourai["touched_ids"]));
        resolved.extend(recorded::resolved_ids(&prepared.recorded, context, true));
        let holds = holds::compute(holds::Inputs {
            rows,
            derivation_rows: &prepared.normalized,
            pairs: &candidates,
            wallets: &wallets,
            has_chain_events: has_chain,
            excluded: &context.excluded,
            blocked: &other_blocked,
            resolved: &resolved,
        })?;
        let kept = holds::without_held(&candidates, &holds.blocked_ids);
        let derived = holds::without_held(derived, &holds.blocked_ids);
        let compiled = pairs::compile(&kept, context, snapshot)?;
        let mut claims = array(&compiled["claims"]).to_vec();
        claims.extend(array(&samourai["fee_claims"]).iter().cloned());
        let pair_blocked = ids(&compiled["blocked_transaction_ids"]);
        let transient: Ids = holds
            .transient_ids
            .difference(&pair_blocked)
            .filter(|id| !other_blocked.contains(*id))
            .cloned()
            .collect();
        let mut all_blocked = other_blocked;
        all_blocked.extend(pair_blocked);
        all_blocked.extend(holds.blocked_ids.iter().cloned());
        let mut quarantines = prepared.derivation_quarantines;
        quarantines.extend(array(&compiled["quarantines"]).iter().cloned());
        quarantines.extend(array(&privacy["quarantines"]).iter().cloned());
        quarantines.extend(array(&samourai["quarantines"]).iter().cloned());
        quarantines.extend(holds.quarantines.iter().cloned());
        Ok::<_, String>(
            json!({"privacy":privacy,"holds":holds,"pair_inputs_before_holds":candidates,"pair_inputs":kept,"derived_pairs":derived,"claims":claims,"native_audits":pairs::native_audits(&derived,context)?,"blocked_transaction_ids":all_blocked,"quarantines":quarantines,"transient_blocked_transaction_ids":transient}),
        )
    };
    Ok(run().unwrap_or_else(|error| json!({"error":error})))
}

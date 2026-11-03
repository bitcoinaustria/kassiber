//! Composed comparison seam through privacy, raw holds and dependency closure.
//! Native HTLC production is composed before graph derivation and reconciliation.
use super::{
    array, channels,
    holds::{self, Ids},
    native, pairs, pipeline, privacy, reconciliation, recorded, samourai, Context,
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
    Ok(compose_impl(snapshot, context, &[], true).unwrap_or_else(|error| json!({"error":error})))
}

pub(super) fn compose(
    snapshot: &Snapshot,
    context: &Context,
    components: &[Value],
) -> Result<Value, String> {
    compose_impl(snapshot, context, components, false)
}

fn compose_impl(
    snapshot: &Snapshot,
    context: &Context,
    components: &[Value],
    diagnostics: bool,
) -> Result<Value, String> {
    let run = || {
        let channel = channels::compute(snapshot)?;
        let channel_roles = channel["roles"].as_object().expect("role map");
        let mut channel_quarantines = Vec::new();
        for (id, role) in channel_roles {
            if role != "channel_open_mismatch" && role != "channel_close_mismatch" {
                continue;
            }
            let row = snapshot
                .transactions
                .iter()
                .find(|row| row.get("id").is_some_and(|v| v == id))
                .map(|row| json!(row))
                .unwrap_or(json!({}));
            channel_quarantines.push(json!({"transaction_id":id,"workspace_id":row["workspace_id"],"profile_id":row["profile_id"],"reason":if role == "channel_open_mismatch" {"channel_open_unresolved"} else {"channel_close_unresolved"},"detail_json":pairs::detail_json(&json!({"channel_role":role,"required_for":"complete_channel_lifecycle"}))?}));
        }
        let channel_blocked = blocked(&channel_quarantines);
        let samourai = samourai::compile(snapshot, context)?;
        let rows = pipeline::Rows::Snapshot(snapshot);
        let has_chain = context
            .canonical
            .iter()
            .iter()
            .any(|event| event.chain_namespace());
        // The snapshot records hold the same fields as `rows`.
        let has_hashes = snapshot.transactions.iter().any(|row| {
            row.get("payment_hash")
                .is_some_and(|hash| !hash.is_null() && hash != "")
        });
        let auto = if has_chain || has_hashes {
            match rows.records() {
                Some(records) => recorded::detect(records)?.0,
                None => recorded::detect(rows.values())?.0,
            }
        } else {
            Vec::new()
        };
        let mut occupied = context.excluded.clone();
        occupied.extend(ids(&samourai["touched_ids"]));
        occupied.extend(channel_roles.keys().cloned());
        occupied.extend(recorded::resolved_ids(&auto, context, true));
        occupied.extend(array(&channel["pairs"]).iter().flat_map(holds::anchors));
        occupied.extend(
            snapshot
                .loan_legs
                .iter()
                .filter_map(|leg| leg.get("transaction_id"))
                .filter(|&id| codec::truth(id))
                .map(codec::pystr),
        );
        let native = native::compute(snapshot, context, &occupied)?;
        let native_trace = diagnostics.then(|| json!({"occupied_ids":occupied,"pairs":native.pairs,"proofs":native.proof_values,"quarantines":native.quarantines}));
        let native_ids: Ids = native
            .proofs
            .keys()
            .flat_map(|(a, b)| [a.clone(), b.clone()])
            .collect();
        let native_blocked = blocked(&native.quarantines);
        let mut reserved = native_ids.clone();
        reserved.extend(native_blocked.iter().cloned());
        let mut prepared =
            pipeline::prepare_with_samourai(snapshot, context, rows, auto, &reserved, &samourai)?;
        prepared
            .candidates
            .extend(array(&channel["pairs"]).iter().cloned());
        let rows = &prepared.rows;
        let wallets: BTreeMap<_, _> = snapshot
            .wallets
            .iter()
            .map(|wallet| (codec::pystr(&json!(wallet)["id"]), json!(wallet)))
            .collect();
        let derived = &prepared.candidates;
        // Unchanged snapshot rows are read from their records, which hold the
        // same fields and read faster.
        let privacy = if let Some(records) = rows.records() {
            privacy::compute(
                records,
                &prepared.recorded,
                derived,
                context,
                &wallets,
                &ids(&samourai["candidate_ids"]),
                &native_ids,
            )?
        } else {
            privacy::compute(
                rows.values(),
                &prepared.recorded,
                derived,
                context,
                &wallets,
                &ids(&samourai["candidate_ids"]),
                &native_ids,
            )?
        };
        let derivation_blocked = blocked(&prepared.derivation_quarantines);
        let mut touched = derivation_blocked.clone();
        touched.extend(derived.iter().flat_map(holds::anchors));
        // Pairs are borrowed from their producers; each holds its rows.
        let mut candidates: Vec<&Value> = prepared
            .recorded
            .iter()
            .filter(|pair| holds::anchors(pair).is_disjoint(&touched))
            .collect();
        candidates.extend(array(&samourai["pairs"]));
        candidates.extend(derived.iter());
        candidates.extend(native.pairs.iter());
        let mut other_blocked = derivation_blocked;
        other_blocked.extend(channel_blocked);
        other_blocked.extend(native_blocked);
        other_blocked.extend(ids(&privacy["blocked_transaction_ids"]));
        other_blocked.extend(ids(&samourai["blocked_transaction_ids"]));
        let mut resolved = context.excluded.clone();
        resolved.extend(native_ids);
        resolved.extend(ids(&samourai["touched_ids"]));
        resolved.extend(recorded::resolved_ids(&prepared.recorded, context, true));
        resolved.extend(channel_roles.keys().cloned());
        resolved.extend(array(&channel["pairs"]).iter().flat_map(holds::anchors));
        resolved.extend(
            snapshot
                .loan_legs
                .iter()
                .filter_map(|leg| leg.get("transaction_id"))
                .filter(|id| codec::truth(id))
                .map(codec::pystr),
        );
        let mut non_events: Ids = snapshot
            .loan_legs
            .iter()
            .filter_map(|leg| leg.get("transaction_id"))
            .filter(|id| !id.is_null() && *id != "")
            .map(codec::text)
            .collect();
        non_events.extend(
            channel_roles
                .iter()
                .filter(|(_, role)| *role == "channel_open" || *role == "channel_close")
                .map(|(id, _)| id.clone()),
        );
        macro_rules! holds {
            ($rows:expr, $derivation:expr) => {
                holds::compute(holds::Inputs {
                    rows: $rows,
                    derivation_rows: $derivation,
                    pairs: &candidates,
                    wallets: &wallets,
                    has_chain_events: has_chain,
                    excluded: &context.excluded,
                    blocked: &other_blocked,
                    resolved: &resolved,
                })?
            };
        }
        let holds = match (rows.records(), prepared.normalized.records()) {
            (Some(rows), Some(derivation)) => holds!(rows, derivation),
            (Some(records), None) => match prepared.normalized.layered() {
                Some(derivation) => holds!(records, &derivation[..]),
                None => holds!(records, prepared.normalized.values()),
            },
            (None, Some(derivation)) => holds!(rows.values(), derivation),
            (None, None) => holds!(rows.values(), prepared.normalized.values()),
        };
        let kept = holds::without_held(&candidates, &holds.blocked_ids);
        let derived = holds::without_held(&derived.iter().collect::<Vec<_>>(), &holds.blocked_ids);
        let mut released_context = None;
        let mut native_reconciliations = Vec::new();
        if components.iter().any(|c| {
            c["effective_state"] == "active"
                && c["conservation_mode"] == "quantity"
                && !codec::truth(&c["economic_terms"])
        }) {
            let mut candidate_context = context.clone();
            candidate_context.excluded.clear();
            let candidates =
                pairs::compile_with_proofs(&kept, &candidate_context, snapshot, &native.proofs)?;
            let mut blocked = other_blocked.clone();
            blocked.extend(ids(&candidates["blocked_transaction_ids"]));
            blocked.extend(holds.pending_ids.iter().cloned());
            blocked.extend(holds.blocked_ids.iter().cloned());
            let source_hashes: Ids = array(&candidates["claims"])
                .iter()
                .map(|claim| codec::text(&claim["source"]["observation_hash"]))
                .collect();
            let anchors: Ids = context
                .observations
                .values()
                .filter(|observation| {
                    source_hashes.contains(&codec::text(&observation["quantity_hash"]))
                })
                .map(|observation| codec::text(&observation["anchor_transaction_id"]))
                .collect();
            let rows = rows
                .values()
                .iter()
                .filter(|row| anchors.contains(&codec::text(&row["id"])))
                .map(|row| (codec::text(&row["id"]), row.clone()))
                .collect();
            native_reconciliations = reconciliation::reconcile(
                components,
                &context.observations,
                array(&candidates["claims"]),
                &blocked,
                &rows,
            )?;
            let reconciled: Ids = native_reconciliations
                .iter()
                .map(|c| codec::text(&c["component_id"]))
                .collect();
            for component in components
                .iter()
                .filter(|c| reconciled.contains(&codec::pystr(&c["id"])))
            {
                for leg in array(&component["legs"]) {
                    released_context
                        .get_or_insert_with(|| context.clone())
                        .excluded
                        .remove(&super::reviewed::anchor(leg));
                }
            }
        }
        let context = released_context.as_ref().unwrap_or(context);
        let compiled = pairs::compile_with_proofs(&kept, context, snapshot, &native.proofs)?;
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
        quarantines.extend(channel_quarantines);
        quarantines.extend(array(&privacy["quarantines"]).iter().cloned());
        quarantines.extend(array(&samourai["quarantines"]).iter().cloned());
        quarantines.extend(native.quarantines);
        quarantines.extend(holds.quarantines.iter().cloned());
        let mut result = json!({"non_event_transaction_ids":non_events,"claims":claims,"native_audits":pairs::native_audits(&derived,context)?,"blocked_transaction_ids":all_blocked,"quarantines":quarantines,"transient_blocked_transaction_ids":transient});
        if diagnostics {
            result.as_object_mut().unwrap().extend(
                json!({"native_transitions":native_trace,"handled_ids":prepared.handled,"privacy":privacy,"holds":holds,"pair_inputs_before_holds":candidates,"pair_inputs":kept,"derived_pairs":derived})
                    .as_object_mut().unwrap().iter_mut().map(|(key, value)| (key.clone(), value.take()))
            );
        }
        if !components.is_empty() {
            result["native_component_reconciliations"] = json!(native_reconciliations);
        }
        Ok::<_, String>(result)
    };
    run()
}

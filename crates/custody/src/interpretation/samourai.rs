//! Tracked Whirlpool lifecycle candidate, exact MOVE and residual-fee compiler.
//! Structured metadata groups evidence; only closed same-event observations
//! authorize a MOVE or an exact Tx0 coordinator fee.
use super::{
    anchor,
    claims::{claim, principal, slice},
    text::{event_repr, field, text_or_empty},
    Context,
};
use crate::{
    codec::{self, text, truth},
    python_text::PythonText,
    snapshot::Snapshot,
};
use kassiber_pyjson::Value as Py;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn compile(snapshot: &Snapshot, context: &Context) -> Result<Value, String> {
    let rows: Vec<Value> = snapshot.transactions.iter().map(|row| json!(row)).collect();
    let present = rows.iter().any(|row| {
        row.get("config_text_mentions_samourai")
            .map(truth)
            .unwrap_or_else(|| text(&row["config_json"]).py_lower().contains("samourai"))
    });
    let mut grouped: BTreeMap<(String, String), Vec<(&Value, String)>> = BTreeMap::new();
    if present {
        for row in &rows {
            // json.loads accepts text, not an already decoded mapping.
            let config = if truth(&row["config_json"]) {
                row["config_json"].as_str()
            } else {
                Some("{}")
            };
            let Some(config) = config.and_then(|text| codec::loads(text).ok()) else {
                continue;
            };
            let metadata = field(&config, "samourai");
            if !matches!(metadata, Py::Dict(_))
                || field(metadata, "role") != &Py::Str("child".into())
            {
                continue;
            }
            let group = text_or_empty(field(metadata, "group_id"))
                .py_strip()
                .to_owned();
            let section = text_or_empty(field(metadata, "section"))
                .py_strip()
                .py_lower();
            let Some(observation) = context.observations.get(&anchor(row)) else {
                continue;
            };
            if group.is_empty() || section.is_empty() {
                continue;
            }
            grouped
                .entry((group, event_repr(&observation["event_key"])))
                .or_default()
                .push((row, section));
        }
    }
    let mut pairs = Vec::new();
    let mut fee_claims = Vec::new();
    let mut candidates = BTreeSet::new();
    let mut touched = BTreeSet::new();
    for ((wallet_group, _), entries) in grouped {
        let outs: Vec<_> = entries
            .iter()
            .filter(|(row, _)| row["direction"] == "outbound")
            .collect();
        let mut ins: Vec<_> = entries
            .iter()
            .filter(|(row, _)| row["direction"] == "inbound")
            .collect();
        let out_sections: BTreeSet<_> = outs.iter().map(|(_, section)| section.as_str()).collect();
        let in_sections: BTreeSet<_> = ins.iter().map(|(_, section)| section.as_str()).collect();
        let tx0 = out_sections.contains("deposit")
            && (in_sections.contains("premix") || in_sections.contains("badbank"));
        let lifecycle = tx0
            || (out_sections.contains("premix") && in_sections.contains("postmix"))
            || (out_sections.contains("postmix") && in_sections.contains("postmix"));
        if !lifecycle || outs.len() != 1 || ins.is_empty() {
            continue;
        }
        let out = outs[0].0;
        let source = &context.observations[&anchor(out)];
        let group_id = format!(
            "samourai:{wallet_group}:{}",
            text(&source["event_key"]["native_event_id"])
        );
        candidates.extend(
            outs.iter()
                .chain(&ins)
                .map(|(row, _)| anchor(row))
                .filter(|id| !id.is_empty()),
        );
        let mut allocated = 0;
        let mut group_touched = BTreeSet::new();
        let mut authoritative = truth(&source["authoritative_chain_observation"]);
        ins.sort_by_key(|(row, _)| text(&row["id"]));
        for (incoming, _) in ins {
            let Some(target) = context.observations.get(&anchor(incoming)) else {
                authoritative = false;
                continue;
            };
            if target["asset"] != source["asset"] {
                authoritative = false;
                continue;
            }
            let amount = principal(target);
            if amount <= 0 || allocated + amount > principal(source) {
                authoritative = false;
                continue;
            }
            if !truth(&target["authoritative_chain_observation"])
                || target["event_key"] != source["event_key"]
            {
                authoritative = false;
            }
            pairs.push(json!({"out":out,"in":incoming,"group_id":group_id,"source":"samourai_internal","out_amount":amount,"allow_unclaimed_residual":true}));
            allocated += amount;
            group_touched.extend([anchor(out), anchor(incoming)]);
        }
        let residual = principal(source) - allocated;
        if allocated > 0 && residual > 0 {
            let exact = tx0 && authoritative && source["fee_attribution"] == "exact";
            let mut fee = claim(
                format!("samourai-fee:{}:{residual}", text(&source["quantity_hash"])),
                slice(&source["quantity_hash"], allocated, principal(source)),
                if exact {
                    "external_confirmed"
                } else {
                    "custody_suspense"
                },
                exact,
                if exact {
                    "samourai_coordinator_fee"
                } else {
                    "implicit_wallet_delta_unallocated"
                },
            );
            if exact {
                fee["atomic_bundle_id"] = json!(format!("pair-group:{group_id}"));
                fee["destination_kind"] = json!("fee");
            }
            fee_claims.push(fee);
        }
        if authoritative {
            touched.extend(group_touched);
        }
    }

    // These pairs are same-event and same-asset by construction. The generic
    // pair compiler's other proof paths belong to subsequent interpreter ports.
    // Preserve its exact sorting, ordinal ids and per-hash cursors here.
    let mut ordered: Vec<_> = pairs.iter().collect();
    ordered.sort_by_key(|pair| {
        (
            text(&pair["out"]["occurred_at"]),
            text(&pair["out"]["id"]),
            text(&pair["in"]["id"]),
            text(&pair["pair_id"]),
        )
    });
    let mut source_cursor = BTreeMap::new();
    let mut target_cursor = BTreeMap::new();
    let mut implicit_sources = BTreeMap::new();
    let mut claims = Vec::new();
    for (ordinal, pair) in ordered.into_iter().enumerate() {
        let out_id = anchor(&pair["out"]);
        let in_id = anchor(&pair["in"]);
        if context.excluded.contains(&out_id) || context.excluded.contains(&in_id) {
            continue;
        }
        let source = &context.observations[&out_id];
        let target = &context.observations[&in_id];
        if source["direction"] != "outbound"
            || target["direction"] != "inbound"
            || !truth(&source["authoritative_chain_observation"])
            || !truth(&target["authoritative_chain_observation"])
            || source["event_key"] != target["event_key"]
            || source["event_key"]["native_namespace"] != "chain"
            || !matches!(
                source["event_key"]["chain"].as_str(),
                Some("bitcoin" | "liquid")
            )
        {
            continue;
        }
        let source_hash = text(&source["quantity_hash"]);
        let target_hash = text(&target["quantity_hash"]);
        let source_start = *source_cursor.get(&source_hash).unwrap_or(&0);
        let target_start = *target_cursor.get(&target_hash).unwrap_or(&0);
        let amount = codec::wide(&pair["out_amount"])
            .min(principal(source) - source_start)
            .min(principal(target) - target_start);
        if amount <= 0 {
            continue;
        }
        let inferred_fee = (codec::wide(&pair["out_amount"]).min(principal(source) - source_start)
            - amount)
            .max(0);
        let mut movement = claim(
            format!("pair:{out_id}:{in_id}:{ordinal}:slice:{source_start}:{amount}"),
            slice(
                &source["quantity_hash"],
                source_start,
                source_start + amount,
            ),
            "internal_verified",
            true,
            "samourai_internal",
        );
        movement["target"] = slice(
            &target["quantity_hash"],
            target_start,
            target_start + amount,
        );
        movement["atomic_bundle_id"] = json!(format!("pair-group:{}", text(&pair["group_id"])));
        claims.push(movement);
        source_cursor.insert(source_hash.clone(), source_start + amount);
        target_cursor.insert(target_hash, target_start + amount);
        if source["fee_attribution"] == "implicit_wallet_delta" {
            implicit_sources.insert(source_hash, source);
        } else if inferred_fee > 0 {
            let fee_start = source_start + amount;
            let mut fee = claim(
                format!("pair:{out_id}:{in_id}:{ordinal}:fee:{fee_start}:{inferred_fee}"),
                slice(
                    &source["quantity_hash"],
                    fee_start,
                    fee_start + inferred_fee,
                ),
                "external_confirmed",
                true,
                "verified_transfer_fee",
            );
            fee["atomic_bundle_id"] = json!(format!("pair-group:{}", text(&pair["group_id"])));
            fee["destination_kind"] = json!("fee");
            claims.push(fee);
            source_cursor.insert(text(&source["quantity_hash"]), fee_start + inferred_fee);
        }
    }
    for (hash, source) in implicit_sources {
        let start = *source_cursor.get(&hash).unwrap_or(&0);
        if start < principal(source) {
            claims.push(claim(
                format!("implicit-wallet-delta:{hash}:{start}"),
                slice(&source["quantity_hash"], start, principal(source)),
                "custody_suspense",
                false,
                "implicit_wallet_delta_unallocated",
            ));
        }
    }
    claims.extend(fee_claims.clone());
    let unverified: BTreeSet<_> = candidates
        .difference(&touched)
        .filter(|id| !context.excluded.contains(*id))
        .cloned()
        .collect();
    let rows_by_id: BTreeMap<_, _> = rows.iter().map(|row| (text(&row["id"]), row)).collect();
    let detail = kassiber_pyjson::dumps(&codec::from_json(&json!({"required_for":"authoritative_same_event_observation", "resolution":"sync every imported Samourai source with a chain observer"}))?, &kassiber_pyjson::Options::new().sort_keys(true)).map_err(|e| e.to_string())?;
    let quarantines: Vec<_> = unverified.iter().map(|id| {
        let row = rows_by_id.get(id).copied().unwrap_or(&Value::Null);
        json!({"transaction_id":id,"workspace_id":row["workspace_id"],"profile_id":row["profile_id"],"reason":"samourai_native_event_unverified","detail_json":detail})
    }).collect();
    Ok(
        json!({"pairs":pairs,"fee_claims":fee_claims,"candidate_ids":candidates,"touched_ids":touched,
        "claims":claims,"blocked_transaction_ids":unverified,"quarantines":quarantines}),
    )
}

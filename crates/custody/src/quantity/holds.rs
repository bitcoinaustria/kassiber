//! Advisory candidates create independent boundary holds, never transfer edges.
use super::{array, delta, exact, number};
use crate::codec;
use crate::python_text::PythonText;
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
fn principal(o: &Value) -> Result<BigInt, String> {
    fn integer(v: &Value) -> Result<BigInt, String> {
        if !codec::truth(v) {
            return Ok(BigInt::from(0));
        }
        if v.is_array() || v.is_object() {
            return Err("TypeError".into());
        }
        exact(&codec::integer(v).map_err(|_| "ValueError")?)
    }
    let amount = integer(&o["amount_msat"])?;
    let fee = integer(&o["fee_msat"])?;
    Ok(
        if text(&o["direction"]).py_strip().py_lower() == "outbound"
            && codec::truth(&o["amount_includes_fee"])
        {
            amount - fee
        } else {
            amount
        },
    )
}
fn text(v: &Value) -> String {
    if codec::truth(v) {
        codec::pystr(v)
    } else {
        String::new()
    }
}
fn strings(v: &Value) -> Vec<String> {
    array(v).iter().map(text).collect()
}
fn unique(v: impl Iterator<Item = String>) -> Vec<String> {
    v.collect::<BTreeSet<_>>().into_iter().collect()
}
fn err(message: &str, details: Value) -> (String, Value) {
    (message.into(), details)
}
fn slice(o: &Value) -> Result<Value, String> {
    Ok(
        json!({"$type":"QuantitySlice","observation_hash":o["quantity_hash"],"start_msat":0,"end_msat":number(&principal(o)?)}),
    )
}
fn hold(o: &Value, gap: &str, id: String) -> Result<Value, String> {
    Ok(
        json!({"$type":"CustodyGapHold","hold_id":id,"gap_id":gap,"transaction_id":o["transaction_id"],"direction":o["direction"],"quantity":slice(o)?,"evidence_detail_hash":o["evidence_detail_hash"]}),
    )
}
fn claim(h: &Value, reason: &str) -> Value {
    json!({"$type":"QuantityClaim","claim_id":h["hold_id"],"source":h["quantity"],"target":null,"state":"custody_suspense","priority":{"$enum":"ClaimPriority.ACCOUNTING_CONVENTION","value":40},"reason":reason,"atomic_bundle_id":null,"destination_kind":null,"external_economic_subtype":null,"transfer_kind":null,"transfer_policy":null,"component_id":null,"allow_cross_rail":false})
}
fn validate<'a>(
    c: &Value,
    observations: &'a BTreeMap<String, Value>,
) -> Result<Vec<&'a Value>, (String, Value)> {
    let gap = &c["gap_id"];
    let mut all = Vec::new();
    for (field, direction) in [("source_ids", "outbound"), ("return_ids", "inbound")] {
        let ids = strings(&c[field]);
        let mut sorted = ids.clone();
        sorted.sort();
        if ids.is_empty() || ids != sorted {
            return Err(err(
                "candidate transaction ids must be non-empty and canonically ordered",
                json!({"gap_id":gap,"direction":direction}),
            ));
        }
        if unique(ids.iter().cloned()).len() != ids.len() {
            return Err(err(
                "candidate transaction ids must be unique",
                json!({"gap_id":gap,"direction":direction}),
            ));
        }
        for id in ids {
            let o = observations.get(&id).ok_or_else(|| {
                err(
                    "candidate references a missing canonical observation",
                    json!({"gap_id":gap,"transaction_id":id}),
                )
            })?;
            // These observations have already passed canonical admission. Keep
            // validation here too: this private seam is independently tested.
            if super::validate_observation(o).is_err() {
                return Err(err(
                    "candidate references an invalid canonical observation",
                    json!({"gap_id":gap,"transaction_id":id,"reason":validation_message(o)}),
                ));
            }
            if o["transaction_id"] != id {
                return Err(err(
                    "observation mapping key does not match transaction identity",
                    json!({"gap_id":gap,"transaction_id":id,"observation_transaction_id":o["transaction_id"]}),
                ));
            }
            if o["direction"] != direction {
                return Err(err(
                    "candidate observation has the wrong direction",
                    json!({"gap_id":gap,"transaction_id":id,"expected":direction,"actual":o["direction"]}),
                ));
            }
            all.push(o);
        }
    }
    let sources = strings(&c["source_ids"]);
    let returns = strings(&c["return_ids"]);
    if sources.iter().any(|s| returns.contains(s)) {
        return Err(err(
            "candidate source and return observations must be disjoint",
            json!({"gap_id":gap}),
        ));
    }
    if !codec::truth(&c["profile_id"]) {
        return Err(err(
            "candidate profile scope must be non-empty",
            json!({"gap_id":gap}),
        ));
    }
    let profiles = unique(all.iter().map(|o| text(&o["profile_id"])));
    if profiles != vec![text(&c["profile_id"])] {
        return Err(err(
            "candidate profile scope does not match every observation",
            json!({"gap_id":gap,"candidate_profile_id":c["profile_id"],"observation_profile_ids":profiles}),
        ));
    }
    if !codec::truth(&c["asset"]) {
        return Err(err(
            "custody gap ids require profile, asset, source, and return ids",
            json!({}),
        ));
    }
    let mut identity = vec![text(&c["profile_id"]), text(&c["asset"])];
    identity.extend(sources.iter().cloned());
    identity.push("->".into());
    identity.extend(returns.iter().cloned());
    let expected_id = format!("cg_{}", &codec::sha256(&identity.join("\x1f"))[..24]);
    if c["gap_id"] != expected_id {
        return Err(err(
            "candidate identity does not match its boundary transactions",
            json!({"gap_id":gap,"expected_gap_id":expected_id}),
        ));
    }
    if unique(all.iter().map(|o| text(&o["quantity_hash"]))).len() != all.len() {
        return Err(err(
            "candidate boundary contains duplicate canonical quantity hashes",
            json!({"gap_id":gap}),
        ));
    }
    let assets = unique(all.iter().map(|o| text(&o["asset"])));
    if assets != vec![text(&c["asset"])] {
        return Err(err(
            "candidate asset does not match every observation",
            json!({"gap_id":gap,"assets":assets}),
        ));
    }
    let chains = unique(all.iter().map(|o| text(&o["event_key"]["chain"])));
    let networks = unique(all.iter().map(|o| text(&o["event_key"]["network"])));
    if chains.len() != 1 || networks.len() != 1 {
        return Err(err(
            "candidate observations cross chain or network scope",
            json!({"gap_id":gap,"chains":chains,"networks":networks}),
        ));
    }
    let (src, dst) = all.split_at(sources.len());
    let sw = unique(src.iter().map(|o| text(&o["wallet_id"])));
    let dw = unique(dst.iter().map(|o| text(&o["wallet_id"])));
    if sw != strings(&c["source_wallet_ids"]) || dw != strings(&c["destination_wallet_ids"]) {
        return Err(err(
            "candidate wallet scope does not match canonical observations",
            json!({"gap_id":gap,"source_wallet_ids":sw,"destination_wallet_ids":dw}),
        ));
    }
    let total: BigInt = src.iter().map(|o| principal(o).unwrap()).sum();
    let ret: BigInt = dst.iter().map(|o| principal(o).unwrap()).sum();
    let retained = total.clone().min(ret.clone());
    if total == BigInt::from(0) {
        return Err(err("division by zero", json!({})));
    }
    let expected = json!({"source_total_msat":number(&total),"source_fee_msat":number(&src.iter().map(|o|exact(&o["fee_msat"]).unwrap()).sum()),"source_debit_msat":number(&src.iter().map(|o|-delta(*o).unwrap()).sum()),"return_total_msat":number(&ret),"retained_msat":number(&retained),"residual_msat":number(&(&total-&retained)),"excess_msat":number(&(&ret-&retained)),"coverage_ppm":number(&(&retained*1_000_000/&total))});
    let actual: serde_json::Map<_, _> = expected
        .as_object()
        .unwrap()
        .keys()
        .map(|k| (k.clone(), c[k].clone()))
        .collect();
    if expected != Value::Object(actual.clone()) {
        return Err(err(
            "candidate quantities do not match canonical observations",
            json!({"gap_id":gap,"expected":expected,"actual":actual}),
        ));
    }
    Ok(all)
}
fn validation_message(o: &Value) -> String {
    for k in ["transaction_id", "anchor_transaction_id", "wallet_id"] {
        if !codec::truth(&o[k]) {
            return "quantity observations require transaction, anchor, and wallet ids".into();
        }
    }
    for k in ["quantity_hash", "evidence_detail_hash"] {
        let mut probe = json!({"transaction_id":"t","anchor_transaction_id":"t","wallet_id":"w","quantity_hash":"a".repeat(64),"evidence_detail_hash":"a".repeat(64),"direction":"outbound","asset":"BTC","amount_msat":1,"fee_msat":0,"amount_includes_fee":false});
        probe[k] = o[k].clone();
        if super::validate_observation(&probe).is_err() {
            return format!("quantity observation {k} must be lowercase SHA-256");
        }
    }
    if !matches!(o["direction"].as_str(), Some("inbound" | "outbound")) {
        return "quantity observation direction must be inbound or outbound".into();
    }
    if !codec::truth(&o["asset"]) {
        return "quantity observations require an asset".into();
    }
    for k in ["amount_msat", "fee_msat"] {
        if exact(&o[k]).map_or(true, |n| n < BigInt::from(0)) {
            return format!("quantity observation {k} must be non-negative");
        }
    }
    if o["direction"] == "inbound" && codec::truth(&o["amount_includes_fee"]) {
        return "inbound quantity cannot use amount_includes_fee".into();
    }
    if o["amount_msat"] == 0 {
        return "zero quantity is valid only for a fee-only outbound".into();
    }
    "an included fee cannot exceed the observed amount".into()
}
/// Independent source and return holds, plus source-only suspense claims.
pub fn compose(
    canonical: &Value,
    ignored: &[Value],
    dismissed: &Value,
    search: &Value,
) -> Result<Value, String> {
    compose_canon(super::Canon::Json(canonical), ignored, dismissed, search)
}

/// `compose`; a typed canonical input is read as JSON only when a hold needs it.
pub(super) fn compose_canon(
    canonical: super::Canon<'_>,
    ignored: &[Value],
    dismissed: &Value,
    search: &Value,
) -> Result<Value, String> {
    if !codec::truth(&search["search_complete"])
        && array(&search["accounting_candidates"]).is_empty()
        && array(&search["blocking_source_ids"]).is_empty()
    {
        return Ok(json!([[], [], [], []]));
    }
    let canonical = canonical.json();
    let by_hash: BTreeMap<_, _> = array(&canonical["events"])
        .iter()
        .flat_map(|e| array(&e["legs"]))
        .map(|o| (text(&o["quantity_hash"]), o))
        .collect();
    let mut observations = BTreeMap::new();
    for e in array(&canonical["events"]) {
        for a in array(&e["observation_aliases"]) {
            observations.insert(
                text(&a[0]),
                (*by_hash.get(&text(&a[1])).ok_or("KeyError")?).clone(),
            );
        }
    }
    let ignored: BTreeSet<_> = ignored
        .iter()
        .filter(|v| codec::truth(v))
        .map(codec::pystr)
        .collect();
    let mut claims = Vec::new();
    let mut issues = Vec::new();
    let mut holds = Vec::new();
    let mut ids = BTreeSet::new();
    if !codec::truth(&search["search_complete"]) {
        for id in strings(&search["blocking_source_ids"])
            .into_iter()
            .collect::<BTreeSet<_>>()
        {
            if ignored.contains(&id) {
                continue;
            }
            let Some(o) = observations.get(&id) else {
                continue;
            };
            if o["direction"] != "outbound" || principal(o)? <= BigInt::from(0) {
                continue;
            }
            ids.insert(id);
            let h = hold(
                o,
                "capacity-incomplete",
                format!("gap-capacity-hold:{}", text(&o["quantity_hash"])),
            )?;
            claims.push(claim(&h, "custody_gap_search_incomplete_structured_source"));
            holds.push(h);
        }
    }
    for c in array(&search["accounting_candidates"]) {
        let source_ids = strings(&c["source_ids"]);
        let return_ids = strings(&c["return_ids"]);
        if source_ids
            .iter()
            .chain(&return_ids)
            .any(|id| ignored.contains(id))
            || !codec::truth(&c["promotion_eligible"])
        {
            continue;
        }
        let gap = text(&c["gap_id"]);
        if dismissed.get(&gap).and_then(Value::as_str)
            == Some(crate::gaps::candidate_fingerprint(c)?.as_str())
        {
            continue;
        }
        ids.extend(source_ids.iter().chain(&return_ids).cloned());
        let mut txs = source_ids.clone();
        txs.extend(return_ids);
        txs.sort();
        let (kind, state, reason, details) = match validate(c, &observations) {
            Ok(all) => {
                let mut held = Vec::new();
                for o in all {
                    if principal(o)? > BigInt::from(0) {
                        held.push(o["transaction_id"].clone());
                        holds.push(hold(
                            o,
                            &gap,
                            format!("gap-hold:{gap}:{}", text(&o["quantity_hash"])),
                        )?);
                    }
                }
                (
                    "custody_gap_review_hold",
                    "custody_suspense",
                    "custody_gap_review_required",
                    json!({"gap_id":gap,"retained_msat":c["retained_msat"],"residual_msat":c["residual_msat"],"excess_msat":c["excess_msat"],"held_transaction_ids":held}),
                )
            }
            Err((message, details)) => {
                if message == "division by zero" {
                    return Err("ZeroDivisionError".into());
                }
                let mut seen = BTreeSet::new();
                for id in source_ids {
                    if let Some(o) = observations.get(&id) {
                        if o["direction"] == "outbound"
                            && seen.insert(text(&o["quantity_hash"]))
                            && principal(o)? > BigInt::from(0)
                        {
                            let h = hold(
                                o,
                                &gap,
                                format!(
                                    "candidate:{gap}:compile-failed:{}",
                                    text(&o["quantity_hash"])
                                ),
                            )?;
                            let mut cl = claim(&h, "custody_gap_hold_compile");
                            cl["atomic_bundle_id"] =
                                json!(format!("candidate:{gap}:compile-failed"));
                            claims.push(cl);
                        }
                    }
                }
                (
                    "custody_gap_hold_compile_failed",
                    "conflicting",
                    "custody_gap_hold_compile",
                    json!({"gap_id":gap,"message":message,"compiler_details":details}),
                )
            }
        };
        let tag = if kind == "custody_gap_review_hold" {
            "custody_gap_hold"
        } else {
            "gap_compile"
        };
        let issue_id = super::hashes::issue_id(&[
            kassiber_pyjson::Value::Str(tag.into()),
            kassiber_pyjson::Value::Str(gap.into()),
        ])?
        .0;
        issues.push(json!({"$type":"QuantityIssue","issue_id":issue_id,"issue_type":kind,"state":state,"asset":c["asset"],"amount_msat":null,"occurred_at":c["started_at"],"transaction_ids":txs,"reason":reason,"details":details}));
    }
    let mut sources = BTreeMap::new();
    let mut order = Vec::new();
    for h in &holds {
        if h["direction"] == "outbound" {
            let hash = text(&h["quantity"]["observation_hash"]);
            if let std::collections::btree_map::Entry::Vacant(entry) = sources.entry(hash.clone()) {
                order.push(hash);
                entry.insert(h);
            }
        }
    }
    let claimed: Vec<_> = claims.iter().map(|c| c["source"].clone()).collect();
    for hash in order {
        let h = sources[&hash];
        if !claimed.contains(&h["quantity"]) {
            claims.push(claim(h, "custody_gap_review_required"));
        }
    }
    holds.sort_by_key(|h| text(&h["hold_id"]));
    Ok(json!([claims, issues, ids, holds]))
}

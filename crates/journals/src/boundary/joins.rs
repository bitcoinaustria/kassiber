use crate::*;
use kassiber_custody::{journal_support as graph, quantity::balances};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
fn int(v: &Value) -> BigInt {
    kassiber_custody::codec::integer_or_zero(v)
        .ok()
        .and_then(|v| v.to_string().parse().ok())
        .unwrap_or_default()
}
fn legacy_txid(v: &Value) -> String {
    let text = if truth(v) { string(v) } else { String::new() };
    if text.len() == 64 && text.bytes().all(|c| c.is_ascii_hexdigit()) {
        text.to_lowercase()
    } else {
        text
    }
}
pub(super) fn differences(c: &Value, holdings: &Holdings) -> Result<Value, Refusal> {
    let current = holdings
        .pairs
        .iter()
        .map(|(k, v)| {
            Ok(json!([
                string(&k[0]),
                string(&k[3]),
                crate::render::msat(&v.quantity.0)?
            ]))
        })
        .collect::<Result<Vec<_>, Refusal>>()?;
    let mut reasons = json!({});
    for leg in array(&c["loan_legs"]) {
        if !leg["transaction_id"].is_null() {
            reasons[string(&leg["transaction_id"])] =
                json!(format!("loan_{}_non_event", string(&leg["role"])));
        }
    }
    if let Some(roles) = c["channel_roles"].as_object() {
        for (id, role) in roles {
            reasons[id] = json!(format!("lightning_{}_non_event", string(role)));
        }
    }
    for component in array(&c["active_components"]) {
        if int(&component["validation"]["suspense_msat"]) <= BigInt::from(0) {
            continue;
        }
        for leg in array(&component["legs"]) {
            if leg["role"] == "source" && truth(&leg["transaction_id"]) {
                reasons[string(&leg["transaction_id"])] =
                    json!("custody_component_residual_suspense");
            }
        }
    }
    Ok(balances::compare_wallet_balances(
        &c["quantity_state"]["projection"],
        &current,
        &reasons,
    )?)
}
fn downstream(reason: &str) -> bool {
    matches!(
        reason,
        "custody_basis_barrier"
            | "transfer_pair_dependency_blocked"
            | "basis_provenance_incomplete"
            | "derived_transfer_group_blocked"
            | "at_swap_basis_carry_unresolved"
            | "bitcoin_rail_carry_basis_unresolved"
    )
}
pub(super) fn ownership_review(c: &Value, qs: &[QuarantineRow]) -> Result<(Value, Value), Refusal> {
    let mut blocked = BTreeMap::new();
    for q in qs {
        let entry = blocked
            .entry(string(&q.transaction_id))
            .or_insert_with(|| q.reason.clone());
        if downstream(entry) && !downstream(&q.reason) {
            *entry = q.reason.clone();
        }
    }
    if blocked.is_empty() {
        return Ok((json!({"total":0,"by_reason":{}}), json!([])));
    }
    let rows = array(&c["rows"]);
    let by_id: HashMap<_, _> = rows.iter().map(|r| (string(&r["id"]), r)).collect();
    let mut active = BTreeSet::new();
    let mut unavailable = BTreeSet::new();
    for component in array(&c["active_components"]) {
        let legs: HashMap<_, _> = array(&component["legs"])
            .iter()
            .map(|l| (string(&l["id"]), l))
            .collect();
        for a in array(&component["allocations"]) {
            let source = legs
                .get(&string(&a["source_leg_id"]))
                .copied()
                .unwrap_or(&Value::Null);
            let sink = legs
                .get(&string(&a["sink_leg_id"]))
                .copied()
                .unwrap_or(&Value::Null);
            let out = fallback(&source["anchor_transaction_id"], &source["transaction_id"]);
            let inbound = fallback(&sink["anchor_transaction_id"], &sink["transaction_id"]);
            if out.is_null() || out == "" {
                continue;
            }
            let out = string(out);
            let inbound = if inbound.is_null() || inbound == "" {
                String::new()
            } else {
                string(inbound)
            };
            active.insert((out.clone(), inbound.clone()));
            if inbound.is_empty() {
                unavailable.insert(out);
                continue;
            }
            let o = by_id.get(&out).copied().unwrap_or(&Value::Null);
            let i = by_id.get(&inbound).copied().unwrap_or(&Value::Null);
            let kind = component["component_type"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("manual");
            let policy = component["conversion_policy"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("carrying-value");
            if o.is_null()
                || i.is_null()
                || string(&o["asset"]).to_uppercase() != string(&i["asset"]).to_uppercase()
                || policy != "carrying-value"
                || !matches!(kind, "manual" | "coinjoin" | "whirlpool")
            {
                unavailable.insert(out);
                unavailable.insert(inbound);
            }
        }
    }
    let mut inbounds: HashMap<String, Vec<&Value>> = HashMap::new();
    let mut groups: HashMap<String, Vec<&Value>> = HashMap::new();
    let scopes: HashMap<_, _> = rows
        .iter()
        .map(|r| (string(&r["id"]), graph::onchain_scope(r)))
        .collect();
    for r in rows {
        if r["direction"] == "inbound" && int(&r["amount"]) > BigInt::from(0) {
            inbounds.entry(string(&r["wallet_id"])).or_default().push(r);
        }
        let legacy = legacy_txid(&r["external_id"]);
        if !legacy.is_empty() {
            groups
                .entry(format!(
                    "legacy:{legacy}:{}",
                    string(&r["asset"]).to_uppercase()
                ))
                .or_default()
                .push(r);
        }
        if let Some(s) = &scopes[&string(&r["id"])] {
            groups.entry(format!("physical:{s:?}")).or_default().push(r);
        }
    }
    let mut candidates = vec![];
    let mut counts = BTreeMap::<String, usize>::new();
    for (id, reason) in blocked {
        if !matches!(
            reason.as_str(),
            "owned_fanout_unresolved"
                | "ownership_transfer_destination_ambiguous"
                | "ownership_transfer_source_ambiguous"
        ) {
            continue;
        }
        let Some(source) = by_id.get(&id).copied() else {
            continue;
        };
        if unavailable.contains(&id)
            || source["direction"] != "outbound"
            || int(&source["amount"]) <= BigInt::from(0)
        {
            continue;
        }
        let parsed = graph::parsed(source)?;
        let mut scope = None;
        let mut destinations = BTreeMap::<String, BigInt>::new();
        if let Some(parsed) = &parsed {
            scope = graph::ownership_scope(source, parsed, &c["owned_index"])?;
            let Some(s) = &scope else {
                continue;
            };
            let mut ambiguous = false;
            for output in array(&parsed["outputs"]) {
                let owners = graph::script_owners(
                    &output["script"],
                    &(s.0.clone(), s.1.clone()),
                    &c["owned_index"],
                )?;
                if owners.contains(&string(&source["wallet_id"])) {
                    continue;
                }
                if owners.len() > 1 {
                    ambiguous = true;
                    break;
                }
                if let Some(owner) = owners.first() {
                    *destinations.entry(owner.clone()).or_default() +=
                        int(&output["value_sats"]) * 1000;
                }
            }
            if ambiguous {
                continue;
            }
        } else if reason == "owned_fanout_unresolved" {
            scope = scopes[&id].clone();
            let legacy = legacy_txid(&source["external_id"]);
            if scope.is_none() && legacy.is_empty() {
                continue;
            }
            let key = scope
                .as_ref()
                .map(|s| format!("physical:{s:?}"))
                .unwrap_or_else(|| {
                    format!(
                        "legacy:{legacy}:{}",
                        string(&source["asset"]).to_uppercase()
                    )
                });
            let group = groups.get(&key).map(Vec::as_slice).unwrap_or(&[]);
            let outs: Vec<_> = group
                .iter()
                .filter(|r| r["direction"] == "outbound" && int(&r["amount"]) > BigInt::from(0))
                .collect();
            if outs.len() != 1 || outs[0]["id"] != source["id"] {
                continue;
            }
            for inbound in group {
                if inbound["direction"] == "inbound"
                    && int(&inbound["amount"]) > BigInt::from(0)
                    && inbound["wallet_id"] != source["wallet_id"]
                {
                    *destinations
                        .entry(string(&inbound["wallet_id"]))
                        .or_default() += int(&inbound["amount"]);
                }
            }
        }
        let txid = parsed
            .as_ref()
            .map(|p| fallback(&p["txid"], &source["external_id"]))
            .unwrap_or(&source["external_id"]);
        let txid = string(txid).trim().to_lowercase();
        let scope = scope.or_else(|| scopes[&id].clone());
        for (wallet, amount) in destinations {
            let mut exact = vec![];
            let mut compatible = vec![];
            for inbound in inbounds.get(&wallet).into_iter().flatten() {
                let iid = string(&inbound["id"]);
                if string(&inbound["asset"]).to_uppercase()
                    != string(&source["asset"]).to_uppercase()
                    || unavailable.contains(&iid)
                    || active.contains(&(id.clone(), iid.clone()))
                {
                    continue;
                }
                let received = int(&inbound["amount"]);
                let diff = &received - &amount;
                let abs = if diff < BigInt::from(0) { -diff } else { diff };
                let magnitude = if amount < BigInt::from(0) {
                    -amount.clone()
                } else {
                    amount.clone()
                };
                let tolerance = std::cmp::max(magnitude / 100, BigInt::from(2_500_000));
                if abs > tolerance {
                    continue;
                }
                let iscope = &scopes[&iid];
                if scope.is_some() && iscope.is_some() && scope != *iscope {
                    continue;
                }
                if txid.len() == 64
                    && txid.bytes().all(|b| b.is_ascii_hexdigit())
                    && iscope.as_ref().is_some_and(|s| s.2 != txid)
                {
                    continue;
                }
                if parsed.is_some()
                    && reason != "ownership_transfer_source_ambiguous"
                    && scope.is_some()
                    && *iscope == scope
                    && int(&source["amount"]) == amount
                    && received == amount
                {
                    exact.push(*inbound);
                } else {
                    compatible.push(*inbound);
                }
            }
            let (selected, confidence) = if exact.is_empty() {
                (compatible, "strong")
            } else {
                (exact, "exact")
            };
            let size = selected.len();
            for inbound in selected {
                candidates.push(candidate(
                    source,
                    inbound,
                    &amount,
                    &reason,
                    &format!("ownership-review:{id}:{wallet}"),
                    confidence,
                    size,
                ));
                *counts.entry(reason.clone()).or_default() += 1;
            }
        }
    }
    graph::clear_memos();
    Ok((
        json!({"total":candidates.len(),"by_reason":counts}),
        json!(candidates),
    ))
}
fn candidate(
    out: &Value,
    inbound: &Value,
    amount: &BigInt,
    reason: &str,
    conflict: &str,
    confidence: &str,
    size: usize,
) -> Value {
    let mut v = json!({"out_id":string(&out["id"]),"in_id":string(&inbound["id"]),"out_amount_msat":number(&amount.to_string()).expect("integer"),"in_amount_msat":number(&int(&inbound["amount"]).to_string()).expect("integer"),"confidence":confidence,"method":"ownership_graph","swap_fee_msat":number(&(amount-int(&inbound["amount"])).to_string()).expect("integer"),"swap_fee_kind":"ownership_graph_delta","default_kind":"manual","default_policy":"carrying-value","conflict_set_id":conflict,"conflict_size":size,"evidence_provider":"ownership_graph","evidence_id":reason,"evidence_kind":"owned_output","evidence_conflicts":[],"evidence_send_amount_msat":null,"evidence_receive_amount_msat":null});
    for (prefix, row) in [("out", out), ("in", inbound)] {
        for key in [
            "asset",
            "wallet_id",
            "wallet_label",
            "wallet_kind",
            "occurred_at",
        ] {
            let value = if key == "wallet_kind" && !truth(&row[key]) {
                String::new()
            } else {
                string(&row[key])
            };
            v[format!("{prefix}_{key}")] = json!(value);
        }
    }
    for k in [
        "evidence_status",
        "evidence_version",
        "evidence_taproot",
        "evidence_cooperative",
        "evidence_spend_path",
    ] {
        v[k] = json!("");
    }
    v
}

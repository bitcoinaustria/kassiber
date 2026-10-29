//! Private selected-row and complete diagnostic composition.
//! Snapshot stage admission remains a separate parity gate.
use super::*;
use crate::python_text::PythonText;
use std::collections::BTreeMap;
fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn internal(d: &Value) -> bool {
    !d["target"].is_null()
        && matches!(
            d["state"].as_str(),
            Some("internal_verified" | "internal_reviewed")
        )
}
fn strings(v: &Value) -> BTreeSet<String> {
    array(v)
        .iter()
        .filter(|v| codec::truth(v))
        .map(string)
        .collect()
}
fn slice_key(v: &Value) -> Result<(String, BigInt, BigInt), String> {
    Ok((
        string(&v["observation_hash"]),
        integer_big(&v["start_msat"])?,
        integer_big(&v["end_msat"])?,
    ))
}
fn observation<'a>(
    observations: &'a BTreeMap<String, Value>,
    slice: &Value,
) -> Result<&'a Value, String> {
    observations
        .get(&string(&slice["observation_hash"]))
        .ok_or_else(|| "KeyError".into())
}
/// Close selected internal paths and propagate their complete root origins.
pub(super) fn connected(
    decisions: &[Value],
    observations: &BTreeMap<String, Value>,
    explicit: &BTreeSet<String>,
) -> Result<Value, String> {
    let mut neighbors: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for d in decisions.iter().filter(|d| internal(d)) {
        let a = string(&observation(observations, &d["source"])?["anchor_transaction_id"]);
        let b = string(&observation(observations, &d["target"])?["anchor_transaction_id"]);
        neighbors.entry(a.clone()).or_default().insert(b.clone());
        neighbors.entry(b).or_default().insert(a);
    }
    let mut blocked = explicit.clone();
    let mut frontier: Vec<String> = explicit.iter().cloned().collect();
    while let Some(id) = frontier.pop() {
        for neighbor in neighbors.get(&id).into_iter().flatten() {
            if blocked.insert(neighbor.clone()) {
                frontier.push(neighbor.clone());
            }
        }
    }
    let mut origins: BTreeMap<String, BTreeSet<String>> = explicit
        .iter()
        .map(|id| (id.clone(), BTreeSet::from([id.clone()])))
        .collect();
    let mut frontier: Vec<_> = explicit.iter().cloned().collect();
    while let Some(id) = frontier.pop() {
        let inherited = origins[&id].clone();
        for neighbor in neighbors.get(&id).into_iter().flatten() {
            let known = origins.entry(neighbor.clone()).or_default();
            if !inherited.is_subset(known) {
                known.extend(inherited.clone());
                // Roots can bridge other roots; propagate every growing set.
                frontier.push(neighbor.clone());
            }
        }
    }
    Ok(json!({"blocked":blocked,"origins":origins}))
}
pub(super) fn pool(o: &Value) -> Value {
    let network = match o["event_key"]["network"].as_str().unwrap_or("") {
        "liquidv1" => "main",
        "liquidtestnet" => "test",
        "elementsregtest" => "regtest",
        n => n,
    };
    let asset = string(&o["asset"]);
    let exposure = if matches!(asset.as_str(), "BTC" | "LBTC") {
        "bitcoin".into()
    } else {
        format!("asset:{asset}")
    };
    json!({"profile_id":o["profile_id"],"network":network,"exposure":exposure,"unit":"msat"})
}
pub(super) fn barrier(o: &Value, v: &Value) -> Option<Value> {
    let own = pool(o);
    array(&v["pool_barriers"])
        .iter()
        .find(|p| {
            ["profile_id", "network", "exposure", "unit"]
                .iter()
                .all(|k| p[0][*k] == own[*k])
        })
        .map(|p| p[1].clone())
}
fn orders(events: &Value) -> BTreeMap<String, Vec<String>> {
    let mut result = BTreeMap::new();
    for event in array(events) {
        let mut order = vec![array(&event["legs"])
            .iter()
            .map(|o| string(&o["occurred_at"]))
            .min()
            .unwrap_or_default()];
        for k in ["chain", "network", "native_namespace", "native_event_id"] {
            order.push(string(&event["event_key"][k]));
        }
        for leg in array(&event["legs"]) {
            result.insert(string(&leg["quantity_hash"]), order.clone());
        }
    }
    result
}
fn held(o: &Value, v: &Value, orders: &BTreeMap<String, Vec<String>>) -> bool {
    let Some(b) = barrier(o, v) else {
        return false;
    };
    let Some(order) = orders.get(&string(&o["quantity_hash"])) else {
        return true;
    };
    let b: Vec<String> = array(&b).iter().map(string).collect();
    order[0] > b[0] || (order[0] == b[0] && *order != b)
}
fn privacy(row: &Value) -> bool {
    matches!(
        text(&row["privacy_boundary"])
            .py_strip()
            .py_lower()
            .replace('-', "_")
            .as_str(),
        "coinjoin" | "collaborative" | "payjoin" | "payment_in_coinjoin" | "sweep"
    )
}
pub(super) fn evaluate(v: &Value) -> Result<Value, String> {
    let diagnostics = codec::from_json(&v["interpreter_quarantines"])?;
    evaluate_lossless(v, crate::graph::stored::list(&diagnostics)).map(|v| codec::to_json(&v))
}

/// Diagnostic records remain CPython values for their entire lifetime.
pub(super) fn evaluate_lossless(v: &Value, diagnostics: &[Py]) -> Result<Py, String> {
    let finalize = v["op"] == "finalize";
    let payouts = if v["op"] == "compose" || finalize {
        payouts::valid(&v["payouts"], &v["rows"])?
    } else {
        Value::Null
    };
    let observations: BTreeMap<String, Value> = array(&v["observations"])
        .iter()
        .map(|o| (string(&o["quantity_hash"]), o.clone()))
        .collect();
    let mut decisions = array(&v["decisions"]).to_vec();
    // Resolve every source before sorting, matching Python's key admission.
    let mut keyed = Vec::new();
    for d in decisions.drain(..) {
        let source = observation(&observations, &d["source"])?;
        keyed.push((
            (string(&source["occurred_at"]), slice_key(&d["source"])?),
            d,
        ));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    decisions = keyed.into_iter().map(|(_, d)| d).collect();
    let explicit = strings(&v["blocked_transaction_ids"]);
    let closure = connected(&decisions, &observations, &explicit)?;
    if v["op"] == "connected" {
        return codec::from_json(&json!({"blocked":closure["blocked"]}));
    }
    let blocked = strings(&closure["blocked"]);
    let non_events = strings(&v["non_event_transaction_ids"]);
    let mut blocked_anchors = strings(&v["blocked_anchor_ids"]);
    if finalize {
        for issue in array(&v["issues"]) {
            if text(&issue["reason"]).starts_with("custody_component_")
                || matches!(
                    issue["issue_type"].as_str(),
                    Some("component_claim_compile_failed" | "quantity_claim_bundle_invalid")
                )
            {
                blocked_anchors.extend(array(&issue["transaction_ids"]).iter().map(string));
            }
        }
    }
    let order = orders(&v["events"]);
    if v["op"] == "basis" {
        return codec::from_json(&json!(array(&v["observations"])
            .iter()
            .map(|o| held(o, v, &order))
            .collect::<Vec<_>>()));
    }
    let eligible: BTreeSet<_> = array(&v["eligible"])
        .iter()
        .map(slice_key)
        .collect::<Result<_, _>>()?;
    let rows = &v["rows"];
    let mut projected = Vec::new();
    let mut intra = Vec::new();
    let mut cross = Vec::new();
    let mut moves = Vec::new();
    let mut barrier_receipts = Vec::new();
    let mut quarantines =
        super::quarantines::Quarantines::with_interpreter(v, &closure, diagnostics)?;
    let mut fees = FeeAllocation::new(&decisions)?;
    for d in &decisions {
        let source = observation(&observations, &d["source"])?;
        let target = if d["target"].is_null() {
            None
        } else {
            Some(observation(&observations, &d["target"])?)
        };
        if blocked.contains(&string(&source["anchor_transaction_id"]))
            || target.is_some_and(|t| blocked.contains(&string(&t["anchor_transaction_id"])))
        {
            continue;
        }
        if !eligible.contains(&slice_key(&d["source"])?) {
            if finalize {
                quarantines.basis(source)?;
            }
            continue;
        }
        if internal(d) {
            let target = target.expect("internal has target");
            let out = slice_row(
                source,
                &d["source"],
                rows,
                "move-out",
                &fees.move_fee(d, source)?,
            )?;
            let input = slice_row(target, &d["target"], rows, "move-in", &BigInt::from(0))?;
            let id = if codec::truth(&d["selected_claim_id"]) {
                d["selected_claim_id"].clone()
            } else {
                out["id"].clone()
            };
            moves.push(id.clone());
            if source["asset"] == target["asset"] {
                let mut pair = json!({"out":out,"in":input,"pair_id":id,"source":d["reason"],"policy":"carrying-value","group_id":d["atomic_bundle_id"]});
                if privacy(&out) || privacy(&input) {
                    pair["kind"] = json!("custody-reviewed-coinjoin");
                }
                intra.push(pair);
            } else {
                let id_text = string(&id);
                let component = id_text
                    .strip_prefix("component:")
                    .map(|s| s.split(':').next().unwrap());
                let durable = id_text
                    .strip_prefix("pair:")
                    .map(|s| s.split(':').next().unwrap())
                    .unwrap_or(&id_text);
                cross.push(json!({"pair_id":durable,"component_id":if codec::truth(&d["component_id"]) {d["component_id"].clone()} else {json!(component)},"out_id":out["id"],"in_id":input["id"],"out_transaction_id":source["anchor_transaction_id"],"in_transaction_id":target["anchor_transaction_id"],"out_asset":source["asset"],"in_asset":target["asset"],"policy":if codec::truth(&d["transfer_policy"]) {d["transfer_policy"].clone()} else {json!("carrying-value")},"kind":if codec::truth(&d["transfer_kind"]) {d["transfer_kind"].clone()} else {json!("custody_cross_rail")}}));
            }
            projected.extend([out, input]);
            continue;
        }
        if non_events.contains(&string(&source["anchor_transaction_id"])) {
            let fee = fees.raw(source)?;
            if fee != BigInt::from(0) {
                projected.push(fee_row(source, &fee, rows, "non-event-fee")?);
            }
            continue;
        }
        if !matches!(
            d["state"].as_str(),
            Some("external_confirmed" | "external_presumed")
        ) {
            continue;
        }
        if d["destination_kind"] == "fee" {
            if let Some(amount) = fees.reviewed_fee(d)? {
                projected.push(fee_row(source, &amount, rows, "reviewed-fee")?);
            }
            continue;
        }
        let payout = &payouts[string(&source["anchor_transaction_id"])];
        let asset = text(&payout["payout_asset"]).py_upper();
        let source_asset = string(&source["asset"]);
        let carry = codec::truth(payout)
            && payout["policy"] == "carrying-value"
            && source_asset != asset
            && (text(&v["profile"]["tax_country"]).py_lower() == "at"
                || (BTreeSet::from([
                    source_asset.py_strip().py_upper(),
                    asset.py_strip().py_upper(),
                ]) == BTreeSet::from(["BTC".to_owned(), "LBTC".to_owned()])));
        if carry {
            let settlement = payouts::settlement(source, payout, rows)?;
            if settlement.is_null() {
                if finalize {
                    quarantines.payout(source, payout)?;
                }
                continue;
            }
            let out = slice_row(
                source,
                &d["source"],
                rows,
                "direct-payout-source",
                &fees.raw(source)?,
            )?;
            let input = settlement[0].clone();
            cross.push(json!({"pair_id":format!("direct-payout:{}",string(&payout["id"])),"component_id":null,"kind":if codec::truth(&payout["kind"]) {payout["kind"].clone()} else {json!("direct-swap-payout")},"policy":"carrying-value","out_id":out["id"],"in_id":input["id"],"out_transaction_id":source["anchor_transaction_id"],"in_transaction_id":null,"out_asset":source["asset"],"in_asset":asset}));
            projected.extend([out, input, settlement[1].clone()]);
            continue;
        }
        let mut row = slice_row(source, &d["source"], rows, "external", &fees.raw(source)?)?;
        if !d["external_economic_subtype"].is_null() {
            row["kind"] = d["external_economic_subtype"].clone();
            row["custody_external_economic_subtype"] = d["external_economic_subtype"].clone();
            row["privacy_boundary"] = Value::Null;
        }
        let amount =
            integer_big(&d["source"]["end_msat"])? - integer_big(&d["source"]["start_msat"])?;
        projected.push(payouts::price(row, payout, &amount)?);
    }
    // Index the complete decision population once, including unselected targets.
    // Re-scanning it for every receipt makes finalization quadratic.
    let mut targets_by_observation: BTreeMap<String, Vec<(BigInt, BigInt)>> = BTreeMap::new();
    for decision in &decisions {
        if !decision["target"].is_null() {
            let (hash, start, end) = slice_key(&decision["target"])?;
            targets_by_observation
                .entry(hash)
                .or_default()
                .push((start, end));
        }
    }
    let mut ordered: Vec<_> = array(&v["observations"]).iter().collect();
    ordered.sort_by_key(|o| (string(&o["occurred_at"]), string(&o["quantity_hash"])));
    for o in ordered {
        if o["direction"] != "inbound" || rows.get(string(&o["transaction_id"])).is_none() {
            continue;
        }
        let anchor = string(&o["anchor_transaction_id"]);
        if non_events.contains(&anchor) {
            continue;
        }
        if held(o, v, &order) {
            barrier_receipts.push(o["quantity_hash"].clone());
            continue;
        }
        if blocked.contains(&anchor) || blocked_anchors.contains(&anchor) {
            continue;
        }
        let mut targets = targets_by_observation
            .get(&string(&o["quantity_hash"]))
            .cloned()
            .unwrap_or_default();
        targets.sort();
        let mut cursor = BigInt::from(0);
        for (start, end) in targets {
            if start > cursor {
                let q = json!({"observation_hash":o["quantity_hash"],"start_msat":number(&cursor),"end_msat":number(&start)});
                projected.push(slice_row(o, &q, rows, "inbound", &BigInt::from(0))?);
            }
            cursor = cursor.max(end);
        }
        let principal = integer_big(&o["amount_msat"])?;
        if cursor < principal {
            let q = json!({"observation_hash":o["quantity_hash"],"start_msat":number(&cursor),"end_msat":number(&principal)});
            projected.push(slice_row(o, &q, rows, "inbound", &BigInt::from(0))?);
        }
    }
    let hashes: BTreeSet<_> = projected
        .iter()
        .map(|r| text(&r["custody_quantity_hash"]))
        .collect();
    for o in array(&v["observations"]) {
        if o["direction"] == "outbound"
            && integer_big(&o["fee_msat"])? > BigInt::from(0)
            && !hashes.contains(&string(&o["quantity_hash"]))
            && !blocked.contains(&string(&o["anchor_transaction_id"]))
            && !non_events.contains(&string(&o["anchor_transaction_id"]))
            && !held(o, v, &order)
        {
            projected.push(fee_row(o, &integer_big(&o["fee_msat"])?, rows, "fee-only")?);
        }
    }
    projected.sort_by_key(|r| (string(&r["occurred_at"]), string(&r["id"])));
    moves.sort_by_key(string);
    if finalize {
        quarantines.finish(&projected, &barrier_receipts, &observations)?;
        cross = super::conversions::evaluate(
            &json!({"rows":projected,"pairs":cross,"reviewed":array(&v["reviewed"])}),
        )?
        .as_array()
        .expect("conversion array")
        .clone();
        let quarantines = quarantines.sorted()?;
        validate(&projected, &intra).map_err(|(family, _)| family.to_owned())?;
        let mut result = codec::from_json(&json!({"$type":"FinalizedTaxProjection",
            "rows":projected,"intra_pairs":intra,"cross_asset_pairs":cross,"selected_move_ids":moves}))?;
        crate::graph::stored::put(&mut result, "quarantines", Py::List(quarantines));
        return Ok(result);
    }
    validate(&projected, &intra).map_err(|(family, _)| family.to_owned())?;
    codec::from_json(
        &json!({"rows":projected,"intra_pairs":intra,"cross_asset_pairs":cross,"selected_move_ids":moves,"barrier_held_receipts":barrier_receipts,"blocked_transaction_ids":closure["blocked"]}),
    )
}

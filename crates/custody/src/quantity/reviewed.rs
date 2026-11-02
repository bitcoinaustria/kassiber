//! Reviewed allocations are comparison-only compiler inputs, never snapshot facts.
use super::{array, exact, number, principal};
use crate::evidence::canonical::LegRef;
use crate::{codec, python_text::PythonText};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

#[derive(Debug)]
pub struct Error {
    pub code: String,
    pub message: String,
    pub details: Value,
}
fn error(message: &str, details: Value) -> Error {
    Error {
        code: "custody_component_claim_compile".into(),
        message: message.into(),
        details,
    }
}
fn family(code: &str) -> Error {
    Error {
        code: code.into(),
        message: code.into(),
        details: json!({}),
    }
}
fn s(v: &Value) -> String {
    codec::pystr(v)
}
fn text(v: &Value) -> String {
    if codec::truth(v) {
        s(v)
    } else {
        String::new()
    }
}
fn fallback(a: &Value, b: &Value) -> Value {
    if codec::truth(a) {
        a.clone()
    } else {
        b.clone()
    }
}
fn integer(v: &Value) -> Result<BigInt, Error> {
    let v = if codec::truth(v) { v.clone() } else { json!(0) };
    if v.is_array() || v.is_object() {
        let mut e = family("TypeError");
        e.message = format!(
            "int() argument must be a string, a bytes-like object or a real number, not '{}'",
            if v.is_array() { "list" } else { "dict" }
        );
        return Err(e);
    }
    let parsed = codec::integer(&v).map_err(|_| {
        let mut e = family("ValueError");
        e.message = format!(
            "invalid literal for int() with base 10: {}",
            crate::interpretation::python_value_string(&kassiber_pyjson::Value::List(vec![
                codec::from_json(&v).expect("JSON operand")
            ]))
            .trim_start_matches('[')
            .trim_end_matches(']')
        );
        e
    })?;
    exact(&parsed).map_err(|_| family("ValueError"))
}
fn tx(l: &Value) -> String {
    text(&fallback(&l["anchor_transaction_id"], &l["transaction_id"]))
}
fn location(l: &Value) -> Result<String, Error> {
    let loc = text(&fallback(&l["wallet_id"], &l["location_ref"]));
    if loc.is_empty() {
        return Err(error(
            "transaction-less route leg needs a custody location",
            json!({"leg_id":l["id"]}),
        ));
    }
    Ok(serde_json::to_string(&json!([
        loc,
        text(&l["asset"]).py_upper(),
        text(&l["exposure"]),
        text(&fallback(&l["conservation_unit"], &json!("msat"))),
        text(&l["rail"])
    ]))
    .unwrap())
}
fn infer(c: &Value) -> Result<Vec<Value>, Error> {
    let mut explicit = array(&c["allocations"]).to_vec();
    if !explicit.is_empty() {
        let mut keyed = explicit
            .drain(..)
            .map(|a| Ok((integer(&a["ordinal"])?, text(&a["id"]), a)))
            .collect::<Result<Vec<_>, Error>>()?;
        keyed.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        return Ok(keyed.into_iter().map(|(_, _, a)| a).collect());
    }
    let legs = array(&c["legs"])
        .iter()
        .map(|l| Ok((l, integer(&l["amount_msat"])?)))
        .collect::<Result<Vec<_>, Error>>()?;
    let sources: Vec<_> = legs
        .iter()
        .filter(|(l, n)| l["role"] == "source" && *n > BigInt::from(0))
        .collect();
    let sinks: Vec<_> = legs
        .iter()
        .filter(|(l, n)| l["role"] != "source" && *n > BigInt::from(0))
        .collect();
    let quantity = text(&fallback(&c["conservation_mode"], &json!("quantity"))) == "quantity";
    let allocation_error = |message: &str| Error {
        code: "custody_component_allocation_required".into(),
        message: message.into(),
        details: json!({}),
    };
    if !quantity && (sources.len() != 1 || sinks.len() != 1) {
        return Err(allocation_error(
            "multi-leg conversion components require explicit source-to-sink allocations",
        ));
    }
    let pairs: Vec<_> = if sources.len() == 1 {
        sinks.iter().map(|t| (sources[0], *t)).collect()
    } else if sinks.len() == 1
        && matches!(
            sinks[0].0["role"].as_str(),
            Some("destination" | "retained")
        )
    {
        sources.iter().map(|src| (*src, sinks[0])).collect()
    } else {
        return Err(allocation_error(
            "N:M custody components require explicit source-to-sink allocations",
        ));
    };
    Ok(pairs.into_iter().enumerate().map(|(i,(src,sink))| json!({"id":format!("inferred:{i}"),"ordinal":i,"source_leg_id":src.0["id"],"sink_leg_id":sink.0["id"],"source_amount_msat":number(if quantity && sources.len()==1 { &sink.1 } else { &src.1 }),"sink_amount_msat":number(if sources.len()==1 { &sink.1 } else { &src.1 })})).collect())
}
type Token = (String, BigInt, Vec<String>);
fn flatten(
    id: &str,
    legs: &BTreeMap<String, &Value>,
    allocations: Vec<Value>,
    conversion: bool,
    retained: &BTreeSet<String>,
) -> Result<Vec<Value>, Error> {
    let mut pending = allocations;
    let mut pools: BTreeMap<String, VecDeque<Token>> = BTreeMap::new();
    let mut flattened = Vec::new();
    while !pending.is_empty() {
        let mut progressed = false;
        let mut index = 0;
        while index < pending.len() {
            let a = &pending[index];
            let src_id = text(&a["source_leg_id"]);
            let sink_id = text(&a["sink_leg_id"]);
            let src = legs.get(&src_id);
            let sink = legs.get(&sink_id);
            if src.is_none() || sink.is_none() || src.unwrap()["role"] != "source" {
                return Err(error(
                    "component allocation endpoints are invalid",
                    json!({"component_id":id,"allocation_id":a["id"]}),
                ));
            }
            let src = src.unwrap();
            let sink = sink.unwrap();
            let amount = integer(&a["source_amount_msat"])?;
            let target = integer(&a["sink_amount_msat"])?;
            if amount <= BigInt::from(0) && target <= BigInt::from(0) {
                pending.remove(index);
                progressed = true;
                continue;
            }
            if amount != target {
                if !conversion {
                    return Err(error(
                        "quantity claim allocation does not conserve exact msat",
                        json!({"component_id":id,"allocation_id":a["id"]}),
                    ));
                }
                if tx(src).is_empty() {
                    return Err(error(
                        "transaction-less conversion sources cannot emit tax claims",
                        json!({"component_id":id,"allocation_id":a["id"]}),
                    ));
                }
                flattened.push(pending.remove(index));
                progressed = true;
                continue;
            }
            let aid = if codec::truth(&a["id"]) {
                s(&a["id"])
            } else {
                flattened.len().to_string()
            };
            let tokens = if !tx(src).is_empty() {
                vec![(src_id, amount.clone(), vec![aid])]
            } else {
                let key = location(src)?;
                let available: BigInt = pools
                    .get(&key)
                    .into_iter()
                    .flatten()
                    .map(|(_, n, _)| n)
                    .sum();
                if available < amount {
                    index += 1;
                    continue;
                }
                let queue = pools.entry(key).or_default();
                let mut remaining = amount.clone();
                let mut tokens = Vec::new();
                while remaining != BigInt::from(0) {
                    let (origin, n, mut path) =
                        queue.pop_front().ok_or_else(|| family("IndexError"))?;
                    let used = remaining.clone().min(n.clone());
                    remaining -= &used;
                    if used < n {
                        queue.push_front((origin.clone(), n - &used, path.clone()));
                    }
                    path.push(aid.clone());
                    tokens.push((origin, used, path));
                }
                tokens
            };
            if matches!(sink["role"].as_str(), Some("destination" | "retained"))
                && tx(sink).is_empty()
                && !retained.contains(&sink_id)
            {
                pools.entry(location(sink)?).or_default().extend(tokens);
            } else {
                for (i, (origin, n, path)) in tokens.into_iter().enumerate() {
                    let mut out = a.clone();
                    out["id"] = if path.len() > 1 {
                        json!(format!("route:{}:{i}", path.join(":")))
                    } else {
                        a["id"].clone()
                    };
                    out["source_leg_id"] = json!(origin);
                    out["source_amount_msat"] = number(&n);
                    out["sink_amount_msat"] = number(&n);
                    flattened.push(out);
                }
            }
            pending.remove(index);
            progressed = true;
        }
        if !progressed {
            let mut ids: Vec<_> = pending.iter().map(|a| text(&a["id"])).collect();
            ids.sort();
            return Err(error(
                "transaction-less component route is incomplete or ambiguous",
                json!({"component_id":id,"pending_allocation_ids":ids}),
            ));
        }
    }
    let mut stranded = BTreeMap::new();
    for (key, tokens) in pools {
        let amount: BigInt = tokens.iter().map(|(_, n, _)| n).sum();
        if amount > BigInt::from(0) {
            let parts: Vec<String> = serde_json::from_str(&key).unwrap();
            stranded.insert(parts.join("|"), number(&amount));
        }
    }
    if !stranded.is_empty() {
        return Err(error(
            "transaction-less component route ends without an observed or classified sink",
            json!({"component_id":id,"stranded_msat":stranded}),
        ));
    }
    Ok(flattened)
}
fn payout(
    term: &Value,
    sink: &Value,
    source: LegRef<'_>,
    id: &str,
    amount: &BigInt,
    target: &BigInt,
) -> Value {
    json!({"id":s(&term["legacy_source_id"]),"component_id":id,"out_transaction_id":source["anchor_transaction_id"],"deleted_at":null,"kind":text(&fallback(&term["review_kind"],&json!("direct-swap-payout"))),"policy":text(&fallback(&term["tax_policy"],&json!("taxable"))),"out_amount":number(amount),"payout_asset":fallback(&term["payout_asset"],&sink["asset"]),"payout_amount":fallback(&term["payout_amount_msat"],&number(target)),"payout_occurred_at":fallback(&term["payout_occurred_at"],&sink["occurred_at"]),"payout_fiat_value":term["payout_fiat_value_exact"],"payout_external_id":term["payout_external_id"],"counterparty":term["counterparty"],"swap_fee_msat":term["swap_fee_msat"],"swap_fee_kind":term["swap_fee_kind"],"notes":term["review_notes"],"confidence_at_review":term["confidence_at_review"],"review_source":term["review_source"]})
}
/// Observation pairs in order, indexed for the lookups a compilation makes.
pub struct Observations<'a> {
    entries: Vec<(String, LegRef<'a>)>,
    last: HashMap<String, usize>,
    first_hash: HashMap<String, usize>,
    positions: HashMap<String, Vec<usize>>,
}
impl<'a> Observations<'a> {
    pub fn new(entries: Vec<(String, LegRef<'a>)>) -> Self {
        let mut last = HashMap::new();
        let mut first_hash = HashMap::new();
        let mut positions: HashMap<String, Vec<usize>> = HashMap::new();
        for (at, (key, o)) in entries.iter().enumerate() {
            last.insert(key.clone(), at);
            if let Some(hash) = o["quantity_hash"].as_str() {
                first_hash.entry(hash.to_owned()).or_insert(at);
            }
            positions.entry(key.clone()).or_default().push(at);
        }
        Self {
            entries,
            last,
            first_hash,
            positions,
        }
    }
}

/// Compile a materialized reviewed component; no journal or tax rows are emitted.
pub fn compile(c: &Value, observations: &Value) -> Result<Value, Error> {
    let observed = if observations.is_array() {
        Some(Observations::new(
            array(observations)
                .iter()
                .map(|p| (s(&p[0]), LegRef::Json(&p[1])))
                .collect(),
        ))
    } else {
        observations.as_object().map(|o| {
            Observations::new(
                o.iter()
                    .map(|(k, v)| (k.clone(), LegRef::Json(v)))
                    .collect(),
            )
        })
    };
    compile_observed(c, observed.as_ref())
}

/// `compile` over prepared observations; `None` is a non-sequence operand,
/// refused at the point the original conversion refused it.
pub fn compile_observed(c: &Value, observations: Option<&Observations>) -> Result<Value, Error> {
    let id = text(&c["id"]);
    if id.is_empty() {
        return Err(error("component id is required", json!({})));
    }
    if c["effective_state"] != "active" {
        return Err(error(
            "only an effective active component can emit quantity claims",
            json!({"component_id":id,"effective_state":c["effective_state"]}),
        ));
    }
    let mut legs = BTreeMap::new();
    for l in array(&c["legs"]) {
        let key = l.get("id").ok_or_else(|| family("KeyError"))?;
        legs.insert(s(key), l);
    }
    let mut leg_order = Vec::new();
    for l in array(&c["legs"]) {
        let k = s(&l["id"]);
        if !leg_order.contains(&k) {
            leg_order.push(k);
        }
    }
    let ordered_legs: Vec<_> = leg_order.iter().map(|k| &legs[k]).collect();
    let guided =
        c["component_type"] == "manual_bridge" && c["evidence_kind"] == "custody_gap_review";
    let mut residual = text(&c["evidence"]["residual_classification"]["classification"]);
    let residuals: BTreeSet<_> = legs
        .values()
        .filter(|l| guided && text(&l["notes"]).starts_with("reviewed_residual:"))
        .map(|l| {
            text(&l["notes"])
                .strip_prefix("reviewed_residual:")
                .unwrap()
                .to_owned()
        })
        .collect();
    if residual.is_empty() && residuals.len() == 1 {
        residual = residuals.into_iter().next().unwrap();
    }
    let kinds: BTreeSet<_> = array(&c["economic_terms"])
        .iter()
        .filter(|t| {
            t["term_kind"] == "transaction_pair"
                && !t["review_kind"].is_null()
                && t["review_kind"] != ""
        })
        .map(|t| text(&t["review_kind"]))
        .collect();
    let transfer_kind = if kinds.len() == 1 {
        json!(kinds.into_iter().next().unwrap())
    } else {
        Value::Null
    };
    let mut retained: BTreeSet<_> = legs
        .iter()
        .filter(|(_, l)| {
            l["role"] == "retained"
                && tx(l).is_empty()
                && l["notes"] == "reviewed_residual:retained_custody"
        })
        .map(|(id, _)| id.clone())
        .collect();
    retained.extend(
        array(&c["economic_terms"])
            .iter()
            .filter(|t| {
                t["term_kind"] == "direct_swap_payout"
                    && !t["target_leg_id"].is_null()
                    && t["target_leg_id"] != ""
            })
            .map(|t| s(&t["target_leg_id"])),
    );
    if array(&c["allocations"]).is_empty() {
        for l in &ordered_legs {
            if l["role"] == "suspense" && integer(&l["amount_msat"])? > BigInt::from(0) {
                return Err(error(
                    "suspense components require explicit allocations",
                    json!({"component_id":id}),
                ));
            }
        }
    }
    let conversion = text(&fallback(&c["conservation_mode"], &json!("quantity"))) == "conversion";
    let inferred = infer(c).map_err(|mut e| {
        if e.code.starts_with("custody_") {
            e.details["projection_code"] = json!(e.code);
            e.details["component_id"] = json!(id);
            e.code = "custody_component_claim_compile".into();
        }
        e
    })?;
    let allocations = flatten(&id, &legs, inferred, conversion, &retained)?;
    let observations = observations.ok_or_else(|| family("TypeError"))?;
    // The last pair with this key, as a reversed scan finds it.
    let observation = |key: &str| {
        observations
            .last
            .get(key)
            .map(|&at| observations.entries[at].1)
            .ok_or_else(|| {
                error(
                    "component allocation references an unavailable observation",
                    json!({"component_id":id,"transaction_id":key}),
                )
            })
    };
    let mut sources: BTreeMap<String, BigInt> = BTreeMap::new();
    let mut targets: BTreeMap<String, BigInt> = BTreeMap::new();
    if conversion {
        let mut totals: Vec<(String, BigInt)> = Vec::new();
        for a in &allocations {
            if let Some(l) = legs.get(&s(&a["source_leg_id"])) {
                if l["role"] == "source" && !tx(l).is_empty() {
                    let o = observation(&tx(l))?;
                    let hash = text(&o["quantity_hash"]);
                    let amount = integer(&a["source_amount_msat"])?;
                    if let Some((_, n)) = totals.iter_mut().find(|(h, _)| *h == hash) {
                        *n += amount;
                    } else {
                        totals.push((hash, amount));
                    }
                }
            }
        }
        for (hash, n) in totals {
            let o = observations
                .first_hash
                .get(&hash)
                .map(|&at| observations.entries[at].1)
                .unwrap();
            let cap = principal(&o).map_err(|e| family(&e))?;
            if n > cap {
                return Err(error(
                    "component claims exceed observed source principal",
                    json!({"component_id":id,"transaction_id":o["transaction_id"],"claimed_msat":number(&n),"principal_msat":number(&cap)}),
                ));
            }
            sources.insert(hash, cap - n);
        }
    }
    let mut claims = Vec::new();
    let mut pairs = Vec::new();
    let mut payouts = Vec::new();
    let mut suspense = BigInt::from(0);
    for (i, a) in allocations.iter().enumerate() {
        let src = legs.get(&s(&a["source_leg_id"]));
        let sink = legs.get(&s(&a["sink_leg_id"]));
        if src.is_none() || sink.is_none() || src.unwrap()["role"] != "source" {
            return Err(error(
                "component allocation endpoints are invalid",
                json!({"component_id":id,"allocation_id":a["id"]}),
            ));
        }
        let src = src.unwrap();
        let sink = sink.unwrap();
        let amount = integer(&a["source_amount_msat"])?;
        let target_amount = integer(&a["sink_amount_msat"])?;
        if amount <= BigInt::from(0) && target_amount <= BigInt::from(0) {
            continue;
        }
        if !conversion && amount != target_amount {
            return Err(error(
                "quantity claim allocation does not conserve exact msat",
                json!({"component_id":id,"allocation_id":a["id"]}),
            ));
        }
        if tx(src).is_empty() {
            return Err(error(
                "flattened component source is not observed",
                json!({"component_id":id,"source_leg_id":src["id"]}),
            ));
        }
        let o = observation(&tx(src))?;
        if o["direction"] != "outbound" {
            return Err(error(
                "quantity claim source observation is not outbound",
                json!({"component_id":id,"source_leg_id":src["id"]}),
            ));
        }
        let role = text(&sink["role"]);
        if !conversion && role == "fee" && sink["notes"] != "reviewed_residual:network_fee" {
            let fee = exact(&o["fee_msat"]).map_err(|e| family(&e))?;
            if amount != fee {
                return Err(error(
                    "component fee allocation does not match the observed fee",
                    json!({"component_id":id,"transaction_id":o["transaction_id"],"claimed_fee_msat":number(&amount),"observed_fee_msat":number(&fee)}),
                ));
            }
            continue;
        }
        let hash = text(&o["quantity_hash"]);
        let start = sources.get(&hash).cloned().unwrap_or_default();
        let end = &start + &amount;
        let cap = principal(&o).map_err(|e| family(&e))?;
        if end > cap {
            return Err(error(
                "component claims exceed observed source principal",
                json!({"component_id":id,"transaction_id":o["transaction_id"],"claimed_msat":number(&end),"principal_msat":number(&cap)}),
            ));
        }
        sources.insert(hash.clone(), end.clone());
        if hash.is_empty() {
            let mut e = family("ValueError");
            e.message = "quantity slices require an observation hash".into();
            return Err(e);
        }
        if start < BigInt::from(0) || end <= start {
            let mut e = family("ValueError");
            e.message = "quantity slices require a non-empty half-open msat range".into();
            return Err(e);
        }
        let term = array(&c["economic_terms"])
            .iter()
            .rev()
            .find(|t| {
                s(&t["source_leg_id"]) == s(&src["id"]) && s(&t["target_leg_id"]) == s(&sink["id"])
            })
            .cloned()
            .unwrap_or(json!({}));
        let mut target = Value::Null;
        let mut cross = false;
        let (state, reason) = match role.as_str() {
            "destination" | "retained" => {
                if tx(sink).is_empty() {
                    if conversion && term["term_kind"] == "direct_swap_payout" {
                        payouts.push(payout(&term, sink, o, &id, &amount, &target_amount));
                        ("external_confirmed", "reviewed_direct_payout_source")
                    } else if !retained.contains(&text(&sink["id"])) {
                        return Err(error(
                            "flattened component target is not observed",
                            json!({"component_id":id,"sink_leg_id":sink["id"]}),
                        ));
                    } else {
                        ("internal_reviewed", "reviewed_retained_custody")
                    }
                } else {
                    let t = observation(&tx(sink))?;
                    if t["direction"] != "inbound" {
                        return Err(error(
                            "quantity claim target observation is not inbound",
                            json!({"component_id":id,"sink_leg_id":sink["id"]}),
                        ));
                    }
                    let th = text(&t["quantity_hash"]);
                    let ts = targets.get(&th).cloned().unwrap_or_default();
                    let te = &ts + &target_amount;
                    let tc = principal(&t).map_err(|e| family(&e))?;
                    if te > tc {
                        return Err(error(
                            "component claims exceed observed target principal",
                            json!({"component_id":id,"transaction_id":t["transaction_id"],"claimed_msat":number(&te),"principal_msat":number(&tc)}),
                        ));
                    }
                    targets.insert(th.clone(), te.clone());
                    if !conversion || (o["asset"] == t["asset"] && amount == target_amount) {
                        if th.is_empty() {
                            let mut e = family("ValueError");
                            e.message = "quantity slices require an observation hash".into();
                            return Err(e);
                        }
                        if ts < BigInt::from(0) || te <= ts {
                            let mut e = family("ValueError");
                            e.message =
                                "quantity slices require a non-empty half-open msat range".into();
                            return Err(e);
                        }
                        target = json!({"$type":"QuantitySlice","observation_hash":th,"start_msat":number(&ts),"end_msat":number(&te)});
                        cross = o["event_key"]["chain"] != t["event_key"]["chain"];
                        (
                            "internal_reviewed",
                            if conversion {
                                "reviewed_native_conversion_residual"
                            } else {
                                "reviewed_custody_component"
                            },
                        )
                    } else {
                        let key = if codec::truth(&a["id"]) {
                            s(&a["id"])
                        } else {
                            i.to_string()
                        };
                        pairs.push(json!({"pair_id":text(&fallback(&term["legacy_source_id"],&json!(format!("component:{id}:conversion:{key}")))),"component_id":id,"out_id":o["anchor_transaction_id"],"in_id":t["anchor_transaction_id"],"out_amount":number(&amount),"in_amount":number(&target_amount),"out_asset":o["asset"],"in_asset":t["asset"],"policy":text(&fallback(&term["tax_policy"],&fallback(&c["conversion_policy"],&json!("taxable")))),"kind":text(&fallback(&term["review_kind"],&fallback(&c["component_type"],&json!("swap")))),"swap_fee_msat":term["swap_fee_msat"],"swap_fee_kind":term["swap_fee_kind"],"notes":term["review_notes"],"confidence_at_review":term["confidence_at_review"],"review_source":term["review_source"]}));
                        ("external_confirmed", "reviewed_taxable_conversion_source")
                    }
                }
            }
            "fee" => {
                if amount != target_amount {
                    return Err(error(
                        "fee allocation must conserve its source quantity",
                        json!({"component_id":id,"allocation_id":a["id"]}),
                    ));
                }
                (
                    "external_confirmed",
                    if conversion {
                        "reviewed_conversion_fee"
                    } else {
                        "reviewed_network_fee"
                    },
                )
            }
            "suspense" => {
                suspense += &amount;
                ("custody_suspense", "reviewed_residual_suspense")
            }
            "external" => {
                if term["term_kind"] == "direct_swap_payout" {
                    payouts.push(payout(&term, sink, o, &id, &amount, &target_amount));
                    ("external_confirmed", "reviewed_direct_payout_source")
                } else {
                    (
                        "external_confirmed",
                        "reviewed_external_component_allocation",
                    )
                }
            }
            _ => {
                return Err(error(
                    "component sink role cannot emit a quantity claim",
                    json!({"component_id":id,"sink_leg_id":sink["id"],"role":role}),
                ))
            }
        };
        let destination = if role == "fee" {
            json!("fee")
        } else if role == "external"
            || conversion && matches!(role.as_str(), "destination" | "retained") && target.is_null()
        {
            json!("external")
        } else if role == "retained" && target.is_null() {
            json!("retained_custody")
        } else {
            Value::Null
        };
        let subtype =
            if role == "external" && text(&sink["notes"]).starts_with("reviewed_residual:") {
                match residual.as_str() {
                    "external_payment" => json!("payment"),
                    "external_disposal" => json!("disposal"),
                    "external_gift" => json!("gift"),
                    "external_loss" => json!("lost"),
                    _ => Value::Null,
                }
            } else {
                Value::Null
            };
        // Match the constructor boundary before returning any bundle.
        if !target.is_null() && amount != target_amount
            || destination == "retained_custody" && state != "internal_reviewed"
        {
            let mut e = family("ValueError");
            e.message = if !target.is_null() {
                "source and target quantity slices must conserve exact msat"
            } else {
                "retained_custody requires a targetless reviewed component claim"
            }
            .into();
            return Err(e);
        }
        claims.push(json!({"$type":"QuantityClaim","claim_id":format!("component:{id}:allocation:{}",if codec::truth(&a["id"]) {s(&a["id"])} else {i.to_string()}),"source":{"$type":"QuantitySlice","observation_hash":hash,"start_msat":number(&start),"end_msat":number(&end)},"target":target,"state":state,"priority":{"$enum":"ClaimPriority.REVIEWED_COMPONENT","value":10},"reason":reason,"atomic_bundle_id":format!("component:{id}"),"component_id":id,"destination_kind":destination,"external_economic_subtype":subtype,"transfer_kind":if conversion {json!(text(&fallback(&c["component_type"],&json!("swap"))))} else {transfer_kind.clone()},"transfer_policy":if conversion {json!(text(&fallback(&c["conversion_policy"],&json!("taxable"))))} else {Value::Null},"allow_cross_rail":cross}));
        super::arbitration::validate_claim(claims.last().expect("just appended")).map_err(|e| {
            let mut e = family(&e);
            e.message =
                if matches!(state, "internal_reviewed" | "internal_verified") && target.is_null() {
                    format!("{state} claims require an observed target slice")
                } else {
                    "only targetless external_confirmed claims may classify an external destination"
                        .into()
                };
            e
        })?;
    }
    if !conversion {
        let source_txs: BTreeSet<String> = legs
            .values()
            .filter(|l| l["role"] == "source")
            .map(|l| tx(l))
            .collect();
        // Only pairs keyed by a source transaction can pass; visit those in order.
        let mut source_positions: Vec<usize> = source_txs
            .iter()
            .filter_map(|t| observations.positions.get(t))
            .flatten()
            .copied()
            .collect();
        source_positions.sort_unstable();
        for &at in &source_positions {
            let (transaction, o) = &observations.entries[at];
            if o["direction"] != "outbound" || !source_txs.contains(transaction) {
                continue;
            }
            let n = sources
                .get(&text(&o["quantity_hash"]))
                .cloned()
                .unwrap_or_default();
            let cap = principal(o).map_err(|e| family(&e))?;
            if n != cap {
                return Err(error(
                    "component did not claim the complete observed source principal",
                    json!({"component_id":id,"transaction_id":transaction,"claimed_msat":number(&n),"principal_msat":number(&cap)}),
                ));
            }
        }
    }
    Ok(
        json!({"$type":"ComponentClaimCompilation","component_id":id,"claims":claims,"suspense_msat":number(&suspense),"reviewed_conversion_pairs":pairs,"reviewed_direct_payouts":payouts}),
    )
}

/// Full reference diagnostics for the private JSONL comparison harness.
pub fn outcome(component: &Value, observations: &Value) -> Value {
    match compile(component, observations) {
        Ok(value) => json!({"expected":value}),
        Err(error) if error.code == "custody_component_claim_compile" => {
            json!({"error":error.code,"message":error.message,"details":error.details})
        }
        Err(error) => json!({"error":error.code}),
    }
}

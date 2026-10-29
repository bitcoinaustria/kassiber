//! The quantity-only, term-free component compiler used by reconciliation.
//! Conversion and economic-term branches are outside this call's admission.
use super::{array, integer_or_zero};
use crate::{codec, python_text::PythonText};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::str::FromStr;

pub(super) fn number(n: &BigInt) -> Value {
    serde_json::from_str(&n.to_string()).expect("computed integer")
}
pub(super) fn amount(v: &Value) -> Result<BigInt, String> {
    BigInt::from_str(&integer_or_zero(v)?.to_string()).map_err(|_| "ValueError".into())
}
pub(super) fn anchor(leg: &Value) -> String {
    codec::text(if codec::truth(&leg["anchor_transaction_id"]) {
        &leg["anchor_transaction_id"]
    } else {
        &leg["transaction_id"]
    })
}
fn error<T>() -> Result<T, String> {
    Err("custody_component_claim_compile".into())
}
type Location = (String, String, String, String, String);
fn location(leg: &Value) -> Result<Location, String> {
    let key = codec::text(if codec::truth(&leg["wallet_id"]) {
        &leg["wallet_id"]
    } else {
        &leg["location_ref"]
    });
    if key.is_empty() {
        return error();
    }
    Ok((
        key,
        codec::text(&leg["asset"]).py_upper(),
        codec::text(&leg["exposure"]),
        if codec::truth(&leg["conservation_unit"]) {
            codec::pystr(&leg["conservation_unit"])
        } else {
            "msat".into()
        },
        codec::text(&leg["rail"]),
    ))
}
fn inferred(component: &Value) -> Result<Vec<Value>, String> {
    let mut explicit = array(&component["allocations"]).to_vec();
    if !explicit.is_empty() {
        let keys = explicit
            .iter()
            .map(|v| Ok((amount(&v["ordinal"])?, codec::text(&v["id"]))))
            .collect::<Result<Vec<_>, String>>()?;
        let mut indices: Vec<_> = (0..explicit.len()).collect();
        indices.sort_by_key(|&i| keys[i].clone());
        return Ok(indices.into_iter().map(|i| explicit[i].take()).collect());
    }
    let mut sources = Vec::new();
    let mut sinks = Vec::new();
    for leg in array(&component["legs"]) {
        if amount(&leg["amount_msat"])? > BigInt::from(0) {
            if leg["role"] == "source" {
                sources.push(leg);
            } else {
                sinks.push(leg);
            }
        }
    }
    let edges: Vec<_> = if sources.len() == 1 {
        sinks
            .iter()
            .map(|sink| (sources[0], *sink, &sink["amount_msat"]))
            .collect()
    } else if sinks.len() == 1
        && matches!(sinks[0]["role"].as_str(), Some("destination" | "retained"))
    {
        sources
            .iter()
            .map(|source| (*source, sinks[0], &source["amount_msat"]))
            .collect()
    } else {
        return error();
    };
    edges.into_iter().enumerate().map(|(i, (source, sink, n))| {
        let n = number(&amount(n)?);
        Ok(json!({"id":format!("inferred:{i}"),"ordinal":i,"source_leg_id":source["id"],"sink_leg_id":sink["id"],"source_amount_msat":n,"sink_amount_msat":n}))
    }).collect()
}
type Token = (String, BigInt, Vec<String>);
fn flatten(
    component: &Value,
    legs: &BTreeMap<String, &Value>,
    finalized: &BTreeSet<String>,
) -> Result<Vec<Value>, String> {
    let mut pending = inferred(component)?;
    let mut pools: BTreeMap<Location, VecDeque<Token>> = BTreeMap::new();
    let mut flattened = Vec::new();
    while !pending.is_empty() {
        let mut deferred = Vec::new();
        let mut progressed = false;
        for allocation in pending {
            let source_id = codec::text(&allocation["source_leg_id"]);
            let sink_id = codec::text(&allocation["sink_leg_id"]);
            let (Some(source), Some(sink)) = (legs.get(&source_id), legs.get(&sink_id)) else {
                return error();
            };
            if source["role"] != "source" {
                return error();
            }
            let a = amount(&allocation["source_amount_msat"])?;
            let b = amount(&allocation["sink_amount_msat"])?;
            if a <= BigInt::from(0) && b <= BigInt::from(0) {
                progressed = true;
                continue;
            }
            if a != b {
                return error();
            }
            let allocation_id = if codec::truth(&allocation["id"]) {
                codec::pystr(&allocation["id"])
            } else {
                flattened.len().to_string()
            };
            let mut tokens = Vec::new();
            if !anchor(source).is_empty() {
                tokens.push((source_id, a, vec![allocation_id]));
            } else {
                let key = location(source)?;
                let available: BigInt = pools
                    .get(&key)
                    .into_iter()
                    .flatten()
                    .map(|(_, n, _)| n)
                    .sum();
                if available < a {
                    deferred.push(allocation);
                    continue;
                }
                let queue = pools.entry(key).or_default();
                let mut remaining = a;
                while remaining > BigInt::from(0) {
                    let (origin, capacity, path) = queue.pop_front().expect("sufficient credits");
                    let used = remaining.clone().min(capacity.clone());
                    let mut next_path = path.clone();
                    next_path.push(allocation_id.clone());
                    tokens.push((origin.clone(), used.clone(), next_path));
                    remaining -= &used;
                    if used < capacity {
                        queue.push_front((origin, capacity - used, path));
                    }
                }
            }
            if matches!(sink["role"].as_str(), Some("destination" | "retained"))
                && anchor(sink).is_empty()
                && !finalized.contains(&sink_id)
            {
                pools.entry(location(sink)?).or_default().extend(tokens);
            } else {
                for (i, (origin, n, path)) in tokens.into_iter().enumerate() {
                    let mut cell = allocation.clone();
                    if path.len() > 1 {
                        cell["id"] = json!(format!("route:{}:{i}", path.join(":")));
                    }
                    cell["source_leg_id"] = json!(origin);
                    cell["source_amount_msat"] = number(&n);
                    cell["sink_amount_msat"] = number(&n);
                    flattened.push(cell);
                }
            }
            progressed = true;
        }
        if !progressed {
            return error();
        }
        pending = deferred;
    }
    if pools
        .values()
        .any(|q| q.iter().map(|(_, n, _)| n).sum::<BigInt>() > BigInt::from(0))
    {
        return error();
    }
    Ok(flattened)
}

pub(super) fn compile(
    component: &Value,
    observations: &BTreeMap<String, Value>,
) -> Result<Vec<Value>, String> {
    let id = codec::text(&component["id"]);
    if id.is_empty() || component["effective_state"] != "active" {
        return error();
    }
    let legs: BTreeMap<_, _> = array(&component["legs"])
        .iter()
        .map(|leg| (codec::pystr(&leg["id"]), leg))
        .collect();
    let finalized: BTreeSet<_> = legs
        .iter()
        .filter(|(_, leg)| {
            leg["role"] == "retained"
                && anchor(leg).is_empty()
                && leg["notes"] == "reviewed_residual:retained_custody"
        })
        .map(|(id, _)| id.clone())
        .collect();
    if array(&component["allocations"]).is_empty() {
        for leg in legs.values() {
            if leg["role"] == "suspense" && amount(&leg["amount_msat"])? > BigInt::from(0) {
                return error();
            }
        }
    }
    let allocations = flatten(component, &legs, &finalized)?;
    let mut source_cursors: BTreeMap<String, BigInt> = BTreeMap::new();
    let mut target_cursors: BTreeMap<String, BigInt> = BTreeMap::new();
    let mut claims = Vec::new();
    for (index, allocation) in allocations.iter().enumerate() {
        let source = legs[&codec::pystr(&allocation["source_leg_id"])];
        let sink = legs[&codec::pystr(&allocation["sink_leg_id"])];
        let a = amount(&allocation["source_amount_msat"])?;
        let b = amount(&allocation["sink_amount_msat"])?;
        // flatten has already removed nonpositive cells and proved equality.
        let source_id = anchor(source);
        if source_id.is_empty() {
            return error();
        }
        let Some(observation) = observations.get(&source_id) else {
            return error();
        };
        if observation["direction"] != "outbound" {
            return error();
        }
        if sink["role"] == "fee" && sink["notes"] != "reviewed_residual:network_fee" {
            if a != amount(&observation["fee_msat"])? {
                return error();
            }
            continue;
        }
        let source_hash = codec::text(&observation["quantity_hash"]);
        let start = source_cursors
            .entry(source_hash.clone())
            .or_default()
            .clone();
        let end = &start + &a;
        if end > super::reconciliation::principal(observation)? {
            return error();
        }
        source_cursors.insert(source_hash.clone(), end.clone());
        let mut target = Value::Null;
        let (state, reason) = match sink["role"].as_str() {
            Some("destination" | "retained") => {
                let target_id = anchor(sink);
                if target_id.is_empty() {
                    if !finalized.contains(&codec::text(&sink["id"])) {
                        return error();
                    }
                    ("internal_reviewed", "reviewed_retained_custody")
                } else {
                    let Some(destination) = observations.get(&target_id) else {
                        return error();
                    };
                    if destination["direction"] != "inbound" {
                        return error();
                    }
                    let hash = codec::text(&destination["quantity_hash"]);
                    let start = target_cursors.entry(hash.clone()).or_default().clone();
                    let end = &start + &b;
                    if end > super::reconciliation::principal(destination)? {
                        return error();
                    }
                    target_cursors.insert(hash.clone(), end.clone());
                    target = json!({"observation_hash":hash,"start_msat":number(&start),"end_msat":number(&end)});
                    ("internal_reviewed", "reviewed_custody_component")
                }
            }
            Some("fee") => ("external_confirmed", "reviewed_network_fee"),
            Some("suspense") => ("custody_suspense", "reviewed_residual_suspense"),
            Some("external") => (
                "external_confirmed",
                "reviewed_external_component_allocation",
            ),
            _ => return error(),
        };
        let key = if codec::truth(&allocation["id"]) {
            codec::pystr(&allocation["id"])
        } else {
            index.to_string()
        };
        // Only the fields consumed by reconciliation cross this private seam.
        claims.push(json!({"claim_id":format!("component:{id}:allocation:{key}"),"source":{"observation_hash":source_hash,"start_msat":number(&start),"end_msat":number(&end)},"target":target,"state":state,"reason":reason,"destination_kind":if sink["role"]=="fee" {json!("fee")} else {Value::Null}}));
    }
    for (transaction, observation) in observations {
        if observation["direction"] != "outbound"
            || !legs
                .values()
                .any(|leg| anchor(leg) == *transaction && leg["role"] == "source")
        {
            continue;
        }
        let claimed = source_cursors
            .get(&codec::text(&observation["quantity_hash"]))
            .cloned()
            .unwrap_or_default();
        if claimed != super::reconciliation::principal(observation)? {
            return error();
        }
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconciliation_reviewed_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_reconciliation_helpers.json"
        ))
        .unwrap();
        for v in vectors.iter().filter(|v| v["op"] == "reviewed") {
            let value = &v["value"];
            let result = compile(
                &value["component"],
                &serde_json::from_value(value["observations"].clone()).unwrap(),
            );
            if let Some(error) = v.get("error") {
                assert_eq!(
                    result.err(),
                    error.as_str().map(str::to_owned),
                    "{}",
                    v["case"]
                );
            } else {
                assert_eq!(json!(result.unwrap()), v["expected"], "{}", v["case"]);
            }
        }
    }
}

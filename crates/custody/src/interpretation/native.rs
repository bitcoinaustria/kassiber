//! Native HTLC production: match full populations before applying reservations.
use super::{
    htlc,
    matcher::{self, amount, number},
    pairs, recorded, Context,
};
use crate::{codec, components::time, evidence, snapshot::Snapshot};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

type Ids = BTreeSet<String>;
pub(super) struct Production {
    pub pairs: Vec<Value>,
    pub proofs: BTreeMap<(String, String), pairs::NativeTransitionProof>,
    pub quarantines: Vec<Value>,
    pub candidates: Vec<Value>,
    pub proof_values: Vec<Value>,
}
fn endpoint(row: &Value, observation: &Value) -> Result<bool, String> {
    Ok(recorded::lightning_row(row)
        || codec::truth(&observation["authoritative_chain_observation"])
            && evidence::current_observation(row).map_err(|e| e.code.to_owned())?)
}
fn instant(v: &Value) -> Result<Option<i64>, String> {
    time::instant(v).map_err(|e| e.code.to_owned())
}
fn dismissed(snapshot: &Snapshot) -> Result<BTreeSet<(String, String)>, String> {
    // Python timestamp() rounds to an f64 number of Unix seconds.
    let seconds = |v: &Value| -> Result<Option<f64>, String> {
        Ok(instant(v)?.map(|v| (v - 62_135_596_800_000_000) as f64 / 1_000_000.0))
    };
    let now = seconds(&json!(snapshot.as_of))?;
    let mut result = BTreeSet::new();
    for r in &snapshot.pair_dismissals {
        if codec::truth(&r["expires_at"]) {
            if let Some(expiry) = seconds(&r["expires_at"])? {
                let Some(now) = now else {
                    return Err("TypeError".into());
                };
                if expiry <= now {
                    continue;
                }
            }
        }
        if codec::truth(&r["out_transaction_id"]) && codec::truth(&r["in_transaction_id"]) {
            result.insert((
                codec::pystr(&r["out_transaction_id"]),
                codec::pystr(&r["in_transaction_id"]),
            ));
        }
    }
    Ok(result)
}
fn ambiguous(
    rows: &[Value],
    context: &Context,
    candidates: &[Value],
    occupied: &Ids,
    dismissed: &BTreeSet<(String, String)>,
) -> Result<Vec<Value>, String> {
    let exact: BTreeSet<_> = candidates
        .iter()
        .filter(|c| {
            c["method"] == "payment_hash" && c["confidence"] == "exact" && c["conflict_size"] == 1
        })
        .map(|c| (codec::text(&c["out_id"]), codec::text(&c["in_id"])))
        .collect();
    let mut native = BTreeMap::new();
    let mut roles = BTreeMap::new();
    for row in rows {
        let id = codec::pystr(&row["id"]);
        if let Some(o) = context.observations.get(&id) {
            if matcher::asset(&o["asset"]).is_some() && endpoint(row, o)? {
                native.insert(id.clone(), row);
                roles.insert(id, matcher::role(row)?);
            }
        }
    }
    for row in rows {
        row.get("direction").ok_or_else(|| "KeyError".to_owned())?;
    }
    let outs: Vec<_> = rows
        .iter()
        .filter(|r| r["direction"] == "outbound")
        .collect();
    let ins: Vec<_> = rows
        .iter()
        .filter(|r| r["direction"] == "inbound")
        .collect();
    let mut held = Ids::new();
    for (outs, ins) in matcher::populations(&outs, &ins) {
        for (out_role, in_role) in [
            ("lightning_node", "chain_htlc"),
            ("chain_htlc", "lightning_node"),
        ] {
            let eligible = |rows: &[&Value], role: &str| -> Result<Vec<(BigInt, Value)>, String> {
                let mut result = Vec::new();
                for row in rows {
                    let id = codec::pystr(&row["id"]);
                    if roles.get(&id) == Some(&role) && !occupied.contains(&id) {
                        let n = amount(row)?;
                        if n > BigInt::from(0) {
                            result.push((n, (*row).clone()));
                        }
                    }
                }
                result.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(result)
            };
            let sources = eligible(&outs, out_role)?;
            let targets = eligible(&ins, in_role)?;
            if sources.is_empty() || targets.is_empty() {
                continue;
            }
            let minimum = sources
                .iter()
                .map(|(n, _)| matcher::fee_threshold(n).map(|t| n - t))
                .collect::<Result<Vec<_>, _>>()?;
            // Conservation window is monotonic; never form the duplicate N:M graph.
            let available = |out: &Value, incoming: &Value| -> Result<bool, String> {
                let key = (codec::pystr(&out["id"]), codec::pystr(&incoming["id"]));
                Ok(!exact.contains(&key)
                    && !dismissed.contains(&key)
                    && out["wallet_id"] != incoming["wallet_id"]
                    && matcher::conserves(out, incoming)?)
            };
            for (i, (n, out)) in sources.iter().enumerate() {
                let start = targets.partition_point(|(v, _)| v < &minimum[i]);
                let end = targets.partition_point(|(v, _)| v <= n);
                for (_, incoming) in &targets[start..end] {
                    if available(out, incoming)? {
                        held.insert(codec::pystr(&out["id"]));
                        break;
                    }
                }
            }
            for (n, incoming) in &targets {
                let start = sources.partition_point(|(v, _)| v < n);
                let end = minimum.partition_point(|v| v <= n);
                for (_, out) in &sources[start..end] {
                    if available(out, incoming)? {
                        held.insert(codec::pystr(&incoming["id"]));
                        break;
                    }
                }
            }
        }
    }
    held.into_iter().map(|id|{let row=native[&id];Ok(json!({"transaction_id":id,"workspace_id":row["workspace_id"],"profile_id":row["profile_id"],"reason":"native_transition_ambiguous","detail_json":pairs::detail_json(&json!({"required_for":"unique_scoped_native_transition"}))?}))}).collect()
}
fn policy(snapshot: &Snapshot, out: &Value, incoming: &Value) -> Result<&'static str, String> {
    use crate::python_text::PythonText;
    if codec::text(out).py_strip().py_upper() == codec::text(incoming).py_strip().py_upper() {
        return Ok("carrying-value");
    }
    let country = codec::text(&snapshot.profile["tax_country"])
        .py_strip()
        .py_lower();
    if !matches!(country.as_str(), "" | "generic" | "at") {
        return Err("ValueError".into());
    }
    Ok(
        if country == "at"
            || BTreeSet::from([
                codec::text(out).py_strip().py_upper(),
                codec::text(incoming).py_strip().py_upper(),
            ]) == BTreeSet::from(["BTC".into(), "LBTC".into()])
                && snapshot
                    .profile
                    .get("bitcoin_rail_carrying_value")
                    .is_none_or(codec::truth)
        {
            "carrying-value"
        } else {
            "taxable"
        },
    )
}
pub(super) fn compute(
    snapshot: &Snapshot,
    context: &Context,
    occupied: &Ids,
) -> Result<Production, String> {
    // Snapshot records hold the rows' fields; reading them leaves the JSON
    // rows unbuilt for books without HTLC evidence.
    if !htlc::presence(&snapshot.transactions) {
        return Ok(Production {
            pairs: Vec::new(),
            proofs: BTreeMap::new(),
            quarantines: Vec::new(),
            candidates: Vec::new(),
            proof_values: Vec::new(),
        });
    }
    let rows = snapshot.rows();
    // suggest_swap_candidates parses its explicit clock before row admission,
    // even though its own dismissal sequence is empty in this producer.
    instant(&json!(snapshot.as_of))?;
    let candidates = matcher::suggest(rows)?;
    let dismissed = dismissed(snapshot)?;
    for row in rows {
        row.get("id").ok_or_else(|| "KeyError".to_owned())?;
    }
    let by_id: BTreeMap<_, _> = rows.iter().map(|r| (codec::pystr(&r["id"]), r)).collect();
    let mut quarantines = ambiguous(rows, context, &candidates, occupied, &dismissed)?;
    let mut pairs = Vec::new();
    let mut proofs = BTreeMap::new();
    let mut proof_values = Vec::new();
    for candidate in &candidates {
        let key = (
            codec::text(&candidate["out_id"]),
            codec::text(&candidate["in_id"]),
        );
        if candidate["confidence"] != "exact"
            || candidate["conflict_size"] != 1
            || !matches!(
                candidate["method"].as_str(),
                Some("payment_hash" | "htlc_refund")
            )
            || dismissed.contains(&key)
            || occupied.contains(&key.0)
            || occupied.contains(&key.1)
        {
            continue;
        }
        let (out, incoming) = (by_id[&key.0], by_id[&key.1]);
        if matcher::asset(&out["asset"]).is_none() || matcher::asset(&incoming["asset"]).is_none() {
            continue;
        }
        let (Some(source), Some(target)) = (
            context.observations.get(&key.0),
            context.observations.get(&key.1),
        ) else {
            continue;
        };
        if !endpoint(out, source)? || !endpoint(incoming, target)? {
            continue;
        }
        let source_when = instant(&source["occurred_at"])?;
        let target_when = instant(&target["occurred_at"])?;
        let a = BigInt::from(super::claims::principal(source));
        let b = BigInt::from(super::claims::principal(target));
        let shortfall = &a - &b;
        let unresolved =
            if a <= BigInt::from(0) || b <= BigInt::from(0) || shortfall < BigInt::from(0) {
                Some((
                    "native_transition_amount_mismatch",
                    "conserving_native_transition_quantities",
                ))
            } else if shortfall > BigInt::from(0)
                && source_when.zip(target_when).is_none_or(|(a, b)| {
                    a.div_euclid(86_400_000_000) != b.div_euclid(86_400_000_000)
                })
            {
                Some((
                    "native_transition_fee_timing_unresolved",
                    "explicit_transition_fee_timeline",
                ))
            } else {
                None
            };
        if let Some((reason, required)) = unresolved {
            let mut detail = json!({"out_transaction_id":key.0,"in_transaction_id":key.1,"source_occurred_at":source["occurred_at"],"target_occurred_at":target["occurred_at"],"source_principal_msat":number(&a),"target_principal_msat":number(&b),"required_for":required});
            if shortfall > BigInt::from(0) {
                detail["unallocated_fee_msat"] = number(&shortfall);
            }
            for row in [out, incoming] {
                quarantines.push(json!({"transaction_id":row["id"],"workspace_id":row["workspace_id"],"profile_id":row["profile_id"],"reason":reason,"detail_json":pairs::detail_json(&detail)?}));
            }
            continue;
        }
        let policy = policy(snapshot, &out["asset"], &incoming["asset"])?;
        let preimage = codec::dumps(
            &codec::from_json(&json!([candidate["method"], key.0, key.1]))?,
            true,
        )?;
        pairs.push(json!({"out":out,"in":incoming,"source":"native_htlc_transition","pair_id":format!("native-htlc-{}",codec::sha256(&preimage)),"kind":candidate["default_kind"],"policy":policy,"out_amount_msat":number(&a)}));
        let value = json!({"candidate":candidate,"source_quantity_hash":source["quantity_hash"],"target_quantity_hash":target["quantity_hash"],"policy":policy});
        proofs.insert(
            key.clone(),
            serde_json::from_value(value.clone()).expect("internal proof"),
        );
        proof_values.push(json!({"key":[key.0,key.1],"proof":value}));
    }
    Ok(Production {
        pairs,
        proofs,
        quarantines,
        candidates,
        proof_values,
    })
}
pub(super) fn compile(snapshot: &Snapshot, context: &Context) -> Value {
    (||->Result<Value,String>{let p=compute(snapshot,context,&Ids::new())?;Ok(json!({"has_evidence":htlc::presence(&snapshot.transactions.iter().map(|r|json!(r)).collect::<Vec<_>>()),"pairs":p.pairs,"proofs":p.proof_values,"quarantines":p.quarantines,"candidates":p.candidates}))})().unwrap_or_else(|error|json!({"error":error}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_production_helper_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_native_helpers.json"
        ))
        .unwrap();
        for v in vectors.iter().filter(|v| v["op"] == "production") {
            let input = &v["value"];
            let snapshot: Snapshot = serde_json::from_value(input["snapshot"].clone()).unwrap();
            let mut context = Context::load(&snapshot).unwrap();
            for (id, fields) in input["overrides"].as_object().unwrap() {
                if fields.is_null() {
                    context.observations.remove(id);
                } else {
                    context
                        .observations
                        .get_mut(id)
                        .unwrap()
                        .to_mut()
                        .as_object_mut()
                        .unwrap()
                        .extend(fields.as_object().unwrap().clone());
                }
            }
            let occupied = super::super::array(&input["occupied"])
                .iter()
                .map(codec::text)
                .collect();
            let actual=compute(&snapshot,&context,&occupied).map(|p|json!({"pairs":p.pairs,"proofs":p.proof_values,"quarantines":p.quarantines,"candidates":p.candidates}));
            if v.get("error").is_some() {
                assert_eq!(
                    actual.err().map(|e| json!(e)),
                    Some(v["error"].clone()),
                    "{}",
                    v["case"]
                );
            } else {
                assert_eq!(actual.unwrap(), v["expected"], "{}", v["case"]);
            }
        }
    }
}

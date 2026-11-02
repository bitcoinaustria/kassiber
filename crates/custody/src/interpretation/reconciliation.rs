//! Reproduce every reviewed allocation through complete authoritative native
//! routes. Certificates select a projection; they never change authorship.
use super::{
    array, chronology, fifo,
    reviewed::{self, amount, number},
};
use crate::codec;
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

type Ids = BTreeSet<String>;
type Expected = BTreeMap<(String, String), BigInt>;
type Pool = (String, String, String, String);
type Credits = Vec<(String, BigInt)>;

pub(super) fn principal(observation: &Value) -> Result<BigInt, String> {
    let mut result = amount(&observation["amount_msat"])?;
    if observation["direction"] == "outbound" && codec::truth(&observation["amount_includes_fee"]) {
        result -= amount(&observation["fee_msat"])?;
    }
    Ok(result)
}
fn pool(observation: &Value) -> Pool {
    (
        codec::text(&observation["wallet_id"]),
        codec::text(&observation["asset"]),
        codec::text(&observation["event_key"]["chain"]),
        codec::text(&observation["event_key"]["network"]),
    )
}
fn hash(slice: &Value) -> String {
    codec::text(&slice["observation_hash"])
}
fn claim_amount(claim: &Value) -> Result<BigInt, String> {
    Ok(amount(&claim["source"]["end_msat"])? - amount(&claim["source"]["start_msat"])?)
}
fn complete(claims: &[&Value], total: &BigInt, target: bool) -> Result<bool, String> {
    let side = if target { "target" } else { "source" };
    let mut slices = claims
        .iter()
        .map(|claim| {
            Ok((
                amount(&claim[side]["start_msat"])?,
                amount(&claim[side]["end_msat"])?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    slices.sort_by(|a, b| a.0.cmp(&b.0));
    let mut cursor = BigInt::from(0);
    for (start, end) in slices {
        if start != cursor {
            return Ok(false);
        }
        cursor = end;
    }
    Ok(cursor == *total)
}
fn credit(credits: &mut Credits, source: &str, amount: BigInt) {
    if let Some((_, balance)) = credits.iter_mut().find(|(key, _)| key == source) {
        *balance += amount;
    } else {
        credits.push((source.to_owned(), amount));
    }
}
pub(super) fn digest(expected: &Expected) -> Result<String, String> {
    let value = json!(expected
        .iter()
        .map(|((source, target), n)| json!([source, target, number(n)]))
        .collect::<Vec<_>>());
    Ok(codec::sha256(&codec::dumps(
        &codec::from_json(&value)?,
        true,
    )?))
}

pub(super) fn reconcile(
    components: &[Value],
    observations: &BTreeMap<String, std::borrow::Cow<'_, Value>>,
    native_claims: &[Value],
    blocked: &Ids,
    rows: &BTreeMap<String, Value>,
) -> Result<Vec<Value>, String> {
    let by_hash: BTreeMap<_, _> = observations
        .values()
        .map(|v| (codec::text(&v["quantity_hash"]), v))
        .collect();
    let protected: Ids = components
        .iter()
        .flat_map(|c| array(&c["legs"]))
        .map(reviewed::anchor)
        .filter(|v| !v.is_empty())
        .collect();
    let mut outgoing: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    let mut incoming: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    let mut claim_amounts = BTreeMap::new();
    for claim in native_claims {
        if claim["target"].is_null()
            || claim["priority"]["value"] != 20
            || claim["state"] != "internal_verified"
        {
            continue;
        }
        let source = by_hash[&hash(&claim["source"])];
        let target = by_hash[&hash(&claim["target"])];
        if !codec::truth(&source["authoritative_chain_observation"])
            || !codec::truth(&target["authoritative_chain_observation"])
            || source["event_key"] != target["event_key"]
            || source["event_key"]["native_namespace"] != "chain"
            || !matches!(
                source["event_key"]["chain"].as_str(),
                Some("bitcoin" | "liquid")
            )
            || source["asset"] != target["asset"]
            || source["wallet_id"] == target["wallet_id"]
            || [source, target]
                .iter()
                .any(|v| blocked.contains(&codec::text(&v["transaction_id"])))
        {
            continue;
        }
        outgoing
            .entry(hash(&claim["source"]))
            .or_default()
            .push(claim);
        incoming
            .entry(hash(&claim["target"]))
            .or_default()
            .push(claim);
        claim_amounts.insert(codec::text(&claim["claim_id"]), claim_amount(claim)?);
    }
    let mut complete_sources = Ids::new();
    for (key, claims) in &outgoing {
        let source = by_hash[key];
        if !complete(claims, &principal(source)?, false)?
            || source["fee_attribution"] != "exact"
            || amount(&source["fee_msat"])? < BigInt::from(0)
        {
            continue;
        }
        let mut valid = true;
        for claim in claims {
            let target = hash(&claim["target"]);
            if !complete(&incoming[&target], &principal(by_hash[&target])?, true)? {
                valid = false;
                break;
            }
        }
        if valid {
            complete_sources.insert(key.clone());
        }
    }
    if complete_sources.is_empty() {
        return Ok(Vec::new());
    }
    let mut stable: Vec<_> = complete_sources.iter().cloned().collect();
    stable.sort_by_key(|key| {
        (
            codec::text(&by_hash[key]["occurred_at"]),
            codec::text(&by_hash[key]["event_key"]["native_event_id"]),
            key.clone(),
        )
    });
    let mut ordered = Vec::new();
    let mut index = 0;
    while index < stable.len() {
        let mut end = index + 1;
        while end < stable.len()
            && by_hash[&stable[end]]["occurred_at"] == by_hash[&stable[index]]["occurred_at"]
        {
            end += 1;
        }
        if end - index == 1 {
            ordered.push(stable[index].clone());
        } else {
            let mut transfers = Vec::new();
            // Dict update preserves the original position if claim ids repeat.
            let mut ids = Vec::new();
            for key in &stable[index..end] {
                let source = by_hash[key];
                let proof = chronology::evidence(
                    rows.get(&codec::text(&source["anchor_transaction_id"]))
                        .unwrap_or(&json!({})),
                )?;
                for claim in &outgoing[key] {
                    let target = by_hash[&hash(&claim["target"])];
                    let transfer = chronology::Transfer {
                        source_hash: key.clone(),
                        from: codec::text(&source["wallet_id"]),
                        to: codec::text(&target["wallet_id"]),
                        proof: proof.clone(),
                    };
                    let id = codec::text(&claim["claim_id"]);
                    if let Some(i) = ids.iter().position(|old| old == &id) {
                        transfers[i] = transfer;
                    } else {
                        ids.push(id);
                        transfers.push(transfer);
                    }
                }
            }
            ordered.extend(chronology::source_order(&transfers));
        }
        index = end;
    }
    let mut activity: BTreeMap<Pool, Vec<&Value>> = BTreeMap::new();
    for observation in by_hash.values() {
        if principal(observation)? != BigInt::from(0)
            || amount(&observation["fee_msat"])? != BigInt::from(0)
        {
            activity
                .entry(pool(observation))
                .or_default()
                .push(observation);
        }
    }
    let mut selected = Vec::new();
    let mut selected_rows = Ids::new();
    'component: for component in components {
        if component["effective_state"] != "active"
            || component["conservation_mode"] != "quantity"
            || codec::truth(&component["economic_terms"])
        {
            continue;
        }
        let anchors: Ids = array(&component["legs"])
            .iter()
            .map(reviewed::anchor)
            .filter(|v| !v.is_empty())
            .collect();
        if !anchors.is_disjoint(blocked) || !anchors.is_disjoint(&selected_rows) {
            continue;
        }
        let scoped = observations
            .iter()
            .filter(|(id, _)| anchors.contains(*id))
            .map(|(k, v)| (k.clone(), Value::clone(v)))
            .collect();
        let reviewed = match reviewed::compile(component, &scoped) {
            Ok(claims) => claims,
            Err(code)
                if matches!(
                    code.as_str(),
                    "custody_component_claim_compile"
                        | "TypeError"
                        | "ValueError"
                        | "UnicodeEncodeError"
                ) =>
            {
                continue
            }
            Err(code) => return Err(code),
        };
        let mut expected = Expected::new();
        let mut source_hashes = Ids::new();
        let mut target_hashes = Ids::new();
        for claim in reviewed {
            let source = hash(&claim["source"]);
            source_hashes.insert(source.clone());
            let target = if !claim["target"].is_null() {
                let target = hash(&claim["target"]);
                target_hashes.insert(target.clone());
                target
            } else if (claim["destination_kind"] == "fee"
                && claim["reason"] == "reviewed_network_fee")
                || (claim["state"] == "custody_suspense"
                    && claim["reason"] == "reviewed_residual_suspense")
            {
                "fee".to_owned()
            } else {
                continue 'component;
            };
            *expected.entry((source, target)).or_default() += claim_amount(&claim)?;
        }
        if source_hashes.is_empty()
            || target_hashes.is_empty()
            || !source_hashes.is_subset(&complete_sources)
        {
            continue;
        }
        for source in &source_hashes {
            let fee = amount(&by_hash[source]["fee_msat"])?;
            if fee != BigInt::from(0) {
                *expected.entry((source.clone(), "fee".into())).or_default() += fee;
            }
        }
        let start = source_hashes
            .iter()
            .map(|key| codec::text(&by_hash[key]["occurred_at"]))
            .min()
            .unwrap();
        let end = target_hashes
            .iter()
            .map(|key| codec::text(&by_hash[key]["occurred_at"]))
            .max()
            .unwrap();
        let mut pools: BTreeMap<Pool, Credits> = BTreeMap::new();
        let mut actual = Expected::new();
        let mut used = Ids::new();
        let mut used_claims = Ids::new();
        let mut seeded = Ids::new();
        let mut intermediate = BTreeSet::new();
        // The reference bisects the same-time ordered list (not a time-sorted
        // replacement); source groups keep their raw occurred_at ordering.
        for key in ordered.iter().filter(|key| {
            let at = codec::text(&by_hash[*key]["occurred_at"]);
            at >= start && at <= end
        }) {
            let source = by_hash[key];
            let source_id = codec::text(&source["transaction_id"]);
            let pool_key = pool(source);
            let credits = pools.entry(pool_key.clone()).or_default();
            if source_hashes.contains(key) {
                let debit: BigInt = expected
                    .iter()
                    .filter(|((s, _), _)| s == key)
                    .map(|(_, n)| n)
                    .sum();
                if debit != principal(source)? + amount(&source["fee_msat"])? {
                    continue 'component;
                }
                credit(credits, key, debit);
                seeded.insert(key.clone());
            } else if credits.is_empty() {
                continue;
            } else if protected.contains(&source_id) && !anchors.contains(&source_id) {
                continue 'component;
            }
            let mut claims = outgoing[key].clone();
            claims.sort_by_key(|claim| {
                amount(&claim["source"]["start_msat"]).expect("canonical slice")
            });
            let mut sinks = claims
                .iter()
                .enumerate()
                .map(|(i, claim)| Ok((i.to_string(), claim_amount(claim)?)))
                .collect::<Result<Vec<_>, String>>()?;
            let fee = amount(&source["fee_msat"])?;
            if fee != BigInt::from(0) {
                sinks.push(("fee".into(), fee.clone()));
            }
            if credits.iter().map(|(_, n)| n).sum::<BigInt>() < principal(source)? + &fee {
                continue 'component;
            }
            let allocation = fifo::allocate(credits, &sinks)?;
            pools.insert(
                pool_key,
                allocation
                    .remaining
                    .into_iter()
                    .filter(|(_, n)| *n != BigInt::from(0))
                    .collect(),
            );
            used.insert(source_id);
            used_claims.extend(claims.iter().map(|claim| codec::text(&claim["claim_id"])));
            for cell in allocation.cells {
                if cell.sink == "fee" {
                    *actual.entry((cell.source, "fee".into())).or_default() += cell.amount;
                    continue;
                }
                let target = by_hash[&hash(
                    &claims[cell.sink.parse::<usize>().expect("enumerated sink")]["target"],
                )];
                let target_id = codec::text(&target["transaction_id"]);
                used.insert(target_id.clone());
                if selected_rows.contains(&target_id)
                    || (protected.contains(&target_id) && !anchors.contains(&target_id))
                {
                    continue 'component;
                }
                let target_hash = codec::text(&target["quantity_hash"]);
                if target_hashes.contains(&target_hash) {
                    *actual.entry((cell.source, target_hash)).or_default() += cell.amount;
                } else {
                    let key = pool(target);
                    intermediate.insert(key.clone());
                    credit(pools.entry(key).or_default(), &cell.source, cell.amount);
                }
            }
        }
        if seeded != source_hashes
            || actual != expected
            || pools.values().any(|v| !v.is_empty())
            || used.is_subset(&anchors)
        {
            continue;
        }
        for key in intermediate {
            if activity[&key].iter().any(|observation| {
                let at = codec::text(&observation["occurred_at"]);
                at >= start
                    && at <= end
                    && !used.contains(&codec::text(&observation["transaction_id"]))
            }) {
                continue 'component;
            }
        }
        let allocations: Vec<_> = expected.iter().map(|((source,target),n)| json!({"$type":"ReconciledAllocation","source_transaction_id":by_hash[source]["anchor_transaction_id"],"target_transaction_id":if target=="fee" {Value::Null} else {by_hash[target]["anchor_transaction_id"].clone()},"asset":by_hash[source]["asset"],"amount_msat":number(n),"destination_kind":if target=="fee" {"fee"} else {"wallet"}})).collect();
        let amounts: Vec<_> = used_claims
            .iter()
            .map(|id| json!([id, number(&claim_amounts[id])]))
            .collect();
        selected.push(json!({"$type":"NativeComponentReconciliation","component_id":codec::pystr(&component["id"]),"component_revision":super::integer(&component["revision"] )?,"native_claim_ids":used_claims,"native_claim_amounts":amounts,"transaction_ids":used,"reviewed_allocation_digest":digest(&expected)?,"allocations":allocations}));
        selected_rows.extend(used);
    }
    selected.sort_by_key(|v| codec::text(&v["component_id"]));
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconciliation_helper_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_reconciliation_helpers.json"
        ))
        .unwrap();
        for v in vectors.iter().filter(|v| v["op"] == "reconcile") {
            let value = &v["value"];
            let result = reconcile(
                array(&value["components"]),
                &serde_json::from_value(value["observations"].clone()).unwrap(),
                array(&value["native_claims"]),
                &array(&value["blocked_transaction_ids"])
                    .iter()
                    .map(codec::text)
                    .collect(),
                &serde_json::from_value(value["rows_by_transaction"].clone()).unwrap(),
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
    #[test]
    fn reconciliation_compilation_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_reconciliation_cases.json"
        ))
        .unwrap();
        for v in vectors {
            let snapshot: crate::snapshot::Snapshot =
                serde_json::from_value(v["input"].clone()).unwrap();
            let components = crate::components::compute(&snapshot).unwrap();
            let evidence = crate::evidence::stage(&snapshot).unwrap();
            assert_eq!(
                super::super::compute(&snapshot, &components, &evidence).unwrap(),
                v["expected"],
                "{}",
                v["case"]
            );
        }
    }
}

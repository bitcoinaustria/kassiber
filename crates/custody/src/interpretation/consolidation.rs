//! Exact multi-source allocation, including collaborative own-delta proof.
use super::{
    fanout::amount,
    ownership::{
        abs, clone_leg, component_inputs, field, output_owners, owners, relation, scope, synthetic,
        value, zero, Derivation, Relation,
    },
    pairs, Context,
};
use crate::fields::{Fields, Row};
use crate::{
    codec::{self, truth},
    evidence,
    graph::{self, ownership::Index, stored as py},
    python_text::PythonText,
    snapshot::Snapshot,
};
use kassiber_pyjson::{BigInt, Value as Py};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn current<R: Row + ?Sized>(row: &R) -> Result<bool, String> {
    if super::integer_or_zero(row.at("observation_authority_version")).ok() != Some(json!(1)) {
        return Ok(false);
    }
    Ok(
        codec::text(row.at("observation_graph_hash"))
            == evidence::graph_hash(row.at("raw_json"))?.0
            && codec::text(row.at("observation_quantity_hash"))
                == evidence::observed_hash(&row.json())?.0,
    )
}

fn collaborative<R: Row>(
    group: &[&R],
    graph: &Py,
    index: Index<'_>,
    physical: &(String, String),
) -> Result<Option<BTreeMap<String, BigInt>>, String> {
    let mut relevant = Vec::new();
    for row in group {
        if amount(row, "amount")? > zero() {
            relevant.push(*row);
        }
    }
    if relevant.is_empty() {
        return Ok(None);
    }
    for row in &relevant {
        if amount(row, "fee")? != zero() || !current(row)? {
            return Ok(None);
        }
    }
    let by_wallet: BTreeMap<_, _> = relevant
        .iter()
        .map(|row| (codec::pystr(row.at("wallet_id")), *row))
        .collect();
    if by_wallet.len() != relevant.len() {
        return Ok(None);
    }
    let mut deltas: BTreeMap<String, BigInt> = BTreeMap::new();
    for (side, sign) in [("inputs", 1i64), ("outputs", -1i64)] {
        for leg in py::list(py::get(graph, side)) {
            let owner_ids = if side == "inputs" {
                owners(index, leg, physical)?
            } else {
                output_owners(index, leg, physical)?
            };
            if owner_ids.len() > 1 {
                return Ok(None);
            }
            if let Some(owner) = owner_ids.into_iter().next() {
                let Some(value) = value(leg) else {
                    return Ok(None);
                };
                *deltas.entry(owner).or_insert_with(zero) += value * sign;
            }
        }
    }
    deltas.retain(|_, delta| *delta != zero());
    if deltas.keys().ne(by_wallet.keys()) || deltas.values().sum::<BigInt>() != zero() {
        return Ok(None);
    }
    for (wallet, delta) in &deltas {
        let row = by_wallet[wallet];
        let direction = if *delta > zero() {
            "outbound"
        } else {
            "inbound"
        };
        if row.at("direction") != direction || amount(row, "amount")? != abs(delta.clone()) * 1000 {
            return Ok(None);
        }
    }
    Ok(Some(
        deltas
            .into_iter()
            .filter_map(|(wallet, delta)| (delta < zero()).then(|| (wallet, -delta)))
            .collect(),
    ))
}

pub(super) fn derive<R: Row>(
    rows: &[R],
    snapshot: &Snapshot,
    handled: &BTreeSet<String>,
) -> Result<Derivation, String> {
    let mut result = Derivation::default();
    let index = Index(snapshot.owned_index.as_ref());
    if index.0.is_none() {
        return Ok(result);
    }
    let parsed = graph::ownership::profile_parsed_typed(rows, index)?;
    let mut groups: Vec<(_, Vec<&R>)> = Vec::new();
    let mut positions = BTreeMap::new();
    for row in rows {
        let graph = parsed.get(&codec::pystr(row.at("id")));
        let Some(scope) = scope(row, graph, index)? else {
            continue;
        };
        if synthetic(row) {
            continue;
        }
        let position = *positions.entry(scope.clone()).or_insert(groups.len());
        if position == groups.len() {
            groups.push((scope, vec![row]));
        } else {
            groups[position].1.push(row);
        }
    }
    let wallets: BTreeMap<_, _> = snapshot
        .wallets
        .iter()
        .map(|w| (codec::pystr(&json!(w)["id"]), json!(w)))
        .collect();
    for ((chain, network, txid_key, asset), group) in groups {
        let physical = (chain.clone(), network);
        let mut senders = Vec::new();
        for row in &group {
            if row.at("direction") == "outbound"
                && (amount(row, "amount")? > zero()
                    || (truth(row.at("custody_native_fee_normalized"))
                        && amount(row, "fee")? > zero()))
            {
                senders.push(*row);
            }
        }
        if senders.len() < 2
            || group
                .iter()
                .any(|r| handled.contains(&codec::pystr(r.at("id"))))
        {
            continue;
        }
        let sender_wallets: BTreeSet<_> = senders
            .iter()
            .map(|r| codec::pystr(r.at("wallet_id")))
            .collect();
        if sender_wallets.len() != senders.len() {
            continue;
        }
        let Some(graph) = senders
            .iter()
            .find_map(|r| parsed.get(&codec::pystr(r.at("id"))))
        else {
            continue;
        };
        let (inputs, complete) = component_inputs(graph, senders[0]);
        if !complete || py::truth(py::get(graph, "evidence_conflicts")) {
            continue;
        }
        let normalized = senders
            .iter()
            .all(|r| truth(r.at("custody_native_fee_normalized")));
        let fees = senders
            .iter()
            .map(|r| amount(r, "fee"))
            .collect::<Result<Vec<_>, _>>()?;
        if !normalized && fees.iter().collect::<BTreeSet<_>>().len() != 1 {
            continue;
        }
        let fee = if normalized {
            fees.iter().sum()
        } else {
            fees[0].clone()
        };
        let collaborative = if fee == zero() && chain == "bitcoin" {
            collaborative(&group, graph, index, &physical)?
        } else {
            None
        };
        let mut destinations = BTreeMap::new();
        let mut external = zero();
        let mut ambiguous = false;
        if let Some(destinations_proven) = &collaborative {
            destinations = destinations_proven.clone();
        } else {
            for output in py::list(py::get(graph, "outputs")) {
                let owner_ids = output_owners(index, output, &physical)?;
                let relation = relation(output, graph, senders[0]);
                if relation == Relation::Different {
                    continue;
                }
                if relation == Relation::Unknown {
                    ambiguous = true;
                    break;
                }
                if py::get(output, "role") == &Py::Str("fee".into()) {
                    continue;
                }
                let Some(value) = value(output) else {
                    ambiguous = true;
                    break;
                };
                if owner_ids.is_empty() {
                    external += value;
                    continue;
                }
                if !owner_ids.is_disjoint(&sender_wallets) {
                    continue;
                }
                if owner_ids.len() > 1 {
                    ambiguous = true;
                    break;
                }
                *destinations
                    .entry(owner_ids.into_iter().next().unwrap())
                    .or_insert_with(zero) += value;
            }
        }
        if ambiguous || external > zero() || destinations.is_empty() {
            continue;
        }
        let outflow = destinations.values().sum::<BigInt>() * 1000;
        let mut input_owners = BTreeSet::new();
        for input in &inputs {
            input_owners.extend(owners(index, input, &physical)?);
        }
        if !sender_wallets.is_subset(&input_owners) {
            continue;
        }
        if collaborative.is_none() {
            let mut owned = !inputs.is_empty();
            for input in &inputs {
                let owners = owners(index, input, &physical)?;
                if owners.is_empty() || !owners.is_subset(&sender_wallets) {
                    owned = false;
                    break;
                }
            }
            if !owned {
                continue;
            }
        }
        let sum = senders
            .iter()
            .map(|r| amount(r, "amount"))
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .sum::<BigInt>();
        if sum
            + if normalized {
                zero()
            } else {
                &fee * (senders.len() as i64 - 1)
            }
            != outflow
        {
            continue;
        }
        let group_ids: BTreeSet<_> = group.iter().map(|r| codec::pystr(r.at("id"))).collect();
        let txid = graph::strict_txid(&json!(field(graph, "txid"))).unwrap_or(txid_key);
        let mut receipts = BTreeMap::new();
        let mut valid = true;
        for (wallet, sats) in &destinations {
            if !wallets.contains_key(wallet) || *sats <= zero() {
                valid = false;
                break;
            }
            let mut candidates = Vec::new();
            for row in &group {
                if row.at("direction") == "inbound"
                    && codec::pystr(row.at("wallet_id")) == *wallet
                    && amount(row, "amount")? > zero()
                {
                    candidates.push(*row);
                }
            }
            let msat = sats * 1000;
            if candidates.len() > 1
                || (!candidates.is_empty() && amount(candidates[0], "amount")? != msat)
            {
                valid = false;
                break;
            }
            for row in rows {
                if row.at("direction") == "inbound"
                    && amount(row, "amount")? > zero()
                    && codec::pystr(row.at("wallet_id")) == *wallet
                    && codec::text(row.at("asset")).py_upper() == asset.py_upper()
                    && !group_ids.contains(&codec::pystr(row.at("id")))
                    && !handled.contains(&codec::pystr(row.at("id")))
                    && !row.onchain_scope().is_some_and(|s| s.2 != txid)
                    && abs(amount(row, "amount")? - &msat)
                        <= (abs(msat.clone()) / 100i64).max(BigInt::from(2_500_000))
                {
                    valid = false;
                    break;
                }
            }
            if !valid {
                break;
            }
            if let Some(receipt) = candidates.first() {
                receipts.insert(wallet.clone(), *receipt);
            }
        }
        if !valid || (!receipts.is_empty() && receipts.len() < destinations.len()) {
            continue;
        }
        senders.sort_by_key(|r| codec::pystr(r.at("id")));
        let bearer = codec::pystr(senders[0].at("id"));
        let block_rows: Vec<_> = receipts.values().copied().collect();
        let mut sinks = destinations.iter();
        let (mut sink, mut remaining) = sinks.next().map(|(id, n)| (id, n * 1000i64)).unwrap();
        let mut capacities = Vec::new();
        let mut source_ids = BTreeSet::new();
        for row in &senders {
            let source = codec::pystr(row.at("id"));
            let capacity = amount(row, "amount")?
                + if normalized || source == bearer {
                    zero()
                } else {
                    fee.clone()
                };
            if source.is_empty() || capacity < zero() {
                return Err("CustodyAllocationError".into());
            }
            source_ids.insert(source);
            capacities.push(capacity);
        }
        if source_ids.len() != senders.len() || destinations.keys().any(|id| id.is_empty()) {
            return Err("CustodyAllocationError".into());
        }
        for (row, capacity) in senders.iter().zip(capacities) {
            let source = codec::pystr(row.at("id"));
            let wallet = codec::pystr(row.at("wallet_id"));
            let mut available = capacity;
            let mut start = zero();
            while available > zero() {
                let cell = available.clone().min(remaining.clone());
                let suffix = if destinations.len() > 1 {
                    format!(":{sink}")
                } else {
                    String::new()
                };
                let leg_fee = if start == zero() {
                    if normalized {
                        amount(row, "fee")?
                    } else if source == bearer {
                        fee.clone()
                    } else {
                        zero()
                    }
                } else {
                    zero()
                };
                let out = clone_leg(
                    row,
                    &cell,
                    &leg_fee,
                    &format!("multi-consol:{txid}:out:{wallet}{suffix}"),
                    &source,
                    None,
                )?;
                let journal = receipts
                    .get(sink)
                    .map(|r| codec::pystr(r.at("id")))
                    .unwrap_or_else(|| source.clone());
                let incoming = clone_leg(
                    row,
                    &cell,
                    &zero(),
                    &format!("multi-consol:{txid}:in:{wallet}{suffix}"),
                    &journal,
                    Some((sink, &wallets[sink])),
                )?;
                result.derived_pairs.push(json!({"out":out,"in":incoming,"out_amount_msat":super::fanout::number(&cell),"source":"multi_source_consolidation","group_id":format!("multi-consol:{txid}"),"group_block_rows":block_rows.iter().map(|row| row.json()).collect::<Vec<_>>()}));
                result.synthetic_rows.extend([out, incoming]);
                available -= &cell;
                remaining -= &cell;
                start += cell;
                if remaining == zero() {
                    if let Some((id, n)) = sinks.next() {
                        sink = id;
                        remaining = n * 1000i64;
                    } else {
                        break;
                    }
                }
            }
        }
        result
            .dropped_out_ids
            .extend(senders.iter().map(|r| codec::pystr(r.at("id"))));
        result
            .dropped_in_ids
            .extend(receipts.values().map(|r| codec::pystr(r.at("id"))));
    }
    Ok(result)
}

pub(super) fn compile(snapshot: &Snapshot, context: &Context) -> Result<Value, String> {
    let rows: Vec<_> = snapshot.transactions.iter().map(|row| json!(row)).collect();
    let handled = BTreeSet::new();
    let result = (|| {
        let derivation = derive(&rows, snapshot, &handled)?;
        let compiled = pairs::compile(&derivation.derived_pairs, context, snapshot)?;
        Ok::<_, String>(
            json!({"derivation":derivation,"handled_ids":handled,"claims":compiled["claims"],"blocked_transaction_ids":compiled["blocked_transaction_ids"],"quarantines":compiled["quarantines"],"native_audits":pairs::native_audits(&derivation.derived_pairs,context)?}),
        )
    })();
    Ok(result.unwrap_or_else(|error| json!({"error":error})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consolidation_collaborative_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_consolidation_helpers.json"
        ))
        .unwrap();
        for vector in vectors {
            let snapshot: Snapshot = serde_json::from_value(vector["input"].clone()).unwrap();
            let rows: Vec<_> = snapshot.transactions.iter().map(|r| json!(r)).collect();
            let group: Vec<_> = rows.iter().collect();
            let graph = codec::loads(vector["parsed_json"].as_str().unwrap()).unwrap();
            let scope = serde_json::from_value(vector["scope"].clone()).unwrap();
            let result =
                collaborative(&group, &graph, Index(snapshot.owned_index.as_ref()), &scope);
            if vector.get("error").is_some() {
                assert_eq!(
                    result.err().map(|e| json!(e)),
                    Some(vector["error"].clone()),
                    "{}",
                    vector["case"]
                );
            } else {
                let actual = result.unwrap().map(|items| {
                    items
                        .into_iter()
                        .map(|(key, n)| (key, super::super::fanout::number(&n)))
                        .collect::<BTreeMap<_, _>>()
                });
                assert_eq!(json!(actual), vector["expected"], "{}", vector["case"]);
            }
        }
    }
    #[test]
    fn consolidation_production_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_consolidation_cases.json"
        ))
        .unwrap();
        for vector in vectors {
            let snapshot: Snapshot = serde_json::from_value(vector["input"].clone()).unwrap();
            let rows = super::super::array(&vector["rows"]);
            let handled = super::super::array(&vector["handled_ids"])
                .iter()
                .map(codec::text)
                .collect();
            let result = derive(rows, &snapshot, &handled);
            if vector.get("error").is_some() {
                assert_eq!(
                    result.err().map(|e| json!(e)),
                    Some(vector["error"].clone()),
                    "{}",
                    vector["case"]
                );
            } else {
                assert_eq!(
                    serde_json::to_value(result.unwrap()).unwrap(),
                    vector["expected"],
                    "{}",
                    vector["case"]
                );
            }
        }
    }
}

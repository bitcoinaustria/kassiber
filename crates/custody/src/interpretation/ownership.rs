//! Isolated graph ownership producer. Python remains the product authority;
//! graph scalars stay typed until their diagnostic wire projection.
use super::{
    fanout::{amount, clone_base, number},
    pairs, Context,
};
use crate::fields::{Fields, Row};
use crate::{
    codec::{self, truth},
    evidence,
    graph::{
        self,
        ownership::{self as index, Index, PhysicalScope, TransferScope},
        stored as py,
    },
    python_text::PythonText,
    snapshot::Snapshot,
};
use kassiber_pyjson::{BigInt, Value as Py};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const SYNTHETIC: &[&str] = &[
    "owned-derive:",
    "recorded-fanout:",
    "cross-split:",
    "direct-payout:",
    "multi-consol:",
];
pub(super) fn synthetic<R: Row + ?Sized>(row: &R) -> bool {
    SYNTHETIC
        .iter()
        .any(|prefix| codec::pystr(row.at("id")).starts_with(prefix))
}
pub(super) fn abs(value: BigInt) -> BigInt {
    if value < BigInt::from(0) {
        -value
    } else {
        value
    }
}
pub(super) fn zero() -> BigInt {
    BigInt::from(0)
}
pub(super) fn field(value: &Py, key: &str) -> String {
    py::text(py::get(value, key))
}
pub(super) fn value(leg: &Py) -> Option<BigInt> {
    if let Py::Int(n) = py::get(leg, "value_sats") {
        Some(n.clone())
    } else {
        None
    }
}
pub(super) fn scope<R: Row + ?Sized>(
    row: &R,
    graph: Option<&Py>,
    index: Index<'_>,
) -> Result<Option<TransferScope>, String> {
    index::scope(row, graph.map(py::project).as_ref(), index)
}
fn physical(scope: &TransferScope) -> PhysicalScope {
    (scope.0.clone(), scope.1.clone())
}
pub(super) fn owners(
    index: Index<'_>,
    leg: &Py,
    scope: &PhysicalScope,
) -> Result<BTreeSet<String>, String> {
    index.input_owners(&py::project(leg), scope)
}
pub(super) fn output_owners(
    index: Index<'_>,
    leg: &Py,
    scope: &PhysicalScope,
) -> Result<BTreeSet<String>, String> {
    index.script_owners(&py::project(py::get(leg, "script")), scope)
}

#[derive(Default, Serialize, serde::Deserialize)]
pub(super) struct Derivation {
    pub(super) derived_pairs: Vec<Value>,
    pub(super) synthetic_rows: Vec<Value>,
    pub(super) out_row_overrides: BTreeMap<String, Value>,
    pub(super) dropped_out_ids: BTreeSet<String>,
    pub(super) dropped_in_ids: BTreeSet<String>,
    pub(super) blocked_sources: Vec<Value>,
}
impl Derivation {
    fn block<R: Row + ?Sized>(&mut self, row: &R, reason: &str, detail: Value, complete: bool) {
        let mut payload = json!({"required_for":if complete {"complete_transfer_component"} else {"ownership_transfer_review"}, "wallet":if truth(row.at("wallet_label")) {row.at("wallet_label")} else {row.at("wallet_id")}, "asset":row.at("asset"), "external_id":row.at("external_id")});
        if let Some(detail) = detail.as_object() {
            payload.as_object_mut().unwrap().extend(detail.clone());
        }
        self.blocked_sources
            .push(json!({"row":row.json(),"reason":reason,"detail":payload}));
    }
}

fn component_asset<R: Row + ?Sized>(graph: &Py, row: &R) -> (String, String) {
    let component = py::get(graph, "component");
    let id = field(component, "asset_id").py_strip().py_lower();
    let asset = if py::truth(py::get(component, "asset")) {
        field(component, "asset")
    } else {
        codec::text(row.at("asset"))
    };
    (id, asset.py_strip().py_upper())
}
#[derive(PartialEq)]
pub(super) enum Relation {
    Same,
    Different,
    Unknown,
}
pub(super) fn relation<R: Row + ?Sized>(leg: &Py, graph: &Py, row: &R) -> Relation {
    if field(graph, "chain").py_lower() != "liquid" {
        return Relation::Same;
    }
    let (component_id, component_asset) = component_asset(graph, row);
    let id = field(leg, "asset_id").py_strip().py_lower();
    let asset = field(leg, "asset").py_strip().py_upper();
    if !component_id.is_empty() && !id.is_empty() {
        return if component_id == id {
            Relation::Same
        } else {
            Relation::Different
        };
    }
    if !component_id.is_empty() {
        return Relation::Unknown;
    }
    if !component_asset.is_empty() && !asset.is_empty() {
        return if component_asset == asset {
            Relation::Same
        } else {
            Relation::Different
        };
    }
    Relation::Unknown
}
pub(super) fn component_inputs<'a, R: Row + ?Sized>(graph: &'a Py, row: &R) -> (Vec<&'a Py>, bool) {
    let inputs = py::list(py::get(graph, "inputs"));
    if field(graph, "chain").py_lower() != "liquid" {
        return (inputs.iter().collect(), true);
    }
    let mut selected = Vec::new();
    let mut complete = true;
    for leg in inputs {
        match relation(leg, graph, row) {
            Relation::Same => selected.push(leg),
            Relation::Unknown => complete = false,
            Relation::Different => {}
        }
    }
    complete &= !selected.is_empty();
    (selected, complete)
}
fn conserves<R: Row + ?Sized>(
    graph: &Py,
    row: &R,
    index: Index<'_>,
    wallet: &str,
    inputs: &[&Py],
    total: &BigInt,
    scope: &PhysicalScope,
) -> Result<(bool, Value), String> {
    if field(graph, "chain").py_lower() != "liquid" {
        return Ok((true, json!({})));
    }
    let Some(input_sats): Option<BigInt> = inputs.iter().map(|leg| value(leg)).sum() else {
        return Ok((false, json!({"missing":"input_value_or_asset"})));
    };
    let mut change = zero();
    for leg in py::list(py::get(graph, "outputs")) {
        if !output_owners(index, leg, scope)?.contains(wallet) {
            continue;
        }
        let relation = relation(leg, graph, row);
        if relation == Relation::Different {
            continue;
        }
        let Some(n) = value(leg).filter(|_| relation != Relation::Unknown) else {
            return Ok((false, json!({"missing":"owned_change_value_or_asset"})));
        };
        change += n;
    }
    let expected = (&input_sats - &change) * 1000;
    Ok((
        expected == *total,
        json!({"input_sats":number(&input_sats),"change_sats":number(&change),"expected_outflow_msat":number(&expected),"row_total_outflow_msat":number(total)}),
    ))
}
fn exact_fee<R: Row + ?Sized>(graph: &Py, row: &R) -> Option<BigInt> {
    if py::truth(py::get(graph, "evidence_conflicts")) {
        return None;
    }
    match field(graph, "chain").py_strip().py_lower().as_str() {
        "bitcoin" => {
            let inputs = py::list(py::get(graph, "inputs"));
            let outputs = py::list(py::get(graph, "outputs"));
            if inputs.is_empty() || outputs.is_empty() {
                return None;
            }
            let input: BigInt = inputs.iter().map(value).sum::<Option<_>>()?;
            let output: BigInt = outputs.iter().map(value).sum::<Option<_>>()?;
            let fee = input - output;
            (fee >= zero()).then(|| fee * 1000)
        }
        "liquid" => {
            let fees: Vec<_> = py::list(py::get(graph, "outputs"))
                .iter()
                .filter(|leg| py::get(leg, "role") == &Py::Str("fee".into()))
                .collect();
            if fees.is_empty() || fees.iter().any(|leg| value(leg).is_none()) {
                return None;
            }
            let (id, asset) = component_asset(graph, row);
            let matching: Vec<_> = fees
                .into_iter()
                .filter(|leg| {
                    let leg_id = field(leg, "asset_id").py_strip().py_lower();
                    let leg_asset = field(leg, "asset").py_strip().py_upper();
                    if !id.is_empty() && !leg_id.is_empty() {
                        id == leg_id
                    } else {
                        !asset.is_empty() && !leg_asset.is_empty() && asset == leg_asset
                    }
                })
                .collect();
            if matching.len() != 1 {
                return None;
            }
            let fee = value(matching[0])?;
            (fee >= zero()).then(|| fee * 1000)
        }
        _ => None,
    }
}
fn row_liquid<R: Row + ?Sized>(row: &R) -> bool {
    let raw = py::object(row.at("raw_json"));
    field(&raw, "chain").py_strip().py_lower() == "liquid"
        || py::truth(py::get(py::get(&raw, "component"), "asset_id"))
        || matches!(
            codec::text(row.at("asset")).py_strip().py_upper().as_str(),
            "LBTC" | "L-BTC"
        )
}
fn graphless<R: Row>(result: &mut Derivation, row: &R, rows: &[R]) -> Result<(), String> {
    let Some(scope) = row.onchain_scope() else {
        return Ok(());
    };
    let ins: Vec<_> = rows
        .iter()
        .filter(|candidate| {
            candidate.at("direction") == "inbound"
                && codec::pystr(candidate.at("wallet_id")) != codec::pystr(row.at("wallet_id"))
                && codec::text(candidate.at("asset")).py_upper()
                    == codec::text(row.at("asset")).py_upper()
                && candidate.onchain_scope().as_ref() == Some(&scope)
        })
        .collect();
    let total: BigInt = ins
        .iter()
        .map(|r| amount(r, "amount"))
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .sum();
    if !ins.is_empty() && total != amount(row, "amount")? {
        result.block(row, "liquid_transfer_graph_incomplete", json!({"row_amount_msat":number(&amount(row,"amount")?),"owned_receipts_msat":number(&total)}), false);
    }
    Ok(())
}
fn contradicts(
    inputs: &[&Py],
    index: Index<'_>,
    wallet: &str,
    scope: &PhysicalScope,
) -> Result<bool, String> {
    for entry in inputs {
        let mut matches = if py::truth(py::get(entry, "outpoint")) {
            index.outpoint(&py::project(py::get(entry, "outpoint")), None)
        } else {
            Vec::new()
        };
        if matches.is_empty() {
            matches = index.script(&py::project(py::get(entry, "script")))?;
        }
        let matches: Vec<_> = matches
            .iter()
            .filter(|m| codec::pystr(&m["wallet_id"]) == wallet)
            .collect();
        if !matches.is_empty()
            && !matches
                .iter()
                .any(|m| index::normalized(&m["chain"], &m["network"]) == *scope)
        {
            return Ok(true);
        }
    }
    if inputs.len() != 1 || !py::truth(py::get(inputs[0], "outpoint")) {
        return Ok(false);
    }
    let txid = field(inputs[0], "outpoint")
        .split(':')
        .next()
        .unwrap_or("")
        .py_lower();
    let unscoped = index.txid(&json!(txid), None);
    if !unscoped.iter().any(|(w, _)| w == wallet) {
        return Ok(false);
    }
    Ok(!index
        .txid(&json!(txid), Some(scope))
        .iter()
        .any(|(w, _)| w == wallet))
}
fn single_source<R: Row + ?Sized>(
    inputs: &[&Py],
    index: Index<'_>,
    wallet: &str,
    row: &R,
    scope: &PhysicalScope,
) -> Result<bool, String> {
    let expected = BTreeSet::from([wallet.to_owned()]);
    let mut strict = !inputs.is_empty();
    for entry in inputs {
        if owners(index, entry, scope)? != expected {
            strict = false;
            break;
        }
    }
    if strict {
        return Ok(true);
    }
    if inputs.len() != 1
        || row.at("direction") != "outbound"
        || amount(row, "amount")? <= zero()
        || codec::pystr(row.at("wallet_id")) != wallet
    {
        return Ok(false);
    }
    if !py::truth(py::get(inputs[0], "outpoint")) {
        return Ok(false);
    }
    if !owners(index, inputs[0], scope)?.is_empty() {
        return Ok(false);
    }
    let txid = field(inputs[0], "outpoint")
        .split(':')
        .next()
        .unwrap_or("")
        .py_lower();
    Ok(index
        .txid(&json!(txid), Some(scope))
        .into_iter()
        .map(|(w, _)| w)
        .collect::<BTreeSet<_>>()
        == expected)
}
fn destination<'a, R: Row>(
    candidates: &[&'a R],
    leg: &BigInt,
    txid: &str,
    consumed: &BTreeSet<String>,
    handled: &BTreeSet<String>,
    asset: &Value,
    scope: &TransferScope,
) -> Result<Result<Option<&'a R>, ()>, String> {
    let mut available = Vec::new();
    for row in candidates {
        let id = codec::pystr(row.at("id"));
        if !consumed.contains(&id)
            && !handled.contains(&id)
            && codec::text(row.at("asset")).py_upper() == codec::text(asset).py_upper()
            && amount(row, "amount")? > zero()
        {
            available.push(*row);
        }
    }
    let mut same = Vec::new();
    for row in &available {
        if amount(row, "amount")? == *leg && row.onchain_scope().as_ref() == Some(scope) {
            same.push(*row);
        }
    }
    if same.len() == 1 {
        return Ok(Ok(Some(same[0])));
    }
    if same.len() >= 2 {
        return Ok(Err(()));
    }
    let tolerance = (abs(leg.clone()) / 100i64).max(BigInt::from(2_500_000));
    // Python evaluates the complete blocking comprehension (including errors).
    let mut blocked = false;
    for row in available {
        let different = row.onchain_scope().is_some_and(|scope| scope.2 != txid);
        if !different && abs(amount(row, "amount")? - leg) <= tolerance {
            blocked = true;
        }
    }
    Ok(if blocked { Err(()) } else { Ok(None) })
}
pub(super) fn clone_leg<R: Row + ?Sized>(
    row: &R,
    amount: &BigInt,
    fee: &BigInt,
    id: &str,
    source: &str,
    dest: Option<(&str, &Value)>,
) -> Result<Value, String> {
    let mut leg = clone_base(row, amount, fee)?;
    leg["id"] = json!(id);
    leg["external_id"] = json!(id);
    leg["kind"] = json!(if dest.is_some() {
        "self_transfer_in"
    } else {
        "self_transfer_out"
    });
    leg["journal_transaction_id"] = json!(source);
    if let Some((wallet, reference)) = dest {
        leg["direction"] = json!("inbound");
        leg["wallet_id"] = json!(wallet);
        for (target, source) in [
            ("wallet_label", "label"),
            ("wallet_account_id", "wallet_account_id"),
            ("account_code", "account_code"),
            ("account_label", "account_label"),
        ] {
            leg[target] = reference[source].clone();
        }
    }
    Ok(leg)
}

pub(super) fn derive<R: Row>(
    rows: &[R],
    snapshot: &Snapshot,
    handled: &BTreeSet<String>,
) -> Result<Derivation, String> {
    let mut result = Derivation::default();
    let index = Index(snapshot.owned_index.as_ref());
    if index.0.is_none() {
        for row in rows {
            if row.at("direction") != "outbound"
                || !row_liquid(row)
                || handled.contains(&codec::pystr(row.at("id")))
            {
                continue;
            }
            graphless(&mut result, row, rows)?;
        }
        return Ok(result);
    }
    let mut sources: BTreeMap<TransferScope, BTreeSet<String>> = BTreeMap::new();
    for row in rows {
        if let Some(scope) = row.onchain_scope() {
            if row.at("direction") == "outbound" {
                sources
                    .entry(scope)
                    .or_default()
                    .insert(codec::pystr(row.at("wallet_id")));
            }
        }
    }
    for row in rows {
        if row.at("direction") != "inbound"
            || handled.contains(&codec::pystr(row.at("id")))
            || !evidence::current_observation_of(row).map_err(|error| {
                // Quantity hashing uses Python int(); preserve the offending
                // field's family after the hash predicate reaches it.
                if error.message == "invalid integer quantity" {
                    amount(row, "amount")
                        .and_then(|_| amount(row, "fee"))
                        .err()
                        .unwrap_or_else(|| error.code.into())
                } else {
                    error.code.into()
                }
            })?
        {
            continue;
        }
        let parsed = py::parse(row.at("raw_json"))?;
        let scope = scope(row, parsed.as_ref(), index)?;
        let (Some(parsed), Some(scope)) = (parsed, scope) else {
            continue;
        };
        let (inputs, _) = component_inputs(&parsed, row);
        let mut missing = BTreeSet::new();
        for input in inputs {
            missing.extend(owners(index, input, &physical(&scope))?);
        }
        if let Some(recorded) = sources.get(&scope) {
            missing.retain(|w| !recorded.contains(w));
        }
        missing.remove(&codec::pystr(row.at("wallet_id")));
        let mut active = BTreeSet::new();
        for wallet in missing {
            if graph::native_wallet_delta_msat(&row.json(), snapshot, &wallet)?
                .is_none_or(|delta| delta > zero())
            {
                active.insert(wallet);
            }
        }
        if !active.is_empty() {
            result.block(
                row,
                "ownership_transfer_source_missing",
                json!({"missing_source_wallet_ids":active}),
                false,
            );
        }
    }
    let mut inbound: BTreeMap<String, Vec<&R>> = BTreeMap::new();
    for row in rows {
        if row.at("direction") == "inbound" && !synthetic(row) {
            inbound
                .entry(codec::pystr(row.at("wallet_id")))
                .or_default()
                .push(row);
        }
    }
    let parsed = index::profile_parsed_typed(rows, index)?;
    let mut out_graphs = BTreeMap::new();
    let mut groups: Vec<(TransferScope, Vec<&R>)> = Vec::new();
    let mut group_positions = BTreeMap::new();
    for row in rows {
        if row.at("direction") != "outbound" || synthetic(row) || amount(row, "amount")? <= zero() {
            continue;
        }
        let id = codec::pystr(row.at("id"));
        let graph = parsed.get(&id);
        out_graphs.insert(id, graph);
        let scope = scope(row, graph, index)?;
        if let (Some(_), Some(scope)) = (graph, scope) {
            let position = *group_positions.entry(scope.clone()).or_insert(groups.len());
            if position == groups.len() {
                groups.push((scope, vec![row]));
            } else {
                groups[position].1.push(row);
            }
        }
    }
    let mut duplicates = BTreeMap::new();
    for (_, group) in &groups {
        let mut economic: BTreeMap<(BigInt, BigInt, bool), Vec<&R>> = BTreeMap::new();
        for row in group {
            economic
                .entry((
                    amount(row, "amount")?,
                    amount(row, "fee")?,
                    truth(row.at("amount_includes_fee")),
                ))
                .or_default()
                .push(*row);
        }
        for repeated in economic.values().filter(|g| g.len() > 1) {
            for row in repeated {
                duplicates.insert(codec::pystr(row.at("id")), repeated.clone());
            }
        }
    }
    let mut consumed = BTreeSet::new();
    for row in rows {
        let source_id = codec::pystr(row.at("id"));
        if row.at("direction") != "outbound"
            || handled.contains(&source_id)
            || synthetic(row)
            || amount(row, "amount")? <= zero()
        {
            continue;
        }
        let Some(graph) = out_graphs.get(&source_id).copied().flatten() else {
            if row_liquid(row) {
                graphless(&mut result, row, rows)?;
            }
            continue;
        };
        let Some(group) = scope(row, Some(graph), index)? else {
            continue;
        };
        let scope = physical(&group);
        let wallet = codec::pystr(row.at("wallet_id"));
        let (inputs, complete) = component_inputs(graph, row);
        if py::truth(py::get(graph, "evidence_conflicts")) || !complete {
            result.block(
                row,
                "ownership_transfer_asset_evidence_incomplete",
                json!({"evidence_conflicts":py::project(py::get(graph,"evidence_conflicts"))}),
                false,
            );
            continue;
        }
        if contradicts(&inputs, index, &wallet, &scope)? {
            result.block(row,"ownership_transfer_source_ambiguous",json!({"canonical_chain":scope.0,"canonical_network":scope.1,"scope_conflict":true}),false);
            continue;
        }
        let mut destinations: BTreeMap<String, (BigInt, BigInt)> = BTreeMap::new();
        let mut ambiguous = false;
        let mut incomplete = false;
        let mut external = zero();
        let mut external_complete = true;
        for output in py::list(py::get(graph, "outputs")) {
            let matches = output_owners(index, output, &scope)?;
            let relation = relation(output, graph, row);
            if matches.is_empty() {
                if relation == Relation::Same && value(output).is_some() {
                    external += value(output).unwrap();
                } else if relation != Relation::Different {
                    external_complete = false;
                }
                continue;
            }
            if relation == Relation::Different {
                continue;
            }
            let Some(n) = value(output).filter(|_| relation != Relation::Unknown) else {
                incomplete = true;
                break;
            };
            if matches.contains(&wallet) {
                continue;
            }
            if matches.len() > 1 {
                ambiguous = true;
                continue;
            }
            let owner = matches.into_iter().next().unwrap();
            let Py::Int(position) = py::get(output, "n") else {
                unreachable!("parsed output index")
            };
            let slot = destinations
                .entry(owner)
                .or_insert_with(|| (zero(), position.clone()));
            slot.0 += n;
            slot.1 = slot.1.clone().min(position.clone());
        }
        if incomplete {
            result.block(
                row,
                "ownership_transfer_asset_evidence_incomplete",
                json!({"missing":"owned_output_value_or_asset"}),
                false,
            );
            continue;
        }
        if ambiguous {
            result.block(
                row,
                "ownership_transfer_ambiguous_output",
                if external_complete && external > zero() {
                    json!({"verified_external_msat":number(&(external*1000))})
                } else {
                    json!({})
                },
                false,
            );
            continue;
        }
        if destinations.is_empty() {
            continue;
        }
        if let Some(duplicate) = duplicates.get(&source_id) {
            let mut ids: Vec<_> = duplicate.iter().map(|r| codec::pystr(r.at("id"))).collect();
            ids.sort();
            result.block(
                row,
                "ownership_transfer_duplicate_outbound",
                json!({"outbound_count":duplicate.len(),"outbound_ids":ids}),
                false,
            );
            continue;
        }
        if !single_source(&inputs, index, &wallet, row, &scope)? {
            result.block(row, "ownership_transfer_source_ambiguous", json!({}), false);
            continue;
        }
        let source_amount = amount(row, "amount")?;
        let mut fee = amount(row, "fee")?;
        let mut total = source_amount.clone();
        let inclusive = truth(row.at("amount_includes_fee"));
        let implicit = inclusive
            && (if truth(row.at("observation_fee_attribution")) {
                codec::pystr(row.at("observation_fee_attribution"))
            } else {
                "unknown".into()
            })
            .py_strip()
            .py_lower()
                == "implicit_wallet_delta";
        let mut exact = None;
        if inclusive && !implicit {
            exact = exact_fee(graph, row);
            fee = exact.clone().unwrap_or_else(zero);
        } else {
            total += &fee;
        }
        let (conserves, detail) = conserves(graph, row, index, &wallet, &inputs, &total, &scope)?;
        if !conserves {
            result.block(
                row,
                "ownership_transfer_conservation_mismatch",
                detail,
                false,
            );
            continue;
        }
        let legs: BigInt = destinations.values().map(|(v, _)| v * 1000).sum();
        if legs > total {
            result.block(row,"ownership_transfer_amount_mismatch",json!({"row_amount_msat":number(&source_amount),"row_total_outflow_msat":number(&total),"owned_outputs_msat":number(&legs)}),false);
            continue;
        }
        let gap = (&total - &legs).max(zero());
        if inclusive && !implicit && gap > zero() {
            let Some(exact) = &exact else {
                result.block(
                    row,
                    "ownership_transfer_fee_evidence_incomplete",
                    json!({"folded_gap_msat":number(&gap),"missing":"exact_network_fee"}),
                    true,
                );
                continue;
            };
            if *exact > gap {
                result.block(
                    row,
                    "ownership_transfer_conservation_mismatch",
                    json!({"folded_gap_msat":number(&gap),"exact_network_fee_msat":number(exact)}),
                    true,
                );
                continue;
            }
        }
        let txid = &group.2;
        let multiple = destinations.len() > 1;
        let budget = fee.min(gap);
        let mut destinations: Vec<_> = destinations.into_iter().collect();
        destinations.sort_by(|a, b| (&a.1 .1, &a.0).cmp(&(&b.1 .1, &b.0)));
        let mut leg_pairs = Vec::new();
        let mut leg_rows = Vec::new();
        let mut ok = true;
        let mut decline = None;
        for (position, (dest, (sats, n))) in destinations.into_iter().enumerate() {
            let leg = sats * 1000;
            if leg <= zero() {
                ok = false;
                break;
            }
            let out = clone_leg(
                row,
                &leg,
                &if position == 0 {
                    budget.clone()
                } else {
                    zero()
                },
                &format!("owned-derive:{txid}:out:{n}"),
                &source_id,
                None,
            )?;
            let decision = destination(
                inbound.get(&dest).map(Vec::as_slice).unwrap_or(&[]),
                &leg,
                txid,
                &consumed,
                handled,
                row.at("asset"),
                &group,
            )?;
            let incoming = match decision {
                Err(()) => {
                    ok = false;
                    decline = Some((
                        "ownership_transfer_destination_ambiguous",
                        json!({"destination_wallet_id":dest,"leg_amount_msat":number(&leg)}),
                    ));
                    break;
                }
                Ok(Some(row)) => {
                    consumed.insert(codec::pystr(row.at("id")));
                    row.json().into_owned()
                }
                Ok(None) => {
                    let Some(reference) = snapshot
                        .wallets
                        .iter()
                        .rev()
                        .find(|w| codec::pystr(&json!(w)["id"]) == dest)
                    else {
                        ok = false;
                        decline = Some((
                            "ownership_transfer_destination_missing_ref",
                            json!({"destination_wallet_id":dest}),
                        ));
                        break;
                    };
                    let row = clone_leg(
                        row,
                        &leg,
                        &zero(),
                        &format!("owned-derive:{txid}:in:{n}"),
                        &source_id,
                        Some((&dest, &json!(reference))),
                    )?;
                    leg_rows.push(row.clone());
                    row
                }
            };
            let mut pair = json!({"out":out,"in":incoming,"source":"ownership_derived"});
            if multiple {
                pair["group_id"] = json!(format!("owned-derive:{txid}"));
            }
            leg_pairs.push(pair);
            leg_rows.push(out);
        }
        if !ok || leg_pairs.is_empty() {
            for pair in leg_pairs {
                consumed.remove(&codec::pystr(&pair["in"]["id"]));
            }
            if let Some((reason, detail)) = decline {
                result.block(row, reason, detail, false);
            }
            continue;
        }
        result.derived_pairs.extend(leg_pairs);
        result.synthetic_rows.extend(leg_rows);
        let residual = total - legs - budget;
        if residual > zero() {
            result
                .out_row_overrides
                .insert(source_id, clone_base(row, &residual, &zero())?);
        } else {
            result.dropped_out_ids.insert(source_id);
        }
    }
    Ok(result)
}

pub(super) fn compile(snapshot: &Snapshot, context: &Context) -> Result<Value, String> {
    let rows: Vec<_> = snapshot.transactions.iter().map(|row| json!(row)).collect();
    // No other isolated interpreter reserves rows for this producer. Pipeline
    // reservations belong to the later consolidation/fan-out/ownership slice.
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
    fn ownership_helper_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_ownership_helpers.json"
        ))
        .unwrap();
        for vector in vectors {
            let graph = codec::loads(vector["graph_json"].as_str().unwrap()).unwrap();
            let row = &vector["row"];
            let result = match vector["op"].as_str().unwrap() {
                "asset" => {
                    let (id, asset) = component_asset(&graph, row);
                    json!([
                        if id.is_empty() {
                            Value::Null
                        } else {
                            json!(id)
                        },
                        asset
                    ])
                }
                "relation" => json!(py::list(py::get(&graph, "inputs"))
                    .iter()
                    .chain(py::list(py::get(&graph, "outputs")))
                    .map(|leg| match relation(leg, &graph, row) {
                        Relation::Same => "same",
                        Relation::Different => "different",
                        Relation::Unknown => "unknown",
                    })
                    .collect::<Vec<_>>()),
                "inputs" => {
                    let (inputs, complete) = component_inputs(&graph, row);
                    json!([
                        inputs
                            .iter()
                            .map(|leg| py::project(leg))
                            .collect::<Vec<_>>(),
                        complete
                    ])
                }
                "row_liquid" => json!(row_liquid(row)),
                "conserves" => {
                    let snapshot: Snapshot =
                        serde_json::from_value(vector["snapshot"].clone()).unwrap();
                    let (inputs, _) = component_inputs(&graph, row);
                    let scope: PhysicalScope =
                        serde_json::from_value(vector["scope"].clone()).unwrap();
                    let total = amount(&json!({"amount":vector["total"]}), "amount").unwrap();
                    let (ok, detail) = conserves(
                        &graph,
                        row,
                        Index(snapshot.owned_index.as_ref()),
                        &codec::pystr(&row["wallet_id"]),
                        &inputs,
                        &total,
                        &scope,
                    )
                    .unwrap();
                    json!([ok, detail])
                }
                "fee" => exact_fee(&graph, row)
                    .as_ref()
                    .map(number)
                    .unwrap_or(Value::Null),
                _ => unreachable!(),
            };
            assert_eq!(result, vector["expected"], "{}", vector["case"]);
        }
    }
    #[test]
    fn ownership_production_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_ownership_cases.json"
        ))
        .unwrap();
        for vector in vectors {
            let snapshot: Snapshot = serde_json::from_value(vector["input"].clone()).unwrap();
            let rows: Vec<_> = snapshot.transactions.iter().map(|r| json!(r)).collect();
            let handled = super::super::array(&vector["handled_ids"])
                .iter()
                .map(codec::text)
                .collect();
            let result = derive(&rows, &snapshot, &handled);
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

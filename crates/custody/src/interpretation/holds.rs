//! Raw-observation safety. A hold only quarantines; it cannot create a MOVE.
use super::{anchor, integer_or_zero, pairs};
use crate::{
    codec::{self, truth},
    components::time,
    evidence, graph,
    graph::stored,
    python_text::PythonText,
};
use kassiber_pyjson::{BigInt, Value as Py};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

type Physical = (String, String, String);
pub(super) type Ids = BTreeSet<String>;
const SYNTHETIC: [&str; 5] = [
    "owned-derive:",
    "recorded-fanout:",
    "cross-split:",
    "direct-payout:",
    "multi-consol:",
];

fn positive(row: &Value) -> Result<bool, String> {
    Ok(
        BigInt::from_str(&integer_or_zero(&row["amount"])?.to_string()).expect("Python integer")
            > BigInt::from(0),
    )
}

/// The conflict sentinel has no asset authority. Unlike transfer matching,
/// Liquid confirmation safety also works without a trusted asset component.
fn conflict_scope(row: &Value, parsed: Option<&Py>) -> Option<Physical> {
    // Only a synthetic id is renamed; other rows are probed as they are.
    let mut renamed = None;
    if SYNTHETIC
        .iter()
        .any(|prefix| codec::text(&row["id"]).starts_with(prefix))
    {
        let probe = renamed.insert(row.clone());
        probe["id"] = json!(format!("physical-anchor:{}", codec::pystr(&row["id"])));
    }
    let probe = renamed.as_ref().unwrap_or(row);
    if let Some((chain, network, txid, _)) = graph::onchain_scope(probe) {
        return Some((chain, network, txid));
    }
    let parsed = parsed?;
    let raw_txid = stored::text(stored::get(parsed, "txid"));
    let raw_txid = raw_txid.py_strip();
    let parsed_txid = graph::strict_txid(&json!(raw_txid));
    if !raw_txid.is_empty() && parsed_txid.is_none() {
        return None;
    }
    let external = graph::strict_txid(&row["external_id"]);
    if parsed_txid.is_some() && external.is_some() && parsed_txid != external {
        return None;
    }
    let txid = parsed_txid.or(external)?;
    let asset = codec::text(&row["asset"])
        .py_strip()
        .py_upper()
        .replace("L-BTC", "LBTC");
    let chain = match asset.as_str() {
        "BTC" => "bitcoin",
        "LBTC" => "liquid",
        _ => return None,
    };
    let raw = graph::transfer_object(&row["raw_json"]);
    let config = graph::transfer_object(if truth(&row["config_json"]) {
        &row["config_json"]
    } else {
        &row["wallet_config_json"]
    });
    let mut families = BTreeSet::new();
    let mut domains = BTreeSet::new();
    for payload in [&raw, &config] {
        let value = &payload["chain"];
        if !value.is_null() && value != "" {
            let spelling = codec::text(value).py_strip().py_lower().replace('_', "-");
            families.insert(match spelling.as_str() {
                "bitcoin" | "btc" | "onchain" => "bitcoin",
                "liquid" | "lbtc" | "elements" => "liquid",
                "core-ln" | "core-lightning" => "bitcoin",
                value if evidence::lightning(value) => "bitcoin",
                _ => return None,
            });
        }
        for key in ["network", "bitcoin_network", "chain_network"] {
            let value = &payload[key];
            if value.is_null() || value == "" {
                continue;
            }
            // Whitespace-only explicit metadata is unsupported, not absent.
            if codec::text(value).py_strip().is_empty() {
                return None;
            }
            let network = evidence::network("bitcoin", value)
                .or_else(|_| evidence::network("liquid", value))
                .ok()?;
            domains.insert(super::recorded::domain_name(&network).to_owned());
        }
    }
    if families.len() > 1 || families.iter().any(|family| *family != chain) || domains.len() > 1 {
        return None;
    }
    let domain = domains
        .into_iter()
        .next()
        .or_else(|| (chain == "bitcoin").then(|| "main".into()))?;
    Some((chain.into(), domain, txid))
}

fn payload(row: &Value, nested: bool) -> Py {
    let raw = stored::object(&row["raw_json"]);
    if nested && matches!(stored::get(&raw, "tx"), Py::Dict(_)) {
        stored::get(&raw, "tx").clone()
    } else {
        raw
    }
}
fn explicit(payload: &Py) -> Option<bool> {
    match stored::get(stored::get(payload, "status"), "confirmed") {
        Py::Bool(value) => Some(*value),
        _ => None,
    }
}

pub(crate) fn capture_confirmation(row: &Value) -> Result<Value, String> {
    let authoritative = evidence::current_observation(row).map_err(|e| e.code.to_owned())?;
    if !authoritative {
        return Ok(Value::Null);
    }
    let Some(confirmed) = explicit(&payload(row, true)) else {
        return Ok(Value::Null);
    };
    let parsed = stored::parse(&row["raw_json"])?;
    let Some(scope) = conflict_scope(row, parsed.as_ref()) else {
        return Ok(Value::Null);
    };
    let observed_at = &row["observation_observed_at"];
    let Some(timestamp) = time::instant(observed_at).map_err(|e| e.code.to_owned())? else {
        return Ok(Value::Null);
    };
    Ok(
        json!({"$type":"ChainConfirmationObservation", "physical_scope":[scope.0,scope.1,scope.2], "observed_at":{"$datetime":crate::gaps::iso_datetime(timestamp).replace('Z', "+00:00")}, "confirmed":confirmed}),
    )
}

/// Pure per-row facts the confirmation detectors read: the parsed graph, its
/// conflict scope and its explicit status. They are derived concurrently and
/// read in each detector's own row order, so refusals keep their order.
pub(super) struct Facts {
    parsed: Result<Option<Py>, String>,
    scope: Option<Physical>,
    explicit: Option<bool>,
}
pub(super) fn facts(rows: &[Value]) -> Vec<Facts> {
    crate::par::map(rows, |row| {
        let parsed = stored::parse(&row["raw_json"]);
        let scope = match &parsed {
            Ok(parsed) => conflict_scope(row, parsed.as_ref()),
            Err(_) => None,
        };
        Facts {
            parsed,
            scope,
            explicit: explicit(&payload(row, true)),
        }
    })
    .into_iter()
    .map(crate::par::unwrap)
    .collect()
}

fn confirmation_states(
    rows: &[Value],
    facts: &[Facts],
) -> Result<BTreeMap<Physical, bool>, String> {
    let mut legacy = BTreeMap::new();
    let mut observed: BTreeMap<Physical, Vec<(i64, bool)>> = BTreeMap::new();
    for (row, fact) in rows.iter().zip(facts) {
        fact.parsed.as_ref().map_err(Clone::clone)?;
        let Some(scope) = fact.scope.clone() else {
            continue;
        };
        let confirmed = fact.explicit.unwrap_or_else(|| truth(&row["confirmed_at"]));
        *legacy.entry(scope.clone()).or_insert(false) |= confirmed;
        // Raw snapshot v1 never contains Python's captured dataclass. Recompute
        // its closed provenance from the raw hashes and chronology instead.
        if !evidence::current_observation(row).map_err(|e| e.code.to_owned())? {
            continue;
        }
        let Some(confirmed) = fact.explicit else {
            continue;
        };
        let Some(timestamp) =
            time::instant(&row["observation_observed_at"]).map_err(|e| e.code.to_owned())?
        else {
            continue;
        };
        observed
            .entry(scope)
            .or_default()
            .push((timestamp, confirmed));
    }
    for (scope, facts) in observed {
        let latest = facts
            .iter()
            .map(|(timestamp, _)| *timestamp)
            .max()
            .expect("observation");
        legacy.insert(
            scope,
            facts
                .iter()
                .filter(|(timestamp, _)| *timestamp == latest)
                .all(|(_, confirmed)| *confirmed),
        );
    }
    Ok(legacy)
}

fn canonical_outpoint(value: &Py) -> Option<String> {
    let spelling = stored::text(value);
    let (txid, vout) = spelling.py_strip().rsplit_once(':')?;
    let txid = graph::strict_txid(&json!(txid))?;
    let vout = super::integer(&json!(vout)).ok()?;
    if vout.to_string().starts_with('-') {
        return None;
    }
    Some(format!("{txid}:{vout}"))
}

pub(super) fn conflicts(rows: &[Value], facts: &[Facts]) -> Result<Ids, String> {
    let states = confirmation_states(rows, facts)?;
    let mut row_txid = BTreeMap::new();
    let mut outpoints: BTreeMap<Physical, BTreeSet<Physical>> = BTreeMap::new();
    for (row, fact) in rows.iter().zip(facts) {
        let parsed = fact.parsed.as_ref().map_err(Clone::clone)?;
        let Some(scope) = fact.scope.clone() else {
            continue;
        };
        row_txid.insert(codec::pystr(&row["id"]), scope.clone());
        if let Some(parsed) = parsed {
            for entry in stored::list(stored::get(parsed, "inputs")) {
                if let Some(outpoint) = canonical_outpoint(stored::get(entry, "outpoint")) {
                    outpoints
                        .entry((scope.0.clone(), scope.1.clone(), outpoint))
                        .or_default()
                        .insert(scope.clone());
                }
            }
        }
    }
    let mut losers = BTreeSet::new();
    for txids in outpoints.values() {
        if txids.len() < 2 {
            continue;
        }
        let confirmed: BTreeSet<_> = txids
            .iter()
            .filter(|txid| states.get(*txid) == Some(&true))
            .collect();
        for txid in txids {
            if confirmed.len() != 1 || !confirmed.contains(txid) {
                losers.insert(txid.clone());
            }
        }
    }
    Ok(row_txid
        .into_iter()
        .filter_map(|(id, txid)| losers.contains(&txid).then_some(id))
        .collect())
}

pub(super) fn pending(rows: &[Value], facts: &[Facts]) -> Result<Ids, String> {
    let states = confirmation_states(rows, facts)?;
    let mut row_txid = BTreeMap::new();
    let mut explicit_txids = BTreeSet::new();
    for (row, fact) in rows.iter().zip(facts) {
        // The stored tx wrapper is accepted by both explicit-state detection
        // and confirmation chronology.
        if fact.explicit.is_none() {
            continue;
        }
        fact.parsed.as_ref().map_err(Clone::clone)?;
        if let Some(scope) = fact.scope.clone() {
            row_txid.insert(codec::pystr(&row["id"]), scope.clone());
            explicit_txids.insert(scope);
        }
    }
    let pending: BTreeSet<_> = states
        .into_iter()
        .filter_map(|(scope, confirmed)| {
            (explicit_txids.contains(&scope) && !confirmed).then_some(scope)
        })
        .collect();
    for (row, fact) in rows.iter().zip(facts) {
        fact.parsed.as_ref().map_err(Clone::clone)?;
        if let Some(scope) = fact.scope.clone() {
            if pending.contains(&scope) {
                row_txid.insert(codec::pystr(&row["id"]), scope);
            }
        }
    }
    Ok(row_txid
        .into_iter()
        .filter_map(|(id, txid)| pending.contains(&txid).then_some(id))
        .collect())
}

pub(super) fn fanout(rows: &[Value], covered: &Ids) -> Result<Ids, String> {
    let mut groups: Vec<(graph::ownership::TransferScope, Vec<&Value>)> = Vec::new();
    let mut index = std::collections::HashMap::new();
    for row in rows {
        if SYNTHETIC
            .iter()
            .any(|p| codec::pystr(&row["id"]).starts_with(p))
        {
            continue;
        }
        let Some(scope) = graph::onchain_scope(row) else {
            continue;
        };
        codec::push_grouped(&mut groups, &mut index, scope, row);
    }
    let mut held = Ids::new();
    for (_, group) in groups {
        let mut outs = Vec::new();
        let mut ins = Vec::new();
        for (direction, legs) in [("outbound", &mut outs), ("inbound", &mut ins)] {
            for row in &group {
                if row["direction"] == direction && positive(row)? {
                    legs.push(*row);
                }
            }
        }
        if outs.is_empty() || ins.is_empty() || outs.len() == 1 && ins.len() == 1 {
            continue;
        }
        if !outs.iter().any(|out| {
            ins.iter().any(|incoming| {
                codec::pystr(&out["wallet_id"]) != codec::pystr(&incoming["wallet_id"])
            })
        }) {
            continue;
        }
        let positive_ids: Ids = outs
            .iter()
            .chain(&ins)
            .map(|row| codec::pystr(&row["id"]))
            .collect();
        if positive_ids.is_subset(covered) {
            continue;
        }
        let mut graph_bearing = false;
        for row in outs.iter().chain(&ins) {
            if stored::parse(&row["raw_json"])?.is_some() {
                graph_bearing = true;
                break;
            }
        }
        if graph_bearing && positive_ids.is_disjoint(covered) {
            continue;
        }
        let handled = group
            .iter()
            .any(|row| covered.contains(&codec::pystr(&row["id"])));
        let uncovered_source = outs
            .iter()
            .any(|row| !covered.contains(&codec::pystr(&row["id"])));
        let mut zero_outs = false;
        for row in &group {
            if row["direction"] == "outbound" && !positive(row)? {
                zero_outs = true;
                break;
            }
        }
        if handled && zero_outs && !uncovered_source {
            continue;
        }
        held.extend(group.iter().map(|row| codec::pystr(&row["id"])));
    }
    Ok(held)
}

pub(super) fn unscoped(rows: &[Value]) -> Result<Ids, String> {
    let mut groups: Vec<((String, String), Vec<&Value>)> = Vec::new();
    let mut positions: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();
    for row in rows {
        let id = codec::text(&row["id"]);
        if id.starts_with("custody-tax:")
            || SYNTHETIC.iter().any(|p| id.starts_with(p))
            || graph::onchain_scope(row).is_some()
        {
            continue;
        }
        let external = codec::text(&row["external_id"]);
        let external = external.py_strip();
        if external.is_empty() {
            continue;
        }
        let external = graph::strict_txid(&json!(external)).unwrap_or_else(|| external.to_owned());
        let key = (external, codec::text(&row["asset"]).py_strip().py_upper());
        // Groups keep first-occurrence order, as a linear search kept them.
        if let Some(&at) = positions.get(&key) {
            groups[at].1.push(row);
        } else {
            positions.insert(key.clone(), groups.len());
            groups.push((key, vec![row]));
        }
    }
    let mut held = Ids::new();
    for (_, group) in groups {
        let mut outs = Vec::new();
        let mut ins = Vec::new();
        for (direction, legs) in [("outbound", &mut outs), ("inbound", &mut ins)] {
            for row in &group {
                if row["direction"] == direction && positive(row)? {
                    legs.push(*row);
                }
            }
        }
        if outs.iter().any(|out| {
            ins.iter()
                .any(|incoming| out["wallet_id"] != incoming["wallet_id"])
        }) {
            held.extend(group.iter().map(|row| codec::pystr(&row["id"])));
        }
    }
    Ok(held)
}

pub(super) fn anchors(pair: &Value) -> Ids {
    [anchor(&pair["out"]), anchor(&pair["in"])]
        .into_iter()
        .filter(|id| !id.is_empty())
        .collect()
}
pub(super) fn without_held(pairs: &[Value], held: &Ids) -> Vec<Value> {
    let groups: Ids = pairs
        .iter()
        .filter(|pair| {
            !pair["group_id"].is_null()
                && pair["group_id"] != ""
                && !anchors(pair).is_disjoint(held)
        })
        .map(|pair| codec::pystr(&pair["group_id"]))
        .collect();
    pairs
        .iter()
        .filter(|pair| {
            anchors(pair).is_disjoint(held) && !groups.contains(&codec::text(&pair["group_id"]))
        })
        .cloned()
        .collect()
}

#[derive(Serialize)]
pub(super) struct Holds {
    pub blocked_ids: Ids,
    pub pending_ids: Ids,
    pub quarantines: Vec<Value>,
    pub transient_ids: Ids,
}
pub(super) struct Inputs<'a> {
    pub rows: &'a [Value],
    pub derivation_rows: &'a [Value],
    pub pairs: &'a [Value],
    pub wallets: &'a BTreeMap<String, Value>,
    pub has_chain_events: bool,
    pub excluded: &'a Ids,
    pub blocked: &'a Ids,
    pub resolved: &'a Ids,
}

pub(super) fn compute(input: Inputs<'_>) -> Result<Holds, String> {
    let facts = facts(input.rows);
    let conflict_ids = if input.has_chain_events {
        conflicts(input.rows, &facts)?
    } else {
        Ids::new()
    };
    let pending_ids = pending(input.rows, &facts)?;
    let by_id: BTreeMap<_, _> = input
        .rows
        .iter()
        .map(|row| (codec::text(&row["id"]), row))
        .collect();
    let mut covered: Ids = input
        .excluded
        .iter()
        .chain(input.blocked)
        .chain(input.resolved)
        .chain(&conflict_ids)
        .chain(&pending_ids)
        .cloned()
        .collect();
    covered.extend(input.pairs.iter().flat_map(anchors));
    let fanout_ids = if input.has_chain_events {
        fanout(input.derivation_rows, &covered)?
    } else {
        Ids::new()
    };
    let unscoped_ids: Ids = unscoped(input.rows)?
        .difference(&covered)
        .filter(|id| !fanout_ids.contains(*id))
        .cloned()
        .collect();
    let mut reasons = BTreeMap::new();
    for (ids, reason) in [
        (&conflict_ids, "conflicting_spend"),
        (&pending_ids, "pending_onchain_confirmation"),
        (&fanout_ids, "owned_fanout_unresolved"),
        (&unscoped_ids, "unscoped_transfer_review"),
    ] {
        for id in ids {
            if input.excluded.contains(id) || input.blocked.contains(id) || !by_id.contains_key(id)
            {
                continue;
            }
            reasons.entry(id.clone()).or_insert(reason);
        }
    }
    let mut dependency_causes: BTreeMap<String, Ids> = BTreeMap::new();
    while !reasons.is_empty() {
        let held: Ids = reasons.keys().cloned().collect();
        let kept = without_held(input.pairs, &held);
        // Pair values are immutable. Membership by value is equivalent to
        // Python object identity: equal pairs have equal anchors and group.
        let dropped: Vec<_> = input
            .pairs
            .iter()
            .filter(|pair| !kept.contains(pair))
            .collect();
        let mut groups: BTreeMap<String, Ids> = BTreeMap::new();
        for pair in &dropped {
            let group = codec::text(&pair["group_id"]);
            if !group.is_empty() {
                groups
                    .entry(group)
                    .or_default()
                    .extend(anchors(pair).intersection(&held).cloned());
            }
        }
        let mut added = false;
        for pair in dropped {
            let anchors = anchors(pair);
            let direct: Ids = anchors.intersection(&held).cloned().collect();
            let causes = if direct.is_empty() {
                groups
                    .get(&codec::text(&pair["group_id"]))
                    .cloned()
                    .unwrap_or_default()
            } else {
                direct
            };
            for id in anchors.difference(&held) {
                if input.excluded.contains(id)
                    || input.blocked.contains(id)
                    || !by_id.contains_key(id)
                {
                    continue;
                }
                reasons.insert(id.clone(), "transfer_pair_dependency_blocked");
                dependency_causes
                    .entry(id.clone())
                    .or_default()
                    .extend(causes.iter().cloned());
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    let mut quarantines = Vec::new();
    for (id, reason) in &reasons {
        let row = by_id[id];
        let wallet_id = codec::text(&row["wallet_id"]);
        let wallet = input.wallets.get(&wallet_id).unwrap_or(&Value::Null);
        let label = wallet
            .get("label")
            .cloned()
            .unwrap_or_else(|| json!(wallet_id));
        let mut detail = json!({"wallet":label,"asset":codec::text(&row["asset"]).py_upper(),"direction":row["direction"],"external_id":codec::text(&row["external_id"])});
        let required = match *reason {
            "pending_onchain_confirmation" => Some("confirmed_chain_history"),
            "owned_fanout_unresolved" | "transfer_pair_dependency_blocked" => {
                Some("complete_transfer_component")
            }
            "unscoped_transfer_review" => Some("physical_transfer_scope_or_custody_component"),
            _ => None,
        };
        if let Some(required) = required {
            detail["required_for"] = json!(required);
        }
        if let Some(causes) = dependency_causes.get(id) {
            detail["blocked_by_transaction_ids"] = json!(causes);
        }
        quarantines.push(json!({"transaction_id":id,"workspace_id":row["workspace_id"],"profile_id":row["profile_id"],"reason":reason,"detail_json":pairs::detail_json(&detail)?}));
    }
    let mut transient_ids: Ids = reasons
        .iter()
        .filter_map(|(id, reason)| {
            matches!(
                *reason,
                "conflicting_spend" | "pending_onchain_confirmation"
            )
            .then_some(id.clone())
        })
        .collect();
    loop {
        let grown: Ids = dependency_causes
            .iter()
            .filter(|(id, causes)| {
                !transient_ids.contains(*id)
                    && !causes.is_empty()
                    && causes.is_subset(&transient_ids)
            })
            .map(|(id, _)| id.clone())
            .collect();
        if grown.is_empty() {
            break;
        }
        transient_ids.extend(grown);
    }
    Ok(Holds {
        blocked_ids: reasons.into_keys().collect(),
        pending_ids,
        quarantines,
        transient_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpretation::array;
    fn check(op: &str) {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_safety_helpers.json"
        ))
        .unwrap();
        let mut count = 0;
        for vector in vectors.iter().filter(|v| v["op"] == op) {
            count += 1;
            let ids = |key: &str| -> Ids { array(&vector[key]).iter().map(codec::text).collect() };
            let actual = (|| -> Result<Value, String> {
                let rows = array(&vector["rows"]);
                Ok(match op {
                    "conflict_scope" => {
                        let parsed = if vector["parsed"].is_null() { None } else { Some(codec::from_json(&vector["parsed"]).unwrap()) };
                        json!(conflict_scope(&vector["row"], parsed.as_ref()).map(|(c,n,t)| (c,n,t,"conflict-only")))
                    }
                    "outpoint" => json!(canonical_outpoint(&codec::from_json(&vector["value"]).unwrap())),
                    "detectors" => json!({"conflicts":conflicts(rows, &facts(rows))?,"pending":pending(rows, &facts(rows))?,"fanout":fanout(rows,&ids("covered"))?,"unscoped":unscoped(rows)?}),
                    "filter" => json!(without_held(array(&vector["pairs"]), &ids("held"))),
                    "holds" => json!(compute(Inputs { rows, derivation_rows: array(&vector["derivation_rows"]), pairs: array(&vector["pairs"]), wallets: &serde_json::from_value(vector["wallets"].clone()).unwrap(), has_chain_events: vector["has_chain_events"].as_bool().unwrap(), excluded: &ids("excluded"), blocked: &ids("blocked"), resolved: &ids("resolved") })?),
                    _ => unreachable!(),
                })
            })().unwrap_or_else(|error| json!({"error":error}));
            assert_eq!(actual, vector["expected"], "{}", vector["case"]);
        }
        assert!(count > 0);
    }
    #[test]
    fn raw_observation_scope_vectors() {
        check("conflict_scope");
        check("outpoint");
    }
    #[test]
    fn raw_observation_detector_vectors() {
        check("detectors");
    }
    #[test]
    fn raw_observation_hold_vectors() {
        check("holds");
    }
    #[test]
    fn held_pair_filter_vectors() {
        check("filter");
    }
}

//! Bitcoin-family transfer scope and Bitcoin evidence fee/receipt normalization.
use crate::codec::{self, int, object, text, truth};
use crate::evidence::{self, EventKey};
use crate::python_text::PythonText;
use crate::snapshot::Snapshot;
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

mod ownership;
mod stored;

fn graph_integer(value: &Value) -> BigInt {
    BigInt::from_str(&value.to_string()).expect("converted graph integer")
}
fn integer_value(value: &BigInt) -> Value {
    serde_json::from_str(&value.to_string()).expect("computed integer")
}
fn strict_txid(v: &Value) -> Option<String> {
    let s = text(v).py_strip().py_lower();
    if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(s)
    } else {
        None
    }
}
fn merge(items: &[Value]) -> Option<Value> {
    let items = items
        .iter()
        .map(codec::from_json)
        .collect::<Result<Vec<_>, _>>()
        .expect("JSON graphs");
    stored::merge(&items).as_ref().map(stored::project)
}
fn parse(raw: &Value) -> Result<Option<Value>, String> {
    Ok(stored::parse(raw)?.as_ref().map(stored::project))
}
// Transfer identity predicates stringify nonfinite Python scalars. Preserve
// their presence instead of turning them into null during outer JSON conversion.
fn transfer_value(value: &Value) -> Value {
    fn decoded(value: &kassiber_pyjson::Value) -> Value {
        use kassiber_pyjson::Value as Py;
        match value {
            Py::Float(n) if !n.is_finite() => json!(kassiber_pyjson::float_repr(*n)),
            Py::List(values) => json!(values.iter().map(decoded).collect::<Vec<_>>()),
            Py::Dict(values) => Value::Object(
                values
                    .iter()
                    .filter_map(|(k, v)| {
                        if let kassiber_pyjson::Key::Str(k) = k {
                            Some((k.as_str()?.to_owned(), decoded(v)))
                        } else {
                            None
                        }
                    })
                    .collect(),
            ),
            _ => codec::to_json(value),
        }
    }
    value
        .as_str()
        .and_then(|s| codec::loads(s).ok())
        .as_ref()
        .map(decoded)
        .unwrap_or_else(|| value.clone())
}
pub(crate) fn transfer_object(value: &Value) -> Value {
    let value = transfer_value(value);
    if value.is_object() {
        value
    } else {
        json!({})
    }
}

/// Physical Bitcoin-family grouping; contradictory graph/configuration facts refuse it.
pub(crate) fn onchain_scope(row: &Value) -> Option<(String, String, String, String)> {
    if [
        "custody-tax:",
        "cross-split:",
        "direct-payout:",
        "multi-consol:",
        "owned-derive:",
        "recorded-fanout:",
    ]
    .iter()
    .any(|p| text(&row["id"]).starts_with(p))
    {
        return None;
    }
    let raw = transfer_object(&row["raw_json"]);
    let config = transfer_object(if truth(&row["config_json"]) {
        &row["config_json"]
    } else {
        &row["wallet_config_json"]
    });
    let decoded = transfer_value(&row["raw_json"]);
    let mut maps = vec![raw.clone()];
    maps.extend(
        decoded
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v.is_object())
            .cloned(),
    );
    for key in ["tx", "ownership_graph"] {
        if raw[key].is_object() {
            maps.push(raw[key].clone());
        }
    }
    let mut txids = BTreeSet::new();
    let mut chains = BTreeSet::new();
    let mut networks = BTreeSet::new();
    for payload in &maps {
        if !text(&payload["txid"]).py_strip().is_empty() {
            txids.insert(strict_txid(&payload["txid"])?);
        }
    }
    if txids.len() > 1 {
        return None;
    }
    let external = strict_txid(&row["external_id"]);
    let graph = txids.into_iter().next();
    if graph.is_some() && external.is_some() && graph != external {
        return None;
    }
    if graph.is_none() && external.is_some() {
        let kind = if truth(&row["external_id_kind"]) {
            text(&row["external_id_kind"])
        } else {
            text(&raw["external_id_kind"])
        };
        let kind = kind.py_strip().py_lower();
        let typed = if !kind.is_empty() {
            ["txid", "transaction_hash", "onchain_txid"].contains(&kind.as_str())
        } else {
            maps.iter()
                .filter_map(Value::as_object)
                .flat_map(|m| m.iter())
                .any(|(k, v)| {
                    let name: String = k
                        .py_lower()
                        .chars()
                        .filter(|c| crate::python_text::isalnum(*c))
                        .collect();
                    [
                        "txid",
                        "txhash",
                        "transactionid",
                        "transactionhash",
                        "onchaintxid",
                        "sendtxid",
                        "lockuptxid",
                        "receivetxid",
                        "claimtxid",
                        "refundtxid",
                    ]
                    .contains(&name.as_str())
                        && strict_txid(v) == external
                })
        };
        if !typed {
            return None;
        }
    }
    let txid = graph.or(external)?;
    let mut kind = text(&row["wallet_kind"])
        .py_strip()
        .py_lower()
        .replace('_', "-");
    if ["core-ln", "core-lightning"].contains(&kind.as_str()) {
        kind = "coreln".into();
    }
    if evidence::lightning(&kind)
        && !maps
            .iter()
            .any(|v| v["vin"].is_array() && v["vout"].is_array())
    {
        return None;
    }
    for payload in maps.iter().chain(std::iter::once(&config)) {
        let c = text(&payload["chain"]);
        if !c.py_strip().is_empty() {
            let c = evidence::chain(&c.py_strip().py_lower()).ok()?;
            if c == "lightning" {
                return None;
            }
            chains.insert(c);
        }
    }
    if chains.len() > 1 {
        return None;
    }
    let asset = text(&row["asset"])
        .py_strip()
        .py_upper()
        .replace("L-BTC", "LBTC");
    let chain = chains.into_iter().next().or_else(|| match asset.as_str() {
        "BTC" => Some("bitcoin".into()),
        "LBTC" => Some("liquid".into()),
        _ => None,
    })?;
    if (asset == "BTC" && chain != "bitcoin") || (asset == "LBTC" && chain != "liquid") {
        return None;
    }
    for payload in maps.iter().chain(std::iter::once(&config)) {
        if !text(&payload["network"]).py_strip().is_empty() {
            networks.insert(evidence::network(&chain, &payload["network"]).ok()?);
        }
    }
    if networks.len() > 1 {
        return None;
    }
    let network = networks
        .into_iter()
        .next()
        .or_else(|| (chain == "bitcoin").then(|| "main".into()))?;
    let identity = if chain == "liquid" {
        let mut ids = BTreeSet::new();
        let mut displays = BTreeSet::new();
        for payload in &maps {
            for component in
                std::iter::once(payload).chain(payload.get("component").filter(|v| v.is_object()))
            {
                if !component["asset_id"].is_null() && component["asset_id"] != "" {
                    ids.insert(strict_txid(&component["asset_id"])?);
                }
                if !component["asset"].is_null() && component["asset"] != "" {
                    displays.insert(
                        text(&component["asset"])
                            .py_strip()
                            .py_upper()
                            .replace("L-BTC", "LBTC"),
                    );
                }
            }
        }
        if ids.len() != 1 || displays.len() > 1 || displays.iter().any(|v| v != &asset) {
            return None;
        }
        let id = ids.into_iter().next()?;
        if asset != "LBTC" && asset != id.py_upper() {
            return None;
        }
        id
    } else {
        asset
    };
    Some((chain, network, txid, identity))
}

fn ownership_scope(
    row: &Value,
    graph: &Value,
    snapshot: &Snapshot,
) -> Result<Option<ownership::TransferScope>, String> {
    ownership::scope(
        row,
        Some(graph),
        ownership::Index(snapshot.owned_index.as_ref()),
    )
}
fn script_owners(
    snapshot: &Snapshot,
    script: &Value,
    scope: (&str, &str),
) -> Result<BTreeSet<String>, String> {
    ownership::Index(snapshot.owned_index.as_ref())
        .script_owners(script, &(scope.0.into(), scope.1.into()))
}
fn input_owners(
    snapshot: &Snapshot,
    entry: &Value,
    scope: (&str, &str),
) -> Result<BTreeSet<String>, String> {
    ownership::Index(snapshot.owned_index.as_ref())
        .input_owners(entry, &(scope.0.into(), scope.1.into()))
}
/// Test-only isolated prerequisite projection, using the same raw snapshot facts.
/// This does not expose an implemented accounting stage.
pub(crate) fn prerequisites(snapshot: &Snapshot) -> Result<Value, String> {
    let rows: Vec<Value> = snapshot.transactions.iter().map(|row| json!(row)).collect();
    let index = ownership::Index(snapshot.owned_index.as_ref());
    let mut graphs = Vec::new();
    let mut scopes = Vec::new();
    let mut lookups = Vec::new();
    for row in &rows {
        let parsed = parse(&row["raw_json"])?;
        let scope = ownership::scope(row, parsed.as_ref(), index)?;
        let mut entries = Vec::new();
        if let Some(graph) = &parsed {
            for entry in graph["inputs"].as_array().unwrap() {
                let outpoint = text(&entry["outpoint"]);
                let txid = outpoint.split(':').next().unwrap_or("");
                let physical = scope.as_ref().map(|s| (s.0.clone(), s.1.clone()));
                entries.push(json!({"outpoint":index.outpoint(&entry["outpoint"],physical.as_ref()),"txid_wallets":index.txid(&json!(txid),physical.as_ref())}));
            }
        }
        graphs.push(parsed);
        scopes.push(scope);
        lookups.push(entries);
    }
    Ok(
        json!({"graphs":graphs,"scopes":scopes,"index_lookups":lookups,"profile_parsed":ownership::profile_parsed(&rows,index)?}),
    )
}
pub fn native_zero_delta_ids(
    rows: &[Value],
    snapshot: &Snapshot,
) -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    if snapshot.owned_index.is_none() {
        return Ok(ids);
    }
    for row in rows {
        if row["direction"] != "outbound" || int(&row["amount"]) != 0 || int(&row["fee"]) != 0 {
            continue;
        }
        if !evidence::current_observation(row)? {
            continue;
        }
        let Some(g) = parse(&row["raw_json"])? else {
            continue;
        };
        let Some(scope) = ownership_scope(row, &g, snapshot)? else {
            continue;
        };
        if scope.0 != "bitcoin" || scope.3 != "BTC" {
            continue;
        }
        if truth(&g["evidence_conflicts"]) || !truth(&g["inputs"]) || !truth(&g["outputs"]) {
            continue;
        }
        let mut valid = true;
        let mut delta = 0i128;
        let mut total = 0i128;
        let wallet = text(&row["wallet_id"]);
        for (sign, side) in [(1i128, "inputs"), (-1i128, "outputs")] {
            for leg in g[side].as_array().unwrap() {
                let Some(value) = leg["value_sats"].as_i64() else {
                    valid = false;
                    break;
                };
                total += sign * i128::from(value) * 1000;
                if !truth(&leg["script"]) || (sign == 1 && !truth(&leg["outpoint"])) {
                    valid = false;
                    break;
                }
                let owners = if sign == 1 {
                    input_owners(snapshot, leg, (&scope.0, &scope.1))?
                } else {
                    script_owners(snapshot, &leg["script"], (&scope.0, &scope.1))?
                };
                if owners.contains(&wallet) {
                    if owners.len() != 1 {
                        valid = false;
                        break;
                    }
                    delta += sign * i128::from(value) * 1000;
                }
            }
        }
        let raw = object(&row["raw_json"]);
        let payload = if raw["tx"].is_object() {
            &raw["tx"]
        } else {
            &raw
        };
        if !payload["fee"].is_null()
            && payload["fee"].as_i64().map(i128::from) != Some(total / 1000)
        {
            valid = false;
        }
        if valid && total >= 0 && delta == 0 {
            ids.insert(text(&row["id"]));
        }
    }
    Ok(ids)
}
pub fn normalize_owned_fees(rows: &mut [Value]) -> Result<(), String> {
    let mut groups: BTreeMap<EventKey, Vec<usize>> = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        if row["_kassiber_verified_chain_observation"].is_object() {
            let key = evidence::event_key(row)?;
            if key.chain == "bitcoin" && key.native_namespace == "chain" {
                groups.entry(key).or_default().push(i);
            }
        }
    }
    for group in groups.values() {
        let outbound: Vec<usize> = group
            .iter()
            .copied()
            .filter(|i| rows[*i]["direction"] == "outbound" && int(&rows[*i]["amount"]) > 0)
            .collect();
        if outbound.is_empty()
            || outbound.iter().any(|i| {
                rows[*i]["observation_fee_attribution"] != "implicit_wallet_delta"
                    || int(&rows[*i]["fee"]) != 0
                    || truth(&rows[*i]["amount_includes_fee"])
            })
        {
            continue;
        }
        if group
            .iter()
            .map(|i| text(&rows[*i]["wallet_id"]))
            .collect::<BTreeSet<_>>()
            .len()
            != group.len()
        {
            continue;
        }
        let raw: Vec<Value> = group
            .iter()
            .map(|i| object(&rows[*i]["raw_json"]))
            .collect();
        let mut scripts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (i, raw) in group.iter().zip(&raw) {
            for script in raw["observer_owned_scripts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                scripts
                    .entry(script.into())
                    .or_default()
                    .insert(text(&rows[*i]["wallet_id"]));
            }
        }
        let Some(graphs): Option<Vec<Value>> = raw.iter().map(parse).collect::<Result<_, _>>()?
        else {
            continue;
        };
        let shapes: BTreeSet<(Vec<String>, Vec<i64>)> = graphs
            .iter()
            .map(|g| {
                let mut ins: Vec<String> = g["inputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| text(&v["outpoint"]))
                    .collect();
                ins.sort();
                let mut outs: Vec<i64> = g["outputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| int(&v["n"]))
                    .collect();
                outs.sort();
                (ins, outs)
            })
            .collect();
        let Some(g) = merge(&graphs) else {
            continue;
        };
        if shapes.len() != 1
            || truth(&g["evidence_conflicts"])
            || !truth(&g["inputs"])
            || !truth(&g["outputs"])
        {
            continue;
        }
        let mut input_totals: BTreeMap<String, i128> = BTreeMap::new();
        let mut output_totals: BTreeMap<String, i128> = BTreeMap::new();
        let mut total_in = 0i128;
        let mut total_out = 0i128;
        let mut valid = true;
        for vin in g["inputs"].as_array().unwrap() {
            let owners = scripts.get(&text(&vin["script"]));
            let Some(value) = vin["value_sats"].as_i64() else {
                valid = false;
                break;
            };
            if !truth(&vin["outpoint"]) || owners.is_none_or(|o| o.len() != 1) {
                valid = false;
                break;
            }
            let owner = owners.unwrap().first().unwrap();
            *input_totals.entry(owner.clone()).or_default() += i128::from(value);
            total_in += i128::from(value);
        }
        for vout in g["outputs"].as_array().unwrap() {
            let owners = scripts.get(&text(&vout["script"]));
            let Some(value) = vout["value_sats"].as_i64() else {
                valid = false;
                break;
            };
            if owners.is_some_and(|o| o.len() > 1) {
                valid = false;
                break;
            }
            total_out += i128::from(value);
            if let Some(owner) = owners.and_then(|o| o.first()) {
                *output_totals.entry(owner.clone()).or_default() += i128::from(value);
            }
        }
        let fee = (total_in - total_out) * 1000;
        if !valid
            || fee < 0
            || raw.iter().any(|r| {
                !r["fee"].is_null()
                    && r["fee"].as_i64().map(i128::from) != Some(total_in - total_out)
            })
        {
            continue;
        }
        for i in group {
            let row = &rows[*i];
            let wallet = text(&row["wallet_id"]);
            let delta = (input_totals.get(&wallet).unwrap_or(&0)
                - output_totals.get(&wallet).unwrap_or(&0))
                * 1000;
            let signed = i128::from(int(&row["amount"]))
                * if row["direction"] == "outbound" {
                    1
                } else {
                    -1
                };
            if delta != signed || int(&row["fee"]) != 0 {
                valid = false;
            }
        }
        if !valid
            || outbound
                .iter()
                .map(|i| i128::from(int(&rows[*i]["amount"])))
                .sum::<i128>()
                < fee
        {
            continue;
        }
        let mut order = outbound.clone();
        order.sort_by_key(|i| text(&rows[*i]["id"]));
        let bearer = order
            .iter()
            .copied()
            .find(|i| i128::from(int(&rows[*i]["amount"])) > fee);
        let mut remaining = fee;
        for i in order {
            let allocated = if let Some(b) = bearer {
                if i == b {
                    fee
                } else {
                    0
                }
            } else {
                let v = remaining.min(i128::from(int(&rows[i]["amount"])));
                remaining -= v;
                v
            };
            let amount = i128::from(int(&rows[i]["amount"])) - allocated;
            rows[i]["amount"] =
                serde_json::from_str(&amount.to_string()).map_err(|e| e.to_string())?;
            rows[i]["fee"] =
                serde_json::from_str(&allocated.to_string()).map_err(|e| e.to_string())?;
            rows[i]["observation_fee_attribution"] = json!("exact");
            rows[i]["custody_native_fee_normalized"] = json!(true);
        }
    }
    Ok(())
}

type ChainScope = (String, String, String, String);

pub fn reconcile_receipts(rows: &[Value], snapshot: &Snapshot) -> Result<Value, String> {
    let mut grouped: Vec<(ChainScope, Vec<Value>)> = Vec::new();
    for row in rows {
        if let Some(scope) = onchain_scope(row) {
            if let Some((_, items)) = grouped.iter_mut().find(|(s, _)| s == &scope) {
                items.push(row.clone());
            } else {
                grouped.push((scope, vec![row.clone()]));
            }
        }
    }
    let mut result = serde_json::Map::new();
    for (scope, group) in grouped {
        if scope.3 != "BTC" {
            continue;
        }
        let inbound: BTreeSet<String> = group
            .iter()
            .filter(|r| r["direction"] == "inbound")
            .map(|r| text(&r["wallet_id"]))
            .collect();
        if inbound.len() <= 1 {
            continue;
        }
        let wallets: BTreeSet<String> = group.iter().map(|r| text(&r["wallet_id"])).collect();
        let parsed: Vec<Option<Value>> = group
            .iter()
            .map(|r| parse(&r["raw_json"]))
            .collect::<Result<_, _>>()?;
        let outputs: Vec<Value> = parsed
            .iter()
            .flatten()
            .next()
            .and_then(|g| g["outputs"].as_array())
            .cloned()
            .unwrap_or_default();
        let owners: Vec<Vec<Value>> = outputs
            .iter()
            .map(|o| {
                snapshot
                    .receipt_sources
                    .iter()
                    .filter(|s| {
                        s.get("chain") == Some(&json!(scope.0))
                            && s.get("network") == Some(&json!(scope.1))
                            && s.get("script_pubkey") == Some(&o["script"])
                            && wallets.contains(&text(s.get("wallet_id").unwrap_or(&Value::Null)))
                    })
                    .cloned()
                    .map(Value::Object)
                    .collect()
            })
            .collect();
        let overlap = owners.iter().any(|sources| {
            sources
                .iter()
                .map(|s| text(&s["wallet_id"]))
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        });
        let valued = !outputs.is_empty() && outputs.iter().all(|o| !o["value_sats"].is_null());
        let observed: BigInt = group
            .iter()
            .filter(|r| r["direction"] == "inbound")
            .try_fold(BigInt::from(0), |sum, r| {
                Ok::<_, String>(sum + graph_integer(&codec::integer_or_zero(&r["amount"])?))
            })?;
        let covered: BigInt = outputs
            .iter()
            .zip(&owners)
            .filter(|(_, s)| valued && !s.is_empty())
            .map(|(o, _)| graph_integer(&o["value_sats"]) * 1000)
            .sum();
        let kinds: BTreeSet<String> = group.iter().map(|r| text(&r["wallet_kind"])).collect();
        let historical = kinds.contains("address")
            && ["descriptor", "xpub", "silent-payment"]
                .iter()
                .any(|k| kinds.contains(*k))
            && valued
            && covered > BigInt::from(0)
            && observed > covered;
        if !overlap && !historical {
            continue;
        }
        for row in &group {
            result.insert(text(&row["id"]), Value::Null);
        }
        if group
            .iter()
            .any(|r| r["direction"] != "inbound" || int(&r["fee"]) != 0)
            || wallets.len() != group.len()
            || parsed.iter().any(Option::is_none)
            || snapshot.owned_index.is_none()
        {
            continue;
        }
        let mut authoritative = true;
        for row in &group {
            authoritative &= truth(&evidence::authority(row)?["authoritative"]);
        }
        if !authoritative {
            continue;
        }
        let first = parsed[0].as_ref().unwrap();
        if parsed.iter().flatten().any(|g| {
            g["inputs"] != first["inputs"]
                || g["outputs"] != first["outputs"]
                || g["evidence_conflicts"] != first["evidence_conflicts"]
        }) || truth(&first["evidence_conflicts"])
            || outputs.iter().any(|o| o["value_sats"].is_null())
            || !truth(&first["inputs"])
        {
            continue;
        }
        let mut inputs_owned_or_incomplete = false;
        for e in first["inputs"].as_array().unwrap() {
            if !truth(&e["script"])
                || !truth(&e["outpoint"])
                || e["value_sats"].is_null()
                || !input_owners(snapshot, e, (&scope.0, &scope.1))?.is_empty()
            {
                inputs_owned_or_incomplete = true;
                break;
            }
        }
        if inputs_owned_or_incomplete {
            continue;
        }
        let mut full: BTreeMap<String, BigInt> = wallets
            .iter()
            .cloned()
            .map(|w| (w, BigInt::from(0)))
            .collect();
        let mut canonical = full.clone();
        let mut assignments = Vec::new();
        for (output, sources) in outputs.iter().zip(&owners) {
            if sources.is_empty() {
                continue;
            }
            let amount = graph_integer(&output["value_sats"]) * 1000;
            let mut by_wallet: BTreeMap<String, Value> = BTreeMap::new();
            for source in sources {
                let wallet = text(&source["wallet_id"]);
                let priority = match text(&source["source"]).as_str() {
                    "inventory" | "descriptor_config" => 0,
                    "address_list" => 1,
                    "sync_target" => 2,
                    _ => 99,
                };
                let entry=by_wallet.entry(wallet).or_insert_with(||json!({"id":source["wallet_id"],"label":source["wallet_label"],"kind":source["wallet_kind"],"deprecated":source["deprecated"],"active_transaction_count":source["active_transaction_count"],"priority":99}));
                entry["priority"] = json!(int(&entry["priority"]).min(priority));
            }
            for w in by_wallet.keys() {
                *full.get_mut(w).unwrap() += &amount;
            }
            let selected = by_wallet
                .values()
                .min_by_key(|v| {
                    (
                        truth(&v["deprecated"]),
                        !["descriptor", "xpub", "silent-payment"]
                            .contains(&text(&v["kind"]).as_str()),
                        int(&v["priority"]),
                        !truth(&v["active_transaction_count"]),
                        text(&v["label"]),
                        text(&v["id"]),
                    )
                })
                .unwrap();
            let owner = text(&selected["id"]);
            *canonical.get_mut(&owner).unwrap() += &amount;
            assignments.push(
                json!({"vout":output["n"],"wallet_id":owner,"amount_msat":integer_value(&amount)}),
            );
        }
        if group.iter().try_fold(false, |bad, r| {
            let w = text(&r["wallet_id"]);
            Ok::<_, String>(
                bad || canonical[&w] <= BigInt::from(0)
                    || ![&full[&w], &canonical[&w]]
                        .contains(&&graph_integer(&codec::integer_or_zero(&r["amount"])?)),
            )
        })? {
            continue;
        }
        for row in &group {
            result.insert(text(&row["id"]),json!({"schema_version":1,"amount_msat":integer_value(&canonical[&text(&row["wallet_id"])]),"outputs":assignments}));
        }
    }
    Ok(Value::Object(result))
}

#[cfg(test)]
mod semantic_tests {
    use super::*;
    #[test]
    fn cpython_onchain_scopes() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_semantics.json"
        ))
        .unwrap();
        for v in vectors.iter().filter(|v| v.get("scope_row").is_some()) {
            assert_eq!(
                json!(onchain_scope(&v["scope_row"])),
                v["onchain_scope"],
                "{}",
                v["case"]
            );
        }
    }
    #[test]
    fn cpython_graph_indexes() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_semantics.json"
        ))
        .unwrap();
        for v in vectors.iter().filter(|v| v.get("raw").is_some()) {
            assert_eq!(
                parse(&v["raw"]).unwrap().unwrap(),
                v["graph"],
                "{}",
                v["case"]
            );
        }
    }

    fn graph_case(case: &Value) -> Result<Value, String> {
        let index = ownership::Index(case.get("index").filter(|v| !v.is_null()));
        let physical = case["scope"].as_array().map(|v| (text(&v[0]), text(&v[1])));
        Ok(match case["operation"].as_str().unwrap() {
            "parse" => json!(parse(&case["raw"])?),
            "merge" => json!(merge(case["graphs"].as_array().unwrap())),
            "scope" => json!(ownership::scope(&case["row"], Some(&case["graph"]), index)?),
            "parsed_scope" => json!(ownership::parsed_scope(
                &case["graph"],
                index,
                &case["row"]
            )?),
            "profile" => json!(ownership::profile_parsed(
                case["rows"].as_array().unwrap(),
                index
            )?),
            "outpoint" => json!(index.outpoint(&case["value"], physical.as_ref())),
            "txid" => json!(index.txid(&case["value"], physical.as_ref())),
            "input_owners" => {
                json!(index.input_owners(&case["entry"], physical.as_ref().unwrap())?)
            }
            "source_scope" => json!(index.source_scope(
                case["graph"]["inputs"].as_array().unwrap(),
                &text(&case["wallet"])
            )?),
            "normalize" => json!(ownership::normalized(&case["value"][0], &case["value"][1])),
            _ => panic!("unknown graph test operation"),
        })
    }
    fn graph_cases(group: &str) {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_graph_cases.json"
        ))
        .unwrap();
        let mut count = 0;
        for case in vectors.iter().filter(|v| v["group"] == group) {
            let expected = case
                .get("error")
                .map(|e| Err(text(e)))
                .unwrap_or_else(|| Ok(case["expected"].clone()));
            assert_eq!(graph_case(case), expected, "{}", case["case"]);
            count += 1;
        }
        assert!(count > 0);
    }
    #[test]
    fn graph_merge_conflicts() {
        graph_cases("merge");
    }
    #[test]
    fn graph_mapping_values() {
        graph_cases("mapping");
    }
    #[test]
    fn graph_mapping_nonfinite() {
        graph_cases("mapping_nonfinite");
    }
    #[test]
    fn graph_mapping_admission() {
        graph_cases("mapping_admission");
    }
    #[test]
    fn graph_scope_recovery() {
        graph_cases("scope");
    }
    #[test]
    fn graph_index_precedence() {
        graph_cases("index");
    }
    #[test]
    fn graph_profile_merge() {
        graph_cases("profile");
    }
}

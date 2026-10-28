//! Bitcoin-family transfer scope and Bitcoin evidence fee/receipt normalization.
use crate::codec::{self, int, object, text, truth, wide};
use crate::evidence::{self, EventKey};
use crate::python_text::PythonText;
use crate::snapshot::Snapshot;
use kassiber_pydecimal::{Context, Decimal, Rounding};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

fn script(entry: &Value) -> Value {
    for key in ["scriptpubkey", "script_hex"] {
        if !entry[key].is_null() {
            return entry[key].clone();
        }
    }
    let nested = &entry["scriptPubKey"];
    if nested.is_string() {
        return nested.clone();
    }
    if truth(&nested["hex"]) {
        nested["hex"].clone()
    } else {
        nested["scriptpubkey"].clone()
    }
}
fn sats(entry: &Value) -> Option<i64> {
    let explicit = !entry["value_sats"].is_null();
    let value = if explicit {
        &entry["value_sats"]
    } else {
        &entry["value"]
    };
    if value.is_null() || value == "" || value.is_boolean() {
        return None;
    }
    let spelling = codec::pystr(value);
    if explicit
        || value
            .as_number()
            .is_some_and(|n| !n.to_string().contains(['.', 'e', 'E']))
        || value.is_string() && !spelling.contains('.')
    {
        return spelling.py_strip().parse::<i64>().ok().or_else(|| {
            if explicit {
                value.as_f64().filter(|v| v.is_finite()).map(|v| v as i64)
            } else {
                None
            }
        });
    }
    let spelling = if value.is_number() {
        codec::pystr(value)
    } else {
        spelling
    };
    Decimal::from_str(&spelling)
        .ok()?
        .mul(&Decimal::from(100_000_000), &Context::RP2)
        .ok()?
        .to_fixed_places(0, Rounding::HalfEven)
        .ok()?
        .parse()
        .ok()
}
fn strict_txid(v: &Value) -> Option<String> {
    let s = text(v).py_strip().py_lower();
    if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(s)
    } else {
        None
    }
}
fn assets(entry: &Value) -> (Value, Value) {
    let mut id = if !entry["asset_id"].is_null() && entry["asset_id"] != "" {
        json!(text(&entry["asset_id"]).py_strip().py_lower())
    } else {
        Value::Null
    };
    let mut asset = if !entry["asset"].is_null() && entry["asset"] != "" {
        json!(text(&entry["asset"]).py_strip().py_upper())
    } else {
        Value::Null
    };
    if id.is_null() && strict_txid(&asset).is_some() {
        id = json!(text(&asset).py_lower());
        asset = Value::Null;
    }
    (id, asset)
}
fn mapping(payload: &Value, chain: &str, network: &str) -> Option<Value> {
    let vin = payload["vin"].as_array()?;
    let vout = payload["vout"].as_array()?;
    let inputs:Vec<Value>=vin.iter().filter(|v|v.is_object()).map(|entry|{
        let txid=strict_txid(&entry["txid"]).filter(|s|s!=&"0".repeat(64));
        let n=codec::integer(&entry["vout"]).ok().and_then(|v|v.as_i64()).filter(|n|*n>=0);
        let outpoint=if entry["vout"].is_null(){None}else{txid.zip(n).map(|(id,n)|format!("{id}:{n}"))};
        let prevout=&entry["prevout"];let (id,asset)=assets(if prevout.is_object(){prevout}else{entry});
        json!({"outpoint":outpoint,"script":if prevout.is_object(){script(prevout)}else{Value::Null},"value_sats":sats(prevout).or_else(||sats(entry)),"asset_id":id,"asset":asset})
    }).collect();
    let outputs:Vec<Value>=vout.iter().enumerate().filter(|(_,v)|v.is_object()).map(|(i,entry)|{let (id,asset)=assets(entry);json!({"n":entry.get("n").and_then(|v|codec::integer(v).ok()).and_then(|v|v.as_i64()).unwrap_or(i as i64),"script":script(entry),"value_sats":sats(entry),"asset_id":id,"asset":asset,"role":if truth(&entry["role"]){entry["role"].clone()}else{Value::Null}})}).collect();
    Some(
        json!({"txid":payload["txid"],"chain":if truth(&payload["chain"]){text(&payload["chain"]).py_strip().py_lower()}else{chain.to_owned()},"network":if truth(&payload["network"]){text(&payload["network"]).py_strip().py_lower()}else{network.to_owned()},"inputs":inputs,"outputs":outputs,"component":object(&payload["component"]),"evidence_conflicts":[]}),
    )
}
fn merge_leg(target: &mut Value, incoming: &Value, conflict: &mut bool) {
    for key in ["script", "value_sats", "asset_id", "asset", "role"] {
        if truth(&incoming[key]) || incoming[key].is_number() {
            if target[key].is_null() || target[key] == "" {
                target[key] = incoming[key].clone();
            } else if target[key] != incoming[key] {
                if key == "role"
                    && [text(&target[key]), text(&incoming[key])]
                        .into_iter()
                        .collect::<BTreeSet<_>>()
                        == BTreeSet::from(["external".into(), "owned".into()])
                {
                    target[key] = json!("owned");
                } else {
                    *conflict = true;
                }
            }
        }
    }
}
fn merge(items: &[Value]) -> Option<Value> {
    let mut merged = items.first()?.clone();
    let mut inputs: BTreeMap<String, Value> = BTreeMap::new();
    let mut positions: BTreeMap<usize, String> = BTreeMap::new();
    let mut outputs: BTreeMap<i64, Value> = BTreeMap::new();
    let mut conflict = false;
    for item in items {
        for field in ["txid", "chain", "network"] {
            if truth(&item[field]) {
                if !truth(&merged[field]) {
                    merged[field] = item[field].clone();
                } else if merged[field] != item[field] {
                    conflict = true;
                }
            }
        }
        conflict |= item["evidence_conflicts"]
            .as_array()
            .is_some_and(|v| !v.is_empty());
        for (position, leg) in item["inputs"].as_array()?.iter().enumerate() {
            let outpoint = text(&leg["outpoint"]).py_strip().to_owned();
            let previous = positions.get(&position).cloned();
            let key = if !outpoint.is_empty() {
                outpoint.clone()
            } else {
                previous
                    .clone()
                    .unwrap_or_else(|| format!("position:{position}"))
            };
            if !outpoint.is_empty() && previous.as_ref().is_some_and(|v| v != &outpoint) {
                if let Some(old) = inputs.remove(previous.as_ref().unwrap()) {
                    if let Some(target) = inputs.get_mut(&outpoint) {
                        merge_leg(target, &old, &mut conflict);
                    } else {
                        inputs.insert(outpoint.clone(), old);
                    }
                }
            }
            positions.insert(position, key.clone());
            if let Some(target) = inputs.get_mut(&key) {
                merge_leg(target, leg, &mut conflict);
            } else {
                inputs.insert(key.clone(), leg.clone());
            }
            if !outpoint.is_empty() {
                inputs.get_mut(&key).unwrap()["outpoint"] = json!(outpoint);
            }
        }
        for leg in item["outputs"].as_array()? {
            let n = int(&leg["n"]);
            if let Some(target) = outputs.get_mut(&n) {
                merge_leg(target, leg, &mut conflict);
            } else {
                outputs.insert(n, leg.clone());
            }
        }
    }
    merged["inputs"] = json!(inputs.into_values().collect::<Vec<_>>());
    merged["outputs"] = json!(outputs.into_values().collect::<Vec<_>>());
    merged["evidence_conflicts"] = if conflict {
        json!(["conflicting_graph_facts"])
    } else {
        json!([])
    };
    if merged["chain"] == "bitcoin" {
        bitcoin_assets(&mut merged);
    }
    Some(merged)
}
fn bitcoin_assets(graph: &mut Value) {
    for side in ["inputs", "outputs"] {
        for leg in graph[side].as_array_mut().unwrap() {
            for key in ["asset_id", "asset"] {
                if !truth(&leg[key]) {
                    leg[key] = json!("BTC");
                }
            }
        }
    }
}
fn parse(raw: &Value) -> Option<Value> {
    let outer = object(raw);
    let chain = text(&outer["chain"]).py_strip().py_lower();
    let network = text(&outer["network"]).py_strip().py_lower();
    let mut items = Vec::new();
    for payload in [&outer, &outer["tx"], &outer["ownership_graph"]] {
        if let Some(g) = mapping(payload, &chain, &network) {
            items.push(g);
        }
    }
    let mut parsed = merge(&items)?;
    if truth(&outer["txid"]) {
        parsed["txid"] = outer["txid"].clone();
    }
    if !chain.is_empty() {
        parsed["chain"] = json!(chain);
    } else if !truth(&parsed["chain"]) {
        parsed["chain"] = json!(if truth(&outer["component"]["asset_id"]) {
            "liquid"
        } else {
            "bitcoin"
        });
    }
    if !network.is_empty() {
        parsed["network"] = json!(network);
    }
    parsed["component"] = object(&outer["component"]);
    if parsed["chain"] == "bitcoin" {
        bitcoin_assets(&mut parsed);
    }
    Some(parsed)
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
) -> Option<(String, String, String, String)> {
    if let Some(scope) = onchain_scope(row) {
        return Some(scope);
    }
    let parsed_id = strict_txid(&graph["txid"]);
    if truth(&graph["txid"]) && parsed_id.is_none() {
        return None;
    }
    let external = strict_txid(&row["external_id"]);
    if parsed_id.is_some() && external.is_some() && parsed_id != external {
        return None;
    }
    let txid = parsed_id.or(external)?;
    let index = snapshot.owned_index.as_ref()?;
    let wallet = text(&row["wallet_id"]);
    let mut inferred = BTreeSet::new();
    for input in graph["inputs"].as_array()? {
        let outpoint = text(&input["outpoint"]);
        let stored = index["by_outpoint"][outpoint.py_lower()].as_array();
        let matches = stored
            .filter(|v| !v.is_empty())
            .or_else(|| index["by_script"][text(&input["script"]).py_lower()].as_array());
        for m in matches
            .into_iter()
            .flatten()
            .filter(|m| text(&m["wallet_id"]) == wallet)
        {
            inferred.insert(normalized_scope(m));
        }
    }
    if inferred.len() != 1 {
        inferred.clear();
        for output in graph["outputs"].as_array()? {
            for m in index["by_script"][text(&output["script"]).py_lower()]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|m| text(&m["wallet_id"]) == wallet)
            {
                inferred.insert(normalized_scope(m));
            }
        }
    }
    let inferred = if inferred.len() == 1 {
        inferred.into_iter().next()
    } else {
        None
    };
    let config = object(if truth(&row["config_json"]) {
        &row["config_json"]
    } else {
        &row["wallet_config_json"]
    });
    let mut chains = BTreeSet::new();
    for value in [&graph["chain"], &config["chain"]] {
        if !text(value).py_strip().is_empty() {
            chains.insert(evidence::chain(&text(value).py_strip().py_lower()).ok()?);
        }
    }
    if chains.len() > 1 {
        return None;
    }
    let explicit_chain = chains.into_iter().next();
    let network_chain = explicit_chain
        .clone()
        .or_else(|| inferred.as_ref().map(|s| s.0.clone()))
        .unwrap_or_else(|| "bitcoin".into());
    let mut networks = BTreeSet::new();
    for value in [&graph["network"], &config["network"]] {
        if !text(value).py_strip().is_empty() {
            networks.insert(evidence::network(&network_chain, value).ok()?);
        }
    }
    if networks.len() > 1 {
        return None;
    }
    let explicit_network = networks.into_iter().next();
    let scope = if let Some(inferred) = inferred {
        if explicit_chain.as_ref().is_some_and(|c| c != &inferred.0)
            || explicit_network.as_ref().is_some_and(|n| n != &inferred.1)
        {
            return None;
        }
        inferred
    } else {
        (
            explicit_chain?,
            explicit_network.unwrap_or_else(|| "main".into()),
        )
    };
    let display = if truth(&graph["component"]["asset"]) {
        text(&graph["component"]["asset"])
    } else {
        text(&row["asset"])
    };
    if scope.0 != "bitcoin" || display.py_strip().py_upper().replace("L-BTC", "LBTC") != "BTC" {
        return None;
    }
    Some((scope.0, scope.1, txid, "BTC".into()))
}
fn normalized_scope(m: &Value) -> (String, String) {
    let c = text(&m["chain"]).py_strip().py_lower();
    let c = evidence::chain(&c).unwrap_or(c);
    let n = evidence::network(&c, &m["network"])
        .unwrap_or_else(|_| text(&m["network"]).py_strip().py_lower());
    (c, n)
}
fn script_owners(snapshot: &Snapshot, script: &Value, scope: (&str, &str)) -> BTreeSet<String> {
    let index = match &snapshot.owned_index {
        Some(v) => v,
        None => return BTreeSet::new(),
    };
    index["by_script"][text(script).py_lower()]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| normalized_scope(m) == (scope.0.into(), scope.1.into()))
        .map(|m| text(&m["wallet_id"]))
        .collect()
}
fn input_owners(snapshot: &Snapshot, entry: &Value, scope: (&str, &str)) -> BTreeSet<String> {
    if let Some(index) = &snapshot.owned_index {
        let outpoint = text(&entry["outpoint"]).py_lower();
        let scoped = index["by_scoped_outpoint"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|v| v["key"] == json!([scope.0, scope.1, outpoint]));
        let matches = if let Some(v) = scoped {
            &v["matches"]
        } else {
            &index["by_outpoint"][&outpoint]
        };
        let owners: BTreeSet<String> = matches
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| normalized_scope(m) == (scope.0.into(), scope.1.into()))
            .map(|m| text(&m["wallet_id"]))
            .collect();
        if !owners.is_empty() {
            return owners;
        }
    }
    script_owners(snapshot, &entry["script"], scope)
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
        let Some(g) = parse(&row["raw_json"]) else {
            continue;
        };
        let Some(scope) = ownership_scope(row, &g, snapshot) else {
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
                    input_owners(snapshot, leg, (&scope.0, &scope.1))
                } else {
                    script_owners(snapshot, &leg["script"], (&scope.0, &scope.1))
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
        let Some(graphs): Option<Vec<Value>> = raw.iter().map(parse).collect() else {
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
        let parsed: Vec<Option<Value>> = group.iter().map(|r| parse(&r["raw_json"])).collect();
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
        let observed: i128 = group
            .iter()
            .filter(|r| r["direction"] == "inbound")
            .map(|r| i128::from(int(&r["amount"])))
            .sum();
        let covered: i128 = outputs
            .iter()
            .zip(&owners)
            .filter(|(_, s)| valued && !s.is_empty())
            .map(|(o, _)| i128::from(int(&o["value_sats"])) * 1000)
            .sum();
        let kinds: BTreeSet<String> = group.iter().map(|r| text(&r["wallet_kind"])).collect();
        let historical = kinds.contains("address")
            && ["descriptor", "xpub", "silent-payment"]
                .iter()
                .any(|k| kinds.contains(*k))
            && valued
            && covered > 0
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
        if first["inputs"].as_array().unwrap().iter().any(|e| {
            !truth(&e["script"])
                || !truth(&e["outpoint"])
                || e["value_sats"].is_null()
                || !input_owners(snapshot, e, (&scope.0, &scope.1)).is_empty()
        }) {
            continue;
        }
        let mut full: BTreeMap<String, i128> = wallets.iter().cloned().map(|w| (w, 0)).collect();
        let mut canonical = full.clone();
        let mut assignments = Vec::new();
        for (output, sources) in outputs.iter().zip(&owners) {
            if sources.is_empty() {
                continue;
            }
            let amount = wide(&output["value_sats"]) * 1000;
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
                *full.get_mut(w).unwrap() += amount;
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
            *canonical.get_mut(&owner).unwrap() += amount;
            assignments.push(json!({"vout":output["n"],"wallet_id":owner,"amount_msat":amount}));
        }
        if group.iter().try_fold(false, |bad, r| {
            let w = text(&r["wallet_id"]);
            Ok::<_, String>(
                bad || canonical[&w] <= 0
                    || ![full[&w], canonical[&w]]
                        .contains(&wide(&codec::integer_or_zero(&r["amount"])?)),
            )
        })? {
            continue;
        }
        for row in &group {
            result.insert(text(&row["id"]),json!({"schema_version":1,"amount_msat":canonical[&text(&row["wallet_id"])],"outputs":assignments}));
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
            assert_eq!(parse(&v["raw"]).unwrap(), v["graph"], "{}", v["case"]);
        }
    }
}

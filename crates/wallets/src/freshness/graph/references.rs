use crate::{
    bitcoin_records::{script, value_sats},
    publication::row::{field, rows, Result},
};
use kassiber_pyjson::Value as Py;
use kassiber_store::rusqlite::{params, Connection};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub(super) fn py(value: &Value) -> Py {
    kassiber_pytext::loads(&value.to_string()).expect("JSON value")
}

pub(super) fn json(value: &Py) -> Value {
    serde_json::from_str(&kassiber_pyjson::dumps(value, &Default::default()).expect("JSON value"))
        .expect("JSON value")
}

pub(super) fn object(value: &Value) -> Value {
    kassiber_custody::confirmation::stored_mapping(value, false).unwrap_or_else(|| json!({}))
}

pub(super) fn string(value: &Value) -> Option<&str> {
    value
        .as_str()
        .map(kassiber_pytext::strip)
        .filter(|s| !s.is_empty())
}

pub(super) fn txid(value: &Value) -> Option<String> {
    let value = string(value)?;
    (value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

fn integer(value: &Value) -> Option<Value> {
    crate::scan::integer(value).map(|n| serde_json::from_str(&n.to_string()).expect("integer"))
}

pub(super) fn scope(row: &Value) -> Option<(String, String)> {
    let (domain, _, valid) = kassiber_setup::networks::evidence(&py(row));
    if !valid {
        return None;
    }
    let (chain, mut network) = kassiber_custody::evidence::scope(row).ok()?;
    if let Some(domain) = domain {
        network = if chain == "liquid" {
            match domain.as_str() {
                "main" => "liquidv1",
                "test" => "liquidtestnet",
                "regtest" => "elementsregtest",
                _ => return None,
            }
            .into()
        } else {
            domain
        };
    }
    Some((chain, network))
}

pub(super) fn complete(raw: &Value) -> bool {
    let (Some(vin), Some(_)) = (raw["vin"].as_array(), raw["vout"].as_array()) else {
        return false;
    };
    vin.iter().all(|entry| {
        if !entry.is_object() {
            return false;
        }
        if string(&entry["txid"]).is_none() && integer(&entry["vout"]).is_none() {
            return true;
        }
        string(&entry["txid"]).is_some()
            && integer(&entry["vout"]).is_some()
            && value_sats(&entry["prevout"]).ok().flatten().is_some()
            && !script(&entry["prevout"]).is_empty()
    })
}

pub(super) fn confidential(entry: &Value) -> bool {
    string(&entry["value_state"])
        .or_else(|| string(&entry["valueState"]))
        .is_some_and(|s| s.eq_ignore_ascii_case("confidential"))
        || [
            "valuecommitment",
            "assetcommitment",
            "surjectionproof",
            "rangeproof",
        ]
        .iter()
        .any(|key| !entry[*key].is_null())
}

fn flag(entry: &Value, names: &[&str]) -> bool {
    for name in names {
        if let Some(value) = entry[*name].as_bool() {
            return value;
        }
        if string(&entry[*name])
            .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1"))
        {
            return true;
        }
    }
    false
}

fn coinbase(entry: &Value) -> bool {
    flag(entry, &["is_coinbase", "isCoinbase", "coinbase"])
        || string(&entry["txid"]).is_some_and(|id| id == "0".repeat(64))
            && integer(&entry["vout"]) == Some(json!(u32::MAX))
}

fn value_script(entry: &Value, include_n: bool) -> Map<String, Value> {
    let mut result = Map::new();
    if include_n {
        if let Some(n) = integer(&entry["n"]) {
            result.insert("n".into(), n);
        }
    }
    if !script(entry).is_empty() {
        result.insert(
            "scriptpubkey".into(),
            json!(kassiber_pytext::strip(script(entry))),
        );
    }
    for (key, value) in [
        (
            "scriptpubkey_type",
            string(&entry["scriptpubkey_type"]).or_else(|| string(&entry["type"])),
        ),
        (
            "scriptpubkey_address",
            string(&entry["scriptpubkey_address"])
                .or_else(|| string(&entry["address"]))
                .or_else(|| string(&entry["scriptPubKey"]["address"]))
                .or_else(|| {
                    entry["scriptPubKey"]["addresses"]
                        .as_array()
                        .and_then(|v| v.first())
                        .and_then(string)
                }),
        ),
        (
            "pegout_address",
            string(&entry["pegout_address"])
                .or_else(|| string(&entry["pegoutAddress"]))
                .or_else(|| string(&entry["pegout"]["scriptpubkey_address"]))
                .or_else(|| string(&entry["pegout"]["scriptpubkeyAddress"])),
        ),
    ] {
        if let Some(value) = value {
            result.insert(key.into(), json!(value));
        }
    }
    if let Some(asset) = string(&entry["asset_id"])
        .or_else(|| string(&entry["assetId"]))
        .or_else(|| string(&entry["asset"]))
    {
        let asset = if asset.len() == 64 {
            asset.to_ascii_lowercase()
        } else if asset.eq_ignore_ascii_case("l-btc") {
            "LBTC".into()
        } else {
            asset.to_uppercase()
        };
        result.insert("asset".into(), json!(asset));
    }
    if confidential(entry) {
        result.insert("value_state".into(), json!("confidential"));
    } else if let Ok(Some(value)) = value_sats(entry) {
        result.insert(
            "value".into(),
            serde_json::from_str(&value.to_string()).expect("integer"),
        );
    }
    result
}

pub(super) fn sanitize(raw: &Value, id: &str) -> Value {
    let mut result = Map::new();
    result.insert(
        "txid".into(),
        json!(string(&raw["txid"]).unwrap_or(id).to_ascii_lowercase()),
    );
    for key in ["version", "locktime", "size", "vsize", "weight"] {
        if let Some(value) = integer(&raw[key]) {
            result.insert(key.into(), value);
        }
    }
    if let Some(vin) = raw["vin"].as_array() {
        result.insert(
            "vin".into(),
            Value::Array(
                vin.iter()
                    .filter(|v| v.is_object())
                    .map(|entry| {
                        let mut clean = Map::new();
                        if let Some(id) = string(&entry["txid"]) {
                            clean.insert("txid".into(), json!(id.to_ascii_lowercase()));
                        }
                        if let Some(n) = integer(&entry["vout"]) {
                            clean.insert("vout".into(), n);
                        }
                        if coinbase(entry) {
                            clean.insert("is_coinbase".into(), json!(true));
                        }
                        if flag(entry, &["is_pegin", "isPegin", "pegin"]) {
                            clean.insert("is_pegin".into(), json!(true));
                        }
                        if entry["prevout"].is_object() {
                            let prevout = value_script(&entry["prevout"], false);
                            if !prevout.is_empty() {
                                clean.insert("prevout".into(), Value::Object(prevout));
                            }
                        }
                        Value::Object(clean)
                    })
                    .collect(),
            ),
        );
    }
    if let Some(vout) = raw["vout"].as_array() {
        result.insert(
            "vout".into(),
            Value::Array(
                vout.iter()
                    .enumerate()
                    .filter(|(_, v)| v.is_object())
                    .map(|(n, entry)| {
                        let mut clean = value_script(entry, true);
                        clean.entry("n").or_insert_with(|| json!(n));
                        Value::Object(clean)
                    })
                    .collect(),
            ),
        );
    }
    Value::Object(result)
}

/// `GRAPH_CACHE_SCHEMA_VERSION`: 2 since cached graphs carry verified
/// Electrum identities (10719a7d5), so older entries are never reused.
const CACHE_SCHEMA_VERSION: i64 = 2;

pub(super) fn cached(
    conn: &Connection,
    chain: &str,
    network: &str,
    id: &str,
) -> Result<Option<Value>> {
    let row = crate::publication::row::first(conn,
        "SELECT payload_json FROM transaction_graph_cache WHERE schema_version=? AND chain=? AND network=? AND txid=?",
        params![CACHE_SCHEMA_VERSION, chain, network, id])?;
    let Some(row) = row else { return Ok(None) };
    let mut raw = object(&json(field(&row, "payload_json")));
    let (Some(vin), Some(_)) = (raw["vin"].as_array(), raw["vout"].as_array()) else {
        return Ok(None);
    };
    if chain == "bitcoin"
        && vin.iter().any(|v| {
            v.is_object()
                && !coinbase(v)
                && (txid(&v["txid"]).is_none() || integer(&v["vout"]).is_none())
        })
    {
        return Ok(None);
    }
    let context = kassiber_setup::context::snapshot(conn)?;
    let binding = kassiber_setup::networks::resolve(
        conn,
        &crate::publication::row::or_text(field(&context, "profile_id")),
    )?;
    if !kassiber_setup::networks::observation_matches_binding(
        &binding,
        &py(&json!({"config_json":{"chain":chain,"network":network},"raw_json":raw})),
        true,
    ) || string(&raw["txid"]).is_some_and(|v| !v.eq_ignore_ascii_case(id))
    {
        return Ok(None);
    }
    raw["txid"] = json!(id);
    Ok(Some(raw))
}

pub(super) fn store(
    conn: &Connection,
    chain: &str,
    network: &str,
    id: &str,
    raw: &Value,
    now: &str,
    check: &dyn Fn() -> Result<()>,
) -> Result<Value> {
    check()?;
    let mut raw = sanitize(raw, id);
    let context = kassiber_setup::context::snapshot(conn)?;
    let profile = crate::publication::row::or_text(field(&context, "profile_id"));
    let binding = kassiber_setup::networks::resolve(conn, &profile)?;
    if field(&binding, "state") == &kassiber_envelope::s("bound") {
        let domain = kassiber_setup::networks::require_chain_domain(
            conn,
            &profile,
            chain,
            &kassiber_envelope::s(network),
            None,
            "graph_lookup",
        )?;
        if kassiber_envelope::truthy(field(&domain, "chain_instance_id")) {
            raw["chain_instance_id"] = json(field(&domain, "chain_instance_id"));
        }
    }
    conn.execute("INSERT INTO transaction_graph_cache(schema_version,chain,network,txid,payload_json,created_at,updated_at)
        VALUES(?,?,?,?,?,?,?) ON CONFLICT(schema_version,chain,network,txid) DO UPDATE SET payload_json=excluded.payload_json,updated_at=excluded.updated_at",
        params![CACHE_SCHEMA_VERSION,chain,network,id,raw.to_string(),now,now]).map_err(crate::database_error)?;
    super::super::execution::commit(conn)?;
    Ok(raw)
}

fn facts(raw: &Value) -> Option<BTreeMap<String, Value>> {
    let (vin, vout) = (raw["vin"].as_array()?, raw["vout"].as_array()?);
    if vin.is_empty() || vout.is_empty() {
        return None;
    }
    let mut facts = BTreeMap::from([
        ("input_count".into(), json!(vin.len())),
        ("output_count".into(), json!(vout.len())),
    ]);
    for (side, entries) in [("in", vin), ("out", vout)] {
        for (index, entry) in entries.iter().enumerate() {
            if !entry.is_object()
                || side == "out" && entry.get("n").is_some_and(|n| n != &json!(index))
            {
                return None;
            }
            let output = if side == "in" {
                let outpoint = match (
                    txid(&entry["txid"]).filter(|v| v != &"0".repeat(64)),
                    integer(&entry["vout"]).filter(|v| v.as_i64().is_some_and(|n| n >= 0)),
                ) {
                    (Some(id), Some(n)) => json!([id, n]),
                    _ if entry.get("coinbase").is_some() => json!("coinbase"),
                    _ => return None,
                };
                facts.insert(format!("in:{index}:outpoint"), outpoint);
                if crate::descriptors::truthy(&entry["prevout"]) && !entry["prevout"].is_object() {
                    return None;
                }
                &entry["prevout"]
            } else {
                entry
            };
            if !script(output).is_empty() {
                facts.insert(format!("{side}:{index}:script"), json!(script(output)));
            }
            if let Ok(Some(value)) = value_sats(output) {
                facts.insert(
                    format!("{side}:{index}:value"),
                    serde_json::from_str(&value.to_string()).ok()?,
                );
            }
            for key in ["asset", "valuecommitment", "assetcommitment"] {
                if !output[key].is_null() {
                    facts.insert(format!("{side}:{index}:{key}"), output[key].clone());
                }
            }
        }
    }
    Some(facts)
}

pub(super) enum Local {
    Conflict,
    Payload(Value),
    Absent,
}

pub(super) fn local(
    conn: &Connection,
    profile: &str,
    chain: &str,
    network: &str,
    id: &str,
    current: &Value,
) -> Result<Local> {
    let rows = rows(conn, "SELECT t.raw_json,t.external_id,t.asset,w.kind AS wallet_kind,w.config_json AS wallet_config_json
        FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.excluded=0
        AND (lower(t.external_id)=? OR lower(CASE WHEN json_valid(t.raw_json) THEN json_extract(t.raw_json,'$.txid') END)=?)
        ORDER BY t.id LIMIT 257", [profile,id,id])?;
    if rows.len() > 256 {
        return Ok(Local::Conflict);
    }
    let mut candidates: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            let row = json(row);
            let raw = kassiber_custody::confirmation::stored_mapping(&row["raw_json"], true)?;
            Some((row, raw, json!({})))
        })
        .collect();
    if let Some(row) = crate::publication::row::first(conn,
        "SELECT payload_json,status_json FROM chain_analysis_observations WHERE profile_id=? AND chain=? AND network=? AND txid=?",
        [profile,chain,network,id])? {
        let raw = object(&json(field(&row, "payload_json")));
        candidates.push((json!({"chain":chain,"network":network,"raw_json":raw}),raw,
            object(&json(field(&row, "status_json")))));
    }
    let current = facts(current).unwrap_or_default();
    let mut combined = current.clone();
    let mut best = None;
    let mut best_size = current.len();
    for (row, raw, status) in candidates {
        if string(&raw["txid"]).is_some_and(|v| !v.eq_ignore_ascii_case(id))
            || string(&row["external_id"])
                .is_some_and(|v| v.len() == 64 && !v.eq_ignore_ascii_case(id))
            || scope(&row).as_ref() != Some(&(chain.into(), network.into()))
            || scope(&json!({"chain":chain,"network":network,"raw_json":raw})).as_ref()
                != Some(&(chain.into(), network.into()))
            || status["removed"] == true
            || status["conflicted"] == true
            || raw["removed"] == true
            || raw["confirmations"].as_f64().is_some_and(|n| n < 0.0)
        {
            continue;
        }
        let Some(facts) = facts(&raw) else { continue };
        if facts
            .iter()
            .any(|(key, value)| combined.get(key).is_some_and(|old| old != value))
        {
            return Ok(Local::Conflict);
        }
        combined.extend(facts.clone());
        if facts.len() > best_size && current.keys().all(|key| facts.contains_key(key)) {
            best_size = facts.len();
            best = Some(raw);
        }
    }
    Ok(best.map(Local::Payload).unwrap_or(Local::Absent))
}

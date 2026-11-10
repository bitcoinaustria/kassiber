//! Physical transaction identity and closed confirmation evidence, following
//! `transfers.onchain_transfer_scope`, `bitcoin_network_domain_evidence` and
//! `ownership_transfers._current_confirmation_states`. Publication uses these
//! only to decide whether journal inputs went stale; they grant no authority.
use super::row::{blank, field, or_text, Result};
use kassiber_envelope::{get, py_str, truthy, AppError};
use kassiber_pyjson::{Key, Value};
use std::collections::BTreeMap;

pub(crate) type Physical = (String, String, String);

const EXTERNAL_TXID_FIELD_KEYS: [&str; 10] = [
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
];
const LIGHTNING_WALLET_KINDS: [&str; 6] = ["cln", "coreln", "lightning", "lnd", "nwc", "phoenix"];
const TRANSFER_SYNTHETIC_PREFIXES: [&str; 6] = [
    "custody-tax:",
    "cross-split:",
    "direct-payout:",
    "multi-consol:",
    "owned-derive:",
    "recorded-fanout:",
];
const OWNERSHIP_SYNTHETIC_PREFIXES: [&str; 5] = [
    "owned-derive:",
    "recorded-fanout:",
    "cross-split:",
    "direct-payout:",
    "multi-consol:",
];

/// `canonical_txid`: 64 hex code points after `str.strip()`, lowercased.
pub(crate) fn canonical_txid(value: &Value) -> Option<String> {
    let text = or_text(value);
    let text = kassiber_pytext::strip(&text);
    (text.chars().count() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| text.to_ascii_lowercase())
}

fn py_lower_strip(value: &Value) -> String {
    kassiber_pytext::lower(kassiber_pytext::strip(&or_text(value)))
}

/// `_json_mapping`: a dict, or `{}` for blank/invalid/non-object JSON.
pub(crate) fn json_mapping(value: &Value) -> Value {
    match value {
        Value::Dict(_) => value.clone(),
        Value::Null => Value::Dict(vec![]),
        Value::Str(text) if matches!(text.as_str(), Some("" | "{}")) => Value::Dict(vec![]),
        other => match kassiber_pytext::loads(&py_str(other)) {
            Ok(parsed @ Value::Dict(_)) => parsed,
            _ => Value::Dict(vec![]),
        },
    }
}

fn json_mapping_sequence(value: &Value) -> Vec<Value> {
    let parsed = match value {
        Value::List(_) => value.clone(),
        Value::Null => return vec![],
        Value::Str(text) if matches!(text.as_str(), Some("" | "{}")) => return vec![],
        other => match kassiber_pytext::loads(&py_str(other)) {
            Ok(parsed) => parsed,
            Err(_) => return vec![],
        },
    };
    match parsed {
        Value::List(items) => items
            .into_iter()
            .filter(|item| matches!(item, Value::Dict(_)))
            .collect(),
        _ => vec![],
    }
}

/// `stored_tx_mapping(raw_json, allow_nested=...)`.
pub(crate) fn stored_tx_mapping(raw: &Value, allow_nested: bool) -> Option<Value> {
    let parsed = match raw {
        Value::Dict(_) => raw.clone(),
        other => {
            let text = or_text(other);
            kassiber_pytext::loads(if text.is_empty() { "{}" } else { &text }).ok()?
        }
    };
    if !matches!(parsed, Value::Dict(_)) {
        return None;
    }
    if allow_nested {
        if let Some(nested @ Value::Dict(_)) = get(&parsed, "tx") {
            return Some(nested.clone());
        }
    }
    Some(parsed)
}

fn canonical_chain(value: &Value) -> Option<&'static str> {
    Some(match py_lower_strip(value).as_str() {
        "bitcoin" | "btc" | "onchain" => "bitcoin",
        "liquid" | "lbtc" | "elements" => "liquid",
        _ => return None,
    })
}

pub(crate) fn canonical_network(chain: &str, value: &Value) -> Option<&'static str> {
    let text = py_lower_strip(value);
    Some(if chain == "bitcoin" {
        match text.as_str() {
            "bitcoin" | "main" | "mainnet" => "main",
            "test" | "testnet" => "test",
            "regtest" => "regtest",
            "signet" => "signet",
            _ => return None,
        }
    } else {
        match text.as_str() {
            "liquid" | "liquidv1" | "main" | "mainnet" => "liquidv1",
            "liquidtestnet" | "test" | "testnet" => "liquidtestnet",
            "elements" | "elementsregtest" | "regtest" => "elementsregtest",
            _ => return None,
        }
    })
}

fn liquid_domain(network: &str) -> Option<&'static str> {
    match network {
        "liquidv1" => Some("main"),
        "liquidtestnet" => Some("test"),
        "elementsregtest" => Some("regtest"),
        _ => None,
    }
}

fn wallet_kind_alias(value: &Value) -> String {
    let normalized = py_lower_strip(value).replace('_', "-");
    match normalized.as_str() {
        "core-ln" | "core-lightning" => "coreln".into(),
        _ => normalized,
    }
}

fn identity_key(key: &Key) -> String {
    let text = match key {
        Key::Str(text) => {
            if text.code_points().next().is_none() {
                String::new()
            } else {
                text.code_points().filter_map(char::from_u32).collect()
            }
        }
        Key::Null => String::new(),
        Key::Bool(false) => String::new(),
        Key::Bool(true) => "True".into(),
        Key::Int(n) if *n == 0.into() => String::new(),
        Key::Int(n) => n.to_string(),
        Key::Float(f) if *f == 0.0 => String::new(),
        Key::Float(f) => kassiber_pyjson::float_repr(*f),
    };
    kassiber_pytext::lower(&text)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn pairs(value: &Value) -> &[(Key, Value)] {
    match value {
        Value::Dict(pairs) => pairs,
        _ => &[],
    }
}

/// Python iterates a JSON object's keys once, with the last duplicate's value.
fn items(value: &Value) -> Vec<(&Key, &Value)> {
    let all = pairs(value);
    all.iter()
        .enumerate()
        .filter(|(index, (key, _))| !all[index + 1..].iter().any(|(later, _)| later == key))
        .map(|(_, (key, value))| (key, value))
        .collect()
}

fn external_id_is_typed_txid(row: &Value, raw: &Value, graphs: &[Value], external: &str) -> bool {
    let declared = if truthy(field(row, "external_id_kind")) {
        field(row, "external_id_kind")
    } else {
        get(raw, "external_id_kind").unwrap_or(&Value::Null)
    };
    let explicit = py_lower_strip(declared);
    if !explicit.is_empty() {
        return matches!(
            explicit.as_str(),
            "txid" | "transaction_hash" | "onchain_txid"
        );
    }
    graphs.iter().any(|payload| {
        items(payload).into_iter().any(|(key, value)| {
            EXTERNAL_TXID_FIELD_KEYS.contains(&identity_key(key).as_str())
                && canonical_txid(value).as_deref() == Some(external)
        })
    })
}

fn liquid_component(graphs: &[Value]) -> Option<(String, String)> {
    let mut ids = std::collections::BTreeSet::new();
    let mut display = std::collections::BTreeSet::new();
    for payload in graphs {
        let mut candidates = vec![payload];
        if let Some(component @ Value::Dict(_)) = get(payload, "component") {
            candidates.push(component);
        }
        for candidate in candidates {
            let raw_id = get(candidate, "asset_id").unwrap_or(&Value::Null);
            if !blank(raw_id) {
                ids.insert(canonical_txid(raw_id)?);
            }
            let raw_asset = get(candidate, "asset").unwrap_or(&Value::Null);
            if !blank(raw_asset) {
                display.insert(
                    kassiber_pytext::strip(&py_str(raw_asset))
                        .to_uppercase()
                        .replace("L-BTC", "LBTC"),
                );
            }
        }
    }
    if ids.len() != 1 || display.len() > 1 {
        return None;
    }
    Some((
        ids.into_iter().next()?,
        display.into_iter().next().unwrap_or_default(),
    ))
}

fn config_mapping(row: &Value) -> Value {
    let config = field(row, "config_json");
    json_mapping(if truthy(config) {
        config
    } else {
        field(row, "wallet_config_json")
    })
}

fn row_asset(row: &Value) -> String {
    kassiber_pytext::strip(&or_text(field(row, "asset")))
        .to_uppercase()
        .replace("L-BTC", "LBTC")
}

/// `onchain_transfer_scope(row)`: `(chain, network, txid, asset identity)`.
pub(crate) fn transfer_scope(row: &Value) -> Option<(String, String, String, String)> {
    let id = or_text(field(row, "id"));
    if TRANSFER_SYNTHETIC_PREFIXES
        .iter()
        .any(|prefix| id.starts_with(prefix))
    {
        return None;
    }
    let raw_value = field(row, "raw_json");
    let raw = json_mapping(raw_value);
    let config = config_mapping(row);
    let mut graphs = vec![raw.clone()];
    graphs.extend(json_mapping_sequence(raw_value));
    for key in ["tx", "ownership_graph"] {
        if let Some(nested @ Value::Dict(_)) = get(&raw, key) {
            graphs.push(nested.clone());
        }
    }
    let mut graph_txids = std::collections::BTreeSet::new();
    for payload in &graphs {
        let value = get(payload, "txid").unwrap_or(&Value::Null);
        let present = !kassiber_pytext::strip(&or_text(value)).is_empty();
        match canonical_txid(value) {
            Some(txid) => {
                graph_txids.insert(txid);
            }
            None if present => return None,
            None => {}
        }
    }
    if graph_txids.len() > 1 {
        return None;
    }
    let external = canonical_txid(field(row, "external_id"));
    let graph = graph_txids.into_iter().next();
    if let (Some(graph), Some(external)) = (&graph, &external) {
        if graph != external {
            return None;
        }
    }
    if graph.is_none() {
        if let Some(external) = &external {
            if !external_id_is_typed_txid(row, &raw, &graphs, external) {
                return None;
            }
        }
    }
    let txid = graph.or(external)?;
    let has_graph = graphs.iter().any(|payload| {
        matches!(get(payload, "vin"), Some(Value::List(_)))
            && matches!(get(payload, "vout"), Some(Value::List(_)))
    });
    let wallet_kind = wallet_kind_alias(field(row, "wallet_kind"));
    if LIGHTNING_WALLET_KINDS.contains(&wallet_kind.as_str()) && !has_graph {
        return None;
    }
    let asset = row_asset(row);
    let chain_values: Vec<&Value> = graphs
        .iter()
        .map(|payload| get(payload, "chain").unwrap_or(&Value::Null))
        .chain(std::iter::once(
            get(&config, "chain").unwrap_or(&Value::Null),
        ))
        .collect();
    let mut chains = std::collections::BTreeSet::new();
    for value in &chain_values {
        if kassiber_pytext::strip(&or_text(value)).is_empty() {
            continue;
        }
        chains.insert(canonical_chain(value)?);
    }
    if chains.len() > 1 {
        return None;
    }
    let inferred = match asset.as_str() {
        "LBTC" => Some("liquid"),
        "BTC" => Some("bitcoin"),
        _ => None,
    };
    let chain = chains.into_iter().next().or(inferred)?;
    if (asset == "BTC" && chain != "bitcoin") || (asset == "LBTC" && chain != "liquid") {
        return None;
    }
    let network_values: Vec<&Value> = graphs
        .iter()
        .map(|payload| get(payload, "network").unwrap_or(&Value::Null))
        .chain(std::iter::once(
            get(&config, "network").unwrap_or(&Value::Null),
        ))
        .collect();
    let mut networks = std::collections::BTreeSet::new();
    for value in &network_values {
        if kassiber_pytext::strip(&or_text(value)).is_empty() {
            continue;
        }
        networks.insert(canonical_network(chain, value)?);
    }
    if networks.len() > 1 {
        return None;
    }
    let network = match networks.into_iter().next() {
        Some(network) => network,
        None if chain == "bitcoin" => "main",
        None => return None,
    };
    let mut identity = asset.clone();
    if chain == "liquid" {
        let (asset_id, component_asset) = liquid_component(&graphs)?;
        if !component_asset.is_empty() && component_asset != asset {
            return None;
        }
        if asset != "LBTC" && asset != asset_id.to_uppercase() {
            return None;
        }
        identity = asset_id;
    }
    Some((chain.into(), network.into(), txid, identity))
}

/// `bitcoin_network_domain_evidence(row)`: `(domain, valid)`.
pub(crate) fn network_domain_evidence(row: &Value) -> (Option<String>, bool) {
    if let Some((chain, network, _, _)) = transfer_scope(row) {
        if chain == "bitcoin" {
            return (Some(network), true);
        }
        let domain = liquid_domain(&network);
        return (domain.map(str::to_owned), domain.is_some());
    }
    let raw = json_mapping(field(row, "raw_json"));
    let config = config_mapping(row);
    let mut families = std::collections::BTreeSet::new();
    for payload in [&raw, &config] {
        let value = get(payload, "chain").unwrap_or(&Value::Null);
        if blank(value) {
            continue;
        }
        if let Some(chain) = canonical_chain(value) {
            families.insert(chain);
        } else if LIGHTNING_WALLET_KINDS.contains(&wallet_kind_alias(value).as_str()) {
            families.insert("lightning");
        } else {
            return (None, false);
        }
    }
    let bases: std::collections::BTreeSet<_> = families
        .iter()
        .map(|family| {
            if *family == "liquid" {
                "liquid"
            } else {
                "bitcoin"
            }
        })
        .collect();
    if bases.len() > 1 {
        return (None, false);
    }
    let asset = row_asset(row);
    let only = |name: &str| bases.len() == 1 && bases.contains(name);
    if (asset == "BTC" && only("liquid")) || (asset == "LBTC" && only("bitcoin")) {
        return (None, false);
    }
    let mut domains = std::collections::BTreeSet::new();
    for payload in [&raw, &config] {
        for key in ["network", "bitcoin_network", "chain_network"] {
            let value = get(payload, key).unwrap_or(&Value::Null);
            if blank(value) {
                continue;
            }
            if let Some(network) = canonical_network("bitcoin", value) {
                domains.insert(network);
            } else if let Some(network) = canonical_network("liquid", value) {
                if let Some(domain) = liquid_domain(network) {
                    domains.insert(domain);
                }
            } else {
                return (None, false);
            }
        }
    }
    if domains.len() > 1 {
        return (None, false);
    }
    (domains.into_iter().next().map(str::to_owned), true)
}

fn physical_onchain_scope(row: &Value) -> Option<(String, String, String, String)> {
    if let Some(scope) = transfer_scope(row) {
        return Some(scope);
    }
    let id = or_text(field(row, "id"));
    if !OWNERSHIP_SYNTHETIC_PREFIXES
        .iter()
        .any(|prefix| id.starts_with(prefix))
    {
        return None;
    }
    let mut probe = row.clone();
    kassiber_envelope::set(
        &mut probe,
        "id",
        Value::from(format!("physical-anchor:{}", py_str(field(row, "id")))),
    );
    transfer_scope(&probe)
}

fn infinite(value: &Value) -> bool {
    matches!(value, Value::Float(f) if f.is_infinite())
}

/// The `txid` slot of `parse_ownership_tx(raw_json)`, or `None` when that parser
/// returns `None`. CPython raises `OverflowError` for an infinite index; the
/// same input fails closed here. Liquid `raw_hex` decoding is not ported.
fn ownership_txid(raw: &Value) -> Result<Option<Value>> {
    let Some(outer) = stored_tx_mapping(raw, false) else {
        return Ok(None);
    };
    let chain = py_lower_strip(get(&outer, "chain").unwrap_or(&Value::Null));
    let looks_liquid = chain == "liquid"
        || matches!(get(&outer, "component"), Some(component @ Value::Dict(_))
            if truthy(get(component, "asset_id").unwrap_or(&Value::Null)));
    let mut txids = Vec::new();
    let mut candidates = vec![outer.clone()];
    for key in ["tx", "ownership_graph"] {
        if let Some(nested @ Value::Dict(_)) = get(&outer, key) {
            candidates.push(nested.clone());
        }
    }
    for payload in &candidates {
        let (Some(Value::List(vin)), Some(Value::List(vout))) =
            (get(payload, "vin"), get(payload, "vout"))
        else {
            continue;
        };
        // `input_outpoint` converts `vout` only after accepting the txid.
        let overflow = vin
            .iter()
            .filter(|entry| {
                let txid = kassiber_pytext::lower(kassiber_pytext::strip(&or_text(
                    get(entry, "txid").unwrap_or(&Value::Null),
                )));
                txid.len() == 64
                    && txid.bytes().all(|b| b.is_ascii_hexdigit())
                    && txid != "0".repeat(64)
            })
            .any(|entry| infinite(get(entry, "vout").unwrap_or(&Value::Null)))
            || vout
                .iter()
                .any(|entry| infinite(get(entry, "n").unwrap_or(&Value::Null)));
        if overflow {
            return Err(AppError::new(
                "observer_state_invalid",
                "Stored transaction graph contains an infinite index",
            ));
        }
        txids.push(get(payload, "txid").cloned().unwrap_or(Value::Null));
    }
    if looks_liquid && truthy(get(&outer, "raw_hex").unwrap_or(&Value::Null)) && txids.is_empty() {
        return Err(AppError::new(
            "not_implemented",
            "Native publication does not decode stored Liquid raw transactions",
        ));
    }
    let Some(first) = txids.first() else {
        return Ok(None);
    };
    let mut merged = first.clone();
    for value in &txids {
        if blank(&merged) && !blank(value) {
            merged = value.clone();
        }
    }
    let outer_txid = get(&outer, "txid").unwrap_or(&Value::Null);
    Ok(Some(if truthy(outer_txid) {
        outer_txid.clone()
    } else {
        merged
    }))
}

/// `_physical_conflict_scope(row, _parse_onchain_tx(raw, allow_partial=True))`.
/// Callers always parse first, so a refused graph fails even with a scope.
pub(crate) fn conflict_scope(row: &Value) -> Result<Option<Physical>> {
    let parsed = ownership_txid(field(row, "raw_json"))?;
    if let Some((chain, network, txid, _)) = physical_onchain_scope(row) {
        return Ok(Some((chain, network, txid)));
    }
    let Some(parsed_txid) = parsed else {
        return Ok(None);
    };
    let raw_txid = kassiber_pytext::strip(&or_text(&parsed_txid)).to_owned();
    let parsed = canonical_txid(&parsed_txid);
    if !raw_txid.is_empty() && parsed.is_none() {
        return Ok(None);
    }
    let external = canonical_txid(field(row, "external_id"));
    if let (Some(parsed), Some(external)) = (&parsed, &external) {
        if parsed != external {
            return Ok(None);
        }
    }
    let Some(txid) = parsed.or(external) else {
        return Ok(None);
    };
    let chain = match row_asset(row).as_str() {
        "BTC" => "bitcoin",
        "LBTC" => "liquid",
        _ => return Ok(None),
    };
    let (domain, valid) = network_domain_evidence(row);
    if !valid {
        return Ok(None);
    }
    let domain = match domain {
        Some(domain) => domain,
        None if chain == "bitcoin" => "main".into(),
        None => return Ok(None),
    };
    Ok(Some((chain.into(), domain, txid)))
}

fn status_confirmed(payload: Option<&Value>) -> Option<bool> {
    match payload.and_then(|payload| get(payload, "status")) {
        Some(status @ Value::Dict(_)) => match get(status, "confirmed") {
            Some(Value::Bool(confirmed)) => Some(*confirmed),
            _ => None,
        },
        _ => None,
    }
}

/// `int(value or 0)` inside `row_has_current_authoritative_observation`.
fn authority_version(value: &Value) -> Result<Option<i64>> {
    if !truthy(value) {
        return Ok(Some(0));
    }
    Ok(match value {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Int(n) => Some(i64::try_from(n.clone()).unwrap_or(i64::MAX)),
        Value::Float(f) if f.is_nan() => None,
        Value::Float(f) if f.is_infinite() => {
            return Err(AppError::new(
                "observer_state_invalid",
                "Stored observation authority is not finite",
            ))
        }
        Value::Float(f) => Some(f.trunc().clamp(i64::MIN as f64, i64::MAX as f64) as i64),
        Value::Str(text) => text
            .as_str()
            .and_then(|text| kassiber_pytext::parse_int(text).ok())
            .map(|n| i64::try_from(n).unwrap_or(i64::MAX)),
        _ => None,
    })
}

/// `row_has_current_authoritative_observation(row)`.
pub(crate) fn current_authority(row: &Value) -> Result<bool> {
    if authority_version(field(row, "observation_authority_version"))? != Some(1) {
        return Ok(false);
    }
    let default = Value::from("{}");
    let raw = get(row, "raw_json").unwrap_or(&default);
    let graph = match raw {
        Value::Str(text) => crate::observer_state::graph_hash(&py_str(&Value::Str(text.clone())))?,
        other => crate::observer_state::graph_hash_value(other)?,
    };
    Ok(or_text(field(row, "observation_graph_hash")) == graph
        && or_text(field(row, "observation_quantity_hash"))
            == crate::observer_state::quantity_hash(row)?)
}

/// `parse_iso_datetime_or_none`, as microseconds since 0001-01-01 UTC.
fn observed_instant(value: &Value) -> Option<i64> {
    if blank(value) {
        return None;
    }
    let raw = py_str(value);
    let raw = kassiber_pytext::strip(&raw);
    if raw.is_empty() {
        return None;
    }
    kassiber_chain::parse_iso_time(raw)
}

/// `capture_confirmation_observation(row)`.
pub(crate) fn confirmation_observation(row: &Value) -> Result<Option<(Physical, i64, bool)>> {
    if !current_authority(row)? {
        return Ok(None);
    }
    let payload = stored_tx_mapping(field(row, "raw_json"), true);
    let Some(confirmed) = status_confirmed(payload.as_ref()) else {
        return Ok(None);
    };
    let scope = conflict_scope(row)?;
    let instant = observed_instant(field(row, "observation_observed_at"));
    Ok(match (scope, instant) {
        (Some(scope), Some(instant)) => Some((scope, instant, confirmed)),
        _ => None,
    })
}

/// `_current_confirmation_states(rows)`.
pub(crate) fn confirmation_states(rows: &[Value]) -> Result<BTreeMap<Physical, bool>> {
    let mut legacy: BTreeMap<Physical, bool> = BTreeMap::new();
    let mut observed: BTreeMap<Physical, Vec<(i64, bool)>> = BTreeMap::new();
    for row in rows {
        let payload = stored_tx_mapping(field(row, "raw_json"), true);
        let Some(physical) = conflict_scope(row)? else {
            continue;
        };
        let confirmed = status_confirmed(payload.as_ref())
            .unwrap_or_else(|| truthy(field(row, "confirmed_at")));
        let entry = legacy.entry(physical.clone()).or_insert(false);
        *entry = *entry || confirmed;
        if let Some((scope, instant, confirmed)) = confirmation_observation(row)? {
            if scope == physical {
                observed
                    .entry(physical)
                    .or_default()
                    .push((instant, confirmed));
            }
        }
    }
    for (physical, evidence) in observed {
        let latest = evidence.iter().map(|(at, _)| *at).max().expect("nonempty");
        legacy.insert(
            physical,
            evidence
                .iter()
                .filter(|(at, _)| *at == latest)
                .all(|(_, confirmed)| *confirmed),
        );
    }
    Ok(legacy)
}

#[cfg(test)]
mod tests;

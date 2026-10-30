//! Closed chain-confirmation observations (`capture_confirmation_observation`).
//! Saved-report dependency capture freezes these; it never repairs evidence.
use crate::codec::{self, text, truth};
use crate::graph;
use crate::python_text::PythonText;
use serde_json::{json, Value};
mod liquid;

/// `(chain, network, txid)` of one current authoritative observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Closed {
    pub chain: String,
    pub network: String,
    pub txid: String,
    pub confirmed: bool,
}

/// Why a row cannot be decided natively. Each refuses rather than guessing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Evidence(String),
}

fn absent(v: &Value) -> bool {
    v.is_null() || v == ""
}
/// `stored_tx_mapping(raw_json, allow_nested=...)`.
pub fn stored_mapping(raw: &Value, nested: bool) -> Option<Value> {
    let parsed = match raw {
        Value::Object(_) => raw.clone(),
        Value::String(s) => {
            codec::to_json(&codec::loads(if s.is_empty() { "{}" } else { s }).ok()?)
        }
        v if !truth(v) => json!({}),
        _ => return None,
    };
    if !parsed.is_object() {
        return None;
    }
    if nested && parsed["tx"].is_object() {
        return Some(parsed["tx"].clone());
    }
    Some(parsed)
}
/// `transfers.canonical_txid`.
fn canonical_txid(v: &Value) -> Option<String> {
    let s = crate::python_text::strip(&text(v)).to_owned();
    (s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}
fn canonical_chain(v: &Value) -> Option<&'static str> {
    match text(v).py_strip().py_lower().as_str() {
        "bitcoin" | "btc" | "onchain" => Some("bitcoin"),
        "liquid" | "lbtc" | "elements" => Some("liquid"),
        _ => None,
    }
}
fn canonical_network(chain: &str, v: &Value) -> Option<&'static str> {
    let t = text(v).py_strip().py_lower();
    if chain == "bitcoin" {
        match t.as_str() {
            "bitcoin" | "main" | "mainnet" => Some("main"),
            "test" | "testnet" => Some("test"),
            "regtest" => Some("regtest"),
            "signet" => Some("signet"),
            _ => None,
        }
    } else {
        match t.as_str() {
            "liquid" | "liquidv1" | "main" | "mainnet" => Some("liquidv1"),
            "liquidtestnet" | "test" | "testnet" => Some("liquidtestnet"),
            "elements" | "elementsregtest" | "regtest" => Some("elementsregtest"),
            _ => None,
        }
    }
}
fn liquid_domain(network: &str) -> Option<&'static str> {
    match network {
        "liquidv1" => Some("main"),
        "liquidtestnet" => Some("test"),
        "elementsregtest" => Some("regtest"),
        _ => None,
    }
}
fn config(row: &Value) -> Value {
    graph::transfer_object(if truth(&row["config_json"]) {
        &row["config_json"]
    } else {
        &row["wallet_config_json"]
    })
}
fn asset(row: &Value) -> String {
    text(&row["asset"])
        .py_strip()
        .py_upper()
        .replace("L-BTC", "LBTC")
}

/// `bitcoin_network_domain_evidence` once transfer scope has already failed.
fn domain_evidence(row: &Value) -> (Option<&'static str>, bool) {
    let raw = graph::transfer_object(&row["raw_json"]);
    let config = config(row);
    let mut families = std::collections::BTreeSet::new();
    for payload in [&raw, &config] {
        let value = &payload["chain"];
        if absent(value) {
            continue;
        }
        if let Some(chain) = canonical_chain(value) {
            families.insert(chain);
            continue;
        }
        let mut kind = text(value).py_strip().py_lower().replace('_', "-");
        if ["core-ln", "core-lightning"].contains(&kind.as_str()) {
            kind = "coreln".into();
        }
        if !crate::evidence::lightning(&kind) {
            return (None, false);
        }
        families.insert("bitcoin");
    }
    if families.len() > 1 {
        return (None, false);
    }
    let asset = asset(row);
    if (asset == "BTC" && families.contains("liquid"))
        || (asset == "LBTC" && families.contains("bitcoin"))
    {
        return (None, false);
    }
    let mut domains = std::collections::BTreeSet::new();
    for payload in [&raw, &config] {
        for key in ["network", "bitcoin_network", "chain_network"] {
            let value = &payload[key];
            if absent(value) {
                continue;
            }
            if let Some(network) = canonical_network("bitcoin", value) {
                domains.insert(network);
            } else if let Some(network) = canonical_network("liquid", value) {
                domains.insert(liquid_domain(network).unwrap());
            } else {
                return (None, false);
            }
        }
    }
    if domains.len() > 1 {
        return (None, false);
    }
    (domains.into_iter().next(), true)
}

/// `_physical_conflict_scope(row, _parse_onchain_tx(raw_json, allow_partial=True))`.
pub fn physical_scope(row: &Value) -> Result<Option<(String, String, String)>, Refusal> {
    if let Some((chain, network, txid, _)) = graph::onchain_scope(row) {
        return Ok(Some((chain, network, txid)));
    }
    let id = text(&row["id"]);
    if [
        "owned-derive:",
        "recorded-fanout:",
        "cross-split:",
        "direct-payout:",
        "multi-consol:",
    ]
    .iter()
    .any(|prefix| id.starts_with(prefix))
    {
        let mut probe = row.clone();
        probe["id"] = json!(format!("physical-anchor:{id}"));
        if let Some((chain, network, txid, _)) = graph::onchain_scope(&probe) {
            return Ok(Some((chain, network, txid)));
        }
    }
    let Some(outer) = stored_mapping(&row["raw_json"], false) else {
        return Ok(None);
    };
    let chain = text(&outer["chain"]).py_strip().py_lower();
    let looks_liquid = chain == "liquid"
        || (outer["component"].is_object() && truth(&outer["component"]["asset_id"]));
    let graphs: Vec<&Value> = [&outer, &outer["tx"], &outer["ownership_graph"]]
        .into_iter()
        .filter(|g| g.is_object() && g["vin"].is_array() && g["vout"].is_array())
        .collect();
    let decodable = looks_liquid && outer["raw_hex"].as_str().is_some_and(liquid::decodable);
    if graphs.is_empty() && !decodable {
        return Ok(None);
    }
    // merge_ownership_txs: the first item's txid, then the first non-empty one.
    let mut merged = graphs
        .first()
        .map(|g| g["txid"].clone())
        .unwrap_or(Value::Null);
    for g in &graphs {
        if absent(&merged) && !absent(&g["txid"]) {
            merged = g["txid"].clone();
        }
    }
    let parsed = if truth(&outer["txid"]) {
        outer["txid"].clone()
    } else {
        merged
    };
    let scope = (|| {
        let raw_txid = text(&parsed).py_strip().to_owned();
        let parsed_txid = canonical_txid(&json!(raw_txid));
        if !raw_txid.is_empty() && parsed_txid.is_none() {
            return None;
        }
        let external = canonical_txid(&row["external_id"]);
        if parsed_txid.is_some() && external.is_some() && parsed_txid != external {
            return None;
        }
        let txid = parsed_txid.or(external)?;
        let chain = match asset(row).as_str() {
            "BTC" => "bitcoin",
            "LBTC" => "liquid",
            _ => return None,
        };
        let (domain, valid) = domain_evidence(row);
        if !valid {
            return None;
        }
        let domain = match domain {
            Some(domain) => domain,
            None if chain == "bitcoin" => "main",
            None => return None,
        };
        Some((chain.to_owned(), domain.to_owned(), txid))
    })();
    Ok(scope)
}

/// `capture_confirmation_observation`; `observed` is the parsed instant.
pub fn closed(row: &Value, observed: bool) -> Result<Option<Closed>, Refusal> {
    if !crate::evidence::current_observation(row).map_err(|e| Refusal::Evidence(e.to_string()))? {
        return Ok(None);
    }
    let status = stored_mapping(&row["raw_json"], true).unwrap_or(json!({}))["status"].clone();
    let Some(confirmed) = status
        .as_object()
        .and_then(|s| s.get("confirmed")?.as_bool())
    else {
        return Ok(None);
    };
    // Python requires both scope and instant; a missing instant decides first.
    if !observed {
        return Ok(None);
    }
    let Some((chain, network, txid)) = physical_scope(row)? else {
        return Ok(None);
    };
    Ok(Some(Closed {
        chain,
        network,
        txid,
        confirmed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conflict_scope_falls_back_only_on_unambiguous_identity() {
        let txid = "A".repeat(64);
        let graphless = json!({"id":"r","asset":"BTC","external_id":txid,"raw_json":"{\"vin\":[],\"vout\":[]}"});
        assert_eq!(
            physical_scope(&graphless).unwrap(),
            Some(("bitcoin".into(), "main".into(), "a".repeat(64)))
        );
        let liquid = json!({"id":"r","asset":"LBTC","external_id":txid,"raw_json":"{\"chain\":\"liquid\",\"network\":\"liquidtestnet\",\"vin\":[],\"vout\":[]}"});
        assert_eq!(
            physical_scope(&liquid).unwrap(),
            Some(("liquid".into(), "test".into(), "a".repeat(64)))
        );
        let contradictory = json!({"id":"r","asset":"LBTC","external_id":txid,"raw_json":"{\"chain\":\"bitcoin\",\"vin\":[],\"vout\":[]}"});
        assert_eq!(physical_scope(&contradictory).unwrap(), None);
        let hex_only = json!({"id":"r","asset":"LBTC","external_id":txid,"raw_json":"{\"chain\":\"liquid\",\"network\":\"liquidv1\",\"raw_hex\":\"00\"}"});
        assert_eq!(physical_scope(&hex_only).unwrap(), None);
        let undecidable_anyway = json!({"id":"r","asset":"USD","external_id":txid,"raw_json":"{\"chain\":\"liquid\",\"raw_hex\":\"00\"}"});
        assert_eq!(physical_scope(&undecidable_anyway).unwrap(), None);
    }
}

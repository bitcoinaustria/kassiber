//! Recorded candidates preserve Python dict insertion order. Labels identify a
//! candidate's origin; the claim compiler independently revalidates its proof.
use super::{anchor, pairs, Context};
use crate::fields::{Fields, Row};
use crate::{
    codec::{text, truth},
    evidence, graph,
    python_text::PythonText,
    snapshot::Snapshot,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(super) fn canonical_hash(value: &Value) -> Option<String> {
    let raw = text(value);
    let raw = raw.py_strip();
    (raw.len() == 64 && raw.bytes().all(|b| b.is_ascii_hexdigit())).then(|| raw.py_lower())
}
fn normalized(value: &Value) -> String {
    text(value).py_strip().py_lower()
}
fn wallet_kind(value: &Value) -> String {
    let kind = normalized(value).replace('_', "-");
    match kind.as_str() {
        "core-ln" | "core-lightning" => "coreln".into(),
        _ => kind,
    }
}
pub(super) fn lightning_row<R: Row + ?Sized>(row: &R) -> bool {
    let source = normalized(row.at("payment_hash_source"));
    if !matches!(source.as_str(), "lnd" | "core_lightning") {
        return false;
    }
    let kind = normalized(row.at("kind"));
    let raw = graph::transfer_object(row.at("raw_json"));
    let import = normalized(&raw["_kassiber_provenance"]["import_source"]);
    match source.as_str() {
        "lnd" => import == "lnd" && matches!(kind.as_str(), "lnd_invoice" | "lnd_pay"),
        "core_lightning" => {
            import == "core-lightning"
                && matches!(
                    kind.as_str(),
                    "cln_invoice" | "cln_pay" | "ln_invoice" | "ln_pay"
                )
        }
        _ => false,
    }
}
fn chain(value: &Value) -> Option<&'static str> {
    match normalized(value).as_str() {
        "bitcoin" | "btc" | "onchain" => Some("bitcoin"),
        "liquid" | "lbtc" | "elements" => Some("liquid"),
        _ => None,
    }
}
fn network(c: &str, value: &Value) -> Option<String> {
    // Transfer metadata treats whitespace-only values as unsupported here;
    // the evidence normalizer's empty-network default does not apply.
    if normalized(value).is_empty() {
        return None;
    }
    evidence::network(c, value).ok()
}
pub(super) fn domain_name(network: &str) -> &str {
    match network {
        "liquidv1" => "main",
        "liquidtestnet" => "test",
        "elementsregtest" => "regtest",
        _ => network,
    }
}
pub(super) fn network_domain<R: Row + ?Sized>(row: &R) -> Option<String> {
    if let Some((_, network, _, _)) = row.onchain_scope() {
        return Some(domain_name(&network).to_owned());
    }
    let raw = graph::transfer_object(row.at("raw_json"));
    let config = graph::transfer_object(if truth(row.at("config_json")) {
        row.at("config_json")
    } else {
        row.at("wallet_config_json")
    });
    let mut families = BTreeSet::new();
    let mut domains = BTreeSet::new();
    for payload in [&raw, &config] {
        let value = &payload["chain"];
        if !value.is_null() && value != "" {
            let family = if let Some(c) = chain(value) {
                c
            } else if evidence::lightning(&wallet_kind(value)) {
                "bitcoin"
            } else {
                return None;
            };
            families.insert(family);
        }
        for key in ["network", "bitcoin_network", "chain_network"] {
            let value = &payload[key];
            if value.is_null() || value == "" {
                continue;
            }
            let n = network("bitcoin", value).or_else(|| network("liquid", value))?;
            domains.insert(domain_name(&n).to_owned());
        }
    }
    let asset = text(row.at("asset"))
        .py_strip()
        .py_upper()
        .replace("L-BTC", "LBTC");
    if families.len() > 1
        || (asset == "BTC" && families.contains("liquid"))
        || (asset == "LBTC" && families.contains("bitcoin"))
        || domains.len() != 1
    {
        return None;
    }
    domains.into_iter().next()
}
fn append_groups<R: Row>(
    groups: Vec<Vec<&R>>,
    source: &str,
    pairs: &mut Vec<Value>,
    matched: &mut BTreeSet<String>,
) -> Result<(), String> {
    // Python compares the raw value with zero before its later int() call.
    let positive = |row: &&R| -> Result<bool, String> {
        let value = row.at("amount");
        if !truth(value) {
            return Ok(false);
        }
        if !value.is_number() && !value.is_boolean() {
            let kind = match value {
                _ if crate::codec::raw_bytes(value).is_some() => "bytes",
                Value::String(_) => "str",
                Value::Array(_) => "list",
                _ => "dict",
            };
            return Err(format!(
                "TypeError: '>' not supported between instances of '{kind}' and 'int'"
            ));
        }
        Ok(value.as_bool().unwrap_or_else(|| {
            value
                .as_number()
                .is_some_and(|n| !n.to_string().starts_with('-'))
        }))
    };
    for group in groups {
        let outs: Vec<_> = group
            .iter()
            .filter(|r| r.at("direction") == "outbound")
            .map(|r| Ok(positive(r)?.then_some(r)))
            .collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .flatten()
            .collect();
        let ins: Vec<_> = group
            .iter()
            .filter(|r| r.at("direction") == "inbound")
            .map(|r| Ok(positive(r)?.then_some(r)))
            .collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .flatten()
            .collect();
        if outs.len() != 1 || ins.len() != 1 {
            continue;
        }
        let (out, incoming) = (*outs[0], *ins[0]);
        if source == "row_matched" {
            if out.at("wallet_id") == incoming.at("wallet_id") {
                continue;
            }
        } else if super::integer_or_zero(out.at("amount"))?
            != super::integer_or_zero(incoming.at("amount"))?
        {
            continue;
        }
        let mut pair = serde_json::Map::new();
        pair.insert("out".into(), out.json().into_owned());
        pair.insert("in".into(), incoming.json().into_owned());
        pair.insert("source".into(), json!(source));
        pairs.push(Value::Object(pair));
        matched.extend([text(out.at("id")), text(incoming.at("id"))]);
    }
    Ok(())
}
type Groups<'a, K, R> = Vec<(K, Vec<&'a R>)>;

pub(super) fn detect<R: Row>(rows: &[R]) -> Result<(Vec<Value>, BTreeSet<String>), String> {
    let mut chain_groups: Groups<(String, String, String, String), R> = Vec::new();
    let mut index = std::collections::HashMap::new();
    for row in rows {
        if super::holds::invalid_transfer_identity(row) {
            continue;
        }
        if let Some(scope) = row.onchain_scope() {
            crate::codec::push_grouped(&mut chain_groups, &mut index, scope, row);
        }
    }
    let mut pairs = Vec::new();
    let mut matched = BTreeSet::new();
    append_groups(
        chain_groups.into_iter().map(|(_, group)| group).collect(),
        "row_matched",
        &mut pairs,
        &mut matched,
    )?;
    let mut hash_groups: Groups<(String, Value, String), R> = Vec::new();
    // Asset values are keyed by their encoding, which is injective.
    let mut hash_index: std::collections::HashMap<(String, String, String), usize> =
        std::collections::HashMap::new();
    for row in rows {
        if matched.contains(&text(row.at("id"))) || !lightning_row(row) {
            continue;
        }
        let (Some(hash), Some(domain)) =
            (canonical_hash(row.at("payment_hash")), network_domain(row))
        else {
            continue;
        };
        let key = (hash, row.at("asset").clone(), domain);
        let encoded = (key.0.clone(), key.1.to_string(), key.2.clone());
        if let Some(&at) = hash_index.get(&encoded) {
            hash_groups[at].1.push(row);
        } else {
            hash_index.insert(encoded, hash_groups.len());
            hash_groups.push((key, vec![row]));
        }
    }
    append_groups(
        hash_groups.into_iter().map(|(_, group)| group).collect(),
        "lightning_payment_hash",
        &mut pairs,
        &mut matched,
    )?;
    Ok((pairs, matched))
}
pub(super) fn exact_native(source: &Value, target: &Value) -> bool {
    truth(&source["authoritative_chain_observation"])
        && truth(&target["authoritative_chain_observation"])
        && source["event_key"] == target["event_key"]
        && source["event_key"]["native_namespace"] == "chain"
        && matches!(
            source["event_key"]["chain"].as_str(),
            Some("bitcoin" | "liquid")
        )
}
pub(super) fn exact_recorded(pair: &Value, source: &Value, target: &Value) -> bool {
    match pairs::source_name(pair).as_str() {
        "row_matched" => exact_native(source, target) && source["wallet_id"] != target["wallet_id"],
        "lightning_payment_hash" => {
            let out = &pair["out"];
            let incoming = &pair["in"];
            let hash = canonical_hash(&out["payment_hash"]);
            let domain = network_domain(out);
            hash.is_some()
                && hash == canonical_hash(&incoming["payment_hash"])
                && lightning_row(out)
                && lightning_row(incoming)
                && source["asset"] == target["asset"]
                && super::claims::principal(source) == super::claims::principal(target)
                && domain.is_some()
                && domain == network_domain(incoming)
        }
        _ => false,
    }
}
pub(super) fn resolved_ids(pairs: &[Value], context: &Context, complete: bool) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for pair in pairs {
        let (out, incoming) = (anchor(&pair["out"]), anchor(&pair["in"]));
        let (Some(source), Some(target)) = (
            context.observations.get(&out),
            context.observations.get(&incoming),
        ) else {
            continue;
        };
        if source["direction"] != "outbound" || target["direction"] != "inbound" {
            continue;
        }
        let exact = if complete {
            super::claims::principal(source) == super::claims::principal(target)
                && exact_recorded(pair, source, target)
        } else {
            exact_native(source, target)
        };
        if exact {
            ids.extend([out, incoming]);
        }
    }
    ids
}
pub(super) fn compile(snapshot: &Snapshot, context: &Context) -> Result<Value, String> {
    let rows: Vec<_> = snapshot.transactions.iter().map(|r| json!(r)).collect();
    let (candidates, matched) = detect(&rows)?;
    let compiled = pairs::compile(&candidates, context, snapshot)?;
    Ok(
        json!({"pairs":candidates,"matched_ids":matched,"claims":compiled["claims"],"blocked_transaction_ids":compiled["blocked_transaction_ids"],"quarantines":compiled["quarantines"],
        "exact_native_pair_ids":resolved_ids(&candidates, context, false),"complete_recorded_pair_ids":resolved_ids(&candidates, context, true),
        "native_audits":pairs::native_audits(&candidates, context)?,
        "physical_scopes":rows.iter().map(graph::onchain_scope).collect::<Vec<_>>(),
        "network_domains":rows.iter().map(network_domain).collect::<Vec<_>>(),
        "lightning_rows":rows.iter().map(lightning_row).collect::<Vec<_>>() }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recorded_cpython_transfer_predicates() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_transfer_cases.json"
        ))
        .unwrap();
        for v in vectors {
            let rows = super::super::array(&v["rows"]);
            if !v["error_code"].is_null() {
                let error = detect(rows).unwrap_err();
                assert_eq!(
                    error.split(": ").next().unwrap(),
                    v["error_code"],
                    "{}",
                    v["case"]
                );
                continue;
            }
            assert_eq!(
                json!(rows.iter().map(graph::onchain_scope).collect::<Vec<_>>()),
                v["scopes"],
                "{} scopes",
                v["case"]
            );
            assert_eq!(
                json!(rows.iter().map(network_domain).collect::<Vec<_>>()),
                v["domains"],
                "{} domains",
                v["case"]
            );
            assert_eq!(
                json!(rows.iter().map(lightning_row).collect::<Vec<_>>()),
                v["lightning"],
                "{} native",
                v["case"]
            );
            let (pairs, matched) = detect(rows).unwrap();
            assert_eq!(json!(pairs), v["pairs"], "{} pairs", v["case"]);
            assert_eq!(json!(matched), v["matched_ids"], "{} matched", v["case"]);
        }
    }
}

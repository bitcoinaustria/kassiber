//! OwnedIndex lookup precedence and graph scope recovery, from raw index facts.
use super::{parse, strict_txid};
use crate::{
    codec::{self, object, text, truth},
    python_text::PythonText,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type PhysicalScope = (String, String);
pub(crate) type TransferScope = (String, String, String, String);

fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn chain(value: &Value) -> Option<String> {
    match text(value).py_strip().py_lower().as_str() {
        "" | "bitcoin" | "btc" | "onchain" => Some("bitcoin".into()),
        "liquid" | "lbtc" | "elements" => Some("liquid".into()),
        _ => None,
    }
}
fn network(chain: &str, value: &Value) -> Option<String> {
    crate::evidence::network(chain, value).ok()
}
pub(crate) fn normalized(chain_value: &Value, network_value: &Value) -> PhysicalScope {
    if let Some(c) = chain(chain_value) {
        if let Some(n) = network(&c, network_value) {
            return (c, n);
        }
    }
    (
        text(chain_value).py_strip().py_lower(),
        text(network_value).py_strip().py_lower(),
    )
}
fn match_scope(item: &Value) -> PhysicalScope {
    normalized(&item["chain"], &item["network"])
}
fn index_scope(scope: &PhysicalScope) -> PhysicalScope {
    let c = if scope.0.is_empty() {
        "bitcoin"
    } else {
        &scope.0
    };
    normalized(&json!(c), &json!(scope.1))
}

#[derive(Clone, Copy)]
pub(crate) struct Index<'a>(pub Option<&'a Value>);
impl Index<'_> {
    pub fn script(&self, script: &Value) -> Result<Vec<Value>, String> {
        if !truth(script) {
            return Ok(Vec::new());
        }
        let key = script.as_str().ok_or("AttributeError")?.py_lower();
        Ok(self
            .0
            .map(|index| array(&index["by_script"][&key]).to_vec())
            .unwrap_or_default())
    }
    pub fn outpoint(&self, outpoint: &Value, scope: Option<&PhysicalScope>) -> Vec<Value> {
        let Some(index) = self.0 else {
            return Vec::new();
        };
        let key = text(outpoint).py_lower();
        if key.is_empty() {
            return Vec::new();
        }
        let scope = scope.map(index_scope);
        let scoped = scope.as_ref().and_then(|s| {
            array(&index["by_scoped_outpoint"])
                .iter()
                .rev()
                .find(|v| v["key"] == json!([s.0, s.1, key]))
        });
        // An explicitly empty scoped list suppresses the legacy outpoint map.
        let mut result = if let Some(row) = scoped {
            array(&row["matches"]).to_vec()
        } else {
            array(&index["by_outpoint"][&key])
                .iter()
                .filter(|m| {
                    scope.as_ref().is_none_or(|s| {
                        // OwnedIndex stamps legacy blank chain before fallback normalization.
                        let c = if truth(&m["chain"]) {
                            m["chain"].clone()
                        } else {
                            json!("bitcoin")
                        };
                        normalized(&c, &m["network"]) == *s
                    })
                })
                .cloned()
                .collect()
        };
        result.sort_by_key(|m| {
            let canonical = matches!(
                m["wallet_kind"].as_str(),
                Some("descriptor" | "xpub" | "silent-payment")
            );
            let priority = match m["source"].as_str() {
                Some("derived") => 0,
                Some("inventory") => 1,
                Some("address_list") => 2,
                Some("derived_history") => 3,
                Some("address_history") => 4,
                _ => 9,
            };
            (
                !canonical,
                priority,
                text(&m["wallet_label"]),
                text(&m["wallet_id"]),
            )
        });
        result
    }
    pub fn txid(&self, txid: &Value, scope: Option<&PhysicalScope>) -> BTreeSet<(String, String)> {
        let Some(index) = self.0 else {
            return BTreeSet::new();
        };
        let key = text(txid).py_lower();
        if key.is_empty() {
            return BTreeSet::new();
        }
        let rows = if let Some(scope) = scope {
            let scope = index_scope(scope);
            array(&index["txid_wallets_by_scope"])
                .iter()
                .rev()
                .find(|row| row["key"] == json!([scope.0, scope.1, key]))
                .map(|row| array(&row["matches"]))
                .unwrap_or(&[])
        } else {
            array(&index["txid_wallets"][&key])
        };
        rows.iter()
            .map(|v| (codec::pystr(&v[0]), codec::pystr(&v[1])))
            .collect()
    }
    pub fn input_owners(
        &self,
        entry: &Value,
        scope: &PhysicalScope,
    ) -> Result<BTreeSet<String>, String> {
        if truth(&entry["outpoint"]) {
            let matches = self.outpoint(&entry["outpoint"], Some(scope));
            if !matches.is_empty() {
                return Ok(matches
                    .iter()
                    .map(|m| codec::pystr(&m["wallet_id"]))
                    .collect());
            }
        }
        self.script_owners(&entry["script"], scope)
    }
    pub fn script_owners(
        &self,
        script: &Value,
        scope: &PhysicalScope,
    ) -> Result<BTreeSet<String>, String> {
        Ok(self
            .script(script)?
            .iter()
            .filter(|m| match_scope(m) == *scope)
            .map(|m| codec::pystr(&m["wallet_id"]))
            .collect())
    }
    pub fn source_scope(
        &self,
        inputs: &[Value],
        wallet: &str,
    ) -> Result<Option<PhysicalScope>, String> {
        let mut scopes = BTreeSet::new();
        for entry in inputs {
            let mut matches = if truth(&entry["outpoint"]) {
                self.outpoint(&entry["outpoint"], None)
            } else {
                Vec::new()
            };
            if matches.is_empty() {
                matches = self.script(&entry["script"])?;
            }
            for m in matches
                .iter()
                .filter(|m| codec::pystr(&m["wallet_id"]) == wallet)
            {
                scopes.insert(match_scope(m));
            }
        }
        Ok((scopes.len() == 1).then(|| scopes.into_iter().next().unwrap()))
    }
}

pub(crate) fn parsed_scope<R: crate::fields::Row + ?Sized>(
    graph: &Value,
    index: Index<'_>,
    row: &R,
) -> Result<Option<PhysicalScope>, String> {
    let wallet = text(row.at("wallet_id"));
    let mut inferred = index.source_scope(array(&graph["inputs"]), &wallet)?;
    if inferred.is_none() {
        let mut scopes = BTreeSet::new();
        for output in array(&graph["outputs"]) {
            for m in index.script(&output["script"])? {
                if codec::pystr(&m["wallet_id"]) == wallet {
                    scopes.insert(match_scope(&m));
                }
            }
        }
        if scopes.len() == 1 {
            inferred = scopes.into_iter().next();
        }
    }
    let config = object(if truth(row.at("config_json")) {
        row.at("config_json")
    } else {
        row.at("wallet_config_json")
    });
    let mut chains = BTreeSet::new();
    for v in [&graph["chain"], &config["chain"]] {
        if !text(v).py_strip().is_empty() {
            let Some(c) = chain(v) else {
                return Ok(None);
            };
            chains.insert(c);
        }
    }
    if chains.len() > 1 {
        return Ok(None);
    }
    let explicit_chain = chains.into_iter().next();
    let network_chain = explicit_chain
        .clone()
        .or_else(|| inferred.as_ref().map(|s| s.0.clone()))
        .unwrap_or_else(|| {
            if matches!(
                text(row.at("asset")).py_strip().py_upper().as_str(),
                "LBTC" | "L-BTC"
            ) {
                "liquid"
            } else {
                "bitcoin"
            }
            .into()
        });
    let mut networks = BTreeSet::new();
    for v in [&graph["network"], &config["network"]] {
        if !text(v).py_strip().is_empty() {
            let Some(n) = network(&network_chain, v) else {
                return Ok(None);
            };
            networks.insert(n);
        }
    }
    if networks.len() > 1 {
        return Ok(None);
    }
    let explicit_network = networks.into_iter().next();
    if let Some(inferred) = inferred {
        if explicit_chain.as_ref().is_some_and(|v| v != &inferred.0)
            || explicit_network.as_ref().is_some_and(|v| v != &inferred.1)
        {
            return Ok(None);
        }
        return Ok(Some(inferred));
    }
    if let Some(c) = explicit_chain {
        if let Some(n) = explicit_network {
            return Ok(Some((c, n)));
        }
        if c == "bitcoin" {
            return Ok(Some(normalized(&json!(c), &Value::Null)));
        }
    }
    Ok(None)
}

pub(crate) fn scope<R: crate::fields::Row + ?Sized>(
    row: &R,
    graph: Option<&Value>,
    index: Index<'_>,
) -> Result<Option<TransferScope>, String> {
    // Python parses the raw graph before trying the ordinary identity predicate.
    let parsed;
    let graph = if graph.is_some_and(truth) {
        graph
    } else {
        parsed = parse(row.at("raw_json"))?;
        parsed.as_ref()
    };
    if let Some(scope) = row.onchain_scope() {
        return Ok(Some(scope));
    }
    let Some(graph) = graph.filter(|_| index.0.is_some()) else {
        return Ok(None);
    };
    let raw_id = text(&graph["txid"]).py_strip().to_owned();
    let parsed_id = strict_txid(&json!(raw_id));
    if !raw_id.is_empty() && parsed_id.is_none() {
        return Ok(None);
    }
    let external = strict_txid(row.at("external_id"));
    if parsed_id.is_some() && external.is_some() && parsed_id != external {
        return Ok(None);
    }
    let txid = parsed_id.or(external);
    let physical = parsed_scope(graph, index, row)?;
    let (Some(txid), Some((c, n))) = (txid, physical) else {
        return Ok(None);
    };
    let asset_id = text(&graph["component"]["asset_id"]).py_strip().py_lower();
    let display = if truth(&graph["component"]["asset"]) {
        text(&graph["component"]["asset"])
    } else {
        text(row.at("asset"))
    };
    let display = display.py_strip().py_upper().replace("L-BTC", "LBTC");
    if display == "BTC" && c != "bitcoin" || display == "LBTC" && c != "liquid" {
        return Ok(None);
    }
    let identity = if c == "liquid" {
        if strict_txid(&json!(asset_id)).is_none() {
            return Ok(None);
        }
        asset_id
    } else {
        display
    };
    Ok(Some((c, n, txid, identity)))
}

pub(crate) fn profile_parsed_typed<R: crate::fields::Row>(
    rows: &[R],
    index: Index<'_>,
) -> Result<BTreeMap<String, kassiber_pyjson::Value>, String> {
    let mut parsed_rows = Vec::new();
    let mut parsed_positions = std::collections::HashMap::new();
    let mut groups: BTreeMap<(String, String, String), Vec<kassiber_pyjson::Value>> =
        BTreeMap::new();
    for row in rows {
        let Some(mut raw_graph) = super::stored::parse(row.at("raw_json"))? else {
            continue;
        };
        let graph = super::stored::project(&raw_graph);
        let Some((c, n, txid, _)) = scope(row, Some(&graph), index)? else {
            continue;
        };
        super::stored::put(&mut raw_graph, "chain", codec::from_json(&json!(c))?);
        super::stored::put(&mut raw_graph, "network", codec::from_json(&json!(n))?);
        let key = (c, n, txid);
        groups
            .entry(key.clone())
            .or_default()
            .push(raw_graph.clone());
        let id = codec::pystr(row.at("id"));
        // A repeated id replaces its first entry in place.
        if let Some(&at) = parsed_positions.get(&id) {
            let (_, old_graph, old_key) = &mut parsed_rows[at];
            *old_graph = raw_graph;
            *old_key = key;
        } else {
            parsed_positions.insert(id.clone(), parsed_rows.len());
            parsed_rows.push((id, raw_graph, key));
        }
    }
    let mut result = BTreeMap::new();
    for (id, graph, key) in parsed_rows {
        let mut items = vec![graph.clone()];
        items.extend(groups[&key].iter().cloned());
        let mut merged = super::stored::merge(&items).unwrap_or_else(|| graph.clone());
        super::stored::put(
            &mut merged,
            "component",
            super::stored::get(&graph, "component").clone(),
        );
        super::stored::put(&mut merged, "chain", codec::from_json(&json!(key.0))?);
        super::stored::put(&mut merged, "network", codec::from_json(&json!(key.1))?);
        result.insert(id, merged);
    }
    Ok(result)
}

/// Only the comparison seam projects nonfinite scalars to tagged JSON.
pub(crate) fn profile_parsed(
    rows: &[Value],
    index: Index<'_>,
) -> Result<BTreeMap<String, Value>, String> {
    Ok(profile_parsed_typed(rows, index)?
        .into_iter()
        .map(|(id, graph)| (id, super::stored::project(&graph)))
        .collect())
}

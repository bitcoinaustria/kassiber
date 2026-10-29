//! The nonheuristic matcher projection consumed by native custody compilation.
mod providers;
use super::{htlc, recorded};
use crate::{codec, graph, graph::stored as s, python_text::PythonText};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

pub(super) fn number(n: &BigInt) -> Value {
    serde_json::from_str(&n.to_string()).expect("computed integer")
}
pub(super) fn amount(row: &Value) -> Result<BigInt, String> {
    let v = &row["amount"];
    if !codec::truth(v) {
        Ok(BigInt::from(0))
    } else {
        s::integer(&codec::from_json(v)?).map_err(str::to_owned)
    }
}
pub(super) fn fee_threshold(n: &BigInt) -> Result<BigInt, String> {
    // Python int * float converts before multiplication, then int truncates.
    let f = n.to_string().parse::<f64>().map_err(|_| "OverflowError")?;
    if !f.is_finite() {
        return Err("OverflowError".into());
    }
    let f = f.abs() * 0.01;
    let value = BigInt::from_str(&format!("{:.0}", f.trunc())).map_err(|_| "OverflowError")?;
    Ok(value.max(BigInt::from(2_500_000)))
}
pub(super) fn conserves(out: &Value, incoming: &Value) -> Result<bool, String> {
    let a = amount(out)?;
    let b = amount(incoming)?;
    Ok(a > BigInt::from(0) && b > BigInt::from(0) && b <= a && &a - &b <= fee_threshold(&a)?)
}
pub(super) fn role(row: &Value) -> Result<&'static str, String> {
    if recorded::lightning_row(row) {
        return Ok("lightning_node");
    }
    Ok(
        match codec::text(&row["payment_hash_source"])
            .py_strip()
            .py_lower()
            .as_str()
        {
            "chain_script_unique_outpoint" => {
                if htlc::claim(row)? {
                    "chain_htlc"
                } else {
                    "chain_htlc_unscoped"
                }
            }
            "chain_script" => "chain_htlc_unscoped",
            "boltz" | "boltz-regtest" | "bullbitcoin" | "generic_ledger" | "importer"
            | "importer_backfill" => "provider_or_import",
            _ => "unknown",
        },
    )
}
pub(super) fn populations<'a>(
    outs: &[&'a Value],
    ins: &[&'a Value],
) -> Vec<(Vec<&'a Value>, Vec<&'a Value>)> {
    // Vec preserves hash/domain encounter order, unlike a sorted map.
    type Domains<'a> = Vec<(Option<String>, [Vec<&'a Value>; 2])>;
    let mut groups: Vec<(String, Domains<'a>)> = Vec::new();
    for (side, rows) in [outs, ins].iter().enumerate() {
        for row in *rows {
            let Some(hash) = recorded::canonical_hash(&row["payment_hash"]) else {
                continue;
            };
            if codec::truth(&row["excluded"]) {
                continue;
            }
            let at = groups
                .iter()
                .position(|(h, _)| *h == hash)
                .unwrap_or_else(|| {
                    groups.push((hash, Vec::new()));
                    groups.len() - 1
                });
            let domains = &mut groups[at].1;
            let domain = recorded::network_domain(row);
            let at = domains
                .iter()
                .position(|(d, _)| *d == domain)
                .unwrap_or_else(|| {
                    domains.push((domain, [Vec::new(), Vec::new()]));
                    domains.len() - 1
                });
            domains[at].1[side].push(row);
        }
    }
    let mut result = Vec::new();
    for (_, domains) in groups {
        let unknown = domains
            .iter()
            .find(|(d, _)| d.is_none())
            .map(|(_, rows)| rows.clone())
            .unwrap_or_default();
        let known: Vec<_> = domains.iter().filter(|(d, _)| d.is_some()).collect();
        if known.is_empty() {
            if !unknown[0].is_empty() && !unknown[1].is_empty() {
                result.push((unknown[0].clone(), unknown[1].clone()));
            }
        } else {
            for (_, rows) in known {
                let mut rows = rows.clone();
                for i in 0..2 {
                    rows[i].extend(&unknown[i]);
                }
                if !rows[0].is_empty() && !rows[1].is_empty() {
                    result.push((rows[0].clone(), rows[1].clone()));
                }
            }
        }
    }
    result
}
fn compatible(a: &Value, b: &Value) -> bool {
    let a = recorded::network_domain(a);
    let b = recorded::network_domain(b);
    a.is_none() || b.is_none() || a == b
}
fn kind(out: &Value, incoming: &Value) -> String {
    if asset(&out["asset"]).is_none() || asset(&incoming["asset"]).is_none() {
        return "manual".into();
    }
    let normalize = |v: &Value| match codec::text(v)
        .py_strip()
        .py_lower()
        .replace('_', "-")
        .as_str()
    {
        "core-ln" | "core-lightning" => "coreln".into(),
        s => s.to_owned(),
    };
    let a = normalize(&out["wallet_kind"]);
    let b = normalize(&incoming["wallet_kind"]);
    let ln = |s: &str| {
        matches!(
            s,
            "cln" | "coreln" | "lightning" | "lnd" | "nwc" | "phoenix"
        )
    };
    let chain = |s: &str| {
        matches!(
            s,
            "address" | "descriptor" | "samourai" | "silent-payment" | "wasabi" | "xpub"
        )
    };
    (if ln(&a) && !ln(&b) {
        "reverse-submarine-swap"
    } else if ln(&b) && !ln(&a) {
        "submarine-swap"
    } else if chain(&a)
        && chain(&b)
        && asset(&out["asset"]) == Some("BTC")
        && asset(&incoming["asset"]) == Some("LBTC")
    {
        "peg-in"
    } else if chain(&a)
        && chain(&b)
        && asset(&out["asset"]) == Some("LBTC")
        && asset(&incoming["asset"]) == Some("BTC")
    {
        "peg-out"
    } else {
        "manual"
    })
    .into()
}
pub(super) fn asset(v: &Value) -> Option<&'static str> {
    match codec::text(v).py_strip().py_upper().as_str() {
        "BTC" | "XBT" | "XXBT" => Some("BTC"),
        "LBTC" | "L-BTC" => Some("LBTC"),
        _ => None,
    }
}
fn build(
    out: &Value,
    incoming: &Value,
    exact: bool,
    method: &str,
    default_kind: Option<&str>,
) -> Result<Value, String> {
    let a = amount(out)?;
    let b = amount(incoming)?;
    let fee = if codec::truth(&out["amount_includes_fee"]) {
        BigInt::from(0)
    } else {
        let fee = match s::integer(&codec::from_json(&out["fee"])?) {
            Ok(n) => n,
            Err("TypeError" | "ValueError") => BigInt::from(0),
            Err(e) => return Err(e.into()),
        };
        fee.max(BigInt::from(0))
    };
    Ok(
        json!({"out_id":codec::pystr(&out["id"]),"in_id":codec::pystr(&incoming["id"]),"confidence":if exact{"exact"}else{"strong"},"method":method,"default_kind":default_kind.map(str::to_owned).unwrap_or_else(||kind(out,incoming)),"swap_fee_msat":number(&(a-b+fee)),"out_occurred_at":codec::text(&out["occurred_at"])}),
    )
}
pub(super) fn suggest(rows: &[Value]) -> Result<Vec<Value>, String> {
    let eligible: Vec<_> = rows
        .iter()
        .filter(|r| {
            !codec::truth(&r["excluded"])
                && matches!(r["direction"].as_str(), Some("outbound" | "inbound"))
        })
        .collect();
    let outs: Vec<_> = eligible
        .iter()
        .copied()
        .filter(|r| r["direction"] == "outbound")
        .collect();
    let ins: Vec<_> = eligible
        .iter()
        .copied()
        .filter(|r| r["direction"] == "inbound")
        .collect();
    let mut hashes = Vec::new();
    for (outs, ins) in populations(&outs, &ins) {
        if outs.len() != 1 || ins.len() != 1 {
            continue;
        }
        let (out, incoming) = (outs[0], ins[0]);
        if out["wallet_id"] == incoming["wallet_id"] {
            continue;
        }
        let a = role(out)?;
        let b = role(incoming)?;
        let roles = BTreeSet::from([a, b]);
        let exact = roles == BTreeSet::from(["lightning_node", "chain_htlc"]);
        let strong = roles == BTreeSet::from(["lightning_node", "provider_or_import"])
            || roles == BTreeSet::from(["provider_or_import"])
            || roles == BTreeSet::from(["chain_htlc", "provider_or_import"]);
        if !exact && !strong {
            continue;
        }
        let a = recorded::network_domain(out);
        let b = recorded::network_domain(incoming);
        if a.is_some() && b.is_some() && a != b || exact && (a.is_none() || b.is_none()) {
            continue;
        }
        if conserves(out, incoming)? {
            hashes.push((out, incoming, exact));
        }
    }
    // Build all producer edges before candidate int admission, in Python order.
    let providers = providers::edges(&outs, &ins)?;
    let mut refunds = Vec::new();
    for incoming in &ins {
        let (funding, index, proof) = htlc::refund(incoming)?;
        if funding.is_empty() {
            continue;
        }
        let sources: Vec<_> = outs
            .iter()
            .copied()
            .filter(|r| {
                codec::truth(&r["external_id"])
                    && codec::pystr(&r["external_id"]).py_lower() == funding.py_lower()
                    && compatible(r, incoming)
            })
            .collect();
        if sources.len() != 1 {
            continue;
        }
        let out = sources[0];
        if codec::text(&out["asset"]).py_upper() != codec::text(&incoming["asset"]).py_upper()
            || !conserves(out, incoming)?
        {
            continue;
        }
        let mut exact = false;
        if let Some(index) = index {
            if let Some(value) = htlc::output(out, &index)? {
                if value != amount(out)? {
                    continue;
                }
                let scopes = graph::onchain_scope(out).zip(graph::onchain_scope(incoming));
                exact = proof
                    && graph::strict_txid(&json!(funding)).is_some()
                    && scopes.is_some_and(|(a, b)| {
                        Some(a.2) == graph::strict_txid(&json!(funding)) && a.0 == b.0 && a.1 == b.1
                    });
            }
        }
        refunds.push((out, *incoming, exact));
    }
    let mut candidates = Vec::new();
    for (out, incoming, exact) in hashes {
        candidates.push(build(out, incoming, exact, "payment_hash", None)?);
    }
    for (out, incoming, exact, kind) in providers {
        candidates.push(build(
            out,
            incoming,
            exact,
            "provider_swap_id",
            kind.as_deref(),
        )?);
    }
    for (out, incoming, exact) in refunds {
        candidates.push(build(
            out,
            incoming,
            exact,
            "htlc_refund",
            Some("swap-refund"),
        )?);
    }
    Ok(finalize(candidates))
}
fn rank(c: &Value) -> (bool, u8) {
    (
        c["confidence"] != "exact",
        match c["method"].as_str() {
            Some("htlc_refund") => 0,
            Some("payment_hash") => 1,
            _ => 2,
        },
    )
}
fn finalize(candidates: Vec<Value>) -> Vec<Value> {
    let mut edges: Vec<Value> = Vec::new();
    let mut positions = BTreeMap::new();
    for c in candidates {
        let key = (codec::text(&c["out_id"]), codec::text(&c["in_id"]));
        if let Some(&at) = positions.get(&key) {
            if rank(&c) < rank(&edges[at]) {
                edges[at] = c;
            }
        } else {
            positions.insert(key, edges.len());
            edges.push(c);
        }
    }
    let mut ordered: Vec<_> = edges
        .iter()
        .filter(|c| c["confidence"] == "exact")
        .cloned()
        .collect();
    ordered.extend(edges.into_iter().filter(|c| c["confidence"] != "exact"));
    // Union the complete edge population before any native eligibility filter.
    // Keep Python's minimum candidate key and count edges, not distinct legs.
    fn root(parent: &mut BTreeMap<String, String>, key: &str) -> String {
        let mut node = key.to_owned();
        while parent[&node] != node {
            let next = parent[&parent[&node]].clone();
            parent.insert(node.clone(), next.clone());
            node = next;
        }
        node
    }
    let key = |c: &Value| {
        format!(
            "{}->{}",
            codec::text(&c["out_id"]),
            codec::text(&c["in_id"])
        )
    };
    let mut parent = BTreeMap::new();
    let mut legs: BTreeMap<String, String> = BTreeMap::new();
    for c in &ordered {
        let k = key(c);
        parent.entry(k.clone()).or_insert(k.clone());
        for leg in [codec::text(&c["out_id"]), codec::text(&c["in_id"])] {
            if let Some(sibling) = legs.get(&leg) {
                let a = root(&mut parent, &k);
                let b = root(&mut parent, sibling);
                if a != b {
                    let (first, last) = if a < b { (a, b) } else { (b, a) };
                    parent.insert(last, first);
                }
            } else {
                legs.insert(leg, k.clone());
            }
        }
    }
    let mut sizes = BTreeMap::new();
    for c in &ordered {
        *sizes.entry(root(&mut parent, &key(c))).or_insert(0_usize) += 1;
    }
    for c in &mut ordered {
        let r = root(&mut parent, &key(c));
        c["conflict_size"] = json!(sizes[&r]);
        c["conflict_set_id"] = json!(r);
    }
    ordered.sort_by_key(|c| {
        (
            c["confidence"] != "exact",
            {
                let n = s::integer(&codec::from_json(&c["swap_fee_msat"]).unwrap()).unwrap();
                if n < BigInt::from(0) {
                    -n
                } else {
                    n
                }
            },
            codec::text(&c["out_occurred_at"]),
            codec::text(&c["out_id"]),
            codec::text(&c["in_id"]),
        )
    });
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_matcher_helper_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_native_helpers.json"
        ))
        .unwrap();
        for vector in vectors
            .iter()
            .filter(|v| v["op"] == "matcher" || v["op"] == "finalize")
        {
            let actual = if vector["op"] == "finalize" {
                Ok(finalize(
                    super::super::array(&vector["value"])
                        .iter()
                        .map(|v| {
                            let fields = [
                                "out_id",
                                "in_id",
                                "confidence",
                                "method",
                                "default_kind",
                                "swap_fee_msat",
                                "out_occurred_at",
                            ];
                            Value::Object(
                                fields
                                    .into_iter()
                                    .map(|k| (k.to_owned(), v[k].clone()))
                                    .collect(),
                            )
                        })
                        .collect(),
                ))
            } else {
                // Direct helper operands can reach Python floats outside the
                // snapshot's signed-64 amount/fee admission. Preserve infinity
                // with a JSON exponent; this is fixture decoding, not v1 input.
                let mut rows = super::super::array(&vector["value"]).to_vec();
                for row in &mut rows {
                    if let Some(value) = row["fee"]["$float"].as_str() {
                        let exponent = match value {
                            "inf" => "1e999",
                            "-inf" => "-1e999",
                            _ => panic!("unsupported direct fee float"),
                        };
                        row["fee"] = serde_json::from_str(exponent).unwrap();
                    }
                }
                suggest(&rows)
            };
            if vector.get("error").is_some() {
                assert_eq!(
                    actual.err().map(|e| json!(e)),
                    Some(vector["error"].clone()),
                    "{}",
                    vector["case"]
                );
            } else {
                assert_eq!(
                    json!(actual.unwrap()),
                    vector["expected"],
                    "{}",
                    vector["case"]
                );
            }
        }
    }
}

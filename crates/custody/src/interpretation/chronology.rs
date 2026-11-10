//! Same-time native MOVE ordering: hard physical dependencies precede soft
//! wallet hints; ties and graph cycles preserve the Python insertion order.
use super::array;
use crate::{evidence, graph};
use kassiber_pyjson::BigInt;
use serde_json::Value;
use std::collections::BTreeSet;
use std::str::FromStr;

type Scope = (String, String, String, String);
#[derive(Clone)]
pub(super) struct Proof {
    pub scope: Scope,
    pub parents: BTreeSet<Scope>,
}
pub(super) fn evidence(row: &Value) -> Result<Option<Proof>, String> {
    if !evidence::current_observation(row)? {
        return Ok(None);
    }
    let Some(scope) = graph::onchain_scope(row) else {
        return Ok(None);
    };
    let raw = graph::transfer_object(&row["raw_json"]);
    let graphs = std::iter::once(&raw).chain(
        ["tx", "ownership_graph"]
            .into_iter()
            .map(|key| &raw[key])
            .filter(|value| value.is_object()),
    );
    let mut parents = BTreeSet::new();
    for graph in graphs {
        for input in array(&graph["vin"]) {
            if !input.is_object() {
                continue;
            }
            let Some(txid) = graph::strict_txid(&input["txid"]) else {
                continue;
            };
            // Python requires type(vout) is int: bool/float/text are not indices.
            if input["vout"].is_number()
                && BigInt::from_str(&input["vout"].to_string()).is_ok_and(|n| n >= BigInt::from(0))
            {
                parents.insert((scope.0.clone(), scope.1.clone(), txid, scope.3.clone()));
            }
        }
    }
    Ok(Some(Proof { scope, parents }))
}

pub(super) struct Transfer {
    pub source_hash: String,
    pub from: String,
    pub to: String,
    pub proof: Option<Proof>,
}
fn descendants(outgoing: &[BTreeSet<usize>], start: usize) -> BTreeSet<usize> {
    let mut pending: Vec<_> = outgoing[start].iter().copied().collect();
    let mut visited = BTreeSet::new();
    while let Some(target) = pending.pop() {
        if visited.insert(target) {
            pending.extend(&outgoing[target]);
        }
    }
    visited
}
pub(super) fn order(transfers: &[Transfer]) -> Vec<usize> {
    let size = transfers.len();
    let mut outgoing = vec![BTreeSet::new(); size];
    for (i, source) in transfers.iter().enumerate() {
        for (j, target) in transfers.iter().enumerate() {
            if i != j
                && source
                    .proof
                    .as_ref()
                    .zip(target.proof.as_ref())
                    .is_some_and(|(a, b)| b.parents.contains(&a.scope))
            {
                outgoing[i].insert(j);
            }
        }
    }
    let hard = outgoing.iter().any(|edges| !edges.is_empty());
    let hard_descendants: Vec<_> = (0..size).map(|i| descendants(&outgoing, i)).collect();
    for (i, source) in transfers.iter().enumerate() {
        for (j, target) in transfers.iter().enumerate() {
            if source
                .proof
                .as_ref()
                .zip(target.proof.as_ref())
                .is_some_and(|(a, b)| a.scope == b.scope)
            {
                continue;
            }
            if i == j || source.to != target.from {
                continue;
            }
            if hard
                && (hard_descendants[i].contains(&j)
                    || hard_descendants[j].contains(&i)
                    || descendants(&outgoing, j).contains(&i))
            {
                continue;
            }
            outgoing[i].insert(j);
        }
    }
    let mut incoming = vec![0; size];
    for targets in &outgoing {
        for &target in targets {
            incoming[target] += 1;
        }
    }
    let mut ready: BTreeSet<_> = (0..size).filter(|&i| incoming[i] == 0).collect();
    let mut ordered = Vec::new();
    while let Some(i) = ready.pop_first() {
        ordered.push(i);
        for &j in &outgoing[i] {
            incoming[j] -= 1;
            if incoming[j] == 0 {
                ready.insert(j);
            }
        }
    }
    if ordered.len() != size {
        return (0..size).collect();
    }
    ordered
}

pub(super) fn source_order(transfers: &[Transfer]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    order(transfers)
        .into_iter()
        .filter_map(|i| {
            let key = &transfers[i].source_hash;
            seen.insert(key.clone()).then(|| key.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn reconciliation_chronology_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_reconciliation_helpers.json"
        ))
        .unwrap();
        for v in vectors.iter().filter(|v| {
            matches!(
                v["op"].as_str(),
                Some("chronology" | "order" | "source_order")
            )
        }) {
            let value = &v["value"];
            let actual = if v["op"] == "order" || v["op"] == "source_order" {
                let transfers: Vec<_> = array(value)
                    .iter()
                    .enumerate()
                    .map(|(i, t)| Transfer {
                        source_hash: t["source_hash"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| i.to_string()),
                        from: crate::codec::text(&t["from_wallet"]),
                        to: crate::codec::text(&t["to_wallet"]),
                        proof: if t["proof"].is_null() {
                            None
                        } else {
                            Some(Proof {
                                scope: serde_json::from_value(t["proof"]["scope"].clone()).unwrap(),
                                parents: serde_json::from_value(t["proof"]["parents"].clone())
                                    .unwrap(),
                            })
                        },
                    })
                    .collect();
                if v["op"] == "source_order" {
                    json!(source_order(&transfers))
                } else {
                    json!(order(&transfers))
                }
            } else {
                evidence(value).unwrap().map_or(
                    Value::Null,
                    |p| json!({"scope":p.scope,"parents":p.parents}),
                )
            };
            assert_eq!(actual, v["expected"], "{}", v["case"]);
        }
    }
}

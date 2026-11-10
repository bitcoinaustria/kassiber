//! Project the shared native local index into the reduced PSBT observer model.
//! The caller owns the enclosing read snapshot; no proposal or private script is stored.
use super::{AdversaryView, Baseline, LinkageEdge, OwnedOutput, Result};
use crate::analysis_index::{analytics, sources, traversal, AnalysisIndex};
use crate::publication::row::rows;
use kassiber_envelope::{get, set};
use kassiber_pyjson::{Options, Value as Py};
use kassiber_store::rusqlite::{Connection, Params};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn json_value(value: &Py) -> Result<Value> {
    let encoded = kassiber_pyjson::dumps(value, &Options::new().compact())
        .map_err(|_| super::inventory::unavailable())?;
    serde_json::from_str(&encoded).map_err(|_| super::inventory::unavailable())
}
fn py(value: &Value) -> Py {
    kassiber_pytext::loads(&value.to_string()).expect("JSON value")
}
fn text(value: &Value) -> String {
    crate::descriptors::text(value)
}
fn truth(value: &Value) -> bool {
    crate::descriptors::truthy(value)
}
fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn map(value: &Value) -> impl Iterator<Item = (&String, &Value)> {
    value.as_object().into_iter().flatten()
}
fn read(conn: &Connection, sql: &str, params: impl Params) -> Result<Vec<Value>> {
    rows(conn, sql, params)?.iter().map(json_value).collect()
}
fn columns(conn: &Connection, table: &str) -> Result<BTreeSet<String>> {
    Ok(read(conn, &format!("PRAGMA table_info({table})"), [])?
        .iter()
        .map(|r| text(&r["name"]))
        .collect())
}
fn has(columns: &BTreeSet<String>, names: &[&str]) -> bool {
    names.iter().all(|n| columns.contains(*n))
}
fn txid(value: &Value) -> Option<String> {
    let value = kassiber_pytext::lower(kassiber_pytext::strip(&text(value)));
    (value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())).then_some(value)
}

pub fn capture(conn: &Connection, profile: &str) -> Result<Baseline> {
    let index = sources::build_index(conn, profile, "owner")?;
    project(conn, profile, &index)
}

/// Accept an already captured shared index only from native composition code.
pub fn project(conn: &Connection, profile: &str, index: &AnalysisIndex) -> Result<Baseline> {
    let snapshot = json_value(index.snapshot())?;
    let mut baseline = Baseline {
        nodes: vec![],
        edges: vec![],
        adversary_views: vec![],
        historical_linkage_bounded: false,
    };
    let networks: BTreeSet<_> = map(&snapshot["nodes"])
        .filter(|(id, n)| {
            n["kind"] == "output"
                && truth(&snapshot["output_facts"][*id]["inventory_observation_count"])
                && n["chain"] == "bitcoin"
                && n["asset"] == "BTC"
        })
        .map(|(_, n)| text(&n["network"]))
        .collect();
    let mut canonical = BTreeMap::new();
    if networks.len() <= 1 {
        for (id, node) in map(&snapshot["nodes"]) {
            let fact = &snapshot["output_facts"][id];
            if node["kind"] != "output"
                || !truth(&fact["inventory_observation_count"])
                || node["chain"] != "bitcoin"
                || node["asset"] != "BTC"
                || !truth(&fact["ownership_known"])
                || truth(&fact["ownership_ambiguous"])
                || array(&node["wallet_ids"]).len() != 1
                || matches!(node["status"].as_str(), Some("conflicting" | "stale"))
                || node["amount_msat"].is_null()
            {
                continue;
            }
            let outpoint = text(&node["outpoint"]);
            let (txid, vout) = outpoint
                .rsplit_once(':')
                .ok_or_else(super::inventory::unavailable)?;
            baseline.nodes.push(OwnedOutput {
                node_id: outpoint.clone(),
                txid: txid.into(),
                vout: vout.parse().map_err(|_| super::inventory::unavailable())?,
                wallet_id: text(&node["wallet_ids"][0]),
                amount_msat: text(&node["amount_msat"])
                    .parse()
                    .map_err(|_| super::inventory::unavailable())?,
                script_key: fact["script"].as_str().map(str::to_owned),
                branch_role: fact["branch_role"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("unknown")
                    .into(),
                branch_evidence_level: fact["branch_evidence_level"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("unknown")
                    .into(),
                change_evidence: fact["change_evidence"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("unavailable")
                    .into(),
            });
            canonical.insert(id.clone(), outpoint);
        }
    }
    let mut fact_nodes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if let Some(network) = baseline
        .nodes
        .first()
        .map(|_| text(&snapshot["nodes"][canonical.keys().next().unwrap()]["network"]))
    {
        let query = traversal::normalize_query(&py(
            &json!({"observer":"public","chain":"bitcoin","network":network,"include_hypotheses":true,"include_relations":false,"node_limit":2000,"edge_limit":6000,"depth":12}),
        ))?;
        let public = index.observer("public")?;
        let result = traversal::query_index(&public, &query)?;
        let selected: BTreeSet<String> = array(&json_value(&result)?["nodes"])
            .iter()
            .map(|n| text(&n["id"]))
            .collect();
        let mut analytics_query = query.clone();
        set(
            &mut analytics_query,
            "_subject_node_ids",
            get(public.snapshot(), "profile_seeds").unwrap().clone(),
        );
        let edge_ids = array(&json_value(&result)?["edges"])
            .iter()
            .map(|n| n["id"].clone())
            .collect::<Vec<_>>();
        set(
            &mut analytics_query,
            "_selected_edge_ids",
            py(&json!(edge_ids)),
        );
        let analysis = json_value(&analytics::analyze_index(
            &public,
            &analytics_query,
            &selected,
        )?)?;
        let physical = json_value(&result)?;
        baseline.historical_linkage_bounded = truth(&physical["coverage"]["budget_exhausted"])
            || array(&analysis["coverage"]["stopped_reasons"])
                .iter()
                .any(|r| r == "hypothesis_budget");
        // Only hypothesis edges whose endpoints survived the physical query can be appended.
        let mut edge_count = array(&physical["edges"]).len();
        for edge in array(&analysis["hypothesis_edges"]) {
            if edge_count >= 6000 {
                baseline.historical_linkage_bounded = true;
                break;
            }
            if !selected.contains(&text(&edge["source"]))
                || !selected.contains(&text(&edge["target"]))
            {
                continue;
            }
            edge_count += 1;
            if let (Some(a), Some(b)) = (
                canonical.get(&text(&edge["source"])),
                canonical.get(&text(&edge["target"])),
            ) {
                if a != b {
                    baseline.edges.push(LinkageEdge {
                        from_node_id: a.clone(),
                        to_node_id: b.clone(),
                        observer_linkage: true,
                    });
                }
            }
        }
        // Preserve transitive links through observed outputs without private ownership.
        for cluster in array(&analysis["clusters"]) {
            let owned: Vec<_> = array(&cluster["node_ids"])
                .iter()
                .filter_map(|id| canonical.get(&text(id)))
                .collect();
            for other in owned.iter().skip(1) {
                baseline.edges.push(LinkageEdge {
                    from_node_id: owned[0].clone(),
                    to_node_id: (*other).clone(),
                    observer_linkage: true,
                });
            }
        }
        for (id, node) in map(&snapshot["nodes"]) {
            if node["kind"] != "transaction"
                || node["chain"] != "bitcoin"
                || node["network"] != network
            {
                continue;
            }
            let members = fact_nodes.entry(text(&node["txid"])).or_default();
            for (side, kind, endpoint) in [
                ("incoming", "spends", "source"),
                ("outgoing", "creates", "target"),
            ] {
                for edge in array(&snapshot[side][id]) {
                    let edge = &snapshot["edges"][text(edge)];
                    if edge["kind"] == kind {
                        if let Some(node) = canonical.get(&text(&edge[endpoint])) {
                            members.insert(node.clone());
                        }
                    }
                }
            }
        }
    }
    // The reduced proposal needs exposure component IDs and counts only; labels,
    // anchor contents and custody/source-funds amounts never enter its baseline.
    let components = super::components(&baseline)?;
    let passive: BTreeSet<_> = baseline
        .edges
        .iter()
        .filter(|e| e.observer_linkage)
        .map(|e| components[e.from_node_id.as_str()].clone())
        .collect();
    let mut tx_nodes = fact_nodes;
    for node in &baseline.nodes {
        tx_nodes
            .entry(node.txid.clone())
            .or_default()
            .insert(node.node_id.clone());
    }
    let network = networks
        .iter()
        .next()
        .filter(|_| !baseline.nodes.is_empty())
        .map(String::as_str);
    let allowed = transaction_ids(conn, profile, network)?;
    let reviewed: BTreeSet<_> = allowed
        .iter()
        .filter(|(_, row)| !truth(&row["excluded"]))
        .map(|(id, _)| id.clone())
        .collect();
    let mut source_txids = Vec::new();
    if has(
        &columns(conn, "source_funds_links")?,
        &[
            "id",
            "profile_id",
            "state",
            "from_source_id",
            "from_transaction_id",
            "to_transaction_id",
            "confidence",
        ],
    ) && has(
        &columns(conn, "transactions")?,
        &["id", "profile_id", "external_id"],
    ) {
        for row in read(conn, "SELECT l.from_source_id,l.from_transaction_id,l.to_transaction_id,to_tx.external_id AS to_external_id,from_tx.external_id AS from_external_id FROM source_funds_links l LEFT JOIN transactions to_tx ON to_tx.profile_id=l.profile_id AND to_tx.id=l.to_transaction_id LEFT JOIN transactions from_tx ON from_tx.profile_id=l.profile_id AND from_tx.id=l.from_transaction_id WHERE l.profile_id=? AND l.state='reviewed' ORDER BY l.id ASC", [profile])? {
            if (!reviewed.contains(&text(&row["to_transaction_id"])) && !row["to_external_id"].is_null())
                || (truth(&row["from_transaction_id"]) && !reviewed.contains(&text(&row["from_transaction_id"])) && !row["from_external_id"].is_null()) { continue; }
            if truth(&row["from_source_id"]) { source_txids.push(row["to_external_id"].clone()); }
            if truth(&row["from_transaction_id"]) { source_txids.extend([row["from_external_id"].clone(),row["to_external_id"].clone()]); }
        }
    }
    let mut counterparty_txids = vec![];
    if has(
        &columns(conn, "transactions")?,
        &["profile_id", "external_id", "counterparty"],
    ) {
        for row in read(conn, "SELECT id,external_id FROM transactions WHERE profile_id=? AND external_id IS NOT NULL AND trim(external_id)!='' AND counterparty IS NOT NULL AND trim(counterparty)!='' ORDER BY external_id ASC", [profile])? {
            if allowed.contains_key(&text(&row["id"])) { counterparty_txids.push(row["external_id"].clone()); }
        }
    }
    let exposed = |candidates: Vec<Value>| -> BTreeSet<String> {
        candidates
            .iter()
            .filter_map(txid)
            .filter_map(|id| tx_nodes.get(&id))
            .flatten()
            .filter_map(|id| components.get(id.as_str()).cloned())
            .collect()
    };
    for (tier, label, clusters, assumptions) in [
        ("passive_chain_watcher", "Passive chain watcher", passive, 3),
        (
            "reviewed_kyc_source_funds_anchor_watcher",
            "Reviewed KYC/source-funds anchor watcher",
            exposed(source_txids),
            2,
        ),
        (
            "known_counterparty_hypothetical",
            "Known-counterparty hypothetical",
            exposed(counterparty_txids),
            2,
        ),
    ] {
        baseline.adversary_views.push(AdversaryView {
            tier: tier.into(),
            label: label.into(),
            exposed_cluster_count: json!(clusters.len()),
            cluster_ids: clusters,
            model_assumption_count: assumptions,
        });
    }
    Ok(baseline)
}

fn transaction_ids(
    conn: &Connection,
    profile: &str,
    network: Option<&str>,
) -> Result<BTreeMap<String, Value>> {
    let cols = columns(conn, "transactions")?;
    if !cols.contains("profile_id") {
        return Ok(BTreeMap::new());
    }
    let joined = cols.contains("wallet_id")
        && has(&columns(conn, "wallets")?, &["id", "kind", "config_json"]);
    let select = if joined {
        "w.kind AS wallet_kind,w.config_json AS wallet_config_json"
    } else {
        "NULL AS wallet_kind,NULL AS wallet_config_json"
    };
    let join = if joined {
        "LEFT JOIN wallets w ON w.id=t.wallet_id"
    } else {
        ""
    };
    let mut result = BTreeMap::new();
    for row in read(conn, &format!("SELECT t.*,{select} FROM transactions t {join} WHERE t.profile_id=? ORDER BY t.external_id ASC"), [profile])? {
        let Ok((chain, resolved)) = kassiber_custody::evidence::scope(&row) else { continue; };
        let asset = if truth(&row["asset"]) { text(&row["asset"]) } else { "BTC".into() };
        if chain == "bitcoin" && network.is_none_or(|n| n == resolved) && crate::config::asset_code(&asset) == "BTC" { result.insert(text(&row["id"]),row); }
    }
    Ok(result)
}

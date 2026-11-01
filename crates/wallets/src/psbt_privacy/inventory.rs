//! Inventory-only admission for the reduced PSBT baseline. Historical graph
//! sources must be excluded by the snapshot owner before calling this adapter.
use super::{AdversaryView, Baseline, LinkageEdge, OwnedOutput, Result};
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::BigInt;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

// At most two physical nodes and one create edge per inventory row, and fewer
// than 500 reuse hypotheses. Admission cannot silently cross query/rule bounds.
pub const MAX_ROWS: usize = 400;

pub fn unavailable() -> AppError {
    AppError::new(
        "not_implemented",
        "PSBT privacy requires a native historical graph for this book",
    )
    .with_details(dict([("component", s("psbt_privacy_baseline"))]))
}

fn string(value: &Value) -> String {
    crate::descriptors::text(value)
}

fn canonical_txid(value: &Value) -> Option<String> {
    let text = kassiber_pytext::lower(kassiber_pytext::strip(&string(value)));
    (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())).then_some(text)
}

fn integer(value: &Value) -> Option<BigInt> {
    if value.is_boolean() || value.is_null() {
        return None;
    }
    let text = string(value);
    let number = kassiber_pytext::parse_int(&text).ok()?;
    (number >= BigInt::from(0) && kassiber_pytext::strip(&text) == number.to_string())
        .then_some(number)
}

fn script(value: &Value) -> Option<String> {
    if value.is_null() || value == "" {
        return None;
    }
    let text = kassiber_pytext::lower(kassiber_pytext::strip(&string(value)));
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let mut input = text.bytes();
    while let Some(high) = input.next() {
        if matches!(high, b'\t'..=b'\r' | b' ') {
            continue;
        }
        if !high.is_ascii_hexdigit() || !input.next().is_some_and(|b| b.is_ascii_hexdigit()) {
            return None;
        }
    }
    (!text.is_empty()).then_some(text)
}

fn branch(label: &Value, index: &Value) -> (String, String, String) {
    static ALNUM: OnceLock<Regex> = OnceLock::new();
    let alnum = ALNUM.get_or_init(|| {
        Regex::new(&format!("^{}$", kassiber_pytext::REGEX_WORD)).expect("Unicode 14 word table")
    });
    let label = if crate::descriptors::truthy(label) {
        string(label)
    } else {
        String::new()
    };
    let normalized: String = kassiber_pytext::lower(kassiber_pytext::strip(&label))
        .chars()
        .map(|c| {
            if c != '_' && alnum.is_match(c.encode_utf8(&mut [0; 4])) {
                c
            } else {
                ' '
            }
        })
        .collect();
    let mut tokens: BTreeSet<_> = normalized.split_whitespace().map(str::to_owned).collect();
    tokens.insert(normalized.replace(' ', ""));
    let role = if ["change", "internal"].iter().any(|t| tokens.contains(*t)) {
        Some("change")
    } else if ["receive", "external", "deposit"]
        .iter()
        .any(|t| tokens.contains(*t))
    {
        Some("receive")
    } else {
        None
    };
    if let Some(role) = role {
        let imported = ["p2pkh", "p2shp2wpkh", "p2sh", "p2wpkh", "p2tr"]
            .iter()
            .any(|t| tokens.contains(*t));
        return (
            role.into(),
            "exact".into(),
            if imported { "imported" } else { "ground_truth" }.into(),
        );
    }
    let index = integer(index).and_then(|n| u32::try_from(n).ok());
    match index.filter(|n| *n < 8) {
        Some(n) => (
            if n % 2 == 0 { "receive" } else { "change" }.into(),
            "derived".into(),
            "heuristic".into(),
        ),
        None => ("unknown".into(), "unknown".into(), "unavailable".into()),
    }
}

struct Output {
    node: OwnedOutput,
    amount: Option<BigInt>,
    observations: usize,
    wallets: BTreeSet<String>,
    conflicting: bool,
    script_conflict: bool,
}

/// Construct only the graph whose sole physical source is unspent inventory.
/// SQL source admission and capture belong to the caller's read snapshot.
pub fn baseline(rows: &[Value]) -> Result<Baseline> {
    if rows.len() > MAX_ROWS {
        return Err(unavailable());
    }
    let mut outputs: BTreeMap<(String, String, String), Output> = BTreeMap::new();
    let mut networks = BTreeSet::new();
    for row in rows {
        let raw = match &row["raw_json"] {
            Value::String(text) => match kassiber_pytext::loads(text) {
                Ok(raw) => {
                    let encoded =
                        kassiber_pyjson::dumps(&raw, &kassiber_pyjson::Options::new().compact())
                            .map_err(|_| unavailable())?;
                    serde_json::from_str(&encoded).map_err(|_| unavailable())?
                }
                Err(_) => Value::Null,
            },
            value => value.clone(),
        };
        if crate::descriptors::truthy(&row["chain_instance_id"])
            || crate::descriptors::truthy(&raw["chain_instance_id"])
        {
            return Err(unavailable());
        }
        let Ok((chain, network)) = kassiber_custody::evidence::scope(row) else {
            continue;
        };
        let (Some(txid), Some(vout)) = (canonical_txid(&row["txid"]), integer(&row["vout"])) else {
            continue;
        };
        if !matches!(chain.as_str(), "bitcoin" | "liquid") {
            continue;
        }
        if canonical_txid(&row["spent_by"]).is_some() {
            return Err(unavailable());
        }
        let script = script(&row["script_pubkey"]);
        // The Bitcoin index always supplies BTC, independently of an inventory
        // row's asset label. Domain resolution, rather than an amount, admits it.
        if chain == "bitcoin" {
            networks.insert(network.clone());
        }
        let vout = u32::try_from(vout).map_err(|_| unavailable())?;
        let id = format!("{txid}:{vout}");
        let amount = integer(&row["amount"]);
        let wallet = if crate::descriptors::truthy(&row["wallet_id"]) {
            string(&row["wallet_id"])
        } else {
            String::new()
        };
        let (role, level, change) = branch(&row["branch_label"], &row["branch_index"]);
        let entry = outputs
            .entry((chain, network, id.clone()))
            .or_insert_with(|| Output {
                node: OwnedOutput {
                    node_id: id,
                    txid,
                    vout,
                    wallet_id: wallet.clone(),
                    amount_msat: amount.clone().unwrap_or_default(),
                    script_key: script.clone(),
                    branch_role: role,
                    branch_evidence_level: level,
                    change_evidence: change,
                },
                amount: amount.clone(),
                observations: 0,
                wallets: BTreeSet::new(),
                conflicting: false,
                script_conflict: false,
            });
        if entry.observations > 0 {
            // Every repeated inventory outpoint is ambiguous even if both rows
            // report the same wallet, amount and branch evidence.
            entry.conflicting |=
                entry.amount.is_some() && amount.is_some() && entry.amount != amount;
            if entry.node.script_key.is_some()
                && script.is_some()
                && entry.node.script_key != script
            {
                entry.conflicting = true;
                entry.script_conflict = true;
                entry.node.script_key = None;
            }
            if entry.node.script_key.is_none() && !entry.script_conflict {
                entry.node.script_key = script;
            }
        }
        entry.observations += 1;
        if !wallet.is_empty() {
            entry.wallets.insert(wallet);
        }
    }
    // The unchanged graph validates indexed attribution subjects even when
    // there are no active datasets. Validate the final merged script, because
    // contradictory observations can erase a previously present script.
    for output in outputs.values() {
        if output
            .node
            .script_key
            .as_ref()
            .is_some_and(|s| s.len() <= 20_000 && !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(AppError::new("validation", "Attribution subject must be a valid address, transaction, output or script in the selected domain"));
        }
    }
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    if networks.len() <= 1 {
        // Reuse is public even when an output's private owner is ambiguous.
        // Safe identical-script components therefore include those outputs as
        // intermediate members before projection onto the admitted owned nodes.
        let mut scripts: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for ((chain, _, _), output) in &outputs {
            if chain != "bitcoin" {
                continue;
            }
            if !output.conflicting {
                if let Some(script) = output
                    .node
                    .script_key
                    .as_ref()
                    .filter(|s| !s.is_empty() && !s.starts_with("6a"))
                {
                    scripts
                        .entry(script.clone())
                        .or_default()
                        .push(output.node.node_id.clone());
                }
            }
        }
        let admitted: BTreeSet<_> = outputs
            .iter()
            .filter(|((chain, _, _), o)| {
                chain == "bitcoin"
                    && o.observations == 1
                    && o.wallets.len() == 1
                    && o.amount.is_some()
                    && !o.conflicting
            })
            .map(|(_, o)| o.node.node_id.clone())
            .collect();
        for members in scripts.values() {
            let owned: Vec<_> = members.iter().filter(|id| admitted.contains(*id)).collect();
            for other in owned.iter().skip(1) {
                edges.push(LinkageEdge {
                    from_node_id: owned[0].clone(),
                    to_node_id: (*other).clone(),
                    observer_linkage: true,
                });
            }
        }
        nodes = outputs
            .into_iter()
            .filter(|((chain, _, _), o)| chain == "bitcoin" && admitted.contains(&o.node.node_id))
            .map(|(_, o)| o.node)
            .collect();
    }
    let mut baseline = Baseline {
        nodes,
        edges,
        adversary_views: Vec::new(),
        historical_linkage_bounded: false,
    };
    let components = super::components(&baseline)?;
    let passive: BTreeSet<_> = baseline
        .edges
        .iter()
        .map(|edge| components[edge.from_node_id.as_str()].clone())
        .collect();
    for (tier, label, clusters, assumptions) in [
        ("passive_chain_watcher", "Passive chain watcher", passive, 3),
        (
            "reviewed_kyc_source_funds_anchor_watcher",
            "Reviewed KYC/source-funds anchor watcher",
            BTreeSet::new(),
            2,
        ),
        (
            "known_counterparty_hypothetical",
            "Known-counterparty hypothetical",
            BTreeSet::new(),
            2,
        ),
    ] {
        baseline.adversary_views.push(AdversaryView {
            tier: tier.into(),
            label: label.into(),
            exposed_cluster_count: clusters.len().into(),
            cluster_ids: clusters,
            model_assumption_count: assumptions,
        });
    }
    Ok(baseline)
}

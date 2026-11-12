//! `core/source_overlap.py`'s sync-target filter: drop every target whose
//! script a better canonical wallet of the same profile already owns. Only
//! finite local evidence counts (address lists, the output inventory and
//! descriptor horizons), as in Python; nothing here makes a request.
//!
//! Python reaches the filter two ways: the prefetch uses an operation-scoped
//! index (`build_profile_source_index` plus candidate supplements) and every
//! later check derives the sources afresh. The two list the sources in
//! different orders; both orders are kept here, and the owner aggregates each
//! wallet's sources, so they name the same owner. The journal pre-build keeps
//! its own port of the index (`kassiber-journals` `prebuild/sources.rs`).
use crate::{
    config,
    descriptors::{self, Plan},
};
use kassiber_envelope::AppError;
use kassiber_store::rusqlite::{types::Value as Sql, Connection};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

type Result<T> = std::result::Result<T, AppError>;

const MAX_TARGETS_PER_BRANCH: i64 = 20_000;

#[derive(Clone)]
struct Source {
    wallet: String,
    label: String,
    kind: String,
    chain: String,
    network: String,
    script: String,
    source: &'static str,
    deprecated: bool,
    count: i64,
    branch: Option<i64>,
    index: Option<i64>,
}

struct Descriptor {
    wallet: String,
    label: String,
    kind: String,
    deprecated: bool,
    plan: Plan,
}

/// The profile's ownership evidence for one wallet's sync, read once before
/// any request so a filter applied after discovery sees the same book.
pub(crate) struct Overlap {
    wallet: String,
    label: String,
    kind: String,
    deprecated: bool,
    counts: HashMap<String, i64>,
    lists: Vec<Source>,
    inventory: Vec<Source>,
    descriptors: Vec<Descriptor>,
    maxes: HashMap<(String, u32), i64>,
}

fn database(error: kassiber_store::rusqlite::Error) -> AppError {
    crate::database_error(error)
}

fn query(conn: &Connection, sql: &str, profile: &str, width: usize) -> Result<Vec<Vec<Sql>>> {
    let mut statement = conn.prepare(sql).map_err(database)?;
    let rows = statement
        .query_map([profile], |row| {
            (0..width).map(|i| row.get::<_, Sql>(i)).collect()
        })
        .map_err(database)?;
    rows.collect::<std::result::Result<_, _>>()
        .map_err(database)
}

/// `str(value or "")` for a column.
fn column(value: &Sql) -> String {
    match value {
        Sql::Null => String::new(),
        Sql::Integer(n) if *n == 0 => String::new(),
        Sql::Integer(n) => n.to_string(),
        Sql::Real(n) if *n == 0.0 => String::new(),
        Sql::Real(n) => n.to_string(),
        Sql::Text(t) => t.clone(),
        Sql::Blob(b) if b.is_empty() => String::new(),
        Sql::Blob(_) => String::new(),
    }
}

fn integer(value: &Sql) -> Option<i64> {
    match value {
        Sql::Integer(n) => Some(*n),
        Sql::Real(n) if n.is_finite() => Some(n.trunc() as i64),
        Sql::Text(t) => kassiber_pytext::parse_int(t)
            .ok()
            .and_then(|n| i64::try_from(n).ok()),
        _ => None,
    }
}

fn json_integer(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Number(n) => n.as_i64().or_else(|| {
            n.as_f64()
                .filter(|v| v.is_finite())
                .map(|v| v.trunc() as i64)
        }),
        Value::String(t) => kassiber_pytext::parse_int(t)
            .ok()
            .and_then(|n| i64::try_from(n).ok()),
        _ => None,
    }
}

/// `_wallet_config`: anything but a JSON object reads as `{}`.
fn wallet_config(text: &str) -> Value {
    match serde_json::from_str::<Value>(if text.is_empty() { "{}" } else { text }) {
        Ok(value @ Value::Object(_)) => value,
        _ => json!({}),
    }
}

fn lower(value: &str) -> String {
    kassiber_pytext::lower(kassiber_pytext::strip(value))
}

/// `normalized_script_hex`, which keeps the text it validated.
fn script_hex(value: &str) -> Option<String> {
    let text = lower(value);
    if text.is_empty() || !text.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        if b" \t\n\r\x0b\x0c".contains(&byte) {
            continue;
        }
        if !byte.is_ascii_hexdigit() || !bytes.next().is_some_and(|b| b.is_ascii_hexdigit()) {
            return None;
        }
    }
    Some(text)
}

/// `_normalize_chain_network`.
fn scope(chain: &str, network: &str) -> (String, String) {
    if chain.is_empty() || network.is_empty() {
        return (String::new(), String::new());
    }
    match descriptors::normalize_chain(chain)
        .and_then(|c| descriptors::normalize_network(&c, network).map(|n| (c, n)))
    {
        Ok(scope) => scope,
        Err(_) => (lower(chain), lower(network)),
    }
}

fn json_text(value: &Value) -> String {
    if descriptors::truthy(value) {
        descriptors::text(value)
    } else {
        String::new()
    }
}

/// `has_descriptor_sync_material`.
fn material(config: &Value) -> bool {
    !kassiber_pytext::strip(&json_text(&config["descriptor"])).is_empty()
        || (!kassiber_pytext::strip(&json_text(&config["xpub"])).is_empty()
            && descriptors::truthy(&config["script_types"]))
}

fn deprecated(config: &Value) -> Result<bool> {
    config::boolean(config.get("deprecated"))
}

impl Overlap {
    /// `wallet` is the routed row Python passes to the filter: its label,
    /// kind and `deprecated` flag describe the candidate's sync targets.
    pub(crate) fn capture(
        conn: &Connection,
        profile: &str,
        wallet: &str,
        label: &str,
        kind: &str,
        routed_config: &Value,
    ) -> Result<Self> {
        let counts: HashMap<String, i64> = query(
            conn,
            "SELECT wallet_id, COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0 GROUP BY wallet_id",
            profile,
            2,
        )?
        .iter()
        .map(|row| (column(&row[0]), integer(&row[1]).unwrap_or(0)))
        .collect();
        let count = |wallet: &str| *counts.get(wallet).unwrap_or(&0);
        let wallets = query(
            conn,
            "SELECT id, label, kind, config_json FROM wallets WHERE profile_id = ? ORDER BY label ASC, id ASC",
            profile,
            4,
        )?;
        let mut lists = Vec::new();
        for row in &wallets {
            let id = column(&row[0]);
            let config = wallet_config(&column(&row[3]));
            let (chain, network) =
                scope(&json_text(&config["chain"]), &json_text(&config["network"]));
            if chain.is_empty() || network.is_empty() {
                continue;
            }
            let addresses = config::addresses(config.get("addresses"))?;
            for (index, address) in addresses.iter().enumerate() {
                let Some(script) = config::bitcoin_address_script(address) else {
                    continue;
                };
                let kind = column(&row[2]);
                let deprecated = deprecated(&config)?;
                lists.push(Source {
                    label: Some(column(&row[1]))
                        .filter(|l| !l.is_empty())
                        .unwrap_or_else(|| id.clone()),
                    kind: if kind.is_empty() {
                        "address".into()
                    } else {
                        kind
                    },
                    chain: chain.clone(),
                    network: network.clone(),
                    script,
                    source: "address_list",
                    deprecated,
                    count: count(&id),
                    branch: None,
                    index: Some(index as i64),
                    wallet: id.clone(),
                });
            }
        }
        let by_id: HashMap<String, &Vec<Sql>> =
            wallets.iter().map(|row| (column(&row[0]), row)).collect();
        let mut inventory = Vec::new();
        for row in query(
            conn,
            "SELECT DISTINCT wallet_id, chain, network, address, script_pubkey, branch_label, branch_index, address_index, raw_json FROM wallet_utxos WHERE profile_id = ?",
            profile,
            9,
        )? {
            let id = column(&row[0]);
            let raw = wallet_config(&column(&row[8]));
            let Some(script) = script_hex(&column(&row[4]))
                .or_else(|| script_hex(&json_text(&raw["script_pubkey"])))
                .or_else(|| config::bitcoin_address_script(&column(&row[3])))
            else {
                continue;
            };
            let owner = by_id.get(&id);
            let config = owner.map_or_else(|| json!({}), |w| wallet_config(&column(&w[3])));
            let (chain, network) = scope(&column(&row[1]), &column(&row[2]));
            inventory.push(Source {
                label: owner
                    .map(|w| column(&w[1]))
                    .unwrap_or_else(|| id.clone()),
                kind: owner.map(|w| column(&w[2])).unwrap_or_default(),
                chain,
                network,
                script,
                source: "inventory",
                deprecated: deprecated(&config)?,
                count: count(&id),
                branch: integer(&row[6]),
                index: integer(&row[7]),
                wallet: id,
            });
        }
        let mut maxes: HashMap<(String, u32), i64> = HashMap::new();
        let mut merge = |wallet: String, branch: i64, value: i64| {
            let Ok(branch) = u32::try_from(branch) else {
                return;
            };
            let entry = maxes.entry((wallet, branch)).or_insert(-1);
            *entry = (*entry).max(value);
        };
        for row in query(
            conn,
            "SELECT wallet_id, branch_index, MAX(address_index) AS max_index FROM wallet_utxos WHERE profile_id = ? AND branch_index IS NOT NULL AND address_index IS NOT NULL GROUP BY wallet_id, branch_index",
            profile,
            3,
        )? {
            if let (Some(branch), Some(max)) = (integer(&row[1]), integer(&row[2])) {
                merge(column(&row[0]), branch, max);
            }
        }
        for row in query(
            conn,
            "SELECT source_key, checkpoint_json FROM freshness_source_states WHERE profile_id = ? AND source_key LIKE 'onchain_wallet:%'",
            profile,
            2,
        )? {
            let key = column(&row[0]);
            let checkpoint = wallet_config(&column(&row[1]));
            if let Some(highest) = checkpoint["highest_used"].as_object() {
                for (branch, value) in highest {
                    if let (Some(branch), Some(value)) =
                        (json_integer(&json!(branch)), json_integer(value))
                    {
                        merge(key["onchain_wallet:".len()..].to_owned(), branch, value);
                    }
                }
            }
        }
        let mut plans = Vec::new();
        for row in &wallets {
            let config = wallet_config(&column(&row[3]));
            if !material(&config) {
                continue;
            }
            let Ok(Some(plan)) = descriptors::load(&config) else {
                continue;
            };
            let id = column(&row[0]);
            plans.push(Descriptor {
                label: Some(column(&row[1]))
                    .filter(|l| !l.is_empty())
                    .unwrap_or_else(|| id.clone()),
                kind: column(&row[2]),
                deprecated: deprecated(&config)?,
                wallet: id,
                plan,
            });
        }
        Ok(Self {
            wallet: wallet.into(),
            label: if label.is_empty() {
                wallet.into()
            } else {
                label.into()
            },
            kind: kind.into(),
            deprecated: deprecated(routed_config)?,
            counts,
            lists,
            inventory,
            descriptors: plans,
            maxes,
        })
    }

    /// `scripts_from_sync_state` for the candidate wallet.
    fn candidates(&self, targets: &[Value], chain: &str, network: &str) -> Vec<Source> {
        targets
            .iter()
            .filter_map(|target| {
                let script = lower(&json_text(&target["script_pubkey"]));
                if script.is_empty() {
                    return None;
                }
                let (chain, network) = scope(
                    &if chain.is_empty() {
                        json_text(&target["chain"])
                    } else {
                        chain.into()
                    },
                    &if network.is_empty() {
                        json_text(&target["network"])
                    } else {
                        network.into()
                    },
                );
                Some(Source {
                    wallet: self.wallet.clone(),
                    label: self.label.clone(),
                    kind: self.kind.clone(),
                    chain,
                    network,
                    script,
                    source: "sync_target",
                    deprecated: self.deprecated,
                    count: 0,
                    branch: target.get("branch_index").and_then(json_integer),
                    index: target.get("address_index").and_then(json_integer),
                })
            })
            .collect()
    }

    fn derive(&self, wallet: &Descriptor, branch: u32, start: i64, end: i64) -> Vec<Source> {
        let (Ok(start), Ok(end)) = (u32::try_from(start), u32::try_from(end)) else {
            return vec![];
        };
        let Ok(targets) = wallet.plan.derive(Some(branch), start, end) else {
            return vec![];
        };
        targets
            .iter()
            .map(|target| Source {
                wallet: wallet.wallet.clone(),
                label: wallet.label.clone(),
                kind: wallet.kind.clone(),
                chain: wallet.plan.chain.clone(),
                network: wallet.plan.network.clone(),
                script: kassiber_pytext::lower(&json_text(&target["script_pubkey"])),
                source: "descriptor_config",
                deprecated: wallet.deprecated,
                count: *self.counts.get(&wallet.wallet).unwrap_or(&0),
                branch: json_integer(&target["branch_index"]),
                index: json_integer(&target["address_index"]),
            })
            .collect()
    }

    /// `_descriptor_config_scripts`: each branch up to its highest known use
    /// plus the gap, merging the candidate's own target horizon.
    fn descriptor_sources(&self, candidate: &BTreeMap<u32, i64>) -> Vec<Source> {
        let mut sources = Vec::new();
        for wallet in &self.descriptors {
            let gap = i64::from(wallet.plan.gap_limit);
            for branch in &wallet.plan.branches {
                let mut known = self
                    .maxes
                    .get(&(wallet.wallet.clone(), branch.index))
                    .copied();
                if wallet.wallet == self.wallet {
                    if let Some(max) = candidate.get(&branch.index) {
                        known = Some(known.unwrap_or(-1).max(*max));
                    }
                }
                let end = known
                    .map_or(gap, |max| max.saturating_add(gap + 1))
                    .clamp(0, MAX_TARGETS_PER_BRANCH);
                sources.extend(self.derive(wallet, branch.index, 0, end));
            }
        }
        sources
    }

    /// `_candidate_descriptor_supplements`: only the candidate's branches whose
    /// horizon grew past the index's baseline.
    fn supplements(&self, baseline: &[Source], candidate: &BTreeMap<u32, i64>) -> Vec<Source> {
        if candidate.is_empty() {
            return vec![];
        }
        let mut ends: HashMap<u32, i64> = HashMap::new();
        for source in baseline {
            if source.wallet != self.wallet {
                continue;
            }
            if let (Some(branch), Some(index)) = (source.branch, source.index) {
                if let Ok(branch) = u32::try_from(branch) {
                    let end = ends.entry(branch).or_insert(0);
                    *end = (*end).max(index.saturating_add(1));
                }
            }
        }
        let mut sources = Vec::new();
        for wallet in self.descriptors.iter().filter(|w| w.wallet == self.wallet) {
            let gap = i64::from(wallet.plan.gap_limit);
            for branch in &wallet.plan.branches {
                let Some(max) = candidate.get(&branch.index) else {
                    continue;
                };
                let start = *ends.get(&branch.index).unwrap_or(&0);
                let end = start
                    .max(max.saturating_add(gap + 1))
                    .min(MAX_TARGETS_PER_BRANCH);
                if end <= start {
                    continue;
                }
                sources.extend(self.derive(wallet, branch.index, start, end));
            }
        }
        sources
    }

    fn baseline(&self) -> Vec<Source> {
        self.descriptor_sources(&BTreeMap::new())
    }

    /// The scripts `filter_sync_state_for_canonical_owner` drops from
    /// `targets`. `indexed` selects the prefetch's operation-scoped index.
    pub(crate) fn dropped(
        &self,
        targets: &[Value],
        chain: &str,
        network: &str,
        indexed: bool,
    ) -> BTreeSet<String> {
        let candidates = self.candidates(targets, chain, network);
        if candidates.is_empty() {
            return BTreeSet::new();
        }
        let mut horizon: BTreeMap<u32, i64> = BTreeMap::new();
        for source in &candidates {
            if let (Some(branch), Some(index)) = (source.branch, source.index) {
                if let Ok(branch) = u32::try_from(branch) {
                    let max = horizon.entry(branch).or_insert(-1);
                    *max = (*max).max(index);
                }
            }
        }
        let mut sources: Vec<Source> = Vec::new();
        sources.extend(self.lists.iter().cloned());
        sources.extend(self.inventory.iter().cloned());
        if indexed {
            let baseline = self.baseline();
            let supplements = self.supplements(&baseline, &horizon);
            sources.extend(baseline);
            sources.extend(candidates);
            sources.extend(supplements);
        } else {
            sources.extend(candidates);
            sources.extend(self.descriptor_sources(&horizon));
        }
        let mut order: Vec<(String, String, String)> = Vec::new();
        let mut groups: HashMap<(String, String, String), Vec<&Source>> = HashMap::new();
        for source in &sources {
            let included = source.wallet == self.wallet || !source.deprecated || source.count > 0;
            if !included || source.chain.is_empty() || source.network.is_empty() {
                continue;
            }
            let (chain, network) = scope(&source.chain, &source.network);
            if chain.is_empty() || network.is_empty() {
                continue;
            }
            let key = (chain, network, source.script.clone());
            let group = groups.entry(key.clone()).or_default();
            if group.is_empty() {
                order.push(key);
            }
            group.push(source);
        }
        let mut dropped = BTreeSet::new();
        for key in &order {
            let group = &groups[key];
            let wallets: BTreeSet<&str> = group.iter().map(|s| s.wallet.as_str()).collect();
            if wallets.len() < 2 || !wallets.contains(self.wallet.as_str()) {
                continue;
            }
            let current: Vec<&&Source> = group
                .iter()
                .filter(|s| s.wallet == self.wallet && s.source == "sync_target")
                .collect();
            if current.is_empty() || recommended(group) == self.wallet {
                continue;
            }
            dropped.extend(current.iter().map(|s| s.script.clone()));
        }
        dropped
    }
}

/// `_overlap_payload(...)["recommended_canonical_wallet_id"]`: each wallet's
/// row aggregates all its sources (retired if any is, its highest activity,
/// its best source priority), so the source order never changes the owner.
fn recommended(group: &[&Source]) -> String {
    let priority = |source: &str| match source {
        "inventory" | "descriptor_config" => 0,
        "address_list" => 1,
        "sync_target" => 2,
        _ => 99,
    };
    struct Row<'a> {
        first: &'a Source,
        deprecated: bool,
        count: i64,
        best: i64,
    }
    let mut rows: BTreeMap<&str, Row> = BTreeMap::new();
    for source in group {
        let row = rows.entry(source.wallet.as_str()).or_insert(Row {
            first: source,
            deprecated: false,
            count: 0,
            best: 99,
        });
        row.deprecated |= source.deprecated;
        row.count = row.count.max(source.count);
        row.best = row.best.min(priority(source.source));
    }
    rows.values()
        .min_by_key(|row| {
            (
                row.deprecated,
                !matches!(
                    row.first.kind.as_str(),
                    "descriptor" | "xpub" | "silent-payment"
                ),
                row.best,
                row.count == 0,
                row.first.label.clone(),
                row.first.wallet.clone(),
            )
        })
        .map(|row| row.first.wallet.clone())
        .unwrap_or_default()
}

/// `_observer_discovered_targets`: owned scripts a dependency observer found,
/// from its outputs, records' owned scripts and owned graph entries.
pub(crate) fn discovered(updates: &[&Value]) -> Vec<Value> {
    let mut targets: Vec<Value> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut add = |value: &Value| {
        let mut target = if value.is_object() {
            value.clone()
        } else {
            json!({"script_pubkey": value})
        };
        let raw = if descriptors::truthy(&target["script_pubkey"]) {
            &target["script_pubkey"]
        } else {
            &target["scriptpubkey"]
        };
        let script = lower(&json_text(raw));
        if script.is_empty() || script_hex(&script).is_none() {
            return;
        }
        target["script_pubkey"] = json!(script);
        if seen.insert(script) {
            targets.push(target);
        }
    };
    for update in updates {
        let facts = &update["facts"];
        if !facts.is_object() {
            continue;
        }
        for output in facts["outputs"].as_array().into_iter().flatten() {
            add(output);
        }
        for record in facts["transaction_records"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if !record.is_object() {
                continue;
            }
            let raw = match &record["raw_json"] {
                Value::String(text) => match serde_json::from_str::<Value>(text) {
                    Ok(value @ Value::Object(_)) => value,
                    _ => continue,
                },
                value @ Value::Object(_) => value.clone(),
                Value::Null => json!({}),
                _ => continue,
            };
            for script in raw["observer_owned_scripts"]
                .as_array()
                .into_iter()
                .flatten()
            {
                add(script);
            }
            let entries = raw["vin"].as_array().into_iter().flatten();
            for entry in entries.chain(raw["vout"].as_array().into_iter().flatten()) {
                if !entry.is_object() {
                    continue;
                }
                let owned = if entry["prevout"].is_object() {
                    &entry["prevout"]
                } else {
                    entry
                };
                if owned["role"] == "owned" {
                    add(owned);
                }
            }
        }
    }
    targets
}

/// The dependency route's expanded state: the preflight targets keyed by
/// script, then each newly discovered one, or `None` when nothing is new.
pub(crate) fn expanded(targets: &[Value], discovered: &[Value]) -> Option<Vec<Value>> {
    let actual: Vec<&Value> = targets
        .iter()
        .map(|t| &t["script_pubkey"])
        .filter(|s| descriptors::truthy(s))
        .collect();
    let new: Vec<&Value> = discovered
        .iter()
        .filter(|t| !actual.contains(&&t["script_pubkey"]))
        .collect();
    if new.is_empty() {
        return None;
    }
    let mut order: Vec<String> = Vec::new();
    let mut by_script: HashMap<String, Value> = HashMap::new();
    for target in targets
        .iter()
        .filter(|t| descriptors::truthy(&t["script_pubkey"]))
    {
        let key = lower(&json_text(&target["script_pubkey"]));
        if by_script.insert(key.clone(), target.clone()).is_none() {
            order.push(key);
        }
    }
    for target in new {
        let key = json_text(&target["script_pubkey"]);
        if !by_script.contains_key(&key) {
            by_script.insert(key.clone(), target.clone());
            order.push(key);
        }
    }
    Some(
        order
            .into_iter()
            .map(|k| by_script.remove(&k).expect("ordered key"))
            .collect(),
    )
}

/// `_filter_sync_targets`: the targets that remain, in order.
pub(crate) fn keep(targets: &[Value], dropped: &BTreeSet<String>) -> Vec<Value> {
    targets
        .iter()
        .filter(|target| !dropped.contains(&lower(&json_text(&target["script_pubkey"]))))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests;

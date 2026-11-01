use crate::python_text::PythonText;
use crate::{codec, evidence, graph};
use kassiber_pyjson::BigInt;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

type Scope = (String, String, String);
type Outpoint = (String, String, String, i64);
type MemoKey = (String, String, String, String);
type Ancestors = Option<Vec<(usize, i128)>>;

pub fn privacy_hop(row: &Value) -> Option<String> {
    let raw = codec::text(&row["privacy_boundary"])
        .py_strip()
        .py_lower()
        .replace('-', "_");
    [
        "coinjoin",
        "collaborative",
        "payjoin",
        "payment_in_coinjoin",
        "sweep",
    ]
    .contains(&raw.as_str())
    .then_some(raw)
}

pub fn scope(row: &Value) -> Option<(String, String, String, String)> {
    graph::onchain_scope(row)
}

pub fn normalize_scope(chain: &Value, network: &Value) -> Option<(String, String)> {
    let chain = evidence::chain(&codec::pystr(chain)).ok()?;
    let network = evidence::network(&chain, network).ok()?;
    Some((chain, network))
}

pub fn txid(v: &Value) -> Option<String> {
    graph::strict_txid(v)
}

fn asset(value: &Value) -> String {
    let raw = codec::text(value);
    let raw = raw.py_strip();
    let lower = raw.py_lower();
    if lower.len() == 64 && lower.bytes().all(|c| c.is_ascii_hexdigit()) {
        lower
    } else if ["l-btc", "lbtc"].contains(&lower.as_str()) {
        "LBTC".into()
    } else {
        raw.py_upper()
    }
}

fn amount(row: &Value) -> i128 {
    codec::int(&row["amount"]) as i128
}

fn skip(row: &Value) -> bool {
    if privacy_hop(row).is_some() {
        return true;
    }
    let config = graph::transfer_object(&row["wallet_config_json"]);
    let samourai = &config["samourai"];
    samourai.is_object()
        && samourai["role"] == "child"
        && !codec::text(&samourai["group_id"]).trim().is_empty()
        && !codec::text(&samourai["section"]).trim().is_empty()
}

fn authoritative(row: &Value) -> bool {
    evidence::authority(row)
        .ok()
        .is_some_and(|a| a["authoritative"] == true)
}

pub fn weighted(weights: &[i128], target: i128) -> Vec<i128> {
    let total: i128 = weights.iter().map(|&n| n.max(0)).sum();
    if target <= 0 || total <= 0 {
        return vec![0; weights.len()];
    }
    let exact: Vec<_> = weights
        .iter()
        .map(|&n| BigInt::from(n.max(0)) * BigInt::from(target))
        .collect();
    let divisor = BigInt::from(total);
    let mut allocations: Vec<i128> = exact
        .iter()
        .map(|n| {
            (n / &divisor)
                .to_string()
                .parse()
                .expect("allocation bounded by target")
        })
        .collect();
    let remainder = target - allocations.iter().sum::<i128>();
    let mut positions: Vec<_> = (0..weights.len()).collect();
    positions.sort_by_key(|&i| (-(exact[i].clone() % &divisor), i));
    for &i in positions.iter().take(remainder as usize) {
        allocations[i] += 1;
    }
    allocations
}

struct Deriver<'a> {
    rows: &'a [Value],
    scopes: Vec<Option<Scope>>,
    groups: BTreeMap<Scope, Vec<usize>>,
    blocked: BTreeSet<Scope>,
    owned: BTreeMap<Outpoint, (String, String, bool)>,
    memo: HashMap<MemoKey, Ancestors>,
}

impl<'a> Deriver<'a> {
    fn new(rows: &'a [Value], owned: &[Value]) -> Self {
        let scopes: Vec<_> = rows
            .iter()
            .map(|row| scope(row).map(|s| (s.0, s.1, s.2)))
            .collect();
        let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
        let mut blocked = BTreeSet::new();
        for (i, scope) in scopes.iter().enumerate() {
            if let Some(scope) = scope {
                groups.entry(scope.clone()).or_default().push(i);
                if skip(&rows[i]) {
                    blocked.insert(scope.clone());
                }
            }
        }
        let mut inventory = BTreeMap::new();
        for row in owned {
            let Some((chain, network)) = normalize_scope(&row["chain"], &row["network"]) else {
                continue;
            };
            let Some(txid) = txid(&row["txid"]) else {
                continue;
            };
            let Ok(vout) = codec::integer(&row["vout"])
                .and_then(|n| n.as_i64().ok_or_else(|| "vout out of range".into()))
            else {
                continue;
            };
            if vout < 0 {
                continue;
            }
            let asset = asset(&row["asset"]);
            let identity = if chain == "liquid" {
                let raw = graph::transfer_object(&row["raw_json"]);
                let raw_asset = txid_value(&raw["asset_id"]);
                let display = txid_value(&Value::String(asset.clone()));
                if !raw["asset_id"].is_null() && raw["asset_id"] != "" && raw_asset.is_none()
                    || raw_asset.is_some() && display.is_some() && raw_asset != display
                {
                    continue;
                }
                let Some(identity) = raw_asset.or(display) else {
                    continue;
                };
                identity
            } else {
                asset
            };
            inventory
                .entry((chain, network, txid, vout))
                .and_modify(|entry: &mut (String, String, bool)| entry.2 = true)
                .or_insert((codec::text(&row["wallet_id"]), identity, false));
        }
        Self {
            rows,
            scopes,
            groups,
            blocked,
            owned: inventory,
            memo: HashMap::new(),
        }
    }

    fn inputs(&self, i: usize, chain: &str, network: &str) -> Option<Vec<(String, i128)>> {
        let row = &self.rows[i];
        let parsed = graph::parse(&row["raw_json"]).ok()??;
        let entries = parsed["inputs"].as_array()?;
        let outer = graph::transfer_object(&row["raw_json"]);
        let declared = [&outer, &outer["tx"], &outer["ownership_graph"]]
            .into_iter()
            .find_map(|v| v["vin"].as_array().map(Vec::len))
            .unwrap_or(0);
        if entries.is_empty() || declared != entries.len() {
            return None;
        }
        let scripts: BTreeSet<_> = outer["observer_owned_scripts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| codec::truth(v))
            .map(codec::pystr)
            .collect();
        let asset = asset(&row["asset"]);
        let identity = if chain == "liquid" {
            txid_value(&Value::String(asset))?
        } else {
            asset
        };
        let mut result = Vec::new();
        for entry in entries {
            let outpoint = codec::text(&entry["outpoint"]);
            let (txid, vout) = outpoint.split_once(':')?;
            let txid = txid_value(&Value::String(txid.into()))?;
            let vout: i64 = vout.parse().ok()?;
            let script = codec::text(&entry["script"]);
            let sats = codec::integer(&entry["value_sats"]).ok()?.as_i64()?;
            if vout < 0
                || script.is_empty()
                || !scripts.contains(&script) && entry["role"] != "owned"
            {
                return None;
            }
            if let Some((wallet, asset, ambiguous)) =
                self.owned
                    .get(&(chain.into(), network.into(), txid.clone(), vout))
            {
                if *ambiguous || *wallet != codec::text(&row["wallet_id"]) || *asset != identity {
                    return None;
                }
            }
            result.push((txid, sats as i128 * 1000));
        }
        Some(result)
    }

    fn ancestors(
        &mut self,
        txid: String,
        wallet: &str,
        chain: &str,
        network: &str,
        budget: &mut i64,
    ) -> Ancestors {
        struct Frame {
            txid: String,
            inputs: Option<Vec<(String, i128)>>,
            next: usize,
            carried: BTreeMap<usize, i128>,
        }
        let key = |txid: &str| {
            (
                wallet.to_owned(),
                chain.to_owned(),
                network.to_owned(),
                txid.to_owned(),
            )
        };
        if let Some(hit) = self.memo.get(&key(&txid)) {
            return hit.clone();
        }
        let frame = |txid| Frame {
            txid,
            inputs: None,
            next: 0,
            carried: BTreeMap::new(),
        };
        let mut stack = vec![frame(txid.clone())];
        while let Some(current) = stack.last_mut() {
            let finish;
            if current.inputs.is_none() {
                *budget -= 1;
                let scope = (chain.to_owned(), network.to_owned(), current.txid.clone());
                let candidates = self.groups.get(&scope).cloned().unwrap_or_default();
                let matching = |direction: &str| {
                    candidates
                        .iter()
                        .copied()
                        .filter(|&i| {
                            self.rows[i]["wallet_id"] == wallet
                                && self.rows[i]["direction"] == direction
                        })
                        .collect::<Vec<_>>()
                };
                let inbound = matching("inbound");
                let outbound = matching("outbound");
                if *budget < 0 || self.blocked.contains(&scope) || inbound.len() > 1 {
                    finish = Some(None);
                } else if let Some(&i) = inbound.first() {
                    finish = Some(
                        (authoritative(&self.rows[i]) && amount(&self.rows[i]) > 0)
                            .then(|| vec![(i, amount(&self.rows[i]))]),
                    );
                } else if outbound.len() != 1 || !authoritative(&self.rows[outbound[0]]) {
                    finish = Some(None);
                } else if let Some(inputs) = self.inputs(outbound[0], chain, network) {
                    current.inputs = Some(inputs);
                    continue;
                } else {
                    finish = Some(None);
                }
            } else if let Some((parent, msat)) = current
                .inputs
                .as_ref()
                .and_then(|inputs| inputs.get(current.next))
                .cloned()
            {
                let parent_key = key(&parent);
                if !self.memo.contains_key(&parent_key) {
                    if stack.iter().any(|f| f.txid == parent) {
                        self.memo.insert(parent_key.clone(), None);
                    } else {
                        stack.push(frame(parent));
                        continue;
                    }
                }
                let current = stack.last_mut().unwrap();
                if let Some(upstream) = self.memo[&parent_key].as_ref() {
                    for ((i, _), share) in upstream.iter().zip(weighted(
                        &upstream.iter().map(|p| p.1).collect::<Vec<_>>(),
                        msat,
                    )) {
                        if share > 0 {
                            *current.carried.entry(*i).or_default() += share;
                        }
                    }
                    current.next += 1;
                    continue;
                } else {
                    finish = Some(None);
                }
            } else {
                let mut result: Vec<_> = current.carried.iter().map(|(&i, &n)| (i, n)).collect();
                result.sort_by_key(|&(i, _)| codec::text(&self.rows[i]["id"]));
                finish = Some((!result.is_empty()).then_some(result));
            }
            if let Some(result) = finish {
                let current = stack.pop().unwrap();
                self.memo.insert(key(&current.txid), result);
            }
        }
        self.memo[&key(&txid)].clone()
    }
}

fn txid_value(v: &Value) -> Option<String> {
    txid(v)
}

pub struct StructuralPair {
    pub from: String,
    pub to: String,
    pub allocation: i128,
    pub from_allocation: i128,
    pub passthrough: bool,
}

pub fn structural_pairs(
    rows: &[Value],
    owned: &[Value],
) -> BTreeMap<(String, String), (i128, i128)> {
    structural_pair_details(rows, owned)
        .into_iter()
        .map(|pair| {
            (
                (pair.from, pair.to),
                (pair.allocation, pair.from_allocation),
            )
        })
        .collect()
}

pub fn structural_pair_details(rows: &[Value], owned: &[Value]) -> Vec<StructuralPair> {
    let mut deriver = Deriver::new(rows, owned);
    let mut pairs = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        if row["direction"] != "outbound" || !authoritative(row) {
            continue;
        }
        let Some(scope) = deriver.scopes[i].clone() else {
            continue;
        };
        if deriver.blocked.contains(&scope) {
            continue;
        }
        let wallet = codec::text(&row["wallet_id"]);
        if deriver.groups[&scope]
            .iter()
            .filter(|&&j| rows[j]["wallet_id"] == wallet && rows[j]["direction"] == "outbound")
            .count()
            != 1
        {
            continue;
        }
        let Some(inputs) = deriver.inputs(i, &scope.0, &scope.1) else {
            continue;
        };
        let mut contributions = BTreeMap::<String, i128>::new();
        for (txid, msat) in inputs {
            *contributions.entry(txid).or_default() += msat;
        }
        let mut budget = 5000;
        let mut parents = BTreeMap::<String, (usize, i128)>::new();
        let mut complete = true;
        for (txid, gross) in &contributions {
            let Some(upstream) =
                deriver.ancestors(txid.clone(), &wallet, &scope.0, &scope.1, &mut budget)
            else {
                complete = false;
                break;
            };
            for ((parent, _), share) in upstream.iter().zip(weighted(
                &upstream.iter().map(|p| p.1).collect::<Vec<_>>(),
                *gross,
            )) {
                if share > 0 {
                    parents
                        .entry(codec::text(&rows[*parent]["id"]))
                        .or_insert((*parent, 0))
                        .1 += share;
                }
            }
        }
        if !complete
            || parents.is_empty()
            || parents.values().any(|&(j, n)| n > amount(&rows[j]))
            || amount(row) <= 0
        {
            continue;
        }
        let passthrough = parents.values().any(|&(j, _)| {
            !contributions.contains_key(&codec::text(&rows[j]["external_id"]).py_lower())
        });
        let shares = weighted(
            &parents.values().map(|p| p.1).collect::<Vec<_>>(),
            amount(row),
        );
        for ((parent, (_, gross)), share) in parents.into_iter().zip(shares) {
            if share > 0 {
                pairs.push(StructuralPair {
                    from: parent,
                    to: codec::text(&row["id"]),
                    allocation: share,
                    from_allocation: gross,
                    passthrough,
                });
            }
        }
    }
    codec::clear_memos();
    pairs
}

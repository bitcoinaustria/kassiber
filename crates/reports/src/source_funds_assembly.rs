//! Intra-wallet provenance only. Neither graph structure nor this projection
//! creates an ownership claim, custody decision, journal row or tax input.
use super::*;
use kassiber_pyjson::BigInt;
use std::collections::{BTreeMap, HashMap, HashSet};

type Scope = (String, String, String);
type Outpoint = (String, String, String, BigInt);
type Distribution = BTreeMap<usize, BigInt>;
type MemoKey = (String, Scope);
type Input = (String, BigInt);
type Inventory = HashMap<Outpoint, Option<(String, String)>>;

pub(super) struct Pair {
    pub from: String,
    pub to: String,
    pub allocation: BigInt,
    pub gross: BigInt,
}

fn json(v: &Value) -> serde_json::Value {
    kassiber_custody::codec::to_json(v)
}
pub(super) fn mapping(v: &Value) -> Value {
    match v {
        Value::Dict(_) => v.clone(),
        Value::Str(raw) => raw
            .as_str()
            .and_then(|s| kassiber_pytext::loads(s).ok())
            .filter(|v| matches!(v, Value::Dict(_)))
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}
pub(super) fn integer(v: &Value) -> Result<BigInt> {
    match v {
        Value::Int(n) => Ok(n.clone()),
        Value::Bool(b) => Ok(i32::from(*b).into()),
        Value::Float(n) if n.is_finite() => Ok(format!("{:.0}",n.trunc()).parse().expect("finite integer")),
        Value::Float(n) => Err(AppError::new("internal_error",if n.is_nan() {"cannot convert float NaN to integer"} else {"cannot convert float infinity to integer"})),
        Value::Str(raw) => kassiber_pytext::parse_int(raw.as_str().unwrap_or("")).map_err(|error| AppError::new("internal_error",match error {
            kassiber_pytext::NumberError::Invalid=>format!("invalid literal for int() with base 10: {}",kassiber_pytext::repr_str(raw.as_str().unwrap_or(""))),
            kassiber_pytext::NumberError::TooManyDigits(n)=>format!("Exceeds the limit (4300 digits) for integer string conversion: value has {n} digits; use sys.set_int_max_str_digits() to increase the limit"),
        })),
        _=>Err(AppError::new("internal_error",format!("int() argument must be a string, a bytes-like object or a real number, not '{}'",kassiber_envelope::type_name(v)))),
    }
}
fn number(row: &Value, key: &str) -> Result<BigInt> {
    if truthy(value(row, key)) {
        integer(value(row, key))
    } else {
        Ok(0.into())
    }
}
fn txid(v: &Value) -> Option<String> {
    let Value::Str(raw) = v else { return None };
    let raw = kassiber_pytext::strip(raw.as_str()?);
    (raw.len() == 64 && raw.bytes().all(|c| c.is_ascii_hexdigit()))
        .then(|| raw.to_ascii_lowercase())
}
pub(super) fn asset(v: &Value) -> String {
    if truthy(v) {
        kassiber_wallets::config::asset_code(&py_str(v))
    } else {
        String::new()
    }
}
fn scope(row: &Value) -> Option<Scope> {
    kassiber_custody::journal_support::onchain_scope(&json(row)).map(|(c, n, t, _)| (c, n, t))
}
fn authoritative(row: &Value) -> bool {
    kassiber_custody::evidence::authority(&json(row))
        .ok()
        .and_then(|v| v["authoritative"].as_bool())
        .unwrap_or(false)
}
fn blocked(row: &Value) -> bool {
    let boundary = kassiber_pytext::lower(kassiber_pytext::strip(&text(row, "privacy_boundary")))
        .replace('-', "_");
    let config = mapping(value(row, "wallet_config_json"));
    let metadata = value(&config, "samourai");
    [
        "coinjoin",
        "collaborative",
        "payjoin",
        "payment_in_coinjoin",
        "sweep",
    ]
    .contains(&boundary.as_str())
        || (text(metadata, "role") == "child"
            && truthy(value(metadata, "group_id"))
            && !kassiber_pytext::strip(&text(metadata, "group_id")).is_empty()
            && truthy(value(metadata, "section"))
            && !kassiber_pytext::strip(&text(metadata, "section")).is_empty())
}

fn inventory(conn: &Connection, profile: &str) -> Result<Inventory> {
    let mut result = HashMap::new();
    let columns = rows(conn, "PRAGMA table_info(wallet_utxos)", [])?;
    let has = |key| columns.iter().any(|r| text(r, "name") == key);
    let query = format!(
        "SELECT wallet_id,chain,{},txid,vout,asset,{} FROM wallet_utxos WHERE profile_id=?",
        if has("network") {
            "network"
        } else {
            "NULL AS network"
        },
        if has("raw_json") {
            "raw_json"
        } else {
            "NULL AS raw_json"
        }
    );
    for row in rows(conn, &query, [profile])? {
        let Some(txid) = txid(value(&row, "txid")) else {
            continue;
        };
        let Ok(vout) = integer(value(&row, "vout")) else {
            continue;
        };
        if vout < 0.into() {
            continue;
        }
        let chain = kassiber_pytext::lower(kassiber_pytext::strip(&text(&row, "chain")));
        let chain = match chain.as_str() {
            "" | "bitcoin" | "btc" | "onchain" => "bitcoin",
            "liquid" | "lbtc" | "elements" => "liquid",
            _ => continue,
        };
        let Ok(network) = kassiber_custody::evidence::network(chain, &json(value(&row, "network")))
        else {
            continue;
        };
        let display = if truthy(value(&row, "asset")) {
            asset(value(&row, "asset"))
        } else {
            "BTC".into()
        };
        let identity = if chain == "liquid" {
            let raw = mapping(value(&row, "raw_json"));
            let raw_asset = value(&raw, "asset_id");
            let raw_id = self::txid(raw_asset);
            let display_id = self::txid(&s(display.as_str()));
            if (!matches!(raw_asset, Value::Null) && raw_asset != &s("") && raw_id.is_none())
                || (raw_id.is_some() && display_id.is_some() && raw_id != display_id)
            {
                continue;
            }
            let Some(identity) = raw_id.or(display_id) else {
                continue;
            };
            identity
        } else {
            display
        };
        let key = (chain.into(), network, txid, vout);
        if let Some(previous) = result.get_mut(&key) {
            *previous = None
        } else {
            result.insert(key, Some((text(&row, "wallet_id"), identity)));
        }
    }
    Ok(result)
}

fn owned_inputs(row: &Value, scope: &Scope, index: &Inventory) -> Option<Vec<Input>> {
    let parsed = kassiber_custody::journal_support::parsed(&json(row)).ok()??;
    let parsed = kassiber_custody::codec::from_json(&parsed).ok()?;
    let Value::List(entries) = value(&parsed, "inputs") else {
        return None;
    };
    let outer = mapping(value(row, "raw_json"));
    let declared = [
        &outer,
        value(&outer, "tx"),
        value(&outer, "ownership_graph"),
    ]
    .into_iter()
    .find_map(|v| match value(v, "vin") {
        Value::List(v) => Some(v.len()),
        _ => None,
    })
    .unwrap_or(0);
    if entries.is_empty() || entries.len() != declared {
        return None;
    }
    let scripts = match value(&outer, "observer_owned_scripts") {
        Value::List(v) => v.clone(),
        _ => vec![],
    };
    let mut result = Vec::new();
    for entry in entries {
        let outpoint = text(entry, "outpoint");
        let (t, v) = outpoint.split_once(':')?;
        let t = txid(&s(t))?;
        let v = integer(&s(v)).ok()?;
        let amount = integer(value(entry, "value_sats")).ok()?;
        let script = text(entry, "script");
        if v < 0.into()
            || script.is_empty()
            || (!scripts.contains(&s(script.as_str())) && text(entry, "role") != "owned")
        {
            return None;
        }
        if let Some(info) = index.get(&(scope.0.clone(), scope.1.clone(), t.clone(), v)) {
            let (wallet, identity) = info.as_ref()?;
            let asset = asset(value(row, "asset"));
            let asset = if scope.0 == "liquid" {
                txid(&s(asset)).unwrap_or_default()
            } else {
                asset
            };
            if *wallet != text(row, "wallet_id") || *identity != asset {
                return None;
            }
        }
        result.push((t, amount * 1000));
    }
    Some(result)
}

fn allocate(weights: &[BigInt], target: &BigInt) -> Vec<BigInt> {
    let total: BigInt = weights.iter().filter(|n| **n > 0.into()).sum();
    if *target <= 0.into() || total <= 0.into() {
        return vec![0.into(); weights.len()];
    }
    let products: Vec<_> = weights.iter().map(|n| n.max(&0.into()) * target).collect();
    let mut result: Vec<_> = products.iter().map(|n| n / &total).collect();
    let mut order: Vec<_> = (0..weights.len()).collect();
    order.sort_by_key(|&i| (-(products[i].clone() % &total), i));
    let remainder = target - result.iter().sum::<BigInt>();
    let count =
        usize::try_from(remainder).expect("largest remainder is bounded by number of weights");
    for i in order.into_iter().take(count) {
        result[i] += 1
    }
    result
}
fn carry(into: &mut Distribution, upstream: &Distribution, amount: &BigInt, rows: &[Value]) {
    let mut ordered: Vec<_> = upstream.iter().collect();
    ordered.sort_by_key(|(i, _)| text(&rows[**i], "id"));
    let shares = allocate(
        &ordered
            .iter()
            .map(|(_, n)| (*n).clone())
            .collect::<Vec<_>>(),
        amount,
    );
    for ((&parent, _), share) in ordered.into_iter().zip(shares) {
        if share > 0.into() {
            *into.entry(parent).or_default() += share
        }
    }
}

struct Frame {
    key: MemoKey,
    path: HashSet<String>,
    inputs: Option<Vec<Input>>,
    next: usize,
    carried: Distribution,
}
impl Frame {
    fn new(key: MemoKey, path: HashSet<String>) -> Self {
        Self {
            key,
            path,
            inputs: None,
            next: 0,
            carried: BTreeMap::new(),
        }
    }
}
struct Deriver<'a> {
    rows: &'a [Value],
    scopes: HashMap<Scope, Vec<usize>>,
    blocked: HashSet<Scope>,
    inventory: Inventory,
    memo: HashMap<MemoKey, Option<Distribution>>,
}
impl Deriver<'_> {
    fn ancestors(&mut self, key: MemoKey, budget: &mut i32) -> Result<Option<Distribution>> {
        if let Some(result) = self.memo.get(&key) {
            return Ok(result.clone());
        }
        let mut stack = vec![Frame::new(key.clone(), HashSet::new())];
        while let Some(frame) = stack.last_mut() {
            let current = frame.key.clone();
            let mut finish = None;
            if frame.inputs.is_none() {
                *budget -= 1;
                let candidates = self.scopes.get(&current.1).cloned().unwrap_or_default();
                let candidates: Vec<_> = candidates
                    .into_iter()
                    .filter(|&i| text(&self.rows[i], "wallet_id") == current.0)
                    .collect();
                let inbound: Vec<_> = candidates
                    .iter()
                    .copied()
                    .filter(|&i| text(&self.rows[i], "direction") == "inbound")
                    .collect();
                if *budget < 0 || self.blocked.contains(&current.1) || inbound.len() > 1 {
                    finish = Some(None)
                } else if let Some(&parent) = inbound.first() {
                    let amount = number(&self.rows[parent], "amount")?;
                    finish = Some(
                        (authoritative(&self.rows[parent]) && amount > 0.into())
                            .then(|| BTreeMap::from([(parent, amount)])),
                    );
                } else {
                    let outbound: Vec<_> = candidates
                        .into_iter()
                        .filter(|&i| text(&self.rows[i], "direction") == "outbound")
                        .collect();
                    frame.inputs = if outbound.len() == 1 && authoritative(&self.rows[outbound[0]])
                    {
                        owned_inputs(&self.rows[outbound[0]], &current.1, &self.inventory)
                    } else {
                        None
                    };
                    if frame.inputs.is_none() {
                        finish = Some(None)
                    }
                }
            } else if let Some(inputs) = &frame.inputs {
                if let Some((txid, amount)) = inputs.get(frame.next) {
                    let child = (
                        current.0.clone(),
                        (current.1 .0.clone(), current.1 .1.clone(), txid.clone()),
                    );
                    if let Some(upstream) = self.memo.get(&child) {
                        if let Some(upstream) = upstream {
                            carry(&mut frame.carried, upstream, amount, self.rows);
                            frame.next += 1
                        } else {
                            finish = Some(None)
                        }
                    } else if *txid == current.1 .2 || frame.path.contains(txid) {
                        self.memo.insert(child, None);
                    } else {
                        let mut path = frame.path.clone();
                        path.insert(current.1 .2.clone());
                        stack.push(Frame::new(child, path));
                    }
                } else {
                    finish = Some((!frame.carried.is_empty()).then(|| frame.carried.clone()))
                }
            }
            if let Some(result) = finish {
                self.memo.insert(current, result);
                stack.pop();
            }
        }
        Ok(self.memo.get(&key).cloned().flatten())
    }
}

pub(super) fn derive(conn: &Connection, profile: &str, rows: &[Value]) -> Result<Vec<Pair>> {
    derive_loaded(rows, inventory(conn, profile)?)
}

fn derive_loaded(rows: &[Value], inventory: Inventory) -> Result<Vec<Pair>> {
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            kassiber_custody::journal_support::clear_memos();
        }
    }
    let _clear = Clear;
    let scopes: Vec<_> = rows.iter().map(scope).collect();
    let mut deriver = Deriver {
        rows,
        scopes: HashMap::new(),
        blocked: HashSet::new(),
        inventory,
        memo: HashMap::new(),
    };
    for (i, scope) in scopes.iter().enumerate() {
        if let Some(scope) = scope {
            deriver.scopes.entry(scope.clone()).or_default().push(i);
            if blocked(&rows[i]) {
                deriver.blocked.insert(scope.clone());
            }
        }
    }
    let mut result = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let Some(scope) = &scopes[i] else { continue };
        let wallet = text(row, "wallet_id");
        if text(row, "direction") != "outbound"
            || deriver.blocked.contains(scope)
            || !authoritative(row)
            || deriver.scopes[scope]
                .iter()
                .filter(|&&i| {
                    text(&rows[i], "wallet_id") == wallet
                        && text(&rows[i], "direction") == "outbound"
                })
                .count()
                != 1
        {
            continue;
        }
        let Some(inputs) = owned_inputs(row, scope, &deriver.inventory) else {
            continue;
        };
        let mut contributions = BTreeMap::<String, BigInt>::new();
        for (txid, amount) in inputs {
            *contributions.entry(txid).or_default() += amount
        }
        let mut parents = Distribution::new();
        let mut budget = 5000;
        let mut resolved = true;
        for (txid, amount) in contributions {
            let Some(upstream) = deriver.ancestors(
                (wallet.clone(), (scope.0.clone(), scope.1.clone(), txid)),
                &mut budget,
            )?
            else {
                resolved = false;
                break;
            };
            carry(&mut parents, &upstream, &amount, rows);
        }
        if !resolved || parents.is_empty() {
            continue;
        }
        // Python orders shares by row id, independently of transaction chronology.
        let mut parents: Vec<_> = parents.into_iter().collect();
        parents.sort_by_key(|(i, _)| text(&rows[*i], "id"));
        let mut excessive = false;
        for (parent, gross) in &parents {
            if *gross > number(&rows[*parent], "amount")? {
                excessive = true
            }
        }
        if excessive {
            continue;
        }
        let shares = allocate(
            &parents.iter().map(|(_, n)| n.clone()).collect::<Vec<_>>(),
            &number(row, "amount")?,
        );
        for ((parent, gross), allocation) in parents.into_iter().zip(shares) {
            if allocation > 0.into() {
                result.push(Pair {
                    from: text(&rows[parent], "id"),
                    to: text(row, "id"),
                    allocation,
                    gross,
                })
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_funds_structural_lineage_pins_authority_wallets_change_chains_and_exact_msat() {
        let fixture =
            kassiber_pytext::loads(include_str!("../tests/data/source_funds_batch.json")).unwrap();
        let Value::List(vectors) = value(&fixture, "structural_vectors") else {
            panic!()
        };
        assert!(vectors.len() >= 20);
        for case in vectors {
            let Value::List(rows) = value(case, "rows") else {
                panic!()
            };
            let Value::List(entries) = value(case, "inventory") else {
                panic!()
            };
            let mut index = Inventory::new();
            for entry in entries {
                let Value::List(key) = value(entry, "key") else {
                    panic!()
                };
                let info = value(entry, "info");
                index.insert(
                    (
                        py_str(&key[0]),
                        py_str(&key[1]),
                        py_str(&key[2]),
                        integer(&key[3]).unwrap(),
                    ),
                    (!truthy(value(info, "ambiguous")))
                        .then(|| (text(info, "wallet_id"), text(info, "asset_identity"))),
                );
            }
            let pairs = derive_loaded(rows, index).unwrap();
            let pairs = Value::List(
                pairs
                    .into_iter()
                    .map(|p| {
                        dict([
                            ("from_transaction_id", s(p.from)),
                            ("to_transaction_id", s(p.to)),
                            ("allocation", Value::Int(p.allocation)),
                            ("gross", Value::Int(p.gross)),
                        ])
                    })
                    .collect(),
            );
            assert_eq!(pairs, *value(case, "pairs"), "{}", text(case, "name"));
        }
    }
}

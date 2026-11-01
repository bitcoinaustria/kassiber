//! Python channel_lifecycle parity. These are internal probes and cloned legs,
//! never new snapshot facts or independent custody authority.
use super::{fanout::number, text};
use crate::{
    codec,
    graph::{self, stored},
    python_text::PythonText,
    snapshot::Snapshot,
};
use kassiber_pyjson::{BigInt, Value as Py};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

type Scope = (String, String, String);
type Outpoint = (String, BigInt);
type Sweeps = BTreeSet<(String, Outpoint)>;
type Wallets = BTreeMap<String, Value>;
const OPEN: &str = "channel_open";
const CLOSE: &str = "channel_close";
const OPEN_MISMATCH: &str = "channel_open_mismatch";
const CLOSE_MISMATCH: &str = "channel_close_mismatch";
fn amount(row: &Value, key: &str) -> Result<BigInt, String> {
    let value = codec::from_json(&row[key])?;
    if stored::truth(&value) {
        stored::integer(&value).map_err(str::to_owned)
    } else {
        Ok(BigInt::from(0))
    }
}
fn zero() -> BigInt {
    BigInt::from(0)
}
fn absent(v: &Value) -> bool {
    v.is_null() || v == ""
}
fn pystr(v: &Value) -> String {
    text::string(&codec::from_json(v).expect("JSON value"))
}
fn raw(row: &Value) -> Py {
    let v = &row["raw_json"];
    if v.is_object() {
        return codec::from_json(v).expect("JSON object");
    }
    if !codec::truth(v) {
        return Py::Dict(Vec::new());
    }
    codec::loads(&pystr(v))
        .ok()
        .filter(|p| matches!(p, Py::Dict(_)))
        .unwrap_or(Py::Dict(Vec::new()))
}
// Canonical identities cannot contain lone surrogates. Preserve Python repr
// for containers, but decline unencodable scalar identities before UTF-8 text.
fn identity_text(v: &Py) -> Option<String> {
    if let Py::Str(value) = v {
        value.as_str().map(str::to_owned)
    } else {
        Some(text::string(v))
    }
}
fn outpoint(v: &Py) -> Option<Outpoint> {
    let spelling = if stored::truth(v) {
        identity_text(v)?
    } else {
        String::new()
    };
    let (txid, index) = spelling.py_strip().rsplit_once(':')?;
    let txid = graph::strict_txid(&json!(txid))?;
    let index = stored::integer(&Py::Str(index.into())).ok()?;
    (index >= zero()).then_some((txid, index))
}
fn txid_from_outpoint(v: &Value) -> Option<String> {
    let s = if codec::truth(v) {
        pystr(v)
    } else {
        String::new()
    };
    let s = s.py_strip();
    graph::strict_txid(&json!(s)).or_else(|| outpoint(&Py::Str(s.into())).map(|v| v.0))
}
fn tx_scope(row: &Value) -> Option<Scope> {
    // Only a row without an asset is copied to default it.
    if codec::text(&row["asset"]).py_strip().is_empty() {
        let mut probe = row.clone();
        probe["asset"] = json!("BTC");
        return bitcoin_scope(&probe);
    }
    bitcoin_scope(row)
}
fn bitcoin_scope(probe: &Value) -> Option<Scope> {
    let (chain, network, txid, asset) = graph::onchain_scope(probe)?;
    (chain == "bitcoin" && asset == "BTC").then_some((chain, network, txid))
}
fn channel_scope(row: &Value, txid: Option<String>) -> Option<Scope> {
    let txid = txid?;
    // Re-encode only the two independent scope sources, retaining Python scalars.
    let observed = raw(row);
    let mut evidence = Py::Dict(Vec::new());
    let mut direct = json!({});
    for key in ["chain", "network"] {
        let value = stored::get(&observed, key);
        if !matches!(value, Py::Null)
            && !matches!(value,Py::Str(s) if s.code_points().next().is_none())
        {
            stored::put(&mut evidence, key, value.clone());
        }
        if !absent(&row[key]) {
            direct[key] = row[key].clone();
        }
    }
    if direct.as_object().is_some_and(|v| !v.is_empty()) {
        stored::put(
            &mut evidence,
            "ownership_graph",
            codec::from_json(&direct).ok()?,
        );
    }
    let config = if codec::truth(&row["config_json"]) {
        &row["config_json"]
    } else if codec::truth(&row["wallet_config_json"]) {
        &row["wallet_config_json"]
    } else {
        &Value::Null
    };
    bitcoin_scope(
        &json!({"id":format!("channel-lifecycle:{txid}"),"external_id":txid,"external_id_kind":"txid","asset":"BTC","config_json":config,"raw_json":codec::dumps(&evidence,true).ok()?}),
    )
}
fn funding_scope(row: &Value) -> Option<Scope> {
    let txid = (!absent(&row["funding_txid"]))
        .then(|| txid_from_outpoint(&row["funding_txid"]))
        .flatten();
    let point = (!absent(&row["funding_outpoint"]))
        .then(|| txid_from_outpoint(&row["funding_outpoint"]))
        .flatten();
    if (!absent(&row["funding_txid"]) && txid.is_none())
        || (!absent(&row["funding_outpoint"]) && point.is_none())
    {
        return None;
    }
    if txid.is_some() && point.is_some() && txid != point {
        return None;
    }
    channel_scope(row, txid.or(point))
}
fn closing_scope(row: &Value) -> Option<Scope> {
    if absent(&row["closing_txid"]) {
        return None;
    }
    channel_scope(row, graph::strict_txid(&row["closing_txid"]))
}
fn vin(row: &Value) -> BTreeSet<Outpoint> {
    let payload = raw(row);
    let nested = stored::get(&payload, "tx");
    let payload = if matches!(nested, Py::Dict(_)) {
        nested
    } else {
        &payload
    };
    stored::list(stored::get(payload, "vin"))
        .iter()
        .filter(|v| matches!(v, Py::Dict(_)))
        .filter_map(|v| {
            outpoint(&Py::Str(
                format!(
                    "{}:{}",
                    identity_text(stored::get(v, "txid"))?,
                    identity_text(stored::get(v, "vout"))?
                )
                .into(),
            ))
        })
        .collect()
}
fn trusted_sweeps(row: &Value) -> Result<Sweeps, String> {
    let payload = raw(row);
    let provenance = stored::get(&payload, "_kassiber_provenance");
    let source = stored::get(provenance, "import_source");
    // Python tests membership in a set, which rejects unhashable JSON values.
    if matches!(source, Py::List(_) | Py::Dict(_)) {
        return Err("TypeError".into());
    }
    if !stored::equal(source, &Py::Str("lnd_adapter".into()), false) {
        return Ok(Sweeps::new());
    }
    Ok(
        stored::list(stored::get(&payload, "channel_close_local_sweeps"))
            .iter()
            .filter_map(|v| {
                if !matches!(v, Py::Dict(_)) {
                    return None;
                }
                let txid = stored::get(v, "sweep_txid");
                // canonical_txid requires str, unlike the outpoint parser.
                let Py::Str(txid) = txid else {
                    return None;
                };
                Some((
                    graph::strict_txid(&json!(txid.as_str()?))?,
                    outpoint(stored::get(v, "outpoint"))?,
                ))
            })
            .collect(),
    )
}
fn tolerance(amount: &BigInt) -> Result<BigInt, String> {
    let float = amount
        .to_string()
        .parse::<f64>()
        .map_err(|_| "OverflowError")?;
    if !float.is_finite() {
        return Err("OverflowError".into());
    }
    let threshold = BigInt::from_str(&format!("{:.0}", (float.abs() * 0.01).trunc()))
        .map_err(|_| "OverflowError")?;
    Ok(threshold.max(BigInt::from(2_500_000)))
}
fn close_mismatch(received: &BigInt, balance: &BigInt) -> Result<bool, String> {
    if balance < &zero() {
        return Ok(true);
    }
    if balance == &zero() {
        return Ok(false);
    }
    let gap = balance - received;
    Ok(if gap < zero() { -gap } else { gap } > tolerance(balance)?)
}
fn funding_mismatch(row: &Value, funded: &BigInt, strict: bool) -> Result<bool, String> {
    if funded < &zero() {
        return Ok(true);
    }
    if funded == &zero() {
        return Ok(false);
    }
    let recorded = amount(row, "amount")?;
    if strict {
        return Ok(&recorded != funded);
    }
    Ok(recorded - funded > tolerance(funded)?)
}
fn atomic(row: &Value) -> bool {
    let id = codec::text(&row["channel_id"]);
    id.starts_with("lnd:") || id.starts_with("coreln:")
}
#[derive(Clone)]
struct Entry {
    row: Value,
    direct: bool,
    local: BTreeSet<Outpoint>,
}
struct Group {
    legs: Vec<Value>,
    total_msat: BigInt,
    balance_msat: BigInt,
    mismatch: bool,
}
// Vec preserves Python dict insertion order: first candidate scope in tx_rows.
fn close_groups(
    keys: &BTreeSet<Scope>,
    balances: &BTreeMap<Scope, BigInt>,
    rows: &[Value],
    sweeps: &BTreeMap<Scope, Sweeps>,
) -> Result<Vec<(Scope, Group)>, String> {
    let mut candidates: Vec<(Scope, Vec<Entry>)> = Vec::new();
    for row in rows {
        if row["direction"] != "inbound" {
            continue;
        }
        let Some(key) = tx_scope(row) else {
            continue;
        };
        let direct = keys.contains(&key);
        let points = vin(row);
        let candidate_keys = if direct {
            vec![key.clone()]
        } else {
            points
                .iter()
                .map(|(txid, _)| (key.0.clone(), key.1.clone(), txid.clone()))
                .filter(|k| keys.contains(k))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        for candidate in candidate_keys {
            let local = points
                .iter()
                .filter(|point| {
                    point.0 == candidate.2
                        && sweeps
                            .get(&candidate)
                            .is_some_and(|s| s.contains(&(key.2.clone(), (*point).clone())))
                })
                .cloned()
                .collect();
            if !candidates.iter().any(|(k, _)| k == &candidate) {
                candidates.push((candidate.clone(), Vec::new()));
            }
            candidates
                .iter_mut()
                .find(|(k, _)| k == &candidate)
                .unwrap()
                .1
                .push(Entry {
                    row: row.clone(),
                    direct,
                    local,
                });
        }
    }
    let mut groups = Vec::new();
    for (key, mut entries) in candidates {
        entries.sort_by_key(|e| (codec::text(&e.row["occurred_at"]), pystr(&e.row["id"])));
        let balance = balances.get(&key).cloned().unwrap_or_else(zero);
        let mut selected: Vec<_> = entries.iter().filter(|e| e.direct).collect();
        let direct_total: BigInt = selected
            .iter()
            .map(|e| amount(&e.row, "amount"))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum();
        let matches: Vec<_> = entries.iter().filter(|e| !e.direct).collect();
        let mut ambiguous = false;
        if matches.len() == 1 {
            if balance > zero() && direct_total >= balance { /* later peer payout */
            } else {
                selected.push(matches[0]);
                ambiguous = matches[0].local.is_empty();
            }
        } else if matches.len() > 1 {
            let local: Vec<_> = matches
                .iter()
                .copied()
                .filter(|e| !e.local.is_empty())
                .collect();
            let mut seen = BTreeSet::new();
            let mut duplicate = false;
            for entry in &local {
                if !seen.is_disjoint(&entry.local) {
                    duplicate = true;
                }
                seen.extend(entry.local.iter().cloned());
            }
            if !local.is_empty() && !duplicate {
                selected.extend(local);
            } else {
                selected.extend(matches);
                ambiguous = true;
            }
        }
        let legs: Vec<_> = selected.iter().map(|e| e.row.clone()).collect();
        let total: BigInt = legs
            .iter()
            .map(|r| amount(r, "amount"))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum();
        let mismatch = ambiguous || close_mismatch(&total, &balance)?;
        groups.push((
            key,
            Group {
                legs,
                total_msat: total,
                balance_msat: balance,
                mismatch,
            },
        ));
    }
    Ok(groups)
}
fn remember(
    owners: &mut BTreeMap<Scope, String>,
    ambiguous: &mut BTreeSet<Scope>,
    key: &Scope,
    wallet: &str,
) {
    if let Some(existing) = owners.get(key) {
        if existing != wallet {
            ambiguous.insert(key.clone());
            owners.remove(key);
        }
    } else if !ambiguous.contains(key) {
        owners.insert(key.clone(), wallet.into());
    }
}
fn add_positive(map: &mut BTreeMap<Scope, BigInt>, key: &Scope, value: BigInt) {
    if value > zero() {
        *map.entry(key.clone()).or_insert_with(zero) += value;
    }
}
fn clone_leg(
    row: &Value,
    wallet: &Value,
    id: String,
    direction: &str,
    fee: &BigInt,
) -> Result<Value, String> {
    let mut row = row.clone();
    for key in ["id", "label"] {
        if wallet.get(key).is_none() {
            return Err("KeyError".into());
        }
    }
    row["journal_transaction_id"] = row["id"].clone();
    row["id"] = json!(id);
    row["description"] = json!(id);
    row["direction"] = json!(direction);
    row["fee"] = number(fee);
    row["wallet_id"] = wallet["id"].clone();
    row["wallet_label"] = wallet["label"].clone();
    for key in ["wallet_account_id", "account_code", "account_label"] {
        row[key] = wallet[key].clone();
    }
    row["kind"] = json!(if direction == "inbound" {
        "self_transfer_in"
    } else {
        "self_transfer_out"
    });
    Ok(row)
}
fn transfer_pairs(
    channels: &[Value],
    rows: &[Value],
    wallets: &Wallets,
) -> Result<Vec<Value>, String> {
    let (mut funding, mut closing) = (BTreeMap::new(), BTreeMap::new());
    let (mut ambiguous_funding, mut ambiguous_closing) = (BTreeSet::new(), BTreeSet::new());
    let (mut amounts, mut balances, mut sweeps) = (
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::<Scope, Sweeps>::new(),
    );
    let mut strict_funding = BTreeSet::new();
    let mut close_funding: BTreeMap<Scope, BTreeSet<Scope>> = BTreeMap::new();
    let mut channel_funding: BTreeMap<(String, String), BTreeSet<Scope>> = BTreeMap::new();
    let mut close_channels: BTreeMap<Scope, BTreeSet<(String, String)>> = BTreeMap::new();
    for row in channels {
        if !codec::truth(&row["wallet_id"]) {
            continue;
        }
        let wallet = pystr(&row["wallet_id"]);
        if !wallets.contains_key(&wallet) {
            continue;
        }
        let owner =
            (!absent(&row["channel_id"])).then(|| (wallet.clone(), pystr(&row["channel_id"])));
        let fund = funding_scope(row);
        if let Some(key) = &fund {
            let funded = amount(row, "funding_amount_msat")?;
            if atomic(row) {
                strict_funding.insert(key.clone());
            }
            if atomic(row) && funded <= zero() {
                funding.remove(key);
                ambiguous_funding.insert(key.clone());
            } else {
                remember(&mut funding, &mut ambiguous_funding, key, &wallet);
            }
            if let Some(owner) = &owner {
                channel_funding
                    .entry(owner.clone())
                    .or_default()
                    .insert(key.clone());
            }
            add_positive(&mut amounts, key, funded);
        }
        if let Some(key) = closing_scope(row) {
            sweeps
                .entry(key.clone())
                .or_default()
                .extend(trusted_sweeps(row)?);
            let balance = amount(row, "close_balance_msat")?;
            if atomic(row) && balance <= zero() {
                closing.remove(&key);
                ambiguous_closing.insert(key.clone());
            } else {
                remember(&mut closing, &mut ambiguous_closing, &key, &wallet);
            }
            if let Some(fund) = fund {
                close_funding.entry(key.clone()).or_default().insert(fund);
            }
            if let Some(owner) = &owner {
                close_channels
                    .entry(key.clone())
                    .or_default()
                    .insert(owner.clone());
            }
            add_positive(&mut balances, &key, balance);
        }
    }
    for (key, owners) in close_channels {
        for owner in owners {
            for fund in channel_funding.get(&owner).into_iter().flatten() {
                if fund.0 == key.0 && fund.1 == key.1 {
                    close_funding
                        .entry(key.clone())
                        .or_default()
                        .insert(fund.clone());
                }
            }
        }
    }
    let mut pairs = Vec::new();
    let mut opened = BTreeSet::new();
    for row in rows {
        let id = pystr(&row["id"]);
        let Some(key) = tx_scope(row) else {
            continue;
        };
        let value = amount(row, "amount")?;
        if value <= zero() {
            continue;
        }
        if row["direction"] == "outbound" {
            if let Some(wallet) = funding.get(&key) {
                if funding_mismatch(
                    row,
                    amounts.get(&key).unwrap_or(&zero()),
                    strict_funding.contains(&key),
                )? {
                    continue;
                }
                opened.insert(key);
                let incoming = clone_leg(
                    row,
                    &wallets[wallet],
                    format!("channel-open:{id}:in:{wallet}"),
                    "inbound",
                    &zero(),
                )?;
                pairs.push(
                    json!({"out":row,"in":incoming,"source":"channel_lifecycle","kind":OPEN}),
                );
            }
        }
    }
    let eligible: BTreeSet<_> = closing
        .keys()
        .filter(|k| {
            !ambiguous_closing.contains(*k)
                && close_funding
                    .get(*k)
                    .is_some_and(|f| !f.is_disjoint(&opened))
        })
        .cloned()
        .collect();
    if eligible.is_empty() {
        return Ok(pairs);
    }
    for (key, group) in close_groups(&eligible, &balances, rows, &sweeps)? {
        if group.mismatch {
            continue;
        }
        let wallet = &closing[&key];
        let fee = if group.balance_msat > group.total_msat {
            &group.balance_msat - &group.total_msat
        } else {
            zero()
        };
        let mut keeper: Option<(BigInt, String)> = None;
        if fee > zero() {
            for leg in &group.legs {
                let key = (amount(leg, "amount")?, pystr(&leg["id"]));
                if keeper.as_ref().is_none_or(|k| key > *k) {
                    keeper = Some(key);
                }
            }
        }
        for row in &group.legs {
            if amount(row, "amount")? <= zero() {
                continue;
            }
            let id = pystr(&row["id"]);
            let leg_fee = if keeper.as_ref().is_some_and(|k| k.1 == id) {
                fee.clone()
            } else {
                zero()
            };
            let out = clone_leg(
                row,
                &wallets[wallet],
                format!("channel-close:{id}:out:{wallet}"),
                "outbound",
                &leg_fee,
            )?;
            pairs.push(json!({"out":out,"in":row,"source":"channel_lifecycle","kind":CLOSE}));
        }
    }
    Ok(pairs)
}
fn roles(channels: &[Value], rows: &[Value]) -> Result<BTreeMap<String, String>, String> {
    let (mut funding, mut closing, mut atomic_funding, mut atomic_closing) = (
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
    );
    let (mut amounts, mut balances, mut sweeps) = (
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::<Scope, Sweeps>::new(),
    );
    for row in channels {
        if let Some(key) = funding_scope(row) {
            funding.insert(key.clone());
            if atomic(row) {
                atomic_funding.insert(key.clone());
            }
            add_positive(&mut amounts, &key, amount(row, "funding_amount_msat")?);
        }
        if let Some(key) = closing_scope(row) {
            closing.insert(key.clone());
            sweeps
                .entry(key.clone())
                .or_default()
                .extend(trusted_sweeps(row)?);
            if atomic(row) {
                atomic_closing.insert(key.clone());
            }
            add_positive(&mut balances, &key, amount(row, "close_balance_msat")?);
        }
    }
    let atomic_rows: Vec<_> = channels.iter().filter(|r| atomic(r)).cloned().collect();
    let wallets: Wallets = atomic_rows.iter().filter(|r|codec::truth(&r["wallet_id"])).map(|r| {
        let id = pystr(&r["wallet_id"]); (id.clone(),json!({"id":id,"label":id,"wallet_account_id":null,"account_code":null,"account_label":null}))
    }).collect();
    let mut atomic_ids = BTreeSet::new();
    if !atomic_rows.is_empty() {
        for pair in transfer_pairs(&atomic_rows, rows, &wallets)? {
            let row = if pair["kind"] == OPEN {
                &pair["out"]
            } else {
                &pair["in"]
            };
            atomic_ids.insert(pystr(&row["id"]));
        }
    }
    let mut roles = BTreeMap::new();
    if !closing.is_empty() {
        for (key, group) in close_groups(&closing, &balances, rows, &sweeps)? {
            for row in group.legs {
                let id = pystr(&row["id"]);
                let mismatch =
                    group.mismatch || (atomic_closing.contains(&key) && !atomic_ids.contains(&id));
                roles.insert(id, if mismatch { CLOSE_MISMATCH } else { CLOSE }.into());
            }
        }
    }
    for row in rows {
        let Some(key) = tx_scope(row) else {
            continue;
        };
        let id = pystr(&row["id"]);
        if row["direction"] == "outbound" && funding.contains(&key) {
            let mismatch = funding_mismatch(
                row,
                amounts.get(&key).unwrap_or(&zero()),
                atomic_funding.contains(&key),
            )? || (atomic_funding.contains(&key) && !atomic_ids.contains(&id));
            roles.insert(id, if mismatch { OPEN_MISMATCH } else { OPEN }.into());
        }
    }
    Ok(roles)
}
fn context(records: &[Value], rows: &[Value], wallets: &Wallets) -> Result<Value, String> {
    if records.is_empty() {
        return Ok(json!({"roles":{},"pairs":[]}));
    }
    let mut channels = Vec::new();
    for record in records {
        // These indexed fields are storage columns: missing columns are KeyError.
        let get = |key: &str| {
            record
                .get(key)
                .cloned()
                .ok_or_else(|| "KeyError".to_owned())
        };
        if !codec::truth(&get("txid")?) {
            continue;
        }
        let mut row = json!({});
        for key in [
            "wallet_id",
            "channel_id",
            "config_json",
            "raw_json",
            "chain",
            "network",
        ] {
            row[key] = get(key)?;
        }
        if get("tag")? == "channel_close" {
            row["closing_txid"] = get("txid")?;
            row["close_balance_msat"] = get("amount_msat")?;
        } else {
            row["funding_txid"] = get("txid")?;
            row["funding_outpoint"] = get("outpoint")?;
            row["funding_amount_msat"] = get("amount_msat")?;
        }
        channels.push(row);
    }
    // Role admission precedes the second pair build, exactly as Python.
    let roles = roles(&channels, rows)?;
    let pairs = transfer_pairs(&channels, rows, wallets)?;
    Ok(json!({"roles":roles,"pairs":pairs}))
}
pub(crate) fn compute(snapshot: &Snapshot) -> Result<Value, String> {
    // An empty channel population reads no row; see `context`.
    if snapshot.channel_records.is_empty() {
        return Ok(json!({"roles":{},"pairs":[]}));
    }
    context(
        &snapshot
            .channel_records
            .iter()
            .map(|r| json!(r))
            .collect::<Vec<_>>(),
        snapshot.rows(),
        &snapshot
            .wallets
            .iter()
            .map(|w| (pystr(&json!(w)["id"]), json!(w)))
            .collect(),
    )
}
pub(super) fn admit_typed(snapshot: &crate::snapshot::BookSnapshot) -> Result<(), String> {
    if snapshot.channel_records.is_empty() {
        return Ok(());
    }
    context(
        &snapshot
            .channel_records
            .iter()
            .map(|r| json!(r))
            .collect::<Vec<_>>(),
        &snapshot
            .transactions
            .iter()
            .map(|r| r.json())
            .collect::<Vec<_>>(),
        &snapshot
            .wallets
            .iter()
            .map(|w| {
                let row = w.json();
                (pystr(&row["id"]), row)
            })
            .collect(),
    )
    .map(|_| ())
}
pub(super) fn compile(snapshot: &Snapshot) -> Value {
    compute(snapshot).unwrap_or_else(|error| json!({"error":error}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn operands(value: &Value) -> (Vec<Value>, Vec<Value>, Wallets) {
        (
            super::super::array(&value["channels"]).to_vec(),
            super::super::array(&value["rows"]).to_vec(),
            serde_json::from_value(value["wallets"].clone()).unwrap(),
        )
    }
    #[test]
    fn channel_lifecycle_helper_vectors() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_channel_helpers.json"
        ))
        .unwrap();
        for vector in vectors {
            let v = &vector["value"];
            let result = (|| -> Result<Value, String> {
                Ok(match vector["op"].as_str().unwrap() {
                    "outpoint" => json!(outpoint(&codec::from_json(v).unwrap())
                        .map(|(t, n)| json!([t, number(&n)]))),
                    "txid" => json!(txid_from_outpoint(v)),
                    "tx_scope" => json!(tx_scope(v)),
                    "funding_scope" => json!(funding_scope(v)),
                    "closing_scope" => json!(closing_scope(v)),
                    "vin" => json!(vin(v)
                        .iter()
                        .map(|(t, n)| json!([t, number(n)]))
                        .collect::<Vec<_>>()),
                    "sweeps" => json!(trusted_sweeps(v)?
                        .iter()
                        .map(|(t, (p, n))| json!([t, [p, number(n)]]))
                        .collect::<Vec<_>>()),
                    "funding_mismatch" => json!(funding_mismatch(
                        &v["row"],
                        &BigInt::from_str(&v["funded"].to_string()).unwrap(),
                        v["strict"] == true
                    )?),
                    "close_mismatch" => json!(close_mismatch(
                        &BigInt::from_str(&v["received"].to_string()).unwrap(),
                        &BigInt::from_str(&v["balance"].to_string()).unwrap()
                    )?),
                    "roles" => {
                        let (channels, rows, _) = operands(v);
                        json!(roles(&channels, &rows)?)
                    }
                    "pairs" => {
                        let (channels, rows, wallets) = operands(v);
                        json!(transfer_pairs(&channels, &rows, &wallets)?)
                    }
                    "context" => {
                        let (_, rows, wallets) = operands(v);
                        context(super::super::array(&v["records"]), &rows, &wallets)?
                    }
                    _ => unreachable!(),
                })
            })();
            if vector.get("error").is_some() {
                assert_eq!(
                    result.err().map(|e| json!(e)),
                    Some(vector["error"].clone()),
                    "{}",
                    vector["case"]
                );
            } else {
                assert_eq!(result.unwrap(), vector["expected"], "{}", vector["case"]);
            }
        }
    }
}

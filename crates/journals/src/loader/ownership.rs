use super::*;
use kassiber_pyjson::BigInt;
use kassiber_wallets::{config::bitcoin_address_script, descriptors};
use std::collections::BTreeMap;

#[derive(Default)]
struct Groups {
    entries: Vec<(String, Vec<Value>)>,
    positions: HashMap<String, usize>,
}
impl Groups {
    fn add(&mut self, key: String, value: Value) {
        let next = self.entries.len();
        let index = *self.positions.entry(key.clone()).or_insert_with(|| {
            self.entries.push((key, vec![]));
            next
        });
        self.entries[index].1.push(value);
    }
    fn matches(&self, key: &str) -> &[Value] {
        self.positions
            .get(key)
            .map(|index| self.entries[*index].1.as_slice())
            .unwrap_or(&[])
    }
    fn encode(&self, references: &BTreeSet<String>) -> Value {
        Value::Dict(
            self.entries
                .iter()
                .filter(|(key, _)| references.contains(&key.to_lowercase()))
                .map(|(key, values)| (Key::from(key.as_str()), list(values.clone())))
                .collect(),
        )
    }
}
#[derive(Default)]
struct Index {
    scripts: Groups,
    addresses: Groups,
    outpoints: Groups,
    scoped: BTreeMap<(String, String, String), Vec<Value>>,
    txids: Groups,
    scoped_txids: BTreeMap<(String, String, String), Vec<Value>>,
}
fn address_key(value: &Value) -> String {
    let value = if truth(value) {
        text(value)
    } else {
        String::new()
    };
    let value = kassiber_pytext::strip(&value);
    let lower = value.to_lowercase();
    if [
        "bc1", "tb1", "bcrt1", "lq1", "tlq1", "ex1", "tex1", "el1", "ert1",
    ]
    .iter()
    .any(|p| lower.starts_with(p))
    {
        lower
    } else {
        value.into()
    }
}
pub(super) fn integer(value: &Value) -> Option<BigInt> {
    match value {
        Value::Int(value) => Some(value.clone()),
        Value::Bool(value) => Some(BigInt::from(u8::from(*value))),
        Value::Float(value) if value.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", value.trunc())).ok()
        }
        Value::Str(value) => value
            .as_str()
            .and_then(|value| kassiber_pytext::parse_int(value).ok()),
        Value::Bytes(bytes) if bytes.is_ascii() => std::str::from_utf8(bytes)
            .ok()
            .and_then(|value| kassiber_pytext::parse_int(value).ok()),
        _ => None,
    }
}
/// Python callers catching TypeError/ValueError still propagate int(inf)'s
/// OverflowError. Do not treat that refusal as missing or stale evidence.
pub(super) fn legacy_integer(value: &Value) -> Result<Option<BigInt>> {
    if matches!(value, Value::Float(value) if value.is_infinite()) {
        outpoint_integer(value)?;
    }
    Ok(integer(value))
}
pub(super) fn outpoint_integer(value: &Value) -> Result<BigInt> {
    if let Value::Bytes(bytes) = value {
        let invalid = || {
            Error::Reference(crate::Refusal::new(
                "ValueError",
                format!(
                    "invalid literal for int() with base 10: {}",
                    kassiber_pyjson::bytes_repr(bytes)
                        .chars()
                        .take(200)
                        .collect::<String>()
                ),
            ))
        };
        if !bytes.is_ascii() {
            return Err(invalid());
        }
        return match outpoint_integer(&Value::from(std::str::from_utf8(bytes).unwrap())) {
            Err(Error::Reference(error))
                if error.message.starts_with("invalid literal for int()") =>
            {
                Err(invalid())
            }
            result => result,
        };
    }
    if let Value::Str(value) = value {
        let raw = value
            .as_str()
            .ok_or(Error::Unsupported("surrogate outpoint index"))?;
        return kassiber_pytext::parse_int(raw).map_err(|error| {
            let error = if error == kassiber_pytext::NumberError::Invalid && raw.len() > 4300 {
                let body = raw.trim_start_matches(|c: char| {
                    if c.is_ascii() {
                        matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
                    } else {
                        kassiber_pytext::is_space(c as u32)
                    }
                });
                let body = body.strip_prefix(['+', '-']).unwrap_or(body);
                let end = body.char_indices().find(|(_, c)| *c != '_' && kassiber_pytext::decimal_value(*c as u32).is_none()).map(|(index, _)| index).unwrap_or(body.len());
                match kassiber_pytext::parse_int(&body[..end]) {
                    Err(error @ kassiber_pytext::NumberError::TooManyDigits(_)) => error,
                    _ => error,
                }
            } else {
                error
            };
            Error::Reference(crate::Refusal::new("ValueError", match error {
                kassiber_pytext::NumberError::Invalid => format!("invalid literal for int() with base 10: {}", kassiber_pytext::repr_py(value).chars().take(200).collect::<String>()),
                kassiber_pytext::NumberError::TooManyDigits(count) => format!("Exceeds the limit (4300 digits) for integer string conversion: value has {count} digits; use sys.set_int_max_str_digits() to increase the limit"),
            }))
        });
    }
    if let Value::Float(value) = value {
        if !value.is_finite() {
            return Err(Error::Reference(if value.is_nan() {
                crate::Refusal::new("ValueError", "cannot convert float NaN to integer")
            } else {
                crate::Refusal::new("OverflowError", "cannot convert float infinity to integer")
            }));
        }
    }
    integer(value).ok_or_else(|| {
        Error::Reference(crate::Refusal::new(
            "TypeError",
            format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                kassiber_envelope::type_name(value)
            ),
        ))
    })
}
pub(super) fn normalized_script(value: &Value) -> Option<String> {
    let raw = if truth(value) {
        text(value)
    } else {
        String::new()
    };
    let value = kassiber_pytext::strip(&raw).to_lowercase();
    // Python's callers use `normalized_script_hex(...) or ...`: an empty
    // result must allow raw-script/address fallback, never create a source.
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if b" \t\n\r\x0b\x0c".contains(&byte) {
            continue;
        }
        if !byte.is_ascii_hexdigit() || !bytes.next().is_some_and(|b| b.is_ascii_hexdigit()) {
            return None;
        }
    }
    Some(value)
}
pub(super) fn config(wallet: &Value) -> Result<Value> {
    let raw = get(wallet, "config_json");
    let parsed = match raw {
        Value::Str(value) => value
            .as_str()
            .and_then(|value| kassiber_pytext::loads(value).ok())
            .unwrap_or_else(|| dict([])),
        Value::Bytes(_) => {
            if let Value::Bytes(bytes) = raw {
                kassiber_pyjson::json_bytes_string(bytes)
                    .ok()
                    .and_then(|text| kassiber_pytext::loads_py(&text).ok())
                    .unwrap_or_else(|| dict([]))
            } else {
                unreachable!()
            }
        }
        _ => dict([]),
    };
    match parsed {
        Value::Dict(_) => Ok(parsed),
        _ => Err(Error::Reference(crate::Refusal::new(
            "AttributeError",
            format!(
                "'{}' object has no attribute 'get'",
                kassiber_envelope::type_name(&parsed)
            ),
        ))),
    }
}
pub(super) fn addresses(config: &Value) -> Result<Vec<String>> {
    let value = get(config, "addresses");
    let values = match value {
        Value::Null => vec![],
        Value::Str(value) => value
            .as_str()
            .ok_or(Error::Unsupported("surrogate address list"))?
            .split(',')
            .map(Value::from)
            .collect(),
        Value::List(values) => values.clone(),
        Value::Dict(values) => values
            .iter()
            .map(|(key, _)| match key {
                Key::Str(key) => Value::from(key.clone()),
                _ => Value::Null,
            })
            .collect(),
        _ => {
            return Err(Error::Reference(crate::Refusal::new(
                "TypeError",
                format!(
                    "'{}' object is not iterable",
                    kassiber_envelope::type_name(value)
                ),
            )))
        }
    };
    let mut seen = BTreeSet::new();
    Ok(values
        .into_iter()
        .filter_map(|value| {
            let value = text(&value);
            let value = kassiber_pytext::strip(&value);
            if value.is_empty() {
                return None;
            }
            let key = bitcoin_address_script(value)
                .map(|s| format!("script:{s}"))
                .unwrap_or_else(|| format!("text:{value}"));
            seen.insert(key).then(|| value.to_owned())
        })
        .collect())
}
pub(super) fn chain_network(chain: &Value, network: &Value) -> (String, String) {
    let chain = if truth(chain) {
        text(chain)
    } else {
        "bitcoin".into()
    };
    let network = if truth(network) {
        text(network)
    } else {
        String::new()
    };
    descriptors::normalize_chain(&chain)
        .and_then(|c| descriptors::normalize_network(&c, &network).map(|n| (c, n)))
        .unwrap_or_else(|_| {
            (
                kassiber_pytext::strip(&chain).to_lowercase(),
                kassiber_pytext::strip(&network).to_lowercase(),
            )
        })
}
pub(super) fn has_descriptor(config: &Value) -> bool {
    let present = |key| {
        get(config, key) != &Value::Null
            && !kassiber_pytext::strip(&text(get(config, key))).is_empty()
    };
    present("descriptor") || (present("xpub") && truth(get(config, "script_types")))
}
/// `item` with its branch label, when the caller asked for branch labels.
fn with_branch(branches: bool, item: &Value, label: String) -> Value {
    let mut item = item.clone();
    if branches {
        set(&mut item, "branch_label", Value::from(label));
    }
    item
}
fn match_facts(wallet: &Value, account: &str, chain: &str, network: &str, source: &str) -> Value {
    dict([
        ("wallet_id", Value::from(text(get(wallet, "id")))),
        ("wallet_label", Value::from(text(get(wallet, "label")))),
        ("account", Value::from(account)),
        ("chain", Value::from(chain)),
        ("network", Value::from(network)),
        ("source", Value::from(source)),
        (
            "wallet_kind",
            Value::from(if truth(get(wallet, "kind")) {
                text(get(wallet, "kind"))
            } else {
                String::new()
            }),
        ),
    ])
}
fn wallet_pair(value: &Value) -> (String, String) {
    let Value::List(values) = value else {
        unreachable!("wallet pair")
    };
    (text(&values[0]), text(&values[1]))
}
impl Index {
    fn address(&mut self, address: &Value, item: &Value) {
        let key = address_key(address);
        if !key.is_empty() {
            self.addresses.add(key, item.clone());
        }
    }
    fn script(&mut self, script: Option<String>, item: &Value) {
        if let Some(script) = script.filter(|s| !s.is_empty()) {
            self.scripts.add(script.to_lowercase(), item.clone());
        }
    }
    fn outpoint(&mut self, txid: &Value, vout: &Value, item: &Value) -> Result<()> {
        if !truth(txid) || vout == &Value::Null {
            return Ok(());
        }
        let vout = outpoint_integer(vout)?;
        let key = format!("{}:{vout}", text(txid).to_lowercase());
        self.outpoints.add(key.clone(), item.clone());
        let (chain, network) = chain_network(get(item, "chain"), get(item, "network"));
        self.scoped
            .entry((chain, network, key))
            .or_default()
            .push(item.clone());
        Ok(())
    }
    fn txid(&mut self, txid: &Value, item: &Value) {
        if !truth(txid) {
            return;
        }
        let key = text(txid).to_lowercase();
        let wallet = list(vec![
            get(item, "wallet_id").clone(),
            get(item, "wallet_label").clone(),
        ]);
        if !self.txids.matches(&key).contains(&wallet) {
            self.txids.add(key.clone(), wallet.clone());
        }
        let (chain, network) = chain_network(get(item, "chain"), get(item, "network"));
        let wallets = self.scoped_txids.entry((chain, network, key)).or_default();
        if !wallets.contains(&wallet) {
            wallets.push(wallet);
        }
    }
    fn encode(self, references: &BTreeSet<String>) -> Value {
        let scoped = |values: BTreeMap<(String, String, String), Vec<Value>>, sort: bool| {
            list(
                values
                    .into_iter()
                    .filter(|(key, _)| references.contains(&key.2.to_lowercase()))
                    .map(|(key, mut matches)| {
                        if sort {
                            matches.sort_by_key(wallet_pair);
                        }
                        dict([
                            (
                                "key",
                                list(vec![
                                    Value::from(key.0),
                                    Value::from(key.1),
                                    Value::from(key.2),
                                ]),
                            ),
                            ("matches", list(matches)),
                        ])
                    })
                    .collect(),
            )
        };
        let mut txids = self.txids;
        for (_, values) in &mut txids.entries {
            values.sort_by_key(wallet_pair);
        }
        dict([
            ("by_script", self.scripts.encode(references)),
            ("by_outpoint", self.outpoints.encode(references)),
            ("by_scoped_outpoint", scoped(self.scoped, false)),
            ("txid_wallets_by_scope", scoped(self.scoped_txids, true)),
            ("txid_wallets", txids.encode(references)),
        ])
    }
}

pub(super) fn load(
    conn: &Connection,
    profile_id: &str,
    rows: &[kassiber_custody::snapshot::Record],
) -> Result<(Value, Vec<Value>)> {
    if !rows.iter().any(|row| {
        row.get("observation_authority_version")
            .is_some_and(|v| !v.is_null())
    }) {
        return Ok((Value::Null, vec![]));
    }
    let (index, warnings) = build(conn, profile_id, false)?;
    let mut references = BTreeSet::new();
    for row in rows {
        let native = serde_json::json!({"raw_json":row.get("raw_json")});
        if let Some(graph) =
            kassiber_custody::journal_support::parsed(&native).map_err(Error::Invalid)?
        {
            for entry in graph["inputs"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(graph["outputs"].as_array().into_iter().flatten())
            {
                if let Some(script) = entry["script"].as_str().filter(|s| !s.is_empty()) {
                    references.insert(script.to_lowercase());
                }
                if let Some(outpoint) = entry["outpoint"].as_str().filter(|s| !s.is_empty()) {
                    references.insert(outpoint.to_lowercase());
                    references.insert(outpoint.split(':').next().unwrap_or("").to_lowercase());
                }
            }
            if let Some(txid) = graph["txid"].as_str().filter(|s| !s.is_empty()) {
                references.insert(txid.to_lowercase());
            }
        }
    }
    Ok((index.encode(&references), warnings))
}

/// `build_owned_index(conn, profile_id, load_profile_wallets(...))` for a
/// caller that knows which scripts, outpoints and txids it will look up; the
/// encoded index keeps only those keys. Matches also carry their wallet
/// branch label, which the journal snapshot omits.
pub fn owned_index(
    conn: &Connection,
    profile_id: &str,
    references: &BTreeSet<String>,
) -> Result<(Value, Vec<Value>)> {
    let (index, warnings) = build(conn, profile_id, true)?;
    Ok((index.encode(references), warnings))
}

/// [`owned_index`] plus every wallet id the whole index names by script,
/// address or outpoint (`transaction_verify._observed_wallet_ids`).
pub fn owned_index_and_wallets(
    conn: &Connection,
    profile_id: &str,
    references: &BTreeSet<String>,
) -> Result<(Value, Vec<Value>, BTreeSet<String>)> {
    let (index, warnings) = build(conn, profile_id, true)?;
    let wallets = [&index.scripts, &index.addresses, &index.outpoints]
        .into_iter()
        .flat_map(|groups| &groups.entries)
        .flat_map(|(_, matches)| matches)
        .map(|item| text(get(item, "wallet_id")))
        .collect();
    Ok((index.encode(references), warnings, wallets))
}

fn build(conn: &Connection, profile_id: &str, branches: bool) -> Result<(Index, Vec<Value>)> {
    let wallets=query(conn,"SELECT w.id,w.label,w.kind,w.config_json,a.code AS account_code,a.label AS account_label FROM wallets w LEFT JOIN accounts a ON a.id=w.account_id WHERE w.profile_id=? ORDER BY w.label ASC",[profile_id])?;
    let summary_rows = query(
        conn,
        "SELECT id,label,kind,config_json FROM wallets WHERE profile_id=?",
        [profile_id],
    )?;
    let summaries = summary_rows
        .iter()
        .map(|wallet| Ok((text(get(wallet, "id")), (wallet, config(wallet)?))))
        .collect::<Result<HashMap<_, _>>>()?;
    let mut index = Index::default();
    let mut warnings = vec![];
    let mut highest = HashMap::<String, BTreeMap<String, BigInt>>::new();
    // One batch per canonical descriptor and horizon, shared by current and
    // retired policies and wallets. Keep wallet-specific facts out of the memo.
    let mut derived =
        HashMap::<(String, u32), std::result::Result<Vec<serde_json::Value>, String>>::new();
    for row in query(conn,"SELECT wallet_id,txid,vout,address,script_pubkey,branch_label,branch_index,address_index,chain,network FROM wallet_utxos WHERE profile_id=?",[profile_id])? {
        let wallet_id=text(get(&row,"wallet_id"));
        if !wallets.is_empty() && !summaries.contains_key(&wallet_id) { continue; }
        let (mut wallet,config)=summaries.get(&wallet_id).map(|(wallet,config)|((*wallet).clone(),config.clone())).unwrap_or((dict([("id",Value::from(wallet_id.clone())),("label",Value::from(wallet_id.clone()))]),dict([])));
        if matches!(get(&wallet,"label"), Value::Str(label) if label.is("")) {
            set(&mut wallet,"label",Value::from(wallet_id.clone()));
        }
        let chain=if truth(get(&row,"chain")) { get(&row,"chain") } else { get(&config,"chain") };
        let network=if truth(get(&row,"network")) { get(&row,"network") } else { get(&config,"network") };
        let (chain,network)=chain_network(chain,network);
        let item=with_branch(branches,&match_facts(&wallet,"",&chain,&network,"inventory"),if truth(get(&row,"branch_label")) { text(get(&row,"branch_label")) } else { String::new() });
        index.address(get(&row,"address"),&item);
        let script=if truth(get(&row,"address")) { bitcoin_address_script(&text(get(&row,"address"))) } else { None };
        index.script(script.or_else(||normalized_script(get(&row,"script_pubkey"))),&item);
        index.outpoint(get(&row,"txid"),get(&row,"vout"),&item)?;
        index.txid(get(&row,"txid"),&item);
        let branch=if truth(get(&row,"branch_label")) { text(get(&row,"branch_label")) } else { String::new() };
        if let Value::Int(value)=get(&row,"address_index") { if !branch.is_empty() && value>=&BigInt::from(0) {
            highest.entry(wallet_id).or_default().entry(branch).and_modify(|n|*n=n.clone().max(value.clone())).or_insert(value.clone());
        } }
    }
    for wallet in &wallets {
        let wallet_id = text(get(wallet, "id"));
        let config = config(wallet)?;
        let account = ["account_code", "account_label"]
            .into_iter()
            .map(|key| get(wallet, key))
            .find(|value| truth(value))
            .map(text)
            .unwrap_or_default();
        let configured = if truth(get(&config, "ownership_scan_to_index")) {
            legacy_integer(get(&config, "ownership_scan_to_index"))?
        } else {
            Some(0.into())
        };
        if configured.is_none() {
            warnings.push(Value::from(format!(
                "Wallet '{}': ownership scan depth is invalid",
                text(get(wallet, "label"))
            )));
        }
        let scan = configured
            .unwrap_or(0.into())
            .max(500.into())
            .min(20_000.into());
        let mut configs = vec![(config, false)];
        for row in query(conn,"SELECT private_material_json FROM wallet_policy_epochs WHERE wallet_id=? AND status='retired' ORDER BY created_at ASC,id ASC",[&wallet_id])? {
            let historic=parsed(get(&row,"private_material_json"));
            if matches!(&historic,Value::Dict(values) if !values.is_empty()) { configs.push((historic,true)); }
        }
        for (config, history) in configs {
            let (chain, network) = chain_network(get(&config, "chain"), get(&config, "network"));
            let item = match_facts(
                wallet,
                &account,
                &chain,
                &network,
                if history {
                    "address_history"
                } else {
                    "address_list"
                },
            );
            let item = with_branch(branches, &item, "address".into());
            for address in addresses(&config)? {
                index.address(&Value::from(address.clone()), &item);
                index.script(bitcoin_address_script(&address), &item);
            }
            if !has_descriptor(&config) {
                continue;
            }
            let floor = if truth(get(&config, "scan_to_index")) {
                legacy_integer(get(&config, "scan_to_index"))?
            } else {
                Some(0.into())
            };
            let Some(floor) = floor else {
                warnings.push(Value::from(format!(
                    "Wallet '{}': historic scan floor is invalid",
                    text(get(wallet, "label"))
                )));
                continue;
            };
            let plan = descriptors::load(&kassiber_custody::codec::to_json(&config));
            let plan = match plan {
                Ok(Some(plan)) => plan,
                Ok(None) => continue,
                Err(error) => {
                    let code = error.into_app().code;
                    warnings.push(Value::from(format!(
                        "Wallet '{}': descriptor not scanned ({code})",
                        text(get(wallet, "label"))
                    )));
                    continue;
                }
            };
            let used = highest
                .get(&wallet_id)
                .and_then(|values| values.values().max())
                .cloned()
                .unwrap_or((-1).into());
            let ceiling = (scan.clone().max(floor) + 1_u32)
                .max(used + plan.gap_limit + 1_u32)
                .clamp(1.into(), 20_001.into());
            let ceiling =
                u32::try_from(ceiling).map_err(|_| Error::Invalid("derivation ceiling".into()))?;
            let targets = match derived
                .entry((plan.fingerprint.clone(), ceiling))
                .or_insert_with(|| plan.derive(None, 0, ceiling).map_err(|e| e.into_app().code))
            {
                Ok(targets) => targets,
                Err(code) => {
                    warnings.push(Value::from(format!(
                        "Wallet '{}': descriptor not scanned ({code})",
                        text(get(wallet, "label"))
                    )));
                    continue;
                }
            };
            let item = match_facts(
                wallet,
                &account,
                &plan.chain,
                &plan.network,
                if history {
                    "derived_history"
                } else {
                    "derived"
                },
            );
            for target in targets {
                let item = with_branch(
                    branches,
                    &item,
                    target["branch_label"].as_str().unwrap_or("").to_owned(),
                );
                index.script(target["script_pubkey"].as_str().map(str::to_owned), &item);
                index.address(
                    &Value::from(target["address"].as_str().unwrap_or("")),
                    &item,
                );
                if let Some(address) = target["unconfidential_address"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                {
                    index.address(&Value::from(address), &item);
                }
            }
        }
    }
    for row in query(conn,"SELECT external_id,wallet_id,raw_json FROM transactions WHERE profile_id=? AND external_id IS NOT NULL AND raw_json IS NOT NULL",[profile_id])? {
        let txid=text(get(&row,"external_id")); let txid=kassiber_pytext::strip(&txid).to_lowercase();
        if txid.len()!=64 || !txid.bytes().all(|b|b.is_ascii_hexdigit()) { continue; }
        let Value::List(observations)=parsed(get(&row,"raw_json")) else { continue; };
        for observation in observations {
            if !matches!(observation,Value::Dict(_)) { continue; }
            let observed=if truth(get(&observation,"txid")) { text(get(&observation,"txid")) } else { String::new() };
            if kassiber_pytext::strip(&observed).to_lowercase()!=txid { continue; }
            let address=if truth(get(&observation,"address")) { text(get(&observation,"address")) } else { String::new() };
            if kassiber_pytext::strip(&address).is_empty() || matches!(get(&observation,"vout"),Value::Bool(_)) { continue; }
            let Some(vout)=integer(get(&observation,"vout")).filter(|v|*v>=0.into()) else { continue; };
            let matches=index.addresses.matches(&address_key(&Value::from(kassiber_pytext::strip(&address)))).to_vec();
            let owners=matches.iter().map(|item|text(get(item,"wallet_id"))).collect::<BTreeSet<_>>();
            if owners != BTreeSet::from([text(get(&row,"wallet_id"))]) { continue; }
            for item in &matches { index.outpoint(&Value::from(txid.clone()),&Value::from(vout.clone()),item)?; }
        }
    }
    Ok((index, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_loader_scoped_index_preserves_evidence_order_and_sorts_wallet_pairs() {
        let a = match_facts(
            &dict([("id", Value::from("a")), ("label", Value::from("A"))]),
            "",
            "bitcoin",
            "main",
            "inventory",
        );
        let b = match_facts(
            &dict([("id", Value::from("b")), ("label", Value::from("B"))]),
            "",
            "bitcoin",
            "main",
            "inventory",
        );
        let mut index = Index::default();
        for item in [&b, &a] {
            index
                .outpoint(&Value::from("tx"), &Value::from(0), item)
                .unwrap();
            index.txid(&Value::from("tx"), item);
        }
        let encoded = index.encode(&BTreeSet::from(["tx".into(), "tx:0".into()]));
        let Value::List(outpoints) = get(&encoded, "by_scoped_outpoint") else {
            panic!("outpoints")
        };
        assert_eq!(get(&outpoints[0], "matches"), &list(vec![b, a]));
        let Value::List(txids) = get(&encoded, "txid_wallets_by_scope") else {
            panic!("txids")
        };
        assert_eq!(
            get(&txids[0], "matches"),
            &list(vec![
                list(vec![Value::from("a"), Value::from("A")]),
                list(vec![Value::from("b"), Value::from("B")])
            ])
        );
    }

    #[test]
    fn native_loader_ownership_helpers_keep_scope_and_evidence_separate() {
        assert_eq!(address_key(&Value::from(" BC1QABC ")), "bc1qabc");
        assert_eq!(address_key(&Value::from(" Base58ABC ")), "Base58ABC");
        assert_eq!(address_key(&Value::Null), "");
        for (input, expected) in [
            (" 00AA ", Some("00aa")),
            ("00  AA", Some("00  aa")),
            ("", None),
            ("a", None),
            ("ag", None),
            ("g0", None),
            ("a ", None),
        ] {
            assert_eq!(normalized_script(&Value::from(input)).as_deref(), expected);
        }
        assert_eq!(normalized_script(&Value::Null), None);
        for (input, expected) in [
            (Value::from(-2), Some((-2).into())),
            (Value::Bool(false), Some(0.into())),
            (Value::from("١٢"), Some(12.into())),
            (Value::Float(-1.9), Some((-1).into())),
            (Value::Null, None),
            (Value::from("no"), None),
        ] {
            assert_eq!(integer(&input), expected);
        }
        assert_eq!(
            chain_network(&Value::Null, &Value::Null),
            ("bitcoin".into(), "main".into())
        );
        assert_eq!(
            chain_network(&Value::from(" Unknown "), &Value::from(" Custom ")),
            ("unknown".into(), "custom".into())
        );
        for (input, expected) in [
            ("{}", false),
            ("{\"descriptor\": \" \"}", false),
            ("{\"descriptor\": \"d\"}", true),
            ("{\"xpub\":\"x\"}", false),
            ("{\"xpub\":\"x\",\"script_types\":[\"wpkh\"]}", true),
        ] {
            assert_eq!(
                has_descriptor(&kassiber_pytext::loads(input).unwrap()),
                expected
            );
        }
        for value in [Value::Null, Value::from("malformed")] {
            assert_eq!(config(&dict([("config_json", value)])).unwrap(), dict([]));
        }
        for addresses_json in [
            "null",
            "\" a, ,b,a \"",
            "[\" a \",\"b\",\"a\",\"\"]",
            "{\"a\": 1, \"b\": 2}",
        ] {
            let result = addresses(&dict([(
                "addresses",
                kassiber_pytext::loads(addresses_json).unwrap(),
            )]))
            .unwrap();
            assert_eq!(
                result,
                if addresses_json == "null" {
                    vec![]
                } else {
                    vec!["a", "b"]
                }
            );
        }
        let item_a = match_facts(
            &dict([
                ("id", Value::from("a")),
                ("label", Value::from("A")),
                ("kind", Value::Null),
            ]),
            "treasury",
            "bitcoin",
            "mainnet",
            "inventory",
        );
        let item_b = match_facts(
            &dict([
                ("id", Value::from("b")),
                ("label", Value::from("B")),
                ("kind", Value::from("custom")),
            ]),
            "",
            "bitcoin",
            "testnet",
            "inventory",
        );
        assert_eq!(get(&item_a, "wallet_kind"), &Value::from(""));
        assert_eq!(get(&item_b, "wallet_kind"), &Value::from("custom"));
        let mut index = Index::default();
        index.address(&Value::Null, &item_a);
        index.address(&Value::from("BC1QABC"), &item_a);
        index.script(None, &item_a);
        index.script(Some(String::new()), &item_a);
        index.script(Some("AA".into()), &item_a);
        assert_eq!(index.addresses.entries.len(), 1);
        assert_eq!(index.scripts.entries.len(), 1);
        index
            .outpoint(&Value::Null, &Value::from(0), &item_a)
            .unwrap();
        index
            .outpoint(&Value::from("TX"), &Value::Null, &item_a)
            .unwrap();
        index
            .outpoint(&Value::from("TX"), &Value::from(0), &item_a)
            .unwrap();
        index.txid(&Value::Null, &item_a);
        for item in [&item_b, &item_a, &item_a] {
            index.txid(&Value::from("TX"), item);
        }
        assert_eq!(index.txids.entries.len(), 1);
        let encoded = index.encode(&BTreeSet::from(["aa".into(), "tx:0".into(), "tx".into()]));
        assert_eq!(
            get(&encoded, "by_script"),
            &dict([("aa", list(vec![item_a.clone()]))])
        );
        assert_eq!(
            get(&encoded, "by_outpoint"),
            &dict([("tx:0", list(vec![item_a]))])
        );
        assert_eq!(
            get(&encoded, "txid_wallets"),
            &dict([(
                "tx",
                list(vec![
                    list(vec![Value::from("a"), Value::from("A")]),
                    list(vec![Value::from("b"), Value::from("B")])
                ])
            )])
        );
        let Value::List(scoped) = get(&encoded, "txid_wallets_by_scope") else {
            panic!("scope")
        };
        assert_eq!(scoped.len(), 2);
        for item in scoped {
            let Value::List(matches) = get(item, "matches") else {
                panic!("matches")
            };
            assert_eq!(matches.len(), 1);
        }
        assert_eq!(
            get(&scoped[0], "key"),
            &list(vec![
                Value::from("bitcoin"),
                Value::from("main"),
                Value::from("tx")
            ])
        );
        let mut groups = Groups::default();
        groups.add("unreferenced".into(), Value::from(1));
        assert_eq!(groups.matches("missing"), &[]);
        assert_eq!(groups.encode(&BTreeSet::new()), dict([]));
    }

    #[test]
    fn native_loader_outpoint_integer_preserves_python_refusals() {
        for (value, expected) in [
            (Value::from("١_٢"), BigInt::from(12)),
            (Value::Float(1.9), BigInt::from(1)),
            (Value::Bool(true), BigInt::from(1)),
            (Value::from(i64::MIN), BigInt::from(i64::MIN)),
        ] {
            assert_eq!(outpoint_integer(&value).unwrap(), expected);
        }
        for (value, family, message) in [
            (Value::from(""), "ValueError", "invalid literal for int() with base 10: ''".into()),
            (Value::from("'quoted'"), "ValueError", "invalid literal for int() with base 10: \"'quoted'\"".into()),
            (Value::from("9".repeat(4301)), "ValueError", "Exceeds the limit (4300 digits) for integer string conversion: value has 4301 digits; use sys.set_int_max_str_digits() to increase the limit".into()),
            (Value::from(format!("{}x", "9".repeat(4301))), "ValueError", "Exceeds the limit (4300 digits) for integer string conversion: value has 4301 digits; use sys.set_int_max_str_digits() to increase the limit".into()),
            (Value::from(format!(" \t-{}x", "9".repeat(4301))), "ValueError", "Exceeds the limit (4300 digits) for integer string conversion: value has 4301 digits; use sys.set_int_max_str_digits() to increase the limit".into()),
            (Value::from(format!("\u{a0}+{}x", "9".repeat(4301))), "ValueError", "Exceeds the limit (4300 digits) for integer string conversion: value has 4301 digits; use sys.set_int_max_str_digits() to increase the limit".into()),
            (Value::from("x".repeat(5000)), "ValueError", format!("invalid literal for int() with base 10: '{}", "x".repeat(199))),
            (Value::Float(f64::NAN), "ValueError", "cannot convert float NaN to integer".into()),
            (Value::Float(f64::INFINITY), "OverflowError", "cannot convert float infinity to integer".into()),
            (Value::Float(f64::NEG_INFINITY), "OverflowError", "cannot convert float infinity to integer".into()),
            (Value::Null, "TypeError", "int() argument must be a string, a bytes-like object or a real number, not 'NoneType'".into()),
        ] {
            let Err(Error::Reference(refusal)) = outpoint_integer(&value) else {
                panic!("expected outpoint scalar refusal");
            };
            assert_eq!(refusal.family, family);
            assert_eq!(refusal.message, message);
            assert!(refusal.code.is_none());
        }
    }
}

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
        _ => None,
    }
}
pub(super) fn normalized_script(value: &Value) -> Option<String> {
    let raw = if truth(value) {
        text(value)
    } else {
        String::new()
    };
    let value = kassiber_pytext::strip(&raw).to_lowercase();
    if !value.len().is_multiple_of(2) {
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
    let parsed = if truth(raw) { parsed(raw) } else { dict([]) };
    match parsed {
        Value::Dict(_) => Ok(parsed),
        Value::Null if !matches!(raw,Value::Str(raw) if raw.as_str().is_some_and(|raw|raw.trim() == "null")) => {
            Ok(dict([]))
        }
        _ => Err(Error::Invalid("wallet config is not a mapping".into())),
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
        _ => return Err(Error::Invalid("address list is not iterable".into())),
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
        let vout = integer(vout).ok_or_else(|| Error::Invalid("outpoint index".into()))?;
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
    rows: &[Value],
) -> Result<(Value, Vec<Value>)> {
    if !rows
        .iter()
        .any(|row| get(row, "observation_authority_version") != &Value::Null)
    {
        return Ok((Value::Null, vec![]));
    }
    let wallets=query(conn,"SELECT w.id,w.label,w.kind,w.config_json,a.code AS account_code,a.label AS account_label FROM wallets w LEFT JOIN accounts a ON a.id=w.account_id WHERE w.profile_id=? ORDER BY w.label ASC",[profile_id])?;
    let summaries = wallets
        .iter()
        .map(|wallet| Ok((text(get(wallet, "id")), (wallet, config(wallet)?))))
        .collect::<Result<HashMap<_, _>>>()?;
    let mut index = Index::default();
    let mut warnings = vec![];
    let mut highest = HashMap::<String, BTreeMap<String, BigInt>>::new();
    for row in query(conn,"SELECT wallet_id,txid,vout,address,script_pubkey,branch_label,branch_index,address_index,chain,network FROM wallet_utxos WHERE profile_id=?",[profile_id])? {
        let wallet_id=text(get(&row,"wallet_id"));
        if !wallets.is_empty() && !summaries.contains_key(&wallet_id) { continue; }
        let (wallet,config)=summaries.get(&wallet_id).map(|(wallet,config)|((*wallet).clone(),config.clone())).unwrap_or((dict([("id",Value::from(wallet_id.clone())),("label",Value::from(wallet_id.clone()))]),dict([])));
        let chain=if truth(get(&row,"chain")) { get(&row,"chain") } else { get(&config,"chain") };
        let network=if truth(get(&row,"network")) { get(&row,"network") } else { get(&config,"network") };
        let (chain,network)=chain_network(chain,network);
        let item=match_facts(&wallet,"",&chain,&network,"inventory");
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
            integer(get(&config, "ownership_scan_to_index"))
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
            for address in addresses(&config)? {
                index.address(&Value::from(address.clone()), &item);
                index.script(bitcoin_address_script(&address), &item);
            }
            if !has_descriptor(&config) {
                continue;
            }
            let floor = if truth(get(&config, "scan_to_index")) {
                integer(get(&config, "scan_to_index"))
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
            let targets = match plan.derive(
                None,
                0,
                u32::try_from(ceiling).map_err(|_| Error::Invalid("derivation ceiling".into()))?,
            ) {
                Ok(targets) => targets,
                Err(error) => {
                    let code = error.into_app().code;
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
    let mut references = BTreeSet::new();
    for row in rows {
        let native =
            serde_json::json!({"raw_json":kassiber_custody::codec::to_json(get(row,"raw_json"))});
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

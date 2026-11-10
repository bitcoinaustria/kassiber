use super::ownership::{addresses, has_descriptor, legacy_integer, normalized_script};
use super::*;
use kassiber_wallets::{config::bitcoin_address_script, descriptors};

fn config(wallet: &Value) -> Result<Value> {
    let value = parsed(get(wallet, "config_json"));
    Ok(if matches!(value, Value::Dict(_)) {
        value
    } else {
        dict([])
    })
}

fn scope(chain: &Value, network: &Value) -> (String, String) {
    if chain == &Value::Null
        || chain == &Value::from("")
        || network == &Value::Null
        || network == &Value::from("")
    {
        return (String::new(), String::new());
    }
    ownership::chain_network(chain, network)
}
fn source(
    profile: &str,
    wallet: &Value,
    chain: &str,
    network: &str,
    script: &str,
    kind: &str,
    counts: &HashMap<String, Value>,
) -> Result<Value> {
    let id = text(get(wallet, "id"));
    let config = config(wallet)?;
    let native = kassiber_custody::codec::to_json(get(&config, "deprecated"));
    let deprecated = kassiber_wallets::config::boolean(Some(&native)).map_err(app_refusal)?;
    Ok(dict([
        ("profile_id", Value::from(profile)),
        ("wallet_id", Value::from(id.clone())),
        (
            "wallet_label",
            Value::from(if truth(get(wallet, "label")) {
                text(get(wallet, "label"))
            } else {
                id.clone()
            }),
        ),
        (
            "wallet_kind",
            Value::from(if truth(get(wallet, "kind")) {
                text(get(wallet, "kind"))
            } else {
                if kind == "address_list" {
                    "address"
                } else {
                    ""
                }
                .into()
            }),
        ),
        ("chain", Value::from(chain)),
        ("network", Value::from(network)),
        ("script_pubkey", Value::from(script)),
        ("source", Value::from(kind)),
        ("deprecated", Value::Bool(deprecated)),
        (
            "active_transaction_count",
            counts.get(&id).cloned().unwrap_or(Value::from(0)),
        ),
        ("branch_index", Value::Null),
        ("branch_label", Value::Null),
        ("address_index", Value::Null),
    ]))
}
pub(super) fn load<'a>(
    conn: &Connection,
    profile_id: &str,
    rows: impl IntoIterator<Item = &'a kassiber_custody::snapshot::Record>,
) -> Result<Vec<Value>> {
    let mut groups =
        std::collections::BTreeMap::<_, (BTreeSet<String>, Vec<serde_json::Value>)>::new();
    for row in rows {
        if !may_have_txid(row) {
            continue;
        }
        // Scope needs only identity/graph/network facts, not the wide SQL row.
        let native = serde_json::Value::Object(
            [
                "id",
                "raw_json",
                "config_json",
                "wallet_config_json",
                "external_id",
                "external_id_kind",
                "wallet_kind",
                "asset",
            ]
            .into_iter()
            .map(|key| {
                (
                    key.into(),
                    row.get(key).cloned().unwrap_or(serde_json::Value::Null),
                )
            })
            .collect(),
        );
        if let Some(scope) = kassiber_custody::journal_support::onchain_scope(&native) {
            if scope.0 != "bitcoin" || scope.3 != "BTC" {
                continue;
            }
            let (wallets, group) = groups.entry(scope).or_default();
            if row.get("direction").and_then(serde_json::Value::as_str) == Some("inbound") {
                wallets.insert(
                    row.get("wallet_id")
                        .map(kassiber_custody::codec::pystr)
                        .unwrap_or_default(),
                );
            }
            group.push(native);
        }
    }
    let mut scripts = BTreeSet::new();
    let mut candidate = false;
    for (wallets, group) in groups.values() {
        if wallets.len() > 1 {
            candidate = true;
            for row in group {
                if let Some(graph) =
                    kassiber_custody::journal_support::parsed(row).map_err(Error::Invalid)?
                {
                    for output in graph["outputs"].as_array().into_iter().flatten() {
                        if let Some(script) = output["script"].as_str() {
                            scripts.insert(script.to_owned());
                        }
                    }
                }
            }
        }
    }
    if !candidate {
        return Ok(vec![]);
    }
    let counts=query(conn,"SELECT wallet_id,COUNT(*) AS count FROM transactions WHERE profile_id=? AND excluded=0 GROUP BY wallet_id",[profile_id])?.into_iter().map(|row|(text(get(&row,"wallet_id")),get(&row,"count").clone())).collect::<HashMap<_,_>>();
    let wallets=query(conn,"SELECT id,label,kind,config_json FROM wallets WHERE profile_id=? ORDER BY label ASC,id ASC",[profile_id])?;
    let mut sources = vec![];
    for wallet in &wallets {
        let config = config(wallet)?;
        let (chain, network) = scope(get(&config, "chain"), get(&config, "network"));
        if chain.is_empty() || network.is_empty() {
            continue;
        }
        for (index, address) in addresses(&config)?.into_iter().enumerate() {
            let Some(script) = bitcoin_address_script(&address) else {
                continue;
            };
            let mut item = source(
                profile_id,
                wallet,
                &chain,
                &network,
                &script,
                "address_list",
                &counts,
            )?;
            set(&mut item, "branch_label", Value::from("address"));
            set(&mut item, "address_index", Value::from(index));
            sources.push(item);
        }
    }
    let by_id = wallets
        .iter()
        .map(|wallet| (text(get(wallet, "id")), wallet))
        .collect::<HashMap<_, _>>();
    for row in query(conn,"SELECT DISTINCT wallet_id,chain,network,address,script_pubkey,branch_label,branch_index,address_index,raw_json FROM wallet_utxos WHERE profile_id=?",[profile_id])? {
        let raw=parsed(get(&row,"raw_json"));
        let script=normalized_script(get(&row,"script_pubkey")).or_else(||normalized_script(get(&raw,"script_pubkey"))).or_else(||bitcoin_address_script(&text(get(&row,"address"))));
        let Some(script)=script else { continue; };
        let id=text(get(&row,"wallet_id"));
        let fallback=dict([("id",Value::from(id.clone())),("label",Value::from(id.clone()))]);
        let wallet=by_id.get(&id).copied().unwrap_or(&fallback);
        let (chain,network)=scope(get(&row,"chain"),get(&row,"network"));
        let mut item=source(profile_id,wallet,&chain,&network,&script,"inventory",&counts)?;
        set(&mut item,"wallet_label",Value::from(text(get(wallet,"label"))));
        for key in ["branch_index","branch_label","address_index"] { set(&mut item,key,get(&row,key).clone()); } sources.push(item);
    }
    let mut maxes = HashMap::<String, HashMap<u32, kassiber_pyjson::BigInt>>::new();
    for row in query(conn,"SELECT wallet_id,branch_index,MAX(address_index) AS max_index FROM wallet_utxos WHERE profile_id=? AND branch_index IS NOT NULL AND address_index IS NOT NULL GROUP BY wallet_id,branch_index",[profile_id])? {
        let branch=legacy_integer(get(&row,"branch_index"))?;
        if let Some(branch)=branch {
            let index=legacy_integer(get(&row,"max_index"))?;
            if let (Ok(branch),Some(index))=(u32::try_from(branch),index) { maxes.entry(text(get(&row,"wallet_id"))).or_default().insert(branch,index); }
        }
    }
    let mut checkpoints = HashMap::<String, HashMap<u32, kassiber_pyjson::BigInt>>::new();
    for row in query(conn,"SELECT source_key,checkpoint_json FROM freshness_source_states WHERE profile_id=? AND source_key LIKE ?",[profile_id,"onchain_wallet:%"])? {
        let config = parsed(get(&row,"checkpoint_json"));
        if let Value::Dict(values) = get(&config,"highest_used") {
            let id = text(get(&row,"source_key"));
            let id = id.strip_prefix("onchain_wallet:").unwrap_or(&id).to_owned();
            let mut branches = HashMap::new();
            for (key,value) in values {
                let Key::Str(key) = key else { continue; };
                if let Some(branch) = key.as_str().and_then(|s|kassiber_pytext::parse_int(s).ok()) {
                    let index=legacy_integer(value)?;
                    if let (Ok(branch),Some(index)) = (u32::try_from(branch),index) {
                    // Python's int-normalized aliases and duplicate source
                    // rows use the last value before merging source classes.
                    branches.insert(branch,index);
                    }
                }
            }
            if !branches.is_empty() { checkpoints.insert(id,branches); }
        }
    }
    for (id, branches) in checkpoints {
        for (branch, index) in branches {
            maxes
                .entry(id.clone())
                .or_default()
                .entry(branch)
                .and_modify(|n| *n = n.clone().max(index.clone()))
                .or_insert(index);
        }
    }
    for branches in maxes.values_mut() {
        for index in branches.values_mut() {
            *index = index.clone().max((-1).into());
        }
    }
    for wallet in &wallets {
        let config = config(wallet)?;
        if !has_descriptor(&config) {
            continue;
        }
        let Ok(Some(plan)) = descriptors::load(&kassiber_custody::codec::to_json(&config)) else {
            continue;
        };
        for branch in &plan.branches {
            let known = maxes
                .get(&text(get(wallet, "id")))
                .and_then(|m| m.get(&branch.index));
            let end = known
                .map(|n| n + plan.gap_limit + 1_u32)
                .unwrap_or(plan.gap_limit.into())
                .clamp(0.into(), 20_000.into());
            let Ok(targets) = plan.derive(
                Some(branch.index),
                0,
                u32::try_from(end)
                    .map_err(|_| Error::Invalid("receipt derivation horizon".into()))?,
            ) else {
                continue;
            };
            for target in targets {
                let mut item = source(
                    profile_id,
                    wallet,
                    &plan.chain,
                    &plan.network,
                    target["script_pubkey"].as_str().unwrap_or(""),
                    "descriptor_config",
                    &counts,
                )?;
                for key in ["branch_index", "branch_label", "address_index"] {
                    set(
                        &mut item,
                        key,
                        kassiber_custody::codec::from_json(&target[key]).map_err(Error::Invalid)?,
                    );
                }
                sources.push(item);
            }
        }
    }
    Ok(sources
        .into_iter()
        .filter(|source| scripts.contains(&text(get(source, "script_pubkey"))))
        .collect())
}

// A canonical identity must contain 64 consecutive ASCII hex digits either
// in external_id or in the decoded graph. Escaped JSON takes the full parser;
// this is only a negative lexical filter, never identity/ownership admission.
fn may_have_txid(row: &kassiber_custody::snapshot::Record) -> bool {
    fn run(value: &str) -> bool {
        let mut count = 0;
        value.bytes().any(|b| {
            count = if b.is_ascii_hexdigit() { count + 1 } else { 0 };
            count >= 64
        })
    }
    let external = row.get("external_id").unwrap_or(&serde_json::Value::Null);
    if external
        .as_str()
        .map(run)
        .unwrap_or_else(|| external.is_number() && run(&external.to_string()))
    {
        return true;
    }
    match row.get("raw_json") {
        Some(serde_json::Value::String(raw)) => raw.contains('\\') || run(raw),
        Some(serde_json::Value::Object(_) | serde_json::Value::Array(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_loader_receipt_defaults_preserve_missing_scope_and_source_facts() {
        for missing in [Value::Null, Value::from("")] {
            assert_eq!(
                scope(&missing, &Value::from("main")),
                (String::new(), String::new())
            );
            assert_eq!(
                scope(&Value::from("bitcoin"), &missing),
                (String::new(), String::new())
            );
        }
        assert_eq!(
            scope(&Value::from(" BTC "), &Value::from("mainnet")),
            ("bitcoin".into(), "main".into())
        );
        for raw in [Value::Null, Value::from("broken"), Value::from("[]")] {
            assert_eq!(config(&dict([("config_json", raw)])).unwrap(), dict([]));
        }
        let wallet = dict([
            ("id", Value::from("w")),
            ("label", Value::Null),
            ("kind", Value::Null),
            ("config_json", Value::from("{}")),
        ]);
        for (kind, expected) in [("address_list", "address"), ("inventory", "")] {
            let item = source("p", &wallet, "", "", "aa", kind, &HashMap::new()).unwrap();
            assert_eq!(get(&item, "wallet_id"), &Value::from("w"));
            assert_eq!(get(&item, "wallet_label"), &Value::from("w"));
            assert_eq!(get(&item, "wallet_kind"), &Value::from(expected));
            assert_eq!(get(&item, "chain"), &Value::from(""));
            assert_eq!(get(&item, "network"), &Value::from(""));
            assert_eq!(get(&item, "active_transaction_count"), &Value::from(0));
        }
    }

    #[test]
    fn native_loader_receipt_filter_never_rejects_possible_identities() {
        use serde_json::{json, Value as J};
        let txid = "a".repeat(64);
        let check = |external: J, raw: J| {
            may_have_txid(
                &json!({"external_id": external, "raw_json": raw})
                    .as_object()
                    .unwrap()
                    .clone()
                    .into(),
            )
        };
        assert!(check(json!(txid), J::Null));
        assert!(check(
            json!("invoice"),
            json!(format!("{{\"txid\":\"{txid}\"}}"))
        ));
        assert!(check(J::Null, json!(r#"{"txid":"\u0061"}"#)));
        assert!(check(J::Null, json!({"txid": txid})));
        assert!(check(J::Null, json!([{"txid": txid}])));
        assert!(check(
            serde_json::from_str(&"1".repeat(64)).unwrap(),
            J::Null
        ));
        for (external, raw) in [
            (J::Null, J::Null),
            (json!(false), json!(0)),
            (json!("invoice"), json!("{}")),
            (json!("a".repeat(63)), json!("a".repeat(63))),
            (
                json!(format!("{}z{}", "a".repeat(32), "a".repeat(32))),
                J::Null,
            ),
        ] {
            assert!(!check(external, raw));
        }
    }
}

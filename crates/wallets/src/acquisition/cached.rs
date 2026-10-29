//! Incremental protocol fetch policy, matching Python compatibility adapters.
//! Results are preparation data; membership is not retraction authority here.
use super::*;
use crate::compatibility_cache as cache;

fn identity(a: &Acquisition, c: &CacheInput) -> Value {
    cache::backend_identity(&c.config, &c.network, a.backend.batch_size())
}
fn highest(cp: &mut Value, target: &Value, used: bool) -> Result<()> {
    if !used {
        return Ok(());
    }
    if !cp["highest_used"].is_object() {
        if crate::descriptors::truthy(&cp["highest_used"]) {
            return Err(invalid());
        }
        cp["highest_used"] = json!({});
    }
    let branch = crate::descriptors::text(target.get("branch_index").unwrap_or(&json!(0)));
    let index = metadata_count(&target["address_index"])?;
    let previous = if crate::descriptors::truthy(&cp["highest_used"][&branch]) {
        crate::scan::integer(&cp["highest_used"][&branch]).ok_or_else(invalid)?
    } else {
        (-1).into()
    };
    cp["highest_used"][&branch] =
        serde_json::from_str(&previous.max(index).to_string()).expect("integer");
    Ok(())
}
fn count(stats: &Value) -> Option<u64> {
    stats["chain_stats"]["tx_count"]
        .as_u64()?
        .checked_add(stats["mempool_stats"]["tx_count"].as_u64()?)
}
// Python's display/cache counters use int(value or 0). Completeness uses the
// separate strict integer count above: coercion must never establish absence.
fn metadata_count(value: &Value) -> Result<kassiber_pyjson::BigInt> {
    if !crate::descriptors::truthy(value) {
        return Ok(0.into());
    }
    crate::scan::integer(value).ok_or_else(invalid)
}
fn integer_json(value: kassiber_pyjson::BigInt) -> Value {
    serde_json::from_str(&value.to_string()).expect("integer")
}
fn normalized_headers(saved: &Value) -> Map<String, Value> {
    saved
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| {
            (!key.is_empty()
                && key
                    .chars()
                    .all(|c| kassiber_pytext::decimal_value(c as u32).is_some()))
            .then(|| crate::scan::integer(&json!(key)).map(|n| (n.to_string(), value.clone())))
            .flatten()
        })
        .collect()
}
fn matches(tx: &Value, script: &str) -> bool {
    let (Some(vin), Some(vout)) = (tx["vin"].as_array(), tx["vout"].as_array()) else {
        return false;
    };
    fn output(o: &Value) -> &Value {
        for key in ["scriptpubkey", "script_hex"] {
            if !o[key].is_null() {
                return &o[key];
            }
        }
        let nested = &o["scriptPubKey"];
        if nested.is_string() {
            nested
        } else if crate::descriptors::truthy(&nested["hex"]) {
            &nested["hex"]
        } else {
            &nested["scriptpubkey"]
        }
    }
    vin.iter().any(|i| output(&i["prevout"]) == script) || vout.iter().any(|o| output(o) == script)
}
fn finish_members(
    prior: &BTreeMap<String, Value>,
    current: BTreeMap<String, Value>,
    key: &str,
) -> Value {
    let complete = prior.keys().all(|s| current.contains_key(s))
        && current.values().all(|e| e["complete"] == true);
    let retained = if complete {
        current
    } else {
        prior.keys().chain(current.keys()).map(|s| {
            let ids:BTreeSet<_>=prior.get(s).into_iter().chain(current.get(s)).flat_map(|e|e["txids"].as_array().into_iter().flatten()).filter_map(Value::as_str).map(str::to_owned).collect();
            (s.clone(),json!({"txids":ids,"fingerprint":current.get(s).map(|e|&e["fingerprint"]),"complete":false}))
        }).collect()
    };
    json!({"version":1,"backend_key":key,"scripts":retained})
}
impl Acquisition {
    pub(super) fn cached_esplora(
        &self,
        gate: &Gate,
        consent: &Consent,
        deadline: Instant,
        cancel: &Cancellation,
        c: &CacheInput,
    ) -> Result<Value> {
        let client = HttpClient::new(self.backend.clone(), gate.clone(), &self.version);
        let key = cache::esplora_key(&c.config, &identity(self, c));
        let prior = cache::memberships(&c.checkpoint["esplora_history_memberships"], &key);
        let mut cp = if c.checkpoint.is_object() {
            c.checkpoint.clone()
        } else {
            json!({})
        };
        let mut scripts = vec![];
        let mut transactions = Map::new();
        let mut stats = Map::new();
        let mut members = BTreeMap::new();
        let mut changed = 0;
        for target in &self.targets {
            cancel.check(deadline)?;
            let hash = script_hash(target["script_pubkey"].as_str().ok_or_else(invalid)?)?;
            let path = format!("scripthash/{hash}");
            let before = client.get_json(&path, Some(consent), deadline)?;
            let fingerprint = cache::stats_fingerprint(&before);
            let mempool = metadata_count(&before["mempool_stats"]["tx_count"])?;
            let total = metadata_count(&before["chain_stats"]["tx_count"])? + &mempool;
            let dirty = mempool > 0.into();
            highest(&mut cp, target, total > 0.into())?;
            stats.insert(hash.clone(),json!({"fingerprint":fingerprint,"tx_count":integer_json(total),"mempool_tx_count":integer_json(mempool),"mempool_dirty":dirty}));
            if cache::esplora_reusable(
                &c.checkpoint["esplora_scripthashes"][&hash],
                prior.get(&hash),
                &fingerprint,
            ) {
                members.insert(hash.clone(), prior[&hash].clone());
                scripts.push(json!({"target":target,"before":before,"reused":true}));
                continue;
            }
            changed += 1;
            let history =
                esplora_history(&client, &path, self.max_pages, consent, deadline, cancel)?;
            let after = client.get_json(&path, Some(consent), deadline)?;
            let ids: BTreeSet<_> = history
                .iter()
                .filter(|tx| matches(tx, target["script_pubkey"].as_str().unwrap()))
                .filter_map(|tx| tx["txid"].as_str().map(str::to_ascii_lowercase))
                .collect();
            let complete = self.max_pages.is_none()
                && count(&before) == Some(history.len() as u64)
                && count(&after) == count(&before)
                && cache::stats_fingerprint(&after) == fingerprint
                && ids.len() == history.len();
            members.insert(
                hash,
                json!({"txids":ids,"fingerprint":fingerprint,"complete":complete}),
            );
            for tx in &history {
                transactions.insert(tx["txid"].as_str().ok_or_else(invalid)?.into(), tx.clone());
            }
            scripts.push(json!({"target":target,"before":before,"after":after,"history":history,"reused":false}));
        }
        let skipped = !c.checkpoint.as_object().is_none_or(Map::is_empty)
            && changed == 0
            && !scripts.is_empty();
        if !skipped {
            for script in &mut scripts {
                cancel.check(deadline)?;
                let h = script_hash(
                    script["target"]["script_pubkey"]
                        .as_str()
                        .ok_or_else(invalid)?,
                )?;
                script["utxos"] =
                    client.get_json(&format!("scripthash/{h}/utxo"), Some(consent), deadline)?;
            }
        }
        cp["backend"] = identity(self, c);
        cp["esplora_scripthashes"] = Value::Object(stats);
        cp["esplora_history_memberships"] = finish_members(&prior, members, &key);
        cancel.check(deadline)?;
        Ok(
            json!({"scripts":scripts,"transactions":transactions,"freshness_checkpoint":cp,"scripts_changed":changed,"scripts_unchanged":self.targets.len()-changed,"utxos_skipped_unchanged":skipped}),
        )
    }
    pub(super) fn cached_electrum(
        &self,
        gate: &Gate,
        consent: &Consent,
        deadline: Instant,
        cancel: &Cancellation,
        c: &CacheInput,
    ) -> Result<Value> {
        let mut client =
            ElectrumClient::connect(self.backend.clone(), gate, Some(consent.clone()), deadline)?;
        let hashes = self
            .targets
            .iter()
            .map(|t| script_hash(t["script_pubkey"].as_str().ok_or_else(invalid)?))
            .collect::<Result<Vec<_>>>()?;
        let statuses = calls(
            &mut client,
            "blockchain.scripthash.subscribe",
            &hashes.iter().map(|h| json!([h])).collect::<Vec<_>>(),
            deadline,
            cancel,
        )?;
        let mut cp = if c.checkpoint.is_object() {
            c.checkpoint.clone()
        } else {
            json!({})
        };
        let mut next_status = Map::new();
        let mut entries = Map::new();
        let mut changed_hashes = vec![];
        let mut scripts = vec![];
        for ((target, hash), status) in self.targets.iter().zip(&hashes).zip(statuses) {
            next_status.insert(hash.clone(), status.clone());
            highest(&mut cp, target, !status.is_null())?;
            let reused = cache::electrum_reusable(&c.checkpoint, hash, &status);
            if reused {
                entries.insert(
                    hash.clone(),
                    if status.is_null() {
                        json!({})
                    } else {
                        c.checkpoint["electrum_history_entries"][hash]
                            .as_object()
                            .cloned()
                            .map_or(json!({}), Value::Object)
                    },
                );
            } else {
                changed_hashes.push(hash.clone());
            }
            scripts.push(json!({"target":target,"status":status,"reused":reused}));
        }
        let fetched = calls(
            &mut client,
            "blockchain.scripthash.get_history",
            &changed_hashes
                .iter()
                .map(|h| json!([h]))
                .collect::<Vec<_>>(),
            deadline,
            cancel,
        )?;
        let mut txids = BTreeMap::new();
        let mut dirty = BTreeSet::new();
        for (h, history) in changed_hashes.iter().zip(fetched) {
            let history = if history.is_null() {
                json!([])
            } else {
                history
            };
            let (current, changes, needs_recheck) =
                cache::changed_entries(&history, &c.checkpoint["electrum_history_entries"][h])?;
            entries.insert(h.clone(), current);
            if needs_recheck {
                dirty.insert(h.clone());
            }
            for entry in changes {
                txids.insert(
                    entry["tx_hash"].as_str().ok_or_else(invalid)?.to_owned(),
                    entry,
                );
            }
            for (index, hash) in hashes.iter().enumerate() {
                if hash == h {
                    scripts[index]["history"] = history.clone();
                }
            }
        }
        let ordered: Vec<_> = txids.keys().cloned().collect();
        let mut transactions = Map::new();
        for id in &ordered {
            if let Some(tx) = cache::stored_transaction(&c.history, id) {
                transactions.insert(id.clone(), tx);
            }
        }
        let missing: Vec<_> = ordered
            .iter()
            .filter(|id| !transactions.contains_key(*id))
            .cloned()
            .collect();
        let mut fetched_count = missing.len();
        fetch_transactions(&mut client, &missing, &mut transactions, deadline, cancel)?;
        let mut parents = BTreeSet::new();
        for id in &ordered {
            for input in transactions[id]["vin"].as_array().ok_or_else(invalid)? {
                if !coinbase_input(input) {
                    parents.insert(input["txid"].as_str().ok_or_else(invalid)?.to_owned());
                }
            }
        }
        let mut missing = vec![];
        for id in parents {
            if transactions.contains_key(&id) {
                continue;
            }
            if let Some(tx) = cache::stored_transaction(&c.history, &id) {
                transactions.insert(id, tx);
            } else {
                missing.push(id);
            }
        }
        fetched_count += missing.len();
        fetch_transactions(&mut client, &missing, &mut transactions, deadline, cancel)?;
        for id in &ordered {
            let mut tx = transactions[id].clone();
            normalize_graph(&mut tx, &transactions)?;
            tx["txid"] = json!(id);
            transactions.insert(id.clone(), tx);
        }
        let mut headers = normalized_headers(&c.checkpoint["electrum_headers"]);
        let heights: BTreeSet<_> = txids
            .values()
            .filter_map(|h| positive_height(&h["height"]))
            .collect();
        let mut hits = 0;
        for h in heights {
            let k = h.to_string();
            if headers.contains_key(&k) {
                hits += 1;
            } else {
                let raw = client.call("blockchain.block.header", Some(&json!([h])), deadline)?;
                headers.insert(
                    k,
                    json!(header_timestamp(raw.as_str().ok_or_else(invalid)?)?),
                );
            }
        }
        let skipped = !c.checkpoint.as_object().is_none_or(Map::is_empty)
            && changed_hashes.is_empty()
            && !scripts.is_empty();
        if !skipped {
            let utxos = calls(
                &mut client,
                "blockchain.scripthash.listunspent",
                &hashes.iter().map(|h| json!([h])).collect::<Vec<_>>(),
                deadline,
                cancel,
            )?;
            for (script, unspent) in scripts.iter_mut().zip(utxos) {
                script["utxos"] = unspent;
            }
        }
        cp.as_object_mut().unwrap().remove("electrum_known_txids");
        cp["backend"] = identity(self, c);
        cp["electrum_stored_graph_version"] = json!(1);
        cp["electrum_scripthash_statuses"] = Value::Object(next_status);
        cp["electrum_history_entries"] = Value::Object(entries);
        cp["electrum_dirty_scripthashes"] = json!(dirty);
        cp["electrum_headers"] = Value::Object(headers);
        cancel.check(deadline)?;
        Ok(
            json!({"scripts":scripts,"transactions":transactions,"freshness_checkpoint":cp,"scripts_changed":changed_hashes.len(),"scripts_unchanged":self.targets.len()-changed_hashes.len(),"utxos_skipped_unchanged":skipped,"transactions_fetched":fetched_count,"header_cache_hits":hits}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acquisition::tests::{deadline, gate, run, scope, targets as raw_targets};
    use kassiber_net::{
        test_support::{FakeElectrum, FakeProvider},
        Offline,
    };
    fn targets() -> Vec<Value> {
        raw_targets()
            .into_iter()
            .map(|mut t| {
                t["network"] = json!("main");
                t
            })
            .collect()
    }
    #[test]
    fn compatibility_cached_network_is_explicit_even_without_targets() {
        let backend = json!({"kind":"esplora","url":"http://127.0.0.1:1"});
        assert!(Acquisition::new_cached(
            scope(),
            &backend,
            &targets(),
            "fixture",
            &json!({}),
            &json!({}),
            "regtest"
        )
        .is_err());
        let a = Acquisition::new_cached(
            scope(),
            &backend,
            &[],
            "fixture",
            &json!({}),
            &json!({}),
            "regtest",
        )
        .unwrap();
        assert_eq!(
            identity(&a, a.cache.as_ref().unwrap())["network"],
            "regtest"
        );
    }
    #[test]
    fn compatibility_cached_indices_fail_closed_and_header_keys_normalize_like_python() {
        let target = json!({"branch_index":0,"address_index":"٤"});
        let mut cp = json!({"highest_used":{"0":2.9}});
        highest(&mut cp, &target, true).unwrap();
        assert_eq!(cp["highest_used"]["0"], 4);
        assert!(highest(&mut json!({"highest_used":{"0":"invalid"}}), &target, true).is_err());
        assert!(highest(&mut json!({}), &json!({"address_index":"invalid"}), true).is_err());
        assert_eq!(
            normalized_headers(&json!({"005":123,"٦":124,"invalid":125,"-1":126," 7 ":127})),
            json!({"5":123,"6":124}).as_object().unwrap().clone()
        );
    }
    #[test]
    fn compatibility_cached_metadata_coercions_never_establish_complete_counts() {
        for (value, expected) in [
            (json!(null), 0),
            (json!(false), 0),
            (json!(""), 0),
            (json!("١"), 1),
            (json!(true), 1),
            (json!(1.9), 1),
            (json!(-1), -1),
        ] {
            assert_eq!(metadata_count(&value).unwrap(), expected.into());
            assert_eq!(
                count(&json!({"chain_stats":{"tx_count":value},"mempool_stats":{"tx_count":0}})),
                None
            );
        }
        assert!(metadata_count(&json!("invalid")).is_err());
        assert_eq!(
            count(&json!({"chain_stats":{"tx_count":1},"mempool_stats":{"tx_count":0}})),
            Some(1)
        );
    }
    #[test]
    fn compatibility_cached_membership_requires_matching_graph_and_retains_partial_positives() {
        for output in [
            json!({"scriptpubkey":"51"}),
            json!({"script_hex":"51"}),
            json!({"scriptPubKey":{"hex":"51"}}),
            json!({"scriptPubKey":"51"}),
        ] {
            assert!(matches(&json!({"vin":[],"vout":[output.clone()]}), "51"));
            assert!(matches(
                &json!({"vin":[{"prevout":output}],"vout":[]}),
                "51"
            ));
        }
        assert!(!matches(
            &json!({"vin":[],"vout":[{"scriptpubkey":"52"}]}),
            "51"
        ));
        assert!(!matches(&json!({"vout":[{"scriptpubkey":"51"}]}), "51"));
        let prior = BTreeMap::from([("s".into(), json!({"txids":["a"],"complete":true}))]);
        let current = BTreeMap::from([(
            "s".into(),
            json!({"txids":["b"],"fingerprint":"f","complete":false}),
        )]);
        let p = finish_members(&prior, current, "k");
        assert_eq!(p["scripts"]["s"]["txids"], json!(["a", "b"]));
        assert_eq!(p["scripts"]["s"]["complete"], false);
        let current = BTreeMap::from([(
            "s".into(),
            json!({"txids":["b"],"fingerprint":"f","complete":true}),
        )]);
        assert_eq!(
            finish_members(&prior, current, "k")["scripts"]["s"]["txids"],
            json!(["b"])
        );
    }
    #[test]
    fn compatibility_cached_esplora_fetches_once_then_reuses_only_complete_membership() {
        let stats = json!({"chain_stats":{"tx_count":1},"mempool_stats":{"tx_count":0}});
        let tx = json!({"txid":"a".repeat(64),"vin":[],"vout":[{"scriptpubkey":"51","value":1}]});
        let server = FakeProvider::replies(vec![
            (200, stats.to_string()),
            (200, json!([tx]).to_string()),
            (200, "[]".into()),
            (200, stats.to_string()),
            (200, "[]".into()),
            (200, stats.to_string()),
        ])
        .unwrap();
        let backend = json!({"kind":"esplora","name":"fixture","url":server.base()});
        let a = Acquisition::new_cached(
            scope(),
            &backend,
            &targets(),
            "fixture",
            &json!({}),
            &json!({}),
            "main",
        )
        .unwrap();
        let first = run(&a).unwrap();
        let hash = script_hash("51").unwrap();
        let checkpoint = &first["freshness_checkpoint"];
        assert_eq!(
            checkpoint["esplora_history_memberships"]["scripts"][&hash]["complete"],
            true
        );
        assert_eq!(first["scripts_changed"], 1);
        assert_eq!(first["utxos_skipped_unchanged"], false);
        let a = Acquisition::new_cached(
            scope(),
            &backend,
            &targets(),
            "fixture",
            checkpoint,
            &json!({}),
            "main",
        )
        .unwrap();
        let second = run(&a).unwrap();
        assert_eq!(second["scripts_changed"], 0);
        assert_eq!(second["scripts_unchanged"], 1);
        assert_eq!(second["utxos_skipped_unchanged"], true);
        assert_eq!(server.requests().len(), 6);
        assert_eq!(second["freshness_checkpoint"], *checkpoint);
        let prior = BTreeMap::from([(
            "omitted".into(),
            json!({"txids":["a".repeat(64)],"complete":true}),
        )]);
        let retained = finish_members(&prior, BTreeMap::new(), "k");
        assert_eq!(
            retained["scripts"]["omitted"]["txids"],
            json!(["a".repeat(64)])
        );
        assert_eq!(retained["scripts"]["omitted"]["complete"], false);
    }
    #[test]
    fn compatibility_cached_electrum_skips_history_graph_headers_and_inventory() {
        let response = |id, v| format!("{}\n", json!({"id":id,"result":v}));
        let server = FakeElectrum::script(vec![
            (1, response(1, json!(["fixture", "1.6"]))),
            (1, response(2, json!("status"))),
        ])
        .unwrap();
        let hash = script_hash("51").unwrap();
        let checkpoint = json!({"electrum_stored_graph_version":1,"electrum_scripthash_statuses":{hash.clone():"status"},"electrum_history_entries":{hash:{}},"electrum_headers":{"5":123},"electrum_known_txids":["legacy"]});
        let a = Acquisition::new_cached(
            scope(),
            &json!({"kind":"electrum","url":server.url()}),
            &targets(),
            "fixture",
            &checkpoint,
            &json!({}),
            "main",
        )
        .unwrap();
        let p = run(&a).unwrap();
        assert_eq!(p["scripts_changed"], 0);
        assert_eq!(p["utxos_skipped_unchanged"], true);
        assert_eq!(p["transactions_fetched"], 0);
        assert_eq!(server.requests().len(), 2);
        assert!(p["freshness_checkpoint"]
            .get("electrum_known_txids")
            .is_none());
        assert_eq!(p["freshness_checkpoint"]["electrum_headers"]["5"], 123);
    }
    #[test]
    fn compatibility_cached_paths_require_bound_consent_and_online() {
        for (kind, url) in [
            ("electrum", "tcp://127.0.0.1:1"),
            ("esplora", "http://127.0.0.1:1"),
        ] {
            let a = Acquisition::new_cached(
                scope(),
                &json!({"kind":kind,"url":url}),
                &[],
                "fixture",
                &json!({"saved":true}),
                &json!({}),
                "main",
            )
            .unwrap();
            assert_eq!(
                a.run(
                    &gate(Offline::Online),
                    &scope(),
                    None,
                    deadline(),
                    &Cancellation::default(),
                    |_| {}
                )
                .unwrap_err()
                .code,
                "interaction_required"
            );
            let c = AcquisitionConsent::explicit_trigger(&a);
            let other = Acquisition::new_cached(
                scope(),
                &json!({"kind":kind,"url":url}),
                &[],
                "fixture",
                &json!({"saved":true}),
                &json!({}),
                "main",
            )
            .unwrap();
            let foreign = AcquisitionConsent::explicit_trigger(&other);
            assert_eq!(
                a.run(
                    &gate(Offline::Online),
                    &scope(),
                    Some(&foreign),
                    deadline(),
                    &Cancellation::default(),
                    |_| {}
                )
                .unwrap_err()
                .code,
                "interaction_required"
            );
            let mut stale = scope();
            stale.source_revision = "other".into();
            assert_eq!(
                a.run(
                    &gate(Offline::Online),
                    &stale,
                    Some(&c),
                    deadline(),
                    &Cancellation::default(),
                    |_| {}
                )
                .unwrap_err()
                .code,
                "stale_context"
            );
            assert_eq!(
                a.run(
                    &gate(Offline::Offline),
                    &scope(),
                    Some(&c),
                    deadline(),
                    &Cancellation::default(),
                    |_| {}
                )
                .unwrap_err()
                .code,
                "network_egress_disabled"
            );
        }
    }
}

//! Incremental protocol fetch policy, matching Python compatibility adapters.
//! Complete source-bound histories can prepare scoped absence candidates.
//! The coordinator must still apply them atomically with all positive facts.
use super::*;
use crate::compatibility_cache as cache;

fn identity(a: &Acquisition, c: &CacheInput) -> Value {
    cache::backend_identity(&c.config, &a.chain, &c.network, a.backend.batch_size())
}
fn checkpoint(saved: &Value) -> Value {
    let mut cp = if saved.is_object() {
        saved.clone()
    } else {
        json!({})
    };
    // Python writes dict(checkpoint.get("highest_used") or {}) even when the
    // complete history is empty and no target advances the highest index.
    if !crate::descriptors::truthy(&cp["highest_used"]) {
        cp["highest_used"] = json!({});
    }
    cp
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
/// `_cache_anchor_height`: a JSON integer block height a checkpoint can hold.
fn anchor_height(value: &Value) -> Option<u64> {
    value.as_u64().filter(|h| *h <= u64::from(u32::MAX))
}
/// The anchor format version; Python compares it numerically (`2.0 == 2`).
fn anchor_version(checkpoint: &Value, version: f64) -> bool {
    checkpoint["compatibility_history_anchor_version"].as_f64() == Some(version)
}
/// `_esplora_block_hash`: the canonical hash at a height, or `None`.
fn esplora_block_hash(
    client: &HttpClient,
    height: u64,
    consent: &Consent,
    deadline: Instant,
) -> Option<String> {
    let text = client
        .get_text(&format!("block-height/{height}"), Some(consent), deadline)
        .ok()?;
    let text = kassiber_pytext::strip(&text).to_ascii_lowercase();
    (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())).then_some(text)
}
/// `_electrum_header_fingerprint` of `blockchain.block.header`, or `None`.
fn electrum_header(client: &mut ElectrumClient, height: u64, deadline: Instant) -> Option<String> {
    let header = client
        .call("blockchain.block.header", Some(&json!([height])), deadline)
        .ok()?;
    kassiber_chain::electrum_header_fingerprint(&header)
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
) -> (Value, Vec<String>, bool) {
    let complete = prior.keys().all(|s| current.contains_key(s))
        && current.values().all(|e| e["complete"] == true);
    let ids = |entries: &BTreeMap<String, Value>| -> BTreeSet<String> {
        entries
            .values()
            .flat_map(|e| e["txids"].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    let retracted = if complete {
        ids(prior).difference(&ids(&current)).cloned().collect()
    } else {
        vec![]
    };
    let retained = if complete {
        current
    } else {
        prior.keys().chain(current.keys()).map(|s| {
            let ids:BTreeSet<_>=prior.get(s).into_iter().chain(current.get(s)).flat_map(|e|e["txids"].as_array().into_iter().flatten()).filter_map(Value::as_str).map(str::to_owned).collect();
            (s.clone(),json!({"txids":ids,"fingerprint":current.get(s).map(|e|&e["fingerprint"]),"complete":false}))
        }).collect()
    };
    (
        json!({"version":1,"backend_key":key,"scripts":retained}),
        retracted,
        complete,
    )
}
impl Acquisition {
    pub(super) fn cached_esplora(
        &self,
        gate: &Gate,
        consent: &Consent,
        deadline: Instant,
        cancel: &dyn Control,
        c: &CacheInput,
        history_ready: &mut dyn FnMut(&Value) -> Result<()>,
    ) -> Result<Value> {
        let client = HttpClient::new(self.backend.clone(), gate.clone(), &self.version);
        cancel.check(deadline)?;
        let mut tip_hash = client.esplora_tip_hash(Some(consent), deadline);
        let mut same_tip = tip_hash
            .as_ref()
            .is_some_and(|tip| c.checkpoint["esplora_tip_hash"] == *tip);
        // Bind the anchor to the observed hash's height. On a taller tip, the
        // saved tip must still be on the current chain for histories to be
        // reused; anything unprovable refreshes them.
        let previous_height = anchor_height(&c.checkpoint["esplora_tip_height"]);
        let mut tip_height = if same_tip { previous_height } else { None };
        if tip_hash.is_some() && tip_height.is_none() {
            cancel.check(deadline)?;
            let observed = client
                .get_text("blocks/tip/height", Some(consent), deadline)
                .ok()
                .and_then(|s| kassiber_pytext::parse_int(kassiber_pytext::strip(&s)).ok())
                .and_then(|n| anchor_height(&integer_json(n)));
            if let Some(observed) = observed {
                cancel.check(deadline)?;
                if esplora_block_hash(&client, observed, consent, deadline) == tip_hash {
                    tip_height = Some(observed);
                    if let Some(previous) = previous_height.filter(|p| observed > *p) {
                        cancel.check(deadline)?;
                        let previous = esplora_block_hash(&client, previous, consent, deadline);
                        same_tip =
                            previous.is_some_and(|hash| c.checkpoint["esplora_tip_hash"] == *hash);
                    }
                }
            }
        }
        // Older checkpoints can hold confirmations above their recorded anchor.
        let same_tip = same_tip && anchor_version(&c.checkpoint, 2.0);
        let key = cache::esplora_key(&c.config, &identity(self, c));
        let prior = cache::memberships(&c.checkpoint["esplora_history_memberships"], &key);
        let mut cp = checkpoint(&c.checkpoint);
        let mut scripts = vec![];
        let mut transactions = Map::new();
        let mut stats = Map::new();
        let mut members = BTreeMap::new();
        let workers = self.backend.batch_size().clamp(1, 8);
        let total = self.targets.len();
        let report = |event: Value| cancel.progress(event);
        let fetched = |index: usize, total: usize| -> Result<()> {
            if index.is_multiple_of(workers) || index == total {
                report(
                    json!({"phase":"backend_fetch","targets_checked":index,"target_count":total,
                    "processed":index,"total":total}),
                )?;
            }
            Ok(())
        };
        let mut observations = vec![];
        for (index, target) in self.targets.iter().enumerate() {
            cancel.check(deadline)?;
            let hash = script_hash(target["script_pubkey"].as_str().ok_or_else(invalid)?)?;
            let before = client.get_json(&format!("scripthash/{hash}"), Some(consent), deadline)?;
            observations.push((target, hash, before));
            fetched(index + 1, total)?;
        }
        let mut changed = 0;
        for (index, (target, hash, before)) in observations.into_iter().enumerate() {
            cancel.check(deadline)?;
            let fingerprint = cache::stats_fingerprint(&before);
            let mempool = metadata_count(&before["mempool_stats"]["tx_count"])?;
            let count = metadata_count(&before["chain_stats"]["tx_count"])? + &mempool;
            let dirty = mempool > 0.into();
            highest(&mut cp, target, count > 0.into())?;
            stats.insert(hash.clone(),json!({"fingerprint":fingerprint,"tx_count":integer_json(count),"mempool_tx_count":integer_json(mempool),"mempool_dirty":dirty}));
            let reused = same_tip
                && cache::esplora_reusable(
                    &c.checkpoint["esplora_scripthashes"][&hash],
                    prior.get(&hash),
                    &fingerprint,
                );
            if reused {
                members.insert(hash.clone(), prior[&hash].clone());
            } else {
                changed += 1;
            }
            scripts.push(json!({"target":target,"before":before,"reused":reused}));
            if (index + 1).is_multiple_of(workers) || index + 1 == total {
                report(
                    json!({"phase":"backend_fetch","targets_checked":index+1,"target_count":total,
                    "scripts_changed":changed,"scripts_unchanged":index+1-changed,"processed":index+1,"total":total}),
                )?;
            }
        }
        let mut histories = vec![];
        for script in scripts.iter().filter(|s| s["reused"] != true) {
            cancel.check(deadline)?;
            let target = &script["target"];
            let hash = script_hash(target["script_pubkey"].as_str().ok_or_else(invalid)?)?;
            let path = format!("scripthash/{hash}");
            let history =
                esplora_history(&client, &path, self.max_pages, consent, deadline, cancel)?;
            cancel.check(deadline)?;
            let after = client.get_json(&path, Some(consent), deadline)?;
            histories.push((hash, history, after));
            fetched(histories.len(), changed)?;
        }
        for (index, (script, (hash, history, after))) in scripts
            .iter_mut()
            .filter(|s| s["reused"] != true)
            .zip(histories)
            .enumerate()
        {
            cancel.check(deadline)?;
            let before = &script["before"];
            let fingerprint = cache::stats_fingerprint(before);
            let ids: BTreeSet<_> = history
                .iter()
                .filter(|tx| matches(tx, script["target"]["script_pubkey"].as_str().unwrap()))
                .filter_map(|tx| tx["txid"].as_str().map(str::to_ascii_lowercase))
                .collect();
            let complete = self.max_pages.is_none()
                && count(before) == Some(history.len() as u64)
                && count(&after) == count(before)
                && cache::stats_fingerprint(&after) == fingerprint
                && ids.len() == history.len();
            members.insert(
                hash,
                json!({"txids":ids,"fingerprint":fingerprint,"complete":complete}),
            );
            for tx in &history {
                transactions.insert(tx["txid"].as_str().ok_or_else(invalid)?.into(), tx.clone());
            }
            script["after"] = after;
            script["history"] = json!(history);
            if (index + 1).is_multiple_of(workers) || index + 1 == changed {
                report(
                    json!({"phase":"backend_fetch","targets_checked":index+1,"target_count":changed,
                    "known_txids":transactions.len(),"processed":index+1,"total":changed}),
                )?;
            }
        }
        // A block can arrive after the tip read. Every newer entry must belong
        // to the current chain, and that chain must still hold the first tip:
        // otherwise reused rows could be promoted onto a replacement fork.
        if let Some(height) = tip_height {
            let raced: Vec<&Value> = transactions
                .values()
                .map(|tx| &tx["status"])
                .filter(|status| anchor_height(&status["block_height"]).unwrap_or(0) > height)
                .collect();
            if !raced.is_empty() {
                let heights: BTreeSet<u64> = raced
                    .iter()
                    .filter_map(|status| anchor_height(&status["block_height"]))
                    .collect();
                let mut observed = BTreeMap::new();
                for h in heights {
                    cancel.check(deadline)?;
                    observed.insert(h, esplora_block_hash(&client, h, consent, deadline));
                }
                let on_chain = raced.iter().all(|status| {
                    let h = anchor_height(&status["block_height"]).expect("raced height");
                    observed[&h]
                        .as_ref()
                        .is_some_and(|hash| status["block_hash"] == *hash)
                });
                cancel.check(deadline)?;
                let initial = esplora_block_hash(&client, height, consent, deadline);
                if on_chain && initial.is_some() && initial == tip_hash {
                    let (highest, hash) = observed.pop_last().expect("raced heights");
                    tip_height = Some(highest);
                    tip_hash = hash;
                } else {
                    tip_height = None;
                    tip_hash = None;
                }
            }
        }
        // Python's Liquid records decode every history transaction and each
        // input's previous transaction from `tx/<txid>/hex`.
        let liquid = self.chain == "liquid";
        let mut raw_transactions = Map::new();
        let hex = |id: &str, into: &mut Map<String, Value>| -> Result<()> {
            if into.contains_key(id) {
                return Ok(());
            }
            cancel.check(deadline)?;
            let text = client.get_text(&format!("tx/{id}/hex"), Some(consent), deadline)?;
            let text = kassiber_pytext::strip(&text).to_owned();
            liquid_transaction(&text, id)?;
            into.insert(id.to_owned(), json!({"raw_hex": text}));
            Ok(())
        };
        if liquid {
            let mut ordered: Vec<(i64, &String)> = transactions
                .iter()
                .map(|(id, tx)| (tx["status"]["block_time"].as_i64().unwrap_or(0), id))
                .collect();
            ordered.sort();
            for (_, id) in &ordered {
                hex(id, &mut raw_transactions)?;
            }
            for (_, id) in &ordered {
                for parent in liquid_parents(&raw_transactions[id.as_str()])? {
                    hex(&parent, &mut raw_transactions)?;
                }
            }
        }
        cancel.check(deadline)?;
        history_ready(&json!({"transactions":transactions}))?;
        let skipped = !c.checkpoint.as_object().is_none_or(Map::is_empty)
            && changed == 0
            && !scripts.is_empty();
        // Python obtains the depth reference after history, before UTXOs. A
        // missing tip means unknown depth, never an invented zero height.
        let mut depth_height = None;
        if !skipped {
            cancel.check(deadline)?;
            depth_height = client
                .get_text("blocks/tip/height", Some(consent), deadline)
                .ok()
                .and_then(|s| kassiber_pytext::parse_int(kassiber_pytext::strip(&s)).ok())
                .and_then(|n| i64::try_from(n).ok());
            let mut utxos_seen = 0;
            for (index, script) in scripts.iter_mut().enumerate() {
                cancel.check(deadline)?;
                let h = script_hash(
                    script["target"]["script_pubkey"]
                        .as_str()
                        .ok_or_else(invalid)?,
                )?;
                script["utxos"] =
                    client.get_json(&format!("scripthash/{h}/utxo"), Some(consent), deadline)?;
                utxos_seen += script["utxos"].as_array().map_or(0, Vec::len);
                if liquid {
                    // Each Liquid output decodes its transaction.
                    for utxo in script["utxos"].as_array().into_iter().flatten() {
                        hex(
                            utxo["txid"].as_str().ok_or_else(invalid)?,
                            &mut raw_transactions,
                        )?;
                    }
                }
                if (index + 1).is_multiple_of(workers) || index + 1 == total {
                    report(
                        json!({"phase":"backend_fetch","targets_checked":index+1,"target_count":total,
                        "utxos_seen":utxos_seen,"processed":index+1,"total":total}),
                    )?;
                }
            }
        }
        cp["backend"] = identity(self, c);
        cp["esplora_tip_hash"] = json!(tip_hash);
        cp["compatibility_history_anchor_version"] = json!(2);
        cp["esplora_tip_height"] = json!(tip_height);
        cp["esplora_scripthashes"] = Value::Object(stats);
        let (membership, retracted, complete) = finish_members(&prior, members, &key);
        cp["esplora_history_memberships"] = membership;
        cancel.check(deadline)?;
        Ok(
            json!({"scripts":scripts,"transactions":transactions,"freshness_checkpoint":cp,"scripts_changed":changed,"scripts_unchanged":self.targets.len()-changed,"utxos_skipped_unchanged":skipped,"esplora_history_complete":complete,"observer_retracted_external_ids":retracted,"tip_height":depth_height,"raw_transactions":raw_transactions}),
        )
    }
    pub(super) fn cached_electrum(
        &self,
        gate: &Gate,
        consent: &Consent,
        deadline: Instant,
        cancel: &dyn Control,
        c: &CacheInput,
    ) -> Result<Value> {
        let mut client =
            ElectrumClient::connect(self.backend.clone(), gate, Some(consent.clone()), deadline)?;
        cancel.check(deadline)?;
        let (mut tip_fingerprint, tip_anchor) = client.tip_anchor(deadline);
        let mut tip_height = tip_anchor.map(u64::from);
        let mut same_tip = tip_fingerprint
            .as_ref()
            .is_some_and(|tip| c.checkpoint["electrum_tip_fingerprint"] == *tip);
        // On a taller tip, the saved tip header must still be on the chain.
        let previous_height = anchor_height(&c.checkpoint["electrum_tip_height"]);
        if let (false, true, Some(previous), Some(height)) = (
            same_tip,
            tip_fingerprint.is_some(),
            previous_height,
            tip_height,
        ) {
            if height > previous {
                cancel.check(deadline)?;
                same_tip = electrum_header(&mut client, previous, deadline)
                    .is_some_and(|fp| c.checkpoint["electrum_tip_fingerprint"] == *fp);
            }
        }
        let same_tip = same_tip && anchor_version(&c.checkpoint, 3.0);
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
        let mut cp = checkpoint(&c.checkpoint);
        let mut next_status = Map::new();
        let mut entries = Map::new();
        let mut changed_hashes = vec![];
        let mut scripts = vec![];
        // Python's progress: every `batch_size` scripts and the last one.
        let batch = self.backend.batch_size().max(1);
        let total = self.targets.len();
        for (index, ((target, hash), status)) in
            self.targets.iter().zip(&hashes).zip(statuses).enumerate()
        {
            next_status.insert(hash.clone(), status.clone());
            highest(&mut cp, target, !status.is_null())?;
            let reused = cache::electrum_reusable(&c.checkpoint, hash, &status, same_tip);
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
            if (index + 1).is_multiple_of(batch) || index + 1 == total {
                let changed = changed_hashes.len();
                cancel.progress(json!({"phase":"backend_fetch","target_count":total,
                    "targets_checked":index+1,"scripts_changed":changed,
                    "scripts_unchanged":index+1-changed,"processed":index+1,"total":total}))?;
            }
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
        for (position, (h, history)) in changed_hashes.iter().zip(fetched).enumerate() {
            let history = if history.is_null() {
                json!([])
            } else {
                history
            };
            let (current, changes, needs_recheck) = cache::changed_entries(
                &history,
                if same_tip {
                    &c.checkpoint["electrum_history_entries"][h]
                } else {
                    &Value::Null
                },
            )?;
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
            let changed = changed_hashes.len();
            if (position + 1).is_multiple_of(batch) || position + 1 == changed {
                cancel.progress(json!({"phase":"backend_fetch","target_count":changed,
                    "targets_checked":position+1,"known_txids":txids.len(),
                    "processed":position+1,"total":changed}))?;
            }
        }
        let ordered: Vec<_> = txids.keys().cloned().collect();
        // Liquid keeps the verified raw bytes; its records decode them. Python
        // offers stored graphs to Bitcoin only.
        let liquid = self.chain == "liquid";
        let mut transactions = Map::new();
        for id in ordered.iter().filter(|_| !liquid) {
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
        let fetch = |client: &mut ElectrumClient, ids: &[String], into: &mut Map<String, Value>| {
            if liquid {
                fetch_liquid(client, ids, into, deadline, cancel)
            } else {
                fetch_transactions(client, ids, into, deadline, cancel)
            }
        };
        fetch(&mut client, &missing, &mut transactions)?;
        let mut parents = BTreeSet::new();
        for id in &ordered {
            if liquid {
                // Every input, coinbase included, as Python looks them up.
                parents.extend(liquid_parents(&transactions[id])?);
                continue;
            }
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
            match cache::stored_transaction(&c.history, &id).filter(|_| !liquid) {
                Some(tx) => {
                    transactions.insert(id, tx);
                }
                None => missing.push(id),
            }
        }
        fetched_count += missing.len();
        fetch(&mut client, &missing, &mut transactions)?;
        for id in ordered.iter().filter(|_| !liquid) {
            let mut tx = transactions[id].clone();
            normalize_graph(&mut tx, &transactions)?;
            transactions.insert(id.clone(), tx);
        }
        let mut headers = if same_tip {
            normalized_headers(&c.checkpoint["electrum_headers"])
        } else {
            Map::new()
        };
        let heights: BTreeSet<_> = txids
            .values()
            .filter_map(|h| positive_height(&h["height"]))
            .collect();
        let mut hits = 0;
        // The exact header that supplied each timestamp fetched in this run.
        let mut fingerprints: BTreeMap<u64, Option<String>> = BTreeMap::new();
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
                if let Ok(h) = u64::try_from(h) {
                    fingerprints.insert(h, kassiber_chain::electrum_header_fingerprint(&raw));
                }
            }
        }
        // Histories can race above the tip read. Re-read every newer header
        // that supplied a timestamp, and the first tip, before advancing the
        // anchor to the highest cached block; a mixed-fork cache is untrusted.
        let cached: BTreeSet<u64> = headers.keys().filter_map(|k| k.parse().ok()).collect();
        let highest = cached.last().copied().unwrap_or(0);
        if let Some(height) = tip_height.filter(|h| highest > *h) {
            let mut raced_valid = true;
            for h in cached.iter().filter(|h| **h > height) {
                cancel.check(deadline)?;
                let reread = electrum_header(&mut client, *h, deadline);
                if reread.is_none() || reread != fingerprints.get(h).cloned().flatten() {
                    raced_valid = false;
                    break;
                }
            }
            cancel.check(deadline)?;
            let initial = electrum_header(&mut client, height, deadline);
            if raced_valid && initial.is_some() && initial == tip_fingerprint {
                tip_fingerprint = fingerprints.get(&highest).cloned().flatten();
                tip_height = tip_fingerprint.is_some().then_some(highest);
            } else {
                tip_fingerprint = None;
                tip_height = None;
            }
        }
        let skipped = !c.checkpoint.as_object().is_none_or(Map::is_empty)
            && changed_hashes.is_empty()
            && !scripts.is_empty();
        // Python lists UTXOs over a second connection after re-reading the
        // tip, which fixes their depth, then reads every confirmed UTXO's
        // header for its block time.
        let mut depth_height = None;
        let mut utxo_headers = Map::new();
        let mut utxo_transactions = Map::new();
        if !skipped {
            drop(client);
            cancel.check(deadline)?;
            let mut client = ElectrumClient::connect(
                self.backend.clone(),
                gate,
                Some(consent.clone()),
                deadline,
            )?;
            depth_height = client
                .call("blockchain.headers.subscribe", Some(&json!([])), deadline)
                .ok()
                .filter(Value::is_object)
                .and_then(|tip| crate::scan::integer(&tip["height"]))
                .map(integer_json);
            let utxos = calls(
                &mut client,
                "blockchain.scripthash.listunspent",
                &hashes.iter().map(|h| json!([h])).collect::<Vec<_>>(),
                deadline,
                cancel,
            )?;
            let heights: BTreeSet<i64> = utxos
                .iter()
                .flat_map(|unspent| unspent.as_array().into_iter().flatten())
                .filter_map(|utxo| positive_height(&utxo["height"]))
                .collect();
            let raw = calls(
                &mut client,
                "blockchain.block.header",
                &heights.iter().map(|h| json!([h])).collect::<Vec<_>>(),
                deadline,
                cancel,
            )?;
            for (height, header) in heights.iter().zip(raw) {
                utxo_headers.insert(
                    height.to_string(),
                    json!(header_timestamp(header.as_str().ok_or_else(invalid)?)?),
                );
            }
            // `compatibility_electrum_utxos_for_wallet` decodes each Liquid
            // output's transaction, fetched once over this connection.
            for txid in utxos
                .iter()
                .flat_map(|unspent| unspent.as_array().into_iter().flatten())
                .filter(|_| liquid)
                .map(|utxo| utxo["tx_hash"].as_str().ok_or_else(invalid))
                .collect::<Result<Vec<_>>>()?
            {
                if !utxo_transactions.contains_key(txid) {
                    cancel.check(deadline)?;
                    fetch_liquid(
                        &mut client,
                        &[txid.to_owned()],
                        &mut utxo_transactions,
                        deadline,
                        cancel,
                    )?;
                }
            }
            for (script, unspent) in scripts.iter_mut().zip(utxos) {
                script["utxos"] = unspent;
            }
        }
        cp.as_object_mut().unwrap().remove("electrum_known_txids");
        cp["backend"] = identity(self, c);
        cp["electrum_tip_fingerprint"] = json!(tip_fingerprint);
        cp["compatibility_history_anchor_version"] = json!(3);
        cp["electrum_tip_height"] = json!(tip_height);
        cp["electrum_stored_graph_version"] = json!(2);
        cp["electrum_scripthash_statuses"] = Value::Object(next_status);
        cp["electrum_history_entries"] = Value::Object(entries);
        cp["electrum_dirty_scripthashes"] = json!(dirty);
        cp["electrum_headers"] = Value::Object(headers);
        cancel.check(deadline)?;
        Ok(
            json!({"scripts":scripts,"transactions":transactions,"histories":txids,"freshness_checkpoint":cp,"scripts_changed":changed_hashes.len(),"scripts_unchanged":self.targets.len()-changed_hashes.len(),"utxos_skipped_unchanged":skipped,"transactions_fetched":fetched_count,"header_cache_hits":hits,"tip_height":depth_height,"utxo_headers":utxo_headers,"utxo_transactions":utxo_transactions}),
        )
    }
}

#[cfg(test)]
mod anchor_vectors;
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
        let p = finish_members(&prior, current, "k").0;
        assert_eq!(p["scripts"]["s"]["txids"], json!(["a", "b"]));
        assert_eq!(p["scripts"]["s"]["complete"], false);
        let current = BTreeMap::from([(
            "s".into(),
            json!({"txids":["b"],"fingerprint":"f","complete":true}),
        )]);
        assert_eq!(
            finish_members(&prior, current, "k").0["scripts"]["s"]["txids"],
            json!(["b"])
        );
    }
    #[test]
    fn compatibility_cached_esplora_fetches_once_then_reuses_only_complete_membership() {
        let stats = json!({"chain_stats":{"tx_count":1},"mempool_stats":{"tx_count":0}});
        let tx = json!({"txid":"a".repeat(64),"vin":[],"vout":[{"scriptpubkey":"51","value":1}]});
        // Each round reads the tip hash, then binds its height to that hash;
        // a taller tip proves the saved tip block before reusing histories.
        let fetch = |tip: &str, height: &str, previous: Option<&str>| {
            let mut replies = vec![(200, tip.into()), (200, height.into()), (200, tip.into())];
            replies.extend(previous.map(|hash| (200, hash.to_owned())));
            replies.extend([
                (200, stats.to_string()),
                (200, json!([tx.clone()]).to_string()),
                (200, "[]".into()),
                (200, stats.to_string()),
                (200, height.into()),
                (200, "[]".into()),
            ]);
            replies
        };
        let mut replies = fetch(&"a".repeat(64), "12", None);
        replies.extend([(200, "a".repeat(64)), (200, stats.to_string())]);
        // A reorged saved tip, then a checkpoint without a saved hash.
        replies.extend(fetch(&"b".repeat(64), "13", Some(&"c".repeat(64))));
        replies.extend(fetch(&"b".repeat(64), "13", Some(&"a".repeat(64))));
        let server = FakeProvider::replies(replies).unwrap();
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
        assert_eq!(first["tip_height"], 12);
        assert!(server.requests()[1].starts_with("GET /blocks/tip/height "));
        assert!(server.requests()[2].starts_with("GET /block-height/12 "));
        assert!(server.requests()[7].starts_with("GET /blocks/tip/height "));
        assert!(server.requests()[8].contains("/utxo "));
        assert_eq!(checkpoint["esplora_tip_height"], 12);
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
        assert_eq!(server.requests().len(), 11);
        assert_eq!(second["freshness_checkpoint"], *checkpoint);
        for mut saved in [checkpoint.clone(), checkpoint.clone()] {
            if server.requests().len() > 11 {
                saved.as_object_mut().unwrap().remove("esplora_tip_hash");
            }
            let a = Acquisition::new_cached(
                scope(),
                &backend,
                &targets(),
                "fixture",
                &saved,
                &json!({}),
                "main",
            )
            .unwrap();
            let refreshed = run(&a).unwrap();
            assert_eq!(refreshed["scripts_changed"], 1);
            assert_eq!(refreshed["utxos_skipped_unchanged"], false);
            assert_eq!(
                refreshed["freshness_checkpoint"]["esplora_tip_hash"],
                "b".repeat(64)
            );
            assert_eq!(refreshed["transactions"]["a".repeat(64)], tx);
        }
        assert_eq!(server.requests().len(), 31);
        let prior = BTreeMap::from([(
            "omitted".into(),
            json!({"txids":["a".repeat(64)],"complete":true}),
        )]);
        let retained = finish_members(&prior, BTreeMap::new(), "k").0;
        assert_eq!(
            retained["scripts"]["omitted"]["txids"],
            json!(["a".repeat(64)])
        );
        assert_eq!(retained["scripts"]["omitted"]["complete"], false);
    }
    #[test]
    fn cached_electrum_fresh_graph_preserves_decoder_shape() {
        let tx = bitcoin::Transaction {
            version: bitcoin::transaction::Version::TWO,
            lock_time: bitcoin::absolute::LockTime::ZERO,
            input: vec![bitcoin::TxIn::default()],
            output: vec![bitcoin::TxOut {
                value: bitcoin::Amount::from_sat(10),
                script_pubkey: bitcoin::ScriptBuf::from_hex("51").unwrap(),
            }],
        };
        let id = tx.compute_txid().to_string();
        let reply = |id, v| format!("{}\n", json!({"id":id,"result":v}));
        // UTXOs are listed over a second connection after a fresh tip read.
        let server = FakeElectrum::sessions(vec![
            vec![
                (1, reply(1, json!(["fixture", "1.6"]))),
                (1, reply(2, json!({"height":5,"hex":"00".repeat(80)}))),
                (1, reply(3, json!("status"))),
                (1, reply(4, json!([{"tx_hash":id,"height":0}]))),
                (
                    1,
                    reply(5, json!(bitcoin::consensus::encode::serialize_hex(&tx))),
                ),
            ],
            vec![
                (1, reply(1, json!(["fixture", "1.6"]))),
                (1, reply(2, json!({"height":6,"hex":"00".repeat(80)}))),
                (1, reply(3, json!([]))),
            ],
        ])
        .unwrap();
        let a = Acquisition::new_cached(
            scope(),
            &json!({"kind":"electrum","url":server.url()}),
            &targets(),
            "fixture",
            &json!({}),
            &json!({}),
            "main",
        )
        .unwrap();
        let result = run(&a).unwrap();
        assert!(result["transactions"][&id].get("txid").is_none());
        assert_eq!(
            result["transactions"][&id]["_kassiber_electrum_graph"]["kind"],
            "bitcoin_electrum"
        );
        assert_eq!(result["transactions_fetched"], 1);
        // The second connection's tip fixes the UTXO depth.
        assert_eq!(result["tip_height"], 6);
        assert_eq!(server.requests().len(), 8);
    }
    #[test]
    fn compatibility_cached_electrum_skips_history_graph_headers_and_inventory() {
        let response = |id, v| format!("{}\n", json!({"id":id,"result":v}));
        let server = FakeElectrum::script(vec![
            (1, response(1, json!(["fixture", "1.6"]))),
            (1, response(2, json!({"height":5,"hex":"00".repeat(80)}))),
            (1, response(3, json!("status"))),
        ])
        .unwrap();
        let hash = script_hash("51").unwrap();
        let checkpoint = json!({"electrum_tip_fingerprint":"5b6fb58e61fa475939767d68a446f97f1bff02c0e5935a3ea8bb51e6515783d8", "compatibility_history_anchor_version":3,"electrum_tip_height":5,"electrum_stored_graph_version":2,"electrum_scripthash_statuses":{hash.clone():"status"},"electrum_history_entries":{hash:{}},"electrum_headers":{"5":123},"electrum_known_txids":["legacy"]});
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
        assert_eq!(server.requests().len(), 3);
        assert!(p["freshness_checkpoint"]
            .get("electrum_known_txids")
            .is_none());
        assert_eq!(p["freshness_checkpoint"]["electrum_headers"]["5"], 123);
        assert_eq!(
            p["freshness_checkpoint"]["electrum_tip_fingerprint"],
            checkpoint["electrum_tip_fingerprint"]
        );
    }
    #[test]
    fn compatibility_tip_change_refreshes_history_and_headers_but_reuses_valid_graphs() {
        for saved_tip in [Value::Null, json!("old-tip")] {
            let id = "a".repeat(64);
            let hash = script_hash("51").unwrap();
            let header = format!("{}{}{}", "00".repeat(68), "00f15365", "00".repeat(8));
            let history = json!([{"tx_hash":id,"height":5}]);
            let checkpoint = json!({"electrum_tip_fingerprint":saved_tip,
                "electrum_stored_graph_version":2,"electrum_scripthash_statuses":{hash.clone():"status"},
                "electrum_history_entries":{hash.clone():{id.clone():history[0]}},"electrum_headers":{"5":123}});
            let graph = json!({id.clone():{"txid":id,"vin":[],"vout":[{"script_hex":"51","value_sats":1}],
                "_kassiber_electrum_graph":{"kind":"bitcoin_electrum","version":2}}});
            let response = |id, result| format!("{}\n", json!({"id":id,"result":result}));
            let server = FakeElectrum::sessions(vec![
                vec![
                    (1, response(1, json!(["fixture", "1.6"]))),
                    (1, response(2, json!({"height":5,"hex":"00".repeat(80)}))),
                    (1, response(3, json!("status"))),
                    (1, response(4, history)),
                    (1, response(5, json!(header))),
                ],
                vec![
                    (1, response(1, json!(["fixture", "1.6"]))),
                    (1, response(2, json!({"height":5,"hex":"00".repeat(80)}))),
                    (1, response(3, json!([]))),
                ],
            ])
            .unwrap();
            let a = Acquisition::new_cached(
                scope(),
                &json!({"kind":"electrum","url":server.url()}),
                &targets(),
                "fixture",
                &checkpoint,
                &graph,
                "main",
            )
            .unwrap();
            let actual = run(&a).unwrap();
            assert_eq!(actual["scripts_changed"], 1);
            assert_eq!(actual["transactions_fetched"], 0);
            assert_eq!(actual["header_cache_hits"], 0);
            assert_eq!(actual["utxos_skipped_unchanged"], false);
            assert_eq!(
                actual["freshness_checkpoint"]["electrum_headers"]["5"],
                1_700_000_000
            );
            assert_eq!(actual["transactions"][&id]["vout"][0]["value"], 1);
            assert_eq!(actual["transactions"][&id]["vout"][0]["scriptpubkey"], "51");
            assert_eq!(server.requests().len(), 8);
            assert_eq!(actual["tip_height"], 5);
        }
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

#[cfg(test)]
mod authority_tests {
    use super::*;

    fn members(entries: &[(&str, &[&str], bool)]) -> BTreeMap<String, Value> {
        entries
            .iter()
            .map(|(s, ids, complete)| {
                (
                    s.to_string(),
                    json!({"txids":ids,"complete":complete,"fingerprint":"f"}),
                )
            })
            .collect()
    }
    #[test]
    fn esplora_forced_scan_discards_fetch_caches_but_preserves_versioned_memberships() {
        let membership = json!({"version":1,"backend_key":"source","scripts":{}});
        let checkpoint = json!({"esplora_history_memberships":membership,"esplora_scripthashes":{"cache":true},"highest_used":{"0":8},
            "observer_instances":{"other":{},"bdk":{"tip_height":7,"tip_hash":"h","canonical_txids":["a"],"highest_used":{"0":1}},
                "lwk":{"tip_height":null,"canonical_txids":[]},"bad":[]}});
        // Each observer keeps its rebuild baseline: the persisted tip for the
        // lag guard and the txids it may retract against; caches are dropped.
        assert_eq!(
            full_scan_checkpoint(&checkpoint),
            json!({"esplora_history_memberships":membership,"observer_instances":{
                "bdk":{"tip_height":7,"canonical_txids":["a"]},"lwk":{"canonical_txids":[]},"other":{}}})
        );
        assert_eq!(
            full_scan_checkpoint(&json!({"observer_instances":{"bad":1}})),
            json!({})
        );
        assert_eq!(full_scan_checkpoint(&json!([])), json!({}));
        for saved in [
            json!(null),
            json!([]),
            json!(true),
            json!({"version":true}),
            json!({"version":1.0}),
            json!({"version":"1"}),
            json!({"version":2}),
        ] {
            assert_eq!(
                full_scan_checkpoint(&json!({"esplora_history_memberships":saved})),
                json!({})
            );
        }
    }
    #[test]
    fn esplora_absence_requires_complete_coverage_and_unions_live_script_memberships() {
        let prior = members(&[("s1", &["a", "b"], true), ("s2", &["c"], true)]);
        let current = members(&[("s1", &["b"], true), ("s2", &["a", "d"], true)]);
        let (checkpoint, retracted, complete) = finish_members(&prior, current.clone(), "source");
        assert!(complete);
        assert_eq!(retracted, ["c"]);
        assert_eq!(checkpoint["scripts"], json!(current));
        assert_eq!(checkpoint["version"], 1);
        assert_eq!(checkpoint["backend_key"], "source");
        // Complete emptiness is authority; omitted or partial scope is not.
        let empty = members(&[("s1", &[], true), ("s2", &[], true)]);
        let (_, retracted, complete) = finish_members(&prior, empty, "source");
        assert!(complete);
        assert_eq!(retracted, ["a", "b", "c"]);
        for current in [
            members(&[("s1", &[], true)]),
            members(&[("s1", &[], true), ("s2", &[], false)]),
        ] {
            let (checkpoint, retracted, complete) = finish_members(&prior, current, "source");
            assert!(!complete);
            assert!(retracted.is_empty());
            assert_eq!(checkpoint["scripts"]["s1"]["txids"], json!(["a", "b"]));
            assert_eq!(checkpoint["scripts"]["s2"]["txids"], json!(["c"]));
            assert_eq!(checkpoint["scripts"]["s1"]["complete"], false);
        }
    }
    #[test]
    fn esplora_partial_positive_then_complete_absence_preserves_its_retraction_baseline() {
        let prior = members(&[("s1", &["a"], true), ("s2", &["b"], true)]);
        let (partial, retracted, complete) =
            finish_members(&prior, members(&[("s1", &["c"], false)]), "source");
        assert!(!complete);
        assert!(retracted.is_empty());
        assert_eq!(partial["scripts"]["s1"]["txids"], json!(["a", "c"]));
        let saved: BTreeMap<String, Value> =
            serde_json::from_value(partial["scripts"].clone()).unwrap();
        let (_, retracted, complete) = finish_members(
            &saved,
            members(&[("s1", &[], true), ("s2", &["b"], true)]),
            "source",
        );
        assert!(complete);
        assert_eq!(retracted, ["a", "c"]);
    }
}

#[cfg(test)]
mod empty_checkpoint_tests {
    use super::*;
    #[test]
    fn empty_rounds_still_write_the_python_highest_used_mapping() {
        for absent in [
            json!({}),
            json!({"highest_used":null}),
            json!({"highest_used":false}),
            json!({"highest_used":[]}),
        ] {
            assert_eq!(checkpoint(&absent), json!({"highest_used":{}}));
        }
        let prior = json!({"highest_used":{"0":4},"saved":true});
        assert_eq!(checkpoint(&prior), prior);
    }
}

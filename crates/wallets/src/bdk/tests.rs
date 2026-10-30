use super::*;
use bitcoin::{absolute, transaction, Amount, OutPoint, ScriptBuf, TxIn, TxOut};
use kassiber_net::{Ledger, Offline};
use std::time::Duration;

fn scope() -> Scope {
    Scope {
        database_instance: "db".into(),
        profile_id: "p".into(),
        wallet_id: "w".into(),
        environment_id: "main".into(),
        source_revision: "1".into(),
    }
}

#[test]
fn bdk_record_projection_preserves_evidence_and_fences_reuse() {
    let a = action(None);
    let mut fake = Fake::new(&a, 2, true, false);
    let mut p = a.execute(&mut fake, 100).unwrap();
    for id in &p.canonical_txids {
        p.aggregate
            .tx_graph
            .first_seen
            .insert(id.parse().unwrap(), 100);
    }
    let records = p.records("fixture", "1:fixture", None, None).unwrap();
    assert_eq!(records.records.len(), 1);
    let record = &records.records[0];
    assert_eq!(record["amount"], "0.00099001");
    assert_eq!(record["fee"], "0");
    assert_eq!(record["description"], "Synced from fixture");
    let raw: Value = serde_json::from_str(record["raw_json"].as_str().unwrap()).unwrap();
    assert_eq!(raw["observer"], "bdk");
    assert_eq!(raw["observed_at"], 100);
    assert_eq!(raw["vin"][0]["prevout"]["value"], 100_003);
    assert_eq!(raw["vin"][0]["prevout"]["scriptpubkey"], "51");
    assert_eq!(raw["vout"][0]["value"], 99_001);
    assert_eq!(raw["confirmation_status"], "confirmed");
    assert_eq!(raw["confirmations"], 3);
    assert_eq!(
        raw["status"]["block_hash"],
        fake.header.block_hash().to_string()
    );
    assert_eq!(raw["status"]["block_time"], fake.header.time);
    assert_eq!(raw["block_height"], 10);
    assert_eq!(records.reused_records, 0);
    let prior = json!({"record_fingerprint_version":"1:fixture","record_fingerprints":records.fingerprints});
    let current = Some(p.canonical_txids.clone());
    // A full scan never reuses even an otherwise matching stored row.
    assert_eq!(
        p.records("fixture", "1:fixture", Some(&prior), current.clone())
            .unwrap()
            .records
            .len(),
        1
    );
    p.full_scan = false;
    let reused = p
        .records("fixture", "1:fixture", Some(&prior), current.clone())
        .unwrap();
    assert!(reused.records.is_empty());
    assert_eq!(reused.reused_records, 1);
    assert_eq!(reused.fingerprints, records.fingerprints);
    for (name, version, previous, current) in [
        ("changed", "1:fixture", Some(&prior), current.clone()),
        ("fixture", "2:fixture", Some(&prior), current),
        (
            "fixture",
            "1:fixture",
            None,
            Some(p.canonical_txids.clone()),
        ),
        ("fixture", "1:fixture", Some(&prior), None),
    ] {
        assert_eq!(
            p.records(name, version, previous, current)
                .unwrap()
                .records
                .len(),
            1
        );
    }
    let a = action(None);
    let mut p = a.execute(&mut Fake::new(&a, 1, false, false), 200).unwrap();
    let mempool = p.records("fixture", "1:fixture", None, None).unwrap();
    let raw: Value =
        serde_json::from_str(mempool.records[0]["raw_json"].as_str().unwrap()).unwrap();
    assert_eq!(raw["status"], json!({"confirmed":false}));
    assert_eq!(raw["confirmation_status"], "mempool");
    assert_eq!(raw["confirmations"], 0);
    assert!(raw["block_height"].is_null());
    assert_eq!(raw["observed_at"], 200);
    let prior = json!({"record_fingerprint_version":"1:fixture","record_fingerprints":mempool.fingerprints});
    p.full_scan = false;
    let unreused = p
        .records(
            "fixture",
            "1:fixture",
            Some(&prior),
            Some(p.canonical_txids.clone()),
        )
        .unwrap();
    assert_eq!(unreused.records.len(), 1);
    assert_eq!(unreused.reused_records, 0);
    let p = a.execute(&mut Fake::new(&a, 1, true, true), 200).unwrap();
    let coinbase = p.records("fixture", "1:fixture", None, None).unwrap();
    let raw: Value =
        serde_json::from_str(coinbase.records[0]["raw_json"].as_str().unwrap()).unwrap();
    assert!(raw["vin"][0]["prevout"].is_null());
    assert_eq!(raw["vin"][0]["txid"], "0".repeat(64));
    assert_eq!(raw["vin"][0]["vout"], u32::MAX);
}

fn prepared_spend(missing_prevout: bool) -> Prepared {
    let a = action(None);
    let mut wallet = a.fresh_wallet().unwrap();
    wallet
        .reveal_addresses_to(KeychainKind::External, 0)
        .for_each(drop);
    let script = wallet
        .public_descriptor(KeychainKind::External)
        .at_derivation_index(0)
        .unwrap()
        .script_pubkey();
    let owned = OutPoint::new(Txid::from_byte_array([1; 32]), 0);
    let other = OutPoint::new(Txid::from_byte_array([2; 32]), 1);
    let mut tx = fixture_tx(script.clone(), false).1;
    tx.input[0].previous_output = owned;
    tx.input[0].witness = bitcoin::Witness::from_slice(&[&[0xab, 0xcd][..]]);
    tx.input.push(TxIn {
        previous_output: other,
        ..Default::default()
    });
    let mut update = TxUpdate::<ConfirmationBlockTime>::default();
    update.txs.push(Arc::new(tx.clone()));
    update.txouts.insert(
        owned,
        TxOut {
            value: Amount::from_sat(100_000),
            script_pubkey: script.clone(),
        },
    );
    if !missing_prevout {
        update.txouts.insert(
            other,
            TxOut {
                value: Amount::from_sat(3_000),
                script_pubkey: script,
            },
        );
    }
    update.seen_ats.insert((tx.compute_txid(), 100));
    wallet
        .apply_update(Update {
            tx_update: update,
            ..Default::default()
        })
        .unwrap();
    let aggregate = wallet.take_staged().unwrap();
    Prepared {
        wallet,
        aggregate,
        canonical_txids: BTreeSet::from([tx.compute_txid().to_string()]),
        retracted_external_ids: BTreeSet::new(),
        rebuilt: false,
        full_scan: true,
    }
}

#[test]
fn bdk_owned_spend_requires_proven_fee_and_preserves_exact_witness_graph() {
    let p = prepared_spend(false);
    let record = &p
        .records("fixture", "1:fixture", None, None)
        .unwrap()
        .records[0];
    assert_eq!(record["direction"], "outbound");
    assert_eq!(record["amount"], "0");
    assert_eq!(record["fee"], "0.00003999");
    let raw: Value = serde_json::from_str(record["raw_json"].as_str().unwrap()).unwrap();
    assert_eq!(raw["vin"][0]["witness"], json!(["abcd"]));
    assert_eq!(raw["fee"], 3999);
    let e = prepared_spend(true)
        .records("fixture", "1:fixture", None, None)
        .err()
        .unwrap();
    assert_eq!(e.code, "observer_accounting_ambiguous");
    let details: Value = serde_json::from_str(
        &kassiber_pyjson::dumps(&e.details.unwrap(), &Default::default()).unwrap(),
    )
    .unwrap();
    assert_eq!(details["missing"], "network_fee");
}
fn config() -> Value {
    json!({"kind":"electrum","url":"tcp://127.0.0.1:1","batch_size":2})
}
fn descriptors() -> (String, String) {
    let v: Value = serde_json::from_str(include_str!(
        "../../tests/vectors/python_observer_state.json"
    ))
    .unwrap();
    (
        v["changesets"][0]["input"]["descriptor"]
            .as_str()
            .unwrap()
            .into(),
        v["changesets"][0]["input"]["change_descriptor"]
            .as_str()
            .unwrap()
            .into(),
    )
}
fn action(prior: Option<&Value>) -> Execution {
    let (a, b) = descriptors();
    Execution::new(scope(), &config(), &a, Some(&b), "main", 3, prior).unwrap()
}
fn fixture_tx(script: ScriptBuf, coinbase: bool) -> (Transaction, Transaction) {
    let parent = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: Amount::from_sat(100_003),
            script_pubkey: ScriptBuf::from_hex("51").unwrap(),
        }],
    };
    let current = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn {
            previous_output: if coinbase {
                OutPoint::null()
            } else {
                OutPoint::new(parent.compute_txid(), 0)
            },
            ..Default::default()
        }],
        output: vec![TxOut {
            value: Amount::from_sat(99_001),
            script_pubkey: script,
        }],
    };
    (parent, current)
}
struct Fake {
    history: BTreeMap<String, Value>,
    txs: BTreeMap<String, String>,
    calls: Vec<(String, Value)>,
    header: Header,
    malformed: bool,
    tip: u32,
    overrides: BTreeMap<String, Value>,
}
impl Fake {
    fn new(a: &Execution, index: u32, confirmed: bool, coinbase: bool) -> Self {
        let wallet = a.wallet().unwrap();
        let script = wallet
            .public_descriptor(KeychainKind::External)
            .at_derivation_index(index)
            .unwrap()
            .script_pubkey();
        let hash = crate::scan::script_hash(&script.to_hex_string()).unwrap();
        let (parent, tx) = fixture_tx(script, coinbase);
        let header = Header {
            merkle_root: bitcoin::TxMerkleNode::from_raw_hash(tx.compute_txid().to_raw_hash()),
            ..bitcoin::constants::genesis_block(bitcoin::Network::Bitcoin).header
        };
        Self {
            history: BTreeMap::from([(
                hash,
                json!([{"tx_hash":tx.compute_txid().to_string(),"height":if confirmed{10}else{0}}]),
            )]),
            txs: BTreeMap::from([
                (
                    tx.compute_txid().to_string(),
                    consensus::encode::serialize_hex(&tx),
                ),
                (
                    parent.compute_txid().to_string(),
                    consensus::encode::serialize_hex(&parent),
                ),
            ]),
            calls: vec![],
            header,
            malformed: false,
            tip: 12,
            overrides: BTreeMap::new(),
        }
    }
}
impl Rpc for Fake {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.calls.push((method.into(), params.clone()));
        if let Some(value) = self.overrides.get(method) {
            return Ok(value.clone());
        }
        match method {
            "blockchain.scripthash.get_history" => Ok(self
                .history
                .get(params[0].as_str().unwrap())
                .cloned()
                .unwrap_or(json!([]))),
            "blockchain.transaction.get" => {
                Ok(json!(self.txs.get(params[0].as_str().unwrap()).unwrap()))
            }
            "blockchain.transaction.get_merkle" => Ok(json!({"pos":0,"merkle":[]})),
            "blockchain.headers.subscribe" => Ok(json!({"height":self.tip})),
            "blockchain.block.headers" => Ok(
                json!({"count":8,"headers":vec![consensus::encode::serialize_hex(&self.header);8]}),
            ),
            "blockchain.block.header" => {
                let h = if params[0] == 0 {
                    bitcoin::constants::genesis_block(bitcoin::Network::Bitcoin).header
                } else {
                    self.header
                };
                Ok(json!(consensus::encode::serialize_hex(&h)))
            }
            _ => panic!("unexpected {method}"),
        }
    }
    fn batch(&mut self, method: &str, params: Vec<Value>) -> Result<Vec<Value>> {
        if self.malformed {
            self.calls.push((method.into(), json!("malformed")));
            if self.calls.iter().filter(|(m, _)| m == method).count() > 1 {
                return Err(invalid());
            }
            return Ok(vec![]);
        }
        params.into_iter().map(|p| self.call(method, p)).collect()
    }
}
#[test]
fn bdk_full_scan_and_restart_use_the_native_index_graph_and_merge() {
    let a = action(None);
    let mut fake = Fake::new(&a, 2, true, false);
    let p = a.execute(&mut fake, 100).unwrap();
    assert_eq!(p.wallet.transactions().count(), 1);
    assert_eq!(
        p.wallet.list_unspent().next().unwrap().txout.value.to_sat(),
        99_001
    );
    assert_eq!(p.wallet.derivation_index(KeychainKind::External), Some(5));
    assert_eq!(p.wallet.derivation_index(KeychainKind::Internal), Some(2));
    let state = p.changeset();
    assert_eq!(state["tx_graph"]["txs"].as_array().unwrap().len(), 1);
    assert_eq!(state["tx_graph"]["txouts"][0]["value_sats"], 100_003);
    assert_eq!(state["tx_graph"]["anchors"].as_array().unwrap().len(), 1);
    assert_eq!(p.wallet.latest_checkpoint().height(), 12);
    let reload = action(Some(&state));
    assert_eq!(reload.wallet().unwrap().list_unspent().count(), 1);
    let restarted = reload.execute(&mut fake, 101).unwrap();
    assert_eq!(restarted.changeset(), state);
    assert_eq!(restarted.wallet.transactions().count(), 1);
}

fn projection_branches() -> Vec<Branch> {
    vec![
        Branch {
            key: "receive".into(),
            index: 0,
            label: "receive".into(),
            internal: false,
        },
        Branch {
            key: "change".into(),
            index: 1,
            label: "change".into(),
            internal: true,
        },
    ]
}

#[test]
fn bdk_complete_update_orders_records_by_descending_chain_position() {
    let mut wallet = action(None).wallet().unwrap();
    let script = wallet
        .public_descriptor(KeychainKind::External)
        .at_derivation_index(0)
        .unwrap()
        .script_pubkey();
    let mut tx_update = TxUpdate::<ConfirmationBlockTime>::default();
    let mut ids = vec![];
    for index in 0..3 {
        let (mut parent, mut tx) = fixture_tx(script.clone(), false);
        parent.lock_time = absolute::LockTime::from_consensus(index);
        let point = OutPoint::new(parent.compute_txid(), 0);
        tx.input[0].previous_output = point;
        tx_update.txouts.insert(point, parent.output[0].clone());
        ids.push(tx.compute_txid().to_string());
        tx_update
            .seen_ats
            .insert((tx.compute_txid(), 100 + u64::from(index)));
        tx_update.txs.push(Arc::new(tx));
    }
    wallet
        .apply_update(Update {
            tx_update,
            ..Default::default()
        })
        .unwrap();
    let p = Prepared {
        aggregate: wallet.take_staged().unwrap(),
        wallet,
        canonical_txids: ids.iter().cloned().collect(),
        retracted_external_ids: BTreeSet::new(),
        rebuilt: false,
        full_scan: true,
    };
    let records = p
        .records("fixture", "1:fixture", None, None)
        .unwrap()
        .records;
    ids.reverse();
    assert_eq!(
        records
            .iter()
            .map(|r| r["txid"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        ids
    );
}

#[test]
fn bdk_complete_update_covers_outputs_policy_horizon_and_private_state() {
    let a = action(None);
    let mut fake = Fake::new(&a, 2, true, false);
    let p = a.execute(&mut fake, 100).unwrap();
    let u = p
        .update(
            &projection_branches(),
            "main",
            "fixture",
            "1:fixture",
            None,
            None,
        )
        .unwrap();
    assert_eq!(u["state"]["bdk_changeset"], p.changeset());
    assert_eq!(u["state"]["canonical_txids"], json!(p.canonical_txids));
    assert_eq!(u["state"]["schema_version"], 1);
    assert_eq!(u["state"]["record_fingerprint_version"], "1:fixture");
    assert_eq!(
        u["facts"]["transaction_records"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        u["state"]["record_fingerprints"].as_object().unwrap().len(),
        1
    );
    assert_eq!(
        u["facts"]["coverage"],
        json!([
        {"branch_key":"receive","scanned_to":6,"highest_used":2,"details":{"branch_index":0,"observer":"bdk"}},
        {"branch_key":"change","scanned_to":3,"highest_used":null,"details":{"branch_index":1,"observer":"bdk"}}])
    );
    let cp = &u["facts"]["freshness_checkpoint"];
    assert_eq!(
        *cp,
        json!({"observer":"bdk","tip_height":12,"tip_hash":p.wallet.latest_checkpoint().hash().to_string(),"highest_used":{"0":2},"canonical_txids":p.canonical_txids})
    );
    let outputs = u["facts"]["outputs"].as_array().unwrap();
    assert_eq!(outputs.len(), 1);
    let output = &outputs[0];
    let script = p.wallet.list_unspent().next().unwrap().txout.script_pubkey;
    assert_eq!(
        *output,
        json!({"txid":p.canonical_txids.iter().next().unwrap(),"vout":0,"asset":"BTC","amount_sats":99001,
        "confirmation_status":"confirmed","confirmations":3,"block_height":10,"block_time":fake.header.time,
        "chain":"bitcoin","network":"main","branch_index":0,"branch_label":"receive","address_index":2,
        "address":bitcoin::Address::from_script(&script,bitcoin::Network::Bitcoin).unwrap().to_string(),"script_pubkey":script.to_hex_string(),
        "derivation_path":null,"derivation_paths":[],"key_origins":[],"raw":{"source":"bdk_wallet","confirmed":true}})
    );
    let reload = action(Some(&p.changeset()));
    let restarted = reload.execute(&mut fake, 101).unwrap();
    let reused = restarted
        .update(
            &projection_branches(),
            "main",
            "fixture",
            "1:fixture",
            Some(&u["state"]),
            Some(p.canonical_txids.clone()),
        )
        .unwrap();
    assert_eq!(reused["facts"]["transaction_records"], json!([]));
    assert_eq!(reused["facts"]["reused_records"], 1);
    assert_eq!(reused["facts"]["outputs"], u["facts"]["outputs"]);
    assert_eq!(reused["facts"]["coverage"], u["facts"]["coverage"]);
}

#[test]
fn bdk_complete_update_keeps_mempool_unknown_height_and_clears_empty_inventory() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, false, false);
    let p = a.execute(&mut fake, 100).unwrap();
    let u = p
        .update(
            &projection_branches(),
            "main",
            "fixture",
            "1:fixture",
            None,
            None,
        )
        .unwrap();
    let output = &u["facts"]["outputs"][0];
    assert_eq!(output["confirmation_status"], "mempool");
    assert_eq!(output["confirmations"], 0);
    assert_eq!(output["block_height"], Value::Null);
    assert_eq!(output["block_time"], Value::Null);
    assert_eq!(output["raw"]["confirmed"], false);
    fake.history.clear();
    let empty = a
        .execute(&mut fake, 101)
        .unwrap()
        .update(
            &projection_branches(),
            "main",
            "fixture",
            "1:fixture",
            None,
            None,
        )
        .unwrap();
    assert_eq!(empty["facts"]["outputs"], json!([]));
    assert_eq!(
        empty["facts"]["freshness_checkpoint"]["highest_used"],
        json!({})
    );
    let mut branches = projection_branches();
    branches[0].internal = true;
    assert_eq!(
        p.update(&branches, "main", "fixture", "1:fixture", None, None)
            .unwrap_err()
            .code,
        "observer_capability_unsupported"
    );
}
#[test]
fn bdk_snapshot_precedes_histories_and_caches_transactions_and_anchors_per_request() {
    let a = action(None);
    let mut fake = Fake::new(&a, 2, true, false);
    let wallet = a.wallet().unwrap();
    let script = wallet
        .public_descriptor(KeychainKind::Internal)
        .at_derivation_index(1)
        .unwrap()
        .script_pubkey();
    fake.history.insert(
        crate::scan::script_hash(&script.to_hex_string()).unwrap(),
        fake.history.values().next().unwrap().clone(),
    );
    a.execute(&mut fake, 100).unwrap();
    assert_eq!(fake.calls[0].0, "blockchain.headers.subscribe");
    assert_eq!(fake.calls[1].0, "blockchain.block.headers");
    assert_eq!(
        fake.calls
            .iter()
            .filter(|(m, _)| m == "blockchain.transaction.get_merkle")
            .count(),
        1
    );
    assert_eq!(
        fake.calls
            .iter()
            .filter(|(m, _)| m == "blockchain.transaction.get")
            .count(),
        2
    );
}
#[test]
fn bdk_behind_tip_refuses_but_disconnected_tip_rebuilds_before_histories() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, true, false);
    let state = a.execute(&mut fake, 100).unwrap().changeset();
    let a = action(Some(&state));
    fake.calls.clear();
    fake.tip = 11;
    assert_eq!(
        a.execute(&mut fake, 101).err().unwrap().code,
        "backend_tip_behind"
    );
    assert_eq!(fake.calls.len(), 1);
    fake.calls.clear();
    fake.tip = 12;
    fake.header.time += 1;
    let rebuilt = a.execute(&mut fake, 101).unwrap();
    assert!(rebuilt.rebuilt());
    assert_eq!(rebuilt.canonical_txids().len(), 1);
    assert_eq!(
        rebuilt.changeset()["tx_graph"]["anchors"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_ne!(
        rebuilt.changeset()["tx_graph"]["anchors"],
        state["tx_graph"]["anchors"]
    );
    assert_eq!(fake.calls[1].0, "blockchain.block.header");
    assert_eq!(fake.calls[2].0, "blockchain.headers.subscribe");
    assert_eq!(fake.calls[3].0, "blockchain.block.headers");
}
#[test]
fn bdk_malformed_suffix_header_and_history_refuse_without_prepared_state() {
    let a = action(None);
    for (method, value) in [
        ("blockchain.headers.subscribe", json!({"height":-1})),
        ("blockchain.block.headers", json!({"count":0,"headers":[]})),
        (
            "blockchain.block.headers",
            json!({"count":7,"headers":vec!["00";8]}),
        ),
        (
            "blockchain.block.headers",
            json!({"count":8,"headers":vec!["00";8]}),
        ),
        ("blockchain.scripthash.get_history", json!({})),
        (
            "blockchain.scripthash.get_history",
            json!([{"tx_hash":"invalid","height":1}]),
        ),
    ] {
        let mut fake = Fake::new(&a, 0, true, false);
        fake.overrides.insert(method.into(), value);
        assert_eq!(
            a.execute(&mut fake, 100).err().unwrap().code,
            "protocol_error"
        );
    }
}
#[test]
fn bdk_preflight_refuses_unsupported_transport_secret_descriptors_and_zero_gap() {
    let (r, c) = descriptors();
    let mut backend = config();
    backend["kind"] = json!("esplora");
    assert_eq!(
        Execution::new(scope(), &backend, &r, Some(&c), "main", 3, None)
            .err()
            .unwrap()
            .code,
        "not_implemented"
    );
    assert!(Execution::new(
        scope(),
        &config(),
        "wpkh(private-key)",
        None,
        "main",
        3,
        None
    )
    .is_err());
    assert!(Execution::new(scope(), &config(), &r, Some(&c), "main", 0, None).is_err());
}
#[test]
fn bdk_proxy_failure_never_connects_directly_to_the_backend() {
    use kassiber_net::test_support::FakeElectrum;
    let server = FakeElectrum::script(vec![]).unwrap();
    let mut backend = config();
    backend["url"] = json!(server.url());
    backend["proxy"] = json!("socks5://127.0.0.1:1");
    let (r, c) = descriptors();
    let a = Execution::new(scope(), &backend, &r, Some(&c), "main", 3, None).unwrap();
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    assert!(a
        .run(
            &gate,
            &scope(),
            Some(&ExecutionConsent::explicit_trigger(&a)),
            Instant::now() + Duration::from_secs(2),
            &Cancellation::default(),
            100
        )
        .is_err());
    assert!(server.requests().is_empty());
}
#[test]
fn bdk_incremental_widens_until_a_full_unused_gap_and_fixed_stops() {
    let a = action(None);
    let mut empty = Fake::new(&a, 20, true, false);
    empty.history.clear();
    let state = a.execute(&mut empty, 100).unwrap().changeset();
    let a = action(Some(&state));
    let mut fake = Fake::new(&a, 2, false, false);
    fake.header = empty.header;
    let script = a
        .wallet()
        .unwrap()
        .public_descriptor(KeychainKind::External)
        .at_derivation_index(5)
        .unwrap()
        .script_pubkey();
    let (mut parent, mut tx) = fixture_tx(script.clone(), false);
    parent.lock_time = absolute::LockTime::from_consensus(1);
    tx.input[0].previous_output = OutPoint::new(parent.compute_txid(), 0);
    fake.txs.insert(
        parent.compute_txid().to_string(),
        consensus::encode::serialize_hex(&parent),
    );
    fake.history.insert(
        crate::scan::script_hash(&script.to_hex_string()).unwrap(),
        json!([{"tx_hash":tx.compute_txid().to_string(),"height":0}]),
    );
    fake.txs.insert(
        tx.compute_txid().to_string(),
        consensus::encode::serialize_hex(&tx),
    );
    let p = a.execute(&mut fake, 200).unwrap();
    assert_eq!(p.wallet.derivation_index(KeychainKind::External), Some(8));
    assert_eq!(p.wallet.transactions().count(), 2);
    assert_eq!(
        p.changeset()["tx_graph"]["first_seen"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(p.changeset()["tx_graph"]["first_seen"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["timestamp"] == 200));
    let fixed = "wpkh(0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798)";
    let mut a = Execution::new(scope(), &config(), fixed, None, "main", 3, None).unwrap();
    let mut fake = Fake::new(&a, 0, true, true);
    let state = a.execute(&mut fake, 100).unwrap().changeset();
    a = Execution::new(scope(), &config(), fixed, None, "main", 3, Some(&state)).unwrap();
    let p = a.execute(&mut fake, 101).unwrap();
    assert_eq!(p.wallet.derivation_index(KeychainKind::External), Some(0));
    assert_eq!(p.changeset(), state);
    assert!(p.changeset()["tx_graph"]["txouts"]
        .as_array()
        .unwrap()
        .is_empty());
}
#[test]
fn bdk_widening_refuses_after_exactly_sixty_four_unsettled_passes() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, false, false);
    fake.history.clear();
    let state = a.execute(&mut fake, 100).unwrap().changeset();
    let a = action(Some(&state));
    let wallet = a.wallet().unwrap();
    for key in [KeychainKind::External, KeychainKind::Internal] {
        for index in 0..=200 {
            let script = wallet
                .public_descriptor(key)
                .at_derivation_index(index)
                .unwrap()
                .script_pubkey();
            let (mut parent, mut tx) = fixture_tx(script.clone(), false);
            parent.lock_time = absolute::LockTime::from_consensus(
                index * 2 + u32::from(key == KeychainKind::Internal) + 1,
            );
            tx.input[0].previous_output = OutPoint::new(parent.compute_txid(), 0);
            fake.history.insert(
                crate::scan::script_hash(&script.to_hex_string()).unwrap(),
                json!([{"tx_hash":tx.compute_txid().to_string(),"height":0}]),
            );
            for tx in [parent, tx] {
                fake.txs.insert(
                    tx.compute_txid().to_string(),
                    consensus::encode::serialize_hex(&tx),
                );
            }
        }
    }
    fake.calls.clear();
    assert_eq!(
        a.execute(&mut fake, 101).err().unwrap().code,
        "observer_scan_limit"
    );
    assert_eq!(
        fake.calls
            .iter()
            .filter(|(m, _)| m == "blockchain.headers.subscribe")
            .count(),
        65 // One durable-height preflight, then 64 BDK snapshots.
    );
}
#[test]
fn bdk_eviction_and_reappearance_merge_times_without_erasing_observed_transactions() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, false, false);
    let history = fake.history.clone();
    let first = a.execute(&mut fake, 100).unwrap().changeset();
    let a = action(Some(&first));
    fake.history.clear();
    let absent = a.execute(&mut fake, 101).unwrap();
    assert_eq!(absent.wallet.transactions().count(), 0);
    let state = absent.changeset();
    assert_eq!(state["tx_graph"]["txs"], first["tx_graph"]["txs"]);
    assert_eq!(
        state["tx_graph"]["first_seen"],
        first["tx_graph"]["first_seen"]
    );
    assert_eq!(
        state["tx_graph"]["last_seen"],
        first["tx_graph"]["last_seen"]
    );
    assert_eq!(state["tx_graph"]["last_evicted"][0]["timestamp"], 101);
    fake.history = history;
    let a = action(Some(&state));
    let present = a.execute(&mut fake, 102).unwrap();
    assert_eq!(present.wallet.transactions().count(), 1);
    assert_eq!(
        present.changeset()["tx_graph"]["first_seen"],
        first["tx_graph"]["first_seen"]
    );
    assert_eq!(
        present.changeset()["tx_graph"]["last_seen"][0]["timestamp"],
        102
    );
    assert_eq!(
        present.changeset()["tx_graph"]["last_evicted"][0]["timestamp"],
        101
    );
}
#[test]
fn bdk_bad_protocol_identity_and_merkle_do_not_establish_confirmation() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, true, false);
    let first = fake.history.values().next().unwrap()[0]["tx_hash"]
        .as_str()
        .unwrap()
        .to_owned();
    let other = fake
        .txs
        .values()
        .find(|raw| {
            let tx: Transaction = consensus::deserialize(&Vec::from_hex(raw).unwrap()).unwrap();
            tx.compute_txid().to_string() != first
        })
        .unwrap()
        .clone();
    fake.txs.insert(first, other);
    assert_eq!(
        a.execute(&mut fake, 100).err().unwrap().code,
        "protocol_error"
    );
    let mut fake = Fake::new(&a, 0, true, false);
    fake.header.merkle_root = bitcoin::TxMerkleNode::all_zeros();
    let p = a.execute(&mut fake, 100).unwrap();
    assert!(p.changeset()["tx_graph"]["anchors"]
        .as_array()
        .unwrap()
        .is_empty());
    let mut fake = Fake::new(&a, 0, true, false);
    fake.malformed = true;
    assert_eq!(
        a.execute(&mut fake, 100).err().unwrap().code,
        "protocol_error"
    );
    assert_eq!(
        fake.calls
            .iter()
            .filter(|(m, _)| m == "blockchain.scripthash.get_history")
            .count(),
        1
    );
}
#[test]
fn bdk_state_reconstruction_preserves_all_fields_and_refuses_source_changes() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, false, false);
    let p = a.execute(&mut fake, 100).unwrap();
    let mut state = p.changeset();
    let txid = state["tx_graph"]["first_seen"][0]["txid"].clone();
    state["tx_graph"]["last_evicted"] = json!([{"txid":txid,"timestamp":99}]);
    state["locked_outpoints"] = json!([{"txid":txid,"vout":0,"locked":true}]);
    assert_eq!(
        bdk_state::encode(&bdk_state::decode(&state).unwrap()),
        state
    );
    let (receive, change) = descriptors();
    assert!(Execution::new(
        scope(),
        &config(),
        &change,
        Some(&change),
        "main",
        3,
        Some(&state)
    )
    .is_err());
    assert!(Execution::new(
        scope(),
        &config(),
        &change,
        Some(&receive),
        "main",
        3,
        Some(&state)
    )
    .is_err());
    assert!(Execution::new(scope(), &config(), &receive, None, "main", 3, Some(&state)).is_err());
    assert!(Execution::new(
        scope(),
        &config(),
        &receive,
        Some(&change),
        "regtest",
        3,
        Some(&state)
    )
    .is_err());
    state["schema_version"] = json!(2);
    assert!(Execution::new(
        scope(),
        &config(),
        &receive,
        Some(&change),
        "main",
        3,
        Some(&state)
    )
    .is_err());
}
#[test]
fn bdk_requires_exact_consent_and_online_even_for_loopback() {
    let a = action(None);
    let other = action(None);
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    let offline = Gate::new(|| Offline::Offline, Ledger::default());
    let d = Instant::now() + Duration::from_secs(3);
    let c = Cancellation::default();
    let foreign = ExecutionConsent::explicit_trigger(&other);
    for proof in [None, Some(&foreign)] {
        assert_eq!(
            a.run(&gate, &scope(), proof, d, &c, 100)
                .err()
                .unwrap()
                .code,
            "interaction_required"
        );
    }
    let own = ExecutionConsent::explicit_trigger(&a);
    assert_eq!(
        a.run(&offline, &scope(), Some(&own), d, &c, 100)
            .err()
            .unwrap()
            .code,
        "network_egress_disabled"
    );
    let mut wrong = scope();
    wrong.source_revision = "2".into();
    assert_eq!(
        a.run(&gate, &wrong, Some(&own), d, &c, 100)
            .err()
            .unwrap()
            .code,
        "stale_context"
    );
    own.revoke();
    assert!(a.run(&gate, &scope(), Some(&own), d, &c, 100).is_err());
    let own = ExecutionConsent::explicit_trigger(&a);
    own.started.store(true, Ordering::SeqCst);
    assert_eq!(
        a.run(&gate, &scope(), Some(&own), d, &c, 100)
            .err()
            .unwrap()
            .code,
        "interaction_required"
    );
    c.cancel();
    assert_eq!(
        a.run(&gate, &scope(), None, d, &c, 100).err().unwrap().code,
        "cancelled"
    );
    let mut backend = config();
    backend["proxy"] = json!(false);
    let (r, ch) = descriptors();
    assert!(Execution::new(scope(), &backend, &r, Some(&ch), "main", 3, None).is_err());
}

#[test]
fn bdk_native_codec_carries_every_aggregate_component_in_serialized_hash_order() {
    let vectors: Value = serde_json::from_str(include_str!(
        "../../tests/vectors/python_observer_state.json"
    ))
    .unwrap();
    let v = &vectors["changesets"][1]["expected"];
    assert_eq!(bdk_state::encode(&bdk_state::decode(v).unwrap()), *v);
}
#[test]
fn bdk_merkle_proofs_bind_txid_order_siblings_and_root() {
    let id = Txid::from_str(&format!("{:064x}", 12345)).unwrap();
    let other = Txid::from_str(&format!("{:064x}", 67890)).unwrap();
    let mut engine = sha256d::Hash::engine();
    engine.input(id.as_byte_array());
    engine.input(other.as_byte_array());
    let mut header = bitcoin::constants::genesis_block(bitcoin::Network::Bitcoin).header;
    header.merkle_root = bitcoin::TxMerkleNode::from_raw_hash(sha256d::Hash::from_engine(engine));
    assert!(merkle(id, header, &json!({"pos":0,"merkle":[other.to_string()]})).unwrap());
    assert!(merkle(other, header, &json!({"pos":1,"merkle":[id.to_string()]})).unwrap());
    assert!(!merkle(id, header, &json!({"pos":1,"merkle":[other.to_string()]})).unwrap());
    assert!(!merkle(id, header, &json!({"pos":0,"merkle":[id.to_string()]})).unwrap());
}

#[test]
fn bdk_offline_toggle_before_return_discards_prepared_state() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, true, false);
    let prepared = a.execute(&mut fake, 100).unwrap();
    assert_eq!(prepared.wallet.transactions().count(), 1);
    // Preparation succeeded online; the independent return fence must still
    // discard that complete private result after offline or cancellation.
    let offline = Gate::new(|| Offline::Offline, Ledger::default());
    let consent = ExecutionConsent::explicit_trigger(&a);
    let deadline = Instant::now() + Duration::from_secs(2);
    let cancel = Cancellation::default();
    assert_eq!(
        a.finish(&offline, &consent, deadline, &cancel, prepared)
            .err()
            .unwrap()
            .code,
        "network_egress_disabled"
    );
    let prepared = a.execute(&mut fake, 101).unwrap();
    cancel.cancel();
    let online = Gate::new(|| Offline::Online, Ledger::default());
    assert_eq!(
        a.finish(&online, &consent, deadline, &cancel, prepared)
            .err()
            .unwrap()
            .code,
        "cancelled"
    );
}

fn refresh_action(prior: Option<&Value>, refresh: Refresh) -> Execution {
    let (a, b) = descriptors();
    Execution::new_refresh(scope(), &config(), &a, Some(&b), "main", 3, prior, refresh).unwrap()
}
fn history_of(p: &Prepared) -> Refresh {
    Refresh {
        canonical_txids: p.canonical_txids().clone(),
        ..Default::default()
    }
}
#[test]
fn bdk_canonical_reorg_rebuild_replaces_stale_state_and_keeps_absence_baseline() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, true, false);
    let first = a.execute(&mut fake, 100).unwrap();
    assert_eq!(first.canonical_txids().len(), 1);
    let state = first.changeset();
    let a = refresh_action(Some(&state), history_of(&first));
    fake.header.time += 1;
    let reanchored = a.execute(&mut fake, 101).unwrap();
    assert!(reanchored.rebuilt());
    assert_eq!(reanchored.canonical_txids(), first.canonical_txids());
    assert!(reanchored.retracted_external_ids().is_empty());
    let history_calls = fake
        .calls
        .iter()
        .filter(|(m, _)| m == "blockchain.scripthash.get_history")
        .count();
    // Both scans are full scans; rebuilding never incrementally widens a fresh
    // wallet or accidentally scans the prior wallet's revealed horizon.
    assert_eq!(history_calls, 16);
    assert_eq!(
        reanchored.changeset(),
        action(None).execute(&mut fake, 101).unwrap().changeset()
    );
    // A second reorg drops the confirmed transaction. None of its old graph,
    // prevouts, anchors, timestamps or revealed horizon survives the rebuild.
    let state = reanchored.changeset();
    let a = refresh_action(Some(&state), history_of(&reanchored));
    fake.header.time += 1;
    fake.history.clear();
    let absent = a.execute(&mut fake, 102).unwrap();
    assert!(absent.rebuilt());
    assert!(absent.canonical_txids().is_empty());
    assert_eq!(absent.retracted_external_ids(), first.canonical_txids());
    assert_eq!(
        absent.changeset(),
        action(None).execute(&mut fake, 102).unwrap().changeset()
    );
    assert_eq!(
        absent.wallet.derivation_index(KeychainKind::External),
        Some(2)
    );
}
#[test]
fn bdk_retractions_use_canonical_membership_and_never_raw_graph_absence() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, false, false);
    let first = a.execute(&mut fake, 100).unwrap();
    let original_history = fake.history.clone();
    let state = first.changeset();
    let a = refresh_action(Some(&state), history_of(&first));
    fake.history.clear();
    let absent = a.execute(&mut fake, 101).unwrap();
    assert!(!absent.rebuilt());
    assert_eq!(absent.retracted_external_ids(), first.canonical_txids());
    assert!(absent.canonical_txids().is_empty());
    assert_eq!(
        absent.changeset()["tx_graph"]["txs"],
        state["tx_graph"]["txs"]
    );
    let state = absent.changeset();
    let a = refresh_action(Some(&state), history_of(&absent));
    let still_absent = a.execute(&mut fake, 102).unwrap();
    assert!(still_absent.retracted_external_ids().is_empty());
    fake.history = original_history;
    let state = still_absent.changeset();
    let a = refresh_action(Some(&state), history_of(&still_absent));
    let returned = a.execute(&mut fake, 103).unwrap();
    assert_eq!(returned.canonical_txids(), first.canonical_txids());
    assert!(returned.retracted_external_ids().is_empty());
    // A confirmed observation remains canonical when the current history omits
    // it but its block is still on the same chain, exactly as BDK decides.
    let a = action(None);
    let mut fake = Fake::new(&a, 0, true, false);
    let first = a.execute(&mut fake, 100).unwrap();
    fake.history.clear();
    let state = first.changeset();
    let a = refresh_action(Some(&state), history_of(&first));
    let retained = a.execute(&mut fake, 104).unwrap();
    assert_eq!(retained.canonical_txids(), first.canonical_txids());
    assert!(retained.retracted_external_ids().is_empty());
}
#[test]
fn bdk_canonical_replacement_withdraws_only_the_previous_wallet_transaction() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, false, false);
    let first = a.execute(&mut fake, 100).unwrap();
    let old = first.canonical_txids().first().unwrap();
    let mut replacement: Transaction =
        consensus::deserialize(&Vec::from_hex(&fake.txs[old]).unwrap()).unwrap();
    replacement.output[0].value = Amount::from_sat(98_000);
    let id = replacement.compute_txid().to_string();
    fake.txs
        .insert(id.clone(), consensus::encode::serialize_hex(&replacement));
    *fake.history.values_mut().next().unwrap() = json!([{"tx_hash":id,"height":0}]);
    let state = first.changeset();
    let a = refresh_action(Some(&state), history_of(&first));
    let replaced = a.execute(&mut fake, 101).unwrap();
    assert_eq!(replaced.canonical_txids(), &BTreeSet::from([id]));
    assert_eq!(replaced.retracted_external_ids(), first.canonical_txids());
    assert_eq!(
        replaced.changeset()["tx_graph"]["txs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn bdk_forced_rebuild_ignores_invalid_state_but_fences_checkpoint_height_and_history() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, false, false);
    let first = a.execute(&mut fake, 100).unwrap();
    let history = Refresh {
        force_full: true,
        checkpoint_tip: Some(12),
        ..history_of(&first)
    };
    let a = refresh_action(Some(&json!({"invalid":"old-version"})), history);
    fake.tip = 11;
    fake.calls.clear();
    let error = a.execute(&mut fake, 101).err().unwrap();
    assert_eq!(error.code, "backend_tip_behind");
    assert!(error.retryable);
    assert_eq!(
        error.details,
        Some(
            kassiber_pytext::loads(
                r#"{"observer":"bdk","backend_height":11,"persisted_height":12}"#
            )
            .unwrap()
        )
    );
    assert_eq!(
        error.hint.as_deref(),
        Some(
            "Wait for the backend to catch up or select a fully synchronized backend, then retry."
        )
    );
    assert_eq!(fake.calls.len(), 1);
    fake.tip = 12;
    fake.history.clear();
    let absent = a.execute(&mut fake, 102).unwrap();
    assert!(absent.canonical_txids().is_empty());
    assert_eq!(absent.retracted_external_ids(), first.canonical_txids());
    assert_eq!(
        absent.changeset(),
        action(None).execute(&mut fake, 102).unwrap().changeset()
    );
    // Force-full without a checkpoint still rebuilds, but has no borrowed
    // source/height baseline and performs no remote-hash preflight.
    let a = refresh_action(
        Some(&first.changeset()),
        Refresh {
            force_full: true,
            ..Default::default()
        },
    );
    fake.calls.clear();
    let absent = a.execute(&mut fake, 103).unwrap();
    assert!(absent.retracted_external_ids().is_empty());
    assert_eq!(fake.calls[1].0, "blockchain.block.headers");
}
#[test]
fn bdk_chain_snapshot_recovers_agreement_and_preserves_a_later_behind_tip() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, true, false);
    let genesis = CheckPoint::new(BlockId {
        height: 0,
        hash: bitcoin::constants::genesis_block(bitcoin::Network::Bitcoin).block_hash(),
    });
    let mut old = fake.header;
    old.time += 1;
    let prior = genesis
        .clone()
        .extend([2, 10, 12].map(|height| BlockId {
            height,
            hash: old.block_hash(),
        }))
        .unwrap();
    let mut protocol = Protocol {
        rpc: &mut fake,
        txs: BTreeMap::new(),
        headers: BTreeMap::from([(2, old), (0, old)]),
        anchors: BTreeMap::new(),
    };
    let recovered = protocol.chain_snapshot(prior.clone()).unwrap();
    assert_eq!(recovered.height(), 12);
    assert_eq!(recovered.hash(), protocol.rpc.header.block_hash());
    assert_eq!(
        recovered.get(2).unwrap().hash(),
        protocol.rpc.header.block_hash()
    );
    assert_eq!(recovered.get(0).unwrap().hash(), genesis.hash());
    assert!(protocol
        .rpc
        .calls
        .iter()
        .any(|(m, p)| m == "blockchain.block.header" && p[0] == 2));
    protocol.rpc.tip = 11;
    protocol.rpc.calls.clear();
    let retained = protocol.chain_snapshot(prior.clone()).unwrap();
    assert_eq!(retained, prior);
    assert_eq!(protocol.rpc.calls.len(), 1);
    protocol.rpc.tip = 12;
    let wrong_genesis = CheckPoint::new(BlockId {
        height: 0,
        hash: old.block_hash(),
    });
    assert_eq!(
        protocol.chain_snapshot(wrong_genesis).err().unwrap().code,
        "protocol_error"
    );
}
#[test]
fn bdk_rebuild_paths_require_exact_consent_online_scope_and_return_fence() {
    for force_full in [false, true] {
        let a = action(None);
        let mut fake = Fake::new(&a, 0, true, false);
        let first = a.execute(&mut fake, 100).unwrap();
        let refresh = Refresh {
            force_full,
            checkpoint_tip: Some(12),
            ..history_of(&first)
        };
        let a = refresh_action(Some(&first.changeset()), refresh);
        let other = action(None);
        let gate = Gate::new(|| Offline::Online, Ledger::default());
        let offline = Gate::new(|| Offline::Offline, Ledger::default());
        let d = Instant::now() + Duration::from_secs(3);
        let c = Cancellation::default();
        for proof in [None, Some(ExecutionConsent::explicit_trigger(&other))].iter() {
            assert_eq!(
                a.run(&gate, &scope(), proof.as_ref(), d, &c, 101)
                    .err()
                    .unwrap()
                    .code,
                "interaction_required"
            );
        }
        let proof = ExecutionConsent::explicit_trigger(&a);
        assert_eq!(
            a.run(&offline, &scope(), Some(&proof), d, &c, 101)
                .err()
                .unwrap()
                .code,
            "network_egress_disabled"
        );
        let mut wrong = scope();
        wrong.source_revision = "2".into();
        assert_eq!(
            a.run(&gate, &wrong, Some(&proof), d, &c, 101)
                .err()
                .unwrap()
                .code,
            "stale_context"
        );
        fake.header.time += 1;
        let prepared = a.execute(&mut fake, 101).unwrap();
        assert_eq!(
            a.finish(&offline, &proof, d, &c, prepared)
                .err()
                .unwrap()
                .code,
            "network_egress_disabled"
        );
        let prepared = a.execute(&mut fake, 102).unwrap();
        c.cancel();
        assert_eq!(
            a.finish(&gate, &proof, d, &c, prepared).err().unwrap().code,
            "cancelled"
        );
    }
}

#[test]
fn bdk_preflight_does_not_seed_the_scan_header_cache() {
    let a = action(None);
    let mut fake = Fake::new(&a, 0, true, false);
    let mut protocol = Protocol {
        rpc: &mut fake,
        txs: BTreeMap::new(),
        headers: BTreeMap::new(),
        anchors: BTreeMap::new(),
    };
    let old = protocol.rpc.header;
    protocol.headers.insert(
        10,
        Header {
            time: old.time - 1,
            ..old
        },
    );
    assert_eq!(protocol.remote_block_hash(10).unwrap(), old.block_hash());
    assert!(protocol.headers.is_empty());
    protocol.rpc.header.time += 1;
    assert_eq!(protocol.header(10).unwrap(), protocol.rpc.header);
    assert_eq!(
        protocol
            .rpc
            .calls
            .iter()
            .filter(|(m, _)| m == "blockchain.block.header")
            .count(),
        2
    );
}

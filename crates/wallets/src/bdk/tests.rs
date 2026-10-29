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
fn bdk_behind_or_disconnected_tip_refuses_before_history_preparation() {
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
    assert_eq!(
        a.execute(&mut fake, 101).err().unwrap().code,
        "observer_state_rebuild_required"
    );
    assert_eq!(fake.calls.len(), 2);
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
        64
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

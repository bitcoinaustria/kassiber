use super::*;
use bitcoin::{
    absolute, consensus::serialize, transaction, Amount, OutPoint, ScriptBuf, Sequence, TxIn,
    TxOut, Witness,
};
use kassiber_net::{
    test_support::{FakeElectrum, FakeProvider},
    Ledger, Offline,
};
use std::time::Duration;

pub(super) fn scope() -> Scope {
    Scope {
        database_instance: "db".into(),
        profile_id: "p".into(),
        wallet_id: "w".into(),
        environment_id: "regtest".into(),
        source_revision: "1".into(),
    }
}
pub(super) fn targets() -> Vec<Value> {
    vec![json!({"chain":"bitcoin","network":"regtest","script_pubkey":"51"})]
}
pub(super) fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
pub(super) fn gate(state: Offline) -> Gate {
    Gate::new(move || state, Ledger::default())
}
fn action(kind: &str, url: &str) -> Acquisition {
    Acquisition::new(
        scope(),
        &json!({"kind":kind,"url":url,"batch_size":2}),
        &targets(),
        "fixture",
    )
    .unwrap()
}
pub(super) fn run(a: &Acquisition) -> Result<Value> {
    a.run(
        &gate(Offline::Online),
        &scope(),
        Some(&AcquisitionConsent::explicit_trigger(a)),
        deadline(),
        &Cancellation::default(),
        |_| {},
    )
}
fn txid(n: u64) -> String {
    format!("{n:064x}")
}
fn esplora_tx(n: u64) -> Value {
    json!({"txid":txid(n),"status":{"confirmed":true,"block_height":100,"block_time":1700000000},"vin":[],"vout":[]})
}
fn response(id: usize, result: Value) -> String {
    format!("{}\n", json!({"id":id,"result":result}))
}
fn transaction(witness: bool) -> (Transaction, Transaction) {
    let prev = Transaction {
        version: transaction::Version(2),
        lock_time: absolute::LockTime::ZERO,
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: Amount::from_sat(100_003),
            script_pubkey: ScriptBuf::from_hex("51").unwrap(),
        }],
    };
    let tx = Transaction {
        version: transaction::Version(2),
        lock_time: absolute::LockTime::from_consensus(42),
        input: vec![TxIn {
            previous_output: OutPoint::new(prev.compute_txid(), 0),
            script_sig: ScriptBuf::new(),
            sequence: Sequence(0xfffffffd),
            witness: if witness {
                Witness::from_slice(&[vec![0xab; 73], vec![0xcd; 33]])
            } else {
                Witness::default()
            },
        }],
        output: vec![TxOut {
            value: Amount::from_sat(99_001),
            script_pubkey: ScriptBuf::from_hex("51").unwrap(),
        }],
    };
    (prev, tx)
}
fn hex(tx: &Transaction) -> String {
    serialize(tx).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn acquisition_every_path_requires_current_scope_and_exact_single_use_consent() {
    for (kind, scheme) in [
        ("esplora", "http"),
        ("esplora", "https"),
        ("electrum", "tcp"),
        ("electrum", "ssl"),
    ] {
        for host in ["127.0.0.1:1", "never-resolve.invalid:50001"] {
            let a = action(kind, &format!("{scheme}://{host}"));
            let other = action(kind, &format!("{scheme}://{host}"));
            let grant = AcquisitionConsent::explicit_trigger(&a);
            let foreign = AcquisitionConsent::explicit_trigger(&other);
            let gate = gate(Offline::Online);
            for consent in [None, Some(&foreign)] {
                assert_eq!(
                    a.run(
                        &gate,
                        &scope(),
                        consent,
                        deadline(),
                        &Cancellation::default(),
                        |_| {}
                    )
                    .unwrap_err()
                    .code,
                    "interaction_required"
                );
            }
            for changed in [
                Scope {
                    database_instance: "other".into(),
                    ..scope()
                },
                Scope {
                    profile_id: "other".into(),
                    ..scope()
                },
                Scope {
                    wallet_id: "other".into(),
                    ..scope()
                },
                Scope {
                    environment_id: "other".into(),
                    ..scope()
                },
                Scope {
                    source_revision: "other".into(),
                    ..scope()
                },
            ] {
                assert_eq!(
                    a.run(
                        &gate,
                        &changed,
                        Some(&grant),
                        deadline(),
                        &Cancellation::default(),
                        |_| {}
                    )
                    .unwrap_err()
                    .code,
                    "stale_context"
                );
            }
            grant.revoke();
            assert_eq!(
                a.run(
                    &gate,
                    &scope(),
                    Some(&grant),
                    deadline(),
                    &Cancellation::default(),
                    |_| {}
                )
                .unwrap_err()
                .code,
                "interaction_required"
            );
            assert!(gate.attempts().records.is_empty());
            assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
        }
    }
}

#[test]
fn acquisition_offline_cancellation_and_deadlines_precede_all_protocol_paths() {
    for (kind, scheme) in [("esplora", "http"), ("electrum", "tcp")] {
        for empty in [false, true] {
            let target_list = if empty { vec![] } else { targets() };
            let a = Acquisition::new(
                scope(),
                &json!({"kind":kind,"url":format!("{scheme}://127.0.0.1:1")}),
                &target_list,
                "fixture",
            )
            .unwrap();
            for state in [Offline::Offline, Offline::Unavailable] {
                let g = gate(state);
                assert_eq!(
                    a.run(
                        &g,
                        &scope(),
                        Some(&AcquisitionConsent::explicit_trigger(&a)),
                        deadline(),
                        &Cancellation::default(),
                        |_| {}
                    )
                    .unwrap_err()
                    .code,
                    "network_egress_disabled"
                );
                assert!(g.attempts().records.is_empty());
            }
            let g = gate(Offline::Online);
            let c = Cancellation::default();
            c.cancel();
            assert_eq!(
                a.run(
                    &g,
                    &scope(),
                    Some(&AcquisitionConsent::explicit_trigger(&a)),
                    deadline(),
                    &c,
                    |_| {}
                )
                .unwrap_err()
                .code,
                "cancelled"
            );
            assert_eq!(
                a.run(
                    &g,
                    &scope(),
                    Some(&AcquisitionConsent::explicit_trigger(&a)),
                    Instant::now(),
                    &Cancellation::default(),
                    |_| {}
                )
                .unwrap_err()
                .code,
                "backend_timeout"
            );
            assert!(g.attempts().records.is_empty());
        }
    }
}

#[test]
fn acquisition_esplora_paginates_deduplicates_and_retains_both_statistics() {
    let page: Vec<_> = (1..=25).map(esplora_tx).collect();
    let server = FakeProvider::replies(vec![
        (200, "{\"before\":true}".into()),
        (200, json!(page).to_string()),
        (200, json!([esplora_tx(26)]).to_string()),
        (
            200,
            json!([esplora_tx(1),{"txid":txid(27),"status":{"confirmed":false}}]).to_string(),
        ),
        (200, "{\"after\":true}".into()),
        (200, "[]".into()),
        (200, "102".into()),
    ])
    .unwrap();
    let result = run(&action("esplora", server.base())).unwrap();
    assert_eq!(result["transactions"].as_object().unwrap().len(), 27);
    assert_eq!(
        result["scripts"][0]["history"].as_array().unwrap().len(),
        27
    );
    assert_eq!(result["scripts"][0]["before"], json!({"before":true}));
    assert_eq!(result["scripts"][0]["after"], json!({"after":true}));
    assert_eq!(result["positions"][0]["confirmations"], 3);
    assert_eq!(result["positions"][26]["block_height"], Value::Null);
    assert_eq!(result["positions"][26]["confirmations"], Value::Null);
    assert!(server.requests()[2].contains(&format!("/txs/chain/{} ", txid(25))));
    assert!(server.requests()[3].contains("/txs/mempool "));
    assert!(server
        .requests()
        .iter()
        .all(|r| r.contains("User-Agent: kassiber/fixture\r\n")));
}

#[test]
fn acquisition_page_limits_still_fetch_mempool_and_unknown_tip_never_becomes_zero() {
    let server = FakeProvider::replies(vec![
        (200, "{}".into()),
        (
            200,
            json!((1..=25).map(esplora_tx).collect::<Vec<_>>()).to_string(),
        ),
        (200, json!([{"txid":txid(26)}]).to_string()),
        (200, "{}".into()),
        (200, "[]".into()),
        (200, "unknown".into()),
    ])
    .unwrap();
    let a = Acquisition::new(
        scope(),
        &json!({"kind":"esplora","url":server.base(),"maxpages":1}),
        &targets(),
        "fixture",
    )
    .unwrap();
    let result = run(&a).unwrap();
    assert_eq!(
        result["scripts"][0]["history"].as_array().unwrap().len(),
        26
    );
    assert_eq!(result["tip_height"], Value::Null);
    assert_eq!(result["positions"][0]["confirmations"], Value::Null);
    assert!(server.requests()[2].contains("/txs/mempool "));
}

#[test]
fn acquisition_repeated_cursors_and_invalid_txids_refuse_without_partial_results() {
    let page = json!((1..=25).map(esplora_tx).collect::<Vec<_>>()).to_string();
    let server =
        FakeProvider::replies(vec![(200, "{}".into()), (200, page.clone()), (200, page)]).unwrap();
    assert_eq!(
        run(&action("esplora", server.base())).unwrap_err().code,
        "protocol_error"
    );
    assert_eq!(server.requests().len(), 3);
    let server = FakeProvider::replies(vec![
        (200, "{}".into()),
        (200, "[{\"txid\":\"../private-material\"}]".into()),
    ])
    .unwrap();
    let error = run(&action("esplora", server.base())).unwrap_err();
    assert_eq!(error.code, "protocol_error");
    assert!(!error.message.contains("private-material"));
}

#[test]
fn acquisition_electrum_fetches_prevouts_headers_and_exact_witness_graph() {
    let (prev, tx) = transaction(true);
    let mut header = [0u8; 80];
    header[68..72].copy_from_slice(&1700000000u32.to_le_bytes());
    let header: String = header.iter().map(|b| format!("{b:02x}")).collect();
    let server = FakeElectrum::script(vec![
        (1,response(1,json!(["fixture","1.6"]))),(1,response(2,json!("status"))),
        (1,response(3,json!([{"tx_hash":tx.compute_txid().to_string(),"height":100}]))),
        (1,response(4,json!([{"tx_hash":tx.compute_txid().to_string(),"height":100,"tx_pos":0,"value":99001}]))),
        (1,response(5,json!(hex(&tx)))),(1,response(6,json!(hex(&prev)))),
        (1,response(7,json!(header))),(1,response(8,json!({"height":102}))),
    ]).unwrap();
    let result = run(&action("electrum", server.url())).unwrap();
    let graph = &result["transactions"][tx.compute_txid().to_string()];
    assert_eq!(
        graph["vin"][0]["prevout"],
        json!({"scriptpubkey":"51","value":100003})
    );
    assert_eq!(graph["vout"][0]["value"], 99001);
    assert_eq!(graph["vout"][0]["scriptpubkey"], "51");
    assert_eq!(graph["vin"][0]["witness"][0], "ab".repeat(73));
    assert_eq!(graph["locktime"], 42);
    assert_eq!(graph["size"], serialize(&tx).len());
    assert_eq!(graph["vsize"], tx.vsize());
    assert_eq!(graph["weight"], tx.weight().to_wu());
    assert_eq!(
        graph["_kassiber_electrum_graph"],
        json!({"kind":"bitcoin_electrum","version":2})
    );
    assert_eq!(result["headers"]["100"], 1700000000);
    assert_eq!(result["positions"][0]["confirmations"], 3);
    assert_eq!(server.requests().len(), 8);
}

#[test]
fn acquisition_decoding_and_positions_preserve_exact_amounts_and_unknown_heights() {
    let (_, mut tx) = transaction(false);
    tx.output[0].value = Amount::from_sat(2_100_000_000_000_001);
    let decoded = decode_transaction(&tx, &hex(&tx)).unwrap();
    assert_eq!(decoded["vout"][0]["value_sats"], 2_100_000_000_000_001u64);
    assert_eq!(decoded["total_output_sats"], 2_100_000_000_000_001u64);
    assert!(decoded["vin"][0].get("witness").is_none());
    for height in [json!(0), json!(-1), json!(null), json!("garbage")] {
        assert_eq!(positive_height(&height), None);
    }
    assert_eq!(positive_height(&json!("٤")), Some(4));
    assert_eq!(positive_height(&json!(3.9)), Some(3));
    assert_eq!(confirmations(Some(100), Some(102)), Some(3));
    for (height, tip) in [
        (Some(100), Some(99)),
        (None, Some(102)),
        (Some(100), None),
        (Some(0), Some(102)),
    ] {
        assert_eq!(confirmations(height, tip), None);
    }
    for header in ["00".repeat(71), "secret".into()] {
        assert_eq!(
            header_timestamp(&header).unwrap_err().code,
            "protocol_error"
        );
    }
    // Elements headers are longer; both keep the time at bytes 68..72.
    let mut elements = "00".repeat(68) + "00f15365" + &"00".repeat(40);
    assert_eq!(header_timestamp(&elements).unwrap(), 1_700_000_000);
    elements.truncate(144);
    assert_eq!(header_timestamp(&elements).unwrap(), 1_700_000_000);
}

#[test]
fn acquisition_cancellation_revocation_and_replay_return_no_partial_batch() {
    for revoke in [false, true] {
        let server = FakeProvider::replies(vec![
            (200, "{}".into()),
            (200, "[]".into()),
            (200, "[]".into()),
            (200, "{}".into()),
            (200, "[]".into()),
            (200, "100".into()),
        ])
        .unwrap();
        let a = action("esplora", server.base());
        let consent = AcquisitionConsent::explicit_trigger(&a);
        let cancel = Cancellation::default();
        let g = gate(Offline::Online);
        let error = a
            .run(&g, &scope(), Some(&consent), deadline(), &cancel, |_| {
                if revoke {
                    consent.revoke()
                } else {
                    cancel.cancel()
                }
            })
            .unwrap_err();
        assert_eq!(
            error.code,
            if revoke {
                "interaction_required"
            } else {
                "cancelled"
            }
        );
        assert_eq!(server.requests().len(), 5);
        assert_eq!(
            a.run(
                &g,
                &scope(),
                Some(&consent),
                deadline(),
                &Cancellation::default(),
                |_| {}
            )
            .unwrap_err()
            .code,
            "interaction_required"
        );
    }
}

#[test]
fn acquisition_proxy_failure_has_no_direct_fallback() {
    let http = FakeProvider::new("{}").unwrap();
    let tcp = FakeElectrum::script(vec![]).unwrap();
    for (kind, url) in [("esplora", http.base()), ("electrum", tcp.url())] {
        let a = Acquisition::new(
            scope(),
            &json!({"kind":kind,"url":url,"tor_proxy":"socks5h://127.0.0.1:1"}),
            &targets(),
            "fixture",
        )
        .unwrap();
        assert!(run(&a).is_err());
    }
    assert!(http.requests().is_empty());
    assert!(tcp.requests().is_empty());
}

#[test]
fn acquisition_invalid_proxy_types_refuse_instead_of_selecting_direct() {
    for kind in ["esplora", "electrum"] {
        let url = if kind == "esplora" {
            "http://127.0.0.1:1"
        } else {
            "tcp://127.0.0.1:1"
        };
        for raw in [json!(true), json!(false), json!(1), json!([]), json!({})] {
            for key in ["proxy", "tor_proxy"] {
                for nested in [false, true] {
                    let mut backend = json!({"kind":kind,"url":url});
                    if nested {
                        backend["config"] = json!({key:raw});
                    } else {
                        backend[key] = raw.clone();
                    }
                    let error = Acquisition::new(scope(), &backend, &targets(), "fixture")
                        .err()
                        .unwrap();
                    assert_eq!(error.code, "app_error");
                    assert_eq!(error.message, "Invalid proxy URL");
                }
            }
        }
    }
}

#[test]
fn acquisition_coinbase_never_fetches_or_attaches_the_null_outpoint() {
    let (tx, _) = transaction(false);
    let mut header = [0u8; 80];
    header[68..72].copy_from_slice(&1700000000u32.to_le_bytes());
    let header: String = header.iter().map(|b| format!("{b:02x}")).collect();
    let server = FakeElectrum::script(vec![
        (1, response(1, json!(["fixture", "1.6"]))),
        (1, response(2, json!("status"))),
        (
            1,
            response(
                3,
                json!([{"tx_hash":tx.compute_txid().to_string(),"height":100}]),
            ),
        ),
        (1, response(4, json!([]))),
        (1, response(5, json!(hex(&tx)))),
        (1, response(6, json!(header))),
        (1, response(7, json!({"height":102}))),
    ])
    .unwrap();
    let result = run(&action("electrum", server.url())).unwrap();
    let graph = &result["transactions"][tx.compute_txid().to_string()];
    assert!(graph["vin"][0].get("prevout").is_none());
    assert_eq!(result["transactions"].as_object().unwrap().len(), 1);
    assert_eq!(server.requests().len(), 7);
    let mut ordinary = json!({"vin":[{"txid":"0".repeat(64),"vout":0}],"vout":[]});
    let mut lookup = Map::new();
    lookup.insert(
        "0".repeat(64),
        json!({"vout":[{"script_hex":"51","value_sats":7}]}),
    );
    normalize_graph(&mut ordinary, &lookup).unwrap();
    assert_eq!(ordinary["vin"][0]["prevout"]["value"], 7);
}

#[test]
fn acquisition_raw_transaction_identity_and_encoding_fail_closed() {
    let (prev, tx) = transaction(false);
    let own = hex(&tx);
    for (raw, message) in [
        (hex(&prev), "Electrum returned different transaction bytes"),
        (
            "private-invalid-raw".into(),
            "Electrum returned invalid transaction bytes",
        ),
        ("00".into(), "Electrum returned invalid transaction bytes"),
        // Trailing and truncated encodings never reach the txid comparison.
        (
            format!("{own}00"),
            "Electrum returned invalid transaction bytes",
        ),
        (
            own[..own.len() - 2].to_owned(),
            "Electrum returned invalid transaction bytes",
        ),
    ] {
        let server = FakeElectrum::script(vec![
            (1, response(1, json!(["fixture", "1.6"]))),
            (1, response(2, json!("status"))),
            (
                1,
                response(
                    3,
                    json!([{"tx_hash":tx.compute_txid().to_string(),"height":100}]),
                ),
            ),
            (1, response(4, json!([]))),
            (1, response(5, json!(raw))),
        ])
        .unwrap();
        let error = run(&action("electrum", server.url())).unwrap_err();
        assert_eq!(error.code, "invalid_observation");
        assert_eq!(error.message, message);
        assert_eq!(server.requests().len(), 5);
    }
}

#[test]
fn acquisition_rechecks_offline_before_returning_observed_facts() {
    let server = FakeProvider::replies(vec![
        (200, "{}".into()),
        (200, "[]".into()),
        (200, "[]".into()),
        (200, "{}".into()),
        (200, "[]".into()),
        (200, "100".into()),
    ])
    .unwrap();
    let offline = Arc::new(AtomicBool::new(false));
    let state = offline.clone();
    let gate = Gate::new(
        move || {
            if state.load(Ordering::SeqCst) {
                Offline::Offline
            } else {
                Offline::Online
            }
        },
        Ledger::default(),
    );
    let a = action("esplora", server.base());
    let error = a
        .run(
            &gate,
            &scope(),
            Some(&AcquisitionConsent::explicit_trigger(&a)),
            deadline(),
            &Cancellation::default(),
            |_| offline.store(true, Ordering::SeqCst),
        )
        .unwrap_err();
    assert_eq!(error.code, "network_egress_disabled");
    assert_eq!(server.requests().len(), 5);
}

#[test]
fn acquisition_cached_graph_normalization_keeps_integer_sats_and_existing_prevouts() {
    for (value, expected) in [
        (json!(20_000.0), json!(20_000)),
        (json!("2.5"), json!(2)),
        (json!("3.5"), json!(4)),
        (json!("-0.0"), json!(0)),
    ] {
        let mut tx = json!({"vin":[{"txid":"a".repeat(64),"vout":0,
            "prevout":{"scriptpubkey":"52","value":21_000}}],
            "vout":[{"script_hex":"51","value_sats":value}]});
        normalize_graph(&mut tx, &Map::new()).unwrap();
        assert_eq!(tx["vout"][0]["value"], expected);
        assert!(tx["vout"][0]["value"]
            .as_number()
            .unwrap()
            .to_string()
            .bytes()
            .all(|b| b.is_ascii_digit()));
        assert_eq!(
            tx["vin"][0]["prevout"],
            json!({"scriptpubkey":"52","value":21_000})
        );
        let once = tx.clone();
        normalize_graph(&mut tx, &Map::new()).unwrap();
        assert_eq!(tx, once);
    }
}

#[test]
fn liquid_transactions_must_match_their_txid() {
    // Every Liquid reply, Electrum's or Esplora's `tx/<txid>/hex`, must be a
    // canonical encoding of the requested transaction.
    let vectors: Value =
        serde_json::from_str(include_str!("../../tests/vectors/lwk_chain.json")).unwrap();
    let phase = &vectors["phases"]["compat_initial"]["transactions"];
    let (txid, hex) = phase.as_object().unwrap().iter().next().unwrap();
    let hex = hex.as_str().unwrap();
    assert_eq!(
        liquid_transaction(hex, txid).unwrap().txid().to_string(),
        *txid
    );
    assert_eq!(
        liquid_transaction(hex, &"00".repeat(32)).unwrap_err().code,
        "invalid_observation"
    );
    assert_eq!(
        liquid_transaction(&format!("{hex}00"), txid)
            .unwrap_err()
            .code,
        "invalid_observation"
    );
    assert_eq!(
        liquid_transaction("zz", txid).unwrap_err().code,
        "invalid_observation"
    );
}

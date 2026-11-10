use super::*;
use crate::descriptors;
use kassiber_net::{Ledger, Offline};
use lwk_wollet::elements::hashes::Hash;
use serde_json::json;
use std::time::Duration;

const VECTORS: &str = include_str!("../../tests/vectors/lwk_chain.json");

fn vectors() -> Value {
    serde_json::from_str(VECTORS).unwrap()
}
fn txid(name: &str) -> String {
    vectors()["transactions"][name].as_str().unwrap().to_owned()
}

fn scope() -> Scope {
    Scope {
        database_instance: "db".into(),
        profile_id: "p".into(),
        wallet_id: "w".into(),
        environment_id: "regtest".into(),
        source_revision: "1".into(),
    }
}

fn config(descriptor: &str) -> Value {
    json!({"chain":"liquid","network":"elementsregtest","descriptor":descriptor,"gap_limit":3})
}

fn plan(config: &Value) -> Plan {
    descriptors::load(config)
        .ok()
        .flatten()
        .expect("descriptor plan")
}

/// The loopback fake's Electrum answers, served from memory.
struct Fake {
    phase: Value,
    hits: Vec<String>,
    fail: Option<(usize, AppError)>,
    substitute: bool,
}
impl Fake {
    fn new(phase: &str) -> Self {
        Self {
            phase: vectors()["phases"][phase].clone(),
            hits: vec![],
            fail: None,
            substitute: false,
        }
    }
}
impl Rpc for Fake {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        if let Some((at, error)) = &self.fail {
            if self.hits.len() == *at {
                return Err(error.clone());
            }
        }
        self.hits.push(method.to_owned());
        let p = &self.phase;
        Ok(match method {
            "blockchain.headers.subscribe" => {
                json!({"height":p["tip"],"hex":p["headers"][p["tip"].to_string()]})
            }
            "blockchain.block.header" => p["headers"][params[0].to_string()].clone(),
            "blockchain.scripthash.get_history" => p["history"]
                .get(params[0].as_str().unwrap())
                .cloned()
                .unwrap_or(json!([])),
            "blockchain.transaction.get" => {
                let transactions = p["transactions"].as_object().unwrap();
                if self.substitute {
                    // Answer every request with one fixed, unrelated transaction.
                    transactions.values().next().unwrap().clone()
                } else {
                    transactions[params[0].as_str().unwrap()].clone()
                }
            }
            other => panic!("unexpected {other}"),
        })
    }
}

fn action(phase_config: &Value, refresh: Refresh) -> Execution {
    Execution::new(
        scope(),
        &json!({"name":"fixture","kind":"electrum","url":"tcp://127.0.0.1:1"}),
        &plan(phase_config),
        "elementsregtest",
        vectors()["policy_asset"].as_str().unwrap(),
        vec!["receive".into(), "change".into()],
        refresh,
    )
    .unwrap()
}

fn fresh() -> Refresh {
    Refresh {
        force_full: false,
        prior: None,
        prior_coverage: vec![],
        checkpoint: json!({}),
        values: BTreeMap::new(),
    }
}

fn default_config() -> Value {
    config(vectors()["descriptor"].as_str().unwrap())
}

fn scan(phase: &str, refresh: Refresh) -> Prepared {
    execute_offline(&action(&default_config(), refresh), &mut Fake::new(phase)).unwrap()
}

fn after(first: &Prepared) -> Refresh {
    Refresh {
        force_full: false,
        prior: Some(first.update["state"].clone()),
        prior_coverage: first.update["facts"]["coverage"]
            .as_array()
            .unwrap()
            .clone(),
        checkpoint: first.update["facts"]["freshness_checkpoint"].clone(),
        values: first.values.clone(),
    }
}

fn records<'a>(update: &'a Value, name: &str) -> Vec<&'a Value> {
    let id = txid(name);
    update["facts"]["transaction_records"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["txid"] == id)
        .collect()
}
fn raw(record: &Value) -> Value {
    serde_json::from_str(record["raw_json"].as_str().unwrap()).unwrap()
}

#[test]
fn lwk_records_carry_exact_amounts_fees_assets_and_htlc_evidence() {
    let update = scan("initial", fresh()).update;
    let fund = records(&update, "fund");
    assert_eq!(fund.len(), 2);
    let issued = vectors()["issued_asset"].as_str().unwrap().to_owned();
    assert_eq!(fund[0]["asset"], "LBTC");
    // The explicit 50,000 to receive/1 is booked with the confidential receipt.
    assert_eq!(fund[0]["amount"], "0.0015");
    assert_eq!(fund[0]["fee"], "0");
    assert_eq!(fund[0]["kind"], "deposit");
    assert_eq!(fund[0]["direction"], "inbound");
    assert_eq!(fund[0]["amount_includes_fee"], false);
    assert_eq!(fund[1]["asset"], issued.as_str());
    assert_eq!(fund[1]["amount"], "0.00007");
    let fund_raw = raw(fund[0]);
    assert_eq!(fund_raw["component"]["transaction_fee_sats"], 300);
    assert_eq!(fund_raw["component"]["fee_sats"], 0);
    assert_eq!(fund_raw["observer"], "lwk");
    assert_eq!(fund_raw["chain"], "liquid");
    assert_eq!(fund_raw["network"], "elementsregtest");
    assert_eq!(fund_raw["ownership_graph_version"], 1);
    assert_eq!(fund_raw["vout"][4]["role"], "fee");
    assert_eq!(fund_raw["vout"][4]["value_sats"], 300);
    assert_eq!(fund_raw["vout"][1]["role"], "owned");
    assert!(fund_raw["vout"][3].get("role").is_none());
    assert!(fund[0]["raw_json"]
        .as_str()
        .unwrap()
        .starts_with("{\"chain\": \"liquid\", \"component\": {"));

    let spend = records(&update, "spend");
    assert_eq!(spend[0]["amount"], "0.0006");
    assert_eq!(spend[0]["fee"], "0.000005");
    assert_eq!(spend[0]["kind"], "withdrawal");
    assert_eq!(raw(spend[0])["component"]["net_sats"], -60_500);
    assert_eq!(raw(spend[0])["component"]["fee_attribution"], "exact");
    assert_eq!(raw(spend[0])["vin"][0]["prevout"]["value_sats"], 100_000);

    let asset_send = records(&update, "asset_send");
    assert_eq!(asset_send[0]["kind"], "fee");
    assert_eq!(asset_send[0]["amount"], "0");
    assert_eq!(asset_send[0]["fee"], "0.000002");
    assert_eq!(asset_send[1]["kind"], "withdrawal");
    assert_eq!(asset_send[1]["asset"], issued.as_str());
    assert_eq!(asset_send[1]["fee"], "0");

    // Spending the explicit output: -50,000 explicit, +29,000 change.
    let mixed = records(&update, "mixed");
    assert_eq!(mixed[0]["direction"], "outbound");
    assert_eq!(mixed[0]["amount"], "0.00021");
    assert_eq!(mixed[0]["amount_includes_fee"], true);
    assert_eq!(raw(mixed[0])["component"]["net_sats"], -21_000);
    assert_eq!(raw(mixed[0])["vin"][0]["prevout"]["role"], "owned");
    assert!(raw(mixed[0])["vin"][1].get("prevout").is_none());

    let claim = records(&update, "claim");
    assert_eq!(
        claim[0]["payment_hash_source"],
        "chain_script_unique_outpoint"
    );
    assert_eq!(
        claim[0]["payment_hash"],
        raw(claim[0])["htlc_spend"]["payment_hash"]
    );
    assert_eq!(raw(claim[0])["htlc_spend"]["role"], "claim");
    assert_eq!(raw(claim[0])["htlc_spend"]["funding_vout"], 0);
    assert!(claim[0].get("swap_refund_funding_txid").is_none());
    let refund = records(&update, "refund");
    assert_eq!(refund[0]["swap_refund_funding_vout"], 1);
    assert_eq!(
        refund[0]["swap_refund_funding_txid"],
        raw(refund[0])["htlc_spend"]["funding_txid"]
    );
    assert_eq!(raw(refund[0])["htlc_spend"]["role"], "refund");
    assert!(refund[0].get("payment_hash").is_none());
    assert!(raw(spend[0]).get("htlc_spend").is_none());

    let mempool = records(&update, "mempool");
    assert_eq!(mempool[0]["confirmed_at"], Value::Null);
    assert_eq!(mempool[0]["occurred_at"], facts::UNKNOWN_OCCURRED_AT);
    assert_eq!(spend[0]["occurred_at"], "2023-11-14T22:16:20Z");
    assert_eq!(spend[0]["confirmed_at"], spend[0]["occurred_at"]);
    assert!(records(&update, "wrong_key").is_empty());
    assert_eq!(spend[0]["description"], "Synced from fixture");
}

#[test]
fn lwk_mixed_input_spend_keeps_the_fee_inside_the_exact_delta() {
    assert_eq!(facts::allocated_fee(-100, 30, true, true), (0, true));
    assert_eq!(facts::allocated_fee(-100, 30, true, false), (30, false));
    assert_eq!(facts::allocated_fee(-100, 30, false, false), (0, false));
    assert_eq!(facts::allocated_fee(100, 30, true, false), (0, false));
    assert_eq!(facts::allocated_fee(-100, 0, true, false), (0, false));
    assert_eq!(facts::allocated_fee(0, 30, true, false), (0, false));
}

#[test]
fn lwk_outputs_coverage_and_checkpoint_describe_the_scanned_wollet() {
    let update = scan("initial", fresh()).update;
    let outputs = update["facts"]["outputs"].as_array().unwrap();
    assert_eq!(outputs.len(), 4);
    let points: Vec<_> = outputs
        .iter()
        .map(|o| (o["txid"].as_str().unwrap(), o["vout"].as_u64().unwrap()))
        .collect();
    let mut sorted = points.clone();
    sorted.sort();
    assert_eq!(points, sorted);
    let mempool = outputs
        .iter()
        .find(|o| o["txid"] == txid("mempool"))
        .unwrap();
    assert_eq!(mempool["confirmation_status"], "mempool");
    assert_eq!(mempool["confirmations"], 0);
    assert_eq!(mempool["block_height"], Value::Null);
    assert_eq!(mempool["branch_label"], "change");
    assert_eq!(mempool["branch_index"], 1);
    assert_eq!(mempool["address_index"], 3);
    let refund = outputs
        .iter()
        .find(|o| o["txid"] == txid("refund"))
        .unwrap();
    assert_eq!(refund["confirmations"], 2);
    assert_eq!(refund["block_height"], 5);
    assert_eq!(refund["amount_sats"], 15_000);
    assert_eq!(refund["asset"], "LBTC");
    assert_eq!(refund["confirmation_status"], "confirmed");
    assert_eq!(
        refund["raw"],
        json!({"source":"lwk_wollet","confidential":true})
    );
    assert!(refund["address"].as_str().unwrap().starts_with("el1"));
    assert_eq!(
        update["facts"]["coverage"],
        json!([
            {"branch_key":"receive","scanned_to":4,"highest_used":3,"details":{"observer":"lwk"}},
            {"branch_key":"change","scanned_to":5,"highest_used":4,"details":{"observer":"lwk"}},
        ])
    );
    let checkpoint = &update["facts"]["freshness_checkpoint"];
    assert_eq!(checkpoint["observer"], "lwk");
    assert_eq!(checkpoint["tip_height"], 6);
    assert_eq!(checkpoint["highest_used"], json!({"0":3,"1":4}));
    assert_eq!(checkpoint["canonical_txids"].as_array().unwrap().len(), 7);
    assert_eq!(
        update["state"]["canonical_txids"],
        checkpoint["canonical_txids"]
    );
    assert_eq!(update["state"]["schema_version"], 1);
    assert!(checkpoint["canonical_txids"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t != &json!(txid("wrong_key"))));
}

#[test]
fn lwk_coverage_is_the_exclusive_request_bound_widened_only_by_discovery() {
    let keys = vec!["receive".to_owned(), "change".to_owned()];
    let highest = BTreeMap::from([("receive", 9u32)]);
    assert_eq!(
        facts::coverage(&keys, 4, &highest),
        vec![
            json!({"branch_key":"receive","scanned_to":10,"highest_used":9,"details":{"observer":"lwk"}}),
            json!({"branch_key":"change","scanned_to":5,"highest_used":null,"details":{"observer":"lwk"}}),
        ]
    );
    assert_eq!(
        facts::coverage(&keys, 4, &BTreeMap::from([("change", 2u32)]))[1]["scanned_to"],
        5
    );
}

#[test]
fn lwk_restart_reuses_the_store_and_a_reorg_retracts_only_the_dropped_spend() {
    let first = scan("initial", fresh());
    assert_eq!(first.values.keys().collect::<Vec<_>>(), ["000000000000"]);
    let again = scan("initial", after(&first));
    // The horizon now starts past every discovered index (4 + gap 3).
    let mut expected = first.update.clone();
    for point in expected["facts"]["coverage"].as_array_mut().unwrap() {
        point["scanned_to"] = json!(8);
    }
    assert_eq!(again.update, expected);
    // Nothing changed: LWK persists no further update.
    assert_eq!(again.values, first.values);
    let reorg = scan("reorg", after(&first));
    // With the mixed spend gone, the explicit output is held again.
    let explicit: Vec<_> = reorg.update["facts"]["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["txid"] == txid("fund") && o["vout"] == 1)
        .collect();
    assert_eq!(explicit.len(), 1);
    assert_eq!(explicit[0]["amount_sats"], 50_000);
    assert_eq!(explicit[0]["raw"]["confidential"], false);
    assert!(first.update["facts"]["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|o| o["raw"]["confidential"] == true));
    assert_eq!(
        reorg.update["facts"]["retracted_external_ids"],
        json!([txid("mixed")])
    );
    assert_eq!(
        reorg.values.keys().collect::<Vec<_>>(),
        ["000000000000", "000000000001"]
    );
    let mempool = records(&reorg.update, "mempool");
    assert_ne!(mempool[0]["confirmed_at"], Value::Null);
    // A new store reconstruction from the same values reproduces the update.
    let replay = scan("reorg", after(&first));
    assert_eq!(replay.update, reorg.update);
}

#[test]
fn lwk_previous_txids_come_from_state_or_a_forced_rebuild_checkpoint() {
    let first = scan("initial", fresh());
    let checkpoint = first.update["facts"]["freshness_checkpoint"].clone();
    // Ordinary refresh without stored state has no absence baseline.
    let ordinary = scan(
        "reorg",
        Refresh {
            checkpoint: checkpoint.clone(),
            ..fresh()
        },
    );
    assert_eq!(
        ordinary.update["facts"]["retracted_external_ids"],
        json!([])
    );
    let forced = scan(
        "reorg",
        Refresh {
            force_full: true,
            checkpoint: checkpoint.clone(),
            prior: Some(json!({"schema_version": 5})),
            ..fresh()
        },
    );
    assert_eq!(
        forced.update["facts"]["retracted_external_ids"],
        json!([txid("mixed")])
    );
    // Valid stored state wins over the checkpoint, even when forced.
    let stored = scan(
        "reorg",
        Refresh {
            force_full: true,
            prior: Some(
                json!({"schema_version":1,"canonical_txids":[txid("claim"),"f".repeat(64)]}),
            ),
            checkpoint,
            ..fresh()
        },
    );
    assert_eq!(
        stored.update["facts"]["retracted_external_ids"],
        json!(["f".repeat(64)])
    );
}

#[test]
fn lwk_incompatible_state_requires_a_rebuild_before_any_request() {
    let error = Execution::new(
        scope(),
        &json!({"name":"fixture","kind":"electrum","url":"tcp://127.0.0.1:1"}),
        &plan(&default_config()),
        "elementsregtest",
        vectors()["policy_asset"].as_str().unwrap(),
        vec!["receive".into()],
        Refresh {
            prior: Some(json!({"schema_version": 2})),
            ..fresh()
        },
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "observer_state_rebuild_required");
    let esplora = Execution::new(
        scope(),
        &json!({"name":"fixture","kind":"esplora","url":"https://x"}),
        &plan(&default_config()),
        "elementsregtest",
        vectors()["policy_asset"].as_str().unwrap(),
        vec!["receive".into()],
        fresh(),
    )
    .err()
    .unwrap();
    assert_eq!(esplora.code, "not_implemented");
}

#[test]
fn lwk_forced_rebuild_refuses_a_lagging_backend_but_an_ordinary_refresh_does_not() {
    let checkpoint = json!({"tip_height": 6});
    let mut fake = Fake::new("lagging");
    let error = execute_offline(
        &action(
            &default_config(),
            Refresh {
                force_full: true,
                checkpoint: checkpoint.clone(),
                ..fresh()
            },
        ),
        &mut fake,
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "backend_tip_behind");
    assert!(error.retryable);
    assert_eq!(
        fake.hits,
        [
            "blockchain.headers.subscribe",
            "blockchain.headers.subscribe"
        ]
    );
    let equal = Fake::new("lagging");
    assert!(execute_offline(
        &action(
            &default_config(),
            Refresh {
                force_full: true,
                checkpoint: json!({"tip_height": 3}),
                ..fresh()
            },
        ),
        &mut { equal },
    )
    .is_ok());
    assert!(scan(
        "lagging",
        Refresh {
            checkpoint,
            ..fresh()
        }
    )
    .update["facts"]["retracted_external_ids"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn lwk_scan_horizon_follows_prior_coverage_and_the_checkpoint() {
    let run = |refresh: Refresh| action(&default_config(), refresh).scan_to().unwrap();
    assert_eq!(run(fresh()), 2);
    let coverage = vec![
        json!({"branch_key":"receive","highest_used":7}),
        json!({"highest_used":-1}),
    ];
    assert_eq!(
        run(Refresh {
            prior: Some(json!({"schema_version":1})),
            prior_coverage: coverage.clone(),
            ..fresh()
        }),
        10
    );
    // Coverage of an incompatible payload is not evidence.
    assert_eq!(
        run(Refresh {
            force_full: true,
            prior: Some(json!({"schema_version":3})),
            prior_coverage: coverage,
            ..fresh()
        }),
        2
    );
    assert_eq!(
        run(Refresh {
            checkpoint: json!({"highest_used":{"0":"12","1":true,"2":-4,"3":"x"}}),
            ..fresh()
        }),
        15
    );
    let error = action(
        &default_config(),
        Refresh {
            checkpoint: json!({"highest_used":{"0": u64::from(u32::MAX)}}),
            ..fresh()
        },
    )
    .scan_to()
    .err()
    .unwrap();
    assert_eq!(error.code, "backend_sync_failed");
}

#[test]
fn lwk_transport_fences_pass_through_and_other_failures_are_redacted() {
    for (code, expected) in [
        ("cancelled", "cancelled"),
        ("interaction_required", "interaction_required"),
        ("network_egress_disabled", "network_egress_disabled"),
        ("stale_context", "stale_context"),
        ("network_error", "backend_sync_failed"),
        ("protocol_error", "backend_sync_failed"),
    ] {
        for at in [0, 3] {
            let mut fake = Fake::new("initial");
            fake.fail = Some((at, AppError::new(code, "read token=abcd1234 failed")));
            let error = execute_offline(&action(&default_config(), fresh()), &mut fake)
                .err()
                .unwrap();
            assert_eq!(error.code, expected, "{code} at {at}");
            // No request follows the first failure.
            assert_eq!(fake.hits.len(), at);
            if expected == "backend_sync_failed" {
                assert_eq!(
                    error.message,
                    "LWK Liquid observation failed for backend 'fixture'"
                );
                let details =
                    kassiber_pyjson::dumps(error.details.as_ref().unwrap(), &Default::default())
                        .unwrap();
                assert!(details.contains("\"dependency\": \"lwk\""));
                assert!(!details.contains("abcd1234"));
                assert!(details.contains("token=[redacted]"));
                assert!(error.retryable);
            }
        }
    }
}

#[test]
fn lwk_backend_rejects_substituted_transactions_and_never_broadcasts() {
    let mut fake = Fake::new("initial");
    fake.substitute = true;
    let error = execute_offline(&action(&default_config(), fresh()), &mut fake)
        .err()
        .unwrap();
    assert_eq!(error.code, "backend_sync_failed");
    let mut fake = Fake::new("initial");
    let backend = backend::Backend::new(&mut fake);
    let tx: lwk_wollet::elements::Transaction = lwk_wollet::elements::encode::deserialize(
        &<Vec<u8> as lwk_wollet::elements::hex::FromHex>::from_hex(
            vectors()["phases"]["initial"]["transactions"][txid("fund")]
                .as_str()
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(backend.broadcast(&tx).is_err());
    assert_eq!(
        backend.take_failure().unwrap().code,
        "observer_capability_unsupported"
    );
    drop(backend);
    assert!(fake.hits.is_empty());
}

#[test]
fn lwk_backend_requires_headers_for_the_requested_heights() {
    struct Shifted(Fake);
    impl Rpc for Shifted {
        fn call(&mut self, method: &str, params: Value) -> Result<Value> {
            if method == "blockchain.block.header" {
                let height = params[0].as_u64().unwrap() + 1;
                return self.0.call(method, json!([height.min(6)]));
            }
            self.0.call(method, params)
        }
    }
    let mut shifted = Shifted(Fake::new("initial"));
    let error = execute_offline(&action(&default_config(), fresh()), &mut shifted)
        .err()
        .unwrap();
    assert_eq!(error.code, "backend_sync_failed");
}

#[test]
fn lwk_corrupt_store_fails_before_the_backend_is_contacted() {
    let mut values = BTreeMap::new();
    values.insert("000000000000".to_owned(), b"not an update".to_vec());
    let a = action(&default_config(), Refresh { values, ..fresh() });
    let error = a.restore().err().unwrap();
    assert_eq!(error.code, "backend_sync_failed");
    assert_eq!(format!("{:?}", Buffered::default()), "Buffered(..)");
}

#[test]
fn lwk_run_requires_this_actions_consent_and_scope() {
    let a = action(&default_config(), fresh());
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    let deadline = Instant::now() + Duration::from_secs(5);
    let cancel = crate::scan::Cancellation::default();
    let other = action(&default_config(), fresh());
    let foreign = ExecutionConsent::explicit_trigger(&other);
    assert_eq!(
        a.run(&gate, &scope(), Some(&foreign), deadline, &cancel)
            .err()
            .unwrap()
            .code,
        "interaction_required"
    );
    let mut changed = scope();
    changed.source_revision = "2".into();
    let grant = ExecutionConsent::explicit_trigger(&a);
    assert_eq!(
        a.run(&gate, &changed, Some(&grant), deadline, &cancel)
            .err()
            .unwrap()
            .code,
        "stale_context"
    );
    let offline = Gate::new(|| Offline::Offline, Ledger::default());
    assert_eq!(
        a.run(&offline, &scope(), Some(&grant), deadline, &cancel)
            .err()
            .unwrap()
            .code,
        "network_egress_disabled"
    );
    cancel.cancel();
    assert_eq!(
        a.run(&gate, &scope(), Some(&grant), deadline, &cancel)
            .err()
            .unwrap()
            .code,
        "cancelled"
    );
}

#[test]
fn lwk_network_and_asset_codes_follow_the_python_mapping() {
    assert_eq!(network("liquidv1", "").unwrap(), Network::Liquid);
    assert_eq!(
        network(" LiquidTestnet ", "").unwrap(),
        Network::TestnetLiquid
    );
    let policy = vectors()["policy_asset"].as_str().unwrap().to_owned();
    assert_eq!(
        network("elementsregtest", &policy)
            .unwrap()
            .policy_asset()
            .to_string(),
        policy
    );
    assert!(network("elementsregtest", "zz").is_err());
    assert!(network("main", &policy).is_err());
    assert_eq!(facts::asset_code(&policy.to_uppercase(), &policy), "LBTC");
    assert_eq!(facts::asset_code("AB", &policy), "ab");
    assert_eq!(facts::asset_code(&policy, ""), policy);
}

fn descriptor_text(config: &Value) -> std::result::Result<String, String> {
    descriptor::descriptor_text(&plan(config)).map_err(|e| e.message)
}

#[test]
fn lwk_descriptor_translation_changes_only_the_outer_wrapper_and_view_key() {
    let base = vectors()["descriptor"].as_str().unwrap().to_owned();
    let slip = "f46e151e2e0d81601e9e25334f7a9f4ffaf9790192b387af036a70c88304e933";
    let xpub = base
        .split("elwpkh(")
        .nth(1)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(
        descriptor_text(&config(&base)).unwrap(),
        format!("ct(slip77({slip}),elwpkh({xpub}/<0;1>/*))")
    );
    let nested = base
        .replace("elwpkh(", "elsh(wpkh(")
        .replace("/*))", "/*)))");
    assert_eq!(
        descriptor_text(&config(&nested)).unwrap(),
        format!("ct(slip77({slip}),elsh(wpkh({xpub}/<0;1>/*)))")
    );
    let taproot = base.replace("elwpkh(", "eltr(");
    assert!(descriptor_text(&config(&taproot))
        .unwrap()
        .contains(",eltr("));
    let separate = json!({"chain":"liquid","network":"elementsregtest","gap_limit":3,
        "descriptor":base.replace("<0;1>","0"),"change_descriptor":base.replace("<0;1>","1")});
    assert_eq!(
        descriptor_text(&separate).unwrap(),
        descriptor_text(&config(&base)).unwrap()
    );
    let different = json!({"chain":"liquid","network":"elementsregtest","gap_limit":3,
        "descriptor":base.replace("<0;1>","0"),"change_descriptor":base.replace("<0;1>","2")});
    assert_eq!(
        descriptor_text(&different).err().unwrap(),
        "Separate Liquid receive/change descriptors are not structurally equivalent"
    );
    let swapped = json!({"chain":"liquid","network":"elementsregtest","gap_limit":3,
        "descriptor":base.replace("<0;1>","1"),"change_descriptor":base.replace("<0;1>","0")});
    assert!(descriptor_text(&swapped).is_err());
    assert!(descriptor::wollet_descriptor_string(&plan(&config(&base)))
        .unwrap()
        .contains('#'));
    // A public blinding key cannot unblind.
    let public = format!("ct(02{},elwpkh({xpub}/<0;1>/*))", "11".repeat(32));
    if let Some(plan) = descriptors::load(&config(&public)).ok().flatten() {
        assert_eq!(
            descriptor::descriptor_text(&plan).err().unwrap().message,
            "LWK requires private view material"
        );
    }
}

#[test]
fn lwk_wrapper_translation_matches_python_on_raw_text() {
    for (input, expected) in [
        ("ct(k,wpkh(a))", "ct(k,elwpkh(a))"),
        ("ct(k,wsh(multi(1,a,b)))", "ct(k,elwsh(multi(1,a,b)))"),
        ("ct(k,tr(a))", "ct(k,eltr(a))"),
        ("ct(k,sh(wpkh(a)))", "ct(k,elsh(wpkh(a)))"),
        ("ct(k,pkh(a))", "ct(k,pkh(a))"),
        ("ct(slip77(f(x)),wpkh(a))", "ct(slip77(f(x)),elwpkh(a))"),
    ] {
        assert_eq!(descriptor::translate_wrapper(input).unwrap(), expected);
    }
    assert!(descriptor::translate_wrapper("ct(k)").is_err());
}

#[test]
fn lwk_compatibility_reasons_name_every_unported_route() {
    let p = plan(&default_config());
    let reason = |backend: Value| descriptor::compatibility_reason(&backend, &p);
    let electrum = |extra: Value| {
        let mut b = json!({"name":"e","kind":"electrum","url":"tcp://x:1"});
        b.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        b
    };
    assert_eq!(reason(electrum(json!({}))).unwrap(), None);
    assert_eq!(reason(electrum(json!({"timeout":"30"}))).unwrap(), None);
    assert_eq!(reason(electrum(json!({"insecure":"off"}))).unwrap(), None);
    for (extra, expected) in [
        (json!({"proxy":"socks5h://p:1"}), "proxy_transport"),
        (json!({"tor_proxy":"socks5h://p:1"}), "proxy_transport"),
        (
            json!({"config":{"proxy":"socks5h://p:1"}}),
            "proxy_transport",
        ),
        (json!({"url":"tcp://abc.ONION.:1"}), "proxy_transport"),
        (json!({"url":"ssl://[fe80::1]:50002"}), ""),
        (json!({"certificate":"/ca"}), "custom_ca"),
        (json!({"insecure":true}), "insecure_tls"),
        (json!({"insecure":" Yes "}), "insecure_tls"),
        (json!({"timeout":60}), "custom_timeout"),
        (json!({"timeout":"30.0"}), "custom_timeout"),
        (json!({"timeout":false}), "custom_timeout"),
        (
            json!({"kind":"esplora","url":"https://x"}),
            "http_route_policy",
        ),
        (
            json!({"kind":"Liquid-Esplora","url":"https://x"}),
            "http_route_policy",
        ),
        (json!({"kind":"esplora","insecure":"1"}), "insecure_tls"),
        (
            json!({"kind":"esplora","certificate":"/ca","insecure":"0"}),
            "custom_ca",
        ),
        (
            json!({"kind":"esplora","certificate":"/ca","proxy":"p"}),
            "custom_ca",
        ),
        (json!({"kind":"bitcoinrpc"}), "backend_kind"),
        (json!({"kind":"electrum","certificate":"  "}), ""),
    ] {
        let got = reason(electrum(extra.clone())).unwrap().unwrap_or("");
        assert_eq!(got, expected, "{extra}");
    }
    assert_eq!(
        reason(electrum(json!({"insecure":"maybe"})))
            .err()
            .unwrap()
            .message,
        "Invalid boolean value: maybe"
    );
    let different = plan(
        &json!({"chain":"liquid","network":"elementsregtest","gap_limit":3,
        "descriptor":vectors()["descriptor"].as_str().unwrap().replace("<0;1>","0"),
        "change_descriptor":vectors()["descriptor"].as_str().unwrap().replace("<0;1>","2")}),
    );
    assert_eq!(
        descriptor::compatibility_reason(&electrum(json!({})), &different).unwrap(),
        Some("separate_change_descriptor")
    );
    assert_eq!(
        descriptor::compatibility_reason(&electrum(json!({"kind":"esplora"})), &different).unwrap(),
        Some("separate_change_descriptor")
    );
}

/// The scanned wallet transactions, restored offline from the stored update.
fn wallet_txs() -> Vec<lwk_wollet::WalletTx> {
    let first = scan("initial", fresh());
    let a = action(
        &default_config(),
        Refresh {
            values: first.values,
            ..fresh()
        },
    );
    let (wollet, _) = a.restore().unwrap();
    wollet.transactions().unwrap()
}

fn source() -> facts::Source<'static> {
    facts::Source {
        network: "elementsregtest",
        policy_asset: "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225",
        backend_name: "fixture",
        branch_keys: &[],
    }
}

fn foreign_input(index: u32) -> lwk_wollet::elements::TxIn {
    lwk_wollet::elements::TxIn {
        previous_output: lwk_wollet::elements::OutPoint::new(
            lwk_wollet::elements::Txid::from_raw_hash(
                lwk_wollet::elements::hashes::sha256d::Hash::hash(&index.to_le_bytes()),
            ),
            index,
        ),
        ..Default::default()
    }
}

#[test]
fn lwk_outbound_mixed_input_spend_books_the_exact_delta_with_the_fee_inside() {
    let spend_id = txid("spend");
    let mut spend = wallet_txs()
        .into_iter()
        .find(|t| t.txid.to_string() == spend_id)
        .unwrap();
    let sole = facts::records(&source(), &spend).unwrap();
    assert_eq!(sole[0]["amount"], "0.0006");
    assert_eq!(sole[0]["fee"], "0.000005");
    spend.tx.input.push(foreign_input(7));
    spend.inputs.push(None);
    let mixed = facts::records(&source(), &spend).unwrap();
    assert_eq!(mixed[0]["amount"], "0.000605");
    assert_eq!(mixed[0]["fee"], "0");
    assert_eq!(mixed[0]["amount_includes_fee"], true);
    assert_eq!(mixed[0]["kind"], "withdrawal");
    let component = &raw(&mixed[0])["component"];
    assert_eq!(component["fee_attribution"], "implicit_wallet_delta");
    assert_eq!(component["fee_sats"], 0);
    assert_eq!(component["transaction_fee_sats"], 500);
}

#[test]
fn lwk_htlc_evidence_needs_one_claim_input_or_one_refund_outpoint() {
    let txs = wallet_txs();
    let find = |name: &str| {
        let id = txid(name);
        txs.iter()
            .find(|t| t.txid.to_string() == id)
            .unwrap()
            .clone()
    };
    let mut claim = find("claim");
    claim.tx.input.push(foreign_input(3));
    claim.inputs.push(None);
    let batched = facts::records(&source(), &claim).unwrap();
    assert!(batched[0].get("payment_hash").is_none());
    assert!(raw(&batched[0]).get("htlc_spend").is_none());
    let mut refund = find("refund");
    let mut second = refund.tx.input[0].clone();
    second.previous_output.vout += 1;
    refund.tx.input.push(second);
    refund.inputs.push(None);
    let two = facts::records(&source(), &refund).unwrap();
    assert!(two[0].get("swap_refund_funding_txid").is_none());
    assert!(raw(&two[0]).get("htlc_spend").is_none());
    // The same refund outpoint twice is still one link, but two inputs
    // carry no single-input attestation.
    let mut repeated = find("refund");
    let same = repeated.tx.input[0].clone();
    repeated.tx.input.push(same);
    repeated.inputs.push(None);
    let once = facts::records(&source(), &repeated).unwrap();
    assert_eq!(once[0]["swap_refund_funding_vout"], 1);
    assert!(raw(&once[0]).get("htlc_spend").is_none());
}

#[test]
fn lwk_separate_change_with_other_blinding_is_not_canonicalized() {
    let base = vectors()["descriptor"].as_str().unwrap().to_owned();
    assert!(base.contains("f46e151e"));
    let other = base.replace(
        "f46e151e2e0d81601e9e25334f7a9f4ffaf9790192b387af036a70c88304e933",
        "0505050505050505050505050505050505050505050505050505050505050505",
    );
    let config = json!({"chain":"liquid","network":"elementsregtest","gap_limit":3,
        "descriptor":base.replace("<0;1>","0"),"change_descriptor":other.replace("<0;1>","1")});
    if let Some(plan) = descriptors::load(&config).ok().flatten() {
        assert!(descriptor::canonical_multipath(&plan).is_none());
        assert_eq!(
            descriptor::compatibility_reason(&json!({"kind":"electrum","url":"tcp://x:1"}), &plan)
                .unwrap(),
            Some("separate_change_descriptor")
        );
    } else {
        panic!("separate change plan must load");
    }
}

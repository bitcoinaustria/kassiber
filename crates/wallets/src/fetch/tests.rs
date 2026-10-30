use super::*;
use crate::publication::testbook::{load, snapshot};
use kassiber_net::{Ledger, Offline};
use std::time::Duration;

fn book() -> Connection {
    let cases: Value = serde_json::from_str(include_str!(
        "../../tests/vectors/python_wallet_publication.json"
    ))
    .unwrap();
    load(&kassiber_pytext::loads(&cases["cases"][0]["before"].to_string()).unwrap())
}
fn backend(kind: &str) -> Value {
    json!({"name":"fixture","kind":kind,"url":if kind=="electrum"{"tcp://unresolvable.invalid:50001"}else{"http://unresolvable.invalid:80"}})
}
fn run(
    action: &Fetch,
    gate: &Gate,
    scope: &Scope,
    consent: Option<&FetchConsent>,
    cancel: &Cancellation,
) -> AppError {
    action
        .run(
            gate,
            scope,
            consent,
            Instant::now() + Duration::from_secs(1),
            cancel,
            100,
        )
        .unwrap_err()
}

#[test]
fn fetch_preflight_and_missing_foreign_revoked_consent_make_no_traffic_or_writes() {
    for (kind, url) in [
        ("electrum", "tcp://unresolvable.invalid:50001"),
        ("electrum", "ssl://unresolvable.invalid:50002"),
        ("electrum", "tcp://127.0.0.1:1"),
        ("electrum", "ssl://127.0.0.1:1"),
        ("esplora", "http://unresolvable.invalid:80"),
        ("esplora", "http://127.0.0.1:1"),
    ] {
        let conn = book();
        let before = snapshot(&conn);
        let ledger = Ledger::default();
        let gate = Gate::new(|| Offline::Online, ledger.clone());
        let mut backend = backend(kind);
        backend["url"] = json!(url);
        let action = Fetch::new(&conn, "p", "a", &backend, false, "fixture").unwrap();
        assert_eq!(snapshot(&conn), before);
        assert_eq!(
            run(
                &action,
                &gate,
                action.scope(),
                None,
                &Cancellation::default()
            )
            .code,
            "interaction_required"
        );
        let other = Fetch::new(&conn, "p", "a", &backend, false, "fixture").unwrap();
        let foreign = FetchConsent::explicit_trigger(&other);
        assert_eq!(
            run(
                &action,
                &gate,
                action.scope(),
                Some(&foreign),
                &Cancellation::default()
            )
            .code,
            "interaction_required"
        );
        let grant = FetchConsent::explicit_trigger(&action);
        grant.revoke();
        assert_eq!(
            run(
                &action,
                &gate,
                action.scope(),
                Some(&grant),
                &Cancellation::default()
            )
            .code,
            "interaction_required"
        );
        assert!(gate.attempts().records.is_empty());
        assert!(ledger.snapshot(0, 100).records.is_empty());
        assert_eq!(snapshot(&conn), before);
    }
}

#[test]
fn fetch_changed_scope_offline_and_cancelled_actions_make_no_traffic() {
    let conn = book();
    let action = Fetch::new(&conn, "p", "a", &backend("esplora"), false, "fixture").unwrap();
    let ledger = Ledger::default();
    let gate = Gate::new(|| Offline::Online, ledger.clone());
    for field in 0..5 {
        let mut scope = action.scope().clone();
        match field {
            0 => scope.database_instance.push('x'),
            1 => scope.profile_id.push('x'),
            2 => scope.wallet_id.push('x'),
            3 => scope.environment_id.push('x'),
            _ => scope.source_revision.push('x'),
        }
        let grant = FetchConsent::explicit_trigger(&action);
        assert_eq!(
            run(
                &action,
                &gate,
                &scope,
                Some(&grant),
                &Cancellation::default()
            )
            .code,
            "stale_context"
        );
        assert!(
            !grant.started.load(Ordering::SeqCst),
            "a stale scope must not consume the exact action's grant"
        );
        assert!(gate.attempts().records.is_empty());
    }
    let gate = Gate::new(|| Offline::Offline, ledger.clone());
    let grant = FetchConsent::explicit_trigger(&action);
    assert_eq!(
        run(
            &action,
            &gate,
            action.scope(),
            Some(&grant),
            &Cancellation::default()
        )
        .code,
        "network_egress_disabled"
    );
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        run(&action, &gate, action.scope(), Some(&grant), &cancel).code,
        "cancelled"
    );
    assert!(gate.attempts().records.is_empty());
    assert!(ledger.snapshot(0, 100).records.is_empty());
}

#[test]
fn fetch_grant_revocation_reaches_every_protocol_stage() {
    let conn = book();
    let ledger = Ledger::default();
    let gate = Gate::new(|| Offline::Online, ledger.clone());
    let deadline = Instant::now() + Duration::from_secs(1);
    let cancel = Cancellation::default();
    let bdk = Fetch::new(&conn, "p", "a", &backend("electrum"), false, "fixture").unwrap();
    let grant = FetchConsent::explicit_trigger(&bdk);
    let execution = &bdk.observers[0].action;
    let inherited = ExecutionConsent::for_fetch(execution, grant.transport.clone());
    grant.revoke();
    assert!(
        matches!(execution.run(&gate,bdk.scope(),Some(&inherited),deadline,&cancel,100),Err(e) if e.code=="interaction_required")
    );
    let fetch = Fetch::new(&conn, "p", "a", &backend("esplora"), false, "fixture").unwrap();
    let grant = FetchConsent::explicit_trigger(&fetch);
    let discovery = fetch.compatibility.as_ref().unwrap();
    let inherited = DiscoveryConsent::for_fetch(discovery, grant.transport.clone());
    let acquisition = Acquisition::new_cached(
        fetch.scope().clone(),
        &fetch.backend,
        &[],
        "fixture",
        &json!({}),
        &json!({}),
        "main",
    )
    .unwrap();
    let acquired = AcquisitionConsent::for_fetch(&acquisition, grant.transport.clone());
    grant.revoke();
    assert_eq!(
        discovery
            .run(
                &gate,
                fetch.scope(),
                Some(&inherited),
                deadline,
                &cancel,
                |_| {}
            )
            .unwrap_err()
            .code,
        "interaction_required"
    );
    assert_eq!(
        acquisition
            .run(
                &gate,
                fetch.scope(),
                Some(&acquired),
                deadline,
                &cancel,
                |_| {}
            )
            .unwrap_err()
            .code,
        "interaction_required"
    );
    assert!(gate.attempts().records.is_empty());
    assert!(ledger.snapshot(0, 100).records.is_empty());
}

#[test]
fn fetch_grant_is_consumed_once_even_after_backend_failure() {
    let server = kassiber_net::test_support::FakeProvider::new("malformed").unwrap();
    let conn = book();
    let action = Fetch::new(
        &conn,
        "p",
        "a",
        &json!({"name":"fixture","kind":"esplora","url":server.base(),"batch_size":1}),
        false,
        "fixture",
    )
    .unwrap();
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    let grant = FetchConsent::explicit_trigger(&action);
    assert_eq!(
        run(
            &action,
            &gate,
            action.scope(),
            Some(&grant),
            &Cancellation::default()
        )
        .code,
        "protocol_error"
    );
    let hits = server.requests().len();
    assert!(hits > 0);
    assert_eq!(
        run(
            &action,
            &gate,
            action.scope(),
            Some(&grant),
            &Cancellation::default()
        )
        .code,
        "interaction_required"
    );
    assert_eq!(server.requests().len(), hits);
}

#[test]
fn fetch_unsupported_routes_refuse_before_traffic_and_transactions() {
    let conn = book();
    let before = snapshot(&conn);
    let mut other = backend("esplora");
    other["name"] = json!("other");
    let mut nested = backend("esplora");
    nested["config"] = json!({"insecure":true});
    for config in [backend("bitcoinrpc"), other, nested] {
        assert!(
            matches!(Fetch::new(&conn,"p","a",&config,false,"fixture"),Err(e) if e.code=="native_fallback_required")
        );
    }
    assert!(
        matches!(Fetch::new(&conn,"p","foreign",&backend("esplora"),false,"fixture"),Err(e) if e.code=="not_found")
    );
    assert_eq!(snapshot(&conn), before);
    conn.execute_batch("BEGIN").unwrap();
    assert!(
        matches!(Fetch::new(&conn,"p","a",&backend("esplora"),false,"fixture"),Err(e) if e.code=="observer_prepare_in_transaction")
    );
    conn.execute_batch("ROLLBACK").unwrap();
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('b','w','p','Other','descriptor','fixed')",[]).unwrap();
    assert!(
        matches!(Fetch::new(&conn,"p","a",&backend("esplora"),false,"fixture"),Err(e) if e.code=="native_fallback_required")
    );
}

#[test]
fn fetch_esplora_output_facts_preserve_exact_amounts_depth_and_unknowns() {
    let mut acquired = json!({"freshness_checkpoint":{"highest_used":{"0":2}},"scripts_changed":1,"scripts_unchanged":3,
        "transactions":{"a":{}},"esplora_history_complete":true,"observer_retracted_external_ids":["old"],"tip_height":12,
        "scripts":[{"target":{"address":"bc1-fixture","script_pubkey":"51","branch_label":"receive","branch_index":0,"address_index":2},
            "utxos":[{"txid":"a","vout":1,"value":2100000000000000u64,"status":{"confirmed":true,"block_height":10,"block_time":1700000000}}]}]});
    let meta = esplora_metadata(&acquired, "main").unwrap();
    assert_eq!(
        meta,
        json!({"observer_route":"compatibility","observer_compatibility_reason":"http_route_policy",
        "freshness_checkpoint":{"highest_used":{"0":2}},"scripts_changed":1,"scripts_unchanged":3,"known_txids":1,
        "esplora_history_complete":true,"observer_retracted_external_ids":["old"],"utxos":[{
            "txid":"a","vout":1,"asset":"BTC","amount_sats":2100000000000000u64,"confirmation_status":"confirmed",
            "confirmations":3,"block_height":10,"block_time":"2023-11-14T22:13:20Z","chain":"bitcoin","network":"main",
            "address":"bc1-fixture","script_pubkey":"51","address_label":"receive #2","branch_label":"receive","branch_index":0,"address_index":2,
            "raw":{"source":"esplora_scripthash_utxo","confirmed":true}}]})
    );
    acquired["tip_height"] = Value::Null;
    assert_eq!(
        esplora_metadata(&acquired, "main").unwrap()["utxos"][0]["confirmations"],
        Value::Null
    );
    acquired["scripts"][0]["utxos"][0]["status"] = json!({"confirmed":false});
    let meta = esplora_metadata(&acquired, "test").unwrap();
    assert_eq!(meta["utxos"][0]["network"], "test");
    assert_eq!(meta["utxos"][0]["confirmation_status"], "mempool");
    assert_eq!(meta["utxos"][0]["block_height"], Value::Null);
    for invalid in [json!(1.1), json!(-1), Value::Null, json!("ambiguous")] {
        acquired["scripts"][0]["utxos"][0]["value"] = invalid;
        assert_eq!(
            esplora_metadata(&acquired, "main").unwrap_err().code,
            "protocol_error"
        );
    }
    acquired["scripts"][0]["utxos"] = Value::Null;
    assert_eq!(
        esplora_metadata(&acquired, "main").unwrap_err().code,
        "native_fallback_required"
    );
    acquired["scripts"] = json!([]);
    assert_eq!(
        esplora_metadata(&acquired, "main").unwrap()["utxos"],
        json!([])
    );
}

#[test]
fn fetch_esplora_unchanged_cache_retains_no_inventory_claim() {
    let acquired = json!({"utxos_skipped_unchanged":true,"freshness_checkpoint":{"previous":true},
        "scripts_changed":0,"scripts_unchanged":6,"transactions":{},"esplora_history_complete":true,"observer_retracted_external_ids":[]});
    let meta = esplora_metadata(&acquired, "main").unwrap();
    assert_eq!(meta["utxos_skipped_unchanged"], true);
    assert!(meta.get("utxos").is_none());
    assert_eq!(meta["known_txids"], 0);
    assert_eq!(
        meta["freshness_checkpoint"],
        acquired["freshness_checkpoint"]
    );
}

#[test]
fn fetch_reuse_counts_distinct_observer_sources_instead_of_duplicate_script_labels() {
    let conn = book();
    let vectors: Value =
        serde_json::from_str(include_str!("../../tests/vectors/python_descriptors.json")).unwrap();
    let mut config = vectors["plans"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "multi-script-p2tr+p2pkh+p2wpkh+p2sh-p2wpkh")
        .unwrap()["config"]
        .clone();
    config["backend"] = json!("fixture");
    for (types, count, reuse) in [
        (json!(["p2wpkh", "p2wpkh"]), 1, true),
        (json!(["p2wpkh", "p2tr"]), 2, false),
    ] {
        config["script_types"] = types;
        conn.execute(
            "UPDATE wallets SET config_json=? WHERE id='a'",
            [config.to_string()],
        )
        .unwrap();
        let action = Fetch::new(&conn, "p", "a", &backend("electrum"), false, "fixture").unwrap();
        assert_eq!(action.observers.len(), count);
        assert!(action
            .observers
            .iter()
            .all(|o| o.current.is_some() == reuse));
    }
}

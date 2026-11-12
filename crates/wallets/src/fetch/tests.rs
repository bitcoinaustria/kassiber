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
    let Action::Bdk {
        action: execution, ..
    } = &bdk.observers[0].action
    else {
        panic!("Bitcoin Electrum uses BDK");
    };
    let inherited = ExecutionConsent::for_fetch(execution, grant.transport().unwrap());
    grant.revoke();
    assert!(
        matches!(execution.run(&gate,bdk.scope(),Some(&inherited),deadline,&cancel,100),Err(e) if e.code=="interaction_required")
    );
    let fetch = Fetch::new(&conn, "p", "a", &backend("esplora"), false, "fixture").unwrap();
    let grant = FetchConsent::explicit_trigger(&fetch);
    let discovery = fetch.compatibility.as_ref().unwrap();
    let inherited = DiscoveryConsent::for_fetch(discovery, grant.transport().unwrap());
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
    let acquired = AcquisitionConsent::for_fetch(&acquisition, grant.transport().unwrap());
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
    for config in [other, backend("custom")] {
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
    // Another wallet in the profile is ownership evidence, not a refusal.
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('b','w','p','Other','descriptor','fixed')",[]).unwrap();
    let action = Fetch::new(&conn, "p", "a", &backend("esplora"), false, "fixture").unwrap();
    assert!(action.skip.is_none());
}

fn duplicate(conn: &Connection, label: &str) {
    conn.execute(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
         SELECT 'b','w','p',?,'descriptor',config_json,'fixed' FROM wallets WHERE id='a'",
        [label],
    )
    .unwrap();
}

#[test]
fn fetch_overlap_skip_makes_no_request_even_offline() {
    for kind in ["electrum", "esplora"] {
        let conn = book();
        duplicate(&conn, "Alpha");
        let before = snapshot(&conn);
        let ledger = Ledger::default();
        let gate = Gate::new(|| Offline::Offline, ledger.clone());
        let action = Fetch::new(&conn, "p", "a", &backend(kind), true, "fixture").unwrap();
        let targets = action.targets.len();
        assert_eq!(action.skip, Some(targets));
        let consent = FetchConsent::explicit_trigger(&action);
        action.check_permission(&gate, Some(&consent)).unwrap();
        let fetch = action
            .run(
                &gate,
                action.scope(),
                Some(&consent),
                Instant::now() + Duration::from_secs(1),
                &Cancellation::default(),
                100,
            )
            .unwrap();
        // The skip reports its own wall time, as Python's discovery does.
        let mut skip = fetch["skip_outcome"].clone();
        assert!(skip["elapsed_ms"].is_u64(), "{skip}");
        assert_eq!(skip["elapsed_ms"], fetch["elapsed_ms"]);
        skip["elapsed_ms"] = json!(0);
        assert_eq!(
            skip,
            json!({"wallet":"Cold","status":"skipped",
                "reason":"all sync targets are already covered by canonical wallet sources",
                "target_count":0,"filtered_overlap_targets":targets,"elapsed_ms":0,"force_full":true})
        );
        assert!(fetch["sync_state"].is_null());
        assert!(ledger.snapshot(0, 100).records.is_empty());
        assert_eq!(snapshot(&conn), before);
        // The grant is still single use.
        assert_eq!(
            run(
                &action,
                &gate,
                action.scope(),
                Some(&consent),
                &Cancellation::default()
            )
            .code,
            "interaction_required"
        );
        action.revalidate(&conn).unwrap();
    }
}

#[test]
fn fetch_trimmed_electrum_descriptor_takes_the_compatibility_route() {
    let conn = book();
    let action = Fetch::new(&conn, "p", "a", &backend("electrum"), false, "fixture").unwrap();
    assert_eq!((action.reason, action.observers.len()), ("", 1));
    let script = action.targets[1]["script_pubkey"]
        .as_str()
        .unwrap()
        .to_owned();
    conn.execute(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
        VALUES('b','w','p','Alpha','descriptor','{}','fixed')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,txid,vout,outpoint,asset,amount,chain,network,
         script_pubkey,confirmation_status,first_seen_at,last_seen_at) VALUES('u','w','p','b','t',0,'t:0','BTC',1,
         'bitcoin',?,?,'confirmed','fixed','fixed')",
        [action.network.clone(), script],
    )
    .unwrap();
    let before = snapshot(&conn);
    let ledger = Ledger::default();
    let gate = Gate::new(|| Offline::Offline, ledger.clone());
    // BDK scans whole descriptors: no observer is prepared, and the
    // compatibility observer discovers online. A transport reason wins.
    let trimmed = Fetch::new(&conn, "p", "a", &backend("electrum"), false, "fixture").unwrap();
    assert_eq!(trimmed.reason, "source_overlap_partial_descriptor");
    assert!(trimmed.observers.is_empty() && trimmed.compatibility.is_some());
    // Revalidation, which leaves the index out, keeps the prepared route.
    trimmed.revalidate(&conn).unwrap();
    let mut proxied = backend("electrum");
    proxied["proxy"] = json!("socks5h://127.0.0.1:9050");
    let proxied = Fetch::new(&conn, "p", "a", &proxied, false, "fixture").unwrap();
    assert_eq!(proxied.reason, "proxy_transport");
    let grant = FetchConsent::explicit_trigger(&trimmed);
    assert_eq!(
        run(
            &trimmed,
            &gate,
            trimmed.scope(),
            Some(&grant),
            &Cancellation::default()
        )
        .code,
        "network_egress_disabled"
    );
    assert!(ledger.snapshot(0, 100).records.is_empty());
    assert_eq!(snapshot(&conn), before);
    let esplora = Fetch::new(&conn, "p", "a", &backend("esplora"), false, "fixture").unwrap();
    assert_eq!(esplora.reason, "http_route_policy");
}

#[test]
fn electrum_compatibility_reasons_follow_python_order() {
    let conn = book();
    let reason = |extra: Value| {
        let mut config = backend("electrum");
        for (key, value) in extra.as_object().unwrap() {
            config[key] = value.clone();
        }
        let action = Fetch::new(&conn, "p", "a", &config, false, "fixture")
            .unwrap_or_else(|e| panic!("{extra}: {e:?}"));
        // A compatibility route prepares no BDK observer.
        assert_eq!(
            action.observers.is_empty(),
            !action.reason.is_empty(),
            "{extra}"
        );
        action.reason
    };
    for (extra, expected) in [
        (json!({}), ""),
        (json!({"timeout": 45, "insecure": true}), ""),
        (json!({"proxy": ""}), ""),
        (
            json!({"proxy": "socks5h://127.0.0.1:9050"}),
            "proxy_transport",
        ),
        (json!({"tor_proxy": "127.0.0.1:9050"}), "proxy_transport"),
        (
            json!({"config": {"proxy": "127.0.0.1:9050"}}),
            "proxy_transport",
        ),
        (json!({"url": "tcp://abc.onion:50001"}), "proxy_transport"),
        (json!({"url": "ABC.ONION:50001"}), "proxy_transport"),
        (json!({"url": "tcp://onion.example:50001"}), ""),
        (json!({"certificate": "/ca.pem"}), "custom_ca"),
        (
            json!({"certificate": "/ca.pem", "proxy": "127.0.0.1:9050"}),
            "proxy_transport",
        ),
    ] {
        assert_eq!(reason(extra.clone()), expected, "{extra}");
    }
}

#[test]
fn fetch_esplora_output_facts_preserve_exact_amounts_depth_and_unknowns() {
    let mut acquired = json!({"freshness_checkpoint":{"highest_used":{"0":2}},"scripts_changed":1,"scripts_unchanged":3,
        "transactions":{"a":{}},"esplora_history_complete":true,"observer_retracted_external_ids":["old"],"tip_height":12,
        "scripts":[{"target":{"address":"bc1-fixture","script_pubkey":"51","branch_label":"receive","branch_index":0,"address_index":2},
            "utxos":[{"txid":"a","vout":1,"value":2100000000000000u64,"status":{"confirmed":true,"block_height":10,"block_time":1700000000}}]}]});
    let meta = esplora_metadata(&acquired, "compatibility", "http_route_policy", &|| {
        esplora_outputs(&acquired, "main")
    })
    .unwrap();
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
        esplora_metadata(&acquired, "compatibility", "http_route_policy", &|| {
            esplora_outputs(&acquired, "main")
        })
        .unwrap()["utxos"][0]["confirmations"],
        Value::Null
    );
    acquired["scripts"][0]["utxos"][0]["status"] = json!({"confirmed":false});
    let meta = esplora_metadata(&acquired, "compatibility", "http_route_policy", &|| {
        esplora_outputs(&acquired, "test")
    })
    .unwrap();
    assert_eq!(meta["utxos"][0]["network"], "test");
    assert_eq!(meta["utxos"][0]["confirmation_status"], "mempool");
    assert_eq!(meta["utxos"][0]["block_height"], Value::Null);
    for invalid in [json!(1.1), json!(-1), Value::Null, json!("ambiguous")] {
        acquired["scripts"][0]["utxos"][0]["value"] = invalid;
        assert_eq!(
            esplora_metadata(&acquired, "compatibility", "http_route_policy", &|| {
                esplora_outputs(&acquired, "main")
            })
            .unwrap_err()
            .code,
            "protocol_error"
        );
    }
    acquired["scripts"][0]["utxos"] = Value::Null;
    assert_eq!(
        esplora_metadata(&acquired, "compatibility", "http_route_policy", &|| {
            esplora_outputs(&acquired, "main")
        })
        .unwrap_err()
        .code,
        "native_fallback_required"
    );
    acquired["scripts"] = json!([]);
    assert_eq!(
        esplora_metadata(&acquired, "compatibility", "http_route_policy", &|| {
            esplora_outputs(&acquired, "main")
        })
        .unwrap()["utxos"],
        json!([])
    );
}

#[test]
fn fetch_esplora_unchanged_cache_retains_no_inventory_claim() {
    let acquired = json!({"utxos_skipped_unchanged":true,"freshness_checkpoint":{"previous":true},
        "scripts_changed":0,"scripts_unchanged":6,"transactions":{},"esplora_history_complete":true,"observer_retracted_external_ids":[]});
    let meta = esplora_metadata(&acquired, "compatibility", "http_route_policy", &|| {
        esplora_outputs(&acquired, "main")
    })
    .unwrap();
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

const LWK_DESCRIPTOR: &str = "ct(slip77(L5QrJcTEfb3mLzWWobx79wu3pLvHdKdqcPDsoqoU5RNn4RSE8gv3),elwpkh(xpub6Bm9M1SxZdzL3TxdNV8897FgtTLBgehR1wVNnMyJ5VLRK5n3tFqXxrCVnVQj4zooN4eFSkf6Sma84reWc5ZCXMxPbLXQs3BcaBdTd4YQa3B/<0;1>/*))";

fn liquid_book(extra: Value) -> Connection {
    let conn = book();
    let mut config = json!({"chain":"liquid","network":"liquidv1","descriptor":LWK_DESCRIPTOR,
        "gap_limit":3,"backend":"fixture"});
    config
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    for (key, value) in extra.as_object().unwrap() {
        if value.is_null() {
            config.as_object_mut().unwrap().remove(key);
        }
    }
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id='a'",
        [config.to_string()],
    )
    .unwrap();
    conn
}

#[test]
fn fetch_liquid_builds_one_lwk_observer_or_names_its_compatibility_route() {
    let conn = liquid_book(json!({}));
    let before = snapshot(&conn);
    let action = Fetch::new(&conn, "p", "a", &backend("electrum"), false, "fixture").unwrap();
    assert_eq!(action.observers.len(), 1);
    assert!(matches!(action.observers[0].action, Action::Lwk(_)));
    assert_eq!(action.observers[0].identity["observer_kind"], "lwk");
    assert_eq!(action.observers[0].identity["chain"], "liquid");
    assert_eq!(
        action.observers[0].identity["branch_keys"],
        json!(["receive", "change"])
    );
    assert_eq!(action.reason, "");
    assert_eq!(
        action.policy_asset,
        "6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d"
    );
    assert_eq!(snapshot(&conn), before);
    // `lwk_compatibility_reason` selects Python's Liquid compatibility
    // observer, which prepares no LWK observer and discovers online.
    let route = |config: &Value| {
        let action = Fetch::new(&conn, "p", "a", config, false, "f").unwrap();
        assert!(action.observers.is_empty() && action.compatibility.is_some());
        action.reason
    };
    let mut timeout = backend("electrum");
    timeout["timeout"] = json!(5);
    assert_eq!(route(&timeout), "custom_timeout");
    let mut insecure = backend("electrum");
    insecure["url"] = json!("ssl://unresolvable.invalid:50002");
    insecure["insecure"] = json!("yes");
    assert_eq!(route(&insecure), "insecure_tls");
    assert_eq!(route(&backend("esplora")), "http_route_policy");
    let mut proxied = backend("electrum");
    proxied["proxy"] = json!("socks5h://127.0.0.1:9050");
    assert_eq!(route(&proxied), "proxy_transport");
    assert_eq!(snapshot(&conn), before);
    let mut chain = backend("electrum");
    chain["chain"] = json!("bitcoin");
    let error = Fetch::new(&conn, "p", "a", &chain, false, "f")
        .err()
        .unwrap();
    assert_eq!(
        error.message,
        "Backend 'fixture' is configured for bitcoin, but source refresh requires liquid"
    );
    let unnamed = liquid_book(json!({"backend": null}));
    let error = Fetch::new(&unnamed, "p", "a", &backend("electrum"), false, "f")
        .err()
        .unwrap();
    assert_eq!(
        error.message,
        "Liquid wallets must name a backend explicitly; no public Liquid default is built in"
    );
    let regtest = liquid_book(json!({"network":"elementsregtest","policy_asset":"lbtc"}));
    if let Ok(action) = Fetch::new(&regtest, "p", "a", &backend("electrum"), false, "f") {
        assert_eq!(
            action.policy_asset,
            "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225"
        );
    }
}

#[test]
fn fetch_liquid_loads_versioned_values_and_revalidates_them() {
    let conn = liquid_book(json!({}));
    let id = observer_state::instance_id("a", "a", "descriptor:default");
    conn.execute(
        "INSERT INTO chain_observer_instances(id,workspace_id,profile_id,logical_wallet_id,source_wallet_id,
         source_key,observer_kind,chain,network,state_version,state_epoch,state_json,created_at,updated_at)
         VALUES(?,'w','p','a','a','descriptor:default','lwk','liquid','liquidv1',1,1,'{\"schema_version\":1}','t','t')",
        [&id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chain_observer_values(observer_id,namespace_version,key,value,updated_at) VALUES(?,2,'k',x'00','t')",
        [&id],
    )
    .unwrap();
    let error = Fetch::new(&conn, "p", "a", &backend("electrum"), false, "f")
        .err()
        .unwrap();
    assert_eq!(error.code, "observer_state_rebuild_required");
    // A forced rebuild never loads the opaque store.
    let forced = Fetch::new(&conn, "p", "a", &backend("electrum"), true, "f").unwrap();
    assert_eq!(forced.observers[0].values, Some(BTreeMap::new()));
    conn.execute("UPDATE chain_observer_values SET namespace_version=1", [])
        .unwrap();
    let action = Fetch::new(&conn, "p", "a", &backend("electrum"), false, "f").unwrap();
    assert_eq!(
        action.observers[0].values,
        Some(BTreeMap::from([("k".to_owned(), vec![0u8])]))
    );
    action.revalidate(&conn).unwrap();
    conn.execute("UPDATE chain_observer_values SET value=x'01'", [])
        .unwrap();
    assert_eq!(
        action.revalidate(&conn).err().unwrap().code,
        "stale_context"
    );
}

#[test]
fn esplora_compatibility_reasons_follow_python_order() {
    let reason = |extra: Value| {
        let mut backend = backend("esplora");
        for (key, value) in extra.as_object().unwrap() {
            backend[key] = value.clone();
        }
        esplora_reason(&backend).map_err(|e| e.message)
    };
    for (extra, expected) in [
        (json!({}), "http_route_policy"),
        (json!({"timeout": "30"}), "http_route_policy"),
        (json!({"timeout": " 30 "}), "http_route_policy"),
        (json!({"timeout": 45}), "custom_timeout"),
        (json!({"timeout": "30.0"}), "custom_timeout"),
        (json!({"timeout": "soon"}), "custom_timeout"),
        (json!({"token": "t", "timeout": 45}), "custom_http_auth"),
        (json!({"auth_header": "Basic x"}), "custom_http_auth"),
        (
            json!({"proxy": "127.0.0.1:9050", "token": "t"}),
            "proxy_transport",
        ),
        (json!({"url": "http://abc.onion./"}), "proxy_transport"),
        (
            json!({"url": "http://user@ABC.ONION:80/x"}),
            "proxy_transport",
        ),
        (json!({"url": "http://onion.example/"}), "http_route_policy"),
        (json!({"certificate": "/ca.pem", "proxy": "p"}), "custom_ca"),
        (
            json!({"insecure": "yes", "certificate": "/ca.pem"}),
            "insecure_tls",
        ),
        (json!({"insecure": false, "timeout": 45}), "custom_timeout"),
        (json!({"insecure": "", "timeout": 45}), "custom_timeout"),
        (json!({"config": {"timeout": 45}}), "http_route_policy"),
    ] {
        assert_eq!(reason(extra.clone()), Ok(expected), "{extra}");
    }
    assert_eq!(
        reason(json!({"insecure": "maybe"})),
        Err("Invalid boolean value: maybe".into())
    );
    // Esplora takes these natively; only Electrum's refuse.
    let conn = book();
    for extra in [
        json!({"timeout": 45}),
        json!({"proxy": "127.0.0.1:9050"}),
        json!({"token": "t"}),
    ] {
        let mut config = backend("esplora");
        for (key, value) in extra.as_object().unwrap() {
            config[key] = value.clone();
        }
        let action = Fetch::new(&conn, "p", "a", &config, false, "fixture").unwrap();
        assert_ne!(action.reason, "http_route_policy");
    }
}

#[test]
fn address_lists_prepare_their_targets_without_a_request() {
    let conn = book();
    let descriptor = Fetch::new(&conn, "p", "a", &backend("esplora"), false, "fixture").unwrap();
    let address = descriptor.targets[0]["address"]
        .as_str()
        .unwrap()
        .to_owned();
    let script = descriptor.targets[0]["script_pubkey"].clone();
    let set = |config: Value| {
        conn.execute(
            "UPDATE wallets SET config_json=? WHERE id='a'",
            [config.to_string()],
        )
        .unwrap();
    };
    set(json!({"addresses": [address, " ", address], "network": "main"}));
    let action = Fetch::new(&conn, "p", "a", &backend("esplora"), false, "fixture").unwrap();
    assert!(action.plan.is_none() && action.skip.is_none());
    assert_eq!(action.reason, "address_list");
    assert_eq!(
        action.targets,
        vec![
            json!({"chain":"bitcoin","network":"main","branch_index":0,"branch_label":"address",
            "address_index":0,"address":address,"unconfidential_address":null,"script_pubkey":script})
        ]
    );
    let refused = |config: Value, kind: &str| {
        set(config);
        Fetch::new(&conn, "p", "a", &backend(kind), false, "fixture")
            .err()
            .map(|e| (e.code, e.message))
            .unwrap()
    };
    // Python's compatibility Electrum observer serves address lists too.
    set(json!({"addresses": [address], "network": "main"}));
    let electrum = Fetch::new(&conn, "p", "a", &backend("electrum"), false, "fixture").unwrap();
    assert_eq!(
        (electrum.reason, electrum.targets.len()),
        ("address_list", 1)
    );
    assert_eq!(
        refused(json!({"addresses": [address], "chain": "liquid", "network": "liquidv1"}), "esplora"),
        ("app_error".into(), "Liquid live refresh currently requires descriptor-backed wallets so outputs can be unblinded locally".into())
    );
    assert_eq!(
        refused(json!({"addresses": ["not-an-address"]}), "esplora"),
        (
            "app_error".into(),
            "Invalid Bitcoin address: not-an-address".into()
        )
    );
    let mut regtest = backend("esplora");
    regtest["network"] = json!("regtest");
    set(json!({"addresses": [address], "network": "main"}));
    assert_eq!(
        Fetch::new(&conn, "p", "a", &regtest, false, "fixture")
            .err()
            .unwrap()
            .message,
        "Backend 'fixture' is configured for regtest, but source refresh requires main"
    );
}

fn header(time: u32) -> String {
    let mut header = [0u8; 80];
    header[68..72].copy_from_slice(&time.to_le_bytes());
    hex(&header)
}

#[test]
fn electrum_address_list_fetch_builds_python_records_and_outputs() {
    let conn = book();
    let descriptor = Fetch::new(&conn, "p", "a", &backend("esplora"), false, "fixture").unwrap();
    let address = descriptor.targets[0]["address"].clone();
    let script = descriptor.targets[0]["script_pubkey"]
        .as_str()
        .unwrap()
        .to_owned();
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id='a'",
        [json!({"addresses":[address],"network":"main","backend":"fixture"}).to_string()],
    )
    .unwrap();
    let tx = bitcoin::Transaction {
        version: bitcoin::transaction::Version::TWO,
        lock_time: bitcoin::absolute::LockTime::ZERO,
        input: vec![bitcoin::TxIn::default()],
        output: vec![bitcoin::TxOut {
            value: bitcoin::Amount::from_sat(10_000),
            script_pubkey: bitcoin::ScriptBuf::from_hex(&script).unwrap(),
        }],
    };
    let id = tx.compute_txid().to_string();
    let reply = |id: u64, v: Value| format!("{}\n", json!({"id":id,"result":v}));
    let block = header(1_700_000_000);
    // Histories, graphs and headers over one connection; outputs over a
    // second one after a fresh tip read, as Python lists them.
    let server = kassiber_net::test_support::FakeElectrum::sessions(vec![
        vec![
            (1, reply(1, json!(["fixture", "1.6"]))),
            (1, reply(2, json!({"height":5,"hex":"00".repeat(80)}))),
            (1, reply(3, json!("status"))),
            (1, reply(4, json!([{"tx_hash":id,"height":3}]))),
            (
                1,
                reply(5, json!(bitcoin::consensus::encode::serialize_hex(&tx))),
            ),
            (1, reply(6, json!(block))),
        ],
        vec![
            (1, reply(1, json!(["fixture", "1.6"]))),
            (1, reply(2, json!({"height":6,"hex":"00".repeat(80)}))),
            (
                1,
                reply(
                    3,
                    json!([{"tx_hash":id,"tx_pos":0,"value":10_000,"height":3}]),
                ),
            ),
            (1, reply(4, json!(block))),
        ],
    ])
    .unwrap();
    let mut config = backend("electrum");
    config["url"] = json!(server.url());
    let action = Fetch::new(&conn, "p", "a", &config, false, "fixture").unwrap();
    let grant = FetchConsent::explicit_trigger(&action);
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    let fetch = action
        .run(
            &gate,
            action.scope(),
            Some(&grant),
            Instant::now() + Duration::from_secs(10),
            &Cancellation::default(),
            100,
        )
        .unwrap();
    assert_eq!(server.requests().len(), 10);
    let meta = &fetch["adapter_meta"];
    assert_eq!(
        (
            &meta["observer_route"],
            &meta["observer_compatibility_reason"]
        ),
        (&json!("bitcoin_script"), &json!("address_list"))
    );
    assert_eq!(
        (
            &meta["transactions_fetched"],
            &meta["known_txids"],
            &meta["header_cache_hits"]
        ),
        (&json!(1), &json!(1), &json!(0))
    );
    assert_eq!(
        meta["freshness_checkpoint"]["electrum_headers"],
        json!({"3":1_700_000_000})
    );
    let utxo = &meta["utxos"][0];
    assert_eq!(
        (
            &utxo["txid"],
            &utxo["confirmations"],
            &utxo["block_time"],
            &utxo["address_label"]
        ),
        (
            &json!(id),
            &json!(4),
            &json!("2023-11-14T22:13:20Z"),
            &json!("address #0")
        )
    );
    assert_eq!(utxo["raw"]["source"], "electrum_scripthash_listunspent");
    let records = fetch["normalized_records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        (
            &records[0]["txid"],
            &records[0]["occurred_at"],
            &records[0]["direction"]
        ),
        (
            &json!(id),
            &json!("2023-11-14T22:13:20Z"),
            &json!("inbound")
        )
    );
    assert_eq!(fetch["sync_state"]["targets"].as_array().unwrap().len(), 1);
}

#[test]
fn stored_electrum_graphs_follow_python_selection() {
    let conn = book();
    let graph = json!({"_kassiber_electrum_graph":{"kind":"bitcoin_electrum","version":2},"vin":[],"vout":[]})
        .to_string();
    let hash = observer_state::graph_hash(&graph).unwrap();
    let insert = |id: &str, wallet: &str, kind: Option<&str>, kinds: &str, hash: &str| {
        conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,external_id_kind,fingerprint,
             occurred_at,direction,asset,amount,fee,raw_json,created_at)
             VALUES(?,'w','p',?,?,?,?,'2025-01-01T00:00:00Z','inbound','BTC',1,0,?,'fixed')",
            kassiber_store::rusqlite::params![id, wallet, format!("x-{id}"), kind, format!("f-{id}"), graph],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chain_observation_provenance(transaction_id,workspace_id,profile_id,wallet_id,authority_version,
             observer_ids_json,observer_kinds_json,chain,network,application_revision,graph_hash,quantity_hash,
             fee_attribution,observed_at,updated_at)
             VALUES(?,'w','p',?,1,'[]',?,'bitcoin','main','r',?,'q','exact','fixed','fixed')",
            kassiber_store::rusqlite::params![id, wallet, kinds, hash],
        )
        .unwrap();
    };
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('b','w','p','Other','descriptor','fixed')", []).unwrap();
    insert("t1", "a", Some("txid"), r#"["bdk","electrum"]"#, &hash);
    insert("t2", "a", Some("txid"), r#"["bdk"]"#, &hash);
    insert("t3", "a", Some("txid"), r#"["electrum"]"#, "stale");
    insert("t4", "a", None, r#"["electrum"]"#, &hash);
    insert("t5", "b", Some("txid"), r#"["electrum"]"#, &hash);
    let config = json!({"backend":"fixture"});
    // Only a stored Electrum backend of that exact name selects the cache.
    assert_eq!(stored_history(&conn, "p", "a", &config).unwrap(), json!({}));
    conn.execute(
        "INSERT INTO backends(name,kind,url,config_json,created_at,updated_at)
         VALUES('fixture',' Electrum ','tcp://unresolvable.invalid:50001','{}','fixed','fixed')",
        [],
    )
    .unwrap();
    assert_eq!(
        stored_history(&conn, "p", "a", &config).unwrap(),
        json!({"x-t1": graph})
    );
    assert_eq!(
        stored_history(&conn, "p", "a", &json!({"backend":"FIXTURE"})).unwrap(),
        json!({})
    );
    assert_eq!(
        stored_history(&conn, "p", "a", &json!({})).unwrap(),
        json!({})
    );
    // A forced fetch reads none; its repair keeps the original's cache.
    let action = Fetch::new(&conn, "p", "a", &backend("electrum"), false, "fixture").unwrap();
    assert_eq!(action.history, json!({"x-t1": graph}));
    assert_eq!(action.repair(&conn).unwrap().history, action.history);
    let forced = Fetch::new(&conn, "p", "a", &backend("electrum"), true, "fixture").unwrap();
    assert_eq!(forced.history, json!({}));
    conn.execute("UPDATE backends SET kind='esplora'", [])
        .unwrap();
    assert_eq!(stored_history(&conn, "p", "a", &config).unwrap(), json!({}));
}

/// `identities_for_wallet`'s BDK instances: a Samourai child observes under
/// its parent with its role key and `change_descriptor`'s branch keys, a
/// parent observes nothing, xpubs one instance per script type, descriptors
/// one default instance.
#[test]
fn bdk_identities_follow_python_source_keys() {
    let keys = |config: Value| -> Vec<(String, String, Option<Vec<String>>)> {
        bdk_identities("w", &config)
            .into_iter()
            .map(|i| (i.logical_wallet_id, i.source_key, i.branch_keys))
            .collect()
    };
    let child = json!({"role":"child","parent_wallet_id":" parent ","section":"postmix",
        "script_type":"p2wpkh","root_path":"m/84'/0'/2147483646'"});
    assert_eq!(
        keys(json!({"descriptor":"d","change_descriptor":"c","samourai":child})),
        [(
            "parent".into(),
            "samourai:postmix:p2wpkh:m/84'/0'/2147483646'".into(),
            Some(vec!["receive".into(), "change".into()])
        )]
    );
    assert_eq!(
        keys(json!({"descriptor":"d","samourai":{"role":"child"}})),
        [(
            "w".into(),
            "samourai:source:descriptor:account".into(),
            Some(vec!["receive".into()])
        )]
    );
    assert!(keys(json!({"descriptor":"d","samourai":{"role":" parent "}})).is_empty());
    // Another role, or metadata that is not an object, observes as usual.
    for samourai in [json!({"role":"other"}), json!("child")] {
        assert_eq!(
            keys(json!({"descriptor":"d","samourai":samourai})),
            [("w".into(), "descriptor:default".into(), None)]
        );
    }
    assert_eq!(
        keys(json!({"xpub":"x","script_types":[" P2WPKH ","p2tr","",null],"descriptor":"d"})),
        [
            ("w".into(), "xpub:p2tr".into(), None),
            ("w".into(), "xpub:p2wpkh".into(), None)
        ]
    );
    assert_eq!(
        keys(json!({"xpub":"x","script_types":[],"descriptor":"d"})),
        [("w".into(), "descriptor:default".into(), None)]
    );
    assert!(keys(json!({"xpub":"x","script_types":[]})).is_empty());
}

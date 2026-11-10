use super::*;
use kassiber_net::{
    test_support::{FakeElectrum, FakeProvider},
    Ledger, Offline,
};
use std::time::Duration;

fn wallet(name: &str, gap: u32) -> Value {
    let vectors: Value =
        serde_json::from_str(include_str!("../../tests/vectors/python_descriptors.json")).unwrap();
    let mut config = vectors["plans"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == name)
        .unwrap()["config"]
        .clone();
    config["gap_limit"] = json!(gap);
    config
}
fn scope() -> Scope {
    Scope {
        database_instance: "db".into(),
        profile_id: "p".into(),
        wallet_id: "w".into(),
        environment_id: "main".into(),
        source_revision: "v1".into(),
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn gate(state: Offline) -> Gate {
    Gate::new(move || state, Ledger::default())
}
fn action(
    kind: &str,
    url: &str,
    checkpoint: Value,
    config: Option<Value>,
) -> CompatibilityDiscovery {
    CompatibilityDiscovery::new(
        scope(),
        &config.unwrap_or_else(|| wallet("wpkh", 3)),
        &json!({"kind":kind,"url":url,"batch_size":2}),
        &checkpoint,
        "fixture",
    )
    .unwrap()
}

#[test]
fn gap_resets_at_used_addresses_and_scans_branches_independently() {
    let plan = descriptors::load(&wallet("wpkh", 3))
        .map_err(descriptors::Error::into_app)
        .unwrap()
        .unwrap();
    let mut calls = vec![];
    let targets = scan_plan(
        &plan,
        &json!({}),
        2,
        &Cancellation::default(),
        deadline(),
        &mut |_| {},
        |batch| {
            calls.push(
                batch
                    .iter()
                    .map(|t| {
                        (
                            t["branch_index"].as_u64().unwrap(),
                            t["address_index"].as_u64().unwrap(),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
            Ok(batch
                .iter()
                .map(|t| {
                    t["branch_index"] == 0 && [1, 4].contains(&t["address_index"].as_u64().unwrap())
                })
                .collect())
        },
    )
    .unwrap();
    assert_eq!(targets.iter().filter(|t| t["branch_index"] == 0).count(), 8);
    assert_eq!(targets.iter().filter(|t| t["branch_index"] == 1).count(), 3);
    assert_eq!(
        calls,
        vec![
            vec![(0, 0), (0, 1)],
            vec![(0, 2), (0, 3)],
            vec![(0, 4), (0, 5)],
            vec![(0, 6), (0, 7)],
            vec![(1, 0), (1, 1)],
            vec![(1, 2), (1, 3)]
        ]
    );
}

#[test]
fn highest_used_prefix_is_retained_without_probes_and_progress_counts_whole_batches() {
    let plan = descriptors::load(&wallet("wpkh", 3))
        .map_err(descriptors::Error::into_app)
        .unwrap()
        .unwrap();
    let mut events = vec![];
    let mut calls = vec![];
    let targets = scan_plan(
        &plan,
        &json!({"highest_used":{"0":"٤","1":-1}}),
        2,
        &Cancellation::default(),
        deadline(),
        &mut |v| events.push(v),
        |batch| {
            calls.push((
                batch[0]["branch_index"].clone(),
                batch[0]["address_index"].clone(),
            ));
            Ok(vec![false; batch.len()])
        },
    )
    .unwrap();
    assert_eq!(targets.len(), 11);
    assert_eq!(
        calls,
        vec![
            (json!(0), json!(5)),
            (json!(0), json!(7)),
            (json!(1), json!(0)),
            (json!(1), json!(2))
        ]
    );
    assert_eq!(
        events,
        vec![
            json!({"phase":"discovery","targets_checked":2,"retained_targets":7,"branch_index":0,"gap_limit":3,"unused_streak":2,"processed":2}),
            json!({"phase":"discovery","targets_checked":4,"retained_targets":8,"branch_index":0,"gap_limit":3,"unused_streak":3,"processed":4}),
            json!({"phase":"discovery","targets_checked":6,"retained_targets":10,"branch_index":1,"gap_limit":3,"unused_streak":2,"processed":6}),
            json!({"phase":"discovery","targets_checked":8,"retained_targets":11,"branch_index":1,"gap_limit":3,"unused_streak":3,"processed":8})
        ]
    );
}

#[test]
fn fixed_and_gap_one_branches_emit_index_zero_without_usage_queries() {
    for config in [wallet("wpkh-fixed", 20), wallet("wpkh", 1)] {
        let plan = descriptors::load(&config)
            .map_err(descriptors::Error::into_app)
            .unwrap()
            .unwrap();
        let targets = scan_plan(
            &plan,
            &json!({"highest_used":{"0":9}}),
            2,
            &Cancellation::default(),
            deadline(),
            &mut |_| panic!("unexpected progress"),
            |_| panic!("unexpected query"),
        )
        .unwrap();
        assert_eq!(targets.len(), plan.branches.len());
        assert!(targets.iter().all(|t| t["address_index"] == 0));
    }
}

#[test]
fn usage_cardinality_and_exhausted_checkpoint_fail_closed() {
    let plan = descriptors::load(&wallet("wpkh", 3))
        .map_err(descriptors::Error::into_app)
        .unwrap()
        .unwrap();
    let error = scan_plan(
        &plan,
        &json!({}),
        2,
        &Cancellation::default(),
        deadline(),
        &mut |_| {},
        |_| Ok(vec![]),
    )
    .unwrap_err();
    assert_eq!(
        error.message,
        "Compatibility descriptor discovery returned an unexpected number of usage checks"
    );
    let error = scan_plan(
        &plan,
        &json!({"highest_used":{"0":2147483647}}),
        2,
        &Cancellation::default(),
        deadline(),
        &mut |_| {},
        |_| panic!("unexpected query"),
    )
    .unwrap_err();
    assert_eq!(error.code, "validation");
}

#[test]
fn cancelled_discovery_returns_no_partial_result_and_stops_before_the_next_batch() {
    let plan = descriptors::load(&wallet("wpkh", 3))
        .map_err(descriptors::Error::into_app)
        .unwrap()
        .unwrap();
    let cancel = Cancellation::default();
    let mut calls = 0;
    let result = scan_plan(
        &plan,
        &json!({}),
        2,
        &cancel,
        deadline(),
        &mut |_| cancel.cancel(),
        |batch| {
            calls += 1;
            Ok(vec![false; batch.len()])
        },
    );
    assert_eq!(result.unwrap_err().code, "cancelled");
    assert_eq!(calls, 1);
}

#[test]
fn every_discovery_path_requires_exact_action_consent_and_current_scope_before_traffic() {
    for host in ["127.0.0.1:1", "never-resolve.invalid:50001"] {
        for (kind, scheme) in [
            ("esplora", "http"),
            ("esplora", "https"),
            ("electrum", "tcp"),
            ("electrum", "ssl"),
        ] {
            let a = action(kind, &format!("{scheme}://{host}"), json!({}), None);
            let other = action(kind, &format!("{scheme}://{host}"), json!({}), None);
            let grant = DiscoveryConsent::explicit_trigger(&a);
            let other_grant = DiscoveryConsent::explicit_trigger(&other);
            let gate = gate(Offline::Online);
            for proof in [None, Some(&other_grant)] {
                assert_eq!(
                    a.run(
                        &gate,
                        &scope(),
                        proof,
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
fn offline_refuses_both_paths_including_cached_and_fixed_scans() {
    for (kind, scheme) in [("esplora", "http"), ("electrum", "tcp")] {
        for config in [
            wallet("wpkh", 3),
            wallet("wpkh-fixed", 3),
            wallet("wpkh", 1),
        ] {
            let a = action(
                kind,
                &format!("{scheme}://127.0.0.1:1"),
                json!({}),
                Some(config),
            );
            let grant = DiscoveryConsent::explicit_trigger(&a);
            for state in [Offline::Offline, Offline::Unavailable] {
                let gate = gate(state);
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
                    "network_egress_disabled"
                );
                assert!(gate.attempts().records.is_empty());
            }
        }
    }
}

#[test]
fn cancellation_and_expired_deadlines_refuse_before_either_transport() {
    for (kind, scheme) in [("esplora", "http"), ("electrum", "tcp")] {
        let a = action(kind, &format!("{scheme}://127.0.0.1:1"), json!({}), None);
        let grant = DiscoveryConsent::explicit_trigger(&a);
        let gate = gate(Offline::Online);
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            a.run(&gate, &scope(), Some(&grant), deadline(), &cancel, |_| {})
                .unwrap_err()
                .code,
            "cancelled"
        );
        assert_eq!(
            a.run(
                &gate,
                &scope(),
                Some(&grant),
                Instant::now(),
                &Cancellation::default(),
                |_| {}
            )
            .unwrap_err()
            .code,
            "backend_timeout"
        );
        assert!(gate.attempts().records.is_empty());
    }
}

#[test]
fn esplora_cached_positive_membership_avoids_probes_and_http_workers_are_bounded() {
    let server =
        FakeProvider::new(r#"{"chain_stats":{"tx_count":0},"mempool_stats":{"tx_count":0}}"#)
            .unwrap();
    let config = wallet("wpkh", 3);
    let plan = descriptors::load(&config)
        .map_err(descriptors::Error::into_app)
        .unwrap()
        .unwrap();
    let hash = script_hash(
        plan.derive(Some(0), 0, 1)
            .map_err(descriptors::Error::into_app)
            .unwrap()[0]["script_pubkey"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let a = action(
        "esplora",
        server.base(),
        json!({"esplora_scripthashes":{hash:{"tx_count":1}}}),
        Some(config),
    );
    let grant = DiscoveryConsent::explicit_trigger(&a);
    let mut events = vec![];
    let result = a
        .run(
            &gate(Offline::Online),
            &scope(),
            Some(&grant),
            deadline(),
            &Cancellation::default(),
            |v| events.push(v),
        )
        .unwrap();
    assert_eq!(result["targets"].as_array().unwrap().len(), 7);
    assert_eq!(events.last().unwrap()["targets_checked"], 8);
    let requests = server.requests();
    assert_eq!(requests.len(), 7);
    assert!(requests
        .iter()
        .all(|r| r.contains("User-Agent: kassiber/fixture\r\n")));
    // Overscan is observable in Python: checked includes the whole batch while
    // retained targets stop exactly at the unused gap.
    assert_eq!(result["history_cache"], json!({}));
}

#[test]
fn esplora_mempool_history_resets_gap_and_invalid_counters_do_not_hide_usage() {
    let server = FakeProvider::replies(vec![
        (200, r#"{"mempool_stats":{"tx_count":1}}"#.into()),
        (200, r#"{"chain_stats":{"tx_count":0}}"#.into()),
    ])
    .unwrap();
    let config = wallet("wpkh", 2);
    let a = CompatibilityDiscovery::new(
        scope(),
        &config,
        &json!({"kind":"mempool","url":server.base(),"batch_size":1}),
        &json!({}),
        "fixture",
    )
    .unwrap();
    let grant = DiscoveryConsent::explicit_trigger(&a);
    let result = a
        .run(
            &gate(Offline::Online),
            &scope(),
            Some(&grant),
            deadline(),
            &Cancellation::default(),
            |_| {},
        )
        .unwrap();
    assert_eq!(result["targets"].as_array().unwrap().len(), 5);
    assert_eq!(server.requests().len(), 5);
    let malformed =
        FakeProvider::new(r#"{"chain_stats":{"tx_count":"private-invalid-counter"}}"#).unwrap();
    let a = action("esplora", malformed.base(), json!({}), None);
    let grant = DiscoveryConsent::explicit_trigger(&a);
    let error = a
        .run(
            &gate(Offline::Online),
            &scope(),
            Some(&grant),
            deadline(),
            &Cancellation::default(),
            |_| {},
        )
        .unwrap_err();
    assert_eq!(error.code, "protocol_error");
    assert!(!error.message.contains("private"));
}

#[test]
fn electrum_status_batches_preserve_cached_non_null_and_requery_null() {
    let server = FakeElectrum::script(vec![
        (1, "{\"id\":1,\"result\":[\"fixture\",\"1.6\"]}\n".into()),
        (1, "{\"id\":2,\"result\":null}\n".into()),
        (
            2,
            "{\"id\":4,\"result\":null}\n{\"id\":3,\"result\":null}\n".into(),
        ),
        (
            2,
            "{\"id\":6,\"result\":null}\n{\"id\":5,\"result\":null}\n".into(),
        ),
        (
            2,
            "{\"id\":8,\"result\":null}\n{\"id\":7,\"result\":null}\n".into(),
        ),
    ])
    .unwrap();
    let config = wallet("wpkh", 3);
    let plan = descriptors::load(&config)
        .map_err(descriptors::Error::into_app)
        .unwrap()
        .unwrap();
    let first = plan
        .derive(Some(0), 0, 2)
        .map_err(descriptors::Error::into_app)
        .unwrap();
    let h0 = script_hash(first[0]["script_pubkey"].as_str().unwrap()).unwrap();
    let h1 = script_hash(first[1]["script_pubkey"].as_str().unwrap()).unwrap();
    let a = action(
        "electrum",
        server.url(),
        json!({"electrum_scripthash_statuses":{h0:"",h1.clone():null}}),
        Some(config),
    );
    let grant = DiscoveryConsent::explicit_trigger(&a);
    let result = a
        .run(
            &gate(Offline::Online),
            &scope(),
            Some(&grant),
            deadline(),
            &Cancellation::default(),
            |_| {},
        )
        .unwrap();
    assert_eq!(result["targets"].as_array().unwrap().len(), 7);
    let requests = server.requests();
    assert_eq!(requests.len(), 8);
    let first: Value = serde_json::from_str(&requests[1]).unwrap();
    assert_eq!(first["method"], "blockchain.scripthash.subscribe");
    assert_eq!(first["params"], json!([h1]));
}

#[test]
fn proxy_failure_never_retries_the_destination_directly() {
    let http = FakeProvider::new("{}").unwrap();
    let electrum = FakeElectrum::script(vec![]).unwrap();
    for (kind, url) in [("esplora", http.base()), ("electrum", electrum.url())] {
        let a = CompatibilityDiscovery::new(
            scope(),
            &wallet("wpkh", 2),
            &json!({"kind":kind,"url":url,"tor_proxy":"socks5h://127.0.0.1:1"}),
            &json!({}),
            "fixture",
        )
        .unwrap();
        let grant = DiscoveryConsent::explicit_trigger(&a);
        assert!(a
            .run(
                &gate(Offline::Online),
                &scope(),
                Some(&grant),
                deadline(),
                &Cancellation::default(),
                |_| {}
            )
            .is_err());
    }
    assert!(http.requests().is_empty());
    assert!(electrum.requests().is_empty());
}

#[test]
fn unsupported_sources_spending_keys_and_network_conflicts_refuse_before_clients() {
    let error = CompatibilityDiscovery::new(
        scope(),
        &wallet("empty", 3),
        &json!({"kind":"esplora","url":"http://127.0.0.1:1"}),
        &json!({}),
        "fixture",
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "not_implemented");
    for kind in ["bitcoinrpc", "coreln", "lnd", "nwc"] {
        assert_eq!(
            CompatibilityDiscovery::new(
                scope(),
                &wallet("wpkh", 3),
                &json!({"kind":kind,"url":"http://127.0.0.1:1"}),
                &json!({}),
                "fixture"
            )
            .err()
            .unwrap()
            .code,
            "not_implemented"
        );
    }
    assert_eq!(
        CompatibilityDiscovery::new(
            scope(),
            &wallet("private-wpkh", 3),
            &json!({"kind":"esplora","url":"http://127.0.0.1:1"}),
            &json!({}),
            "fixture"
        )
        .err()
        .unwrap()
        .code,
        "wallet_spending_private_material"
    );
    let mut config = wallet("wpkh", 3);
    config["sp_descriptor"] = json!("synthetic-scan-material");
    assert_eq!(
        CompatibilityDiscovery::new(
            scope(),
            &config,
            &json!({"kind":"esplora","url":"http://127.0.0.1:1"}),
            &json!({}),
            "fixture"
        )
        .err()
        .unwrap()
        .code,
        "not_implemented"
    );
    for backend in [
        json!({"kind":"esplora","url":"http://127.0.0.1:1","network":"test"}),
        json!({"kind":"esplora","url":"http://127.0.0.1:1","config_json":"{\"chain_instance_id\":\"different\"}"}),
    ] {
        assert!(CompatibilityDiscovery::new(
            scope(),
            &wallet("wpkh", 3),
            &backend,
            &json!({}),
            "fixture"
        )
        .is_err());
    }
}

#[test]
fn script_hash_and_python_checkpoint_integer_coercions_are_exact() {
    assert_eq!(
        script_hash("0014ac").unwrap(),
        "d86b488b4a2a804029c1dd339e903f96ebd5bdb18cbb228ee7591c8f5042242f"
    );
    assert_eq!(script_hash("00ff").unwrap(), script_hash("00FF").unwrap());
    assert_eq!(script_hash("invalid").unwrap_err().code, "validation");
    for (value, expected) in [
        (json!("٤"), 4),
        (json!(true), 1),
        (json!(3.9), 3),
        (json!(-3.9), -3),
    ] {
        assert_eq!(integer(&value), Some(expected.into()));
    }
    assert_eq!(integer(&json!("invalid")), None);
    for stats in [
        json!([]),
        json!({"chain_stats":[1]}),
        json!({"mempool_stats":{"tx_count":"secret"}}),
    ] {
        assert_eq!(has_history(&stats).unwrap_err().code, "protocol_error");
    }
}

#[test]
fn completed_action_grants_cannot_be_replayed_even_for_a_fixed_descriptor() {
    let a = action(
        "esplora",
        "http://127.0.0.1:1",
        json!({}),
        Some(wallet("wpkh-fixed", 3)),
    );
    let grant = DiscoveryConsent::explicit_trigger(&a);
    let gate = gate(Offline::Online);
    assert!(a
        .run(
            &gate,
            &scope(),
            Some(&grant),
            deadline(),
            &Cancellation::default(),
            |_| {}
        )
        .is_ok());
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
}

#[test]
fn revocation_between_http_batches_prevents_further_requests() {
    let server = FakeProvider::new(r#"{"chain_stats":{"tx_count":0}}"#).unwrap();
    let a = action("esplora", server.base(), json!({}), None);
    let grant = DiscoveryConsent::explicit_trigger(&a);
    let error = a
        .run(
            &gate(Offline::Online),
            &scope(),
            Some(&grant),
            deadline(),
            &Cancellation::default(),
            |_| grant.revoke(),
        )
        .unwrap_err();
    assert_eq!(error.code, "interaction_required");
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn esplora_large_backend_batches_are_capped_at_eight_targets() {
    let server = FakeProvider::new(r#"{"chain_stats":{"tx_count":0}}"#).unwrap();
    let action = CompatibilityDiscovery::new(
        scope(),
        &wallet("wpkh", 3),
        &json!({"kind":"esplora","url":server.base(),"batch_size":100}),
        &json!({}),
        "fixture",
    )
    .unwrap();
    let grant = DiscoveryConsent::explicit_trigger(&action);
    let mut events = vec![];
    let result = action
        .run(
            &gate(Offline::Online),
            &scope(),
            Some(&grant),
            deadline(),
            &Cancellation::default(),
            |event| events.push(event),
        )
        .unwrap();
    assert_eq!(result["targets"].as_array().unwrap().len(), 6);
    assert_eq!(
        events
            .iter()
            .map(|event| event["targets_checked"].clone())
            .collect::<Vec<_>>(),
        vec![json!(8), json!(16)]
    );
    assert_eq!(server.requests().len(), 16);
}

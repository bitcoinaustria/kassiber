use super::*;
use kassiber_net::{
    test_support::{FakeElectrum, FakeProvider},
    Gate, Ledger, Offline,
};
use serde_json::json;
fn gate(offline: Offline) -> Gate {
    Gate::new(move || offline, Ledger::default())
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}
fn backend(url: &str, electrum: bool) -> Backend {
    Backend::new(
        &json!({"name":"fixture", "url":url, "username":"user", "password":"secret"}),
        electrum,
    )
    .unwrap()
}

#[test]
fn every_client_refuses_without_consent_and_offline_before_any_attempt() {
    // An unused loopback port and a non-loopback name exercise connect and DNS
    // paths; the independent attempt buffer must remain empty for both.
    for host in ["127.0.0.1:1", "never-resolve.invalid:50001"] {
        for secure in [false, true] {
            for offline in [Offline::Online, Offline::Offline, Offline::Unavailable] {
                let gate = gate(offline);
                let http_backend = backend(
                    &format!("{}://{host}", if secure { "https" } else { "http" }),
                    false,
                );
                let proof = Consent::explicit_trigger(http_backend.request());
                let http = HttpClient::new(http_backend.clone(), gate.clone(), "test");
                let core = CoreClient::new(http_backend, gate.clone(), "test");
                for consent in [None, Some(&proof)] {
                    if offline == Offline::Online && consent.is_some() {
                        continue;
                    }
                    let expected = if consent.is_none() {
                        "interaction_required"
                    } else {
                        "network_egress_disabled"
                    };
                    assert_eq!(
                        http.get_json("blocks", consent, deadline())
                            .unwrap_err()
                            .code,
                        expected
                    );
                    assert_eq!(
                        http.get_text("blocks/tip/height", consent, deadline())
                            .unwrap_err()
                            .code,
                        expected
                    );
                    assert_eq!(http.esplora_tip_hash(consent, deadline()), None);
                    assert_eq!(
                        http.probe(consent, deadline()).err().unwrap().code,
                        expected
                    );
                    assert_eq!(
                        core.call("getblockchaininfo", None, None, consent, deadline())
                            .unwrap_err()
                            .code,
                        expected
                    );
                    let socket = backend(
                        &format!("{}://{host}", if secure { "ssl" } else { "tcp" }),
                        true,
                    );
                    let grant = consent.map(|_| Consent::explicit_trigger(socket.request()));
                    assert_eq!(
                        ElectrumClient::connect(socket, &gate, grant, deadline())
                            .err()
                            .unwrap()
                            .code,
                        expected
                    );
                }
                assert!(gate.attempts().records.is_empty());
                assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
            }
        }
    }
}
#[test]
fn electrum_batches_reorder_ignore_notifications_and_split_at_configured_size() {
    let server = FakeElectrum::script(vec![
        (1,"{\"id\":1,\"result\":[\"fixture\",\"1.6\"]}\n".into()),
        (2,"{\"method\":\"notification\"}\n{\"id\":3,\"result\":\"third\"}\n{\"id\":3,\"result\":\"duplicate\"}\n{\"id\":2,\"result\":\"second\"}\n".into()),
        (1,"{\"id\":4,\"result\":\"fourth\"}\n".into()),
    ]).unwrap();
    let config = Backend::new(
        &json!({"name":"fixture","url":server.url(),"batch_size":2}),
        true,
    )
    .unwrap();
    let proof = Consent::explicit_trigger(config.request());
    let mut client =
        ElectrumClient::connect(config, &gate(Offline::Online), Some(proof), deadline()).unwrap();
    let requests = vec![("blockchain.transaction.get".into(), json!(["tx", true])); 3];
    assert_eq!(
        client.batch_call(&requests, deadline()).unwrap(),
        vec![json!("second"), json!("third"), json!("fourth")]
    );
    let lines = server.requests();
    let first: Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(
        first,
        json!({"jsonrpc":"2.0","id":1,"method":"server.version","params":["Kassiber","1.6"]})
    );
    assert_eq!(serde_json::from_str::<Value>(&lines[3]).unwrap()["id"], 4);
}
#[test]
fn electrum_policy_change_stops_existing_session_and_write() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let server =
        FakeElectrum::script(vec![(1, "{\"id\":1,\"result\":\"ready\"}\n".into())]).unwrap();
    let config = backend(server.url(), true);
    let proof = Consent::explicit_trigger(config.request());
    let offline = Arc::new(AtomicBool::new(false));
    let current = offline.clone();
    let gate = Gate::new(
        move || {
            if current.load(Ordering::SeqCst) {
                Offline::Offline
            } else {
                Offline::Online
            }
        },
        Ledger::default(),
    );
    let mut client =
        ElectrumClient::connect(config, &gate, Some(proof.clone()), deadline()).unwrap();
    offline.store(true, Ordering::SeqCst);
    assert_eq!(
        client
            .call("server.banner", None, deadline())
            .unwrap_err()
            .code,
        "network_egress_disabled"
    );
    offline.store(false, Ordering::SeqCst);
    proof.revoke();
    assert_eq!(
        client
            .call("server.banner", None, deadline())
            .unwrap_err()
            .code,
        "interaction_required"
    );
    assert_eq!(server.requests().len(), 1);
}
#[test]
fn electrum_protocol_and_rpc_errors_are_typed_and_redacted() {
    for (reply, code) in [
        ("not json\n", "app_error"),
        ("[]\n", "app_error"),
        (
            "{\"id\":1,\"error\":{\"code\":-1,\"message\":\"token=private\"}}\n",
            "electrum_rpc_error",
        ),
    ] {
        let server = FakeElectrum::script(vec![(1, reply.into())]).unwrap();
        let config = backend(server.url(), true);
        let proof = Consent::explicit_trigger(config.request());
        let error =
            ElectrumClient::connect(config, &gate(Offline::Online), Some(proof), deadline())
                .err()
                .unwrap();
        assert_eq!(error.code, code);
        assert!(!error.message.contains("private"));
    }
}
#[test]
fn esplora_headers_paths_json_text_and_auth_match_python() {
    let server =
        FakeProvider::replies(vec![(200, "{\"height\":42}".into()), (200, "42".into())]).unwrap();
    let config = Backend::new(
        &json!({"url":format!("{}/api?key=x",server.base()),"token":"secret"}),
        false,
    )
    .unwrap();
    let proof = Consent::explicit_trigger(config.request());
    let client = HttpClient::new(config, gate(Offline::Online), "fixture");
    assert_eq!(
        client
            .get_json("tx/test", Some(&proof), deadline())
            .unwrap(),
        json!({"height":42})
    );
    assert_eq!(
        client
            .get_text("blocks/tip/height", Some(&proof), deadline())
            .unwrap(),
        "42"
    );
    let requests = server.requests();
    assert!(requests[0].starts_with("GET /api/tx/test?key=x HTTP/1.1\r\n"));
    assert!(requests[0].contains("Authorization: Bearer secret\r\n"));
    assert!(requests[0].contains("User-Agent: kassiber/fixture\r\n"));
    assert!(requests[1].contains("Accept: text/plain\r\n"));
}
#[test]
fn core_request_shape_wallet_path_and_typed_http_error_mapping() {
    let server = FakeProvider::replies(vec![
        (200,"{\"id\":\"kassiber-getwalletinfo\",\"result\":{\"ok\":true},\"error\":null}".into()),
        (500,"{\"id\":\"kassiber-getblockchaininfo\",\"result\":null,\"error\":{\"code\":-8,\"message\":\"rescan failed token=secret\"}}".into()),
        (500,"{\"id\":\"wrong\",\"error\":{\"code\":-8}}".into()),
        (200,"{\"error\":{\"code\":-8,\"category\":\"rescan failed\"}}".into()),
        (500,"{\"id\":\"kassiber-getblockchaininfo\",\"error\":{\"code\":-8,\"category\":\"rescan failed\"}}".into()),
    ]).unwrap();
    let config = backend(&format!("{}/rpc?x=1", server.base()), false);
    let proof = Consent::explicit_trigger(config.request());
    let client = CoreClient::new(config, gate(Offline::Online), "fixture");
    assert_eq!(
        client
            .call(
                "getwalletinfo",
                None,
                Some("a /ü"),
                Some(&proof),
                deadline()
            )
            .unwrap(),
        json!({"ok":true})
    );
    let request = server.requests()[0].clone();
    assert!(
        request.starts_with("POST /rpc/wallet/a%20%2F%C3%BC?x=1 HTTP/1.1\r\n"),
        "{request}"
    );
    assert!(request.contains("Authorization: Basic dXNlcjpzZWNyZXQ="));
    let payload: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        payload,
        json!({"jsonrpc":"1.0","id":"kassiber-getwalletinfo","method":"getwalletinfo","params":[]})
    );
    let error = client
        .call("getblockchaininfo", None, None, Some(&proof), deadline())
        .unwrap_err();
    assert_eq!(
        error.message,
        "Bitcoin Core RPC getblockchaininfo failed (-8): rescan failed"
    );
    assert_eq!(
        error.details,
        Some(details(
            json!({"rpc_method":"getblockchaininfo","rpc_error_code":-8})
        ))
    );
    assert_eq!(
        client
            .call("getblockchaininfo", None, None, Some(&proof), deadline())
            .unwrap_err()
            .message,
        "Bitcoin Core returned an invalid RPC error response"
    );
    // Python recognizes its sanitized category on a successful HTTP envelope,
    // but an HTTP error cannot assert a category without a matching message.
    for message in [
        "Bitcoin Core RPC getblockchaininfo failed (-8): rescan failed",
        "Bitcoin Core RPC getblockchaininfo failed (-8)",
    ] {
        assert_eq!(
            client
                .call("getblockchaininfo", None, None, Some(&proof), deadline())
                .unwrap_err()
                .message,
            message
        );
    }
}
#[test]
fn retry_status_attempt_limit_and_nonretryable_errors_match_python() {
    for (status, count, code) in [
        (429, 3, "rate_limited"),
        (503, 3, "rate_limited"),
        (404, 1, "app_error"),
    ] {
        let server = FakeProvider::script(vec![(
            status,
            vec![("Retry-After".into(), "0".into())],
            "no".into(),
        )])
        .unwrap();
        let config = backend(server.base(), false);
        let proof = Consent::explicit_trigger(config.request());
        let client = HttpClient::new(config, gate(Offline::Online), "fixture");
        assert_eq!(
            client
                .get_json("blocks", Some(&proof), deadline())
                .unwrap_err()
                .code,
            code
        );
        assert_eq!(server.requests().len(), count);
    }
}
#[test]
fn checked_http_refuses_before_transport_and_rechecks_before_retry() {
    for stop_before_request in [true, false] {
        let server = FakeProvider::script(vec![(
            503,
            vec![("Retry-After".into(), "0".into())],
            "retry".into(),
        )])
        .unwrap();
        let config = backend(server.base(), false);
        let proof = Consent::explicit_trigger(config.request());
        let gate = gate(Offline::Online);
        let client = HttpClient::new(config, gate.clone(), "fixture");
        let check = || {
            if stop_before_request || !server.requests().is_empty() {
                Err(AppError::new("cancelled", "Request ended"))
            } else {
                Ok(())
            }
        };
        assert_eq!(
            client
                .get_json_checked("tx/reference", Some(&proof), deadline(), &check)
                .unwrap_err()
                .code,
            "cancelled"
        );
        assert_eq!(server.requests().len(), usize::from(!stop_before_request));
        if stop_before_request {
            assert!(gate.attempts().records.is_empty());
            assert!(gate.ledger().snapshot(0, 100).records.is_empty());
        }
    }
}
#[test]
fn checked_http_cancellation_interrupts_retry_backoff() {
    let server = FakeProvider::script(vec![(
        503,
        vec![("Retry-After".into(), "2".into())],
        "retry".into(),
    )])
    .unwrap();
    let config = backend(server.base(), false);
    let proof = Consent::explicit_trigger(config.request());
    let client = HttpClient::new(config, gate(Offline::Online), "fixture");
    let started = Instant::now();
    let check = || {
        if started.elapsed() >= Duration::from_millis(150) {
            Err(AppError::new("cancelled", "Request ended during backoff"))
        } else {
            Ok(())
        }
    };
    assert_eq!(
        client
            .get_json_checked("tx/reference", Some(&proof), deadline(), &check)
            .unwrap_err()
            .code,
        "cancelled"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn socket_forms_and_proxy_trust_scope_are_inert() {
    assert_eq!(
        config::socket_endpoint("example.com").unwrap(),
        ("example.com".into(), 50002, true)
    );
    assert_eq!(
        config::socket_endpoint("tcp://[::1]").unwrap(),
        ("::1".into(), 50001, false)
    );
    assert!(config::socket_endpoint("tls://example.com").is_err());
    let gate = gate(Offline::Online);
    let a = backend("http://127.0.0.1:1", false);
    let b = backend("http://127.0.0.1:2", false);
    let proof = Consent::explicit_trigger(a.request());
    let client = HttpClient::new(b, gate.clone(), "fixture");
    assert_eq!(
        client.probe(Some(&proof), deadline()).err().unwrap().code,
        "interaction_required"
    );
    let onion = backend("http://fixture.onion", false);
    let proof = Consent::explicit_trigger(onion.request());
    assert_eq!(
        HttpClient::new(onion, gate.clone(), "fixture")
            .probe(Some(&proof), deadline())
            .err()
            .unwrap()
            .code,
        "network_proxy_required"
    );
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn every_probe_requires_exact_action_arguments_and_book_and_refuses_offline() {
    use probes::{ProbeConsent, Probes};
    for kind in probes::KINDS {
        for offline in [Offline::Online, Offline::Offline, Offline::Unavailable] {
            let gate = gate(offline);
            let service = Probes::new(gate.clone(), "fixture");
            let args = json!({"url":"http://127.0.0.1:1","username":"u","password":"p"});
            assert_eq!(
                service
                    .run(kind, &args, "book", None, None)
                    .unwrap_err()
                    .code,
                "interaction_required"
            );
            let grant = ProbeConsent::explicit(kind, &args, "book");
            assert_eq!(
                service
                    .run(kind, &args, "other-book", None, Some(&grant))
                    .unwrap_err()
                    .code,
                "interaction_required"
            );
            assert_eq!(
                service
                    .run(kind, &json!({}), "book", None, Some(&grant))
                    .unwrap_err()
                    .code,
                "interaction_required"
            );
            if offline != Offline::Online && *kind != "ui.backends.detect_core" {
                let args = match *kind {
                    "ui.backends.electrum.test" => json!({"url":"tcp://127.0.0.1:1"}),
                    "ui.backends.btcpay.test" => {
                        json!({"url":"http://127.0.0.1:1","api_key":"test"})
                    }
                    _ => args,
                };
                let grant = ProbeConsent::explicit(kind, &args, "book");
                let result = service.run(kind, &args, "book", None, Some(&grant));
                match result {
                    Ok(v) => assert!(v["ok"] == false || v["reachable"] == false),
                    Err(e) => assert_eq!(e.code, "network_egress_disabled"),
                }
            }
            assert!(gate.attempts().records.is_empty());
            assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
        }
    }
}

#[test]
fn probes_use_one_session_and_one_http_or_store_request() {
    use probes::{ProbeConsent, Probes};
    let server = FakeProvider::new("[]").unwrap();
    let service = Probes::new(gate(Offline::Online), "fixture");
    for (kind, args) in [
        ("ui.backends.http.test", json!({"url":server.base()})),
        (
            "ui.backends.btcpay.test",
            json!({"url":server.base(),"api_key":"secret"}),
        ),
    ] {
        let grant = ProbeConsent::explicit(kind, &args, "book");
        assert_eq!(
            service
                .run(kind, &args, "book", None, Some(&grant))
                .unwrap()["ok"],
            true
        );
    }
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].starts_with("GET /api/v1/stores HTTP/1.1"));
    assert!(requests[1].contains("Authorization: token secret"));
    let server = FakeElectrum::script(vec![
        (1, "{\"id\":1,\"result\":[\"fixture\",\"1.6\"]}\n".into()),
        (1, "{\"id\":2,\"result\":\"banner\"}\n".into()),
    ])
    .unwrap();
    let kind = "ui.backends.electrum.test";
    let args = json!({"url":server.url()});
    let grant = ProbeConsent::explicit(kind, &args, "book");
    let result = service
        .run(kind, &args, "book", None, Some(&grant))
        .unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(result["logs"][4], "Server version: ['fixture', '1.6']");
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn core_probe_preserves_unknown_pruned_coverage_and_single_snapshot() {
    let server=FakeProvider::replies(vec![
        (200,json!({"result":{"chain":"main","blocks":42,"headers":42,"pruned":true,"pruneheight":10,"initialblockdownload":false}}).to_string()),
        (200,json!({"result":{"version":270000,"connections":8}}).to_string()),
        (200,json!({"result":"prunehash"}).to_string()),
        (200,json!({"result":{"time":1700000000}}).to_string()),
        (200,json!({"result":[]}).to_string()),
        (200,json!({"result":"besthash"}).to_string()),
        (200,json!({"result":{"filter":"00"}}).to_string()),
    ]).unwrap();
    let service = probes::Probes::new(gate(Offline::Online), "fixture");
    let args = json!({"url":server.base(),"username":"u","password":"p","birthday":"2023-11-15"});
    let kind = "ui.backends.bitcoinrpc.test";
    let grant = probes::ProbeConsent::explicit(kind, &args, "book");
    let result = service
        .run(kind, &args, "book", None, Some(&grant))
        .unwrap();
    assert_eq!(result["reachable"], true);
    assert_eq!(result["earliest_retained_at"], "2023-11-14T22:13:20Z");
    assert_eq!(result["rescan_start_at"], "2023-11-14T22:00:00Z");
    assert_eq!(result["birthday_covered"], Value::Null);
    assert_eq!(result["rescan_possible"], Value::Null);
    assert_eq!(result["warnings"], json!(["pruned"]));
    assert_eq!(server.requests().len(), 7);
}

#[test]
fn watch_only_methods_and_expired_deadlines_refuse_before_connection() {
    let gate = gate(Offline::Online);
    let config = backend("http://127.0.0.1:1", false);
    let proof = Consent::explicit_trigger(config.request());
    let core = CoreClient::new(config.clone(), gate.clone(), "fixture");
    assert_eq!(
        core.call(
            "sendrawtransaction",
            Some(&json!(["hex"])),
            None,
            Some(&proof),
            deadline()
        )
        .unwrap_err()
        .code,
        "validation"
    );
    let expired = Instant::now() - Duration::from_millis(1);
    assert_eq!(
        core.call("listsinceblock", None, None, Some(&proof), expired)
            .unwrap_err()
            .code,
        "backend_timeout"
    );
    assert_eq!(
        HttpClient::new(config, gate.clone(), "fixture")
            .get_json("tx", Some(&proof), expired)
            .unwrap_err()
            .code,
        "backend_timeout"
    );
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
}

#[test]
fn watch_only_wallet_administration_refuses_key_material_before_connection() {
    let gate = gate(Offline::Online);
    let config = backend("http://127.0.0.1:1", false);
    let proof = Consent::explicit_trigger(config.request());
    let core = CoreClient::new(config, gate.clone(), "fixture");
    let xprv = format!("tprv8{}", "Z".repeat(106));
    let wif = format!("K{}", "x".repeat(51));
    let refused = [
        ("createwallet", json!(["w", false])),
        ("createwallet", json!(["w"])),
        (
            "importdescriptors",
            json!([[{"desc": format!("wpkh({xprv}/0/*)")}]]),
        ),
        (
            "importdescriptors",
            json!([[{"desc": "addr(x)"}, {"desc": format!("wpkh({wif})")}]]),
        ),
        ("importdescriptors", json!([[{"range": [0, 1]}]])),
        ("importdescriptors", json!(["wpkh(x)"])),
        ("importmulti", json!([[{"scriptPubKey": {"address": "a"}}]])),
        ("importmulti", json!([[{"watchonly": true, "keys": [wif]}]])),
        (
            "importmulti",
            json!([[{"watchonly": true, "redeemscript": "51"}]]),
        ),
        ("importprivkey", json!([wif])),
    ];
    for (method, params) in refused {
        let error = core
            .call(method, Some(&params), Some("w"), Some(&proof), deadline())
            .unwrap_err();
        assert_eq!(error.code, "validation", "{method} {params}");
    }
    // The watch-only shapes pass the guard and only then meet the deadline.
    let expired = Instant::now() - Duration::from_millis(1);
    let public = format!("tpub8{}", "Z".repeat(106));
    let allowed = [
        ("loadwallet", json!(["w", true])),
        (
            "createwallet",
            json!(["w", true, true, "", false, true, true]),
        ),
        (
            "importdescriptors",
            json!([[{"desc": format!("wpkh({public}/0/*)#abcd"), "range": [0, 9]}]]),
        ),
        (
            "importmulti",
            json!([[{"scriptPubKey": {"address": "a"}, "watchonly": true}], {"rescan": true}]),
        ),
    ];
    for (method, params) in allowed {
        let error = core
            .call(method, Some(&params), Some("w"), Some(&proof), expired)
            .unwrap_err();
        assert_eq!(error.code, "backend_timeout", "{method} {params}");
    }
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
}

#[test]
fn configured_socks_proxy_failure_never_tries_target() {
    let target = FakeProvider::new("{}").unwrap();
    // The owned proxy rejects the SOCKS method negotiation. A configured
    // failing proxy must never cause a connection to the separate target.
    let proxy = FakeElectrum::script(vec![(0, "\u{5}\u{1}".into())]).unwrap();
    let config = Backend::new(
        &json!({"url":target.base(),"tor_proxy":proxy.url().replacen("tcp://", "socks5h://", 1)}),
        false,
    )
    .unwrap();
    let proof = Consent::explicit_trigger(config.request());
    assert!(HttpClient::new(config, gate(Offline::Online), "fixture")
        .get_json("blocks", Some(&proof), deadline())
        .is_err());
    assert!(target.requests().is_empty());
}

#[test]
fn probe_invalid_inputs_refuse_before_any_connection() {
    let gate = gate(Offline::Online);
    let service = probes::Probes::new(gate.clone(), "fixture");
    for (kind, args, message) in [
        (
            "ui.backends.http.test",
            json!({"url":3}),
            "url must be a string",
        ),
        (
            "ui.backends.electrum.test",
            json!({"url":"tcp://127.0.0.1:1","proxy":false}),
            "proxy must be a string",
        ),
        (
            "ui.backends.bitcoinrpc.test",
            json!({"url":"http://127.0.0.1:1","config":[]}),
            "config must be an object",
        ),
        (
            "ui.backends.bitcoinrpc.test",
            json!({"url":"http://127.0.0.1:1","birthday":"bad"}),
            "Invalid occurred_at/date value 'bad'",
        ),
    ] {
        let consent = probes::ProbeConsent::explicit(kind, &args, "book");
        let error = service
            .run(kind, &args, "book", None, Some(&consent))
            .unwrap_err();
        assert_eq!(error.code, "validation");
        assert_eq!(error.message, message);
    }
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn oversized_probe_timeouts_refuse_before_connection() {
    let http = FakeProvider::new("{}").unwrap();
    let electrum = FakeElectrum::script(vec![
        (1, "{\"id\":1,\"result\":\"ready\"}\n".into()),
        (1, "{\"id\":2,\"result\":\"banner\"}\n".into()),
    ])
    .unwrap();
    let gate = gate(Offline::Online);
    let service = probes::Probes::new(gate.clone(), "fixture");
    for (kind, url) in [
        ("ui.backends.http.test", http.base()),
        ("ui.backends.electrum.test", electrum.url()),
    ] {
        let args = json!({"url":url,"timeout":u64::MAX});
        let proof = probes::ProbeConsent::explicit(kind, &args, "book");
        let result = service
            .run(kind, &args, "book", None, Some(&proof))
            .unwrap();
        assert_eq!(result["ok"], false);
    }
    assert!(gate.attempts().records.is_empty());
    assert!(http.requests().is_empty());
    assert!(electrum.requests().is_empty());
}

#[test]
fn core_listsinceblock_has_64_mib_budget_and_invalid_http_error_is_capped() {
    let large = "x".repeat(RESPONSE_LIMIT + 1);
    let server=FakeProvider::replies(vec![
        (200,json!({"result":large}).to_string()),
        (500,json!({"id":"kassiber-getblockchaininfo","result":null,"error":{"code":-1,"message":"x".repeat(65536)}}).to_string()),
    ]).unwrap();
    let config = backend(server.base(), false);
    let proof = Consent::explicit_trigger(config.request());
    let client = CoreClient::new(config, gate(Offline::Online), "fixture");
    assert_eq!(
        client
            .call(
                "listsinceblock",
                None,
                None,
                Some(&proof),
                Instant::now() + Duration::from_secs(10)
            )
            .unwrap()
            .as_str()
            .unwrap()
            .len(),
        RESPONSE_LIMIT + 1
    );
    assert_eq!(
        client
            .call("getblockchaininfo", None, None, Some(&proof), deadline())
            .unwrap_err()
            .message,
        "Bitcoin Core returned an invalid RPC error response"
    );
}

#[test]
fn detect_candidates_ignore_remote_configuration_and_refs_exclude_secrets() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("bitcoin.conf"),
        "rpcconnect=remote.invalid\nrpcuser=user\nrpcpassword=secret\n",
    )
    .unwrap();
    std::fs::write(root.path().join(".cookie"), "user:cookie-secret").unwrap();
    assert!(detection::candidates(root.path()).unwrap().is_empty());
    std::fs::write(
        root.path().join("bitcoin.conf"),
        "[regtest]\nrpcbind=[::1]:38333\nrpcuser=user\nrpcpassword=secret\n",
    )
    .unwrap();
    let candidates = detection::candidates(root.path()).unwrap();
    assert_eq!(candidates.len(), 2); // main cookie, regtest basic
    assert_eq!(candidates[1]["url"], "http://[::1]:38333");
    let reference = detection::credential_ref(&candidates[1]);
    let mut changed = candidates[1].clone();
    changed["password"] = json!("changed");
    assert_eq!(detection::credential_ref(&changed), reference);
    let gate = gate(Offline::Offline);
    for candidate in candidates {
        let config = Backend::new(&candidate, false).unwrap();
        let proof = Consent::explicit_trigger(config.request());
        assert_eq!(
            CoreClient::new(config, gate.clone(), "fixture")
                .call("getblockchaininfo", None, None, Some(&proof), deadline())
                .unwrap_err()
                .code,
            "network_egress_disabled"
        );
    }
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
}

#[test]
fn invalid_batch_size_is_refused_before_transport() {
    for size in [json!(0), json!(-1), json!("bad")] {
        assert!(Backend::new(&json!({"url":"tcp://127.0.0.1:1","batch_size":size}), true).is_err());
    }
}

#[test]
fn saved_core_config_url_override_cannot_forward_saved_credentials() {
    let server = FakeProvider::new("{}").unwrap();
    let service = probes::Probes::new(gate(Offline::Online), "fixture");
    let args = json!({"backend":"fixture","config":{"url":server.base()}});
    let saved = json!({"name":"fixture","kind":"bitcoinrpc","url":"http://127.0.0.1:1","username":"saved","password":"private"});
    let kind = "ui.backends.bitcoinrpc.test";
    let grant = probes::ProbeConsent::explicit(kind, &args, "book");
    let result = service
        .run(kind, &args, "book", Some(saved), Some(&grant))
        .unwrap();
    assert_eq!(result["reachable"], false);
    assert!(result["error"]["message"]
        .as_str()
        .unwrap()
        .contains("requires USERNAME/PASSWORD"));
    assert!(server.requests().is_empty());
}

#[test]
fn esplora_tip_hash_is_optional_canonical_and_consent_bound() {
    for (status, text, expected) in [
        (
            200,
            format!(" \n{}\t", "AB".repeat(32)),
            Some("ab".repeat(32)),
        ),
        (200, "00".repeat(31), None),
        (200, "gg".repeat(32), None),
        (404, "missing".into(), None),
    ] {
        let server = FakeProvider::replies(vec![(status, text)]).unwrap();
        let backend = backend(server.base(), false);
        let proof = Consent::explicit_trigger(backend.request());
        let http = HttpClient::new(backend, gate(Offline::Online), "test");
        assert_eq!(http.esplora_tip_hash(None, deadline()), None);
        assert!(server.requests().is_empty());
        assert_eq!(http.esplora_tip_hash(Some(&proof), deadline()), expected);
        assert_eq!(server.requests().len(), 1);
        assert!(server.requests()[0].starts_with("GET /blocks/tip/hash "));
    }
}
#[test]
fn backend_error_bodies_withhold_the_credentials_the_request_carried() {
    // A token straddling the 200-character cut, in raw, percent-encoded and
    // JSON-escaped spellings, and the proxy password the route would send.
    let token = "esplora+bearer/T0ken-qqqqqqqqqqqqqqqq";
    let body = format!(
        "{} {token} esplora%2Bbearer%2FT0ken-qqqqqqqqqqqqqqqq esplora+bearer\\/T0ken-qqqqqqqqqqqqqqqq",
        "x".repeat(190)
    );
    for auth in [
        json!({"token": token}),
        json!({"auth_header": format!("Bearer {token}")}),
    ] {
        let server = FakeProvider::script(vec![(401, vec![], body.clone())]).unwrap();
        let mut config = json!({"name": "fixture", "url": server.base()});
        for (key, value) in auth.as_object().unwrap() {
            config[key] = value.clone();
        }
        let backend = Backend::new(&config, false).unwrap();
        let proof = Consent::explicit_trigger(backend.request());
        let client = HttpClient::new(backend, gate(Offline::Online), "fixture");
        let error = client
            .get_json("blocks", Some(&proof), deadline())
            .unwrap_err();
        assert_eq!(
            error.message,
            format!(
                "HTTP 401 from backend for http://<redacted-host>: {} <redacted",
                "x".repeat(190)
            )
        );
        assert_eq!(server.requests().len(), 1);
    }
}
#[test]
fn backend_credential_forms_cover_config_passwords_proxies_and_extra_keys() {
    let backend = Backend::new(
        &json!({"name":"fixture","url":"http://127.0.0.1:1","rpcuser":"rpc","config_json":"{\"rpcpassword\":4242}",
                "tor_proxy":"socks5h://u:proxy%2Fpw@127.0.0.1:9050"}),
        false,
    )
    .unwrap();
    let forms = backend.credential_forms(&["extra-key-1"]);
    let text = "4242 rpc:4242 cnBjOjQyNDI= proxy/pw extra-key-1 rpc";
    assert_eq!(
        kassiber_credentials::withhold(text, &forms),
        "<redacted> <redacted> <redacted> <redacted> <redacted> rpc"
    );
    assert_eq!(
        backend.proxy_url().as_deref(),
        Some("socks5h://u:proxy%2Fpw@127.0.0.1:9050")
    );
}
#[test]
fn electrum_errors_withhold_backend_credentials() {
    let token = "electrum-config-T0ken-77";
    let server = FakeElectrum::script(vec![(
        1,
        format!("{{\"id\":1,\"error\":{{\"code\":1,\"message\":\"no {token}\"}}}}\n"),
    )])
    .unwrap();
    let config = Backend::new(
        &json!({"name":"fixture","url":server.url(),"token":token}),
        true,
    )
    .unwrap();
    let proof = Consent::explicit_trigger(config.request());
    let error = ElectrumClient::connect(config, &gate(Offline::Online), Some(proof), deadline())
        .err()
        .unwrap();
    assert_eq!(
        error.message,
        "Electrum call server.version failed (1): no <redacted>"
    );
}

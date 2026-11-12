use super::*;
use kassiber_net::{test_support::FakeProvider, Ledger, Offline};
use serde_json::json;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use std::time::Duration;

pub(super) fn projected(snapshot: ProfitabilitySnapshot) -> Value {
    dict([
        (
            "channels",
            Value::List(
                snapshot
                    .channels
                    .into_iter()
                    .map(|c| {
                        dict([
                            ("id", Value::Str(c.id)),
                            ("peer_alias", Value::Str(c.peer_alias)),
                            ("capacity_sat", Value::Int(c.capacity_sat)),
                            (
                                "earned_routing_sat",
                                c.earned_routing_sat.map(Value::Int).unwrap_or(Value::Null),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "routing",
            snapshot
                .routing
                .map(|r| {
                    dict([
                        ("window_label", Value::from(r.window_label.as_str())),
                        ("routing_revenue_sat", Value::Int(r.routing_revenue_sat)),
                        ("payment_cost_sat", Value::Int(r.payment_cost_sat)),
                        ("rebalance_cost_sat", Value::Int(r.rebalance_cost_sat)),
                        ("onchain_cost_sat", Value::Int(r.onchain_cost_sat)),
                        ("net_profit_sat", Value::Int(r.net_profit_sat)),
                        ("forward_count", Value::Int(r.forward_count.into())),
                        ("payment_count", Value::Int(r.payment_count.into())),
                        ("rebalance_count", Value::Int(r.rebalance_count.into())),
                    ])
                })
                .unwrap_or(Value::Null),
        ),
    ])
}

#[test]
fn lnd_report_python_vectors_pin_projection_and_exact_requests() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../fixtures/python_lightning.json")).unwrap()
    else {
        panic!("fixture cases");
    };
    assert_eq!(cases.len(), 581);
    for case in cases {
        let name = py_str(field(&case, "name"));
        let Value::List(calls) = field(&case, "calls") else {
            panic!("fixture calls");
        };
        let replies = calls
            .iter()
            .map(|call| {
                (
                    200,
                    kassiber_pyjson::dumps(field(call, "response"), &Default::default()).unwrap(),
                )
            })
            .collect();
        let node = FakeProvider::replies(replies).unwrap();
        let mut a = action();
        a.window_days = u16::try_from(integer(field(&case, "window")).unwrap()).unwrap();
        let client = LndReportClient::new(&config(node.base()), gate(), "test", a).unwrap();
        let result = client.snapshot_checked(
            Some(&Consent::explicit_trigger(client.request())),
            deadline(),
            || Ok(()),
        );
        if truthy(field(&case, "error")) {
            // Python exceptions surface as internal errors; an AppError keeps its code.
            let error = py_str(field(&case, "error"));
            let code = error.strip_prefix("AppError:").unwrap_or("internal_error");
            assert_eq!(result.unwrap_err().code, code, "{name}");
        } else {
            let snapshot = result.unwrap();
            assert_eq!(
                snapshot.to_dict(),
                field(&case, "snapshot").clone(),
                "{name}"
            );
            assert_eq!(
                snapshot.to_ai_dict(),
                field(&case, "ai_snapshot").clone(),
                "{name}"
            );
            assert_eq!(
                projected(snapshot.profitability()),
                field(&case, "expected").clone(),
                "{name}"
            );
        }
        let requests = node.requests();
        assert_eq!(requests.len(), calls.len(), "{name}");
        for (request, call) in requests.iter().zip(calls) {
            let mut path = py_str(field(call, "path"));
            if let Value::Dict(params) = field(call, "params") {
                path.push('?');
                path.push_str(
                    &params
                        .iter()
                        .map(|(key, value)| {
                            let kassiber_pyjson::Key::Str(key) = key else {
                                panic!("query key");
                            };
                            format!("{}={}", key.as_str().unwrap(), py_str(value))
                        })
                        .collect::<Vec<_>>()
                        .join("&"),
                );
            }
            assert_eq!(
                request.lines().next().unwrap(),
                format!("{} {path} HTTP/1.1", py_str(field(call, "method"))),
                "{name}"
            );
            let body = request.split("\r\n\r\n").nth(1).unwrap();
            if matches!(field(call, "payload"), Value::Null) {
                assert!(body.is_empty(), "{name}");
            } else {
                let parsed = kassiber_pytext::loads(body).unwrap();
                let options = kassiber_pyjson::Options {
                    sort_keys: true,
                    ..Default::default()
                };
                assert_eq!(
                    kassiber_pyjson::dumps(&parsed, &options).unwrap(),
                    kassiber_pyjson::dumps(field(call, "payload"), &options).unwrap(),
                    "{name}"
                );
            }
        }
    }
}

pub(super) fn action() -> ReportAction {
    ReportAction {
        operation: "reports.lightning-profitability".into(),
        book_id: "disposable-book".into(),
        profile_id: "profile".into(),
        connection_id: "lnd-wallet".into(),
        window_days: 30,
        export_file: None,
    }
}
// Generous budgets: a loopback `connect()` to the fake node occasionally
// stalls past 2 s on macOS (docs/reference/macos-validation.md, A2).
fn config(url: &str) -> Json {
    json!({"name":"fake-lnd","kind":"lnd","url":url,"token":"ABCD","timeout":30})
}
fn gate() -> Gate {
    Gate::new(|| Offline::Online, Ledger::default())
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

#[test]
fn lnd_report_construction_and_refusals_make_no_attempts() {
    let gate = gate();
    let client = LndReportClient::new(
        &config("http://must-not-resolve.invalid"),
        gate.clone(),
        "test",
        action(),
    )
    .unwrap();
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
    assert_eq!(
        client.fetch(None, deadline()).unwrap_err().code,
        "interaction_required"
    );
    let proof = Consent::explicit_trigger(client.request());
    proof.revoke();
    assert_eq!(
        client.fetch(Some(&proof), deadline()).unwrap_err().code,
        "interaction_required"
    );
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
}

#[test]
fn lnd_report_consent_binds_book_profile_connection_window_export_and_credentials() {
    let gate = gate();
    let original = LndReportClient::new(
        &config("http://must-not-resolve.invalid"),
        gate.clone(),
        "test",
        action(),
    )
    .unwrap();
    let consent = Consent::explicit_trigger(original.request());
    let mut changes = Vec::new();
    let mut a = action();
    a.operation = "ui.reports.lightning_profitability".into();
    changes.push((a, config("http://must-not-resolve.invalid")));
    let mut a = action();
    a.book_id = "other".into();
    changes.push((a, config("http://must-not-resolve.invalid")));
    let mut a = action();
    a.profile_id = "other".into();
    changes.push((a, config("http://must-not-resolve.invalid")));
    let mut a = action();
    a.connection_id = "other".into();
    changes.push((a, config("http://must-not-resolve.invalid")));
    let mut a = action();
    a.window_days = 31;
    changes.push((a, config("http://must-not-resolve.invalid")));
    let mut a = action();
    a.export_file = Some("target/disposable.csv".into());
    changes.push((a, config("http://must-not-resolve.invalid")));
    let mut c = config("http://must-not-resolve.invalid");
    c["token"] = json!("ccdd");
    changes.push((action(), c));
    let mut c = config("http://must-not-resolve.invalid");
    c["url"] = json!("http://must-not-resolve.invalid/other");
    changes.push((action(), c));
    for (a, c) in changes {
        let changed = LndReportClient::new(&c, gate.clone(), "test", a).unwrap();
        assert_eq!(
            changed.fetch(Some(&consent), deadline()).unwrap_err().code,
            "interaction_required"
        );
    }
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
}

#[test]
fn lnd_report_offline_and_unavailable_refuse_before_loopback() {
    let node = FakeProvider::new("{}").unwrap();
    let state = Arc::new(AtomicU8::new(0));
    let current = state.clone();
    let gate = Gate::new(
        move || match current.load(Ordering::SeqCst) {
            0 => Offline::Online,
            1 => Offline::Offline,
            _ => Offline::Unavailable,
        },
        Ledger::default(),
    );
    let client =
        LndReportClient::new(&config(node.base()), gate.clone(), "test", action()).unwrap();
    let consent = Consent::explicit_trigger(client.request());
    for (value, code) in [
        (1, "network_egress_disabled"),
        (2, "network_egress_disabled"),
    ] {
        state.store(value, Ordering::SeqCst);
        assert_eq!(
            client.fetch(Some(&consent), deadline()).unwrap_err().code,
            code
        );
    }
    assert!(node.requests().is_empty());
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
}

#[test]
fn lnd_checked_report_refuses_between_reads_and_before_return() {
    for stop in [1, 2, 9, 10] {
        let node = FakeProvider::new("{}").unwrap();
        let client = LndReportClient::new(&config(node.base()), gate(), "test", action()).unwrap();
        let consent = Consent::explicit_trigger(client.request());
        let mut checks = 0;
        let result = client.fetch_checked(Some(&consent), deadline(), || {
            checks += 1;
            if checks == stop {
                Err(AppError::new("interaction_required", "scope changed"))
            } else {
                Ok(())
            }
        });
        assert_eq!(result.unwrap_err().code, "interaction_required");
        assert_eq!(node.requests().len(), stop - 1);
    }
}

#[test]
fn lnd_report_acquisition_pins_requests_window_rounding_and_discard_boundary() {
    let now = 2_000_000_000i64;
    let start = now - 30 * 86400;
    let replies = vec![
        json!({"best_header_timestamp":now,"identity_pubkey":"operator-secret"}),
        json!({"confirmed_balance":"5"}),
        json!({"balance":"5"}),
        json!({}),
        json!({"channels":[
            {"chan_id":"1","private":true,"remote_pubkey":"02private","capacity":"9007199254740993","pending_htlcs":[{"payment_preimage":"secret"}]},
            {"chan_id":"2","peer_alias":"Public,\npeer","capacity":"5"}
        ]}),
        json!({"channels":[{"chan_id":"3","private":true,"remote_pubkey":"01closed","capacity":"8"}]}),
        json!({"forwarding_events":[
            {"timestamp":start - 1,"chan_id_out":"1","fee_msat":"999999"},
            {"timestamp":start,"chan_id_out":"1","fee_msat":"1501","failure_source_pubkey":"secret"},
            {"timestamp":now,"chan_id_out":"1","fee_msat":"1501","route":{"hops":["secret"]}},
            {"timestamp":"0","chan_id_out":"2","fee_msat":"-1"},
            {"timestamp":now,"chan_id_out":"2","fee":"2","fee_msat":"0"}
        ],"last_offset_index":"5"}),
        json!({"forwarding_events":[]}),
        json!({"payments":[
            {"payment_index":"1","creation_date":start - 1,"status":"SUCCEEDED","fee_msat":"999999"},
            {"payment_index":"2","creation_date":start,"status":"SUCCEEDED","fee_msat":"1999","payment_preimage":"secret","payment_request":"secret"},
            {"payment_index":"3","creation_time_ns":format!("{now}000000000"),"status":"SUCCESS","fee_msat":"1999","htlcs":[{"route":{"hops":["secret"]}}]},
            {"payment_index":"4","creation_date":now,"status":"COMPLETE","fee_sat":"2"},
            {"payment_index":"5","creation_date":now,"status":"FAILED","fee_sat":"99999"}
        ],"last_index_offset":5}),
        json!({"payments":[]}),
        json!({"invoices":[{"r_preimage":"secret","payment_addr":"secret","route_hints":["secret"]}],"last_index_offset":1}),
        json!({"invoices":[]}),
        json!({"node":{"alias":" Closed alias "}}),
        json!({"node":{"alias":" Private alias "}}),
    ];
    let node =
        FakeProvider::replies(replies.into_iter().map(|v| (200, v.to_string())).collect()).unwrap();
    let gate = gate();
    let client = LndReportClient::new(
        &config(&format!("{}/prefix/", node.base())),
        gate.clone(),
        "test",
        action(),
    )
    .unwrap();
    assert!(node.requests().is_empty());
    let snapshot = client
        .fetch(
            Some(&Consent::explicit_trigger(client.request())),
            deadline(),
        )
        .unwrap();
    let routing = snapshot.routing.as_ref().unwrap();
    assert_eq!(routing.routing_revenue_sat, 5.into());
    assert_eq!(routing.payment_cost_sat, 4.into());
    assert_eq!(routing.net_profit_sat, 1.into());
    assert_eq!(routing.forward_count, 4);
    assert_eq!(routing.payment_count, 3);
    assert_eq!(routing.rebalance_cost_sat, 0.into());
    assert_eq!(routing.rebalance_count, 0);
    assert_eq!(routing.onchain_cost_sat, 0.into());
    assert_eq!(snapshot.channels.len(), 2);
    assert_eq!(
        snapshot.channels[0].peer_alias,
        PyStr::from("Private alias")
    );
    assert_eq!(
        snapshot.channels[0].capacity_sat,
        BigInt::from(9_007_199_254_740_993u64)
    );
    assert_eq!(snapshot.channels[0].earned_routing_sat, Some(2.into()));
    assert_eq!(snapshot.channels[1].earned_routing_sat, Some(1.into()));
    let safe = format!("{snapshot:?}");
    for forbidden in [
        "secret",
        "02private",
        "01closed",
        "payment_preimage",
        "route_hints",
        "macaroon",
    ] {
        assert!(!safe.contains(forbidden), "{forbidden}");
    }
    let requests = node.requests();
    let paths = requests
        .iter()
        .map(|r| r.lines().next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "GET /prefix/v1/getinfo HTTP/1.1",
            "GET /prefix/v1/balance/blockchain HTTP/1.1",
            "GET /prefix/v1/balance/channels HTTP/1.1",
            "GET /prefix/v1/fees HTTP/1.1",
            "GET /prefix/v1/channels HTTP/1.1",
            "GET /prefix/v1/channels/closed HTTP/1.1",
            "POST /prefix/v1/switch HTTP/1.1",
            "POST /prefix/v1/switch HTTP/1.1",
            "GET /prefix/v1/payments?include_incomplete=true&index_offset=0&max_payments=1000&reversed=false HTTP/1.1",
            "GET /prefix/v1/payments?include_incomplete=true&index_offset=5&max_payments=1000&reversed=false HTTP/1.1",
            "GET /prefix/v1/invoices?index_offset=0&num_max_invoices=1000&reversed=false HTTP/1.1",
            "GET /prefix/v1/invoices?index_offset=1&num_max_invoices=1000&reversed=false HTTP/1.1",
            "GET /prefix/v1/graph/node/01closed HTTP/1.1",
            "GET /prefix/v1/graph/node/02private HTTP/1.1"
        ]
    );
    for request in &requests {
        for (header, expected) in [
            ("grpc-metadata-macaroon", "abcd"),
            ("user-agent", "kassiber/test"),
        ] {
            let actual = request
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case(header))
                .map(|(_, value)| value.trim());
            assert_eq!(actual, Some(expected));
        }
    }
    let body = requests[6].split("\r\n\r\n").nth(1).unwrap();
    assert_eq!(
        serde_json::from_str::<Json>(body).unwrap(),
        json!({"start_time":start.to_string(),"end_time":now.to_string(),"index_offset":"0","num_max_events":1000})
    );
    let body = requests[7].split("\r\n\r\n").nth(1).unwrap();
    assert_eq!(
        serde_json::from_str::<Json>(body).unwrap(),
        json!({"start_time":start.to_string(),"end_time":now.to_string(),"index_offset":"5","num_max_events":1000})
    );
    let ledger = format!("{:?}", gate.ledger().snapshot(0, 2000));
    assert!(!ledger.contains("abcd"));
    assert!(!ledger.contains("prefix"));
}

#[test]
fn lnd_report_errors_refuse_redirects_shapes_and_raw_secrets() {
    for (status, body, code) in [
        (401, "macaroon=secret", "backend_error"),
        (503, "preimage=secret", "backend_error"),
        (200, "not-json-secret", "backend_error"),
        (200, "[]", "backend_error"),
    ] {
        let node = FakeProvider::replies(vec![(status, body.into())]).unwrap();
        let client = LndReportClient::new(&config(node.base()), gate(), "test", action()).unwrap();
        let error = client
            .fetch(
                Some(&Consent::explicit_trigger(client.request())),
                deadline(),
            )
            .unwrap_err();
        assert_eq!(error.code, code);
        assert!(!format!("{error:?}").contains("secret"));
        assert_eq!(node.requests().len(), 1);
    }
    let destination = FakeProvider::new("{}").unwrap();
    let source = FakeProvider::script(vec![(
        302,
        vec![("Location".into(), destination.base().into())],
        "{}".into(),
    )])
    .unwrap();
    let client = LndReportClient::new(&config(source.base()), gate(), "test", action()).unwrap();
    assert!(client
        .fetch(
            Some(&Consent::explicit_trigger(client.request())),
            deadline()
        )
        .is_err());
    assert_eq!(source.requests().len(), 1);
    assert!(destination.requests().is_empty());
}

#[test]
fn lnd_report_invalid_credentials_and_channel_identifiers_fail_closed() {
    for token in ["", "a", "not-hex", "aa\r\nAuthorization: other"] {
        let mut c = config("http://must-not-resolve.invalid");
        c["token"] = json!(token);
        assert!(LndReportClient::new(&c, gate(), "test", action()).is_err());
    }
    for row in [
        json!({"chan_id":"peer-pubkey"}),
        json!({"chan_id":"1","channel_point":"preimage"}),
    ] {
        let node = FakeProvider::replies(vec![
            (200, "{}".into()),
            (200, "{}".into()),
            (200, "{}".into()),
            (200, "{}".into()),
            (200, json!({"channels":[row]}).to_string()),
        ])
        .unwrap();
        let client = LndReportClient::new(&config(node.base()), gate(), "test", action()).unwrap();
        assert!(client
            .fetch(
                Some(&Consent::explicit_trigger(client.request())),
                deadline()
            )
            .is_err());
        assert_eq!(node.requests().len(), 9);
    }
}

#[test]
fn lnd_report_policy_change_after_final_resource_refuses_optional_aliases_and_projection() {
    for private in [false, true] {
        let mut replies = vec![(200, "{}".into()); 9];
        if private {
            replies[4].1 =
                json!({"channels":[{"chan_id":"1","private":true,"remote_pubkey":"peer"}]})
                    .to_string();
        }
        let node = Arc::new(FakeProvider::replies(replies).unwrap());
        let observed = node.clone();
        let gate = Gate::new(
            move || {
                if observed.requests().len() >= 9 {
                    Offline::Offline
                } else {
                    Offline::Online
                }
            },
            Ledger::default(),
        );
        let client = LndReportClient::new(&config(node.base()), gate, "test", action()).unwrap();
        let result = client.fetch(
            Some(&Consent::explicit_trigger(client.request())),
            deadline(),
        );
        assert_eq!(result.unwrap_err().code, "network_egress_disabled");
        assert_eq!(node.requests().len(), 9);
    }
}

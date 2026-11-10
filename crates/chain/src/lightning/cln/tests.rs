use super::*;
use kassiber_net::{Ledger, Offline};
use serde_json::json;

#[test]
fn cln_report_python_vectors_pin_projection_requests_and_ancillary_refusals() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../../fixtures/python_cln.json")).unwrap()
    else {
        panic!("cases");
    };
    assert_eq!(cases.len(), 690);
    for case in cases {
        let name = py_str(field(&case, "name"));
        let data = field(&case, "data");
        let mut calls = Vec::new();
        let result = acquire(
            |method| {
                calls.push(dict([
                    ("method", Value::from(method.name())),
                    ("args", Value::Null),
                ]));
                let Value::List(failures) = field(&case, "failures") else {
                    panic!("failures");
                };
                if failures
                    .iter()
                    .any(|v| matches!(v, Value::Str(s) if s.is(method.name())))
                {
                    return Err(AppError::new("sync_error", "synthetic failure"));
                }
                Ok(field(data, method.name()).clone())
            },
            u16::try_from(integer(field(&case, "window")).unwrap()).unwrap(),
            u64::try_from(integer(field(&case, "now")).unwrap()).unwrap(),
        );
        assert_eq!(Value::List(calls), field(&case, "calls").clone(), "{name}");
        if let Some(error) = get(&case, "error") {
            assert_eq!(
                result.unwrap_err().code,
                py_str(field(error, "code")),
                "{name}"
            );
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
            let result = super::super::tests::projected(snapshot.profitability());
            assert_eq!(result, field(&case, "expected").clone(), "{name}");
            let encoded = kassiber_pyjson::dumps(&result, &Default::default()).unwrap();
            assert!(!encoded.contains("DISCARDED_CLN_SECRET"), "{name}");
        }
    }
}

fn config() -> Json {
    json!({"kind":"coreln", "name":"fake", "url":"cln://local", "lightning_cli":"nonexistent-lightning-fixture", "timeout":1})
}
fn action() -> ReportAction {
    super::super::tests::action()
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(1)
}

#[test]
fn cln_report_consent_binds_action_and_every_process_option_before_launch() {
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    let original = ClnReportClient::new(&config(), gate.clone(), action()).unwrap();
    let proof = Consent::explicit_trigger(original.request());
    for key in [
        "lightning_cli",
        "lightning_dir",
        "rpc_file",
        "network",
        "name",
        "url",
    ] {
        let mut config = config();
        config[key] = json!("changed-private-value");
        let client = ClnReportClient::new(&config, gate.clone(), action()).unwrap();
        assert_eq!(
            client.fetch(Some(&proof), deadline()).unwrap_err().code,
            "interaction_required",
            "{key}"
        );
    }
    for key in [
        "operation",
        "book",
        "profile",
        "connection",
        "window",
        "export",
    ] {
        let mut action = action();
        match key {
            "operation" => action.operation = "ui.reports.lightning_profitability".into(),
            "book" => action.book_id = "other".into(),
            "profile" => action.profile_id = "other".into(),
            "connection" => action.connection_id = "other".into(),
            "window" => action.window_days = 31,
            _ => action.export_file = Some("target/other.csv".into()),
        }
        let client = ClnReportClient::new(&config(), gate.clone(), action).unwrap();
        assert_eq!(
            client.fetch(Some(&proof), deadline()).unwrap_err().code,
            "interaction_required",
            "{key}"
        );
    }
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
}

#[test]
fn cln_report_offline_unavailable_missing_revoked_and_optional_policy_refuse() {
    for state in [Offline::Online, Offline::Offline, Offline::Unavailable] {
        let gate = Gate::new(move || state, Ledger::default());
        let client = ClnReportClient::new(&config(), gate.clone(), action()).unwrap();
        assert!(gate.attempts().records.is_empty());
        assert_eq!(
            client.fetch(None, deadline()).unwrap_err().code,
            "interaction_required"
        );
        let proof = Consent::explicit_trigger(client.request());
        if state != Offline::Online {
            assert_eq!(
                client.fetch(Some(&proof), deadline()).unwrap_err().code,
                "network_egress_disabled"
            );
        }
        proof.revoke();
        assert!(client.fetch(Some(&proof), deadline()).is_err());
        assert!(gate.attempts().records.is_empty());
        assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
    }
    for code in [
        "interaction_required",
        "network_egress_disabled",
        "backend_timeout",
    ] {
        let mut calls = 0;
        let result = acquire(
            |_| {
                calls += 1;
                if calls == 1 {
                    Ok(dict([] as [(&str, Value); 0]))
                } else {
                    Err(AppError::new(code, "policy refusal"))
                }
            },
            30,
            0,
        );
        assert_eq!(result.unwrap_err().code, code);
        assert_eq!(calls, 2);
    }
}

#[test]
fn cln_report_commando_configuration_is_refused_inert_and_secret_free() {
    // lightning-cli takes a commando rune only in argv, so any commando
    // field refuses the connection before a request or process exists.
    for (url, peer, token) in [
        ("cln://commando", None, None),
        ("CLN://Commando", None, None),
        ("cln://commando", Some("private-peer"), Some("private-rune")),
        ("cln://local", Some("private-peer"), None),
        ("cln://local", None, Some("private-rune")),
        ("cln://local", Some("private-peer"), Some("private-rune")),
    ] {
        let mut config = config();
        config["url"] = json!(url);
        config["commando_peer_id"] = json!(peer);
        config["token"] = json!(token);
        let gate = Gate::new(|| Offline::Online, Ledger::default());
        let error = ClnReportClient::new(&config, gate.clone(), action())
            .err()
            .unwrap();
        assert_eq!(error.code, "validation", "{url} {peer:?} {token:?}");
        assert_eq!(
            error.message,
            "Core Lightning commando connections are not supported"
        );
        assert!(error.hint.as_deref().unwrap().contains("rpc_file"));
        let output = kassiber_pyjson::dumps(&error.envelope(None), &Default::default()).unwrap();
        assert!(!output.contains("private-peer"));
        assert!(!output.contains("private-rune"));
        assert!(gate.attempts().records.is_empty());
        assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
    }
    // Blank values are absent, as in Python's `backend_value`.
    let mut config = config();
    config["commando_peer_id"] = json!("  ");
    config["token"] = json!("");
    assert!(ClnReportClient::new(
        &config,
        Gate::new(|| Offline::Online, Ledger::default()),
        action()
    )
    .is_ok());
}

#[cfg(unix)]
fn fake_cli(body: &str) -> (tempfile::TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let program = root.path().join("fake-lightning-cli");
    std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    (root, program)
}

#[test]
#[cfg(unix)]
fn cln_checked_report_refuses_between_resources_and_before_return() {
    for stop in [1, 2, 6, 10] {
        let (_root, program) = fake_cli("echo '{}'");
        let gate =
            Gate::new(|| Offline::Online, Ledger::default()).with_cln_fixture(program.clone());
        let client = ClnReportClient::new(
            &json!({"kind":"coreln", "lightning_cli":program}),
            gate.clone(),
            action(),
        )
        .unwrap();
        let proof = Consent::explicit_trigger(client.request());
        let mut checks = 0;
        let result = client.fetch_checked(Some(&proof), deadline(), || {
            checks += 1;
            if checks == stop {
                Err(AppError::new("interaction_required", "scope changed"))
            } else {
                Ok(())
            }
        });
        assert_eq!(result.unwrap_err().code, "interaction_required");
        // The ledger records every launch; attempts are only observed when a
        // test egress guard is armed in the environment.
        assert_eq!(gate.ledger().snapshot(0, 100).records.len(), stop - 1);
    }
}

#[test]
#[cfg(unix)]
fn cln_report_loopback_transport_pins_commands_optional_errors_and_discard() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../../fixtures/python_cln.json")).unwrap()
    else {
        panic!("cases");
    };
    let case = &cases[0];
    let data = field(case, "data");
    let methods = [
        ClnMethod::GetInfo,
        ClnMethod::ListFunds,
        ClnMethod::ListPeerChannels,
        ClnMethod::ListForwards,
        ClnMethod::ListPays,
        ClnMethod::ListInvoices,
        ClnMethod::ListIncome,
        ClnMethod::ListBalances,
        ClnMethod::ListAccountEvents,
    ];
    // The fake `lightning-cli` answers each method from a file in the
    // "rpc file" directory and logs its arguments there.
    let replies = tempfile::tempdir().unwrap();
    for method in &methods {
        std::fs::write(
            replies.path().join(format!("{}.json", method.name())),
            kassiber_pyjson::dumps(field(data, method.name()), &Default::default()).unwrap(),
        )
        .unwrap();
    }
    let base = replies.path().to_string_lossy().into_owned();
    let (_root, program) = fake_cli(concat!(
        r#"dir=; for a in "$@"; do case "$a" in --rpc-file=*) dir=${a#--rpc-file=};; esac; method=$a; done"#,
        "\n",
        r#"{ printf '['; sep=; for a in "$@"; do printf '%s"%s"' "$sep" "$a"; sep=', '; done; echo ']'; } >> "$dir/args.log""#,
        "\n",
        r#"cat "$dir/$method.json""#,
    ));
    let gate = Gate::new(|| Offline::Online, Ledger::default()).with_cln_fixture(program.clone());
    let config = json!({"kind":"coreln", "lightning_cli":program, "rpc_file":base, "lightning_dir":"disposable lightning directory", "network":"regtest"});
    let client = ClnReportClient::new(&config, gate.clone(), action()).unwrap();
    assert!(!replies.path().join("args.log").exists());
    let result = client
        .fetch(
            Some(&Consent::explicit_trigger(client.request())),
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
    // The client windows routing at the current time while the vectors pin
    // the projection at a frozen instant: compare with the same projection
    // of the same responses evaluated now.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let local = acquire(|method| Ok(field(data, method.name()).clone()), 30, now).unwrap();
    let result = super::super::tests::projected(result);
    assert_eq!(
        result,
        super::super::tests::projected(local.profitability())
    );
    let log = std::fs::read_to_string(replies.path().join("args.log")).unwrap();
    let hits: Vec<&str> = log.lines().collect();
    assert_eq!(hits.len(), 9);
    for (hit, method) in hits.iter().zip(methods) {
        let args: Vec<String> = serde_json::from_str(hit).unwrap();
        assert_eq!(
            args,
            [
                "--json".to_owned(),
                "--raw".to_owned(),
                "--lightning-dir=disposable lightning directory".to_owned(),
                format!("--rpc-file={base}"),
                "--network=regtest".to_owned(),
                method.name().to_owned()
            ]
        );
    }
    let records = gate.ledger().snapshot(0, 2000).records;
    assert_eq!(records.len(), 9);
    assert!(records
        .iter()
        .all(|r| r.operation == kassiber_net::Operation::Process && r.scheme == "cln"));
    assert!(!format!("{records:?}").contains("disposable lightning directory"));
}

#[test]
#[cfg(unix)]
fn cln_report_process_errors_are_typed_and_never_expose_raw_output() {
    for (body, code) in [
        (
            "echo private-stdout; echo private-stderr >&2; exit 4",
            "sync_error",
        ),
        ("echo private-non-json", "sync_error"),
        ("printf '\\377'", "sync_error"),
        ("exec sleep 10", "backend_timeout"),
    ] {
        let (_root, program) = fake_cli(body);
        let gate =
            Gate::new(|| Offline::Online, Ledger::default()).with_cln_fixture(program.clone());
        let config = json!({"kind":"coreln", "lightning_cli":program, "rpc_file":"private-rpc"});
        let client = ClnReportClient::new(&config, gate, action()).unwrap();
        // A freshly written script's first exec can take ~100 ms on macOS;
        // only the timeout case may be that short.
        let budget = Duration::from_millis(if code == "backend_timeout" { 100 } else { 2000 });
        let error = client
            .fetch(
                Some(&Consent::explicit_trigger(client.request())),
                Instant::now() + budget,
            )
            .unwrap_err();
        assert_eq!(error.code, code);
        assert!(error.retryable);
        let rendered = kassiber_pyjson::dumps(&error.envelope(None), &Default::default()).unwrap();
        for secret in [
            "private-stdout",
            "private-stderr",
            "private-non-json",
            "private-rpc",
        ] {
            assert!(!rendered.contains(secret));
        }
    }
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    let config = config();
    let program = PathBuf::from(config["lightning_cli"].as_str().unwrap());
    let client = ClnReportClient::new(&config, gate.with_cln_fixture(program), action()).unwrap();
    assert_eq!(
        client
            .fetch(
                Some(&Consent::explicit_trigger(client.request())),
                deadline()
            )
            .unwrap_err()
            .code,
        "dependency_missing"
    );
}

//! Child processes test environment semantics without mutating the test runner's env.
use kassiber_net::{Consent, Endpoint, Gate, Ledger, Offline, Request, Subsystem, TlsTrust};
use std::process::Command;
use std::time::Duration;

#[test]
fn probe() {
    let Ok(scenario) = std::env::var("KASSIBER_NET_TEST_SCENARIO") else {
        return;
    };
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    if scenario == "init" {
        return;
    }
    if scenario == "online-gate" {
        for subsystem in [Subsystem::Ai, Subsystem::Update] {
            let request = Request::new(
                subsystem,
                Endpoint::new("localhost", 9).unwrap(),
                TlsTrust::Plaintext,
            );
            assert!(gate
                .authorize(request.clone(), Some(Consent::explicit_trigger(&request)))
                .is_ok());
        }
        assert!(gate.attempts().records.is_empty());
        return;
    }
    if scenario.starts_with("cln-") {
        let request = Request::cln(
            kassiber_net::ClnConfig {
                program: "must-not-launch-lightning-cli".into(),
                lightning_dir: None,
                rpc_file: None,
                network: None,
            },
            [1; 32],
        );
        let consent = Consent::explicit_trigger(&request);
        let route = gate.authorize(request, Some(consent));
        if scenario == "cln-product" {
            assert!(matches!(route, Err(kassiber_net::Error::NoEgress)));
            assert!(gate.attempts().records.is_empty());
        } else {
            let _ = route.unwrap().cln(
                kassiber_net::ClnMethod::GetInfo,
                std::time::Instant::now() + Duration::from_secs(1),
                32,
            );
            panic!("guard failed to stop a process");
        }
        return;
    }
    let request = Request::new(
        Subsystem::Sync,
        Endpoint::new("127.0.0.1", 9).unwrap(),
        TlsTrust::Plaintext,
    );
    let proof = Consent::explicit_trigger(&request);
    let route = gate.authorize(request, Some(proof));
    if scenario == "product" {
        assert!(matches!(route, Err(kassiber_net::Error::NoEgress)));
        assert!(gate.attempts().records.is_empty());
    } else {
        let _ = route.unwrap().connect(Duration::from_secs(1));
        panic!("guard failed to stop a connection");
    }
}

fn child(scenario: &str, test: &str, product: &str) -> std::process::Output {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "probe", "--nocapture"])
        .env("KASSIBER_NET_TEST_SCENARIO", scenario)
        .env("KASSIBER_TEST_NO_EGRESS", test)
        .env("KASSIBER_NO_EGRESS", product)
        .output()
        .unwrap()
}

#[test]
fn environment_override_is_destination_blind() {
    for value in ["1", "true", "YES", "on"] {
        let output = child("product", "", value);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn armed_guard_never_runs_silently_without_instrumentation() {
    for value in ["1", "strict", "0"] {
        // A nonempty value, even "0", arms Python child sitecustomize too.
        let output = child("init", value, "");
        if cfg!(feature = "test-observer") {
            assert!(output.status.success());
        } else {
            assert_eq!(output.status.code(), Some(78));
            assert!(String::from_utf8_lossy(&output.stderr).contains("guard could not start"));
        }
    }
}

#[test]
#[cfg(feature = "test-observer")]
fn strict_guard_panics_in_child_before_loopback_connect() {
    let output = child("connect", "strict", "");
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("guard failed"));
}

#[test]
fn ai_and_updates_match_python_online_gate_outside_product_override() {
    assert!(child("online-gate", "", "1").status.success());
}

#[test]
fn cln_process_product_override_refuses_before_spawn() {
    for value in ["1", "true", "YES", "on"] {
        assert!(child("cln-product", "", value).status.success());
    }
}

#[test]
#[cfg(feature = "test-observer")]
fn cln_process_observer_blocks_unapproved_child_launches() {
    for value in ["1", "strict"] {
        let output = child("cln-connect", value, "");
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("guard failed"));
    }
}

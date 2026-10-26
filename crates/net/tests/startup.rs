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

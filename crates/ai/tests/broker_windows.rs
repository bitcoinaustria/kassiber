//! `test_sensitive_windows_provider_fails_before_resolution_or_spawn`
//! (`tests/test_ai_broker.py`): Windows has no process-group kill, so a
//! private CLI turn is refused before the broker is looked up or started.
#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kassiber_ai::broker::{BrokerChat, BrokerClient};
use kassiber_envelope::{dict, s};
use kassiber_net::{BrokerProgram, Gate, Ledger, Offline};
use kassiber_pyjson::Value;

#[test]
fn sensitive_windows_provider_fails_before_resolution_or_spawn() {
    let gate = Gate::new(|| Offline::Online, Ledger::default()).with_test_observer(true);
    let resolved = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&resolved);
    let mut client =
        BrokerClient::new("claude-cli://default", Duration::from_secs(5), gate.clone())
            .unwrap()
            .with_locate(Arc::new(move || {
                flag.store(true, Ordering::SeqCst);
                BrokerProgram::new(None, "index.mjs".into())
            }));
    let messages = vec![dict([
        ("role", s("user")),
        ("content", s("private fixture")),
    ])];
    let options = dict([("sensitive_context", Value::Bool(true))]);
    let error = client
        .stream_chat(
            &BrokerChat {
                messages: Some(&messages),
                context: None,
                model: "test",
                options: &options,
                tools: &[],
            },
            &mut |_| Ok(()),
        )
        .unwrap_err();
    assert_eq!(error.code, "ai_sensitive_provider_unavailable");
    assert!(!resolved.load(Ordering::SeqCst));
    assert!(gate.attempts().records.is_empty());
}

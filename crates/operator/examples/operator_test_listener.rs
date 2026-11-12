//! Test-harness only: a Rust listener on the gated test runtime directory
//! that answers `ping` and `status` the way the Python broker shapes them,
//! so the Python client can be pointed at it
//! (`tests/test_rust_operator_protocol.py`). It is not a broker: it holds no
//! lease, queues nothing and answers every other action with an error.
//!
//! Usage: `operator_test_listener <connections>`. It prints `ready`, then one
//! line per request holding the request re-encoded by the Rust encoder.

use kassiber_envelope::{dict, get, s};
use kassiber_operator::build::{current_build_identity, BuildInputs};
use kassiber_operator::protocol::{encode_json_frame, PROTOCOL_VERSION};
use kassiber_operator::runtime::{listen, Runtime};
use kassiber_pyjson::Value;
use std::io::Write;

fn main() {
    let count: usize = std::env::args()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .unwrap_or(1);
    let build = current_build_identity(BuildInputs {
        version: env!("CARGO_PKG_VERSION"),
        channel: None,
        commit: None,
    });
    let runtime = Runtime::from_process();
    let mut listener = match listen(&runtime) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("listen failed: {error:?}");
            std::process::exit(1);
        }
    };
    println!("ready");
    std::io::stdout().flush().unwrap();
    for _ in 0..count {
        let mut channel = listener.accept().unwrap();
        let request = channel.receive_json().unwrap();
        let frame = encode_json_frame(&request).unwrap();
        println!(
            "{}",
            frame.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        std::io::stdout().flush().unwrap();
        let version_ok = get(&request, "version") == Some(&Value::from(PROTOCOL_VERSION));
        let action = match get(&request, "action") {
            Some(Value::Str(action)) => action.as_str().unwrap_or_default().to_owned(),
            _ => String::new(),
        };
        let data = match action.as_str() {
            "ping" if version_ok => Some(dict([
                ("broker", s("running")),
                ("generation", s("rust-test-listener")),
                ("native_auth_available", Value::Bool(false)),
                ("native_auth_identity", Value::Null),
                ("caller_context", Value::Bool(true)),
                ("build", build.clone()),
            ])),
            "status" if version_ok => Some(dict([
                ("broker", s("running")),
                ("generation", s("rust-test-listener")),
                ("leases", Value::List(Vec::new())),
                ("broker_build", build.clone()),
            ])),
            _ => None,
        };
        let response = match data {
            Some(data) => dict([("ok", Value::Bool(true)), ("data", data)]),
            None => dict([
                ("ok", Value::Bool(false)),
                (
                    "error",
                    dict([
                        ("code", s("operator_protocol_version_mismatch")),
                        ("message", s("operator protocol version mismatch")),
                        ("hint", Value::Null),
                        (
                            "details",
                            dict([("supported", Value::from(PROTOCOL_VERSION))]),
                        ),
                        ("retryable", Value::Bool(false)),
                    ]),
                ),
            ]),
        };
        channel.send_json(&response).unwrap();
    }
    listener.close().unwrap();
}

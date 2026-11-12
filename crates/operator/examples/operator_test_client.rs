//! Test-harness only: runs the Rust client's `ping` and `status` against
//! whatever broker listens at the gated test runtime directory and prints
//! each result (or error) as one compact, sorted JSON line, for comparison
//! with the Python client (`tests/test_rust_operator_protocol.py`).
//!
//! Usage: `operator_test_client [data_root]`.

use kassiber_envelope::{dict, s};
use kassiber_operator::build::{current_build_identity, BuildInputs};
use kassiber_operator::client::{BrokerClient, SystemHost};
use kassiber_pyjson::{dumps, Options, Value};

fn line(result: kassiber_operator::Result<Value>) -> String {
    let value = match result {
        Ok(value) => dict([("value", value)]),
        Err(kassiber_operator::Error::App(error)) => dict([(
            "error",
            dict([
                ("code", s(error.code.as_str())),
                ("message", s(error.message.as_str())),
            ]),
        )]),
        Err(error) => dict([("error", dict([("class", s(error.python_class()))]))]),
    };
    dumps(&value, &Options::new().compact().sort_keys(true)).unwrap()
}

fn main() {
    let data_root = std::env::args().nth(1);
    let build = current_build_identity(BuildInputs {
        version: env!("CARGO_PKG_VERSION"),
        channel: None,
        commit: None,
    });
    let client = BrokerClient::new(SystemHost::new(build));
    println!("{}", line(client.ping()));
    println!("{}", line(client.status(None)));
    if let Some(data_root) = data_root {
        println!("{}", line(client.status(Some(&data_root))));
    }
}

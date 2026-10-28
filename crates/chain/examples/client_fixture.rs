//! Test-only differential driver; not a product CLI or AI dispatch surface.
//! The input is synthetic fixture data supplied by tests/test_rust_chain_candidate.py.
use kassiber_chain::{Backend, Consent, CoreClient, ElectrumClient, HttpClient};
use kassiber_net::{Gate, Ledger, Offline};
use serde_json::{json, Value};
use std::io::Read;
use std::time::{Duration, Instant};
fn run(input: &Value) -> kassiber_chain::Result<Value> {
    let protocol = input["protocol"].as_str().unwrap();
    let backend = Backend::new(&input["backend"], protocol == "electrum")?;
    let consent = Consent::explicit_trigger(backend.request());
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    let deadline = Instant::now() + Duration::from_secs(3);
    match protocol {
        "core" => CoreClient::new(backend, gate, "fixture").call(
            input["method"].as_str().unwrap(),
            input.get("params"),
            input["wallet"].as_str(),
            Some(&consent),
            deadline,
        ),
        "electrum" => {
            let mut client = ElectrumClient::connect(backend, &gate, Some(consent), deadline)?;
            let requests = input["requests"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| (r[0].as_str().unwrap().to_owned(), r[1].clone()))
                .collect::<Vec<_>>();
            Ok(json!(client.batch_call(&requests, deadline)?))
        }
        _ => HttpClient::new(backend, gate, "fixture").get_json(
            input["path"].as_str().unwrap(),
            Some(&consent),
            deadline,
        ),
    }
}
fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let input = serde_json::from_str::<Value>(&input).unwrap();
    let result = match run(&input) {
        Ok(value) => json!({"result": value}),
        Err(error) => json!({"error": {"code": error.code, "message": error.message}}),
    };
    println!("{result}");
}

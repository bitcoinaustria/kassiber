//! Bounded stdin oracle channel. Synthetic secrets never enter argv or output.
use kassiber_envelope::{cli_text, success};
use kassiber_net::{Consent, Gate, Ledger, Offline};
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::file_import::{exchange_api, Identity};
use serde_json::Value;
use std::io::Read;
fn main() {
    let mut raw = String::new();
    std::io::stdin()
        .take(16 * 1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let input: Value = serde_json::from_str(&raw).unwrap();
    let conn = Connection::open(input["book"].as_str().unwrap()).unwrap();
    if let Some(key) = input["passphrase"].as_str() {
        conn.pragma_update(None, "key", key).unwrap();
    }
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    let expected = input["expected"].as_str().unwrap();
    let identity = Identity {
        ids: input["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().into())
            .collect(),
        timestamp: input["timestamp"].as_str().unwrap().into(),
    };
    let result = exchange_api::execute(
        &conn,
        "p",
        input["wallet"].as_str().unwrap_or(""),
        &input["backend"],
        expected,
        &gate,
        "test",
        &mut |route| Some(Consent::explicit_trigger(route.request())),
        Some(&identity),
    );
    let envelope = match result {
        Ok(data) => success(&format!("wallets.sync-{expected}"), data),
        Err(e) => e.envelope(None),
    };
    print!("{}", cli_text(&envelope).unwrap());
}

use kassiber_wallets::psbt;
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let gate = kassiber_net::Gate::new(
        || kassiber_net::Offline::Unavailable,
        kassiber_net::Ledger::default(),
    );
    let mut raw = String::new();
    std::io::stdin()
        .take(32 * 1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let cases: Vec<Value> = serde_json::from_str(&raw).unwrap();
    let results: Vec<_> = cases.iter().map(|case| {
        use bitcoin::hex::FromHex;
        let bytes = case["binary_hex"].as_str().map(|v| Vec::<u8>::from_hex(v).unwrap());
        let network = case["network"].as_str().unwrap_or("main");
        let subject = case["subject"].as_str().unwrap_or("psbt:transaction");
        let result = match (case["operation"].as_str().unwrap(),bytes) {
            ("structure",Some(v)) => psbt::decode_structure(&v),
            ("structure",None) => psbt::decode_text_structure(case["text"].as_str().unwrap()),
            (_,Some(v)) => psbt::analyze(&v,network,subject),
            (_,None) => psbt::analyze_text(case["text"].as_str().unwrap(),network,subject),
        };
        match result {
            Ok(v) => json!({"ok":v}),
            Err(e) => json!({"error":{"code":e.code,"message":e.message,"hint":e.hint,"details":e.details.map(|v| serde_json::from_str::<Value>(&kassiber_pyjson::dumps(&v,&Default::default()).unwrap()).unwrap()),"retryable":e.retryable}}),
        }
    }).collect();
    assert!(gate.attempts().records.is_empty());
    println!("{}", json!(results));
}

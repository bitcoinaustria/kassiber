//! Pure JSON oracle driver, test-harness only. No product registration or I/O
//! beyond the bounded stdin/stdout fixture channel; no database or transport.
use kassiber_wallets::{
    bitcoin_records,
    composition::{compose, ObserverFacts},
};
use serde_json::{json, Value};
use std::io::Read;
fn list<'a>(v: &'a Value, name: &str) -> &'a [Value] {
    v[name].as_array().map_or(&[], Vec::as_slice)
}
fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
fn main() {
    let mut raw = String::new();
    std::io::stdin()
        .take(16 * 1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let cases: Value = serde_json::from_str(&raw).unwrap();
    let results: Vec<_> = cases.as_array().unwrap().iter().map(|case| {
        let i = &case["input"];
        let result = if case["operation"] == "compose" {
            let observers: Vec<_> = list(i,"observers").iter().map(|o| ObserverFacts {
                observer_id:text(&o["observer_id"]).into(), observer_kind:text(&o["observer_kind"]).into(),
                records:list(o,"records").to_vec(), outputs:list(o,"outputs").to_vec(),
                retracted_external_ids:list(o,"retracted_external_ids").iter().map(|v|text(v).to_owned()).collect(),
                checkpoint:o.get("checkpoint").cloned().unwrap_or(json!({})), reused_records:o["reused_records"].as_u64().unwrap_or(0),
            }).collect();
            compose(&observers,list(i,"adapter_records"),i.get("adapter_metadata").unwrap_or(&json!({})),i["backend"].as_str().unwrap_or("fixture")).map(|c| json!({"records":c.records,"metadata":c.metadata,"provenance_entries":c.provenance_entries}))
        } else {
            let scripts = list(i,"scripts").iter().map(|v|text(v).to_owned()).collect();
            let backend = i["backend"].as_str().unwrap_or("fixture");
            if case["operation"] == "electrum" {
                bitcoin_records::from_electrum(text(&i["txid"]),&i["tx"],text(&i["occurred_at"]),&scripts,backend)
            } else { bitcoin_records::from_esplora(&i["tx"],&scripts,backend) }.map(|v| v.unwrap_or(Value::Null))
        };
        match result { Ok(v) => json!({"result":v}), Err(e) => json!({"error":{"code":e.code,"message":e.message,"hint":e.hint,"details":e.details.map(|v|serde_json::from_str::<Value>(&kassiber_pyjson::dumps(&v,&Default::default()).unwrap()).unwrap())}}) }
    }).collect();
    println!("{}", json!(results));
}

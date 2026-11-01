use kassiber_wallets::analysis_features::{
    evaluate, extract, normalize, script_type, signature_encoding,
};
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mut raw = String::new();
    std::io::stdin()
        .take(16 * 1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let cases: Vec<Value> = serde_json::from_str(&raw).unwrap();
    let results: Vec<_> = cases
        .iter()
        .map(|case| {
            let source = case["source"].as_str().unwrap_or("stored_transaction");
            let subject = case["subject"].as_str();
            match case["operation"].as_str().unwrap() {
                "extract" => extract(
                    &case["raw"],
                    source,
                    subject,
                    case["signatures"].as_array().map_or(&[], Vec::as_slice),
                ),
                "normalize" => normalize(&case["snapshot"], source, subject).unwrap_or(Value::Null),
                "evaluate" => json!(evaluate(
                    &case["snapshot"],
                    case.get("collaboration").filter(|v| !v.is_null())
                )),
                "script" => json!(script_type(&case["value"])),
                "signature" => {
                    use bitcoin::hex::FromHex;
                    Vec::<u8>::from_hex(case["value"].as_str().unwrap())
                        .ok()
                        .and_then(|v| signature_encoding(&v, case["schnorr"] == true))
                        .unwrap_or(Value::Null)
                }
                other => panic!("Unknown fixture operation: {other}"),
            }
        })
        .collect();
    println!("{}", json!(results));
}

//! Byte stream for comparing the ledger under both serde_json feature sets.
fn main() {
    let document: serde_json::Value =
        serde_json::from_str(include_str!("../tests/data/ledger_fixtures.json")).unwrap();
    let mut count = 0;
    for case in document["cases"].as_array().unwrap() {
        if !case["request"].is_null() {
            println!(
                "{}\t{}",
                case["name"],
                kassiber_tax_core::ledger(&case["request"].to_string())
            );
            count += 1;
        }
    }
    assert!(count >= 100);
}

//! CPython-generated synthetic stages, independent of the binding and database.
use kassiber_custody::custody;
use serde_json::Value;
fn check(case: &str) {
    let vectors: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_evidence_cases.json"
    ))
    .unwrap();
    let v = vectors
        .iter()
        .find(|v| v["case"] == case)
        .expect("named case");
    let result: Value = serde_json::from_str(&custody(&v["input"].to_string())).unwrap();
    assert_eq!(
        result["stages"]["evidence"],
        v["evidence"],
        "{case}: {}",
        result.get("error").unwrap_or(&Value::Null)
    );
}
#[test]
fn duplicate_fee_keeper() {
    check("duplicate_fee_keeper");
}
#[test]
fn receipt_deprecated() {
    check("receipt_deprecated");
}
#[test]
fn receipt_priority() {
    check("receipt_priority");
}
#[test]
fn receipt_label() {
    check("receipt_label");
}
#[test]
fn receipt_id() {
    check("receipt_id");
}
#[test]
fn receipt_success() {
    check("receipt_success");
}
#[test]
fn receipt_unbounded_graph() {
    check("receipt_unbounded_graph");
}
#[test]
fn duplicate_slot() {
    check("duplicate_slot");
}
#[test]
fn tampered_graph() {
    check("tampered_graph");
}
#[test]
fn tampered_quantity() {
    check("tampered_quantity");
}
#[test]
fn historical_overcount() {
    check("historical_overcount");
}
#[test]
fn nonaddress_overcount() {
    check("nonaddress_overcount");
}
#[test]
fn shared_fee() {
    check("shared_fee");
}
#[test]
fn zero_delta() {
    check("zero_delta");
}
#[test]
fn owned_fee_unbounded_cancellation() {
    check("owned_fee_huge_cancel");
}
#[test]
fn owned_fee_unbounded_shape() {
    check("owned_fee_shape");
}
#[test]
fn owned_fee_float() {
    check("owned_fee_float_fee");
}
#[test]
fn owned_fee_false() {
    check("owned_fee_false_fee");
}
#[test]
fn owned_fee_nan() {
    check("owned_fee_nan_fee");
}

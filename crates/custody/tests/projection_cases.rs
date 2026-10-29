use kassiber_custody::finalized_projection;
use serde_json::Value;
fn check(op: &str) {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_primitives.json"
    ))
    .unwrap();
    let mut count = 0;
    for case in cases.as_array().unwrap().iter().filter(|c| c["op"] == op) {
        let expected: serde_json::Map<String, Value> = ["expected", "error", "message"]
            .into_iter()
            .filter_map(|key| case.get(key).map(|v| (key.to_owned(), v.clone())))
            .collect();
        assert_eq!(
            finalized_projection::primitive(case),
            Value::Object(expected),
            "{}",
            case["case"]
        );
        count += 1;
    }
    assert!(count > 0);
}
#[test]
fn projection_contract_vectors() {
    check("validate");
}
#[test]
fn projection_identity_vectors() {
    check("ids");
}
#[test]
fn projection_base_row_vectors() {
    check("base");
}
#[test]
fn projection_fee_row_vectors() {
    check("fee");
}

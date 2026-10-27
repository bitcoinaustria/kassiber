//! Each frozen Python slice has a selectable mutation witness.
use kassiber_custody::{interpretation, snapshot::Snapshot};
use serde_json::Value;

fn check(case: &str) {
    let vectors: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_interpretation_cases.json"
    ))
    .unwrap();
    let vector = vectors.iter().find(|v| v["case"] == case).unwrap();
    let snapshot: Snapshot = serde_json::from_value(vector["input"].clone()).unwrap();
    snapshot.validate().unwrap();
    assert_eq!(
        interpretation::partial(&snapshot).unwrap(),
        vector["partial"],
        "{case}"
    );
}

#[test]
fn interpretation_shared_aliases() {
    check("shared_aliases");
}
#[test]
fn interpretation_shared_reservations() {
    check("shared_reservations");
}
#[test]
fn interpretation_shared_anchor_alias() {
    check("shared_anchor_alias");
}

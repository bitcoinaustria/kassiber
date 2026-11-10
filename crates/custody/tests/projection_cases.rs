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

#[test]
fn projection_slice_pricing_vectors() {
    check("slice");
}
#[test]
fn projection_payout_admission_vectors() {
    check("payout_valid");
}
#[test]
fn projection_payout_time_vectors() {
    check("payout_time");
}
#[test]
fn projection_payout_settlement_vectors() {
    check("payout_settlement");
}
#[test]
fn projection_payout_price_vectors() {
    check("payout_price");
}
#[test]
fn projection_fee_allocation_vectors() {
    check("fees");
}

fn check_composition(op: &str) {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_composition.json"
    ))
    .unwrap();
    let mut count = 0;
    for case in cases.as_array().unwrap().iter().filter(|c| c["op"] == op) {
        let expected = if let Some(error) = case.get("error") {
            serde_json::json!({"error":error})
        } else {
            serde_json::json!({"expected":case["expected"]})
        };
        assert_eq!(
            finalized_projection::primitive(case),
            expected,
            "{}",
            case["case"]
        );
        count += 1;
    }
    assert!(count > 0);
}
#[test]
fn projection_selected_composition_vectors() {
    check_composition("compose");
}
#[test]
fn projection_connected_path_vectors() {
    check_composition("connected");
}
#[test]
fn projection_scoped_basis_vectors() {
    check_composition("basis");
}

#[test]
fn projection_reviewed_conversion_vectors() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_conversions.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let expected = if let Some(error) = case.get("error") {
            serde_json::json!({"error":error})
        } else {
            serde_json::json!({"expected":case["expected"]})
        };
        assert_eq!(
            finalized_projection::primitive(case),
            expected,
            "{}",
            case["case"]
        );
    }
}

#[test]
fn projection_without_reviewed_conversions_keeps_existing_pairs() {
    let input = serde_json::json!({
        "op":"conversions",
        "rows":[],
        "pairs":[{"pair_id":"existing","out_id":"source","in_id":"target","policy":"carrying-value"}],
        "reviewed":[],
    });
    assert_eq!(
        finalized_projection::primitive(&input),
        serde_json::json!({"expected":input["pairs"]})
    );
}

#[test]
fn projection_quarantine_composition_vectors() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_quarantines.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let expected = if let Some(error) = case.get("error") {
            serde_json::json!({"error":error})
        } else {
            serde_json::json!({"expected":case["expected"]})
        };
        assert_eq!(
            finalized_projection::primitive(case),
            expected,
            "{}",
            case["case"]
        );
    }
}

#[test]
fn projection_dependency_origins_complete_and_sorted() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_quarantines.json"
    ))
    .unwrap();
    let case = cases
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["case"] == "origins_fixed_point")
        .unwrap();
    let actual = finalized_projection::primitive(case);
    let quarantine = actual["expected"]["quarantines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["transaction_id"] == "other")
        .unwrap();
    let detail: Value = serde_json::from_str(quarantine["detail_json"].as_str().unwrap()).unwrap();
    assert_eq!(
        detail["blocked_by_transaction_ids"],
        serde_json::json!(["source", "target"])
    );
}

fn check_review(name: &str) {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_review.json"
    ))
    .unwrap();
    let case = cases
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["case"] == name)
        .unwrap();
    assert_eq!(
        finalized_projection::primitive(case),
        serde_json::json!({"expected":case["expected"]}),
        "{name}"
    );
}
#[test]
fn projection_target_blocked() {
    check_review("projection_target_blocked");
}
#[test]
fn projection_moves_sorted() {
    check_review("projection_moves_sorted");
}
#[test]
fn projection_same_asset_not_carry() {
    check_review("projection_same_asset_not_carry");
}
#[test]
fn projection_nonzero_slice_pricing() {
    check_review("projection_nonzero_slice_pricing");
}
#[test]
fn projection_inbound_fee_not_taxable() {
    check_review("projection_inbound_fee_not_taxable");
}
#[test]
fn projection_sweep_privacy() {
    check_review("projection_sweep_privacy");
}

#[test]
fn projection_review_operator_sweep() {
    use kassiber_custody::{codec, finalized_projection::runtime, snapshot::Snapshot};
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_adapter.json"
    ))
    .unwrap();
    {
        let name = "interpreter_non_event_population";
        let case = cases
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["case"] == name)
            .unwrap();
        let snapshot: Snapshot = serde_json::from_value(case["input"].clone()).unwrap();
        let quantity = codec::from_json(&case["quantity"]).unwrap();
        let interpreter = codec::from_json(&case["interpretation"]).unwrap();
        let result = runtime::stage(&snapshot, &quantity, &interpreter).unwrap();
        let expected = codec::from_json(&case["expected"]).unwrap();
        assert_eq!(
            codec::dumps(&result, true).unwrap(),
            codec::dumps(&expected, true).unwrap(),
            "{name}"
        );
    }
    check_composition("compose");
    check_composition("connected");
    check_composition("basis");
    projection_quarantine_composition_vectors();
    projection_payout_time_vectors();
    for name in [
        "projection_target_blocked",
        "projection_moves_sorted",
        "projection_same_asset_not_carry",
        "projection_nonzero_slice_pricing",
        "projection_inbound_fee_not_taxable",
        "projection_sweep_privacy",
    ] {
        check_review(name);
    }
}

#[test]
fn projection_unreviewed_payout_slice() {
    check_review("projection_unreviewed_payout_slice");
}
#[test]
fn projection_payout_component_binding() {
    check_review("projection_payout_component_binding");
}

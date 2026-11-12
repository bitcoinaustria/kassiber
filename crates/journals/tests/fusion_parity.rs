//! The pipeline's fusion builds the same ledger request and response as the
//! comparison harness's `custody_with_fusion`, which reads its inputs from
//! the request and response texts.
use kassiber_custody::codec;
use kassiber_journals::{compute, Fusion};
use serde_json::{json, Value};

fn harness_fusion(request: &str) -> Value {
    let response = kassiber_custody::custody_with_fusion(request, true, kassiber_tax_core::ledger);
    let response = codec::to_json(&codec::loads(&response).expect("custody response"));
    response["stages"]["fusion"].clone()
}

fn pipeline_fusion(request: &str) -> Value {
    match compute(request, true).expect("admitted snapshot").fusion {
        Fusion::Skipped => panic!("a finalized build fuses"),
        Fusion::Empty => json!({"outcome":"empty"}),
        Fusion::Refused(error) => json!({"outcome":"refusal","error":error}),
        Fusion::Ledger { request, response } => {
            let response: Value = serde_json::from_str(&response).expect("ledger response");
            json!({"outcome":"request","request_text":request,"response":response})
        }
    }
}

#[test]
fn pipeline_fusion_matches_the_harness() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_stage.json"
    ))
    .unwrap();
    let mut outcomes = std::collections::BTreeMap::new();
    for case in cases.as_array().unwrap() {
        let request = case["input"].to_string();
        if compute(&request, true).is_err() {
            *outcomes.entry("upstream refusal").or_insert(0) += 1;
            continue;
        }
        let mut expected = harness_fusion(&request);
        if let Some(object) = expected.as_object_mut() {
            // The harness also echoes the request document; its wire text is compared.
            object.remove("request");
        }
        let actual = pipeline_fusion(&request);
        assert_eq!(actual, expected, "{}", case["case"]);
        *outcomes
            .entry(match actual["outcome"].as_str().unwrap() {
                "request" => "request",
                "empty" => "empty",
                _ => "refusal",
            })
            .or_insert(0) += 1;
    }
    // The fixture must exercise the engine, not only empty projections.
    assert!(
        outcomes.get("request").copied().unwrap_or(0) > 0,
        "{outcomes:?}"
    );
}

#[test]
fn decision_only_builds_skip_fusion() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_stage.json"
    ))
    .unwrap();
    let request = cases[0]["input"].to_string();
    let computed = compute(&request, false).expect("admitted snapshot");
    assert!(computed.stages.finalized_projection.is_none());
    assert_eq!(computed.fusion, Fusion::Skipped);
}

#[test]
fn typed_engine_boundary_matches_text_and_refusal_attributes() {
    use kassiber_journals::{boundary, engine};
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tax-engine/core/tests/data/ledger_fixtures.json"
    ))
    .unwrap();
    let mut successes = 0;
    let mut refusals = 0;
    for case in cases["cases"].as_array().unwrap() {
        let request = &case["request"];
        if request.is_null() {
            continue;
        }
        let text = kassiber_custody::ledger_request::request_json(request);
        let expected = kassiber_tax_core::ledger(&text);
        let expected: Value = serde_json::from_str(&expected).unwrap();
        let expected = boundary::ledger_response(&expected);
        let actual = engine::ledger(request);
        match (actual, expected) {
            (Ok(actual), Ok(expected)) => {
                successes += 1;
                assert_eq!(
                    actual.to_json().unwrap(),
                    expected.to_json().unwrap(),
                    "{}",
                    case["name"]
                );
            }
            (Err(actual), Err(expected)) => {
                refusals += 1;
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(expected).unwrap(),
                    "{}",
                    case["name"]
                );
            }
            _ => panic!("boundary outcome differs: {}", case["name"]),
        }
    }
    assert!(successes >= 100);
    assert!(refusals >= 25);
}

#[test]
fn typed_engine_boundary_keeps_wire_columns_for_wide_amounts() {
    use kassiber_journals::{boundary, engine};
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tax-engine/core/tests/data/ledger_fixtures.json"
    ))
    .unwrap();
    let mut request = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| &case["request"])
        .find(|request| {
            request["rows"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        })
        .unwrap()
        .clone();
    for field in ["amount_msat", "fee_msat"] {
        request["rows"][0][field] = serde_json::from_str("18446744073709551616").unwrap();
        let text = kassiber_custody::ledger_request::request_json(&request);
        let expected: Value = serde_json::from_str(&kassiber_tax_core::ledger(&text)).unwrap();
        let expected = boundary::ledger_response(&expected).unwrap_err();
        let actual = engine::ledger(&request).unwrap_err();
        assert!(actual.message.contains(" at line 1 column "));
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        request["rows"][0][field] = json!(1);
    }
}

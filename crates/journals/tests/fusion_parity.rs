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

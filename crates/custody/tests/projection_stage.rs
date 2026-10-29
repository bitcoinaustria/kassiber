use kassiber_custody::{codec, finalized_projection::runtime, snapshot::Snapshot};
use kassiber_pyjson::{Key, Value as Py};

fn get<'a>(v: &'a Py, name: &str) -> &'a Py {
    match v {
        Py::Dict(entries) => {
            &entries
                .iter()
                .find(|(k, _)| *k == Key::Str(name.into()))
                .expect(name)
                .1
        }
        _ => panic!("expected record"),
    }
}

#[test]
fn projection_raw_adapter_vectors() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_stage.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let raw = case["input"].to_string();
        let response =
            codec::loads(&kassiber_custody::custody_with_finalized(&raw, false)).unwrap();
        let stages = get(&response, "stages");
        let snapshot: Snapshot = serde_json::from_str(&raw).unwrap();
        let result = runtime::stage(
            &snapshot,
            get(stages, "quantity"),
            get(stages, "interpretation"),
        );
        if let Some(error) = case["expected"].get("error") {
            assert_eq!(
                result.unwrap_err(),
                error.as_str().unwrap(),
                "{}",
                case["case"]
            );
        } else {
            let expected = codec::from_json(&case["expected"]).unwrap();
            assert_eq!(
                codec::dumps(&result.unwrap(), true).unwrap(),
                codec::dumps(&expected, true).unwrap(),
                "{}",
                case["case"]
            );
        }
    }
}

#[test]
fn projection_adapter_input_families() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_adapter.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let snapshot: Snapshot = serde_json::from_value(case["input"].clone()).unwrap();
        let mut quantity = codec::from_json(&case["quantity"]).unwrap();
        if case["case"] == "ordinary_rows" {
            // Ignored details must not cross exact serde admission. Python's
            // compiler skips this issue before reading its surrogate detail.
            let issues = codec::loads(r#"[{"reason":"transfer_fee_implausible","issue_type":"custody_interpreter_blocked","transaction_ids":["source"],"issue_id":"ignored","details":{"ignored":"\ud800"}}]"#).unwrap();
            let Py::Dict(entries) = &mut quantity else {
                panic!("quantity record")
            };
            entries
                .iter_mut()
                .find(|(key, _)| *key == Key::Str("issues".into()))
                .unwrap()
                .1 = issues;
        }
        let interpreter = codec::from_json(&case["interpretation"]).unwrap();
        let result = runtime::stage(&snapshot, &quantity, &interpreter);
        if let Some(error) = case["expected"].get("error") {
            assert_eq!(
                result.unwrap_err(),
                error.as_str().unwrap(),
                "{}",
                case["case"]
            );
        } else {
            let expected = codec::from_json(&case["expected"]).unwrap();
            assert_eq!(
                codec::dumps(&result.unwrap(), true).unwrap(),
                codec::dumps(&expected, true).unwrap(),
                "{}",
                case["case"]
            );
        }
    }
}

#[test]
fn projection_capture_precedes_rates() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_admission.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        for finalized in [false, true] {
            let response: serde_json::Value = serde_json::from_str(
                &kassiber_custody::custody_with_finalized(&case["input"].to_string(), finalized),
            )
            .unwrap();
            assert_eq!(response["error"]["code"], case["expected"]["error"]);
            assert_eq!(response["error"]["stage"], case["expected"]["stage"]);
            assert!(response.get("stages").is_none());
        }
    }
}

#[test]
fn projection_adapter_lossless_diagnostics() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_adapter.json"
    ))
    .unwrap();
    let case = &cases[0];
    let snapshot: Snapshot = serde_json::from_value(case["input"].clone()).unwrap();
    let quantity = codec::from_json(&case["quantity"]).unwrap();
    let interpreter = codec::loads(r#"{"non_event_transaction_ids":[],"blocked_transaction_ids":[],"quarantines":[{"transaction_id":"\ud800","reason":"r","detail":{"n":NaN,"i":Infinity,"id":"\ud801"}}]}"#).unwrap();
    let result = runtime::stage(&snapshot, &quantity, &interpreter).unwrap();
    assert_eq!(
        codec::dumps(get(&result, "quarantines"), true).unwrap(),
        codec::dumps(get(&interpreter, "quarantines"), true).unwrap()
    );
}

#[test]
fn projection_unknown_surrogate_blocker_is_irrelevant() {
    let Py::List(cases) = codec::loads(include_str!(
        "../../../tests/fixtures/custody_interpretation_wrappers.json"
    ))
    .unwrap() else {
        panic!("wrapper vectors")
    };
    let case = cases
        .iter()
        .find(|c| get(c, "case") == &Py::Str("migration_1_0".into()))
        .unwrap();
    let raw = codec::dumps(get(case, "input"), true).unwrap();
    let response = codec::loads(&kassiber_custody::custody(&raw)).unwrap();
    let projection = get(get(&response, "stages"), "finalized_projection");
    let Py::List(rows) = get(projection, "rows") else {
        panic!("projection rows")
    };
    assert_eq!(rows.len(), 2);
    assert!(codec::dumps(get(projection, "quarantines"), true)
        .unwrap()
        .contains("\\ud800"));
}

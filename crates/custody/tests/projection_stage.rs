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
        let response = codec::loads(&kassiber_custody::custody(&raw)).unwrap();
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
        let quantity = codec::from_json(&case["quantity"]).unwrap();
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

use kassiber_custody::{codec, finalized_projection::runtime, snapshot::Snapshot};
use kassiber_pyjson::{Key, Options, Value as Py};

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
fn projection_loaded_rows_keep_bytes_until_serialization() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_stage.json"
    ))
    .unwrap();
    for bytes in [vec![], vec![255, 0], b"77".to_vec()] {
        let mut input = cases[0]["input"].clone();
        for row in input["transactions"].as_array_mut().unwrap() {
            row["note"] = codec::to_json(&Py::Bytes(bytes.clone()));
        }
        let snapshot: Snapshot = serde_json::from_value(input).unwrap();
        let stages = kassiber_custody::run_loaded(&snapshot, true).unwrap();
        let projection = stages.finalized_projection.unwrap();
        let Py::List(rows) = get(&projection, "rows") else {
            panic!("projection rows")
        };
        assert_eq!(rows.len(), 2);
        for row in rows {
            assert_eq!(get(row, "note"), &Py::Bytes(bytes.clone()));
        }
        assert_eq!(
            kassiber_pyjson::dumps(&projection, &Options::new())
                .unwrap_err()
                .to_string(),
            "Object of type bytes is not JSON serializable"
        );
    }
}

#[test]
fn projection_json_rows_keep_literal_private_tag_maps() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_stage.json"
    ))
    .unwrap();
    for note in [
        serde_json::json!({"$kassiber_bytes":"ff00"}),
        serde_json::json!({"$kassiber_dict":{"nested":{"$kassiber_bytes":"ff00"}}}),
    ] {
        let mut input = cases[0]["input"].clone();
        for row in input["transactions"].as_array_mut().unwrap() {
            row["note"] = note.clone();
        }
        let snapshot = kassiber_custody::parse(&input.to_string()).unwrap();
        let stages = kassiber_custody::run(&snapshot, true).unwrap();
        let projection = stages.finalized_projection.unwrap();
        let Py::List(rows) = get(&projection, "rows") else {
            panic!("projection rows")
        };
        assert_eq!(rows.len(), 2);
        let expected = codec::loads(&note.to_string()).unwrap();
        for row in rows {
            assert_eq!(get(row, "note"), &expected);
        }
        assert!(codec::dumps(&projection, true).is_ok());
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
fn projection_borrowed_rows_match_materialized_rows() {
    use serde_json::{json, Map, Value};
    let mut cases: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_adapter.json"
    ))
    .unwrap();
    let mut empty = cases
        .iter()
        .find(|c| c["case"] == "ordinary_rows")
        .unwrap()
        .clone();
    empty["case"] = json!("empty_source_row");
    empty["input"]["transactions"][0] = json!({});
    empty["quantity"]["projection"]["observations"][0]["transaction_id"] = json!("");
    cases.push(empty);
    for case in cases {
        let snapshot: Snapshot = serde_json::from_value(case["input"].clone()).unwrap();
        let quantity = codec::from_json(&case["quantity"]).unwrap();
        let interpreter = codec::from_json(&case["interpretation"]).unwrap();
        for count in [
            0,
            snapshot.transactions.len().saturating_sub(1),
            snapshot.transactions.len(),
        ] {
            let captured: Vec<_> = (0..count).map(|index| json!({"row":index})).collect();
            let mut rows = Map::new();
            for (row, confirmation) in snapshot.transactions.iter().zip(&captured) {
                let mut row = Value::Object(row.clone().into_map());
                row["_closed_confirmation_observation"] = confirmation.clone();
                let id = if codec::truth(&row["id"]) {
                    codec::pystr(&row["id"])
                } else {
                    String::new()
                };
                rows.insert(id, row);
            }
            let encode = |result: Result<Py, kassiber_custody::finalized_projection::Error>| {
                result
                    .map(|v| codec::dumps(&v, true).unwrap())
                    .map_err(|e| (e.code, e.message))
            };
            assert_eq!(
                encode(runtime::stage_with_confirmations(
                    &snapshot,
                    captured,
                    &quantity,
                    &interpreter
                )),
                encode(runtime::stage_with_rows(
                    &snapshot,
                    Value::Object(rows),
                    &quantity,
                    &interpreter
                )),
                "{}:{count}",
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

#[test]
fn projection_unused_event_fields_still_require_exact_admission() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_adapter.json"
    ))
    .unwrap();
    let case = cases
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["case"] == "ordinary_rows")
        .unwrap();
    let snapshot: Snapshot = serde_json::from_value(case["input"].clone()).unwrap();
    let interpreter = codec::from_json(&case["interpretation"]).unwrap();
    for events in [
        r#"{"events":[{"unused":"\ud800"}]}"#,
        r#"{"events":[{"unused":NaN}]}"#,
    ] {
        let mut quantity = codec::from_json(&case["quantity"]).unwrap();
        let Py::Dict(entries) = &mut quantity else {
            panic!("quantity")
        };
        entries
            .iter_mut()
            .find(|(key, _)| *key == Key::Str("canonical_input".into()))
            .unwrap()
            .1 = codec::loads(events).unwrap();
        assert_eq!(
            runtime::stage(&snapshot, &quantity, &interpreter).unwrap_err(),
            "ValueError"
        );
    }
}

#[test]
fn projection_receipts_before_barrier_keep_event_order() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_adapter.json"
    ))
    .unwrap();
    let case = cases
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["case"] == "barrier_held_receipt")
        .unwrap();
    let snapshot: Snapshot = serde_json::from_value(case["input"].clone()).unwrap();
    let interpreter = codec::from_json(&case["interpretation"]).unwrap();
    let mut quantity = case["quantity"].clone();
    quantity["tax_eligibility"]["pool_barriers"][0][1][0] =
        serde_json::json!("3000-01-01T00:00:00Z");
    let result = runtime::stage(
        &snapshot,
        &codec::from_json(&quantity).unwrap(),
        &interpreter,
    )
    .unwrap();
    let Py::List(rows) = get(&result, "rows") else {
        panic!("rows")
    };
    assert_eq!(rows.len(), 3);
    assert!(rows
        .iter()
        .any(|row| get(row, "journal_transaction_id") == &Py::Str("other".into())));
    assert_eq!(get(&result, "quarantines"), &Py::List(vec![]));
}

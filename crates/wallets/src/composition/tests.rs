use super::*;

#[test]
fn observer_composition_matches_python_vectors() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../tests/vectors/python_bitcoin_records.json"
    ))
    .unwrap();
    for case in cases
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["operation"] == "compose")
    {
        let input = &case["input"];
        let observers: Vec<_> = entries(input, "observers")
            .iter()
            .map(|o| ObserverFacts {
                observer_id: text(&o["observer_id"]).into(),
                observer_kind: text(&o["observer_kind"]).into(),
                records: entries(o, "records").to_vec(),
                outputs: entries(o, "outputs").to_vec(),
                retracted_external_ids: entries(o, "retracted_external_ids")
                    .iter()
                    .map(|v| text(v).to_owned())
                    .collect(),
                checkpoint: o.get("checkpoint").cloned().unwrap_or(json!({})),
                reused_records: o["reused_records"].as_u64().unwrap_or(0),
            })
            .collect();
        let metadata = input.get("adapter_metadata").cloned().unwrap_or(json!({}));
        let result = compose(
            &observers,
            entries(input, "adapter_records"),
            &metadata,
            "fixture",
        );
        let actual = match result {
            Ok(c) => {
                json!({"result":{"records":c.records,"metadata":c.metadata,"provenance_entries":c.provenance_entries}})
            }
            Err(e) => {
                json!({"error":{"code":e.code,"message":e.message,"hint":e.hint,"details":e.details.map(|v| serde_json::from_str::<Value>(&kassiber_pyjson::dumps(&v,&Default::default()).unwrap()).unwrap())}})
            }
        };
        assert_eq!(actual, case["expected"], "{}", case["name"]);
    }
}

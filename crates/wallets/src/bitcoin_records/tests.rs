use super::*;

#[test]
fn bitcoin_records_match_python_vectors() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../tests/vectors/python_bitcoin_records.json"
    ))
    .unwrap();
    for case in cases
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["operation"] != "compose")
    {
        let input = &case["input"];
        let scripts = entries(input, "scripts")
            .iter()
            .map(|v| text(v).to_owned())
            .collect();
        let result = if case["operation"] == "electrum" {
            from_electrum(
                text(&input["txid"]),
                &input["tx"],
                text(&input["occurred_at"]),
                &scripts,
                "fixture",
            )
        } else {
            from_esplora(&input["tx"], &scripts, "fixture")
        };
        let actual = match result {
            Ok(record) => json!({"result":record}),
            Err(e) => {
                json!({"error":{"code":e.code,"message":e.message,"hint":e.hint,"details":e.details.map(|v| serde_json::from_str::<Value>(&dumps(&v,&Default::default()).unwrap()).unwrap())}})
            }
        };
        assert_eq!(actual, case["expected"], "{}", case["name"]);
    }
}

#[test]
fn bitcoin_projection_preserves_integral_values_and_refuses_overflow() {
    assert_eq!(value_sats(&json!({"value":true})).unwrap(), None);
    assert_eq!(value_sats(&json!({"value_sats":"nonsense"})).unwrap(), None);
    assert_eq!(
        value_sats(&json!({"value":"0.000000015"})).unwrap(),
        Some(2)
    );
    assert_eq!(
        value_sats(&json!({"value":"0.000000025"})).unwrap(),
        Some(2)
    );
    let tx = json!({"txid":"a","vin":[],"vout":[{"scriptpubkey":"51","value":i128::MAX.to_string()},{"scriptpubkey":"51","value":2}]});
    assert_eq!(
        from_esplora(&tx, &BTreeSet::from(["51".into()]), "fixture")
            .unwrap_err()
            .code,
        "protocol_error"
    );
    assert_eq!(
        timestamp(&json!(253_402_300_800_i64)).unwrap_err().code,
        "protocol_error"
    );
}

#[test]
fn electrum_raw_graph_preserves_python_decimal_json_bytes() {
    let tx = json!({"vin":[],"vout":[{"scriptpubkey":"51","value":99001,"value_sats":99001}],"total_output_sats":99001,"_kassiber_electrum_graph":{"kind":"bitcoin_electrum","version":1}});
    let record = from_electrum(
        "tx",
        &tx,
        "2023-11-14T22:13:20Z",
        &BTreeSet::from(["51".into()]),
        "fixture",
    )
    .unwrap()
    .unwrap();
    assert_eq!(record["amount"], "0.00099001");
    assert_eq!(record["raw_json"],"{\"_kassiber_electrum_graph\": {\"kind\": \"bitcoin_electrum\", \"version\": 2}, \"fee\": 0, \"observer_owned_scripts\": [\"51\"], \"status\": {\"confirmed\": true}, \"total_output_sats\": 99001.0, \"vin\": [], \"vout\": [{\"scriptpubkey\": \"51\", \"value\": 99001, \"value_sats\": 99001.0}]}");
}

#[test]
fn bitcoin_malformed_graphs_refuse_before_projecting_related_outputs() {
    let scripts = BTreeSet::from(["51".into()]);
    for tx in [
        json!([]),
        json!("opaque"),
        json!({"vin":null,"vout":[{"scriptpubkey":"51","value":10}]}),
        json!({"vin":1,"vout":[{"scriptpubkey":"51","value":10}]}),
        json!({"vin":[],"vout":{}}),
    ] {
        assert_eq!(
            from_esplora(&tx, &scripts, "fixture").unwrap_err().code,
            "protocol_error"
        );
        assert_eq!(
            from_electrum("id", &tx, UNKNOWN, &scripts, "fixture")
                .unwrap_err()
                .code,
            "protocol_error"
        );
    }
    let tx = json!({"vin":[],"vout":[{"scriptpubkey":"51","value":10}],"status":"unproved"});
    assert_eq!(
        from_esplora(&tx, &scripts, "fixture").unwrap_err().code,
        "protocol_error"
    );
}

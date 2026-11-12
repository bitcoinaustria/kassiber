use kassiber_custody::snapshot::{RateRow, SqlValue, TxRow, WalletRef, WorklistRow};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

fn roundtrip<T: DeserializeOwned + Serialize>(text: &str) -> T {
    let row: T = serde_json::from_str(text).unwrap();
    assert_eq!(serde_json::to_string(&row).unwrap(), text);
    row
}

#[test]
fn typed_rows_keep_presence_order_unknown_columns_and_storage_classes() {
    let row = roundtrip::<TxRow>(
        r#"{"fee":null,"amount":9223372036854775807,"id":"tx","future":{"z":[true,3.0],"a":null},"raw_json":{},"fiat_rate":-0.0}"#,
    );
    assert!(row.fee.present().unwrap().is_null());
    assert!(row.wallet_id.present().is_none());
    assert!(matches!(row.amount.value(), SqlValue::Int(i64::MAX)));
    assert!(matches!(row.fiat_rate.value(), SqlValue::Real(v) if v.is_sign_negative()));
    assert_eq!(row.json()["future"], json!({"z":[true,3.0],"a":null}));
    roundtrip::<WalletRef>(r#"{"config_json":"{}","kind":"custom","id":"wallet"}"#);
    roundtrip::<RateRow>(
        r#"{"fiat_rate_exact":"1.00000000000000001","asset":"BTC","fiat_rate":1.0}"#,
    );
    roundtrip::<WorklistRow>(
        r#"{"amount":"123","excluded":false,"fee":null,"wallet_config_json":{}}"#,
    );
}

#[test]
fn typed_scalars_preserve_historical_transport_shapes() {
    for text in [
        "null",
        "true",
        "false",
        "-9223372036854775808",
        "18446744073709551616",
        "-9223372036854775809",
        "340282366920938463463374607431768211456",
        "0.0",
        "-0.0",
        "1.0000000000000002",
        r#""12""#,
        r#"{"z":[1,{}],"a":false}"#,
    ] {
        let value: SqlValue = serde_json::from_str(text).unwrap();
        let expected: Value = serde_json::from_str(text).unwrap();
        assert_eq!(value.json(), expected, "{text}");
        assert_eq!(serde_json::to_string(&value).unwrap(), text);
        let value_text = expected.to_string();
        let from_value: SqlValue = serde_json::from_value(expected).unwrap();
        assert_eq!(serde_json::to_string(&from_value).unwrap(), value_text);
    }
}

#[test]
fn typed_sql_quantities_refuse_float_bool_text_and_out_of_range() {
    let text = include_str!("../../../tests/fixtures/custody_projection_stage.json");
    let cases: Value = serde_json::from_str(text).unwrap();
    let mut input = cases[0]["input"].clone();
    assert!(!input["transactions"].as_array().unwrap().is_empty());
    for amount in [
        json!(1.0),
        json!(true),
        json!("1"),
        serde_json::from_str("9223372036854775808").unwrap(),
    ] {
        input["transactions"][0]["amount"] = amount;
        let error = kassiber_custody::parse(&input.to_string()).unwrap_err();
        assert_eq!(error.code, "custody_snapshot_invalid");
        assert_eq!(
            error.message,
            "amount must be an exact signed-64 msat integer"
        );
    }
}

#[test]
fn typed_identifiers_share_only_within_their_book() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_stage.json"
    ))
    .unwrap();
    let mut input = cases[0]["input"].clone();
    input["wallets"][0]["id"] = json!("same-wallet");
    input["transactions"][0]["wallet_id"] = json!("same-wallet");
    let text = input.to_string();
    let first = kassiber_custody::parse(&text).unwrap();
    let wallet = first.wallets[0].id.value().identifier().unwrap();
    let row = first.transactions[0]
        .wallet_id
        .value()
        .identifier()
        .unwrap();
    assert!(wallet.shares_storage(row));
    let second = kassiber_custody::parse(&text).unwrap();
    let other = second.wallets[0].id.value().identifier().unwrap();
    assert_eq!(wallet.as_str(), other.as_str());
    assert!(!wallet.shares_storage(other));
}

#[test]
fn typed_network_containers_wait_for_accounting_admission() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_stage.json"
    ))
    .unwrap();
    for facts in [
        json!({}),
        json!({"observations":null,"references":[]}),
        json!({"observations":{},"references":[]}),
        json!([]),
    ] {
        let mut input = cases[0]["input"].clone();
        input["book_network"] = facts;
        let snapshot = kassiber_custody::parse(&input.to_string()).unwrap();
        let error = kassiber_custody::run(&snapshot, false).err().unwrap();
        assert_eq!(error.stage, "components");
        assert_eq!(error.code, "custody_snapshot_invalid");
        assert_eq!(error.message, "missing network observations");
    }
}

#[test]
fn typed_snapshot_keeps_absent_nullable_fields_absent() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_stage.json"
    ))
    .unwrap();
    let mut input = cases[0]["input"].clone();
    input.as_object_mut().unwrap().remove("book_network");
    input.as_object_mut().unwrap().remove("owned_index");
    let snapshot = kassiber_custody::parse(&input.to_string()).unwrap();
    assert!(snapshot.book_network.is_none());
    assert!(snapshot.owned_index.is_none());
    assert_eq!(serde_json::to_value(snapshot).unwrap(), input);
}

#[test]
fn typed_network_scalar_storage_classes() {
    use kassiber_custody::snapshot::{NetworkArray, NetworkFacts, NetworkObject, NetworkRow};
    for text in ["18446744073709551616", "-9223372036854775809"] {
        let value: Value = serde_json::from_str(text).unwrap();
        let object: NetworkObject<NetworkFacts> = serde_json::from_value(value.clone()).unwrap();
        let array: NetworkArray<NetworkObject<NetworkRow>> = serde_json::from_value(value).unwrap();
        assert_eq!(serde_json::to_string(&object).unwrap(), text);
        assert_eq!(serde_json::to_string(&array).unwrap(), text);
    }
}

/// Every stage and fusion, sorted, as Python would print them.
fn canonical_stages(response: kassiber_pyjson::Value) -> String {
    kassiber_custody::codec::dumps(&response, true).unwrap()
}

#[test]
fn loaded_encoding_answers_like_the_v1_adapter_for_v1_books() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_stage.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        let request = case["input"].to_string();
        for finalized in [false, true] {
            let v1 = kassiber_custody::custody_with_fusion(
                &request,
                finalized,
                kassiber_tax_core::ledger,
            );
            let loaded = kassiber_custody::custody_loaded_with_fusion(
                &request,
                finalized,
                kassiber_tax_core::ledger,
            );
            let loaded: Value = serde_json::from_str(&loaded).unwrap();
            assert_eq!(
                canonical_stages(kassiber_custody::codec::from_json(&loaded).unwrap()),
                canonical_stages(kassiber_custody::codec::loads(&v1).unwrap()),
                "{} finalized={finalized}",
                case["case"]
            );
        }
    }
}

#[test]
fn loaded_encoding_keeps_the_legacy_scalars_the_v1_adapter_refuses() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/custody_projection_stage.json"
    ))
    .unwrap();
    let mut input = cases[0]["input"].clone();
    for (column, value) in [
        ("amount", json!(1.5)),
        ("fee", json!("unavailable")),
        ("amount", json!({"$kassiber_bytes": "3737"})),
    ] {
        input["transactions"][0][column] = value.clone();
        let request = input.to_string();
        let v1: Value = serde_json::from_str(&kassiber_custody::custody_with_fusion(
            &request,
            true,
            kassiber_tax_core::ledger,
        ))
        .unwrap();
        assert_eq!(v1["error"]["code"], "custody_snapshot_invalid", "{value}");
        let snapshot: kassiber_custody::snapshot::Snapshot =
            serde_json::from_str(&request).unwrap();
        let loaded: Value = serde_json::from_str(&kassiber_custody::custody_loaded_with_fusion(
            &request,
            true,
            kassiber_tax_core::ledger,
        ))
        .unwrap();
        match kassiber_custody::run_loaded(&snapshot, true) {
            Err(refusal) => {
                let expected: Value = serde_json::from_str(&refusal.document()).unwrap();
                assert_eq!(loaded, expected, "{value}");
            }
            Ok(stages) => assert_eq!(loaded["stages"]["components"], stages.components),
        }
        input = cases[0]["input"].clone();
    }
}

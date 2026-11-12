//! Pins the lossless seam independently of the serde-only fixture driver.
use kassiber_custody::{codec, finalized_projection::compose_lossless};
use kassiber_pyjson::{Options, Value as Py};
use serde_json::json;

fn compose(diagnostics: &str) -> Result<String, String> {
    let Py::List(diagnostics) = codec::loads(diagnostics)? else {
        panic!("list fixture")
    };
    let output = compose_lossless(
        &json!({"op":"finalize", "rows":{}, "payouts":[], "reviewed":[]}),
        &diagnostics,
    )?;
    kassiber_pyjson::dumps(&output, &Options::new().sort_keys(true)).map_err(|e| e.to_string())
}
fn quarantines(diagnostics: &str) -> String {
    let output = compose(diagnostics).unwrap();
    let expected = format!(
        "{{\"$type\": \"FinalizedTaxProjection\", \"cross_asset_pairs\": [], \"intra_pairs\": [], \"quarantines\": {diagnostics}, \"rows\": [], \"selected_move_ids\": []}}"
    );
    assert_eq!(output, expected);
    output
}
#[test]
fn projection_lossless_special_values() {
    quarantines(
        r#"[{"detail_json": {"huge": 100000000000000000000000000000000000000000000000000, "inf": Infinity, "nan": NaN, "negative": -Infinity, "zero": -0.0}, "reason": "r", "transaction_id": "a"}]"#,
    );
}
#[test]
fn projection_lossless_surrogate_keys_and_values() {
    quarantines(
        r#"[{"reason": "r", "transaction_id": "\ud800"}, {"reason": "r", "transaction_id": "\udfff"}]"#,
    );
    quarantines(
        r#"[{"detail_json": {"\ud800": ["\udfff", "\ud800x"]}, "reason": "\udfff", "transaction_id": "\ud800"}]"#,
    );
}
#[test]
fn projection_lossless_order_and_last_write() {
    let output = compose(
        r#"[
        {"transaction_id":"b","reason":"z","detail_json":"first"},
        {"transaction_id":"a","reason":"z","detail_json":"second"},
        {"transaction_id":"b","reason":"a","detail_json":"third"},
        {"transaction_id":"b","reason":"z","detail_json":"last"}
    ]"#,
    )
    .unwrap();
    assert_eq!(output, "{\"$type\": \"FinalizedTaxProjection\", \"cross_asset_pairs\": [], \"intra_pairs\": [], \"quarantines\": [{\"detail_json\": \"second\", \"reason\": \"z\", \"transaction_id\": \"a\"}, {\"detail_json\": \"third\", \"reason\": \"a\", \"transaction_id\": \"b\"}, {\"detail_json\": \"last\", \"reason\": \"z\", \"transaction_id\": \"b\"}], \"rows\": [], \"selected_move_ids\": []}");
}
#[test]
fn projection_lossless_missing_reason_refusal() {
    // A fallback grouping key does not fill in the copied record. Sorting
    // still indexes the original reason and must refuse, as Python does.
    assert_eq!(
        compose(r#"[{"transaction_id":"a"}]"#),
        Err("KeyError".into())
    );
    assert!(compose(r#"[{"transaction_id":"","reason":"r"}]"#).is_ok());
}

//! The JSON boundary: request validation and response shapes. RP2 parity
//! itself is checked by `engine_fixtures`.

use kassiber_tax_core::{check_entry, compute};
use serde_json::{json, Value};

fn ts() -> Value {
    json!({
        "raw": "2024-01-01T00:00:00Z",
        "us": 1_704_067_200_000_000_i64,
        "offset_s": 0,
        "date": "2024-01-01",
        "year": 2024,
        "vienna_date": "2024-01-01",
        "display": "2024-01-01 00:00:00+00:00",
        "iso": "2024-01-01T00:00:00+00:00"
    })
}

fn buy() -> Value {
    json!({
        "kind": "in",
        "row": 1,
        "unique_id": "b1",
        "asset": "BTC",
        "transaction_type": "BUY",
        "exchange": "A",
        "holder": "Profile",
        "spot_price": "100E0",
        "crypto_in": "1E0",
        "fiat_in_no_fee": "100E0",
        "fiat_in_with_fee": "100E0",
        "fiat_fee": "0E0",
        "ts": ts(),
        "text": ""
    })
}

fn compute_request(method: &str, operation: &str, assets: Value) -> String {
    json!({
        "schema_version": 1,
        "operation": operation,
        "country": {"kind": "generic", "long_term_days": 365},
        "method": method,
        "assets": assets
    })
    .to_string()
}

fn one_asset() -> Value {
    json!([{"asset": "BTC", "entries": [buy()]}])
}

/// The error class and message of a response, or `None` when it succeeded.
fn error(response: &str) -> Option<(String, String)> {
    let value: Value = serde_json::from_str(response).expect("response is JSON");
    assert_eq!(value["schema_version"], 1, "{response}");
    if value["ok"] == Value::Bool(true) {
        return None;
    }
    Some((
        value["error"]["class"].as_str()?.to_owned(),
        value["error"]["message"].as_str()?.to_owned(),
    ))
}

fn check(entry: Value) -> Option<(String, String)> {
    error(&check_entry(
        &json!({"schema_version": 1, "operation": "check_entry", "entry": entry}).to_string(),
    ))
}

#[test]
fn a_valid_entry_and_history_succeed() {
    assert_eq!(check(buy()), None);
    let response = compute(&compute_request("fifo", "compute", one_asset()));
    assert_eq!(error(&response), None);
    let value: Value = serde_json::from_str(&response).expect("response is JSON");
    assert_eq!(value["assets"][0]["in_transactions"], json!([1]));
    assert_eq!(value["assets"][0]["balances"][0]["final_balance"], "1E0");
}

#[test]
fn malformed_requests_are_request_errors() {
    let class = |response: String| error(&response).map(|(class, _)| class);
    let request_error = Some("RequestError".to_owned());
    assert_eq!(class(check_entry("not json")), request_error);
    assert_eq!(class(compute("{}")), request_error);
    assert_eq!(
        class(check_entry(
            &json!({"schema_version": 2, "entry": buy()}).to_string()
        )),
        request_error
    );
    assert_eq!(
        class(check_entry(
            &json!({"schema_version": 1, "operation": "compute", "entry": buy()}).to_string()
        )),
        request_error
    );
    let mut entry = buy();
    entry["crypto_in"] = json!("1.5");
    assert_eq!(check(entry).map(|(class, _)| class), request_error);
    let mut entry = buy();
    entry["crypto_sent"] = json!("1E0");
    assert_eq!(check(entry).map(|(class, _)| class), request_error);
    let mut entry = buy();
    entry["unexpected"] = json!(true);
    assert_eq!(check(entry).map(|(class, _)| class), request_error);
    let mut entry = buy();
    entry["ts"]["date"] = json!("2024-01-02");
    assert_eq!(check(entry).map(|(class, _)| class), request_error);
    let two = json!([
        {"asset": "BTC", "entries": [buy()]},
        {"asset": "BTC", "entries": [buy()]}
    ]);
    assert_eq!(
        class(compute(&compute_request("fifo", "compute", two))),
        request_error
    );
    assert_eq!(
        class(compute(&compute_request("fifo", "compute", json!([])))),
        request_error
    );
}

#[test]
fn country_hooks_answer_without_assets_outside_austria() {
    let parse =
        |response: String| -> Value { serde_json::from_str(&response).expect("response is JSON") };
    // Generic books have no country hooks: validation passes and the
    // multi-asset runner declines, so the caller computes each asset.
    let validated = parse(compute(&compute_request("fifo", "validate", one_asset())));
    assert_eq!(validated, json!({"schema_version": 1, "ok": true}));
    let multi = parse(compute(&compute_request(
        "moving_average",
        "compute_multi",
        one_asset(),
    )));
    assert_eq!(
        multi,
        json!({"schema_version": 1, "ok": true, "handled": false})
    );
    let two = json!([
        {"asset": "BTC", "entries": [buy()]},
        {"asset": "BTC", "entries": [buy()]}
    ]);
    assert_eq!(
        error(&compute(&compute_request("fifo", "validate", two))).map(|(class, _)| class),
        Some("RequestError".to_owned())
    );
    for method in ["moving_average", "moving_average_at"] {
        let response = compute(&compute_request(method, "compute", one_asset()));
        assert_eq!(error(&response), None, "{method}");
    }
}

#[test]
fn parse_errors_keep_their_class_and_position() {
    // The asset check precedes the timestamp check, as in RP2.
    let mut entry = buy();
    entry["ts"] = json!({"raw": "2024-01-01", "error": "Parameter 'timestamp' value has no timezone info: 2024-01-01"});
    assert_eq!(
        check(entry.clone()),
        Some((
            "ValueError".to_owned(),
            "Parameter 'timestamp' value has no timezone info: 2024-01-01".to_owned()
        ))
    );
    entry["ts"]["error_class"] = json!("TypeError");
    assert_eq!(
        check(entry).map(|(class, _)| class),
        Some("TypeError".to_owned())
    );
}

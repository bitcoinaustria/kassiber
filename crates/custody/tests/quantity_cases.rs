//! Independent CPython outputs, including behavior pins and adversarial input.
use kassiber_custody::quantity::{allocation, balances, project};
use serde_json::Value;
fn vectors() -> &'static Value {
    static V: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_quantity_primitives.json"
        ))
        .unwrap()
    })
}
#[test]
fn quantity_arbitration_vectors() {
    for case in vectors()["project"].as_array().unwrap() {
        let actual = project(
            case["observations"].as_array().unwrap(),
            case["claims"].as_array().unwrap(),
        );
        if let Some(error) = case["error"].as_str() {
            assert_eq!(actual.unwrap_err(), error, "{}", case["case"]);
        } else {
            assert_eq!(actual.unwrap(), case["expected"], "{}", case["case"]);
        }
    }
}
#[test]
fn quantity_fifo_vectors() {
    for case in vectors()["allocation"].as_array().unwrap() {
        let actual = allocation::allocate_msat_fifo(
            &case["sources"],
            &case["sinks"],
            case.get("amount_msat"),
        );
        if let Some(error) = case["error"].as_str() {
            let actual = actual.unwrap_err();
            assert_eq!(actual.code, error, "{}", case["case"]);
            assert_eq!(actual.message, case["message"]);
            assert_eq!(actual.details, case["details"]);
        } else {
            assert_eq!(actual.unwrap(), case["expected"], "{}", case["case"]);
        }
    }
}
#[test]
fn quantity_balance_vectors() {
    for case in vectors()["balances"].as_array().unwrap() {
        let actual = balances::compare_wallet_balances(
            &case["projection"],
            case["current"].as_array().unwrap(),
            &case["known"],
        );
        if let Some(error) = case["error"].as_str() {
            assert_eq!(actual.unwrap_err(), error);
        } else {
            assert_eq!(actual.unwrap(), case["expected"]);
        }
    }
}

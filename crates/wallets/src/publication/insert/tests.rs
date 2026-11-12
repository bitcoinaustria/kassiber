use super::*;
use crate::publication::testbook::{load, snapshot, text, Clock};
use std::cell::RefCell;

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => panic!("expected list"),
    }
}

fn error_value(error: &AppError) -> Value {
    dict([
        ("code", s(error.code.clone())),
        ("message", s(error.message.clone())),
        ("hint", error.hint.clone().map_or(Value::Null, Value::from)),
        ("details", error.details.clone().unwrap_or(Value::Null)),
    ])
}

#[test]
fn insertion_matches_python_oracle_cases() {
    let vectors =
        kassiber_pytext::loads(include_str!("../../../tests/vectors/python_insert.json")).unwrap();
    let cases = list(get(&vectors, "cases").unwrap());
    assert!(cases.len() >= 40);
    for case in cases {
        let name = py_str(get(case, "name").unwrap());
        let wallet = py_str(get(case, "wallet").unwrap());
        let clock = RefCell::new(Clock::new("id"));
        let mut conn: Option<Connection> = None;
        for (index, step) in list(get(case, "steps").unwrap()).iter().enumerate() {
            let conn = conn.get_or_insert_with(|| load(get(step, "before").unwrap()));
            let records = list(get(step, "records").unwrap());
            let scope = Scope {
                workspace_id: "w",
                profile_id: "p",
                profile_fiat_currency: "EUR",
                wallet_id: &wallet,
                wallet_label: if wallet == "a" { "Wällét" } else { "foreign" },
            };
            let savepoint = conn.unchecked_transaction().unwrap();
            let outcome = insert_records(
                &savepoint,
                &scope,
                records,
                "backend:fixture",
                &mut || clock.borrow_mut().id(),
                &mut || clock.borrow_mut().now(),
            );
            let actual = match &outcome {
                Ok(result) => dict([("result", result.clone())]),
                Err(error) => dict([("error", error_value(error))]),
            };
            if outcome.is_ok() {
                savepoint.commit().unwrap();
            } else {
                savepoint.rollback().unwrap();
            }
            assert_eq!(
                text(&actual),
                text(get(step, "outcome").unwrap()),
                "{name} step {index} outcome"
            );
            assert_eq!(
                text(&snapshot(conn)),
                text(get(step, "after").unwrap()),
                "{name} step {index} rows"
            );
        }
    }
}

#[test]
fn observer_quantity_corrections_bound_the_recalculated_total() {
    let existing = |rate: &str| dict([("fiat_rate_exact", s(rate)), ("fiat_rate", Value::Null)]);
    let amount: Decimal = "100".parse().unwrap();
    let mut updates = Updates(Vec::new());
    recomputed_value(&existing("2.5"), &amount, &mut updates).unwrap();
    assert_eq!(updates.get("fiat_value_exact"), Some(&Value::from("250.0")));
    for (rate, message) in [
        ("1e18", "fiat_value is outside the supported decimal range"),
        ("NaN", "fiat_rate must be a finite decimal number"),
        ("1e19", "fiat_rate is outside the supported decimal range"),
    ] {
        let error =
            recomputed_value(&existing(rate), &amount, &mut Updates(Vec::new())).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("validation", message),
            "{rate}"
        );
    }
}

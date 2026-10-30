use super::*;
use crate::publication::policy::tests::error_value;
use crate::publication::testbook::{load, snapshot, text, Clock};
use std::cell::RefCell;

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => panic!("expected list"),
    }
}

#[test]
fn report_impacts_match_python_oracle_cases() {
    let vectors =
        kassiber_pytext::loads(include_str!("../../../tests/vectors/python_reports.json")).unwrap();
    let cases = list(get(&vectors, "cases").unwrap());
    assert!(cases.len() >= 13);
    for case in cases {
        let name = py_str(get(case, "name").unwrap());
        let clock = RefCell::new(Clock::new("id"));
        for (index, step) in list(get(case, "steps").unwrap()).iter().enumerate() {
            // Reopen each round: the oracle may have observed new chain
            // evidence between impact publications. Preserve the report id
            // sequence and replay the persisted inbox from the full book.
            let conn = load(get(step, "before").unwrap());
            let retracted: Vec<String> = list(get(step, "retracted").unwrap())
                .iter()
                .map(py_str)
                .collect();
            let savepoint = conn.unchecked_transaction().unwrap();
            let outcome = record_wallet_changes(
                &savepoint,
                "p",
                "a",
                &retracted,
                &mut || clock.borrow_mut().id(),
                &mut || clock.borrow_mut().now(),
            );
            let expected = get(step, "outcome").unwrap();
            let actual = match &outcome {
                Ok(count) => dict([("result", Value::Int((*count).into()))]),
                Err(error) => error_value(error, expected),
            };
            if outcome.is_ok() {
                savepoint.commit().unwrap();
            } else {
                savepoint.rollback().unwrap();
            }
            assert_eq!(text(&actual), text(expected), "{name} step {index} outcome");
            assert_eq!(
                text(&snapshot(&conn)),
                text(get(step, "after").unwrap()),
                "{name} step {index} rows"
            );
        }
    }
}

use super::*;
use crate::publication::testbook::{load, snapshot, text, Clock};
use kassiber_envelope::{dict, s};
use std::cell::RefCell;

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => panic!("expected list"),
    }
}

/// Python raises a bare exception class where Rust returns `internal_error`.
pub(crate) fn error_value(error: &AppError, expected: &Value) -> Value {
    let python = get(expected, "error").map(|e| py_str(get(e, "code").unwrap()));
    let code = match python {
        Some(code) if code.ends_with("Error") && error.code == "internal_error" => code,
        _ => error.code.clone(),
    };
    dict([(
        "error",
        dict([
            ("code", s(code)),
            ("message", s(error.message.clone())),
            ("hint", error.hint.clone().map_or(Value::Null, Value::from)),
            ("details", error.details.clone().unwrap_or(Value::Null)),
        ]),
    )])
}

#[test]
fn policy_coverage_matches_python_oracle_cases() {
    let vectors =
        kassiber_pytext::loads(include_str!("../../../tests/vectors/python_policy.json")).unwrap();
    let cases = list(get(&vectors, "cases").unwrap());
    assert!(cases.len() >= 8);
    for case in cases {
        let name = py_str(get(case, "name").unwrap());
        let clock = RefCell::new(Clock::new("id"));
        let mut conn: Option<Connection> = None;
        for (index, step) in list(get(case, "steps").unwrap()).iter().enumerate() {
            let conn = conn.get_or_insert_with(|| load(get(step, "before").unwrap()));
            let ident = get(step, "identity").unwrap();
            let text_of = |key: &str| py_str(get(ident, key).unwrap());
            let branch_keys: Vec<String> = list(get(ident, "branch_keys").unwrap())
                .iter()
                .map(py_str)
                .collect();
            let (wallet, key, kind, chain, network) = (
                text_of("source_wallet_id"),
                text_of("source_key"),
                text_of("observer_kind"),
                text_of("chain"),
                text_of("network"),
            );
            let identity = Identity {
                source_wallet_id: &wallet,
                source_key: &key,
                observer_kind: &kind,
                chain: &chain,
                network: &network,
                branch_keys: &branch_keys,
            };
            let coverage: Vec<Point> = list(get(step, "coverage").unwrap())
                .iter()
                .map(|p| {
                    let p = list(p);
                    let int = |v: &Value| match v {
                        Value::Int(n) => Some(i64::try_from(n.clone()).unwrap()),
                        _ => None,
                    };
                    Point {
                        branch_key: py_str(&p[0]),
                        scanned_to: int(&p[1]).unwrap(),
                        highest_used: int(&p[2]),
                    }
                })
                .collect();
            let savepoint = conn.unchecked_transaction().unwrap();
            let outcome = record_coverage(
                &savepoint,
                &identity,
                &coverage,
                &mut || clock.borrow_mut().id(),
                &mut || clock.borrow_mut().now(),
            );
            let expected = get(step, "outcome").unwrap();
            let actual = match &outcome {
                Ok(epoch) => dict([("result", s(epoch.clone()))]),
                Err(error) => error_value(error, expected),
            };
            if outcome.is_ok() {
                savepoint.commit().unwrap();
            } else {
                savepoint.rollback().unwrap();
            }
            assert_eq!(text(&actual), text(expected), "{name} step {index} outcome");
            assert_eq!(
                text(&snapshot(conn)),
                text(get(step, "after").unwrap()),
                "{name} step {index} rows"
            );
        }
    }
}

use super::*;
use crate::publication::testbook::{load, snapshot, text, Clock};

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => panic!("expected list"),
    }
}

#[test]
fn inventory_matches_python_oracle_cases() {
    let vectors =
        kassiber_pytext::loads(include_str!("../../../tests/vectors/python_inventory.json"))
            .unwrap();
    let cases = list(get(&vectors, "cases").unwrap());
    assert!(cases.len() >= 13);
    for case in cases {
        let name = py_str(get(case, "name").unwrap());
        let mut clock = Clock::new("id");
        let mut conn: Option<Connection> = None;
        for (index, step) in list(get(case, "steps").unwrap()).iter().enumerate() {
            let conn = conn.get_or_insert_with(|| load(get(step, "before").unwrap()));
            let config: Value = conn
                .query_row("SELECT config_json FROM wallets WHERE id='a'", [], |row| {
                    row.get::<_, String>(0)
                })
                .map(Value::from)
                .unwrap();
            let backend = list(get(step, "backend").unwrap());
            let (backend_name, backend_kind) = (py_str(&backend[0]), py_str(&backend[1]));
            let source = Source {
                workspace_id: "w",
                profile_id: "p",
                wallet_id: "a",
                wallet_config_json: &config,
                backend_name: &backend_name,
                backend_kind: &backend_kind,
                chain: "bitcoin",
                network: "main",
            };
            let savepoint = conn.unchecked_transaction().unwrap();
            let outcome = update(
                &savepoint,
                &source,
                list(get(step, "outputs").unwrap()),
                &clock.now(),
            );
            let actual = match &outcome {
                Ok(result) => dict([("result", result.clone())]),
                Err(error) => dict([(
                    "error",
                    dict([
                        ("code", s(error.code.clone())),
                        ("message", s(error.message.clone())),
                        ("hint", error.hint.clone().map_or(Value::Null, Value::from)),
                        ("details", error.details.clone().unwrap_or(Value::Null)),
                    ]),
                )]),
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

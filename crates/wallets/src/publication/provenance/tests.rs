use super::*;
use crate::publication::testbook::{load, snapshot, text, Clock};

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => panic!("expected list"),
    }
}

fn int(value: &Value) -> u32 {
    match value {
        Value::Int(n) => u32::try_from(n.clone()).unwrap(),
        _ => panic!("expected int"),
    }
}

fn serialized(inputs: &AccountingInputs) -> Value {
    let states = inputs
        .states
        .iter()
        .map(|((a, b, c), confirmed)| {
            Value::List(vec![
                Value::List(vec![s(a.clone()), s(b.clone()), s(c.clone())]),
                Value::Bool(*confirmed),
            ])
        })
        .collect();
    let mut bindings: Vec<Value> = inputs
        .bindings
        .iter()
        .map(|(id, values)| Value::List(vec![id.clone(), Value::List(values.clone())]))
        .collect();
    bindings.sort_by_key(text);
    dict([
        ("states", Value::List(states)),
        ("bindings", Value::List(bindings)),
    ])
}

#[test]
fn provenance_and_accounting_inputs_match_python_oracle() {
    let vectors = kassiber_pytext::loads(include_str!(
        "../../../tests/vectors/python_provenance.json"
    ))
    .unwrap();
    let cases = list(get(&vectors, "cases").unwrap());
    assert!(cases.len() >= 14);
    for case in cases {
        let name = py_str(get(case, "name").unwrap());
        let conn = load(get(case, "before").unwrap());
        let clock = list(get(case, "clock").unwrap());
        let mut clock = Clock::resume("id", int(&clock[0]), int(&clock[1]));
        let entries = list(get(case, "entries").unwrap());
        let resolved = list(get(case, "resolved").unwrap());
        let before = accounting_inputs(&conn, "p", entries).unwrap();
        assert_eq!(
            text(&serialized(&before)),
            text(get(case, "inputs_before").unwrap()),
            "{name} inputs before"
        );
        let savepoint = conn.unchecked_transaction().unwrap();
        let chain = if name == "non-native-chain" {
            "lightning"
        } else {
            "bitcoin"
        };
        let outcome = persist(
            &savepoint,
            &Target {
                workspace_id: "w",
                profile_id: "p",
                wallet_id: "a",
                application_revision: "revision",
                chain,
                network: "main",
            },
            entries,
            resolved,
            &mut || clock.now(),
        );
        let actual = match &outcome {
            Ok(count) => dict([("result", Value::Int((*count).into()))]),
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
            text(get(case, "outcome").unwrap()),
            "{name} outcome"
        );
        assert_eq!(
            text(&snapshot(&conn)),
            text(get(case, "after").unwrap()),
            "{name} rows"
        );
        let after = accounting_inputs(&conn, "p", entries).unwrap();
        assert_eq!(
            text(&serialized(&after)),
            text(get(case, "inputs_after").unwrap()),
            "{name} inputs after"
        );
        assert_eq!(
            before.same(&after),
            text(get(case, "inputs_before").unwrap()) == text(get(case, "inputs_after").unwrap()),
            "{name} comparison"
        );
        assert_eq!(
            journal_current(&conn, "p").unwrap(),
            get(case, "journal_current") == Some(&Value::Bool(true)),
            "{name} journal"
        );
    }
}

#[test]
fn entries_project_final_records_and_skip_blank_keys() {
    let records = [
        dict([
            ("txid", s(" AB ")),
            ("asset", s("l-btc")),
            ("direction", s("Inbound")),
        ]),
        dict([
            ("txid", s("")),
            ("external_id", s("")),
            ("asset", s("BTC")),
            ("direction", s("in")),
        ]),
    ];
    let observers = [
        ObserverRecords {
            id: "b",
            kind: "bdk",
            records: &records,
        },
        ObserverRecords {
            id: "a",
            kind: "x",
            records: &records[..1],
        },
    ];
    let entries = entries_for_facts(
        &observers,
        Some(&[dict([
            ("external_id", s("ab")),
            ("asset", s("LBTC")),
            ("direction", s("outbound")),
        ])]),
    );
    assert_eq!(
        text(&Value::List(entries)),
        r#"[{"asset": "LBTC", "direction": "outbound", "external_id": "ab", "observer_ids": ["a", "b"], "observer_kinds": ["bdk", "x"]}]"#
    );
}

use super::*;
use kassiber_pyjson::Value;

fn vectors() -> Value {
    kassiber_pytext::loads(include_str!("../../../tests/vectors/python_physical.json")).unwrap()
}

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => panic!("expected list"),
    }
}

fn text(value: &Value) -> String {
    value.as_str_lossy()
}

trait Lossy {
    fn as_str_lossy(&self) -> String;
}
impl Lossy for Value {
    fn as_str_lossy(&self) -> String {
        py_str(self)
    }
}

fn expected(outcome: &Value) -> Option<&Value> {
    get(outcome, "result")
}

#[test]
fn physical_scope_and_confirmation_match_python_vectors() {
    let vectors = vectors();
    let cases = list(get(&vectors, "cases").unwrap());
    assert!(cases.len() >= 900);
    let mut rows = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let row = get(case, "row").unwrap().clone();
        let scope = transfer_scope(&row).map(|(a, b, c, d)| vec![a, b, c, d]);
        let want = expected(get(case, "transfer_scope").unwrap()).unwrap();
        assert_eq!(
            scope,
            match want {
                Value::Null => None,
                other => Some(list(other).iter().map(text).collect()),
            },
            "transfer scope {index}"
        );
        let (domain, valid) = network_domain_evidence(&row);
        let want = list(expected(get(case, "domain").unwrap()).unwrap());
        assert_eq!(
            (domain, valid),
            (
                match &want[0] {
                    Value::Null => None,
                    other => Some(text(other)),
                },
                want[1] == Value::Bool(true)
            ),
            "domain {index}"
        );
        let conflict = get(case, "conflict").unwrap();
        match (conflict_scope(&row), expected(conflict)) {
            (Ok(actual), Some(want)) => assert_eq!(
                actual.map(|(a, b, c)| vec![a, b, c]),
                match want {
                    Value::Null => None,
                    other => Some(list(other).iter().map(text).collect()),
                },
                "conflict {index}"
            ),
            (Err(_), None) => {}
            (actual, _) => panic!("conflict {index}: {actual:?} vs {conflict:?}"),
        }
        let observed = get(case, "observation").unwrap();
        match (confirmation_observation(&row), expected(observed)) {
            (Ok(None), Some(Value::Null)) => {}
            (Ok(Some(((a, b, c), at, confirmed))), Some(want)) => {
                let want = list(want);
                assert_eq!(
                    vec![a, b, c],
                    list(&want[0]).iter().map(text).collect::<Vec<_>>(),
                    "observation {index}"
                );
                assert_eq!(Value::Int(at.into()), want[1], "observation {index}");
                assert_eq!(Value::Bool(confirmed), want[2], "observation {index}");
            }
            (Err(_), None) => {}
            (actual, _) => panic!("observation {index}: {actual:?} vs {observed:?}"),
        }
        rows.push(row);
    }
    for (index, group) in list(get(&vectors, "groups").unwrap()).iter().enumerate() {
        let members: Vec<Value> = list(get(group, "rows").unwrap())
            .iter()
            .map(|i| {
                rows[usize::try_from(match i {
                    Value::Int(n) => n.clone(),
                    _ => unreachable!(),
                })
                .unwrap()]
                .clone()
            })
            .collect();
        let states = get(group, "states").unwrap();
        match (confirmation_states(&members), expected(states)) {
            (Ok(actual), Some(want)) => {
                let actual: Vec<(Vec<String>, bool)> = actual
                    .into_iter()
                    .map(|((a, b, c), v)| (vec![a, b, c], v))
                    .collect();
                let want: Vec<(Vec<String>, bool)> = list(want)
                    .iter()
                    .map(|pair| {
                        let pair = list(pair);
                        (
                            list(&pair[0]).iter().map(text).collect(),
                            pair[1] == Value::Bool(true),
                        )
                    })
                    .collect();
                assert_eq!(actual, want, "states {index}");
            }
            (Err(_), None) => {}
            (actual, _) => panic!("states {index}: {actual:?} vs {states:?}"),
        }
    }
}

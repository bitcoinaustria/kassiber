use kassiber_journals::{native, scalar};
use scalar::Value;

fn list(value: &Value) -> &[Value] {
    let Value::List(items) = value else {
        panic!("expected list")
    };
    items
}

#[test]
fn native_builder_matches_python_states_refusals_and_uuid_tapes() {
    let mut failures = vec![];
    let mut count = 0;
    for source in [
        include_str!("data/render.json"),
        include_str!("data/render-legacy-floats.json"),
        include_str!("data/render-response-refusals.json"),
        include_str!("data/render-scalar-coercions.json"),
        include_str!("data/render-branches.json"),
        include_str!("data/render-native-builder.json"),
        include_str!("data/render-native-graph.json"),
    ] {
        let cases = scalar::decode(source).unwrap();
        for case in list(&cases) {
            count += 1;
            let field = native::field;
            let ids = list(field(case, "entry_ids"));
            let mut consumed = 0;
            let result = native::build(
                field(case, "profile"),
                field(case, "custody"),
                field(case, "response"),
                || {
                    let id = ids
                        .get(consumed)
                        .map(scalar::text)
                        .and_then(|s| s.as_str().map(str::to_owned))
                        .unwrap_or_default();
                    consumed += 1;
                    id
                },
            );
            let name = scalar::repr(field(case, "name"));
            let expected = field(case, "expected");
            let actual = match result {
                Ok(state) => scalar::dict([(
                    "state",
                    Value::from(scalar::encode(&state.snapshot()).unwrap()),
                )]),
                Err(error) => scalar::dict([("error", error.0)]),
            };
            if scalar::encode(&actual).unwrap() != scalar::encode(expected).unwrap() {
                failures.push(format!(
                    "{name}: actual {}, expected {}",
                    scalar::encode(&actual).unwrap(),
                    scalar::encode(expected).unwrap()
                ));
            }
            if consumed != ids.len() {
                failures.push(format!("{name}: UUID tape {consumed}/{}", ids.len()));
            }
        }
    }
    assert_eq!(count, 1438);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#![cfg(feature = "test-harness")]
use kassiber_envelope::{dict, get, py_str, s};
use kassiber_pyjson::{Options, Value};
use kassiber_transfers::review_operations::{canonical, digest, normalize};

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap()
}

#[test]
fn review_operations_match_python_normalization_digests_bounds_and_refusals() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("review_operations.json")).unwrap()
    else {
        panic!("operation fixtures")
    };
    let mut successes = 0;
    let mut failures = 0;
    for case in cases {
        let operations = if let Some(recipe) = get(&case, "recipe") {
            let Value::Int(count) = field(recipe, "count") else {
                panic!("recipe length")
            };
            let notes =
                py_str(field(recipe, "character")).repeat(usize::try_from(count.clone()).unwrap());
            Value::List(vec![dict([
                ("type", s("custody_component")),
                (
                    "request",
                    dict([
                        ("action", s("create")),
                        ("spec", dict([("notes", s(notes))])),
                    ]),
                ),
            ])])
        } else {
            field(&case, "operations").clone()
        };
        let before = kassiber_pyjson::dumps(&operations, &Options::default()).unwrap();
        let actual = normalize(&operations);
        assert_eq!(
            kassiber_pyjson::dumps(&operations, &Options::default()).unwrap(),
            before,
            "normalization does not edit its input"
        );
        if get(&case, "python_type_error").is_some() {
            // Python raises a bare TypeError for an unhashable type
            // (tests/rust_port/repro_review_operation_unhashable_type.py);
            // Rust keeps the validation refusal.
            let error = actual.unwrap_err();
            assert_eq!(
                (error.code.as_str(), error.message.as_str()),
                ("validation", "Review operation type is unsupported")
            );
            failures += 1;
        } else if let Some(expected) = get(&case, "error") {
            let error = actual.unwrap_err();
            let actual = dict([
                ("code", s(error.code)),
                ("message", s(error.message)),
                ("hint", error.hint.map(s).unwrap_or(Value::Null)),
                ("details", error.details.unwrap_or(Value::Null)),
            ]);
            assert_eq!(
                canonical(&actual).unwrap(),
                canonical(expected).unwrap(),
                "{operations:?}"
            );
            failures += 1;
        } else {
            let normalized = actual.unwrap_or_else(|error| panic!("{operations:?}: {error:?}"));
            let encoded = canonical(&normalized).unwrap();
            if let Some(expected) = get(&case, "canonical") {
                assert_eq!(encoded, py_str(expected), "{operations:?}");
            } else {
                assert_eq!(Value::from(encoded.len()), *field(&case, "encoded_bytes"));
            }
            assert_eq!(digest(&normalized).unwrap(), py_str(field(&case, "digest")));
            assert_eq!(
                canonical(&normalize(&normalized).unwrap()).unwrap(),
                encoded,
                "normalization is idempotent"
            );
            successes += 1;
        }
    }
    assert!(successes > 30 && failures > 100);
}

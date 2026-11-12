#![cfg(feature = "test-harness")]
use kassiber_envelope::{dict, get, py_str, s, set, AppError};
use kassiber_pyjson::{Options, Value};
use kassiber_transfers::{review_artifact, review_operations, Scope};

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap()
}
fn failure(error: AppError) -> Value {
    dict([
        ("code", s(error.code)),
        ("message", s(error.message)),
        ("hint", error.hint.map(s).unwrap_or(Value::Null)),
        ("details", error.details.unwrap_or(Value::Null)),
    ])
}
fn fixture() -> Value {
    kassiber_pytext::loads(include_str!("review_artifacts.json")).unwrap()
}
fn base(fixture: &Value) -> Value {
    let Value::List(cases) = field(fixture, "assembly") else {
        panic!("assembly cases")
    };
    field(&cases[0], "payload").clone()
}
fn recipe(fixture: &Value, recipe: &Value) -> Value {
    let mut payload = base(fixture);
    let Value::Int(count) = field(recipe, "count") else {
        panic!("recipe length")
    };
    let count = usize::try_from(count.clone()).unwrap();
    let tax = field(field(&payload, "before"), "tax").clone();
    set(
        &mut payload,
        "before",
        dict([
            (
                "padding",
                s(py_str(field(recipe, "character")).repeat(count)),
            ),
            ("tax", tax),
        ]),
    );
    payload
}
fn compare(actual: &Value, expected: &Value, case: &Value) {
    assert_eq!(
        review_operations::canonical(actual).unwrap(),
        review_operations::canonical(expected).unwrap(),
        "{}",
        py_str(field(case, "name"))
    );
}

#[test]
fn review_artifact_content_matches_python_bounds_digests_and_order() {
    let fixture = fixture();
    let Value::List(cases) = field(&fixture, "assembly") else {
        panic!("assembly cases")
    };
    for case in cases {
        let payload = get(case, "recipe")
            .map(|r| recipe(&fixture, r))
            .unwrap_or_else(|| field(case, "payload").clone());
        let normalized = review_operations::normalize(field(&payload, "operations")).unwrap();
        let actual = review_artifact::assemble(
            &Scope {
                workspace_id: "w",
                profile_id: "p",
            },
            field(&payload, "base_input_version").clone(),
            normalized,
            field(&payload, "before").clone(),
            field(&payload, "after").clone(),
        );
        if let Some(expected) = get(case, "error") {
            compare(&failure(actual.unwrap_err()), expected, case);
        } else if let Some(expected) = get(case, "result") {
            compare(&actual.unwrap(), expected, case);
        } else {
            let artifact = actual.unwrap();
            assert_eq!(
                &Value::from(review_operations::canonical(&artifact).unwrap().len()),
                field(case, "encoded_bytes")
            );
            assert_eq!(field(&artifact, "digest"), field(case, "digest"));
        }
    }
    assert_eq!(cases.len(), 11);
}

#[test]
fn review_artifact_admission_matches_python_refusal_precedence_and_canonicality() {
    let fixture = fixture();
    let Value::List(cases) = field(&fixture, "admission") else {
        panic!("admission cases")
    };
    let mut accepted = 0;
    let mut rejected = 0;
    for case in cases {
        let artifact = if let Some(r) = get(case, "recipe") {
            let mut payload = recipe(&fixture, r);
            let digest = review_operations::digest(&payload).unwrap();
            set(&mut payload, "digest", s(digest));
            payload
        } else {
            field(case, "artifact").clone()
        };
        let before = kassiber_pyjson::dumps(&artifact, &Options::default()).unwrap();
        let actual = review_artifact::admit(
            &artifact,
            &Scope {
                workspace_id: "w",
                profile_id: "p",
            },
            field(case, "key"),
        );
        if let Some(expected) = get(case, "error") {
            compare(&failure(actual.unwrap_err()), expected, case);
            rejected += 1;
        } else {
            compare(&actual.unwrap(), field(case, "result"), case);
            accepted += 1;
        }
        assert_eq!(
            kassiber_pyjson::dumps(&artifact, &Options::default()).unwrap(),
            before
        );
    }
    assert!(accepted > 10 && rejected > 60);
    assert_eq!(cases.len(), 116);
}

#[test]
fn review_artifact_equality_retains_python_mapping_and_exact_numeric_semantics() {
    let parse = |text| kassiber_pytext::loads(text).unwrap();
    assert!(review_artifact::equal(
        &parse(r#"{"a":true,"b":[1.0]}"#),
        &parse(r#"{"b":[1],"a":1}"#)
    ));
    assert!(!review_artifact::equal(
        &parse("9007199254740993"),
        &parse("9007199254740992.0")
    ));
    assert!(!review_artifact::equal(&parse("[1,2]"), &parse("[2,1]")));
    assert!(!review_artifact::equal(
        &parse(r#"{"a":1}"#),
        &parse(r#"{"a":"1"}"#)
    ));
    assert!(review_artifact::equal(
        &Value::Dict(vec![(kassiber_pyjson::Key::Int(1.into()), 2.into())]),
        &Value::Dict(vec![(kassiber_pyjson::Key::Bool(true), 2.into())]),
    ));
}

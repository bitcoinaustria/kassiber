use kassiber_diagnostics::{is_sensitive_key, sanitize_details, sanitize_text};
use kassiber_pyjson::{dumps, Options};
use serde_json::Value;
fn vectors() -> Value {
    serde_json::from_str(include_str!("redaction.json")).unwrap()
}
#[test]
fn all_text_rules_match_python_bytes() {
    for case in vectors()["text"].as_array().unwrap() {
        assert_eq!(
            sanitize_text(case["input"].as_str().unwrap()),
            case["expected"].as_str().unwrap(),
            "{}",
            case["name"]
        );
    }
}
#[test]
fn every_sensitive_key_matches_python() {
    for case in vectors()["sensitive_keys"].as_array().unwrap() {
        assert_eq!(
            is_sensitive_key(case["key"].as_str().unwrap()),
            case["expected"].as_bool().unwrap(),
            "{}",
            case["key"]
        );
    }
}
#[test]
fn recursive_details_shapes_match_python() {
    for case in vectors()["details"].as_array().unwrap() {
        let input = kassiber_pytext::loads(case["input_bytes"].as_str().unwrap()).unwrap();
        let result = dumps(
            &sanitize_details(&input),
            &Options {
                indent: Some(2.into()),
                ..Options::default()
            },
        )
        .unwrap()
            + "\n";
        assert_eq!(result, case["expected_bytes"].as_str().unwrap());
    }
}

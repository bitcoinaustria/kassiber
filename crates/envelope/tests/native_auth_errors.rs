//! Checks `tests/data/native_auth_errors.json`, which
//! `tests/test_native_auth_error_redaction.py` checks against Python: the
//! Touch ID helper's fixed errors pass the traceback sanitizer unchanged and
//! anything else after `auth:` is still redacted, identically in both runtimes.

use kassiber_envelope::redaction::{
    sanitize_traceback_text, NATIVE_AUTH_ERROR_MESSAGES, NATIVE_AUTH_ERROR_PREFIX,
    NATIVE_AUTH_ERROR_STATUS_TEMPLATES,
};
use serde_json::Value;

fn vectors() -> Value {
    serde_json::from_str(include_str!("data/native_auth_errors.json")).unwrap()
}

fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap())
        .collect()
}

#[test]
fn rust_lists_match_the_shared_vectors() {
    let vectors = vectors();
    assert_eq!(
        NATIVE_AUTH_ERROR_PREFIX,
        vectors["prefix"].as_str().unwrap()
    );
    let mut messages = NATIVE_AUTH_ERROR_MESSAGES.to_vec();
    messages.sort_unstable();
    assert_eq!(messages, strings(&vectors["messages"]));
    let templates: Vec<(&str, &str)> = vectors["status_templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            let pair = strings(pair);
            (pair[0], pair[1])
        })
        .collect();
    assert_eq!(NATIVE_AUTH_ERROR_STATUS_TEMPLATES, templates.as_slice());
}

#[test]
fn known_helper_errors_pass_unchanged() {
    for text in strings(&vectors()["allowed"]) {
        assert_eq!(sanitize_traceback_text(text), text);
    }
}

#[test]
fn anything_else_after_auth_is_still_redacted() {
    for case in vectors()["redacted"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        assert_eq!(
            sanitize_traceback_text(input),
            case["expected"].as_str().unwrap(),
            "{input:?}"
        );
    }
}

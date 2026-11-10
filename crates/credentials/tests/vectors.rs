//! The Python scrub's outputs (`tools/export_vectors.py`), reproduced exactly.

use kassiber_credentials::{
    authorization_credentials, backend_credentials, credential_forms, header_credentials,
    request_credentials, url_credentials, withhold_credentials,
};
use serde_json::Value;

fn vectors() -> Value {
    serde_json::from_str(include_str!("data/vectors.json")).unwrap()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_owned())
        .collect()
}

fn pairs(value: &Value) -> Vec<(String, String)> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            (
                pair[0].as_str().unwrap().to_owned(),
                pair[1].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn forms_and_withholding_match_python() {
    let vectors = vectors();
    for case in vectors["forms"].as_array().unwrap() {
        let credentials = strings(&case["credentials"]);
        assert_eq!(
            credential_forms(&credentials).as_slice(),
            strings(&case["forms"]),
            "{case}"
        );
    }
    for case in vectors["withhold"].as_array().unwrap() {
        let credentials = strings(&case["credentials"]);
        let text = case["text"].as_str().unwrap();
        assert_eq!(
            withhold_credentials(text, &credentials),
            case["result"].as_str().unwrap(),
            "{case}"
        );
    }
}

#[test]
fn authorization_values_match_python() {
    for case in vectors()["authorization"].as_array().unwrap() {
        let value = case["value"].as_str().unwrap();
        assert_eq!(
            authorization_credentials(value),
            strings(&case["result"]),
            "{case}"
        );
    }
}

#[test]
fn url_credentials_match_python() {
    for case in vectors()["url"].as_array().unwrap() {
        let url = case["url"].as_str().unwrap();
        let default = case["default_scheme"].as_str();
        assert_eq!(
            url_credentials(url, default),
            strings(&case["result"]),
            "{case}"
        );
    }
}

#[test]
fn header_and_request_credentials_match_python() {
    for case in vectors()["headers"].as_array().unwrap() {
        let headers = pairs(&case["headers"]);
        assert_eq!(
            header_credentials(&headers),
            strings(&case["header_result"]),
            "{case}"
        );
        assert_eq!(
            request_credentials(case["url"].as_str(), &headers, case["proxy_url"].as_str()),
            strings(&case["request_result"]),
            "{case}"
        );
    }
}

#[test]
fn backend_credentials_match_python() {
    for case in vectors()["backend"].as_array().unwrap() {
        let backend = case["backend"].as_object().unwrap();
        let text = |key: &str| match backend.get(key) {
            Some(Value::String(text)) => kassiber_pytext::strip(text).to_owned(),
            Some(Value::Number(number)) if number.is_i64() || number.is_u64() => number.to_string(),
            _ => String::new(),
        };
        assert_eq!(
            backend_credentials(text),
            strings(&case["result"]),
            "{case}"
        );
    }
}

//! How a provider's refusals map onto Kassiber's error codes, and the one
//! reading of an error body every retry decision uses (`kassiber/ai/client.py`).

use kassiber_envelope::{dict, redaction::redact_secret_text, set, AppError};
use kassiber_pyjson::{Options, Value};

use crate::pyval::{entries, field, int, length, prefix, s, string};

/// `provider_error_body_preview(body)`: the first 512 code points, redacted.
pub fn body_preview(body: &str) -> (String, bool) {
    (redact_secret_text(&prefix(body, 512)), length(body) > 512)
}

/// `_http_error_app_error(exc)`: a refused HTTP request.
pub fn http_error(status: u16, body: &str) -> AppError {
    let mut details = dict([("status", Value::from(status))]);
    if !body.is_empty() {
        let (preview, truncated) = body_preview(body);
        set(&mut details, "body", s(preview));
        set(&mut details, "body_truncated", Value::Bool(truncated));
    }
    let (code, message, hint, retryable) = match status {
        401 | 403 => (
            "ai_auth_failed",
            "AI provider rejected the request as unauthorized".to_owned(),
            "Check the provider's API key in Settings → AI providers.",
            false,
        ),
        429 => (
            "ai_rate_limited",
            "AI provider is rate-limiting requests".to_owned(),
            "Wait a few seconds and try again, or pick a different provider.",
            true,
        ),
        500..=599 => (
            "ai_unavailable",
            format!("AI provider returned a server error ({status})"),
            "The provider may be temporarily unavailable. Try again or switch providers.",
            true,
        ),
        _ => (
            "ai_request_invalid",
            format!("AI provider rejected the request ({status})"),
            "Check the model name and request shape.",
            false,
        ),
    };
    AppError::new(code, message)
        .with_details(details)
        .with_hint(hint)
        .retryable(retryable)
}

/// `_network_error_app_error(exc)`.
pub fn network_error(reason: &str) -> AppError {
    AppError::new("ai_unavailable", "AI provider is unreachable")
        .with_details(dict([("reason", s(reason))]))
        .with_hint("Is the provider running? For local Ollama, try `ollama serve`. For remote providers, check connectivity.")
        .retryable(true)
}

/// `_offline_error("Remote AI providers are")`, as `require_online` raises it.
pub fn offline_error() -> AppError {
    AppError::new(
        "network_egress_disabled",
        "Remote AI providers are disabled: Kassiber is in offline mode",
    )
    .with_hint("Turn off offline mode to connect.")
}

pub fn cancelled() -> AppError {
    AppError::new("ai_cancelled", "AI request cancelled")
}

/// A `kassiber_net` failure on a provider route, as Python's client reports
/// the matching urllib failure.
pub fn transport_error(error: kassiber_net::Error) -> AppError {
    use kassiber_net::Error;
    match error {
        Error::RedirectRefused => AppError::new(
            "ai_request_invalid",
            "AI provider attempted an off-origin redirect",
        ),
        Error::Offline | Error::OfflineUnavailable => offline_error(),
        Error::Io(_)
        | Error::Tls
        | Error::InvalidTimeout
        | Error::HttpInvalid
        | Error::ResponseTooLarge
        | Error::RedirectLimit => network_error(&error.to_string()),
        error => error.app_error(),
    }
}

/// `_provider_error(error, invalid_message=...)`: an error object a provider
/// sent inside a 200 stream or body. A numeric `code` becomes `status`,
/// marked `in_stream`.
pub fn provider_error(error: &Value, invalid_message: &str) -> AppError {
    let raw_code = field(error, "code");
    let status = int(raw_code);
    let provider_code = match raw_code {
        Value::Null => String::new(),
        other => kassiber_pytext::lower(&kassiber_envelope::py_str(other)),
    };
    let error_type = kassiber_pytext::lower(&match field(error, "type") {
        value if kassiber_envelope::truthy(value) => kassiber_envelope::py_str(value),
        _ => String::new(),
    });
    let encoded = kassiber_pyjson::dumps(error, &Options::new().sort_keys(true).default_str(true))
        .unwrap_or_default();
    let (preview, truncated) = body_preview(&encoded);
    let mut details = dict([
        (
            "provider_code",
            if provider_code.is_empty() {
                Value::Null
            } else {
                s(&provider_code)
            },
        ),
        ("body", s(preview)),
        ("body_truncated", Value::Bool(truncated)),
        ("in_stream", Value::Bool(true)),
    ]);
    if let Some(status) = status {
        set(&mut details, "status", Value::from(status));
    }
    if status == Some(429) || provider_code.contains("rate") || error_type.contains("rate_limit") {
        return AppError::new("ai_rate_limited", "AI provider is rate-limiting requests")
            .with_details(details)
            .retryable(true);
    }
    if matches!(status, Some(401 | 403))
        || ["api_key", "auth", "forbidden", "permission"]
            .iter()
            .any(|marker| provider_code.contains(marker))
    {
        return AppError::new(
            "ai_auth_failed",
            "AI provider rejected the request as unauthorized",
        )
        .with_details(details);
    }
    if status.is_some_and(|status| (400..500).contains(&status))
        || ["invalid_prompt", "invalid_request", "invalid_request_error"]
            .contains(&provider_code.as_str())
        || error_type == "invalid_request_error"
    {
        return AppError::new("ai_request_invalid", invalid_message).with_details(details);
    }
    AppError::new(
        "ai_unavailable",
        "AI provider failed while generating a response",
    )
    .with_details(details)
    .retryable(true)
}

/// `_error_status(exc, in_stream=...)`: an in-stream code counts only where
/// asked for.
pub fn error_status(error: &AppError, in_stream: bool) -> Option<i64> {
    let details = error.details.as_ref().unwrap_or(&Value::Null);
    if *field(details, "in_stream") == Value::Bool(true) && !in_stream {
        return None;
    }
    int(field(details, "status"))
}

/// `_error_text(exc)`: the redacted body and the message, lowercased.
fn error_text(error: &AppError) -> String {
    let details = error.details.as_ref().unwrap_or(&Value::Null);
    let body = string(field(details, "body")).unwrap_or_default();
    kassiber_pytext::lower(&format!("{body} {}", error.message))
}

/// `_ErrorParts`: what an error body says about the request, never what it
/// echoes of it.
pub struct ErrorParts {
    pub fields: Vec<String>,
    pub paths: Vec<String>,
    pub messages: Vec<String>,
}

impl ErrorParts {
    /// Whether a structured path, or without one a message, names a word.
    pub fn names(&self, words: &[&str]) -> bool {
        if !self.fields.is_empty() {
            return self
                .paths
                .iter()
                .any(|path| words.iter().any(|word| path.contains(word)));
        }
        self.messages
            .iter()
            .any(|message| words.iter().any(|word| message.contains(word)))
    }
}

const FIELD_KEYS: &[&str] = &["param", "parameter", "field", "property", "loc", "path"];
const MESSAGE_KEYS: &[&str] = &["message", "msg", "detail", "description", "reason"];
const CONTAINER_KEYS: &[&str] = &["error", "errors", "detail", "details"];
const LOCATION_PREFIXES: &[&str] = &["body", "query", "json", "request"];

/// `_field_path(value)`.
fn field_path(value: &Value) -> Vec<String> {
    let segments: Vec<String> = match value {
        Value::List(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::Str(_) | Value::Int(_) | Value::Bool(_) => {
                    Some(kassiber_envelope::py_str(item))
                }
                _ => None,
            })
            .collect(),
        Value::Str(_) => kassiber_envelope::py_str(value)
            .split(['.', '/', '[', ']'])
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => return Vec::new(),
    };
    let mut path: Vec<String> = segments
        .iter()
        .map(|segment| {
            kassiber_pytext::lower(kassiber_pytext::strip(segment).trim_matches(['\'', '"']))
        })
        .filter(|segment| !segment.is_empty())
        .collect();
    while path
        .first()
        .is_some_and(|first| LOCATION_PREFIXES.contains(&first.as_str()))
    {
        path.remove(0);
    }
    path
}

/// `_error_body_parts(exc)`; `None` for a truncated body that does not
/// parse, which relaxes nothing.
pub fn error_body_parts(error: &AppError) -> Option<ErrorParts> {
    let details = error.details.as_ref().unwrap_or(&Value::Null);
    let body = match field(details, "body") {
        Value::Str(text) => crate::pyval::lossy(text),
        _ => String::new(),
    };
    if kassiber_pytext::strip(&body).is_empty() {
        return Some(ErrorParts {
            fields: Vec::new(),
            paths: Vec::new(),
            messages: vec![kassiber_pytext::lower(&error.message)],
        });
    }
    let payload = match kassiber_pytext::loads(&body) {
        Ok(payload) => payload,
        Err(_) if kassiber_envelope::truthy(field(details, "body_truncated")) => return None,
        Err(_) => {
            return Some(ErrorParts {
                fields: Vec::new(),
                paths: Vec::new(),
                messages: vec![kassiber_pytext::lower(&body)],
            })
        }
    };
    let mut parts = ErrorParts {
        fields: Vec::new(),
        paths: Vec::new(),
        messages: Vec::new(),
    };
    fn visit(node: &Value, depth: usize, parts: &mut ErrorParts) {
        if depth > 6 {
            return;
        }
        match node {
            Value::Str(_) => parts
                .messages
                .push(kassiber_pytext::lower(&kassiber_envelope::py_str(node))),
            Value::List(items) => {
                for item in items {
                    visit(item, depth + 1, parts);
                }
            }
            Value::Dict(_) => {
                for (key, value) in entries(node) {
                    let name = kassiber_pytext::lower(&key);
                    if FIELD_KEYS.contains(&name.as_str()) {
                        let path = field_path(value);
                        if let Some(first) = path.first() {
                            if !parts.fields.contains(first) {
                                parts.fields.push(first.clone());
                            }
                            parts.paths.push(path.join("."));
                        }
                    } else if MESSAGE_KEYS.contains(&name.as_str())
                        || CONTAINER_KEYS.contains(&name.as_str())
                    {
                        visit(value, depth + 1, parts);
                    }
                }
            }
            _ => {}
        }
    }
    visit(&payload, 0, &mut parts);
    Some(parts)
}

/// `_STORE_FIELD.search(text)`: `store` as a whole word.
fn store_word(text: &str) -> bool {
    let word = |c: char| c.is_ascii_lowercase() || c == '_';
    text.match_indices("store").any(|(index, _)| {
        !text[..index].chars().next_back().is_some_and(word)
            && !text[index + 5..].chars().next().is_some_and(word)
    })
}

/// `_names_store(exc)`: a 400/422 whose field path, or without one whose
/// message, names `store`.
pub fn names_store(error: &AppError) -> bool {
    if !matches!(error_status(error, false), Some(400 | 422)) {
        return false;
    }
    let Some(parts) = error_body_parts(error) else {
        return false;
    };
    if !parts.fields.is_empty() {
        return parts.fields.iter().any(|field| field == "store");
    }
    parts.messages.iter().any(|message| store_word(message))
}

/// `_store_refused(exc)`: the turn's error when `store: false` is refused.
/// Kassiber never resends without it because of an error.
pub fn store_refused(error: AppError) -> AppError {
    let mut details = match error.details {
        Some(details @ Value::Dict(_)) => details,
        _ => dict::<&str>([]),
    };
    set(&mut details, "reason", s("store_refused"));
    AppError::new(
        "ai_store_refused",
        "This endpoint refused Kassiber's no-retention request (store: false)",
    )
    .with_details(details)
    .with_hint("Run Test connection in Settings → AI providers to see your options, or `kassiber ai providers test <name>`.")
}

const NOT_A_MISSING_ROUTE_WORDS: &[&str] = &["model", "tool", "endpoints found"];
const UNKNOWN_ENDPOINT_PHRASES: &[&str] = &[
    "unexpected endpoint",
    "unknown endpoint",
    "unsupported endpoint",
    "endpoint not found",
    "no such endpoint",
    "route not found",
    "unknown route",
];
const UNSERVED_FORMAT_PHRASES: &[&str] = &[
    "not found",
    "unexpected endpoint",
    "unknown endpoint",
    "unsupported endpoint",
    "not implemented",
];

/// `route_missing(exc)`: the server has no such path. A 404, 405 or 501, or
/// a 400 that names the endpoint as unknown; never one about the model or
/// the tools.
pub fn route_missing(error: &AppError) -> bool {
    let status = error_status(error, false);
    if !matches!(status, Some(400 | 404 | 405 | 501)) {
        return false;
    }
    let text = error_text(error);
    if NOT_A_MISSING_ROUTE_WORDS
        .iter()
        .any(|word| text.contains(word))
    {
        return false;
    }
    status != Some(400)
        || UNKNOWN_ENDPOINT_PHRASES
            .iter()
            .any(|phrase| text.contains(phrase))
}

/// `_format_not_served(exc)`: for the connection test, broader than
/// [`route_missing`].
pub fn format_not_served(error: &AppError) -> bool {
    if route_missing(error) {
        return true;
    }
    let details = error.details.as_ref().unwrap_or(&Value::Null);
    let reason = field(details, "reason");
    if *reason == s("no_stream_events") || *reason == s("no_answer") {
        return true;
    }
    let text = error_text(error);
    matches!(error_status(error, false), Some(400 | 404 | 405 | 501))
        && UNSERVED_FORMAT_PHRASES
            .iter()
            .any(|phrase| text.contains(phrase))
        && !NOT_A_MISSING_ROUTE_WORDS
            .iter()
            .any(|word| text.contains(word))
}

/// `_tools_refused(exc)`: for the connection test, the endpoint or model
/// refused the tool definition.
pub fn tools_refused(error: &AppError) -> bool {
    let details = error.details.as_ref().unwrap_or(&Value::Null);
    let in_stream =
        *field(details, "in_stream") == Value::Bool(true) && error.code == "ai_request_invalid";
    if !matches!(error_status(error, true), Some(400 | 404 | 422 | 500 | 501)) && !in_stream {
        return false;
    }
    error_body_parts(error)
        .is_some_and(|parts| parts.names(&["tool", "function call", "function_call"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(status: Option<i64>, body: Option<&str>, in_stream: bool) -> AppError {
        let mut details = dict::<&str>([]);
        if let Some(status) = status {
            set(&mut details, "status", Value::from(status));
        }
        if let Some(body) = body {
            set(&mut details, "body", s(body));
        }
        if in_stream {
            set(&mut details, "in_stream", Value::Bool(true));
        }
        AppError::new("ai_request_invalid", "AI provider rejected the request")
            .with_details(details)
    }

    #[test]
    fn status_codes_map_like_python() {
        for (status, code, retryable) in [
            (401, "ai_auth_failed", false),
            (403, "ai_auth_failed", false),
            (429, "ai_rate_limited", true),
            (500, "ai_unavailable", true),
            (599, "ai_unavailable", true),
            (400, "ai_request_invalid", false),
            (404, "ai_request_invalid", false),
            (302, "ai_request_invalid", false),
        ] {
            let error = http_error(status, "");
            assert_eq!(
                (error.code.as_str(), error.retryable),
                (code, retryable),
                "{status}"
            );
            assert_eq!(
                *error.details.as_ref().unwrap(),
                dict([("status", Value::from(status))])
            );
        }
        let long = format!(
            "{{\"error\": \"sk-abcdefghijklmnopqrstuvwxyz0123456789 {}\"}}",
            "x".repeat(600)
        );
        let error = http_error(400, &long);
        let details = error.details.unwrap();
        assert_eq!(*field(&details, "body_truncated"), Value::Bool(true));
        let body = string(field(&details, "body")).unwrap();
        assert!(
            body.chars().count() <= 512 && !body.contains("sk-abcdefghijklmnop"),
            "{body}"
        );
    }

    #[test]
    fn missing_routes_and_unserved_formats() {
        assert!(route_missing(&with(
            Some(404),
            Some("{\"detail\":\"Not Found\"}"),
            false
        )));
        assert!(route_missing(&with(Some(405), None, false)));
        assert!(route_missing(&with(Some(501), None, false)));
        assert!(!route_missing(&with(
            Some(404),
            Some("model 'x' not found"),
            false
        )));
        assert!(!route_missing(&with(
            Some(404),
            Some("No endpoints found that support tool use"),
            false
        )));
        assert!(!route_missing(&with(Some(400), Some("not found"), false)));
        assert!(route_missing(&with(
            Some(400),
            Some("Unexpected endpoint or method. (POST /v1/responses)"),
            false
        )));
        assert!(!route_missing(&with(Some(404), None, true)));
        assert!(!route_missing(&with(Some(500), None, false)));
        assert!(format_not_served(&with(
            Some(400),
            Some("Not Found"),
            false
        )));
        assert!(!format_not_served(&with(
            Some(400),
            Some("model not found"),
            false
        )));
        let mut no_answer = with(None, None, false);
        set(
            no_answer.details.as_mut().unwrap(),
            "reason",
            s("no_answer"),
        );
        assert!(format_not_served(&no_answer));
        let mut dropped = with(None, None, false);
        set(
            dropped.details.as_mut().unwrap(),
            "reason",
            s("stream_incomplete"),
        );
        assert!(!format_not_served(&dropped));
    }

    #[test]
    fn store_refusals_read_paths_or_messages_never_echoed_input() {
        assert!(names_store(&with(
            Some(400),
            Some("{\"error\":{\"param\":\"store\"}}"),
            false
        )));
        assert!(names_store(&with(
            Some(422),
            Some("Unrecognized request argument supplied: store"),
            false
        )));
        assert!(!names_store(&with(
            Some(400),
            Some("{\"error\":{\"param\":\"temperature\",\"message\":\"store\"}}"),
            false
        )));
        assert!(!names_store(&with(
            Some(400),
            Some("{\"detail\":[{\"loc\":[\"body\",\"temperature\"],\"input\":{\"store\":false}}]}"),
            false
        )));
        assert!(!names_store(&with(
            Some(400),
            Some("datastore unavailable"),
            false
        )));
        assert!(!names_store(&with(Some(500), Some("store"), false)));
        assert!(!names_store(&with(Some(400), Some("store"), true)));
        let mut truncated = with(Some(400), Some("{\"error\": \"store"), false);
        set(
            truncated.details.as_mut().unwrap(),
            "body_truncated",
            Value::Bool(true),
        );
        assert!(!names_store(&truncated));
        let refused = store_refused(with(Some(400), None, false));
        assert_eq!(refused.code, "ai_store_refused");
        assert_eq!(
            *field(refused.details.as_ref().unwrap(), "reason"),
            s("store_refused")
        );
    }

    #[test]
    fn tool_refusals_and_in_stream_errors() {
        assert!(tools_refused(&with(
            Some(400),
            Some("{\"error\":\"llama3 does not support tools\"}"),
            false
        )));
        assert!(tools_refused(&with(
            Some(500),
            Some("--enable-auto-tool-choice required"),
            false
        )));
        assert!(!tools_refused(&with(Some(401), Some("tool"), false)));
        assert!(tools_refused(&with(
            None,
            Some("No endpoints found that support tool use"),
            true
        )));
        let error = provider_error(
            &kassiber_pytext::loads(
                r#"{"code": 404, "message": "No endpoints found that support tool use"}"#,
            )
            .unwrap(),
            "AI provider rejected the request",
        );
        assert_eq!(error.code, "ai_request_invalid");
        assert!(tools_refused(&error));
        assert!(!route_missing(&error));
        for (raw, code) in [
            (r#"{"code": 429}"#, "ai_rate_limited"),
            (r#"{"code": "rate_limit_exceeded"}"#, "ai_rate_limited"),
            (r#"{"type": "rate_limit_error"}"#, "ai_rate_limited"),
            (r#"{"code": "invalid_api_key"}"#, "ai_auth_failed"),
            (r#"{"code": 403}"#, "ai_auth_failed"),
            (r#"{"type": "invalid_request_error"}"#, "ai_request_invalid"),
            (r#"{"code": "server_error"}"#, "ai_unavailable"),
            (r#"{"code": true}"#, "ai_unavailable"),
        ] {
            assert_eq!(
                provider_error(&kassiber_pytext::loads(raw).unwrap(), "x").code,
                code,
                "{raw}"
            );
        }
    }
}

//! `redact_tool_arguments` and `redact_ai_tool_result`
//! (`kassiber/ai/tools.py:3669`, `:3694`): the secret floor on everything
//! that crosses into consent previews or provider-bound content, and the
//! location scrub on tool results.

use std::sync::OnceLock;

use fancy_regex::Regex;
use kassiber_envelope::redaction::{is_sensitive_key, redact_secret_py};
use kassiber_pyjson::{Key, PyStr, Value};

use crate::pyre;

/// `SENSITIVE_ARGUMENT_KEY_PARTS`: key substrings blanked on top of the
/// shared `is_sensitive_key` list.
pub const SENSITIVE_ARGUMENT_KEY_PARTS: [&str; 17] = [
    "api_key",
    "auth",
    "blinding",
    "config_json",
    "cookie",
    "descriptor",
    "header",
    "mnemonic",
    "password",
    "passphrase",
    "private",
    "recovery",
    "secret",
    "seed",
    "token",
    "wif",
    "xprv",
];

const REDACTED: &str = "<redacted>";
const REDACTED_URL: &str = "<redacted-url>";
const REDACTED_PATH: &str = "<redacted-path>";

/// `_AI_TOOL_RESULT_URL_RE`.
fn url_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| pyre::compile(r#"(?i)\b[a-z][a-z0-9+.-]*://[^\s'"<>]+"#))
}

/// `_AI_TOOL_RESULT_ABSOLUTE_PATH_RE`: at least two segments, so typed UI
/// routes such as `/transactions` are not taken for local paths.
fn path_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        pyre::compile(
            r"(?:[A-Za-z]:[\\/]|[\\/]{2}|~[\\/]|(?<![A-Za-z0-9_])[\\/])[\w@.+ -]+(?:[\\/][\w@.+ -]+)+",
        )
    })
}

/// `str(key)` for a dict key.
fn key_text(key: &Key) -> PyStr {
    match key {
        Key::Str(text) => text.clone(),
        Key::Null => PyStr::from("None"),
        Key::Bool(true) => PyStr::from("True"),
        Key::Bool(false) => PyStr::from("False"),
        Key::Int(n) => PyStr::from(n.to_string()),
        Key::Float(f) => PyStr::from(kassiber_pyjson::float_repr(*f)),
    }
}

/// Whether a model- or builder-supplied key names secret material.
pub fn argument_key_is_sensitive(key: &PyStr) -> bool {
    let text = pyre::lossy(key);
    let lowered = kassiber_pytext::lower(&text);
    is_sensitive_key(&text)
        || SENSITIVE_ARGUMENT_KEY_PARTS
            .iter()
            .any(|part| lowered.contains(part))
}

/// `redact_tool_arguments(value)`: a UI-safe preview of model-supplied
/// arguments. `debug` keys (sanitized tracebacks on error envelopes) are
/// dropped at every level.
pub fn redact_tool_arguments(value: &Value) -> Value {
    match value {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .iter()
                .filter_map(|(key, item)| {
                    let text = key_text(key);
                    if text.is("debug") {
                        return None;
                    }
                    let item = if argument_key_is_sensitive(&text) {
                        Value::Str(PyStr::from(REDACTED))
                    } else {
                        redact_tool_arguments(item)
                    };
                    Some((Key::Str(text), item))
                })
                .collect(),
        ),
        Value::List(items) => Value::List(items.iter().map(redact_tool_arguments).collect()),
        Value::Str(text) => Value::Str(redact_secret_py(text)),
        other => other.clone(),
    }
}

fn scrub_text(text: &str) -> String {
    let Ok(without_urls) = url_pattern()
        .try_replacen(text, 0, REDACTED_URL)
        .map(|t| t.into_owned())
    else {
        // The engine gave up; withhold the text rather than pass it through.
        return REDACTED_PATH.to_owned();
    };
    match path_pattern().find(&without_urls) {
        // Paths may hold punctuation and spaces, so there is no safe end:
        // keep the text before the path and redact the whole remainder.
        Ok(Some(found)) => format!("{}{REDACTED_PATH}", &without_urls[..found.start()]),
        Ok(None) => without_urls,
        Err(_) => REDACTED_PATH.to_owned(),
    }
}

fn scrub_locations(value: Value) -> Value {
    match value {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .into_iter()
                .map(|(key, item)| (key, scrub_locations(item)))
                .collect(),
        ),
        Value::List(items) => Value::List(items.into_iter().map(scrub_locations).collect()),
        Value::Str(text) => Value::Str(pyre::with_surrogates(&text, scrub_text)),
        other => other,
    }
}

/// `redact_ai_tool_result(value)`: the secret floor, then local paths and
/// URLs anywhere inside free text.
pub fn redact_ai_tool_result(value: &Value) -> Value {
    scrub_locations(redact_tool_arguments(value))
}

//! Small Python-value idioms shared by the provider store, the HTTP client
//! and discovery: `dict.get`, `isinstance` checks, `str_or_none` and the
//! sanitising of untrusted JSON they read.

use kassiber_envelope::{get, py_str};
use kassiber_pyjson::{Key, PyStr, Value};

/// `d.get(key)`, `None` when `d` is not a dict or lacks the key.
pub(crate) fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or(&Value::Null)
}

/// `isinstance(v, str) and v` as text.
pub(crate) fn text(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) if text.code_points().next().is_some() => Some(lossy(text)),
        _ => None,
    }
}

/// `isinstance(v, str)` as text, empty included.
pub(crate) fn string(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(lossy(text)),
        _ => None,
    }
}

pub(crate) fn lossy(text: &PyStr) -> String {
    text.code_points()
        .map(|c| char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// `str_or_none(v)`: `str(v).strip()`, or `None` when that is empty.
pub(crate) fn str_or_none(value: &Value) -> Option<String> {
    if *value == Value::Null {
        return None;
    }
    let rendered = match value {
        Value::Str(text) => lossy(text),
        other => py_str(other),
    };
    let stripped = kassiber_pytext::strip(&rendered);
    (!stripped.is_empty()).then(|| stripped.to_owned())
}

/// `isinstance(v, int) and not isinstance(v, bool)`.
pub(crate) fn int(value: &Value) -> Option<i64> {
    match value {
        Value::Int(number) => i64::try_from(number).ok(),
        _ => None,
    }
}

/// The dict's string-keyed entries, in order.
pub(crate) fn entries(value: &Value) -> impl Iterator<Item = (String, &Value)> {
    let pairs: &[(Key, Value)] = match value {
        Value::Dict(pairs) => pairs,
        _ => &[],
    };
    pairs.iter().filter_map(|(key, value)| match key {
        Key::Str(key) => Some((lossy(key), value)),
        _ => None,
    })
}

pub(crate) fn s(text: impl AsRef<str>) -> Value {
    kassiber_envelope::s(text.as_ref())
}

pub(crate) fn opt(text: Option<impl AsRef<str>>) -> Value {
    text.map_or(Value::Null, s)
}

/// `dict(value)` without `key`, keeping order.
pub(crate) fn without(value: &Value, keys: &[&str]) -> Value {
    match value {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .iter()
                .filter(|(key, _)| !keys.iter().any(|k| *key == Key::from(*k)))
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `dict.pop(key, None)`.
pub(crate) fn pop(value: &mut Value, key: &str) -> Value {
    if let Value::Dict(pairs) = value {
        if let Some(index) = pairs.iter().position(|(k, _)| *k == Key::from(key)) {
            return pairs.remove(index).1;
        }
    }
    Value::Null
}

pub(crate) fn has(value: &Value, key: &str) -> bool {
    get(value, key).is_some()
}

/// `s[:limit]` in code points.
pub(crate) fn prefix(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// `len(s)` in code points.
pub(crate) fn length(text: &str) -> usize {
    text.chars().count()
}

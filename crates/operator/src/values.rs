//! Python object semantics the client relies on when it reads replies.

use kassiber_pyjson::{BigInt, Key, PyStr, Value};

pub use kassiber_envelope::{dict, get, s, set, truthy};

/// Python's `==` for JSON-shaped values: `True == 1 == 1.0`, dicts compare
/// as mappings regardless of order, strings by code point.
pub fn py_eq(left: &Value, right: &Value) -> bool {
    if let (Some(a), Some(b)) = (number(left), number(right)) {
        return a == b;
    }
    match (left, right) {
        (Value::Null, Value::Null) => true,
        (Value::Str(a), Value::Str(b)) => a == b,
        (Value::Bytes(a), Value::Bytes(b)) => a == b,
        (Value::List(a), Value::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| py_eq(x, y))
        }
        (Value::Dict(a), Value::Dict(b)) => {
            let a = deduplicated(a);
            let b = deduplicated(b);
            a.len() == b.len()
                && a.iter().all(|(key, value)| {
                    b.iter()
                        .any(|(other, item)| key_eq(key, other) && py_eq(value, item))
                })
        }
        _ => false,
    }
}

/// The value a Python dict holds for each key: the last of duplicates.
fn deduplicated(pairs: &[(Key, Value)]) -> Vec<(&Key, &Value)> {
    let mut kept: Vec<(&Key, &Value)> = Vec::new();
    for (key, value) in pairs {
        if let Some(slot) = kept.iter_mut().find(|(seen, _)| key_eq(seen, key)) {
            slot.1 = value;
        } else {
            kept.push((key, value));
        }
    }
    kept
}

fn number(value: &Value) -> Option<Num> {
    match value {
        Value::Bool(b) => Some(Num::Int(BigInt::from(*b as u8))),
        Value::Int(n) => Some(Num::Int(n.clone())),
        Value::Float(f) => Some(Num::Float(*f)),
        _ => None,
    }
}

fn key_number(key: &Key) -> Option<Num> {
    match key {
        Key::Bool(b) => Some(Num::Int(BigInt::from(*b as u8))),
        Key::Int(n) => Some(Num::Int(n.clone())),
        Key::Float(f) => Some(Num::Float(*f)),
        _ => None,
    }
}

/// A Python number for exact mixed comparison.
enum Num {
    Int(BigInt),
    Float(f64),
}

impl PartialEq for Num {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Num::Int(a), Num::Int(b)) => a == b,
            (Num::Float(a), Num::Float(b)) => a == b,
            (Num::Int(i), Num::Float(f)) | (Num::Float(f), Num::Int(i)) => int_eq_float(i, *f),
        }
    }
}

fn int_eq_float(int: &BigInt, float: f64) -> bool {
    if !float.is_finite() || float.fract() != 0.0 {
        return false;
    }
    // Every integral float is an exact integer; compare its digits.
    format!("{float:.0}") == int.to_string()
}

fn key_eq(left: &Key, right: &Key) -> bool {
    if let (Some(a), Some(b)) = (key_number(left), key_number(right)) {
        return a == b;
    }
    match (left, right) {
        (Key::Null, Key::Null) => true,
        (Key::Str(a), Key::Str(b)) => a == b,
        _ => false,
    }
}

/// `value is True`.
pub fn is_true(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Bool(true)))
}

/// `value is False`.
pub fn is_false(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Bool(false)))
}

/// `isinstance(value, str)`.
pub fn as_str(value: Option<&Value>) -> Option<&PyStr> {
    match value {
        Some(Value::Str(text)) => Some(text),
        _ => None,
    }
}

/// `dict.get(key)` with `None` for a missing key.
pub fn get_or_null(value: &Value, key: &str) -> Value {
    get(value, key).cloned().unwrap_or(Value::Null)
}

/// `dict.pop(key, None)`.
pub fn pop(value: &mut Value, key: &str) -> Value {
    let mut found = Value::Null;
    if let Value::Dict(pairs) = value {
        // Remove every duplicate; Python's dict holds one, the last.
        let mut index = 0;
        while index < pairs.len() {
            if matches!(&pairs[index].0, Key::Str(name) if name.as_str() == Some(key)) {
                found = pairs.remove(index).1;
            } else {
                index += 1;
            }
        }
    }
    found
}

/// `str(value)` keeping lone surrogates, for messages built from replies.
pub fn py_str_value(value: &Value) -> PyStr {
    match value {
        Value::Str(text) => text.clone(),
        other => PyStr::from(kassiber_envelope::py_repr(other)),
    }
}

/// A JSON `str` from Rust text.
pub fn text(value: &str) -> Value {
    s(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Value {
        kassiber_pytext::loads(text).unwrap()
    }

    #[test]
    fn equality_follows_python() {
        for (left, right, equal) in [
            (r#"{"a": 1, "b": true}"#, r#"{"b": 1.0, "a": true}"#, true),
            (r#"{"a": 1}"#, r#"{"a": 1, "a": 2}"#, false),
            (r#"{"a": 2}"#, r#"{"a": 1, "a": 2}"#, true),
            (r#"[1, "x"]"#, r#"[1.0, "x"]"#, true),
            (r#""1""#, "1", false),
            ("null", "false", false),
            ("0", "false", true),
            ("1.5", "1", false),
            ("12345678901234567890", "12345678901234567890.0", false),
            ("18446744073709551616", "18446744073709551616.0", true),
            ("9007199254740993", "9007199254740992.0", false),
            (r#"{"origin": "abc"}"#, r#"{"origin": "abd"}"#, false),
            (r#"{"x": {}}"#, r#"{"x": []}"#, false),
        ] {
            assert_eq!(
                py_eq(&parse(left), &parse(right)),
                equal,
                "{left} == {right}"
            );
        }
    }
}

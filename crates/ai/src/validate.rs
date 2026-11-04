//! `_validate_ai_tool_arguments` (`kassiber/daemon.py:6489`): the catalog
//! schema enforced on model-supplied arguments, check by check in Python's
//! order, with `re.search` pattern semantics.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use fancy_regex::Regex;
use kassiber_envelope::{dict, get, json_ready, AppError};
use kassiber_pyjson::{dumps, BigInt, Key, Options, PyStr, Value};

use crate::catalog::ToolEntry;
use crate::pyre;
use crate::ToolError;

/// The largest argument document accepted, in characters of its compact,
/// ASCII-escaped JSON (`json.dumps(..., separators=(",", ":"))`).
pub const MAX_ARGUMENT_CHARS: usize = 65_536;

/// Deepest schema nesting the validator descends.
pub const MAX_SCHEMA_DEPTH: usize = 16;

fn invalid(message: impl Into<PyStr>) -> AppError {
    let message: PyStr = message.into();
    AppError::new("validation", "").with_python_message(message)
}

fn path_message(prefix: &str, path: &PyStr, suffix: &str) -> PyStr {
    let mut points: Vec<u32> = prefix.chars().map(u32::from).collect();
    points.extend(path.code_points());
    points.extend(suffix.chars().map(u32::from));
    PyStr::from_code_points(points).expect("code points of strings")
}

fn path_error(path: &PyStr, suffix: &str) -> AppError {
    invalid(path_message("AI tool argument ", path, suffix))
}

fn join_path(path: &PyStr, separator: &str, part: &PyStr) -> PyStr {
    let mut points: Vec<u32> = path.code_points().collect();
    points.extend(separator.chars().map(u32::from));
    points.extend(part.code_points());
    PyStr::from_code_points(points).expect("code points of strings")
}

/// `isinstance(x, int)`, which includes `bool`.
fn int_like(value: &Value) -> Option<BigInt> {
    match value {
        Value::Int(n) => Some(n.clone()),
        Value::Bool(b) => Some(BigInt::from(u8::from(*b))),
        _ => None,
    }
}

/// `isinstance(x, (int, float))`, `bool` included, as a number.
#[derive(Clone)]
enum Number {
    Int(BigInt),
    Float(f64),
}

fn number_like(value: &Value) -> Option<Number> {
    match value {
        Value::Float(f) => Some(Number::Float(*f)),
        other => int_like(other).map(Number::Int),
    }
}

fn exact_floor(f: f64) -> BigInt {
    format!("{:.0}", f.floor())
        .parse()
        .expect("an integral float prints as digits")
}

/// Python's exact int/float comparison; `None` when a NaN is involved.
fn compare(a: &Number, b: &Number) -> Option<Ordering> {
    match (a, b) {
        (Number::Int(a), Number::Int(b)) => Some(a.cmp(b)),
        (Number::Float(a), Number::Float(b)) => a.partial_cmp(b),
        (Number::Int(n), Number::Float(f)) => compare_int_float(n, *f),
        (Number::Float(f), Number::Int(n)) => compare_int_float(n, *f).map(Ordering::reverse),
    }
}

fn compare_int_float(n: &BigInt, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    if f.is_infinite() {
        return Some(if f > 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    let floor = exact_floor(f);
    Some(match n.cmp(&floor) {
        Ordering::Equal if f == f.floor() => Ordering::Equal,
        Ordering::Equal => Ordering::Less,
        other => other,
    })
}

/// Python `==` between JSON-shaped values (`1 == 1.0 == True`).
pub(crate) fn py_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Str(a), Value::Str(b)) => a == b,
        (Value::List(a), Value::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| py_eq(a, b))
        }
        (Value::Dict(a), Value::Dict(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, value)| {
                    b.iter()
                        .find(|(other, _)| key_eq(key, other))
                        .is_some_and(|(_, other)| py_eq(value, other))
                })
        }
        _ => match (number_like(a), number_like(b)) {
            (Some(a), Some(b)) => compare(&a, &b) == Some(Ordering::Equal),
            _ => false,
        },
    }
}

fn key_eq(a: &Key, b: &Key) -> bool {
    match (a, b) {
        (Key::Str(a), Key::Str(b)) => a == b,
        _ => false,
    }
}

/// `_ai_schema_type_matches(value, expected)`.
fn type_matches(value: &Value, expected: &PyStr) -> bool {
    match expected.as_str() {
        Some("null") => matches!(value, Value::Null),
        Some("boolean") => matches!(value, Value::Bool(_)),
        Some("integer") => matches!(value, Value::Int(_)),
        Some("number") => match value {
            Value::Int(_) => true,
            Value::Float(f) => f.is_finite(),
            _ => false,
        },
        Some("string") => matches!(value, Value::Str(_)),
        Some("array") => matches!(value, Value::List(_)),
        Some("object") => matches!(value, Value::Dict(_)),
        _ => false,
    }
}

/// Compiled catalog patterns, by their Python text.
fn pattern(text: &str) -> Regex {
    static CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache.lock().unwrap_or_else(|poison| poison.into_inner());
    cache
        .entry(text.to_owned())
        .or_insert_with(|| pyre::compile(text))
        .clone()
}

fn str_keys(value: &Value) -> &[(Key, Value)] {
    match value {
        Value::Dict(pairs) => pairs,
        _ => &[],
    }
}

fn key_text(key: &Key) -> PyStr {
    match key {
        Key::Str(key) => key.clone(),
        other => PyStr::from(kassiber_envelope::py_str(&match other {
            Key::Null => Value::Null,
            Key::Bool(b) => Value::Bool(*b),
            Key::Int(n) => Value::Int(n.clone()),
            Key::Float(f) => Value::Float(*f),
            Key::Str(_) => unreachable!(),
        })),
    }
}

fn property<'a>(properties: Option<&'a Value>, key: &Key) -> Option<&'a Value> {
    let Key::Str(key) = key else {
        return None;
    };
    match properties {
        Some(Value::Dict(pairs)) => pairs.iter().rev().find_map(|(name, schema)| match name {
            Key::Str(name) if name == key => Some(schema),
            _ => None,
        }),
        _ => None,
    }
}

fn int_keyword(schema: &Value, keyword: &str) -> Option<BigInt> {
    get(schema, keyword).and_then(int_like)
}

fn number_keyword(schema: &Value, keyword: &str) -> Option<Number> {
    get(schema, keyword).and_then(number_like)
}

fn len_big(len: usize) -> BigInt {
    BigInt::from(len)
}

/// `_validate_ai_schema_value(value, schema, path=..., depth=...)`.
fn validate_value(
    value: &Value,
    schema: &Value,
    path: &PyStr,
    depth: usize,
) -> Result<(), ToolError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(invalid("AI tool arguments are nested too deeply").into());
    }
    let expected: Vec<Value> = match get(schema, "type") {
        Some(Value::Str(text)) => vec![Value::Str(text.clone())],
        Some(Value::List(items)) => items.clone(),
        _ => Vec::new(),
    };
    if !expected.is_empty()
        && !expected
            .iter()
            .any(|item| matches!(item, Value::Str(name) if type_matches(value, name)))
    {
        return Err(path_error(path, " has the wrong type")
            .with_details(dict([
                ("path", Value::Str(path.clone())),
                ("expected", Value::List(expected)),
            ]))
            .into());
    }
    if let Some(Value::List(allowed)) = get(schema, "enum") {
        if !allowed.iter().any(|item| py_eq(value, item)) {
            return Err(path_error(path, " is unsupported")
                .with_details(dict([
                    ("path", Value::Str(path.clone())),
                    ("allowed", Value::List(allowed.clone())),
                ]))
                .into());
        }
    }
    if let Value::Dict(pairs) = value {
        let properties = match get(schema, "properties") {
            Some(properties @ Value::Dict(_)) => Some(properties),
            _ => None,
        };
        let required = match get(schema, "required") {
            Some(Value::List(required)) => required.clone(),
            _ => Vec::new(),
        };
        let missing: Vec<Value> = required
            .into_iter()
            .filter(|key| match key {
                Value::Str(key) => !pairs
                    .iter()
                    .any(|(name, _)| matches!(name, Key::Str(name) if name == key)),
                _ => true,
            })
            .collect();
        if !missing.is_empty() {
            return Err(path_error(path, " is missing required fields")
                .with_details(dict([
                    ("path", Value::Str(path.clone())),
                    ("missing", Value::List(missing)),
                ]))
                .into());
        }
        let additional = get(schema, "additionalProperties")
            .cloned()
            .unwrap_or(Value::Bool(true));
        let mut unknown: Vec<PyStr> = pairs
            .iter()
            .filter(|(key, _)| property(properties, key).is_none())
            .map(|(key, _)| key_text(key))
            .collect();
        unknown.sort();
        if additional == Value::Bool(false) && !unknown.is_empty() {
            return Err(path_error(path, " received unsupported fields")
                .with_details(dict([
                    ("path", Value::Str(path.clone())),
                    (
                        "unknown",
                        Value::List(unknown.into_iter().map(Value::Str).collect()),
                    ),
                ]))
                .into());
        }
        for (key, item) in str_keys(value).iter() {
            let child = match property(properties, key) {
                Some(child @ Value::Dict(_)) => Some(child),
                _ if matches!(additional, Value::Dict(_)) => Some(&additional),
                _ => None,
            };
            if let Some(child) = child {
                let child_path = join_path(path, ".", &key_text(key));
                validate_value(item, child, &child_path, depth + 1)?;
            }
        }
    }
    if let Value::List(items) = value {
        if int_keyword(schema, "minItems").is_some_and(|minimum| len_big(items.len()) < minimum) {
            return Err(path_error(path, " has too few items").into());
        }
        if int_keyword(schema, "maxItems").is_some_and(|maximum| len_big(items.len()) > maximum) {
            return Err(path_error(path, " has too many items").into());
        }
        if get(schema, "uniqueItems") == Some(&Value::Bool(true)) {
            let options = Options::new().sort_keys(true);
            let mut seen = std::collections::HashSet::new();
            for item in items {
                let canonical = dumps(&json_ready(item.clone()), &options)
                    .map_err(|_| ToolError::Unexpected)?;
                if !seen.insert(canonical) {
                    return Err(path_error(path, " contains duplicate items").into());
                }
            }
        }
        if let Some(item_schema @ Value::Dict(_)) = get(schema, "items") {
            for (index, item) in items.iter().enumerate() {
                let item_path = join_path(path, "", &PyStr::from(format!("[{index}]")));
                validate_value(item, item_schema, &item_path, depth + 1)?;
            }
        }
    }
    if let Value::Str(text) = value {
        // Published schemas are a contract at external boundaries (MCP), so
        // every declared keyword is enforced, not only the types.
        let length = len_big(pyre::char_len(text));
        if int_keyword(schema, "minLength").is_some_and(|minimum| length < minimum) {
            return Err(path_error(path, " is too short").into());
        }
        if int_keyword(schema, "maxLength").is_some_and(|maximum| length > maximum) {
            return Err(path_error(path, " is too long").into());
        }
        if let Some(Value::Str(source)) = get(schema, "pattern") {
            let source = source.as_str().ok_or(ToolError::Unexpected)?;
            if !pyre::search(&pattern(source), text) {
                return Err(path_error(path, " does not match its required format").into());
            }
        }
    }
    if let Some(number) = match value {
        Value::Int(n) => Some(Number::Int(n.clone())),
        Value::Float(f) => Some(Number::Float(*f)),
        _ => None,
    } {
        let at_most = |bound: &Number, wanted: &[Ordering]| {
            compare(&number, bound).is_some_and(|order| wanted.contains(&order))
        };
        if number_keyword(schema, "exclusiveMinimum")
            .is_some_and(|bound| at_most(&bound, &[Ordering::Less, Ordering::Equal]))
        {
            return Err(path_error(path, " must be greater than its minimum").into());
        }
        if number_keyword(schema, "exclusiveMaximum")
            .is_some_and(|bound| at_most(&bound, &[Ordering::Greater, Ordering::Equal]))
        {
            return Err(path_error(path, " must be less than its maximum").into());
        }
        if number_keyword(schema, "minimum").is_some_and(|bound| at_most(&bound, &[Ordering::Less]))
        {
            return Err(path_error(path, " is below its minimum").into());
        }
        if number_keyword(schema, "maximum")
            .is_some_and(|bound| at_most(&bound, &[Ordering::Greater]))
        {
            return Err(path_error(path, " exceeds its maximum").into());
        }
    }
    Ok(())
}

/// `_validate_ai_schema_value(value, schema, path=path)`.
pub fn validate_schema_value(value: &Value, schema: &Value, path: &str) -> Result<(), ToolError> {
    validate_value(value, schema, &PyStr::from(path), 0)
}

/// `len(json.dumps(json_ready(arguments), separators=(",", ":")))`.
pub fn argument_chars(arguments: &Value) -> Result<usize, ToolError> {
    dumps(
        &json_ready(arguments.clone()),
        &Options::new().separators(",", ":"),
    )
    .map(|text| text.len())
    .map_err(|_| ToolError::Unexpected)
}

/// `_validate_ai_tool_arguments(entry, arguments)`.
pub fn validate_arguments(entry: &ToolEntry, arguments: &Value) -> Result<(), ToolError> {
    if argument_chars(arguments)? > MAX_ARGUMENT_CHARS {
        return Err(AppError::new("validation", "AI tool arguments are too large").into());
    }
    if !matches!(entry.parameters, Value::Dict(_)) {
        return Err(AppError::new("validation", "AI tool schema is invalid").into());
    }
    validate_value(
        arguments,
        &entry.parameters,
        &PyStr::from(entry.name.as_str()),
        0,
    )
}

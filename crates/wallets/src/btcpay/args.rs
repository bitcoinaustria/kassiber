//! The daemon's argument helpers (`_optional_str_arg`, `_required_str_arg`)
//! over Python strings, so lone surrogates survive as they do in Python.
use kassiber_envelope::{dict, get, s, type_name, AppError};
use kassiber_pyjson::{PyStr, Value};

/// `str(value)`.
pub fn py_text(value: &Value) -> PyStr {
    match value {
        Value::Str(text) => text.clone(),
        other => kassiber_envelope::py_str(other).into(),
    }
}

/// `_optional_str_arg`: absent or `None` is `None`; a non-string refuses;
/// the stripped text, or `None` when empty.
pub fn opt_str(args: &Value, key: &str) -> Result<Option<PyStr>, AppError> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(text)) => {
            let text = kassiber_pytext::strip_py(text);
            Ok((text.code_points().next().is_some()).then_some(text))
        }
        Some(other) => Err(
            AppError::new("validation", format!("{key} must be a string"))
                .with_details(dict([("type", s(type_name(other)))])),
        ),
    }
}

/// `_required_str_arg`.
pub fn req_str(args: &Value, key: &str, label: &str) -> Result<PyStr, AppError> {
    opt_str(args, key)?.ok_or_else(|| {
        AppError::new("validation", format!("{label} is required."))
            .with_hint(format!("Enter {}.", kassiber_pytext::lower(label)))
    })
}

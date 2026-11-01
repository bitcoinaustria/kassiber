use crate::{number, truth, Refusal};
use kassiber_pyjson::BigInt;
use kassiber_pytext::{parse_int, repr_str, NumberError};
use serde_json::Value;

/// Python `int(value)`, retaining refusal family and message. This conversion
/// applies only at the legacy diagnostic/compatibility seams, never to facts
/// admitted as integer msat by custody or the tax engine.
pub(crate) fn integer(value: &Value) -> Result<Value, Refusal> {
    if let Some(bytes) = kassiber_custody::codec::raw_bytes(value) {
        let parsed = if bytes.is_ascii() {
            parse_int(std::str::from_utf8(&bytes).expect("ASCII bytes"))
        } else {
            Err(NumberError::Invalid)
        };
        let integer = parsed.map_err(|error| {
            Refusal::new("ValueError", match error {
                NumberError::Invalid => format!("invalid literal for int() with base 10: {}", kassiber_pyjson::bytes_repr(&bytes)),
                NumberError::TooManyDigits(digits) => format!("Exceeds the limit (4300 digits) for integer string conversion: value has {digits} digits; use sys.set_int_max_str_digits() to increase the limit"),
            })
        })?;
        return number(&integer.to_string());
    }
    match value {
        Value::Bool(v) => Ok(Value::from(i64::from(*v))),
        Value::Number(n) => {
            let text = n.to_string();
            if !text.contains(['.', 'e', 'E']) {
                return Ok(value.clone());
            }
            let float = text
                .parse::<f64>()
                .map_err(|_| Refusal::new("ValueError", "cannot convert float NaN to integer"))?;
            if float.is_infinite() {
                return Err(Refusal::new(
                    "OverflowError",
                    "cannot convert float infinity to integer",
                ));
            }
            let integer = float.trunc();
            number(&format!("{integer:.0}"))
        }
        Value::String(text) => {
            let value = parse_int(text).map_err(|error| {
                let message = match error {
                    NumberError::Invalid => format!(
                        "invalid literal for int() with base 10: {}",
                        repr_str(text)
                    ),
                    NumberError::TooManyDigits(digits) => format!(
                        "Exceeds the limit (4300 digits) for integer string conversion: value has {digits} digits; use sys.set_int_max_str_digits() to increase the limit"
                    ),
                };
                Refusal::new("ValueError", message)
            })?;
            number(&value.to_string())
        }
        _ => Err(Refusal::new(
            "TypeError",
            format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                match value {
                    Value::Null => "NoneType",
                    Value::Array(_) => "list",
                    _ => "dict",
                }
            ),
        )),
    }
}

pub(crate) fn integer_or_zero(value: &Value) -> Result<Value, Refusal> {
    if truth(value) {
        integer(value)
    } else {
        Ok(Value::from(0))
    }
}

pub(crate) fn wide_or_zero(value: &Value) -> Result<BigInt, Refusal> {
    if let Some(n) = value.as_i64() {
        return Ok(n.into());
    }
    if let Some(n) = value.as_u64() {
        return Ok(n.into());
    }
    integer_or_zero(value)?
        .to_string()
        .parse()
        .map_err(|_| Refusal::new("ValueError", "invalid integer quantity"))
}

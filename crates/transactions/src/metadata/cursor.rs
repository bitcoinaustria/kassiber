//! Python's sorted compact JSON and permissive URL-safe base64 cursors.
use super::*;
use kassiber_envelope::truthy;
use kassiber_store::rusqlite::types::Value as SqlValue;

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn encode(row: &Value, filters: &Value, keys: &[&str]) -> Result<Value> {
    let mut fields = vec![("filters", filters.clone())];
    fields.extend(keys.iter().map(|key| (*key, value(row, key))));
    let bytes = json(&dict(fields))?.into_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for shift in (0..chunk.len() + 1).map(|i| 18 - i * 6) {
            out.push(ALPHABET[((bits >> shift) & 63) as usize] as char);
        }
    }
    Ok(s(out))
}

fn unbase64(raw: &str) -> Option<Vec<u8>> {
    if !raw.is_ascii() {
        return None;
    }
    let mut out = Vec::new();
    let mut bits = 0u32;
    let mut count = 0;
    let mut padding = 0;
    // b64decode(validate=False) ignores non-alphabet characters and stray '='
    // until it completes a final group. Padding is based on the original length.
    for byte in raw
        .bytes()
        .chain(std::iter::repeat_n(b'=', (4 - raw.len() % 4) % 4))
    {
        if byte == b'=' {
            if count >= 2 {
                padding += 1;
                if count + padding >= 4 {
                    return Some(out);
                }
            }
            continue;
        }
        let normalized = match byte {
            b'+' => b'-',
            b'/' => b'_',
            b => b,
        };
        let Some(n) = ALPHABET.iter().position(|b| *b == normalized) else {
            continue;
        };
        padding = 0;
        bits = (bits << 6) | n as u32;
        count += 1;
        if count >= 2 {
            out.push((bits >> (6 * count - 8 * (count - 1))) as u8);
        }
        if count == 4 {
            count = 0;
            bits = 0;
        }
    }
    (count == 0).then_some(out)
}

pub fn decode(
    raw: Option<&str>,
    filters: &Value,
    keys: &[&str],
    nonempty: bool,
) -> Result<Option<Value>> {
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let decoded = unbase64(raw)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| kassiber_pytext::loads(&text).ok())
        .filter(|row| {
            get(row, "filters").is_some_and(|v| equivalent(v, filters))
                && keys
                    .iter()
                    .all(|k| get(row, k).is_some_and(|v| !nonempty || truthy(v)))
        });
    decoded.map(Some).ok_or_else(|| AppError::new("validation", format!("Invalid cursor: {raw}"))
        .with_hint("Pass the exact next_cursor value from the previous response; do not modify it or change filters."))
}

pub(crate) fn equivalent(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Dict(a), Value::Dict(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, value)| {
                    b.iter()
                        .find(|(k, _)| k == key)
                        .is_some_and(|(_, other)| equivalent(value, other))
                })
        }
        (Value::List(a), Value::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equivalent(a, b))
        }
        (Value::Bool(a), Value::Int(b)) | (Value::Int(b), Value::Bool(a)) => {
            *b == i64::from(*a).into()
        }
        (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
            b.is_finite()
                && b.fract() == 0.0
                && kassiber_pytext::parse_int(&format!("{b:.0}")).is_ok_and(|n| n == *a)
        }
        (Value::Bool(a), Value::Float(b)) | (Value::Float(b), Value::Bool(a)) => {
            *b == f64::from(u8::from(*a))
        }
        _ => a == b,
    }
}

pub(crate) fn limit(limit: Option<i64>, default: i64) -> Result<i64> {
    let limit = limit.unwrap_or(default);
    if limit <= 0 {
        return Err(AppError::new("validation", "--limit must be positive"));
    }
    if limit > 1000 {
        return Err(
            AppError::new("validation", "--limit cannot exceed 1000").with_hint(
                "Use cursor-based pagination instead of larger limits; max page size is 1000.",
            ),
        );
    }
    Ok(limit)
}

pub fn bind(v: &Value) -> Result<SqlValue> {
    Ok(match v {
        Value::Null => SqlValue::Null,
        Value::Str(t) => SqlValue::Text(
            t.as_str()
                .ok_or_else(|| AppError::new("validation", "stored text is not valid UTF-8"))?
                .into(),
        ),
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Int(n) => SqlValue::Integer(i64::try_from(n.clone()).map_err(|_| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        })?),
        Value::Float(n) => SqlValue::Real(*n),
        _ => return Err(AppError::new("validation", "unsupported stored value")),
    })
}

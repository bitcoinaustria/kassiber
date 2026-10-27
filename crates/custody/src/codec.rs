//! CPython JSON loading/encoding at commitment boundaries. serde is the outer
//! wire codec; it must never format persisted commitment preimages.
use kassiber_pyjson::{BigInt, Key, Options, PyStr, Value as Py};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::str::FromStr;

pub fn from_json(v: &Value) -> Result<Py, String> {
    Ok(match v {
        Value::Null => Py::Null,
        Value::Bool(v) => Py::Bool(*v),
        Value::String(v) => Py::Str(v.as_str().into()),
        Value::Number(n) => {
            let s = n.to_string();
            if s.contains(['.', 'e', 'E']) {
                Py::Float(s.parse::<f64>().map_err(|e| e.to_string())?)
            } else {
                Py::Int(BigInt::from_str(&s).map_err(|e| e.to_string())?)
            }
        }
        Value::Array(v) => Py::List(v.iter().map(from_json).collect::<Result<_, _>>()?),
        Value::Object(v) => Py::Dict(
            v.iter()
                .map(|(k, v)| Ok((Key::Str(k.as_str().into()), from_json(v)?)))
                .collect::<Result<_, String>>()?,
        ),
    })
}

pub fn to_json(v: &Py) -> Value {
    match v {
        Py::Null => Value::Null,
        Py::Bool(v) => Value::Bool(*v),
        Py::Int(v) => serde_json::from_str(&v.to_string()).unwrap_or(Value::Null),
        Py::Float(v) => serde_json::Number::from_f64(*v)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Py::Decimal(v) => Value::String(v.to_string()),
        Py::Str(v) => Value::String(v.as_str().map(str::to_owned).unwrap_or_else(|| {
            v.code_points()
                .map(|c| char::from_u32(c).unwrap_or('\u{fffd}'))
                .collect()
        })),
        Py::List(v) => Value::Array(v.iter().map(to_json).collect()),
        Py::Dict(v) => Value::Object(
            v.iter()
                .filter_map(|(k, v)| {
                    if let Key::Str(k) = k {
                        Some((k.as_str()?.to_owned(), to_json(v)))
                    } else {
                        None
                    }
                })
                .collect(),
        ),
    }
}

pub fn dumps(v: &Py, ascii: bool) -> Result<String, String> {
    let options = Options::new()
        .sort_keys(true)
        .compact()
        .ensure_ascii(ascii)
        .default_str(true);
    match kassiber_pyjson::dumps(v, &options) {
        Ok(text) => Ok(text),
        Err(kassiber_pyjson::Error::Surrogate) => Err(surrogate_error(v)?),
        Err(error) => Err(error.to_string()),
    }
}

// json.dumps itself accepts a lone surrogate; CPython refuses it only while
// UTF-8 encoding the preimage. Retain that exception text in canonical issues.
fn surrogate_error(v: &Py) -> Result<String, String> {
    let ascii = kassiber_pyjson::dumps(
        v,
        &Options::new().sort_keys(true).compact().default_str(true),
    )
    .map_err(|e| e.to_string())?;
    let bytes = ascii.as_bytes();
    let mut at = 0;
    let mut points = Vec::new();
    while at < bytes.len() {
        if bytes[at] == b'\\' && bytes.get(at + 1) == Some(&b'u') {
            let point =
                u32::from_str_radix(&ascii[at + 2..at + 6], 16).map_err(|e| e.to_string())?;
            if point >= 32 {
                at += 6;
                if (0xd800..=0xdbff).contains(&point) && bytes.get(at..at + 2) == Some(b"\\u") {
                    let low = u32::from_str_radix(&ascii[at + 2..at + 6], 16)
                        .map_err(|e| e.to_string())?;
                    if (0xdc00..=0xdfff).contains(&low) {
                        points.push(0x10000 + ((point - 0xd800) << 10) + low - 0xdc00);
                        at += 6;
                        continue;
                    }
                }
                points.push(point);
                continue;
            }
        }
        if bytes[at] == b'\\' {
            points.push(u32::from(bytes[at]));
            at += 1;
            if at < bytes.len() {
                points.push(u32::from(bytes[at]));
                at += 1;
            }
        } else {
            points.push(u32::from(bytes[at]));
            at += 1;
        }
    }
    let start = points
        .iter()
        .position(|p| (0xd800..=0xdfff).contains(p))
        .ok_or("surrogate position missing")?;
    let length = points[start..]
        .iter()
        .take_while(|p| (0xd800..=0xdfff).contains(*p))
        .count();
    Ok(if length == 1 {
        format!("'utf-8' codec can't encode character '\\u{:04x}' in position {}: surrogates not allowed", points[start],start)
    } else {
        format!(
            "'utf-8' codec can't encode characters in position {}-{}: surrogates not allowed",
            start,
            start + length - 1
        )
    })
}
pub fn hash_py(v: &Py, ascii: bool) -> Result<(String, String), String> {
    let text = dumps(v, ascii)?;
    Ok((sha256(&text), text))
}
pub fn hash(v: &Value, ascii: bool) -> Result<(String, String), String> {
    hash_py(&from_json(v)?, ascii)
}
pub fn sha256(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// Python accepts nonfinite constants, unbounded ints and lone surrogates.
/// Those are needed even when the outer request only holds raw JSON as text.
/// Encoding an unpaired surrogate with ensure_ascii=false still refuses it.
pub fn loads(text: &str) -> Result<Py, String> {
    struct Parser<'a> {
        text: &'a [u8],
        at: usize,
    }
    impl Parser<'_> {
        fn ws(&mut self) {
            while self
                .text
                .get(self.at)
                .is_some_and(|b| b" \t\r\n".contains(b))
            {
                self.at += 1;
            }
        }
        fn take(&mut self, token: &[u8]) -> bool {
            if self.text[self.at..].starts_with(token) {
                self.at += token.len();
                true
            } else {
                false
            }
        }
        fn hex(&mut self) -> Result<u32, String> {
            let bytes = self
                .text
                .get(self.at..self.at + 4)
                .ok_or("invalid unicode escape")?;
            let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
            let point = u32::from_str_radix(text, 16).map_err(|e| e.to_string())?;
            self.at += 4;
            Ok(point)
        }
        fn string(&mut self) -> Result<PyStr, String> {
            if !self.take(b"\"") {
                return Err("expected string".into());
            }
            let mut points = Vec::new();
            loop {
                let byte = *self.text.get(self.at).ok_or("unterminated string")?;
                self.at += 1;
                match byte {
                    b'"' => break,
                    b'\\' => {
                        let escape = *self.text.get(self.at).ok_or("unterminated escape")?;
                        self.at += 1;
                        let point = match escape {
                            b'"' => 34,
                            b'\\' => 92,
                            b'/' => 47,
                            b'b' => 8,
                            b'f' => 12,
                            b'n' => 10,
                            b'r' => 13,
                            b't' => 9,
                            b'u' => {
                                let high = self.hex()?;
                                if (0xd800..=0xdbff).contains(&high)
                                    && self.text[self.at..].starts_with(b"\\u")
                                {
                                    let saved = self.at;
                                    self.at += 2;
                                    let low = self.hex()?;
                                    if (0xdc00..=0xdfff).contains(&low) {
                                        0x10000 + ((high - 0xd800) << 10) + low - 0xdc00
                                    } else {
                                        self.at = saved;
                                        high
                                    }
                                } else {
                                    high
                                }
                            }
                            _ => return Err("invalid string escape".into()),
                        };
                        points.push(point);
                    }
                    0..=31 => return Err("invalid control character".into()),
                    32..=127 => points.push(u32::from(byte)),
                    _ => {
                        self.at -= 1;
                        let c = std::str::from_utf8(&self.text[self.at..])
                            .map_err(|e| e.to_string())?
                            .chars()
                            .next()
                            .ok_or("invalid unicode")?;
                        self.at += c.len_utf8();
                        points.push(u32::from(c));
                    }
                }
            }
            PyStr::from_code_points(points).map_err(|e| e.to_string())
        }
        fn value(&mut self, depth: usize) -> Result<Py, String> {
            if depth > 1000 {
                return Err("JSON recursion limit".into());
            }
            self.ws();
            Ok(match self.text.get(self.at) {
                Some(b'"') => Py::Str(self.string()?),
                Some(b'{') => {
                    self.at += 1;
                    self.ws();
                    let mut values: Vec<(Key, Py)> = Vec::new();
                    if !self.take(b"}") {
                        loop {
                            self.ws();
                            let key = Key::Str(self.string()?);
                            self.ws();
                            if !self.take(b":") {
                                return Err("expected colon".into());
                            }
                            let value = self.value(depth + 1)?;
                            if let Some(old) = values.iter_mut().find(|(k, _)| *k == key) {
                                old.1 = value;
                            } else {
                                values.push((key, value));
                            }
                            self.ws();
                            if self.take(b"}") {
                                break;
                            }
                            if !self.take(b",") {
                                return Err("expected comma".into());
                            }
                        }
                    }
                    Py::Dict(values)
                }
                Some(b'[') => {
                    self.at += 1;
                    self.ws();
                    let mut values = Vec::new();
                    if !self.take(b"]") {
                        loop {
                            values.push(self.value(depth + 1)?);
                            self.ws();
                            if self.take(b"]") {
                                break;
                            }
                            if !self.take(b",") {
                                return Err("expected comma".into());
                            }
                        }
                    }
                    Py::List(values)
                }
                _ if self.take(b"true") => Py::Bool(true),
                _ if self.take(b"false") => Py::Bool(false),
                _ if self.take(b"null") => Py::Null,
                _ if self.take(b"NaN") => Py::Float(f64::NAN),
                _ if self.take(b"Infinity") => Py::Float(f64::INFINITY),
                _ if self.take(b"-Infinity") => Py::Float(f64::NEG_INFINITY),
                _ => {
                    let start = self.at;
                    while self
                        .text
                        .get(self.at)
                        .is_some_and(|b| b"0123456789+-.eE".contains(b))
                    {
                        self.at += 1;
                    }
                    let text = std::str::from_utf8(&self.text[start..self.at])
                        .map_err(|e| e.to_string())?;
                    // serde enforces the JSON grammar; unbounded integers and
                    // float overflow remain Python's own numeric types.
                    if text.is_empty() || serde_json::from_str::<Value>(text).is_err() {
                        return Err("invalid JSON value".into());
                    }
                    if text.contains(['.', 'e', 'E']) {
                        Py::Float(text.parse::<f64>().map_err(|e| e.to_string())?)
                    } else {
                        if text.trim_start_matches('-').len() > 4300 {
                            return Err("integer string conversion limit".into());
                        }
                        Py::Int(BigInt::from_str(text).map_err(|e| e.to_string())?)
                    }
                }
            })
        }
    }
    let mut parser = Parser {
        text: text.as_bytes(),
        at: 0,
    };
    let value = parser.value(0)?;
    parser.ws();
    if parser.at != parser.text.len() {
        return Err("extra JSON data".into());
    }
    Ok(value)
}

pub fn object(v: &Value) -> Value {
    if v.is_object() {
        return v.clone();
    }
    if let Some(text) = v.as_str() {
        if let Ok(parsed) = loads(text) {
            let value = to_json(&parsed);
            if value.is_object() {
                return value;
            }
        }
    }
    serde_json::json!({})
}

pub fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n
            .to_string()
            .parse::<f64>()
            .map(|n| n != 0.0)
            .unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
pub fn pystr(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => {
            let s = n.to_string();
            if s.contains(['.', 'e', 'E']) {
                kassiber_pyjson::float_repr(s.parse().unwrap_or(0.0))
            } else {
                s
            }
        }
        _ => v.to_string(),
    }
}
pub fn text(v: &Value) -> String {
    if truth(v) {
        pystr(v)
    } else {
        String::new()
    }
}
pub fn integer(v: &Value) -> Result<Value, String> {
    if let Some(b) = v.as_bool() {
        return Ok(serde_json::json!(i64::from(b)));
    }
    if let Some(n) = v.as_number() {
        if n.to_string().contains(['.', 'e', 'E']) {
            let float = n
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or("invalid integer quantity")?;
            return serde_json::from_str(&format!("{:.0}", float.trunc()))
                .map_err(|e| e.to_string());
        }
    }
    let text = pystr(v);
    let text = crate::python_text::strip(&text).replace('_', "");
    let n = BigInt::from_str(&text).map_err(|_| "invalid integer quantity".to_owned())?;
    serde_json::from_str(&n.to_string()).map_err(|e| e.to_string())
}
pub fn integer_or_zero(v: &Value) -> Result<Value, String> {
    if !truth(v) {
        Ok(serde_json::json!(0))
    } else {
        integer(v)
    }
}
pub fn int(v: &Value) -> i64 {
    integer(v).ok().and_then(|v| v.as_i64()).unwrap_or(0)
}

pub fn wide(v: &Value) -> i128 {
    integer(v)
        .ok()
        .and_then(|v| v.to_string().parse().ok())
        .unwrap_or(0)
}

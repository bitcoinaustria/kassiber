//! CPython JSON loading/encoding at commitment boundaries. serde is the outer
//! wire codec; it must never format persisted commitment preimages.
use kassiber_pyjson::{BigInt, Key, Options, PyStr, Value as Py};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::rc::Rc;
use std::str::FromStr;

/// Arrays at least this long convert their items concurrently, in order.
const PARALLEL_ITEMS: usize = 1024;

pub fn from_json(v: &Value) -> Result<Py, String> {
    if let Some(bytes) = raw_bytes(v) {
        return Ok(Py::Bytes(bytes));
    }
    if let Some(dict) = escaped_dict(v) {
        return dict
            .iter()
            .map(|(k, v)| Ok((Key::from(k.as_str()), from_json(v)?)))
            .collect::<Result<_, String>>()
            .map(Py::Dict);
    }
    Ok(match v {
        Value::Array(v) if v.len() >= PARALLEL_ITEMS => Py::List(
            crate::par::map(v, from_json)
                .into_iter()
                .map(crate::par::unwrap)
                .collect::<Result<_, _>>()?,
        ),
        Value::Null => Py::Null,
        Value::Bool(v) => Py::Bool(*v),
        Value::String(v) => Py::Str(v.as_str().into()),
        Value::Number(n) if n.as_i64().is_some() => Py::Int(n.as_i64().unwrap().into()),
        Value::Number(n) if n.as_u64().is_some() => Py::Int(n.as_u64().unwrap().into()),
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

/// Appends `kassiber_pyjson::dumps(&from_json(v)?, options)` for a value
/// `level` containers deep, or returns `None` where that conversion or
/// encoding could fail or differ, so the caller takes the original path.
pub(crate) fn dumps_json_into(
    v: &Value,
    options: &Options,
    level: usize,
    out: &mut String,
) -> Option<()> {
    if raw_bytes(v).is_some() || escaped_dict(v).is_some() {
        return None;
    }
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::String(s) => kassiber_pyjson::dumps_str_into(s, options, out),
        Value::Number(n) => {
            let s = n.to_string();
            if s.contains(['.', 'e', 'E']) {
                let f = s.parse::<f64>().ok()?;
                if !f.is_finite() || options.separators.is_some() || !options.allow_nan {
                    return None;
                }
                out.push_str(&kassiber_pyjson::float_repr(f));
            } else {
                // A plain integer of at most 18 digits is written as its i64.
                let digits = s.strip_prefix('-').unwrap_or(&s);
                if digits.is_empty()
                    || digits.len() > 18
                    || !digits.bytes().all(|b| b.is_ascii_digit())
                {
                    return None;
                }
                out.push_str(&s.parse::<i64>().ok()?.to_string());
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return Some(());
            }
            if level >= kassiber_pyjson::MAX_DEPTH
                || options.indent.is_some()
                || options.separators.is_some()
            {
                return None;
            }
            out.push('[');
            if items.len() >= PARALLEL_ITEMS {
                let texts = crate::par::map(items, |item| {
                    let mut text = String::new();
                    dumps_json_into(item, options, level + 1, &mut text).map(|()| text)
                });
                for (index, text) in texts.into_iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&crate::par::unwrap(text)?);
                }
            } else {
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    dumps_json_into(item, options, level + 1, out)?;
                }
            }
            out.push(']');
        }
        Value::Object(items) => {
            if items.is_empty() {
                out.push_str("{}");
                return Some(());
            }
            if level >= kassiber_pyjson::MAX_DEPTH
                || options.indent.is_some()
                || options.separators.is_some()
                || options.sort_keys
            {
                return None;
            }
            out.push('{');
            for (index, (key, item)) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                kassiber_pyjson::dumps_str_into(key, options, out);
                out.push_str(": ");
                dumps_json_into(item, options, level + 1, out)?;
            }
            out.push('}');
        }
    }
    Some(())
}

/// Whether `dumps(v, true) == dumps(&from_json(&to_json(v))?, true)` holds
/// with neither side failing: no non-finite float, `Decimal`, surrogate,
/// non-string or repeated key, int near the digit limit, or deep nesting.
/// Exactness checks may skip their round trip when this holds.
pub(crate) fn lossless(v: &Py) -> bool {
    fn walk(v: &Py, depth: usize) -> bool {
        match v {
            Py::Null | Py::Bool(_) => true,
            Py::Int(n) => n.bits() < 14_000,
            Py::Float(f) => f.is_finite(),
            Py::Str(s) => !s.has_surrogates(),
            Py::Decimal(_) | Py::Bytes(_) => false,
            Py::List(items) if items.len() >= PARALLEL_ITEMS => {
                depth < 900
                    && crate::par::map(items, |item| walk(item, depth + 1))
                        .into_iter()
                        .all(crate::par::unwrap)
            }
            Py::List(items) => depth < 900 && items.iter().all(|item| walk(item, depth + 1)),
            Py::Dict(items) => {
                if depth >= 900 {
                    return false;
                }
                let mut keys = Vec::with_capacity(items.len());
                for (key, item) in items {
                    match key {
                        Key::Str(k) if !k.has_surrogates() => keys.push(k),
                        _ => return false,
                    }
                    if !walk(item, depth + 1) {
                        return false;
                    }
                }
                keys.sort_unstable();
                keys.windows(2).all(|pair| pair[0] != pair[1])
            }
        }
    }
    walk(v, 0)
}

pub fn to_json(v: &Py) -> Value {
    match v {
        Py::Null => Value::Null,
        Py::Bool(v) => Value::Bool(*v),
        Py::Int(v) => {
            if let Ok(n) = i64::try_from(v) {
                Value::from(n)
            } else if let Ok(n) = u64::try_from(v) {
                Value::from(n)
            } else {
                serde_json::from_str(&v.to_string()).unwrap_or(Value::Null)
            }
        }
        Py::Float(v) => serde_json::Number::from_f64(*v)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Py::Decimal(v) => Value::String(v.to_string()),
        Py::Str(v) => Value::String(v.as_str().map(str::to_owned).unwrap_or_else(|| {
            v.code_points()
                .map(|c| char::from_u32(c).unwrap_or('\u{fffd}'))
                .collect()
        })),
        // Only the private adapter tag denotes a byte scalar. Literal maps
        // with a reserved key are escaped below, so raw JSON cannot forge it.
        Py::Bytes(bytes) => {
            use std::fmt::Write;
            let mut hex = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                write!(hex, "{byte:02x}").unwrap();
            }
            serde_json::json!({"$kassiber_bytes": hex})
        }
        Py::List(v) if v.len() >= PARALLEL_ITEMS => Value::Array(
            crate::par::map(v, to_json)
                .into_iter()
                .map(crate::par::unwrap)
                .collect(),
        ),
        Py::List(v) => Value::Array(v.iter().map(to_json).collect()),
        Py::Dict(v) => json_dict(
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
fn hash_json_into(v: &Value, options: &Options, depth: usize, out: &mut String) -> Option<()> {
    use std::fmt::Write;
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Value::String(value) => kassiber_pyjson::dumps_str_into(value, options, out),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                write!(out, "{value}").ok()?;
            } else if let Some(value) = value.as_u64() {
                write!(out, "{value}").ok()?;
            } else {
                let text = value.to_string();
                if !text.contains(['.', 'e', 'E']) {
                    return None;
                }
                let value: f64 = text.parse().ok()?;
                if !value.is_finite() {
                    return None;
                }
                out.push_str(&kassiber_pyjson::float_repr(value));
            }
        }
        Value::Array(values) => {
            if !values.is_empty() && depth >= 900 {
                return None;
            }
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                hash_json_into(value, options, depth + 1, out)?;
            }
            out.push(']');
        }
        Value::Object(values) => {
            if !values.is_empty() && depth >= 900 {
                return None;
            }
            let mut fields: Vec<_> = values.iter().collect();
            fields.sort_unstable_by(|a, b| a.0.cmp(b.0));
            out.push('{');
            for (index, (key, value)) in fields.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                kassiber_pyjson::dumps_str_into(key, options, out);
                out.push(':');
                hash_json_into(value, options, depth + 1, out)?;
            }
            out.push('}');
        }
    }
    Some(())
}

pub fn hash(v: &Value, ascii: bool) -> Result<(String, String), String> {
    let mut text = String::new();
    if hash_json_into(v, &Options::new().ensure_ascii(ascii), 0, &mut text).is_some() {
        Ok((sha256(&text), text))
    } else {
        hash_py(&from_json(v)?, ascii)
    }
}
pub fn sha256(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// Python accepts nonfinite constants, unbounded ints and lone surrogates.
/// Those are needed even when the outer request only holds raw JSON as text.
/// Encoding an unpaired surrogate with ensure_ascii=false still refuses it.
pub fn loads(text: &str) -> Result<Py, String> {
    loads_bytes(text.as_bytes())
}

/// json.loads(bytes), including encoding detection and raw surrogate identity.
pub fn load_json_bytes(bytes: &[u8]) -> Result<Py, String> {
    let text = kassiber_pyjson::json_bytes_string(bytes).map_err(str::to_owned)?;
    loads_bytes(text.as_surrogatepass_bytes())
}

/// JSON after Python's surrogatepass byte decoding. Cursor byte inputs can
/// contain raw surrogate code points, distinct from JSON-escaped pairs.
pub(crate) fn loads_bytes(text: &[u8]) -> Result<Py, String> {
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
            // Fast path: no escape or control character before the closing
            // quote, and valid UTF-8 (which excludes surrogates) decodes to
            // exactly these bytes.
            let rest = &self.text[self.at..];
            if let Some(end) = rest.iter().position(|&b| b == b'"' || b == b'\\' || b < 32) {
                if rest[end] == b'"' {
                    if let Ok(text) = std::str::from_utf8(&rest[..end]) {
                        self.at += end + 1;
                        return Ok(PyStr::from(text));
                    }
                }
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
                        let width = match byte {
                            0xc2..=0xdf => 2,
                            0xe0..=0xef => 3,
                            0xf0..=0xf4 => 4,
                            _ => return Err("invalid unicode".into()),
                        };
                        let encoded = self
                            .text
                            .get(self.at..self.at + width)
                            .ok_or("invalid unicode")?;
                        let point = if width == 3
                            && encoded[0] == 0xed
                            && (0xa0..=0xbf).contains(&encoded[1])
                            && (0x80..=0xbf).contains(&encoded[2])
                        {
                            (u32::from(encoded[0] & 15) << 12)
                                | (u32::from(encoded[1] & 63) << 6)
                                | u32::from(encoded[2] & 63)
                        } else {
                            u32::from(
                                std::str::from_utf8(encoded)
                                    .map_err(|e| e.to_string())?
                                    .chars()
                                    .next()
                                    .ok_or("invalid unicode")?,
                            )
                        };
                        self.at += width;
                        points.push(point);
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
                    // A plain JSON integer of at most 18 digits is an exact i64.
                    let digits = text.strip_prefix('-').unwrap_or(text).as_bytes();
                    if !digits.is_empty()
                        && digits.len() <= 18
                        && digits.iter().all(u8::is_ascii_digit)
                        && (digits[0] != b'0' || digits.len() == 1)
                    {
                        let number: i64 = text
                            .parse()
                            .map_err(|e: std::num::ParseIntError| e.to_string())?;
                        return Ok(Py::Int(BigInt::from(number)));
                    }
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
    let mut parser = Parser { text, at: 0 };
    let value = parser.value(0)?;
    parser.ws();
    if parser.at != parser.text.len() {
        return Err("extra JSON data".into());
    }
    Ok(value)
}

/// Pure conversions of embedded JSON text, memoized by kind and exact text for
/// one custody operation. Results depend only on the text, so a hit is the
/// value the conversion would compute again.
#[derive(Clone, Copy)]
pub(crate) enum Memo {
    Object,
    Transfer,
}

thread_local! {
    static MEMOS: std::cell::RefCell<[crate::fxhash::HashMap<String, Rc<Value>>; 2]> =
        std::cell::RefCell::new([Default::default(), Default::default()]);
}

pub(crate) fn memo(kind: Memo, text: &str, convert: impl FnOnce() -> Value) -> Value {
    memo_shared(kind, text, convert).as_ref().clone()
}

pub(crate) fn memo_shared(kind: Memo, text: &str, convert: impl FnOnce() -> Value) -> Rc<Value> {
    if let Some(hit) = MEMOS.with(|m| m.borrow()[kind as usize].get(text).cloned()) {
        return hit;
    }
    let value = Rc::new(convert());
    MEMOS.with(|m| m.borrow_mut()[kind as usize].insert(text.to_owned(), value.clone()));
    value
}

/// Appends `item` to its key's group, creating groups in first-seen order:
/// the order a linear search over `groups` keeps, with a hashed lookup.
pub(crate) fn push_grouped<K: Eq + std::hash::Hash + Clone, T>(
    groups: &mut Vec<(K, Vec<T>)>,
    index: &mut std::collections::HashMap<K, usize, impl std::hash::BuildHasher>,
    key: K,
    item: T,
) {
    if let Some(&at) = index.get(&key) {
        groups[at].1.push(item);
    } else {
        index.insert(key.clone(), groups.len());
        groups.push((key, vec![item]));
    }
}

/// Releases memoized conversions at the end of an operation.
pub(crate) fn clear_memos() {
    MEMOS.with(|m| {
        m.borrow_mut()
            .iter_mut()
            .for_each(crate::fxhash::HashMap::clear)
    });
    crate::graph::clear_scopes();
}

pub fn object(v: &Value) -> Value {
    if let Some(bytes) = raw_bytes(v) {
        return load_json_bytes(&bytes)
            .ok()
            .map(|value| to_json(&value))
            .filter(Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
    }
    if v.is_object() {
        return v.clone();
    }
    if let Some(text) = v.as_str() {
        return memo(Memo::Object, text, || {
            if let Ok(parsed) = loads(text) {
                let value = to_json(&parsed);
                if value.is_object() {
                    return value;
                }
            }
            serde_json::json!({})
        });
    }
    serde_json::json!({})
}

pub fn truth(v: &Value) -> bool {
    if let Some(bytes) = raw_bytes(v) {
        return !bytes.is_empty();
    }
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        // Plain integers are zero exactly when their value is.
        Value::Number(n) if n.as_i64().is_some() => n.as_i64() != Some(0),
        Value::Number(n) if n.as_u64().is_some() => true,
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
    if let Some(bytes) = raw_bytes(v) {
        return kassiber_pyjson::bytes_repr(&bytes);
    }
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
    if let Some(bytes) = raw_bytes(v) {
        let text = std::str::from_utf8(&bytes)
            .ok()
            .filter(|s| s.is_ascii())
            .ok_or("invalid integer quantity")?;
        return integer(&Value::String(text.into()));
    }
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
    // int(str) accepts Unicode decimal digits and single separators between
    // digits. Its whitespace excludes the four C0 separators str.strip takes.
    if text.chars().any(|c| matches!(c, '\u{1c}'..='\u{1f}')) {
        return Err("invalid integer quantity".into());
    }
    let text = crate::python_text::strip(&text);
    let mut normalized = String::new();
    let mut digit = false;
    for (i, c) in text.chars().enumerate() {
        if i == 0 && matches!(c, '+' | '-') {
            normalized.push(c);
        } else if c == '_' && digit {
            digit = false;
        } else if let Some(n) = crate::python_text::decimal_digit(c) {
            normalized.push(char::from(b'0' + n));
            digit = true;
        } else {
            return Err("invalid integer quantity".into());
        }
    }
    if !digit {
        return Err("invalid integer quantity".into());
    }
    let n = BigInt::from_str(&normalized).map_err(|_| "invalid integer quantity".to_owned())?;
    serde_json::from_str(&n.to_string()).map_err(|e| e.to_string())
}

/// Restore a literal mapping escaped by the transitional JSON record adapter.
pub fn escaped_dict(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    let map = value.as_object()?;
    if map.len() == 1 {
        map.get("$kassiber_dict")?.as_object()
    } else {
        None
    }
}

/// Preserve mapping identity when its sole key overlaps a private scalar tag.
pub fn json_dict(map: serde_json::Map<String, Value>) -> Value {
    if map.len() == 1 && (map.contains_key("$kassiber_bytes") || map.contains_key("$kassiber_dict"))
    {
        serde_json::json!({"$kassiber_dict": map})
    } else {
        Value::Object(map)
    }
}

/// JSON requests contain literal maps, never native byte scalars. Escape the
/// adapter's private tags when admitting that separate, text-based seam.
pub(crate) fn escape_wire_tags(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(escape_wire_tags),
        Value::Object(map) => {
            map.values_mut().for_each(escape_wire_tags);
            if map.len() == 1
                && (map.contains_key("$kassiber_bytes") || map.contains_key("$kassiber_dict"))
            {
                *value = json_dict(std::mem::take(map));
            }
        }
        _ => {}
    }
}

/// Internal representation used only by the transitional JSON-shaped custody
/// records and the existing test recording codec. This is a bytes scalar, not
/// a mapping and not decoded text; consumers must apply their own conversion.
pub fn raw_bytes(value: &Value) -> Option<Vec<u8>> {
    let map = value.as_object()?;
    if map.len() != 1 {
        return None;
    }
    let hex = map.get("$kassiber_bytes")?.as_str()?;
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let digit = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                _ => None,
            };
            Some(digit(pair[0])? * 16 + digit(pair[1])?)
        })
        .collect()
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

/// Arithmetic only on values already converted by `integer` (or typed facts).
pub fn wide(v: &Value) -> i128 {
    debug_assert!(
        v.is_number() && !v.to_string().contains(['.', 'e', 'E']),
        "unconverted integer fact: {v}"
    );
    v.to_string()
        .parse()
        .expect("converted integer exceeds custody arithmetic range")
}
/// `wide` when the converted integer fits, else `None` for an exact fallback.
pub fn checked_wide(v: &Value) -> Option<i128> {
    v.to_string().parse().ok()
}
/// Exact form of a converted integer Python admits beyond `i128`.
pub fn big(v: &Value) -> BigInt {
    BigInt::from_str(&v.to_string()).expect("unconverted integer fact")
}
pub fn number(n: &BigInt) -> Value {
    Value::Number(n.to_string().parse().expect("integer literal"))
}

#[cfg(test)]
mod byte_tests {
    use super::*;
    #[test]
    fn raw_bytes_preserve_identity_truth_integer_and_literal_maps() {
        for bytes in [
            vec![],
            vec![255, 0],
            b"77".to_vec(),
            b"{\"chain\":\"bitcoin\"}".to_vec(),
        ] {
            let value = Py::Bytes(bytes.clone());
            let stored = to_json(&value);
            assert_eq!(from_json(&stored).unwrap(), value);
            assert_eq!(truth(&stored), !bytes.is_empty());
            assert_eq!(pystr(&stored), kassiber_pyjson::bytes_repr(&bytes));
            assert!(!lossless(&value));
            assert_eq!(
                kassiber_pyjson::dumps(&value, &Options::new()).unwrap_err(),
                kassiber_pyjson::Error::BytesNotSerializable
            );
            let mut out = String::new();
            assert_eq!(dumps_json_into(&stored, &Options::new(), 0, &mut out), None);
        }
        let Py::Str(raw) = load_json_bytes(b"\"\xed\xa0\x80\xed\xb0\x80\"").unwrap() else {
            panic!("raw string")
        };
        assert_eq!(raw.code_points().collect::<Vec<_>>(), vec![0xd800, 0xdc00]);
        let Py::Str(escaped) = load_json_bytes(br#""\ud800\udc00""#).unwrap() else {
            panic!("escaped string")
        };
        assert_eq!(escaped.code_points().collect::<Vec<_>>(), vec![0x10000]);
        assert_eq!(
            integer(&to_json(&Py::Bytes(b"77".to_vec()))).unwrap(),
            serde_json::json!(77)
        );
        assert!(integer(&to_json(&Py::Bytes("٧٧".as_bytes().to_vec()))).is_err());
        for key in ["$kassiber_bytes", "$kassiber_dict", "$bytes"] {
            let value = Py::Dict(vec![(Key::from(key), Py::from("ff00"))]);
            assert_eq!(from_json(&to_json(&value)).unwrap(), value);
        }
        assert_eq!(
            object(&to_json(&Py::Bytes(b"{\"chain\":\"bitcoin\"}".to_vec()))),
            serde_json::json!({"chain":"bitcoin"})
        );
        assert_eq!(
            object(&to_json(&Py::Bytes(vec![255, 0]))),
            serde_json::json!({})
        );
    }
}

#[cfg(test)]
mod byte_wire_tests {
    use super::*;
    #[test]
    fn json_snapshot_cannot_forge_internal_byte_scalars() {
        let mut snapshot = crate::snapshot::Snapshot {
            schema_version: 1,
            operation: "custody".into(),
            as_of: "now".into(),
            ..Default::default()
        };
        for key in ["$kassiber_bytes", "$kassiber_dict"] {
            let literal = serde_json::json!({key:"ff00"});
            snapshot.transactions =
                vec![serde_json::from_value(serde_json::json!({"note":literal})).unwrap()];
            let encoded = serde_json::to_string(&snapshot).unwrap();
            let typed = crate::parse(&encoded).unwrap();
            let decoded = typed.stages();
            assert_eq!(
                typed.text().unwrap(),
                kassiber_pyjson::dumps(&loads(&encoded).unwrap(), &Options::new()).unwrap()
            );
            let expected = Py::Dict(vec![(Key::from(key), Py::from("ff00"))]);
            assert_eq!(
                from_json(&decoded.transactions[0]["note"]).unwrap(),
                expected
            );
            let mut out = String::new();
            assert_eq!(
                dumps_json_into(
                    &decoded.transactions[0]["note"],
                    &Options::new(),
                    0,
                    &mut out
                ),
                None
            );
        }
    }
}

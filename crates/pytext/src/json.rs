//! `json.loads(text)` with CPython 3.11's C scanner rules and messages.

use std::collections::HashMap;
use std::fmt;

use kassiber_pyjson::{Key, PyStr, Value};
use num_bigint::BigInt;

use crate::number::INT_MAX_STR_DIGITS;

/// Deepest nesting of arrays and objects [`loads`] accepts. CPython raises
/// `RecursionError` near its recursion limit of 1,000 frames, which counts
/// its caller's frames too.
pub const MAX_NESTING: usize = 1000;

/// What went wrong, in the exception CPython raises.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonErrorKind {
    /// `json.JSONDecodeError`: `msg` at code point `pos`.
    Decode {
        msg: &'static str,
        pos: usize,
        lineno: usize,
        colno: usize,
    },
    /// `ValueError` from an int literal with more than 4,300 digits.
    IntTooLarge { digits: usize },
    /// `RecursionError` from nesting deeper than [`MAX_NESTING`]; `object`
    /// says which container was open.
    TooDeep { object: bool },
    /// `ValueError` from a `parse_constant` hook that refuses `NaN`,
    /// `Infinity` and `-Infinity` ([`loads_py_without_constants`]).
    Constant { name: &'static str },
}

/// A failed [`loads`]. Its `Display` is `str(exc)` of the Python exception.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    pub kind: JsonErrorKind,
}

impl JsonError {
    /// Whether Python raises `json.JSONDecodeError` (the daemon catches
    /// only that; the other two escape its loop).
    pub fn is_decode_error(&self) -> bool {
        matches!(self.kind, JsonErrorKind::Decode { .. })
    }
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            JsonErrorKind::Decode {
                msg,
                pos,
                lineno,
                colno,
            } => write!(f, "{msg}: line {lineno} column {colno} (char {pos})"),
            JsonErrorKind::IntTooLarge { digits } => write!(
                f,
                "Exceeds the limit ({INT_MAX_STR_DIGITS} digits) for integer string conversion: \
                 value has {digits} digits; use sys.set_int_max_str_digits() to increase the limit"
            ),
            JsonErrorKind::TooDeep { object } => write!(
                f,
                "maximum recursion depth exceeded while decoding a JSON {} from a unicode string",
                if *object { "object" } else { "array" }
            ),
            JsonErrorKind::Constant { name } => write!(f, "invalid JSON constant {name}"),
        }
    }
}

impl std::error::Error for JsonError {}

/// `json.loads(text)` for a `str`.
pub fn loads(text: &str) -> Result<Value, JsonError> {
    let points: Vec<u32> = text.chars().map(u32::from).collect();
    loads_points(&points, true)
}

/// json.loads of a Python string, preserving raw surrogate code points.
pub fn loads_py(text: &PyStr) -> Result<Value, JsonError> {
    let points: Vec<u32> = text.code_points().collect();
    loads_points(&points, true)
}

/// `json.loads(text, parse_constant=reject)`: `NaN`, `Infinity` and
/// `-Infinity` are not JSON and fail with [`JsonErrorKind::Constant`]. A
/// number literal that overflows to infinity (`1e999`) is still a float.
pub fn loads_py_without_constants(text: &PyStr) -> Result<Value, JsonError> {
    let points: Vec<u32> = text.code_points().collect();
    loads_points(&points, false)
}

fn loads_points(points: &[u32], constants: bool) -> Result<Value, JsonError> {
    let scanner = Scanner {
        s: points,
        constants,
    };
    if points.first() == Some(&0xFEFF) {
        return Err(scanner.decode_error("Unexpected UTF-8 BOM (decode using utf-8-sig)", 0));
    }
    let start = scanner.skip_ws(0);
    let (value, end) = match scanner.scan_value(start) {
        Ok(found) => found,
        Err(Fail::Stop(pos)) => return Err(scanner.decode_error("Expecting value", pos)),
        Err(Fail::Error(error)) => return Err(error),
    };
    let end = scanner.skip_ws(end);
    if end != points.len() {
        return Err(scanner.decode_error("Extra data", end));
    }
    Ok(value)
}

enum Fail {
    /// The C scanner's `StopIteration(idx)`: no value starts at `idx`.
    Stop(usize),
    Error(JsonError),
}

impl From<JsonError> for Fail {
    fn from(error: JsonError) -> Self {
        Fail::Error(error)
    }
}

enum Scanned {
    Value(Value, usize),
    Open { object: bool, after: usize },
}

enum Frame {
    Array(Vec<Value>),
    Object {
        pairs: Vec<(Key, Value)>,
        index: HashMap<PyStr, usize>,
        /// The name whose value is being scanned.
        key: PyStr,
    },
}

struct Scanner<'a> {
    s: &'a [u32],
    /// Whether `NaN`, `Infinity` and `-Infinity` parse (Python's default).
    constants: bool,
}

fn is_ws(point: u32) -> bool {
    matches!(point, 0x20 | 0x09 | 0x0A | 0x0D)
}

fn is_digit(point: Option<u32>) -> bool {
    point.is_some_and(|p| (u32::from('0')..=u32::from('9')).contains(&p))
}

impl Scanner<'_> {
    fn at(&self, idx: usize) -> Option<u32> {
        self.s.get(idx).copied()
    }

    fn is(&self, idx: usize, c: char) -> bool {
        self.at(idx) == Some(u32::from(c))
    }

    fn skip_ws(&self, mut idx: usize) -> usize {
        while self.at(idx).is_some_and(is_ws) {
            idx += 1;
        }
        idx
    }

    fn decode_error(&self, msg: &'static str, pos: usize) -> JsonError {
        let before = &self.s[..pos.min(self.s.len())];
        let newline = u32::from('\n');
        let lineno = before.iter().filter(|&&p| p == newline).count() + 1;
        let colno = match before.iter().rposition(|&p| p == newline) {
            Some(index) => pos - index,
            None => pos + 1,
        };
        JsonError {
            kind: JsonErrorKind::Decode {
                msg,
                pos,
                lineno,
                colno,
            },
        }
    }

    /// `parse_constant(name)`: the float, or the refusal.
    fn constant(&self, name: &'static str, value: f64) -> Result<(Value, usize), Fail> {
        if !self.constants {
            return Err(JsonError {
                kind: JsonErrorKind::Constant { name },
            }
            .into());
        }
        Ok((Value::Float(value), name.len()))
    }

    fn matches_word(&self, idx: usize, word: &str) -> bool {
        word.chars()
            .enumerate()
            .all(|(offset, c)| self.is(idx + offset, c))
    }

    /// `scan_once` for a whole document, iterative so that nesting up to
    /// [`MAX_NESTING`] fits any thread's stack.
    fn scan_value(&self, mut idx: usize) -> Result<(Value, usize), Fail> {
        let mut stack: Vec<Frame> = Vec::new();
        loop {
            let (mut value, mut next) = match self.scan_scalar_or_open(idx)? {
                Scanned::Value(value, next) => (value, next),
                Scanned::Open { object, after } => {
                    if stack.len() + 1 > MAX_NESTING {
                        return Err(JsonError {
                            kind: JsonErrorKind::TooDeep { object },
                        }
                        .into());
                    }
                    let start = self.skip_ws(after);
                    if object && self.is(start, '}') {
                        (Value::Dict(Vec::new()), start + 1)
                    } else if !object && self.is(start, ']') {
                        (Value::List(Vec::new()), start + 1)
                    } else if object {
                        let (key, value_at) = self.read_key(start)?;
                        stack.push(Frame::Object {
                            pairs: Vec::new(),
                            index: HashMap::new(),
                            key,
                        });
                        idx = value_at;
                        continue;
                    } else {
                        stack.push(Frame::Array(Vec::new()));
                        idx = start;
                        continue;
                    }
                }
            };
            // Attach the finished value to its container, closing every
            // container that ends here, until one expects another value.
            loop {
                let Some(frame) = stack.last_mut() else {
                    return Ok((value, next));
                };
                let close = match frame {
                    Frame::Array(items) => {
                        items.push(value);
                        ']'
                    }
                    Frame::Object { pairs, index, key } => {
                        let key = std::mem::take(key);
                        // A repeated key keeps its first position and takes
                        // the last value, as dict assignment does.
                        match index.get(&key) {
                            Some(&at) => pairs[at].1 = value,
                            None => {
                                index.insert(key.clone(), pairs.len());
                                pairs.push((Key::Str(key), value));
                            }
                        }
                        '}'
                    }
                };
                let at = self.skip_ws(next);
                if self.is(at, close) {
                    value = match stack.pop() {
                        Some(Frame::Array(items)) => Value::List(items),
                        Some(Frame::Object { pairs, .. }) => Value::Dict(pairs),
                        None => unreachable!("a frame was just inspected"),
                    };
                    next = at + 1;
                    continue;
                }
                if !self.is(at, ',') {
                    return Err(self.decode_error("Expecting ',' delimiter", at).into());
                }
                let after = self.skip_ws(at + 1);
                idx = match stack.last_mut() {
                    Some(Frame::Object { key, .. }) => {
                        let (next_key, value_at) = self.read_key(after)?;
                        *key = next_key;
                        value_at
                    }
                    _ => after,
                };
                break;
            }
        }
    }

    /// A property name, its `:` delimiter, and the whitespace around them;
    /// returns the name and where its value starts.
    fn read_key(&self, idx: usize) -> Result<(PyStr, usize), Fail> {
        if !self.is(idx, '"') {
            return Err(self
                .decode_error("Expecting property name enclosed in double quotes", idx)
                .into());
        }
        let (key, next) = self.scan_string(idx + 1)?;
        let colon = self.skip_ws(next);
        if !self.is(colon, ':') {
            return Err(self.decode_error("Expecting ':' delimiter", colon).into());
        }
        Ok((key, self.skip_ws(colon + 1)))
    }

    fn scan_scalar_or_open(&self, idx: usize) -> Result<Scanned, Fail> {
        let Some(first) = self.at(idx) else {
            return Err(Fail::Stop(idx));
        };
        let value = match char::from_u32(first) {
            Some('"') => {
                let (text, end) = self.scan_string(idx + 1)?;
                return Ok(Scanned::Value(Value::Str(text), end));
            }
            Some('{') => {
                return Ok(Scanned::Open {
                    object: true,
                    after: idx + 1,
                })
            }
            Some('[') => {
                return Ok(Scanned::Open {
                    object: false,
                    after: idx + 1,
                })
            }
            Some('n') if self.matches_word(idx, "null") => (Value::Null, 4),
            Some('t') if self.matches_word(idx, "true") => (Value::Bool(true), 4),
            Some('f') if self.matches_word(idx, "false") => (Value::Bool(false), 5),
            Some('N') if self.matches_word(idx, "NaN") => self.constant("NaN", f64::NAN)?,
            Some('I') if self.matches_word(idx, "Infinity") => {
                self.constant("Infinity", f64::INFINITY)?
            }
            Some('-') if self.matches_word(idx, "-Infinity") => {
                self.constant("-Infinity", f64::NEG_INFINITY)?
            }
            _ => {
                let (value, end) = self.match_number(idx)?;
                return Ok(Scanned::Value(value, end));
            }
        };
        Ok(Scanned::Value(value.0, idx + value.1))
    }

    fn match_number(&self, start: usize) -> Result<(Value, usize), Fail> {
        let mut idx = start;
        if self.is(idx, '-') {
            idx += 1;
            if idx >= self.s.len() {
                return Err(Fail::Stop(start));
            }
        }
        match self.at(idx).and_then(char::from_u32) {
            Some('1'..='9') => {
                idx += 1;
                while is_digit(self.at(idx)) {
                    idx += 1;
                }
            }
            Some('0') => idx += 1,
            _ => return Err(Fail::Stop(start)),
        }
        let mut is_float = false;
        if self.is(idx, '.') && is_digit(self.at(idx + 1)) {
            is_float = true;
            idx += 2;
            while is_digit(self.at(idx)) {
                idx += 1;
            }
        }
        // `idx < end_idx` in C: an exponent needs a following character.
        if idx + 1 < self.s.len() && (self.is(idx, 'e') || self.is(idx, 'E')) {
            let e_start = idx;
            idx += 1;
            if idx + 1 < self.s.len() && (self.is(idx, '-') || self.is(idx, '+')) {
                idx += 1;
            }
            while is_digit(self.at(idx)) {
                idx += 1;
            }
            if is_digit(self.at(idx - 1)) {
                is_float = true;
            } else {
                idx = e_start;
            }
        }
        // Every scanned character is ASCII.
        let literal: String = self.s[start..idx]
            .iter()
            .filter_map(|&p| char::from_u32(p))
            .collect();
        if is_float {
            let value = literal.parse::<f64>().map_err(|_| Fail::Stop(start))?;
            return Ok((Value::Float(value), idx));
        }
        let digits = literal.trim_start_matches('-').len();
        if digits > INT_MAX_STR_DIGITS {
            return Err(JsonError {
                kind: JsonErrorKind::IntTooLarge { digits },
            }
            .into());
        }
        let value = literal.parse::<BigInt>().map_err(|_| Fail::Stop(start))?;
        Ok((Value::Int(value), idx))
    }

    /// `scanstring_unicode`: `end` is the index after the opening quote.
    fn scan_string(&self, mut end: usize) -> Result<(PyStr, usize), Fail> {
        let begin = end - 1;
        let len = self.s.len();
        let mut chunks: Vec<u32> = Vec::new();
        loop {
            let mut next = end;
            let mut c = 0u32;
            while next < len {
                c = self.s[next];
                if c == u32::from('"') || c == u32::from('\\') {
                    break;
                }
                if c <= 0x1F {
                    return Err(self
                        .decode_error("Invalid control character at", next)
                        .into());
                }
                next += 1;
            }
            if next == len {
                c = 0;
            }
            if c != u32::from('"') && c != u32::from('\\') {
                return Err(self
                    .decode_error("Unterminated string starting at", begin)
                    .into());
            }
            chunks.extend_from_slice(&self.s[end..next]);
            next += 1;
            if c == u32::from('"') {
                end = next;
                break;
            }
            if next == len {
                return Err(self
                    .decode_error("Unterminated string starting at", begin)
                    .into());
            }
            let escape = self.s[next];
            if escape != u32::from('u') {
                end = next + 1;
                let decoded = match char::from_u32(escape) {
                    Some('"') => '"',
                    Some('\\') => '\\',
                    Some('/') => '/',
                    Some('b') => '\u{8}',
                    Some('f') => '\u{c}',
                    Some('n') => '\n',
                    Some('r') => '\r',
                    Some('t') => '\t',
                    _ => return Err(self.decode_error("Invalid \\escape", end - 2).into()),
                };
                chunks.push(u32::from(decoded));
                continue;
            }
            next += 1;
            end = next + 4;
            if end >= len {
                return Err(self.decode_error("Invalid \\uXXXX escape", next - 1).into());
            }
            let mut point = self.hex4(next, end)?;
            next = end;
            if (0xD800..=0xDBFF).contains(&point)
                && end + 6 < len
                && self.is(next, '\\')
                && self.is(next + 1, 'u')
            {
                let low = self.hex4(end + 2, end + 6)?;
                if (0xDC00..=0xDFFF).contains(&low) {
                    point = 0x10000 + (((point - 0xD800) << 10) | (low - 0xDC00));
                    end += 6;
                }
            }
            chunks.push(point);
        }
        // Code points from the input or from four hex digits, joined pairs
        // included, are all at most U+10FFFF.
        let text = PyStr::from_code_points(chunks).map_err(|_| Fail::Stop(begin))?;
        Ok((text, end))
    }

    /// Four hex digits ending before `end`; the error points four places
    /// before `end`, at the `u`, as the C scanner reports it.
    fn hex4(&self, from: usize, end: usize) -> Result<u32, Fail> {
        let mut point = 0u32;
        for idx in from..end {
            let digit = self
                .at(idx)
                .and_then(char::from_u32)
                .and_then(|c| c.to_digit(16));
            match digit {
                Some(digit) => point = (point << 4) | digit,
                None => {
                    return Err(self.decode_error("Invalid \\uXXXX escape", end - 5).into());
                }
            }
        }
        Ok(point)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn json_parser_preserves_raw_surrogate_identity() {
        let text = kassiber_pyjson::PyStr::from_code_points(vec![34, 0xd800, 0xdc00, 34]).unwrap();
        let kassiber_pyjson::Value::Str(raw) = super::loads_py(&text).unwrap() else {
            panic!("string")
        };
        assert_eq!(raw.code_points().collect::<Vec<_>>(), vec![0xd800, 0xdc00]);
        let kassiber_pyjson::Value::Str(escaped) = super::loads(r#""\ud800\udc00""#).unwrap()
        else {
            panic!("string")
        };
        assert_eq!(escaped.code_points().collect::<Vec<_>>(), vec![0x10000]);
    }
    use super::*;

    fn error(text: &str) -> String {
        loads(text).unwrap_err().to_string()
    }

    #[test]
    fn messages_match_cpython() {
        assert_eq!(error(""), "Expecting value: line 1 column 1 (char 0)");
        assert_eq!(
            error("{\"a\": }"),
            "Expecting value: line 1 column 7 (char 6)"
        );
        assert_eq!(error("[1,]"), "Expecting value: line 1 column 4 (char 3)");
        assert_eq!(
            error("{\"a\":1,}"),
            "Expecting property name enclosed in double quotes: line 1 column 8 (char 7)"
        );
        assert_eq!(error("{} x"), "Extra data: line 1 column 4 (char 3)");
        assert_eq!(
            error("\"abc"),
            "Unterminated string starting at: line 1 column 1 (char 0)"
        );
        assert_eq!(
            error("\"a\nb\""),
            "Invalid control character at: line 1 column 3 (char 2)"
        );
        assert_eq!(
            error("\"\\x\""),
            "Invalid \\escape: line 1 column 2 (char 1)"
        );
        assert_eq!(
            error("\"\\u12\""),
            "Invalid \\uXXXX escape: line 1 column 3 (char 2)"
        );
        assert_eq!(
            error("\"\\u1234"),
            "Invalid \\uXXXX escape: line 1 column 3 (char 2)"
        );
        assert_eq!(
            error("[\n1 2]"),
            "Expecting ',' delimiter: line 2 column 3 (char 4)"
        );
        assert_eq!(
            error("\u{feff}{}"),
            "Unexpected UTF-8 BOM (decode using utf-8-sig): line 1 column 1 (char 0)"
        );
        assert_eq!(error("-"), "Expecting value: line 1 column 1 (char 0)");
    }

    #[test]
    fn values_match_cpython() {
        let value =
            loads(r#"{"a": 1, "b": [true, null, -0.5e1], "a": "\ud83d\ude00\ud800"}"#).unwrap();
        let Value::Dict(pairs) = value else {
            panic!("expected a dict");
        };
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0, Key::from("a"));
        assert_eq!(
            pairs[0].1,
            Value::Str(PyStr::from_code_points([0x1F600, 0xD800]).unwrap())
        );
        assert_eq!(
            pairs[1].1,
            Value::List(vec![Value::Bool(true), Value::Null, Value::Float(-5.0)])
        );
        assert_eq!(loads("1E5").unwrap(), Value::Float(100000.0));
        assert_eq!(
            loads("1e").unwrap_err().to_string(),
            "Extra data: line 1 column 2 (char 1)"
        );
        assert_eq!(
            loads("01").unwrap_err().to_string(),
            "Extra data: line 1 column 2 (char 1)"
        );
        assert!(matches!(loads("NaN").unwrap(), Value::Float(f) if f.is_nan()));
        // `parse_constant=reject`: only the three constants are refused.
        for (text, name) in [
            ("[NaN]", "NaN"),
            ("{\"a\": Infinity}", "Infinity"),
            ("-Infinity", "-Infinity"),
        ] {
            let error = loads_py_without_constants(&PyStr::from(text)).unwrap_err();
            assert_eq!(error.kind, JsonErrorKind::Constant { name });
            assert_eq!(error.to_string(), format!("invalid JSON constant {name}"));
        }
        assert!(matches!(
            loads_py_without_constants(&PyStr::from("1e999")).unwrap(),
            Value::Float(f) if f.is_infinite()
        ));
        assert_eq!(
            loads_py_without_constants(&PyStr::from("[1, \"NaN\"]")).unwrap(),
            loads("[1, \"NaN\"]").unwrap()
        );
        assert!(!loads(&"9".repeat(4301)).unwrap_err().is_decode_error());
        let deep = "[".repeat(MAX_NESTING + 1);
        assert!(matches!(
            loads(&deep).unwrap_err().kind,
            JsonErrorKind::TooDeep { object: false }
        ));
    }
}

#[cfg(test)]
mod literal_surrogate_tests {
    use super::*;

    #[test]
    fn loads_py_preserves_literal_surrogates_without_escape_pairing() {
        let raw = PyStr::from_code_points([34, 0xd800, 0xdc00, 34]).unwrap();
        assert_eq!(
            loads_py(&raw).unwrap(),
            Value::Str(PyStr::from_code_points([0xd800, 0xdc00]).unwrap())
        );
        assert_eq!(
            loads(r#""\ud800\udc00""#).unwrap(),
            Value::Str(PyStr::from_code_points([0x10000]).unwrap())
        );
        let invalid = PyStr::from_code_points([34, 92, 0xd800, 34]).unwrap();
        assert!(loads_py(&invalid).is_err());
    }
}

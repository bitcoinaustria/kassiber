//! `json.dumps` and its options.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;

use num_bigint::{BigInt, Sign};

use crate::float::float_repr;
use crate::value::{Key, PyStr, Value};

/// CPython's default `sys.get_int_max_str_digits()`: `json.dumps` refuses an
/// int with more decimal digits than this (`ValueError`).
pub const INT_MAX_STR_DIGITS: usize = 4300;

/// Deepest nesting of lists and dicts [`dumps`] encodes. CPython stops at its
/// recursion limit (1000 frames, including its caller's), so every document
/// CPython encodes is within this bound.
pub const MAX_DEPTH: usize = 1000;

/// Lists at least this long are encoded in [`PARALLEL_CHUNKS`] concurrent
/// chunks; the text is the same as encoding them in turn.
const PARALLEL_ITEMS: usize = 1024;
const PARALLEL_CHUNKS: usize = 8;

/// `indent`: a number of spaces, or the text itself.
///
/// Python's `indent=0`, a negative number, and `""` all start each item on a
/// new line without indentation; give those as `Spaces(0)` or `Text("")`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Indent {
    /// `indent=<n>`
    Spaces(usize),
    /// `indent="<text>"`
    Text(String),
}

impl From<usize> for Indent {
    fn from(spaces: usize) -> Self {
        Indent::Spaces(spaces)
    }
}

impl From<&str> for Indent {
    fn from(text: &str) -> Self {
        Indent::Text(text.to_owned())
    }
}

/// The keyword arguments of `json.dumps` that change its output. The
/// defaults are Python's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// `ensure_ascii` (default `true`): escape every non-ASCII character.
    pub ensure_ascii: bool,
    /// `allow_nan` (default `true`): write `NaN`, `Infinity`, and
    /// `-Infinity`; otherwise refuse them.
    pub allow_nan: bool,
    /// `sort_keys` (default `false`).
    pub sort_keys: bool,
    /// `indent` (default `None`).
    pub indent: Option<Indent>,
    /// `separators` as `(item, key)`. `None` is Python's default:
    /// `(", ", ": ")`, or `(",", ": ")` with an indent.
    pub separators: Option<(String, String)>,
    /// `default=str`: write a `Decimal` as `str(Decimal)`. Without it a
    /// `Decimal` is refused, as `json.dumps` raises `TypeError`.
    pub default_str: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            ensure_ascii: true,
            allow_nan: true,
            sort_keys: false,
            indent: None,
            separators: None,
            default_str: false,
        }
    }
}

impl Options {
    /// Python's defaults: `json.dumps(obj)`.
    pub fn new() -> Self {
        Options::default()
    }

    /// `ensure_ascii=<on>`
    pub fn ensure_ascii(mut self, on: bool) -> Self {
        self.ensure_ascii = on;
        self
    }

    /// `allow_nan=<on>`
    pub fn allow_nan(mut self, on: bool) -> Self {
        self.allow_nan = on;
        self
    }

    /// `sort_keys=<on>`
    pub fn sort_keys(mut self, on: bool) -> Self {
        self.sort_keys = on;
        self
    }

    /// `indent=<indent>`
    pub fn indent(mut self, indent: impl Into<Indent>) -> Self {
        self.indent = Some(indent.into());
        self
    }

    /// `separators=(<item>, <key>)`
    pub fn separators(mut self, item: &str, key: &str) -> Self {
        self.separators = Some((item.to_owned(), key.to_owned()));
        self
    }

    /// `separators=(",", ":")`
    pub fn compact(self) -> Self {
        self.separators(",", ":")
    }

    /// `default=str` when `on`.
    pub fn default_str(mut self, on: bool) -> Self {
        self.default_str = on;
        self
    }
}

/// Why [`dumps`] refused a value. Each variant but [`Error::NanKeyOrder`],
/// [`Error::TooDeep`], and [`Error::Surrogate`] is the exception `json.dumps`
/// raises for the same value; when a value has several problems, the first
/// in Python's encoding order is reported, as Python would raise it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// `ValueError: Out of range float values are not JSON compliant`: an
    /// infinity or NaN, as a value or a key, with `allow_nan=False`.
    OutOfRangeFloat,
    /// `TypeError: Object of type Decimal is not JSON serializable`: a
    /// `Decimal` without `default=str`.
    NotSerializable,
    /// `TypeError: Object of type bytes is not JSON serializable`.
    BytesNotSerializable,
    /// `TypeError: '<' not supported between instances of ...`: `sort_keys`
    /// over keys Python cannot order (strings with numbers, or `None` with
    /// any other key).
    UnorderableKeys,
    /// `sort_keys` over two or more numeric keys, one of them NaN. Python
    /// sorts them with an inconsistent comparison; that order is refused
    /// here rather than reproduced.
    NanKeyOrder,
    /// `ValueError: Exceeds the limit (4300 digits) for integer string
    /// conversion`: an int, as a value or a key, with more than
    /// [`INT_MAX_STR_DIGITS`] digits.
    IntTooLarge,
    /// Lists and dicts nested deeper than [`MAX_DEPTH`]; CPython raises
    /// `RecursionError` at about that depth.
    TooDeep,
    /// With `ensure_ascii=False`, the output would contain a surrogate. The
    /// Python result holds it, but no UTF-8 encoder accepts it, so encoding
    /// it for a hash, a file, or SQLite raises `UnicodeEncodeError`.
    Surrogate,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::OutOfRangeFloat => "out of range float values are not JSON compliant",
            Error::NotSerializable => "object of type Decimal is not JSON serializable",
            Error::BytesNotSerializable => "Object of type bytes is not JSON serializable",
            Error::UnorderableKeys => "sort_keys cannot order these keys",
            Error::NanKeyOrder => "sort_keys over a NaN key is not reproduced",
            Error::IntTooLarge => "int exceeds the limit (4300 digits) for string conversion",
            Error::TooDeep => "lists and dicts are nested too deeply",
            Error::Surrogate => "output with ensure_ascii=False would contain a surrogate",
        })
    }
}

impl std::error::Error for Error {}

/// `json.dumps(value, **options)`, byte for byte.
///
/// The result has no trailing newline. With `ensure_ascii` (the default) it
/// is ASCII.
pub fn dumps(value: &Value, options: &Options) -> Result<String, Error> {
    let indent = options.indent.as_ref().map(|indent| match indent {
        Indent::Spaces(n) => " ".repeat(*n),
        Indent::Text(text) => text.clone(),
    });
    let (item_separator, key_separator) = match &options.separators {
        Some((item, key)) => (item.as_str(), key.as_str()),
        None if indent.is_some() => (",", ": "),
        None => (", ", ": "),
    };
    let mut encoder = Encoder {
        options,
        indent: indent.as_deref(),
        item_separator,
        key_separator,
        out: String::new(),
        surrogate: false,
    };
    encoder.value(value, 0)?;
    if encoder.surrogate {
        return Err(Error::Surrogate);
    }
    Ok(encoder.out)
}

/// Appends the text [`dumps`] writes for `value` when it sits `level`
/// containers deep inside a larger document, depth limit included.
///
/// A [`Error::Surrogate`] is reported at the end of this value; in a larger
/// document Python would report a later error first, so callers combining
/// parts must order such failures themselves. With `ensure_ascii` it cannot
/// occur. On an error `out` holds partial output.
pub fn dumps_into(
    value: &Value,
    options: &Options,
    level: usize,
    out: &mut String,
) -> Result<(), Error> {
    let indent = options.indent.as_ref().map(|indent| match indent {
        Indent::Spaces(n) => " ".repeat(*n),
        Indent::Text(text) => text.clone(),
    });
    let (item_separator, key_separator) = match &options.separators {
        Some((item, key)) => (item.as_str(), key.as_str()),
        None if indent.is_some() => (",", ": "),
        None => (", ", ": "),
    };
    let mut encoder = Encoder {
        options,
        indent: indent.as_deref(),
        item_separator,
        key_separator,
        out: std::mem::take(out),
        surrogate: false,
    };
    let result = encoder.value(value, level);
    *out = encoder.out;
    result?;
    if encoder.surrogate {
        return Err(Error::Surrogate);
    }
    Ok(())
}

/// Appends a JSON string as [`dumps`] writes `text` under `options`.
pub fn dumps_str_into(text: &str, options: &Options, out: &mut String) {
    let mut encoder = Encoder {
        options,
        indent: None,
        item_separator: "",
        key_separator: "",
        out: std::mem::take(out),
        surrogate: false,
    };
    encoder.string_bytes(text.as_bytes());
    *out = encoder.out;
}

/// Bytes written as themselves: printable ASCII other than '"' and '\\'.
static PLAIN: [bool; 256] = {
    let mut plain = [false; 256];
    let mut byte = 0x20;
    while byte <= 0x7E {
        plain[byte] = byte != b'"' as usize && byte != b'\\' as usize;
        byte += 1;
    }
    plain
};

struct Encoder<'a> {
    options: &'a Options,
    indent: Option<&'a str>,
    item_separator: &'a str,
    key_separator: &'a str,
    out: String,
    /// A surrogate was dropped from the output; reported only if nothing
    /// Python raises comes later.
    surrogate: bool,
}

impl Encoder<'_> {
    fn value(&mut self, value: &Value, level: usize) -> Result<(), Error> {
        match value {
            Value::Null => self.out.push_str("null"),
            Value::Bool(true) => self.out.push_str("true"),
            Value::Bool(false) => self.out.push_str("false"),
            Value::Int(n) => {
                let text = int_text(n)?;
                self.out.push_str(&text);
            }
            Value::Float(x) => {
                let text = self.float_text(*x)?;
                self.out.push_str(&text);
            }
            Value::Str(s) => self.string(s),
            Value::Bytes(bytes) => {
                if !self.options.default_str {
                    return Err(Error::BytesNotSerializable);
                }
                self.string(&PyStr::from(crate::bytes_repr(bytes)));
            }
            Value::Decimal(d) => {
                if !self.options.default_str {
                    return Err(Error::NotSerializable);
                }
                self.string(&PyStr::from(d.to_string()));
            }
            Value::List(items) => self.list(items, level)?,
            Value::Dict(items) => self.dict(items, level)?,
        }
        Ok(())
    }

    fn list(&mut self, items: &[Value], level: usize) -> Result<(), Error> {
        if items.is_empty() {
            self.out.push_str("[]");
            return Ok(());
        }
        if level >= MAX_DEPTH {
            return Err(Error::TooDeep);
        }
        self.out.push('[');
        self.open(level);
        if items.len() >= PARALLEL_ITEMS {
            self.items_concurrently(items, level)?;
        } else {
            self.items(items, level)?;
        }
        self.close(level);
        self.out.push(']');
        Ok(())
    }

    /// The list's items, separated, at `level`.
    fn items(&mut self, items: &[Value], level: usize) -> Result<(), Error> {
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                self.separate(level);
            }
            self.value(item, level + 1)?;
        }
        Ok(())
    }

    /// [`Self::items`] for a long list: contiguous chunks are encoded on
    /// scoped threads (or in turn, when no thread is available) and joined in
    /// order. The first chunk with an error reports the error sequential
    /// encoding reaches first; dropped surrogates are combined.
    fn items_concurrently(&mut self, items: &[Value], level: usize) -> Result<(), Error> {
        let size = items.len().div_ceil(PARALLEL_CHUNKS);
        let encode = |chunk: &[Value]| {
            let mut encoder = Encoder {
                options: self.options,
                indent: self.indent,
                item_separator: self.item_separator,
                key_separator: self.key_separator,
                out: String::new(),
                surrogate: false,
            };
            encoder
                .items(chunk, level)
                .map(|()| (encoder.out, encoder.surrogate))
        };
        let parts: Vec<_> = std::thread::scope(|scope| {
            let workers: Vec<_> = items
                .chunks(size)
                .map(|chunk| {
                    let encode = &encode;
                    (
                        chunk,
                        std::thread::Builder::new()
                            .spawn_scoped(scope, move || encode(chunk))
                            .ok(),
                    )
                })
                .collect();
            workers
                .into_iter()
                .map(|(chunk, worker)| match worker {
                    Some(worker) => worker
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                    None => encode(chunk),
                })
                .collect()
        });
        for (index, part) in parts.into_iter().enumerate() {
            let (text, surrogate) = part?;
            if index > 0 {
                self.separate(level);
            }
            self.out.push_str(&text);
            self.surrogate |= surrogate;
        }
        Ok(())
    }

    fn dict(&mut self, items: &[(Key, Value)], level: usize) -> Result<(), Error> {
        if items.is_empty() {
            self.out.push_str("{}");
            return Ok(());
        }
        if level >= MAX_DEPTH {
            return Err(Error::TooDeep);
        }
        // Python sorts the items before writing any of them.
        let mut ordered: Vec<&(Key, Value)> = items.iter().collect();
        if self.options.sort_keys {
            sort_items(&mut ordered)?;
        }
        self.out.push('{');
        self.open(level);
        for (index, (key, value)) in ordered.into_iter().enumerate() {
            if index > 0 {
                self.separate(level);
            }
            self.key(key)?;
            self.out.push_str(self.key_separator);
            self.value(value, level + 1)?;
        }
        self.close(level);
        self.out.push('}');
        Ok(())
    }

    /// After an opening bracket: the first item's line and indentation.
    fn open(&mut self, level: usize) {
        if let Some(indent) = self.indent {
            self.out.push('\n');
            self.out.extend(core::iter::repeat_n(indent, level + 1));
        }
    }

    /// Between items: the item separator, then the next line.
    fn separate(&mut self, level: usize) {
        self.out.push_str(self.item_separator);
        self.open(level);
    }

    /// Before a closing bracket: back to the container's indentation.
    fn close(&mut self, level: usize) {
        if let Some(indent) = self.indent {
            self.out.push('\n');
            self.out.extend(core::iter::repeat_n(indent, level));
        }
    }

    fn key(&mut self, key: &Key) -> Result<(), Error> {
        let text = match key {
            Key::Str(s) => {
                self.string(s);
                return Ok(());
            }
            Key::Float(x) => self.float_text(*x)?,
            Key::Bool(true) => "true".to_owned(),
            Key::Bool(false) => "false".to_owned(),
            Key::Null => "null".to_owned(),
            Key::Int(n) => int_text(n)?,
        };
        // Digits, letters, '.', '+', and '-': nothing to escape.
        self.out.push('"');
        self.out.push_str(&text);
        self.out.push('"');
        Ok(())
    }

    fn float_text(&self, x: f64) -> Result<String, Error> {
        if x.is_finite() {
            return Ok(float_repr(x));
        }
        if !self.options.allow_nan {
            return Err(Error::OutOfRangeFloat);
        }
        Ok(if x.is_nan() {
            "NaN"
        } else if x > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        }
        .to_owned())
    }

    /// A JSON string as `encode_basestring_ascii` or, without
    /// `ensure_ascii`, `encode_basestring` writes it.
    fn string(&mut self, s: &PyStr) {
        self.string_bytes(s.bytes());
    }

    /// [`Self::string`] over generalized UTF-8 bytes.
    fn string_bytes(&mut self, bytes: &[u8]) {
        self.out.reserve(bytes.len() + 2);
        self.out.push('"');
        // Printable ASCII other than '"' and '\\' is written as is under both
        // options; copy such runs whole and encode everything else per point.
        let mut at = 0;
        while at < bytes.len() {
            let start = at;
            while at < bytes.len() && PLAIN[usize::from(bytes[at])] {
                at += 1;
            }
            if at > start {
                self.out
                    .push_str(core::str::from_utf8(&bytes[start..at]).expect("ASCII run"));
            }
            if at < bytes.len() {
                // The two escaped ASCII characters need no code point decoding.
                match bytes[at] {
                    b'"' => {
                        self.out.push_str("\\\"");
                        at += 1;
                    }
                    b'\\' => {
                        self.out.push_str("\\\\");
                        at += 1;
                    }
                    _ => {
                        let mut points = PyStr::code_points_of(&bytes[at..]);
                        let point = points.next().expect("whole code point");
                        at = bytes.len() - points.rest_len();
                        self.point(point);
                    }
                }
            }
        }
        self.out.push('"');
    }

    /// One code point as either encoder writes it.
    fn point(&mut self, point: u32) {
        match point {
            0x22 => self.out.push_str("\\\""),
            0x5C => self.out.push_str("\\\\"),
            0x08 => self.out.push_str("\\b"),
            0x0C => self.out.push_str("\\f"),
            0x0A => self.out.push_str("\\n"),
            0x0D => self.out.push_str("\\r"),
            0x09 => self.out.push_str("\\t"),
            0x00..=0x1F => self.escape(point),
            0x20..=0x7E => self.out.push(point as u8 as char),
            _ if self.options.ensure_ascii => {
                if point >= 0x1_0000 {
                    let offset = point - 0x1_0000;
                    self.escape(0xD800 | (offset >> 10));
                    self.escape(0xDC00 | (offset & 0x3FF));
                } else {
                    self.escape(point);
                }
            }
            _ => match char::from_u32(point) {
                Some(c) => self.out.push(c),
                None => self.surrogate = true,
            },
        }
    }

    /// `\uXXXX` with lowercase hex digits, as CPython writes it.
    fn escape(&mut self, unit: u32) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        self.out.push_str("\\u");
        for shift in [12, 8, 4, 0] {
            self.out.push(HEX[((unit >> shift) & 0xF) as usize] as char);
        }
    }
}

/// `int.__repr__`, refusing what CPython's digit limit refuses.
fn int_text(n: &BigInt) -> Result<String, Error> {
    // Any int of 14,300 bits or more has over 4,300 digits; skip converting it.
    if n.bits() >= 14_300 {
        return Err(Error::IntTooLarge);
    }
    let text = n.to_string();
    let digits = text.len() - usize::from(n.sign() == Sign::Minus);
    if digits > INT_MAX_STR_DIGITS {
        return Err(Error::IntTooLarge);
    }
    Ok(text)
}

/// Python's `sorted(dct.items())` for keys a dict can hold together:
/// strings by code point, numbers (ints, floats, and bools) by exact value.
fn sort_items(items: &mut [&(Key, Value)]) -> Result<(), Error> {
    if items.len() < 2 {
        return Ok(());
    }
    if items.iter().all(|(key, _)| matches!(key, Key::Str(_))) {
        items.sort_by(|(a, _), (b, _)| match (a, b) {
            (Key::Str(a), Key::Str(b)) => a.cmp(b),
            _ => Ordering::Equal,
        });
        return Ok(());
    }
    let numeric = |key: &Key| matches!(key, Key::Int(_) | Key::Float(_) | Key::Bool(_));
    if !items.iter().all(|(key, _)| numeric(key)) {
        return Err(Error::UnorderableKeys);
    }
    if items
        .iter()
        .any(|(key, _)| matches!(key, Key::Float(x) if x.is_nan()))
    {
        return Err(Error::NanKeyOrder);
    }
    items.sort_by(|(a, _), (b, _)| compare_numbers(a, b));
    Ok(())
}

/// Python's exact comparison of two numeric keys, none of them NaN.
fn compare_numbers(a: &Key, b: &Key) -> Ordering {
    match (a, b) {
        (Key::Float(x), Key::Float(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Key::Float(x), other) => compare_int_float(&int_value(other), *x).reverse(),
        (other, Key::Float(y)) => compare_int_float(&int_value(other), *y),
        (a, b) => int_value(a).cmp(&int_value(b)),
    }
}

/// The int an int or bool key stands for; sort_items admits no other here.
fn int_value(key: &Key) -> Cow<'_, BigInt> {
    match key {
        Key::Int(n) => Cow::Borrowed(n),
        Key::Bool(b) => Cow::Owned(BigInt::from(u8::from(*b))),
        Key::Float(_) | Key::Str(_) | Key::Null => Cow::Owned(BigInt::default()),
    }
}

/// Compares an int with a finite or infinite float exactly, as Python does.
fn compare_int_float(n: &BigInt, x: f64) -> Ordering {
    if x.is_infinite() {
        return if x > 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    // x = ±m * 2^e exactly.
    let bits = x.to_bits();
    let biased = ((bits >> 52) & 0x7FF) as i64;
    let fraction = bits & ((1 << 52) - 1);
    let (m, e) = if biased == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), biased - 1075)
    };
    let mut scaled = BigInt::from(m);
    if x.is_sign_negative() {
        scaled = -scaled;
    }
    if e >= 0 {
        n.cmp(&(scaled << e as usize))
    } else {
        (n << e.unsigned_abs() as usize).cmp(&scaled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(items: Vec<(Key, Value)>) -> Value {
        Value::Dict(items)
    }

    #[test]
    fn writes_the_default_and_compact_layouts() {
        let doc = dict(vec![
            (
                "b".into(),
                Value::List(vec![1.into(), Value::Null, true.into()]),
            ),
            ("a".into(), 1.5.into()),
        ]);
        assert_eq!(
            dumps(&doc, &Options::new()).unwrap(),
            r#"{"b": [1, null, true], "a": 1.5}"#
        );
        assert_eq!(
            dumps(&doc, &Options::new().compact().sort_keys(true)).unwrap(),
            r#"{"a":1.5,"b":[1,null,true]}"#
        );
        assert_eq!(
            dumps(&doc, &Options::new().indent(2)).unwrap(),
            "{\n  \"b\": [\n    1,\n    null,\n    true\n  ],\n  \"a\": 1.5\n}"
        );
    }

    #[test]
    fn sorts_numeric_keys_by_exact_value() {
        let big = BigInt::from(1u64 << 53) + 1;
        let doc = dict(vec![
            (Key::Int(big), "i".into()),
            (Key::Float(9007199254740992.0), "f".into()),
            (Key::Bool(true), "t".into()),
            (Key::Float(-0.0), "z".into()),
        ]);
        assert_eq!(
            dumps(&doc, &Options::new().sort_keys(true)).unwrap(),
            r#"{"-0.0": "z", "true": "t", "9007199254740992.0": "f", "9007199254740993": "i"}"#
        );
    }

    #[test]
    fn refuses_what_python_refuses_in_its_order() {
        let opts = Options::new().allow_nan(false);
        let nan_first = Value::List(vec![f64::NAN.into(), Value::Decimal(1.into())]);
        assert_eq!(dumps(&nan_first, &opts), Err(Error::OutOfRangeFloat));
        let decimal_first = Value::List(vec![Value::Decimal(1.into()), f64::NAN.into()]);
        assert_eq!(dumps(&decimal_first, &opts), Err(Error::NotSerializable));
        let mixed = dict(vec![("a".into(), 1.into()), (Key::Int(1.into()), 2.into())]);
        assert_eq!(
            dumps(&mixed, &Options::new().sort_keys(true)),
            Err(Error::UnorderableKeys)
        );
        assert!(dumps(&mixed, &Options::new()).is_ok());
        let nan_keys = dict(vec![
            (Key::Float(f64::NAN), 1.into()),
            (Key::Int(1.into()), 2.into()),
        ]);
        assert_eq!(
            dumps(&nan_keys, &Options::new().sort_keys(true)),
            Err(Error::NanKeyOrder)
        );
        let lone = Value::Str(PyStr::from_code_points([0xD800]).unwrap());
        assert_eq!(dumps(&lone, &Options::new()).unwrap(), r#""\ud800""#);
        assert_eq!(
            dumps(&lone, &Options::new().ensure_ascii(false)),
            Err(Error::Surrogate)
        );
        let then_nan = Value::List(vec![lone, f64::NAN.into()]);
        assert_eq!(
            dumps(
                &then_nan,
                &Options::new().ensure_ascii(false).allow_nan(false)
            ),
            Err(Error::OutOfRangeFloat)
        );
    }

    #[test]
    fn applies_the_int_digit_limit() {
        let limit = BigInt::from(10).pow(4299);
        assert_eq!(
            dumps(&Value::Int(limit.clone()), &Options::new())
                .unwrap()
                .len(),
            4300
        );
        assert_eq!(
            dumps(&Value::Int(-limit.clone()), &Options::new())
                .unwrap()
                .len(),
            4301
        );
        assert_eq!(
            dumps(&Value::Int(limit * 10), &Options::new()),
            Err(Error::IntTooLarge)
        );
    }

    #[test]
    fn long_lists_encode_as_their_items_in_turn() {
        let item = |i: usize| match i % 4 {
            0 => Value::Int(i.into()),
            1 => Value::Str(format!("é{i}\u{1f600}").as_str().into()),
            2 => dict(vec![("k".into(), Value::Float(i as f64 / 3.0))]),
            _ => Value::List(vec![Value::Null, Value::Bool(i % 8 == 3)]),
        };
        let items: Vec<Value> = (0..PARALLEL_ITEMS * 2 + 7).map(item).collect();
        for (options, open, separator, close) in [
            (Options::new(), "[", ", ", "]"),
            (Options::new().compact().sort_keys(true), "[", ",", "]"),
            (
                Options::new().indent(2).ensure_ascii(false),
                "[\n  ",
                ",\n  ",
                "\n]",
            ),
        ] {
            // Each item alone in a one-item list is encoded at the same level.
            let texts: Vec<String> = items
                .iter()
                .map(|item| {
                    let text = dumps(&Value::List(vec![item.clone()]), &options).unwrap();
                    text[open.len()..text.len() - close.len()].to_owned()
                })
                .collect();
            let expected = format!("{open}{}{close}", texts.join(separator));
            assert_eq!(
                dumps(&Value::List(items.clone()), &options).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn long_lists_report_the_first_error_in_order() {
        let mut items: Vec<Value> = (0..PARALLEL_ITEMS * 3)
            .map(|i| Value::Int(i.into()))
            .collect();
        items[PARALLEL_ITEMS + 5] = Value::Float(f64::NAN);
        items[PARALLEL_ITEMS * 2 + 9] = Value::Decimal("1.5".parse().unwrap());
        let strict = Options::new().allow_nan(false);
        assert_eq!(
            dumps(&Value::List(items.clone()), &strict),
            Err(Error::OutOfRangeFloat)
        );
        items[PARALLEL_ITEMS + 5] = Value::Int(0.into());
        assert_eq!(
            dumps(&Value::List(items.clone()), &strict),
            Err(Error::NotSerializable)
        );
        items[3] = Value::Str(PyStr::from_code_points([0xd800]).unwrap());
        let unicode = Options::new().ensure_ascii(false).default_str(true);
        assert_eq!(
            dumps(&Value::List(items.clone()), &unicode),
            Err(Error::Surrogate)
        );
        items[PARALLEL_ITEMS * 2 + 9] = Value::Decimal("1.5".parse().unwrap());
        assert_eq!(
            dumps(&Value::List(items), &Options::new().ensure_ascii(false)),
            Err(Error::NotSerializable)
        );
    }

    #[test]
    fn refuses_nesting_beyond_the_depth_limit() {
        let mut deep = Value::List(vec![1.into()]);
        for _ in 1..MAX_DEPTH {
            deep = Value::List(vec![deep]);
        }
        assert!(dumps(&deep, &Options::new()).is_ok());
        let deeper = Value::List(vec![deep]);
        assert_eq!(dumps(&deeper, &Options::new()), Err(Error::TooDeep));
    }

    #[test]
    fn writes_decimals_as_their_str_with_default_str() {
        let doc = Value::List(vec![
            Value::Decimal("1E+3".parse().unwrap()),
            Value::Decimal("-0.000001".parse().unwrap()),
            Value::Decimal("1E-7".parse().unwrap()),
        ]);
        assert_eq!(
            dumps(&doc, &Options::new().default_str(true)).unwrap(),
            r#"["1E+3", "-0.000001", "1E-7"]"#
        );
    }
}

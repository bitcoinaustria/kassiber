//! The Python values `json.dumps` accepts, in Python's own shapes.

use core::fmt;

use kassiber_pydecimal::Decimal;
use num_bigint::BigInt;

/// A Python `str`: a sequence of Unicode code points.
///
/// Unlike Rust's `str` it may hold surrogates (U+D800 to U+DFFF), alone or
/// in pairs, as Python strings can (`json.loads('"\\ud800"')`). A pair of
/// surrogate code points stays two code points, distinct from the astral
/// character they would encode in UTF-16, as in Python.
///
/// The bytes are generalized UTF-8: every code point, surrogates included,
/// in its UTF-8 form. A string without surrogates is plain UTF-8, and byte
/// order is code point order, which is how Python compares strings.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PyStr(Vec<u8>);

/// A code point above U+10FFFF, which no Python string can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidCodePoint(pub u32);

impl fmt::Display for InvalidCodePoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "code point {:#x} is above U+10FFFF", self.0)
    }
}

impl std::error::Error for InvalidCodePoint {}

impl PyStr {
    /// The empty string.
    pub fn new() -> Self {
        PyStr(Vec::new())
    }

    /// The string of these code points; surrogates are allowed.
    pub fn from_code_points<I>(points: I) -> Result<Self, InvalidCodePoint>
    where
        I: IntoIterator<Item = u32>,
    {
        let mut bytes = Vec::new();
        for point in points {
            push_code_point(&mut bytes, point)?;
        }
        Ok(PyStr(bytes))
    }

    /// The string as UTF-8, or `None` when it holds a surrogate.
    pub fn as_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.0).ok()
    }

    /// Whether the string holds a surrogate code point.
    pub fn has_surrogates(&self) -> bool {
        self.as_str().is_none()
    }

    /// The code points, in order.
    pub fn code_points(&self) -> CodePoints<'_> {
        CodePoints { bytes: &self.0 }
    }

    /// The generalized UTF-8 bytes.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// UTF-8 with surrogatepass: raw surrogate points stay distinct from
    /// escaped JSON surrogate pairs. Used only by native compatibility codecs.
    pub fn as_surrogatepass_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Whether this is exactly `text`, without allocating.
    pub fn is(&self, text: &str) -> bool {
        self.0 == text.as_bytes()
    }

    /// The code points of whole generalized UTF-8 sequences.
    pub(crate) fn code_points_of(bytes: &[u8]) -> CodePoints<'_> {
        CodePoints { bytes }
    }
}

fn push_code_point(bytes: &mut Vec<u8>, point: u32) -> Result<(), InvalidCodePoint> {
    // Truncation to u8 is intended: every value is masked or range-checked.
    match point {
        0..=0x7F => bytes.push(point as u8),
        0x80..=0x7FF => {
            bytes.push(0xC0 | (point >> 6) as u8);
            bytes.push(0x80 | (point & 0x3F) as u8);
        }
        0x800..=0xFFFF => {
            bytes.push(0xE0 | (point >> 12) as u8);
            bytes.push(0x80 | ((point >> 6) & 0x3F) as u8);
            bytes.push(0x80 | (point & 0x3F) as u8);
        }
        0x1_0000..=0x10_FFFF => {
            bytes.push(0xF0 | (point >> 18) as u8);
            bytes.push(0x80 | ((point >> 12) & 0x3F) as u8);
            bytes.push(0x80 | ((point >> 6) & 0x3F) as u8);
            bytes.push(0x80 | (point & 0x3F) as u8);
        }
        _ => return Err(InvalidCodePoint(point)),
    }
    Ok(())
}

/// The code points of a [`PyStr`].
#[derive(Clone, Debug)]
pub struct CodePoints<'a> {
    bytes: &'a [u8],
}

impl CodePoints<'_> {
    /// Bytes not yet decoded.
    pub(crate) fn rest_len(&self) -> usize {
        self.bytes.len()
    }
}

impl Iterator for CodePoints<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        let (&lead, rest) = self.bytes.split_first()?;
        // PyStr only ever holds whole generalized UTF-8 sequences.
        let (len, initial) = match lead {
            0x00..=0x7F => (1, u32::from(lead)),
            0xC0..=0xDF => (2, u32::from(lead & 0x1F)),
            0xE0..=0xEF => (3, u32::from(lead & 0x0F)),
            _ => (4, u32::from(lead & 0x07)),
        };
        let continuation = rest.get(..len - 1)?;
        self.bytes = &rest[len - 1..];
        Some(continuation.iter().fold(initial, |point, &byte| {
            (point << 6) | u32::from(byte & 0x3F)
        }))
    }
}

impl From<&str> for PyStr {
    fn from(text: &str) -> Self {
        PyStr(text.as_bytes().to_vec())
    }
}

impl From<String> for PyStr {
    fn from(text: String) -> Self {
        PyStr(text.into_bytes())
    }
}

impl From<Box<str>> for PyStr {
    fn from(text: Box<str>) -> Self {
        String::from(text).into()
    }
}

impl From<char> for PyStr {
    fn from(c: char) -> Self {
        PyStr(c.to_string().into_bytes())
    }
}

impl fmt::Debug for PyStr {
    /// Writes the string quoted, with surrogates as `\u{d800}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"")?;
        for point in self.code_points() {
            match char::from_u32(point) {
                Some(c) => write!(f, "{}", c.escape_debug())?,
                None => write!(f, "\\u{{{point:x}}}")?,
            }
        }
        f.write_str("\"")
    }
}

/// A Python object `json.dumps` serializes: the JSON-native types, plus
/// `Decimal`, which needs `default=str`.
///
/// Lists and dicts keep their order; a dict is a list of pairs because
/// Python dicts keep insertion order and `sort_keys` is an encoding option.
/// Build a dict as Python would hold it: keys that compare equal in Python
/// (`1`, `1.0`, and `True`) cannot all be present at once.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `None`
    Null,
    /// `True` or `False`
    Bool(bool),
    /// An `int` of any size.
    Int(BigInt),
    /// A `float`, including infinities and NaN.
    Float(f64),
    /// A `str`.
    Str(PyStr),
    /// Raw Python `bytes`, including SQLite BLOBs. Never implicitly text.
    Bytes(Vec<u8>),
    /// A `decimal.Decimal`. JSON has no decimal type: `default=str` writes it
    /// as the string `str(Decimal)`, and without it `json.dumps` refuses it.
    Decimal(Decimal),
    /// A `list` or `tuple`.
    List(Vec<Value>),
    /// A `dict`, in insertion order.
    Dict(Vec<(Key, Value)>),
}

/// CPython's bytes repr (also its str), with quote selection and ASCII escapes.
pub fn bytes_repr(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let quote = if bytes.contains(&b'\'') && !bytes.contains(&b'"') {
        b'"'
    } else {
        b'\''
    };
    let mut out = String::from("b");
    out.push(char::from(quote));
    for &byte in bytes {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'\t' => out.push_str("\\t"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b if b == quote => {
                out.push('\\');
                out.push(char::from(b));
            }
            32..=126 => out.push(char::from(byte)),
            b => write!(out, "\\x{b:02x}").unwrap(),
        }
    }
    out.push(char::from(quote));
    out
}

/// A dict key `json.dumps` accepts. Non-string keys become JSON strings:
/// `"null"`, `"true"`, the int's digits, or the float's `repr`.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    /// `None`
    Null,
    /// `True` or `False`
    Bool(bool),
    /// An `int` of any size.
    Int(BigInt),
    /// A `float`, including infinities and NaN.
    Float(f64),
    /// A `str`.
    Str(PyStr),
}

macro_rules! from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for Value {
            fn from(n: $t) -> Self {
                Value::Int(BigInt::from(n))
            }
        }

        impl From<$t> for Key {
            fn from(n: $t) -> Self {
                Key::Int(BigInt::from(n))
            }
        }
    )*};
}

from_int!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

macro_rules! from_scalar {
    ($($t:ty => $variant:ident),*) => {$(
        impl From<$t> for Value {
            fn from(v: $t) -> Self {
                Value::$variant(v.into())
            }
        }

        impl From<$t> for Key {
            fn from(v: $t) -> Self {
                Key::$variant(v.into())
            }
        }
    )*};
}

from_scalar!(
    bool => Bool,
    BigInt => Int,
    f64 => Float,
    &str => Str,
    String => Str,
    Box<str> => Str,
    PyStr => Str
);

impl From<Decimal> for Value {
    fn from(d: Decimal) -> Self {
        Value::Decimal(d)
    }
}

impl From<Vec<Value>> for Value {
    fn from(items: Vec<Value>) -> Self {
        Value::List(items)
    }
}

impl From<Vec<(Key, Value)>> for Value {
    fn from(items: Vec<(Key, Value)>) -> Self {
        Value::Dict(items)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    /// `None` becomes `null`.
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_points_round_trip_including_surrogates() {
        let points = [
            0x41, 0xE9, 0xD800, 0xDC00, 0xD83D, 0xDE00, 0xFFFF, 0x1F600, 0x10FFFF,
        ];
        let s = PyStr::from_code_points(points).unwrap();
        assert_eq!(s.code_points().collect::<Vec<_>>(), points);
        assert!(s.has_surrogates());
        assert_eq!(
            PyStr::from_code_points([0x110000]),
            Err(InvalidCodePoint(0x110000))
        );
    }

    #[test]
    fn plain_strings_are_utf8_and_order_by_code_point() {
        let s = PyStr::from("caf\u{e9} \u{1F600}");
        assert_eq!(s.as_str(), Some("caf\u{e9} \u{1F600}"));
        let surrogate = PyStr::from_code_points([0xD800]).unwrap();
        // U+D7FF < U+D800 < U+E000 < U+1F600, as Python compares them.
        assert!(PyStr::from("\u{D7FF}") < surrogate);
        assert!(surrogate < PyStr::from("\u{E000}"));
        assert!(PyStr::from("\u{FFFF}") < PyStr::from("\u{1F600}"));
        assert_eq!(format!("{surrogate:?}"), "\"\\u{d800}\"");
    }
}

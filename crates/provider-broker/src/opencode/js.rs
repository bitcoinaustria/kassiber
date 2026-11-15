//! JSON as the TypeScript adapter sees it: `JSON.parse` into JavaScript
//! values and `JSON.stringify` back.
//!
//! OpenCode's configuration passes through the adapter (the user's inline
//! config, provider settings, the server's effective config), and what the
//! adapter hands OpenCode must be the document the TypeScript broker would
//! have written: keys in JavaScript property order (array-index keys first,
//! ascending, then the rest in insertion order), numbers as JavaScript
//! prints them. `serde_json` in this crate sorts object keys and keeps
//! number text as parsed, so it cannot carry these documents.
//!
//! Strings are Rust strings: a lone UTF-16 surrogate escape (`"\ud800"`),
//! which JavaScript keeps, becomes U+FFFD here.

use std::fmt::Write;

/// A JavaScript value parsed from JSON.
#[derive(Clone, Debug, PartialEq)]
pub enum Js {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Js>),
    Obj(Obj),
}

/// A plain object in JavaScript property order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Obj(Vec<(String, Js)>);

/// A canonical array index (`"0"`, `"17"`, below 2³² − 1): such keys come
/// first in a JavaScript object, in numeric order.
fn array_index(key: &str) -> Option<u32> {
    let bytes = key.as_bytes();
    if bytes.is_empty() || bytes.len() > 10 || (bytes.len() > 1 && bytes[0] == b'0') {
        return None;
    }
    if !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    key.parse::<u64>()
        .ok()
        .filter(|index| *index < u64::from(u32::MAX))
        .map(|index| index as u32)
}

impl Obj {
    pub fn new() -> Self {
        Self::default()
    }

    /// `object[key]`.
    pub fn get(&self, key: &str) -> Option<&Js> {
        self.0
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Js> {
        self.0
            .iter_mut()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// `object[key] = value`: an existing key keeps its place.
    pub fn set(&mut self, key: impl Into<String>, value: Js) {
        let key = key.into();
        if let Some(slot) = self.0.iter_mut().find(|(name, _)| *name == key) {
            slot.1 = value;
            return;
        }
        let at = match array_index(&key) {
            Some(index) => self
                .0
                .iter()
                .position(|(name, _)| array_index(name).is_none_or(|other| other > index))
                .unwrap_or(self.0.len()),
            None => self.0.len(),
        };
        self.0.insert(at, (key, value));
    }

    /// `delete object[key]`.
    pub fn remove(&mut self, key: &str) {
        self.0.retain(|(name, _)| name != key);
    }

    /// `Object.entries(object)`.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Js)> {
        self.0.iter().map(|(name, value)| (name.as_str(), value))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Build an object from entries, assigning each in turn.
    pub fn from_entries<K: Into<String>>(entries: impl IntoIterator<Item = (K, Js)>) -> Self {
        let mut object = Self::new();
        for (key, value) in entries {
            object.set(key, value);
        }
        object
    }
}

impl Js {
    pub fn str(text: impl Into<String>) -> Self {
        Js::Str(text.into())
    }

    pub fn obj<K: Into<String>>(entries: impl IntoIterator<Item = (K, Js)>) -> Self {
        Js::Obj(Obj::from_entries(entries))
    }

    /// `value?.[key]` on a value that may not be an object: arrays and
    /// primitives have no such (own) key here.
    pub fn get(&self, key: &str) -> Option<&Js> {
        match self {
            Js::Obj(object) => object.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Js::Str(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&Obj> {
        match self {
            Js::Obj(object) => Some(object),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Js]> {
        match self {
            Js::Arr(items) => Some(items),
            _ => None,
        }
    }

    /// JavaScript truthiness.
    pub fn truthy(&self) -> bool {
        match self {
            Js::Null => false,
            Js::Bool(flag) => *flag,
            Js::Num(number) => *number != 0.0 && !number.is_nan(),
            Js::Str(text) => !text.is_empty(),
            Js::Arr(_) | Js::Obj(_) => true,
        }
    }

    /// `String(value)`.
    pub fn to_js_string(&self) -> String {
        match self {
            Js::Null => "null".into(),
            Js::Bool(flag) => flag.to_string(),
            Js::Num(number) => number_string(*number),
            Js::Str(text) => text.clone(),
            Js::Arr(items) => items
                .iter()
                .map(|item| match item {
                    Js::Null => String::new(),
                    other => other.to_js_string(),
                })
                .collect::<Vec<_>>()
                .join(","),
            Js::Obj(_) => "[object Object]".into(),
        }
    }

    /// `a === b`: two objects or arrays are never the same value here, as
    /// two separately parsed ones are never the same object.
    pub fn strict_eq(&self, other: &Js) -> bool {
        match (self, other) {
            (Js::Null, Js::Null) => true,
            (Js::Bool(a), Js::Bool(b)) => a == b,
            (Js::Num(a), Js::Num(b)) => a == b,
            (Js::Str(a), Js::Str(b)) => a == b,
            _ => false,
        }
    }

    /// A request option (`serde_json`) as a JavaScript value.
    pub fn from_serde(value: &serde_json::Value) -> Js {
        match value {
            serde_json::Value::Null => Js::Null,
            serde_json::Value::Bool(flag) => Js::Bool(*flag),
            serde_json::Value::Number(number) => Js::Num(number.as_f64().unwrap_or(f64::NAN)),
            serde_json::Value::String(text) => Js::Str(text.clone()),
            serde_json::Value::Array(items) => Js::Arr(items.iter().map(Js::from_serde).collect()),
            serde_json::Value::Object(map) => Js::Obj(Obj::from_entries(
                map.iter()
                    .map(|(key, value)| (key.clone(), Js::from_serde(value))),
            )),
        }
    }
}

/// `object(value)` in `opencode2.ts`: a plain object, else an empty one.
pub fn object(value: Option<&Js>) -> Obj {
    match value {
        Some(Js::Obj(object)) => object.clone(),
        _ => Obj::new(),
    }
}

/// `Number.prototype.toString()`: the shortest digits that round-trip,
/// laid out as ECMAScript's Number::toString lays them out.
pub fn number_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value == 0.0 {
        return "0".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    let sign = if value < 0.0 { "-" } else { "" };
    // Rust's `{:e}` prints the shortest round-trip digits: `d[.ddd]e<exp>`.
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i64 = exponent.parse().unwrap_or(0);
    let k = digits.len() as i64;
    let n = exponent + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let exponent = n - 1;
        let sign = if exponent < 0 { '-' } else { '+' };
        let head = if k == 1 {
            digits.clone()
        } else {
            format!("{}.{}", &digits[..1], &digits[1..])
        };
        format!("{head}e{sign}{}", exponent.abs())
    };
    format!("{sign}{body}")
}

/// `JSON.stringify(value)`.
pub fn stringify(value: &Js) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

fn write_value(out: &mut String, value: &Js) {
    match value {
        Js::Null => out.push_str("null"),
        Js::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Js::Num(number) if number.is_finite() => out.push_str(&number_string(*number)),
        Js::Num(_) => out.push_str("null"),
        Js::Str(text) => write_string(out, text),
        Js::Arr(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(out, item);
            }
            out.push(']');
        }
        Js::Obj(object) => {
            out.push('{');
            for (index, (key, item)) in object.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(out, key);
                out.push(':');
                write_value(out, item);
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// How deep `parse` follows nested arrays and objects before it gives up.
const MAX_DEPTH: usize = 2_000;

/// `JSON.parse(text)`; `None` where JavaScript throws a `SyntaxError`.
pub fn parse(text: &str) -> Option<Js> {
    let mut parser = Parser {
        text: text.as_bytes(),
        at: 0,
    };
    parser.space();
    let value = parser.value(0)?;
    parser.space();
    (parser.at == parser.text.len()).then_some(value)
}

struct Parser<'a> {
    text: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn literal(&mut self, word: &[u8], value: Js) -> Option<Js> {
        self.text[self.at..].starts_with(word).then(|| {
            self.at += word.len();
            value
        })
    }

    fn value(&mut self, depth: usize) -> Option<Js> {
        if depth > MAX_DEPTH {
            return None;
        }
        match self.peek()? {
            b'n' => self.literal(b"null", Js::Null),
            b't' => self.literal(b"true", Js::Bool(true)),
            b'f' => self.literal(b"false", Js::Bool(false)),
            b'"' => self.string().map(Js::Str),
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.peek() == Some(b']') {
                    self.at += 1;
                    return Some(Js::Arr(items));
                }
                loop {
                    self.space();
                    items.push(self.value(depth + 1)?);
                    self.space();
                    match self.peek()? {
                        b',' => self.at += 1,
                        b']' => {
                            self.at += 1;
                            return Some(Js::Arr(items));
                        }
                        _ => return None,
                    }
                }
            }
            b'{' => {
                self.at += 1;
                let mut object = Obj::new();
                self.space();
                if self.peek() == Some(b'}') {
                    self.at += 1;
                    return Some(Js::Obj(object));
                }
                loop {
                    self.space();
                    if self.peek()? != b'"' {
                        return None;
                    }
                    let key = self.string()?;
                    self.space();
                    if self.peek()? != b':' {
                        return None;
                    }
                    self.at += 1;
                    self.space();
                    let value = self.value(depth + 1)?;
                    object.set(key, value);
                    self.space();
                    match self.peek()? {
                        b',' => self.at += 1,
                        b'}' => {
                            self.at += 1;
                            return Some(Js::Obj(object));
                        }
                        _ => return None,
                    }
                }
            }
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Option<Js> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek()? {
            b'0' => self.at += 1,
            b'1'..=b'9' => {
                self.digits();
            }
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return None;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return None;
            }
        }
        let text = std::str::from_utf8(&self.text[start..self.at]).ok()?;
        text.parse::<f64>().ok().map(Js::Num)
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.text.get(self.at..self.at + 4)?;
        let text = std::str::from_utf8(digits).ok()?;
        if !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        self.at += 4;
        u32::from_str_radix(text, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while let Some(byte) = self.peek() {
                if byte == b'"' || byte == b'\\' || byte < 0x20 {
                    break;
                }
                self.at += 1;
            }
            out.push_str(std::str::from_utf8(&self.text[start..self.at]).ok()?);
            match self.peek()? {
                b'"' => {
                    self.at += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.at += 1;
                    let escape = self.peek()?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let unit = self.hex4()?;
                            if (0xD800..0xDC00).contains(&unit)
                                && self.text[self.at..].starts_with(b"\\u")
                            {
                                let save = self.at;
                                self.at += 2;
                                match self.hex4() {
                                    Some(low) if (0xDC00..0xE000).contains(&low) => {
                                        let code =
                                            0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                                        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                                        continue;
                                    }
                                    _ => self.at = save,
                                }
                            }
                            out.push(char::from_u32(unit).unwrap_or('\u{fffd}'));
                        }
                        _ => return None,
                    }
                }
                // An unescaped control character.
                _ => return None,
            }
        }
    }
}

/// `encodeURIComponent(text)`.
pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// A string's length in UTF-16 code units, as `String.prototype.length`.
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints_in_javascript_property_order() {
        let value = parse(r#"{"b":1,"2":true,"a":[null,"x"],"10":{},"1":-0.5,"b":3}"#).unwrap();
        assert_eq!(
            stringify(&value),
            r#"{"1":-0.5,"2":true,"10":{},"b":3,"a":[null,"x"]}"#
        );
    }

    #[test]
    fn numbers_print_as_javascript_prints_them() {
        for (text, printed) in [
            ("1.0", "1"),
            ("1e3", "1000"),
            ("0.7", "0.7"),
            ("123.456", "123.456"),
            ("0.000001", "0.000001"),
            ("1e-7", "1e-7"),
            ("1.5e-7", "1.5e-7"),
            ("1e21", "1e+21"),
            ("123456789012345678901", "123456789012345680000"),
            ("-0", "0"),
            ("9007199254740993", "9007199254740992"),
            ("1e400", "null"),
        ] {
            assert_eq!(stringify(&parse(text).unwrap()), printed, "{text}");
        }
    }

    #[test]
    fn strings_escape_as_json_stringify_does() {
        let value = parse(r#""a\"\\\/\b\f\n\r\t\u0001\u00e9\ud83d\ude00 ""#).unwrap();
        assert_eq!(
            stringify(&value),
            "\"a\\\"\\\\/\\b\\f\\n\\r\\t\\u0001é😀 \""
        );
    }

    #[test]
    fn refuses_what_json_parse_refuses() {
        for text in [
            "",
            "{",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "1.",
            ".5",
            "+1",
            "'x'",
            "\"\u{1}\"",
            "nul",
            "{a:1}",
            "\u{feff}{}",
            "[1] x",
            "\"\\x\"",
            "NaN",
        ] {
            assert!(parse(text).is_none(), "{text:?}");
        }
        assert_eq!(parse(" \t\r\n{} \n"), Some(Js::Obj(Obj::new())));
    }

    #[test]
    fn string_conversion_follows_javascript() {
        let value = parse(r#"[["kassiber_x"], [1, null, true], {}, null, 2.5]"#).unwrap();
        let items = value.as_arr().unwrap();
        assert_eq!(items[0].to_js_string(), "kassiber_x");
        assert_eq!(items[1].to_js_string(), "1,,true");
        assert_eq!(items[2].to_js_string(), "[object Object]");
        assert_eq!(items[3].to_js_string(), "null");
        assert_eq!(items[4].to_js_string(), "2.5");
    }

    #[test]
    fn encodes_uri_components() {
        assert_eq!(
            encode_uri_component("/tmp/a b:é!*'()~"),
            "%2Ftmp%2Fa%20b%3A%C3%A9!*'()~"
        );
    }
}

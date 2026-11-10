//! `str.strip()` and `repr(str)`.

use std::fmt::Write as _;

use kassiber_pyjson::PyStr;

use crate::unicode::{is_printable, is_space};

/// `text.strip()`: trims the characters `str.isspace` accepts, which include
/// the separators `\x1c` to `\x1f` that Rust's `trim` keeps.
pub fn strip(text: &str) -> &str {
    text.trim_matches(|c: char| is_space(u32::from(c)))
}

/// `text.strip()` for a string that may hold surrogates.
pub fn strip_py(text: &PyStr) -> PyStr {
    let points: Vec<u32> = text.code_points().collect();
    let start = points.iter().position(|&p| !is_space(p));
    let Some(start) = start else {
        return PyStr::new();
    };
    let end = points.iter().rposition(|&p| !is_space(p)).unwrap_or(start);
    // The points came from a PyStr, so every one is a valid code point.
    PyStr::from_code_points(points[start..=end].iter().copied()).unwrap_or_default()
}

/// `repr(text)`.
pub fn repr_str(text: &str) -> String {
    repr_points(text.chars().map(u32::from))
}

/// `repr(text)` for a string that may hold surrogates; they print as
/// `\udXXX` escapes, as in Python.
pub fn repr_py(text: &PyStr) -> String {
    repr_points(text.code_points())
}

fn repr_points<I>(points: I) -> String
where
    I: Iterator<Item = u32> + Clone,
{
    // CPython quotes with ' unless the text holds ' and no ".
    let (mut single, mut double) = (false, false);
    for point in points.clone() {
        single |= point == u32::from('\'');
        double |= point == u32::from('"');
    }
    let quote = if single && !double { '"' } else { '\'' };
    let mut out = String::new();
    out.push(quote);
    for point in points {
        match char::from_u32(point) {
            Some(c) if c == quote || c == '\\' => {
                out.push('\\');
                out.push(c);
            }
            Some('\t') => out.push_str("\\t"),
            Some('\n') => out.push_str("\\n"),
            Some('\r') => out.push_str("\\r"),
            Some(_) if point < 0x20 || point == 0x7F => {
                let _ = write!(out, "\\x{point:02x}");
            }
            Some(c) if point < 0x7F || is_printable(point) => out.push(c),
            _ if point <= 0xFF => {
                let _ = write!(out, "\\x{point:02x}");
            }
            _ if point <= 0xFFFF => {
                let _ = write!(out, "\\u{point:04x}");
            }
            _ => {
                let _ = write!(out, "\\U{point:08x}");
            }
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_uses_python_whitespace() {
        assert_eq!(strip("\x1c\t x \u{3000}\x1f"), "x");
        assert_eq!(strip("\u{200b}x"), "\u{200b}x");
        let surrogate = PyStr::from_code_points([0x20, 0xD800, 0x20]).unwrap();
        assert_eq!(
            strip_py(&surrogate),
            PyStr::from_code_points([0xD800]).unwrap()
        );
        assert_eq!(strip_py(&PyStr::from(" \n ")), PyStr::new());
    }

    #[test]
    fn repr_matches_cpython() {
        assert_eq!(repr_str("status"), "'status'");
        assert_eq!(repr_str("it's"), "\"it's\"");
        assert_eq!(repr_str("'\""), "'\\'\"'");
        assert_eq!(
            repr_str("a\\b\n\t\r\x00\x7f"),
            "'a\\\\b\\n\\t\\r\\x00\\x7f'"
        );
        assert_eq!(
            repr_str("caf\u{e9} \u{a0}\u{200b}\u{1F600}"),
            "'caf\u{e9} \\xa0\\u200b\u{1F600}'"
        );
        assert_eq!(repr_str("\u{E0001}"), "'\\U000e0001'");
        let lone = PyStr::from_code_points([0x61, 0xDC00]).unwrap();
        assert_eq!(repr_py(&lone), "'a\\udc00'");
    }
}

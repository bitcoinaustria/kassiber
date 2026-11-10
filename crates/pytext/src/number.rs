//! `int(text)` and `float(text)` for `str` arguments.
//!
//! CPython first maps non-ASCII whitespace to a space and non-ASCII decimal
//! digits to ASCII ones, refusing any other non-ASCII character; then it
//! trims ASCII whitespace only, so `\x1c` to `\x1f`, which `str.strip()`
//! removes, make both conversions fail.

use num_bigint::BigInt;

use crate::unicode::{decimal_value, is_space};

/// CPython's default `sys.get_int_max_str_digits()`.
pub const INT_MAX_STR_DIGITS: usize = 4300;

/// Why `int()` or `float()` raised `ValueError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NumberError {
    /// `invalid literal for int() with base 10` or `could not convert string
    /// to float`.
    Invalid,
    /// `Exceeds the limit (4300 digits) for integer string conversion`, with
    /// the number of digits.
    TooManyDigits(usize),
}

/// `_PyUnicode_TransformDecimalAndSpaceToASCII`, refusing the characters
/// CPython turns into `?`.
fn to_ascii(text: &str) -> Option<Vec<u8>> {
    text.chars()
        .map(|c| {
            let point = u32::from(c);
            if point < 0x80 {
                Some(point as u8)
            } else if is_space(point) {
                Some(b' ')
            } else {
                // Decimal values are 0..=9, so the digit stays ASCII.
                decimal_value(point).map(|digit| b'0' + digit as u8)
            }
        })
        .collect()
}

/// `Py_ISSPACE`: ASCII whitespace only.
fn is_ascii_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

fn trim_ascii_space(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|&b| !is_ascii_space(b));
    let Some(start) = start else {
        return &[];
    };
    let end = bytes
        .iter()
        .rposition(|&b| !is_ascii_space(b))
        .unwrap_or(start);
    &bytes[start..=end]
}

/// Drops underscores that sit between two digits; `None` for any other
/// underscore, as `_Py_string_to_number_with_underscores` refuses it.
fn remove_digit_underscores(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut prev = 0u8;
    for &byte in bytes {
        if byte == b'_' {
            if !prev.is_ascii_digit() {
                return None;
            }
        } else {
            if prev == b'_' && !byte.is_ascii_digit() {
                return None;
            }
            out.push(byte);
        }
        prev = byte;
    }
    (prev != b'_').then_some(out)
}

/// `int(text)` (base 10).
pub fn parse_int(text: &str) -> Result<BigInt, NumberError> {
    let ascii = to_ascii(text).ok_or(NumberError::Invalid)?;
    let body = trim_ascii_space(&ascii);
    let (negative, digits) = match body.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, body),
    };
    if digits.first().is_none_or(|b| !b.is_ascii_digit()) {
        return Err(NumberError::Invalid);
    }
    let digits = remove_digit_underscores(digits).ok_or(NumberError::Invalid)?;
    if !digits.iter().all(u8::is_ascii_digit) {
        return Err(NumberError::Invalid);
    }
    if digits.len() > INT_MAX_STR_DIGITS {
        return Err(NumberError::TooManyDigits(digits.len()));
    }
    let magnitude = BigInt::parse_bytes(&digits, 10).ok_or(NumberError::Invalid)?;
    Ok(if negative { -magnitude } else { magnitude })
}

/// Whether `bytes` is `_Py_dg_strtod`'s whole decimal: digits with an
/// optional fraction, at least one digit, and an optional exponent.
fn is_decimal_literal(bytes: &[u8]) -> bool {
    let mut i = 0;
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - start
    };
    let mut mantissa = digits(&mut i);
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        mantissa += digits(&mut i);
    }
    if mantissa == 0 {
        return false;
    }
    if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
        i += 1;
        if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == bytes.len()
}

/// `float(text)`.
pub fn parse_float(text: &str) -> Result<f64, NumberError> {
    let ascii = to_ascii(text).ok_or(NumberError::Invalid)?;
    let body = trim_ascii_space(&ascii);
    if body.is_empty() {
        return Err(NumberError::Invalid);
    }
    let body = if body.contains(&b'_') {
        remove_digit_underscores(body).ok_or(NumberError::Invalid)?
    } else {
        body.to_vec()
    };
    let (negative, rest) = match body.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, &body[..]),
    };
    let lowered = rest.to_ascii_lowercase();
    let magnitude = match lowered.as_slice() {
        b"inf" | b"infinity" => f64::INFINITY,
        b"nan" => f64::NAN,
        _ if is_decimal_literal(rest) => {
            // The literal is ASCII and validated; Rust's parse rounds
            // correctly, as CPython's dtoa does.
            let literal = std::str::from_utf8(rest).map_err(|_| NumberError::Invalid)?;
            literal.parse::<f64>().map_err(|_| NumberError::Invalid)?
        }
        _ => return Err(NumberError::Invalid),
    };
    Ok(if negative { -magnitude } else { magnitude })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_accepts_what_cpython_accepts() {
        for (text, value) in [
            ("\u{85}5", 5),
            (" 5 ", 5),
            ("\u{3000}5\u{3000}", 5),
            ("1_0", 10),
            ("+5", 5),
            ("-5", -5),
            ("\u{663}", 3),
            ("007", 7),
            ("\u{ff15}", 5),
            ("0_0", 0),
            ("-0", 0),
        ] {
            assert_eq!(parse_int(text), Ok(BigInt::from(value)), "{text:?}");
        }
        for text in [
            "\x1c5", "5\x1c", "_10", "10_", "1__0", "- 5", "0x10", "1e5", "", " ", "5\0", "1.5",
            "+_1", "\u{b2}",
        ] {
            assert_eq!(parse_int(text), Err(NumberError::Invalid), "{text:?}");
        }
        assert_eq!(
            parse_int(&"9".repeat(4301)),
            Err(NumberError::TooManyDigits(4301))
        );
        assert!(parse_int(&"9".repeat(4300)).is_ok());
    }

    #[test]
    fn float_accepts_what_cpython_accepts() {
        for (text, value) in [
            ("1_000.5", 1000.5),
            ("1e1_0", 1e10),
            ("1.", 1.0),
            (".5", 0.5),
            ("1E-5", 1e-5),
            ("  -1.5e3  ", -1500.0),
            ("\u{661}.\u{665}", 1.5),
            ("\u{661}_\u{662}", 12.0),
            ("1e5", 100000.0),
        ] {
            assert_eq!(parse_float(text), Ok(value), "{text:?}");
        }
        assert_eq!(parse_float("-Infinity"), Ok(f64::NEG_INFINITY));
        assert_eq!(parse_float("inf"), Ok(f64::INFINITY));
        assert!(parse_float("+nan").unwrap().is_nan());
        assert!(parse_float("nAn").unwrap().is_nan());
        assert!(parse_float("-0").unwrap().is_sign_negative());
        for text in [
            "1._5", "1.5_", "in_f", ".", "e5", "1e", "1e+", "1_e5", "1e_5", "infinit", "0x1p3",
            "1d5", "\x1c5", "",
        ] {
            assert_eq!(parse_float(text), Err(NumberError::Invalid), "{text:?}");
        }
    }
}

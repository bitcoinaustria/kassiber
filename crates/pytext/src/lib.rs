//! CPython's text semantics that Kassiber's machine contracts expose.
//!
//! The CLI and the daemon echo Python text into their envelopes: argparse
//! messages quote values with `repr`, the daemon strips request lines with
//! `str.strip()` and reports `json.loads` failures with their exact message,
//! and `int()` and `float()` decide which arguments parse. This crate gives
//! the Rust backend (docs/plan/21-rust-backend.md) the same answers:
//!
//! - [`is_space`], [`strip`], and [`strip_py`]: `str.isspace` and
//!   `str.strip()`, including the separators `\x1c` to `\x1f`;
//! - [`repr_str`] and [`repr_py`]: `repr(str)`, with CPython's quote choice
//!   and its escapes for characters `str.isprintable` rejects;
//! - [`parse_int`] and [`parse_float`]: `int(text)` and `float(text)`,
//!   including surrounding whitespace, underscores between digits, non-ASCII
//!   decimal digits, and the 4,300-digit limit;
//! - [`loads`]: `json.loads` into [`kassiber_pyjson::Value`], with the C
//!   scanner's error messages and positions;
//! - [`zone`]: `zoneinfo.ZoneInfo` key validation, TZif loading and offsets.
//!
//! The Unicode tables and `tests/data/vectors.txt` were taken from CPython
//! 3.11 (Unicode 14.0.0) and are frozen; their generators went with the
//! Python tree (tag `archive/python-backend`).

mod case;
mod json;
mod number;
mod text;
mod unicode;
#[rustfmt::skip]
mod unicode_tables;

pub fn utf8_error(text: &kassiber_pyjson::PyStr) -> Option<String> {
    let points: Vec<_> = text.code_points().collect();
    let start = points.iter().position(|p| (0xd800..=0xdfff).contains(p))?;
    let count = points[start..]
        .iter()
        .take_while(|p| (0xd800..=0xdfff).contains(*p))
        .count();
    Some(if count == 1 {
        format!("'utf-8' codec can't encode character '\\u{:04x}' in position {start}: surrogates not allowed", points[start])
    } else {
        format!(
            "'utf-8' codec can't encode characters in position {start}-{}: surrogates not allowed",
            start + count - 1
        )
    })
}

pub use case::{lower, upper};
pub use json::{
    loads, loads_py, loads_py_without_constants, JsonError, JsonErrorKind, MAX_NESTING,
};
pub use number::{parse_float, parse_int, NumberError};
pub use text::{repr_py, repr_str, strip, strip_py};
pub use unicode::{decimal_value, is_alpha, is_printable, is_space, unicode_version};
pub use unicode_tables::{REGEX_ASCII_CASES, REGEX_DIGIT, REGEX_SPACE, REGEX_WORD};

mod iso_datetime;
pub use iso_datetime::{iso_datetime, IsoDateTime};
#[rustfmt::skip]
mod casefold;
pub use casefold::casefold;
pub mod zone;

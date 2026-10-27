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
//!   scanner's error messages and positions.
//!
//! The Unicode tables come from CPython 3.11 (Unicode 14.0.0) through
//! `scripts/gen_unicode_tables.py`; `tests/data/vectors.txt` comes from the
//! same interpreter through `scripts/gen_vectors.py`.

mod json;
mod number;
mod text;
mod unicode;
mod unicode_tables;

pub use json::{loads, JsonError, JsonErrorKind, MAX_NESTING};
pub use number::{parse_float, parse_int, NumberError};
pub use text::{repr_py, repr_str, strip, strip_py};
pub use unicode::{decimal_value, is_printable, is_space, unicode_version};

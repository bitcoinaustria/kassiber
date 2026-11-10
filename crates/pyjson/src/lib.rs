//! CPython's `json.dumps` output, byte for byte, and `repr(float)`.
//!
//! Kassiber hashes and stores text that Python's `json.dumps` wrote: custody
//! evidence hashes, graph and quantity hashes, gap fingerprints, issue ids,
//! and JSON columns. Recomputed commitments are compared with stored ones,
//! so the Rust backend (docs/plan/21-rust-backend.md) must write the same
//! bytes. [`dumps`] takes a [`Value`] shaped like the Python object (ordered
//! dicts, unbounded ints, floats, strings that may hold surrogates, and
//! `Decimal`) and [`Options`] mirroring the keyword arguments Kassiber
//! passes: `separators`, `indent`, `ensure_ascii`, `sort_keys`, `allow_nan`,
//! and `default=str`.
//!
//! What matches CPython 3.11:
//!
//! - floats as `float.__repr__` ([`float_repr`]): shortest round-trip
//!   digits with CPython's tie rule and layout, and `NaN`, `Infinity`, and
//!   `-Infinity` when `allow_nan`;
//! - ints of any size, with CPython's 4,300-digit conversion limit;
//! - string escaping for both `ensure_ascii` settings: `\"`, `\\`, the five
//!   short escapes, lowercase `\u00XX` for other control characters, and
//!   with `ensure_ascii` every character outside printable ASCII as `\uXXXX`
//!   (DEL included), astral characters as surrogate pairs, and lone
//!   surrogates as themselves;
//! - non-string keys (`null`, `true`, `false`, int digits, float `repr`);
//!   `sort_keys` ordering strings by code point and numbers by exact value,
//!   and refusing what Python cannot order;
//! - both separator defaults (`", "` and `": "`, or `","` with an indent)
//!   and the indented layout of Python's pure encoder;
//! - `Decimal` through `default=str` as `str(Decimal)`, from
//!   `kassiber-pydecimal`;
//! - which error comes first when a value has several, so a refusal is the
//!   one Python raises.
//!
//! Where it differs, it refuses instead of guessing: see [`Error`]
//! ([`Error::NanKeyOrder`], [`Error::TooDeep`], [`Error::Surrogate`]).
//! `skipkeys`, `check_circular`, `cls`, and callables other than `str` for
//! `default` are out of scope; Kassiber passes none of them. The vectors in
//! `tests/data/json_vectors.txt` come from CPython; `README.md` lists the
//! surveyed call variants and how to regenerate both.

mod bytes;
mod encode;
mod float;
mod value;

pub use bytes::json_bytes_string;
pub use encode::{
    dumps, dumps_into, dumps_str_into, Error, Indent, Options, INT_MAX_STR_DIGITS, MAX_DEPTH,
};
pub use float::float_repr;
pub use value::{bytes_repr, CodePoints, InvalidCodePoint, Key, PyStr, Value};

pub use kassiber_pydecimal::Decimal;
pub use num_bigint::BigInt;

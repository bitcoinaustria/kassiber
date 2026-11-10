//! Lookups in the generated CPython Unicode tables.

use crate::unicode_tables::{ALPHA, DECIMAL_RUNS, NOT_PRINTABLE, SPACE, UNICODE_VERSION};

/// `chr(point).isalpha()` (letters only, excluding combining marks).
pub fn is_alpha(point: u32) -> bool {
    find(ALPHA, point).is_some()
}

fn find(table: &[(u32, u32)], point: u32) -> Option<(u32, u32)> {
    let index = table.partition_point(|&(_, end)| end < point);
    table
        .get(index)
        .copied()
        .filter(|&(start, _)| start <= point)
}

/// The Unicode version the tables describe (CPython 3.11's `unidata_version`).
pub fn unicode_version() -> &'static str {
    UNICODE_VERSION
}

/// `chr(point).isspace()`.
pub fn is_space(point: u32) -> bool {
    find(SPACE, point).is_some()
}

/// `chr(point).isprintable()`. Surrogates are not printable.
pub fn is_printable(point: u32) -> bool {
    find(NOT_PRINTABLE, point).is_none()
}

/// `unicodedata.decimal(chr(point))`, or `None` for a non-digit.
pub fn decimal_value(point: u32) -> Option<u32> {
    find(DECIMAL_RUNS, point).map(|(start, _)| (point - start) % 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_answer_like_cpython() {
        for point in [0x9, 0xA, 0xD, 0x1C, 0x1F, 0x20, 0x85, 0xA0, 0x2028, 0x3000] {
            assert!(is_space(point), "{point:#x}");
        }
        for point in [0x8, 0xE, 0x21, 0x180E, 0x200B, 0xFEFF] {
            assert!(!is_space(point), "{point:#x}");
        }
        assert!(is_printable(u32::from(' ')));
        assert!(is_printable(0xE9));
        assert!(!is_printable(0x7F));
        assert!(!is_printable(0xA0));
        assert!(!is_printable(0xD800));
        assert!(!is_printable(0x200B));
        assert_eq!(decimal_value(u32::from('7')), Some(7));
        assert_eq!(decimal_value(0x0663), Some(3));
        assert_eq!(decimal_value(0xFF15), Some(5));
        assert_eq!(decimal_value(0x1D7D9), Some(1));
        assert_eq!(decimal_value(0xB2), None);
        assert_eq!(unicode_version(), "14.0.0");
    }
}

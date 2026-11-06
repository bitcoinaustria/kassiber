//! Tests for the generated `python_text` module, kept out of it so
//! `tests/custody_python_text.py` can regenerate that file unchanged.
use crate::python_text::*;

#[test]
fn borrowed_case_mappings_match_python() {
    for text in [
        "", "main", "Main", "MAIN", "lbtc", "LBTC", "core-LN", "\u{130}x", "\u{3a3}a", "ß",
    ] {
        assert_eq!(lower(text), text.py_lower(), "{text:?}");
        assert_eq!(upper(text), text.py_upper(), "{text:?}");
    }
    assert!(matches!(lower("main"), std::borrow::Cow::Borrowed(_)));
    assert!(matches!(upper("LBTC"), std::borrow::Cow::Borrowed(_)));
}

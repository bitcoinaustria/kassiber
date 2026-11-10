//! `reports privacy-hygiene` and `ui.reports.privacy_hygiene` through the
//! binary, replayed against the frozen Python reference (`privacy_transport`).
//!
//! The former `privacy_hygiene_frozen_oracle_matches_unchanged_python` only
//! re-ran Python against `crates/reports/tests/data/privacy_hygiene.json`;
//! that file stays the oracle, and `crates/reports/tests/privacy_hygiene.rs`
//! checks Rust against it.
#![cfg(feature = "test-harness")]

mod privacy_transport;

#[test]
fn privacy_hygiene_cli_daemon_bytes_all_tables_and_zero_fallback() {
    // 6 cases: 8 CLI calls each, and a daemon session for the 5 well-formed.
    assert_eq!(privacy_transport::replay("hygiene", "hygiene/"), 6 * 8 + 5);
}

#[test]
fn privacy_hygiene_corrupt_daemon_markers_preserve_native_debug_contract() {
    // 3 corrupt profile fields, with and without --debug.
    assert_eq!(privacy_transport::replay("hygiene", "hygiene-debug/"), 6);
}

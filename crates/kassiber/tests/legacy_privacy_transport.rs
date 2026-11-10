//! The legacy privacy snapshot over the daemon and the configuration report
//! CLI, replayed against the frozen Python reference (`privacy_transport`).
#![cfg(feature = "test-harness")]

mod privacy_transport;

#[test]
fn legacy_daemon_acceptance_bytes_all_tables_zero_fallback() {
    // 6 cases, online and offline, 28 requests each.
    assert_eq!(privacy_transport::replay("legacy", "legacy-daemon/"), 12);
}

#[test]
fn legacy_cli_configuration_report_remains_unchanged() {
    assert_eq!(privacy_transport::replay("legacy", "legacy-cli/"), 12);
}

#[test]
fn legacy_corrupt_amount_has_honest_native_diagnostics() {
    assert_eq!(privacy_transport::replay("legacy", "legacy-corrupt/"), 2);
}

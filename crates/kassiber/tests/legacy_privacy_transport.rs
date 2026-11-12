#![cfg(feature = "test-harness")]
use std::path::Path;
use std::process::Command;

fn reference(selection: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let python = std::env::var_os("KASSIBER_REFERENCE_PYTHON")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join(".venv/bin/python"));
    let scratch = tempfile::tempdir().unwrap();
    let result = Command::new(python)
        .current_dir(&root)
        .args([
            "-m",
            "pytest",
            "tests/test_rust_legacy_privacy_services.py",
            "-q",
            "-s",
            "-k",
            selection,
            "--basetemp",
        ])
        .arg(scratch.path())
        .env("KASSIBER_RUST_BIN", env!("CARGO_BIN_EXE_kassiber"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    println!("{}", String::from_utf8_lossy(&result.stdout));
}
#[test]
fn legacy_daemon_acceptance_bytes_all_tables_zero_fallback() {
    reference("test_legacy_daemon_bytes");
}
#[test]
fn legacy_cli_configuration_report_remains_unchanged() {
    reference("test_legacy_registration_preserves");
}
#[test]
fn legacy_corrupt_amount_has_honest_native_diagnostics() {
    reference("test_legacy_corrupt_amount");
}

//! The `kassiber` binary, black box: what holds in every build.

use std::process::{Command, Output, Stdio};

fn kassiber(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kassiber"));
    command
        .args(args)
        .env_remove("KASSIBER_TEST_NO_EGRESS")
        .env_remove("KASSIBER_NO_EGRESS")
        .env_remove("KASSIBER_RUST_TEST_FAKE_STORE")
        .env_remove("COLUMNS")
        .stdin(Stdio::null());
    command
}

fn run(command: &mut Command) -> Output {
    let home = tempfile::tempdir().unwrap();
    command
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("LOCALAPPDATA", home.path())
        .env("XDG_DATA_HOME", home.path().join("data-home"))
        .env("XDG_CONFIG_HOME", home.path().join("config-home"));
    command.output().expect("the binary runs")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn prints_the_product_version() {
    let output = run(&mut kassiber(&["--version"]));
    assert!(output.status.success());
    let expected = match (
        option_env!("KASSIBER_BUILD_CHANNEL"),
        option_env!("KASSIBER_BUILD_COMMIT"),
    ) {
        (None, None) => format!("Kassiber {}\n", env!("CARGO_PKG_VERSION")),
        _ => return,
    };
    assert_eq!(text(&output.stdout), expected);
}

#[test]
fn an_armed_test_guard_needs_the_observer_build() {
    for value in ["1", "strict", "0"] {
        let output = run(kassiber(&["--version"]).env("KASSIBER_TEST_NO_EGRESS", value));
        if cfg!(feature = "test-harness") {
            assert!(output.status.success(), "{value}");
        } else {
            // Never run unguarded where the suite expects a guard.
            assert_eq!(output.status.code(), Some(78), "{value}");
            assert!(text(&output.stderr).contains("guard could not start"));
            assert!(output.stdout.is_empty());
        }
    }
    // The product switch needs no test build.
    let output = run(kassiber(&["--version"]).env("KASSIBER_NO_EGRESS", "1"));
    assert!(output.status.success());
}

#[test]
fn usage_errors_follow_machine_anywhere() {
    let output = run(&mut kassiber(&["status", "--machine"]));
    assert_eq!(output.status.code(), Some(2));
    let stdout = text(&output.stdout);
    assert!(stdout.starts_with("{\n  \"kind\": \"error\",\n  \"schema_version\": 1,\n  \"error\": {\n    \"code\": \"usage_error\",\n    \"message\": \"unrecognized arguments: --machine\","), "{stdout}");
    assert!(output.stderr.is_empty());
    let output = run(&mut kassiber(&["status", "--mach"]));
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(text(&output.stderr).ends_with("kassiber: error: unrecognized arguments: --mach\n"));
}

#[test]
fn real_store_creates_schema_and_requires_explicit_init() {
    let output = run(&mut kassiber(&["--machine", "status"]));
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stdout).contains("\"code\": \"not_initialized\""));
    let output = run(&mut kassiber(&["--machine", "init"]));
    assert!(output.status.success(), "{}", text(&output.stdout));
    let output = run(&mut kassiber(&["daemon"]));
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("daemon.ready"));
    let output = run(&mut kassiber(&["--machine", "reports", "summary"]));
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stdout).contains("\"command\": \"reports.summary\""));
}

#[test]
fn describes_all_commands_and_keeps_unported_execution_closed() {
    let output = run(&mut kassiber(&["--machine", "commands", "describe"]));
    assert!(output.status.success());
    let stdout = text(&output.stdout);
    assert!(
        stdout.contains(&format!(
            "\"count\": {},",
            kassiber_cli::describe::leaves().len()
        )),
        "{stdout}"
    );
    let output = run(&mut kassiber(&[
        "--machine",
        "commands",
        "describe",
        "wallets",
    ]));
    assert!(output.status.success());
    assert!(text(&output.stdout).contains("\"kind\": \"wallets.sync\""));
    for (family, action) in [
        ("networks", "inventory"),
        ("backends", "list"),
        ("offline", "status"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("unported");
        let output = run(&mut kassiber(&[
            "--machine",
            "--data-root",
            data.to_str().unwrap(),
            family,
            action,
        ]));
        assert_eq!(output.status.code(), Some(1));
        assert!(text(&output.stdout).contains("\"code\": \"not_implemented\""));
        assert!(!data.exists(), "{family} bootstrapped an unsupported book");
    }
}

#[test]
fn hidden_argv_forms_dispatch_by_name() {
    let output = run(&mut kassiber(&["--operator-broker-server"]));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stderr),
        "not_implemented: the operator broker server is not implemented in the Rust backend yet\n"
    );
    let output = run(&mut kassiber(&["--migrate-default-state-root"]));
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

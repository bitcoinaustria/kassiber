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
fn the_stub_store_fails_closed() {
    let output = run(&mut kassiber(&["--machine", "status"]));
    assert_eq!(output.status.code(), Some(1));
    let stdout = text(&output.stdout);
    assert!(stdout.contains("\"code\": \"not_implemented\""), "{stdout}");
    assert!(stdout.contains("\"component\": \"store\""), "{stdout}");

    // The daemon cannot resolve its project, so it never reports ready.
    // Without --data-root the state-root migration comes first, as in Python.
    for (args, step) in [
        (&["daemon"][..], "the default state-root migration"),
        (
            &["--data-root", "/nonexistent/kassiber", "daemon"][..],
            "opening a project",
        ),
    ] {
        let output = run(&mut kassiber(args));
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            text(&output.stderr),
            format!("error: {step} is not implemented in the Rust backend yet\n")
        );
    }

    let output = run(&mut kassiber(&["--machine", "reports", "summary"]));
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stdout).contains("\"command\": \"reports.summary\""));
}

#[test]
fn describes_only_what_it_implements() {
    let output = run(&mut kassiber(&["--machine", "commands", "describe"]));
    assert!(output.status.success());
    let stdout = text(&output.stdout);
    assert!(stdout.contains("\"count\": 3,"), "{stdout}");
    let output = run(&mut kassiber(&[
        "--machine",
        "commands",
        "describe",
        "wallets",
    ]));
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stdout).contains("\"code\": \"not_implemented\""));
}

#[test]
fn hidden_argv_forms_are_refused_by_name() {
    for (form, message) in [
        ("--operator-broker-server", "the operator broker server"),
        (
            "--migrate-default-state-root",
            "the default state-root migration",
        ),
    ] {
        let output = run(&mut kassiber(&[form]));
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            text(&output.stderr),
            format!("not_implemented: {message} is not implemented in the Rust backend yet\n")
        );
    }
}

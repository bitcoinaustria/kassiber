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
    // A once-unported leaf now reaches the book (here a fresh default one).
    let output = run(&mut kassiber(&["--machine", "journals", "list"]));
    assert!(
        !text(&output.stdout).contains("not_implemented"),
        "{output:?}"
    );
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
    let mut checked = Vec::new();
    for leaf in kassiber_cli::describe::leaves()
        .into_iter()
        .filter(|leaf| !leaf.implemented())
    {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("unported");
        let mut args = vec!["--machine", "--data-root", data.to_str().unwrap()];
        args.extend(witness_argv(&leaf));
        let output = run(&mut kassiber(&args));
        let command = leaf.dotted();
        assert_eq!(output.status.code(), Some(1), "{command}: {output:?}");
        assert!(
            text(&output.stdout).contains("\"code\": \"not_implemented\""),
            "{command}: {output:?}"
        );
        assert!(!data.exists(), "{command} bootstrapped an unsupported book");
        checked.push(command);
    }
    // Chat and the MCP server/call are native too (native chat removed
    // their gates): no leaf answers not_implemented in any build.
    assert_eq!(checked, Vec::<String>::new());
    assert!(text(&output.stdout).contains("\"kind\": \"wallets.derive\""));
    assert!(text(&output.stdout).contains("\"kind\": \"wallets.import-csv\""));
    assert!(text(&output.stdout).contains("\"kind\": \"wallets.import-json\""));
    assert!(text(&output.stdout).contains("\"kind\": \"wallets.import-phoenix\""));
}

/// A leaf's path with every required option, positional and required
/// mutually exclusive group filled by a witness value.
fn witness_argv(leaf: &kassiber_cli::describe::Leaf) -> Vec<&'static str> {
    use kassiber_cli::spec::{parsers, Nargs, ValueType};

    let parser = &parsers()[leaf.parser];
    let required_choices: Vec<usize> = parser
        .mutex_groups
        .iter()
        .filter(|group| group.required)
        .map(|group| group.actions[0])
        .collect();
    let mut argv = leaf.path.clone();
    for (index, action) in parser.actions.iter().enumerate() {
        let required_positional =
            action.is_positional() && matches!(action.nargs, Nargs::Single | Nargs::OneOrMore);
        if !(action.required || required_positional || required_choices.contains(&index)) {
            continue;
        }
        if let Some(option) = action.option_strings.first() {
            argv.push(option);
        }
        if action.nargs != Nargs::Zero {
            argv.push(
                action
                    .choices
                    .and_then(|choices| choices.first().copied())
                    .unwrap_or(match action.value_type {
                        ValueType::Str => "unported-witness",
                        _ => "1",
                    }),
            );
        }
    }
    argv
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Acceptance for the last native leaves (journals, quarantine, identify,
/// analyze-file, reveal-descriptor): every leaf of the tree runs natively,
/// chat and MCP included since native chat removed their gates. Each runs once with witness arguments on
/// its own copy of an initialized book and must not answer `not_implemented`
/// (the gate, a service family's fallback or a store default) or crash.
///
/// The environment keeps every run off the account: a scrubbed environment
/// (no session bus, so no keyring), temporary home, XDG and operator runtime
/// directories, the offline switch on and the egress kill switch set; test
/// builds also arm the strict attempt observer, which aborts on any socket.
/// The book has no default AI provider, so nothing names a local endpoint.
/// `operator unlock` alone is not run: it may launch a detached broker.
#[test]
fn every_leaf_but_gated_chat_and_mcp_runs_natively() {
    let base = tempfile::tempdir().unwrap();
    let home = base.path().join("home");
    let runtime = base.path().join("runtime");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&runtime).unwrap();
    let offline = base.path().join("offline-mode.json");
    std::fs::write(&offline, r#"{"schema_version": 1, "enabled": true}"#).unwrap();
    let book = base.path().join("book");
    let command = |cwd: &std::path::Path, args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kassiber"));
        command
            .env_clear()
            .args(args)
            .current_dir(cwd)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &home)
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("KASSIBER_OPERATOR_RUNTIME_DIR", runtime.join("operator"))
            .env("KASSIBER_OPERATOR_ALLOW_TEST_RUNTIME_DIR", "1")
            .env("KASSIBER_OFFLINE_PREFERENCE_FILE", &offline)
            .env("KASSIBER_NO_EGRESS", "1")
            .env("KASSIBER_DISABLE_UPDATE_CHECK", "1")
            .stdin(Stdio::null());
        if cfg!(feature = "test-harness") {
            command.env("KASSIBER_TEST_NO_EGRESS", "strict");
        }
        command
    };
    let init = command(
        base.path(),
        &["--machine", "--data-root", book.to_str().unwrap(), "init"],
    )
    .output()
    .unwrap();
    assert!(init.status.success(), "{init:?}");
    // The seeded default AI provider is a loopback endpoint; without a
    // default, `ai models` and the like name no provider to contact.
    let cleared = command(
        base.path(),
        &[
            "--machine",
            "--data-root",
            book.to_str().unwrap(),
            "ai",
            "providers",
            "clear-default",
        ],
    )
    .output()
    .unwrap();
    assert!(cleared.status.success(), "{cleared:?}");
    let mut ran = 0;
    for (index, leaf) in kassiber_cli::describe::leaves().iter().enumerate() {
        let dotted = leaf.dotted();
        assert!(leaf.implemented(), "{dotted} is not native");
        if dotted == "operator.unlock" {
            continue;
        }
        let data = base.path().join(format!("book-{index}"));
        let cwd = base.path().join(format!("cwd-{index}"));
        copy_tree(&book, &data);
        std::fs::create_dir_all(&cwd).unwrap();
        let mut args = vec!["--machine", "--data-root", data.to_str().unwrap()];
        args.extend(witness_argv(leaf));
        // Files, not pipes: a large catalog would fill an undrained pipe.
        let (out, err) = (cwd.join("stdout"), cwd.join("stderr"));
        let mut child = command(&cwd, &args)
            .stdout(std::fs::File::create(&out).unwrap())
            .stderr(std::fs::File::create(&err).unwrap())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{dotted} did not finish");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let status = child.wait().unwrap();
        let stdout = std::fs::read_to_string(&out).unwrap();
        let stderr = std::fs::read_to_string(&err).unwrap();
        // Exit 101 is a panic, including the strict egress observer's.
        assert!(
            matches!(status.code(), Some(0..=2)),
            "{dotted}: {status:?} {stdout} {stderr}"
        );
        assert!(
            !stdout.contains("\"code\": \"not_implemented\""),
            "{dotted} answered not_implemented: {stdout}"
        );
        ran += 1;
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&cwd);
    }
    // Every leaf but `operator unlock` ran.
    assert!(ran + 1 == kassiber_cli::describe::leaves().len());
}

#[test]
fn hidden_argv_forms_dispatch_by_name() {
    // `--operator-broker-server` starts a real broker on the account's
    // endpoint, so it runs only against a gated test runtime directory
    // (tests/operator_broker.rs).
    let output = run(&mut kassiber(&["--migrate-default-state-root"]));
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

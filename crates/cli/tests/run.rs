//! `kassiber_cli::run` in process, against the stub and fake stores.

use std::io::Write;
use std::sync::{Arc, Mutex};

use kassiber_cli::{run, Environment, Streams};
use kassiber_runtime::fake::{FakeConfig, FakeStore};
use kassiber_runtime::store::{BookStore, StubStore, UnlockMode};
use kassiber_runtime::BuildInfo;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

const BUILD: BuildInfo = BuildInfo {
    version: "0.22.78",
    channel: None,
    commit: None,
};

fn invoke(store: Arc<dyn BookStore>, env: &Environment, argv: &[&str]) -> (i32, String, String) {
    let out = Captured::default();
    let err = Captured::default();
    let argv: Vec<String> = argv.iter().map(|a| (*a).to_owned()).collect();
    let code = run(
        &argv,
        env,
        store,
        BUILD,
        Streams {
            stdin: Box::new(std::io::empty()),
            stdin_is_tty: false,
            stdout: Box::new(out.clone()),
            stderr: Box::new(err.clone()),
        },
    );
    (code, out.text(), err.text())
}

fn fake(config: FakeConfig) -> Arc<dyn BookStore> {
    Arc::new(FakeStore::new(config))
}

#[test]
fn journal_plain_registration_forwards_selectors_and_finishes_checkpoint() {
    let store = FakeStore::new(FakeConfig::default());
    let (code, out, err) = invoke(
        Arc::new(store.clone()),
        &Environment::default(),
        &[
            "--machine",
            "--data-root",
            "/disposable",
            "journals",
            "process",
            "--workspace",
            "chosen-workspace",
            "--profile",
            "chosen-profile",
        ],
    );
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(out.contains("\"kind\": \"journals.process\""), "{out}");
    assert!(out.contains("\"workspace\": \"chosen-workspace\""), "{out}");
    assert!(out.contains("\"profile\": \"chosen-profile\""), "{out}");
    let events = store.events();
    let index = |event| events.iter().position(|e| e == event).unwrap();
    assert!(index("journals process") < index("journals checkpoint"));
    assert!(index("journals checkpoint") < index("close"));
}

#[test]
fn journal_auto_pair_registration_forwards_flag_and_finishes_checkpoint() {
    let store = FakeStore::new(FakeConfig::default());
    let (code, out, err) = invoke(
        Arc::new(store.clone()),
        &Environment::default(),
        &[
            "--machine",
            "--data-root",
            "/disposable",
            "journals",
            "process",
            "--auto-pair",
        ],
    );
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(out.contains("\"auto_pair\": true"), "{out}");
    let events = store.events();
    let index = |event| events.iter().position(|e| e == event).unwrap();
    assert!(index("journals process") < index("journals checkpoint"));
    assert!(index("journals checkpoint") < index("close"));
}

#[test]
fn hidden_forms_and_unported_leaves_fail_closed() {
    let env = Environment::default();
    let (code, out, err) = invoke(Arc::new(StubStore), &env, &["--operator-broker-server"]);
    assert_eq!((code, out.as_str()), (1, ""));
    assert_eq!(
        err,
        "not_implemented: the operator broker server is not implemented in the Rust backend yet\n"
    );
    let (code, out, _) = invoke(
        Arc::new(StubStore),
        &env,
        &["--machine", "wallets", "kinds"],
    );
    assert_eq!(code, 1);
    assert!(out.contains("\"code\": \"not_implemented\""), "{out}");
    assert!(out.contains("\"command\": \"wallets.kinds\""), "{out}");
    let (code, out, err) = invoke(Arc::new(StubStore), &env, &["wallets", "kinds"]);
    assert_eq!((code, out.as_str()), (1, ""));
    assert_eq!(
        err,
        "error: the command `wallets kinds` is not implemented in the Rust backend yet\n"
    );
}

#[test]
fn status_reads_the_book_in_every_format() {
    let env = Environment::default();
    let config = FakeConfig {
        project_id: Some("proj".into()),
        ..FakeConfig::default()
    };
    let (code, out, err) = invoke(fake(config.clone()), &env, &["--machine", "status"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.starts_with("{\n  \"kind\": \"status\",\n  \"schema_version\": 1,\n  \"data\": {\n    \"version\": \"0.22.78\","));
    assert!(out.contains("\"project_id\": \"proj\""));
    let (code, out, _) = invoke(fake(config.clone()), &env, &["status"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("field "), "{out}");
    assert!(
        out.contains("{'mode': 'local', 'authenticated': True}"),
        "{out}"
    );
    let (code, out, _) = invoke(fake(config.clone()), &env, &["--format", "plain", "status"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("\nauth: {\"mode\": \"local\", \"authenticated\": true}\n"),
        "{out}"
    );
    let (code, out, _) = invoke(fake(config), &env, &["--format", "csv", "status"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("version,schema_version,auth,"), "{out}");
    assert!(
        out.contains(",\"{\"\"mode\"\": \"\"local\"\", \"\"authenticated\"\": true}\","),
        "{out}"
    );
}

#[test]
fn status_migrates_then_routes_then_bootstraps_and_honours_env_file() {
    let env = Environment::default();
    let store = FakeStore::new(FakeConfig {
        workspace: None,
        profile: None,
        ..FakeConfig::default()
    });
    let (code, out, _) = invoke(Arc::new(store.clone()), &env, &["--machine", "status"]);
    assert_eq!(code, 0, "{out}");
    // An unselected workspace or profile is an empty string, as in Python.
    assert!(out.contains("\"current_workspace\": \"\","), "{out}");
    assert!(out.contains("\"current_profile\": \"\","), "{out}");
    assert_eq!(
        store.events()[..3],
        [
            "migrate default state root",
            "unlock mode",
            "bootstrap /fake/kassiber/data"
        ]
    );

    let store = FakeStore::new(FakeConfig::default());
    let (code, out, _) = invoke(
        Arc::new(store.clone()),
        &env,
        &[
            "--machine",
            "--data-root",
            "/d",
            "--env-file",
            "/d/custom.env",
            "status",
        ],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("\"env_file\": \"/d/custom.env\""), "{out}");
    // merge_db_backends supplied the default backend when the book opened.
    assert!(out.contains("\"default_backend\": \"mempool\""), "{out}");
    assert_eq!(
        store.events()[0],
        "unlock mode",
        "no migration with --data-root"
    );
}

#[test]
fn status_errors_are_typed() {
    let env = Environment::default();
    let uninitialized = FakeConfig {
        initialized: false,
        ..FakeConfig::default()
    };
    let (code, out, _) = invoke(fake(uninitialized), &env, &["--machine", "status"]);
    assert_eq!(code, 1);
    assert!(out.contains("\"code\": \"not_initialized\""), "{out}");
    assert!(
        out.contains("\"data_root\": \"/fake/kassiber/data\""),
        "{out}"
    );

    let locked = FakeConfig {
        encrypted: true,
        passphrase: Some("pw".into()),
        ..FakeConfig::default()
    };
    let (code, out, _) = invoke(fake(locked), &env, &["--machine", "status"]);
    assert_eq!(code, 1);
    assert!(out.contains("\"code\": \"interaction_required\""), "{out}");
    assert!(out.contains("\"reason\": \"database_passphrase\""), "{out}");

    let brokered = FakeConfig {
        unlock_mode: UnlockMode::Brokered,
        ..FakeConfig::default()
    };
    // Without a broker, an ordinary command asks for a lease and starts
    // nothing; the client only probes the disposable endpoint.
    let runtime = tempfile::tempdir().unwrap();
    let isolated = Environment {
        operator_runtime: Some(kassiber_operator::runtime::Runtime {
            account_home: None,
            override_dir: Some(runtime.path().join("run").into_os_string()),
            override_gate: Some("1".into()),
            frozen: false,
        }),
        ..Environment::default()
    };
    let (code, out, _) = invoke(fake(brokered.clone()), &isolated, &["--machine", "status"]);
    assert_eq!(code, 1);
    assert!(out.contains("\"code\": \"interaction_required\""), "{out}");
    assert!(
        out.contains("\"reason\": \"operator_lease_required\""),
        "{out}"
    );
    assert_eq!(
        std::fs::read_dir(runtime.path().join("run"))
            .unwrap()
            .count(),
        0,
        "no broker was started"
    );
    let direct = Environment {
        operator_direct: Some("1".into()),
        ..Environment::default()
    };
    assert_eq!(
        invoke(fake(brokered), &direct, &["--machine", "status"]).0,
        0
    );

    let (code, out, _) = invoke(
        fake(FakeConfig::default()),
        &env,
        &["--machine", "--project", "p", "--data-root", "/r", "status"],
    );
    assert_eq!(code, 1);
    assert!(
        out.contains("\"code\": \"invalid_flag_combination\""),
        "{out}"
    );
}

#[test]
fn operator_bindings_are_checked_before_any_store_access() {
    let stray = Environment {
        operator_expected_project_identity: Some("x".into()),
        ..Environment::default()
    };
    let (code, out, _) = invoke(Arc::new(StubStore), &stray, &["--machine", "status"]);
    assert_eq!(code, 1);
    assert!(
        out.contains("\"code\": \"operator_project_binding_invalid\""),
        "{out}"
    );
    assert!(out.contains("only valid in a broker child"), "{out}");
    let child = Environment {
        operator_child: Some("1".into()),
        ..Environment::default()
    };
    let (_, out, _) = invoke(Arc::new(StubStore), &child, &["--machine", "status"]);
    assert!(
        out.contains("operator child project binding is missing"),
        "{out}"
    );
}

#[test]
fn format_rules_match_python() {
    let env = Environment::default();
    let (code, out, err) = invoke(
        Arc::new(StubStore),
        &env,
        &["--machine", "--format", "csv", "status"],
    );
    assert_eq!((code, out.as_str()), (1, ""));
    assert_eq!(
        err,
        "error: --machine requires --format json, got --format csv\n"
    );
    let (code, out, err) = invoke(Arc::new(StubStore), &env, &["--machine", "bogus"]);
    assert_eq!((code, err.as_str()), (2, ""));
    assert!(out.contains("\"code\": \"usage_error\""));
    let (code, out, err) = invoke(Arc::new(StubStore), &env, &["bogus"]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert!(err.starts_with("usage: kassiber [-h]"));
    assert!(
        err.contains("\nkassiber: error: argument command: invalid choice: 'bogus' (choose from 'daemon', 'init', 'status',"),
        "{err}"
    );
    assert!(err.ends_with(", 'ai')\n"), "{err}");
}

#[test]
fn describe_honours_output_and_needs_no_store() {
    let dir = std::env::temp_dir().join(format!("kassiber-cli-run-{}", std::process::id()));
    let target = dir.join("nested").join("catalog.json");
    let env = Environment::default();
    let (code, out, _) = invoke(
        Arc::new(StubStore),
        &env,
        &[
            "--machine",
            "--output",
            target.to_str().unwrap(),
            "commands",
            "describe",
            "status",
        ],
    );
    assert_eq!((code, out.as_str()), (0, ""));
    let written = std::fs::read_to_string(&target).unwrap();
    assert!(written.starts_with("{\n  \"kind\": \"commands.describe\""));
    assert!(written.ends_with("}\n"));
    std::fs::remove_dir_all(dir).unwrap();
    let (code, _, err) = invoke(
        Arc::new(StubStore),
        &env,
        &["--diagnostics-out", "auto", "commands", "describe"],
    );
    assert_eq!(code, 0);
    assert!(err.is_empty());
}

#[cfg(feature = "test-probe")]
#[test]
fn the_probe_reports_implementation_without_running_the_command() {
    let probe = Environment {
        test_probe: true,
        ..Environment::default()
    };
    for (argv, expected, path) in [
        (&["--machine", "status"][..], 0, "status"),
        (&["--machine", "init"][..], 0, "init"),
        (&["--machine", "wallets", "list"][..], 0, "wallets.list"),
        (
            &["--machine", "journals", "process"][..],
            0,
            "journals.process",
        ),
        (
            &["--machine", "journals", "process", "--auto-pair"][..],
            0,
            "journals.process",
        ),
        (
            &["--machine", "--diagnostics-out", "auto", "status"][..],
            0,
            "status",
        ),
        (
            &["--machine", "diagnostics", "collect"][..],
            0,
            "diagnostics.collect",
        ),
        (&["--machine", "wallets", "sync"][..], 0, "wallets.sync"),
    ] {
        let (code, out, err) = invoke(Arc::new(StubStore), &probe, argv);
        assert_eq!(code, expected, "{argv:?}");
        assert_eq!(out, format!("{path}\n"));
        assert!(err.is_empty(), "{argv:?}: {err}");
    }
    let (code, out, _) = invoke(Arc::new(StubStore), &probe, &["--machine", "bogus"]);
    assert_eq!(code, 2);
    assert!(out.contains("\"code\": \"usage_error\""), "{out}");
}

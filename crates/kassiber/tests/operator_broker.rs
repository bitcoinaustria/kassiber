//! `kassiber --operator-broker-server` end to end: the Rust client against
//! the Rust broker, whose children are this binary (the real-child cases of
//! `tests/test_operator_service.py` and `tests/test_operator_integration.py`).
//!
//! Every broker runs in a disposable 0700 runtime directory through the
//! gated test override, with the integration fixture's login-session guard
//! (`KASSIBER_RUST_TEST_OPERATOR_SESSION=fixture`) and an isolated owner
//! root. HOME is left alone; books are explicit `--data-root`s.
#![cfg(all(unix, feature = "test-harness"))]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use kassiber_operator::client::{BrokerClient, Host, PreparedArguments, SubmitOptions, SystemHost};
use kassiber_operator::runtime::Runtime;
use kassiber_pyjson::{PyStr, Value};

const PASSPHRASE: &str = "correct horse battery staple";

/// The binary under test; every process these tests start.
fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kassiber"));
    command.env_remove("KASSIBER_RUST_TEST_FAKE_STORE");
    command
}

struct Broker {
    runtime_dir: tempfile::TempDir,
    owners: tempfile::TempDir,
    process: Option<Child>,
}

fn private_dir(prefix: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

impl Broker {
    fn environment(&self, command: &mut Command) {
        command
            .env("KASSIBER_OPERATOR_RUNTIME_DIR", self.runtime_dir.path())
            .env("KASSIBER_OPERATOR_ALLOW_TEST_RUNTIME_DIR", "1")
            .env("XDG_RUNTIME_DIR", self.runtime_dir.path())
            .env("KASSIBER_RUST_TEST_OPERATOR_SESSION", "fixture")
            .env("KASSIBER_RUST_TEST_OWNER_ROOT", self.owners.path())
            .env("KASSIBER_TEST_NO_EGRESS", "strict")
            .env_remove("KASSIBER_NO_EGRESS")
            .env_remove("KASSIBER_OPERATOR_DIRECT")
            .env_remove("KASSIBER_OPERATOR_CHILD");
    }

    fn command(&self) -> Command {
        let mut command = binary();
        command.arg("--operator-broker-server");
        self.environment(&mut command);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        command
    }

    fn start() -> Broker {
        let mut broker = Broker {
            runtime_dir: private_dir("kb-rt-"),
            owners: private_dir("kb-own-"),
            process: None,
        };
        broker.process = Some(broker.command().spawn().unwrap());
        broker.wait_for_ping();
        broker
    }

    fn runtime(&self) -> Runtime {
        Runtime {
            account_home: None,
            override_dir: Some(self.runtime_dir.path().as_os_str().to_owned()),
            override_gate: Some("1".into()),
            frozen: false,
        }
    }

    /// A client of this broker's build (the Python integration switch does
    /// the same: the test process is not the broker's executable).
    fn client(&self) -> BrokerClient<SystemHost> {
        let probe = BrokerClient::new(SystemHost {
            runtime: self.runtime(),
            build: Value::Null,
            executable: PathBuf::from(env!("CARGO_BIN_EXE_kassiber")),
        });
        let build = get(&probe.ping().unwrap(), "build").clone();
        BrokerClient::new(SystemHost {
            runtime: self.runtime(),
            build,
            executable: PathBuf::from(env!("CARGO_BIN_EXE_kassiber")),
        })
    }

    fn wait_for_ping(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let probe = BrokerClient::new(SystemHost {
            runtime: self.runtime(),
            build: Value::Null,
            executable: PathBuf::from(env!("CARGO_BIN_EXE_kassiber")),
        });
        while probe.ping().is_err() {
            if let Some(status) = self.process.as_mut().unwrap().try_wait().unwrap() {
                let mut stderr = String::new();
                use std::io::Read;
                self.process
                    .as_mut()
                    .unwrap()
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                panic!("the broker exited before rendezvous ({status}): {stderr}");
            }
            assert!(Instant::now() < deadline, "the broker did not rendezvous");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// A direct run of this binary on the book, with the passphrase on
    /// stdin as descriptor 0.
    fn direct(&self, data_root: &Path, args: &[&str]) -> Output {
        let mut command = binary();
        command
            .arg("--data-root")
            .arg(data_root)
            .args(["--machine", "--db-passphrase-fd", "0"])
            .args(args);
        self.environment(&mut command);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(PASSPHRASE.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    kassiber_envelope::get(value, key).unwrap_or_else(|| panic!("{key} missing in {value:?}"))
}

fn text(value: &Value, key: &str) -> String {
    kassiber_envelope::py_str(get(value, key))
}

fn json(text: &str) -> Value {
    kassiber_pytext::loads(text).unwrap_or_else(|_| panic!("not JSON: {text}"))
}

/// An encrypted book with one workspace and one profile; their ids.
fn book(broker: &Broker) -> (tempfile::TempDir, String, String) {
    let data_root = tempfile::tempdir().unwrap();
    kassiber_store::encryption::create_empty_database(
        &kassiber_paths::resolve_database_path(data_root.path()),
        PASSPHRASE,
    )
    .unwrap();
    let workspace = broker.direct(data_root.path(), &["workspaces", "create", "Workspace A"]);
    assert!(workspace.status.success(), "{workspace:?}");
    let workspace = text(
        get(&json(&String::from_utf8_lossy(&workspace.stdout)), "data"),
        "id",
    );
    let profile = broker.direct(
        data_root.path(),
        &[
            "profiles",
            "create",
            "--workspace",
            &workspace,
            "--fiat-currency",
            "EUR",
            "Book A",
        ],
    );
    assert!(profile.status.success(), "{profile:?}");
    let profile = text(
        get(&json(&String::from_utf8_lossy(&profile.stdout)), "data"),
        "id",
    );
    (data_root, workspace, profile)
}

fn root(dir: &tempfile::TempDir) -> String {
    dir.path().to_string_lossy().into_owned()
}

fn argv(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn unlock(client: &BrokerClient<SystemHost>, data_root: &str, passphrase: &str) -> Value {
    client
        .unlock(
            data_root,
            passphrase.as_bytes(),
            None,
            "accounting_decisions",
            "password",
            false,
            None,
            None,
        )
        .unwrap()
}

/// A client whose working directory is `directory` rather than this
/// process's, which parallel tests share.
struct Caller {
    inner: SystemHost,
    directory: PathBuf,
}

impl Host for Caller {
    fn connect(
        &self,
        timeout: Duration,
        io_timeout: Option<Duration>,
    ) -> kassiber_operator::Result<Box<dyn kassiber_operator::protocol::FrameChannel>> {
        self.inner.connect(timeout, io_timeout)
    }
    fn launch_broker(&self) -> kassiber_operator::Result<()> {
        self.inner.launch_broker()
    }
    fn startup_timeout(&self) -> Duration {
        self.inner.startup_timeout()
    }
    fn build_identity(&self) -> Value {
        self.inner.build_identity()
    }
    fn token_hex(&self, bytes: usize) -> kassiber_operator::Result<String> {
        self.inner.token_hex(bytes)
    }
    fn current_dir(&self) -> std::io::Result<PyStr> {
        Ok(PyStr::from(self.directory.to_string_lossy().as_ref()))
    }
    fn env_var(&self, key: &str) -> Option<String> {
        self.inner.env_var(key)
    }
    fn now(&self) -> Instant {
        self.inner.now()
    }
    fn sleep(&self, duration: Duration) {
        self.inner.sleep(duration)
    }
    fn warn(&self, text: &str) {
        self.inner.warn(text)
    }
}

fn run<H: Host>(
    client: &BrokerClient<H>,
    data_root: &str,
    prepared: &PreparedArguments,
    admin: Option<&[u8]>,
) -> Value {
    let accepted = client
        .submit(data_root, prepared, admin, &SubmitOptions::new())
        .unwrap();
    let id = match get(&accepted, "operation_id") {
        Value::Str(id) => id.clone(),
        other => panic!("operation id {other:?}"),
    };
    client.wait(&id).unwrap()
}

#[test]
fn password_unlock_submit_status_and_lock() {
    let broker = Broker::start();
    let (data_root, workspace, profile) = book(&broker);
    let client = broker.client();
    let unlocked = unlock(&client, &root(&data_root), PASSPHRASE);
    assert_eq!(text(&unlocked, "lease"), "unlocked");
    assert_eq!(text(&unlocked, "authentication_method"), "password");
    assert!(!format!("{unlocked:?}").contains(&root(&data_root)));
    let status = run(
        &client,
        &root(&data_root),
        &PreparedArguments::new(argv(&[
            "--data-root",
            &root(&data_root),
            "--machine",
            "status",
        ])),
        None,
    );
    assert_eq!(text(&status, "state"), "completed", "{status:?}");
    let stdout = text(&status, "stdout");
    assert_eq!(text(&json(&stdout), "kind"), "status");
    assert!(!stdout.contains(PASSPHRASE));
    assert!(!text(&status, "stderr").contains(PASSPHRASE));
    let second = broker.client();
    let profiles = run(
        &second,
        &root(&data_root),
        &PreparedArguments::new(argv(&[
            "--data-root",
            &root(&data_root),
            "--machine",
            "profiles",
            "get",
            "--workspace",
            &workspace,
            "--profile",
            &profile,
        ])),
        None,
    );
    assert_eq!(text(&profiles, "state"), "completed", "{profiles:?}");
    assert_eq!(
        text(get(&json(&text(&profiles, "stdout")), "data"), "id"),
        profile
    );
    let locked = client.lock(&root(&data_root), None, false).unwrap();
    assert_eq!(get(&locked, "locked"), &Value::Bool(true));
    assert_eq!(
        text(&client.status(Some(&root(&data_root))).unwrap(), "lease"),
        "locked"
    );
}

#[test]
fn a_child_writes_relative_output_in_the_callers_directory() {
    let broker = Broker::start();
    let (data_root, _, _) = book(&broker);
    let caller = tempfile::tempdir().unwrap();
    let client = broker.client();
    unlock(&client, &root(&data_root), PASSPHRASE);
    // The client sends its own working directory: submit through a client
    // whose directory is the caller's.
    let client = BrokerClient::new(Caller {
        inner: SystemHost {
            runtime: broker.runtime(),
            build: client.host().build_identity(),
            executable: PathBuf::from(env!("CARGO_BIN_EXE_kassiber")),
        },
        directory: caller.path().to_owned(),
    });
    let completed = run(
        &client,
        &root(&data_root),
        &PreparedArguments::new(argv(&[
            "--data-root",
            &root(&data_root),
            "--machine",
            "--output",
            "status.json",
            "status",
        ])),
        None,
    );
    assert_eq!(text(&completed, "state"), "completed", "{completed:?}");
    let written = std::fs::read_to_string(caller.path().join("status.json")).unwrap();
    assert_eq!(text(&json(&written), "kind"), "status");
    assert!(!Path::new(env!("CARGO_BIN_EXE_kassiber"))
        .parent()
        .unwrap()
        .join("status.json")
        .exists());
}

#[test]
fn large_command_secrets_are_fed_concurrently() {
    let broker = Broker::start();
    let (data_root, _, _) = book(&broker);
    let client = broker.client();
    unlock(&client, &root(&data_root), PASSPHRASE);
    let mut prepared = PreparedArguments::new(argv(&[
        "--data-root",
        &root(&data_root),
        "--machine",
        "backends",
        "create",
        "multi-secret",
        "--kind",
        "esplora",
        "--url",
        "http://127.0.0.1:3002",
        "--token-fd",
        "broker-secret-token",
        "--auth-header-fd",
        "broker-secret-auth-header",
    ]));
    prepared.secrets = vec![
        (
            "broker-secret-token".into(),
            zeroize::Zeroizing::new(vec![b't'; 5000]),
        ),
        (
            "broker-secret-auth-header".into(),
            zeroize::Zeroizing::new(vec![b'a'; 5000]),
        ),
    ];
    let completed = run(&client, &root(&data_root), &prepared, None);
    assert_eq!(text(&completed, "state"), "completed", "{completed:?}");
    assert!(!text(&completed, "stdout").contains(&"t".repeat(100)));
}

#[test]
fn brokered_passphrase_rotation_revokes_the_stale_lease() {
    let broker = Broker::start();
    let (data_root, _, _) = book(&broker);
    let client = broker.client();
    unlock(&client, &root(&data_root), PASSPHRASE);
    let mut prepared = PreparedArguments::new(argv(&[
        "--machine",
        "secrets",
        "change-passphrase",
        "--new-passphrase-fd",
        "broker-secret-new-passphrase",
    ]));
    prepared.secrets = vec![(
        "broker-secret-new-passphrase".into(),
        zeroize::Zeroizing::new(b"new correct horse battery staple".to_vec()),
    )];
    let completed = run(
        &client,
        &root(&data_root),
        &prepared,
        Some(PASSPHRASE.as_bytes()),
    );
    assert_eq!(text(&completed, "state"), "completed", "{completed:?}");
    assert_eq!(
        text(&client.status(Some(&root(&data_root))).unwrap(), "lease"),
        "locked"
    );
    assert!(!broker
        .direct(data_root.path(), &["status"])
        .status
        .success());
}

#[test]
fn admin_work_needs_fresh_authentication_and_reports_typed_errors() {
    let broker = Broker::start();
    let (data_root, _, _) = book(&broker);
    let client = broker.client();
    unlock(&client, &root(&data_root), PASSPHRASE);
    let prepared = PreparedArguments::new(argv(&["--machine", "secrets", "verify"]));
    let error = client
        .submit(
            &root(&data_root),
            &prepared,
            Some(b"wrong passphrase"),
            &SubmitOptions::new(),
        )
        .unwrap_err();
    assert_eq!(error.code(), Some("unlock_failed"));
    let verified = run(
        &client,
        &root(&data_root),
        &prepared,
        Some(PASSPHRASE.as_bytes()),
    );
    assert_eq!(text(&verified, "state"), "completed", "{verified:?}");
    // Scope is required, the lease grants nothing admin, and a command for
    // another project is refused.
    let error = client
        .submit(
            &root(&data_root),
            &PreparedArguments::new(argv(&["--machine", "transactions", "list"])),
            None,
            &SubmitOptions::new(),
        )
        .unwrap_err();
    assert_eq!(error.code(), Some("operator_scope_required"));
    let other = tempfile::tempdir().unwrap();
    let error = client
        .submit(
            &root(&data_root),
            &PreparedArguments::new(argv(&["--data-root", &root(&other), "--machine", "status"])),
            None,
            &SubmitOptions::new(),
        )
        .unwrap_err();
    assert_eq!(error.code(), Some("operator_project_mismatch"));
    let details = format!("{:?}", error.app().unwrap().details);
    assert!(!details.contains(&root(&other)) && !details.contains(&root(&data_root)));
}

#[test]
fn two_projects_and_their_books_remain_independent() {
    let broker = Broker::start();
    let (first, first_workspace, first_profile) = book(&broker);
    let (second, second_workspace, second_profile) = book(&broker);
    let client = broker.client();
    let one = unlock(&client, &root(&first), PASSPHRASE);
    let two = unlock(&client, &root(&second), PASSPHRASE);
    assert_ne!(text(&one, "project"), text(&two, "project"));
    for (data_root, workspace, profile) in [
        (&first, &first_workspace, &first_profile),
        (&second, &second_workspace, &second_profile),
    ] {
        let completed = run(
            &client,
            &root(data_root),
            &PreparedArguments::new(argv(&[
                "--data-root",
                &root(data_root),
                "--machine",
                "profiles",
                "get",
                "--workspace",
                workspace,
                "--profile",
                profile,
            ])),
            None,
        );
        assert_eq!(text(&completed, "state"), "completed");
        assert_eq!(
            text(get(&json(&text(&completed, "stdout")), "data"), "id"),
            **profile
        );
    }
    client.lock(&root(&first), None, false).unwrap();
    assert_eq!(
        text(&client.status(Some(&root(&first))).unwrap(), "lease"),
        "locked"
    );
    assert_eq!(
        text(&client.status(Some(&root(&second))).unwrap(), "lease"),
        "unlocked"
    );
    let all = client.status(None).unwrap();
    match get(&all, "leases") {
        Value::List(leases) => assert_eq!(leases.len(), 1),
        other => panic!("{other:?}"),
    }
}

#[test]
fn simultaneous_startup_elects_one_broker() {
    let mut broker = Broker {
        runtime_dir: private_dir("kb-rt-"),
        owners: private_dir("kb-own-"),
        process: None,
    };
    let mut brokers: Vec<Child> = (0..2).map(|_| broker.command().spawn().unwrap()).collect();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut loser = None;
    while loser.is_none() && Instant::now() < deadline {
        for (index, child) in brokers.iter_mut().enumerate() {
            if let Some(status) = child.try_wait().unwrap() {
                loser = Some((index, status));
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let (index, status) = loser.expect("one broker should lose the election");
    assert_eq!(status.code(), Some(1));
    let survivor = brokers.remove(1 - index);
    broker.process = Some(survivor);
    broker.wait_for_ping();
}

#[test]
fn a_child_refuses_a_database_whose_identity_changed() {
    let broker = Broker::start();
    let (data_root, _, _) = book(&broker);
    let canonical = std::fs::canonicalize(data_root.path()).unwrap();
    let project = kassiber_ownership::OwnerNamespace::isolated(broker.owners.path().to_owned())
        .canonical_project(&canonical)
        .unwrap();
    for (expected_identity, code) in [
        (&"f".repeat(32), "operator_project_replaced"),
        (&"0".repeat(32), "operator_project_replaced"),
    ] {
        let mut command = binary();
        broker.environment(&mut command);
        command
            .env("KASSIBER_OPERATOR_DIRECT", "1")
            .env("KASSIBER_OPERATOR_CHILD", "1")
            .env(
                "KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY",
                &project.identity,
            )
            .env(
                "KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY",
                expected_identity,
            )
            .arg("--data-root")
            .arg(&canonical)
            .args([
                "--machine",
                "--db-passphrase-fd",
                "0",
                "secrets",
                "forget-unlock",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(PASSPHRASE.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let envelope = json(&String::from_utf8_lossy(&output.stdout));
        assert_eq!(text(get(&envelope, "error"), "code"), code, "{envelope:?}");
    }
    // A project binding naming another file is refused before anything opens.
    let mut command = binary();
    broker.environment(&mut command);
    let output = command
        .env("KASSIBER_OPERATOR_DIRECT", "1")
        .env("KASSIBER_OPERATOR_CHILD", "1")
        .env(
            "KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY",
            "a".repeat(64),
        )
        .env(
            "KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY",
            "a".repeat(32),
        )
        .arg("--data-root")
        .arg(&canonical)
        .args(["--machine", "status"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let envelope = json(&String::from_utf8_lossy(&output.stdout));
    assert_eq!(
        text(get(&envelope, "error"), "code"),
        "operator_project_replaced"
    );
    assert_eq!(
        text(get(get(&envelope, "error"), "details"), "project"),
        project.public_id
    );
    // A vanished caller directory is reported on stdout, never written to
    // a relative --output.
    let output = binary()
        .env("KASSIBER_OPERATOR_CHILD", "1")
        .env("KASSIBER_OPERATOR_DIRECT", "1")
        .env(
            "KASSIBER_OPERATOR_WORKING_DIRECTORY",
            data_root.path().join("gone"),
        )
        .env(
            "KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY",
            &project.identity,
        )
        .env(
            "KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY",
            "a".repeat(32),
        )
        .arg("--data-root")
        .arg(&canonical)
        .args(["--machine", "--output", "out.json", "status"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let envelope = json(&String::from_utf8_lossy(&output.stdout));
    assert_eq!(
        text(get(&envelope, "error"), "code"),
        "operator_working_directory_unavailable"
    );
    let _ = PyStr::from("");
}

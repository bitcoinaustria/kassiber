//! `tests/test_operator_protocol.py`, ported: the endpoint, its election,
//! stale-socket recovery and the peer checks. Every runtime directory is a
//! disposable directory; none of these tests touches the account home.
#![cfg(unix)]

use kassiber_ipc::unix::{current_uid, PeerCredentials};
use kassiber_operator::client::{BrokerClient, Host, PreparedArguments, PING_TIMEOUT};
use kassiber_operator::protocol::FrameChannel;
use kassiber_operator::runtime::{
    connect, ensure_private_directory, listen, remove_stale_socket, stale_probe_allows_unlink,
    verify_unix_peer, Runtime, SOCKET_FILENAME,
};
use kassiber_operator::Error;
use kassiber_pyjson::{PyStr, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn parse(text: &str) -> Value {
    kassiber_pytext::loads(text).unwrap()
}

/// A private directory with a short path: AF_UNIX paths stop at 104 bytes
/// on macOS and 108 on Linux. macOS's per-user `TMPDIR` under
/// `/private/var/folders/…` alone leaves too little room, so use `/tmp` there.
fn private_dir() -> tempfile::TempDir {
    let mut builder = tempfile::Builder::new();
    builder.prefix("kb-op-");
    #[cfg(target_os = "macos")]
    let dir = builder.tempdir_in("/tmp").unwrap();
    #[cfg(not(target_os = "macos"))]
    let dir = builder.tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

fn overridden(dir: &Path) -> Runtime {
    Runtime {
        account_home: None,
        override_dir: Some(dir.as_os_str().to_owned()),
        override_gate: Some("1".into()),
        frozen: false,
    }
}

fn code(error: &Error) -> &str {
    error.code().unwrap_or("<not an AppError>")
}

#[test]
fn listener_authenticates_same_user_and_round_trips() {
    let dir = private_dir();
    let runtime = overridden(dir.path());
    let mut server = listen(&runtime).unwrap();
    let accepted = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            let mut channel = server.accept().unwrap();
            let request = channel.receive_json().unwrap();
            channel.send_json(&parse(r#"{"ok": true}"#)).unwrap();
            (request, channel.peer_pid())
        });
        let mut channel = connect(&runtime, Duration::from_secs(2), None).unwrap();
        channel.send_json(&parse(r#"{"ping": true}"#)).unwrap();
        assert_eq!(channel.receive_json().unwrap(), parse(r#"{"ok": true}"#));
        worker.join().unwrap()
    });
    server.close().unwrap();
    assert_eq!(accepted.0, parse(r#"{"ping": true}"#));
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    assert_eq!(accepted.1, Some(std::process::id()));
    assert!(!dir.path().join(SOCKET_FILENAME).exists());
}

#[test]
fn permissive_runtime_directory_is_rejected() {
    let dir = private_dir();
    for (mode, accepted) in [
        (0o700, true),
        (0o755, false),
        (0o750, false),
        (0o710, false),
        (0o701, false),
        (0o707, false),
    ] {
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode)).unwrap();
        let result = overridden(dir.path()).runtime_dir();
        assert_eq!(result.is_ok(), accepted, "mode {mode:o}");
        if let Err(error) = result {
            assert_eq!(code(&error), "unsafe_operator_runtime_directory");
            assert_eq!(
                error.to_string(),
                "operator runtime directory permissions are too broad"
            );
            assert_eq!(
                error.app().unwrap().hint.as_deref(),
                Some("Restrict the directory to mode 0700 and retry.")
            );
        }
    }
}

#[test]
fn runtime_directory_of_another_user_is_rejected() {
    if current_uid() == 0 {
        return;
    }
    // The root directory exists and belongs to uid 0, never to this test.
    let error = ensure_private_directory(Path::new("/")).unwrap_err();
    assert_eq!(code(&error), "unsafe_operator_runtime_directory");
    assert_eq!(
        error.to_string(),
        "operator runtime directory is owned by another OS user"
    );
}

#[test]
fn runtime_directory_symlinks_and_files_are_refused() {
    let dir = private_dir();
    let target = dir.path().join("target");
    std::fs::create_dir(&target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let error = ensure_private_directory(&link).unwrap_err();
    assert_eq!(code(&error), "unsafe_operator_runtime_directory");
    assert_eq!(
        error.to_string(),
        "operator runtime directory may not be a symlink"
    );
    // An existing file fails at mkdir, as Python's FileExistsError does.
    let file = dir.path().join("file");
    std::fs::write(&file, b"").unwrap();
    let error = ensure_private_directory(&file).unwrap_err();
    assert_eq!(code(&error), "operator_runtime_unavailable");
    assert!(error.app().unwrap().retryable);
}

#[test]
fn missing_parents_get_default_modes_and_the_leaf_is_private() {
    let dir = private_dir();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let runtime = Runtime {
        account_home: Some(home.clone()),
        override_dir: None,
        override_gate: None,
        frozen: true,
    };
    let selected = runtime.runtime_dir().unwrap();
    assert_eq!(selected, home.join(".kassiber/run").canonicalize().unwrap());
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&selected), 0o700);
    // Python's mkdir(parents=True) creates parents without the leaf's mode.
    let umask_default = {
        let probe = dir.path().join("probe");
        std::fs::create_dir(&probe).unwrap();
        mode(&probe)
    };
    assert_eq!(mode(&home.join(".kassiber")), umask_default);
}

#[test]
fn owned_stale_socket_is_recovered() {
    let dir = private_dir();
    let endpoint = dir.path().join(SOCKET_FILENAME);
    drop(kassiber_ipc::unix::bind_listener(&endpoint, 0o600, 1).unwrap());
    assert!(endpoint.exists());
    let mut server = listen(&overridden(dir.path())).unwrap();
    assert!(endpoint.exists());
    server.close().unwrap();
}

#[test]
fn live_endpoints_and_non_sockets_are_never_unlinked() {
    let dir = private_dir();
    let endpoint = dir.path().join(SOCKET_FILENAME);
    let live = kassiber_ipc::unix::bind_listener(&endpoint, 0o600, 4).unwrap();
    let error = remove_stale_socket(&endpoint).unwrap_err();
    assert_eq!(code(&error), "operator_broker_running");
    assert_eq!(error.to_string(), "the operator broker is already running");
    assert!(endpoint.exists());
    drop(live);
    std::fs::remove_file(&endpoint).unwrap();
    std::fs::write(&endpoint, b"not a socket").unwrap();
    let error = remove_stale_socket(&endpoint).unwrap_err();
    assert_eq!(code(&error), "unsafe_operator_endpoint");
    assert_eq!(error.to_string(), "operator endpoint is not a socket");
    assert!(endpoint.exists());
    std::fs::remove_file(&endpoint).unwrap();
    std::os::unix::fs::symlink(dir.path().join("absent"), &endpoint).unwrap();
    let error = listen(&overridden(dir.path())).err().unwrap();
    assert_eq!(code(&error), "unsafe_operator_endpoint");
    assert!(endpoint.is_symlink());
}

#[test]
fn only_refused_or_vanished_probes_allow_the_unlink() {
    use std::io::{Error as IoError, ErrorKind};
    for (kind, unlink) in [
        (ErrorKind::ConnectionRefused, true),
        (ErrorKind::NotFound, true),
        (ErrorKind::TimedOut, false),
        (ErrorKind::WouldBlock, false),
        (ErrorKind::PermissionDenied, false),
        (ErrorKind::Other, false),
    ] {
        assert_eq!(
            stale_probe_allows_unlink(&Err(IoError::from(kind))),
            unlink,
            "{kind:?}"
        );
    }
    let dir = private_dir();
    let endpoint = dir.path().join("probe.sock");
    let _live = kassiber_ipc::unix::bind_listener(&endpoint, 0o600, 1).unwrap();
    let connected = kassiber_ipc::unix::connect(&endpoint, Some(Duration::from_secs(1)));
    assert!(!stale_probe_allows_unlink(&connected));
}

#[test]
fn startup_lock_prevents_a_second_listener_from_touching_endpoint() {
    let dir = private_dir();
    let mut server = listen(&overridden(dir.path())).unwrap();
    let error = listen(&overridden(dir.path())).err().unwrap();
    assert_eq!(code(&error), "operator_broker_running");
    assert_eq!(
        error.to_string(),
        "the operator broker is already starting or running"
    );
    assert!(dir.path().join(SOCKET_FILENAME).exists());
    server.close().unwrap();
    // Closing released the election.
    listen(&overridden(dir.path())).unwrap().close().unwrap();
}

#[test]
fn listener_close_does_not_unlink_a_replacement_endpoint() {
    let dir = private_dir();
    let mut server = listen(&overridden(dir.path())).unwrap();
    let endpoint = dir.path().join(SOCKET_FILENAME);
    let displaced = dir.path().join("displaced.sock");
    std::fs::rename(&endpoint, &displaced).unwrap();
    let replacement = kassiber_ipc::unix::bind_listener(&endpoint, 0o600, 1).unwrap();
    server.close().unwrap();
    assert!(endpoint.exists());
    drop(replacement);
}

#[test]
fn production_environment_variation_cannot_split_broker_election() {
    // Different HOME, XDG_RUNTIME_DIR and an ungated override: the account
    // home alone decides, so both runtimes meet at one endpoint.
    let dir = private_dir();
    let account = dir.path().join("account");
    std::fs::create_dir(&account).unwrap();
    std::fs::set_permissions(&account, std::fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = |override_dir: &str| Runtime {
        account_home: Some(account.clone()),
        override_dir: Some(dir.path().join(override_dir).into_os_string()),
        override_gate: Some("0".into()),
        frozen: false,
    };
    let mut server = listen(&runtime("override-one")).unwrap();
    let error = listen(&runtime("override-two")).err().unwrap();
    assert_eq!(code(&error), "operator_broker_running");
    let received = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            let mut channel = server.accept().unwrap();
            let request = channel.receive_json().unwrap();
            channel.send_json(&parse(r#"{"winner": true}"#)).unwrap();
            request
        });
        let mut channel = connect(&runtime("override-two"), Duration::from_secs(2), None).unwrap();
        channel
            .send_json(&parse(r#"{"same_account": true}"#))
            .unwrap();
        assert_eq!(
            channel.receive_json().unwrap(),
            parse(r#"{"winner": true}"#)
        );
        worker.join().unwrap()
    });
    assert_eq!(received, parse(r#"{"same_account": true}"#));
    server.close().unwrap();
    assert!(!dir.path().join("override-one").exists());
}

#[test]
fn runtime_override_requires_explicit_nonfrozen_test_gate() {
    let dir = private_dir();
    let account = dir.path().join("account");
    std::fs::create_dir(&account).unwrap();
    let override_dir = private_dir();
    let account_runtime = account.join(".kassiber/run");
    let runtime = |gate: &str, frozen: bool| Runtime {
        account_home: Some(account.clone()),
        override_dir: Some(override_dir.path().as_os_str().to_owned()),
        override_gate: Some(gate.into()),
        frozen,
    };
    assert_eq!(
        runtime("0", false).runtime_dir().unwrap(),
        account_runtime.canonicalize().unwrap()
    );
    assert_eq!(
        runtime("1", false).runtime_dir().unwrap(),
        override_dir.path().canonicalize().unwrap()
    );
    assert_eq!(
        runtime("1", true).runtime_dir().unwrap(),
        account_runtime.canonicalize().unwrap()
    );
    assert_eq!(
        runtime("true", false).runtime_dir().unwrap(),
        account_runtime.canonicalize().unwrap()
    );
    // An empty override is no override.
    let empty = Runtime {
        override_dir: Some("".into()),
        ..runtime("1", false)
    };
    assert_eq!(
        empty.runtime_dir().unwrap(),
        account_runtime.canonicalize().unwrap()
    );
}

#[test]
fn account_home_failures_are_typed() {
    let missing = Runtime {
        account_home: None,
        override_dir: None,
        override_gate: None,
        frozen: true,
    };
    let error = missing.runtime_dir().unwrap_err();
    assert_eq!(code(&error), "operator_runtime_unavailable");
    assert_eq!(
        error.to_string(),
        "the operator account home is unavailable"
    );
    let relative = Runtime {
        account_home: Some(PathBuf::from("relative/home")),
        ..missing
    };
    let error = relative.runtime_dir().unwrap_err();
    assert_eq!(code(&error), "unsafe_operator_runtime_directory");
    assert_eq!(error.to_string(), "the operator account home is unsafe");
}

#[test]
fn cross_user_unix_peer_is_rejected() {
    let uid = current_uid();
    let other = PeerCredentials {
        uid: uid.wrapping_add(1),
        pid: Some(1234),
    };
    let error = verify_unix_peer(Ok(other), uid).unwrap_err();
    assert_eq!(code(&error), "operator_peer_rejected");
    assert_eq!(
        error.to_string(),
        "operator IPC peer is a different OS user"
    );
    assert_eq!(
        verify_unix_peer(Ok(PeerCredentials { uid, pid: Some(77) }), uid).unwrap(),
        Some(77)
    );
    assert_eq!(
        verify_unix_peer(Ok(PeerCredentials { uid, pid: None }), uid).unwrap(),
        None
    );
    let error = verify_unix_peer(
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported)),
        uid,
    )
    .unwrap_err();
    assert_eq!(code(&error), "operator_peer_credentials_unavailable");
}

#[test]
fn windows_contract_names_acl_and_bilateral_sid_checks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let pipe = std::fs::read_to_string(root.join("ipc/src/windows/pipe.rs")).unwrap();
    for primitive in [
        "PIPE_REJECT_REMOTE_CLIENTS",
        "FILE_FLAG_FIRST_PIPE_INSTANCE",
        "ConvertStringSecurityDescriptorToSecurityDescriptorW",
        "GetNamedPipeClientProcessId",
        "GetNamedPipeServerProcessId",
        "D:P(A;;GA;;;{sid})",
        "operator named-pipe read timed out",
        "operator named-pipe write timed out",
        "FILE_FLAG_OVERLAPPED",
        "GetOverlappedResultEx",
        "CancelIoEx",
    ] {
        assert!(pipe.contains(primitive), "{primitive}");
    }
    let runtime = std::fs::read_to_string(root.join("operator/src/runtime.rs")).unwrap();
    let accept = runtime
        .split("impl WindowsBrokerListener {")
        .nth(1)
        .unwrap()
        .split("pub fn close")
        .next()
        .unwrap();
    assert!(accept.contains("Some(DEFAULT_WINDOWS_IO_TIMEOUT)"));
    assert!(accept.contains("operator named-pipe client SID did not match"));
    assert!(runtime.contains("operator named-pipe server SID did not match"));
    let process = std::fs::read_to_string(root.join("ipc/src/windows/process.rs")).unwrap();
    assert!(process.contains("PROC_THREAD_ATTRIBUTE_HANDLE_LIST"));
}

/// A host whose broker is whatever listens at a test runtime directory.
struct TestHost(Runtime);

impl Host for TestHost {
    fn connect(
        &self,
        timeout: Duration,
        io_timeout: Option<Duration>,
    ) -> kassiber_operator::Result<Box<dyn FrameChannel>> {
        connect(&self.0, timeout, io_timeout)
    }
    fn launch_broker(&self) -> kassiber_operator::Result<()> {
        panic!("this test must not launch a broker")
    }
    fn startup_timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
    fn build_identity(&self) -> Value {
        parse(r#"{"version": "test"}"#)
    }
    fn token_hex(&self, _: usize) -> kassiber_operator::Result<String> {
        Ok("0".repeat(32))
    }
    fn current_dir(&self) -> std::io::Result<PyStr> {
        Ok(PyStr::from("/"))
    }
    fn env_var(&self, _: &str) -> Option<String> {
        None
    }
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration)
    }
    fn warn(&self, _: &str) {}
}

fn wedged_endpoint(dir: &Path, body: impl FnOnce(&BrokerClient<TestHost>) + Send) {
    let endpoint = dir.join(SOCKET_FILENAME);
    let listener = kassiber_ipc::unix::bind_listener(&endpoint, 0o600, 1).unwrap();
    let (release, wait) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let (_connection, _) = listener.accept().unwrap();
            let _ = wait.recv_timeout(Duration::from_secs(5));
        });
        body(&BrokerClient::new(TestHost(overridden(dir))));
        release.send(()).unwrap();
    });
}

#[test]
fn ping_read_is_bounded_when_endpoint_accepts_but_never_replies() {
    let dir = private_dir();
    wedged_endpoint(dir.path(), |client| {
        let started = Instant::now();
        let error = client.ping().unwrap_err();
        let elapsed = started.elapsed();
        assert!(error.is_timeout(), "{error:?}");
        assert!(elapsed < Duration::from_millis(1500), "{elapsed:?}");
        assert!(
            elapsed >= PING_TIMEOUT - Duration::from_millis(50),
            "{elapsed:?}"
        );
    });
}

#[test]
fn simple_requests_are_bounded_at_two_seconds() {
    let dir = private_dir();
    wedged_endpoint(dir.path(), |client| {
        let started = Instant::now();
        let error = client.cancel(&PyStr::from("g.client.x")).unwrap_err();
        let elapsed = started.elapsed();
        assert!(error.is_timeout(), "{error:?}");
        assert!(elapsed >= Duration::from_millis(1950), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(3000), "{elapsed:?}");
    });
}

#[test]
fn status_and_lock_report_a_stopped_broker_without_launching() {
    let dir = private_dir();
    let client = BrokerClient::new(TestHost(overridden(dir.path())));
    assert_eq!(
        client.status(None).unwrap(),
        parse(r#"{"broker": "stopped", "lease": "locked"}"#)
    );
    assert_eq!(
        client.lock("/project", None, false).unwrap(),
        parse(r#"{"broker": "stopped", "locked": true, "lease_existed": false}"#)
    );
    assert_eq!(client.stop().unwrap(), parse(r#"{"broker": "stopped"}"#));
    let error = client
        .submit(
            "/project",
            &PreparedArguments::new(vec!["status".into()]),
            None,
            &kassiber_operator::client::SubmitOptions::default(),
        )
        .unwrap_err();
    assert_eq!(code(&error), "interaction_required");
}

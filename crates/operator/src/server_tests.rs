//! `tests/test_operator_server.py`: handlers through a scripted channel,
//! the real service over the fake backend, and stop/close ordering on a
//! real listener in a disposable runtime directory.

use super::*;
use crate::service::tests::{argv, code, ok_runner, root, text, unlock, Closing, Fake};
use std::collections::VecDeque;

/// A channel that replays queued client frames and records what the
/// server sends (`mock.Mock()` channels in Python).
#[derive(Default)]
struct Script {
    json: VecDeque<Value>,
    secrets: VecDeque<Vec<u8>>,
    sent: Vec<Value>,
    secret_challenges: Vec<String>,
    peer_pid: Option<u32>,
}

struct Scripted(Script);

impl FrameChannel for Scripted {
    fn send_json(&mut self, payload: &Value) -> Result<()> {
        self.0.sent.push(payload.clone());
        Ok(())
    }
    fn receive_json(&mut self) -> Result<Value> {
        self.0.json.pop_front().ok_or(Error::Eof("closed"))
    }
    fn send_secret(&mut self, _: &str, _: &[u8]) -> Result<()> {
        unreachable!("the server never sends secrets")
    }
    fn receive_secret(&mut self, expected: &str) -> Result<Zeroizing<Vec<u8>>> {
        self.0.secret_challenges.push(expected.to_owned());
        self.0
            .secrets
            .pop_front()
            .map(Zeroizing::new)
            .ok_or(Error::Eof("closed"))
    }
    fn peer_pid(&self) -> Option<u32> {
        self.0.peer_pid
    }
    fn close(&mut self) {}
}

fn channel() -> Scripted {
    Scripted(Script::default())
}

fn build() -> Value {
    dict([
        ("version", s("0.0.0-test")),
        ("channel", Value::Null),
        ("commit", Value::Null),
        ("frozen", Value::Bool(false)),
        ("origin", s("aaaaaaaaaaaa")),
    ])
}

/// A native-auth bridge that accepts one helper identity, supplies a
/// passphrase and records what the broker asked of it.
#[derive(Default)]
struct Signed {
    /// `None`: bound as `p` * 64 / `d` * 32; `Some(code)`: refused.
    binding_error: Option<&'static str>,
    calls: std::sync::Mutex<Vec<String>>,
    stored: std::sync::Mutex<Vec<Vec<u8>>>,
}

impl Signed {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

impl NativeAuth for Signed {
    fn helper_identity(&self) -> Result<String> {
        Ok("signed-helper".into())
    }
    fn validate(&self, expected: &str) -> Result<()> {
        self.record(format!("validate {expected}"));
        if expected == "signed-helper" {
            return Ok(());
        }
        Err(Error::App(AppError::new(
            "native_auth_helper_mismatch",
            "helper mismatch",
        )))
    }
    fn binding(&self, data_root: &str) -> Result<(String, String)> {
        self.record(format!("binding {data_root}"));
        match self.binding_error {
            Some(code) => Err(Error::App(AppError::new(code, "unbound"))),
            None => Ok(("p".repeat(64), "d".repeat(32))),
        }
    }
    fn passphrase(&self, data_root: &str, expected: &str) -> Result<Zeroizing<Vec<u8>>> {
        self.record(format!("passphrase {data_root} {expected}"));
        Ok(Zeroizing::new(b"native-secret".to_vec()))
    }
    fn store(&self, data_root: &str, passphrase: &[u8], expected: &str) -> Result<()> {
        self.record(format!("store {data_root} {expected}"));
        self.stored.lock().unwrap().push(passphrase.to_vec());
        Ok(())
    }
    fn delete(&self, data_root: &str, expected: &str) -> Result<()> {
        self.record(format!("delete {data_root} {expected}"));
        Ok(())
    }
}

fn broker(fake: &Arc<Fake>, native: Arc<dyn NativeAuth>) -> (BrokerServer, Closing) {
    broker_with(fake, native, ok_runner())
}

fn broker_with(
    fake: &Arc<Fake>,
    native: Arc<dyn NativeAuth>,
    runner: Arc<dyn crate::service::Runner>,
) -> (BrokerServer, Closing) {
    let service = OperatorService::new("generation", Arc::clone(fake) as Arc<dyn Backend>, runner);
    let server = BrokerServer::detached(ServerParts {
        generation: "generation".into(),
        service: service.clone(),
        native,
        build: build(),
        guard: SessionGuard::system(),
    });
    (server, Closing(service))
}

fn request(fields: Vec<(&str, Value)>) -> Value {
    let mut request = dict([("version", Value::Int(BigInt::from(PROTOCOL_VERSION)))]);
    for (key, value) in fields {
        set(&mut request, key, value);
    }
    request
}

fn strings(items: &[&str]) -> Value {
    Value::List(items.iter().map(|item| s(item)).collect())
}

fn data(response: &Value) -> Value {
    get(response, "data").cloned().unwrap_or(Value::Null)
}

fn error_code(response: &Value) -> String {
    text(get(response, "error").expect("error"), "code")
}

#[test]
fn test_native_auth_restart_stops_admission_after_response() {
    let fake = Fake::new();
    let (server, _closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    let mut channel = channel();
    channel
        .0
        .json
        .push_back(request(vec![("action", s("restart_for_native_auth"))]));
    server.serve_channel(&mut channel);
    assert_eq!(channel.0.sent.len(), 1);
    assert_eq!(text(&data(&channel.0.sent[0]), "restart"), "accepted");
    assert!(server.is_stopped());
}

#[test]
fn test_a_busy_broker_refuses_the_restart_and_keeps_serving() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    unlock(&closing, &root(&tmp)).unwrap();
    let mut channel = channel();
    channel
        .0
        .json
        .push_back(request(vec![("action", s("restart_for_native_auth"))]));
    server.serve_channel(&mut channel);
    assert_eq!(error_code(&channel.0.sent[0]), "operator_broker_busy");
    assert!(!server.is_stopped());
}

#[test]
fn test_version_mismatch_and_unknown_actions_are_refused() {
    let fake = Fake::new();
    let (server, _closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    for (version, action, expected) in [
        (
            Value::Int(BigInt::from(2)),
            "ping",
            "operator_protocol_version_mismatch",
        ),
        (
            Value::Bool(true),
            "ping",
            "operator_protocol_version_mismatch",
        ),
        (Value::Float(3.0), "nope", "operator_protocol_error"),
    ] {
        let mut channel = channel();
        channel
            .0
            .json
            .push_back(dict([("version", version), ("action", s(action))]));
        server.serve_channel(&mut channel);
        assert_eq!(error_code(&channel.0.sent[0]), expected);
    }
    // End of file before a request answers nothing.
    let mut silent = channel();
    server.serve_channel(&mut silent);
    assert!(silent.0.sent.is_empty());
}

#[cfg(unix)]
fn listening(fake: &Arc<Fake>) -> (BrokerServer, tempfile::TempDir) {
    use std::os::unix::fs::PermissionsExt;
    let runtime_dir = tempfile::Builder::new()
        .prefix("kb-srv-")
        .tempdir()
        .unwrap();
    std::fs::set_permissions(runtime_dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = crate::runtime::Runtime {
        account_home: None,
        override_dir: Some(runtime_dir.path().as_os_str().to_owned()),
        override_gate: Some("1".into()),
        frozen: false,
    };
    let service = OperatorService::new(
        "generation",
        Arc::clone(fake) as Arc<dyn Backend>,
        ok_runner(),
    );
    let trusted = runtime_dir.path().to_owned();
    let server = BrokerServer::start(
        ServerParts {
            generation: "generation".into(),
            service,
            native: Arc::new(NativeAuthUnavailable),
            build: build(),
            guard: SessionGuard {
                runtime_dir: Some(trusted.as_os_str().to_owned()),
                trust: Arc::new(move |root, _| {
                    std::fs::canonicalize(&trusted).is_ok_and(|t| t == root)
                }),
                logind: Arc::new(|| None),
            },
        },
        &runtime,
    )
    .unwrap();
    (server, runtime_dir)
}

#[cfg(unix)]
#[test]
fn test_signal_stop_leaves_service_cleanup_for_final_close() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, runtime_dir) = listening(&fake);
    let socket = runtime_dir.path().join(crate::runtime::SOCKET_FILENAME);
    assert!(socket.exists());
    unlock(server.service(), &root(&tmp)).unwrap();
    server.request_stop();
    assert!(server.is_stopped());
    assert!(!socket.exists());
    // The service still holds its lease until the final close.
    assert_eq!(
        text(
            &server.service().status(Some(&root(&tmp))).unwrap(),
            "lease"
        ),
        "unlocked"
    );
    server.serve_forever(None);
    server.close().unwrap();
    assert_eq!(
        code(server.service().status(Some(&root(&tmp)))),
        "operator_broker_stopped"
    );
}

#[cfg(unix)]
#[test]
fn test_reentrant_signal_stop_never_waits_on_active_close_lock() {
    let fake = Fake::new();
    let (server, runtime_dir) = listening(&fake);
    let socket = runtime_dir.path().join(crate::runtime::SOCKET_FILENAME);
    {
        let _held = server.inner.close_lock.lock().unwrap();
        server.request_stop();
        assert!(server.is_stopped());
        assert!(socket.exists(), "the active close owns listener shutdown");
    }
    server.close().unwrap();
    assert!(!socket.exists());
}

#[cfg(unix)]
#[test]
fn test_close_retries_service_cleanup_after_listener_is_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let owner = fake.scripted_owner();
    owner
        .release_failures
        .lock()
        .unwrap()
        .push_back("transient owner release failure".into());
    let (server, _runtime_dir) = listening(&fake);
    unlock(server.service(), &root(&tmp)).unwrap();
    let error = server.close().unwrap_err();
    assert_eq!(error.to_string(), "transient owner release failure");
    assert!(server.inner.listener_closed.load(Ordering::SeqCst));
    server.close().unwrap();
    assert_eq!(owner.releases.load(Ordering::SeqCst), 2);
}

#[cfg(unix)]
#[test]
fn test_serving_answers_a_real_client_and_stops() {
    let fake = Fake::new();
    let (server, runtime_dir) = listening(&fake);
    let serving = {
        let server = server.clone();
        std::thread::spawn(move || server.serve_forever(None))
    };
    let runtime = crate::runtime::Runtime {
        account_home: None,
        override_dir: Some(runtime_dir.path().as_os_str().to_owned()),
        override_gate: Some("1".into()),
        frozen: false,
    };
    let mut client = crate::runtime::connect(
        &runtime,
        Duration::from_secs(2),
        Some(Duration::from_secs(5)),
    )
    .unwrap();
    client
        .send_json(&request(vec![("action", s("ping"))]))
        .unwrap();
    let ping = client.receive_json().unwrap();
    assert_eq!(text(&data(&ping), "broker"), "running");
    assert_eq!(
        get(&data(&ping), "native_auth_available"),
        Some(&Value::Bool(false))
    );
    assert!(crate::values::py_eq(
        get(&data(&ping), "build").unwrap(),
        &build()
    ));
    server.request_stop();
    serving.join().unwrap();
    server.close().unwrap();
}

#[cfg(unix)]
#[test]
fn test_excess_clients_get_retryable_backpressure() {
    let fake = Fake::new();
    let (server, runtime_dir) = listening(&fake);
    let serving = {
        let server = server.clone();
        std::thread::spawn(move || server.serve_forever(None))
    };
    let runtime = crate::runtime::Runtime {
        account_home: None,
        override_dir: Some(runtime_dir.path().as_os_str().to_owned()),
        override_gate: Some("1".into()),
        frozen: false,
    };
    let connect = || {
        crate::runtime::connect(
            &runtime,
            Duration::from_secs(2),
            Some(Duration::from_secs(5)),
        )
        .unwrap()
    };
    server
        .inner
        .clients
        .store(MAX_CLIENT_THREADS, Ordering::SeqCst);
    let mut refused = connect();
    let refusal = refused.receive_json().unwrap();
    assert_eq!(error_code(&refusal), "operator_client_limit");
    let error = get(&refusal, "error").unwrap();
    assert_eq!(get(error, "retryable"), Some(&Value::Bool(true)));
    assert_eq!(
        get(get(error, "details").unwrap(), "limit"),
        Some(&Value::Int(BigInt::from(64)))
    );
    // Below the limit a client is served and its slot is returned.
    server
        .inner
        .clients
        .store(MAX_CLIENT_THREADS - 1, Ordering::SeqCst);
    let mut client = connect();
    client
        .send_json(&request(vec![("action", s("ping"))]))
        .unwrap();
    assert_eq!(
        get(&client.receive_json().unwrap(), "ok"),
        Some(&Value::Bool(true))
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while server.inner.clients.load(Ordering::SeqCst) != MAX_CLIENT_THREADS - 1 {
        assert!(
            std::time::Instant::now() < deadline,
            "the slot was not returned"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    server.request_stop();
    serving.join().unwrap();
    server.close().unwrap();
}

#[test]
fn test_linux_refuses_broker_without_logout_lifetime_primitive() {
    let fake = Fake::new();
    let runtime_dir = tempfile::tempdir().unwrap();
    let runtime = crate::runtime::Runtime {
        account_home: None,
        override_dir: Some(runtime_dir.path().as_os_str().to_owned()),
        override_gate: Some("1".into()),
        frozen: false,
    };
    let service = OperatorService::new(
        "generation",
        Arc::clone(&fake) as Arc<dyn Backend>,
        ok_runner(),
    );
    let _closing = Closing(service.clone());
    let started = BrokerServer::start(
        ServerParts {
            generation: "generation".into(),
            service,
            native: Arc::new(NativeAuthUnavailable),
            build: build(),
            guard: SessionGuard {
                runtime_dir: None,
                trust: Arc::new(|_, _| true),
                logind: Arc::new(|| None),
            },
        },
        &runtime,
    );
    if cfg!(target_os = "linux") {
        assert_eq!(
            code(started.map(|_| ())),
            "operator_session_lifetime_unavailable"
        );
        // Refused before listening.
        assert!(!runtime_dir
            .path()
            .join(crate::runtime::SOCKET_FILENAME)
            .exists());
    }
    // logind reporting a logout is refused too.
    if cfg!(target_os = "linux") {
        let service = OperatorService::new(
            "generation",
            Arc::clone(&fake) as Arc<dyn Backend>,
            ok_runner(),
        );
        let _closing = Closing(service.clone());
        let started = BrokerServer::start(
            ServerParts {
                generation: "generation".into(),
                service,
                native: Arc::new(NativeAuthUnavailable),
                build: build(),
                guard: SessionGuard {
                    runtime_dir: Some(runtime_dir.path().as_os_str().to_owned()),
                    trust: Arc::new(|_, _| true),
                    logind: Arc::new(|| Some(false)),
                },
            },
            &runtime,
        );
        assert_eq!(
            code(started.map(|_| ())),
            "operator_session_lifetime_unavailable"
        );
    }
}

/// An unlocked lease and an `admin` submission's continuation frames.
fn admin_submit(
    server: &BrokerServer,
    data_root: &str,
    command: &[&str],
    secrets: Vec<Vec<u8>>,
) -> (Result<Value>, Script) {
    let mut channel = channel();
    channel.0.secrets.extend(secrets);
    let result = server.handle(
        &mut channel,
        &request(vec![
            ("action", s("submit")),
            ("data_root", s(data_root)),
            ("operation_id", s("generation.operation")),
            ("argv", strings(command)),
        ]),
    );
    (result, channel.0)
}

#[test]
fn test_admin_submission_requires_challenge_bound_fresh_auth() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    unlock(&closing, &root(&tmp)).unwrap();
    let (result, script) = admin_submit(
        &server,
        &root(&tmp),
        &["secrets", "verify"],
        vec![b"fresh-admin-passphrase".to_vec()],
    );
    let response = result.unwrap();
    assert_eq!(get(&response, "ok"), Some(&Value::Bool(true)));
    assert_eq!(text(&data(&response), "state"), "queued");
    assert_eq!(get(&data(&response), "build"), Some(&build()));
    let continuation = &script.sent[0];
    assert_eq!(text(continuation, "continue"), "secrets");
    assert_eq!(get(continuation, "challenges"), Some(&Value::Dict(vec![])));
    let admin = text(continuation, "admin_challenge");
    assert_eq!(admin.len(), 48);
    assert_eq!(script.secret_challenges, [admin]);
    // The fresh authentication opened the database once more.
    assert_eq!(fake.opened(), 2);
}

#[test]
fn test_failed_admin_verification_never_submits() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    unlock(&closing, &root(&tmp)).unwrap();
    fake.open_with(|_, _, _| {
        Err(Error::App(AppError::new(
            "unlock_failed",
            "wrong passphrase",
        )))
    });
    let (result, _) = admin_submit(
        &server,
        &root(&tmp),
        &["secrets", "verify"],
        vec![b"wrong-admin-passphrase".to_vec()],
    );
    assert_eq!(code(result), "unlock_failed");
    assert!(closing.state().operations.is_empty());
}

#[test]
fn test_remember_unlock_reuses_the_fresh_authentication_as_its_passphrase() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    unlock(&closing, &root(&tmp)).unwrap();
    let (result, _) = admin_submit(
        &server,
        &root(&tmp),
        &["secrets", "remember-unlock"],
        vec![b"fresh".to_vec()],
    );
    let accepted = result.unwrap();
    let state = closing.state();
    let operation = &state.operations[&text(&data(&accepted), "operation_id")];
    let label = operation.argv.last().unwrap().clone();
    assert!(label.starts_with("broker-secret-") && label.len() == "broker-secret-".len() + 32);
    assert_eq!(operation.argv[operation.argv.len() - 2], "--passphrase-fd");
}

#[test]
fn test_multiple_secret_frames_follow_serialized_label_order() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    // The worker may dispatch before the test looks, taking the secrets off
    // the queued operation, so check what the dispatch itself carries.
    let (dispatched, received) = std::sync::mpsc::channel();
    let dispatched = Mutex::new(dispatched);
    let runner = move |dispatch: &crate::service::Dispatch, _: &[u8]| {
        let secrets: Vec<(String, Vec<u8>)> = dispatch
            .secret_arguments
            .iter()
            .map(|(label, secret)| (label.clone(), secret.to_vec()))
            .collect();
        let _ = dispatched
            .lock()
            .unwrap()
            .send((dispatch.id.clone(), secrets));
        Ok(crate::service::OperationResult::new(0, "", ""))
    };
    let (server, closing) = broker_with(&fake, Arc::new(NativeAuthUnavailable), Arc::new(runner));
    unlock(&closing, &root(&tmp)).unwrap();
    let mut channel = channel();
    channel
        .0
        .secrets
        .extend([b"first-secret".to_vec(), b"second-secret".to_vec()]);
    let response = server
        .handle(
            &mut channel,
            &request(vec![
                ("action", s("submit")),
                ("data_root", s(&root(&tmp))),
                ("operation_id", s("generation.operation")),
                (
                    "argv",
                    strings(&["status", "broker-secret-z", "broker-secret-a"]),
                ),
                (
                    "secret_labels",
                    strings(&["broker-secret-z", "broker-secret-a"]),
                ),
            ]),
        )
        .unwrap();
    let challenges = get(&channel.0.sent[0], "challenges").unwrap().clone();
    assert_eq!(
        channel.0.secret_challenges,
        [
            text(&challenges, "broker-secret-a"),
            text(&challenges, "broker-secret-z")
        ]
    );
    assert_eq!(
        get(&channel.0.sent[0], "admin_challenge"),
        Some(&Value::Null)
    );
    let (operation_id, secrets) = received
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the operation was dispatched");
    assert_eq!(operation_id, text(&data(&response), "operation_id"));
    assert_eq!(
        secrets,
        [
            ("broker-secret-a".to_owned(), b"first-secret".to_vec()),
            ("broker-secret-z".to_owned(), b"second-secret".to_vec())
        ]
    );
}

#[test]
fn test_secret_labels_are_validated() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    unlock(&closing, &root(&tmp)).unwrap();
    let too_many: Vec<String> = (0..33)
        .map(|index| format!("broker-secret-{index}"))
        .collect();
    let too_many: Vec<&str> = too_many.iter().map(String::as_str).collect();
    for labels in [
        strings(&["token"]),
        strings(&["broker-secret-a", "broker-secret-a"]),
        strings(&too_many),
        s("broker-secret-a"),
    ] {
        let mut channel = channel();
        let result = server.handle(
            &mut channel,
            &request(vec![
                ("action", s("submit")),
                ("data_root", s(&root(&tmp))),
                ("operation_id", s("generation.operation")),
                ("argv", strings(&["status"])),
                ("secret_labels", labels),
            ]),
        );
        assert_eq!(code(result), "operator_protocol_error");
        assert!(channel.0.sent.is_empty());
    }
    let mut channel = channel();
    let result = server.handle(
        &mut channel,
        &request(vec![
            ("action", s("submit")),
            ("data_root", s(&root(&tmp))),
            ("operation_id", s("generation.operation")),
            (
                "argv",
                Value::List(vec![s("status"), Value::Int(BigInt::from(1))]),
            ),
        ]),
    );
    assert_eq!(result.unwrap_err().to_string(), "invalid broker argv");
}

fn submit_with_caller_context(fields: Vec<(&str, Value)>) -> Result<(Option<String>, bool)> {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    unlock(&closing, &root(&tmp)).unwrap();
    let mut all = vec![
        ("action", s("submit")),
        ("data_root", s(&root(&tmp))),
        ("operation_id", s("generation.operation")),
        ("argv", strings(&["status"])),
    ];
    all.extend(fields);
    let mut channel = channel();
    let response = server.handle(&mut channel, &request(all))?;
    let state = closing.state();
    let operation = &state.operations[&text(&data(&response), "operation_id")];
    Ok((operation.working_directory.clone(), operation.no_egress))
}

#[test]
fn test_submission_binds_the_callers_directory_and_egress_switch() {
    let caller = tempfile::tempdir().unwrap();
    let bound = submit_with_caller_context(vec![
        ("working_directory", s(&root(&caller))),
        ("no_egress", Value::Bool(true)),
    ])
    .unwrap();
    assert_eq!(bound, (Some(root(&caller)), true));
}

#[test]
fn test_older_clients_without_caller_context_keep_prior_behavior() {
    assert_eq!(submit_with_caller_context(vec![]).unwrap(), (None, false));
}

#[test]
fn test_submission_rejects_relative_or_missing_working_directories() {
    let root_dir = tempfile::tempdir().unwrap();
    let missing = root_dir.path().join("gone").to_string_lossy().into_owned();
    for (value, expected) in [
        (s("relative/dir"), "operator_protocol_error"),
        (s(&missing), "operator_working_directory_unavailable"),
        (Value::Int(BigInt::from(7)), "operator_protocol_error"),
        (s(""), "operator_protocol_error"),
        (
            s(&format!("/{}", "a".repeat(4096))),
            "operator_protocol_error",
        ),
    ] {
        assert_eq!(
            code(submit_with_caller_context(vec![(
                "working_directory",
                value
            )])),
            expected
        );
    }
}

#[test]
fn test_submission_rejects_a_non_boolean_egress_flag() {
    assert_eq!(
        code(submit_with_caller_context(vec![("no_egress", s("1"))])),
        "operator_protocol_error"
    );
    assert_eq!(
        code(submit_with_caller_context(vec![(
            "agent_session_token",
            s(&"t".repeat(129))
        )])),
        "operator_protocol_error"
    );
}

#[test]
fn test_ping_and_status_name_the_brokers_build() {
    let fake = Fake::new();
    let (server, _closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    let ping = server
        .handle(&mut channel(), &request(vec![("action", s("ping"))]))
        .unwrap();
    let ping = data(&ping);
    assert_eq!(get(&ping, "build"), Some(&build()));
    assert_eq!(get(&ping, "caller_context"), Some(&Value::Bool(true)));
    assert_eq!(get(&ping, "native_auth_identity"), Some(&Value::Null));
    assert_eq!(text(&ping, "generation"), "generation");
    let status = server
        .handle(&mut channel(), &request(vec![("action", s("status"))]))
        .unwrap();
    let status = data(&status);
    assert_eq!(get(&status, "broker_build"), Some(&build()));
    assert_eq!(get(&status, "leases"), Some(&Value::List(vec![])));
    // A signed helper is advertised by its public-safe identity.
    let (signed, _closing) = broker(&fake, Arc::new(Signed::default()));
    let ping = signed
        .handle(&mut channel(), &request(vec![("action", s("ping"))]))
        .unwrap();
    assert_eq!(
        get(&data(&ping), "native_auth_identity"),
        Some(&s("signed-helper"))
    );
    assert_eq!(
        get(&data(&ping), "native_auth_available"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn test_the_command_line_follows_only_after_the_broker_names_its_build() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    unlock(&closing, &root(&tmp)).unwrap();
    let mut channel = channel();
    channel.0.peer_pid = Some(7);
    channel.0.json.push_back(dict([(
        "argv",
        strings(&["--data-root", &root(&tmp), "--machine", "status"]),
    )]));
    let response = server
        .handle(
            &mut channel,
            &request(vec![
                ("action", s("submit")),
                ("data_root", s(&root(&tmp))),
                ("operation_id", s("generation.client.1")),
                ("argv_follows", Value::Bool(true)),
            ]),
        )
        .unwrap();
    assert_eq!(
        channel.0.sent[0],
        dict([
            ("ok", Value::Bool(true)),
            ("continue", s("argv")),
            ("build", build()),
        ])
    );
    let state = closing.state();
    let operation = &state.operations["generation.client.1"];
    assert_eq!(operation.argv.last().unwrap(), "status");
    assert_eq!(get(&data(&response), "build"), Some(&build()));
}

#[test]
fn test_submit_for_another_build_is_refused_before_admission() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    unlock(&closing, &root(&tmp)).unwrap();
    let mut other = build();
    set(&mut other, "origin", s(&"f".repeat(12)));
    let mut channel = channel();
    let error = server
        .handle(
            &mut channel,
            &request(vec![
                ("action", s("submit")),
                ("data_root", s(&root(&tmp))),
                ("operation_id", s("generation.client.1")),
                ("argv", strings(&["--machine", "status"])),
                ("expected_build", other),
            ]),
        )
        .unwrap_err();
    let error = error.app().unwrap();
    assert_eq!(error.code, "operator_broker_build_mismatch");
    assert_eq!(
        get(error.details.as_ref().unwrap(), "broker_build"),
        Some(&build())
    );
    assert!(channel.0.sent.is_empty());
    assert!(closing.state().operations.is_empty());
    // The same build, keys in any order, is accepted.
    let mut channel = self::channel();
    let same = Value::Dict(match build() {
        Value::Dict(mut pairs) => {
            pairs.reverse();
            pairs
        }
        _ => unreachable!(),
    });
    server
        .handle(
            &mut channel,
            &request(vec![
                ("action", s("submit")),
                ("data_root", s(&root(&tmp))),
                ("operation_id", s("generation.client.1")),
                ("argv", strings(&["--machine", "status"])),
                ("expected_build", same),
            ]),
        )
        .unwrap();
}

#[test]
fn test_password_unlock_cannot_claim_touch_id_authentication() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    let mut channel = channel();
    let result = server.handle(
        &mut channel,
        &request(vec![
            ("action", s("unlock")),
            ("data_root", s(&root(&tmp))),
            ("authentication_method", s("touch_id")),
        ]),
    );
    assert_eq!(code(result), "operator_invalid_authentication_method");
    assert!(channel.0.sent.is_empty());
    assert!(closing.state().leases.is_empty());
}

#[test]
fn test_password_unlock_is_challenge_bound() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, _closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    let mut channel = channel();
    channel.0.secrets.push_back(b"passphrase".to_vec());
    let response = server
        .handle(
            &mut channel,
            &request(vec![
                ("action", s("unlock")),
                ("data_root", s(&root(&tmp))),
                ("capability", s("read")),
                ("duration_seconds", Value::Int(BigInt::from(600))),
            ]),
        )
        .unwrap();
    let continuation = &channel.0.sent[0];
    assert_eq!(text(continuation, "label"), "database_passphrase");
    assert_eq!(get(continuation, "build"), Some(&build()));
    assert!(get(continuation, "agent_control_challenge").is_none());
    assert_eq!(
        channel.0.secret_challenges,
        [text(continuation, "challenge")]
    );
    assert_eq!(text(&data(&response), "capability"), "read");
    assert_eq!(text(&data(&response), "authentication_method"), "password");
    // Bad arguments after the secret frame are typed refusals.
    for (field, value, expected) in [
        (
            "capability",
            s("admin"),
            "operator_invalid_lease_capability",
        ),
        ("capability", s("root"), "operator_invalid_lease_capability"),
        (
            "capability",
            Value::Int(BigInt::from(1)),
            "operator_invalid_lease_capability",
        ),
        ("duration_seconds", s("8h"), "operator_invalid_duration"),
        (
            "duration_seconds",
            Value::Bool(true),
            "operator_invalid_duration",
        ),
        ("only_if_locked", s("yes"), "operator_protocol_error"),
        ("agent_scope", s("yes"), "operator_protocol_error"),
        (
            "idle_timeout_seconds",
            Value::Bool(true),
            "operator_protocol_error",
        ),
    ] {
        let mut channel = self::channel();
        channel.0.secrets.push_back(b"passphrase".to_vec());
        let result = server.handle(
            &mut channel,
            &request(vec![
                ("action", s("unlock")),
                ("data_root", s(&root(&tmp))),
                (field, value),
            ]),
        );
        assert_eq!(code(result), expected, "{field}");
    }
}

#[test]
fn test_touch_id_method_is_assigned_by_the_broker() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = broker(&fake, Arc::new(Signed::default()));
    let mut channel = channel();
    let result = server.handle(
        &mut channel,
        &request(vec![
            ("action", s("unlock_touch_id")),
            ("data_root", s(&root(&tmp))),
            ("capability", s("operator")),
            ("native_auth_identity", s("signed-helper")),
        ]),
    );
    // The bound identities come from the policy binding, which this
    // project does not have, so the service refuses the replaced project.
    assert_eq!(code(result), "operator_project_replaced");
    assert!(channel.0.sent.is_empty());
    assert!(closing.state().leases.is_empty());
    // Without a signed helper nothing is attempted.
    let (unsigned, _closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    let result = unsigned.handle(
        &mut self::channel(),
        &request(vec![
            ("action", s("unlock_touch_id")),
            ("data_root", s(&root(&tmp))),
            ("native_auth_identity", s("signed-helper")),
        ]),
    );
    assert_eq!(code(result), "native_auth_unavailable");
}

/// The binding is checked before the lease arguments, as in Python, and
/// the bridge is asked for nothing after a refusal.
#[test]
fn test_touch_id_unlock_checks_the_binding_before_lease_arguments() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let native = Arc::new(Signed {
        binding_error: Some("operator_policy_binding_required"),
        ..Signed::default()
    });
    let (server, closing) = broker(&fake, Arc::clone(&native) as Arc<dyn NativeAuth>);
    let result = server.handle(
        &mut channel(),
        &request(vec![
            ("action", s("unlock_touch_id")),
            ("data_root", s(&root(&tmp))),
            ("capability", s("not-a-capability")),
            ("native_auth_identity", s("signed-helper")),
        ]),
    );
    assert_eq!(code(result), "operator_policy_binding_required");
    let canonical = root(&tmp);
    assert_eq!(
        native.calls(),
        [
            "validate signed-helper".to_owned(),
            format!("binding {canonical}")
        ]
    );
    assert!(closing.state().leases.is_empty());
}

/// `test_touch_id_configuration_is_challenge_bound_and_wipes_auth`, through
/// the real service: a `fresh_native_auth` challenge, fresh authentication,
/// the policy binding, then the signed helper's store or delete.
#[test]
fn test_touch_id_configuration_is_challenge_bound_and_binds_the_policy() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let canonical = root(&tmp);
    for configured in [true, false] {
        let native = Arc::new(Signed::default());
        let (server, _closing) = broker(&fake, Arc::clone(&native) as Arc<dyn NativeAuth>);
        let mut channel = channel();
        channel.0.secrets.push_back(b"fresh-passphrase".to_vec());
        let response = server
            .handle(
                &mut channel,
                &request(vec![
                    ("action", s("touch_id_configure")),
                    ("data_root", s(&canonical)),
                    ("configured", Value::Bool(configured)),
                    ("native_auth_identity", s("signed-helper")),
                ]),
            )
            .unwrap();
        assert_eq!(text(&channel.0.sent[0], "label"), "fresh_native_auth");
        assert_eq!(
            channel.0.secret_challenges,
            [text(&channel.0.sent[0], "challenge")]
        );
        assert_eq!(
            data(&response),
            dict([
                ("configured", Value::Bool(configured)),
                ("auth", s("touch_id")),
            ])
        );
        let action = if configured { "store" } else { "delete" };
        assert_eq!(
            native.calls(),
            [
                "validate signed-helper".to_owned(),
                format!("{action} {canonical} signed-helper")
            ]
        );
        if configured {
            assert_eq!(
                *native.stored.lock().unwrap(),
                [b"fresh-passphrase".to_vec()]
            );
        }
        let binds = fake.bind_calls.lock().unwrap().clone();
        let (bound_root, database, project) = binds.last().unwrap().clone();
        assert_eq!(bound_root, canonical);
        let identity = fake.canonical_project(&canonical).unwrap().identity;
        assert_eq!(database, identity[..32]);
        assert_eq!(project, Some(identity));
    }
}

/// A failed fresh authentication binds nothing and leaves the helper alone.
#[test]
fn test_touch_id_configuration_refuses_a_wrong_passphrase_before_the_helper() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    fake.open_with(|_, _, _| {
        Err(Error::App(AppError::new(
            "unlock_failed",
            "the passphrase did not unlock the database",
        )))
    });
    let native = Arc::new(Signed::default());
    let (server, _closing) = broker(&fake, Arc::clone(&native) as Arc<dyn NativeAuth>);
    let mut channel = channel();
    channel.0.secrets.push_back(b"wrong".to_vec());
    let result = server.handle(
        &mut channel,
        &request(vec![
            ("action", s("touch_id_configure")),
            ("data_root", s(&root(&tmp))),
            ("configured", Value::Bool(true)),
            ("native_auth_identity", s("signed-helper")),
        ]),
    );
    assert_eq!(code(result), "unlock_failed");
    assert_eq!(native.calls(), ["validate signed-helper".to_owned()]);
    assert!(fake.bind_calls.lock().unwrap().is_empty());
}

#[test]
fn test_touch_id_configuration_rejects_non_boolean_state() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, _closing) = broker(&fake, Arc::new(Signed::default()));
    let mut channel = channel();
    let result = server.handle(
        &mut channel,
        &request(vec![
            ("action", s("touch_id_configure")),
            ("data_root", s(&root(&tmp))),
            ("configured", s("yes")),
            ("native_auth_identity", s("signed-helper")),
        ]),
    );
    assert_eq!(code(result), "operator_protocol_error");
    assert!(channel.0.sent.is_empty());
}

#[test]
fn test_touch_id_configuration_rejects_helper_mismatch_before_challenge() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    for native in [
        Arc::new(Signed::default()) as Arc<dyn NativeAuth>,
        Arc::new(NativeAuthUnavailable),
    ] {
        let (server, _closing) = broker(&fake, native);
        let mut channel = channel();
        let result = server.handle(
            &mut channel,
            &request(vec![
                ("action", s("touch_id_configure")),
                ("data_root", s(&root(&tmp))),
                ("configured", Value::Bool(true)),
                ("native_auth_identity", s("another-helper")),
            ]),
        );
        assert!(matches!(
            code(result).as_str(),
            "native_auth_helper_mismatch" | "native_auth_unavailable"
        ));
        assert!(channel.0.sent.is_empty());
        assert!(channel.0.secret_challenges.is_empty());
    }
}

#[test]
fn test_set_mode_is_challenge_bound_fresh_authentication() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, _closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    let mut channel = channel();
    channel.0.secrets.push_back(b"fresh".to_vec());
    let response = server
        .handle(
            &mut channel,
            &request(vec![
                ("action", s("set_mode")),
                ("data_root", s(&root(&tmp))),
                ("mode", s("brokered")),
            ]),
        )
        .unwrap();
    assert_eq!(text(&channel.0.sent[0], "label"), "fresh_admin_auth");
    assert_eq!(text(&data(&response), "mode"), "brokered");
    let result = server.handle(
        &mut self::channel(),
        &request(vec![
            ("action", s("set_mode")),
            ("data_root", s(&root(&tmp))),
        ]),
    );
    assert_eq!(
        result.unwrap_err().to_string(),
        "operator request requires mode"
    );
}

#[test]
fn test_lock_and_operations_validate_their_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, _closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    for fields in [
        vec![("expected_lease_id", Value::Int(BigInt::from(1)))],
        vec![("restore_manual", s("yes"))],
    ] {
        let mut all = vec![("action", s("lock")), ("data_root", s(&root(&tmp)))];
        all.extend(fields);
        assert_eq!(
            server
                .handle(&mut channel(), &request(all))
                .unwrap_err()
                .to_string(),
            "invalid broker lock condition"
        );
    }
    let locked = server
        .handle(
            &mut channel(),
            &request(vec![("action", s("lock")), ("data_root", s(&root(&tmp)))]),
        )
        .unwrap();
    assert_eq!(
        get(&data(&locked), "lease_existed"),
        Some(&Value::Bool(false))
    );
    let unknown = server
        .handle(
            &mut channel(),
            &request(vec![
                ("action", s("operation_status")),
                ("operation_id", s("generation.x")),
                ("include_output", Value::Int(BigInt::from(0))),
            ]),
        )
        .unwrap();
    assert_eq!(text(&data(&unknown), "reason"), "result_not_retained");
    let missing = server.handle(
        &mut channel(),
        &request(vec![("action", s("operation_cancel"))]),
    );
    assert_eq!(
        missing.unwrap_err().to_string(),
        "operator request requires operation_id"
    );
}

#[test]
fn test_error_details_recursively_redact_secret_keys_and_values() {
    let response = error_response(&AppError::new("test", "safe").with_details(dict([
        ("token", s("token-value")),
        (
            "nested",
            dict([
                ("blinding-key", s("blind-value")),
                ("message", s("passphrase=hunter2")),
                (
                    "items",
                    Value::List(vec![s("token=abc"), Value::Int(BigInt::from(3))]),
                ),
            ]),
        ),
        ("database", s("/home/alice/book.sqlite3")),
        ("Path", s("/home/alice")),
        ("data_root", s("/home/alice/data")),
        ("project", s("abcd")),
    ])));
    let rendered = format!("{response:?}");
    for leaked in [
        "token-value",
        "blind-value",
        "hunter2",
        "abc\"",
        "/home/alice",
    ] {
        assert!(!rendered.contains(leaked), "{leaked} in {rendered}");
    }
    let details = get(get(&response, "error").unwrap(), "details").unwrap();
    assert_eq!(text(details, "token"), "[redacted]");
    assert_eq!(text(details, "project"), "abcd");
    assert!(get(details, "database").is_none() && get(details, "Path").is_none());
    let error = get(&response, "error").unwrap();
    assert_eq!(get(error, "hint"), Some(&Value::Null));
    assert_eq!(get(error, "retryable"), Some(&Value::Bool(false)));
}

#[test]
fn test_non_app_failures_answer_a_generic_internal_error() {
    let response = response_for_error(&Error::Io(std::io::Error::other("/home/alice/secret path")));
    assert_eq!(error_code(&response), "operator_internal_error");
    let error = get(&response, "error").unwrap();
    assert_eq!(
        text(error, "message"),
        "the operator broker encountered an internal error"
    );
    assert_eq!(get(error, "retryable"), Some(&Value::Bool(true)));
}

#[test]
fn test_agent_session_open_needs_a_peer_and_an_agent_lease() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = broker(&fake, Arc::new(NativeAuthUnavailable));
    let open = || {
        request(vec![
            ("action", s("agent_session_open")),
            ("data_root", s(&root(&tmp))),
        ])
    };
    let mut anonymous = channel();
    anonymous.0.json.push_back(open());
    server.serve_channel(&mut anonymous);
    assert_eq!(
        error_code(&anonymous.0.sent[0]),
        "agent_pairing_unavailable"
    );
    let mut terminal = channel();
    terminal.0.peer_pid = Some(4242);
    terminal.0.json.push_back(open());
    unlock(&closing, &root(&tmp)).unwrap();
    server.serve_channel(&mut terminal);
    assert_eq!(error_code(&terminal.0.sent[0]), "agent_session_not_needed");
    // An agent lease admits the session, hands its token to this
    // connection only, and closes it when the connection ends.
    closing.lock(&root(&tmp), None, false).unwrap();
    let control = vec![b'c'; 32];
    closing
        .unlock(
            &root(&tmp),
            b"passphrase",
            UnlockRequest {
                capability: Capability::Read,
                agent_scope: true,
                idle_timeout_seconds: Some(900),
                agent_control: Some(&control),
                ..UnlockRequest::default()
            },
        )
        .unwrap();
    let mut agent = channel();
    agent.0.peer_pid = Some(4242);
    agent.0.json.push_back(open());
    agent
        .0
        .json
        .push_back(request(vec![("action", s("agent_session_heartbeat"))]));
    server.serve_channel(&mut agent);
    let opened = data(&agent.0.sent[0]);
    assert_eq!(text(&opened, "state"), "pending");
    assert_eq!(text(&opened, "token").len(), 43);
    assert_eq!(agent.0.sent.len(), 1);
    assert!(!server
        .service()
        .agent_session_alive(&text(&opened, "session_id")));
    let _ = argv(&[]);
}

// ----- `AgentSessionServerTest` (`tests/test_operator_agent_sessions.py`) -----

const AGENT_PID: u32 = 4242;
const CONTROL: [u8; 32] = [b'c'; 32];

/// A channel whose client frames are produced one read at a time, so a
/// test can act between the frames a session's connection sees.
struct Stepped {
    reads: VecDeque<Box<dyn FnMut() -> Result<Value> + Send>>,
    taken: usize,
    sent: Vec<Value>,
}

impl FrameChannel for Stepped {
    fn send_json(&mut self, payload: &Value) -> Result<()> {
        self.sent.push(payload.clone());
        Ok(())
    }
    fn receive_json(&mut self) -> Result<Value> {
        self.taken += 1;
        match self.reads.pop_front() {
            Some(mut read) => read(),
            None => Err(Error::Eof("process exited")),
        }
    }
    fn send_secret(&mut self, _: &str, _: &[u8]) -> Result<()> {
        unreachable!("the server never sends secrets")
    }
    fn receive_secret(&mut self, _: &str) -> Result<Zeroizing<Vec<u8>>> {
        unreachable!("an agent session takes no secret")
    }
    fn peer_pid(&self) -> Option<u32> {
        Some(AGENT_PID)
    }
    fn close(&mut self) {}
}

fn claude(pid: Option<u32>) -> Option<String> {
    (pid == Some(AGENT_PID)).then(|| "claude".to_owned())
}

fn agent_broker(fake: &Arc<Fake>) -> (BrokerServer, Closing) {
    let service = OperatorService::new(
        "generation",
        Arc::clone(fake) as Arc<dyn Backend>,
        ok_runner(),
    );
    let server = BrokerServer::detached_with_labels(
        ServerParts {
            generation: "generation".into(),
            service: service.clone(),
            native: Arc::new(NativeAuthUnavailable),
            build: build(),
            guard: SessionGuard::system(),
        },
        claude,
    );
    (server, Closing(service))
}

fn agent_grant(service: &OperatorService, data_root: &str) {
    service
        .unlock(
            data_root,
            b"passphrase",
            UnlockRequest {
                capability: Capability::Read,
                agent_scope: true,
                idle_timeout_seconds: Some(900),
                agent_control: Some(&CONTROL),
                ..UnlockRequest::default()
            },
        )
        .unwrap();
}

fn session_list(service: &OperatorService, data_root: &str) -> Vec<Value> {
    let status = service.status(Some(data_root)).unwrap();
    match get(get(&status, "agent").unwrap(), "sessions") {
        Some(Value::List(sessions)) => sessions.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn test_a_session_lives_while_heartbeats_arrive_and_ends_with_the_process() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = agent_broker(&fake);
    agent_grant(&closing, &root(&tmp));
    let data_root = root(&tmp);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let heartbeat = || request(vec![("action", s("agent_session_heartbeat"))]);
    let mut reads: VecDeque<Box<dyn FnMut() -> Result<Value> + Send>> = VecDeque::new();
    let open = request(vec![
        ("action", s("agent_session_open")),
        ("data_root", s(&data_root)),
    ]);
    reads.push_back(Box::new(move || Ok(open.clone())));
    for _ in 0..2 {
        let (service, data_root, seen) = (closing.0.clone(), data_root.clone(), Arc::clone(&seen));
        reads.push_back(Box::new(move || {
            seen.lock()
                .unwrap()
                .push(session_list(&service, &data_root));
            Ok(heartbeat())
        }));
    }
    let mut channel = Stepped {
        reads,
        taken: 0,
        sent: Vec::new(),
    };
    server.serve_channel(&mut channel);
    // One reply: the session, its state and the token for this process.
    assert_eq!(channel.sent.len(), 1);
    let opened = data(&channel.sent[0]);
    let Value::Dict(pairs) = &opened else {
        panic!("{opened:?}")
    };
    let keys: Vec<String> = pairs
        .iter()
        .map(|(key, _)| match key {
            Key::Str(key) => key.as_str().unwrap_or_default().to_owned(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(keys, ["session_id", "state", "token"]);
    assert_eq!(text(&opened, "state"), "pending");
    // It was open, with the program's label, while the heartbeats came ...
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    for sessions in &seen {
        assert_eq!(sessions.len(), 1);
        assert_eq!(text(&sessions[0], "label"), "claude");
        assert_eq!(get(&sessions[0], "pid"), Some(&Value::from(AGENT_PID)));
    }
    // ... and closed at end of file, after the fourth read.
    assert_eq!(channel.taken, 4);
    assert!(session_list(&closing, &data_root).is_empty());
    assert!(!closing.agent_session_alive(&text(&opened, "session_id")));
}

#[test]
fn test_a_session_ends_on_any_other_frame_or_when_its_lease_ends() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = agent_broker(&fake);
    let data_root = root(&tmp);
    let open = request(vec![
        ("action", s("agent_session_open")),
        ("data_root", s(&data_root)),
    ]);
    // Any other frame ends it.
    agent_grant(&closing, &data_root);
    let mut reads: VecDeque<Box<dyn FnMut() -> Result<Value> + Send>> = VecDeque::new();
    let first = open.clone();
    reads.push_back(Box::new(move || Ok(first.clone())));
    reads.push_back(Box::new(|| Ok(request(vec![("action", s("ping"))]))));
    reads.push_back(Box::new(|| panic!("no read after another frame")));
    let mut channel = Stepped {
        reads,
        taken: 0,
        sent: Vec::new(),
    };
    server.serve_channel(&mut channel);
    assert_eq!(channel.taken, 2);
    assert!(session_list(&closing, &data_root).is_empty());
    // The end of its lease ends it before the next read.
    let mut reads: VecDeque<Box<dyn FnMut() -> Result<Value> + Send>> = VecDeque::new();
    reads.push_back(Box::new(move || Ok(open.clone())));
    let (service, locked_root) = (closing.0.clone(), data_root.clone());
    reads.push_back(Box::new(move || {
        service.lock(&locked_root, None, false).unwrap();
        Ok(request(vec![("action", s("agent_session_heartbeat"))]))
    }));
    reads.push_back(Box::new(|| panic!("no read after the lease ended")));
    let mut channel = Stepped {
        reads,
        taken: 0,
        sent: Vec::new(),
    };
    server.serve_channel(&mut channel);
    assert_eq!(channel.taken, 2);
    assert_eq!(text(&data(&channel.sent[0]), "state"), "pending");
}

#[test]
fn test_a_refused_session_gets_an_error_and_holds_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = agent_broker(&fake);
    unlock(&closing, &root(&tmp)).unwrap();
    let mut refused = channel();
    refused.0.peer_pid = Some(AGENT_PID);
    refused.0.json.push_back(request(vec![
        ("action", s("agent_session_open")),
        ("data_root", s(&root(&tmp))),
    ]));
    server.serve_channel(&mut refused);
    assert_eq!(refused.0.sent.len(), 1);
    assert_eq!(get(&refused.0.sent[0], "ok"), Some(&Value::Bool(false)));
    assert_eq!(error_code(&refused.0.sent[0]), "agent_session_not_needed");
    assert!(closing.state().agent_sessions.is_empty());
}

#[test]
fn test_decisions_need_the_control_secret_frame() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let (server, closing) = agent_broker(&fake);
    agent_grant(&closing, &root(&tmp));
    let session = closing
        .open_agent_session(&root(&tmp), Some(AGENT_PID), None)
        .unwrap();
    let decide = |allow: Value| {
        request(vec![
            ("action", s("agent_session_decide")),
            ("data_root", s(&root(&tmp))),
            ("session_id", s(&session.id)),
            ("allow", allow),
        ])
    };
    let mut allowed = channel();
    allowed.0.json.push_back(decide(Value::Bool(true)));
    allowed.0.secrets.push_back(CONTROL.to_vec());
    server.serve_channel(&mut allowed);
    let continuation = &allowed.0.sent[0];
    assert_eq!(get(continuation, "continue"), Some(&s("secret")));
    assert_eq!(get(continuation, "label"), Some(&s("agent_control")));
    assert_eq!(
        allowed.0.secret_challenges,
        [text(continuation, "challenge")]
    );
    assert_eq!(text(&data(&allowed.0.sent[1]), "state"), "allowed");
    // A non-boolean decision is refused before any secret is asked for.
    let mut malformed = channel();
    malformed.0.json.push_back(decide(s("yes")));
    server.serve_channel(&mut malformed);
    assert_eq!(malformed.0.sent.len(), 1);
    assert_eq!(error_code(&malformed.0.sent[0]), "operator_protocol_error");
    assert!(malformed.0.secret_challenges.is_empty());
}

/// Mirrors `test_broker_error_response_shows_the_helper_message`: a fixed
/// Touch ID helper error reaches the caller; other text after `auth:` does not.
#[test]
fn test_error_response_shows_known_native_auth_helper_errors() {
    let message = |text: &str| {
        let response = error_response(&AppError::new("native_auth_failed", text));
        let error = kassiber_envelope::get(&response, "error").unwrap().clone();
        kassiber_envelope::get(&error, "message").unwrap().clone()
    };
    for known in [
        "kassiber native auth: This build cannot access the protected Keychain item; reopen a production-entitled build to manage it.",
        "kassiber native auth: macOS protected Keychain rejected the operation (-34018).",
    ] {
        assert_eq!(message(known), s(known));
    }
    assert_eq!(
        message("kassiber native auth: hunter2"),
        s("kassiber native auth: [redacted]")
    );
}

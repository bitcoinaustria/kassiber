//! `BrokeredRoutingTests`, `OutdatedBrokerMcpTests`, the broker half of
//! `ReviewHardeningTests`, `StrictJsonTests` and the access refusals of
//! `AgentAccessTests` in `tests/test_mcp_server.py`, against a scripted
//! broker endpoint and a store that answers only what a call needs. The
//! direct path runs against real books in `crates/kassiber/tests/mcp.rs`.

use super::*;
use kassiber_operator::protocol::FrameChannel;
use kassiber_runtime::store::{
    Bootstrap, OwnerKind, ProjectOwner, ProjectRef, RuntimePaths, Secret, StubStore,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

fn parse(text: &str) -> Value {
    kassiber_pytext::loads(text).unwrap()
}

/// What a call needs from the store before it reaches a broker.
struct Store {
    access: std::path::PathBuf,
    database: std::path::PathBuf,
    mode: UnlockMode,
    projects: bool,
    resolved: AtomicUsize,
    _dir: tempfile::TempDir,
}

impl Store {
    fn new(mode: UnlockMode) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let access = dir.path().join("agent-access.json");
        kassiber_setup::agent_access::set(&access, Some(true), None).unwrap();
        let database = dir.path().join("kassiber.sqlite3");
        std::fs::write(&database, b"not empty").unwrap();
        Store {
            access,
            database,
            mode,
            projects: true,
            resolved: AtomicUsize::new(0),
            _dir: dir,
        }
    }
}

impl BookStore for Store {
    fn migrate_default_state_root(&self) -> Result<(), AppError> {
        panic!("an agent's read never migrates state")
    }
    fn bootstrap(&self, _: &Selection) -> Result<Bootstrap, AppError> {
        panic!("a brokered read never opens the book here")
    }
    fn effective_unlock_mode(&self, selection: &Selection) -> Result<UnlockMode, AppError> {
        assert_eq!(selection.data_root.as_deref(), Some("/book"));
        Ok(self.mode)
    }
    fn database_is_encrypted(&self, root: &str) -> Result<bool, AppError> {
        StubStore.database_is_encrypted(root)
    }
    fn canonical_project(&self, root: &str) -> Result<ProjectRef, AppError> {
        StubStore.canonical_project(root)
    }
    fn acquire_ownership(
        &self,
        project: &ProjectRef,
        kind: OwnerKind,
        generation: &str,
    ) -> Result<Box<dyn ProjectOwner>, AppError> {
        StubStore.acquire_ownership(project, kind, generation)
    }
    fn open_book(
        &self,
        _: &str,
        _: Unlock<'_>,
        _: &mut kassiber_runtime::store::RuntimeConfig,
    ) -> Result<Box<dyn kassiber_runtime::store::Book>, AppError> {
        panic!("a brokered read never opens the book here")
    }
    fn verify_passphrase(
        &self,
        root: &str,
        scope: &str,
        secret: &Secret,
    ) -> Result<bool, AppError> {
        StubStore.verify_passphrase(root, scope, secret)
    }
    fn mark_project_opened(&self, _: &str, _: &str, _: bool) -> Result<(), AppError> {
        panic!("a brokered read never opens the book here")
    }
    fn runtime_paths(&self, _: &Selection) -> Result<RuntimePaths, AppError> {
        self.resolved.fetch_add(1, Ordering::SeqCst);
        Ok(RuntimePaths {
            data_root: "/book".into(),
            database: self.database.to_string_lossy().into_owned(),
            ..RuntimePaths::default()
        })
    }
    fn catalog_has_projects(&self) -> Result<bool, AppError> {
        Ok(self.projects)
    }
    fn agent_access_path(&self) -> Result<std::path::PathBuf, AppError> {
        Ok(self.access.clone())
    }
}

/// A broker endpoint that answers from scripts and records requests.
#[derive(Default)]
struct Endpoint {
    /// One entry per connection: its replies, or a refused connect.
    connections: Mutex<VecDeque<Option<Vec<Value>>>>,
    sent: Arc<Mutex<Vec<Value>>>,
}

struct Replies {
    replies: VecDeque<Value>,
    sent: Arc<Mutex<Vec<Value>>>,
}

impl FrameChannel for Replies {
    fn send_json(&mut self, payload: &Value) -> kassiber_operator::Result<()> {
        self.sent.lock().unwrap().push(payload.clone());
        Ok(())
    }
    fn receive_json(&mut self) -> kassiber_operator::Result<Value> {
        self.replies
            .pop_front()
            .ok_or(kassiber_operator::Error::Eof("closed"))
    }
    fn send_secret(&mut self, _: &str, _: &[u8]) -> kassiber_operator::Result<()> {
        panic!("no secret goes to the broker from an agent read")
    }
    fn receive_secret(
        &mut self,
        _: &str,
    ) -> kassiber_operator::Result<zeroize::Zeroizing<Vec<u8>>> {
        unreachable!()
    }
    fn close(&mut self) {}
}

#[derive(Clone)]
struct Shared(Arc<Endpoint>);

const BUILD: &str = r#"{"version": "1", "origin": "this"}"#;

impl Host for Shared {
    fn connect(
        &self,
        _: Duration,
        _: Option<Duration>,
    ) -> kassiber_operator::Result<Box<dyn FrameChannel>> {
        match self.0.connections.lock().unwrap().pop_front() {
            Some(Some(replies)) => Ok(Box::new(Replies {
                replies: replies.into(),
                sent: Arc::clone(&self.0.sent),
            })),
            Some(None) | None => Err(kassiber_operator::Error::Io(std::io::Error::from(
                std::io::ErrorKind::ConnectionRefused,
            ))),
        }
    }
    fn launch_broker(&self) -> kassiber_operator::Result<()> {
        panic!("an agent's read must never start a broker")
    }
    fn startup_timeout(&self) -> Duration {
        Duration::ZERO
    }
    fn build_identity(&self) -> Value {
        parse(BUILD)
    }
    fn token_hex(&self, _: usize) -> kassiber_operator::Result<String> {
        Ok("00".repeat(16))
    }
    fn current_dir(&self) -> std::io::Result<PyStr> {
        Ok(PyStr::from("/caller"))
    }
    fn env_var(&self, _: &str) -> Option<String> {
        None
    }
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
    fn sleep(&self, _: Duration) {}
    fn warn(&self, _: &str) {}
}

fn ok(data: &str) -> Value {
    parse(&format!(r#"{{"ok": true, "data": {data}}}"#))
}

fn failed(error: &str) -> Value {
    parse(&format!(r#"{{"ok": false, "error": {error}}}"#))
}

fn status(data: &str) -> Option<Vec<Value>> {
    Some(vec![ok(data)])
}

fn ping() -> Option<Vec<Value>> {
    Some(vec![ok(&format!(
        r#"{{"generation": "gen", "caller_context": true, "build": {BUILD}}}"#
    ))])
}

/// A `submit` connection: the argv continuation, then its answer.
fn accepted(state: &str) -> Option<Vec<Value>> {
    Some(vec![
        parse(&format!(
            r#"{{"ok": true, "continue": "argv", "build": {BUILD}}}"#
        )),
        ok(&format!(
            r#"{{"operation_id": "gen.client.{}", "state": "{state}", "build": {BUILD}}}"#,
            "00".repeat(16)
        )),
    ])
}

fn child(stdout: &Value, state: &str) -> Option<Vec<Value>> {
    let stdout = kassiber_envelope::daemon_line(stdout).unwrap();
    Some(vec![ok(&format!(
        r#"{{"state": "{state}", "exit_code": 0, "stdout": {}, "stderr": ""}}"#,
        kassiber_envelope::daemon_line(&s(stdout.trim_end()))
            .unwrap()
            .trim_end()
    ))])
}

fn tools(
    store: Store,
    endpoint: &Arc<Endpoint>,
    workspace: Option<&str>,
    profile: Option<&str>,
) -> BookTools<Shared> {
    let shared = Shared(Arc::clone(endpoint));
    BookTools::new(
        Arc::new(store),
        BuildInfo {
            version: "test",
            channel: None,
            commit: None,
        },
        Environment::default(),
        Box::new(move || BrokerClient::new(shared.clone())),
        Selection {
            data_root: Some("/book".into()),
            ..Selection::default()
        },
        workspace.map(str::to_owned),
        profile.map(str::to_owned),
    )
}

fn endpoint(connections: Vec<Option<Vec<Value>>>) -> Arc<Endpoint> {
    Arc::new(Endpoint {
        connections: Mutex::new(connections.into()),
        sent: Arc::default(),
    })
}

fn call(tools: &BookTools<Shared>, name: &str, arguments: &str) -> ToolOutcome {
    tools
        .call_tool(&PyStr::from(name), &parse(arguments), &Cancelled::default())
        .unwrap()
}

fn error<'a>(outcome: &'a ToolOutcome, key: &str) -> Option<&'a Value> {
    get(get(&outcome.structured, "error").unwrap(), key)
}

fn reason(outcome: &ToolOutcome) -> Option<Value> {
    error(outcome, "details").and_then(|details| get(details, "reason").cloned())
}

fn actions(endpoint: &Endpoint) -> Vec<String> {
    endpoint
        .sent
        .lock()
        .unwrap()
        .iter()
        .filter_map(|request| get(request, "action").map(kassiber_envelope::py_str))
        .collect()
}

fn submitted_argv(endpoint: &Endpoint) -> Vec<String> {
    let sent = endpoint.sent.lock().unwrap();
    let argv = sent
        .iter()
        .find_map(|request| get(request, "argv"))
        .expect("an argv frame");
    let Value::List(items) = argv else {
        panic!("argv is a list")
    };
    items.iter().map(kassiber_envelope::py_str).collect()
}

#[test]
fn no_lease_asks_for_a_user_unlock_without_starting_a_broker() {
    let broker = endpoint(vec![None]);
    let outcome = call(
        &tools(Store::new(UnlockMode::Brokered), &broker, None, None),
        "status",
        "{}",
    );
    assert!(outcome.is_error);
    assert_eq!(error(&outcome, "code"), Some(&s("interaction_required")));
    assert_eq!(reason(&outcome), Some(s("operator_lease_required")));
    let hint = kassiber_envelope::py_str(error(&outcome, "hint").unwrap());
    assert!(hint.contains("operator unlock") && hint.contains("Never ask for the passphrase"));
    // Points at the desktop's unlock for agents too.
    assert!(hint.contains("unlock this book for agents in Kassiber"));
    assert_eq!(
        error(&outcome, "details").unwrap(),
        &parse(
            r#"{"reason": "operator_lease_required", "unlock_mode": "brokered", "user_command": "kassiber operator unlock --capability read"}"#
        )
    );
    assert_eq!(error(&outcome, "retryable"), Some(&Value::Bool(true)));
    assert!(
        actions(&broker).is_empty(),
        "a stopped broker was only probed"
    );
}

#[test]
fn a_lease_runs_one_scoped_mcp_call_operation() {
    let result = parse(
        r#"{"kind": "mcp.call", "schema_version": 1, "data": {"tool": "status", "book": {"profile": "Main"}, "data": {"ok": 1}}}"#,
    );
    let broker = endpoint(vec![
        status(
            r#"{"lease": "unlocked", "default_scope": {"workspace": "ws-id", "profile": "book-id"}}"#,
        ),
        ping(),
        accepted("queued"),
        child(&result, "completed"),
    ]);
    let outcome = call(
        &tools(Store::new(UnlockMode::Brokered), &broker, None, None),
        "transactions_list",
        r#"{"limit": 3}"#,
    );
    assert!(!outcome.is_error, "{:?}", outcome.structured);
    assert_eq!(
        get(&outcome.structured, "data"),
        Some(&parse(r#"{"ok": 1}"#))
    );
    assert_eq!(
        submitted_argv(&broker),
        [
            "--data-root",
            "/book",
            "--machine",
            "mcp",
            "call",
            "--tool",
            "transactions_list",
            "--arguments",
            "{\"limit\":3}",
            "--workspace",
            "ws-id",
            "--profile",
            "book-id",
        ]
    );
    let sent = broker.sent.lock().unwrap();
    let submit = sent
        .iter()
        .find(|request| get(request, "action") == Some(&s("submit")))
        .unwrap();
    // This build only, and no agent session without an agent lease.
    assert_eq!(get(submit, "expected_build"), Some(&parse(BUILD)));
    assert!(get(submit, "agent_session_token").is_none());
    assert_eq!(get(submit, "secret_labels"), Some(&Value::List(vec![])));
}

#[test]
fn the_pinned_book_wins_over_the_lease_default_and_is_required() {
    let result = parse(r#"{"kind": "mcp.call", "schema_version": 1, "data": {"tool": "status"}}"#);
    let broker = endpoint(vec![
        status(
            r#"{"lease": "unlocked", "default_scope": {"workspace": "ws-id", "profile": "book-id"}}"#,
        ),
        ping(),
        accepted("queued"),
        child(&result, "completed"),
    ]);
    let pinned = tools(
        Store::new(UnlockMode::Brokered),
        &broker,
        Some("Biz"),
        Some("Ops"),
    );
    assert!(!call(&pinned, "status", "{}").is_error);
    let argv = submitted_argv(&broker);
    assert_eq!(
        &argv[argv.len() - 4..],
        ["--workspace", "Biz", "--profile", "Ops"]
    );
    // A lease without a default scope needs a pinned server.
    let broker = endpoint(vec![status(
        r#"{"lease": "unlocked", "default_scope": {"workspace": "w"}}"#,
    )]);
    let outcome = call(
        &tools(Store::new(UnlockMode::Brokered), &broker, None, None),
        "status",
        "{}",
    );
    assert_eq!(error(&outcome, "code"), Some(&s("operator_scope_required")));
    assert_eq!(actions(&broker), ["status"]);
}

#[test]
fn an_agent_lease_registers_this_process_once_and_again_when_dropped() {
    let result = parse(
        r#"{"kind": "mcp.call", "schema_version": 1, "data": {"tool": "status", "data": {}}}"#,
    );
    let lease = r#"{"lease": "unlocked", "agent_scope": true, "default_scope": {"workspace": "w", "profile": "p"}}"#;
    let session = |token: &str| {
        Some(vec![ok(&format!(
            r#"{{"session_id": "s", "token": "{token}"}}"#
        ))])
    };
    let dropped = Some(vec![
        parse(&format!(
            r#"{{"ok": true, "continue": "argv", "build": {BUILD}}}"#
        )),
        failed(
            r#"{"code": "interaction_required", "message": "no session", "details": {"reason": "agent_session_required"}}"#,
        ),
    ]);
    let broker = endpoint(vec![
        // First call: open, a refused submit, a new session, a resubmit.
        status(lease),
        session("first"),
        ping(),
        dropped,
        session("second"),
        ping(),
        accepted("queued"),
        child(&result, "completed"),
        // Second call: the live session is reused.
        status(lease),
        ping(),
        accepted("queued"),
        child(&result, "completed"),
    ]);
    let tools = tools(Store::new(UnlockMode::Brokered), &broker, None, None);
    assert!(!call(&tools, "status", "{}").is_error);
    assert!(!call(&tools, "status", "{}").is_error);
    let sent = broker.sent.lock().unwrap();
    let opened = sent
        .iter()
        .filter(|request| get(request, "action") == Some(&s("agent_session_open")))
        .count();
    assert_eq!(opened, 2);
    let tokens: Vec<Value> = sent
        .iter()
        .filter(|request| get(request, "action") == Some(&s("submit")))
        .map(|request| {
            get(request, "agent_session_token")
                .cloned()
                .unwrap_or(Value::Null)
        })
        .collect();
    assert_eq!(tokens, [s("first"), s("second"), s("second")]);
}

#[test]
fn parallel_calls_with_a_stale_session_replace_it_only_once() {
    let session = |token: &str| {
        Some(vec![ok(&format!(
            r#"{{"session_id": "s", "token": "{token}"}}"#
        ))])
    };
    let broker = endpoint(vec![session("h1"), session("h2")]);
    let tools = tools(Store::new(UnlockMode::Brokered), &broker, None, None);
    let client = (tools.client)();
    let stale = tools.agent_session(&client, "/book", None).unwrap();
    // Call A replaces the stale registration ...
    let fresh = tools.agent_session(&client, "/book", Some(&stale)).unwrap();
    // ... and call B, which saw the same stale one, reuses A's.
    let again = tools.agent_session(&client, "/book", Some(&stale)).unwrap();
    assert!(Arc::ptr_eq(&again, &fresh));
    assert!(fresh.alive());
    assert!(!stale.alive());
    assert_eq!(fresh.token, PyStr::from("h2"));
    assert_eq!(
        actions(&broker),
        ["agent_session_open", "agent_session_open"]
    );
}

#[test]
fn a_waiting_agent_is_told_to_ask_the_user() {
    let broker = endpoint(vec![
        status(
            r#"{"lease": "unlocked", "agent_scope": true, "default_scope": {"workspace": "w", "profile": "p"}}"#,
        ),
        Some(vec![ok(r#"{"session_id": "s", "token": "t"}"#)]),
        ping(),
        Some(vec![
            parse(&format!(
                r#"{{"ok": true, "continue": "argv", "build": {BUILD}}}"#
            )),
            failed(
                r#"{"code": "interaction_required", "message": "waiting for the user to allow this agent in Kassiber", "hint": "Ask the user to allow this agent in Kassiber under Settings, AI, External agents, then retry.", "details": {"reason": "agent_pairing_required"}, "retryable": true}"#,
            ),
        ]),
    ]);
    let outcome = call(
        &tools(Store::new(UnlockMode::Brokered), &broker, None, None),
        "status",
        "{}",
    );
    assert!(outcome.is_error);
    assert_eq!(reason(&outcome), Some(s("agent_pairing_required")));
    assert!(kassiber_envelope::py_str(error(&outcome, "hint").unwrap())
        .contains("allow this agent in Kassiber"));
    assert_eq!(error(&outcome, "retryable"), Some(&Value::Bool(true)));
}

#[test]
fn brokered_child_errors_become_tool_errors() {
    let result = parse(
        r#"{"kind": "error", "schema_version": 1, "error": {"code": "operator_capability_denied", "message": "denied", "retryable": false}}"#,
    );
    let broker = endpoint(vec![
        status(r#"{"lease": "unlocked", "default_scope": {"workspace": "w", "profile": "p"}}"#),
        ping(),
        accepted("queued"),
        child(&result, "failed"),
    ]);
    let outcome = call(
        &tools(Store::new(UnlockMode::Brokered), &broker, None, None),
        "status",
        "{}",
    );
    assert!(outcome.is_error);
    assert_eq!(
        error(&outcome, "code"),
        Some(&s("operator_capability_denied"))
    );
    assert_eq!(
        outcome.text,
        PyStr::from("operator_capability_denied: denied")
    );
}

#[test]
fn brokered_results_map_like_python() {
    let id = PyStr::from("gen.op");
    let completed = |text: &str| parse(text);
    // An output error names the broker's code, or the default.
    let error = brokered_result(&id, &completed(r#"{"state": "completed", "output_error": {"code": "operator_result_too_large", "message": "too big"}}"#)).unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        ("operator_result_too_large", "too big")
    );
    assert_eq!(
        error.hint.as_deref(),
        Some("Narrow the request with the tool's limit or filter arguments.")
    );
    let error = brokered_result(
        &id,
        &completed(r#"{"state": "completed", "output_error": {}}"#),
    )
    .unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        (
            "operator_output_unavailable",
            "The operator result output is unavailable."
        )
    );
    // Results are redacted again at this boundary.
    let ok = brokered_result(
        &id,
        &completed(r#"{"state": "completed", "stdout": "{\"kind\": \"mcp.call\", \"data\": {\"data_root\": \"/home/user/book\", \"n\": 1}}"}"#),
    )
    .unwrap();
    assert_eq!(get(&ok, "n"), Some(&Value::from(1)));
    assert_ne!(get(&ok, "data_root"), Some(&s("/home/user/book")));
    // Without a recognizable envelope, the state names the failure.
    for (stdout, state, code, retryable) in [
        (
            "\"not json\"",
            "\"cancelled\"",
            "operator_operation_cancelled",
            true,
        ),
        ("\"  \"", "\"failed\"", "operator_operation_failed", false),
        (
            "\"{\\\"kind\\\": \\\"mcp.call\\\", \\\"data\\\": []}\"",
            "\"completed\"",
            "operator_operation_completed",
            false,
        ),
        ("null", "null", "operator_error", false),
    ] {
        let error = brokered_result(
            &id,
            &completed(&format!(r#"{{"state": {state}, "stdout": {stdout}}}"#)),
        )
        .unwrap_err();
        assert_eq!(error.code, code, "{stdout}");
        assert_eq!(error.retryable, retryable, "{stdout}");
        assert_eq!(
            error.details,
            Some(parse(&format!(
                r#"{{"operation_id": "gen.op", "state": {state}}}"#
            )))
        );
    }
    let error = brokered_result(&id, &completed(r#"{"state": null}"#)).unwrap_err();
    assert_eq!(error.message, "the brokered tool call ended as None");
}

#[test]
fn cancelling_a_queued_call_cancels_its_operation() {
    let broker = endpoint(vec![
        status(r#"{"lease": "unlocked", "default_scope": {"workspace": "w", "profile": "p"}}"#),
        ping(),
        accepted("queued"),
        Some(vec![ok(r#"{"state": "cancelled"}"#)]),
    ]);
    let tools = tools(Store::new(UnlockMode::Brokered), &broker, None, None);
    let cancelled = Cancelled::default();
    cancelled.set();
    let outcome = tools
        .call_tool(&PyStr::from("status"), &parse("{}"), &cancelled)
        .unwrap();
    assert_eq!(error(&outcome, "code"), Some(&s("cancelled")));
    assert_eq!(
        actions(&broker),
        ["status", "ping", "submit", "operation_cancel"]
    );
}

#[test]
fn a_running_call_is_polled_until_it_ends() {
    let result = parse(r#"{"kind": "mcp.call", "schema_version": 1, "data": {"tool": "status"}}"#);
    let broker = endpoint(vec![
        status(r#"{"lease": "unlocked", "default_scope": {"workspace": "w", "profile": "p"}}"#),
        ping(),
        accepted("queued"),
        Some(vec![ok(r#"{"state": "running"}"#)]),
        child(&result, "completed"),
    ]);
    let started = std::time::Instant::now();
    let outcome = call(
        &tools(Store::new(UnlockMode::Brokered), &broker, None, None),
        "status",
        "{}",
    );
    assert!(!outcome.is_error, "{:?}", outcome.structured);
    // Two polls at Python's 0.1 s (`_BROKER_POLL_SECONDS`).
    assert_eq!(BROKER_POLL, Duration::from_millis(100));
    assert!(started.elapsed() >= Duration::from_millis(200));
    assert_eq!(
        actions(&broker),
        [
            "status",
            "ping",
            "submit",
            "operation_status",
            "operation_status"
        ]
    );
}

#[test]
fn a_terminal_admission_without_output_is_read_once_more() {
    let result = parse(r#"{"kind": "mcp.call", "schema_version": 1, "data": {"tool": "status"}}"#);
    let broker = endpoint(vec![
        status(r#"{"lease": "unlocked", "default_scope": {"workspace": "w", "profile": "p"}}"#),
        ping(),
        accepted("completed"),
        child(&result, "completed"),
    ]);
    let outcome = call(
        &tools(Store::new(UnlockMode::Brokered), &broker, None, None),
        "status",
        "{}",
    );
    assert!(!outcome.is_error);
    assert_eq!(
        actions(&broker),
        ["status", "ping", "submit", "operation_status"]
    );
}

#[test]
fn a_broker_that_exits_after_the_lease_check_is_not_restarted() {
    let broker = endpoint(vec![
        status(r#"{"lease": "unlocked", "default_scope": {"workspace": "w", "profile": "p"}}"#),
        None,
    ]);
    let outcome = call(
        &tools(Store::new(UnlockMode::Brokered), &broker, None, None),
        "status",
        "{}",
    );
    assert_eq!(reason(&outcome), Some(s("operator_lease_required")));
}

#[test]
fn mcp_refuses_a_broker_that_ignores_caller_context() {
    let broker = endpoint(vec![
        status(r#"{"lease": "unlocked", "default_scope": {"workspace": "w", "profile": "p"}}"#),
        Some(vec![ok(&format!(
            r#"{{"generation": "old", "build": {BUILD}}}"#
        ))]),
    ]);
    let outcome = call(
        &tools(Store::new(UnlockMode::Brokered), &broker, None, None),
        "status",
        "{}",
    );
    assert_eq!(
        error(&outcome, "code"),
        Some(&s("operator_broker_outdated"))
    );
    assert_eq!(actions(&broker), ["status", "ping"]);
}

#[test]
fn mcp_refuses_a_broker_of_another_build() {
    let broker = endpoint(vec![
        status(r#"{"lease": "unlocked", "default_scope": {"workspace": "w", "profile": "p"}}"#),
        Some(vec![ok(
            r#"{"generation": "g", "caller_context": true, "build": {"version": "1", "origin": "000000000000"}}"#,
        )]),
    ]);
    let outcome = call(
        &tools(Store::new(UnlockMode::Brokered), &broker, None, None),
        "status",
        "{}",
    );
    assert_eq!(
        error(&outcome, "code"),
        Some(&s("operator_broker_build_mismatch"))
    );
    assert_eq!(actions(&broker), ["status", "ping"]);
}

#[test]
fn invalid_arguments_are_refused_before_anything_is_queued() {
    let broker = endpoint(vec![]);
    let outcome = call(
        &tools(Store::new(UnlockMode::Brokered), &broker, None, None),
        "status",
        r#"{"x": 1}"#,
    );
    assert_eq!(error(&outcome, "code"), Some(&s("validation")));
    assert!(broker.sent.lock().unwrap().is_empty());
}

#[test]
fn access_gates_refuse_before_any_project_is_resolved() {
    let store = Store::new(UnlockMode::Brokered);
    kassiber_setup::agent_access::set(&store.access, Some(false), None).unwrap();
    let access = store.access.clone();
    let broker = endpoint(vec![]);
    let tools = tools(store, &broker, None, None);
    let refused = call(&tools, "status", "{}");
    assert_eq!(error(&refused, "code"), Some(&s("mcp_disabled")));
    // MCP off outranks the AI master switch.
    kassiber_setup::agent_access::set(&access, Some(false), Some(false)).unwrap();
    assert_eq!(
        error(&call(&tools, "status", "{}"), "code"),
        Some(&s("mcp_disabled"))
    );
    kassiber_setup::agent_access::set(&access, Some(true), Some(false)).unwrap();
    assert_eq!(
        error(&call(&tools, "status", "{}"), "code"),
        Some(&s("ai_features_disabled"))
    );
    assert!(broker.sent.lock().unwrap().is_empty());
}

#[test]
fn a_one_shot_call_checks_access_before_resolving_the_book() {
    let store = Store::new(UnlockMode::Manual);
    let selection = Selection {
        data_root: Some("/book".into()),
        ..Selection::default()
    };
    require_existing_book(&store, &selection).unwrap();
    assert_eq!(store.resolved.load(Ordering::SeqCst), 1);
    kassiber_setup::agent_access::set(&store.access, Some(false), None).unwrap();
    let error = require_existing_book(&store, &selection).unwrap_err();
    assert_eq!(error.code, "mcp_disabled");
    assert_eq!(
        store.resolved.load(Ordering::SeqCst),
        1,
        "refused before resolving"
    );
    kassiber_setup::agent_access::set(&store.access, Some(true), None).unwrap();
    std::fs::write(&store.database, b"").unwrap();
    assert_eq!(
        require_existing_book(&store, &selection)
            .unwrap_err()
            .message,
        "No Kassiber book exists in the selected project yet"
    );
    let mut fresh = Store::new(UnlockMode::Manual);
    fresh.projects = false;
    let error = require_existing_book(&fresh, &Selection::default()).unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        (
            "not_initialized",
            "No Kassiber project exists on this machine yet"
        )
    );
    assert_eq!(fresh.resolved.load(Ordering::SeqCst), 0);
    // A named project is resolved even with an empty catalog, as in Python.
    let named = Selection {
        project: Some("p".into()),
        ..Selection::default()
    };
    require_existing_book(&fresh, &named).unwrap();
}

#[test]
fn a_fresh_machine_or_a_missing_book_is_not_initialized() {
    let mut store = Store::new(UnlockMode::Manual);
    store.projects = false;
    // An explicit data root skips the catalog check; its empty book does not.
    std::fs::write(&store.database, b"").unwrap();
    let broker = endpoint(vec![]);
    let mut tools = tools(store, &broker, None, None);
    let outcome = call(&tools, "status", "{}");
    assert_eq!(error(&outcome, "code"), Some(&s("not_initialized")));
    assert_eq!(
        error(&outcome, "message"),
        Some(&s("No Kassiber book exists in the selected project yet"))
    );
    tools.selection.data_root = None;
    let outcome = call(&tools, "status", "{}");
    assert_eq!(
        error(&outcome, "message"),
        Some(&s("No Kassiber project exists on this machine yet"))
    );
    assert_eq!(
        outcome.text,
        PyStr::from("not_initialized: No Kassiber project exists on this machine yet\nhint: Ask the user to set Kassiber up first (desktop app or `kassiber init`).")
    );
}

#[test]
fn half_pins_and_arguments_are_checked_like_python() {
    let pin = |workspace: Option<&str>, profile: Option<&str>| {
        let mut args = Namespace::default();
        if let Some(workspace) = workspace {
            args.set("workspace", s(workspace));
        }
        if let Some(profile) = profile {
            args.set("profile", s(profile));
        }
        require_complete_pin(&args)
    };
    assert!(pin(None, None).is_ok());
    assert!(pin(Some("w"), Some("p")).is_ok());
    for (workspace, profile) in [(Some("w"), None), (None, Some("p")), (Some(""), Some("p"))] {
        let error = pin(workspace, profile).unwrap_err();
        assert_eq!(error.code, "validation");
        assert_eq!(
            error.message,
            "pin a book with both --workspace and --profile, or with neither"
        );
    }
    assert_eq!(parse_arguments(None).unwrap(), parse("{}"));
    assert_eq!(
        parse_arguments(Some(&s(r#"{"limit": 3}"#))).unwrap(),
        parse(r#"{"limit": 3}"#)
    );
    for raw in [
        "",
        "[1]",
        "3",
        r#"{"limit": Infinity}"#,
        r#"{"x": NaN}"#,
        "{",
    ] {
        let error = parse_arguments(Some(&s(raw))).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("validation", "--arguments must be a JSON object"),
            "{raw}"
        );
    }
}

#[test]
fn enabling_needs_the_users_terminal_and_disabling_never_does() {
    let store = Store::new(UnlockMode::Manual);
    kassiber_setup::agent_access::set(&store.access, Some(false), None).unwrap();
    let mut stderr = Vec::new();
    for (non_interactive, tty) in [(true, true), (false, false)] {
        let error = configure(
            &store,
            "mcp.enable",
            non_interactive,
            tty,
            &mut std::io::Cursor::new(b"y\n".to_vec()),
            &mut stderr,
        )
        .unwrap_err();
        assert_eq!(error.code, "interaction_required");
        assert_eq!(error.details, Some(parse(r#"{"reason": "user_consent"}"#)));
        assert!(!error.retryable);
    }
    assert!(stderr.is_empty());
    let declined = configure(
        &store,
        "mcp.enable",
        false,
        true,
        &mut std::io::Cursor::new(b"no\n".to_vec()),
        &mut stderr,
    )
    .unwrap();
    assert_eq!(get(&declined, "mcp_enabled"), Some(&Value::Bool(false)));
    let enabled = configure(
        &store,
        "mcp.enable",
        false,
        true,
        &mut std::io::Cursor::new(b"  YES \n".to_vec()),
        &mut stderr,
    )
    .unwrap();
    assert_eq!(get(&enabled, "mcp_available"), Some(&Value::Bool(true)));
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "External agents you connect will read this machine's Kassiber books (read-only), and their model provider receives what they read.\nAllow? [y/N] ".repeat(2)
    );
    let disabled = configure(
        &store,
        "mcp.disable",
        true,
        false,
        &mut std::io::empty(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        disabled,
        parse(
            r#"{"mcp_enabled": false, "ai_features_enabled": null, "mcp_available": false, "reason": "mcp_disabled"}"#
        )
    );
    let status = configure(
        &store,
        "mcp.status",
        true,
        false,
        &mut std::io::empty(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(status, disabled);
    let tools = configure(
        &store,
        "mcp.tools",
        true,
        false,
        &mut std::io::empty(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(get(&tools, "transport"), Some(&s("stdio")));
}

#[test]
fn absolute_paths_follow_posixpath() {
    assert_eq!(absolute("/a/./b/../c"), "/a/c");
    assert_eq!(absolute("//a//b"), "//a/b");
    assert_eq!(absolute("///a"), "/a");
    assert_eq!(absolute("/.."), "/");
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(absolute("x/../y"), format!("{}/y", cwd.display()));
}

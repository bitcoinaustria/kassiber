//! `tests/test_daemon_agent_session.py` against a fake broker with the
//! broker's lease semantics, a scripted clock and a disposable preference
//! file.

use super::*;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
enum Call {
    Unlock(String, Vec<u8>),
    Status(String),
    Decide(String, String, bool),
    Lock(String, String, bool),
    Ping,
}

#[derive(Default)]
struct BrokerState {
    calls: Vec<Call>,
    /// A live lease someone already holds.
    lease_id: Option<String>,
    stopped: bool,
    lock_fails: bool,
    /// What the broker's grant replaces.
    previous_mode: &'static str,
    sessions: Vec<Value>,
    decisions: Vec<(String, bool, Vec<u8>)>,
    /// Lock looks "stopped" but the broker lives.
    connection_lost: bool,
    /// Status reads fail as an unreachable broker's do.
    status_fails: bool,
}

#[derive(Clone)]
struct FakeBroker(Arc<Mutex<BrokerState>>);

impl FakeBroker {
    fn new() -> Self {
        FakeBroker(Arc::new(Mutex::new(BrokerState {
            previous_mode: "manual",
            ..BrokerState::default()
        })))
    }
    fn state(&self) -> std::sync::MutexGuard<'_, BrokerState> {
        self.0.lock().unwrap()
    }
    fn calls(&self) -> Vec<Call> {
        self.state().calls.clone()
    }
    fn names(&self) -> Vec<&'static str> {
        self.calls()
            .iter()
            .map(|call| match call {
                Call::Unlock(..) => "unlock",
                Call::Status(_) => "status",
                Call::Decide(..) => "decide",
                Call::Lock(..) => "lock",
                Call::Ping => "ping",
            })
            .collect()
    }
}

fn refused() -> kassiber_operator::Error {
    kassiber_operator::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
}

fn app(code: &str) -> kassiber_operator::Error {
    kassiber_operator::Error::App(AppError::new(code, code))
}

impl AgentBroker for FakeBroker {
    fn unlock(
        &self,
        data_root: &str,
        passphrase: &[u8],
        _control: &[u8],
    ) -> kassiber_operator::Result<Value> {
        let mut state = self.state();
        state
            .calls
            .push(Call::Unlock(data_root.to_owned(), passphrase.to_vec()));
        if state.lease_id.is_some() {
            return Err(app("operator_lease_exists"));
        }
        state.lease_id = Some("desktop-lease".into());
        Ok(dict([
            ("lease", s("unlocked")),
            ("lease_id", s("desktop-lease")),
            ("expires_at", s("2026-09-28T18:00:00Z")),
            ("previous_mode", s(state.previous_mode)),
        ]))
    }
    fn status(&self, data_root: &str) -> kassiber_operator::Result<Value> {
        let mut state = self.state();
        state.calls.push(Call::Status(data_root.to_owned()));
        if state.status_fails {
            return Err(refused());
        }
        let Some(lease_id) = state.lease_id.clone().filter(|_| !state.stopped) else {
            return Ok(dict([("broker", s("running")), ("lease", s("locked"))]));
        };
        let calls: i64 = state
            .sessions
            .iter()
            .map(|session| match get(session, "calls") {
                Some(Value::Int(calls)) => i64::try_from(calls.clone()).unwrap(),
                _ => 0,
            })
            .sum();
        Ok(dict([
            ("broker", s("running")),
            ("lease", s("unlocked")),
            ("lease_id", s(lease_id.as_str())),
            ("agent_scope", Value::Bool(true)),
            (
                "agent",
                dict([
                    ("calls", Value::from(calls)),
                    ("last_call_at", Value::Null),
                    ("idle_remaining_seconds", Value::from(800)),
                    ("sessions", Value::List(state.sessions.clone())),
                ]),
            ),
        ]))
    }
    fn decide(
        &self,
        data_root: &str,
        session_id: &str,
        allow: bool,
        control: &[u8],
    ) -> kassiber_operator::Result<Value> {
        let mut state = self.state();
        state.calls.push(Call::Decide(
            data_root.to_owned(),
            session_id.to_owned(),
            allow,
        ));
        state
            .decisions
            .push((session_id.to_owned(), allow, control.to_vec()));
        for session in &mut state.sessions {
            if get(session, "id") == Some(&s(session_id)) {
                kassiber_envelope::set(
                    session,
                    "state",
                    s(if allow { "allowed" } else { "denied" }),
                );
            }
        }
        Ok(dict([("id", s(session_id))]))
    }
    fn lock(
        &self,
        data_root: &str,
        expected_lease_id: &str,
        restore_manual: bool,
    ) -> kassiber_operator::Result<Value> {
        let mut state = self.state();
        state.calls.push(Call::Lock(
            data_root.to_owned(),
            expected_lease_id.to_owned(),
            restore_manual,
        ));
        if state.lock_fails {
            return Err(app("operator_protocol_error"));
        }
        if state.stopped || state.connection_lost {
            return Ok(dict([
                ("broker", s("stopped")),
                ("locked", Value::Bool(true)),
                ("lease_existed", Value::Bool(false)),
            ]));
        }
        if state
            .lease_id
            .as_deref()
            .is_some_and(|lease| lease != expected_lease_id)
        {
            return Ok(dict([
                ("locked", Value::Bool(false)),
                ("lease_existed", Value::Bool(true)),
                ("kept", Value::Bool(true)),
            ]));
        }
        state.lease_id = None;
        Ok(dict([
            ("locked", Value::Bool(true)),
            ("mode_restored", Value::Bool(restore_manual)),
        ]))
    }
    fn ping(&self) -> kassiber_operator::Result<Value> {
        let mut state = self.state();
        state.calls.push(Call::Ping);
        if state.stopped {
            return Err(refused());
        }
        Ok(dict([("generation", s("g"))]))
    }
}

/// `time.monotonic`, scripted: queued readings first, then a base.
#[derive(Clone)]
struct Clock(Arc<Mutex<(Instant, Vec<Instant>)>>);

impl Clock {
    fn new() -> Self {
        Clock(Arc::new(Mutex::new((Instant::now(), Vec::new()))))
    }
    fn now(&self) -> Instant {
        let mut clock = self.0.lock().unwrap();
        if clock.1.is_empty() {
            clock.0
        } else {
            clock.1.remove(0)
        }
    }
    fn set(&self, at: Instant) {
        self.0.lock().unwrap().0 = at;
    }
    fn queue(&self, readings: &[Instant]) {
        self.0.lock().unwrap().1.extend_from_slice(readings);
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    preference: PathBuf,
    broker: FakeBroker,
    refuse: Arc<Mutex<Option<&'static str>>>,
    clock: Clock,
    access: AgentAccess,
    data_root: String,
    book_open: bool,
    passphrase: Option<String>,
    mode: &'static str,
}

const CONTROL: [u8; 32] = [7; 32];

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let preference = dir.path().join("agent-access.json");
        kassiber_setup::agent_access::set(&preference, Some(true), None).unwrap();
        let broker = FakeBroker::new();
        let refuse: Arc<Mutex<Option<&'static str>>> = Arc::new(Mutex::new(None));
        let clock = Clock::new();
        let access = {
            let (broker, refuse, clock, preference) = (
                broker.clone(),
                Arc::clone(&refuse),
                clock.clone(),
                preference.clone(),
            );
            AgentAccess::with(
                Box::new(move || {
                    if let Some(reason) = *refuse.lock().unwrap() {
                        panic!("{reason}");
                    }
                    Box::new(broker.clone()) as Box<dyn AgentBroker>
                }),
                Box::new(move || clock.now()),
                Box::new(move || Ok(preference.clone())),
                Box::new(|| Ok(Zeroizing::new(CONTROL.to_vec()))),
            )
        };
        Fixture {
            data_root: dir.path().join("data").to_string_lossy().into_owned(),
            _dir: dir,
            preference,
            broker,
            refuse,
            clock,
            access,
            book_open: true,
            passphrase: Some("correct horse".into()),
            mode: "manual",
        }
    }

    fn refuse_broker(&self, reason: &'static str) {
        *self.refuse.lock().unwrap() = Some(reason);
    }

    fn with<T>(&mut self, body: impl FnOnce(&mut AgentAccess, &Desktop<'_>) -> T) -> T {
        let mode = self.mode;
        let unlock_mode = move || mode.to_owned();
        let desktop = Desktop {
            data_root: &self.data_root,
            book_open: self.book_open,
            passphrase: self.passphrase.as_deref(),
            unlock_mode: &unlock_mode,
        };
        body(&mut self.access, &desktop)
    }

    fn state(&mut self, refresh: bool) -> Value {
        self.with(|access, desktop| access.session_state(desktop, refresh))
            .unwrap()
    }
    fn unlock(&mut self) -> Result<Value, AppError> {
        self.with(|access, desktop| access.unlock(desktop))
    }
    fn lock(&mut self) -> Result<Value, AppError> {
        self.with(|access, desktop| access.lock(desktop))
    }
    fn payload(&mut self, kind: &str, args: Value) -> Result<Value, AppError> {
        self.with(|access, desktop| access.payload(desktop, kind, Some(&args)))
    }
    fn lease(&self) -> &DesktopAgentLease {
        self.access.lease.as_ref().expect("a desktop lease")
    }
}

fn json(text: &str) -> Value {
    kassiber_pytext::loads(text).unwrap()
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or_else(|| panic!("{key} missing in {value:?}"))
}

#[test]
fn test_status_never_contacts_the_broker() {
    let mut fixture = Fixture::new();
    fixture.refuse_broker("status must not construct a broker client");
    assert_eq!(
        fixture.state(false),
        json(r#"{"needed": true, "active": false, "expires_at": null}"#)
    );
    let payload = fixture
        .payload("ui.agent_access.status", json("{}"))
        .unwrap();
    assert_eq!(field(&payload, "mcp_available"), &Value::Bool(true));
    assert_eq!(
        field(field(&payload, "session"), "active"),
        &Value::Bool(false)
    );
}

#[test]
fn test_unlock_grants_a_read_lease_only_if_none_exists() {
    let mut fixture = Fixture::new();
    let state = fixture.unlock().unwrap();
    assert_eq!(
        state,
        json(
            r#"{"needed": true, "active": true, "expires_at": "2026-09-28T18:00:00Z", "idle_timeout_seconds": 900}"#
        )
    );
    assert_eq!(
        fixture.broker.calls(),
        [Call::Unlock(
            fixture.data_root.clone(),
            b"correct horse".to_vec()
        )]
    );
    // A fresh random control secret, kept only in the daemon.
    assert_eq!(fixture.lease().control.as_slice(), CONTROL);
    assert!(!format!("{:?}", fixture.lease()).contains("7, 7"));
    // Asking again is idempotent and does not reach the broker.
    fixture.unlock().unwrap();
    assert_eq!(fixture.broker.calls().len(), 1);
}

#[test]
fn test_the_grant_requests_eight_hours_read_password_agent_scope() {
    // The product broker adapter sends exactly Python's unlock options.
    assert_eq!(DESKTOP_AGENT_LEASE_SECONDS, 8 * 60 * 60);
    assert_eq!(AGENT_IDLE_TIMEOUT_SECONDS, 900);
    assert_eq!(CHECK_INTERVAL, Duration::from_secs(30));
    assert_eq!(RETRY_INTERVAL, Duration::from_secs(5));
    assert_eq!(GIVE_UP_AFTER_EXPIRY, Duration::from_secs(60));
}

#[test]
fn test_locking_names_the_lease_and_restores_manual_in_the_broker() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    fixture.lock().unwrap();
    assert_eq!(
        fixture.broker.calls().last(),
        Some(&Call::Lock(
            fixture.data_root.clone(),
            "desktop-lease".into(),
            true
        ))
    );
    assert!(fixture.access.lease.is_none());
}

#[test]
fn test_only_a_book_the_broker_saw_as_manual_is_restored() {
    // The desktop read "manual", but the mode changed before the grant.
    let mut fixture = Fixture::new();
    fixture.broker.state().previous_mode = "brokered";
    fixture.unlock().unwrap();
    fixture.lock().unwrap();
    assert!(matches!(
        fixture.broker.calls().last(),
        Some(Call::Lock(_, _, false))
    ));
}

#[test]
fn test_an_existing_lease_is_neither_replaced_nor_locked() {
    let mut fixture = Fixture::new();
    fixture.broker.state().lease_id = Some("terminal-lease".into());
    let state = fixture.unlock().unwrap();
    assert_eq!(field(&state, "existing_lease"), &Value::Bool(true));
    assert_eq!(field(&state, "active"), &Value::Bool(false));
    assert!(fixture.access.lease.is_none());
    fixture.access.end_lease(false).unwrap();
    assert_eq!(fixture.broker.names(), ["unlock"]);
    assert_eq!(
        fixture.broker.state().lease_id.as_deref(),
        Some("terminal-lease")
    );
}

#[test]
fn test_a_stopped_broker_ends_the_lease_without_touching_the_mode() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    fixture.broker.state().stopped = true;
    fixture.access.end_lease(false).unwrap();
    // Its leases ended with it; nothing writes the mode here.
    assert!(fixture.access.lease.is_none());
    assert_eq!(fixture.broker.names(), ["unlock", "lock", "ping"]);
}

#[test]
fn test_a_failed_lock_keeps_the_lease_visible_and_retries() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    fixture.broker.state().lock_fails = true;
    let error = fixture.lock().unwrap_err();
    assert_eq!(error.code, "agent_lease_lock_failed");
    assert!(error.retryable);
    assert_eq!(field(&fixture.state(false), "active"), &Value::Bool(true));
    // Session-end paths do not raise, and keep the lease for the loop.
    fixture.access.end_lease(false).unwrap();
    assert!(fixture.lease().ending);
    fixture.broker.state().lock_fails = false;
    fixture.access.expire().unwrap(); // before the retry time
    assert!(fixture.access.lease.is_some());
    let retry_at = fixture.lease().retry_at.unwrap();
    fixture.clock.set(retry_at + Duration::from_secs(1));
    fixture.access.expire().unwrap();
    assert!(fixture.access.lease.is_none());
    assert!(fixture.broker.state().lease_id.is_none());
}

#[test]
fn test_a_lost_connection_is_not_taken_for_a_stopped_broker() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    fixture.broker.state().connection_lost = true;
    fixture.access.end_lease(false).unwrap();
    assert!(fixture.lease().ending);
}

#[test]
fn test_the_local_deadline_starts_before_the_grant_request() {
    let mut fixture = Fixture::new();
    let base = Instant::now();
    let (started, granted) = (
        base + Duration::from_secs(100),
        base + Duration::from_secs(250),
    );
    fixture.clock.set(granted);
    fixture.clock.queue(&[started, granted, granted]);
    fixture.unlock().unwrap();
    let lease_seconds = Duration::from_secs(DESKTOP_AGENT_LEASE_SECONDS as u64);
    assert_eq!(fixture.lease().expires, started + lease_seconds);
    // Giving up waits from after the grant, when the broker's lease began.
    assert_eq!(
        fixture.lease().give_up,
        granted + lease_seconds + GIVE_UP_AFTER_EXPIRY
    );
    assert_eq!(fixture.lease().next_check, granted + CHECK_INTERVAL);
}

#[test]
fn test_cleanup_gives_up_once_the_lease_is_long_expired() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    fixture.broker.state().lock_fails = true;
    let late = fixture.lease().give_up + Duration::from_secs(1);
    fixture.clock.set(late);
    fixture.access.expire().unwrap();
    assert!(fixture.access.lease.is_none());
}

#[test]
fn test_an_expired_lease_is_not_shown_and_is_cleaned_up() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    // The broker expired it on its own.
    fixture.broker.state().lease_id = None;
    let after = fixture.lease().expires + Duration::from_secs(1);
    fixture.clock.set(after);
    assert_eq!(field(&fixture.state(false), "active"), &Value::Bool(false));
    fixture.access.expire().unwrap();
    assert!(fixture.access.lease.is_none());
    assert!(matches!(
        fixture.broker.calls().last(),
        Some(Call::Lock(_, _, true))
    ));
}

#[test]
fn test_expire_leaves_a_live_lease_alone() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    fixture.access.expire().unwrap();
    assert!(fixture.access.lease.is_some());
    assert_eq!(fixture.broker.calls().len(), 1);
}

#[test]
fn test_unlock_requires_agents_on() {
    let mut fixture = Fixture::new();
    fixture.refuse_broker("no broker contact while agents are off");
    kassiber_setup::agent_access::set(&fixture.preference, Some(false), None).unwrap();
    assert_eq!(fixture.unlock().unwrap_err().code, "mcp_disabled");
    kassiber_setup::agent_access::set(&fixture.preference, Some(true), Some(false)).unwrap();
    assert_eq!(fixture.unlock().unwrap_err().code, "ai_features_disabled");
}

#[test]
fn test_unlock_requires_an_unlocked_encrypted_book() {
    let mut fixture = Fixture::new();
    fixture.refuse_broker("no broker contact without a passphrase");
    fixture.passphrase = None;
    assert_eq!(field(&fixture.state(false), "needed"), &Value::Bool(false));
    let error = fixture.unlock().unwrap_err();
    assert_eq!(error.code, "agent_unlock_unavailable");
    assert_eq!(
        error.hint.as_deref(),
        Some("Open and unlock the encrypted book first; plaintext books need no unlock.")
    );
    fixture.passphrase = Some("pw".into());
    fixture.book_open = false;
    assert_eq!(
        fixture.unlock().unwrap_err().code,
        "agent_unlock_unavailable"
    );
}

#[test]
fn test_a_remembered_unlock_book_needs_no_agent_unlock() {
    let mut fixture = Fixture::new();
    fixture.refuse_broker("no broker contact for unattended books");
    fixture.mode = "unattended";
    assert_eq!(field(&fixture.state(false), "needed"), &Value::Bool(false));
    assert_eq!(
        fixture.unlock().unwrap_err().code,
        "agent_unlock_not_needed"
    );
}

#[test]
fn test_turning_agents_off_ends_the_agent_lease() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    let payload = fixture
        .payload(
            "ui.agent_access.configure",
            json(r#"{"mcp_enabled": false}"#),
        )
        .unwrap();
    assert_eq!(field(&payload, "mcp_available"), &Value::Bool(false));
    assert_eq!(fixture.broker.names().last(), Some(&"lock"));
    assert_eq!(
        field(field(&payload, "session"), "active"),
        &Value::Bool(false)
    );
    assert_eq!(
        kassiber_envelope::daemon_line(&payload).unwrap(),
        "{\"mcp_enabled\":false,\"ai_features_enabled\":null,\"mcp_available\":false,\"reason\":\"mcp_disabled\",\"session\":{\"needed\":true,\"active\":false,\"expires_at\":null}}\n"
    );
}

#[test]
fn test_a_mock_context_never_reaches_the_broker() {
    let mut fixture = Fixture::new();
    fixture.refuse_broker("mock contexts must not reach the broker");
    fixture.access.end_lease(false).unwrap();
    fixture.access.expire().unwrap();
    assert_eq!(fixture.access.due(), None);
}

#[test]
fn test_a_refresh_without_a_desktop_session_stays_in_memory() {
    let mut fixture = Fixture::new();
    fixture.refuse_broker("no session: a refresh must not contact the broker");
    assert_eq!(field(&fixture.state(true), "active"), &Value::Bool(false));
    let payload = fixture
        .payload("ui.agent_access.status", json(r#"{"refresh": true}"#))
        .unwrap();
    assert_eq!(
        field(field(&payload, "session"), "active"),
        &Value::Bool(false)
    );
}

#[test]
fn test_a_refresh_shows_waiting_agents_and_activity() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    fixture.broker.state().sessions = vec![
        json(r#"{"id": "s1", "label": "claude", "pid": 41, "state": "pending", "calls": 0}"#),
        json(r#"{"id": "s2", "label": "codex", "pid": 42, "state": "allowed", "calls": 3}"#),
        json(r#"{"id": 3, "label": "not shown"}"#),
        json(r#"{"label": "no id"}"#),
    ];
    let state = fixture.state(true);
    assert_eq!(
        kassiber_envelope::daemon_line(&state).unwrap(),
        "{\"needed\":true,\"active\":true,\"expires_at\":\"2026-09-28T18:00:00Z\",\"idle_timeout_seconds\":900,\"calls\":3,\"last_call_at\":null,\"idle_remaining_seconds\":800,\"agents\":[{\"id\":\"s1\",\"label\":\"claude\",\"pid\":41,\"state\":\"pending\",\"calls\":0,\"last_call_at\":null},{\"id\":\"s2\",\"label\":\"codex\",\"pid\":42,\"state\":\"allowed\",\"calls\":3,\"last_call_at\":null}]}\n"
    );
}

#[test]
fn test_a_refresh_notices_a_session_the_broker_ended() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    // Idle timeout in the broker.
    fixture.broker.state().lease_id = None;
    let state = fixture.state(true);
    assert_eq!(field(&state, "active"), &Value::Bool(false));
    assert!(fixture.access.lease.is_none());
    assert_eq!(fixture.broker.names().last(), Some(&"lock"));
}

#[test]
fn test_allowing_an_agent_proves_the_control_secret() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    fixture.broker.state().sessions = vec![json(
        r#"{"id": "s1", "label": "claude", "pid": 41, "state": "pending", "calls": 0}"#,
    )];
    let payload = fixture
        .payload(
            "ui.agent_access.pairing",
            json(r#"{"session_id": "s1", "allow": true}"#),
        )
        .unwrap();
    assert_eq!(
        fixture.broker.state().decisions,
        [("s1".to_owned(), true, CONTROL.to_vec())]
    );
    let agents = field(field(&payload, "session"), "agents");
    let Value::List(agents) = agents else {
        panic!("{agents:?}")
    };
    assert_eq!(field(&agents[0], "state"), &s("allowed"));
}

#[test]
fn test_pairing_needs_the_desktop_session_and_valid_arguments() {
    let mut fixture = Fixture::new();
    fixture.refuse_broker("no session: nothing to decide");
    let error = fixture
        .with(|access, desktop| access.decide(desktop, Some(&s("s1")), Some(&Value::Bool(true))))
        .unwrap_err();
    assert_eq!(error.code, "agent_unlock_unavailable");
    for args in [
        r#"{"session_id": "s1"}"#,
        r#"{"session_id": "", "allow": true}"#,
        r#"{"allow": "yes", "session_id": "s1"}"#,
        r#"{"session_id": 5, "allow": true}"#,
    ] {
        let error = fixture
            .payload("ui.agent_access.pairing", json(args))
            .unwrap_err();
        assert_eq!(error.code, "validation", "{args}");
        assert_eq!(
            error.message,
            "ui.agent_access.pairing takes a session_id and a boolean allow"
        );
    }
    let error = fixture
        .payload("ui.agent_access.status", json(r#"{"refresh": "yes"}"#))
        .unwrap_err();
    assert_eq!(
        error.message,
        "ui.agent_access.status got unexpected arguments"
    );
    assert_eq!(error.details, Some(json(r#"{"unknown": []}"#)));
    // A refresh flag is an unknown argument of pairing.
    let error = fixture
        .payload(
            "ui.agent_access.pairing",
            json(r#"{"session_id": "s1", "allow": true, "refresh": false}"#),
        )
        .unwrap_err();
    assert_eq!(error.details, Some(json(r#"{"unknown": ["refresh"]}"#)));
}

#[test]
fn test_the_loop_checks_a_live_session_only_every_so_often() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    fixture.access.expire().unwrap();
    assert_eq!(fixture.broker.names(), ["unlock"]);
    assert_eq!(fixture.access.due(), Some(fixture.lease().next_check));
    // Ended in the broker.
    fixture.broker.state().lease_id = None;
    let later = fixture.lease().next_check + Duration::from_secs(1);
    fixture.clock.set(later);
    fixture.access.expire().unwrap();
    assert_eq!(fixture.broker.names(), ["unlock", "status", "lock"]);
    assert!(fixture.access.lease.is_none());
}

#[test]
fn test_a_live_check_waits_another_thirty_seconds() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    let first = fixture.lease().next_check;
    fixture.clock.set(first);
    fixture.access.expire().unwrap();
    assert_eq!(fixture.broker.names(), ["unlock", "status"]);
    assert_eq!(fixture.lease().next_check, first + CHECK_INTERVAL);
    fixture
        .clock
        .set(first + CHECK_INTERVAL - Duration::from_millis(1));
    fixture.access.expire().unwrap();
    assert_eq!(fixture.broker.names(), ["unlock", "status"]);
    // A broker that cannot be reached keeps the session as it is.
    fixture.broker.state().status_fails = true;
    fixture.clock.set(first + CHECK_INTERVAL);
    fixture.access.expire().unwrap();
    assert!(fixture.access.lease.is_some());
}

#[test]
fn test_unlock_and_lock_take_no_arguments() {
    let mut fixture = Fixture::new();
    for kind in ["ui.agent_access.unlock", "ui.agent_access.lock"] {
        let error = fixture
            .payload(kind, json(r#"{"capability": "admin", "a": 1}"#))
            .unwrap_err();
        assert_eq!(error.code, "validation");
        assert_eq!(error.message, format!("{kind} got unexpected arguments"));
        assert_eq!(
            error.details,
            Some(json(r#"{"unknown": ["a", "capability"]}"#))
        );
    }
    // Falsy arguments are no arguments; anything else must be an object.
    fixture.refuse_broker("a status read stays in memory");
    for args in ["null", "[]", "0", "false", "\"\""] {
        fixture
            .payload("ui.agent_access.status", json(args))
            .unwrap();
    }
    for args in ["[1]", "1", "\"x\""] {
        let error = fixture
            .payload("ui.agent_access.status", json(args))
            .unwrap_err();
        assert_eq!(error.message, "agent access arguments must be an object");
    }
}

#[test]
fn test_configure_takes_boolean_fields_only() {
    let mut fixture = Fixture::new();
    fixture.refuse_broker("configuring without a session stays in memory");
    for args in [
        r#"{}"#,
        r#"{"mcp_enabled": 1}"#,
        r#"{"mcp_enabled": true, "extra": 1}"#,
    ] {
        let error = fixture
            .payload("ui.agent_access.configure", json(args))
            .unwrap_err();
        assert_eq!(
            error.message,
            "ui.agent_access.configure takes boolean mcp_enabled and/or ai_features_enabled",
            "{args}"
        );
    }
    let payload = fixture
        .payload(
            "ui.agent_access.configure",
            json(r#"{"ai_features_enabled": false}"#),
        )
        .unwrap();
    assert_eq!(field(&payload, "reason"), &s("ai_features_disabled"));
    assert_eq!(
        std::fs::read(&fixture.preference).unwrap(),
        b"{\"ai_features_enabled\": false, \"mcp_enabled\": true, \"schema_version\": 1}\n"
    );
}

/// A daemon whose agent access is the fixture's.
fn desktop_daemon(fixture: &mut Fixture) -> crate::Daemon {
    let store = kassiber_runtime::fake::FakeStore::new(Default::default());
    let bootstrap =
        kassiber_runtime::store::BookStore::bootstrap(&store, &Default::default()).unwrap();
    let mut daemon = crate::Daemon::new(
        std::sync::Arc::new(store),
        crate::DaemonConfig {
            build: kassiber_runtime::BuildInfo {
                version: "synthetic",
                channel: None,
                commit: None,
            },
            bootstrap,
        },
        crate::output::Output::new(Box::new(std::io::sink())),
    );
    std::mem::swap(&mut daemon.agent_access, &mut fixture.access);
    daemon.unlocked_passphrase = Some(kassiber_runtime::store::Secret::new("correct horse".into()));
    daemon
}

#[test]
fn test_ending_the_desktop_session_ends_the_agent_lease() {
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    let mut daemon = desktop_daemon(&mut fixture);
    daemon.lock().unwrap();
    assert_eq!(fixture.broker.names().last(), Some(&"lock"));
    assert!(fixture.broker.state().lease_id.is_none());
    assert!(daemon.agent_access.lease.is_none());
    assert!(daemon.unlocked_passphrase.is_none());

    // So does the daemon's exit.
    let mut fixture = Fixture::new();
    fixture.unlock().unwrap();
    let mut daemon = desktop_daemon(&mut fixture);
    daemon.shutdown_resources();
    assert_eq!(fixture.broker.names(), ["unlock", "lock"]);
    assert!(daemon.agent_access.lease.is_none());
}

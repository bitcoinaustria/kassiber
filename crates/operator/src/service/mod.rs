//! In-memory leases, per-project queues and recoverable operation state
//! (`kassiber/operator/service.py`).
//!
//! Python guards the service with one re-entrant lock, a re-entrant gate per
//! project and a condition for transitions. Here the state lives behind one
//! `Mutex` that no code holds while calling out (to storage, ownership or a
//! child), project gates are re-entrant per thread, and work that Python
//! does under its re-entrant lock is split into `_locked` helpers taking the
//! guarded state. Every observable step keeps Python's order: which check
//! fails first, which message a cancelled operation carries, and which
//! owner release is retried when.

mod backend;
mod ordered;
#[cfg(test)]
mod tests;

pub use backend::*;

use crate::clock::{format_unix_seconds, now_iso, sha256_hex, token_urlsafe, unix_seconds};
use crate::error::{Error, Result};
use crate::protocol::{constant_time_eq, token_hex, MAX_JSON_FRAME};
use crate::telemetry::{Discard, Level, Telemetry};
use kassiber_envelope::redaction::{redact_secret_text, sanitize_traceback_text};
use kassiber_envelope::{dict, set, AppError};
use kassiber_ownership::CanonicalProject;
use kassiber_pyjson::{BigInt, Value};
use ordered::Ordered;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{JoinHandle, ThreadId};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub const MAX_QUEUED_OPERATIONS: usize = 64;
pub const MAX_RETAINED_RESULTS: usize = 256;
pub const MAX_RETAINED_RESULT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_RETAINED_OPERATION_TOMBSTONES: usize = 1024;
pub const MAX_CACHED_AUTH_BACKOFFS: usize = 256;
pub const ADMIN_AUTH_TTL: Duration = Duration::from_secs(60);
pub const MAX_AGENT_SESSIONS_PER_PROJECT: usize = 8;
pub const MAX_AGENT_SESSIONS: usize = 16;
pub const MIN_AGENT_CONTROL_BYTES: usize = 32;
pub const MIN_AGENT_IDLE_TIMEOUT_SECONDS: i64 = 60;
pub const MAX_REMEMBERED_AGENT_DENIALS: usize = 256;
const OPERATION_STATUS_FRAME_HEADROOM: usize = 64 * 1024;

/// Commands whose completion ends the lease, with the reason queued work
/// is cancelled under.
fn lease_ending_reason(command_path: &str) -> Option<&'static str> {
    match command_path {
        "secrets.change-passphrase" => Some("database passphrase rotation ended the prior lease"),
        "secrets.remember-unlock" => {
            Some("remembered unlock enrollment ended the prior brokered lease")
        }
        "secrets.forget-unlock" => Some("remembered unlock removal ended the prior brokered lease"),
        _ => None,
    }
}

/// The retention and timing limits; tests narrow them as Python's patch
/// the module constants.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_queued_operations: usize,
    pub max_retained_results: usize,
    pub max_retained_result_bytes: usize,
    pub max_tombstones: usize,
    pub max_cached_backoffs: usize,
    pub admin_auth_ttl: Duration,
    pub janitor_interval: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_queued_operations: MAX_QUEUED_OPERATIONS,
            max_retained_results: MAX_RETAINED_RESULTS,
            max_retained_result_bytes: MAX_RETAINED_RESULT_BYTES,
            max_tombstones: MAX_RETAINED_OPERATION_TOMBSTONES,
            max_cached_backoffs: MAX_CACHED_AUTH_BACKOFFS,
            admin_auth_ttl: ADMIN_AUTH_TTL,
            janitor_interval: Duration::from_secs(1),
        }
    }
}

/// An operation's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    ResultUnknown,
}

impl OpState {
    pub fn as_str(self) -> &'static str {
        match self {
            OpState::Queued => "queued",
            OpState::Running => "running",
            OpState::Completed => "completed",
            OpState::Failed => "failed",
            OpState::Cancelled => "cancelled",
            OpState::ResultUnknown => "result_unknown",
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, OpState::Queued | OpState::Running)
    }
}

/// `Operation`.
pub(crate) struct Operation {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) project_identity: String,
    pub(crate) database_identity: String,
    pub(crate) data_root: String,
    pub(crate) argv: Vec<String>,
    pub(crate) command_path: String,
    pub(crate) capability: Capability,
    pub(crate) secret_arguments: Vec<(String, Zeroizing<Vec<u8>>)>,
    pub(crate) admitted_lease_epoch: Option<String>,
    pub(crate) admin_authorized_until: Option<Instant>,
    pub(crate) working_directory: Option<String>,
    pub(crate) no_egress: bool,
    pub(crate) agent_session_id: Option<String>,
    pub(crate) agent_pid: Option<u32>,
    pub(crate) output_withheld: bool,
    pub(crate) state: OpState,
    pub(crate) submitted_at: String,
    pub(crate) started_at: Option<String>,
    pub(crate) finished_at: Option<String>,
    pub(crate) result: Option<OperationResult>,
    pub(crate) output_error: Option<Value>,
    pub(crate) retained_result_bytes: usize,
    pub(crate) cancellation_requested: bool,
}

/// A JSON `str`.
fn s(text: &str) -> Value {
    kassiber_envelope::s(text)
}

fn text_or_null(value: &Option<String>) -> Value {
    value.as_deref().map_or(Value::Null, s)
}

fn int(value: impl Into<BigInt>) -> Value {
    Value::Int(value.into())
}

impl Operation {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: String,
        project_id: String,
        project_identity: String,
        database_identity: String,
        data_root: String,
        argv: Vec<String>,
        command_path: String,
        capability: Capability,
    ) -> Self {
        Operation {
            id,
            project_id,
            project_identity,
            database_identity,
            data_root,
            argv,
            command_path,
            capability,
            secret_arguments: Vec::new(),
            admitted_lease_epoch: None,
            admin_authorized_until: None,
            working_directory: None,
            no_egress: false,
            agent_session_id: None,
            agent_pid: None,
            output_withheld: false,
            state: OpState::Queued,
            submitted_at: now_iso(),
            started_at: None,
            finished_at: None,
            result: None,
            output_error: None,
            retained_result_bytes: 0,
            cancellation_requested: false,
        }
    }

    /// `public_status(include_output=...)`.
    pub(crate) fn public_status(&self, include_output: bool) -> Value {
        let mut payload = dict([
            ("operation_id", s(&self.id)),
            ("project", s(&self.project_id)),
            ("command", s(&self.command_path)),
            ("capability", s(self.capability.as_str())),
            ("state", s(self.state.as_str())),
            ("submitted_at", s(&self.submitted_at)),
            ("started_at", text_or_null(&self.started_at)),
            ("finished_at", text_or_null(&self.finished_at)),
        ]);
        if self.output_withheld {
            set(&mut payload, "output_available", Value::Bool(false));
            set(&mut payload, "output_withheld", s("agent_denied"));
        } else if let Some(result) = &self.result {
            set(&mut payload, "exit_code", int(result.exit_code));
            if let Some(error) = &self.output_error {
                set(&mut payload, "output_available", Value::Bool(false));
                set(&mut payload, "output_error", error.clone());
            } else if include_output {
                set(&mut payload, "stdout", s(&result.stdout));
                set(&mut payload, "stderr", s(&result.stderr));
            }
        }
        payload
    }
}

type SharedOwner = Arc<Mutex<Box<dyn Owner>>>;

/// The default workspace and profile ids a book's context names.
type Scope = (Option<String>, Option<String>);

fn locked<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// `ProjectLease`.
pub(crate) struct Lease {
    pub(crate) data_root: String,
    pub(crate) project: CanonicalProject,
    pub(crate) database_identity: String,
    pub(crate) passphrase: Zeroizing<Vec<u8>>,
    pub(crate) capability: Capability,
    pub(crate) owner: SharedOwner,
    pub(crate) unlocked_at: String,
    pub(crate) expires_at: Option<Instant>,
    pub(crate) duration_seconds: Option<i64>,
    pub(crate) authentication_method: String,
    pub(crate) expires_at_text: Option<String>,
    pub(crate) epoch: String,
    pub(crate) workspace: Option<String>,
    pub(crate) profile: Option<String>,
    pub(crate) revoked: bool,
    pub(crate) running_operations: usize,
    pub(crate) agent_scope: bool,
    pub(crate) agent_control_digest: Option<Vec<u8>>,
    pub(crate) idle_timeout_seconds: Option<i64>,
    pub(crate) last_used: Instant,
    pub(crate) calls: u64,
    pub(crate) last_call_at: Option<String>,
}

impl Lease {
    pub(crate) fn expired(&self) -> bool {
        let now = Instant::now();
        if self.expires_at.is_some_and(|deadline| now >= deadline) {
            return true;
        }
        self.idle_timeout_seconds.is_some_and(|timeout| {
            now.saturating_duration_since(self.last_used).as_secs_f64() >= timeout as f64
        })
    }

    /// `_public_lease_id`: names one lease without revealing its epoch.
    pub(crate) fn public_id(&self) -> String {
        sha256_hex(format!("lease-id:{}", self.epoch).as_bytes())[..24].to_owned()
    }
}

/// `AgentSession`.
pub(crate) struct AgentSession {
    pub(crate) id: String,
    pub(crate) project_identity: String,
    pub(crate) lease_epoch: String,
    pub(crate) pid: u32,
    pub(crate) label: Option<String>,
    pub(crate) opened_at: String,
    pub(crate) state: &'static str,
    pub(crate) calls: u64,
    pub(crate) last_call_at: Option<String>,
    pub(crate) closed: bool,
    pub(crate) token: String,
}

impl AgentSession {
    fn public_status(&self) -> Value {
        dict([
            ("id", s(&self.id)),
            ("label", text_or_null(&self.label)),
            ("pid", int(self.pid)),
            ("state", s(self.state)),
            ("opened_at", s(&self.opened_at)),
            ("calls", int(self.calls)),
            ("last_call_at", text_or_null(&self.last_call_at)),
        ])
    }
}

/// What an opened agent session hands its process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSessionTicket {
    pub id: String,
    pub state: String,
    pub token: String,
}

/// `AdminAuthorization`: single-use fresh authorization bound to one lease.
pub struct AdminAuthorization {
    project_identity: String,
    database_identity: String,
    lease_epoch: String,
    expires_at: Instant,
    consumed: AtomicBool,
}

impl std::fmt::Debug for AdminAuthorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AdminAuthorization([redacted])")
    }
}

impl AdminAuthorization {
    #[cfg(test)]
    pub(crate) fn expire_now(&mut self) {
        self.expires_at = Instant::now() - Duration::from_secs(1);
    }
}

/// A connection or owner whose close failed and is retried later
/// (`_PendingProjectResource`).
struct PendingResource {
    project_identity: String,
    connection: Option<Box<dyn Database>>,
    owner: Option<SharedOwner>,
}

/// A re-entrant per-thread gate (`threading.RLock`).
#[derive(Default)]
struct Gate {
    holder: Mutex<(Option<ThreadId>, usize)>,
    released: Condvar,
}

impl Gate {
    fn enter(&self) {
        let me = std::thread::current().id();
        let mut holder = locked(&self.holder);
        loop {
            match holder.0 {
                None => {
                    *holder = (Some(me), 1);
                    return;
                }
                Some(owner) if owner == me => {
                    holder.1 += 1;
                    return;
                }
                Some(_) => {
                    holder = self
                        .released
                        .wait(holder)
                        .unwrap_or_else(|poison| poison.into_inner());
                }
            }
        }
    }

    fn exit(&self) {
        let mut holder = locked(&self.holder);
        holder.1 = holder.1.saturating_sub(1);
        if holder.1 == 0 {
            holder.0 = None;
            self.released.notify_all();
        }
    }
}

/// One project's FIFO queue and its worker thread (`ProjectWorker`).
pub(crate) struct Worker {
    pub(crate) project_identity: String,
    pub(crate) project_id: String,
    queue: Mutex<(VecDeque<String>, bool)>,
    ready: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Worker {
    pub(crate) fn queued(&self) -> usize {
        locked(&self.queue).0.len()
    }

    fn submit(&self, operation_id: &str, limit: usize) -> Result<()> {
        let mut queue = locked(&self.queue);
        if queue.0.len() >= limit {
            return Err(Error::App(
                AppError::new("operator_queue_full", "the project operation queue is full")
                    .with_details(dict([
                        ("project", s(&self.project_id)),
                        ("limit", int(limit as u64)),
                    ]))
                    .retryable(true),
            ));
        }
        queue.0.push_back(operation_id.to_owned());
        self.ready.notify_one();
        Ok(())
    }

    fn remove(&self, operation_id: &str) -> bool {
        let mut queue = locked(&self.queue);
        match queue.0.iter().position(|id| id == operation_id) {
            Some(index) => {
                queue.0.remove(index);
                true
            }
            None => false,
        }
    }

    fn drain(&self) -> Vec<String> {
        locked(&self.queue).0.drain(..).collect()
    }

    fn stop(&self) {
        locked(&self.queue).1 = true;
        self.ready.notify_all();
    }

    fn next(&self) -> Option<String> {
        let mut queue = locked(&self.queue);
        loop {
            if let Some(id) = queue.0.pop_front() {
                return Some(id);
            }
            if queue.1 {
                return None;
            }
            queue = self
                .ready
                .wait(queue)
                .unwrap_or_else(|poison| poison.into_inner());
        }
    }

    /// Whether the worker thread has exited.
    #[cfg(test)]
    pub(crate) fn finished(&self) -> bool {
        locked(&self.thread)
            .as_ref()
            .is_none_or(|handle| handle.is_finished())
    }
}

#[derive(Default)]
struct State {
    leases: Ordered<Lease>,
    agent_sessions: Vec<AgentSession>,
    agent_denials: Vec<(String, String, u32)>,
    workers: HashMap<String, Arc<Worker>>,
    operations: Ordered<Operation>,
    tombstones: Ordered<(String, String)>,
    lease_aliases: HashMap<String, String>,
    auth_backoffs: Ordered<Arc<crate::backoff::AuthAttemptBackoff>>,
    gates: HashMap<String, (Arc<Gate>, usize)>,
    pending_owner_releases: VecDeque<PendingResource>,
    active_project_transitions: usize,
    sequence: u64,
    closed: bool,
    close_complete: bool,
    close_in_progress: bool,
    close_prepared: bool,
}

struct Inner {
    generation: String,
    backend: Arc<dyn Backend>,
    runner: Arc<dyn Runner>,
    telemetry: Arc<dyn Telemetry>,
    limits: Limits,
    state: Mutex<State>,
    transitions: Condvar,
    janitor_wake: Condvar,
    janitor: Mutex<Option<JoinHandle<()>>>,
}

/// `OperatorService`.
#[derive(Clone)]
pub struct OperatorService {
    inner: Arc<Inner>,
}

/// `unlock(...)`'s keyword arguments.
#[derive(Clone, Debug)]
pub struct UnlockRequest<'a> {
    pub duration_seconds: Option<i64>,
    pub capability: Capability,
    pub authentication_method: &'a str,
    pub expected_project_identity: Option<&'a str>,
    pub expected_database_identity: Option<&'a str>,
    pub only_if_locked: bool,
    pub agent_scope: bool,
    pub idle_timeout_seconds: Option<i64>,
    pub agent_control: Option<&'a [u8]>,
}

impl Default for UnlockRequest<'_> {
    fn default() -> Self {
        UnlockRequest {
            duration_seconds: None,
            capability: Capability::AccountingDecisions,
            authentication_method: "password",
            expected_project_identity: None,
            expected_database_identity: None,
            only_if_locked: false,
            agent_scope: false,
            idle_timeout_seconds: None,
            agent_control: None,
        }
    }
}

/// `submit(...)`'s keyword arguments.
#[derive(Default)]
pub struct SubmitRequest<'a> {
    pub operation_id: Option<String>,
    pub secret_arguments: Vec<(String, Zeroizing<Vec<u8>>)>,
    pub admin_authorization: Option<&'a AdminAuthorization>,
    pub working_directory: Option<String>,
    pub no_egress: bool,
    pub peer_pid: Option<u32>,
    pub agent_session_token: Option<String>,
}

fn app(code: &str, message: &str) -> AppError {
    AppError::new(code, message)
}

fn broker_stopped() -> Error {
    Error::App(app("operator_broker_stopped", "the operator broker is stopping").retryable(true))
}

fn project_replaced(message: &str) -> Error {
    Error::App(app("operator_project_replaced", message))
}

fn protocol_error(message: &str) -> Error {
    Error::App(app("operator_protocol_error", message))
}

/// `_decode_secret`.
fn decode_secret(secret: &[u8]) -> Result<&str> {
    std::str::from_utf8(secret).map_err(|_| {
        Error::App(app(
            "invalid_passphrase_encoding",
            "database passphrase must be UTF-8",
        ))
    })
}

/// `_result_exceeds_protocol_frame`: measured on the ASCII-escaped length.
fn result_exceeds_protocol_frame(result: &OperationResult) -> bool {
    let payload = dict([("stderr", s(&result.stderr)), ("stdout", s(&result.stdout))]);
    let encoded = kassiber_pyjson::dumps(
        &payload,
        &kassiber_pyjson::Options::new().compact().sort_keys(true),
    )
    .map_or(usize::MAX, |text| text.len());
    encoded > MAX_JSON_FRAME - OPERATION_STATUS_FRAME_HEADROOM
}

/// `_operation_request_fingerprint`: a replayed id must repeat the argv and
/// the caller context.
pub(crate) fn request_fingerprint(
    argv: &[String],
    working_directory: Option<&str>,
    no_egress: bool,
) -> String {
    let payload = Value::List(vec![
        Value::List(argv.iter().map(|item| s(item)).collect()),
        working_directory.map_or(Value::Null, s),
        Value::Bool(no_egress),
    ]);
    let encoded = kassiber_pyjson::dumps(&payload, &kassiber_pyjson::Options::new().compact())
        .expect("fingerprint JSON");
    sha256_hex(encoded.as_bytes())
}

/// `_require_explicit_scope`.
fn require_explicit_scope(parsed: &Classified) -> Result<()> {
    let workspace = parsed.workspace.clone().flatten();
    let profile = parsed.profile.clone().flatten();
    let missing: Vec<&str> = if parsed.command_path == "context.set" {
        if workspace.is_some() || profile.is_some() {
            vec![]
        } else {
            vec!["workspace_or_profile"]
        }
    } else {
        [
            ("workspace", parsed.workspace.is_some(), &workspace),
            ("profile", parsed.profile.is_some(), &profile),
        ]
        .into_iter()
        .filter(|(_, declared, value)| *declared && value.is_none())
        .map(|(name, _, _)| name)
        .collect()
    };
    if missing.is_empty() {
        return Ok(());
    }
    Err(Error::App(
        app(
            "operator_scope_required",
            "brokered commands require explicit book scope",
        )
        .with_hint("Pass the command's --workspace and --profile flags explicitly.")
        .with_details(dict([
            ("command", s(&parsed.command_path)),
            ("missing", Value::List(missing.into_iter().map(s).collect())),
        ])),
    ))
}

/// `_classification_argv`: a broker secret label after an `--...-fd` flag
/// parses as descriptor 0.
pub fn classification_argv(argv: &[String]) -> Vec<String> {
    let mut normalized = argv.to_vec();
    for index in 1..normalized.len() {
        if normalized[index].starts_with("broker-secret-")
            && normalized[index - 1].starts_with("--")
            && normalized[index - 1].ends_with("-fd")
        {
            normalized[index] = "0".to_owned();
        }
    }
    normalized
}

/// `_pin_project_arguments`: drop the caller's `--project` and
/// `--data-root` and pin the canonical root first.
pub fn pin_project_arguments(argv: &[String], data_root: &str) -> Vec<String> {
    let mut pinned = vec!["--data-root".to_owned(), data_root.to_owned()];
    let mut index = 0;
    while index < argv.len() {
        let token = &argv[index];
        if token == "--project" || token == "--data-root" {
            index += 2;
            continue;
        }
        if token.starts_with("--project=") || token.starts_with("--data-root=") {
            index += 1;
            continue;
        }
        pinned.push(token.clone());
        index += 1;
    }
    pinned
}

/// `secrets.token_hex(4)`.
fn short_token() -> Result<String> {
    token_hex(4)
}

impl OperatorService {
    pub fn new(generation: &str, backend: Arc<dyn Backend>, runner: Arc<dyn Runner>) -> Self {
        Self::with_options(
            generation,
            backend,
            runner,
            Arc::new(Discard),
            Limits::default(),
        )
    }

    pub fn with_options(
        generation: &str,
        backend: Arc<dyn Backend>,
        runner: Arc<dyn Runner>,
        telemetry: Arc<dyn Telemetry>,
        limits: Limits,
    ) -> Self {
        let service = OperatorService {
            inner: Arc::new(Inner {
                generation: generation.to_owned(),
                backend,
                runner,
                telemetry,
                limits,
                state: Mutex::new(State::default()),
                transitions: Condvar::new(),
                janitor_wake: Condvar::new(),
                janitor: Mutex::new(None),
            }),
        };
        let janitor = service.clone();
        let handle = std::thread::Builder::new()
            .name("operator-lease-expiry".into())
            .spawn(move || janitor.expire_leases())
            .expect("spawn the lease janitor");
        *locked(&service.inner.janitor) = Some(handle);
        service
    }

    pub fn generation(&self) -> &str {
        &self.inner.generation
    }

    fn state(&self) -> MutexGuard<'_, State> {
        locked(&self.inner.state)
    }

    fn log(&self, level: Level, message: &str, fields: Vec<(&str, Value)>) {
        self.inner.telemetry.record(level, message, fields);
    }

    /// `_project_transition`: hold the project's gate, not the state lock,
    /// then release detached owners for the project.
    fn with_transition<T>(
        &self,
        identity: &str,
        allow_closed: bool,
        work: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        struct Entered<'a> {
            service: &'a OperatorService,
            identity: &'a str,
            gate: Arc<Gate>,
            allow_closed: bool,
        }
        impl Drop for Entered<'_> {
            fn drop(&mut self) {
                self.gate.exit();
                let mut state = self.service.state();
                if let Some(entry) = state.gates.get_mut(self.identity) {
                    entry.1 = entry.1.saturating_sub(1);
                }
                if !self.allow_closed {
                    state.active_project_transitions =
                        state.active_project_transitions.saturating_sub(1);
                    self.service.inner.transitions.notify_all();
                }
                let unused = state
                    .gates
                    .get(self.identity)
                    .is_some_and(|entry| entry.1 == 0)
                    && !state.leases.contains_key(self.identity)
                    && !state
                        .pending_owner_releases
                        .iter()
                        .any(|resource| resource.project_identity == self.identity);
                if unused {
                    state.gates.remove(self.identity);
                }
            }
        }
        let gate = {
            let mut state = self.state();
            if state.closed && !allow_closed {
                return Err(broker_stopped());
            }
            let entry = state
                .gates
                .entry(identity.to_owned())
                .or_insert_with(|| (Arc::new(Gate::default()), 0));
            entry.1 += 1;
            let gate = Arc::clone(&entry.0);
            if !allow_closed {
                state.active_project_transitions += 1;
            }
            gate
        };
        gate.enter();
        let entered = Entered {
            service: self,
            identity,
            gate,
            allow_closed,
        };
        let result = work();
        let released = self.release_pending_owners(Some(identity));
        drop(entered);
        // An exception in Python's `finally` replaces the one in flight.
        released?;
        result
    }

    // ----- unlock and fresh authentication -----

    /// `unlock(data_root, passphrase, ...)`.
    pub fn unlock(
        &self,
        data_root: &str,
        passphrase: &[u8],
        request: UnlockRequest<'_>,
    ) -> Result<Value> {
        if request.agent_scope {
            if request.capability != Capability::Read {
                return Err(Error::App(app(
                    "operator_invalid_lease_capability",
                    "an agent session is read-only",
                )));
            }
            if request
                .agent_control
                .is_none_or(|control| control.len() < MIN_AGENT_CONTROL_BYTES)
            {
                return Err(protocol_error(
                    "an agent session needs its holder's control secret",
                ));
            }
            if request
                .idle_timeout_seconds
                .is_none_or(|timeout| timeout < MIN_AGENT_IDLE_TIMEOUT_SECONDS)
            {
                return Err(Error::App(app(
                    "operator_invalid_duration",
                    "an agent session needs an idle timeout of at least 1 minute",
                )));
            }
        } else if request.idle_timeout_seconds.is_some() || request.agent_control.is_some() {
            return Err(protocol_error(
                "idle timeouts and control secrets apply to agent sessions only",
            ));
        }
        if request.capability == Capability::Admin {
            return Err(Error::App(app(
                "operator_invalid_lease_capability",
                "admin is not a lease capability",
            )));
        }
        if !["password", "touch_id"].contains(&request.authentication_method) {
            return Err(Error::App(app(
                "operator_invalid_authentication_method",
                "unsupported operator authentication method",
            )));
        }
        if request.duration_seconds.is_some_and(|seconds| seconds < 60) {
            return Err(Error::App(app(
                "operator_invalid_duration",
                "operator lease duration must be at least 1 minute",
            )));
        }
        let requested_expires_at = match request.duration_seconds {
            None => None,
            Some(seconds) => match unix_seconds().checked_add(seconds) {
                Some(end) if end <= crate::clock::MAX_UNIX_SECONDS => {
                    Some(format_unix_seconds(end))
                }
                _ => {
                    return Err(Error::App(app(
                        "operator_invalid_duration",
                        "operator lease duration exceeds the timestamp range",
                    )))
                }
            },
        };
        let project = self.inner.backend.canonical_project(data_root)?;
        if request
            .expected_project_identity
            .is_some_and(|expected| expected != project.identity)
        {
            return Err(project_replaced(
                "native authentication targeted a different project file",
            ));
        }
        let canonical_data_root = parent_text(&project);
        let alias = project.database.to_string_lossy().into_owned();
        let previous_mode = self.with_transition(&project.identity, false, || {
            self.release_pending_owners(Some(&project.identity))?;
            let (previous, backoff) = {
                let mut state = self.state();
                if let Some(replaced) = state.lease_aliases.get(&alias) {
                    if *replaced != project.identity {
                        return Err(Error::App(
                            app(
                                "operator_project_replaced",
                                "the database file changed while its prior operator lease is active",
                            )
                            .with_hint(
                                "Lock the prior lease before unlocking the replacement database.",
                            ),
                        ));
                    }
                }
                let mut previous = state
                    .leases
                    .get(&project.identity)
                    .map(|lease| (lease.epoch.clone(), lease.revoked, lease.expired()));
                if let Some((_, revoked, expired)) = previous {
                    if revoked || expired {
                        let reason = if expired {
                            "operator lease expired"
                        } else {
                            "operator lease was revoked"
                        };
                        self.revoke_lease_locked(&mut state, &project.identity, reason);
                        let running = state
                            .leases
                            .get(&project.identity)
                            .map_or(0, |lease| lease.running_operations);
                        if running == 0 {
                            previous = None;
                        }
                    }
                }
                let previous_lease = previous
                    .as_ref()
                    .and_then(|_| state.leases.get(&project.identity));
                if request.only_if_locked && previous_lease.is_some_and(|lease| !lease.revoked) {
                    return Err(Error::App(
                        app(
                            "operator_lease_exists",
                            "the project already has an operator lease",
                        )
                        .with_details(dict([("project", s(&project.public_id))])),
                    ));
                }
                if previous_lease.is_some_and(|lease| lease.running_operations > 0) {
                    return Err(Error::App(
                        app(
                            "operator_project_busy",
                            "the project still has a running operator operation",
                        )
                        .with_hint(
                            "Wait for the running operation to finish, then refresh the lease.",
                        )
                        .with_details(dict([("project", s(&project.public_id))]))
                        .retryable(true),
                    ));
                }
                let previous = previous_lease.map(|lease| {
                    (
                        lease.epoch.clone(),
                        Arc::clone(&lease.owner),
                        lease.database_identity.clone(),
                    )
                });
                let backoff =
                    self.auth_backoff_locked(&mut state, &project.identity, &canonical_data_root)?;
                (previous, backoff)
            };
            self.release_pending_owners(Some(&project.identity))?;
            let acquired_here = previous.is_none();
            let owner: SharedOwner = match &previous {
                Some((_, owner, _)) => Arc::clone(owner),
                None => Arc::new(Mutex::new(
                    self.inner
                        .backend
                        .acquire_owner(&project, &self.inner.generation)?,
                )),
            };
            let mut owner_retained = false;
            let opened = (|| -> Result<(String, Scope)> {
                if previous.is_some() {
                    locked(&owner).add_alias(&project)?;
                }
                backoff.check("operator_unlock").map_err(Error::App)?;
                let mut connection = self.inner.backend.open_database(
                    &canonical_data_root,
                    decode_secret(passphrase)?,
                    request.expected_database_identity,
                )?;
                let read = (|| -> Result<(String, Scope)> {
                    let identity = connection
                        .instance_id()?
                        .unwrap_or_else(|| project.identity[..32].to_owned());
                    let context = connection.context()?;
                    if request
                        .expected_database_identity
                        .is_some_and(|expected| expected != identity)
                    {
                        return Err(project_replaced(
                            "native authentication opened a different project database",
                        ));
                    }
                    Ok((identity, context))
                })();
                if let Err(error) = connection.close() {
                    self.retain_pending_resource(
                        &project.identity,
                        Some(connection),
                        acquired_here.then(|| Arc::clone(&owner)),
                    );
                    owner_retained = acquired_here;
                    return Err(error);
                }
                read
            })();
            let (opened_identity, context) = match opened {
                Ok(opened) => {
                    backoff.record_success();
                    opened
                }
                Err(error) => {
                    if matches!(error.code(), Some("unlock_failed" | "passphrase_required")) {
                        backoff.record_failure();
                    }
                    if acquired_here && !owner_retained {
                        self.release_owner_or_retain(&project.identity, &owner)?;
                    }
                    return Err(error);
                }
            };
            let current_project = self.inner.backend.canonical_project(&canonical_data_root)?;
            if current_project.identity != project.identity
                || request
                    .expected_project_identity
                    .is_some_and(|expected| expected != current_project.identity)
            {
                if acquired_here {
                    self.release_owner_or_retain(&project.identity, &owner)?;
                }
                return Err(project_replaced("the project changed during operator unlock"));
            }
            if previous
                .as_ref()
                .is_some_and(|(_, _, identity)| *identity != opened_identity)
            {
                return Err(project_replaced(
                    "operator unlock opened a different project database",
                ));
            }
            // Read in the same transition that replaces it, so a caller that
            // restores the mode later restores what it really was.
            let previous_mode = match self.inner.backend.effective_unlock_mode(&canonical_data_root)
            {
                Ok(mode) => Some(mode),
                Err(Error::App(_)) => None,
                Err(error) => {
                    if acquired_here {
                        self.release_owner_or_retain(&project.identity, &owner)?;
                    }
                    return Err(error);
                }
            };
            if let Err(error) = self.inner.backend.set_unlock_mode(
                &canonical_data_root,
                "brokered",
                Some(&opened_identity),
                Some(&project.identity),
            ) {
                if acquired_here {
                    self.release_owner_or_retain(&project.identity, &owner)?;
                }
                return Err(error);
            }
            let stored = Zeroizing::new(passphrase.to_vec());
            let now = Instant::now();
            let expires = request
                .duration_seconds
                .and_then(|seconds| now.checked_add(Duration::from_secs(seconds as u64)));
            let (lease_changed, stopped) = {
                let mut state = self.state();
                let current_epoch = state
                    .leases
                    .get(&project.identity)
                    .map(|lease| lease.epoch.clone());
                let lease_changed =
                    current_epoch != previous.as_ref().map(|(epoch, _, _)| epoch.clone());
                let stopped = state.closed;
                if !lease_changed && !stopped {
                    let lease = Lease {
                        data_root: canonical_data_root.clone(),
                        project: current_project.clone(),
                        database_identity: opened_identity.clone(),
                        passphrase: stored,
                        capability: request.capability,
                        owner: Arc::clone(&owner),
                        unlocked_at: now_iso(),
                        expires_at: expires,
                        duration_seconds: request.duration_seconds,
                        authentication_method: request.authentication_method.to_owned(),
                        expires_at_text: requested_expires_at.clone(),
                        epoch: token_urlsafe(24)?,
                        workspace: context.0.clone(),
                        profile: context.1.clone(),
                        revoked: false,
                        running_operations: 0,
                        agent_scope: request.agent_scope,
                        agent_control_digest: request
                            .agent_control
                            .filter(|_| request.agent_scope)
                            .map(|control| {
                                use sha2::{Digest, Sha256};
                                Sha256::digest(control).to_vec()
                            }),
                        idle_timeout_seconds: request
                            .idle_timeout_seconds
                            .filter(|_| request.agent_scope),
                        last_used: now,
                        calls: 0,
                        last_call_at: None,
                    };
                    // The replaced lease's passphrase is wiped as it drops.
                    state.leases.insert(project.identity.clone(), lease);
                    state
                        .lease_aliases
                        .insert(alias.clone(), project.identity.clone());
                    if !state.workers.contains_key(&project.identity) {
                        let worker = self.start_worker(&project.identity, &project.public_id);
                        state.workers.insert(project.identity.clone(), worker);
                    }
                }
                (lease_changed, stopped)
            };
            if lease_changed || stopped {
                if acquired_here {
                    self.release_owner_or_retain(&project.identity, &owner)?;
                }
                if stopped {
                    return Err(broker_stopped());
                }
                return Err(Error::App(
                    app("operator_project_busy", "the operator lease changed during unlock")
                        .retryable(true),
                ));
            }
            Ok(previous_mode)
        })?;
        self.log(
            Level::Info,
            "operator lease unlocked",
            vec![
                ("project", s(&project.public_id)),
                ("capability", s(request.capability.as_str())),
                ("authentication_method", s(request.authentication_method)),
            ],
        );
        let mut status = self.status(Some(&canonical_data_root))?;
        set(
            &mut status,
            "previous_mode",
            previous_mode.as_deref().map_or(Value::Null, s),
        );
        Ok(status)
    }

    /// `verify_admin(data_root, secret)`.
    pub fn verify_admin(&self, data_root: &str, secret: &[u8]) -> Result<AdminAuthorization> {
        let project = self.inner.backend.canonical_project(data_root)?;
        self.authenticate_database(data_root, secret, "operator_admin", true, |_| {
            let mut state = self.state();
            let lease = self.require_lease_locked(&mut state, &project)?;
            Ok(AdminAuthorization {
                project_identity: project.identity.clone(),
                database_identity: lease.database_identity.clone(),
                lease_epoch: lease.epoch.clone(),
                expires_at: Instant::now() + self.inner.limits.admin_auth_ttl,
                consumed: AtomicBool::new(false),
            })
        })
    }

    /// `authenticate_database(data_root, secret, scope=..., require_lease=...,
    /// continuation=...)`.
    pub fn authenticate_database<T>(
        &self,
        data_root: &str,
        secret: &[u8],
        scope: &str,
        require_lease: bool,
        continuation: impl FnOnce(&str) -> Result<T>,
    ) -> Result<T> {
        let project = self.inner.backend.canonical_project(data_root)?;
        let canonical_data_root = parent_text(&project);
        self.with_transition(&project.identity, false, || {
            self.release_pending_owners(Some(&project.identity))?;
            let (existing_epoch, backoff) = {
                let mut state = self.state();
                let existing = if require_lease {
                    Some(self.require_lease_locked(&mut state, &project)?.epoch.clone())
                } else {
                    let existing = state
                        .leases
                        .get(&project.identity)
                        .map(|lease| (lease.revoked || lease.expired(), lease.running_operations));
                    match existing {
                        Some((true, running)) => {
                            if running > 0 {
                                return Err(Error::App(
                                    app(
                                        "operator_project_busy",
                                        "the project still has a running operator operation",
                                    )
                                    .retryable(true),
                                ));
                            }
                            self.drop_lease_locked(&mut state, &project.identity);
                            None
                        }
                        Some((false, _)) => state
                            .leases
                            .get(&project.identity)
                            .map(|lease| lease.epoch.clone()),
                        None => None,
                    }
                };
                if let Some(epoch) = &existing {
                    let running = state
                        .leases
                        .get(&project.identity)
                        .filter(|lease| lease.epoch == *epoch)
                        .map_or(0, |lease| lease.running_operations);
                    if running > 0 {
                        return Err(Error::App(
                            app(
                                "operator_project_busy",
                                "the project has a running operator operation",
                            )
                            .with_hint(
                                "Wait for the running operation to finish before performing fresh database authentication.",
                            )
                            .with_details(dict([("project", s(&project.public_id))]))
                            .retryable(true),
                        ));
                    }
                }
                let backoff =
                    self.auth_backoff_locked(&mut state, &project.identity, &canonical_data_root)?;
                (existing, backoff)
            };
            self.release_pending_owners(Some(&project.identity))?;
            let mut temporary_owner: Option<SharedOwner> = match existing_epoch {
                None => Some(Arc::new(Mutex::new(
                    self.inner
                        .backend
                        .acquire_owner(&project, &self.inner.generation)?,
                ))),
                Some(_) => None,
            };
            let outcome = (|| -> Result<T> {
                backoff.check(scope).map_err(Error::App)?;
                let opened = (|| -> Result<String> {
                    let mut connection = self.inner.backend.open_database(
                        &canonical_data_root,
                        decode_secret(secret)?,
                        None,
                    )?;
                    let identity = connection.instance_id().map(|identity| {
                        identity.unwrap_or_else(|| project.identity[..32].to_owned())
                    });
                    if let Err(error) = connection.close() {
                        self.retain_pending_resource(
                            &project.identity,
                            Some(connection),
                            temporary_owner.take(),
                        );
                        return Err(error);
                    }
                    identity
                })();
                let authenticated = match opened {
                    Ok(identity) => {
                        backoff.record_success();
                        identity
                    }
                    Err(error) => {
                        if matches!(error.code(), Some("unlock_failed" | "passphrase_required"))
                        {
                            backoff.record_failure();
                        }
                        return Err(error);
                    }
                };
                let current = self.inner.backend.canonical_project(&canonical_data_root)?;
                if current.identity != project.identity {
                    return Err(project_replaced(
                        "the project changed during fresh authentication",
                    ));
                }
                {
                    let mut state = self.state();
                    let active_identity = if require_lease {
                        let epoch = self.require_lease_locked(&mut state, &current)?.epoch.clone();
                        if Some(&epoch) != existing_epoch.as_ref() {
                            return Err(Error::App(
                                app(
                                    "operator_project_busy",
                                    "the operator lease changed during authentication",
                                )
                                .retryable(true),
                            ));
                        }
                        state
                            .leases
                            .get(&project.identity)
                            .map(|lease| lease.database_identity.clone())
                    } else {
                        state
                            .leases
                            .get(&project.identity)
                            .map(|lease| lease.database_identity.clone())
                    };
                    if active_identity.is_some_and(|identity| identity != authenticated) {
                        return Err(project_replaced(
                            "fresh authentication opened a different project database",
                        ));
                    }
                    if state.closed {
                        return Err(broker_stopped());
                    }
                }
                continuation(&authenticated)
            })();
            if let Some(owner) = temporary_owner {
                let released = locked(&owner).release();
                if let Err(error) = released {
                    self.retain_pending_resource(&project.identity, None, Some(owner));
                    return Err(error);
                }
            }
            outcome
        })
    }

    /// `set_mode_authenticated(data_root, secret, mode)`.
    pub fn set_mode_authenticated(
        &self,
        data_root: &str,
        secret: &[u8],
        mode: &str,
    ) -> Result<Value> {
        let project = self.inner.backend.canonical_project(data_root)?;
        let canonical_data_root = parent_text(&project);
        let selected = self.authenticate_database(
            &canonical_data_root,
            secret,
            "operator_mode",
            false,
            |authenticated| {
                let selected = self.inner.backend.set_unlock_mode(
                    &canonical_data_root,
                    mode,
                    Some(authenticated),
                    Some(&project.identity),
                )?;
                if selected != "brokered" {
                    {
                        let mut state = self.state();
                        let identity = Self::lease_identity_for_path_locked(&state, &project);
                        self.revoke_lease_locked(
                            &mut state,
                            &identity,
                            "operator mode ended the brokered lease",
                        );
                    }
                    self.release_pending_owners(Some(&project.identity))?;
                }
                Ok(selected)
            },
        )?;
        Ok(dict([
            ("mode", s(&selected)),
            ("project", s(&project.public_id)),
        ]))
    }

    // ----- lock and status -----

    /// `lock(data_root, expected_lease_id=..., restore_manual=...)`.
    pub fn lock(
        &self,
        data_root: &str,
        expected_lease_id: Option<&str>,
        restore_manual: bool,
    ) -> Result<Value> {
        let project = self.inner.backend.canonical_project(data_root)?;
        let transition_identity = Self::lease_identity_for_path_locked(&self.state(), &project);
        let outcome = self.with_transition(&transition_identity, false, || {
            let (existed, running, project_identity) = {
                let mut state = self.state();
                let project_identity = Self::lease_identity_for_path_locked(&state, &project);
                let current = state.leases.get(&project_identity);
                let live = current.filter(|lease| !lease.revoked && !lease.expired());
                if let (Some(expected), Some(live)) = (expected_lease_id, live) {
                    if live.public_id() != expected {
                        return Ok(Err(dict([
                            ("project", s(&project.public_id)),
                            ("locked", Value::Bool(false)),
                            ("lease_existed", Value::Bool(true)),
                            ("kept", Value::Bool(true)),
                            ("generation", s(&self.inner.generation)),
                        ])));
                    }
                }
                let existed = current.is_some();
                let mut running = 0;
                if existed {
                    running = current.map_or(0, |lease| lease.running_operations);
                    self.revoke_lease_locked(
                        &mut state,
                        &project_identity,
                        "operator lease was locked",
                    );
                }
                (existed, running, project_identity)
            };
            self.release_pending_owners(Some(&project_identity))?;
            let canonical = parent_text(&project);
            let mut mode_restored = false;
            // A broker that replaced the granting one restores too: the
            // named lease is gone, and this transition still serializes the
            // write with any unlock here.
            let still_brokered = restore_manual
                && match self.inner.backend.effective_unlock_mode(&canonical) {
                    Ok(mode) => mode == "brokered",
                    Err(Error::App(_)) => false,
                    Err(error) => return Err(error),
                };
            if still_brokered {
                self.inner
                    .backend
                    .set_unlock_mode(&canonical, "manual", None, None)?;
                mode_restored = true;
            }
            Ok(Ok((existed, running, mode_restored)))
        })?;
        let (existed, running, mode_restored) = match outcome {
            Err(kept) => return Ok(kept),
            Ok(locked) => locked,
        };
        self.log(
            Level::Info,
            "operator lease locked",
            vec![
                ("project", s(&project.public_id)),
                ("running_operations_finishing", int(running as u64)),
            ],
        );
        Ok(dict([
            ("project", s(&project.public_id)),
            ("locked", Value::Bool(true)),
            ("lease_existed", Value::Bool(existed)),
            ("running_operations_finishing", int(running as u64)),
            ("generation", s(&self.inner.generation)),
            ("mode_restored", Value::Bool(mode_restored)),
        ]))
    }

    /// `status(data_root=None)`.
    pub fn status(&self, data_root: Option<&str>) -> Result<Value> {
        if let Some(data_root) = data_root {
            let project = self.inner.backend.canonical_project(data_root)?;
            let transition_identity = Self::lease_identity_for_path_locked(&self.state(), &project);
            return self.with_transition(&transition_identity, false, || {
                self.project_status(&project)
            });
        }
        let state = self.state();
        let leases = state
            .leases
            .values()
            .filter(|lease| !lease.revoked && !lease.expired())
            .map(|lease| self.lease_status(&state, lease))
            .collect();
        Ok(dict([
            ("broker", s("running")),
            ("generation", s(&self.inner.generation)),
            ("leases", Value::List(leases)),
        ]))
    }

    fn locked_status(&self, project: &CanonicalProject) -> Value {
        dict([
            ("broker", s("running")),
            ("generation", s(&self.inner.generation)),
            ("project", s(&project.public_id)),
            ("lease", s("locked")),
        ])
    }

    fn project_status(&self, project: &CanonicalProject) -> Result<Value> {
        self.release_pending_owners(Some(&project.identity))?;
        let (project_identity, epoch) = {
            let mut state = self.state();
            let project_identity = Self::lease_identity_for_path_locked(&state, project);
            let mut epoch = state
                .leases
                .get(&project_identity)
                .map(|lease| (lease.epoch.clone(), lease.expired()));
            if let Some((_, true)) = epoch {
                self.revoke_lease_locked(&mut state, &project_identity, "operator lease expired");
                epoch = None;
            }
            (project_identity, epoch.map(|(epoch, _)| epoch))
        };
        self.release_pending_owners(Some(&project_identity))?;
        let owner = {
            let state = self.state();
            let lease = epoch
                .as_ref()
                .and_then(|_| state.leases.get(&project_identity));
            let Some(lease) = lease.filter(|lease| !lease.revoked) else {
                return Ok(self.locked_status(project));
            };
            (project_identity == project.identity).then(|| Arc::clone(&lease.owner))
        };
        if let Some(owner) = owner {
            locked(&owner).add_alias(project)?;
            let mut state = self.state();
            let current = state.leases.get_mut(&project_identity);
            match current {
                Some(lease) if Some(&lease.epoch) == epoch.as_ref() && !lease.revoked => {
                    lease.project = project.clone();
                }
                _ => return Ok(self.locked_status(project)),
            }
            state.lease_aliases.insert(
                project.database.to_string_lossy().into_owned(),
                project.identity.clone(),
            );
        }
        let state = self.state();
        let Some(lease) = state
            .leases
            .get(&project_identity)
            .filter(|lease| Some(&lease.epoch) == epoch.as_ref())
        else {
            return Ok(self.locked_status(project));
        };
        let mut payload = dict([
            ("broker", s("running")),
            ("generation", s(&self.inner.generation)),
        ]);
        if let Value::Dict(pairs) = self.lease_status(&state, lease) {
            for (key, value) in pairs {
                if let kassiber_pyjson::Key::Str(name) = &key {
                    if let Some(name) = name.as_str() {
                        set(&mut payload, name, value);
                    }
                }
            }
        }
        if project_identity != project.identity {
            set(&mut payload, "project_file_changed", Value::Bool(true));
        }
        Ok(payload)
    }

    fn lease_status(&self, state: &State, lease: &Lease) -> Value {
        let remaining = lease.expires_at.map(|deadline| {
            let now = Instant::now();
            if deadline > now {
                (deadline - now).as_secs() as i64
            } else {
                0
            }
        });
        let queued = state
            .workers
            .get(&lease.project.identity)
            .map_or(0, |worker| worker.queued());
        let worker_state = if lease.running_operations > 0 {
            "running"
        } else if queued > 0 {
            "queued"
        } else {
            "idle"
        };
        let granted = [
            Capability::Read,
            Capability::Operator,
            Capability::AccountingDecisions,
        ]
        .into_iter()
        .filter(|capability| Capability::allows(lease.capability, *capability))
        .map(|capability| s(capability.as_str()))
        .collect();
        let mut payload = dict([
            ("project", s(&lease.project.public_id)),
            ("lease", s("unlocked")),
            ("lease_id", s(&lease.public_id())),
            ("capability", s(lease.capability.as_str())),
            ("granted_capabilities", Value::List(granted)),
            ("authentication_method", s(&lease.authentication_method)),
            ("unlocked_at", s(&lease.unlocked_at)),
            (
                "duration_seconds",
                lease.duration_seconds.map_or(Value::Null, int),
            ),
            ("expires_at", text_or_null(&lease.expires_at_text)),
            ("remaining_seconds", remaining.map_or(Value::Null, int)),
            ("until_lock", Value::Bool(lease.expires_at.is_none())),
            ("queued_operations", int(queued as u64)),
            ("running_operations", int(lease.running_operations as u64)),
            ("worker_state", s(worker_state)),
            (
                "default_scope",
                dict([
                    ("workspace", text_or_null(&lease.workspace)),
                    ("profile", text_or_null(&lease.profile)),
                ]),
            ),
            ("agent_scope", Value::Bool(lease.agent_scope)),
        ]);
        if lease.agent_scope {
            set(
                &mut payload,
                "agent",
                Self::agent_status_locked(state, lease),
            );
        }
        payload
    }

    // ----- submission -----

    /// `submit(data_root, argv, ...)`. Staged secrets are dropped (wiped)
    /// unless the operation took them.
    pub fn submit(
        &self,
        data_root: &str,
        argv: &[String],
        request: SubmitRequest<'_>,
    ) -> Result<Value> {
        let SubmitRequest {
            operation_id,
            secret_arguments,
            admin_authorization,
            working_directory,
            no_egress,
            peer_pid,
            agent_session_token,
        } = request;
        let parsed = self.parse_argv(argv)?;
        let command_path = parsed.command_path.clone();
        if command_path.starts_with("operator.")
            || ["daemon", "chat", "mcp.serve"].contains(&command_path.as_str())
        {
            return Err(Error::App(
                app(
                    "operator_command_not_brokerable",
                    "this command cannot run inside an operator worker",
                )
                .with_details(dict([("command", s(&command_path))])),
            ));
        }
        let project = self.inner.backend.canonical_project(data_root)?;
        let replaced = || {
            Error::App(
                app(
                    "operator_project_replaced",
                    "the database file changed while its operator lease is active",
                )
                .with_hint("Lock the prior lease before operating on the replacement database."),
            )
        };
        self.with_transition(&project.identity, false, || {
            let mut state = self.state();
            if Self::lease_identity_for_path_locked(&state, &project) != project.identity {
                return Err(replaced());
            }
            self.require_lease_locked(&mut state, &project)?;
            Ok(())
        })?;
        require_explicit_scope(&parsed)?;
        let canonical_data_root = parent_text(&project);
        if parsed.command_path == "backup.import" && parsed.install {
            return Err(Error::App(
                app(
                    "operator_command_not_brokerable",
                    "backup installation cannot run inside an operator worker",
                )
                .with_hint(
                    "Lock the broker lease, select manual mode, and run the restore explicitly for its destination.",
                )
                .with_details(dict([
                    ("lease_project", s(&project.public_id)),
                    ("command", s(&command_path)),
                ])),
            ));
        }
        if parsed.data_root.is_some() || parsed.project.is_some() {
            let requested = self.inner.backend.resolve_data_root(
                parsed.data_root.as_deref(),
                parsed.env_file.as_deref(),
                parsed.project.as_deref(),
            )?;
            let requested_project = self.inner.backend.canonical_project(&requested)?;
            if requested_project.identity != project.identity {
                return Err(Error::App(
                    app(
                        "operator_project_mismatch",
                        "the command targets a different project than the operator lease",
                    )
                    .with_details(dict([
                        ("lease_project", s(&project.public_id)),
                        ("command_project", s(&requested_project.public_id)),
                    ])),
                ));
            }
        }
        self.with_transition(&project.identity, false, || {
            self.release_pending_owners(Some(&project.identity))?;
            let owner = {
                let mut state = self.state();
                if Self::lease_identity_for_path_locked(&state, &project) != project.identity {
                    return Err(replaced());
                }
                Arc::clone(&self.require_lease_locked(&mut state, &project)?.owner)
            };
            let aliased = locked(&owner).add_alias(&project);
            aliased
        })?;
        let mut state = self.state();
        if Self::lease_identity_for_path_locked(&state, &project) != project.identity {
            return Err(replaced());
        }
        self.require_lease_locked(&mut state, &project)?;
        let agent_session_id = if state.leases[&project.identity].agent_scope {
            // Before any replay lookup: only an allowed agent process may
            // learn anything about this lease's operations.
            Some(Self::admit_agent_call_locked(
                &mut state,
                &project.identity,
                &command_path,
                peer_pid,
                agent_session_token.as_deref(),
            )?)
        } else {
            None
        };
        {
            let lease = state.leases.get_mut(&project.identity).expect("lease");
            lease.data_root = canonical_data_root.clone();
            lease.project = project.clone();
        }
        state.lease_aliases.insert(
            project.database.to_string_lossy().into_owned(),
            project.identity.clone(),
        );
        let pinned_argv = pin_project_arguments(argv, &canonical_data_root);
        let generation = &self.inner.generation;
        if let Some(operation_id) = &operation_id {
            if !operation_id.starts_with(&format!("{generation}."))
                || operation_id.chars().count() > 160
            {
                return Err(protocol_error("invalid proposed operation id"));
            }
            let conflict = || {
                Error::App(app(
                    "operator_operation_id_conflict",
                    "the operation id is already bound to another request",
                ))
            };
            if let Some(existing) = state.operations.get(operation_id) {
                if existing.project_id != project.public_id
                    || existing.argv != pinned_argv
                    || existing.working_directory != working_directory
                    || existing.no_egress != no_egress
                {
                    return Err(conflict());
                }
                return Ok(existing.public_status(false));
            }
            if let Some((bound_project, bound_fingerprint)) = state.tombstones.get(operation_id) {
                if *bound_project != project.public_id
                    || *bound_fingerprint
                        != request_fingerprint(
                            &pinned_argv,
                            working_directory.as_deref(),
                            no_egress,
                        )
                {
                    return Err(conflict());
                }
                state.tombstones.move_to_end(operation_id);
                return Ok(dict([
                    ("operation_id", s(operation_id)),
                    ("state", s("result_unknown")),
                    ("reason", s("result_not_retained")),
                    ("generation", s(generation)),
                ]));
            }
        }
        let lease = &state.leases[&project.identity];
        let admin_until = if parsed.capability == Capability::Admin {
            Some(Self::consume_admin_authorization(
                admin_authorization,
                &project,
                lease,
                &command_path,
            )?)
        } else {
            if !Capability::allows(lease.capability, parsed.capability) {
                return Err(Error::App(
                    app(
                        "operator_capability_denied",
                        "the operator lease does not grant this command",
                    )
                    .with_details(dict([
                        ("command", s(&command_path)),
                        ("required", s(parsed.capability.as_str())),
                        ("granted", s(lease.capability.as_str())),
                    ])),
                ));
            }
            None
        };
        let epoch = lease.epoch.clone();
        let database_identity = lease.database_identity.clone();
        state.sequence += 1;
        let operation_id = match operation_id {
            Some(id) => id,
            None => format!(
                "{generation}.{}.{}.{}",
                project.public_id,
                state.sequence,
                short_token()?
            ),
        };
        let mut operation = Operation::new(
            operation_id.clone(),
            project.public_id.clone(),
            project.identity.clone(),
            database_identity,
            canonical_data_root,
            pinned_argv,
            command_path.clone(),
            parsed.capability,
        );
        operation.admitted_lease_epoch = Some(epoch);
        operation.admin_authorized_until = admin_until;
        operation.working_directory = working_directory;
        operation.no_egress = no_egress;
        operation.agent_pid = agent_session_id.as_ref().and(peer_pid);
        operation.agent_session_id = agent_session_id;
        let worker = match state.workers.get(&project.identity) {
            Some(worker) => Arc::clone(worker),
            None => {
                let worker = self.start_worker(&project.identity, &project.public_id);
                state
                    .workers
                    .insert(project.identity.clone(), Arc::clone(&worker));
                worker
            }
        };
        worker.submit(&operation_id, self.inner.limits.max_queued_operations)?;
        operation.secret_arguments = secret_arguments;
        let status = operation.public_status(false);
        state.operations.insert(operation_id, operation);
        self.prune_operations_locked(&mut state);
        drop(state);
        self.log(
            Level::Info,
            "operator operation admitted",
            vec![
                ("project", s(&project.public_id)),
                ("command", s(&command_path)),
                ("capability", s(parsed.capability.as_str())),
                ("state", s("queued")),
            ],
        );
        Ok(status)
    }

    /// `_parse_argv`, through the backend's argparse.
    pub fn parse_argv(&self, argv: &[String]) -> Result<Classified> {
        self.inner.backend.classify(&classification_argv(argv))
    }

    /// `_consume_admin_authorization_locked`: valid once, unexpired, and for
    /// this project, database and lease epoch.
    fn consume_admin_authorization(
        authorization: Option<&AdminAuthorization>,
        project: &CanonicalProject,
        lease: &Lease,
        command_path: &str,
    ) -> Result<Instant> {
        let valid = authorization.filter(|authorization| {
            let already_consumed = authorization.consumed.swap(true, Ordering::SeqCst);
            !already_consumed
                && Instant::now() <= authorization.expires_at
                && authorization.project_identity == project.identity
                && authorization.database_identity == lease.database_identity
                && authorization.lease_epoch == lease.epoch
        });
        match valid {
            Some(authorization) => Ok(authorization.expires_at),
            None => Err(Error::App(
                app(
                    "operator_admin_auth_required",
                    "admin commands require fresh authentication for the active lease",
                )
                .with_details(dict([("command", s(command_path))])),
            )),
        }
    }

    /// `operation_status(operation_id, include_output=...)`.
    pub fn operation_status(&self, operation_id: &str, include_output: bool) -> Value {
        let state = self.state();
        self.operation_status_locked(&state, operation_id, include_output)
    }

    fn operation_status_locked(
        &self,
        state: &State,
        operation_id: &str,
        include_output: bool,
    ) -> Value {
        let generation = &self.inner.generation;
        if !operation_id.starts_with(&format!("{generation}.")) {
            return dict([
                ("operation_id", s(operation_id)),
                ("state", s("result_unknown")),
                ("reason", s("broker_generation_changed")),
                ("generation", s(generation)),
            ]);
        }
        match state.operations.get(operation_id) {
            None => dict([
                ("operation_id", s(operation_id)),
                ("state", s("result_unknown")),
                ("reason", s("result_not_retained")),
                ("generation", s(generation)),
            ]),
            Some(operation) => operation.public_status(include_output),
        }
    }

    /// `cancel(operation_id)`: queued work becomes terminal at once; running
    /// work has no cooperative cancellation.
    pub fn cancel(&self, operation_id: &str) -> Value {
        let mut state = self.state();
        let Some(operation) = state.operations.get(operation_id) else {
            return self.operation_status_locked(&state, operation_id, true);
        };
        if operation.state.is_terminal() {
            return operation.public_status(true);
        }
        if operation.state == OpState::Queued {
            let project_id = operation.project_id.clone();
            let command = operation.command_path.clone();
            let operation = state.operations.get_mut(operation_id).expect("operation");
            operation.cancellation_requested = true;
            let cancelled =
                self.finish_operation_locked(&mut state, operation_id, OpState::Cancelled, None);
            if let Some(worker) = Self::worker_for_public_project_locked(&state, &project_id) {
                worker.remove(operation_id);
            }
            self.log(
                Level::Info,
                "operator queued operation cancelled",
                vec![
                    ("project", s(&project_id)),
                    ("command", s(&command)),
                    ("state", s("cancelled")),
                ],
            );
            return cancelled.unwrap_or(Value::Null);
        }
        let mut payload = operation.public_status(true);
        set(&mut payload, "cancellation", s("not_cancellable"));
        set(
            &mut payload,
            "reason",
            s("running_operation_has_no_cooperative_cancel_contract"),
        );
        payload
    }

    // ----- shutdown -----

    /// `close()`: revoke every lease once, stop the workers, and keep
    /// retrying detached owner releases on later calls.
    pub fn close(&self) -> Result<()> {
        let prepare = {
            let mut state = self.state();
            while state.close_in_progress {
                state = self
                    .inner
                    .transitions
                    .wait(state)
                    .unwrap_or_else(|poison| poison.into_inner());
            }
            if state.close_complete && state.pending_owner_releases.is_empty() {
                return Ok(());
            }
            state.close_in_progress = true;
            state.close_complete = false;
            let prepare = !state.close_prepared;
            state.closed = true;
            self.inner.janitor_wake.notify_all();
            prepare
        };
        let outcome = self.close_steps(prepare);
        let mut state = self.state();
        if outcome.is_ok() {
            state.close_complete = true;
        }
        state.close_in_progress = false;
        self.inner.transitions.notify_all();
        outcome
    }

    /// The revocation `close()` runs once, then the owner releases it
    /// retries.
    fn close_steps(&self, prepare: bool) -> Result<()> {
        {
            if prepare {
                let mut state = self.state();
                while state.active_project_transitions > 0 {
                    state = self
                        .inner
                        .transitions
                        .wait(state)
                        .unwrap_or_else(|poison| poison.into_inner());
                }
                let identities: Vec<String> = state.leases.keys().cloned().collect();
                for identity in identities {
                    self.revoke_lease_locked(&mut state, &identity, "operator broker stopped");
                }
                for worker in state.workers.values() {
                    worker.stop();
                }
                state.close_prepared = true;
            }
            self.release_pending_owners(None)
        }
    }

    /// `prepare_idle_restart()`: stop new work when a handoff is safe.
    pub fn prepare_idle_restart(&self) -> Result<Value> {
        let mut state = self.state();
        let active_operation = state
            .operations
            .values()
            .any(|operation| !operation.state.is_terminal());
        if !state.leases.is_empty()
            || active_operation
            || state.active_project_transitions > 0
            || !state.pending_owner_releases.is_empty()
            || state.close_in_progress
        {
            return Err(Error::App(
                app(
                    "operator_broker_busy",
                    "the operator broker is not idle enough to restart",
                )
                .with_hint(
                    "Wait for active work and cleanup to finish, then lock all leases before retrying Touch ID.",
                )
                .retryable(true),
            ));
        }
        state.closed = true;
        self.inner.janitor_wake.notify_all();
        Ok(dict([
            ("restart", s("accepted")),
            ("generation", s(&self.inner.generation)),
        ]))
    }

    // ----- state helpers (Python's `_locked` methods) -----

    /// `_require_lease_locked`.
    fn require_lease_locked<'s>(
        &self,
        state: &'s mut State,
        project: &CanonicalProject,
    ) -> Result<&'s Lease> {
        let usable = state
            .leases
            .get(&project.identity)
            .map(|lease| !lease.revoked && !lease.expired());
        if usable != Some(true) {
            if usable.is_some() {
                let lease = state.leases.get_mut(&project.identity).expect("lease");
                lease.revoked = true;
                let running = lease.running_operations;
                Self::close_agent_sessions_locked(state, &project.identity);
                if running == 0 {
                    self.drop_lease_locked(state, &project.identity);
                }
            }
            return Err(Error::App(
                app("interaction_required", "this project has no active operator lease")
                    .with_hint(
                        "Ask the user to run `kassiber operator unlock` in their own terminal (add `--capability read` for a read-only session), then retry. Never ask for the passphrase itself.",
                    )
                    .with_details(dict([
                        ("project", s(&project.public_id)),
                        ("reason", s("operator_lease_required")),
                        ("unlock_mode", s("brokered")),
                        ("user_command", s("kassiber operator unlock")),
                    ]))
                    .retryable(true),
            ));
        }
        Ok(&state.leases[&project.identity])
    }

    /// `_auth_backoff_locked`: one cached backoff per project, LRU-bounded.
    fn auth_backoff_locked(
        &self,
        state: &mut State,
        identity: &str,
        data_root: &str,
    ) -> Result<Arc<crate::backoff::AuthAttemptBackoff>> {
        if !state.auth_backoffs.contains(identity) {
            let path = self.inner.backend.auth_backoff_path(data_root)?;
            state.auth_backoffs.insert(
                identity.to_owned(),
                Arc::new(crate::backoff::AuthAttemptBackoff::new(path)),
            );
        }
        state.auth_backoffs.move_to_end(identity);
        while state.auth_backoffs.len() > self.inner.limits.max_cached_backoffs {
            state.auth_backoffs.pop_first();
        }
        Ok(Arc::clone(
            state.auth_backoffs.get(identity).expect("backoff"),
        ))
    }

    /// `_finish_operation_locked`: record the terminal state, drop staged
    /// secrets, and prune retention. Returns the operation's status with
    /// output as it stood before pruning could evict it.
    fn finish_operation_locked(
        &self,
        state: &mut State,
        operation_id: &str,
        terminal: OpState,
        result: Option<OperationResult>,
    ) -> Option<Value> {
        let operation = state.operations.get_mut(operation_id)?;
        Self::finish_operation(operation, terminal, result);
        let status = operation.public_status(true);
        state.operations.move_to_end(operation_id);
        self.prune_operations_locked(state);
        Some(status)
    }

    fn finish_operation(
        operation: &mut Operation,
        terminal: OpState,
        result: Option<OperationResult>,
    ) {
        operation.state = terminal;
        operation.output_error = None;
        operation.retained_result_bytes = 0;
        let mut result = result;
        if let Some(outcome) = &result {
            operation.retained_result_bytes = outcome.stdout.len() + outcome.stderr.len();
            if result_exceeds_protocol_frame(outcome) {
                operation.output_error = Some(dict([
                    ("code", s("operator_result_too_large")),
                    (
                        "message",
                        s("The operation finished, but its output exceeded the broker response limit and was not retained."),
                    ),
                    (
                        "hint",
                        s("Narrow the request or use a file export; reconcile any mutation before retrying."),
                    ),
                    ("retryable", Value::Bool(false)),
                ]));
                result = Some(OperationResult::new(outcome.exit_code, "", ""));
                operation.retained_result_bytes = 0;
            }
        }
        operation.result = result;
        operation.finished_at = Some(now_iso());
        operation.secret_arguments.clear();
    }

    /// `_drop_lease_locked`: detach the lease; its owner is released later.
    fn drop_lease_locked(&self, state: &mut State, identity: &str) {
        let Some(lease) = state.leases.remove(identity) else {
            return;
        };
        if let Some(worker) = state.workers.remove(identity) {
            worker.drain();
            worker.stop();
        }
        // A worker pops an operation before it enters the project gate, so
        // retained queued operations are cancelled as well as the deque.
        let queued: Vec<String> = state
            .operations
            .keys()
            .filter(|id| {
                let operation = state.operations.get(id).expect("operation");
                operation.project_identity == identity && operation.state == OpState::Queued
            })
            .cloned()
            .collect();
        for id in queued {
            if let Some(operation) = state.operations.get_mut(&id) {
                operation.cancellation_requested = true;
            }
            self.finish_operation_locked(
                state,
                &id,
                OpState::Cancelled,
                Some(OperationResult::new(1, "", "operator lease ended\n")),
            );
        }
        state.pending_owner_releases.push_back(PendingResource {
            project_identity: identity.to_owned(),
            connection: None,
            owner: Some(Arc::clone(&lease.owner)),
        });
        state.lease_aliases.retain(|_, value| value != identity);
        drop(lease);
    }

    /// `_release_pending_owners(project_identity=None)`: close retained
    /// connections and release detached owners without the state lock;
    /// failures stay pending and the first one is returned.
    fn release_pending_owners(&self, identity: Option<&str>) -> Result<()> {
        let selected: Vec<PendingResource> = {
            let mut state = self.state();
            let all: Vec<PendingResource> = state.pending_owner_releases.drain(..).collect();
            let mut selected = Vec::new();
            for resource in all {
                if identity.is_none_or(|identity| resource.project_identity == identity) {
                    selected.push(resource);
                } else {
                    state.pending_owner_releases.push_back(resource);
                }
            }
            selected
        };
        let mut first_error = None;
        for mut resource in selected {
            if let Some(connection) = resource.connection.as_mut() {
                if let Err(error) = connection.close() {
                    first_error.get_or_insert(error);
                    self.state().pending_owner_releases.push_back(resource);
                    continue;
                }
                resource.connection = None;
            }
            let Some(owner) = &resource.owner else {
                continue;
            };
            let released = locked(owner).release();
            if let Err(error) = released {
                first_error.get_or_insert(error);
                self.state().pending_owner_releases.push_back(resource);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn retain_pending_resource(
        &self,
        identity: &str,
        connection: Option<Box<dyn Database>>,
        owner: Option<SharedOwner>,
    ) {
        self.state()
            .pending_owner_releases
            .push_back(PendingResource {
                project_identity: identity.to_owned(),
                connection,
                owner,
            });
    }

    /// `_release_owner_or_retain`.
    fn release_owner_or_retain(&self, identity: &str, owner: &SharedOwner) -> Result<()> {
        let released = locked(owner).release();
        if let Err(error) = released {
            self.retain_pending_resource(identity, None, Some(Arc::clone(owner)));
            return Err(error);
        }
        Ok(())
    }

    /// `_revoke_lease_locked`: cancel queued work, and drop the lease unless
    /// an operation still runs.
    fn revoke_lease_locked(&self, state: &mut State, identity: &str, reason: &str) {
        let Some(lease) = state.leases.get_mut(identity) else {
            return;
        };
        lease.revoked = true;
        let running = lease.running_operations;
        Self::close_agent_sessions_locked(state, identity);
        if let Some(worker) = state.workers.get(identity) {
            worker.drain();
        }
        let queued: Vec<String> = state
            .operations
            .keys()
            .filter(|id| {
                let operation = state.operations.get(id).expect("operation");
                operation.project_identity == identity && operation.state == OpState::Queued
            })
            .cloned()
            .collect();
        for id in queued {
            if let Some(operation) = state.operations.get_mut(&id) {
                operation.cancellation_requested = true;
            }
            self.finish_operation_locked(
                state,
                &id,
                OpState::Cancelled,
                Some(OperationResult::new(1, "", format!("{reason}\n"))),
            );
        }
        if running == 0 {
            self.drop_lease_locked(state, identity);
        }
    }

    /// `_lease_identity_for_path_locked`.
    fn lease_identity_for_path_locked(state: &State, project: &CanonicalProject) -> String {
        state
            .lease_aliases
            .get(project.database.to_string_lossy().as_ref())
            .cloned()
            .unwrap_or_else(|| project.identity.clone())
    }

    fn worker_for_public_project_locked(state: &State, public_id: &str) -> Option<Arc<Worker>> {
        state
            .workers
            .values()
            .find(|worker| worker.project_id == public_id)
            .cloned()
    }

    /// `_prune_operations_locked`: keep the latest terminal results by
    /// completion order within the count and byte budgets, leaving
    /// fingerprint tombstones behind.
    fn prune_operations_locked(&self, state: &mut State) {
        let limits = &self.inner.limits;
        let mut terminal: VecDeque<String> = state
            .operations
            .keys()
            .filter(|id| {
                state
                    .operations
                    .get(id)
                    .expect("operation")
                    .state
                    .is_terminal()
            })
            .cloned()
            .collect();
        let mut retained_bytes: usize = terminal
            .iter()
            .map(|id| {
                state
                    .operations
                    .get(id)
                    .expect("operation")
                    .retained_result_bytes
            })
            .sum();
        while !terminal.is_empty()
            && (terminal.len() > limits.max_retained_results
                || (retained_bytes > limits.max_retained_result_bytes && terminal.len() > 1))
        {
            let removable = terminal.pop_front().expect("terminal operation");
            if let Some(operation) = state.operations.remove(&removable) {
                retained_bytes = retained_bytes.saturating_sub(operation.retained_result_bytes);
                state.tombstones.insert(
                    removable.clone(),
                    (
                        operation.project_id.clone(),
                        request_fingerprint(
                            &operation.argv,
                            operation.working_directory.as_deref(),
                            operation.no_egress,
                        ),
                    ),
                );
                state.tombstones.move_to_end(&removable);
            }
        }
        while state.tombstones.len() > limits.max_tombstones {
            state.tombstones.pop_first();
        }
    }

    // ----- agent sessions -----

    fn agent_status_locked(state: &State, lease: &Lease) -> Value {
        let idle_timeout = lease.idle_timeout_seconds.unwrap_or(0);
        let elapsed = Instant::now()
            .saturating_duration_since(lease.last_used)
            .as_secs_f64();
        let idle_remaining = ((idle_timeout as f64 - elapsed).trunc() as i64).max(0);
        let sessions = state
            .agent_sessions
            .iter()
            .filter(|session| Self::agent_session_current(session, lease))
            .map(AgentSession::public_status)
            .collect();
        dict([
            (
                "idle_timeout_seconds",
                lease.idle_timeout_seconds.map_or(Value::Null, int),
            ),
            ("idle_remaining_seconds", int(idle_remaining)),
            ("calls", int(lease.calls)),
            ("last_call_at", text_or_null(&lease.last_call_at)),
            ("sessions", Value::List(sessions)),
        ])
    }

    fn agent_session_current(session: &AgentSession, lease: &Lease) -> bool {
        !session.closed
            && session.project_identity == lease.project.identity
            && session.lease_epoch == lease.epoch
    }

    fn close_agent_sessions_locked(state: &mut State, identity: &str) {
        state
            .agent_sessions
            .retain(|session| session.project_identity != identity);
        state
            .agent_denials
            .retain(|(project, _, _)| project != identity);
    }

    fn agent_denied_locked(state: &State, lease: &Lease, pid: Option<u32>) -> bool {
        pid.is_some_and(|pid| {
            state
                .agent_denials
                .iter()
                .any(|denial| *denial == (lease.project.identity.clone(), lease.epoch.clone(), pid))
        })
    }

    fn denied_error() -> Error {
        Error::App(app("agent_pairing_denied", "the user denied this agent"))
    }

    /// `open_agent_session(data_root, pid=..., label=...)`.
    pub fn open_agent_session(
        &self,
        data_root: &str,
        pid: Option<u32>,
        label: Option<String>,
    ) -> Result<AgentSessionTicket> {
        let Some(pid) = pid else {
            return Err(Error::App(app(
                "agent_pairing_unavailable",
                "this platform cannot identify the agent's process",
            )));
        };
        let project = self.inner.backend.canonical_project(data_root)?;
        let ticket = {
            let mut state = self.state();
            let lease = self.require_lease_locked(&mut state, &project)?;
            if !lease.agent_scope {
                return Err(Error::App(app(
                    "agent_session_not_needed",
                    "this project's operator lease is not an agent session",
                )));
            }
            let (identity, epoch) = (lease.project.identity.clone(), lease.epoch.clone());
            let lease = &state.leases[&project.identity];
            if Self::agent_denied_locked(&state, lease, Some(pid)) {
                return Err(Self::denied_error());
            }
            let denials = state
                .agent_denials
                .iter()
                .filter(|(project, denied_epoch, _)| *project == identity && *denied_epoch == epoch)
                .count();
            if denials >= MAX_REMEMBERED_AGENT_DENIALS {
                // Denials last as long as the grant; rather than forget
                // one, stop taking new requests until the user unlocks again.
                return Err(Error::App(
                    app(
                        "agent_session_limit",
                        "too many agents were denied for this unlock",
                    )
                    .with_hint("Lock the book for agents in Kassiber and unlock it again."),
                ));
            }
            let live = state
                .agent_sessions
                .iter()
                .filter(|session| !session.closed);
            let total = live.clone().count();
            let per_project = live
                .filter(|session| session.project_identity == identity)
                .count();
            if total >= MAX_AGENT_SESSIONS || per_project >= MAX_AGENT_SESSIONS_PER_PROJECT {
                return Err(Error::App(
                    app("agent_session_limit", "too many agent sessions are open")
                        .with_hint(
                            "Close idle agent sessions, or deny waiting ones in Kassiber, then retry.",
                        )
                        .with_details(dict([(
                            "limit",
                            int(MAX_AGENT_SESSIONS_PER_PROJECT as u64),
                        )]))
                        .retryable(true),
                ));
            }
            let session = AgentSession {
                id: token_urlsafe(12)?,
                project_identity: identity,
                lease_epoch: epoch,
                pid,
                label,
                opened_at: now_iso(),
                state: "pending",
                calls: 0,
                last_call_at: None,
                closed: false,
                token: token_urlsafe(32)?,
            };
            let ticket = AgentSessionTicket {
                id: session.id.clone(),
                state: session.state.to_owned(),
                token: session.token.clone(),
            };
            state.agent_sessions.push(session);
            ticket
        };
        self.log(
            Level::Info,
            "agent session opened",
            vec![("project", s(&project.public_id))],
        );
        Ok(ticket)
    }

    /// `agent_session_alive(session_id)`.
    pub fn agent_session_alive(&self, session_id: &str) -> bool {
        let state = self.state();
        let Some(session) = state
            .agent_sessions
            .iter()
            .find(|session| session.id == session_id && !session.closed)
        else {
            return false;
        };
        state
            .leases
            .get(&session.project_identity)
            .is_some_and(|lease| {
                !lease.revoked && !lease.expired() && Self::agent_session_current(session, lease)
            })
    }

    /// `close_agent_session(session_id)`.
    pub fn close_agent_session(&self, session_id: &str) {
        self.state()
            .agent_sessions
            .retain(|session| session.id != session_id);
    }

    /// `decide_agent_session(data_root, session_id, allow=..., control=...)`.
    pub fn decide_agent_session(
        &self,
        data_root: &str,
        session_id: &str,
        allow: bool,
        control: &[u8],
    ) -> Result<Value> {
        let project = self.inner.backend.canonical_project(data_root)?;
        let decided = {
            let mut state = self.state();
            let lease = self.require_lease_locked(&mut state, &project)?;
            let digest = {
                use sha2::{Digest, Sha256};
                Sha256::digest(control).to_vec()
            };
            let authorized = lease.agent_scope
                && lease
                    .agent_control_digest
                    .as_ref()
                    .is_some_and(|expected| constant_time_eq(&digest, expected));
            if !authorized {
                return Err(Error::App(app(
                    "agent_pairing_unauthorized",
                    "only the Kassiber app that unlocked this book can allow agents",
                )));
            }
            let identity = lease.project.identity.clone();
            let epoch = lease.epoch.clone();
            let current = state.agent_sessions.iter().position(|session| {
                session.id == session_id
                    && !session.closed
                    && session.project_identity == identity
                    && session.lease_epoch == epoch
            });
            let Some(index) = current else {
                return Err(Error::App(app(
                    "agent_session_not_found",
                    "that agent session has ended",
                )));
            };
            if allow {
                state.agent_sessions[index].state = "allowed";
                if let Some(lease) = state.leases.get_mut(&project.identity) {
                    lease.last_used = Instant::now();
                }
                state.agent_sessions[index].public_status()
            } else {
                // Denying refuses the process, not just this request: every
                // registration it holds under this grant closes, freeing its
                // slots, and any earlier approval is withdrawn.
                state.agent_sessions[index].state = "denied";
                let pid = state.agent_sessions[index].pid;
                let decided = state.agent_sessions[index].public_status();
                state.agent_sessions.retain(|other| {
                    !(other.pid == pid
                        && !other.closed
                        && other.project_identity == identity
                        && other.lease_epoch == epoch)
                });
                self.withdraw_agent_operations_locked(&mut state, &identity, &epoch, pid);
                state.agent_denials.push((identity, epoch, pid));
                decided
            }
        };
        self.log(
            Level::Info,
            "agent session decided",
            vec![
                ("project", s(&project.public_id)),
                (
                    "state",
                    kassiber_envelope::get(&decided, "state")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
            ],
        );
        Ok(decided)
    }

    /// `_admit_agent_call_locked`: the call's session id.
    fn admit_agent_call_locked(
        state: &mut State,
        identity: &str,
        command_path: &str,
        peer_pid: Option<u32>,
        token: Option<&str>,
    ) -> Result<String> {
        if command_path != "mcp.call" {
            return Err(Error::App(
                app(
                    "agent_session_scope",
                    "an agent session admits only Kassiber MCP tool calls",
                )
                .with_details(dict([("command", s(command_path))])),
            ));
        }
        let lease = state.leases.get(identity).expect("lease");
        // A denial outranks any registration the process still holds.
        if Self::agent_denied_locked(state, lease, peer_pid) {
            return Err(Self::denied_error());
        }
        // Both the process id the OS reports and the token only the
        // registered process received.
        let matching: Vec<usize> = state
            .agent_sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| {
                peer_pid.is_some_and(|pid| session.pid == pid)
                    && token.is_some_and(|token| {
                        constant_time_eq(session.token.as_bytes(), token.as_bytes())
                    })
                    && Self::agent_session_current(session, lease)
            })
            .map(|(index, _)| index)
            .collect();
        if matching.is_empty() {
            return Err(Error::App(
                app(
                    "interaction_required",
                    "this process has no agent session for the book",
                )
                .with_hint(
                    "Open the book through `kassiber mcp serve`, which asks the user to allow it.",
                )
                .with_details(dict([("reason", s("agent_session_required"))]))
                .retryable(true),
            ));
        }
        let Some(&allowed) = matching
            .iter()
            .find(|index| state.agent_sessions[**index].state == "allowed")
        else {
            return Err(Error::App(
                app(
                    "interaction_required",
                    "waiting for the user to allow this agent in Kassiber",
                )
                .with_hint(
                    "Ask the user to allow this agent in Kassiber under Settings, AI, External agents, then retry. Never ask for the passphrase.",
                )
                .with_details(dict([("reason", s("agent_pairing_required"))]))
                .retryable(true),
            ));
        };
        let called_at = now_iso();
        let session = &mut state.agent_sessions[allowed];
        session.calls += 1;
        session.last_call_at = Some(called_at.clone());
        let id = session.id.clone();
        let lease = state.leases.get_mut(identity).expect("lease");
        lease.calls += 1;
        lease.last_call_at = Some(called_at);
        lease.last_used = Instant::now();
        Ok(id)
    }

    /// `_withdraw_agent_operations_locked`: a denied agent's queued reads
    /// never start; finished ones stay unread.
    fn withdraw_agent_operations_locked(
        &self,
        state: &mut State,
        identity: &str,
        epoch: &str,
        pid: u32,
    ) {
        let affected: Vec<String> = state
            .operations
            .keys()
            .filter(|id| {
                let operation = state.operations.get(id).expect("operation");
                operation.agent_pid == Some(pid)
                    && operation.project_identity == identity
                    && operation.admitted_lease_epoch.as_deref() == Some(epoch)
            })
            .cloned()
            .collect();
        for id in affected {
            let (queued, project_id) = {
                let operation = state.operations.get(&id).expect("operation");
                (
                    operation.state == OpState::Queued,
                    operation.project_id.clone(),
                )
            };
            if queued {
                if let Some(operation) = state.operations.get_mut(&id) {
                    operation.cancellation_requested = true;
                }
                self.finish_operation_locked(
                    state,
                    &id,
                    OpState::Cancelled,
                    Some(OperationResult::new(1, "", "the user denied this agent\n")),
                );
                if let Some(worker) = Self::worker_for_public_project_locked(state, &project_id) {
                    worker.remove(&id);
                }
            }
            if let Some(operation) = state.operations.get_mut(&id) {
                operation.output_withheld = true;
            }
        }
    }

    // ----- the janitor -----

    /// `_expire_leases`: every second until close, revoke expired leases and
    /// retry detached owner releases.
    fn expire_leases(&self) {
        loop {
            let (expired, pending) = {
                let state = self.state();
                if state.closed {
                    return;
                }
                let (state, _) = self
                    .inner
                    .janitor_wake
                    .wait_timeout(state, self.inner.limits.janitor_interval)
                    .unwrap_or_else(|poison| poison.into_inner());
                if state.closed {
                    return;
                }
                let expired: Vec<String> = state
                    .leases
                    .iter()
                    .filter(|(_, lease)| lease.expired())
                    .map(|(identity, _)| identity.clone())
                    .collect();
                let pending: Vec<String> = state
                    .pending_owner_releases
                    .iter()
                    .map(|resource| resource.project_identity.clone())
                    .collect();
                (expired, pending)
            };
            let mut seen = Vec::new();
            for identity in expired.into_iter().chain(pending) {
                if seen.contains(&identity) {
                    continue;
                }
                seen.push(identity.clone());
                let mut public_id = identity.chars().take(16).collect::<String>();
                let outcome = self.with_transition(&identity, true, || {
                    {
                        let mut state = self.state();
                        if let Some(lease) = state.leases.get_mut(&identity) {
                            public_id = lease.project.public_id.clone();
                            if lease.expired() {
                                lease.revoked = true;
                                let running = lease.running_operations;
                                self.log(
                                    Level::Info,
                                    "operator lease expired",
                                    vec![
                                        ("project", s(&public_id)),
                                        ("running_operations_finishing", int(running as u64)),
                                    ],
                                );
                                self.revoke_lease_locked(
                                    &mut state,
                                    &identity,
                                    "operator lease expired",
                                );
                            }
                        }
                    }
                    if self.release_pending_owners(Some(&identity)).is_err() {
                        self.log(
                            Level::Error,
                            "operator lease expiry cleanup failed",
                            vec![("project", s(&public_id))],
                        );
                    }
                    Ok(())
                });
                if outcome.is_err() {
                    // One broken handle must not kill the only janitor.
                    self.log(
                        Level::Error,
                        "operator lease expiry cleanup failed",
                        vec![("project", s(&public_id))],
                    );
                }
            }
        }
    }

    // ----- the project worker -----

    fn start_worker(&self, identity: &str, public_id: &str) -> Arc<Worker> {
        let worker = Arc::new(Worker {
            project_identity: identity.to_owned(),
            project_id: public_id.to_owned(),
            queue: Mutex::new((VecDeque::new(), false)),
            ready: Condvar::new(),
            thread: Mutex::new(None),
        });
        let service = self.clone();
        let running = Arc::clone(&worker);
        let handle = std::thread::Builder::new()
            .name(format!("operator-project-{public_id}"))
            .spawn(move || {
                while let Some(operation_id) = running.next() {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        service.run_one(&running, &operation_id)
                    }));
                    let failure = match outcome {
                        Ok(Ok(())) => continue,
                        Ok(Err(error)) => error.to_string(),
                        Err(_) => "the worker panicked".to_owned(),
                    };
                    service.recover_unhandled(&running, &operation_id, &failure);
                }
            })
            .expect("spawn a project worker");
        *locked(&worker.thread) = Some(handle);
        worker
    }

    /// `ProjectWorker._recover_unhandled_exception`.
    fn recover_unhandled(&self, worker: &Worker, operation_id: &str, failure: &str) {
        let result = OperationResult::new(
            1,
            "",
            sanitize_traceback_text(&format!("operator worker failed: {failure}\n")),
        );
        let identity = worker.project_identity.clone();
        // A failed recovery leaves the worker running; Python's would end
        // its thread and strand the queue.
        let _ = self.with_transition(&identity, true, || {
            let mut state = self.state();
            let running = state
                .operations
                .get(operation_id)
                .is_some_and(|operation| operation.state == OpState::Running);
            if running {
                if let Some(lease) = state.leases.get_mut(&identity) {
                    lease.running_operations = lease.running_operations.saturating_sub(1);
                }
            }
            if state
                .operations
                .get(operation_id)
                .is_some_and(|operation| !operation.state.is_terminal())
            {
                self.finish_operation_locked(
                    &mut state,
                    operation_id,
                    OpState::ResultUnknown,
                    Some(result.clone()),
                );
            }
            let (command, terminal) = state
                .operations
                .get(operation_id)
                .map(|operation| (operation.command_path.clone(), operation.state.as_str()))
                .unwrap_or_default();
            self.log(
                Level::Error,
                "operator worker recovered from an internal failure",
                vec![
                    ("project", s(&worker.project_id)),
                    ("command", s(&command)),
                    ("state", s(terminal)),
                ],
            );
            let should_drop = state.leases.get(&identity).is_some_and(|lease| {
                (lease.revoked || lease.expired()) && lease.running_operations == 0
            });
            if should_drop {
                self.drop_lease_locked(&mut state, &identity);
            }
            Ok(())
        });
    }

    /// `ProjectWorker._run_one`: re-check the lease, capability, admin
    /// lifetime and project identity, inherit the owner, run the child,
    /// and record its result.
    fn run_one(&self, worker: &Worker, operation_id: &str) -> Result<()> {
        let identity = worker.project_identity.clone();
        let cancelled = |message: &str| Some(OperationResult::new(1, "", message));
        let dispatched = self.with_transition(&identity, true, || {
            let (owner, epoch, data_root, project_identity) = {
                let mut state = self.state();
                let Some(operation) = state.operations.get(operation_id) else {
                    return Ok(None);
                };
                if operation.cancellation_requested {
                    return Ok(None);
                }
                let (admitted, capability, admin_until) = (
                    operation.admitted_lease_epoch.clone(),
                    operation.capability,
                    operation.admin_authorized_until,
                );
                let usable = state
                    .leases
                    .get(&identity)
                    .map(|lease| !lease.revoked && !lease.expired());
                if usable != Some(true) {
                    self.finish_operation_locked(
                        &mut state,
                        operation_id,
                        OpState::Cancelled,
                        cancelled("operator lease ended before the operation started\n"),
                    );
                    if let Some(lease) = state.leases.get_mut(&identity) {
                        lease.revoked = true;
                        if lease.running_operations == 0 {
                            self.drop_lease_locked(&mut state, &identity);
                        }
                    }
                    return Ok(None);
                }
                let lease = &state.leases[&identity];
                if admitted.as_deref() != Some(lease.epoch.as_str()) {
                    self.finish_operation_locked(
                        &mut state,
                        operation_id,
                        OpState::Cancelled,
                        cancelled("operator lease changed before the operation started\n"),
                    );
                    return Ok(None);
                }
                if capability != Capability::Admin
                    && !Capability::allows(lease.capability, capability)
                {
                    self.finish_operation_locked(
                        &mut state,
                        operation_id,
                        OpState::Cancelled,
                        cancelled("operator lease capability changed before dispatch\n"),
                    );
                    return Ok(None);
                }
                if capability == Capability::Admin
                    && admin_until.is_none_or(|until| Instant::now() > until)
                {
                    self.finish_operation_locked(
                        &mut state,
                        operation_id,
                        OpState::Cancelled,
                        cancelled("fresh admin authentication expired before dispatch\n"),
                    );
                    return Ok(None);
                }
                let operation = state.operations.get(operation_id).expect("operation");
                (
                    Arc::clone(&lease.owner),
                    lease.epoch.clone(),
                    operation.data_root.clone(),
                    operation.project_identity.clone(),
                )
            };
            let current = self.inner.backend.canonical_project(&data_root).ok();
            if current.is_none_or(|current| current.identity != project_identity) {
                let mut state = self.state();
                self.finish_operation_locked(
                    &mut state,
                    operation_id,
                    OpState::Cancelled,
                    cancelled("operator project changed before the operation started\n"),
                );
                return Ok(None);
            }
            let inherited = match locked(&owner).duplicate_for_child() {
                Ok(inherited) => inherited,
                Err(error) => {
                    let mut state = self.state();
                    self.finish_operation_locked(
                        &mut state,
                        operation_id,
                        OpState::Failed,
                        Some(OperationResult::new(
                            1,
                            "",
                            sanitize_traceback_text(&format!(
                                "operator ownership inheritance failed: {error}\n"
                            )),
                        )),
                    );
                    return Ok(None);
                }
            };
            let dispatch = {
                let mut state = self.state();
                let (cancellation, capability, admin_until) = {
                    let operation = state.operations.get(operation_id);
                    (
                        operation.is_none_or(|operation| operation.cancellation_requested),
                        operation.map_or(Capability::Read, |operation| operation.capability),
                        operation.and_then(|operation| operation.admin_authorized_until),
                    )
                };
                let admin_expired = capability == Capability::Admin
                    && admin_until.is_none_or(|until| Instant::now() > until);
                let lease_current = state.leases.get(&identity).is_some_and(|lease| {
                    lease.epoch == epoch && !lease.revoked && !lease.expired()
                });
                if !lease_current || cancellation || admin_expired {
                    self.finish_operation_locked(
                        &mut state,
                        operation_id,
                        OpState::Cancelled,
                        cancelled(if admin_expired {
                            "fresh admin authentication expired before dispatch\n"
                        } else {
                            "operator lease ended before the operation started\n"
                        }),
                    );
                    None
                } else {
                    let lease = state.leases.get_mut(&identity).expect("lease");
                    lease.running_operations += 1;
                    let passphrase = lease.passphrase.clone();
                    let operation = state.operations.get_mut(operation_id).expect("operation");
                    operation.state = OpState::Running;
                    operation.started_at = Some(now_iso());
                    let dispatch = Dispatch {
                        id: operation.id.clone(),
                        project_identity: operation.project_identity.clone(),
                        database_identity: operation.database_identity.clone(),
                        data_root: operation.data_root.clone(),
                        argv: operation.argv.clone(),
                        command_path: operation.command_path.clone(),
                        capability: operation.capability,
                        secret_arguments: std::mem::take(&mut operation.secret_arguments),
                        working_directory: operation.working_directory.clone(),
                        no_egress: operation.no_egress,
                        owner_tokens: inherited.tokens(),
                    };
                    self.log(
                        Level::Info,
                        "operator operation dispatched",
                        vec![
                            ("project", s(&worker.project_id)),
                            ("command", s(&dispatch.command_path)),
                            ("state", s("running")),
                        ],
                    );
                    Some((dispatch, passphrase))
                }
            };
            match dispatch {
                None => {
                    inherited.close()?;
                    Ok(None)
                }
                Some((dispatch, passphrase)) => Ok(Some((dispatch, passphrase, inherited))),
            }
        })?;
        let Some((mut dispatch, passphrase, inherited)) = dispatched else {
            return Ok(());
        };
        let runner = Arc::clone(&self.inner.runner);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runner.run(&dispatch, &passphrase)
        }));
        drop(passphrase);
        let (result, runner_crashed) = match outcome {
            Ok(Ok(result)) => (result, false),
            Ok(Err(error)) => (
                OperationResult::new(
                    1,
                    "",
                    sanitize_traceback_text(&format!("operator child failed: {error}\n")),
                ),
                true,
            ),
            Err(_) => (
                OperationResult::new(1, "", "operator child failed: the runner panicked\n"),
                true,
            ),
        };
        dispatch.owner_tokens.clear();
        dispatch.secret_arguments.clear();
        let cleanup_error = inherited.close().err();
        let result = OperationResult::new(
            result.exit_code,
            result.stdout,
            redact_secret_text(&result.stderr),
        );
        let command_path = dispatch.command_path.clone();
        self.with_transition(&identity, true, || {
            let (terminal, refresh) = {
                let mut state = self.state();
                let current_epoch = match state.leases.get_mut(&identity) {
                    Some(lease) => {
                        lease.running_operations = lease.running_operations.saturating_sub(1);
                        if cleanup_error.is_some() {
                            lease.revoked = true;
                        }
                        Some(lease.epoch.clone())
                    }
                    None => None,
                };
                if cleanup_error.is_some() && current_epoch.is_some() {
                    self.log(
                        Level::Error,
                        "operator ownership cleanup failed; lease revoked",
                        vec![
                            ("project", s(&worker.project_id)),
                            ("command", s(&command_path)),
                        ],
                    );
                }
                // Only a read's own failure is proven; a crash, a signal or
                // a failed mutation may have committed.
                let terminal = if runner_crashed
                    || result.exit_code < 0
                    || (result.exit_code != 0 && dispatch.capability != Capability::Read)
                {
                    OpState::ResultUnknown
                } else if result.exit_code == 0 {
                    OpState::Completed
                } else {
                    OpState::Failed
                };
                self.finish_operation_locked(
                    &mut state,
                    operation_id,
                    terminal,
                    Some(result.clone()),
                );
                if let Some(reason) = lease_ending_reason(&command_path) {
                    self.revoke_lease_locked(&mut state, &identity, reason);
                }
                let refresh = (command_path == "context.set" && terminal == OpState::Completed)
                    .then(|| {
                        state
                            .leases
                            .get(&identity)
                            .filter(|lease| {
                                Some(&lease.epoch) == current_epoch.as_ref() && !lease.revoked
                            })
                            .map(|lease| {
                                (
                                    lease.epoch.clone(),
                                    lease.data_root.clone(),
                                    lease.passphrase.clone(),
                                    lease.database_identity.clone(),
                                    lease.project.public_id.clone(),
                                )
                            })
                    })
                    .flatten();
                (terminal, refresh)
            };
            if let Some(refresh) = refresh {
                self.refresh_scope(&identity, refresh);
            }
            let mut state = self.state();
            self.log(
                Level::Info,
                "operator operation finished",
                vec![
                    ("project", s(&worker.project_id)),
                    ("command", s(&command_path)),
                    ("state", s(terminal.as_str())),
                ],
            );
            let should_drop = state.leases.get(&identity).is_some_and(|lease| {
                (lease.revoked || lease.expired()) && lease.running_operations == 0
            });
            if should_drop {
                self.drop_lease_locked(&mut state, &identity);
            }
            drop(state);
            self.release_pending_owners(Some(&identity))
        })
    }

    /// `_refresh_scope`: re-read the default context under the project gate,
    /// revoking the lease if the book cannot be read.
    fn refresh_scope(
        &self,
        identity: &str,
        (epoch, data_root, passphrase, database_identity, public_id): (
            String,
            String,
            Zeroizing<Vec<u8>>,
            String,
            String,
        ),
    ) {
        let context = (|| -> Result<(Option<String>, Option<String>)> {
            let mut connection = self.inner.backend.open_database(
                &data_root,
                decode_secret(&passphrase)?,
                Some(&database_identity),
            )?;
            let context = connection.context();
            // A failed close replaces the read's own error, as in Python.
            connection.close()?;
            context
        })();
        let mut state = self.state();
        match context {
            Err(error) => {
                self.log(
                    Level::Warning,
                    "operator scope refresh failed",
                    vec![
                        ("project", s(&public_id)),
                        ("error", s(&sanitize_traceback_text(&error.to_string()))),
                    ],
                );
                if state
                    .leases
                    .get(identity)
                    .is_some_and(|lease| lease.epoch == epoch)
                {
                    self.revoke_lease_locked(
                        &mut state,
                        identity,
                        "operator scope could not be refreshed",
                    );
                }
            }
            Ok((workspace, profile)) => {
                if let Some(lease) = state
                    .leases
                    .get_mut(identity)
                    .filter(|lease| lease.epoch == epoch && !lease.revoked)
                {
                    lease.workspace = workspace;
                    lease.profile = profile;
                }
            }
        }
    }
}

/// `str(project.database.parent)`.
fn parent_text(project: &CanonicalProject) -> String {
    project
        .database
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_default()
}

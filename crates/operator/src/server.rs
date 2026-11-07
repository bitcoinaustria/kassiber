//! The per-OS-user broker server (`kassiber/operator/server.py`).
//!
//! One request per connection, at most 64 connections served at once, each
//! on its own thread; secrets arrive only in challenge-bound secret frames.
//! On Linux the server refuses to start, and later stops, when it cannot
//! prove the login it was started in is still alive. Error responses pass
//! the public-safe detail filter: no paths, no sensitive keys, sanitized
//! text.

use crate::build::{current_build_identity, BuildInputs};
use crate::error::{Error, Result};
use crate::protocol::{new_challenge, token_hex, FrameChannel, PROTOCOL_VERSION};
use crate::service::{Backend, Capability, OperatorService, SubmitRequest, UnlockRequest};
use crate::session::{LoginRuntime, TrustRuntime};
use crate::telemetry::{LogRing, Telemetry};
use crate::values::{py_eq, truthy};
use kassiber_envelope::redaction::{is_sensitive_key, sanitize_traceback_text};
use kassiber_envelope::{dict, get, set, AppError};
use kassiber_pyjson::{BigInt, Key, PyStr, Value};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zeroize::Zeroizing;

pub const MAX_CLIENT_THREADS: usize = 64;
/// How often the login-lifetime monitor looks (`_monitor_login_session`).
pub const LOGIN_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// How long the accept loop waits before re-checking for a stop.
const ACCEPT_POLL: Duration = Duration::from_millis(100);

fn s(text: &str) -> Value {
    kassiber_envelope::s(text)
}

/// Touch ID's broker side (`native_auth.py`). Production brokers use the
/// binary's bridge over [`crate::native_auth::NativeAuthHelper`]; builds
/// without one answer as Linux and Windows do.
pub trait NativeAuth: Send + Sync {
    /// `native_auth_helper_identity()`.
    fn helper_identity(&self) -> Result<String>;
    /// `validate_native_auth_helper_identity(expected)`.
    fn validate(&self, expected: &str) -> Result<()>;
    /// `require_project_policy_binding(data_root)`: the bound project and
    /// database identities.
    fn binding(&self, data_root: &str) -> Result<(String, String)>;
    /// `broker_touch_id_passphrase(data_root, expected_helper_identity=...)`.
    fn passphrase(&self, data_root: &str, expected: &str) -> Result<Zeroizing<Vec<u8>>>;
    /// `touch_id_store(data_root, passphrase, expected_helper_identity=...)`.
    fn store(&self, data_root: &str, passphrase: &[u8], expected: &str) -> Result<()>;
    /// `touch_id_delete(data_root, expected_helper_identity=...)`.
    fn delete(&self, data_root: &str, expected: &str) -> Result<()>;
}

/// `_configured_helper_path()` outside a signed macOS app.
pub struct NativeAuthUnavailable;

fn native_auth_unavailable() -> Error {
    Error::App(crate::native_auth::unavailable_error())
}

impl NativeAuth for NativeAuthUnavailable {
    fn helper_identity(&self) -> Result<String> {
        Err(native_auth_unavailable())
    }
    fn validate(&self, _: &str) -> Result<()> {
        Err(native_auth_unavailable())
    }
    fn binding(&self, _: &str) -> Result<(String, String)> {
        Err(native_auth_unavailable())
    }
    fn passphrase(&self, _: &str, _: &str) -> Result<Zeroizing<Vec<u8>>> {
        Err(native_auth_unavailable())
    }
    fn store(&self, _: &str, _: &[u8], _: &str) -> Result<()> {
        Err(native_auth_unavailable())
    }
    fn delete(&self, _: &str, _: &str) -> Result<()> {
        Err(native_auth_unavailable())
    }
}

/// What decides the Linux login-lifetime guard: the runtime directory and
/// logind, each behind a seam (`_login_session_runtime_path_is_trusted`,
/// `_linux_logind_user_alive`) as the Python fixture patches them.
#[derive(Clone)]
pub struct SessionGuard {
    pub runtime_dir: Option<std::ffi::OsString>,
    pub trust: Arc<TrustRuntime>,
    pub logind: Arc<dyn Fn() -> Option<bool> + Send + Sync>,
}

impl SessionGuard {
    /// The production guard: `/run/user/<uid>` on tmpfs, and
    /// `/run/systemd/users/<uid>`.
    pub fn system() -> Self {
        #[cfg(unix)]
        {
            let uid = crate::session::current_uid();
            SessionGuard {
                runtime_dir: std::env::var_os("XDG_RUNTIME_DIR"),
                trust: Arc::new(move |root, info| {
                    crate::session::runtime_path_is_trusted(uid, root, info)
                }),
                logind: Arc::new(move || {
                    crate::session::logind_user_alive(std::path::Path::new("/run/systemd"), uid)
                }),
            }
        }
        #[cfg(not(unix))]
        {
            SessionGuard {
                runtime_dir: None,
                trust: Arc::new(|_, _| false),
                logind: Arc::new(|| None),
            }
        }
    }

    /// `tests/operator_server_fixture.py`: logind unknown, and only the
    /// gated private runtime directory trusted. Builds that are not frozen
    /// only.
    pub fn isolated_test_runtime() -> Option<Self> {
        if crate::build::FROZEN {
            return None;
        }
        let trust = |root: &std::path::Path, info: &std::fs::Metadata| {
            let configured = std::env::var_os(crate::runtime::RUNTIME_OVERRIDE_ENV);
            let gate = std::env::var_os(crate::runtime::TEST_RUNTIME_OVERRIDE_ENV);
            let (Some(configured), Some("1")) =
                (configured, gate.as_deref().and_then(|g| g.to_str()))
            else {
                return false;
            };
            if configured.is_empty() {
                return false;
            }
            let Ok(expected) = std::fs::canonicalize(configured) else {
                return false;
            };
            #[cfg(unix)]
            let owned = {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                info.uid() == crate::session::current_uid()
                    && info.permissions().mode() & 0o7777 == 0o700
            };
            #[cfg(not(unix))]
            let owned = true;
            root == expected && info.is_dir() && owned
        };
        Some(SessionGuard {
            runtime_dir: std::env::var_os("XDG_RUNTIME_DIR"),
            trust: Arc::new(trust),
            logind: Arc::new(|| None),
        })
    }
}

#[cfg(unix)]
type Listener = crate::runtime::UnixBrokerListener;
#[cfg(windows)]
type Listener = crate::runtime::WindowsBrokerListener;

struct Inner {
    generation: String,
    service: OperatorService,
    native: Arc<dyn NativeAuth>,
    build: Value,
    listener: Mutex<Option<Arc<Listener>>>,
    stopped: AtomicBool,
    close_lock: Mutex<()>,
    listener_closed: AtomicBool,
    clients: AtomicUsize,
    guard: SessionGuard,
    session_runtime: Option<LoginRuntime>,
    logind_observed: bool,
    /// `peers.process_label`; tests name the program instead.
    labels: fn(Option<u32>) -> Option<String>,
}

/// `BrokerServer`.
#[derive(Clone)]
pub struct BrokerServer {
    inner: Arc<Inner>,
}

/// The parts of a broker the tests replace.
pub struct ServerParts {
    pub generation: String,
    pub service: OperatorService,
    pub native: Arc<dyn NativeAuth>,
    pub build: Value,
    pub guard: SessionGuard,
}

/// `operator_session_lifetime_unavailable`.
fn session_lifetime_unavailable() -> Error {
    Error::App(
        AppError::new(
            "operator_session_lifetime_unavailable",
            "a logout-bound operator broker is unavailable in this Linux session",
        )
        .with_hint(
            "Use manual mode, or start the broker from a login session with logind or the trusted /run/user/<uid> runtime directory.",
        ),
    )
}

impl BrokerServer {
    /// `BrokerServer()`: check the login lifetime, then listen.
    pub fn start(parts: ServerParts, runtime: &crate::runtime::Runtime) -> Result<Self> {
        let session_runtime = crate::session::login_session_runtime_root(
            parts.guard.runtime_dir.as_deref(),
            &*parts.guard.trust,
        );
        let logind_alive = (parts.guard.logind)();
        if cfg!(target_os = "linux")
            && (logind_alive == Some(false)
                || (logind_alive.is_none() && session_runtime.is_none()))
        {
            return Err(session_lifetime_unavailable());
        }
        let listener = crate::runtime::listen(runtime)?;
        let server = Self::with_listener(
            parts,
            Some(listener),
            session_runtime,
            logind_alive.is_some(),
        );
        if server.inner.session_runtime.is_some() || server.inner.logind_observed {
            let monitor = server.clone();
            std::thread::Builder::new()
                .name("operator-login-session".into())
                .spawn(move || monitor.monitor_login_session())
                .map_err(Error::Io)?;
        }
        Ok(server)
    }

    fn with_listener(
        parts: ServerParts,
        listener: Option<Listener>,
        session_runtime: Option<LoginRuntime>,
        logind_observed: bool,
    ) -> Self {
        BrokerServer {
            inner: Arc::new(Inner {
                generation: parts.generation,
                service: parts.service,
                native: parts.native,
                build: parts.build,
                listener: Mutex::new(listener.map(Arc::new)),
                stopped: AtomicBool::new(false),
                close_lock: Mutex::new(()),
                listener_closed: AtomicBool::new(false),
                clients: AtomicUsize::new(0),
                guard: parts.guard,
                session_runtime,
                logind_observed,
                labels: crate::peers::process_label,
            }),
        }
    }

    /// A detached broker whose agent sessions take their labels from
    /// `labels`, as the Python tests patch `peers.process_label`.
    #[cfg(test)]
    pub(crate) fn detached_with_labels(
        parts: ServerParts,
        labels: fn(Option<u32>) -> Option<String>,
    ) -> Self {
        let server = Self::detached(parts);
        let mut inner = Arc::try_unwrap(server.inner)
            .unwrap_or_else(|_| unreachable!("a new broker is not shared"));
        inner.labels = labels;
        BrokerServer {
            inner: Arc::new(inner),
        }
    }

    /// A server without a listener, for handler tests.
    pub fn detached(parts: ServerParts) -> Self {
        Self::with_listener(parts, None, None, false)
    }

    pub fn service(&self) -> &OperatorService {
        &self.inner.service
    }

    pub fn is_stopped(&self) -> bool {
        self.inner.stopped.load(Ordering::SeqCst)
    }

    fn listener(&self) -> Option<Arc<Listener>> {
        self.inner
            .listener
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// `serve_forever()`: accept until stopped, at most 64 clients at once.
    pub fn serve_forever(&self, stop_signal: Option<kassiber_ipc::signal::StopSignal>) {
        while !self.is_stopped() {
            if stop_signal.is_some_and(|signal| signal.is_set()) {
                self.request_stop();
                break;
            }
            let Some(listener) = self.listener() else {
                break;
            };
            #[cfg(unix)]
            {
                let ready = match listener.as_fd() {
                    Some(fd) => kassiber_ipc::poll::wait_readable(fd, Some(ACCEPT_POLL)),
                    None => break,
                };
                match ready {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(_) if self.is_stopped() => break,
                    Err(_) => continue,
                }
            }
            let channel = match listener.accept() {
                Ok(channel) => channel,
                Err(Error::App(_)) => continue,
                Err(_) if self.is_stopped() => break,
                Err(_) => continue,
            };
            self.admit(channel);
        }
    }

    /// Serve one accepted client on its own thread, or refuse it with
    /// `operator_client_limit`.
    pub fn admit(&self, mut channel: Box<dyn FrameChannel>) {
        let admitted = self
            .inner
            .clients
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                (count < MAX_CLIENT_THREADS).then_some(count + 1)
            })
            .is_ok();
        if !admitted {
            let refusal = error_response(
                &AppError::new(
                    "operator_client_limit",
                    "the operator broker has too many connected clients",
                )
                .with_hint("Close an idle client or retry after an active request finishes.")
                .with_details(dict([(
                    "limit",
                    Value::Int(BigInt::from(MAX_CLIENT_THREADS as u64)),
                )]))
                .retryable(true),
            );
            let _ = channel.send_json(&refusal);
            channel.close();
            return;
        }
        let server = self.clone();
        let spawned = std::thread::Builder::new()
            .name("operator-client".into())
            .spawn(move || {
                struct Slot<'a>(&'a AtomicUsize);
                impl Drop for Slot<'_> {
                    fn drop(&mut self) {
                        self.0.fetch_sub(1, Ordering::SeqCst);
                    }
                }
                let _slot = Slot(&server.inner.clients);
                server.serve_channel(channel.as_mut());
            });
        if spawned.is_err() {
            self.inner.clients.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// `request_stop()`: stop admission; full cleanup is `close()`'s.
    pub fn request_stop(&self) {
        self.inner.stopped.store(true, Ordering::SeqCst);
        let Ok(_guard) = self.inner.close_lock.try_lock() else {
            return;
        };
        if self.inner.listener_closed.load(Ordering::SeqCst) {
            return;
        }
        if self.close_listener().is_ok() {
            self.inner.listener_closed.store(true, Ordering::SeqCst);
        }
    }

    fn close_listener(&self) -> Result<()> {
        let taken = self
            .inner
            .listener
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        let Some(listener) = taken else {
            return Ok(());
        };
        #[cfg(windows)]
        {
            listener.close()
        }
        #[cfg(unix)]
        {
            // The accept loop may hold another reference while it polls;
            // it notices the stop within one poll and drops it.
            let mut listener = listener;
            loop {
                match Arc::try_unwrap(listener) {
                    Ok(mut owned) => return owned.close(),
                    Err(shared) => {
                        listener = shared;
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
        }
    }

    /// `close()`: close the listener, then the service, keeping each
    /// retryable.
    pub fn close(&self) -> Result<()> {
        let _guard = self
            .inner
            .close_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.inner.stopped.store(true, Ordering::SeqCst);
        let mut first_error = None;
        if !self.inner.listener_closed.load(Ordering::SeqCst) {
            match self.close_listener() {
                Ok(()) => self.inner.listener_closed.store(true, Ordering::SeqCst),
                Err(error) => first_error = Some(error),
            }
        }
        if let Err(error) = self.inner.service.close() {
            first_error.get_or_insert(error);
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// `_monitor_login_session`.
    fn monitor_login_session(&self) {
        loop {
            let deadline = std::time::Instant::now() + LOGIN_CHECK_INTERVAL;
            while std::time::Instant::now() < deadline {
                if self.is_stopped() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let logind_alive = if self.inner.logind_observed {
                (self.inner.guard.logind)()
            } else {
                None
            };
            if !crate::session::session_lifetime_is_valid(
                self.inner.session_runtime.as_ref(),
                self.inner.logind_observed,
                logind_alive,
                &*self.inner.guard.trust,
            ) {
                let _ = self.close();
                return;
            }
        }
    }

    /// `_serve_channel`: one request, one response.
    pub fn serve_channel(&self, channel: &mut dyn FrameChannel) {
        let mut restart_after_response = false;
        let request = match channel.receive_json() {
            Ok(request) => request,
            Err(Error::Eof(_)) => return,
            Err(error) => {
                let _ = channel.send_json(&response_for_error(&error));
                return;
            }
        };
        let response = (|| -> Result<Value> {
            require_version(&request)?;
            if get(&request, "action") == Some(&s("agent_session_open")) {
                self.serve_agent_session(channel, &request);
                return Err(Error::Eof("agent session ended"));
            }
            let response = self.handle(channel, &request)?;
            restart_after_response = get(&request, "action") == Some(&s("restart_for_native_auth"))
                && get(&response, "ok") == Some(&Value::Bool(true));
            Ok(response)
        })();
        let response = match response {
            Ok(response) => response,
            Err(Error::Eof(_)) => return,
            Err(error) => response_for_error(&error),
        };
        let _ = channel.send_json(&response);
        channel.close();
        if restart_after_response {
            self.request_stop();
        }
    }

    /// `_serve_agent_session`: hold one MCP server's session while its
    /// connection lives and sends heartbeats.
    fn serve_agent_session(&self, channel: &mut dyn FrameChannel, request: &Value) {
        let service = &self.inner.service;
        let opened = required_string(request, "data_root")
            .and_then(|data_root| self.canonical_data_root(&data_root))
            .and_then(|data_root| {
                let pid = channel.peer_pid();
                service.open_agent_session(&data_root, pid, (self.inner.labels)(pid))
            });
        let ticket = match opened {
            Ok(ticket) => ticket,
            Err(error) => {
                let _ = channel.send_json(&response_for_error(&error));
                return;
            }
        };
        // The token goes only to this connection's process.
        let sent = channel.send_json(&ok(dict([
            ("session_id", s(&ticket.id)),
            ("state", s(&ticket.state)),
            ("token", s(&ticket.token)),
        ])));
        if sent.is_ok() {
            while service.agent_session_alive(&ticket.id) {
                match channel.receive_json() {
                    Ok(message)
                        if get(&message, "action") == Some(&s("agent_session_heartbeat")) => {}
                    _ => break,
                }
            }
        }
        service.close_agent_session(&ticket.id);
    }

    /// `_canonical_data_root`.
    fn canonical_data_root(&self, data_root: &str) -> Result<String> {
        let project = self.backend().canonical_project(data_root)?;
        Ok(project
            .database
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_default())
    }

    fn backend(&self) -> &dyn Backend {
        self.inner.service.backend()
    }

    fn continue_secret(&self, label: &str, challenge: &str) -> Value {
        dict([
            ("ok", Value::Bool(true)),
            ("continue", s("secret")),
            ("label", s(label)),
            ("challenge", s(challenge)),
            ("build", self.inner.build.clone()),
        ])
    }

    /// `_handle`: one action.
    pub fn handle(&self, channel: &mut dyn FrameChannel, request: &Value) -> Result<Value> {
        let service = &self.inner.service;
        let action = get(request, "action");
        let is = |name: &str| action == Some(&s(name));
        if is("ping") {
            let identity = self.inner.native.helper_identity().ok();
            return Ok(ok(dict([
                ("broker", s("running")),
                ("generation", s(&self.inner.generation)),
                ("native_auth_available", Value::Bool(identity.is_some())),
                (
                    "native_auth_identity",
                    identity.as_deref().map_or(Value::Null, s),
                ),
                ("caller_context", Value::Bool(true)),
                ("build", self.inner.build.clone()),
            ])));
        }
        if is("restart_for_native_auth") {
            return Ok(ok(service.prepare_idle_restart()?));
        }
        if is("status") {
            let data_root = match get(request, "data_root") {
                Some(Value::Str(text)) => Some(self.canonical_data_root(&text_of(text)?)?),
                _ => None,
            };
            let mut status = service.status(data_root.as_deref())?;
            set(&mut status, "broker_build", self.inner.build.clone());
            return Ok(ok(status));
        }
        if is("unlock") {
            let data_root = self.canonical_data_root(&required_string(request, "data_root")?)?;
            let method = get(request, "authentication_method")
                .cloned()
                .unwrap_or_else(|| s("password"));
            if !py_eq(&method, &s("password")) {
                return Err(Error::App(AppError::new(
                    "operator_invalid_authentication_method",
                    "password unlock cannot claim another authentication method",
                )));
            }
            let agent_scope = get(request, "agent_scope")
                .cloned()
                .unwrap_or(Value::Bool(false));
            let idle_timeout = get(request, "idle_timeout_seconds")
                .cloned()
                .unwrap_or(Value::Null);
            let Value::Bool(agent_scope) = agent_scope else {
                return Err(protocol("invalid broker agent session request"));
            };
            let idle_timeout = match idle_timeout {
                Value::Null => None,
                Value::Int(seconds) => Some(clamp(&seconds)),
                _ => return Err(protocol("invalid broker agent session request")),
            };
            let challenge = new_challenge()?;
            let control_challenge = if agent_scope {
                Some(new_challenge()?)
            } else {
                None
            };
            let mut continuation = self.continue_secret("database_passphrase", &challenge);
            if let Some(control) = &control_challenge {
                set(&mut continuation, "agent_control_challenge", s(control));
            }
            channel.send_json(&continuation)?;
            let passphrase = channel.receive_secret(&challenge)?;
            let control = match &control_challenge {
                // Only its holder (the desktop that unlocked) can later
                // allow an agent process to use this session.
                Some(challenge) => Some(channel.receive_secret(challenge)?),
                None => None,
            };
            let (duration, capability) = lease_request_args(request)?;
            let only_if_locked = match get(request, "only_if_locked") {
                None => false,
                Some(Value::Bool(flag)) => *flag,
                Some(_) => return Err(protocol("invalid broker unlock condition")),
            };
            return Ok(ok(service.unlock(
                &data_root,
                &passphrase,
                UnlockRequest {
                    duration_seconds: duration,
                    capability,
                    authentication_method: "password",
                    only_if_locked,
                    agent_scope,
                    idle_timeout_seconds: idle_timeout,
                    agent_control: control.as_deref().map(Vec::as_slice),
                    ..UnlockRequest::default()
                },
            )?));
        }
        if is("unlock_touch_id") {
            let data_root = self.canonical_data_root(&required_string(request, "data_root")?)?;
            let identity = required_string(request, "native_auth_identity")?;
            self.inner.native.validate(&identity)?;
            let (project_identity, database_identity) = self.inner.native.binding(&data_root)?;
            let (duration, capability) = lease_request_args(request)?;
            let passphrase = self.inner.native.passphrase(&data_root, &identity)?;
            return Ok(ok(service.unlock(
                &data_root,
                &passphrase,
                UnlockRequest {
                    duration_seconds: duration,
                    capability,
                    authentication_method: "touch_id",
                    expected_project_identity: Some(&project_identity),
                    expected_database_identity: Some(&database_identity),
                    ..UnlockRequest::default()
                },
            )?));
        }
        if is("agent_session_decide") {
            let data_root = self.canonical_data_root(&required_string(request, "data_root")?)?;
            let session_id = required_string(request, "session_id")?;
            let Some(Value::Bool(allow)) = get(request, "allow").cloned() else {
                return Err(protocol("invalid agent session decision"));
            };
            let challenge = new_challenge()?;
            channel.send_json(&self.continue_secret("agent_control", &challenge))?;
            let control = channel.receive_secret(&challenge)?;
            return Ok(ok(service.decide_agent_session(
                &data_root,
                &session_id,
                allow,
                &control,
            )?));
        }
        if is("lock") {
            let expected = match get(request, "expected_lease_id") {
                None | Some(Value::Null) => None,
                Some(Value::Str(text)) => Some(text_of(text)?),
                Some(_) => return Err(protocol("invalid broker lock condition")),
            };
            let restore_manual = match get(request, "restore_manual") {
                None => false,
                Some(Value::Bool(flag)) => *flag,
                Some(_) => return Err(protocol("invalid broker lock condition")),
            };
            let data_root = self.canonical_data_root(&required_string(request, "data_root")?)?;
            return Ok(ok(service.lock(
                &data_root,
                expected.as_deref(),
                restore_manual,
            )?));
        }
        if is("submit") {
            return self.handle_submit(channel, request);
        }
        if is("operation_status") {
            let include_output = get(request, "include_output").is_none_or(truthy);
            let operation_id = required_string(request, "operation_id")?;
            return Ok(ok(service.operation_status(&operation_id, include_output)));
        }
        if is("operation_cancel") {
            return Ok(ok(
                service.cancel(&required_string(request, "operation_id")?)
            ));
        }
        if is("set_mode") {
            let data_root = self.canonical_data_root(&required_string(request, "data_root")?)?;
            let mode = required_string(request, "mode")?;
            let challenge = new_challenge()?;
            channel.send_json(&self.continue_secret("fresh_admin_auth", &challenge))?;
            let authentication = channel.receive_secret(&challenge)?;
            return Ok(ok(service.set_mode_authenticated(
                &data_root,
                &authentication,
                &mode,
            )?));
        }
        if is("touch_id_configure") {
            let data_root = self.canonical_data_root(&required_string(request, "data_root")?)?;
            let identity = required_string(request, "native_auth_identity")?;
            self.inner.native.validate(&identity)?;
            let Some(Value::Bool(configured)) = get(request, "configured").cloned() else {
                return Err(protocol("Touch ID configuration requires a boolean state"));
            };
            let challenge = new_challenge()?;
            channel.send_json(&self.continue_secret("fresh_native_auth", &challenge))?;
            let authentication = channel.receive_secret(&challenge)?;
            return Ok(ok(service.configure_touch_id_authenticated(
                &data_root,
                &authentication,
                configured,
                &identity,
                &*self.inner.native,
            )?));
        }
        Err(protocol("unknown operator broker action"))
    }

    /// `_handle_submit`.
    fn handle_submit(&self, channel: &mut dyn FrameChannel, request: &Value) -> Result<Value> {
        let data_root = self.canonical_data_root(&required_string(request, "data_root")?)?;
        let operation_id = required_string(request, "operation_id")?;
        let argv = if get(request, "argv_follows") == Some(&Value::Bool(true)) {
            // The command line can carry secrets and book data: name this
            // build first, so the client sends it only to its own build.
            channel.send_json(&dict([
                ("ok", Value::Bool(true)),
                ("continue", s("argv")),
                ("build", self.inner.build.clone()),
            ]))?;
            get(&channel.receive_json()?, "argv").cloned()
        } else {
            get(request, "argv").cloned()
        };
        let argv: Vec<String> = match argv {
            Some(Value::List(items)) => items
                .iter()
                .map(|item| match item {
                    Value::Str(text) => text_of(text),
                    _ => Err(protocol("invalid broker argv")),
                })
                .collect::<Result<_>>()?,
            _ => return Err(protocol("invalid broker argv")),
        };
        let labels = get(request, "secret_labels")
            .cloned()
            .unwrap_or(Value::List(vec![]));
        let labels: Vec<String> = match labels {
            Value::List(items) if items.len() <= 32 => items
                .iter()
                .map(|item| match item {
                    Value::Str(text) => text_of(text).and_then(|label| {
                        if label.starts_with("broker-secret-") {
                            Ok(label)
                        } else {
                            Err(protocol("invalid broker secret labels"))
                        }
                    }),
                    _ => Err(protocol("invalid broker secret labels")),
                })
                .collect::<Result<_>>()?,
            _ => return Err(protocol("invalid broker secret labels")),
        };
        let unique: std::collections::HashSet<&String> = labels.iter().collect();
        if unique.len() != labels.len() {
            return Err(protocol("invalid broker secret labels"));
        }
        let working_directory = optional_working_directory(request)?;
        let no_egress = match get(request, "no_egress") {
            None => false,
            Some(Value::Bool(flag)) => *flag,
            Some(_) => return Err(protocol("invalid broker egress flag")),
        };
        let agent_session_token = match get(request, "agent_session_token") {
            None | Some(Value::Null) => None,
            Some(Value::Str(text)) if text.code_points().count() <= 128 => Some(text_of(text)?),
            Some(_) => return Err(protocol("invalid agent session token")),
        };
        if let Some(expected) =
            get(request, "expected_build").filter(|value| !matches!(value, Value::Null))
        {
            if !py_eq(expected, &self.inner.build) {
                // Before any secret or admission: the caller must not have
                // this build's work run by another one.
                return Err(Error::App(
                    AppError::new(
                        "operator_broker_build_mismatch",
                        "the operator broker is another Kassiber build",
                    )
                    .with_hint(
                        "Run `kassiber operator lock` and `kassiber operator stop`, then retry.",
                    )
                    .with_details(dict([("broker_build", self.inner.build.clone())])),
                ));
            }
        }
        let parsed = self.inner.service.parse_argv(&argv)?;
        let mut argv = argv;
        let mut admin_label = None;
        if parsed.command_path == "secrets.remember-unlock"
            && !argv.iter().any(|token| token == "--passphrase-fd")
        {
            let label = format!("broker-secret-{}", token_hex(16)?);
            argv.extend(["--passphrase-fd".to_owned(), label.clone()]);
            admin_label = Some(label);
        }
        let mut challenges: Vec<(String, String)> = Vec::new();
        for label in &labels {
            challenges.push((label.clone(), new_challenge()?));
        }
        challenges.sort();
        let admin_challenge = if parsed.capability == Capability::Admin {
            Some(new_challenge()?)
        } else {
            None
        };
        if !challenges.is_empty() || admin_challenge.is_some() {
            channel.send_json(&dict([
                ("ok", Value::Bool(true)),
                ("continue", s("secrets")),
                (
                    "challenges",
                    Value::Dict(
                        challenges
                            .iter()
                            .map(|(label, challenge)| (Key::from(label.as_str()), s(challenge)))
                            .collect(),
                    ),
                ),
                (
                    "admin_challenge",
                    admin_challenge.as_deref().map_or(Value::Null, s),
                ),
                ("build", self.inner.build.clone()),
            ]))?;
        }
        // JSON sorts the challenges; receive in that same order so frames
        // cannot be assigned to another label.
        let mut secret_arguments = Vec::new();
        for (label, challenge) in &challenges {
            secret_arguments.push((label.clone(), channel.receive_secret(challenge)?));
        }
        let mut authorization = None;
        if let Some(challenge) = &admin_challenge {
            let admin_auth = channel.receive_secret(challenge)?;
            authorization = Some(self.inner.service.verify_admin(&data_root, &admin_auth)?);
            if let Some(label) = &admin_label {
                secret_arguments.push((label.clone(), Zeroizing::new(admin_auth.to_vec())));
            }
        }
        let mut accepted = self.inner.service.submit(
            &data_root,
            &argv,
            SubmitRequest {
                operation_id: Some(operation_id),
                secret_arguments,
                admin_authorization: authorization.as_ref(),
                working_directory,
                no_egress,
                peer_pid: channel.peer_pid(),
                agent_session_token,
            },
        )?;
        set(&mut accepted, "build", self.inner.build.clone());
        Ok(ok(accepted))
    }
}

fn protocol(message: &str) -> Error {
    Error::App(AppError::new("operator_protocol_error", message))
}

/// A wire string as Rust text; a lone surrogate fails as Python's
/// filesystem encoding would (an internal error).
fn text_of(text: &PyStr) -> Result<String> {
    text.as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::Value("a string holds a lone surrogate".into()))
}

/// `_required_string`.
fn required_string(request: &Value, key: &str) -> Result<String> {
    match get(request, key) {
        Some(Value::Str(text)) if text.code_points().next().is_some() => text_of(text),
        _ => Err(protocol(&format!("operator request requires {key}"))),
    }
}

/// Python ints are unbounded; past `i64` a duration overflows the
/// timestamp range on the way up and is below a minute on the way down.
fn clamp(value: &BigInt) -> i64 {
    i64::try_from(value.clone()).unwrap_or(if *value > BigInt::from(0) {
        i64::MAX
    } else {
        i64::MIN
    })
}

/// `_lease_request_args`.
fn lease_request_args(request: &Value) -> Result<(Option<i64>, Capability)> {
    let duration = match get(request, "duration_seconds") {
        None | Some(Value::Null) => None,
        Some(Value::Int(seconds)) => Some(clamp(seconds)),
        Some(Value::Bool(flag)) => Some(i64::from(*flag)),
        Some(_) => {
            return Err(Error::App(AppError::new(
                "operator_invalid_duration",
                "invalid operator duration",
            )))
        }
    };
    let capability = match get(request, "capability") {
        None => Some(Capability::AccountingDecisions),
        Some(Value::Str(text)) => text.as_str().and_then(Capability::parse),
        Some(_) => None,
    };
    let Some(capability) = capability else {
        return Err(Error::App(AppError::new(
            "operator_invalid_lease_capability",
            "invalid operator lease capability",
        )));
    };
    Ok((duration, capability))
}

/// `_optional_working_directory`.
fn optional_working_directory(request: &Value) -> Result<Option<String>> {
    let value = match get(request, "working_directory") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Str(text)) => text,
        Some(_) => return Err(protocol("invalid broker working directory")),
    };
    let text = text_of(value)?;
    if text.is_empty()
        || value.code_points().count() > 4096
        || text.contains('\0')
        || !std::path::Path::new(&text).is_absolute()
    {
        return Err(protocol("invalid broker working directory"));
    }
    if !std::path::Path::new(&text).is_dir() {
        return Err(Error::App(
            AppError::new(
                "operator_working_directory_unavailable",
                "the caller's working directory no longer exists",
            )
            .with_hint("Run the command from an existing directory or pass absolute paths."),
        ));
    }
    Ok(Some(text))
}

/// `_require_version`.
fn require_version(request: &Value) -> Result<()> {
    let version = get(request, "version").cloned().unwrap_or(Value::Null);
    if !py_eq(&version, &Value::Int(BigInt::from(PROTOCOL_VERSION))) {
        return Err(Error::App(
            AppError::new(
                "operator_protocol_version_mismatch",
                "operator protocol version mismatch",
            )
            .with_details(dict([(
                "supported",
                Value::Int(BigInt::from(PROTOCOL_VERSION)),
            )])),
        ));
    }
    Ok(())
}

/// `_ok`.
pub fn ok(data: Value) -> Value {
    dict([("ok", Value::Bool(true)), ("data", data)])
}

/// What a failure answers: an `AppError` through the filter, anything else
/// as `operator_internal_error`.
pub fn response_for_error(error: &Error) -> Value {
    match error {
        Error::App(error) => error_response(error),
        _ => error_response(
            &AppError::new(
                "operator_internal_error",
                "the operator broker encountered an internal error",
            )
            .retryable(true),
        ),
    }
}

/// A wire string as text, a lone surrogate as U+FFFD.
fn lossy(text: &PyStr) -> String {
    match text.as_str() {
        Some(text) => text.to_owned(),
        None => text
            .code_points()
            .map(|point| char::from_u32(point).unwrap_or('\u{fffd}'))
            .collect(),
    }
}

fn sanitized(text: &PyStr) -> Value {
    s(&sanitize_traceback_text(&lossy(text)))
}

/// `_error_response`.
pub fn error_response(error: &AppError) -> Value {
    dict([
        ("ok", Value::Bool(false)),
        (
            "error",
            dict([
                ("code", s(&error.code)),
                ("message", s(&sanitize_traceback_text(&error.message))),
                (
                    "hint",
                    error
                        .hint
                        .as_deref()
                        .filter(|hint| !hint.is_empty())
                        .map_or(Value::Null, |hint| s(&sanitize_traceback_text(hint))),
                ),
                (
                    "details",
                    error
                        .details
                        .as_ref()
                        .map_or(Value::Null, public_safe_details),
                ),
                ("retryable", Value::Bool(error.retryable)),
            ]),
        ),
    ])
}

fn key_text(key: &Key) -> String {
    match key {
        Key::Str(text) => lossy(text),
        Key::Null => "None".into(),
        Key::Bool(true) => "True".into(),
        Key::Bool(false) => "False".into(),
        Key::Int(n) => n.to_string(),
        Key::Float(f) => kassiber_pyjson::float_repr(*f),
    }
}

/// `_public_safe_details`: drop path keys, redact sensitive keys, sanitize
/// every string.
pub fn public_safe_details(value: &Value) -> Value {
    match value {
        Value::Dict(pairs) => {
            let mut safe: Vec<(Key, Value)> = Vec::new();
            for (key, item) in pairs {
                let text = key_text(key);
                if matches!(
                    kassiber_pytext::lower(&text).as_str(),
                    "path" | "database" | "data_root"
                ) {
                    continue;
                }
                let item = if is_sensitive_key(&text) {
                    s("[redacted]")
                } else {
                    public_safe_details(item)
                };
                // A later duplicate replaces an earlier one, as in a dict.
                match safe
                    .iter_mut()
                    .find(|(existing, _)| key_text(existing) == text)
                {
                    Some(slot) => slot.1 = item,
                    None => safe.push((Key::from(text.as_str()), item)),
                }
            }
            Value::Dict(safe)
        }
        Value::List(items) => Value::List(items.iter().map(public_safe_details).collect()),
        Value::Str(text) => sanitized(text),
        Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Null => value.clone(),
        other => s(&sanitize_traceback_text(&kassiber_envelope::py_str(other))),
    }
}

/// The `--operator-broker-server` entry (`server.main()`): start, serve
/// until a signal or a stop, close. Returns the exit code.
pub fn main(
    backend: Arc<dyn Backend>,
    native: Arc<dyn NativeAuth>,
    inputs: BuildInputs<'_>,
    guard: SessionGuard,
    stderr: &mut dyn std::io::Write,
) -> i32 {
    let report = |stderr: &mut dyn std::io::Write, error: &Error| {
        let line = match error {
            Error::App(error) => format!("{}: {}\n", error.code, error.message),
            other => format!("operator_internal_error: {other}\n"),
        };
        let _ = stderr.write_all(line.as_bytes());
    };
    let started = (|| -> Result<BrokerServer> {
        let generation = token_hex(16)?;
        let runner = Arc::new(crate::runner::CliRunner::current(Arc::clone(&backend))?);
        let telemetry: Arc<dyn Telemetry> = Arc::new(LogRing::new(1000));
        let service = OperatorService::with_options(
            &generation,
            backend,
            runner,
            telemetry,
            crate::service::Limits::default(),
        );
        let parts = ServerParts {
            generation,
            service: service.clone(),
            native,
            build: current_build_identity(inputs),
            guard,
        };
        match BrokerServer::start(parts, &crate::runtime::Runtime::from_process()) {
            Ok(server) => Ok(server),
            Err(error) => {
                let _ = service.close();
                Err(error)
            }
        }
    })();
    let server = match started {
        Ok(server) => server,
        Err(error) => {
            report(stderr, &error);
            return 1;
        }
    };
    let signal = kassiber_ipc::signal::install_stop_signals().ok();
    server.serve_forever(signal);
    match server.close() {
        Ok(()) => 0,
        Err(error) => {
            report(stderr, &error);
            1
        }
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;

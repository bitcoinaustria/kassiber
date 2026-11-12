//! The broker client wire for all 14 actions (`kassiber/operator/client.py`).
//!
//! The client never starts a broker except from `unlock`, `mode`,
//! `touch-id enroll`/`forget` and a `submit` that asks for it; status, lock,
//! stop, operation reads and agent sessions only connect to an existing
//! endpoint. Passphrases, command secrets and agent control secrets travel
//! only in `S` frames and only to a broker that named this build in the same
//! connection's reply.

use crate::build::describe_build;
use crate::error::{app, protocol, Error, Result};
use crate::protocol::{FrameChannel, PROTOCOL_VERSION};
use crate::runtime::Runtime;
use crate::values::{
    as_str, dict, get, get_or_null, is_false, is_true, pop, py_eq, py_str_value, s, set, truthy,
};
use kassiber_envelope::AppError;
use kassiber_pyjson::{BigInt, Key, PyStr, Value};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub const AGENT_SESSION_HEARTBEAT: Duration = Duration::from_secs(10);
pub const SOURCE_BROKER_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
pub const FROZEN_BROKER_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
pub const WINDOWS_FROZEN_BROKER_STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
/// `connect()` defaults: 2 s to connect, no I/O deadline.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// `ping`: 1 s to connect and 1 s per operation.
pub const PING_TIMEOUT: Duration = Duration::from_secs(1);
/// `_simple_request`: 2 s per operation.
pub const SIMPLE_REQUEST_IO_TIMEOUT: Duration = Duration::from_secs(2);
/// Agent session open and decide: 5 s per operation.
pub const AGENT_IO_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the client waits for a restarted native-auth broker.
pub const NATIVE_AUTH_RESTART_WAIT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

pub const TERMINAL_OPERATION_STATES: [&str; 4] =
    ["completed", "failed", "cancelled", "result_unknown"];
const KNOWN_OPERATION_STATES: [&str; 5] = ["queued", "running", "completed", "failed", "cancelled"];

const OTHER_BUILD_HINT: &str = "Another Kassiber build runs this user's operator broker. Run \
`kassiber operator lock` and `kassiber operator stop` from any build, then retry.";
const OTHER_PROTOCOL_HINT: &str = "An older or newer Kassiber build runs this user's operator \
broker. Run `kassiber operator lock` and `kassiber operator stop` from that build, or end its \
broker process, then retry.";
const NATIVE_AUTH_HINT: &str =
    "Use password authentication or retry from the signed macOS app CLI.";

/// What the client needs from the process around it. [`SystemHost`] is the
/// real one; tests script connections, launches, entropy and time.
pub trait Host: Send + Sync {
    fn connect(
        &self,
        timeout: Duration,
        io_timeout: Option<Duration>,
    ) -> Result<Box<dyn FrameChannel>>;
    /// Start a detached broker (`broker_server_command()`).
    fn launch_broker(&self) -> Result<()>;
    fn startup_timeout(&self) -> Duration;
    /// `build_identity()` of this process.
    fn build_identity(&self) -> Value;
    /// `secrets.token_hex(bytes)`.
    fn token_hex(&self, bytes: usize) -> Result<String>;
    /// `os.getcwd()`.
    fn current_dir(&self) -> std::io::Result<PyStr>;
    /// `os.environ.get(key)`.
    fn env_var(&self, key: &str) -> Option<String>;
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
    /// `sys.stderr.write(text)`.
    fn warn(&self, text: &str);
}

/// `native_auth_caller_identity`, when this CLI hands a signed launcher's
/// identity to the broker.
pub type CallerIdentity<'a> = Option<&'a dyn Fn() -> Result<String>>;

/// The command line and the secret values its fd/stdin flags carried, by
/// label in the order they were read.
#[derive(Default)]
pub struct PreparedArguments {
    pub argv: Vec<String>,
    pub secrets: Vec<(String, Zeroizing<Vec<u8>>)>,
}

impl PreparedArguments {
    pub fn new(argv: Vec<String>) -> Self {
        PreparedArguments {
            argv,
            secrets: Vec::new(),
        }
    }

    fn secret(&self, label: &str) -> Option<&[u8]> {
        self.secrets
            .iter()
            .rev()
            .find(|(name, _)| name == label)
            .map(|(_, value)| value.as_slice())
    }

    /// `wipe_prepared`.
    pub fn wipe(&mut self) {
        self.secrets.clear();
    }
}

/// Options of `BrokerClient.submit`.
#[derive(Clone, Debug, Default)]
pub struct SubmitOptions<'a> {
    pub start_broker: bool,
    pub require_caller_context: bool,
    pub require_same_build: bool,
    pub agent_session_token: Option<&'a str>,
}

impl SubmitOptions<'_> {
    /// Python's defaults: start a broker, no strict context, any build.
    pub fn new() -> Self {
        SubmitOptions {
            start_broker: true,
            ..SubmitOptions::default()
        }
    }
}

fn request(action: &str, fields: Vec<(&str, Value)>) -> Value {
    let mut pairs = vec![
        ("version", Value::from(PROTOCOL_VERSION)),
        ("action", s(action)),
    ];
    pairs.extend(fields);
    dict(pairs)
}

fn optional_int(value: Option<i64>) -> Value {
    value.map_or(Value::Null, Value::from)
}

/// The operator broker client.
pub struct BrokerClient<H: Host> {
    host: H,
}

impl<H: Host> BrokerClient<H> {
    pub fn new(host: H) -> Self {
        BrokerClient { host }
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    fn this_build(&self) -> Value {
        self.host.build_identity()
    }

    /// `ensure_running`: ping, or launch a broker and wait for its ping.
    pub fn ensure_running(&self) -> Result<Value> {
        match self.ping() {
            Ok(broker) => return Ok(broker),
            // Another build's broker holds the endpoint: launching ours
            // would only lose the election and wait out the timeout.
            Err(Error::App(error)) => self.raise_if_other_protocol(&error)?,
            Err(error) if error.is_connection() => {}
            Err(error) => return Err(error),
        }
        self.host.launch_broker()?;
        let deadline = self.host.now() + self.host.startup_timeout();
        let mut last_error: Option<Error> = None;
        while self.host.now() < deadline {
            match self.ping() {
                Ok(broker) => return Ok(broker),
                Err(Error::App(error)) => {
                    self.raise_if_other_protocol(&error)?;
                    last_error = Some(Error::App(error));
                }
                Err(error) if error.is_connection() => last_error = Some(error),
                Err(error) => return Err(error),
            }
            self.host.sleep(POLL_INTERVAL);
        }
        Err(Error::App(
            app(
                "operator_broker_start_failed",
                "the operator broker did not become ready",
            )
            .with_hint("Run the command again with --debug to inspect local startup failures.")
            .with_details(dict([(
                "reason",
                s(last_error.as_ref().map_or("unknown", Error::python_class)),
            )]))
            .retryable(true),
        ))
    }

    /// `ping`: never starts a broker.
    pub fn ping(&self) -> Result<Value> {
        let mut channel = self.host.connect(PING_TIMEOUT, Some(PING_TIMEOUT))?;
        let result = (|| {
            channel.send_json(&request("ping", vec![]))?;
            receive_data(channel.as_mut())
        })();
        channel.close();
        result
    }

    /// `ensure_native_auth_running`: hand a helper-less idle broker over to
    /// this signed CLI. `caller_identity` is `native_auth_caller_identity`
    /// on macOS with `KASSIBER_NATIVE_AUTH_HELPER` set, otherwise `None`; it
    /// runs after the broker is up, as in Python.
    pub fn ensure_native_auth_running(&self, caller_identity: CallerIdentity<'_>) -> Result<Value> {
        let broker = self.ensure_running()?;
        let Some(caller_identity) = caller_identity else {
            return Ok(broker);
        };
        let expected = caller_identity()?;
        let expected = expected.as_str();
        let matches = |broker: &Value| {
            is_true(get(broker, "native_auth_available"))
                && as_str(get(broker, "native_auth_identity")).is_some_and(|id| id.is(expected))
        };
        if matches(&broker) {
            return Ok(broker);
        }
        let old_generation = get_or_null(&broker, "generation");
        self.simple_request("restart_for_native_auth", vec![])?;
        let deadline = self.host.now() + NATIVE_AUTH_RESTART_WAIT;
        while self.host.now() < deadline {
            let running = match self.ping() {
                Ok(running) => running,
                Err(error) if error.is_connection() || error.app().is_some() => break,
                Err(error) => return Err(error),
            };
            if !py_eq(&get_or_null(&running, "generation"), &old_generation) {
                if matches(&running) {
                    return Ok(running);
                }
                break;
            }
            self.host.sleep(POLL_INTERVAL);
        }
        let replacement = self.ensure_running()?;
        if !matches(&replacement) {
            return Err(Error::App(
                app(
                    "native_auth_unavailable",
                    "the restarted operator broker cannot use native authentication",
                )
                .with_hint(NATIVE_AUTH_HINT),
            ));
        }
        Ok(replacement)
    }

    /// `status`: a stopped broker is a locked lease.
    pub fn status(&self, data_root: Option<&str>) -> Result<Value> {
        match self.simple_request(
            "status",
            vec![("data_root", data_root.map_or(Value::Null, s))],
        ) {
            Err(error) if error.is_connection() => {
                Ok(dict([("broker", s("stopped")), ("lease", s("locked"))]))
            }
            other => other,
        }
    }

    /// `unlock`; with `agent_control`, an agent-only session admitting only
    /// `kassiber mcp` calls the control holder allowed.
    #[allow(clippy::too_many_arguments)]
    pub fn unlock(
        &self,
        data_root: &str,
        passphrase: &[u8],
        duration_seconds: Option<BigInt>,
        capability: &str,
        authentication_method: &str,
        only_if_locked: bool,
        agent_control: Option<&[u8]>,
        idle_timeout_seconds: Option<i64>,
    ) -> Result<Value> {
        self.require_same_build(&self.ensure_running()?)?;
        let mut fields = vec![
            ("data_root", s(data_root)),
            (
                "duration_seconds",
                duration_seconds.map_or(Value::Null, Value::Int),
            ),
            ("capability", s(capability)),
            ("authentication_method", s(authentication_method)),
        ];
        if only_if_locked {
            fields.push(("only_if_locked", Value::Bool(true)));
        }
        if agent_control.is_some() {
            fields.push(("agent_scope", Value::Bool(true)));
            fields.push(("idle_timeout_seconds", optional_int(idle_timeout_seconds)));
        }
        self.with_channel(CONNECT_TIMEOUT, None, |channel| {
            channel.send_json(&request("unlock", fields))?;
            let continuation = receive(channel)?;
            let challenge = secret_challenge(&continuation, "invalid broker unlock challenge")?;
            let control_challenge = get(&continuation, "agent_control_challenge");
            if agent_control.is_some() != as_str(control_challenge).is_some() {
                return Err(protocol("invalid broker unlock challenge"));
            }
            // The broker asking for the secret must be this build too; a ping
            // on an earlier connection could have reached another one.
            self.require_same_build(&continuation)?;
            send_secret(channel, &challenge, passphrase)?;
            if let Some(control) = agent_control {
                let control_challenge = py_str_value(control_challenge.unwrap_or(&Value::Null));
                send_secret(channel, &control_challenge, control)?;
            }
            receive_data(channel)
        })
    }

    /// `decide_agent_session`: allow or deny an agent process, proving the
    /// session's control secret.
    pub fn decide_agent_session(
        &self,
        data_root: &str,
        session_id: &str,
        allow: bool,
        agent_control: &[u8],
    ) -> Result<Value> {
        self.with_channel(CONNECT_TIMEOUT, Some(AGENT_IO_TIMEOUT), |channel| {
            channel.send_json(&request(
                "agent_session_decide",
                vec![
                    ("data_root", s(data_root)),
                    ("session_id", s(session_id)),
                    ("allow", Value::Bool(allow)),
                ],
            ))?;
            let continuation = receive(channel)?;
            let challenge = secret_challenge(&continuation, "invalid broker decision challenge")?;
            self.require_same_build(&continuation)?;
            send_secret(channel, &challenge, agent_control)?;
            receive_data(channel)
        })
    }

    /// `open_agent_session`: ask to use the project's agent session as this
    /// process; the connection stays open with heartbeats while the handle
    /// lives.
    pub fn open_agent_session(&self, data_root: &str) -> Result<AgentSessionHandle> {
        let mut channel = self.host.connect(CONNECT_TIMEOUT, Some(AGENT_IO_TIMEOUT))?;
        let opened = (|| {
            channel.send_json(&request(
                "agent_session_open",
                vec![("data_root", s(data_root))],
            ))?;
            receive_data(channel.as_mut())
        })();
        let opened = match opened {
            Ok(opened) => opened,
            Err(error) => {
                channel.close();
                return Err(error);
            }
        };
        match (
            as_str(get(&opened, "session_id")),
            as_str(get(&opened, "token")),
        ) {
            (Some(session_id), Some(token)) => Ok(AgentSessionHandle::start(
                channel,
                session_id.clone(),
                token.clone(),
            )),
            _ => {
                channel.close();
                Err(protocol("broker did not return an agent session"))
            }
        }
    }

    /// `unlock_touch_id`.
    pub fn unlock_touch_id(
        &self,
        data_root: &str,
        duration_seconds: Option<BigInt>,
        capability: &str,
        caller_identity: CallerIdentity<'_>,
    ) -> Result<Value> {
        let broker = self.ensure_native_auth_running(caller_identity)?;
        self.require_same_build(&broker)?;
        let identity = broker_native_auth_identity(&broker)?;
        self.with_channel(CONNECT_TIMEOUT, None, |channel| {
            channel.send_json(&request(
                "unlock_touch_id",
                vec![
                    ("data_root", s(data_root)),
                    (
                        "duration_seconds",
                        duration_seconds.map_or(Value::Null, Value::Int),
                    ),
                    ("capability", s(capability)),
                    ("native_auth_identity", Value::Str(identity)),
                ],
            ))?;
            receive_data(channel)
        })
    }

    /// `stop`: ask an idle broker of any build to exit.
    pub fn stop(&self) -> Result<Value> {
        let broker = match self.ping() {
            Ok(broker) => broker,
            Err(Error::App(error)) if error.code == "operator_protocol_version_mismatch" => {
                // It would refuse the stop request too: only its own build can.
                return Err(Error::App(
                    app(
                        "operator_broker_build_mismatch",
                        "the running operator broker speaks another protocol version",
                    )
                    .with_hint(OTHER_PROTOCOL_HINT)
                    .with_details(dict([("this_build", self.this_build())])),
                ));
            }
            Err(error) if error.is_connection() => return Ok(dict([("broker", s("stopped"))])),
            Err(error) => return Err(error),
        };
        match self.simple_request("restart_for_native_auth", vec![]) {
            Ok(_) => {}
            Err(Error::App(error)) if error.code == "operator_broker_busy" => {
                return Err(Error::App(
                    app(
                        "operator_broker_busy",
                        "the operator broker is still in use",
                    )
                    .with_hint("Run `kassiber operator lock` and wait for queued work, then retry.")
                    .retryable(true),
                ))
            }
            Err(error) => return Err(error),
        }
        Ok(dict([
            ("broker", s("stopping")),
            ("broker_build", get_or_null(&broker, "build")),
        ]))
    }

    /// `lock`: a stopped broker has nothing to lock.
    pub fn lock(
        &self,
        data_root: &str,
        expected_lease_id: Option<&str>,
        restore_manual: bool,
    ) -> Result<Value> {
        let mut fields = vec![("data_root", s(data_root))];
        if let Some(lease) = expected_lease_id {
            fields.push(("expected_lease_id", s(lease)));
        }
        if restore_manual {
            fields.push(("restore_manual", Value::Bool(true)));
        }
        match self.simple_request("lock", fields) {
            Err(error) if error.is_connection() => Ok(dict([
                ("broker", s("stopped")),
                ("locked", Value::Bool(true)),
                ("lease_existed", Value::Bool(false)),
            ])),
            other => other,
        }
    }

    /// `set_mode` with fresh authentication.
    pub fn set_mode(&self, data_root: &str, mode: &str, authentication: &[u8]) -> Result<Value> {
        self.require_same_build(&self.ensure_running()?)?;
        self.with_channel(CONNECT_TIMEOUT, None, |channel| {
            channel.send_json(&request(
                "set_mode",
                vec![("data_root", s(data_root)), ("mode", s(mode))],
            ))?;
            let continuation = receive(channel)?;
            let challenge = secret_challenge(&continuation, "invalid broker mode challenge")?;
            self.require_same_build(&continuation)?;
            send_secret(channel, &challenge, authentication)?;
            receive_data(channel)
        })
    }

    /// `configure_touch_id` with fresh authentication.
    pub fn configure_touch_id(
        &self,
        data_root: &str,
        authentication: &[u8],
        configured: bool,
        caller_identity: CallerIdentity<'_>,
    ) -> Result<Value> {
        let broker = self.ensure_native_auth_running(caller_identity)?;
        self.require_same_build(&broker)?;
        let identity = broker_native_auth_identity(&broker)?;
        self.with_channel(CONNECT_TIMEOUT, None, |channel| {
            channel.send_json(&request(
                "touch_id_configure",
                vec![
                    ("data_root", s(data_root)),
                    ("configured", Value::Bool(configured)),
                    ("native_auth_identity", Value::Str(identity)),
                ],
            ))?;
            let continuation = receive(channel)?;
            let challenge =
                secret_challenge(&continuation, "invalid broker native-auth challenge")?;
            self.require_same_build(&continuation)?;
            send_secret(channel, &challenge, authentication)?;
            receive_data(channel)
        })
    }

    /// `submit`: queue one brokered command, reconciling a lost
    /// acknowledgement through `operation_status` rather than resubmitting
    /// blindly.
    pub fn submit(
        &self,
        data_root: &str,
        prepared: &PreparedArguments,
        admin_authentication: Option<&[u8]>,
        options: &SubmitOptions<'_>,
    ) -> Result<Value> {
        let broker = if options.start_broker {
            self.ensure_running()?
        } else {
            match self.ping() {
                Ok(broker) => broker,
                Err(Error::App(error)) => {
                    self.raise_if_other_protocol(&error)?;
                    return Err(Error::App(no_lease_error()));
                }
                Err(error) if error.is_connection() => return Err(Error::App(no_lease_error())),
                Err(error) => return Err(error),
            }
        };
        let Some(generation) = as_str(get(&broker, "generation")) else {
            return Err(protocol("broker generation is unavailable"));
        };
        if options.require_same_build {
            self.require_same_build(&broker)?;
        }
        self.require_caller_context_support(&broker, options.require_caller_context)?;
        let token = self.host.token_hex(16)?;
        let operation_id = PyStr::from_code_points(
            generation
                .code_points()
                .chain(".client.".chars().map(u32::from))
                .chain(token.chars().map(u32::from)),
        )
        .map_err(|error| Error::Value(error.to_string()))?;
        let first = self.submit_once(
            data_root,
            prepared,
            &operation_id,
            admin_authentication,
            options,
        );
        match first {
            Ok(accepted) => self.accepted(
                accepted,
                &operation_id,
                options.require_same_build,
                get_or_null(&broker, "build"),
            ),
            Err(error) if error.is_connection() => {
                let retried = self
                    .submit_once(
                        data_root,
                        prepared,
                        &operation_id,
                        admin_authentication,
                        options,
                    )
                    .and_then(|accepted| {
                        // After a lost connection the ping may describe another
                        // broker than the one that accepted: report no build.
                        self.accepted(
                            accepted,
                            &operation_id,
                            options.require_same_build,
                            Value::Null,
                        )
                    });
                match retried {
                    Ok(accepted) => Ok(accepted),
                    Err(Error::App(error)) if error.code == "operator_broker_build_mismatch" => {
                        Err(Error::App(error))
                    }
                    Err(error) if error.is_connection() || error.app().is_some() => {
                        self.reconcile(&operation_id)
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    fn reconcile(&self, operation_id: &PyStr) -> Result<Value> {
        let status = match self.operation_status(operation_id) {
            Ok(status) => status,
            Err(error) if error.is_connection() || error.app().is_some() => dict([
                ("operation_id", Value::Str(operation_id.clone())),
                ("state", s("result_unknown")),
                ("reason", s("broker_status_unavailable")),
            ]),
            Err(error) => return Err(error),
        };
        if as_str(get(&status, "state"))
            .is_some_and(|state| KNOWN_OPERATION_STATES.iter().any(|known| state.is(known)))
        {
            return Ok(status);
        }
        let reason = match get(&status, "reason") {
            Some(Value::Str(reason)) => Value::Str(reason.clone()),
            _ => s("submission_acknowledgement_lost"),
        };
        Err(Error::App(
            app(
                "operator_submission_result_unknown",
                "the broker could not prove the result of the submitted operation",
            )
            .with_hint("Reconcile project state before retrying the operation.")
            .with_details(dict([
                ("operation_id", Value::Str(operation_id.clone())),
                ("state", s("result_unknown")),
                ("reason", reason),
            ])),
        ))
    }

    /// `_submit_once`: the opening request, then the command line once the
    /// broker named its build, then any secrets it challenges for.
    pub fn submit_once(
        &self,
        data_root: &str,
        prepared: &PreparedArguments,
        operation_id: &PyStr,
        admin_authentication: Option<&[u8]>,
        options: &SubmitOptions<'_>,
    ) -> Result<Value> {
        let mut fields = vec![
            ("data_root", s(data_root)),
            ("operation_id", Value::Str(operation_id.clone())),
            // The command line follows once the broker has named its build.
            ("argv_follows", Value::Bool(true)),
            (
                "secret_labels",
                Value::List(
                    prepared
                        .secrets
                        .iter()
                        .map(|(label, _)| s(label.as_str()))
                        .collect(),
                ),
            ),
            (
                "working_directory",
                Value::Str(self.caller_working_directory()?),
            ),
            ("no_egress", Value::Bool(self.caller_disables_egress())),
        ];
        if options.require_same_build {
            // The broker refuses before admitting anything; the checks on its
            // replies below cover a broker that ignores this field.
            fields.push(("expected_build", self.this_build()));
        }
        if let Some(token) = options.agent_session_token {
            fields.push(("agent_session_token", s(token)));
        }
        self.with_channel(CONNECT_TIMEOUT, None, |channel| {
            channel.send_json(&request("submit", fields))?;
            let response = receive(channel)?;
            if !as_str(get(&response, "continue")).is_some_and(|c| c.is("argv")) {
                return Err(protocol("invalid broker submit continuation"));
            }
            // Inline arguments can carry secrets (`--token VALUE`) and book
            // data: they, like every brokered command, go only to this build.
            self.require_same_build(&response)?;
            channel.send_json(&dict([(
                "argv",
                Value::List(prepared.argv.iter().map(|arg| s(arg.as_str())).collect()),
            )]))?;
            let response = receive(channel)?;
            if !as_str(get(&response, "continue")).is_some_and(|c| c.is("secrets")) {
                return data_from_response(&response);
            }
            // Command secrets and fresh admin authentication go only to this
            // build's broker; it named its build in this very reply.
            self.require_same_build(&response)?;
            let Some(Value::Dict(challenges)) = get(&response, "challenges") else {
                return Err(protocol("invalid broker secret challenge"));
            };
            let mut labels: Vec<(&PyStr, &Value)> = Vec::new();
            for (key, value) in challenges {
                // JSON object keys are always strings.
                let Key::Str(label) = key else {
                    return Err(protocol("invalid broker secret challenge"));
                };
                match labels.iter_mut().find(|(seen, _)| *seen == label) {
                    Some(slot) => slot.1 = value,
                    None => labels.push((label, value)),
                }
            }
            labels.sort_by(|(a, _), (b, _)| a.code_points().cmp(b.code_points()));
            for (label, challenge) in labels {
                let secret = label.as_str().and_then(|label| prepared.secret(label));
                let (Some(secret), Value::Str(challenge)) = (secret, challenge) else {
                    return Err(protocol("invalid broker secret challenge"));
                };
                send_secret(channel, challenge, secret)?;
            }
            match get(&response, "admin_challenge") {
                None | Some(Value::Null) => {}
                Some(admin_challenge) => {
                    let Some(admin) = admin_authentication else {
                        return Err(Error::App(app(
                            "operator_admin_auth_required",
                            "fresh authentication is required for this admin command",
                        )));
                    };
                    let Value::Str(admin_challenge) = admin_challenge else {
                        return Err(protocol("invalid broker admin challenge"));
                    };
                    send_secret(channel, admin_challenge, admin)?;
                }
            }
            receive_data(channel)
        })
    }

    /// `operation_status`: an unreachable broker cannot prove a result.
    pub fn operation_status(&self, operation_id: &PyStr) -> Result<Value> {
        match self.simple_request(
            "operation_status",
            vec![
                ("operation_id", Value::Str(operation_id.clone())),
                ("include_output", Value::Bool(true)),
            ],
        ) {
            Err(error) if error.is_connection() => Ok(dict([
                ("operation_id", Value::Str(operation_id.clone())),
                ("state", s("result_unknown")),
                ("reason", s("broker_unreachable")),
                (
                    "hint",
                    s("Reconcile project state before retrying the operation."),
                ),
            ])),
            other => other,
        }
    }

    /// `_accepted`: name the build that accepted the work; withdraw work
    /// another build accepted when this build is required.
    pub fn accepted(
        &self,
        accepted: Value,
        operation_id: &PyStr,
        require_same_build: bool,
        pinged_build: Value,
    ) -> Result<Value> {
        let mut accepted = accepted;
        let mut build = pop(&mut accepted, "build");
        if matches!(build, Value::Null) && !require_same_build {
            // An earlier build's acceptance does not name itself; its ping may.
            build = pinged_build;
        }
        if require_same_build && !py_eq(&build, &self.this_build()) {
            // Another broker took the endpoint after the ping. Its work never
            // reaches the caller, so withdraw it rather than leave it queued.
            match self.cancel(operation_id) {
                Err(error) if !(error.is_connection() || error.app().is_some()) => {
                    return Err(error)
                }
                _ => {}
            }
            self.require_same_build(&dict([("build", build)]))?;
            unreachable!("a differing build was refused above");
        }
        // Queued work runs the broker's build; callers can surface it.
        set(&mut accepted, "broker_build", build);
        Ok(accepted)
    }

    /// `cancel`.
    pub fn cancel(&self, operation_id: &PyStr) -> Result<Value> {
        self.simple_request(
            "operation_cancel",
            vec![("operation_id", Value::Str(operation_id.clone()))],
        )
    }

    /// `wait`: poll until a terminal state, backing off to 0.5 s.
    pub fn wait(&self, operation_id: &PyStr) -> Result<Value> {
        let mut delay = 0.05f64;
        loop {
            let status = self.operation_status(operation_id)?;
            if as_str(get(&status, "state"))
                .is_some_and(|state| TERMINAL_OPERATION_STATES.iter().any(|t| state.is(t)))
            {
                return Ok(status);
            }
            self.host.sleep(Duration::from_secs_f64(delay));
            delay = (delay * 1.4).min(0.5);
        }
    }

    /// `_simple_request`: one request and its data on a 2 s connection.
    pub fn simple_request(&self, action: &str, fields: Vec<(&str, Value)>) -> Result<Value> {
        self.with_channel(
            CONNECT_TIMEOUT,
            Some(SIMPLE_REQUEST_IO_TIMEOUT),
            |channel| {
                channel.send_json(&request(action, fields))?;
                receive_data(channel)
            },
        )
    }

    fn with_channel<T>(
        &self,
        timeout: Duration,
        io_timeout: Option<Duration>,
        body: impl FnOnce(&mut dyn FrameChannel) -> Result<T>,
    ) -> Result<T> {
        let mut channel = self.host.connect(timeout, io_timeout)?;
        let result = body(channel.as_mut());
        channel.close();
        result
    }

    fn raise_if_other_protocol(&self, error: &AppError) -> Result<()> {
        if error.code == "operator_protocol_version_mismatch" {
            return Err(Error::App(
                app(
                    "operator_broker_build_mismatch",
                    "the running operator broker speaks another protocol version",
                )
                .with_hint(OTHER_PROTOCOL_HINT)
                .with_details(dict([("this_build", self.this_build())])),
            ));
        }
        Ok(())
    }

    /// `_require_same_build`: a passphrase or an agent's work goes only to
    /// this build's broker.
    pub fn require_same_build(&self, broker: &Value) -> Result<()> {
        let this_build = self.this_build();
        let build = get(broker, "build");
        if build.is_some_and(|build| py_eq(build, &this_build)) {
            return Ok(());
        }
        Err(Error::App(
            app(
                "operator_broker_build_mismatch",
                format!(
                    "the operator broker is {}, not this build",
                    describe_build(build)
                ),
            )
            .with_hint(OTHER_BUILD_HINT)
            .with_details(dict([
                ("broker_build", build.cloned().unwrap_or(Value::Null)),
                ("this_build", this_build),
            ])),
        ))
    }

    /// `_require_caller_context_support`: never lose the egress kill switch
    /// to an older broker that ignores the caller context.
    fn require_caller_context_support(&self, broker: &Value, strict: bool) -> Result<()> {
        if is_true(get(broker, "caller_context")) {
            return Ok(());
        }
        let hint = "An older operator broker is still running. Ask the user to stop it \
(log out, or end the `kassiber.operator.server` process) and unlock again.";
        if strict || self.caller_disables_egress() {
            return Err(Error::App(
                app(
                    "operator_broker_outdated",
                    "the running operator broker cannot honor KASSIBER_NO_EGRESS",
                )
                .with_hint(hint),
            ));
        }
        self.host.warn(&format!(
            "warning: the running operator broker predates caller working directories; pass absolute paths. {hint}\n"
        ));
        Ok(())
    }

    /// `_caller_working_directory`: an unreadable directory fails rather
    /// than letting the broker's directory stand in.
    fn caller_working_directory(&self) -> Result<PyStr> {
        self.host.current_dir().map_err(|_| {
            Error::App(
                app(
                    "operator_working_directory_unavailable",
                    "the current working directory is no longer available",
                )
                .with_hint("Run the command from an existing directory or pass absolute paths."),
            )
        })
    }

    /// `_caller_disables_egress`.
    pub fn caller_disables_egress(&self) -> bool {
        let value = self.host.env_var("KASSIBER_NO_EGRESS").unwrap_or_default();
        let value = kassiber_pytext::lower(kassiber_pytext::strip(&value));
        matches!(value.as_str(), "1" | "true" | "yes" | "on")
    }
}

/// A challenge carried as a Python `str`; lone surrogates cannot be ASCII.
fn lossless(text: &PyStr) -> Result<String> {
    text.as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::Value("'ascii' codec can't encode the challenge".into()))
}

/// `continuation.get("continue") != "secret" or not isinstance(challenge, str)`.
fn secret_challenge(continuation: &Value, message: &str) -> Result<PyStr> {
    let challenge = as_str(get(continuation, "challenge"));
    match challenge {
        Some(challenge)
            if as_str(get(continuation, "continue")).is_some_and(|c| c.is("secret")) =>
        {
            Ok(challenge.clone())
        }
        _ => Err(protocol(message)),
    }
}

/// `channel.send_secret(challenge, secret)` for a challenge read from JSON.
fn send_secret(channel: &mut dyn FrameChannel, challenge: &PyStr, secret: &[u8]) -> Result<()> {
    channel.send_secret(&lossless(challenge)?, secret)
}

/// `_broker_native_auth_identity`.
fn broker_native_auth_identity(broker: &Value) -> Result<PyStr> {
    match as_str(get(broker, "native_auth_identity")) {
        Some(identity) if is_true(get(broker, "native_auth_available")) => Ok(identity.clone()),
        _ => Err(Error::App(
            app(
                "native_auth_unavailable",
                "the operator broker cannot use this signed native-auth helper",
            )
            .with_hint(NATIVE_AUTH_HINT),
        )),
    }
}

/// `_no_lease_error`.
pub fn no_lease_error() -> AppError {
    app(
        "interaction_required",
        "this project has no active operator lease",
    )
    .with_hint(
        "Ask the user to run `kassiber operator unlock` in their own terminal, then retry. \
Never ask for the passphrase itself.",
    )
    .with_details(dict([
        ("reason", s("operator_lease_required")),
        ("unlock_mode", s("brokered")),
    ]))
    .retryable(true)
}

/// `_receive`: a reply with `ok: false` becomes the broker's `AppError`.
pub fn receive(channel: &mut dyn FrameChannel) -> Result<Value> {
    let response = channel.receive_json()?;
    if is_false(get(&response, "ok")) {
        let Some(error @ Value::Dict(_)) = get(&response, "error") else {
            return Err(protocol("invalid broker error response"));
        };
        let message = match get(error, "message") {
            Some(message) if truthy(message) => py_str_value(message),
            _ => PyStr::from("operator broker error"),
        };
        let code = match get(error, "code") {
            Some(code) if truthy(code) => kassiber_envelope::py_str(code),
            _ => "operator_error".to_owned(),
        };
        let mut failure = AppError::new(code, String::new())
            .with_python_message(message)
            .retryable(get(error, "retryable").is_some_and(truthy));
        // `details=error.get("details")`: None stays None.
        match get(error, "details") {
            None | Some(Value::Null) => {}
            Some(details) => failure = failure.with_details(details.clone()),
        }
        if let Some(Value::Str(hint)) = get(error, "hint") {
            failure.hint = Some(
                hint.code_points()
                    .map(|p| char::from_u32(p).map_or_else(|| format!("\\u{p:04x}"), String::from))
                    .collect::<String>()
                    .into_boxed_str(),
            );
        }
        return Err(Error::App(failure));
    }
    Ok(response)
}

/// `_data_from_response`.
pub fn data_from_response(response: &Value) -> Result<Value> {
    match get(response, "data") {
        Some(data @ Value::Dict(_)) => Ok(data.clone()),
        _ => Err(protocol("invalid broker response")),
    }
}

fn receive_data(channel: &mut dyn FrameChannel) -> Result<Value> {
    data_from_response(&receive(channel)?)
}

struct SessionState {
    closed: bool,
}

/// Keeps one agent session open; the broker ends it when the heartbeats
/// stop (its 30 s frame deadline) or the connection closes.
pub struct AgentSessionHandle {
    pub session_id: PyStr,
    /// Presented with every call of this process; never logged or shown.
    pub token: PyStr,
    state: Arc<(Mutex<SessionState>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AgentSessionHandle {
    fn start(channel: Box<dyn FrameChannel>, session_id: PyStr, token: PyStr) -> Self {
        let state = Arc::new((Mutex::new(SessionState { closed: false }), Condvar::new()));
        let shared = Arc::clone(&state);
        let thread = std::thread::Builder::new()
            .name("kassiber-agent-session".into())
            .spawn(move || beat(channel, shared))
            .ok();
        AgentSessionHandle {
            session_id,
            token,
            state,
            thread,
        }
    }

    pub fn alive(&self) -> bool {
        !self
            .state
            .0
            .lock()
            .map(|state| state.closed)
            .unwrap_or(true)
    }

    /// Stop the heartbeats and close the connection.
    pub fn close(&mut self) {
        {
            let (lock, wake) = &*self.state;
            if let Ok(mut state) = lock.lock() {
                state.closed = true;
            }
            wake.notify_all();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for AgentSessionHandle {
    fn drop(&mut self) {
        self.close();
    }
}

fn beat(mut channel: Box<dyn FrameChannel>, state: Arc<(Mutex<SessionState>, Condvar)>) {
    let (lock, wake) = &*state;
    loop {
        let Ok(guard) = lock.lock() else { break };
        let Ok((guard, timeout)) =
            wake.wait_timeout_while(guard, AGENT_SESSION_HEARTBEAT, |s| !s.closed)
        else {
            break;
        };
        if guard.closed || !timeout.timed_out() {
            break;
        }
        drop(guard);
        if channel
            .send_json(&request("agent_session_heartbeat", vec![]))
            .is_err()
        {
            break;
        }
    }
    if let Ok(mut guard) = lock.lock() {
        guard.closed = true;
    }
    channel.close();
}

/// The real process: the runtime endpoint, a detached broker launch from
/// this executable, OS entropy and the environment.
pub struct SystemHost {
    pub runtime: Runtime,
    pub build: Value,
    pub executable: std::path::PathBuf,
}

impl SystemHost {
    pub fn new(build: Value) -> Self {
        SystemHost {
            runtime: Runtime::from_process(),
            build,
            executable: std::env::current_exe().unwrap_or_default(),
        }
    }
}

impl Host for SystemHost {
    fn connect(
        &self,
        timeout: Duration,
        io_timeout: Option<Duration>,
    ) -> Result<Box<dyn FrameChannel>> {
        crate::runtime::connect(&self.runtime, timeout, io_timeout)
    }

    fn launch_broker(&self) -> Result<()> {
        // Never the first client's directory: start beside the executable.
        let directory = self
            .executable
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        let mut command = kassiber_ipc::process::ChildCommand::new(&self.executable);
        command
            .arg("--operator-broker-server")
            .env_remove("KASSIBER_OPERATOR_DIRECT")
            .current_dir(directory)
            .stdin(kassiber_ipc::process::Stdio::Null)
            .stdout(kassiber_ipc::process::Stdio::Null)
            .stderr(kassiber_ipc::process::Stdio::Null)
            .detach(true);
        command.spawn()?;
        Ok(())
    }

    fn startup_timeout(&self) -> Duration {
        if !self.runtime.frozen {
            SOURCE_BROKER_STARTUP_TIMEOUT
        } else if cfg!(windows) {
            WINDOWS_FROZEN_BROKER_STARTUP_TIMEOUT
        } else {
            FROZEN_BROKER_STARTUP_TIMEOUT
        }
    }

    fn build_identity(&self) -> Value {
        self.build.clone()
    }

    fn token_hex(&self, bytes: usize) -> Result<String> {
        crate::protocol::token_hex(bytes)
    }

    fn current_dir(&self) -> std::io::Result<PyStr> {
        std::env::current_dir().map(|path| crate::arguments::os_text(path.as_os_str()))
    }

    fn env_var(&self, key: &str) -> Option<String> {
        std::env::var_os(key).map(|value| value.to_string_lossy().into_owned())
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn warn(&self, text: &str) {
        use std::io::Write;
        let _ = std::io::stderr().write_all(text.as_bytes());
    }
}

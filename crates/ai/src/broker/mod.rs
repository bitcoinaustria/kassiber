//! The provider broker client (`BrokerAIClient` in
//! `kassiber/ai/broker_client.py`): the CLI agent runtimes (Claude, Codex,
//! OpenCode, Copilot) behind the provider broker, a subprocess this binary
//! serves as `kassiber --provider-broker` (plan 23).
//!
//! Each broker process takes one JSONL request on its first stdin line and
//! answers with JSONL events. A status or model probe reads to EOF under
//! one deadline; a chat streams `delta`, `status`, `tool_call` and `done`
//! events, and after a tool call the process stays alive for the
//! `tool_results` reply. Every launch goes through a `kassiber_net` process
//! route, so the online gate and the attempt observer see it; every leg
//! rechecks the gate, as Python calls `require_online` on each. A leg has
//! its own watchdog, which kills the process group; a cancel sends SIGTERM,
//! and a finished process gets SIGTERM, then SIGKILL after two seconds.
//!
//! Broker output Python cannot decode (not UTF-8, a line over 16 MiB, a
//! probe over 64 MiB, an integer over 4,300 digits, nesting past 1,000)
//! ends the request as `ai_unavailable` (a reviewed difference: Python
//! reads without a bound and lets the decode error escape).

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use kassiber_envelope::{dict, set, truthy, AppError};
use kassiber_net::{
    BrokerError, BrokerOutput, BrokerProcess, BrokerProgram, BrokerSignal, Consent, Gate, Request,
};
use kassiber_pyjson::{PyStr, Value};

use crate::providers::{cli_provider_for_locator, CLI_PROVIDERS};
use crate::pyval::{entries, field, s};
use crate::transport::errors::{offline_error, transport_error};
use crate::transport::{
    ChatDelta, RequestContext, MAX_RESPONSE_BYTES, MAX_SSE_LINE_BYTES, PROVIDER_STATUS_NOTICES,
};
use crate::{get_tool, KindClass};

/// `RUNTIME_STATUS_TIMEOUT_SECONDS`: one status request, however many
/// runtimes it covers (the broker probes them in parallel).
pub const RUNTIME_STATUS_TIMEOUT: Duration = Duration::from_secs(35);
/// The cap on a model listing's timeout.
const MODELS_TIMEOUT_CAP: Duration = Duration::from_secs(35);

/// How a client finds the broker program, afresh for every request.
pub type Locate = Arc<dyn Fn() -> BrokerProgram + Send + Sync>;

/// `KASSIBER_AI_PROVIDER_BROKER` (an executable run without arguments), else
/// this executable with `--provider-broker`.
pub fn default_locate() -> Locate {
    Arc::new(|| BrokerProgram::locate(kassiber_net::running_executable()))
}

/// `_broker_unavailable()`.
pub fn broker_unavailable() -> AppError {
    AppError::new(
        "ai_unavailable",
        "The local AI provider broker is unavailable",
    )
    .with_hint("Reinstall Kassiber, or use an HTTP/Ollama/oMLX provider.")
    .retryable(true)
}

fn no_result() -> AppError {
    AppError::new(
        "ai_unavailable",
        "The local AI provider broker returned no result",
    )
    .retryable(true)
}

fn invalid_output() -> AppError {
    AppError::new(
        "ai_unavailable",
        "The local AI provider broker returned invalid output",
    )
    .retryable(true)
}

fn request_invalid(message: &str) -> AppError {
    AppError::new("ai_request_invalid", message)
}

fn cancelled() -> AppError {
    crate::transport::errors::cancelled()
}

/// `str(value)`, keeping a string's code points.
fn py_str_value(value: &Value) -> Value {
    match value {
        Value::Str(_) => value.clone(),
        other => s(kassiber_envelope::py_str(other)),
    }
}

/// `_broker_event_error(event)`.
fn broker_event_error(event: &Value) -> AppError {
    let code = field(event, "code");
    if code.is("authentication_required") {
        let hint = field(event, "hint");
        return AppError::new(
            "ai_auth_failed",
            "The selected CLI provider requires authentication",
        )
        .with_hint(if truthy(hint) {
            kassiber_envelope::py_str(hint)
        } else {
            "Run the provider's normal login command outside Kassiber.".into()
        });
    }
    if code.is("missing_executable") {
        return AppError::new(
            "ai_unavailable",
            "The selected CLI provider is not installed",
        )
        .with_hint("Install the provider CLI and authenticate it outside Kassiber.")
        .retryable(true);
    }
    let message = field(event, "message");
    let error = match py_str_value(message) {
        Value::Str(text) if truthy(message) => {
            AppError::new("ai_request_invalid", "").with_python_message(text)
        }
        _ => request_invalid("The CLI provider request failed."),
    };
    error.with_hint("Check the provider status in the model picker.")
}

trait Is {
    fn is(&self, text: &str) -> bool;
}

impl Is for Value {
    fn is(&self, text: &str) -> bool {
        matches!(self, Value::Str(value) if value.is(text))
    }
}

/// A gate refusal, as `require_online` raises it.
fn gate_error(error: kassiber_net::Error) -> AppError {
    match error {
        kassiber_net::Error::Offline | kassiber_net::Error::OfflineUnavailable => offline_error(),
        // The broker holds no provider credential to withhold.
        other => transport_error(other, &kassiber_credentials::Forms::default()),
    }
}

fn spawn_error(error: BrokerError) -> AppError {
    match error {
        BrokerError::Gate(error) => gate_error(error),
        BrokerError::Missing | BrokerError::Io => broker_unavailable(),
    }
}

/// One broker route, authorized for this request: the online gate runs here.
fn authorize(gate: &Gate, program: BrokerProgram) -> Result<kassiber_net::Route, AppError> {
    let request = Request::ai_broker(program);
    let consent = Consent::explicit_trigger(&request);
    gate.authorize(request, Some(consent)).map_err(gate_error)
}

/// `json.loads(line)` for one event: `None` for what Python skips (not
/// JSON, or not an object).
fn event(line: &str) -> Result<Option<Value>, AppError> {
    match kassiber_pytext::loads(line) {
        Ok(value @ Value::Dict(_)) => Ok(Some(value)),
        Ok(_) => Ok(None),
        Err(error) if error.is_decode_error() => Ok(None),
        Err(_) => Err(invalid_output()),
    }
}

/// An unexpected pipe failure, as Python's daemon reports the escaped
/// `OSError` (`[Errno 32] Broken pipe`).
fn os_error(error: &std::io::Error) -> AppError {
    let text = error.to_string();
    let message = match error.raw_os_error() {
        Some(code) => {
            let reason = text
                .strip_suffix(&format!(" (os error {code})"))
                .unwrap_or(&text);
            format!("[Errno {code}] {reason}")
        }
        None => text,
    };
    AppError::new("internal_error", message)
}

/// Which runtimes a status request covers.
pub enum Scope<'a> {
    All,
    One(&'a str),
    Many(&'a [&'a str]),
}

/// `BrokerAIClient.runtime_status(...)`: a per-runtime probe starts only that
/// runtime's CLI; an empty list starts nothing.
pub fn runtime_status(gate: &Gate, locate: &Locate, scope: Scope<'_>) -> Result<Value, AppError> {
    let mut request = dict([("command", s("status"))]);
    let names: Option<Vec<&str>> = match scope {
        Scope::All => None,
        Scope::One(name) => {
            set(&mut request, "provider", s(name));
            Some(vec![name])
        }
        Scope::Many(names) => {
            let mut unique: Vec<&str> = Vec::new();
            for name in names {
                if !unique.contains(name) {
                    unique.push(name);
                }
            }
            set(
                &mut request,
                "providers",
                Value::List(unique.iter().map(s).collect()),
            );
            Some(unique)
        }
    };
    for name in names.iter().flatten() {
        if !CLI_PROVIDERS.iter().any(|(_, runtime)| runtime == name) {
            return Err(AppError::new(
                "validation",
                format!("Unsupported CLI AI provider '{name}'"),
            )
            .retryable(false));
        }
    }
    if names.as_ref().is_some_and(Vec::is_empty) {
        return Ok(Value::List(Vec::new()));
    }
    let Value::List(rows) = single_result(gate, locate, &request, RUNTIME_STATUS_TIMEOUT)? else {
        return Ok(Value::List(Vec::new()));
    };
    Ok(Value::List(match names {
        // Trust the scope asked for, not the broker's echo of it.
        Some(names) => rows
            .into_iter()
            .filter(|row| {
                matches!(row, Value::Dict(_))
                    && names.iter().any(|name| field(row, "provider").is(name))
            })
            .collect(),
        None => rows,
    }))
}

/// `BrokerAIClient._single_result(request, timeout=...)`: one request,
/// stdout read to EOF and the process reaped under one deadline. Status and
/// model discovery start the provider CLIs too, which can reach their
/// vendors (sign-in refresh, model catalogs).
pub fn single_result(
    gate: &Gate,
    locate: &Locate,
    request: &Value,
    timeout: Duration,
) -> Result<Value, AppError> {
    let program = locate();
    let route = authorize(gate, program)?;
    let mut process = route.spawn_broker().map_err(spawn_error)?;
    let line = dumps(request) + "\n";
    // A broker that exits before reading is answered by its output alone.
    let _ = process.write_line(line.as_bytes());
    process.close_stdin();
    let mut output = process.take_output().ok_or_else(broker_unavailable)?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(output.read_to_end(MAX_RESPONSE_BYTES));
    });
    let deadline = Instant::now() + timeout;
    let left = |deadline: Instant| deadline.saturating_duration_since(Instant::now());
    let signal = process.signal();
    let read = match receiver.recv_timeout(left(deadline)) {
        Ok(read) if signal.wait_timeout(left(deadline)) => Some(read),
        Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => None,
        Err(mpsc::RecvTimeoutError::Disconnected) => Some(Err(std::io::ErrorKind::Other.into())),
    };
    let Some(read) = read else {
        // A probe that overruns has provider CLIs of its own running; ending
        // only the broker would leave them behind. SIGTERM first lets the
        // broker stop them and clean up; the group is killed if it cannot.
        signal.terminate();
        let grace = Instant::now() + kassiber_net::BROKER_GRACE;
        let ended = matches!(
            receiver.recv_timeout(left(grace)),
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected)
        ) && signal.wait_timeout(left(grace));
        if !ended {
            signal.kill();
            signal.wait();
        }
        process.finish();
        return Err(broker_unavailable());
    };
    process.finish();
    let bytes = read.map_err(|_| invalid_output())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid_output())?;
    for line in kassiber_net::splitlines(&universal_newlines(text)) {
        let Some(event) = event(line)? else {
            continue;
        };
        let kind = field(&event, "type");
        if kind.is("error") {
            return Err(broker_event_error(&event));
        }
        if kind.is("result") {
            return Ok(field(&event, "data").clone());
        }
    }
    Err(no_result())
}

/// Text-mode reading: `\r\n` and a bare `\r` become `\n`.
fn universal_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// `json.dumps(value)` with Python's defaults.
fn dumps(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &Default::default()).unwrap_or_default()
}

/// `_content_text(content)`.
fn content_text(content: &Value) -> Value {
    match content {
        Value::Str(_) => content.clone(),
        Value::List(parts) => {
            let mut joined: Vec<u32> = Vec::new();
            let mut first = true;
            for part in parts {
                let text = field(part, "text");
                let typed = ["input_text", "output_text", "text"]
                    .iter()
                    .any(|kind| field(part, "type").is(kind));
                let Value::Str(text) = text else { continue };
                if !matches!(part, Value::Dict(_)) || !typed {
                    continue;
                }
                if !first {
                    joined.push(u32::from('\n'));
                }
                first = false;
                joined.extend(text.code_points());
            }
            PyStr::from_code_points(joined).map_or_else(|_| s(""), Value::Str)
        }
        _ => s(""),
    }
}

/// `_content_images(content)`: a message's inline images as the broker
/// carries them, media type and base64 (#693). Only data URLs: the daemon
/// inlines every image it shows, and a remote URL would let the provider
/// fetch something the user never attached.
fn content_images(content: &Value) -> Vec<Value> {
    let Value::List(parts) = content else {
        return Vec::new();
    };
    let mut images = Vec::new();
    for part in parts {
        if !matches!(part, Value::Dict(_))
            || !(field(part, "type").is("input_image") || field(part, "type").is("image_url"))
        {
            continue;
        }
        // Responses parts carry the URL itself; Chat parts nest it.
        let mut url = field(part, "image_url");
        if matches!(url, Value::Dict(_)) {
            url = field(url, "url");
        }
        let Value::Str(url) = url else { continue };
        let url = crate::pyval::lossy(url);
        let Some(rest) = url.strip_prefix("data:") else {
            continue;
        };
        let (header, data) = rest.split_once(',').unwrap_or((rest, ""));
        let (media_type, encoding) = header.split_once(';').unwrap_or((header, ""));
        if encoding == "base64" && media_type.starts_with("image/") && !data.is_empty() {
            images.push(dict([("media_type", s(media_type)), ("data", s(data))]));
        }
    }
    images
}

/// One broker message: its text, and a user message's inline images.
fn broker_message(role: Value, content: Value, images: Vec<Value>) -> Value {
    let mut message = vec![("role", role), ("content", content)];
    if !images.is_empty() {
        message.push(("images", Value::List(images)));
    }
    dict(message)
}

/// `_broker_messages_for_context(context)`.
fn messages_for_context(context: &RequestContext) -> Vec<Value> {
    let mut messages = Vec::new();
    for item in &context.input_items {
        if !field(item, "type").is("message") {
            continue;
        }
        let role = field(item, "role");
        let role = if truthy(role) {
            py_str_value(role)
        } else {
            s("user")
        };
        let content = content_text(field(item, "content"));
        let images = if role.is("user") {
            content_images(field(item, "content"))
        } else {
            Vec::new()
        };
        if truthy(&content) || !images.is_empty() {
            messages.push(broker_message(role, content, images));
        }
    }
    messages
}

/// `_broker_tool_definitions(tools)`: name, description, parameters and the
/// catalog's read-only and destructive hints.
fn tool_definitions(tools: &[Value]) -> Vec<Value> {
    let mut definitions = Vec::new();
    for tool in tools {
        let mut function = field(tool, "function");
        if !matches!(function, Value::Dict(_)) && matches!(tool, Value::Dict(_)) {
            function = tool;
        }
        let Value::Str(name) = field(function, "name") else {
            continue;
        };
        if !matches!(function, Value::Dict(_)) {
            continue;
        }
        let entry = name.as_str().and_then(get_tool);
        let description = field(function, "description");
        let parameters = field(function, "parameters");
        definitions.push(dict([
            ("name", Value::Str(name.clone())),
            (
                "description",
                if truthy(description) {
                    description.clone()
                } else {
                    s("")
                },
            ),
            (
                "parameters",
                if truthy(parameters) {
                    parameters.clone()
                } else {
                    dict([("type", s("object"))])
                },
            ),
            (
                "read_only",
                Value::Bool(entry.is_some_and(|e| e.kind_class == KindClass::ReadOnly)),
            ),
            (
                "destructive",
                Value::Bool(entry.is_some_and(|e| e.kind_class == KindClass::Mutating)),
            ),
        ]));
    }
    definitions
}

/// One chat leg's input, as `stream_chat(messages=..., context=...)` takes it.
pub struct BrokerChat<'a> {
    pub messages: Option<&'a [Value]>,
    pub context: Option<&'a RequestContext>,
    pub model: &'a str,
    /// A dict, or `Null` for none.
    pub options: &'a Value,
    pub tools: &'a [Value],
}

struct Shared {
    cancelled: AtomicBool,
    signal: Mutex<Option<BrokerSignal>>,
}

impl Shared {
    fn signal(&self) -> std::sync::MutexGuard<'_, Option<BrokerSignal>> {
        self.signal
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

/// Cancels a [`BrokerClient`] from another thread. Latched for the client's
/// lifetime: a cancel during lookup or launch still stops the turn.
#[derive(Clone)]
pub struct BrokerCanceller(Arc<Shared>);

impl BrokerCanceller {
    /// `cancel()`: SIGTERM to a running broker's group.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        let signal = self.0.signal().clone();
        if let Some(signal) = signal {
            if signal.running() {
                signal.terminate();
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }
}

/// `BrokerAIClient`: provider-neutral over the JSONL broker.
pub struct BrokerClient {
    pub locator: String,
    pub provider: &'static str,
    pub timeout: Duration,
    /// The broker's session cursor from the last `done`, a string or `Null`.
    pub last_provider_session_id: Value,
    gate: Gate,
    locate: Locate,
    shared: Arc<Shared>,
    process: Option<BrokerProcess>,
    pending_call_ids: BTreeSet<PyStr>,
    hook: Option<StageHook>,
}

/// Called at `resolution`, `spawn` and `serialization`, for tests that
/// cancel at each stage.
pub type StageHook = Box<dyn Fn(&str, &BrokerCanceller) + Send>;

impl BrokerClient {
    pub fn new(locator: &str, timeout: Duration, gate: Gate) -> Result<Self, AppError> {
        let normalized = kassiber_pytext::lower(kassiber_pytext::strip(locator));
        let Some(provider) = cli_provider_for_locator(&s(&normalized)) else {
            return Err(AppError::new(
                "validation",
                format!("Unsupported CLI AI provider locator '{locator}'"),
            ));
        };
        Ok(Self {
            locator: normalized,
            provider,
            timeout,
            last_provider_session_id: Value::Null,
            gate,
            locate: default_locate(),
            shared: Arc::new(Shared {
                cancelled: AtomicBool::new(false),
                signal: Mutex::new(None),
            }),
            process: None,
            pending_call_ids: BTreeSet::new(),
            hook: None,
        })
    }

    /// Find the broker program some other way (tests: a fake broker).
    pub fn with_locate(mut self, locate: Locate) -> Self {
        self.locate = locate;
        self
    }

    pub fn canceller(&self) -> BrokerCanceller {
        BrokerCanceller(Arc::clone(&self.shared))
    }

    /// Whether a broker is alive and waiting for tool results.
    pub fn pending_call_ids(&self) -> Vec<PyStr> {
        self.pending_call_ids.iter().cloned().collect()
    }

    /// Whether a broker process is registered.
    pub fn has_process(&self) -> bool {
        self.process.is_some()
    }

    #[doc(hidden)]
    pub fn with_stage_hook(mut self, hook: StageHook) -> Self {
        self.hook = Some(hook);
        self
    }

    fn stage(&self, name: &str) {
        if let Some(hook) = &self.hook {
            hook(name, &self.canceller());
        }
    }

    /// `list_models()`: the broker's `models` command for this runtime.
    pub fn list_models(&self) -> Result<Vec<Value>, AppError> {
        let request = dict([("command", s("models")), ("provider", s(self.provider))]);
        match single_result(
            &self.gate,
            &self.locate,
            &request,
            self.timeout.min(MODELS_TIMEOUT_CAP),
        )? {
            Value::List(models) => Ok(models),
            _ => Ok(Vec::new()),
        }
    }

    fn check_cancelled(&self) -> Result<(), AppError> {
        if self.shared.cancelled.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        Ok(())
    }

    /// `_finish_process(process)`.
    fn finish(&mut self) {
        self.pending_call_ids.clear();
        *self.shared.signal() = None;
        if let Some(process) = self.process.take() {
            process.finish();
        }
    }

    /// `stream_chat(...)`: the answer as [`ChatDelta`]s handed to `emit`.
    /// A tool call ends the leg with the broker kept alive; the next leg
    /// sends the results to it.
    pub fn stream_chat(
        &mut self,
        chat: &BrokerChat<'_>,
        emit: &mut dyn FnMut(ChatDelta) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        self.check_cancelled()?;
        let option = |key: &str| kassiber_envelope::get(chat.options, key);
        let flag = |key: &str| option(key).cloned().unwrap_or(Value::Bool(false));
        let resumes = option("provider_session_id").is_some_and(truthy);
        let sensitive = match flag("sensitive_context") {
            Value::Bool(value) if !(value && (!chat.tools.is_empty() || resumes)) => value,
            _ => {
                return Err(request_invalid(
                    "Sensitive context must be stateless and tool-free",
                ))
            }
        };
        let Value::Bool(fast_mode) = flag("fast_mode") else {
            return Err(request_invalid("Fast mode must be true or false"));
        };
        // Incognito: the provider is asked not to save its session, and no
        // session cursor goes in or comes back. Tools stay available.
        let stateless = match flag("stateless_session") {
            Value::Bool(value) if !(value && resumes) => value,
            _ => return Err(request_invalid(
                "A stateless session must be true or false and cannot resume a provider session",
            )),
        };
        if stateless {
            self.last_provider_session_id = Value::Null;
        }
        if sensitive {
            if cfg!(windows) {
                return Err(AppError::new(
                    "ai_sensitive_provider_unavailable",
                    "Private CLI assistance requires verified process-tree cancellation; use a configured HTTP provider on this platform",
                ));
            }
            self.last_provider_session_id = Value::Null;
            if !self.pending_call_ids.is_empty() {
                return Err(request_invalid(
                    "Sensitive context cannot continue an active tool session",
                ));
            }
        }
        // The broker runs remote CLI providers (Claude, Codex, OpenCode, ACP agents).
        let program = (self.locate)();
        self.stage("resolution");
        let route = authorize(&self.gate, program.clone())?;
        self.check_cancelled()?;
        if !program.available() {
            return Err(broker_unavailable());
        }
        let continuing = !self.pending_call_ids.is_empty()
            && self
                .process
                .as_ref()
                .is_some_and(|process| process.signal().running());
        let outbound = if !continuing {
            self.finish();
            self.check_cancelled()?;
            let spawned = route.spawn_broker();
            self.stage("spawn");
            let process = spawned.map_err(spawn_error)?;
            *self.shared.signal() = Some(process.signal());
            self.process = Some(process);
            if self.shared.cancelled.load(Ordering::SeqCst) {
                self.finish();
                return Err(cancelled());
            }
            self.chat_request(chat, stateless, fast_mode)
        } else {
            drop(route);
            let items: &[Value] = chat.context.map_or(&[], |c| &c.input_items);
            let mut results = Vec::new();
            let mut answered = BTreeSet::new();
            for item in items {
                let Value::Str(call_id) = field(item, "call_id") else {
                    continue;
                };
                if !field(item, "type").is("function_call_output")
                    || !self.pending_call_ids.contains(call_id)
                {
                    continue;
                }
                let output = field(item, "output");
                results.push(dict([
                    ("call_id", Value::Str(call_id.clone())),
                    (
                        "output",
                        if truthy(output) {
                            py_str_value(output)
                        } else {
                            s("")
                        },
                    ),
                ]));
                answered.insert(call_id.clone());
            }
            if answered != self.pending_call_ids {
                return Err(request_invalid(
                    "Kassiber did not provide every pending CLI tool result",
                ));
            }
            self.pending_call_ids.clear();
            dict([
                ("command", s("tool_results")),
                ("results", Value::List(results)),
            ])
        };
        let mut keep_alive = false;
        let result = self.exchange(&outbound, sensitive, stateless, &mut keep_alive, emit);
        if !keep_alive {
            self.finish();
        }
        result
    }

    fn chat_request(&self, chat: &BrokerChat<'_>, stateless: bool, fast_mode: bool) -> Value {
        let mut options: Vec<(String, Value)> = entries(chat.options)
            .filter(|(key, _)| {
                matches!(
                    key.as_str(),
                    "reasoning_effort" | "provider_session_id" | "sensitive_context"
                )
            })
            .map(|(key, value)| (key, value.clone()))
            .collect();
        if stateless {
            options.push(("stateless_session".into(), Value::Bool(true)));
        }
        // Only an explicit request travels; the broker applies it solely for
        // models that advertise a fast serving mode.
        if fast_mode {
            options.push(("fast_mode".into(), Value::Bool(true)));
        }
        let messages = match chat.context {
            Some(context) => messages_for_context(context),
            None => chat
                .messages
                .unwrap_or_default()
                .iter()
                .filter(|message| matches!(message, Value::Dict(_)))
                // `_broker_messages_for_chat(messages)`: text and image parts
                // split as for a tool turn, roles kept in place.
                .map(|message| {
                    let role = field(message, "role");
                    let role = if truthy(role) {
                        py_str_value(role)
                    } else {
                        s("user")
                    };
                    let raw = field(message, "content");
                    let content = match raw {
                        Value::List(_) => content_text(raw),
                        raw if truthy(raw) => py_str_value(raw),
                        _ => s(""),
                    };
                    let images = if role.is("user") {
                        content_images(raw)
                    } else {
                        Vec::new()
                    };
                    broker_message(role, content, images)
                })
                .collect(),
        };
        dict([
            ("command", s("chat")),
            ("request_id", s("daemon-chat")),
            ("provider", s(self.provider)),
            ("model", s(chat.model)),
            ("messages", Value::List(messages)),
            (
                "instructions",
                chat.context
                    .and_then(|c| c.instructions.as_deref())
                    .map_or(Value::Null, s),
            ),
            ("tools", Value::List(tool_definitions(chat.tools))),
            ("options", dict(options)),
        ])
    }

    /// One leg: write the request, read events until a terminal one.
    fn exchange(
        &mut self,
        outbound: &Value,
        sensitive: bool,
        stateless: bool,
        keep_alive: &mut bool,
        emit: &mut dyn FnMut(ChatDelta) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let provider = self.provider;
        let raw = || dict([("provider", s(provider))]);
        let signal = match &self.process {
            Some(process) => process.signal(),
            None => return Err(broker_unavailable()),
        };
        // Reading the pipe blocks with no deadline of its own, so a provider
        // that stops producing output would pin the turn forever. The
        // watchdog kills the whole group instead, which closes stdout.
        let watchdog = Watchdog::start(signal.clone(), self.timeout.max(Duration::from_secs(1)));
        let encoded = dumps(outbound) + "\n";
        self.stage("serialization");
        self.check_cancelled()?;
        let process = self.process.as_mut().ok_or_else(broker_unavailable)?;
        if let Err(error) = process.write_line(encoded.as_bytes()) {
            self.check_cancelled()?;
            // A gate refusal on the recheck before writing.
            if error
                .get_ref()
                .is_some_and(|inner| inner.is::<kassiber_net::Error>())
            {
                return Err(gate_error(kassiber_net::Error::from(error)));
            }
            return Err(os_error(&error));
        }
        self.check_cancelled()?;
        let mut buffered_content: Vec<u32> = Vec::new();
        loop {
            let process = self.process.as_mut().ok_or_else(broker_unavailable)?;
            let output: &mut BrokerOutput = process.output().ok_or_else(broker_unavailable)?;
            let line = match output.read_line(MAX_SSE_LINE_BYTES) {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(error) => {
                    self.check_cancelled()?;
                    if error.kind() == std::io::ErrorKind::InvalidData {
                        return Err(invalid_output());
                    }
                    return Err(os_error(&error));
                }
            };
            self.check_cancelled()?;
            let line = std::str::from_utf8(&line).map_err(|_| invalid_output())?;
            let Some(event) = event(line)? else {
                continue;
            };
            let kind = field(&event, "type");
            if kind.is("error") {
                if sensitive {
                    return Err(AppError::new(
                        "ai_unavailable",
                        "The CLI provider could not complete the private selected-context request",
                    ));
                }
                return Err(broker_event_error(&event));
            }
            if kind.is("delta") {
                let mut delta = dict::<&str>([]);
                for key in ["content", "reasoning"] {
                    if let value @ Value::Str(text) = field(&event, key) {
                        if text.code_points().next().is_some() {
                            set(&mut delta, key, value.clone());
                        }
                    }
                }
                if let Value::Str(content) = field(&delta, "content") {
                    buffered_content.extend(content.code_points());
                }
                if truthy(&delta) {
                    emit(ChatDelta {
                        delta,
                        finish_reason: None,
                        raw: raw(),
                        response_output: None,
                        status_phase: None,
                    })?;
                }
            } else if kind.is("status") {
                if let Value::Str(phase) = field(&event, "phase") {
                    if let Some((known, _)) = PROVIDER_STATUS_NOTICES
                        .iter()
                        .find(|(name, _)| phase.is(name))
                    {
                        emit(ChatDelta {
                            delta: dict::<&str>([]),
                            finish_reason: None,
                            raw: raw(),
                            response_output: None,
                            status_phase: Some((*known).to_owned()),
                        })?;
                    }
                }
            } else if kind.is("tool_call") {
                if sensitive {
                    return Err(request_invalid(
                        "Sensitive context cannot execute provider tools",
                    ));
                }
                let (Value::Str(call_id), Value::Str(name), arguments @ Value::Dict(_)) = (
                    field(&event, "call_id"),
                    field(&event, "name"),
                    field(&event, "arguments"),
                ) else {
                    return Err(request_invalid(
                        "The CLI provider returned an invalid tool call",
                    ));
                };
                if call_id.code_points().next().is_none() || name.code_points().next().is_none() {
                    return Err(request_invalid(
                        "The CLI provider returned an invalid tool call",
                    ));
                }
                self.pending_call_ids.insert(call_id.clone());
                let encoded_arguments = kassiber_pyjson::dumps(
                    arguments,
                    &kassiber_pyjson::Options::new().sort_keys(true).compact(),
                )
                .map_err(|_| request_invalid("The CLI provider returned an invalid tool call"))?;
                let mut response_output = Vec::new();
                if !buffered_content.is_empty() {
                    let content = PyStr::from_code_points(buffered_content.iter().copied())
                        .map_or_else(|_| s(""), Value::Str);
                    response_output.push(dict([
                        ("type", s("message")),
                        ("role", s("assistant")),
                        (
                            "content",
                            Value::List(vec![dict([
                                ("type", s("output_text")),
                                ("text", content),
                            ])]),
                        ),
                    ]));
                }
                response_output.push(dict([
                    ("type", s("function_call")),
                    ("call_id", Value::Str(call_id.clone())),
                    ("name", Value::Str(name.clone())),
                    ("arguments", s(&encoded_arguments)),
                ]));
                *keep_alive = true;
                drop(watchdog);
                return emit(ChatDelta {
                    delta: dict([(
                        "tool_calls",
                        Value::List(vec![dict([
                            ("id", Value::Str(call_id.clone())),
                            ("type", s("function")),
                            (
                                "function",
                                dict([
                                    ("name", Value::Str(name.clone())),
                                    ("arguments", s(&encoded_arguments)),
                                ]),
                            ),
                        ])]),
                    )]),
                    finish_reason: Some("tool_calls".into()),
                    raw: raw(),
                    response_output: Some(response_output),
                    status_phase: None,
                });
            } else if kind.is("done") {
                let session_id = field(&event, "provider_session_id");
                self.last_provider_session_id = match session_id {
                    Value::Str(text)
                        if !(sensitive || stateless) && text.code_points().next().is_some() =>
                    {
                        session_id.clone()
                    }
                    _ => Value::Null,
                };
                let finish = field(&event, "finish_reason");
                let finish_reason = if truthy(finish) {
                    kassiber_envelope::py_str(finish)
                } else {
                    "stop".to_owned()
                };
                drop(watchdog);
                return emit(ChatDelta {
                    delta: dict::<&str>([]),
                    finish_reason: Some(finish_reason),
                    raw: raw(),
                    response_output: None,
                    status_phase: None,
                });
            }
        }
        // EOF without a terminal event is a failure, whatever the exit status.
        self.check_cancelled()?;
        signal.wait_timeout(kassiber_net::BROKER_GRACE);
        if watchdog.fired() {
            return Err(
                AppError::new("ai_unavailable", "The CLI provider stopped responding")
                    .retryable(true),
            );
        }
        Err(AppError::new(
            "ai_unavailable",
            "The CLI provider stopped before completing the response",
        )
        .retryable(true))
    }
}

impl Drop for BrokerClient {
    fn drop(&mut self) {
        self.finish();
    }
}

/// One leg's deadline: SIGKILL to the group when it passes.
struct Watchdog {
    stop: Option<mpsc::Sender<()>>,
    fired: Arc<AtomicBool>,
}

impl Watchdog {
    fn start(signal: BrokerSignal, timeout: Duration) -> Self {
        let (stop, wait) = mpsc::channel::<()>();
        let fired = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&fired);
        std::thread::spawn(move || {
            if let Err(mpsc::RecvTimeoutError::Timeout) = wait.recv_timeout(timeout) {
                flag.store(true, Ordering::SeqCst);
                signal.kill();
            }
        });
        Self {
            stop: Some(stop),
            fired,
        }
    }

    fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_inline_image_data_reaches_the_broker() {
        let content = Value::List(vec![
            dict([("type", s("text")), ("text", s("look"))]),
            dict([
                ("type", s("image_url")),
                (
                    "image_url",
                    dict([("url", s("data:image/png;base64,iVBORw0KGgo="))]),
                ),
            ]),
            dict([
                ("type", s("input_image")),
                ("image_url", s("data:image/jpeg;base64,/9j/4AAQ")),
            ]),
            // Neither a remote URL, a non-image nor an empty payload.
            dict([
                ("type", s("image_url")),
                ("image_url", dict([("url", s("https://example.com/a.png"))])),
            ]),
            dict([
                ("type", s("image_url")),
                (
                    "image_url",
                    dict([("url", s("data:text/plain;base64,aGk="))]),
                ),
            ]),
            dict([
                ("type", s("image_url")),
                ("image_url", dict([("url", s("data:image/png;base64,"))])),
            ]),
        ]);
        assert_eq!(
            content_images(&content),
            vec![
                dict([("media_type", s("image/png")), ("data", s("iVBORw0KGgo="))]),
                dict([("media_type", s("image/jpeg")), ("data", s("/9j/4AAQ"))]),
            ]
        );
        assert!(content_images(&s("text only")).is_empty());
    }
}

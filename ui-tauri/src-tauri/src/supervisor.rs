use crate::secret_store::{
    current_ai_provider_secret_store_policy, current_secret_store_platform,
    native_store_id_for_platform, NativeSecretStore, SecretStore, STORE_ID_SQLCIPHER_INLINE,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

// Packaged one-file Python sidecars can take longer than a development checkout
// to start on cold launch, especially before the database unlock screen.
const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(30);
const DAEMON_INVOKE_TIMEOUT: Duration = Duration::from_secs(15);
/// Per-record inactivity timeout for streaming kinds. The recv clock resets
/// every time a delta arrives, so a long-running stream stays alive as long
/// as the daemon keeps producing output within the window.
const DAEMON_STREAM_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(90);
/// How long the daemon must produce *no output at all* before a non-streaming
/// request timeout is treated as a genuinely wedged/dead process worth killing.
///
/// The Python daemon dispatches requests on a single serial loop, so a slow
/// reply to one non-streaming request (e.g. a routine `ui.logs.snapshot` /
/// `ui.overview.snapshot` poll) usually means the daemon is *busy* serving
/// another, often streaming, request — not that it is dead. Killing the whole
/// process in that case was the root of the daemon-supervisor kill loop: the
/// kill marks the shared process broken, the next request respawns it (locked),
/// and any heavy work re-runs. We now only kill when the daemon has been
/// silent across this window — real evidence of a hang rather than a busy loop.
const DAEMON_SILENCE_KILL_TIMEOUT: Duration = Duration::from_secs(90);
const STDERR_TAIL_LIMIT: usize = 16 * 1024;
const LIFECYCLE_RING_CAPACITY: usize = 64;

#[cfg(target_os = "windows")]
fn hide_console_window(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn hide_console_window(_command: &mut Command) {}

type DaemonResponse = Result<Value, SupervisorError>;
type PendingSender = mpsc::Sender<DaemonResponse>;
type PendingMap = Arc<Mutex<HashMap<String, PendingSender>>>;
type SharedStdin = Arc<Mutex<ChildStdin>>;
type SharedSecretStore = Arc<dyn SecretStore>;
type EventSink = Box<dyn Fn(&Value) + Send + Sync>;
/// Sink for unsolicited daemon→UI event records (`event: true`, no
/// `request_id`). Shared between the supervisor and every spawned
/// process's stdout reader so a sink registered after spawn still
/// receives events.
type SharedEventSink = Arc<Mutex<Option<EventSink>>>;

const SECRET_STORE_CONTROL_REQUEST_KIND: &str = "supervisor.ai_secret_store.request";
const SECRET_STORE_CONTROL_RESPONSE_KIND: &str = "supervisor.ai_secret_store.response";

pub struct DaemonSupervisor {
    process: Mutex<Option<Arc<DaemonProcess>>>,
    resource_dir: Option<PathBuf>,
    data_root: Mutex<Option<PathBuf>>,
    next_request_id: AtomicU64,
    secret_store: SharedSecretStore,
    lifecycle: Mutex<VecDeque<LifecycleRecord>>,
    next_lifecycle_id: AtomicU64,
    event_sink: SharedEventSink,
    /// Per-request budget for non-streaming kinds. Exceeding it no longer kills
    /// the daemon outright (see [`DAEMON_SILENCE_KILL_TIMEOUT`]); it fails the
    /// one slow request retryably. A field (not the bare const) so tests can
    /// shrink it without waiting the full budget.
    invoke_timeout: Duration,
    /// Silence window after which a non-streaming timeout escalates to killing
    /// a genuinely wedged daemon. A field so tests can drive both branches.
    silence_kill_timeout: Duration,
}

/// Daemon lifecycle event kept for the diagnostics screen. `detail` and
/// `stderr_tail` are secret-floor redacted at insert so raw daemon stderr
/// never sits in the ring.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleRecord {
    pub id: u64,
    pub ts_ms: u64,
    pub event: &'static str,
    pub detail: String,
    pub stderr_tail: String,
    pub source: &'static str,
}

struct DaemonProcess {
    child: Mutex<Child>,
    stdin: SharedStdin,
    pending: PendingMap,
    stderr_tail: StderrTail,
    broken: Arc<AtomicBool>,
    /// Wall-clock instant of the most recent line read from the daemon's
    /// stdout (any record: response, stream delta, or event). Used to tell a
    /// busy-but-alive daemon apart from a wedged one when a non-streaming
    /// request times out. Updated by the stdout reader; read on timeout.
    last_activity: Arc<Mutex<Instant>>,
}

#[derive(Clone)]
struct StderrTail {
    bytes: Arc<Mutex<Vec<u8>>>,
}

#[derive(Debug, Clone)]
pub struct SupervisorError {
    pub code: &'static str,
    pub message: String,
    pub hint: Option<&'static str>,
    pub details: Option<Value>,
    pub retryable: bool,
}

impl SupervisorError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
            details: None,
            retryable: false,
        }
    }

    fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    fn hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint);
        self
    }

    fn details(mut self, details: Value) -> Self {
        self.details = Some(redact_sensitive_value(details));
        self
    }

    fn with_stderr_tail(mut self, stderr_tail: String) -> Self {
        let stderr_tail = redact_sensitive_text(&stderr_tail);
        if stderr_tail.is_empty() {
            return self;
        }

        let mut map = match self.details.take() {
            Some(Value::Object(map)) => map,
            Some(details) => {
                let mut map = Map::new();
                map.insert("details".to_string(), details);
                map
            }
            None => Map::new(),
        };
        map.insert("stderr_tail".to_string(), Value::String(stderr_tail));
        self.details = Some(Value::Object(map));
        self
    }
}

fn redact_sensitive_value(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_sensitive_text(&text)),
        Value::Array(items) => {
            Value::Array(items.into_iter().map(redact_sensitive_value).collect())
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    if is_sensitive_key(&key) {
                        (key, Value::String("[redacted]".to_string()))
                    } else {
                        (key, redact_sensitive_value(value))
                    }
                })
                .collect(),
        ),
        other => other,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase().replace('-', "_");
    [
        "api_key",
        "auth_header",
        "auth_response",
        "cookie",
        "descriptor",
        "mnemonic",
        "password",
        "passphrase",
        "private",
        "recovery",
        "secret",
        "seed",
        "token",
        "xprv",
    ]
    .iter()
    .any(|part| lowered.contains(part))
}

fn redact_sensitive_text(text: &str) -> String {
    let mut redact_next = false;
    let mut recovery_tail_words = 0usize;
    let mut words = Vec::new();
    for word in text.split_whitespace() {
        if recovery_tail_words > 0 {
            if looks_like_recovery_tail_word(word) {
                words.push("[redacted]".to_string());
                recovery_tail_words -= 1;
                continue;
            }
            recovery_tail_words = 0;
        }
        if redact_next {
            words.push("[redacted]".to_string());
            redact_next = false;
            continue;
        }
        let recovery_assignment = is_recovery_assignment_word(word);
        let (redacted, redact_following) = redact_sensitive_word(word);
        if recovery_assignment {
            // A spaced recovery phrase: its words follow as separate tokens, so
            // arm the recovery tail instead of `redact_next` (which would eat
            // only the first word and then stay stuck on across the phrase).
            recovery_tail_words = 23;
        } else {
            redact_next = redact_following;
        }
        words.push(redacted);
    }
    words.join(" ")
}

fn is_recovery_assignment_word(word: &str) -> bool {
    let lowered = word.to_ascii_lowercase();
    for marker in [
        "mnemonic",
        "recovery_phrase",
        "recovery-phrase",
        "seed",
        "seed_phrase",
        "seed-phrase",
        "seed_words",
        "seed-words",
    ] {
        if let Some(index) = lowered.find(marker) {
            let after = &word[index + marker.len()..];
            if after.starts_with('=') || after.starts_with(':') {
                return true;
            }
        }
    }
    false
}

fn looks_like_recovery_tail_word(word: &str) -> bool {
    let trimmed = word.trim_matches(|c: char| !c.is_ascii_alphabetic());
    let len = trimmed.len();
    (2..=12).contains(&len) && trimmed.chars().all(|c| c.is_ascii_alphabetic())
}

/// Redacts a single whitespace-delimited token, returning the (possibly
/// rewritten) token and whether the FOLLOWING token must also be redacted.
/// The follow flag is true for `Bearer <token>` and for an assignment whose
/// value is quoted *after* the separator (`"api_key": "secret"`), where
/// `split_whitespace` puts the value in the next token.
fn redact_sensitive_word(word: &str) -> (String, bool) {
    let lowered = word.to_ascii_lowercase();
    if lowered.starts_with("sk-") {
        return ("[redacted]".to_string(), false);
    }
    if contains_extended_key_or_descriptor(&lowered) {
        return ("[redacted]".to_string(), false);
    }
    if lowered.starts_with("bearer") {
        return ("Bearer".to_string(), true);
    }
    for marker in [
        "api_key",
        "api-key",
        "auth_header",
        "auth-header",
        "cookie",
        "descriptor",
        "mnemonic",
        "password",
        "passphrase",
        "recovery_phrase",
        "recovery-phrase",
        "secret",
        "seed",
        "seed_phrase",
        "seed-phrase",
        "seed_words",
        "seed-words",
        "token",
        "xprv",
    ] {
        if let Some(index) = lowered.find(marker) {
            let after = &word[index + marker.len()..];
            // Allow one closing quote between the key and the separator so JSON
            // and Python-dict shapes match too: `api_key=`, `"api_key":`,
            // `'api_key':`. `split_whitespace` leaves the quote on the key token.
            let after_quote = after
                .strip_prefix('"')
                .or_else(|| after.strip_prefix('\''))
                .unwrap_or(after);
            if let Some(value) = after_quote
                .strip_prefix(':')
                .or_else(|| after_quote.strip_prefix('='))
            {
                if value.trim().is_empty() {
                    // `"api_key":` — the value is quoted in the next token.
                    return (word.to_string(), true);
                }
                // Value rides in this token (`api_key=sk...`, `{"api_key":"sk..."}`).
                let key_end = word.len() - value.len();
                return (format!("{}[redacted]", &word[..key_end]), false);
            }
        }
    }
    (word.to_string(), false)
}

fn contains_extended_key_or_descriptor(lowered: &str) -> bool {
    let has_extended_key = [
        "xpub", "ypub", "zpub", "tpub", "upub", "vpub", "xprv", "yprv", "zprv", "tprv", "uprv",
        "vprv",
    ]
    .iter()
    .any(|prefix| lowered.contains(prefix) && lowered.len() >= prefix.len() + 20);
    if has_extended_key {
        return true;
    }
    ["wpkh(", "sh(", "wsh(", "tr(", "pkh(", "combo("]
        .iter()
        .any(|prefix| lowered.contains(prefix) && lowered.len() > prefix.len() + 16)
}

impl DaemonSupervisor {
    pub fn new(resource_dir: Option<PathBuf>) -> Self {
        Self {
            process: Mutex::new(None),
            resource_dir,
            data_root: Mutex::new(None),
            next_request_id: AtomicU64::new(1),
            secret_store: Arc::new(NativeSecretStore),
            lifecycle: Mutex::new(VecDeque::new()),
            next_lifecycle_id: AtomicU64::new(1),
            event_sink: Arc::new(Mutex::new(None)),
            invoke_timeout: DAEMON_INVOKE_TIMEOUT,
            silence_kill_timeout: DAEMON_SILENCE_KILL_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn new_with_process(process: DaemonProcess) -> Self {
        Self {
            process: Mutex::new(Some(Arc::new(process))),
            resource_dir: None,
            data_root: Mutex::new(None),
            next_request_id: AtomicU64::new(1),
            secret_store: Arc::new(NativeSecretStore),
            lifecycle: Mutex::new(VecDeque::new()),
            next_lifecycle_id: AtomicU64::new(1),
            event_sink: Arc::new(Mutex::new(None)),
            invoke_timeout: DAEMON_INVOKE_TIMEOUT,
            silence_kill_timeout: DAEMON_SILENCE_KILL_TIMEOUT,
        }
    }

    /// Override the timeouts so tests can exercise both the non-fatal (busy)
    /// and the silence-kill branches without waiting the production budgets.
    #[cfg(test)]
    fn with_timeouts(mut self, invoke_timeout: Duration, silence_kill_timeout: Duration) -> Self {
        self.invoke_timeout = invoke_timeout;
        self.silence_kill_timeout = silence_kill_timeout;
        self
    }

    #[cfg(test)]
    fn new_with_process_and_secret_store(
        process: DaemonProcess,
        secret_store: SharedSecretStore,
    ) -> Self {
        Self {
            process: Mutex::new(Some(Arc::new(process))),
            resource_dir: None,
            data_root: Mutex::new(None),
            next_request_id: AtomicU64::new(1),
            secret_store,
            lifecycle: Mutex::new(VecDeque::new()),
            next_lifecycle_id: AtomicU64::new(1),
            event_sink: Arc::new(Mutex::new(None)),
            invoke_timeout: DAEMON_INVOKE_TIMEOUT,
            silence_kill_timeout: DAEMON_SILENCE_KILL_TIMEOUT,
        }
    }

    /// Register the sink that receives unsolicited daemon→UI event
    /// records (`event: true`, no `request_id`). The app shell forwards
    /// them to the `daemon://event` Tauri channel.
    pub fn set_event_sink(&self, sink: impl Fn(&Value) + Send + Sync + 'static) {
        if let Ok(mut slot) = self.event_sink.lock() {
            *slot = Some(Box::new(sink));
        }
    }

    pub fn set_data_root(&self, data_root: PathBuf) -> Result<(), SupervisorError> {
        self.replace_data_root(Some(data_root))
    }

    pub fn clear_data_root(&self) -> Result<(), SupervisorError> {
        self.replace_data_root(None)
    }

    pub fn current_data_root(&self) -> Result<Option<PathBuf>, SupervisorError> {
        self.data_root
            .lock()
            .map(|data_root| data_root.clone())
            .map_err(|_| {
                SupervisorError::new("daemon_lock_poisoned", "daemon data-root lock is poisoned")
                    .retryable()
            })
    }

    fn replace_data_root(&self, data_root: Option<PathBuf>) -> Result<(), SupervisorError> {
        let mut slot = self.process.lock().map_err(|_| {
            SupervisorError::new("daemon_lock_poisoned", "daemon process lock is poisoned")
                .retryable()
        })?;
        let mut configured = self.data_root.lock().map_err(|_| {
            SupervisorError::new("daemon_lock_poisoned", "daemon data-root lock is poisoned")
                .retryable()
        })?;
        if *configured == data_root {
            return Ok(());
        }
        let replacement = self.spawn_daemon(data_root.clone())?;
        if let Some(process) = slot.replace(replacement) {
            let stderr_tail = process.stderr_tail();
            process.mark_broken();
            process.kill();
            self.record_lifecycle("replaced", "data root changed", &stderr_tail, "");
        }
        *configured = data_root;
        Ok(())
    }

    fn spawn_daemon(
        &self,
        data_root: Option<PathBuf>,
    ) -> Result<Arc<DaemonProcess>, SupervisorError> {
        let mut args = Vec::new();
        if let Some(data_root) = &data_root {
            args.push("--data-root".into());
            args.push(data_root.to_string_lossy().to_string());
        }
        args.push("daemon".into());
        let command = kassiber_command(self.resource_dir.as_deref(), args);
        let source = command.source;
        match DaemonProcess::spawn_command_with_secret_store(
            command,
            Arc::clone(&self.secret_store),
            Arc::clone(&self.event_sink),
        ) {
            Ok(process) => {
                self.record_lifecycle("spawned", "daemon ready", "", source);
                Ok(Arc::new(process))
            }
            Err(error) => {
                self.record_lifecycle(
                    "spawn_failed",
                    &error.message,
                    &error_stderr_tail(&error),
                    source,
                );
                Err(error)
            }
        }
    }

    fn record_lifecycle(
        &self,
        event: &'static str,
        detail: &str,
        stderr_tail: &str,
        source: &'static str,
    ) {
        let record = LifecycleRecord {
            id: self.next_lifecycle_id.fetch_add(1, Ordering::SeqCst),
            ts_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis() as u64)
                .unwrap_or(0),
            event,
            detail: redact_sensitive_text(detail),
            stderr_tail: redact_sensitive_text(stderr_tail),
            source,
        };
        if let Ok(mut ring) = self.lifecycle.lock() {
            if ring.len() == LIFECYCLE_RING_CAPACITY {
                ring.pop_front();
            }
            ring.push_back(record);
        }
    }

    pub fn lifecycle_snapshot(&self, after_id: u64) -> (Vec<LifecycleRecord>, u64) {
        let Ok(ring) = self.lifecycle.lock() else {
            return (Vec::new(), 0);
        };
        let last_id = ring.back().map(|record| record.id).unwrap_or(0);
        let records = ring
            .iter()
            .filter(|record| record.id > after_id)
            .cloned()
            .collect();
        (records, last_id)
    }

    pub fn invoke(
        &self,
        kind: &str,
        args: Option<Value>,
        app: &AppHandle,
        streaming: bool,
        client_request_id: Option<Value>,
    ) -> Result<Value, SupervisorError> {
        self.invoke_inner(kind, args, streaming, client_request_id, |response| {
            // Single channel per app; the webview filters by request_id
            // from the payload so we don't need a per-stream listener.
            if let Err(error) = app.emit("daemon://stream", response) {
                eprintln!("kassiber: failed to emit stream event: {error}");
            }
        })
    }

    fn invoke_inner<F>(
        &self,
        kind: &str,
        args: Option<Value>,
        streaming: bool,
        client_request_id: Option<Value>,
        mut emit_stream: F,
    ) -> Result<Value, SupervisorError>
    where
        F: FnMut(&Value),
    {
        // Honor a JS-supplied String request_id so streaming transports can
        // filter `daemon://stream` records as they arrive without buffering;
        // fall back to the supervisor-allocated id otherwise.
        let request_id = client_request_id
            .as_ref()
            .and_then(|value| value.as_str().map(String::from))
            .unwrap_or_else(|| self.allocate_request_id());
        let process = self.ensure_process()?;
        let (tx, rx) = mpsc::channel();
        process.register_request(request_id.clone(), tx)?;

        let mut request = json!({
            "request_id": request_id,
            "kind": kind,
        });
        let args = augment_ai_provider_secret_args(kind, args);
        if let Some(args) = args {
            request["args"] = args;
        }

        if let Err(error) = process.write_json_line(&request) {
            process.unregister_request(&request_id);
            process.mark_broken();
            process.kill();
            let stderr_tail = process.stderr_tail();
            self.record_lifecycle("killed", error.code, &stderr_tail, "");
            return Err(error.with_stderr_tail(stderr_tail));
        }

        // For streaming kinds we use a per-record inactivity timeout so a slow
        // model that keeps emitting tokens stays alive past the total-budget.
        // Non-streaming kinds keep the original total deadline.
        let deadline = if streaming {
            None
        } else {
            Some(Instant::now() + self.invoke_timeout)
        };

        let result = loop {
            let remaining = if let Some(deadline) = deadline {
                deadline.saturating_duration_since(Instant::now())
            } else {
                DAEMON_STREAM_INACTIVITY_TIMEOUT
            };
            let mut response = match rx.recv_timeout(remaining) {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => break Err(error.with_stderr_tail(process.stderr_tail())),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // A non-streaming request blew its budget. Historically we
                    // killed the whole daemon here — but the Python daemon
                    // dispatches requests on one serial loop, so an overdue
                    // reply almost always means it is *busy* serving another
                    // (often streaming) request, not that it is dead. Killing it
                    // for a routine poll (ui.logs.snapshot / ui.overview.snapshot
                    // / ui.wallets.list) that lands during a long sync is the
                    // root of the supervisor kill loop: the kill marks the
                    // shared process broken, the next request respawns it
                    // locked, and heavy work re-runs. Only kill on real evidence
                    // of death; otherwise fail THIS request retryably and leave
                    // the daemon (and its in-flight work) running.
                    if let Ok(Some(status)) = process.try_wait() {
                        process.mark_broken();
                        let stderr_tail = process.stderr_tail();
                        self.record_lifecycle("exited", &status.to_string(), &stderr_tail, "");
                        break Err(SupervisorError::new(
                            "daemon_exited",
                            format!("Python daemon exited before answering: {status}"),
                        )
                        .with_stderr_tail(stderr_tail)
                        .retryable());
                    }
                    let silence = process.silence_elapsed();
                    if silence >= self.silence_kill_timeout {
                        process.mark_broken();
                        process.kill();
                        let stderr_tail = process.stderr_tail();
                        self.record_lifecycle("killed", "daemon_timeout", &stderr_tail, "");
                        break Err(SupervisorError::new(
                            "daemon_timeout",
                            format!(
                                "Python daemon produced no output for {} seconds",
                                silence.as_secs()
                            ),
                        )
                        .with_stderr_tail(stderr_tail)
                        .retryable());
                    }
                    // Alive and recently productive — just slow for this one
                    // request. Return early WITHOUT unregistering: the eventual
                    // late response will find the (receiver-dropped) sender and
                    // be discarded by the stdout reader's send-failure path,
                    // rather than hitting the unknown-request_id branch that
                    // would kill the daemon.
                    return Err(SupervisorError::new(
                        "daemon_busy",
                        "The daemon is busy with a long-running operation. Please retry shortly.",
                    )
                    .retryable());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    process.mark_broken();
                    process.kill();
                    let stderr_tail = process.stderr_tail();
                    self.record_lifecycle("killed", "daemon_exited", &stderr_tail, "");
                    break Err(SupervisorError::new(
                        "daemon_exited",
                        "Python daemon stdout reader stopped",
                    )
                    .with_stderr_tail(stderr_tail)
                    .retryable());
                }
            };

            let response_kind = response
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            if response.get("request_id").and_then(request_id_value_key)
                != Some(request_id.as_str())
            {
                process.mark_broken();
                process.kill();
                let stderr_tail = process.stderr_tail();
                self.record_lifecycle("killed", "daemon_request_id_mismatch", &stderr_tail, "");
                break Err(
                    request_id_mismatch(&request_id, &response).with_stderr_tail(stderr_tail)
                );
            }

            if response_kind == kind || response_kind == "error" || response_kind == "auth_required"
            {
                attach_stderr_tail_to_internal_error(&mut response, process.stderr_tail());
                break Ok(response);
            }

            if streaming {
                emit_stream(&response);
                continue;
            }

            // Pre-streaming "progress" passthrough kept for the existing
            // protocol surface; non-streaming kinds still ignore them.
            if response_kind == "progress" {
                continue;
            }

            process.mark_broken();
            process.kill();
            let stderr_tail = process.stderr_tail();
            self.record_lifecycle("killed", "daemon_protocol_error", &stderr_tail, "");
            break Err(SupervisorError::new(
                "daemon_protocol_error",
                "Python daemon emitted a non-terminal record for a non-streaming request",
            )
            .details(json!({
                "request_id": request_id,
                "kind": response_kind,
            }))
            .with_stderr_tail(stderr_tail));
        };
        process.unregister_request(&request_id);
        result
    }

    fn allocate_request_id(&self) -> String {
        let request_id = self.next_request_id.fetch_add(1, Ordering::SeqCst);
        format!("tauri-{request_id}")
    }

    fn ensure_process(&self) -> Result<Arc<DaemonProcess>, SupervisorError> {
        let mut slot = self.process.lock().map_err(|_| {
            SupervisorError::new("daemon_lock_poisoned", "daemon process lock is poisoned")
                .retryable()
        })?;
        let should_restart = match slot.as_ref() {
            Some(process) => {
                if process.is_broken() {
                    let stderr_tail = process.stderr_tail();
                    process.kill();
                    self.record_lifecycle("killed", "daemon marked broken", &stderr_tail, "");
                    *slot = None;
                    true
                } else {
                    match process.try_wait() {
                        Ok(Some(status)) => {
                            let stderr_tail = process.stderr_tail();
                            *slot = None;
                            self.record_lifecycle("exited", &status.to_string(), &stderr_tail, "");
                            return Err(SupervisorError::new(
                                "daemon_exited",
                                format!(
                                    "Python daemon exited before handling the request: {status}"
                                ),
                            )
                            .hint("Restart the Tauri shell and check the daemon smoke test output.")
                            .with_stderr_tail(stderr_tail)
                            .retryable());
                        }
                        Ok(None) => false,
                        Err(error) => {
                            *slot = None;
                            return Err(SupervisorError::new(
                                "daemon_status_failed",
                                format!("Could not inspect Python daemon status: {error}"),
                            )
                            .retryable());
                        }
                    }
                }
            }
            None => true,
        };

        if should_restart {
            let data_root = self
                .data_root
                .lock()
                .map_err(|_| {
                    SupervisorError::new(
                        "daemon_lock_poisoned",
                        "daemon data-root lock is poisoned",
                    )
                    .retryable()
                })?
                .clone();
            *slot = Some(self.spawn_daemon(data_root)?);
        }

        slot.as_ref().cloned().ok_or_else(|| {
            SupervisorError::new("daemon_unavailable", "Python daemon is unavailable")
        })
    }
}

impl DaemonProcess {
    #[cfg(test)]
    fn spawn_command(command: DaemonCommand) -> Result<Self, SupervisorError> {
        Self::spawn_command_with_secret_store(
            command,
            Arc::new(NativeSecretStore),
            Arc::new(Mutex::new(None)),
        )
    }

    #[cfg(test)]
    fn spawn_command_with_event_sink(
        command: DaemonCommand,
        event_sink: SharedEventSink,
    ) -> Result<Self, SupervisorError> {
        Self::spawn_command_with_secret_store(command, Arc::new(NativeSecretStore), event_sink)
    }

    fn spawn_command_with_secret_store(
        command: DaemonCommand,
        secret_store: SharedSecretStore,
        event_sink: SharedEventSink,
    ) -> Result<Self, SupervisorError> {
        let mut process_command = Command::new(&command.program);
        process_command
            .args(&command.args)
            .current_dir(&command.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_console_window(&mut process_command);
        let mut child = process_command.spawn().map_err(|error| {
            SupervisorError::new(
                "daemon_spawn_failed",
                format!(
                    "Could not start Kassiber daemon with {:?}: {error}",
                    command.program
                ),
            )
            .hint(command.failure_hint())
            .details(json!({
                "cwd": command.cwd,
                "source": command.source,
            }))
            .retryable()
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            SupervisorError::new(
                "daemon_spawn_failed",
                "Python daemon stdin was not captured",
            )
        })?;
        let stdin = Arc::new(Mutex::new(stdin));
        let stdout = child.stdout.take().ok_or_else(|| {
            SupervisorError::new(
                "daemon_spawn_failed",
                "Python daemon stdout was not captured",
            )
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            SupervisorError::new(
                "daemon_spawn_failed",
                "Python daemon stderr was not captured",
            )
        })?;
        let stderr_tail = StderrTail::spawn(stderr);

        let broken = Arc::new(AtomicBool::new(false));
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let last_activity = Arc::new(Mutex::new(Instant::now()));
        let (ready_tx, ready_rx) = mpsc::channel();
        spawn_stdout_reader(
            stdout,
            Arc::clone(&pending),
            Arc::clone(&broken),
            Arc::clone(&last_activity),
            ready_tx,
            Arc::clone(&stdin),
            secret_store,
            event_sink,
        );
        let process = Self {
            child: Mutex::new(child),
            stdin,
            pending,
            stderr_tail,
            broken,
            last_activity,
        };
        let ready = match ready_rx.recv_timeout(DAEMON_READY_TIMEOUT) {
            Ok(Ok(ready)) => ready,
            Ok(Err(error)) => {
                process.mark_broken();
                process.kill();
                return Err(error.with_stderr_tail(process.stderr_tail()));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                process.mark_broken();
                process.kill();
                return Err(SupervisorError::new(
                    "daemon_timeout",
                    format!(
                        "Python daemon did not answer within {} seconds",
                        DAEMON_READY_TIMEOUT.as_secs()
                    ),
                )
                .with_stderr_tail(process.stderr_tail())
                .retryable());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                process.mark_broken();
                process.kill();
                return Err(SupervisorError::new(
                    "daemon_exited",
                    "Python daemon stdout reader stopped",
                )
                .with_stderr_tail(process.stderr_tail())
                .retryable());
            }
        };
        if ready.get("kind").and_then(Value::as_str) != Some("daemon.ready") {
            process.mark_broken();
            process.kill();
            return Err(SupervisorError::new(
                "daemon_protocol_error",
                "Python daemon did not emit daemon.ready on startup",
            )
            .details(ready)
            .with_stderr_tail(process.stderr_tail()));
        }

        Ok(process)
    }

    fn register_request(
        &self,
        request_id: String,
        sender: mpsc::Sender<Result<Value, SupervisorError>>,
    ) -> Result<(), SupervisorError> {
        if self.is_broken() {
            return Err(SupervisorError::new(
                "daemon_exited",
                "Python daemon stdout reader stopped",
            )
            .retryable());
        }
        let mut pending = self.pending.lock().map_err(|_| {
            SupervisorError::new(
                "daemon_lock_poisoned",
                "daemon request registry is poisoned",
            )
            .retryable()
        })?;
        if pending.contains_key(&request_id) {
            return Err(SupervisorError::new(
                "daemon_request_id_conflict",
                "daemon request_id is already in flight",
            )
            .details(json!({ "request_id": request_id })));
        }
        pending.insert(request_id, sender);
        Ok(())
    }

    fn unregister_request(&self, request_id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(request_id);
        }
    }

    fn write_json_line(&self, payload: &Value) -> Result<(), SupervisorError> {
        let line = serde_json::to_string(payload).map_err(|error| {
            SupervisorError::new(
                "daemon_protocol_error",
                format!("Could not serialize daemon request: {error}"),
            )
        })?;
        let mut stdin = self.stdin.lock().map_err(|_| {
            SupervisorError::new("daemon_lock_poisoned", "daemon stdin lock is poisoned")
                .retryable()
        })?;
        writeln!(stdin, "{line}").map_err(|error| {
            SupervisorError::new(
                "daemon_write_failed",
                format!("Could not write to Python daemon stdin: {error}"),
            )
            .retryable()
        })?;
        stdin.flush().map_err(|error| {
            SupervisorError::new(
                "daemon_write_failed",
                format!("Could not flush Python daemon stdin: {error}"),
            )
            .retryable()
        })
    }

    fn stderr_tail(&self) -> String {
        self.stderr_tail.text()
    }

    fn try_wait(&self) -> Result<Option<std::process::ExitStatus>, String> {
        let mut child = self
            .child
            .lock()
            .map_err(|_| "daemon child lock is poisoned".to_string())?;
        child.try_wait().map_err(|error| error.to_string())
    }

    fn is_broken(&self) -> bool {
        self.broken.load(Ordering::SeqCst)
    }

    /// How long since the daemon last wrote anything to stdout. A short value
    /// while a request is overdue means the daemon is busy (alive), not wedged.
    fn silence_elapsed(&self) -> Duration {
        self.last_activity
            .lock()
            .map(|last| last.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    fn mark_broken(&self) {
        self.broken.store(true, Ordering::SeqCst);
    }

    fn kill(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct DaemonCommand {
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    source: &'static str,
}

pub fn run_cli(resource_dir: Option<&Path>, args: Vec<String>) -> i32 {
    let command = kassiber_command(resource_dir, args);
    match Command::new(&command.program)
        .args(&command.args)
        .current_dir(&command.cwd)
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!(
                "Could not start Kassiber CLI with {:?}: {error}",
                command.program
            );
            eprintln!("{}", command.failure_hint());
            1
        }
    }
}

impl DaemonCommand {
    fn failure_hint(&self) -> &'static str {
        match self.source {
            "bundled_sidecar" => {
                "The bundled Kassiber CLI sidecar failed to start; reinstall the desktop package or set KASSIBER_PYTHON to override it."
            }
            _ => "Set KASSIBER_PYTHON to a Python with Kassiber importable.",
        }
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

impl StderrTail {
    fn spawn(stderr: ChildStderr) -> Self {
        let tail = Self {
            bytes: Arc::new(Mutex::new(Vec::new())),
        };
        let thread_tail = tail.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut buffer = [0; 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => return,
                    Ok(count) => thread_tail.append(&buffer[..count]),
                    Err(_) => return,
                }
            }
        });
        tail
    }

    fn append(&self, chunk: &[u8]) {
        if let Ok(mut bytes) = self.bytes.lock() {
            bytes.extend_from_slice(chunk);
            if bytes.len() > STDERR_TAIL_LIMIT {
                let overflow = bytes.len() - STDERR_TAIL_LIMIT;
                bytes.drain(..overflow);
            }
        }
    }

    fn text(&self) -> String {
        self.bytes
            .lock()
            .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
            .unwrap_or_default()
    }
}

// Pure wiring: each argument is a distinct shared handle the reader thread
// needs for the lifetime of the process; bundling them into a struct would not
// improve clarity.
#[allow(clippy::too_many_arguments)]
fn spawn_stdout_reader(
    stdout: ChildStdout,
    pending: PendingMap,
    broken: Arc<AtomicBool>,
    last_activity: Arc<Mutex<Instant>>,
    ready_tx: PendingSender,
    stdin: SharedStdin,
    secret_store: SharedSecretStore,
    event_sink: SharedEventSink,
) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut startup = Some(ready_tx);
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line);
            // Any non-empty line — response, stream delta, event, or even a
            // malformed record — proves the daemon is alive and producing
            // output, so refresh the liveness clock the non-streaming timeout
            // path consults before deciding a slow request means a dead daemon.
            if matches!(&read, Ok(count) if *count > 0) {
                if let Ok(mut last) = last_activity.lock() {
                    *last = Instant::now();
                }
            }
            match read {
                Ok(0) => {
                    fail_stdout_reader(
                        &broken,
                        &pending,
                        &mut startup,
                        SupervisorError::new("daemon_exited", "Python daemon closed stdout")
                            .retryable(),
                    );
                    return;
                }
                Ok(_) => match serde_json::from_str::<Value>(line.trim()) {
                    Ok(response) => {
                        if response.get("kind").and_then(Value::as_str) == Some("daemon.ready")
                            && response.get("request_id").is_none()
                        {
                            if let Some(tx) = startup.take() {
                                let _ = tx.send(Ok(response));
                                continue;
                            }
                        }

                        if startup.is_some() {
                            fail_stdout_reader(
                                &broken,
                                &pending,
                                &mut startup,
                                SupervisorError::new(
                                    "daemon_protocol_error",
                                    "Python daemon emitted a request response before daemon.ready",
                                )
                                .details(response),
                            );
                            return;
                        }

                        match handle_secret_store_control_request(&response, &stdin, &secret_store)
                        {
                            Ok(true) => continue,
                            Ok(false) => {}
                            Err(error) => {
                                fail_stdout_reader(&broken, &pending, &mut startup, error);
                                return;
                            }
                        }

                        match handle_daemon_event_record(&response, &event_sink) {
                            Ok(true) => continue,
                            Ok(false) => {}
                            Err(error) => {
                                fail_stdout_reader(&broken, &pending, &mut startup, error);
                                return;
                            }
                        }

                        let Some(request_id) = response
                            .get("request_id")
                            .and_then(request_id_value_key)
                            .map(str::to_string)
                        else {
                            fail_stdout_reader(
                                &broken,
                                &pending,
                                &mut startup,
                                SupervisorError::new(
                                    "daemon_protocol_error",
                                    "Python daemon emitted a response without request_id",
                                )
                                .details(response),
                            );
                            return;
                        };

                        let sender = match pending.lock() {
                            Ok(pending) => pending.get(&request_id).cloned(),
                            Err(_) => {
                                fail_stdout_reader(
                                    &broken,
                                    &pending,
                                    &mut startup,
                                    SupervisorError::new(
                                        "daemon_lock_poisoned",
                                        "daemon request registry is poisoned",
                                    )
                                    .retryable(),
                                );
                                return;
                            }
                        };

                        if let Some(sender) = sender {
                            if sender.send(Ok(response)).is_err() {
                                if let Ok(mut pending) = pending.lock() {
                                    pending.remove(&request_id);
                                }
                            }
                            continue;
                        }

                        fail_stdout_reader(
                            &broken,
                            &pending,
                            &mut startup,
                            SupervisorError::new(
                                "daemon_request_id_mismatch",
                                "Python daemon emitted a response for an unknown request_id",
                            )
                            .details(json!({ "request_id": request_id })),
                        );
                        return;
                    }
                    Err(error) => {
                        fail_stdout_reader(
                            &broken,
                            &pending,
                            &mut startup,
                            SupervisorError::new(
                                "daemon_protocol_error",
                                format!("Python daemon emitted invalid JSON: {error}"),
                            )
                            .details(json!({ "line": line.trim() })),
                        );
                        return;
                    }
                },
                Err(error) => {
                    fail_stdout_reader(
                        &broken,
                        &pending,
                        &mut startup,
                        SupervisorError::new(
                            "daemon_read_failed",
                            format!("Could not read from Python daemon stdout: {error}"),
                        )
                        .retryable(),
                    );
                    return;
                }
            }
        }
    });
}

fn augment_ai_provider_secret_args(kind: &str, args: Option<Value>) -> Option<Value> {
    if !uses_ai_provider_secret_bridge(kind) {
        return args;
    }
    let mut map = match args {
        Some(Value::Object(map)) => map,
        Some(other) => {
            let mut map = Map::new();
            map.insert("_desktop_original_args".to_string(), other);
            map
        }
        None => Map::new(),
    };
    map.insert(
        "_desktop_secret_store_bridge".to_string(),
        Value::Bool(true),
    );

    if matches!(
        kind,
        "ai.providers.set_api_key" | "ai.providers.move_api_key"
    ) {
        let requested = map
            .get("store_id")
            .and_then(Value::as_str)
            .filter(|value| *value != STORE_ID_SQLCIPHER_INLINE);
        let selection = current_ai_provider_secret_store_policy(requested);
        map.insert(
            "_desktop_secret_store_default".to_string(),
            Value::String(selection.store_id.clone()),
        );
        map.insert(
            "_desktop_secret_store_policy".to_string(),
            serde_json::to_value(selection).unwrap_or(Value::Null),
        );
    }

    Some(Value::Object(map))
}

fn uses_ai_provider_secret_bridge(kind: &str) -> bool {
    matches!(
        kind,
        "ai.providers.list"
            | "ai.providers.get"
            | "ai.providers.set_api_key"
            | "ai.providers.move_api_key"
            | "ai.providers.delete"
            | "ai.list_models"
            | "ai.test_connection"
            | "ai.chat"
    )
}

fn handle_secret_store_control_request(
    response: &Value,
    stdin: &SharedStdin,
    secret_store: &SharedSecretStore,
) -> Result<bool, SupervisorError> {
    if response.get("kind").and_then(Value::as_str) != Some(SECRET_STORE_CONTROL_REQUEST_KIND) {
        return Ok(false);
    }
    let response_payload = secret_store_control_response(response, secret_store);
    let line = serde_json::to_string(&response_payload).map_err(|error| {
        SupervisorError::new(
            "secret_store_bridge_failed",
            format!("Could not serialize secret-store bridge response: {error}"),
        )
    })?;
    let mut stdin = stdin.lock().map_err(|_| {
        SupervisorError::new("daemon_lock_poisoned", "daemon stdin lock is poisoned").retryable()
    })?;
    writeln!(stdin, "{line}").map_err(|error| {
        SupervisorError::new(
            "secret_store_bridge_failed",
            format!("Could not write secret-store bridge response: {error}"),
        )
        .retryable()
    })?;
    stdin.flush().map_err(|error| {
        SupervisorError::new(
            "secret_store_bridge_failed",
            format!("Could not flush secret-store bridge response: {error}"),
        )
        .retryable()
    })?;
    Ok(true)
}

fn secret_store_control_response(request: &Value, secret_store: &SharedSecretStore) -> Value {
    let request_id = request.get("request_id").cloned().unwrap_or(Value::Null);
    let result = handle_secret_store_operation(request, secret_store);
    match result {
        Ok(data) => json!({
            "kind": SECRET_STORE_CONTROL_RESPONSE_KIND,
            "schema_version": 1,
            "request_id": request_id,
            "data": data,
        }),
        Err(message) => json!({
            "kind": SECRET_STORE_CONTROL_RESPONSE_KIND,
            "schema_version": 1,
            "request_id": request_id,
            "error": {
                "code": "secret_store_bridge_error",
                "message": redact_sensitive_text(&message),
                "retryable": true,
            },
        }),
    }
}

fn handle_secret_store_operation(
    request: &Value,
    secret_store: &SharedSecretStore,
) -> Result<Value, String> {
    let data = request
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "secret-store bridge request missing data".to_string())?;
    let provider_name = data
        .get("provider_name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "secret-store bridge request missing provider_name".to_string())?;
    let op = data
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| "secret-store bridge request missing op".to_string())?;
    let store_id = data
        .get("store_id")
        .and_then(Value::as_str)
        .unwrap_or(STORE_ID_SQLCIPHER_INLINE);
    if store_id == STORE_ID_SQLCIPHER_INLINE {
        return Err("sqlcipher_inline refs are handled by the Python daemon".to_string());
    }
    let platform = current_secret_store_platform();
    let Some(native_store_id) = native_store_id_for_platform(&platform) else {
        return Err("this platform does not support native AI provider secret storage".to_string());
    };
    if store_id != native_store_id {
        return Err(format!(
            "requested AI provider secret store {store_id:?} is not available on this platform"
        ));
    }

    match op {
        "availability" => Ok(json!({
            "provider_name": provider_name,
            "availability": secret_store.availability(),
        })),
        "get" => {
            let (service, account) = secret_ref_service_account(data)?;
            match secret_store.get(service, account)? {
                Some(secret) => {
                    let secret = String::from_utf8(secret)
                        .map_err(|_| "stored provider API key is not UTF-8".to_string())?;
                    Ok(json!({
                        "provider_name": provider_name,
                        "state": "ok",
                        "secret": secret,
                    }))
                }
                None => Ok(json!({
                    "provider_name": provider_name,
                    "state": "missing",
                    "secret": Value::Null,
                })),
            }
        }
        "exists" => {
            let (service, account) = secret_ref_service_account(data)?;
            let state = if secret_store.exists(service, account)? {
                "ok"
            } else {
                "missing"
            };
            Ok(json!({
                "provider_name": provider_name,
                "state": state,
            }))
        }
        "set" => {
            let (service, account) = secret_ref_service_account(data)?;
            let secret = data
                .get("secret")
                .and_then(Value::as_str)
                .ok_or_else(|| "secret-store set request missing secret".to_string())?;
            secret_store.set(service, account, secret.as_bytes())?;
            Ok(json!({
                "provider_name": provider_name,
                "state": "ok",
            }))
        }
        "delete" => {
            let (service, account) = secret_ref_service_account(data)?;
            secret_store.delete(service, account)?;
            Ok(json!({
                "provider_name": provider_name,
                "state": "missing",
            }))
        }
        other => Err(format!("unsupported secret-store bridge op {other:?}")),
    }
}

fn secret_ref_service_account(data: &Map<String, Value>) -> Result<(&str, &str), String> {
    let service = data
        .get("service")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "secret-store bridge request missing service".to_string())?;
    let account = data
        .get("account")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "secret-store bridge request missing account".to_string())?;
    Ok((service, account))
}

/// Route unsolicited daemon→UI event records (`event: true`).
///
/// Returns `Ok(true)` when the record was an event and has been
/// forwarded (or dropped because no sink is registered yet), `Ok(false)`
/// when the record is not an event and should flow through request_id
/// routing, and `Err` for malformed event records — an event must carry
/// a non-empty `kind` and must not carry a `request_id`.
fn handle_daemon_event_record(
    response: &Value,
    event_sink: &SharedEventSink,
) -> Result<bool, SupervisorError> {
    if response.get("event").and_then(Value::as_bool) != Some(true) {
        return Ok(false);
    }
    if response.get("request_id").is_some() {
        return Err(SupervisorError::new(
            "daemon_protocol_error",
            "Python daemon emitted an event record with a request_id",
        )
        .details(response.clone()));
    }
    if response
        .get("kind")
        .and_then(Value::as_str)
        .filter(|kind| !kind.trim().is_empty())
        .is_none()
    {
        return Err(SupervisorError::new(
            "daemon_protocol_error",
            "Python daemon emitted an event record without a kind",
        )
        .details(response.clone()));
    }
    match event_sink.lock() {
        Ok(sink) => match sink.as_ref() {
            Some(sink) => sink(response),
            None => eprintln!(
                "kassiber: dropping daemon event {:?} (no event sink registered)",
                response.get("kind").and_then(Value::as_str).unwrap_or("")
            ),
        },
        // A sink that panicked poisons the lock; losing events is better
        // than killing a healthy daemon over a UI-side failure.
        Err(_) => eprintln!("kassiber: dropping daemon event (event sink lock is poisoned)"),
    }
    Ok(true)
}

fn fail_stdout_reader(
    broken: &Arc<AtomicBool>,
    pending: &PendingMap,
    startup: &mut Option<PendingSender>,
    error: SupervisorError,
) {
    broken.store(true, Ordering::SeqCst);
    if let Some(tx) = startup.take() {
        let _ = tx.send(Err(error.clone()));
    }
    broadcast_pending_error(pending, error);
}

fn broadcast_pending_error(pending: &PendingMap, error: SupervisorError) {
    let senders = match pending.lock() {
        Ok(mut pending) => pending
            .drain()
            .map(|(_, sender)| sender)
            .collect::<Vec<_>>(),
        Err(_) => return,
    };
    for sender in senders {
        let _ = sender.send(Err(error.clone()));
    }
}

fn request_id_value_key(value: &Value) -> Option<&str> {
    value.as_str()
}

fn error_stderr_tail(error: &SupervisorError) -> String {
    error
        .details
        .as_ref()
        .and_then(|details| details.get("stderr_tail"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn attach_stderr_tail_to_internal_error(response: &mut Value, stderr_tail: String) {
    if stderr_tail.is_empty()
        || response.get("kind").and_then(Value::as_str) != Some("error")
        || response
            .get("error")
            .and_then(Value::as_object)
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
            != Some("internal_error")
    {
        return;
    }

    let Some(error) = response.get_mut("error").and_then(Value::as_object_mut) else {
        return;
    };
    match error.get_mut("details") {
        Some(Value::Object(details)) => {
            details.insert("stderr_tail".to_string(), Value::String(stderr_tail));
        }
        Some(Value::Null) | None => {
            error.insert("details".to_string(), json!({ "stderr_tail": stderr_tail }));
        }
        Some(details) => {
            let previous = details.take();
            *details = json!({
                "details": previous,
                "stderr_tail": stderr_tail,
            });
        }
    }
}

fn request_id_mismatch(expected: &str, response: &Value) -> SupervisorError {
    SupervisorError::new(
        "daemon_request_id_mismatch",
        "Python daemon response request_id did not match the active request",
    )
    .details(json!({
        "expected": expected,
        "actual": response.get("request_id").cloned().unwrap_or(Value::Null),
        "kind": response.get("kind").cloned().unwrap_or(Value::Null),
    }))
}

fn repo_root() -> PathBuf {
    if let Ok(path) = env::var("KASSIBER_REPO_ROOT") {
        // Trust explicit user overrides even when stale; the daemon error then points at the chosen path.
        return PathBuf::from(path);
    }

    let build_repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(PathBuf::from);
    if let Some(path) = build_repo_root {
        if path.exists() {
            return path;
        }
    }

    // Packaged previews use this as a venv search root before falling back to python3.
    env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_python(repo_root: &Path) -> PathBuf {
    let unix_venv = repo_root.join(".venv").join("bin").join("python");
    if unix_venv.exists() {
        return unix_venv;
    }

    let windows_venv = repo_root.join(".venv").join("Scripts").join("python.exe");
    if windows_venv.exists() {
        return windows_venv;
    }

    PathBuf::from("python3")
}

fn kassiber_command(resource_dir: Option<&Path>, args: Vec<String>) -> DaemonCommand {
    if let Ok(python) = env::var("KASSIBER_PYTHON") {
        let repo_root = repo_root();
        let mut python_args = vec!["-m".into(), "kassiber".into()];
        python_args.extend(args);
        return DaemonCommand {
            program: PathBuf::from(python),
            args: python_args,
            cwd: repo_root,
            source: "env_python",
        };
    }

    if let Some(sidecar) = bundled_sidecar(resource_dir) {
        // Packaged macOS builds inherit the bundle's read-only Contents/Resources
        // as cwd, which breaks any third-party library that writes relative paths
        // at import time (e.g. rp2.logger -> ./log/rp2_*.log). Hand the sidecar a
        // writable scratch directory so the daemon never has to dodge EACCES on
        // startup. Per-call code already uses absolute paths under the data root.
        return DaemonCommand {
            program: sidecar,
            args,
            cwd: env::temp_dir(),
            source: "bundled_sidecar",
        };
    }

    let repo_root = repo_root();
    let mut python_args = vec!["-m".into(), "kassiber".into()];
    python_args.extend(args);
    DaemonCommand {
        program: default_python(&repo_root),
        args: python_args,
        cwd: repo_root,
        source: "python_fallback",
    }
}

fn bundled_sidecar(resource_dir: Option<&Path>) -> Option<PathBuf> {
    let resource_dir = resource_dir?;
    let sidecar = sidecar_filename()?;
    [
        // Packaged Tauri builds place resources under the configured
        // `binaries/` directory. The flat fallback keeps manually assembled
        // dev bundles easy to smoke-test.
        resource_dir.join("binaries").join(&sidecar),
        resource_dir.join(&sidecar),
    ]
    .into_iter()
    .find(|path| path.exists())
}

fn sidecar_filename() -> Option<String> {
    let triple = match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        // Not built by the prerelease workflow yet; Linux arm64 falls back to
        // developer Python unless a matching sidecar is manually bundled.
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return None,
    };
    let extension = if env::consts::OS == "windows" {
        ".exe"
    } else {
        ""
    };
    Some(format!("kassiber-cli-{triple}{extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    static TEST_TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn redacts_stderr_tail_and_sensitive_details() {
        let error = SupervisorError::new("internal", "failed")
            .details(json!({
                "api_key": "sk-detail-secret",
                "mnemonic": "abandon abandon abandon",
                "line": "token=btcpay-secret Bearer openai-secret descriptor=wpkh(xpub661MyMwAqRbcF12345678901234567890) recovery_phrase=legal winner thank"
            }))
            .with_stderr_tail(
                "api_key=sk-stderr-secret Bearer stderr-secret passphrase_secret=correct seed_phrase=letter advice cage raw xpub661MyMwAqRbcF12345678901234567890".to_string(),
            );
        let encoded = serde_json::to_string(&error.details).expect("details json");
        assert!(!encoded.contains("abandon"));
        assert!(!encoded.contains("legal"));
        assert!(!encoded.contains("winner"));
        assert!(!encoded.contains("letter"));
        assert!(!encoded.contains("advice"));
        assert!(!encoded.contains("sk-detail-secret"));
        assert!(!encoded.contains("btcpay-secret"));
        assert!(!encoded.contains("openai-secret"));
        assert!(!encoded.contains("sk-stderr-secret"));
        assert!(!encoded.contains("stderr-secret"));
        assert!(!encoded.contains("correct"));
        assert!(!encoded.contains("xpub661MyMwAqRbcF12345678901234567890"));
        assert!(encoded.contains("[redacted]"));
    }

    #[test]
    fn lifecycle_ring_caps_at_64_records() {
        let supervisor = DaemonSupervisor::new(None);
        for index in 0..70u64 {
            supervisor.record_lifecycle("spawn_failed", &format!("failure {index}"), "", "");
        }
        let (records, last_id) = supervisor.lifecycle_snapshot(0);
        assert_eq!(records.len(), LIFECYCLE_RING_CAPACITY);
        assert_eq!(last_id, 70);
        assert_eq!(records.first().map(|record| record.id), Some(7));
        assert_eq!(records.last().map(|record| record.id), Some(70));
    }

    #[test]
    fn lifecycle_records_are_redacted_at_insert() {
        let supervisor = DaemonSupervisor::new(None);
        // 24-word mnemonic assignment: first word in the assignment, 23 tail words.
        let stderr_tail = format!(
            "api_key=sk-test-secret mnemonic=abandon{} art",
            " abandon".repeat(22)
        );
        supervisor.record_lifecycle(
            "killed",
            "daemon rejected api_key=sk-test-secret",
            &stderr_tail,
            "env_python",
        );
        let (records, _) = supervisor.lifecycle_snapshot(0);
        let record = records.first().expect("lifecycle record");
        assert!(!record.detail.contains("sk-test-secret"));
        assert!(record.detail.contains("[redacted]"));
        assert!(!record.stderr_tail.contains("sk-test-secret"));
        assert!(!record.stderr_tail.contains("abandon"));
        assert!(!record.stderr_tail.contains("art"));
        assert!(record.stderr_tail.contains("[redacted]"));
        let encoded = serde_json::to_string(record).expect("record json");
        assert!(encoded.contains("\"tsMs\""));
        assert!(encoded.contains("\"stderrTail\""));
        assert!(encoded.contains("\"source\":\"env_python\""));
    }

    #[test]
    fn redact_sensitive_text_handles_json_and_dict_shapes() {
        // Raw daemon stderr can carry secrets in JSON or Python-dict shape,
        // where the value token starts with a quote rather than with `sk-`, so
        // the per-word prefix checks alone would miss it.
        let cases = [
            r#"{"api_key": "sk-live-001"}"#,
            r#"{"api_key":"btcpay-no-sk-prefix"}"#,
            r#"{'api_key': 'btcpay-no-sk-prefix'}"#,
            r#"{"token": "secret-value-here"}"#,
            r#"{"passphrase":"correct-horse-battery"}"#,
        ];
        for case in cases {
            let redacted = redact_sensitive_text(case);
            assert!(
                redacted.contains("[redacted]"),
                "no redaction marker in {redacted:?}"
            );
            for secret in [
                "sk-live-001",
                "btcpay-no-sk-prefix",
                "secret-value-here",
                "correct-horse-battery",
            ] {
                assert!(
                    !redacted.contains(secret),
                    "{secret:?} leaked while redacting {case:?} -> {redacted:?}"
                );
            }
        }
        // Key names and unrelated values survive.
        let kept = redact_sensitive_text(r#"{"note": "keep", "api_key": "sk-secret-001"}"#);
        assert!(kept.contains("note"));
        assert!(kept.contains("keep"));
        assert!(!kept.contains("sk-secret-001"));
    }

    #[test]
    fn lifecycle_redacts_json_shaped_stderr() {
        let supervisor = DaemonSupervisor::new(None);
        supervisor.record_lifecycle(
            "killed",
            "daemon crashed",
            r#"Traceback: AuthError({"api_key": "sk-leaked-key"})"#,
            "env_python",
        );
        let (records, _) = supervisor.lifecycle_snapshot(0);
        let record = records.first().expect("lifecycle record");
        assert!(!record.stderr_tail.contains("sk-leaked-key"));
        assert!(record.stderr_tail.contains("[redacted]"));
    }

    #[test]
    fn lifecycle_snapshot_filters_by_cursor() {
        let supervisor = DaemonSupervisor::new(None);
        let (records, last_id) = supervisor.lifecycle_snapshot(0);
        assert!(records.is_empty());
        assert_eq!(last_id, 0);

        for index in 0..3u64 {
            supervisor.record_lifecycle("spawned", &format!("spawn {index}"), "", "env_python");
        }
        let (records, last_id) = supervisor.lifecycle_snapshot(1);
        assert_eq!(last_id, 3);
        assert_eq!(
            records.iter().map(|record| record.id).collect::<Vec<_>>(),
            vec![2, 3]
        );

        let (records, last_id) = supervisor.lifecycle_snapshot(3);
        assert!(records.is_empty());
        assert_eq!(last_id, 3);
    }

    #[cfg(unix)]
    fn response_pid(response: &Value) -> i64 {
        response
            .get("data")
            .and_then(Value::as_object)
            .and_then(|data| data.get("pid"))
            .and_then(Value::as_i64)
            .expect("response pid")
    }

    #[cfg(unix)]
    fn write_stub_daemon() -> (PathBuf, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let counter = TEST_TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = env::temp_dir().join(format!(
            "kassiber-supervisor-test-{}-{unique}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        let script = dir.join("stub-daemon.py");
        let test_secret_store_id = native_store_id_for_platform(&current_secret_store_platform())
            .unwrap_or(STORE_ID_SQLCIPHER_INLINE);
        fs::write(
            &script,
            r#"#!/usr/bin/env python3
import json
import os
import sys
import threading
import time

write_lock = threading.Lock()

def emit(payload):
    with write_lock:
        sys.stdout.write(json.dumps(payload, separators=(",", ":")) + "\n")
        sys.stdout.flush()

sys.stderr.write("stub-daemon startup api_key=sk-stub-stderr-secret\n")
sys.stderr.flush()

emit({"kind":"daemon.ready","schema_version":1,"data":{"version":"test","supported_kinds":["slow","fast","locked","secret-get","emit-event","daemon.shutdown"]}})

def slow(request_id):
    emit({"kind":"slow.delta","schema_version":1,"request_id":request_id,"data":{"delta":{"content":"a"}}})
    time.sleep(0.35)
    emit({"kind":"slow","schema_version":1,"request_id":request_id,"data":{"finish_reason":"stop"}})

for line in sys.stdin:
    if not line.strip():
        continue
    request = json.loads(line)
    request_id = request.get("request_id")
    kind = request.get("kind")
    if kind == "slow":
        threading.Thread(target=slow, args=(request_id,), daemon=True).start()
    elif kind == "fast":
        emit({"kind":"fast","schema_version":1,"request_id":request_id,"data":{"ok":True,"pid":os.getpid()}})
    elif kind == "busy":
        # Read but never answer: models the single-threaded daemon being busy
        # on another request. The reader loop stays free for later requests.
        pass
    elif kind == "locked":
        emit({"kind":"auth_required","schema_version":1,"request_id":request_id,"data":{"scope":"unlock_database"}})
    elif kind == "emit-event":
        emit({"kind":"ui.freshness.background","schema_version":1,"event":True,"data":{"enqueued":[],"completed":[]}})
        emit({"kind":"emit-event","schema_version":1,"request_id":request_id,"data":{"ok":True}})
    elif kind == "secret-get":
        emit({"kind":"supervisor.ai_secret_store.request","schema_version":1,"request_id":"secret-control-1","data":{"op":"get","provider_name":"remote","store_id":"__TEST_STORE_ID__","service":"service-hash","account":"remote"}})
        control = json.loads(sys.stdin.readline())
        emit({"kind":"secret-get","schema_version":1,"request_id":request_id,"data":{"control_kind":control.get("kind"),"state":control.get("data",{}).get("state"),"secret":control.get("data",{}).get("secret")}})
    elif kind == "daemon.shutdown":
        emit({"kind":"daemon.shutdown","schema_version":1,"request_id":request_id,"data":{}})
        break
"#
            .replace("__TEST_STORE_ID__", test_secret_store_id),
        )
        .expect("write stub daemon");
        let mut permissions = fs::metadata(&script).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).expect("chmod stub daemon");
        (dir, script)
    }

    #[test]
    #[cfg(unix)]
    fn demuxes_fast_request_while_streaming_request_is_active() {
        let (dir, script) = write_stub_daemon();
        let process = DaemonProcess::spawn_command(DaemonCommand {
            program: script,
            args: Vec::new(),
            cwd: dir.clone(),
            source: "env_python",
        })
        .expect("spawn stub daemon");
        let supervisor = Arc::new(DaemonSupervisor::new_with_process(process));
        let (delta_tx, delta_rx) = mpsc::channel();

        let slow_supervisor = Arc::clone(&supervisor);
        let slow = thread::spawn(move || {
            slow_supervisor.invoke_inner("slow", None, true, Some(json!("slow-1")), |record| {
                let _ = delta_tx.send(record.clone());
            })
        });

        let delta = delta_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("stream delta");
        assert_eq!(
            delta.get("kind").and_then(Value::as_str),
            Some("slow.delta")
        );

        let started = Instant::now();
        let fast = supervisor
            .invoke_inner("fast", None, false, Some(json!("fast-1")), |_| {})
            .expect("fast response");
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "fast request waited for the slow stream to finish"
        );
        assert_eq!(fast.get("kind").and_then(Value::as_str), Some("fast"));
        assert_eq!(
            fast.get("request_id").and_then(Value::as_str),
            Some("fast-1")
        );

        let slow_terminal = slow.join().expect("slow join").expect("slow response");
        assert_eq!(
            slow_terminal.get("kind").and_then(Value::as_str),
            Some("slow")
        );

        let _ = supervisor.invoke_inner(
            "daemon.shutdown",
            None,
            false,
            Some(json!("shutdown-1")),
            |_| {},
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn busy_non_streaming_request_does_not_kill_a_live_daemon() {
        let (dir, script) = write_stub_daemon();
        let process = DaemonProcess::spawn_command(DaemonCommand {
            program: script,
            args: Vec::new(),
            cwd: dir.clone(),
            source: "env_python",
        })
        .expect("spawn stub daemon");
        // Short invoke budget so the busy request times out fast; a generous
        // silence window so the daemon (which just answered the warm-up) is
        // treated as alive, not wedged.
        let supervisor = DaemonSupervisor::new_with_process(process)
            .with_timeouts(Duration::from_millis(200), Duration::from_secs(30));

        // A real response stamps the liveness clock to ~now.
        let warm = supervisor
            .invoke_inner("fast", None, false, Some(json!("warm-1")), |_| {})
            .expect("warm-up response");
        let pid_before = response_pid(&warm);

        // A non-streaming request the busy daemon never answers must fail
        // retryably as daemon_busy WITHOUT killing the process.
        let busy = supervisor
            .invoke_inner("busy", None, false, Some(json!("busy-1")), |_| {})
            .expect_err("busy request should time out");
        assert_eq!(busy.code, "daemon_busy");
        assert!(busy.retryable);

        // The daemon must still answer, on the SAME process (no respawn) — the
        // late/never response to busy-1 must not have taken the daemon down.
        let after = supervisor
            .invoke_inner("fast", None, false, Some(json!("after-1")), |_| {})
            .expect("daemon still answers after a busy timeout");
        assert_eq!(response_pid(&after), pid_before);

        let _ = supervisor.invoke_inner(
            "daemon.shutdown",
            None,
            false,
            Some(json!("shutdown-busy-1")),
            |_| {},
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn silent_daemon_is_killed_after_the_silence_window() {
        let (dir, script) = write_stub_daemon();
        let process = DaemonProcess::spawn_command(DaemonCommand {
            program: script,
            args: Vec::new(),
            cwd: dir.clone(),
            source: "env_python",
        })
        .expect("spawn stub daemon");
        // Zero silence window: a non-streaming timeout while the daemon has
        // produced no output escalates to a kill — genuine-hang recovery is
        // preserved, distinct from the busy (alive) case above.
        let supervisor = DaemonSupervisor::new_with_process(process)
            .with_timeouts(Duration::from_millis(100), Duration::from_secs(0));

        let killed = supervisor
            .invoke_inner("busy", None, false, Some(json!("busy-1")), |_| {})
            .expect_err("silent daemon should be killed");
        assert_eq!(killed.code, "daemon_timeout");
        assert!(killed.retryable);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn auth_required_is_terminal_for_locked_requests() {
        let (dir, script) = write_stub_daemon();
        let process = DaemonProcess::spawn_command(DaemonCommand {
            program: script,
            args: Vec::new(),
            cwd: dir.clone(),
            source: "env_python",
        })
        .expect("spawn stub daemon");
        let supervisor = DaemonSupervisor::new_with_process(process);

        let response = supervisor
            .invoke_inner("locked", None, false, Some(json!("locked-1")), |_| {})
            .expect("auth_required response");

        assert_eq!(
            response.get("kind").and_then(Value::as_str),
            Some("auth_required")
        );
        assert_eq!(
            response.get("request_id").and_then(Value::as_str),
            Some("locked-1")
        );

        let _ = supervisor.invoke_inner(
            "daemon.shutdown",
            None,
            false,
            Some(json!("shutdown-locked-1")),
            |_| {},
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn forwards_unsolicited_event_records_to_event_sink() {
        let (dir, script) = write_stub_daemon();
        let (event_tx, event_rx) = mpsc::channel();
        let event_sink: SharedEventSink =
            Arc::new(Mutex::new(Some(Box::new(move |record: &Value| {
                let _ = event_tx.send(record.clone());
            }) as EventSink)));
        let process = DaemonProcess::spawn_command_with_event_sink(
            DaemonCommand {
                program: script,
                args: Vec::new(),
                cwd: dir.clone(),
                source: "env_python",
            },
            event_sink,
        )
        .expect("spawn stub daemon");
        let supervisor = DaemonSupervisor::new_with_process(process);

        let response = supervisor
            .invoke_inner(
                "emit-event",
                None,
                false,
                Some(json!("emit-event-1")),
                |_| {},
            )
            .expect("emit-event response despite preceding unsolicited event");
        assert_eq!(
            response.get("kind").and_then(Value::as_str),
            Some("emit-event")
        );

        let event = event_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("forwarded daemon event");
        assert_eq!(
            event.get("kind").and_then(Value::as_str),
            Some("ui.freshness.background")
        );
        assert_eq!(event.get("event").and_then(Value::as_bool), Some(true));
        assert!(event.get("request_id").is_none());

        let fast = supervisor
            .invoke_inner("fast", None, false, Some(json!("fast-after-event")), |_| {})
            .expect("daemon stays healthy after an unsolicited event");
        assert_eq!(fast.get("kind").and_then(Value::as_str), Some("fast"));

        let _ = supervisor.invoke_inner(
            "daemon.shutdown",
            None,
            false,
            Some(json!("shutdown-event-sink")),
            |_| {},
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn unsolicited_event_records_without_sink_do_not_break_daemon() {
        let (dir, script) = write_stub_daemon();
        let process = DaemonProcess::spawn_command(DaemonCommand {
            program: script,
            args: Vec::new(),
            cwd: dir.clone(),
            source: "env_python",
        })
        .expect("spawn stub daemon");
        let supervisor = DaemonSupervisor::new_with_process(process);

        // No event sink registered: the event is dropped with a log line,
        // but it must not be treated as a protocol error that kills the
        // daemon (the pre-fix behavior).
        let response = supervisor
            .invoke_inner(
                "emit-event",
                None,
                false,
                Some(json!("emit-event-no-sink")),
                |_| {},
            )
            .expect("emit-event response without a registered sink");
        assert_eq!(
            response.get("kind").and_then(Value::as_str),
            Some("emit-event")
        );

        let fast = supervisor
            .invoke_inner("fast", None, false, Some(json!("fast-no-sink")), |_| {})
            .expect("daemon stays healthy after a dropped event");
        assert_eq!(fast.get("kind").and_then(Value::as_str), Some("fast"));

        let _ = supervisor.invoke_inner(
            "daemon.shutdown",
            None,
            false,
            Some(json!("shutdown-no-sink")),
            |_| {},
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn event_records_with_request_id_or_missing_kind_are_protocol_errors() {
        let sink: SharedEventSink = Arc::new(Mutex::new(None));
        assert!(
            !handle_daemon_event_record(&json!({"kind": "fast", "request_id": "r-1"}), &sink)
                .expect("plain response is not an event")
        );
        assert!(handle_daemon_event_record(
            &json!({"kind": "ui.freshness.worker", "schema_version": 1, "event": true}),
            &sink
        )
        .expect("event without sink is consumed"));
        let with_request_id = handle_daemon_event_record(
            &json!({"kind": "ui.freshness.worker", "event": true, "request_id": "r-2"}),
            &sink,
        )
        .expect_err("event with request_id is a protocol error");
        assert_eq!(with_request_id.code, "daemon_protocol_error");
        let missing_kind = handle_daemon_event_record(&json!({"event": true}), &sink)
            .expect_err("event without kind is a protocol error");
        assert_eq!(missing_kind.code, "daemon_protocol_error");
    }

    #[test]
    #[cfg(unix)]
    fn handles_secret_store_control_requests_without_forwarding_them() {
        let (dir, script) = write_stub_daemon();
        let mock_store = crate::secret_store::MockSecretStore::new(
            crate::secret_store::SecretStoreAvailability::Available {
                identity_strength: crate::secret_store::IdentityStrength::Production,
            },
        );
        mock_store
            .set("service-hash", "remote", b"bridge-secret")
            .expect("seed mock store");
        let secret_store = Arc::new(mock_store);
        let process = DaemonProcess::spawn_command_with_secret_store(
            DaemonCommand {
                program: script,
                args: Vec::new(),
                cwd: dir.clone(),
                source: "env_python",
            },
            secret_store.clone(),
            Arc::new(Mutex::new(None)),
        )
        .expect("spawn stub daemon");
        let supervisor = DaemonSupervisor::new_with_process_and_secret_store(process, secret_store);

        let response = supervisor
            .invoke_inner(
                "secret-get",
                None,
                false,
                Some(json!("secret-get-1")),
                |_| {},
            )
            .expect("secret bridge response");

        assert_eq!(
            response.get("kind").and_then(Value::as_str),
            Some("secret-get")
        );
        assert_eq!(
            response
                .get("data")
                .and_then(Value::as_object)
                .and_then(|data| data.get("control_kind"))
                .and_then(Value::as_str),
            Some(SECRET_STORE_CONTROL_RESPONSE_KIND)
        );
        assert_eq!(
            response
                .get("data")
                .and_then(Value::as_object)
                .and_then(|data| data.get("state"))
                .and_then(Value::as_str),
            Some("ok")
        );
        assert_eq!(
            response
                .get("data")
                .and_then(Value::as_object)
                .and_then(|data| data.get("secret"))
                .and_then(Value::as_str),
            Some("bridge-secret")
        );

        let _ = supervisor.invoke_inner(
            "daemon.shutdown",
            None,
            false,
            Some(json!("shutdown-secret-bridge")),
            |_| {},
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn set_data_root_warms_replacement_daemon_before_returning() {
        let (dir, script) = write_stub_daemon();
        let sidecar = dir.join(sidecar_filename().expect("sidecar name"));
        fs::rename(&script, &sidecar).expect("install stub sidecar");
        let supervisor = DaemonSupervisor::new(Some(dir.clone()));
        let data_root = dir.join("imported-data");
        fs::create_dir_all(&data_root).expect("create data root");

        supervisor
            .set_data_root(data_root.clone())
            .expect("warm replacement daemon");

        assert_eq!(
            supervisor.data_root.lock().expect("data root").clone(),
            Some(data_root)
        );
        let response = supervisor
            .invoke_inner("fast", None, false, Some(json!("fast-after-root")), |_| {})
            .expect("replacement daemon is ready");
        assert_eq!(response.get("kind").and_then(Value::as_str), Some("fast"));

        let _ = supervisor.invoke_inner(
            "daemon.shutdown",
            None,
            false,
            Some(json!("shutdown-replacement")),
            |_| {},
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn set_data_root_records_replaced_with_dying_process_tail() {
        let (dir, script) = write_stub_daemon();
        let sidecar = dir.join(sidecar_filename().expect("sidecar name"));
        fs::rename(&script, &sidecar).expect("install stub sidecar");
        let supervisor = DaemonSupervisor::new(Some(dir.clone()));
        let first_root = dir.join("first-root");
        fs::create_dir_all(&first_root).expect("create first root");
        supervisor
            .set_data_root(first_root)
            .expect("spawn first daemon");

        // The stub writes its stderr line at startup but the tail thread reads
        // it asynchronously; wait for it so the dying process's tail is
        // guaranteed to be non-empty when the replacement lands.
        let first_process = supervisor
            .process
            .lock()
            .expect("process slot")
            .as_ref()
            .cloned()
            .expect("first daemon process");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !first_process.stderr_tail().contains("stub-daemon startup") {
            assert!(
                Instant::now() < deadline,
                "stub daemon stderr tail was not captured"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let second_root = dir.join("second-root");
        fs::create_dir_all(&second_root).expect("create second root");
        supervisor
            .set_data_root(second_root)
            .expect("replace daemon");

        let (records, last_id) = supervisor.lifecycle_snapshot(0);
        assert_eq!(
            records
                .iter()
                .map(|record| record.event)
                .collect::<Vec<_>>(),
            vec!["spawned", "spawned", "replaced"]
        );
        assert_eq!(last_id, 3);
        let replaced = records.last().expect("replaced record");
        assert!(replaced.stderr_tail.contains("stub-daemon startup"));
        assert!(!replaced.stderr_tail.contains("sk-stub-stderr-secret"));
        assert!(replaced.stderr_tail.contains("api_key=[redacted]"));

        let _ = supervisor.invoke_inner(
            "daemon.shutdown",
            None,
            false,
            Some(json!("shutdown-replaced-record")),
            |_| {},
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn set_data_root_same_root_reuses_existing_daemon() {
        let (dir, script) = write_stub_daemon();
        let sidecar = dir.join(sidecar_filename().expect("sidecar name"));
        fs::rename(&script, &sidecar).expect("install stub sidecar");
        let supervisor = DaemonSupervisor::new(Some(dir.clone()));
        let data_root = dir.join("imported-data");
        fs::create_dir_all(&data_root).expect("create data root");

        supervisor
            .set_data_root(data_root.clone())
            .expect("warm replacement daemon");
        let first = supervisor
            .invoke_inner("fast", None, false, Some(json!("fast-before-noop")), |_| {})
            .expect("first daemon response");
        let first_pid = response_pid(&first);

        supervisor
            .set_data_root(data_root.clone())
            .expect("same root is a no-op");
        let second = supervisor
            .invoke_inner("fast", None, false, Some(json!("fast-after-noop")), |_| {})
            .expect("second daemon response");

        assert_eq!(response_pid(&second), first_pid);
        assert_eq!(
            supervisor.data_root.lock().expect("data root").clone(),
            Some(data_root)
        );

        let _ = supervisor.invoke_inner(
            "daemon.shutdown",
            None,
            false,
            Some(json!("shutdown-idempotent-root")),
            |_| {},
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[cfg(unix)]
    fn set_data_root_keeps_existing_daemon_when_replacement_fails() {
        let (old_dir, old_script) = write_stub_daemon();
        let process = DaemonProcess::spawn_command(DaemonCommand {
            program: old_script,
            args: Vec::new(),
            cwd: old_dir.clone(),
            source: "env_python",
        })
        .expect("spawn old daemon");
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let sidecar_dir = env::temp_dir().join(format!(
            "kassiber-supervisor-bad-sidecar-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&sidecar_dir).expect("create bad sidecar dir");
        fs::write(
            sidecar_dir.join(sidecar_filename().expect("sidecar name")),
            b"not executable",
        )
        .expect("write bad sidecar");
        let supervisor = DaemonSupervisor {
            process: Mutex::new(Some(Arc::new(process))),
            resource_dir: Some(sidecar_dir.clone()),
            data_root: Mutex::new(None),
            next_request_id: AtomicU64::new(1),
            secret_store: Arc::new(NativeSecretStore),
            lifecycle: Mutex::new(VecDeque::new()),
            next_lifecycle_id: AtomicU64::new(1),
            event_sink: Arc::new(Mutex::new(None)),
            invoke_timeout: DAEMON_INVOKE_TIMEOUT,
            silence_kill_timeout: DAEMON_SILENCE_KILL_TIMEOUT,
        };

        let error = supervisor
            .set_data_root(sidecar_dir.join("imported-data"))
            .expect_err("replacement spawn should fail");
        assert_eq!(error.code, "daemon_spawn_failed");
        assert_eq!(
            supervisor.data_root.lock().expect("data root").clone(),
            None
        );
        let response = supervisor
            .invoke_inner("fast", None, false, Some(json!("fast-after-fail")), |_| {})
            .expect("old daemon still handles requests");
        assert_eq!(response.get("kind").and_then(Value::as_str), Some("fast"));

        let _ = supervisor.invoke_inner(
            "daemon.shutdown",
            None,
            false,
            Some(json!("shutdown-old")),
            |_| {},
        );
        let _ = fs::remove_dir_all(old_dir);
        let _ = fs::remove_dir_all(sidecar_dir);
    }
}

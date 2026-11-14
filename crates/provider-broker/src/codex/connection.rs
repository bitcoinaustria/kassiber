//! `CodexConnection` in `codex.ts`: JSON-RPC over `codex app-server
//! --stdio`.
//!
//! A detached reader thread owns the app-server's stdout. It answers the
//! server's own requests (Kassiber's dynamic tools through the turn's tool
//! queue, everything else refused), settles pending requests by id, and
//! hands notifications to the registered listener. When stdout ends and the
//! process has exited (Node's `close`), every pending request and the turn
//! fail with the app-server's redacted stderr or its exit code.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{ChildStdin, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use serde_json::value::RawValue;
use serde_json::{Map, Value};

use super::wire::Json;
use crate::cleanup::{track_child, TrackedChild};
use crate::env::provider_environment;
use crate::executables::{provider_command, spawn_failure};
use crate::protocol::{safe_error_message, BrokerFailure, ProviderId};

/// How much stderr the close message may draw on (`stderrPreview`).
const STDERR_PREVIEW: usize = 1_000;

/// A request's answer: its `result`, or the failure message.
pub type Answer = Result<Value, String>;

/// How a turn ended: `turn/completed`'s status and error, or a rejection.
#[derive(Clone, Debug)]
pub struct TurnEnd {
    pub status: Value,
    pub error: Value,
}

/// A dynamic-tool call the app-server asked Kassiber to run.
pub struct ToolCall {
    pub id: Value,
    pub name: String,
    pub call_id: String,
    pub arguments: Box<RawValue>,
}

/// What the turn's tool queue receives.
pub enum ToolMessage {
    Call(ToolCall),
    /// The turn is over; stop taking calls.
    Stop,
}

type Listener = Box<dyn FnMut(&Map<String, Value>, &Settle) + Send>;

#[derive(Default)]
struct State {
    pending: HashMap<u64, Sender<Answer>>,
    closed: Option<String>,
    turn: Option<Result<TurnEnd, String>>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    listener: Mutex<Option<Listener>>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// Settles the turn's completion promise; the first settlement wins.
pub struct Settle {
    shared: Arc<Shared>,
}

impl Settle {
    fn settle(&self, outcome: Result<TurnEnd, String>) {
        let mut state = self.shared.state();
        if state.turn.is_none() {
            state.turn = Some(outcome);
            self.shared.changed.notify_all();
        }
    }

    pub fn resolve(&self, end: TurnEnd) {
        self.settle(Ok(end));
    }

    pub fn reject(&self, message: &str) {
        self.settle(Err(message.to_owned()));
    }
}

/// The app-server's stdin, shared by the requests and the tool replies.
#[derive(Clone)]
pub struct Stdin(Arc<Mutex<Option<ChildStdin>>>);

impl Stdin {
    /// `send(message)`: one JSON line. A gone app-server is noticed by the
    /// reader, so a failed write is not reported here.
    pub fn send(&self, message: &Json) {
        let mut line = message.to_json();
        line.push('\n');
        let mut stdin = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(line.as_bytes());
            let _ = stdin.flush();
        }
    }
}

pub struct Connection {
    shared: Arc<Shared>,
    stdin: Stdin,
    child: Arc<TrackedChild>,
    next_id: AtomicU64,
}

impl Connection {
    /// Start `codex app-server --stdio` with `extra` arguments. Server tool
    /// calls go to `tools`; without it they are refused like every other
    /// provider-native request.
    pub fn start(
        executable: &Path,
        cwd: &Path,
        extra: &[String],
        sensitive: bool,
        tools: Option<Sender<ToolMessage>>,
    ) -> Result<Self, BrokerFailure> {
        let mut args = vec!["app-server".to_owned(), "--stdio".to_owned()];
        args.extend_from_slice(extra);
        let mut env = provider_environment(ProviderId::Codex);
        if sensitive {
            env.retain(|(name, _)| name != "RUST_LOG");
            env.push((OsString::from("RUST_LOG"), OsString::from("off")));
        }
        let mut command =
            provider_command(ProviderId::Codex, executable, &args, Some(cwd), Some(&env));
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| spawn_failure(executable, &error))?;
        let stdin = Stdin(Arc::new(Mutex::new(child.stdin.take())));
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let child = Arc::new(track_child(child));
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            listener: Mutex::new(None),
        });
        let preview = std::thread::Builder::new()
            .name("codex-stderr".into())
            .spawn(move || stderr.map(read_preview).unwrap_or_default())
            .ok();
        let reader = Reader {
            shared: Arc::clone(&shared),
            stdin: stdin.clone(),
            tools,
        };
        let waited = Arc::clone(&child);
        let started = std::thread::Builder::new()
            .name("codex-stdout".into())
            .spawn(move || {
                if let Some(stdout) = stdout {
                    reader.read(stdout);
                }
                let preview = preview
                    .and_then(|thread| thread.join().ok())
                    .unwrap_or_default();
                let code = waited.wait().ok().and_then(|status| status.code());
                reader.close(&preview, code);
            });
        if started.is_err() {
            let _ = child.kill();
            return Err(BrokerFailure::new(
                "Kassiber could not read the Codex app-server.",
            ));
        }
        Ok(Self {
            shared,
            stdin,
            child,
            next_id: AtomicU64::new(1),
        })
    }

    /// The stdin handle tool replies are written through.
    pub fn stdin(&self) -> Stdin {
        self.stdin.clone()
    }

    /// Send a request; its answer arrives on the receiver. A closed
    /// connection answers at once.
    pub fn request_async(&self, method: &str, params: Json) -> Receiver<Answer> {
        let (sender, receiver) = mpsc::channel();
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        {
            let mut state = self.shared.state();
            if let Some(closed) = &state.closed {
                let _ = sender.send(Err(closed.clone()));
                return receiver;
            }
            state.pending.insert(id, sender);
        }
        let id = i64::try_from(id).unwrap_or(i64::MAX);
        self.stdin.send(
            &Json::obj()
                .with("id", id)
                .with("method", method)
                .with("params", params),
        );
        receiver
    }

    /// `request(method, params)`: send and wait for the answer.
    pub fn request(&self, method: &str, params: Json) -> Result<Value, BrokerFailure> {
        self.request_async(method, params)
            .recv()
            .unwrap_or_else(|_| Err("Codex request failed".to_owned()))
            .map_err(BrokerFailure::new)
    }

    /// `notify(method)` without params.
    pub fn notify(&self, method: &str) {
        self.stdin.send(&Json::obj().with("method", method));
    }

    /// `onNotification(listener)`: from now on every notification goes to
    /// `listener`; earlier ones were dropped, as with no TypeScript listener.
    pub fn on_notification(
        &self,
        listener: impl FnMut(&Map<String, Value>, &Settle) + Send + 'static,
    ) {
        *self
            .shared
            .listener
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(Box::new(listener));
    }

    /// `Promise.race([completion, connection.closed])`: the turn's
    /// settlement, or the close error if the app-server went first.
    pub fn wait_turn(&self) -> Result<TurnEnd, BrokerFailure> {
        let mut state = self.shared.state();
        loop {
            if let Some(turn) = &state.turn {
                return turn.clone().map_err(BrokerFailure::new);
            }
            if let Some(closed) = &state.closed {
                return Err(BrokerFailure::new(closed.clone()));
            }
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(|poison| poison.into_inner());
        }
    }

    /// `close()`: SIGTERM the app-server. Nothing it says afterwards reaches
    /// the turn's listener.
    pub fn close(&self) {
        *self
            .shared
            .listener
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
        #[cfg(unix)]
        let _ = self.child.terminate();
        #[cfg(not(unix))]
        let _ = self.child.kill();
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.close();
    }
}

/// The first [`STDERR_PREVIEW`] bytes of stderr; the rest is read and
/// dropped so the app-server never blocks on a full pipe.
fn read_preview(mut stderr: impl Read) -> String {
    let mut kept = Vec::new();
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                if kept.len() < STDERR_PREVIEW {
                    kept.extend_from_slice(&chunk[..read]);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

/// Node readline's lines: split on `\n`, `\r\n` and a lone `\r`, decoded
/// lossily. Stops when `each` returns false.
pub fn for_each_line(stdout: impl Read, mut each: impl FnMut(&str) -> bool) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                let text = String::from_utf8_lossy(&line);
                for part in text.split('\r') {
                    if !each(part) {
                        return;
                    }
                }
            }
        }
    }
}

/// A JSON number that is an integer, as a request id.
fn integer_id(value: &Value) -> Option<u64> {
    if let Some(id) = value.as_u64() {
        return Some(id);
    }
    let number = value.as_f64()?;
    (number.fract() == 0.0 && (0.0..9_007_199_254_740_992.0).contains(&number))
        .then_some(number as u64)
}

struct Reader {
    shared: Arc<Shared>,
    stdin: Stdin,
    tools: Option<Sender<ToolMessage>>,
}

impl Reader {
    fn read(&self, stdout: impl Read) {
        for_each_line(stdout, |line| {
            let Ok(Value::Object(message)) = serde_json::from_str::<Value>(line) else {
                return true;
            };
            self.dispatch(line, &message);
            true
        });
    }

    fn dispatch(&self, line: &str, message: &Map<String, Value>) {
        let method = message.get("method");
        if let Some(id) = message.get("id") {
            if method.is_some_and(crate::protocol::truthy) {
                self.server_request(line, id, method.and_then(Value::as_str), message);
                return;
            }
        }
        if let Some(id) = message.get("id").filter(|id| id.is_number()) {
            let Some(id) = integer_id(id) else {
                return;
            };
            let Some(waiter) = self.shared.state().pending.remove(&id) else {
                return;
            };
            let answer = match message.get("error") {
                Some(error) if crate::protocol::truthy(error) => Err(error
                    .get("message")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                    .unwrap_or("Codex request failed")
                    .to_owned()),
                _ => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = waiter.send(answer);
            return;
        }
        let settle = Settle {
            shared: Arc::clone(&self.shared),
        };
        let mut listener = self
            .shared
            .listener
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(listener) = listener.as_mut() {
            listener(message, &settle);
        }
    }

    /// A request from the app-server: Kassiber's dynamic tools go to the
    /// turn's queue, anything else is refused.
    fn server_request(
        &self,
        line: &str,
        id: &Value,
        method: Option<&str>,
        message: &Map<String, Value>,
    ) {
        if method == Some("item/tool/call") {
            if let Some(tools) = &self.tools {
                let params = message.get("params");
                let text = |name: &str| {
                    params
                        .and_then(|params| params.get(name))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                let call = ToolCall {
                    id: id.clone(),
                    name: text("tool"),
                    call_id: text("callId"),
                    arguments: raw_arguments(line),
                };
                if tools.send(ToolMessage::Call(call)).is_ok() {
                    return;
                }
                self.stdin.send(&denied(id));
                return;
            }
        }
        self.stdin.send(
            &Json::obj().with("id", Json::Value(id.clone())).with(
                "error",
                Json::obj()
                    .with("code", -32001)
                    .with("message", "Provider-native tools are disabled by Kassiber."),
            ),
        );
    }

    /// The `close` handler: fail everything still waiting.
    fn close(&self, stderr: &str, code: Option<i32>) {
        let detail = safe_error_message(stderr);
        let message = if !detail.is_empty() && detail != "Provider request failed." {
            detail
        } else {
            format!(
                "Codex app-server exited with code {}.",
                code.map_or_else(|| "null".to_owned(), |code| code.to_string())
            )
        };
        let mut state = self.shared.state();
        for (_, waiter) in state.pending.drain() {
            let _ = waiter.send(Err(message.clone()));
        }
        state.closed = Some(message);
        self.shared.changed.notify_all();
    }
}

/// The reply to a tool call the bridge refused.
pub fn denied(id: &Value) -> Json {
    tool_reply(id, Json::from("Kassiber denied the tool call."), false)
}

/// `{ id, result: { contentItems: [{ type: "inputText", text }], success } }`.
pub fn tool_reply(id: &Value, text: Json, success: bool) -> Json {
    Json::obj().with("id", Json::Value(id.clone())).with(
        "result",
        Json::obj()
            .with(
                "contentItems",
                vec![Json::obj().with("type", "inputText").with("text", text)],
            )
            .with("success", success),
    )
}

/// `params.arguments` in the line's own key order when it is an object (or
/// an array, which JavaScript's `typeof` also calls one), else `{}`. The
/// last duplicate key wins, as in `JSON.parse`.
fn raw_arguments(line: &str) -> Box<RawValue> {
    fn object(raw: &str) -> Option<HashMap<String, Box<RawValue>>> {
        if !raw.trim_start().starts_with('{') {
            return None;
        }
        serde_json::from_str(raw).ok()
    }
    object(line)
        .and_then(|message| object(message.get("params")?.get()))
        .and_then(|params| params.get("arguments").cloned())
        .filter(|arguments| matches!(arguments.get().as_bytes().first(), Some(b'{' | b'[')))
        .unwrap_or_else(|| RawValue::from_string("{}".to_owned()).expect("valid JSON"))
}

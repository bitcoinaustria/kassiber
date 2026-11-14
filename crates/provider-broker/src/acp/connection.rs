//! A hand-rolled ACP client connection: newline-delimited JSON-RPC 2.0 over
//! an agent's stdin and stdout, as `@agentclientprotocol/sdk` 1.5's
//! `ndJsonStream` and `Connection` speak it for a stable (batch-free) client.
//!
//! What the SDK did on the wire carries over: request ids count from 0, a
//! line that is not JSON is answered with a `Parse error` (id `null`), a
//! non-object or malformed call with `Invalid request`, a JSON-RPC batch
//! closes the connection, a request no handler takes is answered with
//! `Method not found`, a response to an unknown id is ignored, and the end
//! of the agent's stdout closes the connection and fails every pending
//! request with `ACP connection closed`.
//!
//! The adapter's [`Handler`] runs on the reader thread, one message at a
//! time and in order. Once it reports a native-tool attempt
//! ([`Shared::violation`]) the reader answers nothing more but the request
//! it is handling, so nothing the agent sends after a tripwire reaches the
//! event stream.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::value::RawValue;
use serde_json::Value;

use crate::cleanup::{track_child, TrackedChild};
use crate::executables::{node_code, provider_command, spawn_failure};
use crate::protocol::{js_trim, BrokerFailure, ProviderId};

/// How long cleanup waits for a stopped agent to exit. On Windows a process
/// still holding the working directory makes its removal fail.
pub const EXIT_WAIT: Duration = Duration::from_secs(5);

/// The stderr kept for an early-exit message (`stderrPreview.length < 4_096`).
const STDERR_PREVIEW: usize = 4_096;

/// How often a waiting request looks at the agent process.
const POLL: Duration = Duration::from_millis(10);

/// How long an early-exit message waits for the rest of stderr.
const STDERR_DRAIN: Duration = Duration::from_millis(250);

/// After the agent exits, how long a chat waits for its stdout to end before
/// it reports the exit instead. Node raced the two; the end of the stream
/// won whenever the agent itself held stdout (checked against the
/// TypeScript broker), so the exit and its stderr are reported only when
/// something else keeps the stream open.
const CLOSE_AFTER_EXIT: Duration = Duration::from_millis(250);

/// `ACP connection closed`: the SDK's reason when the stream ends.
pub const CLOSED: &str = "ACP connection closed";

/// A JSON-RPC error answer (`RequestError`): its code and message.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub code: f64,
    pub message: String,
}

/// Why a request did not return a result.
#[derive(Clone, Debug, PartialEq)]
pub enum Wait {
    /// The agent answered with an error.
    Rpc(RpcError),
    /// The connection closed (stdout ended, a write failed, a batch arrived).
    Closed(String),
    /// The adapter's tripwire fired.
    Violation,
    /// The deadline passed.
    Timeout,
    /// The agent process exited (chat only).
    Exited,
}

/// What a handler answers an agent's request with.
pub enum Reply {
    /// A JSON-RPC result, already serialized.
    Result(String),
    /// A JSON-RPC error.
    Error {
        code: i64,
        message: String,
        data: Option<String>,
    },
}

/// The client side of the session: requests and notifications the agent
/// sends. Runs on the reader thread.
pub trait Handler: Send + Sync {
    /// An agent request; `None` means no handler takes it (`Method not
    /// found`).
    fn request(&self, shared: &Shared, method: &str, params: Option<&Value>) -> Option<Reply>;
    /// An agent notification.
    fn notification(&self, shared: &Shared, method: &str, params: Option<&Value>);
    /// The answer to one of the client's requests, seen before the waiting
    /// caller gets it and before the next message is read.
    fn response(&self, _id: u64, _answer: &Result<Value, RpcError>) {}
}

#[derive(Default)]
struct State {
    /// Requests sent and not yet answered; an answer to any other id is
    /// ignored, as the SDK ignores a response to an unknown request.
    pending: HashSet<u64>,
    responses: HashMap<u64, Result<Value, RpcError>>,
    closed: Option<String>,
}

/// What the reader thread, the stderr thread and the adapter share.
pub struct Shared {
    child: TrackedChild,
    stdin: Mutex<Option<ChildStdin>>,
    state: Mutex<State>,
    changed: Condvar,
    violation: AtomicBool,
    stderr: Mutex<String>,
    /// The agent's stderr has ended (or was never piped).
    stderr_done: AtomicBool,
    next_id: AtomicU64,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn close(&self, reason: &str) {
        let mut state = self.state();
        if state.closed.is_none() {
            state.closed = Some(reason.to_owned());
        }
        drop(state);
        self.changed.notify_all();
    }

    fn is_closed(&self) -> bool {
        self.state().closed.is_some()
    }

    /// Write one JSON line to the agent. A failed write closes the
    /// connection, as the SDK's write queue does.
    fn write_line(&self, line: &str) -> Result<(), String> {
        let mut stdin = self
            .stdin
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(pipe) = stdin.as_mut() else {
            return Err(CLOSED.to_owned());
        };
        let written = pipe
            .write_all(line.as_bytes())
            .and_then(|()| pipe.write_all(b"\n"))
            .and_then(|()| pipe.flush());
        drop(stdin);
        written.map_err(|error| {
            let reason = format!("write {}", node_code(&error));
            self.close(&reason);
            reason
        })
    }

    /// The tripwire: remember the attempt and ask the agent to exit at
    /// once. Waiting requests return [`Wait::Violation`].
    pub fn violation(&self) {
        if self.violation.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.signal_stop();
        self.changed.notify_all();
    }

    pub fn violated(&self) -> bool {
        self.violation.load(Ordering::SeqCst)
    }

    /// `child.kill()`: SIGTERM on Unix, `TerminateProcess` on Windows. An
    /// exited agent is left alone.
    fn signal_stop(&self) -> io::Result<()> {
        #[cfg(unix)]
        return self.child.terminate();
        #[cfg(not(unix))]
        return self.child.kill();
    }

    fn respond(&self, id: &Value, reply: Reply) {
        let id = serde_json::to_string(id).unwrap_or_else(|_| "null".into());
        let line = match reply {
            Reply::Result(result) => format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#),
            Reply::Error {
                code,
                message,
                data,
            } => error_line(&id, code, &message, data.as_deref()),
        };
        let _ = self.write_line(&line);
    }
}

fn error_line(id: &str, code: i64, message: &str, data: Option<&str>) -> String {
    let message = serde_json::to_string(message).unwrap_or_else(|_| "\"\"".into());
    match data {
        Some(data) => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":{code},"message":{message},"data":{data}}}}}"#
        ),
        None => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":{code},"message":{message}}}}}"#
        ),
    }
}

/// A running agent and its connection.
pub struct AgentProcess {
    shared: Arc<Shared>,
}

/// `startAgent(executable, launch, cwd)`: start the agent with piped
/// standard streams in `cwd`, tracked for termination, and serve its
/// messages to `handler`.
pub fn start_agent(
    provider: ProviderId,
    executable: &Path,
    args: &[String],
    env: &[(OsString, OsString)],
    cwd: &Path,
    handler: Arc<dyn Handler>,
) -> Result<AgentProcess, BrokerFailure> {
    let mut command = provider_command(provider, executable, args, Some(cwd), Some(env));
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| spawn_failure(executable, &error))?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let shared = Arc::new(Shared {
        child: track_child(child),
        stdin: Mutex::new(stdin),
        state: Mutex::new(State::default()),
        changed: Condvar::new(),
        violation: AtomicBool::new(false),
        stderr: Mutex::new(String::new()),
        stderr_done: AtomicBool::new(stderr.is_none()),
        next_id: AtomicU64::new(0),
    });
    if let Some(mut stderr) = stderr {
        let shared = Arc::clone(&shared);
        let _ = std::thread::Builder::new()
            .name("acp-stderr".into())
            .spawn(move || {
                let mut chunk = vec![0u8; 16 * 1024];
                loop {
                    match stderr.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(read) => {
                            let mut preview = shared
                                .stderr
                                .lock()
                                .unwrap_or_else(|poison| poison.into_inner());
                            if preview.encode_utf16().count() < STDERR_PREVIEW {
                                preview.push_str(&String::from_utf8_lossy(&chunk[..read]));
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                shared.stderr_done.store(true, Ordering::SeqCst);
            });
    }
    match stdout {
        Some(stdout) => {
            let reader_shared = Arc::clone(&shared);
            let spawned = std::thread::Builder::new()
                .name("acp-reader".into())
                .spawn(move || read_messages(&reader_shared, stdout, handler.as_ref()));
            if spawned.is_err() {
                shared.close(CLOSED);
            }
        }
        None => shared.close(CLOSED),
    }
    Ok(AgentProcess { shared })
}

/// The reader: one message per line until stdout ends.
fn read_messages(shared: &Shared, stdout: impl Read, handler: &dyn Handler) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
        if shared.is_closed() || shared.violated() {
            // A stopped turn reads on only so the agent never blocks on a
            // full pipe while it exits.
            continue;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        let text = String::from_utf8_lossy(&line);
        let text = js_trim(&text);
        if text.is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            let _ = shared.write_line(&error_line("null", -32700, "Parse error", None));
            continue;
        };
        receive(shared, message, handler);
    }
    shared.close(CLOSED);
}

fn is_id(value: &Value) -> bool {
    match value {
        Value::Null | Value::String(_) => true,
        Value::Number(number) => number.as_f64().is_some_and(f64::is_finite),
        _ => false,
    }
}

/// `receiveWireMessage` and `receiveMessage` for one parsed line.
fn receive(shared: &Shared, message: Value, handler: &dyn Handler) {
    let object = match message {
        Value::Object(object) => object,
        Value::Array(_) => {
            shared.close("JSON-RPC batches are not supported on this connection");
            return;
        }
        other => {
            let data = serde_json::to_string(&other).unwrap_or_else(|_| "null".into());
            let _ = shared.write_line(&error_line("null", -32600, "Invalid request", Some(&data)));
            return;
        }
    };
    let envelope = object.get("jsonrpc").and_then(Value::as_str) == Some("2.0");
    let method = object.get("method");
    let id = object.get("id");
    let is_request = envelope && id.is_some_and(is_id) && method.is_some_and(Value::is_string);
    let is_notification = envelope && id.is_none() && method.is_some_and(Value::is_string);
    let response_shaped = method.is_none()
        && (id.is_some() || object.contains_key("result") || object.contains_key("error"));
    if !is_request && !is_notification && !response_shaped {
        let data = serde_json::to_string(&object).unwrap_or_else(|_| "null".into());
        let _ = shared.write_line(&error_line("null", -32600, "Invalid request", Some(&data)));
        return;
    }
    let params = object.get("params");
    if let Some(method) = method.and_then(Value::as_str) {
        match id {
            Some(id) => {
                let reply = handler.request(shared, method, params).unwrap_or_else(|| {
                    let data = serde_json::to_string(&serde_json::json!({ "method": method }))
                        .unwrap_or_else(|_| "{}".into());
                    Reply::Error {
                        code: -32601,
                        message: format!("\"Method not found\": {method}"),
                        data: Some(data),
                    }
                });
                shared.respond(id, reply);
            }
            None => handler.notification(shared, method, params),
        }
        return;
    }
    // A response, or something shaped like one.
    let Some(id) = id.and_then(Value::as_u64) else {
        return;
    };
    let answer = response_answer(&object, envelope);
    let pending = {
        let mut state = shared.state();
        state.closed.is_none() && state.pending.remove(&id)
    };
    if !pending {
        return;
    }
    // Before any later message is read, so state the answer establishes
    // (the session id) is in place for the updates that follow it.
    handler.response(id, &answer);
    shared.state().responses.insert(id, answer);
    shared.changed.notify_all();
}

/// `handleResponse`: the result, the error, or `Invalid request` for a
/// malformed response.
fn response_answer(
    object: &serde_json::Map<String, Value>,
    envelope: bool,
) -> Result<Value, RpcError> {
    let invalid = || RpcError {
        code: -32600.0,
        message: "Invalid request".into(),
    };
    let id_ok = object.get("id").is_some_and(is_id);
    let result = object.get("result");
    let error = object.get("error");
    if !envelope || !id_ok || result.is_some() == error.is_some() {
        return Err(invalid());
    }
    if let Some(result) = result {
        return Ok(result.clone());
    }
    let error = error.and_then(Value::as_object);
    let code = error
        .and_then(|error| error.get("code"))
        .and_then(Value::as_f64);
    let message = error
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str);
    match (code, message) {
        (Some(code), Some(message)) => Err(RpcError {
            code,
            message: message.to_owned(),
        }),
        _ => Err(invalid()),
    }
}

/// `{"jsonrpc":"2.0","id":N,"method":...,"params":...}` in the SDK's key
/// order.
#[derive(Serialize)]
struct Request<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: &'a RawValue,
}

impl AgentProcess {
    /// What the agent wrote to stderr, up to about 4 KiB. After an exit it
    /// waits briefly for the pipe to drain, as Node delivered stderr before
    /// the exit event.
    pub fn stderr(&self) -> String {
        let deadline = Instant::now() + STDERR_DRAIN;
        while !self.shared.stderr_done.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(POLL);
        }
        self.shared
            .stderr
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// Send a request. `params` is serialized JSON. `before` learns the id
    /// before the request is written, so a handler can recognise its answer.
    pub fn send(&self, method: &str, params: &str, before: impl FnOnce(u64)) -> Result<u64, Wait> {
        if let Some(reason) = self.shared.state().closed.clone() {
            return Err(Wait::Closed(reason));
        }
        let id = self.shared.next_id.fetch_add(1, Ordering::SeqCst);
        before(id);
        self.shared.state().pending.insert(id);
        let params = RawValue::from_string(params.to_owned())
            .map_err(|_| Wait::Closed("Kassiber could not encode an ACP request.".into()))?;
        let line = serde_json::to_string(&Request {
            jsonrpc: "2.0",
            id,
            method,
            params: &params,
        })
        .map_err(|_| Wait::Closed("Kassiber could not encode an ACP request.".into()))?;
        self.shared.write_line(&line).map_err(Wait::Closed)?;
        Ok(id)
    }

    /// Wait for the answer to request `id`.
    ///
    /// The tripwire wins over anything else, then the answer, then a closed
    /// connection. With `watch_exit` an agent that exited while its stdout
    /// stays open ends the wait too (`exitedEarly` in the chat).
    pub fn wait(
        &self,
        id: u64,
        deadline: Option<Instant>,
        watch_exit: bool,
    ) -> Result<Value, Wait> {
        let mut state = self.shared.state();
        let mut exited_at: Option<Instant> = None;
        loop {
            if self.shared.violated() {
                return Err(Wait::Violation);
            }
            if let Some(answer) = state.responses.remove(&id) {
                return answer.map_err(Wait::Rpc);
            }
            if let Some(reason) = &state.closed {
                return Err(Wait::Closed(reason.clone()));
            }
            if watch_exit
                && exited_at.is_none()
                && matches!(self.shared.child.try_wait(), Ok(Some(_)))
            {
                exited_at = Some(Instant::now());
            }
            if exited_at.is_some_and(|at| at.elapsed() >= CLOSE_AFTER_EXIT) {
                return Err(Wait::Exited);
            }
            let now = Instant::now();
            if deadline.is_some_and(|deadline| now >= deadline) {
                return Err(Wait::Timeout);
            }
            let slice = deadline.map_or(POLL, |deadline| (deadline - now).min(POLL));
            state = self
                .shared
                .changed
                .wait_timeout(state, slice)
                .unwrap_or_else(|poison| poison.into_inner())
                .0;
        }
    }

    /// `send` then `wait`.
    pub fn request(
        &self,
        method: &str,
        params: &str,
        deadline: Option<Instant>,
        watch_exit: bool,
    ) -> Result<Value, Wait> {
        let id = self.send(method, params, |_| {})?;
        self.wait(id, deadline, watch_exit)
    }

    /// `agent.stop()`: ask the agent to exit, wait up to [`EXIT_WAIT`] for
    /// it, then close its stdin. No harder kill follows, as in the
    /// TypeScript broker; termination cleanup still has it tracked.
    pub fn stop(&self) {
        let _ = self.shared.signal_stop();
        let _ = self.shared.child.wait_timeout(EXIT_WAIT);
        self.shared.close(CLOSED);
        drop(
            self.shared
                .stdin
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .take(),
        );
    }
}

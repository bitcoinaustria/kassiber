//! The dual-era MCP server over stdio (`kassiber/mcp/protocol.py`).
//!
//! Modern clients (revision 2026-07-28) send self-contained requests that
//! carry their protocol version and capabilities in `_meta`; there is no
//! handshake and no session. Legacy clients (2025-11-25 and 2025-06-18) open
//! with `initialize`, which selects legacy semantics for the rest of the
//! process. The era is chosen per message from how the client speaks.
//!
//! [`StdioLoop`] answers fast methods on the reader thread and runs tool
//! calls one at a time on a worker, so replies can arrive out of order. A
//! cancelled call gets no reply. At end of input, accepted calls get
//! [`SHUTDOWN_DRAIN`] to finish and be answered.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kassiber_envelope::{contains_key, dict, get, s};
use kassiber_pyjson::{dumps, BigInt, Key, Options, PyStr, Value};

pub const MODERN_PROTOCOL_VERSIONS: [&str; 1] = ["2026-07-28"];
/// Newest first: an unknown legacy request is offered the first entry.
/// 2025-03-26 is not offered: that revision requires JSON-RPC batches.
pub const LEGACY_PROTOCOL_VERSIONS: [&str; 2] = ["2025-11-25", "2025-06-18"];
/// `structuredContent` and top-level tool `title` arrived in 2025-06-18.
const LEGACY_STRUCTURED_VERSIONS: [&str; 2] = ["2025-11-25", "2025-06-18"];

pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
pub const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
pub const META_SUBSCRIPTION_ID: &str = "io.modelcontextprotocol/subscriptionId";

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
/// JSON-RPC implementation-defined server error: too much work in flight.
pub const SERVER_OVERLOADED: i64 = -32000;

pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
/// Tool definitions and instructions are static per server version and
/// carry no book data, so any client or cache may keep them.
pub const STATIC_LIST_TTL_MS: i64 = 60_000;
/// Calls run one at a time; a client pipelining more than this many is
/// refused rather than queued without bound.
pub const MAX_INFLIGHT_CALLS: usize = 32;
/// Open `subscriptions/listen` streams; each holds an entry until stdin ends.
pub const MAX_SUBSCRIPTIONS: usize = 16;
/// After stdin closes, accepted calls get this long to finish and be answered.
pub const SHUTDOWN_DRAIN: Duration = Duration::from_secs(30);

/// A JSON-RPC protocol error (not a tool failure, which is `isError`).
#[derive(Clone, Debug, PartialEq)]
pub struct McpError {
    pub code: i64,
    pub message: PyStr,
    pub data: Option<Value>,
}

impl McpError {
    fn new(code: i64, message: impl Into<PyStr>) -> Self {
        McpError {
            code,
            message: message.into(),
            data: None,
        }
    }

    fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// Python's `except Exception` around a call: the reply is `Internal error`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Internal;

/// A call's cancellation flag (`threading.Event`), shared with the reader.
#[derive(Clone, Debug, Default)]
pub struct Cancelled(Arc<AtomicBool>);

impl Cancelled {
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn set(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// One tool call's result: the structured value, its text block, and
/// whether it is a tool error.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutcome {
    pub structured: Value,
    pub text: PyStr,
    pub is_error: bool,
}

/// What the server serves. It knows nothing about Kassiber.
pub trait ToolProvider: Send + Sync {
    /// Read when served: it follows the provider's current configuration.
    fn instructions(&self) -> String;
    /// Era-neutral definitions; the server drops fields per era.
    fn tool_definitions(&self) -> Vec<Value>;
    fn has_tool(&self, name: &PyStr) -> bool;
    /// `client_name` is the host's self-reported name, if it sent one
    /// (display only, never authority).
    fn call_tool(
        &self,
        name: &PyStr,
        arguments: &Value,
        cancelled: &Cancelled,
        client_name: Option<&Value>,
    ) -> Result<ToolOutcome, Internal>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Era {
    modern: bool,
    version: Option<&'static str>,
}

impl Era {
    fn structured(self) -> bool {
        self.modern
            || self
                .version
                .is_some_and(|version| LEGACY_STRUCTURED_VERSIONS.contains(&version))
    }
}

/// `_valid_request_id`: a string or an integer, never a bool or a float.
fn valid_request_id(value: &Value) -> bool {
    matches!(value, Value::Str(_) | Value::Int(_))
}

/// `_id_key`: `1` and `"1"` are different JSON-RPC ids.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IdKey {
    Int(BigInt),
    Str(PyStr),
}

pub fn id_key(value: &Value) -> Option<IdKey> {
    match value {
        Value::Int(n) => Some(IdKey::Int(n.clone())),
        Value::Str(text) => Some(IdKey::Str(text.clone())),
        _ => None,
    }
}

/// `_error_response(request_id, code, message, data)`.
pub fn error_response(request_id: &Value, code: i64, message: PyStr, data: Option<Value>) -> Value {
    let mut error = vec![
        (Key::from("code"), Value::from(code)),
        (Key::from("message"), Value::Str(message)),
    ];
    if let Some(data) = data {
        error.push((Key::from("data"), data));
    }
    let id = if valid_request_id(request_id) {
        request_id.clone()
    } else {
        Value::Null
    };
    dict([
        ("jsonrpc", s("2.0")),
        ("error", Value::Dict(error)),
        ("id", id),
    ])
}

/// `str(value)` keeping lone surrogates, for f-string messages.
fn py_str_value(value: Option<&Value>) -> PyStr {
    match value {
        Some(Value::Str(text)) => text.clone(),
        Some(other) => PyStr::from(kassiber_envelope::py_repr(other)),
        None => PyStr::from("None"),
    }
}

fn concat(prefix: &str, text: &PyStr) -> PyStr {
    PyStr::from_code_points(prefix.chars().map(u32::from).chain(text.code_points()))
        .unwrap_or_default()
}

fn str_list(items: &[&str]) -> Value {
    Value::List(items.iter().map(|item| s(*item)).collect())
}

fn is_text(value: Option<&Value>, text: &str) -> bool {
    matches!(value, Some(Value::Str(value)) if value.is(text))
}

/// `McpServer`: the request handler, with the one piece of process state a
/// legacy `initialize` leaves behind.
pub struct McpServer {
    provider: Arc<dyn ToolProvider>,
    server_info: Value,
    legacy_version: Mutex<Option<&'static str>>,
    /// The legacy handshake's `clientInfo.name`: display-only attribution.
    legacy_client_name: Mutex<Option<Value>>,
}

impl McpServer {
    pub fn new(provider: Arc<dyn ToolProvider>, name: &str, version: &str) -> Self {
        McpServer {
            provider,
            server_info: dict([("name", s(name)), ("version", s(version))]),
            legacy_version: Mutex::new(None),
            legacy_client_name: Mutex::new(None),
        }
    }

    /// `handle(message, cancelled)`: the response for one message, or
    /// `None` for notifications and stray replies.
    pub fn handle(&self, message: &Value, cancelled: Option<&Cancelled>) -> Option<Value> {
        let is_dict = matches!(message, Value::Dict(_));
        if !is_dict || !is_text(get(message, "jsonrpc"), "2.0") {
            let request_id = if is_dict {
                get(message, "id").cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            return Some(error_response(
                &request_id,
                INVALID_REQUEST,
                PyStr::from("Invalid JSON-RPC message"),
                None,
            ));
        }
        let Some(Value::Str(method)) = get(message, "method") else {
            if contains_key(message, "id")
                && (contains_key(message, "result") || contains_key(message, "error"))
            {
                // This server never issues requests, so replies are stray.
                return None;
            }
            let request_id = get(message, "id").cloned().unwrap_or(Value::Null);
            return Some(error_response(
                &request_id,
                INVALID_REQUEST,
                PyStr::from("Invalid JSON-RPC message"),
                None,
            ));
        };
        let request_id = get(message, "id")?;
        if !valid_request_id(request_id) {
            return Some(error_response(
                &Value::Null,
                INVALID_REQUEST,
                PyStr::from("Invalid request id"),
                None,
            ));
        }
        let params = match get(message, "params") {
            None | Some(Value::Null) => Value::Dict(Vec::new()),
            Some(params @ Value::Dict(_)) => params.clone(),
            Some(_) => {
                return Some(error_response(
                    request_id,
                    INVALID_PARAMS,
                    PyStr::from("Request params must be an object"),
                    None,
                ))
            }
        };
        let result = self
            .era_for(method, &params)
            .and_then(|era| self.dispatch(method, &params, era, cancelled));
        Some(match result {
            Ok(result) => dict([
                ("jsonrpc", s("2.0")),
                ("id", request_id.clone()),
                ("result", result),
            ]),
            Err(Failure::Protocol(error)) => {
                error_response(request_id, error.code, error.message, error.data)
            }
            Err(Failure::Internal) => error_response(
                request_id,
                INTERNAL_ERROR,
                PyStr::from("Internal error"),
                None,
            ),
        })
    }

    /// `open_subscription(message)`: acknowledge a modern
    /// `subscriptions/listen` stream. Nothing here changes at runtime, so
    /// the honored filter is empty.
    pub fn open_subscription(&self, message: &Value) -> Result<Value, McpError> {
        let params = match get(message, "params") {
            Some(params @ Value::Dict(_)) => params.clone(),
            _ => Value::Dict(Vec::new()),
        };
        let era = self
            .era_for(&PyStr::from("subscriptions/listen"), &params)
            .map_err(|failure| match failure {
                Failure::Protocol(error) => error,
                Failure::Internal => McpError::new(INTERNAL_ERROR, "Internal error"),
            })?;
        if !era.modern {
            return Err(McpError::new(
                METHOD_NOT_FOUND,
                "Method not found: subscriptions/listen",
            ));
        }
        Ok(dict([
            ("jsonrpc", s("2.0")),
            ("method", s("notifications/subscriptions/acknowledged")),
            (
                "params",
                dict([
                    (
                        "_meta",
                        dict([(
                            META_SUBSCRIPTION_ID,
                            get(message, "id").cloned().unwrap_or(Value::Null),
                        )]),
                    ),
                    ("notifications", Value::Dict(Vec::new())),
                ]),
            ),
        ]))
    }

    /// `close_subscription(request_id)`: the stream's final result.
    pub fn close_subscription(&self, request_id: &Value) -> Value {
        dict([
            ("jsonrpc", s("2.0")),
            ("id", request_id.clone()),
            (
                "result",
                dict([
                    ("resultType", s("complete")),
                    (
                        "_meta",
                        dict([
                            (META_SUBSCRIPTION_ID, request_id.clone()),
                            (META_SERVER_INFO, self.server_info.clone()),
                        ]),
                    ),
                ]),
            ),
        ])
    }

    fn era_for(&self, method: &PyStr, params: &Value) -> Result<Era, Failure> {
        if let Some(meta @ Value::Dict(_)) = get(params, "_meta") {
            if contains_key(meta, META_PROTOCOL_VERSION) {
                let version = get(meta, META_PROTOCOL_VERSION);
                let capabilities = get(meta, META_CLIENT_CAPABILITIES);
                let (Some(Value::Str(version)), Some(Value::Dict(_))) = (version, capabilities)
                else {
                    return Err(McpError::new(
                        INVALID_PARAMS,
                        "Request _meta requires protocolVersion and clientCapabilities",
                    )
                    .into());
                };
                let Some(known) = MODERN_PROTOCOL_VERSIONS
                    .iter()
                    .find(|known| version.is(known))
                else {
                    return Err(McpError::new(
                        UNSUPPORTED_PROTOCOL_VERSION,
                        "Unsupported protocol version",
                    )
                    .with_data(dict([
                        ("supported", str_list(&MODERN_PROTOCOL_VERSIONS)),
                        ("requested", Value::Str(version.clone())),
                    ]))
                    .into());
                };
                return Ok(Era {
                    modern: true,
                    version: Some(known),
                });
            }
        }
        if method.is("initialize") || method.is("ping") {
            // Legacy lifecycles allow ping at any time, even before initialize.
            return Ok(Era {
                modern: false,
                version: None,
            });
        }
        let legacy = *self
            .legacy_version
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match legacy {
            None => Err(McpError::new(
                INVALID_PARAMS,
                "Requests must carry protocolVersion and clientCapabilities in _meta, or follow an initialize handshake",
            )
            .with_data(dict([
                ("supported", str_list(&MODERN_PROTOCOL_VERSIONS)),
                ("legacy", str_list(&LEGACY_PROTOCOL_VERSIONS)),
            ]))
            .into()),
            Some(version) => Ok(Era {
                modern: false,
                version: Some(version),
            }),
        }
    }

    fn dispatch(
        &self,
        method: &PyStr,
        params: &Value,
        era: Era,
        cancelled: Option<&Cancelled>,
    ) -> Result<Value, Failure> {
        let not_found = || McpError::new(METHOD_NOT_FOUND, concat("Method not found: ", method));
        if era.modern {
            if method.is("server/discover") {
                let mut result = vec![
                    (
                        Key::from("supportedVersions"),
                        str_list(&MODERN_PROTOCOL_VERSIONS),
                    ),
                    (Key::from("capabilities"), self.capabilities()),
                    (Key::from("ttlMs"), Value::from(STATIC_LIST_TTL_MS)),
                    (Key::from("cacheScope"), s("public")),
                ];
                let instructions = self.provider.instructions();
                if !instructions.is_empty() {
                    result.push((Key::from("instructions"), s(instructions)));
                }
                return Ok(self.modern(result));
            }
            if method.is("tools/list") {
                reject_cursor(params)?;
                return Ok(self.modern(vec![
                    (Key::from("tools"), self.tools(era)),
                    (Key::from("ttlMs"), Value::from(STATIC_LIST_TTL_MS)),
                    (Key::from("cacheScope"), s("public")),
                ]));
            }
            if method.is("tools/call") {
                let Value::Dict(result) = self.call(params, era, cancelled)? else {
                    unreachable!("call results are objects");
                };
                return Ok(self.modern(result));
            }
            return Err(not_found().into());
        }
        if method.is("initialize") {
            return Ok(self.initialize(params));
        }
        if method.is("ping") {
            return Ok(Value::Dict(Vec::new()));
        }
        if method.is("tools/list") {
            reject_cursor(params)?;
            return Ok(dict([("tools", self.tools(era))]));
        }
        if method.is("tools/call") {
            return self.call(params, era, cancelled);
        }
        Err(not_found().into())
    }

    fn capabilities(&self) -> Value {
        dict([("tools", Value::Dict(Vec::new()))])
    }

    /// `_modern(result)`: `{**result, "resultType", "_meta"}`.
    fn modern(&self, mut result: Vec<(Key, Value)>) -> Value {
        result.push((Key::from("resultType"), s("complete")));
        result.push((
            Key::from("_meta"),
            dict([(META_SERVER_INFO, self.server_info.clone())]),
        ));
        Value::Dict(result)
    }

    fn initialize(&self, params: &Value) -> Value {
        let version = match get(params, "protocolVersion") {
            Some(Value::Str(requested)) => LEGACY_PROTOCOL_VERSIONS
                .iter()
                .find(|known| requested.is(known))
                .copied(),
            _ => None,
        }
        .unwrap_or(LEGACY_PROTOCOL_VERSIONS[0]);
        let client_name = match get(params, "clientInfo") {
            Some(info @ Value::Dict(_)) => {
                get(info, "name").filter(|name| matches!(name, Value::Str(_)))
            }
            _ => None,
        };
        *self
            .legacy_version
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(version);
        *self
            .legacy_client_name
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = client_name.cloned();
        let mut result = vec![
            (Key::from("protocolVersion"), s(version)),
            (Key::from("capabilities"), self.capabilities()),
            (Key::from("serverInfo"), self.server_info.clone()),
        ];
        let instructions = self.provider.instructions();
        if !instructions.is_empty() {
            result.push((Key::from("instructions"), s(instructions)));
        }
        Value::Dict(result)
    }

    fn tools(&self, era: Era) -> Value {
        let structured = era.structured();
        Value::List(
            self.provider
                .tool_definitions()
                .into_iter()
                .map(|definition| match definition {
                    Value::Dict(pairs) if !structured => Value::Dict(
                        pairs
                            .into_iter()
                            .filter(|(key, _)| {
                                *key != Key::from("title") && *key != Key::from("outputSchema")
                            })
                            .collect(),
                    ),
                    other => other,
                })
                .collect(),
        )
    }

    fn call(
        &self,
        params: &Value,
        era: Era,
        cancelled: Option<&Cancelled>,
    ) -> Result<Value, Failure> {
        let name = match get(params, "name") {
            Some(Value::Str(name)) if self.provider.has_tool(name) => name.clone(),
            other => {
                return Err(McpError::new(
                    INVALID_PARAMS,
                    concat("Unknown tool: ", &py_str_value(other)),
                )
                .into())
            }
        };
        let arguments = match get(params, "arguments") {
            None | Some(Value::Null) => Value::Dict(Vec::new()),
            Some(arguments @ Value::Dict(_)) => arguments.clone(),
            Some(_) => {
                return Err(
                    McpError::new(INVALID_PARAMS, "Tool arguments must be an object").into(),
                )
            }
        };
        let fresh = Cancelled::default();
        let client_name = self.client_name(params, era);
        let outcome = self
            .provider
            .call_tool(
                &name,
                &arguments,
                cancelled.unwrap_or(&fresh),
                client_name.as_ref(),
            )
            .map_err(|Internal| Failure::Internal)?;
        let mut result = vec![(
            Key::from("content"),
            Value::List(vec![dict([
                ("type", s("text")),
                ("text", Value::Str(outcome.text)),
            ])]),
        )];
        if era.structured() {
            result.push((Key::from("structuredContent"), outcome.structured));
        }
        if outcome.is_error {
            result.push((Key::from("isError"), Value::Bool(true)));
        }
        Ok(Value::Dict(result))
    }

    /// `_client_name(params, era)`: the host's self-reported name, if it
    /// sent one (never authority).
    fn client_name(&self, params: &Value, era: Era) -> Option<Value> {
        if era.modern {
            let info = match get(params, "_meta") {
                Some(meta @ Value::Dict(_)) => get(meta, META_CLIENT_INFO),
                _ => None,
            };
            return match info {
                Some(info @ Value::Dict(_)) => get(info, "name")
                    .filter(|name| matches!(name, Value::Str(_)))
                    .cloned(),
                _ => None,
            };
        }
        self.legacy_client_name
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

enum Failure {
    Protocol(McpError),
    Internal,
}

impl From<McpError> for Failure {
    fn from(error: McpError) -> Self {
        Failure::Protocol(error)
    }
}

/// `_reject_cursor`: every list fits in one page, so no cursor this server
/// issued exists.
fn reject_cursor(params: &Value) -> Result<(), McpError> {
    match get(params, "cursor") {
        None | Some(Value::Null) => Ok(()),
        Some(_) => Err(McpError::new(INVALID_PARAMS, "Invalid cursor")),
    }
}

/// `bytes.strip()` leaves nothing: only ASCII whitespace.
fn blank(line: &[u8]) -> bool {
    line.iter()
        .all(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
}

/// `readline(limit)`: up to `limit` bytes, through the first newline.
/// `None` at end of input (a read error ends input too).
fn read_line(reader: &mut dyn BufRead, limit: usize) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    while line.len() < limit {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        if available.is_empty() {
            break;
        }
        let room = limit - line.len();
        let window = &available[..available.len().min(room)];
        let (take, done) = match window.iter().position(|&byte| byte == b'\n') {
            Some(at) => (at + 1, true),
            None => (window.len(), false),
        };
        line.extend_from_slice(&window[..take]);
        reader.consume(take);
        if done {
            break;
        }
    }
    (!line.is_empty()).then_some(line)
}

struct Writer {
    stream: Box<dyn Write + Send>,
}

struct State {
    inflight: Vec<(IdKey, Cancelled)>,
    /// Open streams in opening order; a repeated id keeps its place.
    subscriptions: Vec<(IdKey, Value)>,
}

struct Shared {
    server: Arc<McpServer>,
    writer: Mutex<Writer>,
    closed: AtomicBool,
    state: Mutex<State>,
}

type Call = (Value, IdKey, Cancelled);

impl Shared {
    /// `_write(payload)`: one ASCII line; a failed write closes the loop.
    fn write(&self, payload: &Value) {
        // ASCII escapes keep one message on one line and cannot fail to
        // encode (a lone surrogate from book text would otherwise).
        let Ok(mut data) = dumps(payload, &Options::new().compact()) else {
            return;
        };
        data.push('\n');
        let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        if writer
            .stream
            .write_all(data.as_bytes())
            .and_then(|()| writer.stream.flush())
            .is_err()
        {
            self.closed.store(true, Ordering::SeqCst);
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// `StdioLoop`: newline-delimited JSON-RPC over a byte stream pair.
pub struct StdioLoop {
    shared: Arc<Shared>,
    reader: BufReader<Box<dyn Read + Send>>,
    drain: Duration,
}

impl StdioLoop {
    pub fn new(
        server: Arc<McpServer>,
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
    ) -> Self {
        StdioLoop {
            shared: Arc::new(Shared {
                server,
                writer: Mutex::new(Writer { stream: writer }),
                closed: AtomicBool::new(false),
                state: Mutex::new(State {
                    inflight: Vec::new(),
                    subscriptions: Vec::new(),
                }),
            }),
            reader: BufReader::new(reader),
            drain: SHUTDOWN_DRAIN,
        }
    }

    /// How long accepted calls get after end of input (tests shorten it).
    pub fn drain(mut self, drain: Duration) -> Self {
        self.drain = drain;
        self
    }

    /// `run()`: read until end of input, then drain.
    pub fn run(mut self) {
        let (calls, queue) = channel::<Option<Call>>();
        let (finished, done) = channel::<()>();
        let worker = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new()
            .name("kassiber-mcp-tools".into())
            .spawn(move || run_calls(&worker, &queue, &finished));
        if spawned.is_err() {
            return;
        }
        while !self.shared.closed.load(Ordering::SeqCst) {
            let Some(line) = self.read_message() else {
                break;
            };
            if blank(&line) {
                continue;
            }
            let parsed = std::str::from_utf8(&line).ok().and_then(|text| {
                kassiber_pytext::loads_py_without_constants(&PyStr::from(text)).ok()
            });
            let Some(message) = parsed else {
                // Deeply nested JSON must not end the session.
                self.shared.write(&error_response(
                    &Value::Null,
                    PARSE_ERROR,
                    PyStr::from("Parse error"),
                    None,
                ));
                continue;
            };
            self.accept(message, &calls);
        }
        self.shutdown(&calls, &done);
    }

    fn read_message(&mut self) -> Option<Vec<u8>> {
        let line = read_line(&mut self.reader, MAX_MESSAGE_BYTES + 1)?;
        if line.len() > MAX_MESSAGE_BYTES && line.last() != Some(&b'\n') {
            while let Some(rest) = read_line(&mut self.reader, MAX_MESSAGE_BYTES) {
                if rest.last() == Some(&b'\n') {
                    break;
                }
            }
            self.shared.write(&error_response(
                &Value::Null,
                INVALID_REQUEST,
                PyStr::from("Message too large"),
                None,
            ));
            return Some(Vec::new());
        }
        Some(line)
    }

    fn accept(&self, message: Value, calls: &Sender<Option<Call>>) {
        let shared = &self.shared;
        if !matches!(message, Value::Dict(_)) {
            shared.write(&error_response(
                &Value::Null,
                INVALID_REQUEST,
                PyStr::from("Invalid JSON-RPC message"),
                None,
            ));
            return;
        }
        let method = get(&message, "method");
        if is_text(method, "notifications/cancelled") && !contains_key(&message, "id") {
            let key = match get(&message, "params") {
                Some(params @ Value::Dict(_)) => {
                    id_key(get(params, "requestId").unwrap_or(&Value::Null))
                }
                _ => None,
            };
            let event = {
                let mut state = shared.state();
                let event = key.as_ref().and_then(|key| {
                    state
                        .inflight
                        .iter()
                        .find(|(id, _)| id == key)
                        .map(|(_, event)| event.clone())
                });
                if let Some(key) = &key {
                    state.subscriptions.retain(|(id, _)| id != key);
                }
                event
            };
            if let Some(event) = event {
                event.set();
            }
            return;
        }
        let id = get(&message, "id").cloned();
        let key = id.as_ref().and_then(id_key);
        let id = id.unwrap_or(Value::Null);
        if let (true, Some(key)) = (is_text(method, "tools/call"), key.clone()) {
            let event = Cancelled::default();
            let (duplicate, overloaded) = {
                let mut state = shared.state();
                let duplicate = state.inflight.iter().any(|(existing, _)| *existing == key);
                let overloaded = !duplicate && state.inflight.len() >= MAX_INFLIGHT_CALLS;
                if !duplicate && !overloaded {
                    state.inflight.push((key.clone(), event.clone()));
                }
                (duplicate, overloaded)
            };
            if overloaded {
                shared.write(&error_response(
                    &id,
                    SERVER_OVERLOADED,
                    PyStr::from("Too many tool calls in flight; retry after earlier calls finish"),
                    Some(dict([("limit", Value::from(MAX_INFLIGHT_CALLS as i64))])),
                ));
                return;
            }
            if duplicate {
                shared.write(&error_response(
                    &id,
                    INVALID_REQUEST,
                    PyStr::from("Request id is already in flight"),
                    None,
                ));
                return;
            }
            let _ = calls.send(Some((message, key, event)));
            return;
        }
        if let (true, Some(key), true) = (
            is_text(method, "subscriptions/listen"),
            key,
            is_text(get(&message, "jsonrpc"), "2.0"),
        ) {
            // A malformed envelope falls through to ordinary handling, which
            // answers it with the protocol's error instead of a stream.
            if shared.state().subscriptions.len() >= MAX_SUBSCRIPTIONS {
                shared.write(&error_response(
                    &id,
                    SERVER_OVERLOADED,
                    PyStr::from("Too many open subscriptions"),
                    Some(dict([("limit", Value::from(MAX_SUBSCRIPTIONS as i64))])),
                ));
                return;
            }
            let acknowledgement = match shared.server.open_subscription(&message) {
                Ok(acknowledgement) => acknowledgement,
                Err(error) => {
                    shared.write(&error_response(&id, error.code, error.message, error.data));
                    return;
                }
            };
            {
                let mut state = shared.state();
                match state
                    .subscriptions
                    .iter_mut()
                    .find(|(existing, _)| *existing == key)
                {
                    Some(slot) => slot.1 = id,
                    None => state.subscriptions.push((key, id)),
                }
            }
            shared.write(&acknowledgement);
            return;
        }
        if let Some(response) = shared.server.handle(&message, None) {
            shared.write(&response);
        }
    }

    /// `_shutdown()`: EOF ends input, not the calls the client already
    /// sent. Answer them within a bounded wait, then close subscriptions.
    /// Calls the client cancelled explicitly still get no reply.
    fn shutdown(&self, calls: &Sender<Option<Call>>, done: &Receiver<()>) {
        let _ = calls.send(None);
        match done.recv_timeout(self.drain) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) | Err(RecvTimeoutError::Timeout) => {}
        }
        let (pending, subscriptions) = {
            let mut state = self.shared.state();
            let pending: Vec<Cancelled> = state
                .inflight
                .iter()
                .map(|(_, event)| event.clone())
                .collect();
            let subscriptions: Vec<Value> = std::mem::take(&mut state.subscriptions)
                .into_iter()
                .map(|(_, id)| id)
                .collect();
            (pending, subscriptions)
        };
        for event in pending {
            event.set();
        }
        for request_id in subscriptions {
            self.shared
                .write(&self.shared.server.close_subscription(&request_id));
        }
        self.shared.closed.store(true, Ordering::SeqCst);
    }
}

/// `_run_calls()`: one call at a time; a call cancelled before or while it
/// runs gets no reply, and every other one gets exactly one.
fn run_calls(shared: &Shared, queue: &Receiver<Option<Call>>, finished: &Sender<()>) {
    while let Ok(Some((message, key, event))) = queue.recv() {
        let response = if event.is_set() {
            None
        } else {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                shared.server.handle(&message, Some(&event))
            }))
            .unwrap_or_else(|_| {
                // Never leave a call unanswered or the worker dead.
                Some(error_response(
                    get(&message, "id").unwrap_or(&Value::Null),
                    INTERNAL_ERROR,
                    PyStr::from("Internal error"),
                    None,
                ))
            })
        };
        shared.state().inflight.retain(|(id, _)| *id != key);
        if let Some(response) = response {
            if !event.is_set() {
                shared.write(&response);
            }
        }
    }
    let _ = finished.send(());
}

/// `serve(server, reader, writer)`.
pub fn serve(server: Arc<McpServer>, reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>) {
    StdioLoop::new(server, reader, writer).run();
}

#[cfg(test)]
mod tests;

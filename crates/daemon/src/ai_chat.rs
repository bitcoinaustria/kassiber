//! Chat (R6-7): `ai.chat`, `ai.chat.cancel`, `ai.tool_call.consent`, the
//! four `ui.chat.sessions.*` kinds and `ui.chat.history.configure`.
//!
//! `ai.chat` validates, resolves its provider and freezes its book on the
//! request loop, then streams from a worker thread (`kassiber-ai-chat`). The
//! worker never touches the book itself: each piece of book work is a task
//! the request loop runs between requests (`_run_on_daemon_main_thread`,
//! `_drain_daemon_main_thread_tasks`), so a locked or switched book is seen
//! where Python sees it. Registered only behind the `native-chat` gate
//! (design §7 item 1); see docs/reference/rust-shell.md.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender};
use std::sync::Arc;

use kassiber_ai::chat::store::Op;
use kassiber_ai::chat::{self, ActiveChats, ProviderSnapshot, Turn, TurnHost, Worker};
use kassiber_ai::executor::{Book as AiBook, ReadHost};
use kassiber_ai::ToolError;
use kassiber_envelope::{dict, get, py_str, s, success, AppError};
use kassiber_pyjson::{BigInt, PyStr, Value};
use kassiber_runtime::store::AiSurface;

use crate::ai_tools::HostBook;
use crate::input::Input;
use crate::output::{with_request_id, Output, RequestId};
use crate::{Daemon, Failure, Handled};

/// The chat kinds, in Python's `SUPPORTED_KINDS` order.
pub const KINDS: &[&str] = &[
    "ai.chat",
    "ai.chat.cancel",
    "ai.tool_call.consent",
    "ui.chat.sessions.list",
    "ui.chat.sessions.get",
    "ui.chat.sessions.delete",
    "ui.chat.sessions.clear",
    "ui.chat.history.configure",
];

/// The kinds Python answers before its book gate.
pub const CONTROL_KINDS: &[&str] = &["ai.chat.cancel", "ai.tool_call.consent"];

/// Work a chat worker hands to the request loop.
pub(crate) type Task = Box<dyn FnOnce(&mut Daemon) + Send>;

/// `ctx.active_ai_chats` and `ctx.main_thread_tasks`.
pub(crate) struct Chats {
    pub(crate) registry: ActiveChats,
    tasks: Sender<Task>,
    queue: Receiver<Task>,
    workers: Arc<AtomicUsize>,
}

impl Default for Chats {
    fn default() -> Self {
        let (tasks, queue) = channel();
        Chats {
            registry: ActiveChats::default(),
            tasks,
            queue,
            workers: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Chats {
    /// Whether a worker may be waiting on the request loop.
    pub(crate) fn busy(&self) -> bool {
        self.workers.load(Ordering::SeqCst) > 0
    }
}

impl Daemon {
    /// `_drain_daemon_main_thread_tasks(ctx)`.
    pub(crate) fn drain_chat_tasks(&mut self) {
        loop {
            let Ok(task) = self.chats.queue.try_recv() else {
                return;
            };
            // A failing task drops its reply channel; the worker sees it.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task(self)));
        }
    }
}

/// Runs `work` on the request loop and waits for its answer; `None` once
/// the daemon is gone.
fn on_request_loop<T: Send + 'static>(
    tasks: &Sender<Task>,
    work: impl FnOnce(&mut Daemon) -> T + Send + 'static,
) -> Option<T> {
    let (reply, answer) = sync_channel(1);
    tasks
        .send(Box::new(move |daemon| {
            let _ = reply.send(work(daemon));
        }))
        .ok()?;
    answer.recv().ok()
}

fn locked() -> AppError {
    AppError::new(
        "passphrase_required",
        "database is locked; unlock the daemon before running AI tools",
    )
}

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}

/// The executor's book calls, as a worker sends them.
enum BookCall {
    Start,
    InScope(String),
    Context,
    Maintain(bool),
    Read(String, Value, &'static str),
    ActiveProfile,
    Decode(String, Value),
    Issue(String, String),
}

enum BookReply {
    Unit(Result<(), ToolError>),
    Bool(Result<bool, ToolError>),
    Pair(Result<(Value, Value), ToolError>),
    Maybe(Result<Option<Value>, ToolError>),
    Value(Result<Value, ToolError>),
    Text(Result<String, ToolError>),
}

/// The request loop's side of one executor callback: it serves the
/// worker's book calls until the callback returns.
fn serve(daemon: &mut Daemon, calls: Receiver<BookCall>, replies: Sender<BookReply>) {
    while let Ok(call) = calls.recv() {
        let reply = match call {
            BookCall::Start => BookReply::Unit(if daemon.book.is_none() {
                Err(locked().into())
            } else {
                Ok(())
            }),
            call if daemon.book.is_none() => match call {
                BookCall::InScope(_) => BookReply::Bool(Err(locked().into())),
                BookCall::Context => BookReply::Pair(Err(locked().into())),
                BookCall::Maintain(_) => BookReply::Maybe(Err(locked().into())),
                BookCall::ActiveProfile | BookCall::Issue(..) => {
                    BookReply::Text(Err(locked().into()))
                }
                _ => BookReply::Value(Err(locked().into())),
            },
            call => {
                let mut book = HostBook { daemon };
                match call {
                    BookCall::Start => unreachable!(),
                    BookCall::InScope(root) => BookReply::Bool(book.database_in_scope(&root)),
                    BookCall::Context => BookReply::Pair(book.context()),
                    BookCall::Maintain(attempted) => {
                        BookReply::Maybe(book.maintain_for_read(attempted))
                    }
                    BookCall::Read(kind, arguments, source) => {
                        BookReply::Value(book.read(&kind, &arguments, source))
                    }
                    BookCall::ActiveProfile => BookReply::Text(book.active_profile()),
                    BookCall::Decode(profile, arguments) => {
                        BookReply::Value(book.decode_analysis_references(&profile, &arguments))
                    }
                    BookCall::Issue(profile, value) => {
                        BookReply::Text(book.issue_analysis_reference(&profile, &value))
                    }
                }
            }
        };
        if replies.send(reply).is_err() {
            return;
        }
    }
}

/// The worker's view of the book: each call crosses to the request loop.
struct ProxyBook {
    calls: Sender<BookCall>,
    replies: Receiver<BookReply>,
}

impl ProxyBook {
    fn ask(&self, call: BookCall) -> Option<BookReply> {
        self.calls.send(call).ok()?;
        self.replies.recv().ok()
    }
}

macro_rules! answer {
    ($self:ident, $call:expr, $variant:ident) => {
        match $self.ask($call) {
            Some(BookReply::$variant(result)) => result,
            _ => Err(ToolError::Unexpected),
        }
    };
}

impl AiBook for ProxyBook {
    fn database_in_scope(&mut self, data_root: &str) -> Result<bool, ToolError> {
        answer!(self, BookCall::InScope(data_root.to_owned()), Bool)
    }
    fn context(&mut self) -> Result<(Value, Value), ToolError> {
        answer!(self, BookCall::Context, Pair)
    }
    fn maintain_for_read(&mut self, sync_attempted: bool) -> Result<Option<Value>, ToolError> {
        answer!(self, BookCall::Maintain(sync_attempted), Maybe)
    }
    fn read(
        &mut self,
        kind: &str,
        arguments: &Value,
        authored_source: &'static str,
    ) -> Result<Value, ToolError> {
        answer!(
            self,
            BookCall::Read(kind.to_owned(), arguments.clone(), authored_source),
            Value
        )
    }
    fn active_profile(&mut self) -> Result<String, ToolError> {
        answer!(self, BookCall::ActiveProfile, Text)
    }
    fn decode_analysis_references(
        &mut self,
        profile: &str,
        arguments: &Value,
    ) -> Result<Value, ToolError> {
        answer!(
            self,
            BookCall::Decode(profile.to_owned(), arguments.clone()),
            Value
        )
    }
    fn issue_analysis_reference(
        &mut self,
        profile: &str,
        value: &str,
    ) -> Result<String, ToolError> {
        answer!(
            self,
            BookCall::Issue(profile.to_owned(), value.to_owned()),
            Text
        )
    }
}

/// The read executor's request loop, from a worker thread.
struct ProxyReads {
    tasks: Sender<Task>,
}

impl ReadHost for ProxyReads {
    fn on_request_loop(
        &mut self,
        work: &mut dyn FnMut(&mut dyn AiBook) -> Result<Value, ToolError>,
    ) -> Result<Value, ToolError> {
        let (calls, served) = channel();
        let (replies, answers) = channel();
        self.tasks
            .send(Box::new(move |daemon| serve(daemon, served, replies)))
            .map_err(|_| ToolError::Unexpected)?;
        let mut book = ProxyBook {
            calls,
            replies: answers,
        };
        answer!(book, BookCall::Start, Unit)?;
        // The request loop serves this callback's calls until `book` drops.
        work(&mut book)
    }
}

/// The worker's daemon.
struct WorkerHost {
    output: Output,
    request_id: RequestId,
    tasks: Sender<Task>,
    data_root: String,
    reads: ProxyReads,
}

impl WorkerHost {
    /// `_assert_ai_runtime_database_scope(conn, runtime)`, then `op`.
    fn scoped<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Daemon, bool) -> Result<T, AppError> + Send + 'static,
    ) -> Result<T, AppError> {
        let root = self.data_root.clone();
        on_request_loop(&self.tasks, move |daemon| {
            if daemon.book.is_none() {
                return Err(locked());
            }
            let in_scope = HostBook {
                daemon: &mut *daemon,
            }
            .database_in_scope(&root)
            .map_err(|error| match error {
                ToolError::App(error) => error,
                _ => AppError::new("internal_error", "database scope check failed"),
            })?;
            if !in_scope {
                return Err(AppError::new(
                    "stale_context",
                    "The active project changed while the AI turn was running",
                )
                .with_hint("Ask the assistant again in the current project.")
                .retryable(true));
            }
            let encrypted = daemon.store.database_is_encrypted(&root)?;
            work(daemon, encrypted)
        })
        .unwrap_or_else(|| Err(AppError::new("internal_error", "the daemon stopped")))
    }
}

impl TurnHost for WorkerHost {
    fn emit(&mut self, record: Value) -> bool {
        self.output
            .write(&with_request_id(record, &self.request_id))
            .is_ok()
    }

    fn emit_error(&mut self, error: &AppError) -> bool {
        self.output
            .write(&crate::app_error_envelope(error, &self.request_id, None))
            .is_ok()
    }

    fn reads(&mut self) -> &mut dyn ReadHost {
        &mut self.reads
    }

    fn persist(&mut self, mut request: chat::history::Persist) -> Result<Option<PyStr>, AppError> {
        self.scoped(move |daemon, encrypted| {
            request.encrypted = encrypted;
            let session = daemon.book_mut()?.ai_chat(Op::Persist(&request))?;
            Ok(match session {
                Value::Str(id) => Some(id),
                _ => None,
            })
        })
    }

    fn record_detected_api_format(
        &mut self,
        provider: &str,
        base_url: &str,
        detected: &str,
    ) -> Result<(), AppError> {
        let (provider, base_url, detected) = (
            provider.to_owned(),
            base_url.to_owned(),
            detected.to_owned(),
        );
        self.scoped(move |daemon, _| {
            daemon.book_mut()?.ai_chat(Op::RecordDetectedFormat {
                provider: &provider,
                base_url: &base_url,
                detected: &detected,
            })?;
            Ok(())
        })
    }
}

/// The keychain bridge for the provider's key, as the provider kinds use it.
struct KeyHost<'a> {
    output: Output,
    input: &'a mut Input,
    closed: bool,
    cache: Arc<kassiber_ai::discovery::Cache>,
}

impl kassiber_ai::service::Host for KeyHost<'_> {
    fn secret_store(
        &mut self,
        op: &str,
        provider_name: &str,
        store_id: &str,
        service: &str,
        account: &str,
        secret: Option<&str>,
    ) -> Result<Value, AppError> {
        let request = crate::control::SecretStoreRequest {
            op,
            provider_name,
            store_id,
            service,
            account,
            secret,
        };
        match crate::control::secret_store_request(
            &self.output,
            self.input,
            &request,
            crate::control::SECRET_STORE_BRIDGE_TIMEOUT,
        ) {
            Ok(data) => Ok(data),
            Err(crate::control::BridgeError::App(error)) => Err(error),
            Err(crate::control::BridgeError::Closed(_)) => {
                self.closed = true;
                Err(AppError::new("internal_error", "daemon output closed"))
            }
        }
    }

    fn discovery(&self) -> &kassiber_ai::discovery::Cache {
        &self.cache
    }

    fn gate(&self) -> kassiber_ai::service::Gate {
        kassiber_ai::service::system_gate()
    }
}

/// `int(value)` for `ui.chat.sessions.list`'s `limit`.
fn python_int(value: &Value) -> Result<Option<i64>, AppError> {
    let number: Option<BigInt> = match value {
        Value::Bool(flag) => Some(BigInt::from(i64::from(*flag))),
        Value::Int(number) => Some(number.clone()),
        Value::Float(number) if number.is_nan() => None,
        Value::Float(number) if number.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Value::Float(number) => format!("{:.0}", number.trunc()).parse().ok(),
        Value::Str(text) => kassiber_pytext::parse_int(&py_str(&Value::Str(text.clone()))).ok(),
        _ => None,
    };
    // SQLite binds a 64-bit LIMIT; Python's sqlite3 refuses larger ints too.
    Ok(number.map(|n| i64::try_from(n).unwrap_or(i64::MAX)))
}

fn text_arg<'a>(args: &'a Value, key: &str) -> Option<&'a PyStr> {
    match get(args, key) {
        Some(Value::Str(text)) if text.code_points().next().is_some() => Some(text),
        _ => None,
    }
}

/// `_handle_ai_chat_cancel` and `_handle_ai_tool_call_consent`: answered
/// before the book gate; they only touch the RAM registry.
pub(crate) fn control(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request_id: &RequestId,
) -> Handled {
    let args = kassiber_ai::service::coerce_args(args)?;
    let payload = if kind == "ai.chat.cancel" {
        let Some(target) = text_arg(&args, "target_request_id") else {
            return Err(validation("ai.chat.cancel requires target_request_id")
                .with_hint("Pass {target_request_id: '<active ai.chat request_id>'}.")
                .into());
        };
        let (cancelled, queued) = daemon
            .chats
            .registry
            .cancel(&py_str(&Value::Str(target.clone())));
        let mut payload = dict([("cancelled", Value::Bool(cancelled || queued))]);
        if queued {
            kassiber_envelope::set(&mut payload, "queued", Value::Bool(true));
        }
        payload
    } else {
        let Some(target) = text_arg(&args, "target_request_id") else {
            return Err(
                validation("ai.tool_call.consent requires target_request_id")
                    .with_hint("Pass {target_request_id: '<active ai.chat request_id>'}.")
                    .into(),
            );
        };
        let Some(call_id) = text_arg(&args, "call_id") else {
            return Err(validation("ai.tool_call.consent requires call_id").into());
        };
        let decision = get(&args, "decision").cloned().unwrap_or(Value::Null);
        let decision_text = match &decision {
            Value::Str(text)
                if ["allow_once", "allow_session", "deny"]
                    .iter()
                    .any(|known| text.is(known)) =>
            {
                py_str(&decision)
            }
            _ => {
                return Err(validation(
                    "ai.tool_call.consent decision must be allow_once, allow_session, or deny",
                )
                .with_details(dict([("decision", decision)]))
                .into())
            }
        };
        let recorded = daemon.chats.registry.record_consent(
            &py_str(&Value::Str(target.clone())),
            &py_str(&Value::Str(call_id.clone())),
            &decision_text,
        );
        let mut payload = dict([("recorded", Value::Bool(recorded))]);
        if !recorded {
            kassiber_envelope::set(&mut payload, "reason", s("not_found"));
        }
        payload
    };
    Ok((
        Some(with_request_id(success(kind, payload), request_id)),
        false,
    ))
}

/// `_ui_chat_sessions_payload(ctx, kind, args)`.
fn sessions(daemon: &mut Daemon, kind: &str, args: &Value) -> Result<Value, AppError> {
    let (_, profile) = daemon.book_mut()?.resolve_scope(None, None)?;
    let encrypted = || -> Result<bool, AppError> { daemon_encrypted(daemon) };
    match kind {
        "ui.chat.sessions.list" => {
            let default = Value::Int(BigInt::from(50));
            let raw = get(args, "limit").unwrap_or(&default);
            let Some(limit) = python_int(raw)? else {
                return Err(validation("ui.chat.sessions.list limit must be an integer"));
            };
            let encrypted = encrypted()?;
            let book = daemon.book_mut()?;
            let sessions = book.ai_chat(Op::ListSessions {
                profile_id: &profile,
                limit,
            })?;
            let mode = book.ai_chat(Op::HistoryMode)?;
            let enabled = book.ai_chat(Op::HistoryEnabled { encrypted })?;
            Ok(dict([
                ("sessions", sessions),
                ("history_mode", mode),
                ("history_enabled", enabled),
            ]))
        }
        "ui.chat.sessions.clear" => daemon.book_mut()?.ai_chat(Op::ClearSessions {
            profile_id: &profile,
        }),
        "ui.chat.history.configure" => {
            let history = match get(args, "history") {
                None | Some(Value::Null) => None,
                Some(value) => Some(py_str(value)),
            };
            let encrypted = encrypted()?;
            daemon.book_mut()?.ai_chat(Op::Configure {
                history: history.as_deref(),
                encrypted,
            })
        }
        _ => {
            let Some(session_id) = text_arg(args, "session_id").cloned() else {
                return Err(validation(format!("{kind} requires session_id")));
            };
            let op = if kind == "ui.chat.sessions.get" {
                Op::GetSession {
                    profile_id: &profile,
                    session_id: &session_id,
                    include_messages: true,
                }
            } else {
                Op::DeleteSession {
                    profile_id: &profile,
                    session_id: &session_id,
                }
            };
            daemon.book_mut()?.ai_chat(op)
        }
    }
}

fn daemon_encrypted(daemon: &Daemon) -> Result<bool, AppError> {
    daemon.store.database_is_encrypted(&daemon.data_root)
}

/// The book-gated chat kinds.
pub(crate) fn handle(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request_id: &RequestId,
    input: &mut Input,
) -> Handled {
    if let Some(locked) = daemon.open_lazily(request_id)? {
        return Ok((Some(locked), false));
    }
    let args = kassiber_ai::service::coerce_args(args)?;
    if kind != "ai.chat" {
        let payload = sessions(daemon, kind, &args)?;
        return Ok((
            Some(with_request_id(success(kind, payload), request_id)),
            false,
        ));
    }
    start_chat(daemon, &args, request_id, input)
}

/// `ai.chat` on the request loop: validate eagerly, fail fast on an unknown
/// session, resolve the provider (and its key) here, freeze the book, then
/// hand the turn to a worker. The terminal record comes from the worker.
fn start_chat(
    daemon: &mut Daemon,
    args: &Value,
    request_id: &RequestId,
    input: &mut Input,
) -> Handled {
    let mut validated = chat::validate(args)?;
    if let Some(session_id) = &validated.session_id {
        // Also for a turn that is not stored: another book's saved chat is
        // refused rather than silently run without it.
        let book = daemon.book_mut()?;
        let (_, profile) = book.resolve_scope(None, None)?;
        book.ai_chat(Op::GetSession {
            profile_id: &profile,
            session_id,
            include_messages: false,
        })?;
    }
    let provider_args = dict([
        (
            "provider",
            validated.provider.clone().unwrap_or(Value::Null),
        ),
        (
            "_desktop_secret_store_bridge",
            validated.desktop_secret_store_bridge.clone(),
        ),
    ]);
    let mut keys = KeyHost {
        output: daemon.output.clone(),
        input,
        closed: false,
        cache: Arc::clone(&daemon.ai_discovery),
    };
    let resolved = daemon.book_mut()?.ai_providers(
        AiSurface::Daemon,
        kassiber_ai::service::CHAT_PROVIDER,
        Some(&provider_args),
        &mut keys,
    );
    if keys.closed {
        return Err(Failure::Closed);
    }
    let provider = ProviderSnapshot::from_value(&resolved?);
    let scope = daemon.book_mut()?.context()?;
    if let Some(context) = validated.screen_context.take() {
        let bound = daemon.book_mut()?.ai_chat(Op::BindScreen {
            screen_context: &context,
            workspace_id: &scope.workspace_id,
            profile_id: &scope.profile_id,
        })?;
        validated.screen_context = Some(bound);
    }
    if validated.attachment.is_some() {
        // The staging grant resolves against the scope that staged it; the
        // document-import staging sessions are not native yet, so every
        // grant is unknown here, as an expired one is in Python.
        daemon.book_mut()?.resolve_scope(None, None)?;
        return Err(AppError::new(
            "document_import_session_expired",
            "Document import session is unavailable",
        )
        .with_hint("Choose the document and preview it again.")
        .into());
    }
    let state = dict([
        (
            "provider_kind",
            provider.kind.as_deref().map_or(Value::Null, s),
        ),
        ("provider_on_device", Value::Bool(provider.on_device)),
        ("scope_workspace_id", s(scope.workspace_id.as_str())),
        ("scope_profile_id", s(scope.profile_id.as_str())),
    ]);
    let key = chat::registry_key(match request_id {
        RequestId::Missing => None,
        RequestId::Value(value) => Some(value),
    });
    let chat = daemon.chats.registry.register(key.as_deref());
    let turn = Turn {
        key,
        provider,
        validated,
        state,
        data_root: daemon.data_root.clone(),
        chat,
        registry: daemon.chats.registry.clone(),
        gate: kassiber_ai::service::system_gate(),
        locate: kassiber_ai::broker::default_locate(),
        consent_timeout: None,
    };
    let mut host = WorkerHost {
        output: daemon.output.clone(),
        request_id: request_id.clone(),
        tasks: daemon.chats.tasks.clone(),
        data_root: daemon.data_root.clone(),
        reads: ProxyReads {
            tasks: daemon.chats.tasks.clone(),
        },
    };
    let workers = Arc::clone(&daemon.chats.workers);
    workers.fetch_add(1, Ordering::SeqCst);
    let spawned = std::thread::Builder::new()
        .name("kassiber-ai-chat".into())
        .spawn(move || {
            Worker::new(turn).run(&mut host);
            workers.fetch_sub(1, Ordering::SeqCst);
        });
    if spawned.is_err() {
        daemon.chats.workers.fetch_sub(1, Ordering::SeqCst);
        return Err(AppError::new("internal_error", "can't start new thread").into());
    }
    Ok((None, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_limits_follow_python_int() {
        assert_eq!(python_int(&Value::Int(5.into())).unwrap(), Some(5));
        assert_eq!(python_int(&s(" 7 ")).unwrap(), Some(7));
        assert_eq!(python_int(&Value::Float(2.9)).unwrap(), Some(2));
        assert_eq!(python_int(&Value::Bool(true)).unwrap(), Some(1));
        assert_eq!(python_int(&s("x")).unwrap(), None);
        assert_eq!(python_int(&Value::Null).unwrap(), None);
        assert_eq!(
            python_int(&Value::Float(f64::INFINITY)).unwrap_err().code,
            "internal_error"
        );
    }
}

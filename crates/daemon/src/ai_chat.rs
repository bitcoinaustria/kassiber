//! Chat (R6-7): `ai.chat`, `ai.chat.cancel`, `ai.tool_call.consent`, the
//! four `ui.chat.sessions.*` kinds and `ui.chat.history.configure`.
//!
//! `ai.chat` validates, resolves its provider and freezes its book on the
//! request loop, then streams from a worker thread (`kassiber-ai-chat`). The
//! worker never touches the book itself: each piece of book work is a task
//! the request loop runs between requests (`_run_on_daemon_main_thread`,
//! `_drain_daemon_main_thread_tasks`), so a locked or switched book is seen
//! where Python sees it. Every tool a turn calls runs natively: reads
//! through the read executor, writes through the mutating executor after
//! the user's consent (docs/reference/rust-native-chat.md).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender};
use std::sync::Arc;

use kassiber_ai::chat::store::Op;
use kassiber_ai::chat::{self, ActiveChats, ProviderSnapshot, Turn, TurnHost, Worker};
use kassiber_ai::executor::{Book as AiBook, Maintenance, ReadHost};
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
    "ai.chat.permission_mode",
    "ui.chat.sessions.list",
    "ui.chat.sessions.get",
    "ui.chat.sessions.delete",
    "ui.chat.sessions.clear",
    "ui.chat.history.configure",
];

/// The kinds Python answers before its book gate.
pub const CONTROL_KINDS: &[&str] = &[
    "ai.chat.cancel",
    "ai.tool_call.consent",
    "ai.chat.permission_mode",
];

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

/// One executor book call, as a worker sends it: it owns its arguments and
/// runs on the request loop against the open book.
type BookCall = Box<dyn FnOnce(&mut HostBook<'_>) -> BookReply + Send>;

enum BookReply {
    Unit(Result<(), ToolError>),
    Bool(Result<bool, ToolError>),
    Pair(Result<(Value, Value), ToolError>),
    Maybe(Result<Maintenance, ToolError>),
    Value(Result<Value, ToolError>),
    Text(Result<String, ToolError>),
    Run(Result<(Value, Option<Value>), ToolError>),
    /// The book is locked: no call runs.
    Locked,
}

/// The request loop's side of one executor callback: it serves the
/// worker's book calls until the callback returns. A locked book answers
/// every call with `passphrase_required`.
fn serve(daemon: &mut Daemon, calls: Receiver<(BookCall, Sender<BookReply>)>) {
    while let Ok((call, reply)) = calls.recv() {
        let answer = if daemon.book.is_none() {
            BookReply::Locked
        } else {
            call(&mut HostBook { daemon })
        };
        if reply.send(answer).is_err() {
            return;
        }
    }
}

/// The worker's view of the book: each call crosses to the request loop.
struct ProxyBook {
    calls: Sender<(BookCall, Sender<BookReply>)>,
}

impl ProxyBook {
    fn ask(&self, call: BookCall) -> Option<BookReply> {
        let (reply, answer) = channel();
        self.calls.send((call, reply)).ok()?;
        answer.recv().ok()
    }
}

/// Sends `$body` (a closure over the request loop's book, owning its
/// arguments) and unpacks its `$variant` reply; a locked book answers
/// `passphrase_required`.
macro_rules! answer {
    ($self:ident, $variant:ident, $body:expr) => {
        match $self.ask(Box::new(move |book: &mut HostBook<'_>| {
            BookReply::$variant($body(book))
        })) {
            Some(BookReply::$variant(result)) => result,
            Some(BookReply::Locked) => Err(locked().into()),
            _ => Err(ToolError::Unexpected),
        }
    };
}

impl AiBook for ProxyBook {
    fn database_in_scope(&mut self, data_root: &str) -> Result<bool, ToolError> {
        let data_root = data_root.to_owned();
        answer!(self, Bool, |book: &mut HostBook<'_>| book
            .database_in_scope(&data_root))
    }
    fn context(&mut self) -> Result<(Value, Value), ToolError> {
        answer!(self, Pair, |book: &mut HostBook<'_>| book.context())
    }
    fn maintain_for_read(&mut self, sync_attempted: bool) -> Result<Maintenance, ToolError> {
        answer!(self, Maybe, |book: &mut HostBook<'_>| book
            .maintain_for_read(sync_attempted))
    }
    fn read(
        &mut self,
        kind: &str,
        arguments: &Value,
        authored_source: &'static str,
    ) -> Result<Value, ToolError> {
        let (kind, arguments) = (kind.to_owned(), arguments.clone());
        answer!(self, Value, |book: &mut HostBook<'_>| book.read(
            &kind,
            &arguments,
            authored_source
        ))
    }
    fn report_blockers(&mut self, auto_sync: Option<&Value>) -> Result<Value, ToolError> {
        let auto_sync = auto_sync.cloned();
        answer!(self, Value, |book: &mut HostBook<'_>| book
            .report_blockers(auto_sync.as_ref()))
    }
    fn profile_fiat_currency(&mut self) -> Result<String, ToolError> {
        answer!(self, Text, |book: &mut HostBook<'_>| book
            .profile_fiat_currency())
    }
    fn import_runs(
        &mut self,
        request: kassiber_ai::import_runs::ImportRuns<'_>,
    ) -> Result<Value, ToolError> {
        use kassiber_ai::import_runs::ImportRuns;
        // `ImportRuns` without its borrows, to cross to the request loop.
        enum Owned {
            List(Value),
            Review(String),
            Rollback(String),
            InputVersion,
        }
        let owned = match request {
            ImportRuns::List(arguments) => Owned::List(arguments.clone()),
            ImportRuns::Review(batch) => Owned::Review(batch.to_owned()),
            ImportRuns::Rollback(batch) => Owned::Rollback(batch.to_owned()),
            ImportRuns::InputVersion => Owned::InputVersion,
        };
        answer!(self, Value, |book: &mut HostBook<'_>| book.import_runs(
            match &owned {
                Owned::List(arguments) => ImportRuns::List(arguments),
                Owned::Review(batch) => ImportRuns::Review(batch),
                Owned::Rollback(batch) => ImportRuns::Rollback(batch),
                Owned::InputVersion => ImportRuns::InputVersion,
            }
        ))
    }
    fn analyze_file(&mut self, source_file: &str, arguments: &Value) -> Result<Value, ToolError> {
        let (source_file, arguments) = (source_file.to_owned(), arguments.clone());
        answer!(self, Value, |book: &mut HostBook<'_>| book
            .analyze_file(&source_file, &arguments))
    }
    fn attachment_import(
        &mut self,
        source_file: &str,
        request: &Value,
        expected: Option<&Value>,
    ) -> Result<Value, ToolError> {
        let (source_file, request, expected) =
            (source_file.to_owned(), request.clone(), expected.cloned());
        answer!(self, Value, |book: &mut HostBook<'_>| book
            .attachment_import(&source_file, &request, expected.as_ref()))
    }
    fn document_reader(&mut self, provider: &Value) -> Result<Value, ToolError> {
        let provider = provider.clone();
        answer!(self, Value, |book: &mut HostBook<'_>| book
            .document_reader(&provider))
    }
    fn store_document_draft(
        &mut self,
        token: &str,
        draft: &Value,
        provider: &Value,
        detected: Option<&str>,
    ) -> Result<String, ToolError> {
        let (token, draft, provider, detected) = (
            token.to_owned(),
            draft.clone(),
            provider.clone(),
            detected.map(str::to_owned),
        );
        answer!(self, Text, |book: &mut HostBook<'_>| book
            .store_document_draft(
                &token,
                &draft,
                &provider,
                detected.as_deref()
            ))
    }
    fn document_import(
        &mut self,
        draft: &str,
        request: &Value,
        expected: Option<&Value>,
    ) -> Result<Value, ToolError> {
        let (draft, request, expected) = (draft.to_owned(), request.clone(), expected.cloned());
        answer!(self, Value, |book: &mut HostBook<'_>| book.document_import(
            &draft,
            &request,
            expected.as_ref()
        ))
    }
    fn egress_ledger(&mut self, after_id: i64, limit: i64) -> Result<Value, ToolError> {
        answer!(self, Value, |book: &mut HostBook<'_>| book
            .egress_ledger(after_id, limit))
    }
    fn mutate(
        &mut self,
        kind: &str,
        arguments: &Value,
        authored_source: &'static str,
    ) -> Result<Value, ToolError> {
        let (kind, arguments) = (kind.to_owned(), arguments.clone());
        answer!(self, Value, |book: &mut HostBook<'_>| book.mutate(
            &kind,
            &arguments,
            authored_source
        ))
    }
    fn maintenance_run(
        &mut self,
        arguments: &Value,
        sync_attempted: bool,
    ) -> Result<(Value, Option<Value>), ToolError> {
        let arguments = arguments.clone();
        answer!(self, Run, |book: &mut HostBook<'_>| book
            .maintenance_run(&arguments, sync_attempted))
    }
    fn custody_assisted_write(
        &mut self,
        kind: &str,
        arguments: &Value,
        audit: &Value,
    ) -> Result<Value, ToolError> {
        let (kind, arguments, audit) = (kind.to_owned(), arguments.clone(), audit.clone());
        answer!(self, Value, |book: &mut HostBook<'_>| book
            .custody_assisted_write(&kind, &arguments, &audit))
    }
    fn dataset_preview(&mut self, arguments: &Value) -> Result<Value, ToolError> {
        let arguments = arguments.clone();
        answer!(self, Value, |book: &mut HostBook<'_>| book
            .dataset_preview(&arguments))
    }
    fn review(
        &mut self,
        kind: &str,
        arguments: &Value,
        full_access: bool,
    ) -> Result<Value, ToolError> {
        let (kind, arguments) = (kind.to_owned(), arguments.clone());
        answer!(self, Value, |book: &mut HostBook<'_>| book.review(
            &kind,
            &arguments,
            full_access
        ))
    }
    fn review_consent_preview(
        &mut self,
        arguments: &Value,
        full_access: bool,
    ) -> Result<Value, ToolError> {
        let arguments = arguments.clone();
        answer!(self, Value, |book: &mut HostBook<'_>| book
            .review_consent_preview(&arguments, full_access))
    }
    fn append_custody_audit(&mut self, audit: &Value) -> Result<Value, ToolError> {
        let audit = audit.clone();
        answer!(self, Value, |book: &mut HostBook<'_>| book
            .append_custody_audit(&audit))
    }
    fn active_profile(&mut self) -> Result<String, ToolError> {
        answer!(self, Text, |book: &mut HostBook<'_>| book.active_profile())
    }
    fn decode_analysis_references(
        &mut self,
        profile: &str,
        arguments: &Value,
    ) -> Result<Value, ToolError> {
        let (profile, arguments) = (profile.to_owned(), arguments.clone());
        answer!(self, Value, |book: &mut HostBook<'_>| book
            .decode_analysis_references(&profile, &arguments))
    }
    fn issue_analysis_reference(
        &mut self,
        profile: &str,
        value: &str,
    ) -> Result<String, ToolError> {
        let (profile, value) = (profile.to_owned(), value.to_owned());
        answer!(self, Text, |book: &mut HostBook<'_>| book
            .issue_analysis_reference(&profile, &value))
    }
}

/// The read executor's request loop, from a worker thread.
struct ProxyReads {
    tasks: Sender<Task>,
}

impl ReadHost for ProxyReads {
    fn read_document(
        &mut self,
        provider: &Value,
        source_file: &str,
        pages: &Value,
        expected_fiat_currency: Option<&str>,
    ) -> Result<(Value, Option<String>), ToolError> {
        // On the chat's worker: the request loop keeps serving meanwhile.
        crate::ai_tools::read_document(provider, source_file, pages, expected_fiat_currency)
    }
    fn on_request_loop(
        &mut self,
        work: &mut dyn FnMut(&mut dyn AiBook) -> Result<Value, ToolError>,
    ) -> Result<Value, ToolError> {
        let (calls, served) = channel();
        self.tasks
            .send(Box::new(move |daemon| serve(daemon, served)))
            .map_err(|_| ToolError::Unexpected)?;
        let mut book = ProxyBook { calls };
        answer!(book, Unit, |_: &mut HostBook<'_>| Ok(()))?;
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

/// `_handle_ai_chat_cancel`, `_handle_ai_tool_call_consent` and
/// `_handle_ai_chat_permission_mode`: answered before the book gate; they
/// only touch the RAM registry.
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
    } else if kind == "ai.chat.permission_mode" {
        let Some(target) = text_arg(&args, "target_request_id") else {
            return Err(
                validation("ai.chat.permission_mode requires target_request_id")
                    .with_hint("Pass {target_request_id: '<active ai.chat request_id>'}.")
                    .into(),
            );
        };
        let mode = get(&args, "permission_mode")
            .cloned()
            .unwrap_or(Value::Null);
        let Some(mode_text) = kassiber_ai::consent::PERMISSION_MODES
            .iter()
            .find(|known| mode == s(**known))
        else {
            return Err(validation(
                "ai.chat.permission_mode permission_mode must be approval, auto, or full_access",
            )
            .with_details(dict([("permission_mode", mode)]))
            .into());
        };
        // Releases every waiting call the new mode approves.
        let released = daemon
            .chats
            .registry
            .set_permission_mode(&py_str(&Value::Str(target.clone())), mode_text);
        let mut payload = dict([
            ("permission_mode", s(*mode_text)),
            (
                "released_call_ids",
                Value::List(released.iter().flatten().map(|id| s(id.as_str())).collect()),
            ),
            ("updated", Value::Bool(released.is_some())),
        ]);
        if released.is_none() {
            kassiber_envelope::set(&mut payload, "reason", s("not_found"));
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
        // A CLI proxy resolves the format of this model.
        ("model", Value::Str(validated.model.clone())),
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
    // Resolved here, on the request loop: the tool worker only ever sees
    // the resolved paths.
    let attachment_state = resolve_chat_attachments(daemon, &validated.attachments)?;
    let mut state = dict([
        (
            "provider_kind",
            provider.kind.as_deref().map_or(Value::Null, s),
        ),
        ("provider_on_device", Value::Bool(provider.on_device)),
        // Whether this turn may use every tool and unprojected result.
        ("provider_full_access", Value::Bool(provider.full_access)),
        ("scope_workspace_id", s(scope.workspace_id.as_str())),
        ("scope_profile_id", s(scope.profile_id.as_str())),
        // A document read from chat is kept as a draft with the daemon's
        // document grants, like the desktop's document import preview.
        ("document_import_sessions", Value::Bool(true)),
    ]);
    for (key, value) in attachment_state {
        kassiber_envelope::set(&mut state, key, value);
    }
    let key = chat::registry_key(match request_id {
        RequestId::Missing => None,
        RequestId::Value(value) => Some(value),
    });
    let session_id = validated
        .session_id
        .as_ref()
        .map(|id| py_str(&Value::Str(id.clone())));
    let chat = daemon
        .chats
        .registry
        .register(key.as_deref(), session_id.as_deref());
    chat.consent.set_permission_mode(validated.permission_mode);
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
    let (registry, key, chat) = (
        turn.registry.clone(),
        turn.key.clone(),
        Arc::clone(&turn.chat),
    );
    workers.fetch_add(1, Ordering::SeqCst);
    let spawned = std::thread::Builder::new()
        .name("kassiber-ai-chat".into())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Worker::new(turn).run(&mut host);
            }));
            if let Err(panic) = outcome {
                // Python's worker answers an unexpected failure with an
                // internal_error envelope; the turn never stays silent.
                let message = panic
                    .downcast_ref::<&str>()
                    .map(|m| (*m).to_owned())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "panic".to_owned());
                host.emit_error(&AppError::new("internal_error", message));
                registry.set_cancel_handler(&chat, None);
                registry.unregister(key.as_deref(), &chat);
            }
            workers.fetch_sub(1, Ordering::SeqCst);
        });
    if spawned.is_err() {
        daemon.chats.workers.fetch_sub(1, Ordering::SeqCst);
        return Err(AppError::new("internal_error", "can't start new thread").into());
    }
    Ok((None, false))
}

/// `_resolve_chat_attachments(ctx, attachments)`: the chat's grants as
/// runtime state for its tools. Each file gets the id the model names it by
/// (`file-1`, ...) in the order attached. A grant that expired (or belongs to
/// another book) is left out rather than failing the turn: the note tells the
/// model that id is gone and the terminal returns its token (#697).
fn resolve_chat_attachments(
    daemon: &mut Daemon,
    attachments: &[Value],
) -> Result<Vec<(&'static str, Value)>, AppError> {
    if attachments.is_empty() {
        return Ok(Vec::new());
    }
    daemon.book_mut()?.resolve_scope(None, None)?;
    let mut files = Vec::new();
    let mut expired_ids = Vec::new();
    let mut expired_tokens = Vec::new();
    for (index, attachment) in attachments.iter().enumerate() {
        let id = format!("file-{}", index + 1);
        let token = get(attachment, "token")
            .map(kassiber_envelope::py_str)
            .unwrap_or_default();
        let source_file = match daemon.document_source(&token) {
            Ok(source_file) => source_file,
            Err(error) if error.code == "document_import_session_expired" => {
                expired_ids.push(s(id));
                expired_tokens.push(s(token));
                continue;
            }
            Err(error) => return Err(error),
        };
        let filename = std::path::Path::new(&source_file)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        files.push(dict([
            ("id", s(id)),
            ("source_file", s(source_file.as_str())),
            ("filename", s(filename)),
            (
                "label",
                get(attachment, "label").cloned().unwrap_or(Value::Null),
            ),
            (
                "kind",
                s(kassiber_wallets::file_import::analysis::file_kind(
                    &source_file,
                )),
            ),
            (
                "new",
                Value::Bool(get(attachment, "new") == Some(&Value::Bool(true))),
            ),
            // Internal: a document draft is staged against it. Never in the
            // note, a tool result or history.
            ("token", s(token)),
        ]));
    }
    let mut state = vec![("chat_files", Value::List(files))];
    if !expired_ids.is_empty() {
        state.push(("expired_chat_files", Value::List(expired_ids)));
        state.push(("expired_attachment_tokens", Value::List(expired_tokens)));
    }
    Ok(state)
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

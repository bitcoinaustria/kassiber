//! One `ai.chat` turn on its worker thread (`_run_ai_chat_stream` and
//! `_run_ai_chat_tool_loop`, `kassiber/daemon.py:9373`–`9951`).
//!
//! Records go out in Python's order: `ai.chat.status`, `ai.chat.delta`,
//! `ai.chat.tool_call`, `ai.chat.tool_consent_required`,
//! `ai.chat.tool_result`, then one terminal `ai.chat` or an error envelope.
//! A consent is registered before the record that asks for it is written;
//! a cancelled turn still ends with a terminal record; book work runs on the
//! request loop through the [`TurnHost`].

use std::sync::Arc;
use std::time::Duration;

use kassiber_envelope::{dict, get, set, success, AppError};
use kassiber_net::Gate;
use kassiber_pyjson::{dumps, Key, Options, PyStr, Value};
use sha2::{Digest, Sha256};

use super::args::Validated;
use super::client::{ChatClient, ProviderSnapshot};
use super::files;
use super::history::Persist;
use super::registry::{ActiveChat, ActiveChats};
use super::screen::screen_context_for_model;
use crate::broker::Locate;
use crate::catalog::load_tools_entry;
use crate::catalog::{get_tool_py, KindClass};
use crate::consent::CONSENT_TIMEOUT;
use crate::executor::{
    denial_for, execute_mutating, execute_read_only, is_advertised, model_output, parse_tool_call,
    run_auto_reads, tool_result_denied, tool_result_record, ReadHost, Runtime,
};
use crate::planner::{
    cross_book_read_allowed, insert_auto_tool_context_message, planned_auto_reads,
};
use crate::prompt::{build_chat_messages, PromptKind};
use crate::provenance::{answer_provenance, record_tool_usage};
use crate::pyval::{lossy, s};
use crate::redact::{redact_ai_tool_result, redact_tool_arguments};
use crate::select::{build_responses_tools, strip_on_device_only};
use crate::summary::summarize_tool_call;
use crate::transport::{ChatDelta, Receipt, RequestContext, PROVIDER_STATUS_NOTICES};
use crate::ToolError;

/// `_AI_CUT_OFF_FINISH_REASONS`: an answer cut off by its token limit or a
/// content filter runs none of its tool calls.
pub const CUT_OFF_FINISH_REASONS: [&str; 2] = ["content_filter", "length"];

/// What the turn needs from the daemon. Book work runs on the request loop.
pub trait TurnHost {
    /// Writes one record for this request (the host adds `request_id`);
    /// `false` once the output is gone.
    fn emit(&mut self, record: Value) -> bool;
    /// Writes the error envelope that ends a failed turn.
    fn emit_error(&mut self, error: &AppError) -> bool;
    /// The read executor's request loop.
    fn reads(&mut self) -> &mut dyn ReadHost;
    /// `_persist_ai_chat_exchange`'s main-thread part: the scope check, the
    /// stored policy (with the database file's encryption), then the write.
    fn persist(&mut self, request: Persist) -> Result<Option<PyStr>, AppError>;
    /// `record_detected_api_format(conn, name, base_url, detected)` after
    /// the scope check.
    fn record_detected_api_format(
        &mut self,
        provider: &str,
        base_url: &str,
        detected: &str,
    ) -> Result<(), AppError>;
    /// `now_iso()`, for consent timestamps and provenance.
    fn now(&self) -> String {
        crate::clock::now_iso()
    }
}

/// Everything the request loop froze for the turn.
pub struct Turn {
    pub key: Option<String>,
    pub provider: ProviderSnapshot,
    pub validated: Validated,
    /// `runtime.maintenance_state`.
    pub state: Value,
    pub data_root: String,
    pub chat: Arc<ActiveChat>,
    pub registry: ActiveChats,
    pub gate: Gate,
    pub locate: Locate,
    /// Overrides the 300 s consent wait (test harnesses).
    pub consent_timeout: Option<Duration>,
}

/// Why streaming stopped early.
enum Stop {
    /// The cancel flag was seen between chunks.
    Cancelled,
    /// The daemon's output is gone; nothing more can be said.
    Closed,
    Error(AppError),
}

impl From<AppError> for Stop {
    fn from(error: AppError) -> Self {
        Stop::Error(error)
    }
}

/// A sentinel the streaming callback returns to end the provider stream
/// as Python's `break` ends its generator.
fn stop_marker(code: &str) -> AppError {
    AppError::new(code, "")
}

const CANCELLED_MARKER: &str = "\u{0}chat-cancelled";
const CLOSED_MARKER: &str = "\u{0}chat-output-closed";

fn stopped(error: AppError) -> Stop {
    match error.code.as_str() {
        CANCELLED_MARKER => Stop::Cancelled,
        CLOSED_MARKER => Stop::Closed,
        _ => Stop::Error(error),
    }
}

/// `_ai_tool_approval_label(decision)`: the `approval` a tool_call record
/// shows for how the call was approved.
fn approval_label(decision: Option<&str>) -> &'static str {
    match decision {
        Some("mode_auto") => "auto",
        Some("mode_full_access") => "full_access",
        Some("allow_session") => "session",
        _ => "required",
    }
}

/// `_provider_session_resume_fingerprint(messages)`: the branch a resumable
/// cursor was recorded against. A cursor is remembered under the transcript
/// plus its answer; the next request carries that transcript and one new
/// user prompt, so everything before the prompt is looked up. Any other
/// shape resumes nothing.
pub fn resume_fingerprint(messages: &[Value]) -> Option<String> {
    let (last, before) = messages.split_last()?;
    (matches!(last, Value::Dict(_)) && get(last, "role") == Some(&s("user")))
        .then(|| history_fingerprint(before))
}

/// `_provider_session_history_fingerprint(messages)`: binds a provider
/// cursor to the exact visible chat branch.
pub fn history_fingerprint(messages: &[Value]) -> String {
    let text = |message: &Value, key: &str| match get(message, key) {
        Some(value) if kassiber_envelope::truthy(value) => match value {
            Value::Str(text) => Value::Str(text.clone()),
            other => s(kassiber_envelope::py_str(other)),
        },
        _ => s(""),
    };
    let normalized: Vec<Value> = messages
        .iter()
        .filter(|message| matches!(message, Value::Dict(_)))
        .map(|message| {
            dict([
                ("role", text(message, "role")),
                ("content", text(message, "content")),
            ])
        })
        .collect();
    let encoded = dumps(
        &Value::List(normalized),
        &Options::new().sort_keys(true).separators(",", ":"),
    )
    .unwrap_or_default();
    let digest = Sha256::digest(encoded.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `_ai_chat_seed_prefix(messages)`: the turns before the current prompt.
pub fn seed_prefix(messages: &[Value]) -> Vec<Value> {
    let last_user = messages.iter().rposition(
        |message| matches!(get(message, "role"), Some(Value::Str(role)) if role.is("user")),
    );
    match last_user {
        Some(index) if index > 0 => messages[..index].to_vec(),
        _ => Vec::new(),
    }
}

fn status(phase: &str, label: &str) -> Value {
    success(
        "ai.chat.status",
        dict([("phase", s(phase)), ("label", s(label))]),
    )
}

fn delta_record(delta: Value) -> Value {
    success("ai.chat.delta", dict([("delta", delta)]))
}

/// `_ai_chat_notices(runtime)`: the turn's notices, recorded as they stream.
fn turn_notices(state: &mut Value) -> &mut Vec<Value> {
    if !matches!(get(state, "notices"), Some(Value::List(_))) {
        set(state, "notices", Value::List(Vec::new()));
    }
    let Value::Dict(pairs) = state else {
        unreachable!("maintenance state is a dict")
    };
    match pairs
        .iter_mut()
        .find(|(key, _)| *key == Key::from("notices"))
    {
        Some((_, Value::List(items))) => items,
        _ => unreachable!("notices were just set"),
    }
}

/// `_forward_ai_chat_notice(out, request_id, chunk, notices)`: a known
/// provider notice becomes an `ai.chat.status`; `None` for any other chunk.
fn forward_notice(
    chunk: &ChatDelta,
    notices: &mut Vec<Value>,
    host: &mut dyn TurnHost,
) -> Option<bool> {
    let phase = chunk.status_phase.as_deref()?;
    let (_, label) = PROVIDER_STATUS_NOTICES
        .iter()
        .find(|(known, _)| *known == phase)?;
    let written = host.emit(status(phase, label));
    if !notices.iter().any(|notice| *notice == s(phase)) {
        notices.push(s(phase));
    }
    Some(written)
}

/// `AiToolTurnResult`.
struct LegResult {
    tool_calls: Vec<Value>,
    content: String,
    finish_reason: Option<String>,
    response_output: Vec<Value>,
    notices: Vec<Value>,
}

/// `_stream_ai_chat_tool_turn(...)`: one provider leg of the tool loop.
fn stream_leg(
    client: &mut ChatClient,
    validated: &Validated,
    context: &RequestContext,
    tools: &[Value],
    chat: &ActiveChat,
    host: &mut dyn TurnHost,
) -> Result<LegResult, Stop> {
    let mut tool_calls: Vec<Value> = Vec::new();
    let mut content = String::new();
    let mut notices: Vec<Value> = Vec::new();
    let mut finish_reason: Option<String> = None;
    let mut response_output: Option<Vec<Value>> = None;
    if !host.emit(status("waiting_for_model", "Thinking")) {
        return Err(Stop::Closed);
    }
    let model = lossy(&validated.model);
    let streamed = client.stream_tools(
        context,
        &model,
        &validated.options,
        tools,
        &mut |chunk: ChatDelta| {
            if chat.is_cancelled() {
                finish_reason = Some("cancelled".into());
                return Err(stop_marker(CANCELLED_MARKER));
            }
            match forward_notice(&chunk, &mut notices, host) {
                Some(false) => return Err(stop_marker(CLOSED_MARKER)),
                Some(true) => return Ok(()),
                None => {}
            }
            if let Some(Value::List(calls)) = get(&chunk.delta, "tool_calls") {
                tool_calls = calls.clone();
            }
            let mut payload: Vec<(Key, Value)> = Vec::new();
            if let Some(Value::Str(text)) = get(&chunk.delta, "content") {
                if text.code_points().next().is_some() {
                    content.push_str(&lossy(text));
                    payload.push((Key::from("content"), Value::Str(text.clone())));
                }
            }
            if let Some(Value::Str(text)) = get(&chunk.delta, "reasoning") {
                if text.code_points().next().is_some() {
                    payload.push((Key::from("reasoning"), Value::Str(text.clone())));
                }
            }
            if !payload.is_empty() && !host.emit(delta_record(Value::Dict(payload))) {
                return Err(stop_marker(CLOSED_MARKER));
            }
            if let Some(reason) = &chunk.finish_reason {
                finish_reason = Some(reason.clone());
            }
            if let Some(output) = &chunk.response_output {
                response_output = Some(output.clone());
            }
            if chat.is_cancelled() {
                finish_reason = Some("cancelled".into());
                return Err(stop_marker(CANCELLED_MARKER));
            }
            Ok(())
        },
    );
    match streamed.map_err(stopped) {
        Ok(()) | Err(Stop::Cancelled) => {}
        Err(stop) => return Err(stop),
    }
    drop_cut_off_calls(
        finish_reason.as_deref(),
        &mut tool_calls,
        &mut response_output,
    );
    let response_output = match response_output {
        Some(output) => output,
        None => {
            let mut output = Vec::new();
            if !content.is_empty() {
                output.push(dict([
                    ("type", s("message")),
                    ("role", s("assistant")),
                    (
                        "content",
                        Value::List(vec![dict([
                            ("type", s("output_text")),
                            ("text", s(&content)),
                        ])]),
                    ),
                ]));
            }
            for call in &tool_calls {
                let function = match call {
                    Value::Dict(_) => get(call, "function"),
                    _ => None,
                };
                let Some(function @ Value::Dict(_)) = function else {
                    continue;
                };
                let arguments = match get(function, "arguments") {
                    Some(arguments) if kassiber_envelope::truthy(arguments) => arguments.clone(),
                    _ => s(""),
                };
                output.push(dict([
                    ("type", s("function_call")),
                    ("call_id", get(call, "id").cloned().unwrap_or(Value::Null)),
                    (
                        "name",
                        get(function, "name").cloned().unwrap_or(Value::Null),
                    ),
                    ("arguments", arguments),
                ]));
            }
            output
        }
    };
    Ok(LegResult {
        tool_calls,
        content,
        finish_reason,
        response_output,
        notices,
    })
}

/// A call in an answer cut off by its token limit or a content filter may be
/// cut off too: none of the answer's calls runs or is replayed, whichever
/// client streamed it.
fn drop_cut_off_calls(
    finish_reason: Option<&str>,
    tool_calls: &mut Vec<Value>,
    response_output: &mut Option<Vec<Value>>,
) {
    if !finish_reason.is_some_and(|reason| CUT_OFF_FINISH_REASONS.contains(&reason)) {
        return;
    }
    tool_calls.clear();
    if let Some(output) = response_output.as_mut() {
        output.retain(|item| {
            !(matches!(item, Value::Dict(_))
                && matches!(get(item, "type"), Some(Value::Str(kind)) if kind.is("function_call")))
        });
    }
}

/// `_update_review_checkpoint(checkpoint, name, result)`: bounded
/// continuation facts, never model reasoning.
fn update_review_checkpoint(checkpoint: &mut Value, name: &str, result: &Value) {
    if !get(result, "ok").is_some_and(kassiber_envelope::truthy) {
        return;
    }
    let data = match get(result, "envelope") {
        Some(envelope @ Value::Dict(_)) => get(envelope, "data"),
        _ => None,
    };
    let Some(data @ Value::Dict(_)) = data else {
        return;
    };
    match name {
        "ui.review.cases" => {
            for key in ["workspace_id", "profile_id", "input_version", "next_cursor"] {
                if let Some(value) = get(data, key) {
                    set(checkpoint, key, value.clone());
                }
            }
            let ids: Vec<Value> = match get(data, "cases") {
                Some(Value::List(cases)) => cases
                    .iter()
                    .take(100)
                    .filter_map(|case| match get(case, "transaction_id") {
                        Some(id @ Value::Str(_)) => Some(id.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            set(checkpoint, "last_page_transaction_ids", Value::List(ids));
        }
        "ui.review.plan" => {
            set(
                checkpoint,
                "unapplied_plan_digest",
                get(data, "digest").cloned().unwrap_or(Value::Null),
            );
        }
        "ui.review.apply" | "ui.review.receipt" => {
            if let Some(id @ Value::Str(_)) = get(data, "id") {
                let mut receipts = match get(checkpoint, "receipt_ids") {
                    Some(Value::List(items)) => items.clone(),
                    _ => Vec::new(),
                };
                if !receipts.contains(id) {
                    receipts.push(id.clone());
                    let excess = receipts.len().saturating_sub(32);
                    receipts.drain(..excess);
                }
                set(checkpoint, "receipt_ids", Value::List(receipts));
            }
            let digest = get(data, "artifact_digest").cloned().unwrap_or(Value::Null);
            if get(checkpoint, "unapplied_plan_digest")
                .cloned()
                .unwrap_or(Value::Null)
                == digest
            {
                crate::pyval::pop(checkpoint, "unapplied_plan_digest");
            }
            if name == "ui.review.apply" {
                crate::pyval::pop(checkpoint, "next_cursor");
                crate::pyval::pop(checkpoint, "last_page_transaction_ids");
                set(
                    checkpoint,
                    "input_version",
                    get(data, "result_input_version")
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
        }
        _ => {}
    }
}

fn last_user_content(validated: &Validated) -> Option<PyStr> {
    validated
        .messages
        .iter()
        .rev()
        .find_map(|message| match get(message, "role") {
            Some(Value::Str(role)) if role.is("user") => match get(message, "content") {
                Some(Value::Str(content)) => Some(content.clone()),
                _ => Some(PyStr::new()),
            },
            _ => None,
        })
}

/// The worker thread's body.
pub struct Worker {
    turn: Turn,
    receipt: Receipt,
    client: Option<ChatClient>,
}

impl Worker {
    pub fn new(turn: Turn) -> Self {
        Worker {
            turn,
            receipt: Receipt::default(),
            client: None,
        }
    }

    /// `_run_ai_chat_stream(...)`.
    pub fn run(mut self, host: &mut dyn TurnHost) {
        let mut content_parts = String::new();
        let outcome = self.stream(host, &mut content_parts);
        if let Err(stop) = outcome {
            match stop {
                Stop::Closed => {}
                Stop::Cancelled => {
                    self.remember_detected_format(host);
                    self.terminal(host, Some("cancelled".into()), &content_parts);
                }
                Stop::Error(error) => {
                    // A later round can fail after the fallback answered.
                    self.remember_detected_format(host);
                    if self.turn.chat.is_cancelled() {
                        self.terminal(host, Some("cancelled".into()), &content_parts);
                    } else {
                        host.emit_error(&error);
                    }
                }
            }
        }
        let turn = &self.turn;
        turn.registry.set_cancel_handler(&turn.chat, None);
        turn.registry.unregister(turn.key.as_deref(), &turn.chat);
    }

    fn stream(&mut self, host: &mut dyn TurnHost, content_parts: &mut String) -> Result<(), Stop> {
        let mut finish_reason: Option<String> = None;
        if !self.turn.chat.is_cancelled() {
            if !host.emit(status("preparing", "Preparing chat")) {
                return Err(Stop::Closed);
            }
            let client = ChatClient::for_provider(
                &self.turn.provider,
                Duration::from_secs_f64(self.turn.validated.timeout),
                self.turn.gate.clone(),
                Arc::clone(&self.turn.locate),
                self.receipt.clone(),
            )?;
            let canceller = client.canceller();
            self.client = Some(client);
            self.turn
                .registry
                .set_cancel_handler(&self.turn.chat, Some(canceller));
            if !host.emit(status("connecting", "Connecting")) {
                return Err(Stop::Closed);
            }
            // Tool support follows the request, not the transport: CLI
            // providers reach the same loop through the broker's tools.
            let tools_enabled = self.turn.validated.tools_enabled;
            if tools_enabled {
                return self.tool_loop(host);
            }
            let kind = self
                .turn
                .validated
                .system_prompt_kind
                .filter(|kind| *kind != PromptKind::Kassiber);
            let messages = build_chat_messages(
                &self.turn.validated.messages,
                kind,
                self.turn.validated.system_prompt.as_ref(),
                false,
            )?;
            if !host.emit(status("waiting_for_model", "Loading model")) {
                return Err(Stop::Closed);
            }
            let mut options = self.turn.validated.options.clone();
            let cursor = if self.turn.validated.keeps_provider_session() {
                self.turn.registry.provider_session(
                    self.turn
                        .validated
                        .session_id
                        .as_ref()
                        .map(lossy)
                        .as_deref(),
                    &self.turn.provider.name,
                    resume_fingerprint(&self.turn.validated.messages).as_deref(),
                )
            } else {
                None
            };
            let resumed = cursor.is_some();
            if let Some(cursor) = cursor {
                set(&mut options, "provider_session_id", s(cursor));
            }
            let mut messages = messages;
            files::attach_chat_files_to_turn(&mut messages, &self.turn.state, resumed);
            let chat = Arc::clone(&self.turn.chat);
            let model = lossy(&self.turn.validated.model);
            let state = &mut self.turn.state;
            let client = self.client.as_mut().expect("client was just built");
            let streamed =
                client.stream_messages(&messages, &model, &options, &mut |chunk: ChatDelta| {
                    if chat.is_cancelled() {
                        finish_reason = Some("cancelled".into());
                        return Err(stop_marker(CANCELLED_MARKER));
                    }
                    match forward_notice(&chunk, turn_notices(state), host) {
                        Some(false) => return Err(stop_marker(CLOSED_MARKER)),
                        Some(true) => return Ok(()),
                        None => {}
                    }
                    if let Some(Value::Str(text)) = get(&chunk.delta, "content") {
                        content_parts.push_str(&lossy(text));
                    }
                    if let Some(reason) = &chunk.finish_reason {
                        finish_reason = Some(reason.clone());
                    }
                    if !host.emit(delta_record(chunk.delta.clone())) {
                        return Err(stop_marker(CLOSED_MARKER));
                    }
                    if chat.is_cancelled() {
                        finish_reason = Some("cancelled".into());
                        return Err(stop_marker(CANCELLED_MARKER));
                    }
                    Ok(())
                });
            match streamed.map_err(stopped) {
                Ok(()) | Err(Stop::Cancelled) => {}
                Err(stop) => return Err(stop),
            }
        }
        if self.turn.chat.is_cancelled() {
            finish_reason = Some("cancelled".into());
        }
        if finish_reason.as_deref() != Some("cancelled")
            && self.client.is_some()
            && self.turn.validated.keeps_provider_session()
        {
            self.remember_provider_session(content_parts);
        }
        if self.client.is_some() {
            self.remember_detected_format(host);
        }
        let content = content_parts.clone();
        self.terminal(host, finish_reason, &content);
        Ok(())
    }

    fn remember_provider_session(&self, content: &str) {
        let mut visible = self.turn.validated.messages.clone();
        visible.push(dict([("role", s("assistant")), ("content", s(content))]));
        let cursor = self
            .client
            .as_ref()
            .and_then(ChatClient::last_provider_session_id);
        self.turn.registry.remember_provider_session(
            self.turn
                .validated
                .session_id
                .as_ref()
                .map(lossy)
                .as_deref(),
            &self.turn.provider.name,
            cursor.as_deref(),
            &history_fingerprint(&visible),
        );
    }

    /// `_run_ai_chat_tool_loop(...)`.
    fn tool_loop(&mut self, host: &mut dyn TurnHost) -> Result<(), Stop> {
        let chat = Arc::clone(&self.turn.chat);
        // The broker fingerprints its cursor with the advertised tools, so a
        // changed catalog starts a fresh native thread.
        let cursor = if self.turn.validated.keeps_provider_session() {
            self.turn.registry.provider_session(
                self.turn
                    .validated
                    .session_id
                    .as_ref()
                    .map(lossy)
                    .as_deref(),
                &self.turn.provider.name,
                resume_fingerprint(&self.turn.validated.messages).as_deref(),
            )
        } else {
            None
        };
        let resumed = cursor.is_some();
        if let Some(cursor) = cursor {
            set(
                &mut self.turn.validated.options,
                "provider_session_id",
                s(cursor),
            );
        }
        let validated = self.turn.validated.clone();
        let kind = validated.system_prompt_kind.map(PromptKind::as_str);
        let mut messages = build_chat_messages(
            &validated.messages,
            validated.system_prompt_kind,
            validated.system_prompt.as_ref(),
            true,
        )?;
        let mut screen_context = validated.screen_context.clone();
        if let Some(context @ Value::Dict(_)) = &screen_context {
            insert_auto_tool_context_message(&mut messages, &screen_context_for_model(context));
        }
        // An attached export is the signal that the analysis tool matters,
        // more reliably than a keyword in the question.
        let chat_kinds: Vec<String> = match get(&self.turn.state, "chat_files") {
            Some(Value::List(files)) => files
                .iter()
                .map(|entry| {
                    get(entry, "kind")
                        .map(kassiber_envelope::py_str)
                        .unwrap_or_default()
                })
                .collect(),
            _ => Vec::new(),
        };
        if chat_kinds
            .iter()
            .any(|kind| files::CHAT_TABLE_KINDS.contains(&kind.as_str()))
        {
            let mut capabilities: Vec<String> = match screen_context
                .as_ref()
                .and_then(|context| get(context, "capabilities"))
            {
                Some(Value::List(items)) => items.iter().map(kassiber_envelope::py_str).collect(),
                _ => Vec::new(),
            };
            capabilities.push("wallets".into());
            capabilities.sort();
            capabilities.dedup();
            let mut context = screen_context
                .clone()
                .unwrap_or_else(|| Value::Dict(Vec::new()));
            set(
                &mut context,
                "capabilities",
                Value::List(capabilities.into_iter().map(s).collect()),
            );
            screen_context = Some(context);
        }
        // The files' note, and the images and PDFs the provider may see.
        files::attach_chat_files_to_turn(&mut messages, &self.turn.state, resumed);
        let mut entries = build_responses_tools(
            Some(&validated.messages),
            screen_context
                .as_ref()
                .filter(|c| matches!(c, Value::Dict(_))),
            Some(validated.tool_profile),
        )?;
        // The planner is the more specific judge of what a question needs:
        // its reads are offered (schema included) even under a narrow
        // profile, so `advertised_tools` stays both the authorization set and
        // the receipt's count.
        let planned = planned_auto_reads(kind, &validated.messages);
        for read in &planned {
            if let Some(entry) = crate::catalog::get_tool(read.name) {
                if !entries
                    .iter()
                    .any(|offered| offered.provider_name() == entry.provider_name())
                {
                    entries.push(entry);
                }
            }
        }
        // The note names the evidence tool for any attached file, and an
        // image or PDF offers the document reader (no pack offers it).
        let mut offered_for_files: Vec<&str> = Vec::new();
        if !chat_kinds.is_empty() {
            offered_for_files.push("ui.attachments.add_from_chat");
        }
        if chat_kinds
            .iter()
            .any(|kind| files::CHAT_DOCUMENT_KINDS.contains(&kind.as_str()))
        {
            let mut names = crate::select::CHAT_DOCUMENT_TOOL_NAMES.to_vec();
            names.sort_unstable();
            offered_for_files.extend(names);
        }
        for name in offered_for_files {
            if let Some(entry) = crate::catalog::get_tool(name) {
                if !entries
                    .iter()
                    .any(|offered| offered.provider_name() == entry.provider_name())
                {
                    entries.push(entry);
                }
            }
        }
        // `_turn_tool_filter`: schemas this turn's provider may not use.
        let mut entries =
            strip_on_device_only(entries, crate::executor::full_access(&self.turn.state));
        set(
            &mut self.turn.state,
            "tool_profile",
            s(validated.tool_profile),
        );
        crate::pyval::pop(&mut self.turn.state, "pending_tool_definitions");
        // `_ai_turn_can_load_tools`: an HTTP provider receives the tool list
        // with every request, so an added area is offered on the next round;
        // a broker agent fixes its catalog when the turn starts.
        let loading =
            !matches!(self.client, Some(ChatClient::Broker(_))) && validated.tool_profile != "full";
        set(
            &mut self.turn.state,
            "load_tools_enabled",
            Value::Bool(loading),
        );
        if loading {
            entries.push(load_tools_entry());
        }
        let mut tools: Vec<Value> = entries
            .iter()
            .map(|entry| entry.to_responses_tool())
            .collect();
        set(
            &mut self.turn.state,
            "advertised_tools",
            Value::List(
                entries
                    .iter()
                    .map(|entry| s(entry.provider_name()))
                    .collect(),
            ),
        );
        set(
            &mut self.turn.state,
            "cross_book_read_allowed",
            Value::Bool(cross_book_read_allowed(&validated.messages)),
        );
        // Automatic local reads, before the first provider leg.
        {
            let mut closed = false;
            let mut runtime = Runtime {
                data_root: &self.turn.data_root,
                state: &mut self.turn.state,
            };
            let mut emitted: Vec<Value> = Vec::new();
            let context = {
                let reads = host.reads();
                run_auto_reads(
                    &planned,
                    &mut runtime,
                    reads,
                    &mut |record| emitted.push(record),
                    &|| chat.is_cancelled(),
                )
            };
            // `run_auto_reads` hands records out in order; they are written
            // before the provider is contacted.
            for record in emitted {
                if !host.emit(record) {
                    closed = true;
                    break;
                }
            }
            if closed {
                return Err(Stop::Closed);
            }
            if let Some(context) = context {
                insert_auto_tool_context_message(&mut messages, &context);
            }
        }
        let mut response_context = RequestContext::from_messages(&messages);
        let mut finish_reason: Option<String> = None;
        let mut content = String::new();
        let mut checkpoint = Value::Dict(Vec::new());
        let mut exhausted = true;
        let consent_timeout = self.turn.consent_timeout.unwrap_or(CONSENT_TIMEOUT);
        for _ in 0..validated.tool_loop_max_iterations {
            if chat.is_cancelled() {
                finish_reason = Some("cancelled".into());
                exhausted = false;
                break;
            }
            // Schemas a `load_tools` call added last round; already
            // authorized in `advertised_tools`, now offered to the model.
            if let Value::List(pending) =
                crate::pyval::pop(&mut self.turn.state, "pending_tool_definitions")
            {
                tools.extend(pending);
            }
            let client = self.client.as_mut().expect("client built");
            let leg = stream_leg(
                client,
                &self.turn.validated,
                &response_context,
                &tools,
                &chat,
                host,
            )?;
            {
                let notices = turn_notices(&mut self.turn.state);
                for phase in leg.notices {
                    if !notices.contains(&phase) {
                        notices.push(phase);
                    }
                }
            }
            // Every leg streams its text to the user, who sends all of it
            // back as the assistant message: the provider cursor and the
            // saved answer are bound to that visible answer.
            content.push_str(&leg.content);
            finish_reason = leg.finish_reason;
            if chat.is_cancelled() {
                finish_reason = Some("cancelled".into());
                exhausted = false;
                break;
            }
            if leg.tool_calls.is_empty() {
                exhausted = false;
                break;
            }
            // Replay the provider's typed output verbatim.
            response_context.input_items.extend(leg.response_output);
            let mut seen: Vec<PyStr> = Vec::new();
            for (index, raw) in leg.tool_calls.iter().enumerate() {
                if !matches!(raw, Value::Dict(_)) {
                    continue;
                }
                let call = parse_tool_call(raw, index);
                let duplicate = seen.contains(&call.call_id);
                seen.push(call.call_id.clone());
                let entry = get_tool_py(&call.name);
                let advertised = entry.is_some_and(|entry| is_advertised(entry, &self.turn.state));
                let kind_class = entry.map_or("unknown", |entry| entry.kind_class.as_str());
                let display_name: PyStr = entry
                    .map(|entry| PyStr::from(entry.name.as_str()))
                    .unwrap_or_else(|| call.name.clone());
                let preview = redact_tool_arguments(&call.arguments);
                let proposal_seen_at = host.now();
                // A call the consent card cannot describe is malformed: the
                // model gets a validation result instead of a consent prompt.
                let mut summary = None;
                let mut summary_refusal = None;
                if let Some(entry) = entry.filter(|entry| {
                    advertised && entry.requires_consent() && call.argument_error.is_none()
                }) {
                    match summarize_tool_call(entry, &call.arguments) {
                        Ok(text) => summary = Some(text),
                        Err(error) => summary_refusal = Some(error),
                    }
                }
                let consent_gated = entry.is_some_and(|entry| {
                    advertised
                        && !duplicate
                        && entry.requires_consent()
                        && call.argument_error.is_none()
                        && summary_refusal.is_none()
                });
                // The chat's permission mode (or an earlier "allow for this
                // chat") may approve the call without asking.
                let preapproval = entry.filter(|_| consent_gated).and_then(|entry| {
                    chat.consent.preapproval(
                        &lossy(&display_name),
                        entry.reversible,
                        entry.egresses,
                    )
                });
                let needs_consent = consent_gated && preapproval.is_none();
                if needs_consent {
                    // Registered before the record is written, so a decision
                    // that arrives at once is not lost.
                    chat.consent.expect(&lossy(&call.call_id));
                }
                let call_record = |needs_consent: bool, approval: &str| {
                    success(
                        "ai.chat.tool_call",
                        dict([
                            ("call_id", Value::Str(call.call_id.clone())),
                            ("name", Value::Str(display_name.clone())),
                            ("arguments", preview.clone()),
                            ("kind_class", s(kind_class)),
                            ("needs_consent", Value::Bool(needs_consent)),
                            ("approval", s(approval)),
                        ]),
                    )
                };
                let approval = if consent_gated {
                    approval_label(preapproval)
                } else {
                    "not_needed"
                };
                if !host.emit(call_record(needs_consent, approval)) {
                    return Err(Stop::Closed);
                }
                if chat.is_cancelled() {
                    // `_record_nonexecuted_custody_ai_call(... "cancelled_before_consent")`:
                    // the custody AI audit writer is R6-5b's.
                    let _ = proposal_seen_at;
                    finish_reason = Some("cancelled".into());
                    break;
                }
                let result = if duplicate {
                    tool_result_denied("duplicate_tool_call_id", None)
                } else if entry.is_some() && !advertised {
                    tool_result_denied("tool_not_advertised", None)
                } else if let Some(error) = summary_refusal {
                    denial_for(
                        ToolError::App(error),
                        entry.map(|entry| entry.name.as_str()),
                    )
                } else if let Some(entry) =
                    entry.filter(|entry| entry.requires_consent() && call.argument_error.is_none())
                {
                    if needs_consent {
                        let record = success(
                            "ai.chat.tool_consent_required",
                            dict([
                                ("call_id", Value::Str(call.call_id.clone())),
                                ("name", Value::Str(display_name.clone())),
                                (
                                    "summary",
                                    Value::Str(summary.clone().expect("summarised above")),
                                ),
                                ("arguments_preview", preview.clone()),
                                ("reversible", Value::Bool(entry.reversible)),
                                ("egresses", Value::Bool(entry.egresses)),
                                (
                                    "can_allow_session",
                                    Value::Bool(!crate::consent::once_only(&entry.name)),
                                ),
                            ]),
                        );
                        if !host.emit(record) {
                            return Err(Stop::Closed);
                        }
                    }
                    let decision = match preapproval {
                        Some(decision) => decision.to_owned(),
                        None => chat.consent.wait(
                            &lossy(&call.call_id),
                            &lossy(&display_name),
                            &chat.cancelled,
                            consent_timeout,
                            entry.reversible,
                            entry.egresses,
                        ),
                    };
                    if decision == "cancelled" || chat.is_cancelled() {
                        finish_reason = Some("cancelled".into());
                        break;
                    }
                    match decision.as_str() {
                        "deny" => tool_result_denied("user_denied", None),
                        "consent_timeout" => tool_result_denied("consent_timeout", None),
                        _ => {
                            // A click on this call reads "required"; a mode
                            // switch or an earlier session allow names itself.
                            let approval = if preapproval.is_some()
                                || matches!(decision.as_str(), "mode_auto" | "mode_full_access")
                            {
                                approval_label(Some(&decision))
                            } else {
                                "required"
                            };
                            if !host.emit(call_record(false, approval)) {
                                return Err(Stop::Closed);
                            }
                            // Consent follows `requires_consent`; execution
                            // follows `kind_class`. An egressing read runs
                            // through the read executor with a consent bound
                            // to this call.
                            if entry.kind_class == KindClass::Mutating {
                                execute_mutating(&call, &self.turn.state)
                            } else {
                                self.bind_consent(&call.call_id);
                                self.read(&call, host)
                            }
                        }
                    }
                } else {
                    self.read(&call, host)
                };
                record_tool_usage(&mut self.turn.state, &display_name, &result);
                let safe = redact_ai_tool_result(&result);
                update_review_checkpoint(&mut checkpoint, &lossy(&display_name), &safe);
                if !host.emit(tool_result_record(&call.call_id, &result)) {
                    return Err(Stop::Closed);
                }
                response_context.input_items.push(dict([
                    ("type", s("function_call_output")),
                    ("call_id", Value::Str(call.call_id.clone())),
                    // A visual's rows are for the screen; the model reads
                    // its digest.
                    (
                        "output",
                        s(model_output(&crate::visuals::tool_result_for_model(
                            &redact_ai_tool_result(&result),
                        ))),
                    ),
                ]));
                if chat.is_cancelled() {
                    finish_reason = Some("cancelled".into());
                    break;
                }
            }
            if finish_reason.as_deref() == Some("cancelled") {
                exhausted = false;
                break;
            }
        }
        if exhausted {
            finish_reason = Some("tool_loop_max_iterations".into());
            if matches!(&checkpoint, Value::Dict(pairs) if !pairs.is_empty()) {
                let json = dumps(
                    &dict([("review_checkpoint", checkpoint.clone())]),
                    &Options::new().sort_keys(true),
                )
                .unwrap_or_default();
                let continuation = format!(
                    "\n\nReview paused at the tool budget. Continue with the current cases; unapplied proposals must be planned again. Applied receipts remain available.\n```json\n{json}\n```"
                );
                content.push_str(&continuation);
                if !host.emit(delta_record(dict([("content", s(&continuation))]))) {
                    return Err(Stop::Closed);
                }
            }
        }
        if chat.is_cancelled() {
            finish_reason = Some("cancelled".into());
        }
        if finish_reason.as_deref() != Some("cancelled")
            && self.turn.validated.keeps_provider_session()
        {
            self.remember_provider_session(&content);
        }
        self.remember_detected_format(host);
        self.terminal(host, finish_reason, &content);
        Ok(())
    }

    /// Records the consent the user gave this call, for an egressing read.
    fn bind_consent(&mut self, call_id: &PyStr) {
        let mut ids = match get(&self.turn.state, "consented_call_ids") {
            Some(Value::List(ids)) => ids.clone(),
            _ => Vec::new(),
        };
        ids.push(Value::Str(call_id.clone()));
        set(&mut self.turn.state, "consented_call_ids", Value::List(ids));
    }

    fn read(&mut self, call: &crate::executor::ParsedCall, host: &mut dyn TurnHost) -> Value {
        let mut runtime = Runtime {
            data_root: &self.turn.data_root,
            state: &mut self.turn.state,
        };
        execute_read_only(call, &mut runtime, host.reads(), false)
    }

    /// `_remember_detected_api_format(runtime, provider_snapshot, client)`:
    /// best effort, before the terminal record, once per detection.
    fn remember_detected_format(&mut self, host: &mut dyn TurnHost) {
        let Some(detected) = self
            .client
            .as_mut()
            .and_then(ChatClient::take_detected_api_format)
        else {
            return;
        };
        let _ = host.record_detected_api_format(
            &self.turn.provider.name,
            &self.turn.provider.base_url,
            detected,
        );
    }

    /// `_write_ai_chat_terminal(...)`: provenance, the persistence policy,
    /// then the terminal record.
    fn terminal(&mut self, host: &mut dyn TurnHost, finish_reason: Option<String>, content: &str) {
        let egress = dict([
            ("records", Value::List(self.receipt.records())),
            ("gap", Value::Bool(false)),
        ]);
        let provenance = answer_provenance(
            &self.turn.provider.provenance_view(),
            &self.turn.validated.provenance_view(),
            &self.turn.state,
            &egress,
            &host.now(),
        );
        let session_id = self.persist(host, finish_reason.as_deref(), content, &provenance);
        // A new chat gets its id only now; its allows carry to the next turn.
        self.turn.registry.remember_session_allows(
            session_id.as_ref().map(lossy).as_deref(),
            &self.turn.chat.consent.session_allows(),
        );
        let mut payload = dict([
            ("provider", s(&self.turn.provider.name)),
            ("model", Value::Str(self.turn.validated.model.clone())),
            ("finish_reason", finish_reason.map_or(Value::Null, s)),
            ("provenance", provenance),
            ("session_id", session_id.map_or(Value::Null, Value::Str)),
        ]);
        // The renderer's own tokens for files this turn could no longer
        // read, so the chat drops them.
        if let Some(expired @ Value::List(tokens)) =
            get(&self.turn.state, "expired_attachment_tokens")
        {
            if !tokens.is_empty() {
                set(&mut payload, "expired_attachments", expired.clone());
            }
        }
        host.emit(success("ai.chat", payload));
    }

    /// `_persist_ai_chat_exchange(...)`: only with a persistence intent;
    /// storage failures never fail an answered chat.
    fn persist(
        &self,
        host: &mut dyn TurnHost,
        finish_reason: Option<&str>,
        content: &str,
        provenance: &Value,
    ) -> Option<PyStr> {
        let validated = &self.turn.validated;
        if validated.persist == Value::Bool(false) {
            return None;
        }
        let opted_in = validated.persist == Value::Bool(true)
            || matches!(&validated.persist, Value::Str(text) if text.is("auto"))
            || validated.session_id.is_some();
        if !opted_in {
            return None;
        }
        let user_content =
            last_user_content(validated).filter(|c| c.code_points().next().is_some())?;
        if content.is_empty() && validated.session_id.is_none() {
            // Nothing answered (cancelled before output): no empty session.
            return None;
        }
        let scope = match (
            get(&self.turn.state, "scope_workspace_id"),
            get(&self.turn.state, "scope_profile_id"),
        ) {
            (Some(Value::Str(workspace)), Some(Value::Str(profile))) => {
                Some((lossy(workspace), lossy(profile)))
            }
            _ => None,
        };
        let request = Persist {
            encrypted: false,
            scope,
            session_id: validated.session_id.clone(),
            user_content,
            assistant_content: PyStr::from(content),
            provenance: provenance.clone(),
            finish_reason: finish_reason.map(str::to_owned),
            provider: self.turn.provider.name.clone(),
            model: validated.model.clone(),
            seed_prefix: if validated.seed_history {
                seed_prefix(&validated.messages)
            } else {
                Vec::new()
            },
            seed_notices: validated.seed_notices.clone(),
            user_attachments: files::chat_message_attachments(&self.turn.state),
        };
        host.persist(request).ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_off_answers_drop_their_calls() {
        let call = dict([("id", s("c1")), ("function", dict([("name", s("status"))]))]);
        let item = dict([("type", s("function_call")), ("call_id", s("c1"))]);
        let text = dict([("type", s("message"))]);
        for reason in ["length", "content_filter"] {
            let mut calls = vec![call.clone()];
            let mut output = Some(vec![text.clone(), item.clone()]);
            drop_cut_off_calls(Some(reason), &mut calls, &mut output);
            assert!(calls.is_empty(), "{reason}");
            assert_eq!(output, Some(vec![text.clone()]), "{reason}");
        }
        let mut calls = vec![call.clone()];
        let mut output = Some(vec![item.clone()]);
        drop_cut_off_calls(Some("tool_calls"), &mut calls, &mut output);
        assert_eq!((calls.len(), output.map(|o| o.len())), (1, Some(1)));
    }

    #[test]
    fn resume_looks_up_the_transcript_before_the_new_prompt() {
        let message =
            |role: &str, content: &str| dict([("role", s(role)), ("content", s(content))]);
        let earlier = [message("user", "hi"), message("assistant", "Hello")];
        let next = [
            message("user", "hi"),
            message("assistant", "Hello"),
            message("user", "more"),
        ];
        assert_eq!(
            resume_fingerprint(&next),
            Some(history_fingerprint(&earlier))
        );
        assert_eq!(resume_fingerprint(&earlier), None);
        assert_eq!(resume_fingerprint(&[]), None);
        assert_eq!(
            resume_fingerprint(&[message("user", "only")]),
            Some(history_fingerprint(&[]))
        );
    }

    #[test]
    fn fingerprints_bind_role_and_content_only() {
        let message =
            |role: &str, content: &str| dict([("role", s(role)), ("content", s(content))]);
        let base = history_fingerprint(&[message("user", "hi")]);
        assert_eq!(
            base,
            history_fingerprint(&[dict([
                ("role", s("user")),
                ("content", s("hi")),
                ("extra", s("x"))
            ])])
        );
        assert_ne!(base, history_fingerprint(&[message("user", "hi "),]));
        assert_eq!(base.len(), 64);
        assert_eq!(
            seed_prefix(&[
                message("user", "a"),
                message("assistant", "b"),
                message("user", "c")
            ])
            .len(),
            2
        );
        assert!(seed_prefix(&[message("user", "only")]).is_empty());
    }
}

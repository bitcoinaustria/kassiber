//! The OpenAI-compatible HTTP client (`OpenAICompatibleClient` in
//! `kassiber/ai/client.py`): `GET /models`, and `POST /responses` or
//! `/chat/completions`, streamed as Server-Sent Events and read into the
//! same [`ChatDelta`] events whichever format answers.
//!
//! Every request goes through a `kassiber_net` AI route authorized by the
//! [`Gate`] the caller supplies, with consent bound to that request: no
//! proxy from the environment, redirects only to the same origin, offline
//! mode passing only a provider marked local on a loopback URL. A
//! streamed response is capped at 64 MiB and one SSE line at 16 MiB (a
//! reviewed difference: Python reads without a bound).

pub mod accumulate;
pub mod errors;
pub mod sse;
mod stream;
pub mod wire;

use std::time::Duration;

use kassiber_envelope::{dict, set, AppError};
use kassiber_net::{
    Cancel, Consent, Endpoint, Gate, HttpOptions, HttpRequest, HttpStream, RedirectPolicy, Request,
    Subsystem, TlsTrust,
};
use kassiber_pyjson::Value;

use crate::providers::{
    is_loopback_http_url, API_FORMAT_AUTO, API_FORMAT_CHAT_COMPLETIONS, API_FORMAT_RESPONSES,
    WIRE_API_FORMATS,
};
use crate::pyval::{field, s, string, text};
use errors::{
    error_body_parts, error_status, format_not_served, http_error, names_store, route_missing,
    store_refused, tools_refused, transport_error,
};
pub use wire::RequestContext;

/// `DEFAULT_TIMEOUT_SECONDS`.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
/// The cap on one streamed or buffered provider response.
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
/// The cap on one SSE line.
pub const MAX_SSE_LINE_BYTES: usize = 16 * 1024 * 1024;
pub const CONNECTION_TEST_PROMPT: &str = "Reply with OK";
pub const CONNECTION_TEST_MAX_TOKENS: i64 = 16;
pub const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// `CONNECTION_TEST_TOOL`.
pub fn connection_test_tool() -> Value {
    dict([
        ("type", s("function")),
        ("name", s("connection_check")),
        (
            "description",
            s("Not needed for the answer; offered only to check that tools are accepted."),
        ),
        (
            "parameters",
            dict([
                ("type", s("object")),
                ("properties", dict::<&str>([])),
                ("additionalProperties", Value::Bool(false)),
            ]),
        ),
        ("strict", Value::Bool(false)),
    ])
}

/// Provider notices forwarded to callers (`PROVIDER_STATUS_NOTICES`).
pub const PROVIDER_STATUS_NOTICES: &[(&str, &str)] = &[
    (
        "fast_mode_unavailable",
        "Fast mode unavailable; answering at standard speed",
    ),
    (
        "reasoning_effort_unavailable",
        "Reasoning effort unavailable; answering without it",
    ),
];

/// `ChatDelta`: one provider-neutral chunk.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatDelta {
    /// `content`, `reasoning` and normalized `tool_calls`.
    pub delta: Value,
    pub finish_reason: Option<String>,
    pub raw: Value,
    /// The terminal chunk's output Items.
    pub response_output: Option<Vec<Value>>,
    /// A notice for the turn, with an empty delta.
    pub status_phase: Option<String>,
}

impl ChatDelta {
    fn new(delta: Value, finish_reason: Option<String>, raw: Value) -> Self {
        Self {
            delta,
            finish_reason,
            raw,
            response_output: None,
            status_phase: None,
        }
    }
}

/// One generation request.
pub struct ChatRequest<'a> {
    pub context: &'a RequestContext,
    pub model: &'a str,
    pub options: &'a Value,
    pub tools: Option<&'a [Value]>,
    pub tool_choice: &'a Value,
}

/// `_WireCompat`: optional parts an endpoint refused, sticky for the client.
#[derive(Clone, Copy)]
struct Compat {
    include: bool,
    effort: bool,
    reasoning_replay: bool,
    max_completion_tokens: bool,
}

impl Default for Compat {
    fn default() -> Self {
        Self {
            include: true,
            effort: true,
            reasoning_replay: true,
            max_completion_tokens: false,
        }
    }
}

/// `_PreparedRequest`.
struct Prepared {
    path: &'static str,
    body: Value,
    include_sent: bool,
    effort_sent: bool,
    replay_sent: bool,
    max_tokens_sent: bool,
    store_sent: bool,
}

/// How a client is configured from a provider row.
#[derive(Clone)]
pub struct ClientConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub timeout: Duration,
    /// A provider marked local: only it passes offline mode, on loopback.
    pub on_device: bool,
    pub api_format: String,
    pub detected_api_format: Option<String>,
    pub omit_store_field: bool,
}

impl ClientConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: None,
            timeout: DEFAULT_TIMEOUT,
            on_device: false,
            api_format: API_FORMAT_AUTO.into(),
            detected_api_format: None,
            omit_store_field: false,
        }
    }
}

/// `OpenAICompatibleClient`. One per turn: the first accepted request fixes
/// the wire format for the rest of its life.
pub struct HttpClient {
    config: ClientConfig,
    gate: Gate,
    cancel: Cancel,
    compat: Compat,
    /// Notices from blocking calls; streams yield theirs as chunks.
    pub notices: Vec<String>,
    wire_format: Option<&'static str>,
    answers: usize,
    fell_back: bool,
    /// Set once a Chat Completions answer arrived after the auto fallback.
    pub newly_detected_api_format: Option<&'static str>,
}

fn wire(value: &str) -> Option<&'static str> {
    WIRE_API_FORMATS
        .iter()
        .find(|format| **format == value)
        .copied()
}

impl HttpClient {
    pub fn new(config: ClientConfig, gate: Gate) -> Self {
        Self {
            config,
            gate,
            cancel: Cancel::new(),
            compat: Compat::default(),
            notices: Vec::new(),
            wire_format: None,
            answers: 0,
            fell_back: false,
            newly_detected_api_format: None,
        }
    }

    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    /// A handle that cancels this client from another thread: refuses any
    /// further request and shuts the one in flight down.
    pub fn canceller(&self) -> Cancel {
        self.cancel.clone()
    }

    fn cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// `wire_format()`: the format the next generation request speaks.
    pub fn wire_format(&self) -> &'static str {
        if let Some(format) = self.wire_format {
            return format;
        }
        wire(&self.config.api_format)
            .or_else(|| self.config.detected_api_format.as_deref().and_then(wire))
            .unwrap_or(API_FORMAT_RESPONSES)
    }

    /// `_open(path, ...)`: one gated request; a refused one becomes its
    /// mapped error.
    fn open(
        &self,
        path: &str,
        body: Option<Vec<u8>>,
        accept_sse: bool,
        timeout: Duration,
    ) -> Result<HttpStream, AppError> {
        if self.cancelled() {
            return Err(errors::cancelled());
        }
        let url = format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        let parsed = kassiber_net::HttpUrl::parse(&url)
            .map_err(|_| AppError::new("ai_request_invalid", "Invalid AI provider URL"))?;
        let tls = if parsed.scheme() == "https" {
            TlsTrust::python_default().map_err(|error| error.app_error())?
        } else {
            TlsTrust::Plaintext
        };
        let endpoint = Endpoint::new(
            parsed
                .host_str()
                .unwrap_or_default()
                .trim_matches(['[', ']']),
            parsed.port_or_known_default().unwrap_or(0),
        )
        .map_err(|error| error.app_error())?;
        let mut request = Request::new(Subsystem::Ai, endpoint, tls);
        if self.config.on_device && is_loopback_http_url(&self.config.base_url) {
            request = request.on_device_ai();
        }
        let consent = Consent::explicit_trigger(&request);
        let route = self
            .gate
            .authorize(request, Some(consent))
            .map_err(transport_error)?;
        let method = if body.is_some() { "POST" } else { "GET" };
        let invalid = |error: kassiber_net::Error| transport_error(error);
        let mut http = HttpRequest::new(method, &url)
            .map_err(invalid)?
            .header(
                "Accept",
                if accept_sse {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            )
            .map_err(invalid)?
            .header("User-Agent", "kassiber/ai")
            .map_err(invalid)?;
        if let Some(body) = body {
            http = http
                .header("Content-Type", "application/json")
                .map_err(invalid)?
                .body(body);
        }
        if let Some(key) = self.config.api_key.as_deref().filter(|key| !key.is_empty()) {
            http = http
                .header("Authorization", &format!("Bearer {key}"))
                .map_err(invalid)?;
        }
        let opened = http.cancellable(&self.cancel).open(
            route,
            HttpOptions {
                timeout,
                redirects: RedirectPolicy::SameOrigin,
                max_response_bytes: MAX_RESPONSE_BYTES,
                ..Default::default()
            },
        );
        let mut stream = match opened {
            Ok(stream) => stream,
            Err(_) if self.cancelled() => return Err(errors::cancelled()),
            Err(error) => return Err(transport_error(error)),
        };
        if !(200..300).contains(&stream.status) {
            let body = stream.read_to_end().unwrap_or_default();
            return Err(http_error(stream.status, &String::from_utf8_lossy(&body)));
        }
        if self.cancelled() {
            return Err(errors::cancelled());
        }
        Ok(stream)
    }

    fn read_json(&self, stream: &mut HttpStream) -> Result<Result<Value, String>, AppError> {
        let body = stream
            .read_to_end()
            .map_err(|error| errors::network_error(&error.to_string()))?;
        Ok(kassiber_pytext::loads(&String::from_utf8_lossy(&body)).map_err(|e| e.to_string()))
    }

    /// `list_models(strict=...)`: `GET /models`. Forgiving by default: a
    /// 4xx or an unexpected body lists nothing.
    pub fn list_models(&mut self, strict: bool) -> Result<Vec<Value>, AppError> {
        let mut stream = match self.open("models", None, false, self.config.timeout) {
            Ok(stream) => stream,
            Err(error) if error.code == "ai_request_invalid" && !strict => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let payload = match self.read_json(&mut stream)? {
            Ok(payload) => payload,
            Err(_) if strict => {
                return Err(AppError::new(
                    "ai_request_invalid",
                    "AI provider returned a non-JSON models response",
                )
                .with_hint("Check that the base URL points at an OpenAI-compatible /v1 endpoint."))
            }
            Err(_) => return Ok(Vec::new()),
        };
        let Value::List(data) = field(&payload, "data") else {
            if strict {
                return Err(AppError::new(
                    "ai_request_invalid",
                    "AI provider models response did not contain a data list",
                )
                .with_details(dict([("shape", s(kassiber_envelope::type_name(&payload)))]))
                .with_hint(
                    "Check that the base URL points at an OpenAI-compatible /v1 endpoint.",
                ));
            }
            return Ok(Vec::new());
        };
        let mut models = Vec::new();
        for item in data {
            let Some(id) = string(field(item, "id")) else {
                continue;
            };
            let mut row = dict([("id", s(id))]);
            if let owned @ Value::Str(_) = field(item, "owned_by") {
                set(&mut row, "owned_by", owned.clone());
            }
            for (key, value) in
                crate::pyval::entries(&crate::discovery::metadata::http_model_capabilities(item))
            {
                set(&mut row, &key, value.clone());
            }
            models.push(row);
        }
        Ok(models)
    }

    fn prepare(
        &self,
        format: &str,
        omit_store: bool,
        request: &ChatRequest<'_>,
        stream: bool,
    ) -> Prepared {
        if format == API_FORMAT_CHAT_COMPLETIONS {
            let body = wire::chat_completions_body(
                request.context,
                request.model,
                stream,
                wire::ChatBodyOptions {
                    options: request.options,
                    tools: request.tools,
                    tool_choice: request.tool_choice,
                    omit_store,
                    replay_reasoning: self.compat.reasoning_replay,
                    max_completion_tokens: self.compat.max_completion_tokens,
                    effort: self.compat.effort,
                    base_url: &self.config.base_url,
                },
            );
            let replay_sent = match field(&body, "messages") {
                Value::List(messages) => messages.iter().any(|message| {
                    ["reasoning_details", "reasoning_content", "reasoning"]
                        .iter()
                        .any(|key| crate::pyval::has(message, key))
                }),
                _ => false,
            };
            return Prepared {
                path: "chat/completions",
                effort_sent: crate::pyval::has(&body, "reasoning_effort")
                    || crate::pyval::has(&body, "reasoning"),
                max_tokens_sent: crate::pyval::has(&body, "max_tokens"),
                store_sent: crate::pyval::has(&body, "store"),
                include_sent: false,
                replay_sent,
                body,
            };
        }
        // The Responses API never goes without `store: false`.
        let mut body = wire::responses_options(request.options);
        if !self.compat.effort {
            crate::pyval::pop(&mut body, "reasoning");
        }
        let effort_set = kassiber_envelope::truthy(field(field(&body, "reasoning"), "effort"));
        let tools_given = request.tools.is_some_and(|tools| !tools.is_empty());
        if self.compat.include && (effort_set || tools_given) {
            set(
                &mut body,
                "include",
                Value::List(vec![s(wire::REASONING_INCLUDE)]),
            );
        }
        if let Some(tools) = request.tools {
            set(
                &mut body,
                "tools",
                Value::List(wire::responses_tools(tools)),
            );
        }
        if *request.tool_choice != Value::Null {
            set(
                &mut body,
                "tool_choice",
                wire::responses_tool_choice(request.tool_choice),
            );
        }
        let input = wire::replayable_input_items(
            &request.context.input_items,
            self.compat.reasoning_replay,
        );
        let replay_sent = input
            .iter()
            .any(|item| *field(item, "type") == s("reasoning"));
        set(&mut body, "model", s(request.model));
        set(&mut body, "input", Value::List(input));
        set(&mut body, "stream", Value::Bool(stream));
        set(&mut body, "store", Value::Bool(false));
        if let Some(instructions) = request
            .context
            .instructions
            .as_ref()
            .filter(|i| !i.is_empty())
        {
            set(&mut body, "instructions", s(instructions));
        }
        Prepared {
            path: "responses",
            include_sent: crate::pyval::has(&body, "include"),
            effort_sent: effort_set,
            replay_sent,
            max_tokens_sent: false,
            store_sent: true,
            body,
        }
    }

    /// `_relax_refused_parts(exc, prepared)`: drop one optional part a
    /// 400/422 names, at most once per part for the client.
    fn relax(&mut self, error: &AppError, prepared: &Prepared) -> Option<&'static str> {
        if !matches!(error_status(error, false), Some(400 | 422)) {
            return None;
        }
        let parts = error_body_parts(error)?;
        if prepared.max_tokens_sent
            && !self.compat.max_completion_tokens
            && (parts.fields.is_empty() || parts.fields.iter().any(|f| f == "max_tokens"))
            && parts
                .messages
                .iter()
                .any(|message| message.contains("max_completion_tokens"))
        {
            self.compat.max_completion_tokens = true;
            return Some("max_tokens");
        }
        if prepared.include_sent && parts.names(&["include", "encrypted_content"]) {
            self.compat.include = false;
            return Some("include");
        }
        if parts.names(&["reasoning", "effort", "summary", "think"]) {
            let names_replay =
                parts.names(&["reasoning_content", "reasoning_details", "reasoning item"])
                    || parts.paths.iter().any(|path| {
                        (path.starts_with("messages.") || path.starts_with("input."))
                            && path.contains("reasoning")
                    });
            let names_effort = parts.names(&["effort", "think"]);
            if prepared.effort_sent && self.compat.effort && (names_effort || !names_replay) {
                self.compat.effort = false;
                return Some("reasoning");
            }
            if prepared.replay_sent
                && self.compat.reasoning_replay
                && (names_replay || !prepared.effort_sent)
            {
                self.compat.reasoning_replay = false;
                return Some("reasoning_replay");
            }
        }
        None
    }

    /// `_post_generation(...)`: POST, retrying without refused parts; never
    /// without `store: false`.
    fn post(
        &mut self,
        format: &str,
        omit_store: bool,
        request: &ChatRequest<'_>,
        stream: bool,
        timeout: Duration,
    ) -> Result<(HttpStream, Vec<String>), AppError> {
        let mut notices = Vec::new();
        loop {
            let prepared = self.prepare(format, omit_store, request, stream);
            let body = kassiber_pyjson::dumps(&prepared.body, &Default::default())
                .map_err(|error| AppError::new("internal_error", error.to_string()))?;
            match self.open(prepared.path, Some(body.into_bytes()), stream, timeout) {
                Ok(response) => return Ok((response, notices)),
                Err(error) => {
                    if prepared.store_sent && names_store(&error) {
                        return Err(store_refused(error));
                    }
                    match self.relax(&error, &prepared) {
                        None => return Err(error),
                        Some("reasoning") if prepared.effort_sent => {
                            notices.push("reasoning_effort_unavailable".into())
                        }
                        Some(_) => {}
                    }
                }
            }
        }
    }

    fn may_fall_back(&self) -> bool {
        self.config.api_format == API_FORMAT_AUTO
            && self.config.detected_api_format.is_none()
            && self.wire_format.is_none()
    }

    /// `_start(...)`: one generation request. Only an `auto` provider with
    /// nothing detected falls back, once, to Chat Completions on a missing
    /// Responses route.
    fn start(
        &mut self,
        request: &ChatRequest<'_>,
        stream: bool,
        timeout: Duration,
        api_format: Option<&'static str>,
        omit_store: Option<bool>,
    ) -> Result<(&'static str, HttpStream, Vec<String>), AppError> {
        let mut format = api_format.unwrap_or_else(|| self.wire_format());
        let omit = omit_store.unwrap_or(self.config.omit_store_field);
        let (response, notices) = match self.post(format, omit, request, stream, timeout) {
            Ok(sent) => sent,
            Err(error) => {
                if !(api_format.is_none()
                    && format == API_FORMAT_RESPONSES
                    && self.may_fall_back()
                    && route_missing(&error))
                {
                    return Err(error);
                }
                format = API_FORMAT_CHAT_COMPLETIONS;
                let sent = match self.post(format, omit, request, stream, timeout) {
                    Ok(sent) => sent,
                    Err(error) if route_missing(&error) => {
                        let mut refused = AppError::new(
                            "ai_request_invalid",
                            "AI provider has neither a Responses nor a Chat Completions endpoint at this base URL",
                        )
                        .with_hint("Check that the base URL is the API root, usually ending in /v1.");
                        refused.details = error.details;
                        return Err(refused);
                    }
                    Err(error) => return Err(error),
                };
                self.fell_back = true;
                sent
            }
        };
        self.wire_format = Some(format);
        Ok((format, response, notices))
    }

    fn note_answer(&mut self, format: &str) {
        if self.fell_back && format == API_FORMAT_CHAT_COMPLETIONS {
            self.newly_detected_api_format = Some(API_FORMAT_CHAT_COMPLETIONS);
        }
    }

    /// `chat(...)`: one blocking answer in the daemon's shape.
    pub fn chat(
        &mut self,
        request: &ChatRequest<'_>,
        timeout: Option<Duration>,
    ) -> Result<Value, AppError> {
        let timeout = timeout.unwrap_or(self.config.timeout);
        let (format, mut response, notices) = self.start(request, false, timeout, None, None)?;
        for notice in notices {
            if !self.notices.contains(&notice) {
                self.notices.push(notice);
            }
        }
        let payload = self.read_json(&mut response)?.map_err(|reason| {
            AppError::new(
                "ai_request_invalid",
                "AI provider returned a non-JSON response",
            )
            .with_details(dict([("reason", s(reason))]))
        })?;
        if !matches!(payload, Value::Dict(_)) {
            return Err(AppError::new(
                "ai_request_invalid",
                "AI provider returned an invalid response payload",
            ));
        }
        let result = if format == API_FORMAT_CHAT_COMPLETIONS {
            self.chat_completions_result(&payload)?
        } else {
            responses_result(&payload)?
        };
        self.note_answer(format);
        Ok(result)
    }

    fn chat_completions_result(&mut self, payload: &Value) -> Result<Value, AppError> {
        if let Some(error) = wire::chat_completion_stream_error(payload) {
            return Err(error);
        }
        let choice = match field(payload, "choices") {
            Value::List(choices) => choices.first().filter(|c| matches!(c, Value::Dict(_))),
            _ => None,
        }
        .ok_or_else(|| {
            AppError::new(
                "ai_request_invalid",
                "AI provider returned a Chat Completions payload without choices",
            )
        })?;
        let message = field(choice, "message");
        let mut content = wire::chat_text(field(message, "content"));
        if let Some(refusal) = string(field(message, "refusal")) {
            content.push_str(&refusal);
        }
        let reasoning_field = wire::CHAT_REASONING_FIELDS
            .iter()
            .find(|name| text(field(message, name)).is_some())
            .copied();
        let reasoning = reasoning_field
            .and_then(|name| text(field(message, name)))
            .unwrap_or_default();
        let details: Vec<Value> = match field(message, "reasoning_details") {
            Value::List(details) => details
                .iter()
                .filter(|d| matches!(d, Value::Dict(_)))
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        self.answers += 1;
        let mut calls = accumulate::normalized_tool_calls(
            field(message, "tool_calls"),
            &format!("call_{}_", self.answers),
        )?;
        let finish = wire::chat_finish_reason(field(choice, "finish_reason"), !calls.is_empty());
        if finish == "length" || finish == "content_filter" {
            calls.clear();
        }
        let mut result = dict([
            ("role", s("assistant")),
            ("content", s(&content)),
            ("finish_reason", s(&finish)),
            ("usage", field(payload, "usage").clone()),
            (
                "response_output",
                Value::List(wire::chat_completion_output(
                    &content,
                    &reasoning,
                    reasoning_field,
                    details,
                    &calls,
                )),
            ),
        ]);
        if !reasoning.is_empty() {
            set(&mut result, "reasoning", s(reasoning));
        }
        if !calls.is_empty() {
            set(&mut result, "tool_calls", Value::List(calls));
        }
        Ok(result)
    }

    /// `stream_chat(...)`: the answer as [`ChatDelta`]s handed to `emit`.
    pub fn stream_chat(
        &mut self,
        request: &ChatRequest<'_>,
        emit: &mut dyn FnMut(ChatDelta) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let timeout = self.config.timeout;
        self.stream(request, timeout, None, None, emit)
    }

    fn stream(
        &mut self,
        request: &ChatRequest<'_>,
        timeout: Duration,
        api_format: Option<&'static str>,
        omit_store: Option<bool>,
        emit: &mut dyn FnMut(ChatDelta) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let (format, mut response, notices) =
            self.start(request, true, timeout, api_format, omit_store)?;
        for phase in notices {
            let mut notice = ChatDelta::new(dict::<&str>([]), None, dict::<&str>([]));
            notice.status_phase = Some(phase);
            emit(notice)?;
        }
        let cancel = self.cancel.clone();
        let mut splitter = sse::Splitter::new();
        let mut finished = false;
        let mut events = || -> Result<Option<sse::Event>, AppError> {
            if finished {
                return Ok(None);
            }
            loop {
                let line = match response.read_line(MAX_SSE_LINE_BYTES) {
                    Ok(line) => line,
                    // A cancelled turn ends quietly, as Python's closed
                    // response does.
                    Err(_) if cancel.is_cancelled() => return Ok(None),
                    Err(error) => return Err(errors::network_error(&error.to_string())),
                };
                let Some(line) = line else {
                    finished = true;
                    return splitter.finish();
                };
                if let Some(event) = splitter.line(&String::from_utf8_lossy(&line))? {
                    if matches!(event, sse::Event::Done) {
                        finished = true;
                    }
                    return Ok(Some(event));
                }
            }
        };
        if format == API_FORMAT_CHAT_COMPLETIONS {
            self.answers += 1;
            let prefix = format!("call_{}_", self.answers);
            stream::chat_completions(&cancel, &prefix, &mut events, emit)?;
        } else {
            stream::responses(&cancel, &mut events, emit)?;
        }
        if !self.cancelled() {
            self.note_answer(format);
        }
        Ok(())
    }

    /// `probe(model=...)`: the connection test's fixed request. `auto`
    /// tries Responses, then Chat Completions when that is not served.
    pub fn probe(&mut self, model: &str, timeout: Duration) -> Result<Value, AppError> {
        let formats: Vec<&'static str> = match wire(&self.config.api_format) {
            Some(format) => vec![format],
            None => WIRE_API_FORMATS.to_vec(),
        };
        for (position, format) in formats.iter().enumerate() {
            match self.probe_in(format, model, timeout) {
                Err(error) if position + 1 < formats.len() && format_not_served(&error) => continue,
                result => return result,
            }
        }
        unreachable!("the last format returns")
    }

    fn probe_in(
        &mut self,
        format: &'static str,
        model: &str,
        timeout: Duration,
    ) -> Result<Value, AppError> {
        let mut with_tool = true;
        let mut omit_store = self.config.omit_store_field && format == API_FORMAT_CHAT_COMPLETIONS;
        let mut store_accepted = if omit_store {
            Value::Null
        } else {
            Value::Bool(true)
        };
        let finish = loop {
            match self.probe_once(format, model, timeout, with_tool, omit_store) {
                Ok(finish) => break finish,
                Err(error)
                    if error.code == "ai_store_refused"
                        && !omit_store
                        && format == API_FORMAT_CHAT_COMPLETIONS =>
                {
                    omit_store = true;
                    store_accepted = Value::Bool(false);
                }
                Err(error) if with_tool && tools_refused(&error) => with_tool = false,
                Err(error) => return Err(error),
            }
        };
        Ok(dict([
            ("api_format", s(format)),
            ("tools_accepted", Value::Bool(with_tool)),
            ("store_accepted", store_accepted),
            ("finish_reason", crate::pyval::opt(finish)),
        ]))
    }

    fn probe_once(
        &mut self,
        format: &'static str,
        model: &str,
        timeout: Duration,
        with_tool: bool,
        omit_store: bool,
    ) -> Result<Option<String>, AppError> {
        let context = RequestContext {
            instructions: None,
            input_items: vec![dict([
                ("type", s("message")),
                ("role", s("user")),
                ("content", s(CONNECTION_TEST_PROMPT)),
            ])],
        };
        let tools = [connection_test_tool()];
        let options = dict([("max_tokens", Value::from(CONNECTION_TEST_MAX_TOKENS))]);
        let choice = if with_tool { s("auto") } else { Value::Null };
        let request = ChatRequest {
            context: &context,
            model,
            options: &options,
            tools: with_tool.then_some(&tools[..]),
            tool_choice: &choice,
        };
        let mut terminal: Option<Option<String>> = None;
        self.stream(
            &request,
            timeout,
            Some(format),
            Some(omit_store),
            &mut |chunk| {
                if chunk.response_output.is_some() {
                    terminal = Some(chunk.finish_reason);
                }
                Ok(())
            },
        )?;
        terminal.ok_or_else(|| {
            AppError::new(
                "ai_request_invalid",
                "AI provider did not finish an answer to the connection test",
            )
            .with_details(dict([
                ("reason", s("no_answer")),
                ("api_format", s(format)),
            ]))
            .with_hint("Check the base URL and the provider's API format.")
        })
    }
}

/// `_responses_result(payload)`.
fn responses_result(payload: &Value) -> Result<Value, AppError> {
    if *field(payload, "status") == s("failed") || matches!(field(payload, "error"), Value::Dict(_))
    {
        return Err(wire::response_stream_error(&dict([
            ("type", s("response.failed")),
            ("response", payload.clone()),
        ])));
    }
    let reasoning = wire::response_reasoning_summary(payload);
    let mut result = dict([
        ("role", s("assistant")),
        ("content", s(wire::response_text(payload))),
        (
            "finish_reason",
            crate::pyval::opt(wire::response_finish_reason(payload)),
        ),
        ("usage", field(payload, "usage").clone()),
        (
            "response_output",
            Value::List(wire::response_output(payload)),
        ),
    ]);
    if let Some(reasoning) = reasoning {
        set(&mut result, "reasoning", s(reasoning));
    }
    let calls = wire::response_tool_calls(payload);
    if !calls.is_empty() {
        set(&mut result, "tool_calls", Value::List(calls));
    }
    Ok(result)
}

/// `connection_test_model(requested, models)`: the given model, else the
/// only listed one.
pub fn connection_test_model(requested: Option<&str>, models: &[Value]) -> Option<String> {
    if let Some(requested) = requested {
        let stripped = kassiber_pytext::strip(requested);
        if !stripped.is_empty() {
            return Some(stripped.to_owned());
        }
    }
    let ids: Vec<String> = models.iter().filter_map(|m| text(field(m, "id"))).collect();
    (ids.len() == 1).then(|| ids[0].clone())
}

/// `run_connection_test(client, http=True, model=...)` for an HTTP
/// provider: list its models strictly, then send the fixed request.
pub fn run_connection_test(
    client: &mut HttpClient,
    model: Option<&str>,
) -> Result<Value, AppError> {
    let models = client.list_models(true)?;
    let mut result = dict([
        ("model_count", Value::from(models.len())),
        ("models", Value::List(models.clone())),
        ("check_kind", s("models")),
        ("api_format", s(&client.config.api_format)),
        ("detected_api_format", Value::Null),
        ("tools_accepted", Value::Null),
        ("store_accepted", Value::Null),
        (
            "probe",
            dict([
                ("status", s("skipped")),
                ("reason", s("no_model")),
                ("model", Value::Null),
                ("finish_reason", Value::Null),
            ]),
        ),
    ]);
    let Some(probe_model) = connection_test_model(model, &models) else {
        return Ok(result);
    };
    let outcome = client.probe(&probe_model, CONNECTION_TEST_TIMEOUT)?;
    set(
        &mut result,
        "detected_api_format",
        field(&outcome, "api_format").clone(),
    );
    set(
        &mut result,
        "tools_accepted",
        field(&outcome, "tools_accepted").clone(),
    );
    set(
        &mut result,
        "store_accepted",
        field(&outcome, "store_accepted").clone(),
    );
    set(
        &mut result,
        "probe",
        dict([
            ("status", s("ok")),
            ("reason", Value::Null),
            ("model", s(probe_model)),
            ("finish_reason", field(&outcome, "finish_reason").clone()),
        ]),
    );
    Ok(result)
}

//! The two wire formats: the daemon's Responses-style input Items turned
//! into Responses or Chat Completions request bodies, and their answers
//! read back into the same output Items (`kassiber/ai/client.py`).

use kassiber_envelope::{dict, set, truthy, AppError};
use kassiber_pyjson::Value;

use super::errors::provider_error;
use crate::providers::http_reasoning_effort;
use crate::pyval::{entries, field, has, lossy, pop, s, string, text};
use kassiber_credentials::Forms;

/// `ResponsesRequestContext`: explicit stateless Responses input.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RequestContext {
    pub instructions: Option<String>,
    pub input_items: Vec<Value>,
}

impl RequestContext {
    /// `responses_request_context(messages)`: chat messages as Responses
    /// instructions and Items.
    pub fn from_messages(messages: &[Value]) -> Self {
        let mut instructions = Vec::new();
        let mut items = Vec::new();
        for message in messages {
            let role = field(message, "role");
            let content = field(message, "content");
            if *role == s("system") {
                if let Some(content) = text(content) {
                    instructions.push(content);
                }
                continue;
            }
            if *role == s("tool") {
                if let (Some(call_id), Value::Str(_)) =
                    (text(field(message, "tool_call_id")), content)
                {
                    items.push(dict([
                        ("type", s("function_call_output")),
                        ("call_id", s(call_id)),
                        ("output", content.clone()),
                    ]));
                }
                continue;
            }
            if ![s("user"), s("assistant"), s("developer")].contains(role) {
                continue;
            }
            let assistant = *role == s("assistant");
            if let Some(content) = responses_message_content(content) {
                let empty = matches!(&content, Value::Str(t) if t.code_points().next().is_none())
                    || matches!(&content, Value::List(l) if l.is_empty());
                if !empty || !assistant {
                    items.push(dict([
                        ("type", s("message")),
                        ("role", role.clone()),
                        ("content", content),
                    ]));
                }
            }
            if !assistant {
                continue;
            }
            let Value::List(calls) = field(message, "tool_calls") else {
                continue;
            };
            for call in calls {
                let function = field(call, "function");
                if !matches!(function, Value::Dict(_)) {
                    continue;
                }
                let (Some(id), Some(name), Some(arguments)) = (
                    text(field(call, "id")),
                    text(field(function, "name")),
                    string(field(function, "arguments")),
                ) else {
                    continue;
                };
                items.push(dict([
                    ("type", s("function_call")),
                    ("call_id", s(id)),
                    ("name", s(name)),
                    ("arguments", s(arguments)),
                ]));
            }
        }
        RequestContext {
            instructions: (!instructions.is_empty()).then(|| instructions.join("\n\n")),
            input_items: items,
        }
    }
}

/// `_responses_message_content(content)`.
fn responses_message_content(content: &Value) -> Option<Value> {
    match content {
        Value::Str(_) => return Some(content.clone()),
        Value::List(_) => {}
        _ => return None,
    }
    let Value::List(raw_parts) = content else {
        unreachable!()
    };
    let mut parts = Vec::new();
    for part in raw_parts {
        if !matches!(part, Value::Dict(_)) {
            continue;
        }
        let kind = field(part, "type");
        if *kind == s("text") && matches!(field(part, "text"), Value::Str(_)) {
            parts.push(dict([
                ("type", s("input_text")),
                ("text", field(part, "text").clone()),
            ]));
            continue;
        }
        if *kind == s("image_url") {
            let image = field(part, "image_url");
            let url = if matches!(image, Value::Dict(_)) {
                field(image, "url")
            } else {
                image
            };
            if let Some(url) = text(url) {
                let mut item = dict([("type", s("input_image")), ("image_url", s(url))]);
                if let Some(detail) = text(field(image, "detail")) {
                    set(&mut item, "detail", s(detail));
                }
                parts.push(item);
            }
            continue;
        }
        if [s("input_text"), s("input_image"), s("input_file")].contains(kind) {
            parts.push(part.clone());
        }
    }
    Some(Value::List(parts))
}

/// `_responses_tools(tools)`.
pub fn responses_tools(tools: &[Value]) -> Vec<Value> {
    let mut normalized = Vec::new();
    for tool in tools {
        if !matches!(tool, Value::Dict(_)) || *field(tool, "type") != s("function") {
            continue;
        }
        let function = field(tool, "function");
        let item = if matches!(function, Value::Dict(_)) {
            let mut item = dict([("type", s("function"))]);
            for (key, value) in entries(function) {
                set(&mut item, &key, value.clone());
            }
            item
        } else {
            tool.clone()
        };
        if text(field(&item, "name")).is_some() {
            normalized.push(item);
        }
    }
    normalized
}

/// `_responses_tool_choice(tool_choice)`.
pub fn responses_tool_choice(choice: &Value) -> Value {
    if !matches!(choice, Value::Dict(_)) {
        return choice.clone();
    }
    let function = field(choice, "function");
    if matches!(function, Value::Dict(_)) && matches!(field(function, "name"), Value::Str(_)) {
        return dict([
            ("type", s("function")),
            ("name", field(function, "name").clone()),
        ]);
    }
    choice.clone()
}

const RESERVED_OPTIONS: &[&str] = &[
    "conversation",
    "include",
    "input",
    "instructions",
    "messages",
    "model",
    "previous_response_id",
    "store",
    "sensitive_context",
    "stateless_session",
    "fast_mode",
    "provider_session_id",
    "stream",
    "tool_choice",
    "tools",
];

/// `_responses_options(options)`: Chat-era generation options as Responses
/// fields, without anything a caller may not set.
pub fn responses_options(options: &Value) -> Value {
    responses_options_with(options, http_reasoning_effort)
}

/// How a request's `reasoning_effort` option becomes the effort it sends:
/// `http_reasoning_effort` for an OpenAI-compatible endpoint, the proxy's
/// own levels for a CLI proxy.
pub type EffortLevels = fn(&Value) -> Option<&'static str>;

/// `responses_options` with the provider's effort levels.
pub fn responses_options_with(options: &Value, efforts: EffortLevels) -> Value {
    let mut body = match options {
        Value::Dict(_) => options.clone(),
        _ => dict::<&str>([]),
    };
    let max_tokens = pop(&mut body, "max_tokens");
    if max_tokens != Value::Null && !has(&body, "max_output_tokens") {
        set(&mut body, "max_output_tokens", max_tokens);
    }
    let format = pop(&mut body, "response_format");
    if matches!(format, Value::Dict(_)) {
        let mut text_options = match field(&body, "text") {
            value @ Value::Dict(_) => value.clone(),
            _ => dict::<&str>([]),
        };
        if !has(&text_options, "format") {
            set(&mut text_options, "format", format);
        }
        set(&mut body, "text", text_options);
    }
    if let Some(effort) = efforts(&pop(&mut body, "reasoning_effort")) {
        let mut reasoning = match field(&body, "reasoning") {
            value @ Value::Dict(_) => value.clone(),
            _ => dict::<&str>([]),
        };
        set(&mut reasoning, "effort", s(effort));
        set(&mut body, "reasoning", reasoning);
    }
    for key in RESERVED_OPTIONS {
        pop(&mut body, key);
    }
    body
}

pub const REASONING_INCLUDE: &str = "reasoning.encrypted_content";

/// `_replayable_input_items(items, keep_reasoning=...)`: reasoning Items
/// only with their encrypted content.
pub fn replayable_input_items(items: &[Value], keep_reasoning: bool) -> Vec<Value> {
    items
        .iter()
        .filter(|item| {
            !(matches!(item, Value::Dict(_))
                && *field(item, "type") == s("reasoning")
                && (!keep_reasoning || text(field(item, "encrypted_content")).is_none()))
        })
        .cloned()
        .collect()
}

/// `_response_output(payload)`: the output Items, without a function call
/// an incomplete response may have cut off.
pub fn response_output(payload: &Value) -> Vec<Value> {
    let Value::List(output) = field(payload, "output") else {
        return Vec::new();
    };
    if !matches!(payload, Value::Dict(_)) {
        return Vec::new();
    }
    let incomplete = *field(payload, "status") == s("incomplete");
    output
        .iter()
        .filter(|item| {
            matches!(item, Value::Dict(_))
                && !(incomplete
                    && *field(item, "type") == s("function_call")
                    && *field(item, "status") != s("completed"))
        })
        .cloned()
        .collect()
}

fn or_value(first: &Value, second: &Value, fallback: Value) -> Value {
    if truthy(first) {
        first.clone()
    } else if truthy(second) {
        second.clone()
    } else {
        fallback
    }
}

/// `_response_tool_calls(payload)`.
pub fn response_tool_calls(payload: &Value) -> Vec<Value> {
    response_output(payload)
        .iter()
        .enumerate()
        .filter(|(_, item)| *field(item, "type") == s("function_call"))
        .map(|(index, item)| {
            dict([
                (
                    "id",
                    or_value(
                        field(item, "call_id"),
                        field(item, "id"),
                        s(format!("call_{index}")),
                    ),
                ),
                ("type", s("function")),
                (
                    "function",
                    dict([
                        ("name", or_value(field(item, "name"), &Value::Null, s(""))),
                        (
                            "arguments",
                            or_value(field(item, "arguments"), &Value::Null, s("")),
                        ),
                    ]),
                ),
            ])
        })
        .collect()
}

/// `_response_text(payload)`.
pub fn response_text(payload: &Value) -> String {
    let mut out = String::new();
    for item in response_output(payload) {
        let Value::List(content) = field(&item, "content") else {
            continue;
        };
        if *field(&item, "type") != s("message") {
            continue;
        }
        for part in content {
            if !matches!(part, Value::Dict(_)) {
                continue;
            }
            let mut piece = if *field(part, "type") == s("output_text") {
                string(field(part, "text"))
            } else {
                None
            };
            if piece.is_none() && *field(part, "type") == s("refusal") {
                piece = string(field(part, "refusal"));
            }
            if let Some(piece) = piece {
                out.push_str(&piece);
            }
        }
    }
    out
}

/// `_response_reasoning_summary(payload)`.
pub fn response_reasoning_summary(payload: &Value) -> Option<String> {
    let mut out = String::new();
    for item in response_output(payload) {
        let Value::List(summary) = field(&item, "summary") else {
            continue;
        };
        if *field(&item, "type") != s("reasoning") {
            continue;
        }
        for part in summary {
            if let Some(piece) = string(field(part, "text")) {
                out.push_str(&piece);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

fn incomplete_reason(reason: &str) -> String {
    match reason {
        "max_output_tokens" => "length".into(),
        "content_filter" => "content_filter".into(),
        other => other.into(),
    }
}

/// `_response_finish_reason(payload)`.
pub fn response_finish_reason(payload: &Value) -> Option<String> {
    if !matches!(payload, Value::Dict(_)) {
        return None;
    }
    let status = field(payload, "status");
    let reason = text(field(field(payload, "incomplete_details"), "reason"));
    if *status == s("incomplete") {
        return Some(reason.map_or_else(|| "incomplete".into(), |r| incomplete_reason(&r)));
    }
    if !response_tool_calls(payload).is_empty() {
        return Some("tool_calls".into());
    }
    if *status == s("completed") {
        return Some("stop".into());
    }
    if let Some(reason) = reason {
        return Some(incomplete_reason(&reason));
    }
    string(status)
}

/// `_response_stream_error(event, withheld)`.
pub fn response_stream_error(event: &Value, withheld: &Forms) -> AppError {
    let response = field(event, "response");
    let mut error = match field(event, "error") {
        value @ Value::Dict(_) => Some(value.clone()),
        _ => None,
    };
    if error.is_none() {
        if let value @ Value::Dict(_) = field(response, "error") {
            error = Some(value.clone());
        }
    }
    if error.is_none() && *field(event, "type") == s("error") {
        error = Some(crate::pyval::without(event, &["type", "sequence_number"]));
    }
    let error = error.filter(truthy).unwrap_or_else(|| dict::<&str>([]));
    provider_error(
        &error,
        "AI provider rejected the Responses request",
        withheld,
    )
}

const CHAT_OPTIONS: &[&str] = &[
    "temperature",
    "top_p",
    "stop",
    "seed",
    "frequency_penalty",
    "presence_penalty",
    "response_format",
];
/// Where non-standard servers stream reasoning text.
pub const CHAT_REASONING_FIELDS: &[&str] = &["reasoning_content", "reasoning"];

/// `_is_openrouter(base_url)`.
pub fn is_openrouter(base_url: &str) -> bool {
    let host = crate::providers::url_parts(base_url).host;
    host == "openrouter.ai" || host.ends_with(".openrouter.ai")
}

/// `_chat_text(content)`.
pub(crate) fn chat_text(content: &Value) -> String {
    match content {
        Value::Str(text) => lossy(text),
        Value::List(parts) => {
            let mut out = String::new();
            for part in parts {
                let kind = field(part, "type");
                if [s("output_text"), s("input_text"), s("text")].contains(kind) {
                    if let Some(piece) = string(field(part, "text")) {
                        out.push_str(&piece);
                        continue;
                    }
                }
                if *kind == s("refusal") {
                    if let Some(piece) = string(field(part, "refusal")) {
                        out.push_str(&piece);
                    }
                }
            }
            out
        }
        _ => String::new(),
    }
}

/// `_chat_user_content(content)`.
fn chat_user_content(content: &Value) -> Value {
    let Value::List(parts) = content else {
        return match content {
            Value::Str(_) => content.clone(),
            _ => s(""),
        };
    };
    let mut out = Vec::new();
    for part in parts {
        if !matches!(part, Value::Dict(_)) {
            continue;
        }
        let kind = field(part, "type");
        if [s("input_text"), s("text"), s("output_text")].contains(kind)
            && matches!(field(part, "text"), Value::Str(_))
        {
            out.push(dict([
                ("type", s("text")),
                ("text", field(part, "text").clone()),
            ]));
        } else if *kind == s("input_image") {
            if let Some(url) = text(field(part, "image_url")) {
                let mut image = dict([("url", s(url))]);
                if let Some(detail) = text(field(part, "detail")) {
                    set(&mut image, "detail", s(detail));
                }
                out.push(dict([("type", s("image_url")), ("image_url", image)]));
            }
        } else if *kind == s("image_url") {
            out.push(part.clone());
        } else if *kind == s("input_file") {
            let mut file = dict::<&str>([]);
            for key in ["file_id", "file_data", "filename"] {
                if let Some(value) = text(field(part, key)) {
                    set(&mut file, key, s(value));
                }
            }
            if truthy(&file) {
                out.push(dict([("type", s("file")), ("file", file)]));
            }
        }
    }
    Value::List(out)
}

/// `_reasoning_item_text(item)`.
fn reasoning_item_text(item: &Value) -> String {
    let mut out = String::new();
    for key in ["content", "summary"] {
        if let Value::List(parts) = field(item, key) {
            for part in parts {
                if let Some(piece) = string(field(part, "text")) {
                    out.push_str(&piece);
                }
            }
        }
    }
    out
}

/// `chat_completions_messages(context, replay_reasoning=...)`.
pub fn chat_completions_messages(context: &RequestContext, replay_reasoning: bool) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(instructions) = context.instructions.as_ref().filter(|i| !i.is_empty()) {
        messages.push(dict([("role", s("system")), ("content", s(instructions))]));
    }
    let mut pending: Option<Value> = None;
    let flush = |pending: &mut Option<Value>, messages: &mut Vec<Value>| {
        if let Some(mut message) = pending.take() {
            if truthy(field(&message, "tool_calls")) {
                if !truthy(field(&message, "content")) {
                    set(&mut message, "content", Value::Null);
                }
                messages.push(message);
            } else if truthy(field(&message, "content")) {
                messages.push(message);
            }
        }
    };
    let busy = |pending: &Option<Value>| {
        pending.as_ref().is_some_and(|message| {
            truthy(field(message, "content")) || truthy(field(message, "tool_calls"))
        })
    };
    fn assistant(pending: &mut Option<Value>) -> &mut Value {
        pending.get_or_insert_with(|| dict([("role", s("assistant")), ("content", Value::Null)]))
    }
    for item in &context.input_items {
        if !matches!(item, Value::Dict(_)) {
            continue;
        }
        let kind = if truthy(field(item, "type")) {
            field(item, "type").clone()
        } else if has(item, "role") {
            s("message")
        } else {
            Value::Null
        };
        if kind == s("message") {
            let role = field(item, "role");
            if *role == s("assistant") {
                let content = chat_text(field(item, "content"));
                if busy(&pending) {
                    flush(&mut pending, &mut messages);
                }
                set(assistant(&mut pending), "content", s(content));
                continue;
            }
            flush(&mut pending, &mut messages);
            if *role == s("system") || *role == s("developer") {
                let content = chat_text(field(item, "content"));
                if !content.is_empty() {
                    messages.push(dict([("role", s("system")), ("content", s(content))]));
                }
            } else if *role == s("user") {
                messages.push(dict([
                    ("role", s("user")),
                    ("content", chat_user_content(field(item, "content"))),
                ]));
            }
            continue;
        }
        if kind == s("function_call") {
            let call_id = if truthy(field(item, "call_id")) {
                field(item, "call_id")
            } else {
                field(item, "id")
            };
            let (Some(call_id), Some(name)) = (text(call_id), text(field(item, "name"))) else {
                continue;
            };
            let arguments = text(field(item, "arguments")).unwrap_or_else(|| "{}".into());
            let call = dict([
                ("id", s(call_id)),
                ("type", s("function")),
                (
                    "function",
                    dict([("name", s(name)), ("arguments", s(arguments))]),
                ),
            ]);
            let message = assistant(&mut pending);
            let mut calls = match field(message, "tool_calls") {
                Value::List(calls) => calls.clone(),
                _ => Vec::new(),
            };
            calls.push(call);
            set(message, "tool_calls", Value::List(calls));
            continue;
        }
        if kind == s("reasoning") {
            let replay = field(item, "chat_completions_replay");
            if !replay_reasoning || !matches!(replay, Value::Dict(_)) {
                continue;
            }
            if busy(&pending) {
                flush(&mut pending, &mut messages);
            }
            let target = assistant(&mut pending);
            let details = field(replay, "reasoning_details");
            let field_name = field(replay, "field");
            let reasoning = reasoning_item_text(item);
            if matches!(details, Value::List(d) if !d.is_empty()) {
                set(target, "reasoning_details", details.clone());
            } else if let Some(name) = CHAT_REASONING_FIELDS
                .iter()
                .find(|name| *field_name == s(name))
                .filter(|_| !reasoning.is_empty())
            {
                set(target, name, s(reasoning));
            }
            continue;
        }
        if kind == s("function_call_output") {
            flush(&mut pending, &mut messages);
            let Some(call_id) = text(field(item, "call_id")) else {
                continue;
            };
            let output = field(item, "output");
            let content = match output {
                Value::Str(_) => output.clone(),
                other => s(kassiber_pyjson::dumps(other, &Default::default()).unwrap_or_default()),
            };
            messages.push(dict([
                ("role", s("tool")),
                ("tool_call_id", s(call_id)),
                ("content", content),
            ]));
        }
    }
    flush(&mut pending, &mut messages);
    messages
}

/// `_chat_completions_tools(tools)`.
pub fn chat_completions_tools(tools: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for tool in tools {
        if !matches!(tool, Value::Dict(_)) || *field(tool, "type") != s("function") {
            continue;
        }
        let function = match field(tool, "function") {
            function @ Value::Dict(_) => function,
            _ => tool,
        };
        let Some(name) = text(field(function, "name")) else {
            continue;
        };
        let mut definition = dict([("name", s(name))]);
        if let description @ Value::Str(_) = field(function, "description") {
            set(&mut definition, "description", description.clone());
        }
        if let parameters @ Value::Dict(_) = field(function, "parameters") {
            set(&mut definition, "parameters", parameters.clone());
        }
        out.push(dict([("type", s("function")), ("function", definition)]));
    }
    out
}

/// `_chat_completions_tool_choice(tool_choice)`.
pub fn chat_completions_tool_choice(choice: &Value) -> Value {
    if text(choice).is_some() {
        return choice.clone();
    }
    if matches!(choice, Value::Dict(_)) {
        let function = field(choice, "function");
        let name = if matches!(function, Value::Dict(_)) {
            field(function, "name")
        } else {
            field(choice, "name")
        };
        if let Some(name) = text(name) {
            return dict([
                ("type", s("function")),
                ("function", dict([("name", s(name))])),
            ]);
        }
    }
    s("auto")
}

/// The Chat Completions request fields the body carries.
pub struct ChatBodyOptions<'a> {
    pub options: &'a Value,
    pub tools: Option<&'a [Value]>,
    pub tool_choice: &'a Value,
    pub omit_store: bool,
    pub replay_reasoning: bool,
    pub max_completion_tokens: bool,
    pub effort: bool,
    pub base_url: &'a str,
    pub efforts: EffortLevels,
}

/// `_chat_completions_body(...)`: only what the turn needs.
pub fn chat_completions_body(
    context: &RequestContext,
    model: &str,
    stream: bool,
    parts: ChatBodyOptions<'_>,
) -> Value {
    let options = parts.options;
    let mut body = dict([
        ("model", s(model)),
        (
            "messages",
            Value::List(chat_completions_messages(context, parts.replay_reasoning)),
        ),
        ("stream", Value::Bool(stream)),
    ]);
    if !parts.omit_store {
        set(&mut body, "store", Value::Bool(false));
    }
    for key in CHAT_OPTIONS {
        let value = field(options, key);
        if *value != Value::Null {
            set(&mut body, key, value.clone());
        }
    }
    let mut max_tokens = field(options, "max_tokens");
    if *max_tokens == Value::Null {
        max_tokens = field(options, "max_output_tokens");
    }
    if *max_tokens != Value::Null {
        let key = if parts.max_completion_tokens {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        set(&mut body, key, max_tokens.clone());
    }
    let definitions = chat_completions_tools(parts.tools.unwrap_or(&[]));
    if !definitions.is_empty() {
        set(&mut body, "tools", Value::List(definitions));
        set(
            &mut body,
            "tool_choice",
            chat_completions_tool_choice(parts.tool_choice),
        );
    }
    if let Some(effort) = (parts.efforts)(field(options, "reasoning_effort")) {
        if parts.effort {
            if is_openrouter(parts.base_url) {
                set(&mut body, "reasoning", dict([("effort", s(effort))]));
            } else {
                set(&mut body, "reasoning_effort", s(effort));
            }
        }
    }
    body
}

/// `_chat_finish_reason(reason, has_calls)`.
pub fn chat_finish_reason(reason: &Value, has_calls: bool) -> String {
    let finish = text(reason).map(|reason| {
        match reason.as_str() {
            "function_call" => "tool_calls",
            "end_turn" | "eos_token" | "stop_sequence" => "stop",
            "max_tokens" => "length",
            other => return other.to_owned(),
        }
        .to_owned()
    });
    if has_calls && matches!(finish.as_deref(), None | Some("stop")) {
        return "tool_calls".into();
    }
    finish.unwrap_or_else(|| "stop".into())
}

/// `_chat_completion_output(...)`: the answer as output Items.
pub fn chat_completion_output(
    content: &str,
    reasoning: &str,
    reasoning_field: Option<&str>,
    reasoning_details: Vec<Value>,
    tool_calls: &[Value],
) -> Vec<Value> {
    let mut output = Vec::new();
    if !reasoning.is_empty() || !reasoning_details.is_empty() {
        let mut replay = dict([("field", s(reasoning_field.unwrap_or("reasoning_content")))]);
        if !reasoning_details.is_empty() {
            set(
                &mut replay,
                "reasoning_details",
                Value::List(reasoning_details),
            );
        }
        output.push(dict([
            ("type", s("reasoning")),
            ("summary", Value::List(Vec::new())),
            (
                "content",
                Value::List(vec![dict([
                    ("type", s("reasoning_text")),
                    ("text", s(reasoning)),
                ])]),
            ),
            ("chat_completions_replay", replay),
        ]));
    }
    if !content.is_empty() {
        output.push(dict([
            ("type", s("message")),
            ("role", s("assistant")),
            (
                "content",
                Value::List(vec![dict([
                    ("type", s("output_text")),
                    ("text", s(content)),
                ])]),
            ),
        ]));
    }
    for call in tool_calls {
        let function = field(call, "function");
        output.push(dict([
            ("type", s("function_call")),
            ("call_id", field(call, "id").clone()),
            ("name", field(function, "name").clone()),
            ("arguments", field(function, "arguments").clone()),
        ]));
    }
    output
}

/// `_chat_completion_stream_error(event, withheld)`: an error object inside a chunk.
pub fn chat_completion_stream_error(event: &Value, withheld: &Forms) -> Option<AppError> {
    let mut error = field(event, "error").clone();
    if !truthy(&error) && *field(event, "object") == s("error") {
        error = crate::pyval::without(event, &["object"]);
    }
    if !truthy(&error) {
        return None;
    }
    if !matches!(error, Value::Dict(_)) {
        error = dict([("message", s(kassiber_envelope::py_str(&error)))]);
    }
    Some(provider_error(
        &error,
        "AI provider rejected the request",
        withheld,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(raw: &str) -> Value {
        kassiber_pytext::loads(raw).unwrap()
    }

    // `ResponsesBodyContractTest` / `ResponsesNormalizationTest` units.
    #[test]
    fn options_map_onto_responses_fields_and_reserved_ones_are_dropped() {
        let body = responses_options(&load(
            r#"{"max_tokens":5,"response_format":{"type":"json_object"},"reasoning_effort":"ultra","store":true,"stream":true,"temperature":0,"fast_mode":true,"provider_session_id":"x","tools":[]}"#,
        ));
        assert_eq!(
            body,
            load(
                r#"{"temperature":0,"max_output_tokens":5,"text":{"format":{"type":"json_object"}},"reasoning":{"effort":"high"}}"#
            )
        );
        assert_eq!(
            responses_options(&load(
                r#"{"max_tokens":5,"max_output_tokens":9,"reasoning_effort":"auto"}"#
            )),
            load(r#"{"max_output_tokens":9}"#)
        );
    }

    #[test]
    fn messages_become_instructions_and_items() {
        let context = RequestContext::from_messages(&[
            load(r#"{"role":"system","content":"A"}"#),
            load(r#"{"role":"system","content":"B"}"#),
            load(
                r#"{"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"data:x","detail":"low"}},7]}"#,
            ),
            load(
                r#"{"role":"assistant","content":"","tool_calls":[{"id":"c1","function":{"name":"status","arguments":"{}"}},{"id":"","function":{"name":"x","arguments":"{}"}}]}"#,
            ),
            load(r#"{"role":"tool","tool_call_id":"c1","content":"{\"ok\":true}"}"#),
            load(r#"{"role":"other","content":"skip"}"#),
        ]);
        assert_eq!(context.instructions.as_deref(), Some("A\n\nB"));
        assert_eq!(
            Value::List(context.input_items),
            load(
                r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"},{"type":"input_image","image_url":"data:x","detail":"low"}]},{"type":"function_call","call_id":"c1","name":"status","arguments":"{}"},{"type":"function_call_output","call_id":"c1","output":"{\"ok\":true}"}]"#
            )
        );
    }

    #[test]
    fn incomplete_responses_withhold_unfinished_calls() {
        let payload = load(
            r#"{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[{"type":"message","content":[{"type":"output_text","text":"Par"},{"type":"refusal","refusal":"tial"}]},{"type":"function_call","status":"in_progress","call_id":"c","name":"s","arguments":"{"},{"type":"function_call","status":"completed","name":"t"}]}"#,
        );
        assert_eq!(response_finish_reason(&payload).as_deref(), Some("length"));
        assert_eq!(response_text(&payload), "Partial");
        assert_eq!(
            Value::List(response_tool_calls(&payload)),
            load(r#"[{"id":"call_1","type":"function","function":{"name":"t","arguments":""}}]"#)
        );
        assert_eq!(
            response_finish_reason(&load(r#"{"status":"in_progress"}"#)).as_deref(),
            Some("in_progress")
        );
        assert_eq!(
            response_finish_reason(&load(r#"{"incomplete_details":{"reason":"other"}}"#))
                .as_deref(),
            Some("other")
        );
        assert_eq!(response_finish_reason(&load("{}")), None);
    }

    #[test]
    fn chat_finish_reasons() {
        for (reason, calls, finish) in [
            (s("function_call"), false, "tool_calls"),
            (s("end_turn"), false, "stop"),
            (s("max_tokens"), true, "length"),
            (s("stop"), true, "tool_calls"),
            (Value::Null, true, "tool_calls"),
            (Value::Null, false, "stop"),
            (s(""), false, "stop"),
            (s("content_filter"), true, "content_filter"),
        ] {
            assert_eq!(chat_finish_reason(&reason, calls), finish);
        }
    }

    #[test]
    fn stream_errors_and_openrouter_hosts() {
        assert!(
            chat_completion_stream_error(&load(r#"{"error":null}"#), &Forms::default()).is_none()
        );
        assert!(
            chat_completion_stream_error(&load(r#"{"error":{}}"#), &Forms::default()).is_none()
        );
        assert_eq!(
            chat_completion_stream_error(&load(r#"{"error":"boom"}"#), &Forms::default())
                .unwrap()
                .code,
            "ai_unavailable"
        );
        assert_eq!(
            chat_completion_stream_error(
                &load(r#"{"object":"error","code":400,"message":"bad"}"#),
                &Forms::default()
            )
            .unwrap()
            .code,
            "ai_request_invalid"
        );
        let error = response_stream_error(
            &load(
                r#"{"type":"error","sequence_number":3,"code":"rate_limit_exceeded","message":"slow"}"#,
            ),
            &Forms::default(),
        );
        assert_eq!(error.code, "ai_rate_limited");
        assert_eq!(
            response_stream_error(
                &load(r#"{"type":"response.failed","response":{"error":{"code":"server_error"}}}"#),
                &Forms::default()
            )
            .code,
            "ai_unavailable"
        );
        assert_eq!(
            response_stream_error(&load(r#"{"type":"response.failed"}"#), &Forms::default())
                .message,
            "AI provider failed while generating a response"
        );
        assert!(is_openrouter("https://openrouter.ai/api/v1"));
        assert!(is_openrouter("https://EU.OpenRouter.ai/api/v1"));
        assert!(!is_openrouter("https://notopenrouter.ai/v1"));
    }
}

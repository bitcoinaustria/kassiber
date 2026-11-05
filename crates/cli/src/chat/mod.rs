//! `kassiber chat` (`kassiber/cli/chat.py`): the desktop Assistant's daemon
//! tool loop, consent and cancel protocol from a terminal. One-shot answers
//! go to stdout (`--machine` wraps them in a `chat` envelope,
//! `--stream-json` prints the daemon's records); progress, tool
//! announcements, consent prompts and provenance go to the terminal, or to
//! stderr when stdout is piped. Without a prompt it is a REPL.
//!
//! Registered only behind the `native-chat` gate with the daemon's chat
//! kinds (design §7 item 1).

pub mod client;
pub mod render;
pub mod review;

use std::io::{BufRead, Write};
use std::time::Duration;

use kassiber_ai::catalog::{tools, KindClass};
use kassiber_ai::select::CORE_TOOL_NAMES;
use kassiber_envelope::{dict, get, py_str, set, truthy, AppError};
use kassiber_pyjson::{dumps, Key, Options, Value};

use crate::argparse::Namespace;
use client::{compact, random_hex, s, DaemonClient, Launch};
use render::{render_envelope_table, MarkdownStream};

pub const COMMANDS: &[&str] = &["chat"];

const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const CLI_DEFAULT_MODEL: &str = "default";
const DEFAULT_TIMEOUT_SECONDS: f64 = 120.0;
const MAX_TIMEOUT_SECONDS: f64 = 3600.0;

/// `_FINISH_NOTICES`.
pub const FINISH_NOTICES: [(&str, &str); 4] = [
    ("cancelled", "Cancelled."),
    (
        "tool_loop_max_iterations",
        "Stopped at the tool-loop iteration limit. Continue with the remaining cases; check the review receipt before retrying an uncertain apply. Use --tool-loop-max-iterations for a larger bounded turn.",
    ),
    (
        "length",
        "Stopped at the provider token limit; raise --max-tokens for longer answers.",
    ),
    ("content_filter", "Stopped by the provider's content filter."),
];

/// `_REPL_HELP`.
pub const REPL_HELP: &str = "Commands:\n  /help             show this help\n  /tools            list daemon AI tools and their consent class\n  /model [id]       show or switch the model for following turns\n  /provider [name]  show or switch the provider (model re-resolves)\n  /allow <tool>     allow a mutating tool for this session (except exact-review tools)\n  /allowed          show which mutating tools are pre-allowed\n  /new              start a fresh conversation (history cleared)\n  /exit             leave the chat (also /quit or Ctrl-D)\nPress Ctrl-C while the assistant is replying to cancel that turn.\n";

/// `COPILOT_INCOGNITO_NOTICE`.
pub const COPILOT_INCOGNITO_NOTICE: &str =
    "Incognito: GitHub Copilot still keeps each turn's session until that turn ends, in a temporary folder.";

const CONSENT_DECISIONS: [&str; 3] = ["allow_once", "allow_session", "deny"];

/// `dict` without one key, keeping order.
pub(crate) fn without(value: &Value, key: &str) -> Value {
    match value {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .iter()
                .filter(|(k, _)| *k != Key::from(key))
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Where a line goes: the answer stream or stderr.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Out,
    Err,
}

/// The process's streams for one chat.
pub struct Io<'a> {
    pub stdin: &'a mut dyn BufRead,
    pub stdin_is_tty: bool,
    pub stdout: &'a mut dyn Write,
    pub stdout_is_tty: bool,
    pub stderr: &'a mut dyn Write,
    pub stderr_is_tty: bool,
    /// `COLUMNS`, for tool-result tables.
    pub columns: Option<String>,
}

impl Io<'_> {
    fn write(&mut self, target: Target, text: &str) {
        let stream: &mut dyn Write = match target {
            Target::Out => &mut *self.stdout,
            Target::Err => &mut *self.stderr,
        };
        let _ = stream.write_all(text.as_bytes());
        let _ = stream.flush();
    }

    fn is_tty(&self, target: Target) -> bool {
        match target {
            Target::Out => self.stdout_is_tty,
            Target::Err => self.stderr_is_tty,
        }
    }

    /// `_write_dim(text, out)`.
    fn write_dim(&mut self, target: Target, text: &str) {
        if self.is_tty(target) {
            self.write(target, &format!("{DIM}{text}{RESET}"));
        } else {
            self.write(target, text);
        }
    }

    fn read_line(&mut self) -> String {
        let mut line = String::new();
        let _ = self.stdin.read_line(&mut line);
        line
    }
}

/// The chat's arguments, mutable across REPL commands as Python's
/// namespace is.
pub struct Args {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub reasoning_effort_from_provider: bool,
    pub system: Option<String>,
    pub temperature: Value,
    pub max_tokens: Value,
    pub fast: bool,
    pub timeout_seconds: Value,
    pub tool_loop_max_iterations: Value,
    pub tool_profile: String,
    pub no_tools: bool,
    pub yes: bool,
    pub allow_tool: Vec<String>,
    pub stream_json: bool,
    pub machine: bool,
    pub non_interactive: bool,
    pub plain: bool,
    pub incognito: bool,
    pub continue_session: bool,
    pub session: Option<String>,
    pub file: Option<String>,
    pub file_context: Option<String>,
    pub transcript: Option<String>,
    pub attachment: Option<Value>,
}

impl Args {
    pub fn from_namespace(namespace: &Namespace) -> Self {
        let text = |key: &str| namespace.text(key).map(str::to_owned);
        let value = |key: &str| namespace.get(key).cloned().unwrap_or(Value::Null);
        Args {
            provider: text("provider"),
            model: text("model"),
            reasoning_effort: text("reasoning_effort"),
            reasoning_effort_from_provider: false,
            system: text("system"),
            temperature: value("temperature"),
            max_tokens: value("max_tokens"),
            fast: namespace.flag("fast"),
            timeout_seconds: value("timeout_seconds"),
            tool_loop_max_iterations: value("tool_loop_max_iterations"),
            tool_profile: text("tool_profile").unwrap_or_else(|| "core".into()),
            no_tools: namespace.flag("no_tools"),
            yes: namespace.flag("yes"),
            allow_tool: match namespace.get("allow_tool") {
                Some(Value::List(items)) => items.iter().map(py_str).collect(),
                _ => Vec::new(),
            },
            stream_json: namespace.flag("stream_json"),
            machine: namespace.text("format") == Some("json"),
            non_interactive: namespace.flag("non_interactive"),
            plain: namespace.flag("plain"),
            incognito: namespace.flag("incognito"),
            continue_session: namespace.flag("continue_session"),
            session: text("session"),
            file: text("file"),
            file_context: text("file_context"),
            transcript: text("transcript"),
            attachment: None,
        }
    }
}

/// `_timeout_seconds(args)`.
pub fn timeout_seconds(raw: &Value) -> Result<f64, AppError> {
    let timeout = match raw {
        Value::Null => return Ok(DEFAULT_TIMEOUT_SECONDS),
        Value::Float(value) => *value,
        Value::Int(value) => value.to_string().parse::<f64>().unwrap_or(f64::INFINITY),
        Value::Bool(flag) => f64::from(u8::from(*flag)),
        Value::Str(text) => kassiber_pytext::parse_float(&py_str(&Value::Str(text.clone())))
            .map_err(|_| {
                AppError::new("validation", "chat timeout must be a number of seconds")
                    .with_details(dict([("timeout", raw.clone())]))
            })?,
        _ => {
            return Err(
                AppError::new("validation", "chat timeout must be a number of seconds")
                    .with_details(dict([("timeout", raw.clone())])),
            )
        }
    };
    if !timeout.is_finite() || timeout <= 0.0 || timeout > MAX_TIMEOUT_SECONDS {
        return Err(AppError::new(
            "validation",
            "chat timeout must be greater than 0 and at most 3600 seconds",
        )
        .with_details(dict([("timeout", raw.clone())])));
    }
    Ok(timeout)
}

/// `_split_tool_names(values)`: comma lists, plus the display name of any
/// wire name given.
fn split_tool_names(values: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for value in values {
        for item in value.split(',') {
            let item = kassiber_pytext::strip(item);
            if !item.is_empty() && !names.iter().any(|name| name == item) {
                names.push(item.to_owned());
            }
        }
    }
    for entry in tools() {
        let named = entry
            .wire_name
            .as_deref()
            .is_some_and(|wire| !wire.is_empty() && names.iter().any(|name| name == wire));
        if named && !names.contains(&entry.name) {
            names.push(entry.name.clone());
        }
    }
    names
}

fn mutating_tool_names() -> Vec<String> {
    tools()
        .iter()
        .filter(|entry| entry.kind_class == KindClass::Mutating)
        .map(|entry| entry.name.clone())
        .collect()
}

/// `_chat_options(args)`.
fn chat_options(args: &Args) -> Option<Value> {
    let mut options: Vec<(&str, Value)> = Vec::new();
    if args.temperature != Value::Null {
        options.push(("temperature", args.temperature.clone()));
    }
    if args.max_tokens != Value::Null {
        options.push(("max_tokens", args.max_tokens.clone()));
    }
    if let Some(effort) = args
        .reasoning_effort
        .as_deref()
        .filter(|e| !e.is_empty() && *e != "auto")
    {
        options.push(("reasoning_effort", s(effort)));
    }
    if args.fast {
        options.push(("fast_mode", Value::Bool(true)));
    }
    (!options.is_empty()).then(|| dict(options))
}

/// `_build_chat_args(args, messages, session_id=...)`.
fn chat_args(args: &Args, messages: &[Value], session_id: Option<&str>) -> Result<Value, AppError> {
    let tools_enabled = !args.no_tools;
    let mut payload = vec![
        ("provider", args.provider.as_deref().map_or(Value::Null, s)),
        ("model", args.model.as_deref().map_or(Value::Null, s)),
        ("messages", Value::List(messages.to_vec())),
        ("tools_enabled", Value::Bool(tools_enabled)),
        ("tool_profile", s(&args.tool_profile)),
        (
            "timeout_seconds",
            Value::Float(timeout_seconds(&args.timeout_seconds)?),
        ),
        (
            "persist",
            if args.incognito {
                Value::Bool(false)
            } else {
                s("auto")
            },
        ),
        ("session_id", session_id.map_or(Value::Null, s)),
    ];
    if args.tool_loop_max_iterations != Value::Null {
        payload.push((
            "tool_loop_max_iterations",
            args.tool_loop_max_iterations.clone(),
        ));
    }
    if let Some(attachment) = &args.attachment {
        payload.push(("attachment", attachment.clone()));
    }
    if let Some(system) = args.system.as_deref().filter(|system| !system.is_empty()) {
        payload.push(("system_prompt_kind", s("raw")));
        payload.push(("system_prompt", s(system)));
    } else if tools_enabled {
        // Tool-free chats keep the daemon default (no system prompt): the
        // Kassiber prompt instructs tool use the provider would not have.
        payload.push(("system_prompt_kind", s("kassiber")));
    }
    if let Some(options) = chat_options(args) {
        payload.push(("options", options));
    }
    Ok(dict(payload))
}

fn provider_rows(payload: &Value) -> Vec<Value> {
    let rows = match get(payload, "data") {
        Some(data @ Value::Dict(_)) => match get(data, "providers") {
            Some(Value::List(rows)) => rows.clone(),
            _ => Vec::new(),
        },
        Some(Value::List(rows)) => rows.clone(),
        _ => Vec::new(),
    };
    rows.into_iter()
        .filter(|row| matches!(row, Value::Dict(_)))
        .collect()
}

/// `_error_from_envelope(record, message=..., code=...)`.
fn error_from_envelope(record: &Value, message: &str, code: &str) -> AppError {
    let error = match get(record, "error") {
        Some(error @ Value::Dict(_)) => error.clone(),
        _ => Value::Dict(Vec::new()),
    };
    let text = |key: &str, default: &str| match get(&error, key) {
        Some(value) => py_str(value),
        None => default.to_owned(),
    };
    let mut failure = AppError::new(text("code", code), text("message", message))
        .retryable(get(&error, "retryable").is_some_and(truthy));
    if let Some(details) = get(&error, "details").filter(|d| **d != Value::Null) {
        failure = failure.with_details(details.clone());
    }
    if let Some(hint) = get(&error, "hint").filter(|h| **h != Value::Null) {
        failure = failure.with_hint(py_str(hint));
    }
    failure
}

/// `_auth_required_error(record)`.
fn auth_required_error(record: &Value) -> AppError {
    let data = get(record, "data").cloned().unwrap_or(Value::Null);
    let label = match get(&data, "label") {
        Some(Value::Str(label)) if label.code_points().next().is_some() => {
            py_str(&Value::Str(label.clone()))
        }
        _ => "database unlock required".into(),
    };
    let mut error = AppError::new("passphrase_required", label).with_hint(
        "Run the command with `--db-passphrase-fd <FD>` from a parent process, or use an interactive terminal to unlock Kassiber first.",
    );
    if let Some(scope) = get(&data, "scope").filter(|scope| truthy(scope)) {
        error = error.with_details(dict([("scope", scope.clone())]));
    }
    error
}

/// `_read_control_response(client, request_id)`.
fn read_control_response(client: &mut DaemonClient, request_id: &str) -> Result<Value, AppError> {
    loop {
        let record = client.read(true, None)?;
        if get(&record, "request_id") != Some(&s(request_id)) {
            continue;
        }
        if get(&record, "kind") == Some(&s("auth_required")) {
            return Err(auth_required_error(&record));
        }
        if get(&record, "kind") == Some(&s("error")) {
            return Err(error_from_envelope(
                &record,
                "daemon request failed",
                "daemon_request_failed",
            ));
        }
        return Ok(record);
    }
}

fn request(
    client: &mut DaemonClient,
    prefix: &str,
    kind: &str,
    args: Value,
) -> Result<Value, AppError> {
    let request_id = format!("{prefix}-{}", random_hex());
    client.send(
        &dict([
            ("request_id", s(&request_id)),
            ("kind", s(kind)),
            ("args", args),
        ]),
        true,
    )?;
    read_control_response(client, &request_id)
}

/// `_selected_provider_row(rows, provider_name)`.
fn selected_provider_row(rows: &[Value], provider: Option<&str>) -> Option<Value> {
    let wanted = kassiber_pytext::lower(kassiber_pytext::strip(provider.unwrap_or("")));
    if wanted.is_empty() {
        return rows
            .iter()
            .find(|row| get(row, "is_default").is_some_and(truthy))
            .cloned();
    }
    rows.iter()
        .find(|row| {
            let name = match get(row, "name") {
                Some(name) if truthy(name) => py_str(name),
                _ => String::new(),
            };
            kassiber_pytext::lower(kassiber_pytext::strip(&name)) == wanted
        })
        .cloned()
}

fn cli_runtime(row: &Value) -> Option<&'static str> {
    kassiber_ai::providers::cli_provider_for_locator(get(row, "base_url").unwrap_or(&Value::Null))
}

/// `_incognito_provider_notice(client, args)`.
fn incognito_notice(
    client: &mut DaemonClient,
    args: &Args,
) -> Result<Option<&'static str>, AppError> {
    if !args.incognito {
        return Ok(None);
    }
    let rows = provider_rows(&request(
        client,
        "chat-provider-list",
        "ai.providers.list",
        dict::<&str>([]),
    )?);
    let selected = selected_provider_row(&rows, args.provider.as_deref());
    Ok(selected
        .filter(|row| cli_runtime(row) == Some("copilot"))
        .map(|_| COPILOT_INCOGNITO_NOTICE))
}

/// `_resolve_default_model(client, args)`: the model and effort the user
/// left to the provider; an explicit `--reasoning-effort` always wins.
fn resolve_default_model(client: &mut DaemonClient, args: &mut Args) -> Result<(), AppError> {
    let needs_model = args.model.as_deref().is_none_or(str::is_empty);
    let needs_effort = args.reasoning_effort.is_none();
    if !needs_model && !needs_effort {
        return Ok(());
    }
    let rows = provider_rows(&request(
        client,
        "chat-provider-list",
        "ai.providers.list",
        dict::<&str>([]),
    )?);
    let selected = selected_provider_row(&rows, args.provider.as_deref());
    if needs_effort {
        if let Some(Value::Str(effort)) = selected
            .as_ref()
            .and_then(|row| get(row, "default_reasoning_effort"))
        {
            let effort = py_str(&Value::Str(effort.clone()));
            let effort = kassiber_pytext::strip(&effort);
            if !effort.is_empty() {
                args.reasoning_effort = Some(effort.to_owned());
                args.reasoning_effort_from_provider = true;
            }
        }
    }
    if !needs_model {
        return Ok(());
    }
    let Some(selected) = selected else {
        return Err(AppError::new("validation", "AI chat requires a provider")
            .with_hint("Set a default provider or pass --provider."));
    };
    let mut model = match get(&selected, "default_model") {
        Some(Value::Str(model))
            if !kassiber_pytext::strip(&py_str(&Value::Str(model.clone()))).is_empty() =>
        {
            Some(py_str(&Value::Str(model.clone())))
        }
        _ => None,
    };
    if model.is_none() && cli_runtime(&selected).is_some() {
        model = Some(CLI_DEFAULT_MODEL.into());
    }
    let Some(model) = model else {
        // Python names an undefined variable here when a row has no name
        // (design §7 item 11); every row has one, so the row's name is used.
        let name = match get(&selected, "name") {
            Some(name) if truthy(name) => py_str(name),
            _ => "selected provider".into(),
        };
        return Err(
            AppError::new("validation", "AI chat requires a model").with_hint(format!(
                "Pass --model, or set --default-model on provider '{name}'."
            )),
        );
    };
    args.model = Some(model);
    Ok(())
}

/// `_stage_chat_attachment(client, args)`.
fn stage_attachment(client: &mut DaemonClient, args: &Args) -> Result<Option<Value>, AppError> {
    let Some(file) = args.file.as_deref() else {
        return Ok(None);
    };
    let path = expand_user(file);
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    if !path.is_file() {
        return Err(AppError::new(
            "not_found",
            format!("Attachment not found: {}", path.display()),
        )
        .with_hint("Pass an existing export file with --file."));
    }
    let response = request(
        client,
        "chat-attach",
        "internal.document_import.stage",
        dict([("source_file", s(path.to_string_lossy()))]),
    )?;
    let token = match get(&response, "data").and_then(|data| get(data, "token")) {
        Some(Value::Str(token)) if token.code_points().next().is_some() => token.clone(),
        _ => {
            return Err(AppError::new(
                "ai_attachment_failed",
                "could not attach the file to this chat",
            ))
        }
    };
    let label = args
        .file_context
        .as_deref()
        .map(kassiber_pytext::strip)
        .filter(|label| !label.is_empty());
    Ok(Some(dict([
        ("token", Value::Str(token)),
        ("label", label.map_or(Value::Null, s)),
    ])))
}

/// `Path(file).expanduser()` for the current user.
fn expand_user(file: &str) -> std::path::PathBuf {
    let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    match (file.strip_prefix('~'), std::env::var_os(home)) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            let mut path = std::path::PathBuf::from(home);
            path.push(rest.trim_start_matches(['/', '\\']));
            path
        }
        _ => std::path::PathBuf::from(file),
    }
}

/// `_resolve_continuation(client, args)`: `(session id, prior messages,
/// stored session)`.
fn resolve_continuation(
    client: &mut DaemonClient,
    args: &Args,
) -> Result<(Option<String>, Vec<Value>, Option<Value>), AppError> {
    let mut requested = args.session.clone().filter(|session| !session.is_empty());
    if requested.is_none() && !args.continue_session {
        return Ok((None, Vec::new(), None));
    }
    if requested.is_none() {
        let response = request(
            client,
            "chat-sessions-list",
            "ui.chat.sessions.list",
            dict([("limit", Value::Int(1.into()))]),
        )?;
        let first = get(&response, "data")
            .and_then(|data| get(data, "sessions"))
            .and_then(|sessions| match sessions {
                Value::List(sessions) => sessions.first().cloned(),
                _ => None,
            });
        let Some(first) = first else {
            return Err(
                AppError::new("not_found", "no persisted chat sessions to continue")
                    .with_hint("Run a chat with history enabled first, or pass --session <id>."),
            );
        };
        requested = get(&first, "id").map(py_str);
    }
    let requested = requested.unwrap_or_default();
    let response = request(
        client,
        "chat-sessions-get",
        "ui.chat.sessions.get",
        dict([("session_id", s(&requested))]),
    )?;
    let session = match get(&response, "data") {
        Some(data @ Value::Dict(_)) => data.clone(),
        _ => Value::Dict(Vec::new()),
    };
    let messages = match get(&session, "messages") {
        Some(Value::List(messages)) => messages
            .iter()
            .filter(|message| {
                matches!(get(message, "role"), Some(Value::Str(role)) if role.is("user") || role.is("assistant"))
                    && get(message, "content").is_some_and(truthy)
            })
            .map(|message| {
                dict([
                    ("role", get(message, "role").cloned().unwrap_or(Value::Null)),
                    ("content", get(message, "content").cloned().unwrap_or(Value::Null)),
                ])
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok((Some(requested), messages, Some(session)))
}

/// `_interactive_consent(...)`.
fn interactive_consent(
    io: &mut Io<'_>,
    chrome: Target,
    name: &str,
    summary: &str,
    preview: &Value,
) -> String {
    io.write(
        chrome,
        &format!(
            "\nConsent required: {}\n",
            if summary.is_empty() { name } else { summary }
        ),
    );
    io.write(chrome, &format!("Tool: {name}\n"));
    if truthy(preview) {
        let shown = dumps(preview, &Options::new().indent(2).sort_keys(true)).unwrap_or_default();
        io.write(chrome, &format!("Arguments:\n{shown}\n"));
    }
    loop {
        io.write(
            chrome,
            "Allow? [y] once, [s] session, [n] deny, [c] cancel: ",
        );
        let choice = io.read_line();
        if choice.is_empty() {
            return "deny".into();
        }
        match kassiber_pytext::lower(kassiber_pytext::strip(&choice)).as_str() {
            "y" | "yes" => return "allow_once".into(),
            "s" | "session" => return "allow_session".into(),
            "n" | "no" | "deny" => return "deny".into(),
            "c" | "cancel" => return "cancel".into(),
            _ => {}
        }
    }
}

/// `_policy_decision(args, tool_name, stdin)`.
fn policy_decision(args: &Args, tool: &str, stdin_is_tty: bool) -> Option<String> {
    if args.yes
        || split_tool_names(&args.allow_tool)
            .iter()
            .any(|name| name == tool)
    {
        return Some("allow_session".into());
    }
    // Machine and NDJSON outputs are scripted: never prompt into them.
    if args.machine || args.stream_json || args.non_interactive || !stdin_is_tty {
        return Some("deny".into());
    }
    None
}

/// One turn's mutable view.
struct TurnState<'a> {
    request_id: String,
    content: String,
    /// `call_id` → the tool's entry, in first-seen order.
    tool_calls: Vec<(String, Value)>,
    control: Vec<String>,
    consented: Vec<String>,
    session_allowed: Option<&'a mut Vec<String>>,
}

impl TurnState<'_> {
    fn call(&mut self, call_id: &str) -> Option<&mut Value> {
        self.tool_calls
            .iter_mut()
            .find(|(id, _)| id == call_id)
            .map(|(_, entry)| entry)
    }
}

/// `_decide_and_send_consent(...)`.
fn decide_and_send_consent(
    client: &mut DaemonClient,
    args: &Args,
    io: &mut Io<'_>,
    chrome: Target,
    turn: &mut TurnState<'_>,
    call_id: &str,
    name: &str,
    data: &Value,
) -> Result<(), AppError> {
    let mut decision = if name == review::TOOL_NAME {
        let interactive =
            io.stdin_is_tty && !args.machine && !args.stream_json && !args.non_interactive;
        let mut chrome_out: Vec<u8> = Vec::new();
        let decision = review::decide(data, interactive, io.stdin, &mut chrome_out);
        io.write(chrome, &String::from_utf8_lossy(&chrome_out));
        Some(decision)
    } else {
        policy_decision(args, name, io.stdin_is_tty)
    };
    if decision.is_none()
        && turn
            .session_allowed
            .as_ref()
            .is_some_and(|allowed| allowed.iter().any(|tool| tool == name))
    {
        // The daemon's allow_session spans one ai.chat request; the REPL
        // carries the user's "session" answer across turns.
        decision = Some("allow_session".into());
    }
    let decision = match decision {
        Some(decision) => decision,
        None => {
            let summary = match get(data, "summary") {
                Some(summary) if truthy(summary) => py_str(summary),
                _ => name.to_owned(),
            };
            let preview = [get(data, "arguments_preview"), get(data, "arguments")]
                .into_iter()
                .flatten()
                .find(|value| truthy(value))
                .cloned()
                .unwrap_or_else(|| Value::Dict(Vec::new()));
            let decision = interactive_consent(io, chrome, name, &summary, &preview);
            if decision == "allow_session" {
                if let Some(allowed) = turn.session_allowed.as_mut() {
                    if !allowed.iter().any(|tool| tool == name) {
                        allowed.push(name.to_owned());
                    }
                }
            }
            decision
        }
    };
    if decision == "cancel" {
        turn.control.push(send_cancel(client, &turn.request_id)?);
    } else if CONSENT_DECISIONS.contains(&decision.as_str()) {
        let request_id = format!("chat-consent-{}", random_hex());
        client.send(
            &dict([
                ("request_id", s(&request_id)),
                ("kind", s("ai.tool_call.consent")),
                (
                    "args",
                    dict([
                        ("target_request_id", s(&turn.request_id)),
                        ("call_id", s(call_id)),
                        ("decision", s(&decision)),
                    ]),
                ),
            ]),
            true,
        )?;
        turn.control.push(request_id);
    } else {
        return Err(AppError::new("validation", "invalid consent decision")
            .with_details(dict([("decision", s(&decision))])));
    }
    Ok(())
}

fn send_cancel(client: &mut DaemonClient, target: &str) -> Result<String, AppError> {
    let request_id = format!("chat-cancel-{}", random_hex());
    client.send(
        &dict([
            ("request_id", s(&request_id)),
            ("kind", s("ai.chat.cancel")),
            ("args", dict([("target_request_id", s(target))])),
        ]),
        true,
    )?;
    Ok(request_id)
}

/// A finished turn: its answer text, terminal record and tool calls.
pub struct TurnResult {
    pub content: String,
    pub terminal: Value,
    pub tool_calls: Vec<Value>,
}

/// How a turn is shown.
struct Show {
    out: Target,
    chrome: Target,
    render: bool,
    stream_json: bool,
}

/// `_run_turn(...)` and `_stream_turn_records(...)`.
fn run_turn(
    client: &mut DaemonClient,
    args: &Args,
    io: &mut Io<'_>,
    messages: &[Value],
    show: &Show,
    session_allowed: Option<&mut Vec<String>>,
    session_id: Option<&str>,
) -> Result<TurnResult, AppError> {
    let request_id = format!("chat-{}", random_hex());
    client.send(
        &dict([
            ("request_id", s(&request_id)),
            ("kind", s("ai.chat")),
            ("args", chat_args(args, messages, session_id)?),
        ]),
        true,
    )?;
    let pretty = show.render && io.is_tty(show.out) && !args.plain;
    let mut markdown = pretty.then(MarkdownStream::default);
    let mut turn = TurnState {
        request_id,
        content: String::new(),
        tool_calls: Vec::new(),
        control: Vec::new(),
        consented: Vec::new(),
        session_allowed,
    };
    let result = stream_turn(client, args, io, show, &mut turn, markdown.as_mut());
    if result.is_err() {
        // A failed turn must not leak open styles into the next prompt.
        if let Some(markdown) = markdown.as_mut() {
            let rest = markdown.flush();
            io.write(show.out, &rest);
        }
    }
    result
}

fn stream_turn(
    client: &mut DaemonClient,
    args: &Args,
    io: &mut Io<'_>,
    show: &Show,
    turn: &mut TurnState<'_>,
    mut markdown: Option<&mut MarkdownStream>,
) -> Result<TurnResult, AppError> {
    loop {
        let record = client.read(true, None)?;
        let kind = get(&record, "kind").map(py_str).unwrap_or_default();
        let record_id = get(&record, "request_id").cloned();
        if let Some(Value::Str(id)) = &record_id {
            let id = py_str(&Value::Str(id.clone()));
            if turn.control.contains(&id) {
                if kind == "error" {
                    return Err(error_from_envelope(
                        &record,
                        "chat control failed",
                        "chat_control_failed",
                    ));
                }
                continue;
            }
        }
        if record_id != Some(s(&turn.request_id)) {
            continue;
        }
        if show.stream_json {
            io.write(Target::Out, &(compact(&record) + "\n"));
        }
        if kind == "auth_required" {
            return Err(auth_required_error(&record));
        }
        if kind == "error" {
            return Err(error_from_envelope(&record, "chat failed", "chat_failed"));
        }
        let data = match get(&record, "data") {
            Some(data @ Value::Dict(_)) => data.clone(),
            _ => Value::Dict(Vec::new()),
        };
        match kind.as_str() {
            "ai.chat.status" => {
                if let Some(label) = get(&data, "label").filter(|label| truthy(label)) {
                    if show.render {
                        io.write_dim(show.chrome, &format!("{}...\n", py_str(label)));
                    }
                }
            }
            "ai.chat.delta" => {
                let delta = match get(&data, "delta") {
                    Some(delta @ Value::Dict(_)) => delta.clone(),
                    _ => Value::Dict(Vec::new()),
                };
                if let Some(Value::Str(reasoning)) = get(&delta, "reasoning") {
                    if show.render
                        && reasoning.code_points().next().is_some()
                        && io.is_tty(show.out)
                    {
                        io.write(
                            show.out,
                            &format!("{DIM}{}{RESET}", py_str(&Value::Str(reasoning.clone()))),
                        );
                    }
                }
                if let Some(Value::Str(visible)) = get(&delta, "content") {
                    if visible.code_points().next().is_some() {
                        let visible = py_str(&Value::Str(visible.clone()));
                        turn.content.push_str(&visible);
                        if let Some(markdown) = markdown.as_deref_mut() {
                            let text = markdown.feed(&visible);
                            io.write(show.out, &text);
                        } else if show.render {
                            io.write(show.out, &visible);
                        }
                    }
                }
            }
            "ai.chat.tool_call" => {
                let Some(Value::Str(call_id)) = get(&data, "call_id") else {
                    continue;
                };
                let call_id = py_str(&Value::Str(call_id.clone()));
                let needs_consent = get(&data, "needs_consent").is_some_and(truthy);
                let entry = dict([
                    ("call_id", s(&call_id)),
                    ("name", get(&data, "name").cloned().unwrap_or(Value::Null)),
                    (
                        "arguments",
                        get(&data, "arguments")
                            .filter(|value| truthy(value))
                            .cloned()
                            .unwrap_or_else(|| Value::Dict(Vec::new())),
                    ),
                    (
                        "kind_class",
                        get(&data, "kind_class")
                            .cloned()
                            .unwrap_or_else(|| s("unknown")),
                    ),
                    ("needs_consent", Value::Bool(needs_consent)),
                    (
                        "status",
                        s(if needs_consent {
                            "awaiting_consent"
                        } else {
                            "running"
                        }),
                    ),
                ]);
                match turn.call(&call_id) {
                    Some(slot) => *slot = entry,
                    None => turn.tool_calls.push((call_id.clone(), entry)),
                }
                if show.render {
                    let name = get(&data, "name")
                        .map(py_str)
                        .unwrap_or_else(|| "unknown".into());
                    let suffix = if needs_consent {
                        " (needs consent)"
                    } else {
                        ""
                    };
                    io.write(show.chrome, &format!("\nTool: {name}{suffix}\n"));
                }
                if let Some(Value::Str(name)) = get(&data, "name") {
                    let name = py_str(&Value::Str(name.clone()));
                    if needs_consent
                        && name != review::TOOL_NAME
                        && !turn.consented.contains(&call_id)
                    {
                        decide_and_send_consent(
                            client,
                            args,
                            io,
                            show.chrome,
                            turn,
                            &call_id,
                            &name,
                            &data,
                        )?;
                        turn.consented.push(call_id);
                    }
                }
            }
            "ai.chat.tool_consent_required" => {
                let (Some(Value::Str(call_id)), Some(Value::Str(name))) =
                    (get(&data, "call_id"), get(&data, "name"))
                else {
                    continue;
                };
                let call_id = py_str(&Value::Str(call_id.clone()));
                let name = py_str(&Value::Str(name.clone()));
                if turn.consented.contains(&call_id) {
                    continue;
                }
                decide_and_send_consent(
                    client,
                    args,
                    io,
                    show.chrome,
                    turn,
                    &call_id,
                    &name,
                    &data,
                )?;
                turn.consented.push(call_id);
            }
            "ai.chat.tool_result" => {
                let Some(Value::Str(call_id)) = get(&data, "call_id") else {
                    continue;
                };
                let call_id = py_str(&Value::Str(call_id.clone()));
                if turn.call(&call_id).is_none() {
                    turn.tool_calls.push((
                        call_id.clone(),
                        dict([
                            ("call_id", s(&call_id)),
                            ("name", s("Tool")),
                            ("arguments", Value::Dict(Vec::new())),
                            ("kind_class", s("unknown")),
                            ("needs_consent", Value::Bool(false)),
                        ]),
                    ));
                }
                let ok = get(&data, "ok").is_some_and(truthy);
                let reason = get(&data, "reason").cloned().unwrap_or(Value::Null);
                let status = if ok {
                    "done"
                } else if matches!(&reason, Value::Str(r) if r.is("user_denied") || r.is("consent_timeout"))
                {
                    "denied"
                } else {
                    "failed"
                };
                let slot = turn.call(&call_id).expect("inserted above");
                set(slot, "status", s(status));
                set(slot, "reason", reason.clone());
                set(slot, "ok", Value::Bool(ok));
                if show.render && !ok && truthy(&reason) {
                    io.write(
                        show.chrome,
                        &format!("\nTool result: {}\n", py_str(&reason)),
                    );
                }
                if markdown.is_some()
                    && io.is_tty(show.chrome)
                    && ok
                    && !call_id.starts_with("auto_read")
                {
                    if let Some(envelope @ Value::Dict(_)) = get(&data, "envelope") {
                        // The numbers come from the daemon envelope, not from
                        // the model's retelling.
                        let width = render::terminal_width(io.columns.as_deref());
                        if let Some(table) = render_envelope_table(envelope, width) {
                            io.write(show.chrome, &(table + "\n"));
                        }
                    }
                }
            }
            "ai.chat" => {
                if let Some(markdown) = markdown.as_deref_mut() {
                    let rest = markdown.flush();
                    io.write(show.out, &rest);
                }
                if show.render && (!turn.content.is_empty() || io.is_tty(show.out)) {
                    io.write(show.out, "\n");
                }
                return Ok(TurnResult {
                    content: turn.content.clone(),
                    terminal: record,
                    tool_calls: turn
                        .tool_calls
                        .iter()
                        .map(|(_, entry)| entry.clone())
                        .collect(),
                });
            }
            _ => {}
        }
    }
}

/// `_render_turn_footer(terminal, chrome)`.
fn render_footer(io: &mut Io<'_>, terminal: &Value, chrome: Target) {
    let data = match get(terminal, "data") {
        Some(data @ Value::Dict(_)) => data.clone(),
        _ => Value::Dict(Vec::new()),
    };
    let finish = match get(&data, "finish_reason") {
        Some(reason) if truthy(reason) => py_str(reason),
        _ => String::new(),
    };
    if let Some((_, notice)) = FINISH_NOTICES.iter().find(|(reason, _)| *reason == finish) {
        io.write(Target::Err, &format!("{notice}\n"));
    }
    let provenance = match get(&data, "provenance") {
        Some(provenance @ Value::Dict(_)) => provenance.clone(),
        _ => Value::Dict(Vec::new()),
    };
    let mut parts: Vec<String> = Vec::new();
    if let (Some(provider), Some(model)) = (
        get(&data, "provider").filter(|v| truthy(v)),
        get(&data, "model").filter(|v| truthy(v)),
    ) {
        parts.push(format!("{}/{}", py_str(provider), py_str(model)));
    }
    if let Some(Value::List(used)) = get(&provenance, "tools_used") {
        if !used.is_empty() {
            parts.push(format!(
                "tools: {}",
                used.iter().map(py_str).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    if get(&provenance, "auto_journal_processed").is_some_and(truthy) {
        parts.push("journals auto-refreshed".into());
    }
    if get(&provenance, "auto_sync_attempted").is_some_and(truthy) {
        parts.push(
            if get(&provenance, "auto_sync_ok").is_some_and(truthy) {
                "synced"
            } else {
                "sync attempted"
            }
            .into(),
        );
    }
    if !parts.is_empty() {
        io.write_dim(chrome, &format!("— {}\n", parts.join(" · ")));
    }
}

/// `ChatSessionResult.to_payload()`.
fn session_payload(args: &Args, turns: &[TurnResult]) -> Value {
    let last = turns.last();
    let terminal = last
        .and_then(|turn| get(&turn.terminal, "data").cloned())
        .unwrap_or_else(|| Value::Dict(Vec::new()));
    let field = |key: &str, fallback: Value| get(&terminal, key).cloned().unwrap_or(fallback);
    dict([
        (
            "provider",
            field("provider", args.provider.as_deref().map_or(Value::Null, s)),
        ),
        (
            "model",
            field("model", args.model.as_deref().map_or(Value::Null, s)),
        ),
        (
            "message",
            dict([
                ("role", s("assistant")),
                ("content", s(last.map_or("", |turn| turn.content.as_str()))),
            ]),
        ),
        (
            "finish_reason",
            if last.is_some() {
                field("finish_reason", Value::Null)
            } else {
                Value::Null
            },
        ),
        (
            "provenance",
            if last.is_some() {
                field("provenance", Value::Null)
            } else {
                Value::Null
            },
        ),
        (
            "session_id",
            if last.is_some() {
                field("session_id", Value::Null)
            } else {
                Value::Null
            },
        ),
        (
            "tool_calls",
            Value::List(last.map_or_else(Vec::new, |turn| turn.tool_calls.clone())),
        ),
    ])
}

/// What `run_chat_command` needs from the shell.
pub struct Start<'a> {
    pub data_root: &'a str,
    pub env_file: Option<&'a str>,
    /// Resolves the book's passphrase the way ordinary commands unlock
    /// (`resolve_db_passphrase_for_bypass`); `None` for a plaintext book.
    pub passphrase:
        &'a mut dyn FnMut(bool) -> Result<Option<kassiber_runtime::store::Secret>, AppError>,
}

/// `run_chat_command(args)`: the payload for `--machine`, or `None`.
pub fn run(
    args: &mut Args,
    prompt: Option<String>,
    prompt_text: Option<String>,
    io: &mut Io<'_>,
    start: Start<'_>,
) -> Result<Option<Value>, AppError> {
    if prompt.as_deref().is_some_and(|p| !p.is_empty())
        && prompt_text.as_deref().is_some_and(|p| !p.is_empty())
    {
        return Err(AppError::new(
            "validation",
            "pass the chat prompt either positionally or with --prompt, not both",
        ));
    }
    let mut one_shot = prompt_text.or(prompt);
    if args.stream_json && args.machine {
        return Err(AppError::new(
            "validation",
            "--stream-json already emits NDJSON; drop --machine / --format json",
        ));
    }
    if (args.machine || args.stream_json) && one_shot.is_none() {
        return Err(AppError::new(
            "validation",
            "machine chat output requires a one-shot prompt",
        ));
    }
    if args.non_interactive && one_shot.is_none() {
        return Err(AppError::new(
            "interaction_required",
            "non-interactive chat requires a one-shot prompt",
        )
        .with_hint("Pass a prompt argument, --prompt, or `-` to read the prompt from stdin."));
    }
    if args.incognito
        && (args.continue_session || args.session.as_deref().is_some_and(|s| !s.is_empty()))
    {
        return Err(AppError::new(
            "validation",
            "--incognito cannot continue a persisted session",
        ));
    }
    if one_shot.is_none() && !io.stdin_is_tty {
        return Err(AppError::new(
            "validation",
            "interactive chat requires a TTY; pass a prompt for one-shot mode",
        )
        .with_hint("Use `kassiber chat \"...\"` or `kassiber chat --prompt \"...\"`."));
    }
    if one_shot.as_deref() == Some("-") {
        let mut text = String::new();
        let _ = io.stdin.read_to_string(&mut text);
        let text = kassiber_pytext::strip(&text).to_owned();
        if text.is_empty() {
            return Err(AppError::new("validation", "stdin prompt is empty")
                .with_hint("Pipe or type the prompt when using `kassiber chat -`."));
        }
        one_shot = Some(text);
    }
    let transcript = match args.transcript.as_deref() {
        Some(path) => Some(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|error| AppError::new("internal_error", error.to_string()))?,
        ),
        None => None,
    };
    let timeout = timeout_seconds(&args.timeout_seconds)?;
    let allow_prompt = io.stdin_is_tty && !args.non_interactive;
    let passphrase = (start.passphrase)(allow_prompt)?;
    let mut client = DaemonClient::start(Launch {
        data_root: start.data_root,
        env_file: start.env_file,
        passphrase,
        timeout: Duration::from_secs_f64(timeout),
        transcript,
    })?;
    let outcome = session(&mut client, args, one_shot, io);
    client.close();
    outcome
}

fn session(
    client: &mut DaemonClient,
    args: &mut Args,
    one_shot: Option<String>,
    io: &mut Io<'_>,
) -> Result<Option<Value>, AppError> {
    resolve_default_model(client, args)?;
    if let Some(notice) = incognito_notice(client, args)? {
        io.write(Target::Err, &format!("{notice}\n"));
    }
    // Staged once per session, so a REPL can keep discussing the file.
    args.attachment = stage_attachment(client, args)?;
    let render = !args.machine && !args.stream_json;
    let (mut chat_session_id, mut messages, stored) = resolve_continuation(client, args)?;
    if render {
        if let Some(stored) = &stored {
            let title = get(stored, "title")
                .map(py_str)
                .unwrap_or_else(|| chat_session_id.clone().unwrap_or_default());
            io.write(
                Target::Out,
                &format!("Continuing: {title} ({} messages)\n", messages.len()),
            );
        }
    }
    let mut turns: Vec<TurnResult> = Vec::new();
    if let Some(prompt) = one_shot {
        // Piped stdout gets only the answer; everything else goes to stderr.
        let chrome = if io.stdout_is_tty {
            Target::Out
        } else {
            Target::Err
        };
        messages.push(dict([("role", s("user")), ("content", s(&prompt))]));
        let show = Show {
            out: Target::Out,
            chrome,
            render,
            stream_json: args.stream_json,
        };
        let result = run_turn(
            client,
            args,
            io,
            &messages,
            &show,
            None,
            chat_session_id.as_deref(),
        )?;
        if render {
            render_footer(io, &result.terminal, chrome);
        }
        turns.push(result);
        return Ok(args.machine.then(|| session_payload(args, &turns)));
    }
    io.write(
        Target::Out,
        "Kassiber chat. /help for commands, /exit to quit.\n",
    );
    let mut session_allowed: Vec<String> = Vec::new();
    loop {
        io.write(Target::Out, "> ");
        let line = io.read_line();
        if line.is_empty() {
            break;
        }
        let prompt = kassiber_pytext::strip(&line).to_owned();
        if prompt.is_empty() {
            continue;
        }
        if prompt.starts_with('/') {
            let mut parts = prompt.splitn(2, |c: char| kassiber_pytext::is_space(u32::from(c)));
            let command = parts.next().unwrap_or("").to_owned();
            let arg = kassiber_pytext::strip(parts.next().unwrap_or("")).to_owned();
            match command.as_str() {
                "/exit" | "/quit" => break,
                "/help" => io.write(Target::Out, REPL_HELP),
                "/tools" => render_tool_listing(args, io),
                "/model" => {
                    if arg.is_empty() {
                        let model = args.model.clone().unwrap_or_else(|| "None".into());
                        io.write(Target::Out, &format!("model: {model}\n"));
                    } else {
                        io.write(Target::Out, &format!("Switched to model {arg}.\n"));
                        args.model = Some(arg);
                    }
                }
                "/provider" => provider_command(client, args, &arg, io)?,
                "/allow" => allow_command(&arg, &mut session_allowed, io),
                "/allowed" => render_allowed(args, &session_allowed, io),
                "/new" => {
                    messages.clear();
                    chat_session_id = None;
                    io.write(Target::Out, "Started a new conversation.\n");
                }
                _ => io.write(
                    Target::Out,
                    &format!("Unknown command {command}. /help lists commands.\n"),
                ),
            }
            continue;
        }
        messages.push(dict([("role", s("user")), ("content", s(&prompt))]));
        let show = Show {
            out: Target::Out,
            chrome: Target::Out,
            render: true,
            stream_json: false,
        };
        let result = match run_turn(
            client,
            args,
            io,
            &messages,
            &show,
            Some(&mut session_allowed),
            chat_session_id.as_deref(),
        ) {
            Ok(result) => result,
            Err(error)
                if error.code == "daemon_closed" || error.code == "daemon_protocol_error" =>
            {
                return Err(error)
            }
            Err(error) => {
                // Keep the session and its history across a failed turn.
                messages.pop();
                io.write(Target::Err, &format!("Error: {}\n", error.message));
                if let Some(hint) = &error.hint {
                    io.write(Target::Err, &format!("Hint: {hint}\n"));
                }
                continue;
            }
        };
        if let Some(Value::Str(id)) =
            get(&result.terminal, "data").and_then(|data| get(data, "session_id"))
        {
            chat_session_id = Some(py_str(&Value::Str(id.clone())));
        }
        render_footer(io, &result.terminal, Target::Out);
        if !result.content.is_empty() {
            messages.push(dict([
                ("role", s("assistant")),
                ("content", s(&result.content)),
            ]));
        }
        turns.push(result);
    }
    Ok(None)
}

/// `_render_tool_listing(args, out)`.
fn render_tool_listing(args: &Args, io: &mut Io<'_>) {
    let mut catalog: Vec<_> = tools()
        .iter()
        .filter(|entry| {
            args.tool_profile != "core" || CORE_TOOL_NAMES.contains(&entry.name.as_str())
        })
        .collect();
    let width = catalog
        .iter()
        .map(|entry| entry.name.chars().count())
        .max()
        .unwrap_or(0);
    io.write(
        Target::Out,
        &format!("Tool profile: {}\n", args.tool_profile),
    );
    catalog.sort_by(|a, b| (a.kind_class.as_str(), &a.name).cmp(&(b.kind_class.as_str(), &b.name)));
    for entry in catalog {
        let consent = if entry.kind_class == KindClass::Mutating {
            "mutating (asks consent)"
        } else {
            "read-only"
        };
        io.write(
            Target::Out,
            &format!("  {:<width$}  {consent}\n", entry.name),
        );
    }
}

/// `_handle_provider_command(client, args, arg, out)`.
fn provider_command(
    client: &mut DaemonClient,
    args: &mut Args,
    arg: &str,
    io: &mut Io<'_>,
) -> Result<(), AppError> {
    if arg.is_empty() {
        let current = args
            .provider
            .clone()
            .unwrap_or_else(|| "(stored default)".into());
        io.write(Target::Out, &format!("provider: {current}\n"));
        return Ok(());
    }
    let previous = (
        args.provider.clone(),
        args.model.clone(),
        args.reasoning_effort.clone(),
        args.reasoning_effort_from_provider,
    );
    args.provider = Some(arg.to_owned());
    args.model = None;
    if previous.3 {
        args.reasoning_effort = None;
        args.reasoning_effort_from_provider = false;
    }
    if let Err(error) = resolve_default_model(client, args) {
        (
            args.provider,
            args.model,
            args.reasoning_effort,
            args.reasoning_effort_from_provider,
        ) = previous;
        io.write(Target::Out, &format!("Error: {}\n", error.message));
        if let Some(hint) = &error.hint {
            io.write(Target::Out, &format!("Hint: {hint}\n"));
        }
        return Ok(());
    }
    let model = args.model.clone().unwrap_or_default();
    io.write(
        Target::Out,
        &format!("Switched to provider {arg}, model {model}.\n"),
    );
    if let Some(notice) = incognito_notice(client, args)? {
        io.write(Target::Out, &format!("{notice}\n"));
    }
    Ok(())
}

/// `_handle_allow_command(arg, session_allowed, out)`.
fn allow_command(arg: &str, allowed: &mut Vec<String>, io: &mut Io<'_>) {
    if arg.is_empty() {
        io.write(Target::Out, "Usage: /allow <tool-name>\n");
        return;
    }
    let mutating = mutating_tool_names();
    let mut matched: Vec<String> = split_tool_names(&[arg.to_owned()])
        .into_iter()
        .filter(|name| mutating.contains(name))
        .collect();
    matched.sort();
    if let Some(index) = matched.iter().position(|name| name == review::TOOL_NAME) {
        io.write(
            Target::Out,
            "ui.review.apply requires a fresh once-only review; it cannot be pre-allowed.\n",
        );
        matched.remove(index);
        if matched.is_empty() {
            return;
        }
    }
    if matched.is_empty() {
        io.write(
            Target::Out,
            &format!("{arg} is not a known mutating tool; /tools lists them.\n"),
        );
        return;
    }
    for name in &matched {
        if !allowed.contains(name) {
            allowed.push(name.clone());
        }
    }
    io.write(
        Target::Out,
        &format!("Allowed for this session: {}\n", matched.join(", ")),
    );
}

/// `_render_allowed(args, session_allowed, out)`.
fn render_allowed(args: &Args, allowed: &[String], io: &mut Io<'_>) {
    if args.yes {
        io.write(
            Target::Out,
            "Mutating tools are allowed for this session (--yes), except ui.review.apply.\n",
        );
        return;
    }
    let mutating = mutating_tool_names();
    let mut flagged: Vec<String> = split_tool_names(&args.allow_tool)
        .into_iter()
        .filter(|name| mutating.contains(name) && name != review::TOOL_NAME)
        .collect();
    flagged.sort();
    let mut lines: Vec<String> = flagged
        .iter()
        .map(|name| format!("  {name}  (--allow-tool)"))
        .collect();
    let mut session: Vec<&String> = allowed
        .iter()
        .filter(|name| !flagged.contains(name) && *name != review::TOOL_NAME)
        .collect();
    session.sort();
    session.dedup();
    lines.extend(
        session
            .into_iter()
            .map(|name| format!("  {name}  (this session)")),
    );
    if lines.is_empty() {
        io.write(
            Target::Out,
            "No mutating tools are pre-allowed; each will ask for consent.\n",
        );
        return;
    }
    io.write(Target::Out, &(lines.join("\n") + "\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeouts_are_validated_locally() {
        assert_eq!(timeout_seconds(&Value::Float(7.0)).unwrap(), 7.0);
        assert_eq!(timeout_seconds(&Value::Null).unwrap(), 120.0);
        for bad in [0.0, 7200.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                timeout_seconds(&Value::Float(bad)).unwrap_err().code,
                "validation"
            );
        }
    }

    #[test]
    fn wire_names_map_to_display_names() {
        let names = split_tool_names(&["ui_journals_process, status".into()]);
        assert!(names.contains(&"ui.journals.process".to_owned()));
        assert!(names.contains(&"status".to_owned()));
    }

    #[test]
    fn texts_match_python() {
        let document = kassiber_ai::catalog::catalog_document();
        let constants = get(&document, "constants").unwrap();
        assert_eq!(get(constants, "cli_repl_help"), Some(&s(REPL_HELP)));
        assert_eq!(
            get(constants, "copilot_incognito_notice"),
            Some(&s(COPILOT_INCOGNITO_NOTICE))
        );
        assert_eq!(
            get(constants, "cli_finish_notices"),
            Some(&dict(FINISH_NOTICES.iter().map(|(k, v)| (*k, s(*v)))))
        );
    }
}

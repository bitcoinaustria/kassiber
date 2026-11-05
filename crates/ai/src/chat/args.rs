//! `_ai_chat_args(args)` (`kassiber/daemon.py:5268`) and the screen-context,
//! attachment and seed-notice checks it calls, in Python's order: the first
//! failing check is the error the caller sees.

use kassiber_envelope::{dict, get, json_ready, set, AppError};
use kassiber_pyjson::{dumps, BigInt, Key, Options, PyStr, Value};

use crate::projections::contains_path_or_url;
use crate::prompt::{normalize_system_prompt_kind, PromptKind};
use crate::provenance::PROVIDER_STATUS_NOTICES;
use crate::pyval::s;
use crate::redact::redact_tool_arguments;
use crate::select::{select_tool_capabilities, TOOL_CAPABILITY_NAMES, TOOL_PROFILE_NAMES};

/// `_AI_SCREEN_ROUTES`, sorted.
pub const SCREEN_ROUTES: [&str; 25] = [
    "/",
    "/activity",
    "/assistant",
    "/books",
    "/chain-analysis",
    "/connections",
    "/custody-gaps",
    "/diagnostics",
    "/egress",
    "/exit-tax",
    "/imports",
    "/journals",
    "/logs",
    "/overview",
    "/privacy-mirror",
    "/profiles",
    "/quarantine",
    "/reconcile",
    "/reports",
    "/settings",
    "/source-of-funds",
    "/swaps",
    "/tax-events",
    "/transactions",
    "/transfers",
];

/// The entity types a screen may name, sorted.
const ENTITY_TYPES: [&str; 7] = [
    "connection",
    "custody_gap",
    "profile",
    "report",
    "source_funds_case",
    "transaction",
    "wallet",
];

/// `SCREEN_CONTEXT_METRIC_DEFINITIONS`' names, sorted.
pub const SCREEN_METRICS: [&str; 5] = [
    "incoming_count",
    "outgoing_count",
    "review_queue_failed",
    "review_queue_pending",
    "review_queue_review",
];

const SCREEN_FIELDS: [&str; 7] = [
    "capabilities",
    "entity_id",
    "entity_type",
    "filters",
    "metrics",
    "metrics_scope",
    "route",
];

const METRICS_SCOPE_FIELDS: [&str; 3] = ["input_version", "profile_id", "workspace_id"];
const METRICS_SCOPE_OPTIONAL: [&str; 2] = ["pairs_grouped", "wallet"];

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}

fn names(items: &[&str]) -> Value {
    Value::List(items.iter().map(s).collect())
}

fn sorted_keys(value: &Value) -> Vec<PyStr> {
    let mut keys: Vec<PyStr> = match value {
        Value::Dict(pairs) => pairs
            .iter()
            .filter_map(|(key, _)| match key {
                Key::Str(key) => Some(key.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    keys.sort();
    keys
}

fn str_is(value: Option<&Value>, allowed: &[&str]) -> bool {
    matches!(value, Some(Value::Str(text)) if allowed.iter().any(|name| text.is(name)))
}

/// `len(s)` in code points.
fn chars(text: &PyStr) -> usize {
    text.code_points().count()
}

/// `not s.strip()`.
fn is_blank(text: &PyStr) -> bool {
    kassiber_pytext::strip_py(text)
        .code_points()
        .next()
        .is_none()
}

fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| **value != Value::Null)
}

/// `_ai_chat_screen_context(raw)`.
pub fn screen_context(raw: Option<&Value>) -> Result<Option<Value>, AppError> {
    let raw = match raw {
        None | Some(Value::Null) => return Ok(None),
        Some(raw @ Value::Dict(_)) => raw,
        Some(_) => return Err(validation("ai.chat screen_context must be an object")),
    };
    let unknown: Vec<Value> = sorted_keys(raw)
        .into_iter()
        .filter(|key| !SCREEN_FIELDS.iter().any(|field| key.is(field)))
        .map(Value::Str)
        .collect();
    if !unknown.is_empty() {
        return Err(
            validation("ai.chat screen_context received unsupported fields")
                .with_details(dict([("unknown", Value::List(unknown))])),
        );
    }
    let route = present(get(raw, "route"));
    if route.is_some() && !str_is(route, &SCREEN_ROUTES) {
        return Err(
            validation("ai.chat screen_context.route must be a canonical app route")
                .with_details(dict([("allowed", names(&SCREEN_ROUTES))])),
        );
    }
    let entity_type = present(get(raw, "entity_type"));
    if entity_type.is_some() && !str_is(entity_type, &ENTITY_TYPES) {
        return Err(
            validation("ai.chat screen_context.entity_type is unsupported")
                .with_details(dict([("allowed", names(&ENTITY_TYPES))])),
        );
    }
    let entity_id = present(get(raw, "entity_id"));
    if let Some(entity_id) = entity_id {
        let bounded = matches!(entity_id, Value::Str(text)
            if !is_blank(text) && chars(text) <= 256 && !contains_path_or_url(entity_id));
        if !bounded {
            return Err(validation(
                "ai.chat screen_context.entity_id must be a bounded local identifier",
            ));
        }
    }
    let filters = present(get(raw, "filters"));
    if let Some(filters) = filters {
        if !matches!(filters, Value::Dict(pairs) if pairs.len() <= 25) {
            return Err(validation(
                "ai.chat screen_context.filters must be a small object",
            ));
        }
        let oversized = dumps(&json_ready(filters.clone()), &Options::new())
            .map(|text| text.chars().count() > 4096)
            .unwrap_or(true);
        if redact_tool_arguments(filters) != *filters || contains_path_or_url(filters) || oversized
        {
            return Err(validation(
                "ai.chat screen_context.filters contain sensitive or oversized data",
            ));
        }
    }
    let capabilities = present(get(raw, "capabilities"));
    if let Some(capabilities) = capabilities {
        let known = matches!(capabilities, Value::List(items)
            if items.iter().all(|item| str_is(Some(item), &TOOL_CAPABILITY_NAMES)));
        if !known {
            return Err(validation(
                "ai.chat screen_context.capabilities contains an unsupported capability",
            )
            .with_details(dict([("allowed", names(&TOOL_CAPABILITY_NAMES))])));
        }
    }
    let metrics = present(get(raw, "metrics"));
    if let Some(metrics) = metrics {
        let valid = match metrics {
            Value::Dict(pairs) => {
                pairs.len() <= SCREEN_METRICS.len() && pairs.iter().all(|(key, value)| {
                    matches!(key, Key::Str(key) if SCREEN_METRICS.iter().any(|name| key.is(name)))
                        && matches!(value, Value::Int(n)
                                if *n >= BigInt::from(0) && *n <= BigInt::from(1_000_000_000))
                })
            }
            _ => false,
        };
        if !valid {
            return Err(validation(
                "ai.chat screen_context.metrics must map known page metrics to counts",
            )
            .with_details(dict([("allowed", names(&SCREEN_METRICS))])));
        }
    }
    let scope = metrics_scope(get(raw, "metrics_scope"))?;
    let has_metrics = metrics.is_some_and(kassiber_envelope::truthy);
    if has_metrics != scope.is_some() {
        return Err(validation(
            "ai.chat screen_context.metrics and metrics_scope must be sent together",
        ));
    }
    let mut bound: Vec<(Key, Value)> = Vec::new();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            bound.push((Key::from(key), value));
        }
    };
    put("route", route.cloned());
    put("entity_type", entity_type.cloned());
    put(
        "entity_id",
        entity_id.map(|value| match value {
            Value::Str(text) => Value::Str(kassiber_pytext::strip_py(text)),
            other => other.clone(),
        }),
    );
    put("filters", filters.cloned());
    put("capabilities", capabilities.cloned());
    put(
        "metrics",
        metrics.filter(|m| kassiber_envelope::truthy(m)).cloned(),
    );
    put("metrics_scope", scope);
    Ok(Some(Value::Dict(bound)))
}

/// `_ai_chat_screen_metrics_scope(raw)`: shape only.
fn metrics_scope(raw: Option<&Value>) -> Result<Option<Value>, AppError> {
    let raw = match raw {
        None | Some(Value::Null) => return Ok(None),
        Some(raw) => raw,
    };
    let invalid = || {
        validation(
            "ai.chat screen_context.metrics_scope must name the book, input version and wallet filter the metrics were counted on",
        )
        .with_details(dict([
            ("fields", names(&METRICS_SCOPE_FIELDS)),
            ("optional", names(&METRICS_SCOPE_OPTIONAL)),
        ]))
    };
    if !matches!(raw, Value::Dict(_)) {
        return Err(invalid());
    }
    let keys = sorted_keys(raw);
    let has = |name: &str| keys.iter().any(|key| key.is(name));
    if !METRICS_SCOPE_FIELDS.iter().all(|field| has(field))
        || keys.iter().any(|key| {
            !METRICS_SCOPE_FIELDS
                .iter()
                .chain(METRICS_SCOPE_OPTIONAL.iter())
                .any(|name| key.is(name))
        })
    {
        return Err(invalid());
    }
    let wallet = present(get(raw, "wallet"));
    if let Some(wallet) = wallet {
        let safe = matches!(wallet, Value::Str(text)
            if !is_blank(text)
                && chars(text) <= 256
                && !contains_path_or_url(wallet)
                && redact_tool_arguments(wallet) == *wallet);
        if !safe {
            return Err(invalid());
        }
    }
    for key in ["workspace_id", "profile_id"] {
        let value = get(raw, key).unwrap_or(&Value::Null);
        let valid = matches!(value, Value::Str(text)
            if !is_blank(text) && chars(text) <= 256 && !contains_path_or_url(value));
        if !valid {
            return Err(invalid());
        }
    }
    if !matches!(get(raw, "input_version"), Some(Value::Int(n)) if *n >= BigInt::from(0)) {
        return Err(invalid());
    }
    if has("pairs_grouped") && !matches!(get(raw, "pairs_grouped"), Some(Value::Bool(_))) {
        return Err(invalid());
    }
    let mut scope: Vec<(Key, Value)> = METRICS_SCOPE_FIELDS
        .iter()
        .map(|key| {
            (
                Key::from(*key),
                get(raw, key).cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    scope.push((
        Key::from("wallet"),
        match wallet {
            Some(Value::Str(text)) => Value::Str(kassiber_pytext::strip_py(text)),
            _ => Value::Null,
        },
    ));
    if has("pairs_grouped") {
        scope.push((
            Key::from("pairs_grouped"),
            get(raw, "pairs_grouped").cloned().unwrap_or(Value::Null),
        ));
    }
    Ok(Some(Value::Dict(scope)))
}

/// `_ai_chat_attachment(raw)`: an opaque staging grant, never a path.
pub fn attachment(raw: Option<&Value>) -> Result<Option<Value>, AppError> {
    let raw = match raw {
        None | Some(Value::Null) => return Ok(None),
        Some(raw @ Value::Dict(_)) => raw,
        Some(_) => return Err(validation("ai.chat attachment must be an object")),
    };
    let unknown: Vec<Value> = sorted_keys(raw)
        .into_iter()
        .filter(|key| !key.is("token") && !key.is("label"))
        .map(Value::Str)
        .collect();
    if !unknown.is_empty() {
        return Err(validation("ai.chat attachment received unsupported fields")
            .with_details(dict([("unknown", Value::List(unknown))])));
    }
    let token = match get(raw, "token") {
        Some(Value::Str(token)) if !is_blank(token) => token,
        _ => {
            return Err(validation("ai.chat attachment requires a staging token")
                .with_hint("Stage the file first; do not pass a file path."))
        }
    };
    let label = match get(raw, "label").unwrap_or(&Value::Null) {
        Value::Null => None,
        Value::Str(text) => Some(text),
        _ => return Err(validation("ai.chat attachment label must be a string")),
    };
    if label.is_some_and(|text| chars(text) > 500) {
        return Err(validation("ai.chat attachment label is too long")
            .with_details(dict([("max_length", Value::Int(500.into()))])));
    }
    let label = label
        .map(kassiber_pytext::strip_py)
        .filter(|text| text.code_points().next().is_some());
    Ok(Some(dict([
        ("token", Value::Str(kassiber_pytext::strip_py(token))),
        ("label", label.map_or(Value::Null, Value::Str)),
    ])))
}

/// Recognised notices of seeded assistant answers, by message index.
pub type SeedNotices = Vec<(usize, Vec<&'static str>)>;

/// `_ai_chat_seed_notices(raw, messages)`: storage only; the model never
/// sees these.
pub fn seed_notices(raw: Option<&Value>, messages: &[Value]) -> Result<SeedNotices, AppError> {
    let raw = match raw {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::List(items)) => items,
        Some(_) => return Err(validation("ai.chat seed_notices must be an array")),
    };
    let last_user = messages
        .iter()
        .rposition(|message| str_is(get(message, "role"), &["user"]))
        .map_or(-1, |index| index as i64);
    let mut result: SeedNotices = Vec::new();
    for (position, entry) in raw.iter().enumerate() {
        let (index, notices) = match entry {
            Value::Dict(_) => (get(entry, "index"), get(entry, "notices")),
            _ => (None, None),
        };
        let index = match index {
            Some(Value::Int(n)) => i64::try_from(n).ok(),
            _ => None,
        }
        .filter(|n| 0 <= *n && *n < last_user)
        .map(|n| n as usize)
        .filter(|n| str_is(get(&messages[*n], "role"), &["assistant"]));
        let phases = match notices {
            Some(Value::List(items)) if items.iter().all(|item| matches!(item, Value::Str(_))) => {
                Some(items)
            }
            _ => None,
        };
        let (Some(index), Some(phases)) = (index, phases) else {
            return Err(validation(format!(
                "ai.chat seed_notices[{position}] must name a seeded assistant message index and a list of notice phases"
            )));
        };
        let known: Vec<&'static str> = PROVIDER_STATUS_NOTICES
            .iter()
            .copied()
            .filter(|phase| phases.iter().any(|item| str_is(Some(item), &[phase])))
            .collect();
        if !known.is_empty() {
            match result.iter_mut().find(|(existing, _)| *existing == index) {
                Some((_, slot)) => *slot = known,
                None => result.push((index, known)),
            }
        }
    }
    Ok(result)
}

/// The validated `ai.chat` request.
#[derive(Clone, Debug)]
pub struct Validated {
    /// A string or absent.
    pub provider: Option<Value>,
    pub model: PyStr,
    pub messages: Vec<Value>,
    /// A dict.
    pub options: Value,
    pub tools_enabled: bool,
    pub tool_profile: &'static str,
    pub timeout: f64,
    pub tool_loop_max_iterations: i64,
    pub system_prompt_kind: Option<PromptKind>,
    pub system_prompt: Option<Value>,
    pub session_id: Option<PyStr>,
    /// `None`, `True`, `False` or `"auto"`.
    pub persist: Value,
    pub seed_history: bool,
    pub seed_notices: SeedNotices,
    pub screen_context: Option<Value>,
    pub attachment: Option<Value>,
    pub desktop_secret_store_bridge: Value,
}

impl Validated {
    /// The fields `_ai_answer_provenance` reads from `validated`.
    pub fn provenance_view(&self) -> Value {
        dict([
            ("model", Value::Str(self.model.clone())),
            (
                "screen_context",
                self.screen_context.clone().unwrap_or(Value::Null),
            ),
            ("persist", self.persist.clone()),
        ])
    }

    /// `_ai_chat_keeps_provider_session(validated)`: an Incognito turn
    /// neither resumes nor records a native provider session.
    pub fn keeps_provider_session(&self) -> bool {
        get(&self.options, "stateless_session") != Some(&Value::Bool(true))
    }
}

/// `float(value)` as `_ai_chat_args` applies it to `timeout_seconds`.
fn python_float(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(flag) => Some(f64::from(u8::from(*flag))),
        Value::Int(number) => number.to_string().parse::<f64>().ok(),
        Value::Float(number) => Some(*number),
        Value::Str(text) => kassiber_pytext::parse_float(&crate::pyval::lossy(text)).ok(),
        _ => None,
    }
}

/// `int(value)` as `_ai_chat_args` applies it to `tool_loop_max_iterations`:
/// `Ok(None)` is the `TypeError`/`ValueError` Python turns into a validation
/// error; an infinite float raises `OverflowError`, which escapes as an
/// internal error.
fn python_int(value: &Value) -> Result<Option<BigInt>, AppError> {
    Ok(match value {
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
        Value::Str(text) => kassiber_pytext::parse_int(&crate::pyval::lossy(text)).ok(),
        _ => None,
    })
}

/// `_ai_chat_args(args)`.
pub fn validate(args: &Value) -> Result<Validated, AppError> {
    let model = match get(args, "model") {
        Some(Value::Str(model)) if !is_blank(model) => kassiber_pytext::strip_py(model),
        _ => {
            return Err(validation("ai.chat requires a non-empty model")
                .with_hint("Pass {model: '<id>', messages: [...]}."))
        }
    };
    let raw_messages = match get(args, "messages") {
        Some(Value::List(items)) if !items.is_empty() => items,
        _ => return Err(validation("ai.chat requires a non-empty messages array")),
    };
    let mut messages = Vec::with_capacity(raw_messages.len());
    for (index, raw) in raw_messages.iter().enumerate() {
        if !matches!(raw, Value::Dict(_)) {
            return Err(validation(format!(
                "ai.chat messages[{index}] must be an object"
            )));
        }
        let role = get(raw, "role");
        if !str_is(role, &["system", "user", "assistant", "tool"]) {
            return Err(validation(format!(
                "ai.chat messages[{index}].role must be system | user | assistant | tool"
            )));
        }
        let role = role.cloned().unwrap_or(Value::Null);
        let content = match get(raw, "content") {
            Some(content @ Value::Str(_)) => content.clone(),
            _ => {
                return Err(validation(format!(
                    "ai.chat messages[{index}].content must be a string"
                )))
            }
        };
        let mut cleaned = vec![
            (Key::from("role"), role.clone()),
            (Key::from("content"), content),
        ];
        if let Some(tool_call_id) = present(get(raw, "tool_call_id")) {
            if str_is(Some(&role), &["tool"]) {
                if !matches!(tool_call_id, Value::Str(id) if id.code_points().next().is_some()) {
                    return Err(validation(format!(
                        "ai.chat messages[{index}].tool_call_id must be a string"
                    )));
                }
                cleaned.push((Key::from("tool_call_id"), tool_call_id.clone()));
            }
        }
        if let Some(tool_calls) = present(get(raw, "tool_calls")) {
            if str_is(Some(&role), &["assistant"]) {
                let Value::List(items) = tool_calls else {
                    return Err(validation(format!(
                        "ai.chat messages[{index}].tool_calls must be an array"
                    )));
                };
                cleaned.push((
                    Key::from("tool_calls"),
                    Value::List(
                        items
                            .iter()
                            .filter(|item| matches!(item, Value::Dict(_)))
                            .cloned()
                            .collect(),
                    ),
                ));
            }
        }
        messages.push(Value::Dict(cleaned));
    }
    let options = present(get(args, "options"));
    if options.is_some_and(|options| !matches!(options, Value::Dict(_))) {
        return Err(validation("ai.chat options must be an object"));
    }
    if let Some(options) = options {
        if get(options, "fast_mode").is_some_and(|v| !matches!(v, Value::Bool(_))) {
            return Err(validation("ai.chat options.fast_mode must be a boolean"));
        }
        if get(options, "stateless_session").is_some_and(|v| !matches!(v, Value::Bool(_))) {
            return Err(validation(
                "ai.chat options.stateless_session must be a boolean",
            ));
        }
    }
    let provider = present(get(args, "provider"));
    if provider.is_some_and(|v| !matches!(v, Value::Str(_))) {
        return Err(validation("ai.chat provider must be a string"));
    }
    let tools_enabled = match get(args, "tools_enabled") {
        None => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err(validation("ai.chat tools_enabled must be a boolean")),
    };
    let tool_profile = match get(args, "tool_profile") {
        None => "scoped",
        Some(Value::Str(text)) if TOOL_PROFILE_NAMES.iter().any(|name| text.is(name)) => {
            TOOL_PROFILE_NAMES
                .iter()
                .copied()
                .find(|name| text.is(name))
                .unwrap_or("scoped")
        }
        Some(other) => {
            return Err(
                validation("ai.chat tool_profile must be core, scoped, or full").with_details(
                    dict([
                        ("tool_profile", other.clone()),
                        ("supported", names(&TOOL_PROFILE_NAMES)),
                    ]),
                ),
            )
        }
    };
    let default_timeout = Value::Int(BigInt::from(120));
    let raw_timeout = get(args, "timeout_seconds").unwrap_or(&default_timeout);
    let timeout_details = || dict([("timeout_seconds", raw_timeout.clone())]);
    let Some(timeout) = python_float(raw_timeout) else {
        return Err(
            validation("ai.chat timeout_seconds must be a number").with_details(timeout_details())
        );
    };
    if !timeout.is_finite() || timeout <= 0.0 || timeout > 3600.0 {
        return Err(
            validation("ai.chat timeout_seconds must be greater than 0 and at most 3600")
                .with_details(timeout_details()),
        );
    }
    let default_limit = Value::Int(BigInt::from(8));
    let raw_limit = get(args, "tool_loop_max_iterations").unwrap_or(&default_limit);
    let Some(limit) = python_int(raw_limit)? else {
        return Err(validation(
            "ai.chat tool_loop_max_iterations must be an integer",
        ));
    };
    if limit < BigInt::from(1) || limit > BigInt::from(32) {
        return Err(validation(
            "ai.chat tool_loop_max_iterations must be between 1 and 32",
        ));
    }
    let mut tool_loop_max_iterations = i64::try_from(&limit).unwrap_or(8);
    let system_prompt_kind =
        normalize_system_prompt_kind(get(args, "system_prompt_kind"), tools_enabled)?;
    let system_prompt = present(get(args, "system_prompt"));
    if system_prompt.is_some_and(|v| !matches!(v, Value::Str(_))) {
        return Err(validation("ai.chat system_prompt must be a string"));
    }
    if system_prompt.is_some() && system_prompt_kind != Some(PromptKind::Raw) {
        return Err(validation(
            "ai.chat system_prompt is only accepted when system_prompt_kind is raw",
        ));
    }
    let session_id = match get(args, "session_id") {
        None | Some(Value::Null) => None,
        Some(Value::Str(id)) if id.code_points().next().is_some() => Some(id.clone()),
        Some(_) => return Err(validation("ai.chat session_id must be a non-empty string")),
    };
    let persist = get(args, "persist").cloned().unwrap_or(Value::Null);
    let persist_ok = matches!(persist, Value::Null | Value::Bool(_))
        || matches!(&persist, Value::Str(text) if text.is("auto"));
    if !persist_ok {
        return Err(validation(
            "ai.chat persist must be true, false, or \"auto\"",
        ));
    }
    let mut options = options
        .filter(|options| kassiber_envelope::truthy(options))
        .cloned()
        .unwrap_or_else(|| Value::Dict(Vec::new()));
    if persist == Value::Bool(false) {
        // Incognito: a CLI provider is asked not to save its own session,
        // and no provider session cursor is kept.
        set(&mut options, "stateless_session", Value::Bool(true));
    }
    let seed_history = match get(args, "seed_history") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err(validation("ai.chat seed_history must be a boolean")),
    };
    let seed_notices = seed_notices(get(args, "seed_notices"), &messages)?;
    let screen_context = screen_context(get(args, "screen_context"))?;
    let attachment = attachment(get(args, "attachment"))?;
    if get(args, "tool_loop_max_iterations").is_none() && tools_enabled {
        let selected = select_tool_capabilities(Some(&messages), screen_context.as_ref());
        if selected.is_some_and(|set| set.contains("review") || set.contains("source_funds")) {
            tool_loop_max_iterations = 16;
        }
    }
    Ok(Validated {
        provider: provider.cloned(),
        model,
        messages,
        options,
        tools_enabled,
        tool_profile,
        timeout,
        tool_loop_max_iterations,
        system_prompt_kind,
        system_prompt: system_prompt.cloned(),
        session_id,
        persist,
        seed_history,
        seed_notices,
        screen_context,
        attachment,
        desktop_secret_store_bridge: get(args, "_desktop_secret_store_bridge")
            .cloned()
            .unwrap_or(Value::Null),
    })
}

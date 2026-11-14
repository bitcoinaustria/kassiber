//! The Codex adapter (`codex.ts`): `codex app-server --stdio`, spoken as
//! JSON-RPC lines ([`connection`]).
//!
//! Kassiber's tools reach Codex as dynamic tools: the app-server asks the
//! broker to run them (`item/tool/call`) and the broker answers through the
//! [`ToolBridge`]. Every other server request is refused, and the turn fails
//! on the first item that is not text, reasoning or a dynamic tool call.
//! Selected-data turns run under a restricted permission profile installed
//! at startup and verified on the opened thread before anything is sent;
//! Incognito turns use an ephemeral thread. Fast mode is a one-turn service
//! tier, sent only to app-servers that take `serviceTierForTurn` and only
//! when the catalog advertises it for the thread's model.

mod connection;
mod wire;

pub(crate) use self::connection::for_each_line;

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use self::connection::{denied, tool_reply, Connection, ToolMessage, TurnEnd};
use self::wire::Json;
use crate::executables::resolve_executable;
use crate::prompt::{
    prompt_from_messages, prompt_images, system_instructions, CHAT_ONLY_INSTRUCTIONS,
};
use crate::protocol::{
    first_utf16_units, js_space, js_trim, minify_json, provider_status, requested_fast_mode,
    safe_error_message, safe_session_cursor, sensitive_context, stateless_session, truthy,
    BrokerFailure, BrokerModel, BrokerToolDefinition, ChatRequest, Event, ProviderId,
    ProviderState, ProviderStatus,
};
use crate::{Adapter, EventSink, ToolBridge};

/// The adapter [`crate::adapter`] returns for `codex`.
pub static CODEX: Codex = Codex {
    fast_tier_lookup: FAST_TIER_LOOKUP,
};

/// The Codex runtime. `fast_tier_lookup` is how long a fast turn waits for
/// `model/list` before answering at standard speed (`fastTierLookup`).
pub struct Codex {
    pub fast_tier_lookup: Duration,
}

/// `fastTierLookup.timeoutMs`.
pub const FAST_TIER_LOOKUP: Duration = Duration::from_secs(5);

const DISPLAY_NAME: &str = "Codex";
const EXECUTABLE: &str = "codex";

/// The permission profile selected-data turns run under.
pub const SENSITIVE_CODEX_PROFILE: &str = "kassiber-selected-context";

/// Codex's usual id for the Fast tier; the catalog's own id is what is sent.
pub const CODEX_FAST_SERVICE_TIER: &str = "priority";
/// The deprecated `additionalSpeedTiers` spelling, and that era's fast tier.
pub const CODEX_LEGACY_FAST_SERVICE_TIER: &str = "fast";
/// The app-server's id for standard speed ("Use \"default\" for standard
/// speed").
pub const CODEX_STANDARD_SERVICE_TIER: &str = "default";

/// Items a turn may start; anything else is a provider-native tool.
pub const CODEX_NON_TOOL_ITEM_TYPES: [&str; 4] = [
    "userMessage",
    "agentMessage",
    "reasoning",
    "dynamicToolCall",
];

/// The Codex features selected-data turns switch off.
const SENSITIVE_DISABLED_FEATURES: [&str; 26] = [
    "shell_tool",
    "unified_exec",
    "shell_snapshot",
    "code_mode_host",
    "code_mode",
    "code_mode_only",
    "multi_agent",
    "multi_agent_v2",
    "browser_use",
    "browser_use_external",
    "computer_use",
    "apps",
    "plugins",
    "hooks",
    "memories",
    "remote_plugin",
    "image_generation",
    "in_app_browser",
    "workspace_dependencies",
    "skill_search",
    "tool_suggest",
    "skill_mcp_dependency_install",
    "request_permissions_tool",
    "external_agent_memory_import",
    "chronicle",
    "goals",
];

/// `sensitiveCodexConfig()`: the startup configuration of a selected-data
/// turn, in `codex.ts`'s key order.
pub fn sensitive_codex_config() -> Json {
    Json::obj()
        .with("project_doc_max_bytes", 0)
        .with("default_permissions", SENSITIVE_CODEX_PROFILE)
        .with(
            "permissions",
            Json::obj().with(
                SENSITIVE_CODEX_PROFILE,
                Json::obj()
                    .with(
                        "filesystem",
                        Json::obj()
                            .with(":root", "deny")
                            .with(":minimal", "read")
                            .with(":workspace_roots", "read"),
                    )
                    .with("network", Json::obj().with("enabled", false)),
            ),
        )
        .with("history", Json::obj().with("persistence", "none"))
        .with("analytics", Json::obj().with("enabled", false))
        .with("feedback", Json::obj().with("enabled", false))
        .with(
            "otel",
            Json::obj()
                .with("exporter", "none")
                .with("log_user_prompt", false),
        )
        .with(
            "skills",
            Json::obj()
                .with("include_instructions", false)
                .with("config", Vec::new()),
        )
        .with(
            "shell_environment_policy",
            Json::obj()
                .with("inherit", "none")
                .with("set", Json::obj())
                .with("experimental_use_profile", false),
        )
        .with("allow_login_shell", false)
        .with("web_search", "disabled")
        .with(
            "tools",
            Json::obj()
                .with("view_image", false)
                .with("web_search", false),
        )
        .with("mcp_servers", Json::obj())
        .with(
            "features",
            SENSITIVE_DISABLED_FEATURES
                .iter()
                .fold(Json::obj(), |features, name| features.with(name, false)),
        )
}

/// `sensitiveCodexArgs()`: the configuration as `-c key=<toml>` pairs.
pub fn sensitive_codex_args() -> Vec<String> {
    let Json::Obj(entries) = sensitive_codex_config() else {
        return Vec::new();
    };
    entries
        .into_iter()
        .flat_map(|(key, value)| ["-c".to_owned(), format!("{key}={}", value.to_toml())])
        .collect()
}

/// `codexFailureMessage(error)`: only protocol categories and bounded HTTP
/// status codes leave the adapter.
pub fn codex_failure_message(error: &Value) -> String {
    let info = if error.is_object() || error.is_array() {
        error.get("codexErrorInfo")
    } else {
        None
    };
    let fixed = match info.and_then(Value::as_str) {
        Some("unauthorized") => {
            Some("Codex requires authentication. Run `codex login` outside Kassiber.")
        }
        Some("usageLimitExceeded" | "sessionBudgetExceeded") => {
            Some("Codex reported that the account's usage limit was reached.")
        }
        Some("rateLimitExceeded") => Some("Codex is rate limited. Try again later."),
        Some("contextWindowExceeded") => Some("Codex reached the context limit. Start a new chat."),
        Some("modelNotFound") => {
            Some("Codex could not find the selected model. Check models and choose another.")
        }
        Some("badRequest") => {
            Some("Codex rejected the request. Check the selected model and reasoning level.")
        }
        Some("serverOverloaded" | "internalServerError") => {
            Some("Codex reported a provider server error. Try again later.")
        }
        Some("sandboxError") => Some("Codex could not start its restricted execution environment."),
        _ => None,
    };
    if let Some(fixed) = fixed {
        return fixed.to_owned();
    }
    if let Some(info) = info.filter(|info| info.is_object() || info.is_array()) {
        for category in [
            "httpConnectionFailed",
            "responseStreamConnectionFailed",
            "responseStreamDisconnected",
            "responseTooManyFailedAttempts",
        ] {
            let Some(detail) = info
                .get(category)
                .filter(|detail| detail.is_object() || detail.is_array())
            else {
                continue;
            };
            let status = detail
                .get("httpStatusCode")
                .and_then(Value::as_f64)
                .filter(|status| status.fract() == 0.0 && (400.0..=599.0).contains(status));
            return match status {
                Some(status) => {
                    format!("Codex could not connect to the provider (HTTP {status}).")
                }
                None => "Codex lost its connection to the provider. Try again.".to_owned(),
            };
        }
    }
    "Codex reported a provider error.".to_owned()
}

/// The first Codex release whose `turn/start` takes `serviceTierForTurn`.
const PER_TURN_TIER_SINCE: [f64; 3] = [0.0, 151.0, 0.0];

/// A run of ASCII digits starting at `start`, as JavaScript's `Number`
/// reads it, and where it ends.
fn digits(text: &[u8], start: usize) -> Option<(f64, usize)> {
    let end = start
        + text[start.min(text.len())..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
    if end == start {
        return None;
    }
    let number = std::str::from_utf8(&text[start..end]).ok()?.parse().ok()?;
    Some((number, end))
}

/// `major.minor.patch` at `start` (`(\d+)\.(\d+)\.(\d+)`), and its end.
pub(crate) fn version_at(text: &[u8], start: usize) -> Option<([f64; 3], usize)> {
    let (major, end) = digits(text, start)?;
    if text.get(end) != Some(&b'.') {
        return None;
    }
    let (minor, end) = digits(text, end + 1)?;
    if text.get(end) != Some(&b'.') {
        return None;
    }
    let (patch, end) = digits(text, end + 1)?;
    Some(([major, minor, patch], end))
}

/// `supportsPerTurnServiceTier(userAgent)`: whether the app-server's
/// version, from its `userAgent` (`<originator>/<version> (...)`), takes
/// `serviceTierForTurn`. Older servers ignore that unknown field, so an
/// unparseable version (including source builds' 0.0.0) counts as older,
/// as does a prerelease of the first supporting version.
pub fn supports_per_turn_service_tier(user_agent: Option<&str>) -> bool {
    let agent = js_trim(user_agent.unwrap_or_default());
    // `^[^/\s]+\/`
    let Some(slash) = agent.find('/') else {
        return false;
    };
    if slash == 0 || agent[..slash].chars().any(js_space) {
        return false;
    }
    let Some((found, end)) = version_at(agent.as_bytes(), slash + 1) else {
        return false;
    };
    for (found, since) in found.iter().zip(PER_TURN_TIER_SINCE) {
        if *found != since {
            return *found > since;
        }
    }
    agent.as_bytes().get(end) != Some(&b'-')
}

/// `fastServiceTier(model)`: the model's fast tier from `model/list`: a
/// `serviceTiers` entry with the `priority` id or named "Fast", or the
/// deprecated `additionalSpeedTiers: ["fast"]` when an older app-server
/// sends only that. Tier ids are the backend's, so the advertised one is
/// what `turn/start` sends.
pub fn fast_service_tier(model: &Value) -> Option<(String, Option<String>)> {
    if let Some(Value::Array(tiers)) = model.get("serviceTiers") {
        for tier in tiers {
            let Some(id) = tier.get("id").and_then(Value::as_str) else {
                continue;
            };
            if id.is_empty() {
                continue;
            }
            let named_fast = tier
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case("fast"));
            if id == CODEX_FAST_SERVICE_TIER || named_fast {
                let description = tier
                    .get("description")
                    .and_then(Value::as_str)
                    .map(js_trim)
                    .filter(|description| !description.is_empty())
                    .map(|description| first_utf16_units(description, 120).to_owned());
                return Some((id.to_owned(), description));
            }
        }
    }
    let legacy = match model.get("additionalSpeedTiers") {
        Some(Value::Array(tiers)) => tiers
            .iter()
            .any(|tier| tier.as_str() == Some(CODEX_LEGACY_FAST_SERVICE_TIER)),
        _ => false,
    };
    legacy.then(|| (CODEX_LEGACY_FAST_SERVICE_TIER.to_owned(), None))
}

/// JavaScript's `String(value)` (`None` is `undefined`).
pub(crate) fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(Value::Null) => "null".to_owned(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Number(number)) => match (number.as_i64(), number.as_u64(), number.as_f64()) {
            (Some(int), _, _) => int.to_string(),
            (_, Some(int), _) => int.to_string(),
            (_, _, Some(float)) if float.fract() == 0.0 && float.abs() < 1e21 => {
                format!("{float:.0}")
            }
            _ => number.to_string(),
        },
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                item => js_string(Some(item)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_owned(),
    }
}

/// One page of `model/list`: its entries and the next cursor.
fn catalog_page(response: &Value) -> (Vec<Value>, Option<String>) {
    let entries = match response.get("data") {
        Some(Value::Array(entries)) => entries.clone(),
        _ => Vec::new(),
    };
    let cursor = response
        .get("nextCursor")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .map(str::to_owned);
    (entries, cursor)
}

fn cursor_params(cursor: Option<&str>) -> Json {
    match cursor {
        Some(cursor) => Json::obj().with("cursor", cursor),
        None => Json::obj(),
    }
}

/// `modelFastTier(connection, model)` within `deadline`: the tier id
/// `model/list` advertises as Fast for `model` (matched by id or model
/// slug); without a model id the catalog's `isDefault` entry stands in.
/// `None` when there is none, the lookup fails, or the deadline passes.
fn model_fast_tier(
    connection: &Connection,
    model: Option<&str>,
    deadline: Instant,
) -> Option<String> {
    let mut cursor: Option<String> = None;
    loop {
        let answer = connection.request_async("model/list", cursor_params(cursor.as_deref()));
        let left = deadline.saturating_duration_since(Instant::now());
        let response = answer.recv_timeout(left).ok()?.ok()?;
        let (entries, next) = catalog_page(&response);
        for entry in entries {
            // A null entry throws in TypeScript, which ends the lookup.
            if entry.is_null() {
                return None;
            }
            let matches = match model {
                Some(model) => {
                    entry.get("id").and_then(Value::as_str) == Some(model)
                        || entry.get("model").and_then(Value::as_str) == Some(model)
                }
                None => entry.get("isDefault") == Some(&Value::Bool(true)),
            };
            if matches {
                return fast_service_tier(&entry).map(|(id, _)| id);
            }
        }
        cursor = Some(next?);
    }
}

/// `loadModels(connection, perTurn)`: the model catalog. Fast mode is
/// advertised only where a turn can use it, on app-servers that take the
/// one-turn `serviceTierForTurn`.
fn load_models(connection: &Connection, per_turn: bool) -> Result<Vec<BrokerModel>, BrokerFailure> {
    let mut rows = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let response = connection.request("model/list", cursor_params(cursor.as_deref()))?;
        let (entries, next) = catalog_page(&response);
        for model in entries {
            if model.is_null() {
                return Err(BrokerFailure::new(
                    "Codex returned an unreadable model list.",
                ));
            }
            let efforts: Vec<String> = match model.get("supportedReasoningEfforts") {
                Some(Value::Array(entries)) => entries
                    .iter()
                    .map(|entry| match entry {
                        Value::String(effort) => effort.clone(),
                        Value::Object(_) | Value::Array(_) => match entry.get("reasoningEffort") {
                            None | Some(Value::Null) => String::new(),
                            effort => js_string(effort),
                        },
                        _ => String::new(),
                    })
                    .filter(|effort| !effort.is_empty())
                    .collect(),
                _ => Vec::new(),
            };
            let fast = if per_turn {
                fast_service_tier(&model)
            } else {
                None
            };
            let id = match model.get("id") {
                Some(id) if truthy(id) => js_string(Some(id)),
                _ => js_string(model.get("model")),
            };
            rows.push(BrokerModel {
                id,
                display_name: model
                    .get("displayName")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                owned_by: Some("OpenAI Codex".to_owned()),
                supports_reasoning_effort: Some(!efforts.is_empty()),
                reasoning_efforts: Some(efforts),
                supports_fast_mode: fast.as_ref().map(|_| true),
                fast_mode_description: fast.and_then(|(_, description)| description),
                ..BrokerModel::default()
            });
        }
        match next {
            Some(next) => cursor = Some(next),
            None => return Ok(rows),
        }
    }
}

/// `initialize(connection)`: the app-server's `userAgent`, if it sent one.
fn initialize(connection: &Connection) -> Result<Option<String>, BrokerFailure> {
    let response = connection.request(
        "initialize",
        Json::obj()
            .with(
                "clientInfo",
                Json::obj()
                    .with("name", "kassiber")
                    .with("title", "Kassiber")
                    .with("version", "0.22.55"),
            )
            .with("capabilities", Json::obj().with("experimentalApi", true)),
    )?;
    connection.notify("initialized");
    Ok(response
        .get("userAgent")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

/// The tier fields of `turn/start`.
#[derive(Clone, Debug, PartialEq, Eq)]
enum TurnTier {
    /// `serviceTierForTurn`: this turn only.
    Turn(String),
    /// `serviceTier`: standard speed on a server without the per-turn
    /// field; `None` is the legacy retry's `null`.
    Thread(Option<String>),
}

fn fast_mode_unavailable(events: &EventSink) {
    events.write(&Event::Status {
        phase: "fast_mode_unavailable".into(),
        message: "Fast mode unavailable; answering at standard speed".into(),
    });
}

/// `toolFingerprint(request)`: the first 16 hex digits of the SHA-256 of
/// the advertised tools as JSON, so a cursor is only resumed with the
/// catalog it was opened with.
fn tool_fingerprint(tools: &[BrokerToolDefinition]) -> String {
    let normalized: Vec<BrokerToolDefinition> = tools
        .iter()
        .map(|tool| BrokerToolDefinition {
            parameters: minify_json(&tool.parameters),
            ..tool.clone()
        })
        .collect();
    let encoded = serde_json::to_string(&normalized).unwrap_or_default();
    Sha256::digest(encoded.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `/unknown variant\W+default\b/i`: how a legacy app-server's
/// deserializer rejects the `"default"` tier id.
fn legacy_default_tier_rejected(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut from = 0;
    while let Some(found) = lower[from..].find("unknown variant") {
        let mut at = from + found + "unknown variant".len();
        let gap_start = at;
        let bytes = lower.as_bytes();
        while at < bytes.len() && !word(bytes[at]) {
            at += 1;
        }
        if at > gap_start
            && lower[at..].starts_with("default")
            && !bytes.get(at + "default".len()).copied().is_some_and(word)
        {
            return true;
        }
        from += found + 1;
    }
    false
}

/// `/needle|.../i` over ASCII words.
fn mentions(message: &str, needles: &[&str]) -> bool {
    let lower = message.to_ascii_lowercase();
    needles.iter().any(|needle| lower.contains(needle))
}

impl Codex {
    /// `codexStatus(cwd)` with a resolved executable.
    pub fn status_with(&self, executable: &Path, cwd: &Path) -> ProviderStatus {
        let probe = || -> Result<Vec<BrokerModel>, BrokerFailure> {
            let connection = Connection::start(executable, cwd, &[], false, None)?;
            let per_turn = supports_per_turn_service_tier(initialize(&connection)?.as_deref());
            load_models(&connection, per_turn)
        };
        let executable = executable.to_string_lossy();
        match probe() {
            Ok(models) => {
                let ready = !models.is_empty();
                provider_status(
                    ProviderId::Codex,
                    DISPLAY_NAME,
                    if ready {
                        ProviderState::Ready
                    } else {
                        ProviderState::AuthenticationRequired
                    },
                    if ready {
                        "Ready using the existing Codex login."
                    } else {
                        "Run `codex login` outside Kassiber."
                    },
                )
                .with_executable(&executable)
                .with_models(models)
            }
            Err(failure) => {
                let message = safe_error_message(failure.message());
                let needs_login = mentions(&message, &["auth", "login", "unauthor"]);
                provider_status(
                    ProviderId::Codex,
                    DISPLAY_NAME,
                    if needs_login {
                        ProviderState::AuthenticationRequired
                    } else {
                        ProviderState::Error
                    },
                    if needs_login {
                        "Run `codex login` outside Kassiber.".to_owned()
                    } else {
                        message
                    },
                )
                .with_executable(&executable)
            }
        }
    }

    /// `codexChat(request, cwd, toolBridge)` with a resolved executable.
    pub fn chat_with(
        &self,
        executable: &Path,
        request: &ChatRequest,
        cwd: &Path,
        bridge: Option<&ToolBridge>,
        events: &EventSink,
    ) -> Result<(), BrokerFailure> {
        let sensitive = sensitive_context(request)?;
        // Incognito: an ephemeral thread is never written to disk, and
        // Kassiber's tools still work in it; only selected-data turns also
        // drop tools.
        let stateless = stateless_session(request)?;
        std::thread::scope(|scope| {
            let (tools, calls) = match bridge {
                Some(_) => {
                    let (sender, receiver) = mpsc::channel();
                    (Some(sender), Some(receiver))
                }
                None => (None, None),
            };
            let extra = if sensitive {
                sensitive_codex_args()
            } else {
                Vec::new()
            };
            let connection = Connection::start(executable, cwd, &extra, sensitive, tools.clone())?;
            if let (Some(bridge), Some(calls)) = (bridge, calls) {
                let stdin = connection.stdin();
                scope.spawn(move || {
                    for message in calls {
                        let ToolMessage::Call(call) = message else {
                            break;
                        };
                        let stdin = stdin.clone();
                        scope.spawn(move || {
                            let reply = match bridge.request(
                                &call.name,
                                &call.arguments,
                                Some(&call.call_id),
                            ) {
                                Ok(output) => tool_reply(&call.id, Json::Value(output), true),
                                Err(_) => denied(&call.id),
                            };
                            stdin.send(&reply);
                        });
                    }
                });
            }
            let turn = self.turn(&connection, request, cwd, sensitive, stateless, events);
            let turn = match turn {
                Err(_) if sensitive => Err(BrokerFailure::new(
                    "Codex sensitive-context request failed; no provider diagnostic content was retained.",
                )),
                turn => turn,
            };
            connection.close();
            if let Some(tools) = tools {
                let _ = tools.send(ToolMessage::Stop);
            }
            // Calls still waiting on the daemon end with the turn, as the
            // dispatcher's close does right after it; closing here lets
            // their threads finish.
            if let Some(bridge) = bridge {
                bridge.close()?;
            }
            turn
        })
    }

    fn turn(
        &self,
        connection: &Connection,
        request: &ChatRequest,
        cwd: &Path,
        sensitive: bool,
        stateless: bool,
        events: &EventSink,
    ) -> Result<(), BrokerFailure> {
        events.write(&Event::Status {
            phase: "connecting".into(),
            message: "Starting Codex app-server".into(),
        });
        let per_turn = supports_per_turn_service_tier(initialize(connection)?.as_deref());
        let instructions = system_instructions(request);
        let cwd = cwd
            .to_str()
            .ok_or_else(|| BrokerFailure::new("Kassiber working directory is not valid text."))?;
        let mut config = Json::obj()
            .with("web_search", "disabled")
            .with("mcp_servers", Json::obj())
            .with("multi_agent_mode", "explicitRequestOnly");
        if stateless {
            config.set("history", Json::obj().with("persistence", "none"));
        }
        if sensitive {
            config = config.spread(sensitive_codex_config());
        }
        let mut common = Json::obj().with("cwd", cwd);
        if request.model != "default" {
            common.set("model", request.model.as_str());
        }
        common = common
            .with("approvalPolicy", "untrusted")
            .with("approvalsReviewer", "user");
        if sensitive {
            common.set("permissions", SENSITIVE_CODEX_PROFILE);
        } else {
            common.set("sandbox", "read-only");
        }
        common.set("ephemeral", sensitive || stateless);
        if sensitive {
            common.set("environments", Vec::new());
        }
        let chat_instructions = if sensitive {
            CHAT_ONLY_INSTRUCTIONS
        } else {
            instructions.as_str()
        };
        common = common
            .with("baseInstructions", chat_instructions)
            .with("developerInstructions", chat_instructions)
            .with("config", config);
        let dynamic_tools: Vec<Json> = request
            .tools
            .iter()
            .map(|tool| {
                let mut entry = Json::obj()
                    .with("type", "function")
                    .with("name", tool.name.as_str());
                if let Some(description) = &tool.description {
                    entry.set("description", description.as_str());
                }
                entry.with("inputSchema", Json::Raw(minify_json(&tool.parameters)))
            })
            .collect();
        let start_params = common.clone().with("dynamicTools", dynamic_tools);
        let cursor_prefix = format!("kdt-{}:", tool_fingerprint(&request.tools));
        let raw_resume_id = safe_session_cursor(request.option("provider_session_id"));
        let resume_id = match raw_resume_id {
            Some(raw) if raw.starts_with(&cursor_prefix) => {
                Some(raw[cursor_prefix.len()..].to_owned())
            }
            Some(_) if !request.tools.is_empty() => None,
            raw => raw,
        }
        .filter(|id| !id.is_empty());
        let mut resumed = false;
        let opened = match &resume_id {
            Some(resume_id) => {
                match connection.request(
                    "thread/resume",
                    Json::obj()
                        .with("threadId", resume_id.as_str())
                        .spread(common),
                ) {
                    Ok(opened) => {
                        resumed = true;
                        opened
                    }
                    Err(failure) => {
                        if !mentions(
                            &safe_error_message(failure.message()),
                            &["not found", "missing", "unknown"],
                        ) {
                            return Err(failure);
                        }
                        connection.request("thread/start", start_params)?
                    }
                }
            }
            None => connection.request("thread/start", start_params)?,
        };
        let thread = match opened.get("thread") {
            None | Some(Value::Null) => {
                return Err(BrokerFailure::new("Codex did not open a thread."));
            }
            Some(thread) => thread,
        };
        let ephemeral = thread.get("ephemeral") == Some(&Value::Bool(true));
        if sensitive {
            let profile = opened
                .get("activePermissionProfile")
                .and_then(|profile| profile.get("id"))
                .and_then(Value::as_str);
            let no_sources = matches!(opened.get("instructionSources"), Some(Value::Array(sources)) if sources.is_empty());
            if !ephemeral || profile != Some(SENSITIVE_CODEX_PROFILE) || !no_sources {
                return Err(BrokerFailure::new(
                    "Codex could not verify ephemeral restricted execution; no selected data was sent.",
                ));
            }
        }
        if stateless && !ephemeral {
            return Err(BrokerFailure::new(
                "Codex could not open an unsaved thread; nothing was sent.",
            ));
        }
        let thread_id = js_string(thread.get("id"));
        let prompt = format!(
            "{}{}",
            if sensitive {
                format!("{instructions}\n\n")
            } else {
                String::new()
            },
            prompt_from_messages(&request.messages, resumed)
        );
        // Known residual risk: Codex exposes no no-tools profile, so tools are
        // constrained rather than absent. `sandboxPolicy` below sets readOnly
        // with networkAccess: false, and the listener aborts the turn on the
        // first non-text item, but a local read can begin before that abort
        // lands. With the network off, its content can only surface through
        // assistant text on a turn we are already failing. Revisit if Codex
        // ships a tool-free mode.
        connection.on_notification(listener(sensitive, events.clone()));
        let tier = self.turn_tier(
            connection,
            request,
            opened.get("model").and_then(Value::as_str),
            per_turn,
            events,
        )?;
        let mut input = vec![Json::obj().with("type", "text").with("text", prompt)];
        // Inline data URLs only: never a path Codex would read itself.
        for image in prompt_images(&request.messages, resumed)? {
            input.push(Json::obj().with("type", "image").with(
                "url",
                format!("data:{};base64,{}", image.media_type, image.data),
            ));
        }
        let mut params = Json::obj()
            .with("threadId", thread_id.as_str())
            .with("input", input)
            .with("approvalPolicy", "untrusted")
            .with("approvalsReviewer", "user");
        if sensitive {
            params.set("permissions", SENSITIVE_CODEX_PROFILE);
        } else {
            params.set(
                "sandboxPolicy",
                Json::obj()
                    .with("type", "readOnly")
                    .with("networkAccess", false),
            );
        }
        if request.model != "default" {
            params.set("model", request.model.as_str());
        }
        if let Some(effort) = request.option("reasoning_effort") {
            if truthy(effort) && effort.as_str() != Some("auto") {
                params.set("effort", Json::Value(effort.clone()));
            }
        }
        // Always explicit: an omitted tier would inherit a saved
        // `service_tier = "fast"` from the user's Codex config (or an
        // earlier fast turn on a resumed thread) and bill fast without being
        // asked.
        start_turn(connection, params, &tier)?;
        let TurnEnd { status, error } = connection.wait_turn()?;
        if status.as_str() != Some("completed") {
            return Err(BrokerFailure::new(codex_failure_message(&error)));
        }
        events.write(&Event::Done {
            finish_reason: "stop".into(),
            provider_session_id: (!sensitive && !stateless)
                .then(|| format!("{cursor_prefix}{thread_id}")),
        });
        Ok(())
    }

    /// `turnTier(...)`: the service tier for this turn. Fast is sent only
    /// when the turn asked for it, the app-server takes the one-turn
    /// `serviceTierForTurn`, *and* the model the thread really runs
    /// advertises the Fast tier: a CLI `--fast` or a direct `ai.chat` can
    /// name any model, including `default`. The lookup is `model/list` on
    /// this chat's own connection, part of the turn the user started, never
    /// a separate probe. Anything else gets an explicit standard tier, with
    /// a status hint when fast was asked for.
    ///
    /// Fast is never sent as `serviceTier`: that also becomes the thread's
    /// saved tier, so a Kassiber thread later continued in Codex itself
    /// would keep billing fast.
    fn turn_tier(
        &self,
        connection: &Connection,
        request: &ChatRequest,
        thread_model: Option<&str>,
        per_turn: bool,
        events: &EventSink,
    ) -> Result<TurnTier, BrokerFailure> {
        let fast = requested_fast_mode(request)?;
        if !per_turn {
            if fast {
                fast_mode_unavailable(events);
            }
            return Ok(TurnTier::Thread(Some(
                CODEX_STANDARD_SERVICE_TIER.to_owned(),
            )));
        }
        if !fast {
            return Ok(TurnTier::Turn(CODEX_STANDARD_SERVICE_TIER.to_owned()));
        }
        let model = if request.model == "default" {
            thread_model
        } else {
            Some(request.model.as_str())
        };
        // A catalog that never answers must not hold the turn hostage: after
        // a short wait the turn goes ahead at standard speed.
        let deadline = Instant::now() + self.fast_tier_lookup;
        match model_fast_tier(connection, model, deadline).filter(|tier| !tier.is_empty()) {
            Some(tier) => Ok(TurnTier::Turn(tier)),
            None => {
                fast_mode_unavailable(events);
                Ok(TurnTier::Turn(CODEX_STANDARD_SERVICE_TIER.to_owned()))
            }
        }
    }
}

/// `startTurn(connection, params)`. An app-server from before the
/// `"default"` id (its tier was `null | "fast" | "flex"`) rejects that id
/// while deserializing, before any turn starts. Ordinary chat must not break
/// on that, so standard speed is retried once as `null`, which that version
/// reads as "clear the tier". No other tier or error is retried.
fn start_turn(connection: &Connection, params: Json, tier: &TurnTier) -> Result<(), BrokerFailure> {
    let with_tier = |tier: &TurnTier| match tier {
        TurnTier::Turn(id) => params.clone().with("serviceTierForTurn", id.as_str()),
        TurnTier::Thread(Some(id)) => params.clone().with("serviceTier", id.as_str()),
        TurnTier::Thread(None) => params.clone().with("serviceTier", Json::Null),
    };
    match connection.request("turn/start", with_tier(tier)) {
        Ok(_) => Ok(()),
        Err(failure)
            if *tier == TurnTier::Thread(Some(CODEX_STANDARD_SERVICE_TIER.to_owned()))
                && legacy_default_tier_rejected(failure.message()) =>
        {
            connection
                .request("turn/start", with_tier(&TurnTier::Thread(None)))
                .map(drop)
        }
        Err(failure) => Err(failure),
    }
}

/// The turn's notification listener (the `completion` promise in
/// `codex.ts`): stream agent text, fail on provider-native tools and
/// non-retried errors, settle on `turn/completed`.
fn listener(
    sensitive: bool,
    events: EventSink,
) -> impl FnMut(&Map<String, Value>, &connection::Settle) + Send + 'static {
    let mut streamed: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    move |message, settle| {
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params");
        let param = |name: &str| params.and_then(|params| params.get(name));
        let item = param("item").filter(|item| item.is_object() || item.is_array());
        match method {
            Some("item/agentMessage/delta") => {
                if let Some(delta) = param("delta").and_then(Value::as_str) {
                    if delta.is_empty() {
                        return;
                    }
                    if let Some(item_id) = param("itemId").and_then(Value::as_str) {
                        streamed
                            .entry(item_id.to_owned())
                            .or_default()
                            .push_str(delta);
                    }
                    events.write(&Event::Delta {
                        content: Some(delta.to_owned()),
                        reasoning: None,
                    });
                }
            }
            Some("item/completed")
                if item
                    .and_then(|item| item.get("type"))
                    .and_then(Value::as_str)
                    == Some("agentMessage") =>
            {
                let item = item.expect("checked above");
                let already = item
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| streamed.get(id))
                    .map_or("", String::as_str);
                let remainder = item
                    .get("text")
                    .and_then(Value::as_str)
                    .and_then(|text| text.strip_prefix(already))
                    .unwrap_or_default();
                if !remainder.is_empty() {
                    events.write(&Event::Delta {
                        content: Some(remainder.to_owned()),
                        reasoning: None,
                    });
                }
            }
            Some("item/started") if item.is_some() => {
                let item_type = item.and_then(|item| item.get("type"));
                let kind = match item_type {
                    Some(kind) if truthy(kind) => js_string(Some(kind)),
                    _ => String::new(),
                };
                if !CODEX_NON_TOOL_ITEM_TYPES.contains(&kind.as_str())
                    || (sensitive && item_type.and_then(Value::as_str) == Some("dynamicToolCall"))
                {
                    settle.reject(
                        "Codex attempted to use a provider-native tool; Kassiber stopped it.",
                    );
                }
            }
            Some("turn/completed") => {
                let turn = param("turn");
                let status = turn
                    .and_then(|turn| turn.get("status"))
                    .filter(|status| truthy(status))
                    .cloned()
                    .unwrap_or_else(|| Value::String("completed".into()));
                let error = turn
                    .and_then(|turn| turn.get("error"))
                    .cloned()
                    .unwrap_or(Value::Null);
                settle.resolve(TurnEnd { status, error });
            }
            Some("error") => {
                // A retry notification leaves the turn active. Aborting here
                // kills the app-server before it can recover a transient
                // connection.
                if param("willRetry") == Some(&Value::Bool(true)) {
                    events.write(&Event::Status {
                        phase: "retrying".into(),
                        message: "Codex is retrying the request.".into(),
                    });
                } else {
                    settle.reject(&codex_failure_message(
                        param("error").unwrap_or(&Value::Null),
                    ));
                }
            }
            _ => {}
        }
    }
}

impl Adapter for Codex {
    fn status(&self, cwd: &Path) -> Result<ProviderStatus, BrokerFailure> {
        let Some(executable) = resolve_executable(EXECUTABLE) else {
            return Ok(provider_status(
                ProviderId::Codex,
                DISPLAY_NAME,
                ProviderState::MissingExecutable,
                "Install Codex, then run `codex login` outside Kassiber.",
            ));
        };
        Ok(self.status_with(&executable, cwd))
    }

    fn chat(
        &self,
        request: &ChatRequest,
        cwd: &Path,
        bridge: Option<&ToolBridge>,
        events: &EventSink,
    ) -> Result<(), BrokerFailure> {
        let Some(executable) = resolve_executable(EXECUTABLE) else {
            return Err(BrokerFailure::new("Codex is not installed."));
        };
        self.chat_with(&executable, request, cwd, bridge, events)
    }
}

#[cfg(test)]
pub(crate) mod fake_cli;

#[cfg(test)]
mod tests;

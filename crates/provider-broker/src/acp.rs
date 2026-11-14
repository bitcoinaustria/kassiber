//! Agent Client Protocol (<https://agentclientprotocol.com>) adapter
//! (`acp.ts`), and its one agent, GitHub Copilot.
//!
//! ACP gives one wire format for many coding agents, but no portable way to
//! switch off an agent's own shell, file and web tools: the protocol only
//! lets a client answer permission prompts, and agents run some tools
//! without asking. So an agent is listed here only when its CLI can be
//! launched with every built-in tool removed and with none of the user's MCP
//! servers, hooks, plugins, skills, memory or settings loaded. That launch is
//! the boundary; ACP then carries the conversation and the Kassiber MCP
//! server.
//!
//! The session also fails closed behind it:
//!
//! - Kassiber advertises no client file system and no terminal, so `fs/*` and
//!   `terminal/*` requests are refused as unknown methods.
//! - A tool call or permission request is accepted only when its title is
//!   exactly the agent's form for an advertised Kassiber tool and it is not
//!   an `execute` call; anything else stops the agent. Titles can echo
//!   model-chosen text, so this is a tripwire, not the boundary.
//! - Sensitive selected-data requests are refused.
//!
//! The JSON-RPC connection is hand-rolled in [`connection`] (the TypeScript
//! broker used `@agentclientprotocol/sdk`).

mod connection;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::bridge::mcp_command;
use crate::env::provider_environment;
use crate::executables::{resolve_executable, run_provider, RunOptions};
use crate::prompt::{
    images_not_shown_note, prompt_from_messages, prompt_images, system_instructions,
};
use crate::protocol::{
    js_trim, provider_status, safe_error_message, safe_session_cursor, sensitive_context,
    stateless_session, truthy, BrokerFailure, BrokerModel, BrokerToolDefinition, ChatRequest,
    Event, ProviderId, ProviderState, ProviderStatus,
};
use crate::{Adapter, EventSink, ToolBridge};

use connection::{start_agent, AgentProcess, Handler, Reply, RpcError, Shared, Wait};

/// The server name agents see; also the prefix they put on tool titles.
pub const ACP_MCP_SERVER_NAME: &str = "kassiber";

/// ACP major version this client speaks.
const ACP_PROTOCOL_VERSION: f64 = 1.0;

/// Startup budget for `initialize` and `session/new`. The daemon gives a
/// whole status probe 35 seconds before killing the broker, so the agent
/// must be stopped here first, while cleanup can still run.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(25);

/// [`HANDSHAKE_TIMEOUT`], or a shorter budget a test set for its own thread.
fn handshake_timeout() -> Duration {
    #[cfg(all(test, unix))]
    if let Some(budget) = tests::HANDSHAKE_BUDGET.with(std::cell::Cell::get) {
        return budget;
    }
    HANDSHAKE_TIMEOUT
}

/// What a launch needs to know.
pub struct LaunchContext<'a> {
    pub cwd: &'a Path,
    pub request: Option<&'a ChatRequest>,
    /// Command line of the per-turn Kassiber MCP server, when tools are on.
    pub mcp: Option<&'a [String]>,
    /// The provider environment ([`provider_environment`]) the launch
    /// builds on.
    pub env: &'a [(OsString, OsString)],
}

/// How to start the agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentLaunch {
    pub args: Vec<String>,
    pub env: Vec<(OsString, OsString)>,
    /// Pass the Kassiber MCP server in `session/new` instead of the CLI.
    pub session_mcp: bool,
}

/// `AcpAgentSpec`: everything specific to one ACP agent.
pub struct AgentSpec {
    pub id: ProviderId,
    pub display_name: &'static str,
    pub executable: &'static str,
    pub login_hint: &'static str,
    /// Oldest CLI release whose lockdown flags hold in ACP mode. An older or
    /// unreadable version is refused before the agent starts.
    pub minimum_version: Option<[u64; 3]>,
    /// Reasoning levels the CLI accepts, if it has a flag for them.
    pub efforts: &'static [&'static str],
    /// CLI notices the agent sends as ordinary message text.
    pub is_notice: fn(&str) -> bool,
    /// The exact title the agent gives a call to an advertised Kassiber tool.
    pub tool_title: fn(&str) -> String,
    /// Whether a session survives the turn. An agent whose state lives in
    /// the per-turn directory starts fresh each time from the visible
    /// transcript.
    pub persistent_sessions: bool,
    pub launch: fn(&LaunchContext<'_>) -> Result<AgentLaunch, BrokerFailure>,
}

const COPILOT_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// The name an agent gives Kassiber's tool `tool`: `kassiber-<tool>`.
fn copilot_tool_title(tool: &str) -> String {
    format!("{ACP_MCP_SERVER_NAME}-{tool}")
}

/// Sent as `agent_message_chunk` with no marker when the tool allowlist is
/// applied (1.0.93); it describes Kassiber's lockdown, not the answer.
fn copilot_notice(text: &str) -> bool {
    text.starts_with("Info: Disabled tools:")
        || text.starts_with("Info: Unknown tool name in the tool allowlist:")
}

/// `--additional-mcp-config`, in `JSON.stringify`'s key order.
#[derive(Serialize)]
struct McpConfig<'a> {
    #[serde(rename = "mcpServers")]
    mcp_servers: McpServers<'a>,
}

#[derive(Serialize)]
struct McpServers<'a> {
    kassiber: LocalServer<'a>,
}

#[derive(Serialize)]
struct LocalServer<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    command: &'a str,
    args: &'a [String],
    tools: [&'static str; 1],
}

/// Create `path` as a new 0700 directory (`mkdir(path, { mode: 0o700 })`).
fn make_private_dir(path: &Path) -> Result<(), BrokerFailure> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(path)
        .map_err(|error| crate::executables::fs_failure("mkdir", path, &error))
}

/// GitHub Copilot CLI.
///
/// `--available-tools` is an allowlist of what the model may see at all, so
/// shell, file, web, task and skill tools are gone rather than merely denied.
/// Copilot ignores MCP servers passed in `session/new` (verified against
/// 1.0.93: the server is never started), so Kassiber's server goes through
/// `--additional-mcp-config` and its tools are named `kassiber-<tool>`.
///
/// That flag only adds to `~/.copilot/mcp-config.json`, and the same
/// directory holds plugins, settings (including a persisted allow-all) and
/// saved sessions. `COPILOT_HOME` therefore points at an empty directory
/// inside the turn's working directory, so none of it loads. Copilot always
/// saves the session there, conversation included; it is deleted with the
/// directory once Copilot has exited, so only a crash mid-turn leaves it
/// behind. Sign-in still works: Copilot keeps its token in the system
/// keyring or takes `COPILOT_GITHUB_TOKEN` / `GH_TOKEN`; a login stored only
/// in the user's own config file is not found, and status reports a sign-in
/// problem.
fn copilot_launch(context: &LaunchContext<'_>) -> Result<AgentLaunch, BrokerFailure> {
    let home = context.cwd.join("copilot-home");
    make_private_dir(&home)?;
    let mut args: Vec<String> = [
        "--acp",
        "--no-auto-update",
        "--no-custom-instructions",
        "--disable-builtin-mcps",
    ]
    .map(str::to_owned)
    .to_vec();
    if let Some(mcp) = context.mcp {
        let (command, command_args) = match mcp.split_first() {
            Some((command, rest)) => (command.as_str(), rest),
            None => ("", &[][..]),
        };
        let config = McpConfig {
            mcp_servers: McpServers {
                kassiber: LocalServer {
                    kind: "local",
                    command,
                    args: command_args,
                    tools: ["*"],
                },
            },
        };
        args.push("--additional-mcp-config".to_owned());
        args.push(
            serde_json::to_string(&config)
                .map_err(|_| BrokerFailure::new("Kassiber could not encode the MCP config."))?,
        );
    }
    if let Some(request) = context.request {
        if request.model != "default" {
            args.push("--model".to_owned());
            args.push(request.model.clone());
        }
        if let Some(effort) = request.option("reasoning_effort").and_then(Value::as_str) {
            if COPILOT_EFFORTS.contains(&effort) {
                args.push("--reasoning-effort".to_owned());
                args.push(effort.to_owned());
            }
        }
    }
    // Variadic, so it goes last. A name that matches no tool leaves the
    // model with none at all; Copilot only logs that the name is unknown.
    args.push("--available-tools".to_owned());
    let tools = context
        .request
        .map_or(&[][..], |request| &request.tools[..]);
    if context.mcp.is_some() && !tools.is_empty() {
        args.extend(tools.iter().map(|tool| copilot_tool_title(&tool.name)));
    } else {
        args.push(copilot_tool_title("none"));
    }
    let mut env = context.env.to_vec();
    env.push(("COPILOT_HOME".into(), home.into_os_string()));
    env.push(("COPILOT_AUTO_UPDATE".into(), "false".into()));
    Ok(AgentLaunch {
        args,
        env,
        session_mcp: false,
    })
}

/// `COPILOT_AGENT`.
pub static COPILOT_AGENT: AgentSpec = AgentSpec {
    id: ProviderId::Copilot,
    display_name: "GitHub Copilot",
    executable: "copilot",
    login_hint: "Run `copilot login` outside Kassiber.",
    // The first release whose `--available-tools` applies in ACP mode, per
    // Copilot's changelog; older ones hand the model every native tool.
    minimum_version: Some([1, 0, 60]),
    efforts: COPILOT_EFFORTS,
    is_notice: copilot_notice,
    tool_title: copilot_tool_title,
    persistent_sessions: false,
    launch: copilot_launch,
};

/// An ACP agent as a broker [`Adapter`].
pub struct Acp {
    spec: &'static AgentSpec,
}

/// The adapter [`crate::adapter`] returns for `copilot`.
pub static COPILOT: Acp = Acp {
    spec: &COPILOT_AGENT,
};

/// The found CLI and the environment it runs with.
pub struct Runtime {
    pub executable: PathBuf,
    pub env: Vec<(OsString, OsString)>,
}

impl Runtime {
    /// The CLI on `PATH` or in a fallback directory, with this process's
    /// provider environment.
    fn find(spec: &AgentSpec) -> Option<Self> {
        resolve_executable(spec.executable).map(|executable| Self {
            executable,
            env: provider_environment(spec.id),
        })
    }
}

impl Adapter for Acp {
    fn status(&self, cwd: &Path) -> Result<ProviderStatus, BrokerFailure> {
        acp_status(self.spec, Runtime::find(self.spec), cwd)
    }

    fn chat(
        &self,
        request: &ChatRequest,
        cwd: &Path,
        bridge: Option<&ToolBridge>,
        events: &EventSink,
    ) -> Result<(), BrokerFailure> {
        let Some(runtime) = Runtime::find(self.spec) else {
            return Err(BrokerFailure::new(format!(
                "{} is not installed.",
                self.spec.display_name
            )));
        };
        acp_chat(self.spec, &runtime, request, cwd, bridge, events)
    }
}

/// `ToolKind`: a kind outside it reads as absent, as the SDK's schema
/// (`defaultOnError`) treats it.
const TOOL_KINDS: [&str; 10] = [
    "read",
    "edit",
    "delete",
    "move",
    "search",
    "execute",
    "think",
    "fetch",
    "switch_mode",
    "other",
];

/// The parts of an ACP tool call (`ToolCallUpdate`) the tripwire reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCallView {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<String>,
}

impl ToolCallView {
    /// A call's id, and its title and kind when they have the schema's
    /// shape; `None` without a string id.
    fn parse(call: &Value) -> Option<Self> {
        let id = call.get("toolCallId")?.as_str()?.to_owned();
        let title = call.get("title").and_then(Value::as_str).map(str::to_owned);
        let kind = call
            .get("kind")
            .and_then(Value::as_str)
            .filter(|kind| TOOL_KINDS.contains(kind))
            .map(str::to_owned);
        Some(Self { id, title, kind })
    }
}

/// `kassiberToolFor`: the advertised Kassiber tool an ACP tool call is, or
/// `None` for anything else. ACP has no typed MCP field, so this compares
/// the title with the agent's exact form for each advertised tool, with no
/// prefix or suffix tolerance, and never accepts an `execute` call: a shell
/// tool titled with its command line could otherwise spell
/// `kassiber-status`.
pub fn kassiber_tool_for(
    spec: &AgentSpec,
    title: Option<&str>,
    kind: Option<&str>,
    advertised: &[String],
) -> Option<String> {
    if kind == Some("execute") {
        return None;
    }
    let title = js_trim(title?);
    advertised
        .iter()
        .find(|tool| (spec.tool_title)(tool) == title)
        .cloned()
}

/// `modelsFromConfigOptions`: model rows from a session's `model` config
/// option, if the agent has one.
pub fn models_from_config_options(options: Option<&Value>) -> Vec<BrokerModel> {
    let Some(Value::Array(options)) = options else {
        return Vec::new();
    };
    let Some(select) = options.iter().find(|option| {
        option.get("category").and_then(Value::as_str) == Some("model")
            && option.get("type").and_then(Value::as_str) == Some("select")
    }) else {
        return Vec::new();
    };
    let Some(Value::Array(entries)) = select.get("options") else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for entry in entries {
        let Some(object) = entry.as_object() else {
            continue;
        };
        let values: Vec<&Value> = if object.contains_key("group") {
            match object.get("options") {
                Some(Value::Array(values)) => values.iter().collect(),
                _ => Vec::new(),
            }
        } else {
            vec![entry]
        };
        for value in values {
            let Some(id) = value.get("value").and_then(Value::as_str) else {
                continue;
            };
            if js_trim(id).is_empty() {
                continue;
            }
            let name = value
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .unwrap_or(id);
            rows.push(BrokerModel {
                id: id.to_owned(),
                display_name: Some(name.to_owned()),
                ..BrokerModel::default()
            });
        }
    }
    rows
}

fn efforts(spec: &AgentSpec) -> Vec<String> {
    spec.efforts
        .iter()
        .map(|effort| (*effort).to_owned())
        .collect()
}

fn default_model(spec: &AgentSpec) -> BrokerModel {
    BrokerModel {
        id: "default".into(),
        display_name: Some(format!("{} (CLI default)", spec.display_name)),
        supports_reasoning_effort: Some(!spec.efforts.is_empty()),
        reasoning_efforts: (!spec.efforts.is_empty()).then(|| efforts(spec)),
        ..BrokerModel::default()
    }
}

fn with_efforts(spec: &AgentSpec, model: BrokerModel) -> BrokerModel {
    if spec.efforts.is_empty() {
        return model;
    }
    BrokerModel {
        supports_reasoning_effort: Some(true),
        reasoning_efforts: Some(efforts(spec)),
        ..model
    }
}

fn native_tool_attempt(spec: &AgentSpec) -> String {
    format!(
        "{} attempted to use a provider-native tool; Kassiber stopped it.",
        spec.display_name
    )
}

/// `isAuthError`: ACP reserves -32000 for "authentication required";
/// otherwise the message decides (`/auth|log ?in|sign ?in|credential|api
/// key/i`).
fn is_auth_error(code: Option<f64>, message: &str) -> bool {
    if code == Some(-32000.0) {
        return true;
    }
    let lower = message.to_ascii_lowercase();
    [
        "auth",
        "login",
        "log in",
        "signin",
        "sign in",
        "credential",
        "api key",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// One run of ASCII digits at the start of `bytes`, if any.
fn digits(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count()
}

/// `parseVersion`: the first `x.y.z` in a CLI's `--version` output.
pub fn parse_version(text: Option<&str>) -> Option<[f64; 3]> {
    let bytes = text?.as_bytes();
    for start in 0..bytes.len() {
        let mut at = start;
        let mut parts = [0.0; 3];
        let mut matched = true;
        for (index, part) in parts.iter_mut().enumerate() {
            if index > 0 {
                if bytes.get(at) != Some(&b'.') {
                    matched = false;
                    break;
                }
                at += 1;
            }
            let run = digits(&bytes[at..]);
            if run == 0 {
                matched = false;
                break;
            }
            // ASCII digits, so always valid UTF-8 and a valid number.
            *part = std::str::from_utf8(&bytes[at..at + run])
                .ok()
                .and_then(|digits| digits.parse().ok())
                .unwrap_or(f64::INFINITY);
            at += run;
        }
        if matched {
            return Some(parts);
        }
    }
    None
}

/// `versionProblem`: why this CLI version cannot be used, or `None` when it
/// can.
pub fn version_problem(spec: &AgentSpec, version: Option<&str>) -> Option<String> {
    let minimum = spec.minimum_version?.map(|part| part as f64);
    let found = parse_version(version);
    let too_old = match found {
        None => true,
        Some(found) => {
            found[0] < minimum[0]
                || (found[0] == minimum[0]
                    && (found[1] < minimum[1] || (found[1] == minimum[1] && found[2] < minimum[2])))
        }
    };
    if !too_old {
        return None;
    }
    let join = |parts: [f64; 3]| {
        parts
            .iter()
            .map(|part| format!("{part}"))
            .collect::<Vec<_>>()
            .join(".")
    };
    Some(format!(
        "Kassiber needs {} CLI {} or newer to switch off its own tools; {}. Update it outside Kassiber.",
        spec.display_name,
        join(minimum),
        match found {
            Some(found) => format!("{} is installed", join(found)),
            None => "the installed version could not be read".to_owned(),
        }
    ))
}

/// The first `units` UTF-16 code units of `text` (`slice(0, units)`).
fn utf16_prefix(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (index, c) in text.char_indices() {
        used += c.len_utf16();
        if used > units {
            return &text[..index];
        }
    }
    text
}

/// `cliVersion`: the first line of `--version`, at most 80 characters.
fn cli_version(spec: &AgentSpec, runtime: &Runtime) -> Option<String> {
    let output = run_provider(
        spec.id,
        &runtime.executable,
        &["--version"],
        RunOptions {
            limit: Some(16_384),
            env: Some(&runtime.env),
            ..RunOptions::default()
        },
    )
    .ok()?;
    let trimmed = js_trim(&output.stdout);
    let first = match trimmed.find('\n') {
        Some(end) => trimmed[..end].strip_suffix('\r').unwrap_or(&trimmed[..end]),
        None => trimmed,
    };
    let first = utf16_prefix(first, 80);
    (!first.is_empty()).then(|| first.to_owned())
}

/// JavaScript's `String(value)` for a JSON value (`undefined` when absent).
fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => match number.as_f64() {
            Some(float) if float.fract() == 0.0 && float.abs() < 1e21 => format!("{float}"),
            _ => number.to_string(),
        },
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
        Some(other) => other.to_string(),
    }
}

/// `initializeParams()`.
const INITIALIZE_PARAMS: &str = r#"{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},"clientInfo":{"name":"kassiber","version":"1"}}"#;

/// `checkProtocol`.
fn check_protocol(spec: &AgentSpec, init: &Value) -> Result<(), Failure> {
    let version = init.get("protocolVersion");
    if version.and_then(Value::as_f64) == Some(ACP_PROTOCOL_VERSION) {
        return Ok(());
    }
    Err(Failure::Message(format!(
        "{} speaks ACP version {}; Kassiber supports version 1.",
        spec.display_name,
        js_string(version)
    )))
}

/// `init.agentCapabilities?.<path>`.
fn capability<'a>(init: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(init.get("agentCapabilities")?, |value, key| value.get(key))
}

/// `McpServer` (stdio), as `mcpServerFor` builds it.
#[derive(Serialize)]
struct McpServer {
    name: &'static str,
    command: String,
    args: Vec<String>,
    env: [(); 0],
}

fn mcp_server_for(command: &[String]) -> Result<McpServer, BrokerFailure> {
    let Some((executable, args)) = command.split_first() else {
        return Err(BrokerFailure::new(
            "Kassiber MCP bridge command is unavailable.",
        ));
    };
    Ok(McpServer {
        name: ACP_MCP_SERVER_NAME,
        command: executable.clone(),
        args: args.to_vec(),
        env: [],
    })
}

#[derive(Serialize)]
struct NewSession<'a> {
    cwd: &'a str,
    #[serde(rename = "mcpServers")]
    mcp_servers: &'a [McpServer],
}

#[derive(Serialize)]
struct LoadSession<'a> {
    #[serde(rename = "sessionId")]
    session_id: &'a str,
    cwd: &'a str,
    #[serde(rename = "mcpServers")]
    mcp_servers: &'a [McpServer],
}

#[derive(Serialize)]
struct Prompt<'a> {
    #[serde(rename = "sessionId")]
    session_id: &'a str,
    prompt: &'a [ContentBlock],
}

/// A prompt `ContentBlock`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        #[serde(rename = "mimeType")]
        mime_type: String,
        data: String,
    },
}

fn encode(value: &impl Serialize) -> Result<String, Failure> {
    serde_json::to_string(value)
        .map_err(|_| Failure::Message("Kassiber could not encode an ACP request.".into()))
}

fn prompt_text(request: &ChatRequest, resumed: bool) -> String {
    let conversation = prompt_from_messages(&request.messages, resumed);
    // ACP has no system prompt field. A resumed session already holds them.
    if resumed {
        return conversation;
    }
    format!(
        "SYSTEM INSTRUCTIONS:\n{}\n\n{conversation}",
        system_instructions(request)
    )
}

/// `promptBlocks`: the text, then the attached images when the agent says
/// it takes images (`promptCapabilities.image`). One that does not is told
/// they were left out, rather than sent blocks it would reject or ignore.
pub fn prompt_blocks(
    request: &ChatRequest,
    resumed: bool,
    accepts_images: bool,
) -> Result<Vec<ContentBlock>, BrokerFailure> {
    let images = prompt_images(&request.messages, resumed)?;
    let text = prompt_text(request, resumed);
    if images.is_empty() {
        return Ok(vec![ContentBlock::Text { text }]);
    }
    if !accepts_images {
        return Ok(vec![ContentBlock::Text {
            text: text + &images_not_shown_note(images.len()),
        }]);
    }
    let mut blocks = vec![ContentBlock::Text { text }];
    blocks.extend(images.into_iter().map(|image| ContentBlock::Image {
        mime_type: image.media_type,
        data: image.data,
    }));
    Ok(blocks)
}

/// Why a status probe or turn failed, before it becomes a message.
#[derive(Debug)]
enum Failure {
    /// An error answer from the agent (its code matters to `isAuthError`).
    Rpc(RpcError),
    /// The tripwire fired.
    Native,
    Message(String),
}

impl Failure {
    fn from_wait(spec: &AgentSpec, agent: &AgentProcess, wait: Wait, timeout: &str) -> Self {
        match wait {
            Wait::Rpc(error) => Failure::Rpc(error),
            Wait::Closed(reason) => Failure::Message(reason),
            Wait::Violation => Failure::Native,
            Wait::Timeout => Failure::Message(format!("{} {timeout}", spec.display_name)),
            Wait::Exited => {
                let stderr = agent.stderr();
                Failure::Message(format!(
                    "{} ended without a terminal response. {}",
                    spec.display_name,
                    safe_error_message(if stderr.is_empty() {
                        "The agent exited."
                    } else {
                        &stderr
                    })
                ))
            }
        }
    }

    fn message(&self, spec: &AgentSpec) -> String {
        match self {
            Failure::Rpc(error) => error.message.clone(),
            Failure::Native => native_tool_attempt(spec),
            Failure::Message(message) => message.clone(),
        }
    }

    fn is_auth(&self) -> bool {
        match self {
            Failure::Rpc(error) => is_auth_error(Some(error.code), &error.message),
            Failure::Native => false,
            Failure::Message(message) => is_auth_error(None, message),
        }
    }
}

/// `RequestPermissionRequest`, as far as Kassiber reads it.
struct PermissionRequest {
    session_id: String,
    call: ToolCallView,
    options: Vec<(String, String)>,
}

const PERMISSION_KINDS: [&str; 4] = ["allow_once", "allow_always", "reject_once", "reject_always"];

impl PermissionRequest {
    /// The SDK's schema check: a string session id, a tool call with a
    /// string id, and options that each have a string id and name and a
    /// known kind. Anything else is answered `Invalid params`.
    fn parse(params: Option<&Value>) -> Option<Self> {
        let params = params?;
        let session_id = params.get("sessionId")?.as_str()?.to_owned();
        let call = ToolCallView::parse(params.get("toolCall").filter(|call| call.is_object())?)?;
        let mut options = Vec::new();
        for option in params.get("options")?.as_array()? {
            let id = option.get("optionId")?.as_str()?;
            option.get("name")?.as_str()?;
            let kind = option.get("kind")?.as_str()?;
            if !PERMISSION_KINDS.contains(&kind) {
                return None;
            }
            options.push((id.to_owned(), kind.to_owned()));
        }
        Some(Self {
            session_id,
            call,
            options,
        })
    }

    fn select(&self, kinds: &[&str]) -> Reply {
        let chosen = kinds.iter().find_map(|kind| {
            self.options
                .iter()
                .find(|(_, option_kind)| option_kind == kind)
        });
        match chosen {
            Some((id, _)) => selected(id),
            None => cancelled(),
        }
    }

    /// `allowOnce`: the "allow once" answer for an advertised Kassiber tool.
    /// Kassiber's own consent still runs when the call reaches the tool
    /// bridge; this only lets the agent hand the call over.
    fn allow_once(&self) -> Reply {
        self.select(&["allow_once"])
    }

    /// `rejectOnce`.
    fn reject_once(&self) -> Reply {
        self.select(&["reject_once", "reject_always"])
    }
}

fn selected(option_id: &str) -> Reply {
    let id = serde_json::to_string(option_id).unwrap_or_else(|_| "\"\"".into());
    Reply::Result(format!(
        r#"{{"outcome":{{"outcome":"selected","optionId":{id}}}}}"#
    ))
}

fn cancelled() -> Reply {
    Reply::Result(r#"{"outcome":{"outcome":"cancelled"}}"#.into())
}

fn invalid_params() -> Reply {
    Reply::Error {
        code: -32602,
        message: "Invalid params".into(),
        data: None,
    }
}

/// The client during a status probe: every permission request is
/// cancelled and updates are ignored.
struct StatusClient;

impl Handler for StatusClient {
    fn request(&self, _shared: &Shared, method: &str, params: Option<&Value>) -> Option<Reply> {
        if method != "session/request_permission" {
            return None;
        }
        Some(match PermissionRequest::parse(params) {
            Some(_) => cancelled(),
            None => invalid_params(),
        })
    }

    fn notification(&self, _shared: &Shared, _method: &str, _params: Option<&Value>) {}
}

/// A non-empty string session id from a `session/new` answer.
fn session_id_of(result: &Value) -> Option<String> {
    result
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// `acpStatus`.
pub fn acp_status(
    spec: &AgentSpec,
    runtime: Option<Runtime>,
    cwd: &Path,
) -> Result<ProviderStatus, BrokerFailure> {
    let Some(runtime) = runtime else {
        return Ok(provider_status(
            spec.id,
            spec.display_name,
            ProviderState::MissingExecutable,
            format!(
                "Install the {} CLI, then sign in outside Kassiber.",
                spec.display_name
            ),
        ));
    };
    let executable = runtime.executable.to_string_lossy().into_owned();
    let version = cli_version(spec, &runtime);
    let status = |state, message: String| {
        let status = provider_status(spec.id, spec.display_name, state, message)
            .with_executable(&executable);
        match &version {
            Some(version) => status.with_version(version.clone()),
            None => status,
        }
    };
    if let Some(problem) = version_problem(spec, version.as_deref()) {
        return Ok(status(ProviderState::Error, problem));
    }
    let launch = (spec.launch)(&LaunchContext {
        cwd,
        request: None,
        mcp: None,
        env: &runtime.env,
    })?;
    let outcome = match start_agent(
        spec.id,
        &runtime.executable,
        &launch.args,
        &launch.env,
        cwd,
        Arc::new(StatusClient),
    ) {
        Ok(agent) => {
            let outcome = status_handshake(spec, &agent, cwd);
            agent.stop();
            outcome
        }
        Err(failure) => Err(Failure::Message(failure.message().to_owned())),
    };
    Ok(match outcome {
        Ok(models) => {
            let discovered = models
                .into_iter()
                .filter(|model| model.id != "default")
                .map(|model| with_efforts(spec, model));
            let models = std::iter::once(default_model(spec))
                .chain(discovered)
                .collect();
            status(
                ProviderState::Ready,
                format!("Ready using the existing {} login.", spec.display_name),
            )
            .with_models(models)
        }
        Err(failure) if failure.is_auth() => status(
            ProviderState::AuthenticationRequired,
            spec.login_hint.to_owned(),
        ),
        Err(failure) => status(
            ProviderState::Error,
            safe_error_message(&failure.message(spec)),
        ),
    })
}

/// The probe's handshake, within one [`HANDSHAKE_TIMEOUT`]: `initialize`,
/// then a session (the only proof of a working login ACP offers, and the
/// only place an agent lists its models), closed again when the agent can.
fn status_handshake(
    spec: &AgentSpec,
    agent: &AgentProcess,
    cwd: &Path,
) -> Result<Vec<BrokerModel>, Failure> {
    let deadline = Some(Instant::now() + handshake_timeout());
    let fail = |wait| Failure::from_wait(spec, agent, wait, "did not answer the ACP handshake.");
    let init = agent
        .request("initialize", INITIALIZE_PARAMS, deadline, false)
        .map_err(fail)?;
    check_protocol(spec, &init)?;
    let cwd_text = cwd.to_string_lossy();
    let params = encode(&NewSession {
        cwd: &cwd_text,
        mcp_servers: &[],
    })?;
    let session = agent
        .request("session/new", &params, deadline, false)
        .map_err(fail)?;
    if capability(&init, &["sessionCapabilities", "close"]).is_some_and(truthy) {
        let params = match session.get("sessionId") {
            Some(id) => format!(
                r#"{{"sessionId":{}}}"#,
                serde_json::to_string(id).unwrap_or_else(|_| "null".into())
            ),
            None => "{}".into(),
        };
        // A refused close is fine; running out of time or losing the agent
        // still fails the probe, as the whole exchange is raced.
        match agent.request("session/close", &params, deadline, false) {
            Ok(_) | Err(Wait::Rpc(_)) => {}
            Err(wait) => return Err(fail(wait)),
        }
    }
    Ok(models_from_config_options(session.get("configOptions")))
}

/// `finishReason`.
fn finish_reason(stop_reason: Option<&Value>) -> Result<&'static str, Failure> {
    match stop_reason.and_then(Value::as_str) {
        Some("end_turn") => Ok("stop"),
        Some("max_tokens" | "max_turn_requests") => Ok("length"),
        Some("refusal") => Ok("content_filter"),
        _ => Err(Failure::Message("The agent cancelled the response.".into())),
    }
}

/// The client during a chat turn: streams the answer and keeps the
/// tripwire.
struct ChatClient {
    spec: &'static AgentSpec,
    advertised: Vec<String>,
    events: EventSink,
    session: Mutex<Option<String>>,
    /// `session/load` replays history as updates; none of it is new output.
    replaying: AtomicBool,
    /// Text before the prompt is sent is a startup notice, not the answer.
    prompted: AtomicBool,
    /// Each accepted call id and the Kassiber tool it was accepted as.
    calls: Mutex<HashMap<String, String>>,
    /// The ids of the `session/new` and `session/load` requests, whose
    /// answers set and settle the session on the reader thread.
    new_session: Mutex<Option<u64>>,
    load_session: Mutex<Option<u64>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl ChatClient {
    fn is_session(&self, id: Option<&Value>) -> bool {
        let current = lock(&self.session);
        matches!((current.as_deref(), id.and_then(Value::as_str)), (Some(current), Some(id)) if current == id)
    }

    /// `stillKassiber`: whether a call is still the Kassiber tool its id
    /// was accepted as. Updates may omit the title or kind, but a title or
    /// kind they do supply must still match; a known id cannot be retitled
    /// into another tool or into execution.
    fn still_kassiber(&self, call: &ToolCallView, accepted: &str) -> bool {
        if call.kind.as_deref() == Some("execute") {
            return false;
        }
        if call.title.is_none() {
            return true;
        }
        kassiber_tool_for(
            self.spec,
            call.title.as_deref(),
            call.kind.as_deref(),
            &self.advertised,
        )
        .as_deref()
            == Some(accepted)
    }

    /// `acceptToolCall`: accept a call as a Kassiber tool, or report that
    /// it is not one.
    fn accept(&self, call: &ToolCallView) -> bool {
        let mut calls = lock(&self.calls);
        if let Some(accepted) = calls.get(&call.id) {
            return self.still_kassiber(call, accepted);
        }
        let Some(tool) = kassiber_tool_for(
            self.spec,
            call.title.as_deref(),
            call.kind.as_deref(),
            &self.advertised,
        ) else {
            return false;
        };
        calls.insert(call.id.clone(), tool);
        true
    }

    fn text_of(update: &Value) -> Option<&str> {
        let content = update.get("content")?;
        if content.get("type")?.as_str()? != "text" {
            return None;
        }
        content
            .get("text")?
            .as_str()
            .filter(|text| !text.is_empty())
    }
}

impl Handler for ChatClient {
    fn request(&self, shared: &Shared, method: &str, params: Option<&Value>) -> Option<Reply> {
        if method != "session/request_permission" {
            return None;
        }
        let Some(permission) = PermissionRequest::parse(params) else {
            return Some(invalid_params());
        };
        let current = lock(&self.session).clone();
        if current.as_deref() != Some(permission.session_id.as_str())
            || self.replaying.load(Ordering::SeqCst)
        {
            return Some(cancelled());
        }
        if self.accept(&permission.call) {
            return Some(permission.allow_once());
        }
        let answer = permission.reject_once();
        shared.violation();
        Some(answer)
    }

    fn notification(&self, shared: &Shared, method: &str, params: Option<&Value>) {
        if method != "session/update" {
            return;
        }
        let Some(params) = params else {
            return;
        };
        if self.replaying.load(Ordering::SeqCst) || !self.is_session(params.get("sessionId")) {
            return;
        }
        let Some(update) = params.get("update").filter(|update| update.is_object()) else {
            return;
        };
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("agent_message_chunk") => {
                if let Some(text) = Self::text_of(update) {
                    if self.prompted.load(Ordering::SeqCst) && !(self.spec.is_notice)(text) {
                        self.events.write(&Event::Delta {
                            content: Some(text.to_owned()),
                            reasoning: None,
                        });
                    }
                }
            }
            Some("agent_thought_chunk") => {
                if let Some(text) = Self::text_of(update) {
                    if self.prompted.load(Ordering::SeqCst) {
                        self.events.write(&Event::Delta {
                            content: None,
                            reasoning: Some(text.to_owned()),
                        });
                    }
                }
            }
            // An update may be the first we hear of a call. A call whose id,
            // or (for a new call) title, is malformed cannot be checked, so
            // it stops the agent too.
            Some(kind @ ("tool_call" | "tool_call_update")) => {
                let call = ToolCallView::parse(update)
                    .filter(|call| kind == "tool_call_update" || call.title.is_some());
                match call {
                    Some(call) if self.accept(&call) => {}
                    _ => shared.violation(),
                }
            }
            _ => {}
        }
    }

    fn response(&self, id: u64, answer: &Result<Value, RpcError>) {
        if *lock(&self.load_session) == Some(id) {
            if answer.is_err() {
                *lock(&self.session) = None;
            }
            self.replaying.store(false, Ordering::SeqCst);
        }
        if *lock(&self.new_session) == Some(id) {
            if let Ok(result) = answer {
                *lock(&self.session) = session_id_of(result);
            }
        }
    }
}

/// `acpChat`.
pub fn acp_chat(
    spec: &'static AgentSpec,
    runtime: &Runtime,
    request: &ChatRequest,
    cwd: &Path,
    bridge: Option<&ToolBridge>,
    events: &EventSink,
) -> Result<(), BrokerFailure> {
    if sensitive_context(request)? {
        return Err(BrokerFailure::new(format!(
            "{} cannot be held to a stateless, tool-free exchange; use another provider for selected data.",
            spec.display_name
        )));
    }
    // An Incognito turn is not refused: an agent that saves its session
    // anyway (Copilot) keeps it only in the per-turn home, removed with the
    // turn. It never resumes or returns a session cursor.
    let stateless = stateless_session(request)?;
    let keeps_session = spec.persistent_sessions && !stateless;
    if let Some(problem) = version_problem(spec, cli_version(spec, runtime).as_deref()) {
        return Err(BrokerFailure::new(problem));
    }
    let tools: &[BrokerToolDefinition] = if bridge.is_some() {
        &request.tools
    } else {
        &[]
    };
    let advertised: Vec<String> = tools.iter().map(|tool| tool.name.clone()).collect();
    let mcp = match bridge {
        Some(bridge) if !tools.is_empty() => Some(mcp_command(cwd, tools, bridge)?),
        _ => None,
    };
    let launch = (spec.launch)(&LaunchContext {
        cwd,
        request: Some(request),
        mcp: mcp.as_deref(),
        env: &runtime.env,
    })?;
    let mcp_servers = match &mcp {
        Some(mcp) if launch.session_mcp => vec![mcp_server_for(mcp)?],
        _ => Vec::new(),
    };

    events.write(&Event::Status {
        phase: "connecting".into(),
        message: format!("Starting {}", spec.display_name),
    });
    let client = Arc::new(ChatClient {
        spec,
        advertised,
        events: events.clone(),
        session: Mutex::new(None),
        replaying: AtomicBool::new(false),
        prompted: AtomicBool::new(false),
        calls: Mutex::new(HashMap::new()),
        new_session: Mutex::new(None),
        load_session: Mutex::new(None),
    });
    let outcome = match start_agent(
        spec.id,
        &runtime.executable,
        &launch.args,
        &launch.env,
        cwd,
        Arc::clone(&client) as Arc<dyn Handler>,
    ) {
        Ok(agent) => {
            let outcome = run_chat(
                spec,
                &agent,
                &client,
                request,
                cwd,
                &mcp_servers,
                keeps_session,
                events,
            );
            agent.stop();
            outcome
        }
        Err(failure) => Err(Failure::Message(failure.message().to_owned())),
    };
    outcome.map_err(|failure| match failure {
        Failure::Native => BrokerFailure::new(native_tool_attempt(spec)),
        failure if failure.is_auth() => {
            BrokerFailure::new(format!("{} requires authentication.", spec.display_name))
        }
        failure => BrokerFailure::new(failure.message(spec)),
    })
}

#[allow(clippy::too_many_arguments)]
fn run_chat(
    spec: &AgentSpec,
    agent: &AgentProcess,
    client: &ChatClient,
    request: &ChatRequest,
    cwd: &Path,
    mcp_servers: &[McpServer],
    keeps_session: bool,
    events: &EventSink,
) -> Result<(), Failure> {
    let handshake = || Some(Instant::now() + handshake_timeout());
    let fail = |wait, timeout| Failure::from_wait(spec, agent, wait, timeout);
    let init = agent
        .request("initialize", INITIALIZE_PARAMS, handshake(), true)
        .map_err(|wait| fail(wait, "did not answer the ACP handshake."))?;
    check_protocol(spec, &init)?;
    let cwd_text = cwd.to_string_lossy();

    let cursor = if keeps_session {
        safe_session_cursor(request.option("provider_session_id"))
    } else {
        None
    };
    let mut resumed = false;
    if let Some(cursor) = cursor {
        if capability(&init, &["loadSession"]).is_some_and(truthy) {
            client.replaying.store(true, Ordering::SeqCst);
            *lock(&client.session) = Some(cursor.clone());
            let params = encode(&LoadSession {
                session_id: &cursor,
                cwd: &cwd_text,
                mcp_servers,
            })?;
            let loaded = agent
                .send("session/load", &params, |id| {
                    *lock(&client.load_session) = Some(id)
                })
                .and_then(|id| agent.wait(id, handshake(), true));
            match loaded {
                Ok(_) => resumed = true,
                // Agents key saved sessions by directory, and each turn runs
                // in a fresh one, so a failed load just starts over with the
                // transcript. Losing the agent or tripping the wire does not.
                Err(wait @ (Wait::Exited | Wait::Violation)) => {
                    return Err(fail(wait, "did not reopen the session."))
                }
                Err(_) => *lock(&client.session) = None,
            }
            client.replaying.store(false, Ordering::SeqCst);
        }
    }
    if !resumed {
        let params = encode(&NewSession {
            cwd: &cwd_text,
            mcp_servers,
        })?;
        let session = agent
            .send("session/new", &params, |id| {
                *lock(&client.new_session) = Some(id)
            })
            .and_then(|id| agent.wait(id, handshake(), true))
            .map_err(|wait| fail(wait, "did not open a session."))?;
        // The reader set it already; this covers an answer it could not
        // read as a session.
        *lock(&client.session) = session_id_of(&session);
    }
    let Some(active) = lock(&client.session).clone() else {
        return Err(Failure::Message(format!(
            "{} did not open a session.",
            spec.display_name
        )));
    };

    events.write(&Event::Status {
        phase: "streaming".into(),
        message: format!("{} is responding", spec.display_name),
    });
    client.prompted.store(true, Ordering::SeqCst);
    let accepts_images =
        capability(&init, &["promptCapabilities", "image"]) == Some(&Value::Bool(true));
    let blocks = prompt_blocks(request, resumed, accepts_images)
        .map_err(|failure| Failure::Message(failure.message().to_owned()))?;
    let params = encode(&Prompt {
        session_id: &active,
        prompt: &blocks,
    })?;
    let response = agent
        .request("session/prompt", &params, None, true)
        .map_err(|wait| fail(wait, ""))?;
    let finish_reason = finish_reason(response.get("stopReason"))?;
    events.write(&Event::Done {
        finish_reason: finish_reason.to_owned(),
        provider_session_id: keeps_session.then_some(active),
    });
    Ok(())
}

#[cfg(all(test, unix))]
mod tests;

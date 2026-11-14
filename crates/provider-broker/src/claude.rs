//! The Claude adapter (`claude.ts`): `claude --print` with stream-json
//! output, one process per turn.
//!
//! Tool denial is layered, because a coding CLI can otherwise read and
//! write the user's disk from a chat box (see [`chat_args`]), and the
//! stream reader still fails the turn on any tool use that is not one of
//! Kassiber's advertised tools. Selected-data and Incognito turns run with
//! `--no-session-persistence`, refused up front on a CLI without it.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;

use serde::Serialize;
use serde_json::Value;

use crate::bridge::mcp_command;
use crate::cleanup::TrackedChild;
use crate::codex::version_at;
use crate::env::provider_environment;
use crate::executables::{
    provider_command, resolve_executable, run_provider, spawn_failure, RunOptions,
};
use crate::prompt::{prompt_from_messages, prompt_images, system_instructions};
use crate::protocol::{
    first_utf16_units, js_trim, provider_status, requested_fast_mode, safe_error_message,
    safe_session_cursor, sensitive_context, stateless_session, truthy, BrokerFailure, BrokerImage,
    BrokerModel, ChatRequest, Event, ProviderId, ProviderState, ProviderStatus,
};
use crate::{Adapter, EventSink, ToolBridge};

/// The adapter [`crate::adapter`] returns for `claude`.
pub static CLAUDE: Claude = Claude;

pub struct Claude;

const DISPLAY_NAME: &str = "Claude";
const EXECUTABLE: &str = "claude";

/// How much stderr a failed turn's message may draw on (`stderrPreview`).
const STDERR_PREVIEW: usize = 4_096;

/// `sensitiveClaudeEnvironment()`: Claude's environment without debug,
/// trace, logging or telemetry settings, and with nonessential traffic,
/// telemetry and error reporting off.
pub fn sensitive_claude_environment() -> Vec<(OsString, OsString)> {
    let noisy = ["DEBUG", "TRACE", "LOGGING", "TELEMETRY", "OTEL"];
    let mut env: Vec<(OsString, OsString)> = provider_environment(ProviderId::Claude)
        .into_iter()
        .filter(|(name, _)| {
            let name = name.to_string_lossy();
            !noisy.iter().any(|word| name.contains(word))
        })
        .collect();
    for (name, value) in [
        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
        ("DISABLE_TELEMETRY", "1"),
        ("DISABLE_ERROR_REPORTING", "1"),
    ] {
        env.retain(|(existing, _)| existing != name);
        env.push((name.into(), value.into()));
    }
    env
}

/// The efforts `claude --effort` documents. The CLI applies them per
/// session, not per model, and publishes no per-model capability data.
pub const CLAUDE_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Fast mode (`--settings '{"fastMode":true}'`) is honoured only on Opus:
/// the CLI accepts the setting on other models but reports
/// `fast_mode_state: "off"`, so only the `opus` alias advertises it.
pub const CLAUDE_FAST_MODELS: [&str; 1] = ["opus"];
const CLAUDE_FAST_DESCRIPTION: &str = "Faster Opus output, billed at a higher rate";

/// The first CLI that honours `fastMode` in `--settings` under `-p`.
const CLAUDE_FAST_MIN_VERSION: [f64; 3] = [2.0, 1.0, 205.0];

/// `claudeVersionSupportsFast(version)`: whether a `claude --version` line
/// names a CLI that can run fast mode. The first `\d+.\d+.\d+` counts.
pub fn claude_version_supports_fast(version: Option<&str>) -> bool {
    let Some(version) = version else {
        return false;
    };
    let bytes = version.as_bytes();
    let Some(found) = (0..bytes.len()).find_map(|start| version_at(bytes, start)) else {
        return false;
    };
    for (found, floor) in found.0.iter().zip(CLAUDE_FAST_MIN_VERSION) {
        if *found != floor {
            return *found > floor;
        }
    }
    true
}

/// The Claude CLI has no model-enumeration command, so this list is
/// maintained by hand. It holds *aliases*, not pinned ids, deliberately: an
/// alias always resolves to the current model behind it, whereas
/// `claude-sonnet-5` would rot on the next release. The concrete id is only
/// knowable at runtime (the CLI reports it in the `init` event of a real
/// request), so no version shows here.
///
/// `default` is Kassiber's own sentinel, not a Claude alias: it means "send
/// no --model" and let the CLI's own configured default apply.
pub fn claude_models() -> Vec<BrokerModel> {
    [
        ("default", "Claude (CLI default)"),
        ("fable", "Claude Fable"),
        ("opus", "Claude Opus"),
        ("sonnet", "Claude Sonnet"),
        ("haiku", "Claude Haiku"),
    ]
    .into_iter()
    .map(|(id, display_name)| {
        let fast = CLAUDE_FAST_MODELS.contains(&id);
        BrokerModel {
            id: id.to_owned(),
            display_name: Some(display_name.to_owned()),
            owned_by: Some("Anthropic".to_owned()),
            supports_reasoning_effort: Some(true),
            reasoning_efforts: Some(CLAUDE_EFFORTS.iter().map(|e| (*e).to_owned()).collect()),
            supports_fast_mode: fast.then_some(true),
            fast_mode_description: fast.then(|| CLAUDE_FAST_DESCRIPTION.to_owned()),
            ..BrokerModel::default()
        }
    })
    .collect()
}

/// `claudeModelsFor(version)`: the model list, with fast mode withheld from
/// a CLI too old to honour it.
pub fn claude_models_for(version: Option<&str>) -> Vec<BrokerModel> {
    let mut models = claude_models();
    if !claude_version_supports_fast(version) {
        for model in &mut models {
            model.supports_fast_mode = None;
            model.fast_mode_description = None;
        }
    }
    models
}

/// `claudeFastFallbackNotification(message)`: whether a stream-json line is
/// Claude's notice that it retried a fast request at standard speed after
/// usage credits ran out (or hit their limit). Fast mode then stays "on",
/// so `fast_mode_state` cannot tell. The documented signal is a `system`
/// message with subtype `notification` whose text is "Fast mode disabled ·
/// usage credits exhausted"; the same leading words open its "usage credit
/// limit reached" variant, and Claude Code 2.1.287 keys both
/// `fast-mode-overage-rejected`. Only this yes/no crosses, never the text.
pub fn claude_fast_fallback_notification(message: &Value) -> bool {
    if message.get("type").and_then(Value::as_str) != Some("system")
        || message.get("subtype").and_then(Value::as_str) != Some("notification")
    {
        return false;
    }
    if message.get("key").and_then(Value::as_str) == Some("fast-mode-overage-rejected") {
        return true;
    }
    // `/^Fast mode disabled\b/` on the trimmed text.
    message
        .get("text")
        .and_then(Value::as_str)
        .map(js_trim)
        .and_then(|text| text.strip_prefix("Fast mode disabled"))
        .is_some_and(|rest| {
            !rest
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

/// The Claude tools named in `--disallowed-tools`.
pub const CLAUDE_DISABLED_TOOLS: [&str; 12] = [
    "Bash",
    "Read",
    "Write",
    "Edit",
    "MultiEdit",
    "Glob",
    "Grep",
    "WebFetch",
    "WebSearch",
    "NotebookEdit",
    "Task",
    "Agent",
];

/// `claudeFastModeApplies(request)`: fast mode was requested and the
/// selected model honours it.
pub fn claude_fast_mode_applies(request: &ChatRequest) -> Result<bool, BrokerFailure> {
    Ok(requested_fast_mode(request)? && CLAUDE_FAST_MODELS.contains(&request.model.as_str()))
}

#[derive(Serialize)]
struct McpConfig<'a> {
    #[serde(rename = "mcpServers")]
    mcp_servers: McpServers<'a>,
}

#[derive(Serialize)]
struct McpServers<'a> {
    kassiber: McpServer<'a>,
}

#[derive(Serialize)]
struct McpServer<'a> {
    command: &'a str,
    args: &'a [String],
}

/// `chatArgs(request, cwd, toolBridge)`: the CLI invocation.
///
/// Tool denial is layered:
///   `--setting-sources ""`  loads no user/project/local settings, so no
///                           user hooks, plugins or custom agents run
///   `--tools ""`            empties the built-in tool set
///   `--disallowed-tools`    names the file/exec/network tools anyway (it is
///                           variadic, so it goes last)
///   `--strict-mcp-config`   loads only Kassiber's ephemeral MCP server
///   `--allowedTools`        only the `mcp__kassiber__*` names advertised
///   `--permission-mode`     dontAsk, so nothing escalates by prompting
///   `--disable-slash-commands` removes the remaining skill command surface
/// and the reader still hard-fails on any non-Kassiber `tool_use` block.
///
/// `--safe-mode` cannot be used here: it disables even an explicitly
/// supplied MCP server. `--bare` would preserve explicit MCP, but
/// deliberately skips the OAuth/keychain login users already configured
/// and suppresses stream events.
pub fn chat_args(
    request: &ChatRequest,
    cwd: &Path,
    bridge: Option<&ToolBridge>,
) -> Result<Vec<String>, BrokerFailure> {
    let mut args: Vec<String> = [
        "--print",
        "--output-format",
        "stream-json",
        // stream-json refuses to emit under --print without --verbose.
        "--verbose",
        "--include-partial-messages",
        // The broker runs in a fresh empty directory; loading no filesystem
        // setting sources additionally removes user hooks, plugins and custom
        // agents while preserving the user's existing OAuth/keychain
        // authentication.
        "--setting-sources",
        "",
        "--disable-slash-commands",
        "--strict-mcp-config",
        "--permission-mode",
        "dontAsk",
        "--system-prompt",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(system_instructions(request));
    args.extend(["--tools".to_owned(), String::new()]);
    // Selected-data and Incognito turns: Claude keeps no session file.
    if sensitive_context(request)? || stateless_session(request)? {
        args.push("--no-session-persistence".to_owned());
    }
    if let Some(bridge) = bridge.filter(|_| !request.tools.is_empty()) {
        let command = mcp_command(cwd, &request.tools, bridge)?;
        let (program, program_args) = command
            .split_first()
            .ok_or_else(|| BrokerFailure::new("Kassiber could not start its tool server."))?;
        let config = McpConfig {
            mcp_servers: McpServers {
                kassiber: McpServer {
                    command: program,
                    args: program_args,
                },
            },
        };
        args.push("--mcp-config".to_owned());
        args.push(
            serde_json::to_string(&config)
                .map_err(|_| BrokerFailure::new("Kassiber could not write the tool server."))?,
        );
        args.push("--allowedTools".to_owned());
        args.push("mcp__kassiber__*".to_owned());
    }
    if request.model != "default" {
        args.push("--model".to_owned());
        args.push(request.model.clone());
    }
    let session_id = safe_session_cursor(request.option("provider_session_id"));
    // Attached images need a structured message on stdin; plain text
    // otherwise.
    if !prompt_images(&request.messages, session_id.is_some())?.is_empty() {
        args.push("--input-format".to_owned());
        args.push("stream-json".to_owned());
    }
    // `--resume=<id>`, never `--resume <id>`: the attached form cannot be
    // reinterpreted as a separate flag even if validation is ever loosened.
    if let Some(session_id) = &session_id {
        args.push(format!("--resume={session_id}"));
    }
    if let Some(effort) = request.option("reasoning_effort") {
        if truthy(effort) && effort.as_str() != Some("auto") {
            // Node passes a non-string option as `String(value)`.
            args.push("--effort".to_owned());
            args.push(crate::codex::js_string(Some(effort)));
        }
    }
    // `--settings` still applies under `--setting-sources ""`: it is inline
    // configuration, not a settings file the CLI discovers. Fast mode is
    // stated on every turn, off included: `--resume` continues a native
    // session that an earlier turn may have run fast, and "default" can
    // resolve to Opus, so leaving the setting out must not be what turns it
    // off. Claude's settings schema reads `false` as off.
    args.push("--settings".to_owned());
    args.push(format!(
        "{{\"fastMode\":{}}}",
        claude_fast_mode_applies(request)?
    ));
    // Variadic, so it goes last: the CLI consumes tool names until the next
    // flag.
    args.push("--disallowed-tools".to_owned());
    args.extend(CLAUDE_DISABLED_TOOLS.iter().map(|tool| (*tool).to_owned()));
    Ok(args)
}

#[derive(Serialize)]
struct StreamInput<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    message: StreamMessage<'a>,
}

#[derive(Serialize)]
struct StreamMessage<'a> {
    role: &'static str,
    content: Vec<ContentBlock<'a>>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ContentBlock<'a> {
    Text { text: &'a str },
    Image { source: ImageSource<'a> },
}

#[derive(Serialize)]
struct ImageSource<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    media_type: &'a str,
    data: &'a str,
}

/// `claudeStreamInput(prompt, images)`: the turn as Claude's stream-json
/// input: one user message with the prompt and the attached images as
/// base64 image blocks, which `--print` answers once stdin closes.
pub fn claude_stream_input(prompt: &str, images: &[BrokerImage]) -> String {
    let mut content = vec![ContentBlock::Text { text: prompt }];
    content.extend(images.iter().map(|image| ContentBlock::Image {
        source: ImageSource {
            kind: "base64",
            media_type: &image.media_type,
            data: &image.data,
        },
    }));
    let input = StreamInput {
        kind: "user",
        message: StreamMessage {
            role: "user",
            content,
        },
    };
    let mut line = serde_json::to_string(&input).unwrap_or_default();
    line.push('\n');
    line
}

/// The first [`STDERR_PREVIEW`] bytes (and the chunk that crossed it), the
/// rest drained.
fn read_preview(mut stderr: impl Read) -> String {
    let mut kept = Vec::new();
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                if kept.len() < STDERR_PREVIEW {
                    kept.extend_from_slice(&chunk[..read]);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

/// `child.kill()`: SIGTERM if it still runs, then reap it in the
/// background.
fn stop(child: TrackedChild) {
    if matches!(child.try_wait(), Ok(None)) {
        #[cfg(unix)]
        let _ = child.terminate();
        #[cfg(not(unix))]
        let _ = child.kill();
    }
    let _ = std::thread::Builder::new()
        .name("claude-reap".into())
        .spawn(move || {
            let _ = child.wait();
        });
}

impl Claude {
    /// `claudeStatus()` with a resolved executable.
    pub fn status_with(&self, executable: &Path) -> ProviderStatus {
        let options = RunOptions {
            limit: Some(16_384),
            ..RunOptions::default()
        };
        let (auth, version) = std::thread::scope(|scope| {
            let auth = scope.spawn(|| {
                run_provider(
                    ProviderId::Claude,
                    executable,
                    &["auth", "status", "--json"],
                    options,
                )
            });
            let version = scope
                .spawn(|| run_provider(ProviderId::Claude, executable, &["--version"], options));
            let failed = || Err(BrokerFailure::new("Provider status probe failed."));
            (
                auth.join().unwrap_or_else(|_| failed()),
                version.join().unwrap_or_else(|_| failed()),
            )
        });
        let shown = executable.to_string_lossy();
        let (auth, version) = match (auth, version) {
            (Ok(auth), Ok(version)) => (auth, version),
            (Err(failure), _) | (_, Err(failure)) => {
                return provider_status(
                    ProviderId::Claude,
                    DISPLAY_NAME,
                    ProviderState::Error,
                    safe_error_message(failure.message()),
                )
                .with_executable(&shown);
            }
        };
        // Older Claude versions communicate authentication through the exit
        // code; a `null` answer reads like them, as its TypeError does.
        let logged_in = match serde_json::from_str::<Value>(&auth.stdout) {
            Ok(Value::Null) | Err(_) => auth.code == Some(0),
            Ok(answer) => answer.get("loggedIn") == Some(&Value::Bool(true)),
        };
        let version_line = first_utf16_units(js_trim(&version.stdout), 80);
        let version_line = (!version_line.is_empty()).then_some(version_line);
        let status = provider_status(
            ProviderId::Claude,
            DISPLAY_NAME,
            if logged_in {
                ProviderState::Ready
            } else {
                ProviderState::AuthenticationRequired
            },
            if logged_in {
                "Ready using the existing Claude login."
            } else {
                "Run `claude login` outside Kassiber."
            },
        )
        .with_executable(&shown)
        .with_models(claude_models_for(version_line));
        match version_line {
            Some(line) => status.with_version(line),
            None => status,
        }
    }

    /// `claudeChat(request, cwd, toolBridge)` with a resolved executable.
    pub fn chat_with(
        &self,
        executable: &Path,
        request: &ChatRequest,
        cwd: &Path,
        bridge: Option<&ToolBridge>,
        events: &EventSink,
    ) -> Result<(), BrokerFailure> {
        let sensitive = sensitive_context(request)?;
        let stateless = stateless_session(request)?;
        if sensitive || stateless {
            // Fail closed: a CLI without the flag would save the session
            // anyway.
            let help = run_provider(
                ProviderId::Claude,
                executable,
                &["--help"],
                RunOptions {
                    limit: Some(100_000),
                    ..RunOptions::default()
                },
            )?;
            if help.code != Some(0) || !help.stdout.contains("--no-session-persistence") {
                return Err(BrokerFailure::new(if sensitive {
                    "Claude does not support stateless sensitive context; update the CLI."
                } else {
                    "Claude cannot skip saving this session; update the CLI to use Incognito."
                }));
            }
        }
        let resumed = safe_session_cursor(request.option("provider_session_id")).is_some();
        let fast_requested = claude_fast_mode_applies(request)?;
        let args = chat_args(request, cwd, bridge)?;
        let env = if sensitive {
            sensitive_claude_environment()
        } else {
            provider_environment(ProviderId::Claude)
        };
        let mut command =
            provider_command(ProviderId::Claude, executable, &args, Some(cwd), Some(&env));
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let spawned = crate::executables::spawn_provider(&mut command);
        let turn = (|| {
            events.write(&Event::Status {
                phase: "connecting".into(),
                message: "Starting Claude".into(),
            });
            let mut child = spawned.map_err(|error| spawn_failure(executable, &error))?;
            let stdin = child.stdin.take();
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let child = crate::cleanup::track_child(child);
            let preview = stderr.and_then(|stderr| {
                std::thread::Builder::new()
                    .name("claude-stderr".into())
                    .spawn(move || read_preview(stderr))
                    .ok()
            });
            let read = (|| {
                // The prompt goes over stdin, not argv: a long conversation
                // would blow past the platform argument limit.
                let prompt = prompt_from_messages(&request.messages, resumed);
                let images = prompt_images(&request.messages, resumed)?;
                let input = if images.is_empty() {
                    prompt
                } else {
                    claude_stream_input(&prompt, &images)
                };
                if let Some(mut stdin) = stdin {
                    let _ = std::thread::Builder::new()
                        .name("claude-stdin".into())
                        .spawn(move || {
                            let _ = stdin.write_all(input.as_bytes());
                        });
                }
                let Some(stdout) = stdout else {
                    return Ok(false);
                };
                read_stream(
                    stdout,
                    request,
                    sensitive || stateless,
                    fast_requested,
                    events,
                )
            })();
            let answered = match read {
                Ok(answered) => answered,
                Err(failure) => {
                    stop(child);
                    return Err(failure);
                }
            };
            if answered {
                stop(child);
                return Ok(());
            }
            let code = child.wait().ok().and_then(|status| status.code());
            let stderr = preview
                .and_then(|thread| thread.join().ok())
                .unwrap_or_default();
            let detail = safe_error_message(
                if stderr.is_empty() {
                    format!(
                        "Claude exited with code {}",
                        code.map_or_else(|| "null".to_owned(), |code| code.to_string())
                    )
                } else {
                    stderr
                }
                .as_str(),
            );
            Err(BrokerFailure::new(format!(
                "Claude ended without a terminal response. {detail}"
            )))
        })();
        match turn {
            Err(_) if sensitive => Err(BrokerFailure::new(
                "Claude sensitive-context request failed; no provider diagnostic content was retained.",
            )),
            turn => turn,
        }
    }
}

/// What the stream reader keeps across lines.
struct StreamState {
    allowed: Vec<String>,
    stateless: bool,
    fast_requested: bool,
    provider_session_id: Option<String>,
    fast_state: Option<String>,
    fast_fell_back: bool,
}

impl StreamState {
    /// One stream-json line: `Some` ends the turn (`Ok(())` once answered).
    fn line(&mut self, line: &str, events: &EventSink) -> Option<Result<(), BrokerFailure>> {
        let trimmed = js_trim(line);
        if trimmed.is_empty() {
            return None;
        }
        let message = serde_json::from_str::<Value>(trimmed).ok()?;
        if message.is_null() {
            // Reading a field of `null` throws in TypeScript.
            return Some(Err(BrokerFailure::new(
                "Claude sent an unreadable stream line.",
            )));
        }
        if let Some(session) = message.get("session_id").and_then(Value::as_str) {
            self.provider_session_id = Some(session.to_owned());
        }
        // Fast mode can be declined up front (plan) or drop to "cooldown"
        // partway through after a rate limit, so the last reported state
        // wins.
        if let Some(state) = message.get("fast_mode_state").and_then(Value::as_str) {
            self.fast_state = Some(state.to_owned());
        }
        // Out of usage credits, Claude answers at standard speed but keeps
        // the state "on" and says so only through this notification.
        if claude_fast_fallback_notification(&message) {
            self.fast_fell_back = true;
        }
        let kind = message.get("type").and_then(Value::as_str);
        if let Some(event) = message
            .get("event")
            .filter(|event| kind == Some("stream_event") && truthy(event))
        {
            if let Some(failure) = self.stream_event(event, events) {
                return Some(Err(failure));
            }
        }
        if kind != Some("result") {
            return None;
        }
        if message.get("subtype").and_then(Value::as_str) != Some("success") {
            return Some(Err(BrokerFailure::new(
                "Claude did not complete the response.",
            )));
        }
        // Decided once the final state is known; a status hint, not a
        // failure. A CLI that never reports the state (one older than fast
        // mode) did not run fast either, nor did a turn retried after a
        // credits notice.
        if self.fast_requested && (self.fast_state.as_deref() != Some("on") || self.fast_fell_back)
        {
            events.write(&Event::Status {
                phase: "fast_mode_unavailable".into(),
                message: "Fast mode unavailable; answering at standard speed".into(),
            });
        }
        events.write(&Event::Done {
            finish_reason: "stop".into(),
            provider_session_id: if self.stateless {
                None
            } else {
                self.provider_session_id.clone()
            },
        });
        Some(Ok(()))
    }

    /// A raw Anthropic streaming event: text and thinking deltas stream
    /// out; a tool use outside Kassiber's advertised tools fails the turn.
    fn stream_event(&self, event: &Value, events: &EventSink) -> Option<BrokerFailure> {
        let event_type = event.get("type").and_then(Value::as_str);
        let delta = event.get("delta");
        let delta_type = delta
            .and_then(|delta| delta.get("type"))
            .and_then(Value::as_str);
        let delta_text = |name: &str| {
            delta
                .and_then(|delta| delta.get(name))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        if event_type == Some("content_block_delta") && delta_type == Some("text_delta") {
            if let Some(text) = delta_text("text") {
                events.write(&Event::Delta {
                    content: Some(text),
                    reasoning: None,
                });
                return None;
            }
        }
        if event_type == Some("content_block_delta") && delta_type == Some("thinking_delta") {
            if let Some(thinking) = delta_text("thinking") {
                events.write(&Event::Delta {
                    content: None,
                    reasoning: Some(thinking),
                });
                return None;
            }
        }
        let block = event.get("content_block");
        if event_type == Some("content_block_start")
            && block
                .and_then(|block| block.get("type"))
                .and_then(Value::as_str)
                == Some("tool_use")
        {
            let name = block
                .and_then(|block| block.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if name.is_empty() || !self.allowed.iter().any(|tool| tool == name) {
                return Some(BrokerFailure::new(
                    "Claude attempted to use a provider-native tool; Kassiber stopped it.",
                ));
            }
        }
        None
    }
}

/// Read the stream-json lines until the `result` line: `Ok(true)` once the
/// turn is answered (its `done` written), `Ok(false)` when stdout ended
/// first.
fn read_stream(
    stdout: impl Read,
    request: &ChatRequest,
    stateless: bool,
    fast_requested: bool,
    events: &EventSink,
) -> Result<bool, BrokerFailure> {
    let mut state = StreamState {
        allowed: request
            .tools
            .iter()
            .map(|tool| format!("mcp__kassiber__{}", tool.name))
            .collect(),
        stateless,
        fast_requested,
        provider_session_id: None,
        fast_state: None,
        fast_fell_back: false,
    };
    let mut outcome = None;
    crate::codex::for_each_line(stdout, |line| {
        outcome = state.line(line, events);
        outcome.is_none()
    });
    outcome.map_or(Ok(false), |outcome| outcome.map(|()| true))
}

impl Adapter for Claude {
    fn status(&self, _cwd: &Path) -> Result<ProviderStatus, BrokerFailure> {
        let Some(executable) = resolve_executable(EXECUTABLE) else {
            return Ok(provider_status(
                ProviderId::Claude,
                DISPLAY_NAME,
                ProviderState::MissingExecutable,
                "Install Claude Code, then run `claude login` outside Kassiber.",
            ));
        };
        Ok(self.status_with(&executable))
    }

    fn chat(
        &self,
        request: &ChatRequest,
        cwd: &Path,
        bridge: Option<&ToolBridge>,
        events: &EventSink,
    ) -> Result<(), BrokerFailure> {
        let Some(executable) = resolve_executable(EXECUTABLE) else {
            return Err(BrokerFailure::new("Claude is not installed."));
        };
        self.chat_with(&executable, request, cwd, bridge, events)
    }
}

#[cfg(test)]
mod tests;

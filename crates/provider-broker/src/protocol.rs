//! The JSONL broker protocol (`protocol.ts`): requests, events, option
//! checks, the session-cursor shape and the error redaction every failure
//! goes through before it reaches the event stream.

use std::fmt;
use std::io::Write;
use std::sync::{Arc, LazyLock, Mutex};

use fancy_regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

/// A provider runtime the broker can start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Codex,
    Claude,
    #[serde(rename = "opencode")]
    OpenCode,
    Copilot,
}

/// `PROVIDER_IDS`, in the order an unscoped status request reports them.
pub const PROVIDER_IDS: [ProviderId; 4] = [
    ProviderId::Codex,
    ProviderId::Claude,
    ProviderId::OpenCode,
    ProviderId::Copilot,
];

impl ProviderId {
    /// The wire name (`"codex"`, `"claude"`, `"opencode"`, `"copilot"`).
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderId::Codex => "codex",
            ProviderId::Claude => "claude",
            ProviderId::OpenCode => "opencode",
            ProviderId::Copilot => "copilot",
        }
    }

    /// A known wire name, else `None`.
    pub fn parse(value: &Value) -> Option<Self> {
        let name = value.as_str()?;
        PROVIDER_IDS.into_iter().find(|id| id.as_str() == name)
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failure that ends the request: the TypeScript broker's thrown `Error`.
/// Its message is raw; [`error_event`] redacts it on the way out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokerFailure {
    message: String,
}

impl BrokerFailure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BrokerFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BrokerFailure {}

/// Where a model's requests go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelPosture {
    Local,
    Remote,
    Tee,
}

/// `BrokerModel`. Absent fields are left out of the JSON.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BrokerModel {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privacy_posture: Option<ModelPosture>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privacy_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_reasoning_effort: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_efforts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_tools: Option<bool>,
    /// The model offers a faster, higher-cost serving mode (Codex "Fast"
    /// tier, Claude Opus fast mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_fast_mode: Option<bool>,
    /// Provider wording for that mode, e.g. "2x speed, increased usage".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fast_mode_description: Option<String>,
}

/// A status probe's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderState {
    Ready,
    MissingExecutable,
    AuthenticationRequired,
    Error,
}

/// Every CLI provider is remote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusPosture {
    Remote,
}

/// The providers' own tools are always off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NativeTools {
    Disabled,
}

/// `ProviderStatus`. Build it with [`provider_status`], which stamps the
/// invariants, and the `with_*` methods.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProviderStatus {
    pub provider: ProviderId,
    pub display_name: String,
    pub privacy_posture: StatusPosture,
    pub native_tools: NativeTools,
    pub models: Vec<BrokerModel>,
    pub state: ProviderState,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    executable: Option<String>,
}

/// `providerStatus(provider, display_name, { state, message })`: a remote
/// provider whose native tools are off, with no models yet.
pub fn provider_status(
    provider: ProviderId,
    display_name: &str,
    state: ProviderState,
    message: impl Into<String>,
) -> ProviderStatus {
    ProviderStatus {
        provider,
        display_name: display_name.to_owned(),
        privacy_posture: StatusPosture::Remote,
        native_tools: NativeTools::Disabled,
        models: Vec::new(),
        state,
        message: message.into(),
        version: None,
        executable: None,
    }
}

impl ProviderStatus {
    /// The resolved path identifies the user (`/Users/<name>/.local/bin/...`)
    /// and this object is forwarded verbatim to the UI. The basename is all
    /// the surface needs to say which binary answered.
    pub fn with_executable(mut self, executable: &str) -> Self {
        let leaf = executable.rsplit(['/', '\\']).next().unwrap_or_default();
        self.executable = Some(if leaf.is_empty() { executable } else { leaf }.to_owned());
        self
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn with_models(mut self, models: Vec<BrokerModel>) -> Self {
        self.models = models;
        self
    }

    /// The executable's basename, if one was recorded.
    pub fn executable(&self) -> Option<&str> {
        self.executable.as_deref()
    }
}

/// An image the user attached, inline as base64. Never a path or a URL: the
/// daemon reads the file it holds a grant for and passes the bytes, so a
/// provider cannot be pointed at anything else.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrokerImage {
    pub media_type: String,
    pub data: String,
}

/// `BROKER_IMAGE_TYPES`.
pub const BROKER_IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];
/// The daemon's own per-turn limits are lower; these only bound a bad request.
pub const MAX_BROKER_IMAGES: usize = 16;
pub const MAX_BROKER_IMAGE_CHARS: usize = 8 * 1024 * 1024;

/// One transcript message. Images stay unchecked JSON until
/// [`message_images`] validates them.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub images: Option<Vec<Value>>,
}

impl ChatMessage {
    pub fn new(role: &str, content: &str) -> Self {
        Self {
            role: role.to_owned(),
            content: content.to_owned(),
            images: None,
        }
    }
}

/// `/^[A-Za-z0-9+/]+={0,2}$/`.
fn base64_shaped(data: &str) -> bool {
    let body = data.strip_suffix('=').unwrap_or(data);
    let body = body.strip_suffix('=').unwrap_or(body);
    !body.is_empty()
        && body
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/')
}

/// `messageImages(messages)`: the images of these messages, checked; a
/// malformed one fails the turn.
pub fn message_images<'a>(
    messages: impl IntoIterator<Item = &'a ChatMessage>,
) -> Result<Vec<BrokerImage>, BrokerFailure> {
    let images: Vec<&Value> = messages
        .into_iter()
        .flat_map(|message| message.images.iter().flatten())
        .collect();
    if images.len() > MAX_BROKER_IMAGES {
        return Err(BrokerFailure::new("Too many attached images."));
    }
    images
        .into_iter()
        .map(|image| {
            let media_type = image.get("media_type").and_then(Value::as_str);
            let data = image.get("data").and_then(Value::as_str);
            match (media_type, data) {
                (Some(media_type), Some(data))
                    if BROKER_IMAGE_TYPES.contains(&media_type)
                        && !data.is_empty()
                        && data.len() <= MAX_BROKER_IMAGE_CHARS
                        && base64_shaped(data) =>
                {
                    Ok(BrokerImage {
                        media_type: media_type.to_owned(),
                        data: data.to_owned(),
                    })
                }
                _ => Err(BrokerFailure::new("Invalid attached image.")),
            }
        })
        .collect()
}

/// `BrokerToolDefinition`. The parameter schema stays the JSON text the
/// daemon sent, so its key order survives into the MCP manifest.
#[derive(Debug, Deserialize, Serialize)]
pub struct BrokerToolDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Box<RawValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive: Option<Value>,
}

impl Clone for BrokerToolDefinition {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
            read_only: self.read_only.clone(),
            destructive: self.destructive.clone(),
        }
    }
}

impl BrokerToolDefinition {
    /// A definition with these fields (tests and adapters).
    pub fn new(name: &str, description: &str, parameters: &str) -> Result<Self, BrokerFailure> {
        Ok(Self {
            name: name.to_owned(),
            description: Some(description.to_owned()),
            parameters: RawValue::from_string(parameters.to_owned())
                .map_err(|_| BrokerFailure::new("Invalid tool parameters."))?,
            read_only: None,
            destructive: None,
        })
    }
}

/// `ChatRequest`. Options stay JSON: the option checks below refuse a
/// malformed value when an adapter asks for it, as the TypeScript broker
/// does, rather than failing the whole request up front.
#[derive(Clone, Debug, Deserialize)]
pub struct ChatRequest {
    #[serde(default)]
    pub request_id: Option<Value>,
    pub provider: ProviderId,
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub instructions: Option<Value>,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub tools: Vec<BrokerToolDefinition>,
    #[serde(default)]
    pub options: Option<Value>,
}

fn null_as_empty<'de, D>(deserializer: D) -> Result<Vec<BrokerToolDefinition>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<Vec<BrokerToolDefinition>>::deserialize(deserializer)?.unwrap_or_default())
}

impl ChatRequest {
    /// `request.options?.[name]`: `None` when absent (or when the options are
    /// not an object); a present `null` is `Some(Value::Null)`.
    pub fn option(&self, name: &str) -> Option<&Value> {
        self.options.as_ref()?.as_object()?.get(name)
    }

    /// `request.instructions` when it is a string.
    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_ref()?.as_str()
    }

    /// A chat request built in code (tests and adapters' own tests).
    pub fn new(provider: ProviderId, model: &str, messages: Vec<ChatMessage>) -> Self {
        Self {
            request_id: None,
            provider,
            model: model.to_owned(),
            messages,
            instructions: None,
            tools: Vec::new(),
            options: None,
        }
    }

    /// Replace the options with this JSON object.
    pub fn with_options(mut self, options: Value) -> Self {
        self.options = Some(options);
        self
    }
}

/// JavaScript truthiness of a JSON value.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// A boolean option: absent is `false`, anything but a boolean is refused.
fn boolean_option(request: &ChatRequest, name: &str, refusal: &str) -> Result<bool, BrokerFailure> {
    match request.option(name) {
        None => Ok(false),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(_) => Err(BrokerFailure::new(refusal)),
    }
}

/// `requestedFastMode(request)`: whether the turn asked for the model's
/// fast serving mode. Adapters apply it only for models that advertise
/// `supports_fast_mode`; anything that is not a boolean is rejected rather
/// than guessed.
pub fn requested_fast_mode(request: &ChatRequest) -> Result<bool, BrokerFailure> {
    boolean_option(request, "fast_mode", "Invalid fast mode option.")
}

/// `sensitiveContext(request)`: a selected-data grant is a single stateless
/// exchange, never an agent session.
pub fn sensitive_context(request: &ChatRequest) -> Result<bool, BrokerFailure> {
    let value = boolean_option(
        request,
        "sensitive_context",
        "Invalid sensitive context mode.",
    )?;
    if value
        && (!request.tools.is_empty() || request.option("provider_session_id").is_some_and(truthy))
    {
        return Err(BrokerFailure::new(
            "Sensitive context cannot use tools or resume a provider session.",
        ));
    }
    Ok(value)
}

/// `statelessSession(request)`: an Incognito turn. The provider is asked not
/// to save its session (Codex ephemeral thread, Claude
/// `--no-session-persistence`, OpenCode in-memory database), and no session
/// cursor goes in or comes back, so the next turn rebuilds from the visible
/// transcript. Unlike selected-data turns it keeps tools and ordinary error
/// reporting: only persistence changes.
pub fn stateless_session(request: &ChatRequest) -> Result<bool, BrokerFailure> {
    let value = boolean_option(
        request,
        "stateless_session",
        "Invalid stateless session option.",
    )?;
    if value && request.option("provider_session_id").is_some_and(truthy) {
        return Err(BrokerFailure::new(
            "A stateless session cannot resume a provider session.",
        ));
    }
    Ok(value)
}

/// `statusScope(request)`: which runtimes a status request may start:
/// `provider`, else `providers`, else all of them. Unknown names are refused
/// rather than ignored, so a malformed scope can never widen to every
/// runtime. A present `null` counts as given, as it does in JavaScript.
pub fn status_scope(request: &Map<String, Value>) -> Result<Vec<ProviderId>, BrokerFailure> {
    let unknown = || BrokerFailure::new("Unknown provider.");
    if let Some(provider) = request.get("provider") {
        return ProviderId::parse(provider)
            .map(|id| vec![id])
            .ok_or_else(unknown);
    }
    if let Some(providers) = request.get("providers") {
        let Value::Array(providers) = providers else {
            return Err(unknown());
        };
        let mut scope = Vec::new();
        for provider in providers {
            let id = ProviderId::parse(provider).ok_or_else(unknown)?;
            if !scope.contains(&id) {
                scope.push(id);
            }
        }
        return Ok(scope);
    }
    Ok(PROVIDER_IDS.to_vec())
}

/// The `code` of an `error` event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    MissingExecutable,
    AuthenticationRequired,
    ProviderError,
}

/// `BrokerEvent`: one JSONL line on stdout. Fields keep the TypeScript
/// broker's order; absent optional fields are left out.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Status {
        phase: String,
        message: String,
    },
    Delta {
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
    },
    ToolCall {
        call_id: String,
        name: String,
        /// The arguments as the provider sent them, whitespace removed.
        arguments: Box<RawValue>,
    },
    Done {
        finish_reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider_session_id: Option<String>,
    },
    Error {
        code: ErrorCode,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
    },
    Result {
        data: Box<RawValue>,
    },
}

impl Event {
    /// A `result` event carrying `data`.
    pub fn result(data: &impl Serialize) -> Result<Self, BrokerFailure> {
        serde_json::value::to_raw_value(data)
            .map(|data| Event::Result { data })
            .map_err(|_| BrokerFailure::new("Kassiber could not encode the broker result."))
    }
}

/// `writeEvent`: one JSON line per event, flushed, shared by every thread
/// of the turn (the adapter and the tool bridge).
#[derive(Clone)]
pub struct EventSink {
    out: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl EventSink {
    /// The process's standard output.
    pub fn stdout() -> Self {
        Self::new(Box::new(std::io::stdout()))
    }

    pub fn new(out: Box<dyn Write + Send>) -> Self {
        Self {
            out: Arc::new(Mutex::new(out)),
        }
    }

    /// A sink that keeps what it is given, for tests.
    pub fn capture() -> (Self, Captured) {
        let captured = Captured::default();
        (Self::new(Box::new(captured.clone())), captured)
    }

    /// Write one event. A closed stdout means the daemon is gone; the turn
    /// ends on its own, so a write failure is not reported.
    pub fn write(&self, event: &Event) {
        let Ok(mut line) = serde_json::to_vec(event) else {
            return;
        };
        line.push(b'\n');
        let mut out = self.out.lock().unwrap_or_else(|poison| poison.into_inner());
        let _ = out.write_all(&line);
        let _ = out.flush();
    }
}

/// What a [`EventSink::capture`] sink was given.
#[derive(Clone, Default)]
pub struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    /// Every event written so far, parsed.
    pub fn events(&self) -> Vec<Value> {
        self.text()
            .lines()
            .map(|line| serde_json::from_str(line).expect("each event is one JSON line"))
            .collect()
    }

    /// The raw output.
    pub fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap_or_else(|p| p.into_inner()).clone())
            .expect("events are UTF-8")
    }
}

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Valid JSON text without its insignificant whitespace, so a value passed
/// through keeps its key order and cannot break the line framing.
pub fn minify_json(raw: &RawValue) -> Box<RawValue> {
    let mut out = String::with_capacity(raw.get().len());
    let mut in_string = false;
    let mut escaped = false;
    for c in raw.get().chars() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if !matches!(c, ' ' | '\t' | '\n' | '\r') {
            out.push(c);
        }
    }
    // Removing whitespace between tokens of valid JSON keeps it valid.
    RawValue::from_string(out).unwrap_or_else(|_| raw.to_owned())
}

/// Accept a provider session cursor only if it looks like one.
///
/// These cursors are echoed back to us from a prior turn and then handed to
/// a CLI. A value starting with `-` would be parsed as a new top-level flag
/// rather than an id, which turns a resumed chat into arbitrary CLI
/// configuration (`--dangerously-skip-permissions`, `--mcp-config=…`,
/// `--debug-file=…`). The providers all issue id-shaped cursors, so anything
/// outside that shape is dropped and the turn simply starts a fresh provider
/// session.
pub fn safe_session_cursor(value: Option<&Value>) -> Option<String> {
    let trimmed = js_trim(value?.as_str()?);
    if trimmed.is_empty() || trimmed.encode_utf16().count() > 200 {
        return None;
    }
    let mut bytes = trimmed.bytes();
    let first = bytes.next()?;
    (first.is_ascii_alphanumeric()
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte)))
    .then(|| trimmed.to_owned())
}

/// JavaScript's `\s` (and `String.prototype.trim`'s) whitespace: WhiteSpace
/// plus LineTerminator.
pub fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `String.prototype.trim`.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(js_space)
}

/// The JavaScript `\s` class body, for patterns.
const SPACE: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// An ASCII word matched case-insensitively, as JavaScript's `i` flag does
/// outside Unicode mode (a non-ASCII letter never folds to an ASCII one).
fn ascii_ci(word: &str) -> String {
    word.chars()
        .map(|c| {
            if c.is_ascii_alphabetic() {
                format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase())
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// JavaScript's `\b` before a word character: the previous character is not
/// an ASCII word character.
const WORD_START: &str = r"(?<![A-Za-z0-9_])";

struct Redaction {
    pattern: Regex,
    replacement: &'static str,
}

static REDACTIONS: LazyLock<Vec<Redaction>> = LazyLock::new(|| {
    let labels = [
        "bearer",
        "token",
        "api[_ -]?key",
        "authorization",
        "secret",
        "password",
        "passphrase",
    ]
    .iter()
    .map(|label| ascii_ci(label))
    .collect::<Vec<_>>()
    .join("|");
    let rule = |pattern: String, replacement: &'static str| Redaction {
        pattern: Regex::new(&pattern).expect("redaction pattern"),
        replacement,
    };
    vec![
        // URL userinfo, before anything else rewrites the URL.
        rule(format!(r"://[^/@{SPACE}]+@"), "://[redacted]@"),
        // JSON or quoted assignment: "apiKey": "secret". The value may open
        // with a scheme (`token: Bearer x`, `token: Basic 'x'`), which is
        // taken with the credential after it: otherwise the scheme word
        // would be the whole value and the credential would stay. A quoted
        // credential runs to its closing quote, or to the end of the line
        // when it has none, so spaces, commas or a missing quote cannot
        // leave it in place.
        rule(
            format!(
                r#"(["']?(?:{labels})["']?[{SPACE}]*[:=][{SPACE}]*)(?:(?:{}|{})[{SPACE}]+)?(?:(["'])(?:(?!\2).)*\2?|[^"',;{SPACE}]+)"#,
                ascii_ci("bearer"),
                ascii_ci("basic")
            ),
            "${1}[redacted]",
        ),
        // A bare scheme + credential, whether or not a label preceded it.
        rule(
            format!(
                r"{WORD_START}({}|{})[{SPACE}]+[^{SPACE}]+",
                ascii_ci("bearer"),
                ascii_ci("basic")
            ),
            "${1} [redacted]",
        ),
        // Vendor-shaped tokens standing alone.
        rule(
            format!(r"{WORD_START}(sk|pk|rk)-[A-Za-z0-9_-]{{8,}}"),
            "[redacted]",
        ),
        rule(
            format!(r"{WORD_START}(gh[pousr]|xox[abposr])_[A-Za-z0-9_-]{{8,}}"),
            "[redacted]",
        ),
        // Home directories identify the user; keep the leaf for orientation.
        rule(format!(r"/(?:Users|home)/[^/{SPACE}]+"), "~"),
    ]
});

/// The first `units` UTF-16 code units, as `String.prototype.slice(0, units)`
/// counts them. A surrogate pair the cut would split is dropped whole.
pub(crate) fn first_utf16_units(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (index, c) in text.char_indices() {
        used += c.len_utf16();
        if used > units {
            return &text[..index];
        }
    }
    text
}

/// Sanitize a provider failure for the event stream.
///
/// Provider CLIs put credentials, URLs and absolute home paths in their error
/// text, so every pattern below has to be handled, not just `key: value`:
/// a bare `Bearer <token>` after a redacted label, JSON `"apiKey": "..."`,
/// `https://user:pass@host`, `sk-`/`ghp_`-style tokens, and `/Users/<name>` or
/// `/home/<name>` prefixes. Each rule replaces the *secret*, not just the
/// label — an earlier version rewrote `authorization: Bearer sk-x` to
/// `authorization=[redacted] sk-x`, leaving the token in place.
pub fn safe_error_message(raw: &str) -> String {
    let first = match raw.find('\n') {
        Some(end) => raw[..end].strip_suffix('\r').unwrap_or(&raw[..end]),
        None => raw,
    };
    let first = js_trim(first);
    let mut text = if first.is_empty() {
        "Provider request failed.".to_owned()
    } else {
        first.to_owned()
    };
    for redaction in REDACTIONS.iter() {
        if let Ok(replaced) = redaction
            .pattern
            .try_replacen(&text, 0, redaction.replacement)
        {
            text = replaced.into_owned();
        } else {
            // A pattern that cannot finish is not a reason to leak: say
            // nothing of the failure instead.
            return "Provider request failed.".to_owned();
        }
    }
    first_utf16_units(&text, 400).to_owned()
}

/// `errorCode(message)` in `index.ts`.
pub fn error_code(message: &str) -> ErrorCode {
    let lower = message.to_ascii_lowercase();
    if ["not installed", "not found", "enoent"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return ErrorCode::MissingExecutable;
    }
    if ["auth", "login", "unauthor", "credential"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return ErrorCode::AuthenticationRequired;
    }
    ErrorCode::ProviderError
}

/// The `error` event `index.ts` writes for a failed request: the redacted
/// message, or a fixed message and hint when the provider wants a login.
pub fn error_event(failure: &BrokerFailure) -> Event {
    let message = safe_error_message(failure.message());
    let code = error_code(&message);
    let authentication = code == ErrorCode::AuthenticationRequired;
    Event::Error {
        code,
        message: if authentication {
            "The provider requires authentication.".to_owned()
        } else {
            message
        },
        hint: authentication
            .then(|| "Run the provider's normal login command outside Kassiber.".to_owned()),
    }
}

#[cfg(test)]
mod tests;

//! The assistant's data as the web keeps it (`daemon/stream.ts` `AiChatMessage`,
//! `AiChatToolCall`, `AiToolConsentRequest`; `lib/aiCapabilities.ts` provider rows; the saved-chat
//! rows of `ui.chat.sessions.list`).
//!
//! Messages come from a running turn (`chat.rs`), a saved chat (`ui.chat.sessions.get`) or a
//! SYNTHETIC scene (`synthetic.rs`).

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgStatus {
    Pending,
    Streaming,
    Done,
    Error,
    Cancelled,
}

/// What a sent message shows about its file: never a path or a token.
#[derive(Clone, Debug, Default)]
pub struct AttachmentView {
    pub filename: String,
    pub kind: Option<String>,
    pub size_bytes: Option<u64>,
}

/// `AiChatThinkingSegment`: one provider completion round's reasoning.
#[derive(Clone, Debug, Default)]
pub struct ThinkingSegment {
    pub id: String,
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    AwaitingConsent,
    Running,
    Done,
    Denied,
    Error,
}

/// `AiChatToolCall`.
#[derive(Clone, Debug)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
    /// `read_only` | `mutating` | `unknown`. Kept as the web's message type keeps it; like the
    /// web, nothing renders it (mutation detection reads the observed call, `stream.rs`).
    #[allow(dead_code)]
    pub kind_class: String,
    pub needs_consent: bool,
    pub status: ToolStatus,
    /// `required` | `auto` | `full_access` | `session` | `not_needed`.
    pub approval: Option<String>,
    pub result: Option<Value>,
    pub reason: Option<String>,
    /// Server-revalidated effects (`AiReviewPreview`), kept as the daemon sends it.
    pub review_preview: Option<Value>,
    pub segment_id: Option<String>,
}

/// `AiChatMessage`.
#[derive(Clone, Debug)]
pub struct Msg {
    pub id: String,
    pub role: Role,
    pub content: String,
    pub thinking_segments: Vec<ThinkingSegment>,
    pub activity_label: Option<String>,
    /// Provider notices by status phase (e.g. `fast_mode_unavailable`).
    pub notices: Vec<String>,
    pub tool_calls: Vec<ToolCall>,
    pub status: MsgStatus,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub finish_reason: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    /// `AiAnswerProvenance`, kept as the daemon sends it.
    pub provenance: Option<Value>,
    pub attachments: Vec<AttachmentView>,
}

impl Msg {
    pub fn new(id: impl Into<String>, role: Role, content: impl Into<String>) -> Msg {
        Msg {
            id: id.into(),
            role,
            content: content.into(),
            thinking_segments: vec![],
            activity_label: None,
            notices: vec![],
            tool_calls: vec![],
            status: MsgStatus::Done,
            error_code: None,
            error_message: None,
            finish_reason: None,
            provider: None,
            model: None,
            provenance: None,
            attachments: vec![],
        }
    }
}

/// `AiToolConsentRequest`: an approval the running answer waits on.
#[derive(Clone, Debug)]
pub struct Consent {
    pub call_id: String,
    pub name: String,
    pub summary: String,
    pub arguments_preview: Value,
    pub review_preview: Option<Value>,
    pub reversible: bool,
    pub egresses: bool,
    pub can_allow_session: bool,
}

/// A file of this chat as the composer shows it.
#[derive(Clone, Debug)]
pub struct ChatFile {
    pub key: String,
    pub view: AttachmentView,
    /// `uploading` | `ready` | `sent`.
    pub state: FileState,
    /// The book it was chosen in (`StagedAttachment.book`).
    pub book: Option<super::chat::Book>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    Uploading,
    Ready,
    Sent,
}

/// A row of `ui.chat.sessions.list`.
#[derive(Clone, Debug)]
pub struct SessionRow {
    pub id: String,
    pub title: String,
    pub updated_at: String,
    pub message_count: u64,
}

/// `ui.chat.sessions.list` data.
#[derive(Clone, Debug, Default)]
pub struct SessionList {
    pub sessions: Vec<SessionRow>,
    pub history_enabled: bool,
}

impl SessionList {
    pub fn parse(v: &Value) -> SessionList {
        let sessions = v["sessions"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|s| SessionRow {
                        id: s["id"].as_str().unwrap_or_default().to_owned(),
                        title: s["title"].as_str().unwrap_or_default().to_owned(),
                        updated_at: s["updated_at"].as_str().unwrap_or_default().to_owned(),
                        message_count: s["message_count"].as_u64().unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default();
        SessionList {
            sessions,
            history_enabled: v["history_enabled"].as_bool().unwrap_or(false),
        }
    }
}

/// A row of `ai.providers.list` (`AiProviderRow`). `raw` keeps every field for the picker.
#[derive(Clone, Debug)]
pub struct Provider {
    pub name: String,
    pub display_name: Option<String>,
    pub base_url: String,
    pub default_model: Option<String>,
    /// `local` | `remote` | `tee`.
    pub kind: String,
    pub acknowledged_at: Option<String>,
    pub enabled: bool,
    pub supports_reasoning_effort: bool,
    pub is_default: bool,
    pub last_checked_at: Option<String>,
    pub raw: Value,
}

impl Provider {
    pub fn parse_list(v: &Value) -> Vec<Provider> {
        v["providers"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| Provider {
                        name: p["name"].as_str().unwrap_or_default().to_owned(),
                        display_name: p["display_name"].as_str().map(str::to_owned),
                        base_url: p["base_url"].as_str().unwrap_or_default().to_owned(),
                        default_model: p["default_model"].as_str().map(str::to_owned),
                        kind: p["kind"].as_str().unwrap_or("local").to_owned(),
                        acknowledged_at: p["acknowledged_at"].as_str().map(str::to_owned),
                        enabled: p["enabled"].as_bool().unwrap_or(true),
                        supports_reasoning_effort: p["supports_reasoning_effort"]
                            .as_bool()
                            .unwrap_or(false),
                        is_default: p["is_default"].as_bool().unwrap_or(false),
                        last_checked_at: p["last_checked_at"].as_str().map(str::to_owned),
                        raw: p.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `providerDisplayName`.
    pub fn label(&self) -> String {
        self.display_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| self.name.clone())
    }

    /// `isNativeAiProviderLocator`: a coding-agent CLI (`claude-cli://`, `codex-cli://` …).
    pub fn is_cli(&self) -> bool {
        is_cli_locator(&self.base_url)
    }
}

pub fn is_cli_locator(url: &str) -> bool {
    [
        "claude-cli://",
        "codex-cli://",
        "copilot-cli://",
        "opencode-cli://",
    ]
    .iter()
    .any(|p| url.starts_with(p))
}

/// Parses a saved chat (`ui.chat.sessions.get` → `messages`) into the thread's messages, as
/// `storedChatEntries` + `loadConversation` do: user and assistant turns with their content (and
/// notices); tool rounds are not stored.
pub fn stored_messages(v: &Value) -> Vec<Msg> {
    let mut out = vec![];
    for (i, m) in v["messages"].as_array().into_iter().flatten().enumerate() {
        let role = match m["role"].as_str() {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => continue,
        };
        let content = m["content"].as_str().unwrap_or_default();
        if content.is_empty() {
            continue;
        }
        let mut msg = Msg::new(format!("stored-{i}"), role, content);
        let prov = &m["provenance"];
        if role == Role::Assistant {
            msg.notices = prov["notices"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|n| n.as_str().map(str::to_owned))
                .collect();
        } else {
            msg.attachments = prov["attachments"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| {
                    let filename = a["filename"].as_str().filter(|f| !f.is_empty())?.to_owned();
                    Some(AttachmentView {
                        filename,
                        kind: a["kind"].as_str().map(str::to_owned),
                        size_bytes: a["size_bytes"].as_u64(),
                    })
                })
                .collect();
        }
        out.push(msg);
    }
    out
}

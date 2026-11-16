//! The Assistant (`/assistant`) and the right-hand assistant panel.
//!
//! Registered in `main.rs` (view, poll, keys, scenes) and `app.rs` (state, replies, stream
//! records) through the functions below; everything else for this area lives under
//! `src/assistant/`. The conversation engine is `chat.rs` (`ai.chat` streaming, stop, approvals,
//! permission mode, queue, branch/edit, saved chats, book binding).
//!
//! Privacy: mounting reads only what the daemon already has (`ai.providers.list`, cached-only
//! check results, `ui.chat.sessions.*`). Everything that contacts a provider runs only from the
//! user's own action, as in the web: a sent question (`ai.chat`), **Check models**
//! (`ai.list_models` with `refresh`), **Use** on the setup card (`ai.provider_runtime.status`
//! with `refresh`), and an off-device provider's acknowledgement after its confirmation. With
//! AI features off nothing here sends an AI kind; turning them off stops the running answer and
//! drops the conversation, as unmounting the web's session provider does.
//!
//! Web sources: routes/Assistant.tsx, components/ai-02.tsx, components/ai/**,
//! components/ai-elements/**, components/kb/assistant-panel/**.

pub mod chat;
pub mod checks;
pub mod closed;
pub mod composer;
pub mod empty;
pub mod evidence;
pub mod model;
pub mod page;
pub mod panel;
pub mod picker;
pub mod screen;
pub mod stream;
pub mod synthetic;
pub mod think;
pub mod thread;

use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx};
use serde_json::{Value, json};

use crate::app::{App, Load, Reply};
use crate::theme::Tk;
use model::{ChatFile, Consent, Msg, Provider, SessionList, SessionRow};

/// `AiPermissionMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionMode {
    Approval,
    Auto,
    FullAccess,
}

impl PermissionMode {
    pub const ALL: [PermissionMode; 3] = [
        PermissionMode::Approval,
        PermissionMode::Auto,
        PermissionMode::FullAccess,
    ];
    pub fn id(self) -> &'static str {
        match self {
            PermissionMode::Approval => "approval",
            PermissionMode::Auto => "auto",
            PermissionMode::FullAccess => "full_access",
        }
    }
}

/// Which overlay of this area is open (one at a time, as Radix menus are).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    /// The page's "More actions" (…) menu.
    More,
    /// The same menu with its History submenu open.
    History,
    /// The panel tab bar's … menu (Clear chat).
    PanelMore,
    /// The panel's `+` (Open in side panel) menu.
    PanelAdd,
}

/// The panel's own surfaces (`RightPanelSurface` without the review queue, which still opens
/// its page): both are developer-tools surfaces, like their pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Changes,
    Logs,
}

impl Surface {
    pub fn label(self) -> String {
        match self {
            Surface::Changes => crate::i18n::t("assistant:panel.surfaceChanges"),
            Surface::Logs => crate::i18n::t("assistant:panel.surfaceLogs"),
        }
    }
    pub fn icon(self) -> &'static rune::Icon {
        match self {
            Surface::Changes => &crate::icons::GIT_COMPARE_ARROWS,
            Surface::Logs => &crate::icons::SCROLL_TEXT,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Surface::Changes => "changes",
            Surface::Logs => "logs",
        }
    }
}

/// A tab of the right panel (`RightPanelTab`): the assistant, a surface, or the transaction
/// details `txdetail` keeps (its own tabs live in `app.detail.tabs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Front {
    Assistant,
    Surface(Surface),
    Detail,
}

/// The item "Ask about this" pinned (`AssistantFocus` + its book): the typed context replaces
/// the page's while it is pinned; the label stays on the local chip and is never sent.
#[derive(Clone, Debug)]
pub struct Focus {
    pub label: String,
    pub context: Value,
    pub book: chat::Book,
}

/// This area's UI state (`app.assistant`): the web's `AssistantSessionProvider` state plus the
/// UI store's assistant fields.
pub struct Ui {
    // ---- The conversation (AssistantSessionContextValue) ----
    pub messages: Vec<Msg>,
    pub pending_consents: Vec<Consent>,
    pub error: Option<String>,
    pub streaming: bool,
    /// Questions held while an answer streams (or the book is loading).
    pub queued: Vec<chat::Queued>,
    pub session_id: Option<String>,
    pub incognito: bool,
    pub files: Vec<ChatFile>,
    pub focus: Option<Focus>,
    /// `assistantModelSelection`: (provider, model).
    pub selection: Option<(String, String)>,
    /// `AssistantThinkingEffort` id (`auto`, `low` … `ultra`).
    pub effort: String,
    pub fast_mode: bool,
    pub permission: PermissionMode,
    /// `useAssistantDraftStore`: the composer text, shared by the page and the panel.
    pub draft: TextEdit,
    /// A page's handoff parked with the draft (`ParkedHandoff`): its page context and book,
    /// kept while the draft has text, sent as that handoff.
    pub parked: Option<chat::Parked>,
    // ---- The engine (useAiChatStream / AssistantSessionProvider refs) ----
    pub turn: Option<chat::Turn>,
    pub in_flight: chat::InFlight,
    /// The stopped turn's answer, marked cancelled by `abort`.
    pub cancelled_turn: Option<String>,
    /// The book the conversation belongs to (`conversationBookRef`).
    pub book: Option<chat::Book>,
    /// The next send persists the seeded prefix (an explicit branch or edit).
    pub seed_pending: bool,
    /// Tokens a sent message already showed.
    pub shown: std::collections::HashSet<String>,
    /// `assistantChatPermissionModes`: the mode each saved chat last used.
    pub chat_modes: std::collections::HashMap<String, PermissionMode>,
    /// The last page the user was on (the assistant page answers about it).
    pub last_screen: Option<Value>,
    /// Bumped by every fresh conversation (`generationRef`) and every submit (`promptRevisionRef`).
    pub generation: u64,
    pub prompt_revision: u64,
    pub turn_generation: u64,
    pub turn_revision: u64,
    /// The reasoning level was picked by the user (else a provider default may replace it).
    pub effort_chosen: bool,
    pub effort_seed_for: Option<(String, Option<chat::Book>)>,
    /// A saved chat being reopened: the book and conversation it was asked from.
    pub resume_book: Option<chat::Book>,
    pub resume_generation: u64,
    pub evidence: evidence::State,
    pub checks: checks::State,
    /// The answer being read aloud (its key) and the system voice process.
    pub speaking: Option<(String, std::process::Child)>,
    // ---- The right panel (useUiStore / useRightPanelStore) ----
    /// `assistantPanelOpen`: the assistant is one of the panel's tabs.
    pub tab_open: bool,
    /// The panel's own surfaces, in the order they were opened (`surfaces`).
    pub surfaces: Vec<Surface>,
    /// The tab in front (`activeTab`); kept while the panel is hidden, so ⌘I and the title-bar
    /// toggle bring the panel back as it was. A tab that is gone falls back to the first one.
    pub front: Front,
    /// Where the front goes back to when the open transaction closes (`returnTo`).
    pub detail_return: Front,
    /// The open transaction revealed a hidden panel (`openedFor`): closing it hides it again.
    pub opened_for: bool,
    /// The transaction the detail panel showed at the last poll (a newly opened one comes to
    /// the front of the panel, as the web's `setActiveTab(detail)` does).
    pub seen_detail: Option<String>,
    /// The tab chip under the pointer (its icon turns into the close mark).
    pub tab_hover: Option<Front>,
    // ---- Overlays and reads ----
    pub menu: Option<Menu>,
    pub providers: Load<Vec<Provider>>,
    /// Cached-only runtime checks of the coding agents (`ModelSetupCard` status lines).
    pub runtime: Vec<(String, Value)>,
    /// The History submenu's list (`limit: 30`).
    pub sessions: Load<SessionList>,
    /// `RecentChats` (`limit: 3`).
    pub recent: Vec<SessionRow>,
    pub resume_error: Option<String>,
    /// Mount reads already sent (providers, recent chats).
    pub mounted: bool,
    pub recent_requested: bool,
    /// A SYNTHETIC scene filled the conversation.
    pub synthetic: bool,
    /// A file is dragged over the composer (the "Drop to attach" hint).
    pub thread_drop: bool,
    /// The recalled prompt's index among this conversation's user prompts (ArrowUp/Down).
    pub recall: Option<usize>,
    pub next_file: u64,
    /// Files being handed to the daemon, first one in flight.
    pub uploads: Vec<composer::Upload>,
    pub upload_busy: bool,
    pub thread: thread::State,
    pub picker: picker::State,
    // ---- Below the page (AssistantLayout) ----
    /// The closed bar's compact composer is unfolded (focus or the pointer in it).
    pub bar_unfolded: bool,
    pub bar_hover: bool,
    /// Approvals already announced, and the newest approval / failure already acted on.
    pub announced: std::collections::HashSet<String>,
    pub seen_approval: Option<String>,
    pub seen_error: Option<String>,
}

impl Default for Ui {
    fn default() -> Self {
        Ui {
            messages: vec![],
            pending_consents: vec![],
            error: None,
            streaming: false,
            queued: vec![],
            session_id: None,
            incognito: false,
            files: vec![],
            focus: None,
            selection: None,
            effort: "auto".into(),
            fast_mode: false,
            // DEFAULT_ASSISTANT_PERMISSION_MODE.
            permission: PermissionMode::Auto,
            draft: TextEdit::new(""),
            parked: None,
            turn: None,
            in_flight: Default::default(),
            cancelled_turn: None,
            book: None,
            seed_pending: false,
            shown: Default::default(),
            chat_modes: Default::default(),
            last_screen: None,
            generation: 0,
            prompt_revision: 0,
            turn_generation: 0,
            turn_revision: 0,
            effort_chosen: false,
            effort_seed_for: None,
            resume_book: None,
            resume_generation: 0,
            evidence: Default::default(),
            checks: Default::default(),
            speaking: None,
            tab_open: false,
            surfaces: vec![],
            front: Front::Assistant,
            detail_return: Front::Assistant,
            opened_for: false,
            seen_detail: None,
            tab_hover: None,
            menu: None,
            providers: Load::Idle,
            runtime: vec![],
            sessions: Load::Idle,
            recent: vec![],
            resume_error: None,
            mounted: false,
            recent_requested: false,
            synthetic: false,
            thread_drop: false,
            recall: None,
            next_file: 0,
            uploads: vec![],
            upload_busy: false,
            thread: Default::default(),
            picker: Default::default(),
            bar_unfolded: false,
            bar_hover: false,
            announced: Default::default(),
            seen_approval: None,
            seen_error: None,
        }
    }
}

impl Ui {
    /// `ai.providers.list` failed: sends are not held for a seed that will not come.
    pub fn providers_failed(&self) -> bool {
        matches!(self.providers, Load::Failed(_))
    }
}

/// Whether this area renders `path` (the route without its query).
pub fn owns(path: &str) -> bool {
    path == "/assistant"
}

/// The page for a route this area owns. Overlays (menus, dialogs, sheets) go into `layer`.
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    page::view(app, cx, tk, layer)
}

/// The panel exists at all (`AssistantLayout` mounts only with AI features on); otherwise
/// `txdetail` keeps drawing its details on its own, as before.
pub fn hosts_panel(app: &App) -> bool {
    app.prefs.ai_features && !app.locked
}

fn on_assistant_page(app: &App) -> bool {
    owns(app.route.path())
}

/// `assistantColumnClaimed`: the expanded Transactions table holds the whole column
/// (useAssistantColumnClaim), so the panel and the closed bar step aside until the user asks
/// for the assistant, a turn fails or a tool waits for approval.
pub fn column_claimed(app: &App) -> bool {
    app.route.path() == "/transactions" && app.txs.expanded
}

/// The page holding the column gives it back (`release` on a reveal request).
fn release_column(app: &mut App) {
    if column_claimed(app) {
        app.txs.expanded = false;
    }
}

/// The panel holds only what a page opens (a transaction): on the full assistant page, and
/// while a page holds the column.
fn detail_only(app: &App) -> bool {
    on_assistant_page(app) || column_claimed(app)
}

/// The panel's tabs in the web's order: the assistant, the surfaces developer tools allow, then
/// the open transaction's details. The full assistant page shows only a detail it opens
/// (`AssistantLayout`: `showPanel = panelOpen && (hasDetail || enabled)`); the other tabs wait.
pub fn tabs(app: &App) -> Vec<Front> {
    let mut v = Vec::new();
    let page = detail_only(app);
    if app.assistant.tab_open && !page {
        v.push(Front::Assistant);
    }
    if app.prefs.developer_tools && !page {
        v.extend(app.assistant.surfaces.iter().map(|s| Front::Surface(*s)));
    }
    if app.detail.open.is_some() {
        v.push(Front::Detail);
    }
    v
}

/// The tab in front: the remembered one while it exists, else the first tab.
pub fn front(app: &App) -> Option<Front> {
    pick_front(&tabs(app), app.assistant.front)
}

/// Whether the right panel is on screen. On the full assistant page (the assistant itself) it
/// shows only other tabs, never the assistant or the launcher.
pub fn panel_shown(app: &App) -> bool {
    app.prefs.right_panel_open && hosts_panel(app) && (!detail_only(app) || !tabs(app).is_empty())
}

/// The assistant's conversation is on screen in the panel.
pub fn assistant_in_view(app: &App) -> bool {
    panel_shown(app) && front(app) == Some(Front::Assistant)
}

/// The open transaction's details are on screen (they take keys only then).
pub fn detail_in_view(app: &App) -> bool {
    if !hosts_panel(app) {
        return app.detail.open.is_some();
    }
    panel_shown(app) && front(app) == Some(Front::Detail)
}

/// A surface is one of the panel's tabs (shown or not: tabs keep their state behind others).
pub fn surface_open(app: &App, s: Surface) -> bool {
    hosts_panel(app) && app.prefs.developer_tools && app.assistant.surfaces.contains(&s)
}

/// Brings a tab to the front.
pub fn set_front(app: &mut App, f: Front) {
    if f != Front::Detail {
        app.assistant.opened_for = false;
    }
    app.assistant.front = f;
}

/// `openSurface`: adds the surface (once) and brings it to the front of a shown panel.
pub fn open_surface(app: &mut App, s: Surface) {
    if !app.prefs.developer_tools {
        return;
    }
    if !app.assistant.surfaces.contains(&s) {
        app.assistant.surfaces.push(s);
    }
    set_front(app, Front::Surface(s));
    app.prefs.right_panel_open = true;
    app.assistant.menu = None;
    app.detail.add_open = false;
}

/// `closeSurface`: its neighbour comes forward, or the assistant (and from there the first tab).
pub fn close_surface(app: &mut App, s: Surface) {
    let Some(i) = app.assistant.surfaces.iter().position(|x| *x == s) else {
        return;
    };
    app.assistant.surfaces.remove(i);
    if app.assistant.front == Front::Surface(s) {
        let n = app.assistant.surfaces.len();
        app.assistant.front = if n == 0 {
            Front::Assistant
        } else {
            Front::Surface(app.assistant.surfaces[i.min(n - 1)])
        };
    }
}

/// Hides the panel, keeping its tabs (`setRightPanelOpen(false)` also restores a maximized panel).
pub fn hide_panel(app: &mut App) {
    app.prefs.right_panel_open = false;
    app.detail.maximized = false;
    app.assistant.menu = None;
    app.detail.add_open = false;
}

/// Whether the title-bar side-panel button shows: wherever the panel can be (AI features on),
/// except on the full assistant page while the panel has nothing but the assistant to offer
/// (the web hides the toggle there; it returns once a transaction or a surface is a tab).
pub fn toggle_available(app: &App) -> bool {
    hosts_panel(app) && (!on_assistant_page(app) || !tabs(app).is_empty())
}

/// ⌘I / Ctrl+I and the title-bar side-panel button (owner's request, replacing the web's
/// assistant-only toggle): hide the whole panel when it is shown; otherwise show it again with
/// the tab that was in front. With no tab to come back to it opens on the Assistant. On the full
/// assistant page the panel holds only other tabs, so there it shows them or does nothing.
pub fn toggle_side_panel(app: &mut App) {
    if !hosts_panel(app) {
        return;
    }
    // While a page holds the column the panel is out of view, so the toggle shows it (the
    // page gives the column back) rather than closing something unseen.
    if column_claimed(app) {
        open_panel(app);
        app.assistant.draft.focused = true;
        return;
    }
    let has_tabs = !tabs(app).is_empty();
    match toggle_step(panel_shown(app), has_tabs, on_assistant_page(app)) {
        Toggle::Hide => hide_panel(app),
        Toggle::Nothing => {}
        Toggle::OpenAssistant => {
            open_panel(app);
            app.assistant.draft.focused = true;
        }
        Toggle::Restore => {
            app.assistant.menu = None;
            app.prefs.right_panel_open = true;
            if front(app) == Some(Front::Assistant) {
                app.assistant.draft.focused = true;
            }
        }
    }
}

/// What the side-panel toggle does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Hide,
    /// Show the panel again with its remembered front tab.
    Restore,
    /// No tab to come back to: open on the Assistant.
    OpenAssistant,
    Nothing,
}

/// The toggle's decision: hide a shown panel; else bring back its tabs; else the Assistant
/// (which the full assistant page never shows in the panel).
pub fn toggle_step(shown: bool, has_tabs: bool, on_assistant_page: bool) -> Toggle {
    match (shown, has_tabs, on_assistant_page) {
        (true, _, _) => Toggle::Hide,
        (false, true, _) => Toggle::Restore,
        (false, false, true) => Toggle::Nothing,
        (false, false, false) => Toggle::OpenAssistant,
    }
}

/// The tab in front among `tabs`: the remembered one while it exists, else the first.
pub fn pick_front(tabs: &[Front], remembered: Front) -> Option<Front> {
    if tabs.contains(&remembered) {
        Some(remembered)
    } else {
        tabs.first().copied()
    }
}

/// Every frame: start the reads the current route needs (local reads only), advance timers.
/// True when anything changed.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    // AI features off: the assistant page is not a place to be (`navigate({to: "/overview",
    // replace: true})`).
    if owns(app.route.path()) && !app.prefs.ai_features {
        app.route = crate::app::Route("/overview".into());
        crate::shell::sidebar::reveal_route(app);
        changed = true;
    }
    let on_page = owns(app.route.path());
    // The closed bar's composer sends too, so it reads the providers as the panel does.
    if (on_page || panel_shown(app) || closed::bar_mode(app).is_some())
        && app.prefs.ai_features
        && app.daemon.is_some()
        && !app.assistant.mounted
    {
        app.assistant.mounted = true;
        load_providers(app);
        changed = true;
    }
    if on_page
        && app.assistant.messages.is_empty()
        && !app.assistant.recent_requested
        && app.daemon.is_some()
    {
        app.assistant.recent_requested = true;
        app.send(
            "ui.chat.sessions.list",
            json!({"limit": 3}),
            "assistant.recent",
        );
        changed = true;
    }
    if !on_page {
        app.assistant.recent_requested = false;
    }
    // A transaction opened since the last frame comes to the panel's front and shows a hidden
    // panel (`showDetail`); closing the last one returns to the tab it came from, and hides the
    // panel again when the transaction was what showed it.
    let detail = app
        .detail
        .open
        .as_ref()
        .map(|o| o.id().unwrap_or("").to_owned());
    if detail != app.assistant.seen_detail {
        let was_open = app.assistant.seen_detail.is_some();
        match &detail {
            Some(_) => {
                if !was_open {
                    let back = front(app)
                        .filter(|f| *f != Front::Detail)
                        .unwrap_or(app.assistant.front);
                    app.assistant.detail_return = if back == Front::Detail {
                        Front::Assistant
                    } else {
                        back
                    };
                    app.assistant.opened_for = hosts_panel(app) && !panel_shown(app);
                }
                if hosts_panel(app) {
                    app.prefs.right_panel_open = true;
                }
                app.assistant.front = Front::Detail;
            }
            None => {
                if app.assistant.front == Front::Detail {
                    app.assistant.front = app.assistant.detail_return;
                    if app.assistant.opened_for {
                        hide_panel(app);
                    }
                }
                app.assistant.opened_for = false;
            }
        }
        app.assistant.seen_detail = detail;
        changed = true;
    }
    // Turning developer tools off takes their surfaces with it, as it locks their pages.
    if !app.prefs.developer_tools && !app.assistant.surfaces.is_empty() {
        let open: Vec<Surface> = app.assistant.surfaces.clone();
        for s in open {
            close_surface(app, s);
        }
        changed = true;
    }
    changed |= picker::poll(app, tx);
    changed |= closed::poll(app, tx);
    changed |= thread::poll_thought(app, tx);
    changed |= chat::poll(app);
    changed |= evidence::poll(app, tx);
    if std::mem::take(&mut app.assistant.thread.anchor_new_turn) {
        app.assistant.thread.far = [false, false];
        app.assistant.thread.free = [false, false];
        // The fresh question anchors near the top while its answer streams in below it.
        app.assistant.thread.anchor = app
            .assistant
            .messages
            .iter()
            .rev()
            .find(|m| m.role == model::Role::User)
            .map(|m| m.id.clone());
        app.assistant.thread.anchor_top = [f32::NAN; 2];
        app.assistant.thread.content_bottom = [f32::NAN; 2];
        tx.scroll_to_bottom("asst-thread-page");
        tx.scroll_to_bottom("asst-thread-panel");
        changed = true;
    }
    // A settled turn that outgrew the view needs no anchor: the thread just follows its end.
    if app.assistant.thread.anchor.is_some() && !app.assistant.streaming {
        let th = &app.assistant.thread;
        let outgrown = |i: usize| {
            !th.anchor_top[i].is_nan()
                && thread::anchor_room(th.view_h[i], th.anchor_top[i], th.content_bottom[i])
                    .is_none()
        };
        if (0..2).all(|i| outgrown(i) || th.anchor_top[i].is_nan())
            && (0..2).any(|i| !th.anchor_top[i].is_nan())
        {
            app.assistant.thread.anchor = None;
            changed = true;
        }
    }
    // The system voice finished reading.
    if let Some((_, child)) = app.assistant.speaking.as_mut()
        && !matches!(child.try_wait(), Ok(None))
    {
        app.assistant.speaking = None;
        changed = true;
    }
    if app.assistant.speaking.is_some() {
        tx.wake_after(std::time::Duration::from_millis(500));
    }
    changed
}

/// The first installed `say` voice for the app's language (`say -v ?` lists "Name  xx_YY  # …"),
/// cached for the session; None leaves the system default voice.
fn voice_for(lang: crate::i18n::Lang) -> Option<String> {
    use std::sync::OnceLock;
    static VOICES: OnceLock<Vec<(String, String)>> = OnceLock::new();
    let voices = VOICES.get_or_init(|| {
        let out = std::process::Command::new("/usr/bin/say")
            .args(["-v", "?"])
            .output()
            .ok();
        let text = out
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        text.lines()
            .filter_map(|l| {
                let head = l.split('#').next()?.trim_end();
                let (name, locale) = head.rsplit_once(char::is_whitespace)?;
                Some((name.trim().to_owned(), locale.trim().to_owned()))
            })
            .collect()
    });
    // Plain reading voices only: the list also holds novelty voices (Bad News, Bells, Zarvox),
    // so only these names are picked; none installed leaves the system default voice.
    let (locale, preferred): (&str, &[&str]) = if lang == crate::i18n::Lang::De {
        ("de_", &["Anna", "Sandy", "Shelley", "Flo", "Reed", "Eddy"])
    } else {
        (
            "en_",
            &[
                "Samantha", "Daniel", "Karen", "Moira", "Sandy", "Shelley", "Flo", "Reed", "Eddy",
            ],
        )
    };
    preferred.iter().find_map(|want| {
        voices
            .iter()
            .find(|(name, loc)| {
                loc.starts_with(locale) && (name == want || name.starts_with(&format!("{want} (")))
            })
            .map(|(name, _)| name.clone())
    })
}

/// Read aloud / Stop reading one answer with the system voice (local `say`, macOS only).
pub fn toggle_speech(app: &mut App, key: &str, content: &str) {
    let same = app
        .assistant
        .speaking
        .as_ref()
        .is_some_and(|(k, _)| k == key);
    if let Some((_, mut child)) = app.assistant.speaking.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if same || content.trim().is_empty() {
        return;
    }
    // Headless tooling (scenes, scripts, the click-through harness) never speaks out loud.
    if crate::persist::file::tooling_run() {
        return;
    }
    use std::io::Write;
    let mut cmd = std::process::Command::new("/usr/bin/say");
    // Read in the app's language: the first installed system voice for it, else the default.
    if let Some(voice) = voice_for(crate::i18n::lang()) {
        cmd.arg("-v").arg(voice);
    }
    let spawned = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = spawned {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(content.as_bytes());
        }
        app.assistant.speaking = Some((key.to_owned(), child));
    }
}

/// A stream's progress record (`app.rs` hands every non-terminal record to the areas).
pub fn on_progress(app: &mut App, tag: &str, envelope: &Value) {
    chat::on_progress(app, tag, envelope);
}

/// `ai.providers.list` (every assistant surface reads it on mount).
pub fn load_providers(app: &mut App) {
    if !matches!(app.assistant.providers, Load::Ready(_)) {
        app.assistant.providers = Load::Loading;
    }
    app.send("ai.providers.list", Value::Null, "assistant.providers");
}

/// Every daemon reply; act only on this area's own request tags (`"assistant.…"`).
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if !tag.starts_with("assistant.") {
        return;
    }
    match tag {
        "assistant.providers" => match &r.error {
            Some(e) => app.assistant.providers = Load::Failed(e.clone()),
            None => {
                let list = Provider::parse_list(&r.data);
                // `ModelSetupCard` / `useAiProviderChecks`: the coding agents' last check, read
                // `cached_only` (a display read; never starts a probe).
                let agents: Vec<(String, String)> = list
                    .iter()
                    .filter(|p| p.is_cli())
                    .map(|p| (p.name.clone(), p.base_url.clone()))
                    .collect();
                app.assistant.providers = Load::Ready(list);
                // Read once per mount; a refetch after a check keeps the check's own result.
                let first = app.assistant.runtime.is_empty() && app.prefs.ai_features;
                for (name, url) in agents.into_iter().filter(|_| first) {
                    app.send(
                        "ai.provider_runtime.status",
                        json!({"provider": name, "base_url": url, "cached_only": true}),
                        "assistant.runtime",
                    );
                }
            }
        },
        "assistant.runtime" if r.error.is_none() => {
            if let Some(name) = r.data["provider"].as_str() {
                app.assistant.runtime.retain(|(n, _)| n != name);
                app.assistant
                    .runtime
                    .push((name.to_owned(), r.data.clone()));
            }
        }
        "assistant.recent" if r.error.is_none() => {
            app.assistant.recent = SessionList::parse(&r.data).sessions;
        }
        "assistant.sessions" => {
            app.assistant.sessions = match &r.error {
                Some(e) => Load::Failed(e.clone()),
                None => Load::Ready(SessionList::parse(&r.data)),
            };
        }
        "assistant.session_get" => match &r.error {
            Some(e) => {
                app.assistant.resume_error = Some(e.clone());
                // The submenu opens again to show why.
                app.assistant.menu = Some(Menu::History);
            }
            None => chat::resumed(
                app,
                model::stored_messages(&r.data),
                r.data["id"].as_str().map(str::to_owned),
            ),
        },
        "assistant.upload" => composer::on_upload_reply(app, r),
        "assistant.session_delete" => {
            // The list refetches, as the mutation invalidates the query.
            if app.assistant.menu == Some(Menu::History) {
                app.send(
                    "ui.chat.sessions.list",
                    json!({"limit": 30}),
                    "assistant.sessions",
                );
            }
            app.send(
                "ui.chat.sessions.list",
                json!({"limit": 3}),
                "assistant.recent",
            );
        }
        _ => {}
    }
    picker::on_reply(app, tag, r);
    thread::on_reply(app, tag, r);
    chat::on_reply(app, tag, r);
    checks::on_reply(app, tag, r);
    evidence::on_reply(app, tag, r);
}

/// Opens the History submenu: its list reads only while it is open (`enabled: open`).
pub fn open_history(app: &mut App) {
    app.assistant.menu = Some(Menu::History);
    app.assistant.resume_error = None;
    if !matches!(app.assistant.sessions, Load::Ready(_)) {
        app.assistant.sessions = Load::Loading;
    }
    app.send(
        "ui.chat.sessions.list",
        json!({"limit": 30}),
        "assistant.sessions",
    );
}

/// `resumeSession(id)`: a local read of the saved chat, bound to the open book.
pub fn resume(app: &mut App, id: &str) {
    if app.assistant.streaming {
        return;
    }
    // The saved chat belongs to the book open at the click.
    let Some(book) = chat::book_now(app) else {
        chat::refuse_book_loading(app, None);
        return;
    };
    app.assistant.menu = None;
    app.assistant.resume_error = None;
    app.assistant.generation += 1;
    evidence::clear(app);
    app.assistant.resume_generation = app.assistant.generation;
    let args = json!({"session_id": id, "expected_scope": chat::scope_json(&book)});
    app.assistant.resume_book = Some(book);
    app.send("ui.chat.sessions.get", args, "assistant.session_get");
}

/// `ui.chat.sessions.delete`, then `forgetSession(id)`.
pub fn delete_session(app: &mut App, id: &str) {
    app.send(
        "ui.chat.sessions.delete",
        json!({"session_id": id}),
        "assistant.session_delete",
    );
    if let Load::Ready(l) = &mut app.assistant.sessions {
        l.sessions.retain(|s| s.id != id);
    }
    app.assistant.recent.retain(|s| s.id != id);
    // `forgetSession(id)`.
    if app.assistant.session_id.as_deref() == Some(id) {
        app.assistant.session_id = None;
    }
    app.assistant.chat_modes.remove(id);
}

/// `forgetSession()` after Settings → Chat history → Delete all: the open chat is no longer a
/// saved one, and no saved chat keeps a remembered permission mode.
pub fn forget_all_sessions(app: &mut App) {
    app.assistant.session_id = None;
    app.assistant.chat_modes.clear();
    app.assistant.recent.clear();
    if let Load::Ready(l) = &mut app.assistant.sessions {
        l.sessions.clear();
    }
}

/// `reset()` (Clear chat): an empty, unbound conversation; a running answer is cancelled.
pub fn reset(app: &mut App) {
    chat::clear_chat(app);
}

/// `openAssistantPanel`: the assistant tab, in front, in a shown panel.
pub fn open_panel(app: &mut App) {
    release_column(app);
    app.assistant.tab_open = true;
    set_front(app, Front::Assistant);
    app.prefs.right_panel_open = true;
    app.assistant.menu = None;
}

/// "Ask about this" (`useAskAssistantAbout`): pin the transaction as typed context, prefill the
/// question (which never contains its identity) and open the panel. A draft, not a send: the
/// user reviews the question and the model before anything leaves the device.
pub fn ask_about(app: &mut App, tx_id: &str, date: Option<&str>, quarantine: bool) {
    let context = screen::context_for(
        if quarantine {
            "/review"
        } else {
            "/transactions"
        },
        &format!("?tx={}", urlencode(tx_id)),
    );
    if context["entity_type"] != "transaction" || context["entity_id"].is_null() {
        return;
    }
    let day = date
        .map(|d| d.chars().take(10).collect::<String>())
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "—".into());
    let (label_key, prompt_key) = if quarantine {
        (
            "assistant:panel.focusQuarantine",
            "assistant:panel.askAboutQuarantinePrompt",
        )
    } else {
        (
            "assistant:panel.focusTransaction",
            "assistant:panel.askAboutTransactionPrompt",
        )
    };
    // `captureBook()`: not taken before the open book's ids are known.
    let Some(book) = chat::book_now(app) else {
        chat::refuse_book_loading(app, None);
        return;
    };
    app.assistant.focus = Some(Focus {
        label: crate::i18n::tr(label_key, &[("date", &day)]),
        context,
        book,
    });
    chat::replace_draft(app, &crate::i18n::t(prompt_key), true);
    open_panel(app);
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// A page's own request to the assistant (`handOffToAssistant`: the quarantine fix, and any
/// page that hands over a prompt). It captures the page and the open book at the user's action,
/// opens the panel, and sends at once when a model is chosen (queued while an answer streams);
/// without a model it waits in the composer, without the pin, until it is sent or replaced.
pub fn hand_off(app: &mut App, prompt: &str) {
    let Some(book) = chat::book_now(app) else {
        chat::refuse_book_loading(app, None);
        return;
    };
    let context = screen::current_page(app);
    open_panel(app);
    if prompt.trim().is_empty() {
        return;
    }
    chat::send_prompt(app, prompt, Some(chat::Parked { context, book }));
}

/// The quarantine fix's prompt (`review.seedPrompt`), naming the causes that need the user.
pub fn fix_quarantine(app: &mut App, attention_count: usize) {
    let prompt = crate::i18n::tr(
        "assistant:review.seedPrompt",
        &[("count", &attention_count.to_string())],
    );
    hand_off(app, &prompt);
}

/// Key presses before the shell sees them; true when handled.
pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if matches!(ev.key, Key::Escape) && checks::on_escape(app) {
        return true;
    }
    if picker::on_key(app, ev, tx) || thread::on_key(app, ev, tx) {
        return true;
    }
    if app.assistant.menu.is_some() && matches!(ev.key, Key::Escape) {
        // The submenu closes first, then the menu.
        app.assistant.menu = match app.assistant.menu {
            Some(Menu::History) => Some(Menu::More),
            _ => None,
        };
        return true;
    }
    // ⌘I / Ctrl+I shows and hides the whole side panel in its last state.
    if ev.is_command('i') && !ev.mods.shift && !ev.mods.alt && hosts_panel(app) {
        toggle_side_panel(app);
        return true;
    }
    false
}

/// Headless runs: load what the current route needs before the first frame (block on the
/// daemon, as `transactions::prime` does). Called by the `route` scene after navigating.
pub fn prime(app: &mut App) {
    let mut tx = Tx::default();
    poll(app, &mut tx);
    settle(app);
}

/// Pumps the daemon until this area's reads answered (headless scenes only).
pub fn settle(app: &mut App) {
    for _ in 0..1000 {
        app.pump();
        if !app
            .pending
            .values()
            .any(|(tag, _, _)| tag.starts_with("assistant."))
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn scene_page(a: &mut App) {
    a.navigate("/assistant");
    prime(a);
}

fn scene_model(a: &mut App) {
    a.assistant.selection = Some(("ollama".into(), "qwen3.6:35b".into()));
}

fn scene_surface(a: &mut App, s: Surface) {
    a.prefs.developer_tools = true;
    scene_panel(a);
    open_surface(a, s);
    crate::reports::diag::prime_surfaces(a);
}

fn scene_bar(a: &mut App, mode: &'static str, position: &'static str, working: bool) {
    a.settings.prefs.closed_bar = mode;
    a.settings.prefs.dock_position = position;
    a.prefs.right_panel_open = false;
    scene_model(a);
    a.navigate("/overview");
    prime(a);
    if working {
        synthetic::load_streaming(a);
        let mut tx = Tx::default();
        poll(a, &mut tx);
        hide_panel(a);
    }
}

fn scene_panel(a: &mut App) {
    a.prefs.right_panel_open = true;
    a.assistant.tab_open = true;
    prime(a);
}

/// Named states for `--scene` (headless snapshots and the parity harness); prefix names with
/// `assistant-`. Scenes marked SYNTHETIC inject sample data: sending is disabled, so the demo
/// book has no saved chats, and these show every message part for comparison.
pub const SCENES: &[crate::app::Scene] = &[
    ("assistant-page", scene_page),
    ("assistant-more-menu", |a| {
        scene_page(a);
        a.assistant.menu = Some(Menu::More);
    }),
    ("assistant-history", |a| {
        scene_page(a);
        open_history(a);
        settle(a);
    }),
    ("assistant-model", |a| {
        scene_model(a);
        scene_page(a);
    }),
    ("assistant-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_page(a);
    }),
    ("assistant-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_page(a);
    }),
    ("assistant-typed", |a| {
        scene_model(a);
        scene_page(a);
        let mut e = TextEdit::new("Which transfers still need a match?");
        e.focused = true;
        a.assistant.draft = e;
    }),
    // SYNTHETIC: staged files in the composer (no file picker runs headless).
    ("assistant-attachments", |a| {
        scene_model(a);
        scene_page(a);
        a.assistant.files = synthetic::files();
    }),
    ("assistant-panel", scene_panel),
    ("assistant-panel-model", |a| {
        scene_model(a);
        scene_panel(a);
    }),
    ("assistant-panel-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_panel(a);
    }),
    ("assistant-panel-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_panel(a);
    }),
    // Maximized: the same empty state as the page (one shared column).
    ("assistant-panel-max", |a| {
        scene_panel(a);
        a.detail.maximized = true;
    }),
    ("assistant-panel-max-model", |a| {
        scene_model(a);
        scene_panel(a);
        a.detail.maximized = true;
    }),
    ("assistant-panel-max-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_panel(a);
        a.detail.maximized = true;
    }),
    ("assistant-panel-max-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_panel(a);
        a.detail.maximized = true;
    }),
    ("assistant-panel-launcher", |a| {
        a.prefs.right_panel_open = true;
        a.assistant.tab_open = false;
        prime(a);
    }),
    ("assistant-panel-transactions", |a| {
        a.navigate("/transactions");
        crate::transactions::prime(a, false);
        scene_panel(a);
    }),
    // The panel's own surfaces (developer tools on, as their pages need).
    ("assistant-panel-logs", |a| scene_surface(a, Surface::Logs)),
    ("assistant-panel-logs-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_surface(a, Surface::Logs);
    }),
    ("assistant-panel-logs-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_surface(a, Surface::Logs);
    }),
    ("assistant-panel-logs-errors", |a| {
        scene_surface(a, Surface::Logs);
        a.reports.diag.tail.level = crate::reports::diag::tail::Level::Error;
    }),
    ("assistant-panel-changes", |a| {
        scene_surface(a, Surface::Changes)
    }),
    ("assistant-panel-changes-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_surface(a, Surface::Changes);
    }),
    ("assistant-panel-changes-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_surface(a, Surface::Changes);
    }),
    // SYNTHETIC: the demo book has no edit history; three sample edits, the first unfolded.
    ("assistant-panel-changes-sample", |a| {
        scene_surface(a, Surface::Changes);
        crate::reports::diag::changes::inject_sample(a);
    }),
    ("assistant-panel-changes-sample-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_surface(a, Surface::Changes);
        crate::reports::diag::changes::inject_sample(a);
    }),
    // All tabs at once: assistant, both surfaces and a transaction, the Logs tab in front.
    ("assistant-panel-tabs", |a| {
        a.navigate("/transactions");
        crate::transactions::prime(a, false);
        let first = a
            .snapshot()
            .and_then(|s| s.txs.first())
            .map(|t| t.id.clone());
        if let Some(id) = first {
            crate::txdetail::open(a, &id, "details");
            crate::txdetail::settle(a);
        }
        scene_panel(a);
        a.prefs.developer_tools = true;
        let mut tx = Tx::default();
        poll(a, &mut tx);
        open_surface(a, Surface::Changes);
        open_surface(a, Surface::Logs);
        crate::reports::diag::prime_surfaces(a);
    }),
    // The same, the transaction in front (its own tab bar lists the other tabs).
    ("assistant-panel-tabs-detail", |a| {
        a.navigate("/transactions");
        crate::transactions::prime(a, false);
        let first = a
            .snapshot()
            .and_then(|s| s.txs.first())
            .map(|t| t.id.clone());
        scene_panel(a);
        a.prefs.developer_tools = true;
        open_surface(a, Surface::Logs);
        if let Some(id) = first {
            crate::txdetail::open(a, &id, "details");
            crate::txdetail::settle(a);
        }
        let mut tx = Tx::default();
        poll(a, &mut tx);
        crate::reports::diag::prime_surfaces(a);
    }),
    // A panel without tabs on a page with developer tools: the launcher offers every surface.
    ("assistant-panel-launcher-dev", |a| {
        a.prefs.developer_tools = true;
        a.prefs.right_panel_open = true;
        a.assistant.tab_open = false;
        prime(a);
    }),
    ("assistant-panel-add", |a| {
        scene_panel(a);
        a.assistant.menu = Some(Menu::PanelAdd);
    }),
    // "Ask about this" on the first transaction, as the detail header's button does.
    ("assistant-ask-about", |a| {
        a.navigate("/transactions");
        crate::transactions::prime(a, false);
        let first = a
            .snapshot()
            .and_then(|s| s.txs.first())
            .map(|t| (t.id.clone(), t.date.clone()));
        if let Some((id, date)) = first {
            ask_about(a, &id, Some(&date), false);
        }
        prime(a);
    }),
    // SYNTHETIC conversations: every message part type.
    ("assistant-thread", |a| {
        scene_model(a);
        scene_page(a);
        synthetic::load(a);
    }),
    ("assistant-thread-basics", |a| {
        scene_model(a);
        scene_page(a);
        synthetic::load_basics(a);
    }),
    ("assistant-thread-open", |a| {
        scene_model(a);
        scene_page(a);
        synthetic::load_basics(a);
        for k in [
            "asst-think-s1",
            "asst-group-tools:c1",
            "asst-tool-c5",
            "asst-tool-c4",
            "asst-visual-c7-table",
        ] {
            a.assistant.thread.toggles.insert(k.into(), true);
        }
    }),
    ("assistant-thread-visuals", |a| {
        scene_model(a);
        scene_page(a);
        synthetic::load_visuals(a);
    }),
    ("assistant-thread-approvals", |a| {
        scene_model(a);
        scene_page(a);
        synthetic::load_approvals(a);
    }),
    ("assistant-picker", picker::scene_open),
    ("assistant-picker-model", picker::scene_model),
    ("assistant-picker-effort", picker::scene_effort),
    ("assistant-picker-permission", picker::scene_permission),
    ("assistant-picker-search", picker::scene_search),
    // SYNTHETIC: the favorites store is empty on a fresh book.
    ("assistant-picker-favorites", picker::scene_favorites),
    ("assistant-picker-de", picker::scene_de),
    ("assistant-picker-light", picker::scene_light),
    ("assistant-picker-trigger", picker::scene_trigger_traits),
    // SYNTHETIC (a cached Ollama listing; nothing is checked): the overhauled picker's states.
    ("assistant-picker-models", picker::scene_models),
    ("assistant-picker-highlight", picker::scene_highlight),
    (
        "assistant-picker-search-models",
        picker::scene_search_models,
    ),
    ("assistant-picker-remote", picker::scene_remote),
    ("assistant-picker-warning", picker::scene_warning),
    ("assistant-picker-panel", picker::scene_panel),
    ("assistant-picker-panel-max", picker::scene_panel_max),
    ("assistant-thread-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_model(a);
        scene_page(a);
        synthetic::load(a);
    }),
    ("assistant-thread-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_model(a);
        scene_page(a);
        synthetic::load(a);
    }),
    ("assistant-thread-panel", |a| {
        scene_model(a);
        scene_panel(a);
        synthetic::load(a);
    }),
    // A short question typed with a model chosen: the send button is live.
    ("assistant-typed-send", |a| {
        scene_model(a);
        scene_page(a);
        let mut e = TextEdit::new("Hi");
        e.focused = true;
        a.assistant.draft = e;
    }),
    // Live against a loopback stub provider named `stub` (test tooling registers it on a
    // disposable book; on any other book the selection names a provider that is not there).
    ("assistant-live-stub", |a| {
        scene_page(a);
        a.assistant.selection = Some(("stub".into(), "stub-model".into()));
    }),
    ("assistant-live-stub-approval", |a| {
        scene_page(a);
        a.assistant.selection = Some(("stub".into(), "stub-model".into()));
        a.assistant.permission = PermissionMode::Approval;
    }),
    ("assistant-live-stub-bar", |a| {
        scene_bar(a, "composer", "center", false);
        a.assistant.selection = Some(("stub".into(), "stub-model".into()));
        a.assistant.permission = PermissionMode::Approval;
    }),
    ("assistant-live-stub-panel", |a| {
        a.navigate("/transactions");
        crate::transactions::prime(a, false);
        scene_panel(a);
        a.assistant.selection = Some(("stub".into(), "stub-model".into()));
        a.assistant.permission = PermissionMode::Approval;
    }),
    // Live: the page with the Approval permission mode (scripted runs against a local stub).
    ("assistant-live-approval", |a| {
        scene_page(a);
        a.assistant.permission = PermissionMode::Approval;
    }),
    // SYNTHETIC: a question just sent (loader, Stop), and an answer streaming with one queued.
    ("assistant-thread-pending", |a| {
        scene_model(a);
        scene_page(a);
        synthetic::load_pending(a);
    }),
    ("assistant-thread-streaming", |a| {
        scene_model(a);
        scene_page(a);
        synthetic::load_streaming(a);
        a.assistant
            .thread
            .toggles
            .insert("asst-think-ls1".into(), true);
        let mut e = TextEdit::new("And the Liquid peg?");
        e.focused = true;
        a.assistant.draft = e;
    }),
    ("assistant-thread-streaming-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_model(a);
        scene_page(a);
        synthetic::load_streaming(a);
    }),
    ("assistant-thread-streaming-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_model(a);
        scene_page(a);
        synthetic::load_streaming(a);
    }),
    ("assistant-panel-streaming", |a| {
        scene_model(a);
        scene_panel(a);
        synthetic::load_streaming(a);
    }),
    ("assistant-panel-pending", |a| {
        scene_model(a);
        scene_panel(a);
        synthetic::load_pending(a);
    }),
    // SYNTHETIC: a failed turn's error under the thread (the page's alert and the message's line).
    ("assistant-thread-error", |a| {
        scene_model(a);
        scene_page(a);
        synthetic::load_pending(a);
        a.assistant.streaming = false;
        if let Some(m) = a.assistant.messages.last_mut() {
            m.status = model::MsgStatus::Error;
            m.error_code = Some("provider_unreachable".into());
            m.error_message =
                Some("Could not reach the model provider at http://127.0.0.1:11434/v1.".into());
        }
        a.assistant.error =
            Some("Could not reach the model provider at http://127.0.0.1:11434/v1.".into());
    }),
    ("assistant-panel-error", |a| {
        scene_model(a);
        scene_panel(a);
        synthetic::load_pending(a);
        a.assistant.streaming = false;
        if let Some(m) = a.assistant.messages.last_mut() {
            m.status = model::MsgStatus::Error;
            m.error_code = Some("provider_unreachable".into());
            m.error_message = Some("Could not reach the model provider.".into());
        }
        a.assistant.error = Some("Could not reach the model provider.".into());
    }),
    // The off-device confirmation a pick of an unacknowledged provider asks first (shown only).
    ("assistant-confirm-remote", |a| {
        scene_page(a);
        if let Some(p) = a.assistant.providers.ready().and_then(|l| {
            l.iter()
                .find(|p| p.kind != "local" && p.acknowledged_at.is_none())
                .cloned()
        }) {
            a.assistant.checks.confirm = Some((
                crate::i18n::tr(
                    "assistant:modelPicker.remoteConfirm",
                    &[("provider", &p.label())],
                ),
                checks::After::Pick(p.name.clone(), "default".into()),
            ));
        }
    }),
    ("assistant-confirm-remote-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_page(a);
        if let Some(p) = a.assistant.providers.ready().and_then(|l| {
            l.iter()
                .find(|p| p.kind != "local" && p.acknowledged_at.is_none())
                .cloned()
        }) {
            a.assistant.checks.confirm = Some((
                crate::i18n::tr(
                    "assistant:modelPicker.remoteConfirm",
                    &[("provider", &p.label())],
                ),
                checks::After::Pick(p.name.clone(), "default".into()),
            ));
        }
    }),
    // SYNTHETIC: the setup card while an agent's check runs (no probe is started).
    ("assistant-setup-checking", |a| {
        scene_page(a);
        if let Some(p) = a
            .assistant
            .providers
            .ready()
            .and_then(|l| l.iter().find(|p| p.is_cli()).cloned())
        {
            a.assistant.checks.runtime_pending.insert(p.name.clone());
        }
    }),
    // SYNTHETIC: Check models running for the open provider (no probe is started).
    ("assistant-picker-checking", |a| {
        picker::scene_open(a);
        if let Some(picker::View::Provider(p)) = a.assistant.picker.view.clone() {
            a.assistant.checks.checking.insert(p);
        }
    }),
    // SYNTHETIC: an approval row's More menu.
    ("assistant-approval-more", |a| {
        scene_model(a);
        scene_page(a);
        synthetic::load_approvals(a);
        a.assistant.thread.consent_menu = Some("p2".into());
    }),
    // SYNTHETIC: saved chats in the History submenu (history is off on the demo book).
    ("assistant-history-list", |a| {
        scene_page(a);
        open_history(a);
        settle(a);
        a.assistant.sessions = Load::Ready(synthetic::sessions());
    }),
    // The closed bar under a page (Settings → Appearance), the panel hidden.
    ("assistant-bar-composer", |a| {
        scene_bar(a, "composer", "right", false)
    }),
    ("assistant-bar-composer-center", |a| {
        scene_bar(a, "composer", "center", false)
    }),
    ("assistant-bar-composer-left-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_bar(a, "composer", "left", false);
    }),
    ("assistant-bar-launcher", |a| {
        scene_bar(a, "launcher", "right", false)
    }),
    ("assistant-bar-launcher-center", |a| {
        scene_bar(a, "launcher", "center", false)
    }),
    ("assistant-bar-launcher-left-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_bar(a, "launcher", "left", false);
    }),
    // SYNTHETIC: a conversation streaming behind the closed bar (the Working button).
    ("assistant-bar-working", |a| {
        scene_bar(a, "launcher", "right", true)
    }),
    // SYNTHETIC: an approval waiting while the panel is closed: the notice under the page.
    ("assistant-approval-notice", |a| {
        scene_bar(a, "hidden", "right", false);
        scene_model(a);
        synthetic::load_approvals(a);
        let mut tx = Tx::default();
        poll(a, &mut tx);
        hide_panel(a);
    }),
    ("assistant-approval-notice-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_bar(a, "hidden", "right", false);
        scene_model(a);
        synthetic::load_approvals(a);
        let mut tx = Tx::default();
        poll(a, &mut tx);
        hide_panel(a);
    }),
    // SYNTHETIC: the quick-resume list under the empty page.
    ("assistant-recent", |a| {
        scene_model(a);
        scene_page(a);
        a.assistant.recent = synthetic::sessions().sessions.into_iter().take(3).collect();
    }),
];

/// Width the docked panel takes from the window beyond what `txdetail::docked_width` counts
/// (the shell adds both for the nav's auto-fold rule): the panel's one width while this module
/// draws it, less the details' own count whenever the details are not what is on screen.
pub fn docked_width(app: &App) -> f32 {
    if !hosts_panel(app) {
        return 0.0;
    }
    let details = crate::txdetail::docked_width(app);
    let window = app.shell.window_w;
    let sheet = window > 0.0 && window < crate::txdetail::SHEET_BELOW;
    if !panel_shown(app) {
        return -details;
    }
    match front(app) {
        Some(Front::Detail) => 0.0,
        _ if app.detail.maximized || sheet => -details,
        _ => app.detail.width - details,
    }
}

/// The right panel (title bar toggle) beside the page frame: empty, since the panel docks
/// inside the page frame (`dock`), where the web's `RightPanel` sits.
pub fn right_panel(_app: &App, _cx: &mut Cx<App>, _tk: &Tk) -> El {
    rune::el()
}

/// The panel docked inside the page frame, beside the page. This module decides what the dock
/// slot holds: its own tabs (assistant, surfaces, launcher), the open transaction's details
/// (drawn by `txdetail`), or nothing while the panel is hidden. The shell asks `txdetail` only
/// when this returns `None`, so a hidden panel or another tab in front claims the slot with an
/// empty element to keep the open details out of view.
pub fn dock(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> Option<El> {
    if !hosts_panel(app) {
        return None;
    }
    let claim = || {
        app.detail
            .open
            .is_some()
            .then(|| rune::el().key("asst-dock-claim").size(0.0, 0.0))
    };
    if !panel_shown(app) {
        return claim();
    }
    if front(app) == Some(Front::Detail) {
        return Some(
            crate::txdetail::dock(app, cx, tk, layer)
                .unwrap_or_else(|| rune::el().key("asst-dock-claim").size(0.0, 0.0)),
        );
    }
    panel::view(app, cx, tk, layer).or_else(claim)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_hides_a_shown_panel_whatever_is_in_front() {
        assert_eq!(toggle_step(true, true, false), Toggle::Hide);
        assert_eq!(toggle_step(true, true, true), Toggle::Hide);
    }

    #[test]
    fn toggle_restores_the_last_tabs_or_opens_the_assistant() {
        assert_eq!(toggle_step(false, true, false), Toggle::Restore);
        assert_eq!(toggle_step(false, false, false), Toggle::OpenAssistant);
        // The assistant page is the assistant itself: no panel to open there.
        assert_eq!(toggle_step(false, false, true), Toggle::Nothing);
        assert_eq!(toggle_step(false, true, true), Toggle::Restore);
    }

    #[test]
    fn front_is_remembered_while_its_tab_exists() {
        let tabs = [
            Front::Assistant,
            Front::Surface(Surface::Logs),
            Front::Detail,
        ];
        assert_eq!(pick_front(&tabs, Front::Detail), Some(Front::Detail));
        assert_eq!(
            pick_front(&tabs, Front::Surface(Surface::Changes)),
            Some(Front::Assistant)
        );
        assert_eq!(
            pick_front(&[Front::Detail], Front::Assistant),
            Some(Front::Detail)
        );
        assert_eq!(pick_front(&[], Front::Assistant), None);
    }
}

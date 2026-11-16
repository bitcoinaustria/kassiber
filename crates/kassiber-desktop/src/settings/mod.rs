//! The Settings modal: every section of `SettingsScreen` (`/settings/<slug>` opens it over the
//! page that asked for it). SettingsModal.tsx, SettingsNavigation.tsx, settingsSections.ts and
//! settingsModalNavigation.ts live here; the section bodies in the submodules.
//!
//! Reads: opening the modal reads the backend list, maintenance settings and `status`; each
//! section adds the local reads its React panel makes on mount. Nothing here contacts the
//! network: connection tests, provider checks, rate rebuilds and agent sessions run only from
//! their buttons.

pub mod ai;
pub mod aiform;
pub mod appearance;
pub mod backend;
pub mod certtrust;
pub mod controls;
pub mod data;
pub mod desktop;
pub mod dialogs;
pub mod effects;
pub mod market;
pub mod model;
pub mod network;
pub mod prefpath;
pub mod privacy;
pub mod search;
pub mod security;
pub mod subpage;

use std::collections::HashMap;

use rune::{Color, Cx, El, Icon, Key, KeyEvent, ScrollSpec, Shadow, Tx, col, el, row, text};
use serde_json::{Value, json};

use crate::app::{App, Reply, ToastTone};
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

/// One category of the rail (`SETTINGS_SECTIONS`). `key` picks `settings:nav.section.<key>`.
pub struct Section {
    pub slug: &'static str,
    pub key: &'static str,
    pub group: &'static str,
    pub icon: &'static Icon,
}

pub const APPEARANCE: usize = 0;
pub const MARKET: usize = 1;
pub const BITCOIN: usize = 2;
pub const LIGHTNING: usize = 3;
pub const LIQUID: usize = 4;
pub const PRIVACY: usize = 5;
pub const SECURITY: usize = 6;
pub const AI: usize = 7;
pub const DATA: usize = 8;
pub const TERMINAL: usize = 9;
pub const DEVELOPER: usize = 10;

/// The rail order; `app.shell.settings_section` indexes it.
pub static SECTIONS: [Section; 11] = [
    Section {
        slug: "appearance",
        key: "appearance",
        group: "general",
        icon: &I::PALETTE,
    },
    Section {
        slug: "market",
        key: "market",
        group: "general",
        icon: &I::CHART_LINE,
    },
    Section {
        slug: "bitcoin",
        key: "bitcoin",
        group: "data",
        icon: &I::BITCOIN,
    },
    Section {
        slug: "lightning",
        key: "lightning",
        group: "data",
        icon: &I::ZAP,
    },
    Section {
        slug: "liquid",
        key: "liquid",
        group: "data",
        icon: &I::DROPLETS,
    },
    Section {
        slug: "privacy",
        key: "privacy",
        group: "privacySecurity",
        icon: &I::EYE,
    },
    Section {
        slug: "security",
        key: "lock",
        group: "privacySecurity",
        icon: &I::LOCK,
    },
    Section {
        slug: "ai",
        key: "ai",
        group: "assistant",
        icon: &I::BOT,
    },
    Section {
        slug: "data",
        key: "storage",
        group: "dataGroup",
        icon: &I::HARD_DRIVE,
    },
    Section {
        slug: "terminal",
        key: "terminal",
        group: "desktop",
        icon: &I::TERMINAL,
    },
    Section {
        slug: "developer",
        key: "developer",
        group: "desktop",
        icon: &I::WRENCH,
    },
];

const GROUPS: [&str; 6] = [
    "general",
    "data",
    "privacySecurity",
    "assistant",
    "dataGroup",
    "desktop",
];

/// `SETTINGS_SECTION_FOR_SLUG`: canonical slugs and the older menu aliases.
pub fn section_for_slug(slug: &str) -> Option<usize> {
    let s = slug.trim().trim_start_matches('#').to_lowercase();
    Some(match s.as_str() {
        "appearance" | "display" => APPEARANCE,
        "explorers" | "explorer" | "bitcoin" | "backends" => BITCOIN,
        "lightning" => LIGHTNING,
        "liquid" => LIQUID,
        "market" | "rates" => MARKET,
        "privacy" => PRIVACY,
        "security" | "lock" => SECURITY,
        "ai" | "assistant" => AI,
        "data" | "storage" => DATA,
        "terminal" | "desktop" => TERMINAL,
        "developer" | "logs" => DEVELOPER,
        _ => return None,
    })
}

/// Developer-only sections (`DEV_ONLY_SETTINGS_SECTIONS`).
pub fn dev_only(i: usize) -> bool {
    i == LIGHTNING
}

/// The Settings screens' own preferences (the web keeps them in `kb.ui`); `crate::persist` saves
/// and restores them, except `update_checks` (its own consent file, see `effects`).
#[derive(Clone, Debug)]
pub struct LocalPrefs {
    pub clear_clipboard: bool,
    pub desktop_notifications: bool,
    /// Privacy: hide amounts while the screen is shared (`crate::osleaks::screen`).
    pub hide_on_screen_share: bool,
    /// Data: keep unencrypted books out of Time Machine (`crate::osleaks::backup`).
    pub exclude_unencrypted_backups: bool,
    pub update_checks: bool,
    pub auto_lock: bool,
    pub idle_minutes: u32,
    pub require_on_launch: bool,
    pub lock_on_close: bool,
    pub touch_id_unlock: bool,
    pub closed_bar: &'static str,
    pub dock_position: &'static str,
    pub permission_mode: &'static str,
    pub tool_profiles: HashMap<String, String>,
    pub explorer_fallbacks_allowed: bool,
}

impl Default for LocalPrefs {
    fn default() -> Self {
        LocalPrefs {
            clear_clipboard: true,
            desktop_notifications: false,
            hide_on_screen_share: true,
            exclude_unencrypted_backups: true,
            update_checks: false,
            auto_lock: false,
            idle_minutes: 5,
            require_on_launch: false,
            lock_on_close: false,
            touch_id_unlock: false,
            closed_bar: "hidden",
            dock_position: "right",
            permission_mode: "auto",
            tool_profiles: HashMap::new(),
            explorer_fallbacks_allowed: true,
        }
    }
}

/// This area's UI state (`app.settings`).
#[derive(Default)]
pub struct Ui {
    pub prefs: LocalPrefs,
    /// The open select's trigger key.
    pub select: Option<String>,
    /// A copy button showing its check: (key, until).
    pub copied: Option<(String, f64)>,
    /// Disclosures and expanders toggled away from their default.
    pub open: HashMap<String, bool>,
    /// A search result's row: washed until `highlight_until`.
    pub highlight: Option<&'static str>,
    pub highlight_until: f64,
    /// Requested by a search result; revealed once its section shows (stale after 3 s).
    pub pending_anchor: Option<(&'static str, f64)>,
    /// Reads made for this opening: (section loaded last, modal was open).
    pub was_open: bool,
    pub loaded: Option<usize>,
    /// Requests in flight by tag.
    pub busy: HashMap<&'static str, usize>,
    /// `status {storage: true}` (Data & storage only).
    pub storage: Option<Value>,
    pub setting_default: Option<String>,
    /// The market configure in flight: (what, on).
    pub market_change: Option<(&'static str, bool)>,
    pub privacy_filter: Option<&'static str>,
    pub offline_failed: bool,
    pub bn: network::BookNet,
    pub market: market::Market,
    pub backup: data::Backup,
    pub ai: ai::AiUi,
    pub agents: ai::Agents,
    pub terminal: desktop::Terminal,
    pub backend: backend::BackendForm,
    pub aiform: aiform::AiForm,
    pub dialog: dialogs::Dialogs,
    /// The open sub-page's frame (its form's open flag says whether one is open).
    pub sub: Option<subpage::Sub>,
    /// The discard prompt of a sub-page with changes: where leaving goes once confirmed.
    pub discard: Option<subpage::After>,
    /// A sub-page a deep link asked for, waiting on a read: (anchor, asked at).
    pub pending_page: Option<(&'static str, f64)>,
    /// What the preferences do outside the modal (idle lock, OS notifications).
    pub fx: effects::Effects,
}

impl Ui {
    pub fn is_busy(&self, tag: &str) -> bool {
        self.busy.get(tag).copied().unwrap_or(0) > 0
    }
}

/// Sends a request and counts it in flight under its tag.
pub fn send(app: &mut App, kind: &str, args: Value, tag: &'static str) -> bool {
    if app.send(kind, args, tag).is_some() {
        *app.settings.busy.entry(tag).or_insert(0) += 1;
        true
    } else {
        false
    }
}

/// Whether a disclosure or expander is open (`default` until toggled).
pub fn is_open(app: &App, key: &str, default: bool) -> bool {
    app.settings.open.get(key).copied().unwrap_or(default)
}

pub fn toggle_open(app: &mut App, key: &str, default: bool) {
    let now = is_open(app, key, default);
    app.settings.open.insert(key.to_owned(), !now);
}

/// The section shown: the requested one, or the default while developer tools hide it.
pub fn current(app: &App) -> usize {
    let i = app.shell.settings_section.min(SECTIONS.len() - 1);
    if dev_only(i) && !app.prefs.developer_tools {
        APPEARANCE
    } else {
        i
    }
}

/// Every frame: the reads the open section needs, the search-jump reveal, timers.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = effects::poll(app, tx);
    changed |= step(app);
    if !app.shell.settings_open {
        return changed;
    }
    changed |= subpage::poll(app, tx);
    if app.settings.pending_page.is_some() {
        changed |= subpage::resolve_pending(app);
        if app.settings.pending_page.is_some() {
            tx.wake_after(std::time::Duration::from_millis(100));
        }
    }
    // A search result scrolls its row into view once the section shows it.
    if let Some((anchor, at)) = app.settings.pending_anchor {
        if app.clock - at > 3.0 {
            app.settings.pending_anchor = None;
        } else if search::section_of(anchor) == Some(current(app))
            && search::anchor_ready(app, anchor)
        {
            if let Some(key) = search::reveal_key(app, anchor) {
                app.settings.open.insert(key, true);
            }
            tx.scroll_into_view(format!("set-anchor-{anchor}"));
            app.settings.highlight = Some(anchor);
            app.settings.highlight_until = app.clock + 1.5;
            app.settings.pending_anchor = None;
            tx.wake_after(std::time::Duration::from_millis(1550));
            changed = true;
        } else {
            tx.wake_after(std::time::Duration::from_millis(100));
        }
    }
    if app.settings.highlight.is_some() && app.clock >= app.settings.highlight_until {
        app.settings.highlight = None;
        changed = true;
    }
    if let Some((_, until)) = &app.settings.copied
        && *until <= app.clock
    {
        app.settings.copied = None;
        changed = true;
    }
    // The agent session's live read every 3 s while a session is active (a broker read, local).
    if ai::agent_session_active(app)
        && app.settings.agents.next_live <= app.clock
        && current(app) == AI
    {
        app.settings.agents.next_live = app.clock + 3.0;
        if !app.settings.is_busy("settings.agent_live") {
            send(
                app,
                "ui.agent_access.status",
                json!({"refresh": true}),
                "settings.agent_live",
            );
        }
        tx.wake_after(std::time::Duration::from_secs(3));
    }
    changed
}

/// The work of `poll` that needs no `Tx` (also run by `prime`).
fn step(app: &mut App) -> bool {
    if !app.shell.settings_open {
        if app.settings.was_open {
            app.settings.was_open = false;
            if let Some(prev) = app.settings.loaded {
                leave_section(app, prev);
            }
            // `SettingsScreen` unmounts with the modal: its terminal status error goes too.
            app.settings.terminal.error = None;
            app.settings.loaded = None;
            app.settings.select = None;
            // Settings shut from outside (the lock screen): an open sub-page goes with it, its
            // typed values (passphrases) dropped rather than kept for the next opening.
            subpage::close_page(app);
            app.settings.pending_page = None;
            return true;
        }
        return false;
    }
    let mut changed = false;
    if !app.settings.was_open {
        app.settings.was_open = true;
        // `refetchOnMount: "always"` reads of SettingsScreen.
        send(
            app,
            "ui.backends.settings.list",
            Value::Null,
            "settings.backends",
        );
        send(
            app,
            "ui.maintenance.settings",
            Value::Null,
            "settings.maintenance",
        );
        send(app, "status", Value::Null, "settings.status");
        changed = true;
    }
    changed |= backend::fixup(app);
    let sec = current(app);
    if app.settings.loaded != Some(sec) {
        if let Some(prev) = app.settings.loaded {
            leave_section(app, prev);
        }
        app.settings.loaded = Some(sec);
        app.settings.select = None;
        load_section(app, sec);
        changed = true;
    }
    changed
}

/// A section's panel unmounts when another section shows or the modal closes, so what it keeps
/// in `useState` starts over: import, rebuild and backup outcomes and their fields, a book
/// network plan, the exposure filter, provider check errors and drafts. Settings themselves,
/// cached reads and a request still in flight stay.
fn leave_section(app: &mut App, sec: usize) {
    let s = &mut app.settings;
    match sec {
        MARKET if s.market.pending.is_none() && !s.is_busy("settings.rebuild") => {
            s.market = market::Market::default()
        }
        BITCOIN | LIGHTNING | LIQUID => s.bn = network::BookNet::default(),
        PRIVACY => s.privacy_filter = None,
        AI => {
            s.ai.update_error = None;
            s.ai.ack_error = None;
            s.ai.confirming_clear = false;
            s.ai.refusals.clear();
            for c in s.ai.checks.values_mut() {
                c.error = None;
            }
            s.agents.draft_grant = None;
            s.agents.draft_access = None;
            s.agents.session_error = None;
        }
        DATA if ![
            "settings.backup_export",
            "settings.backup_preview",
            "settings.backup_apply",
        ]
        .iter()
        .any(|t| s.is_busy(t)) =>
        {
            s.backup = data::Backup::default()
        }
        _ => {}
    }
}

/// The local reads a section's panel makes on mount.
fn load_section(app: &mut App, sec: usize) {
    match sec {
        BITCOIN => network::load(app),
        PRIVACY => {
            send(app, "ui.network.offline", Value::Null, "settings.offline");
        }
        AI => ai::load(app),
        DATA => {
            send(app, "status", json!({"storage": true}), "settings.storage");
        }
        TERMINAL => desktop::load_terminal(app),
        _ => {}
    }
}

/// Every daemon reply; acts only on `"settings.…"` tags.
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    effects::on_reply(app, tag, r);
    if !tag.starts_with("settings.") {
        return;
    }
    if let Some(n) = app.settings.busy.get_mut(tag) {
        *n = n.saturating_sub(1);
    }
    match tag {
        "settings.storage" => {
            if r.error.is_none() {
                app.settings.storage = Some(r.data["storage"].clone());
            }
        }
        "settings.offline_set" => {
            app.settings.offline_failed = r.error.is_some();
            if r.error.is_none() {
                send(app, "ui.network.offline", Value::Null, "settings.offline");
            }
        }
        "settings.set_default" => {
            let name = app.settings.setting_default.take().unwrap_or_default();
            match &r.error {
                None => {
                    let display = model::backends(app.read("ui.backends.settings.list"), None)
                        .into_iter()
                        .find(|b| b.id == name)
                        .map(|b| b.name)
                        .unwrap_or(name);
                    app.toast(
                        ToastTone::Success,
                        &t("settings:defaultBackend.updatedTitle"),
                        Some(crate::i18n::tr(
                            "settings:defaultBackend.updatedBody",
                            &[("name", &display)],
                        )),
                        None,
                    );
                }
                Some(e) => app.toast(
                    ToastTone::Warning,
                    &t("settings:defaultBackend.failedTitle"),
                    Some(e.clone()),
                    None,
                ),
            }
            refresh_backends(app);
        }
        _ => {
            network::on_reply(app, tag, r);
            market::on_reply(app, tag, r);
            backend::on_reply(app, tag, r);
            ai::on_reply(app, tag, r);
            aiform::on_reply(app, tag, r);
            data::on_reply(app, tag, r);
            dialogs::on_reply(app, tag, r);
        }
    }
}

/// Re-reads the backend list (after an add, edit, delete or new default); the shell's
/// network popover reads the same reply.
pub fn refresh_backends(app: &mut App) {
    send(
        app,
        "ui.backends.settings.list",
        Value::Null,
        "settings.backends",
    );
}

/// Escape closes the innermost overlay: an open select, a dialog, the modal.
pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    // A keydown resets the idle lock (settings sees every key first).
    effects::activity(app);
    if !app.shell.settings_open || app.locked {
        return false;
    }
    if ev.key == Key::Escape {
        if app.settings.select.take().is_some() {
            return true;
        }
        if dialogs::close_top(app) {
            return true;
        }
        if subpage::active(app).is_some() {
            subpage::leave(app, subpage::After::Back);
            return true;
        }
        close(app);
        return true;
    }
    false
}

/// `closeSettings`.
pub fn close(app: &mut App) {
    app.shell.settings_open = false;
    app.settings.select = None;
}

/// `openSettingsSection`: a developer-only category stays shut while developer tools are off;
/// behind the lock screen a request is dropped.
pub fn open_index(app: &mut App, i: usize) -> bool {
    if app.locked || (dev_only(i) && !app.prefs.developer_tools) {
        return false;
    }
    // Another section while a sub-page is open leaves the page first (the discard prompt
    // when it has changes).
    if app.shell.settings_open && subpage::parent(app).is_some_and(|p| p != i) {
        subpage::leave(app, subpage::After::Section(i));
        return false;
    }
    app.shell.popup = None;
    app.shell.settings_open = true;
    app.shell.settings_section = i;
    true
}

/// Opens the modal on the section a `/settings/<slug>` route names (`settingsSectionForHash`;
/// unknown or empty slugs keep the default section). Also takes `<slug>#<anchor>` (a
/// setting-level search result) and a bare `#<slug>` (the legacy hash form).
pub fn open_section(app: &mut App, slug: &str) {
    // A link while a sub-page is open leaves the page first (asking when it has changes).
    if app.shell.settings_open && subpage::active(app).is_some() && !app.locked {
        subpage::leave(app, subpage::After::Link(slug.to_owned()));
        return;
    }
    let (path, hash) = slug.split_once('#').unwrap_or((slug, ""));
    let path = path.split('?').next().unwrap_or("").trim_matches('/');
    let (sec, anchor) = if path.is_empty() {
        (section_for_slug(hash), None)
    } else {
        (section_for_slug(path), Some(hash).filter(|h| !h.is_empty()))
    };
    let sec = sec.unwrap_or(APPEARANCE);
    if open_index(app, sec) {
        // A sub-page anchor (`#addBackend`, `#addProvider`, `#changePassphrase`) opens that page.
        if let Some(a) = anchor
            && subpage::open_anchor(app, sec, a)
        {
            return;
        }
        if let Some(a) = anchor.and_then(search::anchor_id) {
            app.settings.pending_anchor = Some((a, app.clock));
        }
    }
}

/// Headless runs: load what the open section needs before the first frame.
pub fn prime(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        step(app);
        app.pump();
        let settled = app
            .pending
            .values()
            .all(|(tag, _, _)| !tag.starts_with("settings.") && !tag.starts_with("app."));
        if settled && app.settings.was_open {
            break;
        }
        if std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    if app.settings.pending_page.is_some() {
        subpage::resolve_pending(app);
    }
    subpage::settle(app);
    // A requested anchor is revealed without the scroll (the snapshot shows the wash).
    if let Some((a, _)) = app.settings.pending_anchor.take() {
        if let Some(key) = search::reveal_key(app, a) {
            app.settings.open.insert(key, true);
        }
        app.settings.highlight = Some(a);
        app.settings.highlight_until = f64::MAX;
    }
}

// ---- The modal ----

/// The Settings modal, mounted by the shell while `app.shell.settings_open`.
pub fn modal(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) {
    let _ = cx;
    let sec = current(app);
    let w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    // `sm:p-6 lg:p-10`, `sm:max-w-[80rem] sm:max-h-[60rem]`, rounded and bordered from `sm`.
    let pad = if w >= 1024.0 {
        tk.r(2.5)
    } else if w >= 640.0 {
        tk.r(1.5)
    } else {
        0.0
    };
    let framed = w >= 640.0;
    let mw = (w - 2.0 * pad).min(if framed { tk.r(80.0) } else { w });
    // The overlay and its centring box start below the title bar (`top-[var(--kb-window-top-inset)]`).
    let inset = crate::shell::window_top_inset();
    let mh = (h - inset - 2.0 * pad).min(if framed { tk.r(60.0) } else { h - inset });
    // Closing Settings from a sub-page with changes asks first.
    let close_fn = |a: &mut App| subpage::leave(a, subpage::After::Close);
    // The surface's `modal` dismissal. Rune hands Escape to the topmost modal before the app's
    // `on_key` sees it, so Escape lands here: on a sub-page it goes back to the section (the
    // innermost level); a press outside closes Settings.
    let dismiss_fn = |a: &mut App, tx: &mut rune::Tx| {
        if tx.by_keys() && subpage::active(a).is_some() {
            subpage::leave(a, subpage::After::Back);
        } else {
            subpage::leave(a, subpage::After::Close);
        }
    };
    let page = subpage::active(app);
    // `bg-background/60 dark:bg-background/64 backdrop-blur-xs`.
    layer.push(
        el().key("set-backdrop")
            .pin(Some(0.0), Some(inset), Some(0.0), Some(0.0))
            .fill(tk.background.alpha(if tk.dark { 0.64 } else { 0.6 }))
            .backdrop_blur(4.0)
            .role("none")
            .on_press(close_fn),
    );
    let mut overlays: Vec<El> = Vec::new();
    let body = section_body(app, tk, sec, &mut overlays);
    let content = col().w_full().padding(tk.r(1.25)).child(
        col()
            .w_full()
            .max_w(1100.0)
            .min_w(0.0)
            .child(
                col()
                    .w_full()
                    .gap(tk.r(0.25))
                    .pb(tk.r(0.75))
                    .mb(tk.r(0.75))
                    .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                    .border_layout()
                    .child(
                        text(t(&format!(
                            "settings:nav.section.{}.label",
                            SECTIONS[sec].key
                        )))
                        .font(tk.tlg(600.0).lh(tk.r(1.125) * 1.25))
                        .color(tk.fg)
                        .role("heading")
                        .level(2),
                    )
                    .child(
                        text(t(&format!(
                            "settings:nav.section.{}.description",
                            SECTIONS[sec].key
                        )))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .max_w(tk.r(48.0)),
                    ),
            )
            .child(body),
    );
    let mut scroll = col()
        .key(format!("set-body-{}", SECTIONS[sec].slug))
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(content);
    // A sub-page covers the section's content area. The section stays mounted underneath,
    // inert and out of the accessibility tree, so its scroll position is there on Back.
    let mut area = col().key("set-content").w_full().grow(1.0).min_h(0.0);
    let crumb = page.map(|k| subpage::title(app, k));
    if let Some(k) = page {
        scroll = scroll.inert().a11y_hidden();
        area = area
            .child(scroll)
            .child(subpage::view(app, tk, k, &mut overlays));
    } else {
        area = area.child(scroll);
    }
    let main = col()
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .child(header(tk, sec, crumb))
        .child(area);
    let mut surface = row()
        .key("set-dialog")
        .w(mw)
        .h(mh)
        .fill(tk.background)
        .role("dialog")
        .label(t("settings:page.title"))
        .modal_tx(true, dismiss_fn)
        .child(rail(app, tk, sec, w < 768.0))
        .child(main);
    if framed {
        surface = surface
            .radius(tk.r(1.0))
            .border(1.0, tk.border)
            .border_layout()
            .clip_rounded(tk.r(1.0))
            .box_shadow(Shadow {
                y: 25.0,
                blur: 50.0,
                spread: -12.0,
                color: Color::BLACK.alpha(0.4),
                ..Shadow::default()
            });
    }
    layer.push(
        row()
            .key("set-layer")
            .pin(Some(0.0), Some(inset), Some(0.0), Some(0.0))
            .center()
            .child(surface),
    );
    layer.extend(overlays);
    dialogs::mount(app, tk, layer);
}

fn section_body(app: &App, tk: &Tk, sec: usize, layer: &mut Vec<El>) -> El {
    match sec {
        APPEARANCE => appearance::view(app, tk, layer),
        MARKET => market::view(app, tk, layer),
        BITCOIN => network::view(app, tk, layer, model::Layer::Bitcoin),
        LIGHTNING => network::view(app, tk, layer, model::Layer::Lightning),
        LIQUID => network::view(app, tk, layer, model::Layer::Liquid),
        PRIVACY => privacy::view(app, tk, layer),
        SECURITY => security::view(app, tk, layer),
        AI => ai::view(app, tk, layer),
        DATA => data::view(app, tk, layer),
        TERMINAL => desktop::terminal_view(app, tk),
        _ => desktop::developer_view(app, tk),
    }
}

/// `SettingsModalBody` header: `h-12 border-b ps-5 pe-2`, the breadcrumb and the close button.
/// On a sub-page the breadcrumb reads Settings › <section> › <page>, the section crumb going
/// back.
fn header(tk: &Tk, sec: usize, page: Option<String>) -> El {
    let page_open = page.is_some();
    let label = t(&format!("settings:nav.section.{}.label", SECTIONS[sec].key));
    let close_label = t("settings:page.close");
    let chevron = || ic(&I::CHEVRON_RIGHT, tk.r(0.875), tk.muted_fg.alpha(0.7)).a11y_hidden();
    let mut crumbs = row()
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.375))
        .center_y()
        .role("navigation")
        .label(t("settings:page.title"))
        .child(
            text(t("settings:page.title"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
        )
        .child(chevron());
    crumbs = match page {
        Some(p) => crumbs
            .child(subpage::crumb(tk, sec))
            .child(chevron())
            .child(text(p).font(tk.tsm(500.0)).color(tk.fg).truncate()),
        None => crumbs.child(
            text(label.clone())
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .truncate(),
        ),
    };
    // On a sub-page the breadcrumb starts with its one back control (Escape does the same).
    if page_open {
        let back = format!("{} · {label}", t("common:actions.back"));
        crumbs = row()
            .grow(1.0)
            .min_w(0.0)
            .gap(tk.r(0.5))
            .center_y()
            .child(
                crate::txdetail::widgets::button_surface(
                    tk,
                    row()
                        .key("set-page-back")
                        .size(tk.r(2.0), tk.r(2.0))
                        .center(),
                    crate::txdetail::widgets::Variant::Ghost,
                )
                .role("button")
                .label(back.clone())
                .help(format!("{back} (Esc)"))
                .focusable(true)
                .on_press(|a: &mut App| subpage::leave(a, subpage::After::Back))
                .child(ic(&I::CHEVRON_LEFT, tk.r(1.0), tk.fg).a11y_hidden()),
            )
            .child(crumbs);
    }
    row()
        .w_full()
        .h(tk.r(3.0))
        .pl(if page_open { tk.r(0.75) } else { tk.r(1.25) })
        .pr(tk.r(0.5))
        .gap(tk.r(0.75))
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(crumbs)
        .child(
            crate::txdetail::widgets::button_surface(
                tk,
                row().key("set-close").size(tk.r(2.0), tk.r(2.0)).center(),
                crate::txdetail::widgets::Variant::Ghost,
            )
            .role("button")
            .label(close_label.clone())
            .help(format!("{close_label} (Esc)"))
            .focusable(true)
            .on_press(|a: &mut App| subpage::leave(a, subpage::After::Close))
            .child(ic(&I::X, tk.r(1.0), tk.fg)),
        )
}

/// Per-layer backend counts on the rail (`counts`).
fn counts(app: &App) -> [usize; 11] {
    let list: Vec<model::Backend> = app
        .read("ui.backends.settings.list")
        .and_then(|v| v["backends"].as_array())
        .map(|a| a.iter().map(model::backend_from_row).collect())
        .unwrap_or_default();
    let mut c = [0usize; 11];
    c[BITCOIN] = model::for_layer(&list, model::Layer::Bitcoin).len();
    c[LIGHTNING] = model::for_layer(&list, model::Layer::Lightning).len();
    c[LIQUID] = model::for_layer(&list, model::Layer::Liquid).len();
    c[MARKET] = list.iter().filter(|b| b.net_kind == model::Net::Fx).count();
    c
}

/// `SettingsSidebarNav`: grouped rows on the sidebar surface; an icon rail below `md`.
fn rail(app: &App, tk: &Tk, active: usize, narrow: bool) -> El {
    let counts = counts(app);
    let ink = tk.sidebar_fg;
    let muted = tk.sidebar_muted_fg;
    let mut groups = col().w_full().gap(tk.r(1.0));
    for g in GROUPS {
        let items: Vec<usize> = (0..SECTIONS.len())
            .filter(|&i| SECTIONS[i].group == g && (app.prefs.developer_tools || !dev_only(i)))
            .collect();
        if items.is_empty() {
            continue;
        }
        let heading = t(&format!("settings:nav.group.{g}"));
        let mut list = col().w_full().gap(tk.r(0.125));
        for i in items {
            let label = t(&format!("settings:nav.section.{}.label", SECTIONS[i].key));
            let on = i == active;
            let mut r = row()
                .key(format!("set-nav-{}", SECTIONS[i].slug))
                .w_full()
                .h(tk.r(2.0))
                .px(tk.r(0.5))
                .gap(tk.r(0.5))
                .center_y()
                .radius(tk.r(0.375))
                .role("link")
                .label(label.clone())
                .help(label.clone())
                .focusable(true)
                .on_press(move |a: &mut App| {
                    // From a sub-page any section (its own too) leaves the page first.
                    if subpage::active(a).is_some() {
                        subpage::leave(a, subpage::After::Section(i));
                    } else {
                        open_index(a, i);
                    }
                })
                .child(
                    ic(
                        SECTIONS[i].icon,
                        tk.r(1.0),
                        if on { ink } else { muted.alpha(0.7) },
                    )
                    .a11y_hidden(),
                );
            if on {
                r = r.fill(tk.row_active).checked(true);
            } else {
                r = r.hover(tk.row_hover);
            }
            if narrow {
                r = r.justify_center();
            } else {
                r = r.child(
                    text(label)
                        .font(tk.tsm(500.0))
                        .color(if on { ink } else { muted })
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                );
                if counts[i] > 0 {
                    r = r.child(
                        text(counts[i].to_string())
                            .font(tk.txs(400.0).features("'tnum' 1"))
                            .color(muted),
                    );
                }
            }
            list = list.child(r);
        }
        let mut gcol = col()
            .w_full()
            .gap(tk.r(0.25))
            .role("group")
            .label(heading.clone());
        if !narrow {
            gcol = gcol.child(
                text(heading)
                    .font(tk.txs(500.0))
                    .color(muted.alpha(0.8))
                    .px(tk.r(0.5)),
            );
        }
        groups = groups.child(gcol.child(list));
    }
    col()
        .key("set-rail")
        .w(if narrow { tk.r(3.5) } else { tk.r(15.0) })
        .h_full()
        .fill(tk.chrome)
        .border_sides([0.0, 1.0, 0.0, 0.0], tk.border)
        .border_layout()
        .px(tk.r(0.5))
        .py(tk.r(0.75))
        .role("navigation")
        .label(t("settings:nav.ariaLabel"))
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(groups)
}

// ---- Scenes ----

/// The daemon's fixed connection-test lines for the two certificate problems.
const UNTRUSTED_LINE: &str =
    "Connection failed: The server's TLS certificate is not signed by an authority Kassiber trusts";
const CHANGED_LINE: &str = "Connection failed: This server's certificate changed: it no longer matches the certificate trusted for this backend";

/// A synthetic saved Electrum backend over TLS, opened for editing (no daemon state needed).
fn cert_scene(a: &mut App, then: fn(&mut App)) {
    open_scene(a, BITCOIN);
    let b = model::Backend {
        id: "home".into(),
        name: "Home node".into(),
        url: "ssl://node.local:50002".into(),
        net_kind: model::Net::Btc,
        kind: Some("electrum".into()),
        chain: Some("bitcoin".into()),
        network: Some("regtest".into()),
        auth: "none".into(),
        on: false,
        infrastructure_owner: Some(model::Owner::Mine),
        ..model::Backend::default()
    };
    backend::open_edit(a, &b);
    prime(a);
    then(a);
}

fn cert_test_fail(a: &mut App, line: &str) {
    a.settings.backend.test = "fail";
    a.settings.backend.test_log =
        format!("Opening Electrum connection to ssl://node.local:50002\n{line}");
}

fn cert_presented(changed: bool) -> certtrust::Presented {
    certtrust::Presented {
        backend: "home".into(),
        host: "node.local".into(),
        port: 50002,
        fingerprint: "81:4C:F5:98:8C:BF:B3:07:03:EA:D7:34:00:E1:33:39:A2:A4:53:E6:CE:4E:5F:FF:68:DD:8D:9C:5B:17:89:6A".into(),
        subject: Some("node.local".into()),
        issuer: Some("node.local".into()),
        not_before: Some("2026-01-01T00:00:00Z".into()),
        not_after: Some("2036-01-01T00:00:00Z".into()),
        self_signed: true,
        replaces: changed.then(|| "01:F4:A0:43:BF:C6:EC:5F:FD:F3:BD:53:2D:B6:45:B8:A9:79:36:70:93:1B:07:96:90:3F:F0:C4:72:1C:B3:E8".into()),
        clears_insecure: changed,
        revision: "5f".repeat(32),
        certificate: None,
    }
}

fn cert_dialog(
    a: &mut App,
    problem: Option<certtrust::Problem>,
    phase: certtrust::Phase,
    presented: Option<certtrust::Presented>,
    error: Option<&'static str>,
) {
    cert_scene(a, |_| {});
    certtrust::open(a, &backend::CERT_HOST, "home", "Home node", problem);
    let st = &mut a.settings.backend.cert;
    st.phase = phase;
    st.presented = presented;
    st.error = error;
}

fn open_scene(a: &mut App, i: usize) {
    a.shell.settings_open = true;
    a.shell.settings_section = i;
    prime(a);
}

/// Named states for `--scene` (headless snapshots and the parity harness).
pub const SCENES: &[crate::app::Scene] = &[
    ("settings-appearance", |a| open_scene(a, APPEARANCE)),
    ("settings-market", |a| open_scene(a, MARKET)),
    ("settings-bitcoin", |a| open_scene(a, BITCOIN)),
    ("settings-lightning", |a| {
        a.prefs.developer_tools = true;
        open_scene(a, LIGHTNING)
    }),
    ("settings-liquid", |a| open_scene(a, LIQUID)),
    ("settings-privacy", |a| open_scene(a, PRIVACY)),
    ("settings-security", |a| open_scene(a, SECURITY)),
    ("settings-ai", |a| open_scene(a, AI)),
    ("settings-data", |a| open_scene(a, DATA)),
    ("settings-terminal", |a| open_scene(a, TERMINAL)),
    ("settings-developer", |a| open_scene(a, DEVELOPER)),
    ("settings-developer-dev", |a| {
        a.prefs.developer_tools = true;
        open_scene(a, DEVELOPER)
    }),
    ("settings-appearance-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open_scene(a, APPEARANCE)
    }),
    ("settings-privacy-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open_scene(a, PRIVACY)
    }),
    ("settings-appearance-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open_scene(a, APPEARANCE)
    }),
    ("settings-bitcoin-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open_scene(a, BITCOIN)
    }),
    ("settings-backend-edit", |a| {
        open_scene(a, BITCOIN);
        let list = model::backends(a.read("ui.backends.settings.list"), None);
        if let Some(b) = list.into_iter().find(|b| b.id == "bitcoin-mempool-regtest") {
            backend::open_edit(a, &b);
        }
        prime(a);
    }),
    ("settings-backend-add", |a| {
        open_scene(a, LIQUID);
        backend::open_add(a, Some("liquid"));
        prime(a);
    }),
    ("settings-theme-select", |a| {
        open_scene(a, APPEARANCE);
        a.settings.select = Some("set-theme".into());
    }),
    ("settings-reset-book", |a| {
        open_scene(a, DATA);
        dialogs::open_reset_book(a);
    }),
    ("settings-search-jump", |a| {
        open_section(a, "privacy#clearClipboard");
        prime(a);
    }),
    ("settings-search-query", |a| {
        a.shell.popup = Some(crate::shell::Popup::Search);
        a.shell.search = rune::TextEdit::new("clipboard");
        a.shell.search.focused = true;
    }),
    ("settings-ai-form", |a| {
        open_scene(a, AI);
        aiform::open_add(a);
        aiform::scene_pick(a, "local", "ollama");
        subpage::rebase(a);
    }),
    // Sub-pages (owner request: flows open as pages inside the modal, not stacked dialogs).
    ("settings-page-backend-add", |a| {
        open_scene(a, BITCOIN);
        backend::open_add(a, Some("bitcoin"));
        prime(a);
    }),
    ("settings-page-backend-add-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open_scene(a, BITCOIN);
        backend::open_add(a, Some("bitcoin"));
        prime(a);
    }),
    ("settings-page-backend-add-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open_scene(a, BITCOIN);
        backend::open_add(a, Some("bitcoin"));
        prime(a);
    }),
    ("settings-page-backend-add-lightning", |a| {
        a.prefs.developer_tools = true;
        open_scene(a, LIGHTNING);
        backend::open_add(a, Some("lnd"));
        prime(a);
    }),
    ("settings-page-backend-edit-light-de", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.prefs.lang = crate::i18n::Lang::De;
        open_scene(a, BITCOIN);
        let list = model::backends(a.read("ui.backends.settings.list"), None);
        if let Some(b) = list.into_iter().find(|b| b.id == "bitcoin-mempool-regtest") {
            backend::open_edit(a, &b);
        }
        prime(a);
    }),
    ("settings-page-backend-privacy", |a| {
        open_scene(a, PRIVACY);
        let list = model::backends(a.read("ui.backends.settings.list"), None);
        if let Some(b) = list.into_iter().find(|b| b.id == "bitcoin-mempool-regtest") {
            backend::open_edit(a, &b);
        }
        prime(a);
    }),
    ("settings-page-backend-discard", |a| {
        open_scene(a, BITCOIN);
        backend::open_add(a, Some("bitcoin"));
        prime(a);
        a.settings.backend.name = rune::TextEdit::new("My node");
        subpage::leave(a, subpage::After::Back);
    }),
    ("settings-page-backend-discard-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open_scene(a, BITCOIN);
        backend::open_add(a, Some("bitcoin"));
        prime(a);
        a.settings.backend.name = rune::TextEdit::new("Mein Node");
        subpage::leave(a, subpage::After::Section(APPEARANCE));
    }),
    ("settings-page-backend-link", |a| {
        a.navigate("/settings/liquid#addBackend");
        prime(a);
    }),
    ("settings-page-ai-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open_scene(a, AI);
        aiform::open_add(a);
        aiform::scene_pick(a, "local", "ollama");
        subpage::rebase(a);
    }),
    ("settings-page-ai-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open_scene(a, AI);
        aiform::open_add(a);
        aiform::scene_pick(a, "local", "ollama");
        subpage::rebase(a);
    }),
    ("settings-page-ai-link", |a| {
        a.navigate("/settings/ai#addProvider");
        prime(a);
    }),
    ("settings-page-reset-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open_scene(a, DATA);
        dialogs::open_reset_book(a);
    }),
    ("settings-page-reset-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open_scene(a, DATA);
        dialogs::open_reset_book(a);
    }),
    ("settings-page-delete-books", |a| {
        open_scene(a, DATA);
        dialogs::open_delete_books(a);
    }),
    ("settings-page-delete-books-light-de", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.prefs.lang = crate::i18n::Lang::De;
        open_scene(a, DATA);
        dialogs::open_delete_books(a);
    }),
    ("settings-page-passphrase", |a| {
        open_scene(a, SECURITY);
        dialogs::open_change_passphrase(a);
    }),
    ("settings-page-passphrase-light-de", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.prefs.lang = crate::i18n::Lang::De;
        open_scene(a, SECURITY);
        dialogs::open_change_passphrase(a);
    }),
    ("settings-page-touchid", |a| {
        open_scene(a, SECURITY);
        dialogs::open_touch_id(a);
    }),
    ("settings-page-touchid-light-de", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.prefs.lang = crate::i18n::Lang::De;
        open_scene(a, SECURITY);
        dialogs::open_touch_id(a);
    }),
    ("settings-page-delete-confirm", |a| {
        open_scene(a, BITCOIN);
        let list = model::backends(a.read("ui.backends.settings.list"), None);
        if let Some(b) = list.into_iter().find(|b| b.id == "bitcoin-mempool-regtest") {
            backend::open_edit(a, &b);
            dialogs::confirm(
                a,
                crate::i18n::tr("settings:deleteBackend.confirm", &[("name", &b.name)]),
                dialogs::Confirmed::DeleteBackend(b.id.clone()),
            );
        }
        prime(a);
    }),
    ("settings-page-search", |a| {
        a.shell.popup = Some(crate::shell::Popup::Search);
        a.shell.search = rune::TextEdit::new("add bitcoin");
        a.shell.search.focused = true;
    }),
    // ---- Certificate trust (synthetic states: a made-up saved backend and daemon replies,
    // nothing is fetched; `certdelta-real-*` use a backend on a local 127.0.0.1 TLS server) ----
    ("certdelta-cert-row", |a| cert_scene(a, |_| {})),
    ("certdelta-cert-row-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        cert_scene(a, |_| {})
    }),
    ("certdelta-cert-test-untrusted", |a| {
        cert_scene(a, |a| cert_test_fail(a, UNTRUSTED_LINE))
    }),
    ("certdelta-cert-test-changed", |a| {
        cert_scene(a, |a| cert_test_fail(a, CHANGED_LINE))
    }),
    ("certdelta-cert-test-changed-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        cert_scene(a, |a| cert_test_fail(a, CHANGED_LINE))
    }),
    ("certdelta-cert-unsaved", |a| {
        cert_scene(a, |a| {
            a.settings.backend.e_host = rune::TextEdit::new("other.local");
            cert_test_fail(a, UNTRUSTED_LINE);
        })
    }),
    ("certdelta-cert-dialog-intro", |a| {
        cert_dialog(
            a,
            Some(certtrust::Problem::Untrusted),
            certtrust::Phase::Intro,
            None,
            None,
        )
    }),
    ("certdelta-cert-dialog-review", |a| {
        cert_dialog(
            a,
            None,
            certtrust::Phase::Review,
            Some(cert_presented(false)),
            None,
        )
    }),
    ("certdelta-cert-dialog-review-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        cert_dialog(
            a,
            None,
            certtrust::Phase::Review,
            Some(cert_presented(false)),
            None,
        )
    }),
    ("certdelta-cert-dialog-review-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        cert_dialog(
            a,
            None,
            certtrust::Phase::Review,
            Some(cert_presented(false)),
            None,
        )
    }),
    ("certdelta-cert-dialog-changed", |a| {
        cert_dialog(
            a,
            Some(certtrust::Problem::Changed),
            certtrust::Phase::Review,
            Some(cert_presented(true)),
            None,
        )
    }),
    ("certdelta-cert-dialog-changed-intro", |a| {
        cert_dialog(
            a,
            Some(certtrust::Problem::Changed),
            certtrust::Phase::Intro,
            None,
            None,
        )
    }),
    ("certdelta-cert-dialog-changed-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        cert_dialog(
            a,
            Some(certtrust::Problem::Changed),
            certtrust::Phase::Review,
            Some(cert_presented(true)),
            None,
        )
    }),
    ("certdelta-cert-dialog-changed-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        cert_dialog(
            a,
            Some(certtrust::Problem::Changed),
            certtrust::Phase::Review,
            Some(cert_presented(true)),
            None,
        )
    }),
    ("certdelta-cert-dialog-fetching", |a| {
        cert_dialog(
            a,
            Some(certtrust::Problem::Untrusted),
            certtrust::Phase::Fetching,
            None,
            None,
        )
    }),
    ("certdelta-cert-dialog-done", |a| {
        let mut p = cert_presented(false);
        p.certificate = Some("/data/certificates/814cf598.pem".into());
        cert_dialog(a, None, certtrust::Phase::Done, Some(p), None)
    }),
    ("certdelta-cert-dialog-mismatch", |a| {
        cert_dialog(a, None, certtrust::Phase::Error, None, Some("mismatch"))
    }),
    ("certdelta-cert-dialog-expired", |a| {
        cert_dialog(a, None, certtrust::Phase::Error, None, Some("unpinnable"))
    }),
    ("certdelta-cert-dialog-offline-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        cert_dialog(a, None, certtrust::Phase::Error, None, Some("offline"))
    }),
    // The notification centre with failing sources grouped by cause (a synthetic freshness
    // snapshot: one changed certificate, one untrusted, one refused connection).
    ("certdelta-notifications-causes", |a| {
        crate::wallets::common::settle(a);
        let sources = serde_json::json!({"sources": [
            {"source_key": "w:1", "source_label": "Cold storage", "status": "failed", "last_error_message": CHANGED_LINE},
            {"source_key": "w:2", "source_label": "Spending", "status": "failed", "last_error_message": CHANGED_LINE},
            {"source_key": "w:3", "source_label": "Home node", "status": "failed", "last_error_message": UNTRUSTED_LINE},
            {"source_key": "w:4", "source_label": "Savings", "status": "failed", "last_error_message": "network I/O failed: connection refused"}
        ]});
        a.reads.insert(
            "ui.freshness.status".into(),
            crate::app::Reply {
                kind: "ui.freshness.status".into(),
                data: sources,
                error: None,
                error_detail: Value::Null,
                elapsed: std::time::Duration::ZERO,
            },
        );
        a.shell.toggle(crate::shell::Popup::Notifications);
    }),
    // The real flow's starting point: the backend `local-tls` (a local self-signed TLS server)
    // opened for editing. Nothing is contacted until a script presses Test / Fetch / Trust.
    ("certdelta-real-edit", |a| {
        open_scene(a, BITCOIN);
        let list = model::backends(a.read("ui.backends.settings.list"), None);
        if let Some(b) = list.into_iter().find(|b| b.id == "local-tls") {
            backend::open_edit(a, &b);
        }
        prime(a);
    }),
    // Its dialog opened from the row (the intro; nothing fetched).
    ("certdelta-real-dialog", |a| {
        open_scene(a, BITCOIN);
        let list = model::backends(a.read("ui.backends.settings.list"), None);
        if let Some(b) = list.into_iter().find(|b| b.id == "local-tls") {
            backend::open_edit(a, &b);
            certtrust::open(a, &backend::CERT_HOST, &b.id, &b.name, None);
        }
        prime(a);
    }),
    ("certdelta-real-dialog-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open_scene(a, BITCOIN);
        let list = model::backends(a.read("ui.backends.settings.list"), None);
        if let Some(b) = list.into_iter().find(|b| b.id == "local-tls") {
            backend::open_edit(a, &b);
            certtrust::open(a, &backend::CERT_HOST, &b.id, &b.name, None);
        }
        prime(a);
    }),
    ("settings-ai-expanded", |a| {
        open_scene(a, AI);
        a.settings.open.insert("set-ai-card-ollama".into(), true);
        a.settings.open.insert("set-ai-notsetup".into(), true);
    }),
    // The pages' Advanced folds opened (and the book-network connection list).
    ("settings-market-advanced", |a| {
        open_scene(a, MARKET);
        a.settings.open.insert(market::ADVANCED.into(), true);
    }),
    ("settings-security-advanced", |a| {
        open_scene(a, SECURITY);
        a.settings.open.insert("set-adv-security".into(), true);
    }),
    ("settings-data-advanced", |a| {
        open_scene(a, DATA);
        a.settings.open.insert(data::ADVANCED.into(), true);
    }),
    ("settings-bitcoin-connections", |a| {
        open_scene(a, BITCOIN);
        a.settings.open.insert("set-bn-wallets".into(), true);
    }),
    // Outcome rows with their × (synthetic outcomes: nothing is imported or written).
    ("settings-market-imported", |a| {
        open_scene(a, MARKET);
        a.settings.open.insert(market::ADVANCED.into(), true);
        a.settings.market.import_error = Some(t("settings:marketData.importPathError"));
        a.settings.market.import_result = Some(json!({
            "totals": {"pairs": 1, "samples": 1440, "skipped_rows": 0, "skipped_files": 0},
            "summary": [{"pair": "BTC-EUR", "first_timestamp": "2025-01-01T00:00:00Z", "last_timestamp": "2025-01-01T23:59:00Z", "samples": 1440, "granularity": "minute"}],
        }));
    }),
    ("settings-data-backup-done", |a| {
        open_scene(a, DATA);
        a.settings.open.insert(data::ADVANCED.into(), true);
        a.settings.backup.message = Some(crate::i18n::tr(
            "settings:backup.exported",
            &[("path", "/tmp/kassiber-backup.kbk")],
        ));
    }),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_aliases_resolve() {
        assert_eq!(section_for_slug("backends"), Some(BITCOIN));
        assert_eq!(section_for_slug("#lock"), Some(SECURITY));
        assert_eq!(section_for_slug("Storage"), Some(DATA));
        assert_eq!(section_for_slug("logs"), Some(DEVELOPER));
        assert_eq!(section_for_slug("rates"), Some(MARKET));
        assert_eq!(section_for_slug("nope"), None);
        for (i, s) in SECTIONS.iter().enumerate() {
            assert_eq!(section_for_slug(s.slug), Some(i));
        }
    }
}

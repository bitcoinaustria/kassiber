//! The window frame around every screen: sidebar, title bar, page panel, overlays.
//! (ui-tauri/src/components/kb/AppShell.tsx and friends.)
//!
//! Layout follows the web shell: a 40 pt title bar over a body row of the sidebar (16 rem, or a
//! 3 rem icon rail) and the page panel, a rounded card inset 0.375 rem from the right and
//! bottom. Popups (menus, popovers, the search palette) are one at a time (`ShellUi::popup`)
//! and mount into the overlay layer above everything.

pub(crate) mod books;
pub(crate) mod crash;
mod keys;
pub(crate) mod maint;
mod network;
mod notifications;
pub(crate) mod overlays;
pub(crate) mod search;
pub(crate) mod sidebar;
pub(crate) mod titlebar;
pub(crate) mod transient;
pub(crate) mod wordmark;

use rune::{Color, Cx, El, Icon, ScrollSpec, Shadow, TextEdit, col, el, row, text};
use rune_kit::Placement;
use serde_json::Value;

use crate::app::{App, Reply, ToastTone};
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

pub use keys::on_key;
pub use transient::on_leave;

/// Which shell popup is open (only one at a time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popup {
    Search,
    Books,
    UserMenu,
    RefreshMenu,
    Notifications,
    Network,
    Theme,
    Privacy,
    Alpha,
}

/// A connection probe's outcome, by backend name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    Waiting,
    Running,
    Ok,
    Failed(String),
}

/// A finished job or event for the Notifications "Recent" section (RAM only, like the web).
#[derive(Clone, Debug)]
pub struct Recent {
    pub title: String,
    pub body: Option<String>,
    pub tone: ToastTone,
    pub at: f64,
    /// The book it was filed under (`app::book_key`); shown only while that book is open.
    pub book: String,
}

pub struct ShellUi {
    pub popup: Option<Popup>,
    /// The sync card is in the foreground (not "Continue in background").
    pub sync_card_open: bool,
    /// The book refresh failed condition (`book-refresh.failed`), until a run returns.
    pub sync_failed: bool,
    /// The running or failed job's progress (`activeMaintenanceProgress`).
    pub maint: Option<maint::Maint>,
    /// The book refresh in flight (its option and progress so far).
    pub run: maint::Run,
    /// The journal-processing failed condition (`journals.process-failed`), with its message.
    pub journals_failed: Option<String>,
    /// The "Skip to main content" link has keyboard focus (it shows only then, `sr-only
    /// focus:not-sr-only`).
    pub skip_focused: bool,
    pub support_open: bool,
    pub extras_open: bool,
    /// The narrow-window rail opened by the toggle (never rewrites `prefs.nav_collapsed`).
    pub nav_peek: bool,
    pub search: TextEdit,
    pub search_sel: usize,
    /// The last `ui.transactions.resolve` answer for a txid-like query: (query, transaction).
    pub resolved: Option<(String, Value)>,
    /// The txid lookup in flight; only its own answer is taken.
    pub resolving: Option<(String, crate::app::Ticket)>,
    /// A txid-shaped query waiting for typing to pause: (query, app-clock time it is sent).
    pub resolve_due: Option<(String, f64)>,
    pub books_filter: TextEdit,
    pub books_sel: usize,
    pub switching_profile: Option<String>,
    pub books_error: Option<String>,
    pub settings_open: bool,
    /// The Settings modal's selected section (index into `settings::SECTIONS`).
    pub settings_section: usize,
    pub recent: Vec<Recent>,
    /// The open book's key (`app::book_key`), None while it is unknown (a switch, the lock).
    pub book: Option<String>,
    pub probes: Vec<(String, Probe)>,
    pub probing: bool,
    /// The window's size as last laid out (narrow-window rail, ⌘B, palette height).
    pub window_w: f32,
    pub window_h: f32,
    /// The page panel's rect as last laid out: the refresh card's layer covers exactly it.
    pub panel_rect: [f32; 4],
    /// The animated wordmarks (nav, lock screen, Welcome, the sync card's K).
    pub marks: wordmark::Marks,
    /// The route path the leave hook last saw (`transient::on_leave`).
    pub page: String,
}

impl Default for ShellUi {
    fn default() -> Self {
        ShellUi {
            popup: None,
            sync_card_open: false,
            sync_failed: false,
            maint: None,
            run: maint::Run::default(),
            journals_failed: None,
            skip_focused: false,
            support_open: false,
            extras_open: false,
            nav_peek: false,
            search: TextEdit::new(""),
            search_sel: 0,
            resolved: None,
            resolving: None,
            resolve_due: None,
            books_filter: TextEdit::new(""),
            books_sel: 0,
            switching_profile: None,
            books_error: None,
            settings_open: false,
            settings_section: 0,
            recent: vec![],
            book: None,
            probes: vec![],
            probing: false,
            window_w: 0.0,
            panel_rect: [0.0; 4],
            window_h: 0.0,
            marks: Default::default(),
            page: String::new(),
        }
    }
}

impl ShellUi {
    pub fn close_popups(&mut self) {
        self.popup = None;
    }

    pub fn toggle(&mut self, p: Popup) {
        self.popup = if self.popup == Some(p) { None } else { Some(p) };
        if self.popup == Some(Popup::Search) {
            self.search = TextEdit::new("");
            self.search.focused = true;
            self.search_sel = 0;
        }
        if self.popup == Some(Popup::Books) {
            self.books_filter = TextEdit::new("");
            self.books_filter.focused = true;
            self.books_sel = 0;
            self.books_error = None;
        }
    }

    /// Files a Recent entry under the open book (`lib/notificationScope.ts`): while the open
    /// book is unknown nothing is filed, so no entry lands in the wrong book (the toast stays).
    pub fn note(&mut self, title: &str, body: Option<String>, tone: ToastTone, at: f64) {
        let Some(book) = self.book.clone() else {
            return;
        };
        self.recent.insert(
            0,
            Recent {
                title: title.to_owned(),
                body,
                tone,
                at,
                book,
            },
        );
        self.recent.truncate(20);
    }

    /// The open book's Recent entries.
    pub fn recent_here(&self) -> impl Iterator<Item = &Recent> {
        self.recent
            .iter()
            .filter(move |r| self.book.as_deref() == Some(r.book.as_str()))
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "shell.resolve" => search::on_resolve(app, r),
        "shell.switch_profile" => books::on_switched(app, r),
        "shell.select_project" => books::on_project_selected(app, r),
        "shell.offline_set" => network::on_offline_set(app, r),
        t if t.starts_with("shell.probe") => network::on_probe(app, r),
        _ => {}
    }
}

/// A progress record: the book refresh's and journal run's go to their maintenance progress.
pub fn on_progress(app: &mut App, tag: &str, env: &Value) {
    maint::on_progress(app, tag, env);
}

/// Unsolicited daemon events, after `App` started the book refresh cycle the freshness and
/// watch-inbox events ask for (`app::is_book_refresh_event`). Nothing else to do here yet.
pub fn on_event(_app: &mut App, _env: &Value) {}

pub struct NavItem {
    /// i18n key of the label (`nav:book.*`); `NavItem::text` translates it.
    pub label: &'static str,
    pub icon: &'static Icon,
    pub href: &'static str,
}

impl NavItem {
    pub fn text(&self) -> String {
        t(self.label)
    }
}

pub const MAIN_NAV: &[NavItem] = &[
    NavItem {
        label: "nav:book.overview",
        icon: &I::GAUGE,
        href: "/overview",
    },
    NavItem {
        label: "nav:book.transactions",
        icon: &I::CLIPBOARD_LIST,
        href: "/transactions",
    },
    NavItem {
        label: "nav:book.wallets",
        icon: &I::WALLET_CARDS,
        href: "/connections",
    },
    NavItem {
        label: "nav:book.swaps",
        icon: &I::ARROW_LEFT_RIGHT,
        href: "/transfers",
    },
    NavItem {
        label: "nav:book.review",
        icon: &I::INBOX,
        href: "/review",
    },
    NavItem {
        label: "nav:book.reports",
        icon: &I::CHART_COLUMN,
        href: "/reports",
    },
    NavItem {
        label: "nav:book.assistant",
        icon: &I::MESSAGE_SQUARE_TEXT,
        href: "/assistant",
    },
];

/// Every page a route can show, for the title and the palette: (path, i18n key of the title).
/// `/diagnostics` only redirects to Logs in the web, so it shares Logs' title.
pub const PAGES: &[(&str, &str)] = &[
    ("/overview", "nav:book.overview"),
    ("/transactions", "nav:book.transactions"),
    ("/connections", "nav:book.wallets"),
    ("/review", "nav:book.review"),
    ("/reports", "nav:book.reports"),
    ("/assistant", "nav:book.assistant"),
    ("/activity", "nav:book.activity"),
    ("/logs", "nav:book.logs"),
    ("/transfers", "nav:book.swaps"),
    ("/journals", "nav:book.ledger"),
    ("/reconcile", "nav:book.reconcile"),
    ("/source-of-funds", "chrome:routeMeta.sourceOfFunds.title"),
    ("/exit-tax", "nav:book.exitTax"),
    ("/privacy-mirror", "nav:book.privacyMirror"),
    ("/chain-analysis", "nav:book.chainAnalysis"),
    ("/egress", "nav:book.egress"),
    ("/books", "chrome:routeMeta.books.title"),
    ("/diagnostics", "nav:book.logs"),
    ("/imports", "chrome:routeMeta.imports.title"),
    ("/custody-gaps", "nav:book.custodyGaps"),
    ("/settings", "nav:book.settings"),
];

/// The route-level guards (`routeTree.tsx` `beforeLoad`): the identity gate, the developer
/// pages (`requireDeveloperTools`) and the assistant with AI features off. Every navigation
/// passes them: `App::navigate` (through `redirect`), Back/Forward from the keyboard, the title
/// bar, mouse buttons and the history swipe (`App::go_back`/`go_forward`), native intents and
/// search. A hidden link is not an access boundary (ui-tauri/AGENTS.md).
pub fn route_guard(app: &App, path: &str) -> Option<String> {
    if let Some(target) = crate::onboarding::redirect(app, path) {
        return Some(target);
    }
    let dev_only =
        DEV_LOCKED.contains(&path) || DEV_HIDDEN.contains(&path) || path == "/diagnostics";
    if dev_only && !app.prefs.developer_tools {
        return Some("/overview".into());
    }
    if crate::assistant::owns(path) && !app.prefs.ai_features {
        return Some("/overview".into());
    }
    None
}

/// The web router's redirects (`routeTree.tsx`), applied by `App::navigate`: the target to land
/// on instead, or None to navigate as asked. `/settings[/<slug>]` opens the Settings modal over
/// the Overview, as `openSettingsOverOverview` does.
pub fn redirect(app: &mut App, to: &str) -> Option<String> {
    let path = to.split(['?', '#']).next().unwrap_or(to);
    let query = &to[path.len()..];
    // The guards first (`/settings/lightning` is the Settings modal's own to refuse).
    if !path.starts_with("/settings")
        && let Some(target) = route_guard(app, path)
    {
        return Some(target);
    }
    match path {
        "/quarantine" => Some(format!("/review{query}")),
        "/tax-events" => Some("/journals".into()),
        "/transfers" | "/swaps" | "/custody-gaps" => crate::custody::redirect(app, path, query),
        "/profiles" => Some("/books".into()),
        "/assitant" => Some(
            if app.prefs.ai_features {
                "/assistant"
            } else {
                "/overview"
            }
            .into(),
        ),
        "/proof-funds" => Some("/source-of-funds".into()),
        "/diagnostics" => Some("/logs".into()),
        p if p == "/settings" || p.starts_with("/settings/") => {
            // `<slug>`, `<slug>#<anchor>` (a setting-level search result) or a legacy `#<slug>`;
            // the page under the modal stays (`useSettingsModalNavigation`).
            crate::settings::open_section(
                app,
                to.strip_prefix("/settings")
                    .unwrap_or("")
                    .trim_start_matches('/'),
            );
            Some(app.route.0.clone())
        }
        _ => None,
    }
}

/// Routes greyed out and inert without developer tools.
pub const DEV_LOCKED: &[&str] = &[
    "/activity",
    "/custody-gaps",
    "/exit-tax",
    "/source-of-funds",
];
/// Routes whose rows disappear without developer tools.
pub const DEV_HIDDEN: &[&str] = &["/egress", "/logs", "/settings/lightning"];

/// The translated title of a route, in the current language. Interned so callers keep the
/// `&'static str` they had before localization (at most a few dozen strings per language).
pub fn page_title(path: &str) -> &'static str {
    let key = PAGES
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, k)| *k)
        .or_else(|| {
            path.starts_with("/connections/")
                .then_some("chrome:routeMeta.walletDetail.title")
        })
        .or_else(|| {
            path.starts_with("/books/")
                .then_some("chrome:routeMeta.booksOverview.title")
        })
        .unwrap_or("chrome:shell.fallbackTitle");
    intern(t(key))
}

fn intern(s: String) -> &'static str {
    thread_local! {
        static INTERNED: std::cell::RefCell<std::collections::HashSet<&'static str>> = Default::default();
    }
    INTERNED.with(|set| {
        let mut set = set.borrow_mut();
        if let Some(found) = set.get(s.as_str()) {
            return *found;
        }
        let leaked: &'static str = Box::leak(s.into_boxed_str());
        set.insert(leaked);
        leaked
    })
}

/// Navigation from a nav link (the route guard runs in `App::navigate`).
pub fn go(app: &mut App, href: &str) {
    app.navigate(href);
}

/// Whether the nav shows as the icon rail: the saved preference, or a window narrower than
/// 1100 pt (plus a docked side panel) unless the toggle opened it for now.
pub fn nav_folded(app: &App, cx: &Cx<App>) -> bool {
    if app.prefs.nav_collapsed {
        return true;
    }
    narrow(app, cx) && !app.shell.nav_peek
}

pub fn narrow(app: &App, cx: &Cx<App>) -> bool {
    let w = cx
        .layout_size("sh-root")
        .map(|[w, _]| w)
        .unwrap_or(app.shell.window_w);
    narrow_at(app, w)
}

/// Narrow, from the width the root last reported (key handlers have no `Cx`).
pub fn narrow_now(app: &App) -> bool {
    narrow_at(app, app.shell.window_w)
}

fn narrow_at(app: &App, w: f32) -> bool {
    let panel = crate::assistant::docked_width(app) + crate::txdetail::docked_width(app);
    w > 0.0 && w < 1100.0 + panel
}

/// The toggle: in a narrow window it opens the rail only for now.
pub fn toggle_nav(app: &mut App, narrow: bool) {
    if narrow {
        app.shell.nav_peek = !app.shell.nav_peek;
    } else {
        app.prefs.nav_collapsed = !app.prefs.nav_collapsed;
    }
}

/// `.kb-glass-popover`: popover fill, a hairline in the foreground at 10 %, a soft drop shadow.
pub(crate) fn glass(tk: &Tk, content: El) -> El {
    let shadow = if tk.dark {
        Shadow {
            y: 18.0,
            blur: 44.0,
            spread: -18.0,
            color: Color::BLACK.alpha(0.8),
            ..Shadow::default()
        }
    } else {
        Shadow {
            y: 16.0,
            blur: 40.0,
            spread: -18.0,
            color: Color::BLACK.alpha(0.55),
            ..Shadow::default()
        }
    };
    // `.kb-glass-popover`: color-mix(popover 18%, popover at 80 %) = popover at 83.6 %, over a
    // 12 px backdrop blur (saturate(1.14) isn't available and is left out).
    content
        .radius(tk.r(0.375))
        .fill(tk.popover.alpha(0.836))
        .backdrop_blur(12.0)
        .border(1.0, tk.fg.alpha(0.10))
        .border_layout()
        .box_shadow(shadow)
}

// ---- Glass overlay motion ----
//
// Rune plays a newly mounted subtree's entrance on its topmost new element and draws a fading
// element as one isolated layer, inside which a `backdrop_blur` sees only what that layer has
// drawn so far (nothing, under a transparent full-window wrapper). A glass surface under a
// fading layer root therefore shows its tint alone until the fade's spring settles (~0.8 s),
// then the blur pops in; leaving, the wrapper's fade drops the blur at once. So a glass surface
// is its own layer root wherever it can be (`centered`, `anchored`): its own opacity fades its
// blur with it, entering and leaving. Where a wrapper has to stay (a group of glass rows, a
// card positioned inside the page), the wrapper takes `layer_root`: it lands at once and the
// glass inside plays its own entrance.

/// `--kb-window-top-inset`: on macOS the window's own 40 pt title bar (WINDOW_TITLEBAR_HEIGHT_PX,
/// drawn over the content), elsewhere 0 (the native frame sits outside the page).
pub(crate) fn window_top_inset() -> f32 {
    if cfg!(target_os = "macos") { 40.0 } else { 0.0 }
}

/// A full-window overlay's wash under the title bar, as every web overlay sits
/// (`fixed inset-x-0 top-[var(--kb-window-top-inset)] bottom-0`): on macOS the 40 pt bar and its
/// traffic lights stay clear of the wash. Add the press handler / a11y role at the call site.
pub(crate) fn window_backdrop(key: impl Into<String>, fill: Color) -> El {
    el().key(key.into())
        .pin(Some(0.0), Some(window_top_inset()), Some(0.0), Some(0.0))
        .fill(fill)
}

/// Pins a full-window surface (an expanded chart or graph, a sheet, a full-window dialog layer)
/// to the window below the title bar: `inset-x-0 top-(--kb-window-top-inset) bottom-0`.
#[allow(dead_code)] // for the expanded chart/graph and the narrow-window side sheet (other areas)
pub(crate) fn pin_below_titlebar(surface: El) -> El {
    surface.pin(Some(0.0), Some(window_top_inset()), Some(0.0), Some(0.0))
}

/// Where `DialogContent` sits (ui/dialog.tsx): `top: calc(var(--kb-window-top-inset) +
/// var(--kb-dialog-top))`, `--kb-dialog-top: max(1rem, 8vh)`.
pub(crate) fn dialog_top(tk: &Tk, win_h: f32) -> f32 {
    window_top_inset() + tk.r(1.0).max(0.08 * win_h)
}

/// A wrapper that holds glass and must stay one (see above): no entrance of its own, so the
/// glass inside enters by its own transition with its blur from the first frame. It still fades
/// out as before when it goes, without the blur for those 0.2 s.
pub(crate) fn layer_root(el: El) -> El {
    el.transition(rune::Transition::asymmetric(
        rune::Transition::identity(),
        rune::Transition::opacity(),
    ))
}

/// A dialog surface `width` wide as its own layer root, centred in a `win_w` window at `top`:
/// what a full-window `row().justify_center().pt(top).child(surface)` places, without the
/// wrapper, so the surface's entrance and exit carry its blur.
pub(crate) fn centered(surface: El, win_w: f32, width: f32, top: f32) -> El {
    surface.absolute(((win_w - width) * 0.5).max(0.0), top)
}

/// `DialogContent`'s entrance (ui/dialog.tsx): `fade-in-0 zoom-in-98`, `duration-200`.
pub(crate) fn dialog_enter() -> rune::Transition {
    rune::Transition::scale(0.98)
}

/// `SheetContent side="right"` (ui/sheet.tsx): `fade-in-0 slide-in-from-right-8`, `duration-200`.
pub(crate) fn sheet_enter(tk: &Tk) -> rune::Transition {
    rune::Transition::offset(tk.r(2.0), 0.0).combined(rune::Transition::opacity())
}

/// Popover / DropdownMenu / Select content (ui/popover.tsx, dropdown-menu.tsx, select.tsx):
/// `fade-in-0 zoom-in-95` about the Radix transform origin (the trigger-facing edge at the
/// alignment point) with `slide-in-from-{toward the trigger}-2`, on `tw-animate-css`'s 150 ms.
pub(crate) fn popover_enter(tk: &Tk, side: rune::Side, align: rune::Align) -> rune::Transition {
    let along = match align {
        rune::Align::Start => 0.0,
        rune::Align::Center => 0.5,
        rune::Align::End => 1.0,
    };
    let d = tk.r(0.5);
    let ((ax, ay), (dx, dy)) = match side {
        rune::Side::Bottom => ((along, 0.0), (0.0, -d)),
        rune::Side::Top => ((along, 1.0), (0.0, d)),
        rune::Side::Left => ((1.0, along), (d, 0.0)),
        rune::Side::Right => ((0.0, along), (-d, 0.0)),
    };
    rune::Transition::scale_anchor(0.95, rune::Vec2::new(ax, ay))
        .combined(rune::Transition::offset(dx, dy))
        .springs(rune::SpringPair::new(
            rune::Spring::new(0.25, 0.0),
            rune::Spring::new(0.2, 0.0),
        ))
}

fn placement_split(p: Placement) -> (rune::Side, rune::Align) {
    use rune::{Align, Side};
    match p {
        Placement::Top => (Side::Top, Align::Center),
        Placement::TopStart => (Side::Top, Align::Start),
        Placement::TopEnd => (Side::Top, Align::End),
        Placement::Bottom => (Side::Bottom, Align::Center),
        Placement::BottomStart => (Side::Bottom, Align::Start),
        Placement::BottomEnd => (Side::Bottom, Align::End),
        Placement::Left => (Side::Left, Align::Center),
        Placement::Right => (Side::Right, Align::Center),
        Placement::RightStart => (Side::Right, Align::Start),
    }
}

/// `rune_kit::popover::anchored` for a glass surface, entering as the web's popovers do
/// ([`popover_enter`]) rather than appearing at once.
pub(crate) fn anchored(tk: &Tk, surface: El, anchor: &str, placement: Placement, gap: f32) -> El {
    let (side, align) = placement_split(placement);
    rune_kit::popover::anchored(&tk.kit(), surface, anchor, placement, gap)
        .transition(popover_enter(tk, side, align))
}

/// The `.kb-glass-dialog` surface without an entrance: what the Add connection dialog wears and
/// every title-bar popover now shares (owner feedback: one visual language for the nav bar's
/// overlays). `--background` at 80 % over a 16 pt (dark) / 12 pt (light) backdrop blur, 1 rem
/// corners clipping the content; dark: a `#fff` 8 % hairline, a 4 % top highlight and
/// `0 24px 72px -20px` black at 90 %; light: a foreground 10 % hairline and `0 24px 64px -24px`
/// at 65 %. (`saturate(1.14)` isn't available and is left out.)
pub(crate) fn glass_surface(tk: &Tk, content: El) -> El {
    let el = content
        .radius(tk.r(1.0))
        .fill(tk.background.alpha(0.8))
        .backdrop_blur(if tk.dark { 16.0 } else { 12.0 })
        .clip_rounded(tk.r(1.0));
    if tk.dark {
        el.border(1.0, Color::WHITE.alpha(0.08))
            .border_layout()
            .box_shadow(Shadow {
                y: 1.0,
                color: Color::WHITE.alpha(0.04),
                inset: true,
                ..Shadow::default()
            })
            .box_shadow(Shadow {
                y: 24.0,
                blur: 72.0,
                spread: -20.0,
                color: Color::BLACK.alpha(0.9),
                ..Shadow::default()
            })
    } else {
        el.border(1.0, tk.fg.alpha(0.10))
            .border_layout()
            .box_shadow(Shadow {
                y: 24.0,
                blur: 64.0,
                spread: -24.0,
                color: Color::BLACK.alpha(0.65),
                ..Shadow::default()
            })
    }
}

/// `.kb-glass-dialog` (palette, settings dialogs): [`glass_surface`], entering as
/// `DialogContent` does ([`dialog_enter`]); mount it as its own layer root ([`centered`]).
pub(crate) fn glass_dialog(tk: &Tk, content: El) -> El {
    glass_surface(tk, content).transition(dialog_enter())
}

/// A title-bar popover's header in the dialog's rhythm (DialogHeader `gap-2`, the title
/// `leading-none font-semibold`), one step smaller for an anchored surface: `px-4 pt-4 pb-3`,
/// the title `text-base`, an optional `text-sm` muted description, an optional trailing control
/// group, and the dialog's `border-b` under it.
pub(crate) fn popover_header(tk: &Tk, title: &str, desc: Option<&str>, trailing: Option<El>) -> El {
    let mut texts = col().grow(1.0).min_w(0.0).gap(tk.r(0.5)).child(
        text(title.to_owned())
            .font(tk.tbase(600.0).lh(tk.r(1.0)))
            .color(tk.fg)
            .role("heading")
            .level(2)
            .wrap(),
    );
    if let Some(d) = desc {
        texts = texts.child(
            text(d.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    let mut r = row()
        .w_full()
        .px(tk.r(1.0))
        .pt(tk.r(1.0))
        .pb(tk.r(0.75))
        .gap(tk.r(0.75))
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(texts);
    if let Some(t) = trailing {
        r = r.child(t);
    }
    r
}

/// The inset around a popover menu's rows inside the 1 rem glass corners (`p-1.5`).
pub(crate) fn menu_pad(tk: &Tk) -> f32 {
    tk.r(0.375)
}

/// Mounts an anchored popup in `layer`: a scrim that closes it on an outside press, and the
/// content as a modal (Escape closes, focus stays inside, the opener gets focus back).
pub(crate) fn mount_popup(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    anchor: &str,
    placement: Placement,
    content: El,
) {
    mount_popup_as(tk, layer, key, anchor, placement, content, true);
}

/// A Radix overlay. `modal` (DropdownMenu's default, Dialog): a scrim takes outside presses
/// and the page goes inert to pointer and assistive technology until it closes. Non-modal
/// (Popover, `DropdownMenu modal={false}`): no scrim, the page stays live and in the
/// accessibility tree; Escape, the trigger and a press on the window's background close it
/// (`sh-root`'s background press).
pub(crate) fn mount_popup_as(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    anchor: &str,
    placement: Placement,
    content: El,
    modal: bool,
) {
    mount_popup_at(tk, layer, key, anchor, placement, content, modal, 4.0);
}

/// [`mount_popup_as`] with Radix's `sideOffset` (4 by default; the book switcher uses 6).
#[allow(clippy::too_many_arguments)]
pub(crate) fn mount_popup_at(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    anchor: &str,
    placement: Placement,
    content: El,
    modal: bool,
    offset: f32,
) {
    // Every shell popup wears the dialog glass ([`glass_surface`]), anchored and entering as a
    // popover ([`anchored`]).
    let mut surface =
        glass_surface(tk, content.key(format!("{key}-surface"))).focus_return(anchor.to_owned());
    if modal {
        layer.push(rune_kit::scrim(format!("{key}-scrim"), |s: &mut App| {
            s.shell.popup = None
        }));
        surface = surface.modal(true, |s: &mut App| s.shell.popup = None);
    }
    layer.push(anchored(tk, surface, anchor, placement, offset));
}

/// [`mount_popup_at`] for a dropdown menu built with [`crate::ui::popup::Popup::finish_custom`]
/// (its list keyed `{key}-surface`, the glass drawn on the list itself): the list takes the
/// menu's keys (arrows without wrapping, typeahead, Escape closing only it) and stays the one
/// `menu` in the tree; focus returns to the popup's trigger, which may differ from `anchor`
/// (the refresh caret inside its button group).
#[allow(clippy::too_many_arguments)]
pub(crate) fn mount_menu_at(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    anchor: &str,
    placement: Placement,
    content: El,
    keys: crate::ui::popup::KeyHandler,
    modal: bool,
    offset: f32,
) {
    let back = keys.anchor().to_owned();
    let mut surface = keys
        .attach_to_list(glass_surface(tk, content))
        .focus_return(back);
    if modal {
        layer.push(rune_kit::scrim(format!("{key}-scrim"), |s: &mut App| {
            s.shell.popup = None
        }));
        surface = surface.modal(true, |s: &mut App| s.shell.popup = None);
    }
    layer.push(anchored(tk, surface, anchor, placement, offset));
}

/// A dropdown menu row (`rounded-sm px-2 py-1.5 text-sm`, accent on hover).
/// A DropdownMenuItem whose icon carries `mr-2` (the user menu's items).
pub(crate) fn menu_row_spaced(
    tk: &Tk,
    key: impl Into<String>,
    glyph: &'static Icon,
    label: &str,
    shortcut: Option<&str>,
) -> El {
    let mut r = row()
        .key(key.into())
        .w_full()
        // `px-2 py-1.5`: one text-sm line makes it 2 rem tall, a wrapped label grows it.
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.25))
        .hover(tk.accent)
        .focus_highlight(tk.accent)
        .role("menuitem")
        .label(label.to_owned())
        .focusable(true)
        .child(ic(glyph, tk.r(1.0), tk.muted_fg).mr(tk.r(0.5)))
        .child(
            text(label.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        );
    if let Some(s) = shortcut {
        r = r.child(menu_shortcut(tk, s));
    }
    r
}

pub(crate) fn menu_row(
    tk: &Tk,
    key: impl Into<String>,
    glyph: Option<&'static Icon>,
    label: &str,
    shortcut: Option<&str>,
) -> El {
    let mut r = row()
        .key(key.into())
        .w_full()
        // `px-2 py-1.5`: one text-sm line makes it 2 rem tall, a wrapped label grows it.
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.25))
        .hover(tk.accent)
        .focus_highlight(tk.accent)
        .role("menuitem")
        .label(label.to_owned())
        .focusable(true);
    if let Some(g) = glyph {
        r = r.child(ic(g, tk.r(1.0), tk.muted_fg));
    }
    // Labels wrap inside the item as the web's do (a long label in a `w-60` menu takes two lines).
    r = r.child(
        text(label.to_owned())
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .wrap()
            .grow(1.0)
            .min_w(0.0),
    );
    if let Some(s) = shortcut {
        r = r.child(menu_shortcut(tk, s));
    }
    r
}

/// A radio/checkbox menu row: the indicator column then the label.
pub(crate) fn check_row(
    tk: &Tk,
    key: impl Into<String>,
    checked: bool,
    radio: bool,
    glyph: Option<&'static Icon>,
    label: &str,
    detail: Option<&str>,
) -> El {
    // DropdownMenuRadioItem / CheckboxItem: `gap-2 rounded-sm py-1.5 pr-2 pl-8 text-sm`, the
    // indicator a `size-3.5` box at `left-2` (a `size-2` filled circle, or a `size-4` check).
    let indicator = row().size(tk.r(0.875), tk.r(0.875)).center().a11y_hidden();
    let indicator = match (checked, radio) {
        (true, true) => indicator.child(crate::ui::dot(tk.r(0.5), tk.fg)),
        (true, false) => indicator.child(ic(&I::CHECK, tk.r(1.0), tk.fg)),
        (false, _) => indicator,
    };
    // Label and detail wrap inside the item (a long German description in the `w-72` menu).
    let mut label_col = col().grow(1.0).min_w(0.0).gap(tk.r(0.125)).child(
        text(label.to_owned())
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .wrap(),
    );
    if let Some(d) = detail {
        label_col = label_col.child(
            text(d.to_owned())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    let mut content = row()
        .grow(1.0)
        .min_w(0.0)
        .ml(tk.r(0.625))
        .gap(tk.r(0.5))
        .center_y();
    if let Some(g) = glyph {
        content = content.child(ic(g, tk.r(1.0), tk.muted_fg));
    }
    let accessible = match detail {
        Some(d) => format!("{label} {d}"),
        None => label.to_owned(),
    };
    row()
        .key(key.into())
        .w_full()
        .py(tk.r(0.375))
        .pl(tk.r(0.5))
        .pr(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.25))
        .hover(tk.accent)
        .focus_highlight(tk.accent)
        .role(if radio {
            "menuitemradio"
        } else {
            "menuitemcheckbox"
        })
        .focus_selected(radio && checked)
        .checked(checked)
        .label(accessible)
        .focusable(true)
        .child(indicator)
        .child(content.child(label_col))
}

/// DropdownMenuLabel: `px-2 py-1.5 text-sm font-medium`.
pub(crate) fn menu_label(tk: &Tk, label: &str) -> El {
    row()
        .w_full()
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .center_y()
        .child(
            text(label.to_owned())
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .grow(1.0),
        )
}

/// DropdownMenuShortcut: `ml-auto text-xs tracking-widest text-muted-foreground`.
pub(crate) fn menu_shortcut(tk: &Tk, s: &str) -> El {
    text(s.to_owned())
        .font(tk.txs(400.0).tracking(tk.r(0.75) * 0.1))
        .color(tk.muted_fg)
}

/// DropdownMenuSeparator: `-mx-1 my-1 h-px bg-border`.
pub(crate) fn menu_sep(tk: &Tk) -> El {
    el().h(1.0)
        .w_full()
        .mx(-tk.r(0.25))
        .my(tk.r(0.25))
        .fill(tk.border)
}

/// The whole window: title bar over sidebar + page panel, plus overlays.
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, page: El, layer: &mut Vec<El>) -> El {
    // Web keyboard semantics (this UI ports a web one): Return presses a focused button as
    // Space does, every pressable control is a Tab stop in document order, and a modal that
    // opens without an autofocus control focuses itself. Rune's default is AppKit's, where
    // Return only presses rows, links and the default button.
    cx.web_focus();
    wordmark::observe(app, cx);
    let is_narrow = narrow(app, cx);
    let folded = nav_folded(app, cx);
    let main_content = if app.onboarding.import_root_error.is_some() {
        overlays::import_root_screen(app, tk)
    } else if app.locked {
        overlays::lock_screen(app, tk)
    } else {
        page
    };
    let mut panel = col()
        .key("page-panel")
        .on_layout(|s: &mut App, r: [f32; 4]| s.shell.panel_rect = r)
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .fill(tk.background)
        .radius(tk.r(1.0))
        // The 1 px border takes layout room, as the web's does; nothing else insets the page.
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .border_layout()
        .clip_rounded(tk.r(1.0))
        .child({
            // The docked transaction detail panel sits beside the scrolling page (both screens).
            let main = col()
                .key("app-main")
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .children((!crate::txdetail::view::page_hidden(app)).then_some(main_content));
            // A two-finger horizontal swipe over the page goes back or forward (`crate::nav`).
            let main = crate::nav::swipe_area(main);
            // The assistant's approval notice and closed bar sit under the page's scroll.
            let below = if app.locked || crate::txdetail::view::page_hidden(app) {
                vec![]
            } else {
                crate::assistant::closed::below_page(app, cx, tk, layer)
            };
            let main = if below.is_empty() {
                main
            } else {
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .h_full()
                    .child(main.h(0.0).min_h(0.0))
                    .children(below)
            };
            let dock = if app.locked {
                None
            } else {
                crate::assistant::dock(app, cx, tk, layer)
                    .or_else(|| crate::txdetail::dock(app, cx, tk, layer))
            };
            row()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .child(main)
                .children(dock)
        });
    // The card shows for a running or failed book refresh until "Continue in background"
    // (running) or "Close" (failed); the progress line shows under it and for any other job.
    let card = !app.locked
        && app.shell.sync_card_open
        && app
            .shell
            .maint
            .as_ref()
            .is_some_and(|m| m.job == maint::Job::BookRefresh);
    let job_running = app.shell.maint.as_ref().is_some_and(|m| !m.failed);
    if (app.busy.reads > 0 || job_running) && !app.locked {
        panel = panel.child(overlays::route_progress(app, tk, !card));
    }
    if card {
        // In the window's overlay layer over the panel's rect, not inside the page: page
        // content that composites on its own (the chart's dots and value tags) would otherwise
        // draw over the card. The first frame, before the panel has a rect, it sits in the page.
        let r = app.shell.panel_rect;
        if r[2] > 0.0 && r[3] > 0.0 {
            layer.push(overlays::sync_card(app, tk, Some(r)));
        } else {
            panel = panel.child(overlays::sync_card(app, tk, None));
        }
    }
    if !app.locked {
        panel = panel.children(crate::nav::affordance(app, tk));
    }
    let mut body = row().w_full().grow(1.0).min_h(0.0);
    if !app.locked {
        body = body.child(sidebar::view(app, tk, folded, layer));
    } else {
        body = body.pl(tk.r(0.375));
    }
    body = body.child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .pr(tk.r(0.375))
            .pb(tk.r(0.375))
            .child(panel),
    );
    if app.prefs.right_panel_open
        && app.prefs.ai_features
        && !app.locked
        && app.route.path() != "/assistant"
    {
        body = body.child(crate::assistant::right_panel(app, cx, tk));
    }
    // A press outside a non-modal popover (on anything that isn't a control) closes it; only
    // while one is open, so the window isn't a pressable group otherwise.
    let non_modal_open = matches!(
        app.shell.popup,
        Some(Popup::Books | Popup::Notifications | Popup::Alpha | Popup::Privacy)
    );
    let mut root = col()
        .key("sh-root")
        .w_full()
        .h_full()
        .fill(tk.chrome)
        .on_layout(|s: &mut App, r: [f32; 4]| {
            s.shell.window_w = r[2];
            s.shell.window_h = r[3];
        })
        .child(skip_link(app, tk, layer))
        .child(titlebar::view(app, tk, folded, is_narrow, layer))
        .child(body);
    // The mouse's Back and Forward buttons go back and forward anywhere (`crate::nav`). That
    // makes the root pressable: named "Close" while a press on it closes a popover, otherwise
    // left out of the accessibility tree as a scrim is (a press on bare chrome does nothing).
    root = crate::nav::history_buttons(root, non_modal_open);
    root = if non_modal_open {
        root.label(crate::i18n::t("common:actions.close"))
    } else {
        root.role("none")
    };
    // Follow the system appearance: a marker keyed by what the system says mounts afresh
    // whenever it changes, and its first layout records it.
    let sys = cx.system_dark();
    if app.prefs.theme == crate::app::ThemeMode::System && sys != app.system_dark {
        root = root.child(
            el().key(format!("sh-sysdark-{sys}"))
                .absolute(0.0, 0.0)
                .size(1.0, 1.0)
                .on_layout(move |s: &mut App, _| s.system_dark = sys),
        );
    }
    if !app.locked {
        search::mount(app, tk, layer);
    }
    if app.shell.settings_open {
        crate::settings::modal(app, cx, tk, layer);
    }
    overlays::toasts(app, tk, layer);
    root
}

const SKIP_KEY: &str = "sh-skip";

/// AppShell's "Skip to main content" link: the window's first Tab stop, moving focus into the
/// page. Off-screen until focused (`sr-only`); focused, the pill shows at the top left
/// (`focus:not-sr-only focus:absolute focus:top-4 focus:left-4`).
fn skip_link(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let label = t("chrome:shell.skipToContent");
    if app.shell.skip_focused {
        layer.push(
            row()
                .key("sh-skip-pill")
                // Below the title bar, clear of the window controls, in the popovers' style.
                .absolute(tk.r(0.75), 40.0 + tk.r(0.5))
                .px(tk.r(0.625))
                .py(tk.r(0.375))
                .radius(tk.r(0.5))
                .fill(tk.popover)
                .border(1.0, tk.ring)
                .border_layout()
                .a11y_hidden()
                .inert()
                .child(text(label.clone()).font(tk.txs(500.0)).color(tk.fg)),
        );
    }
    row()
        .key(SKIP_KEY)
        .absolute(0.0, 0.0)
        .size(1.0, 1.0)
        .clip_rounded(0.0)
        .role("link")
        .label(label)
        .focusable(!app.locked)
        .no_focus_ring()
        .on_press_tx(|_: &mut App, tx: &mut rune::Tx| tx.focus("app-main", true))
}

/// Every frame: whether the skip link has focus (Rune reports focus to handlers and polls).
pub fn skip_link_poll(app: &mut App, tx: &mut rune::Tx) -> bool {
    // Shown for keyboard focus only (`:focus-visible`): focus that lands on it by itself (a
    // window opening) or from the pointer shows nothing.
    let focused = tx.focused() == Some(SKIP_KEY) && tx.focus_visible();
    let changed = focused != app.shell.skip_focused;
    app.shell.skip_focused = focused;
    changed
}

//! What the app remembers between launches: the web's persisted `kb.ui` store (the fields the
//! Rune app has), the last page, and the window frame. One versioned, owner-only JSON file in
//! Tauri's app config directory (see `file`), never in the book, never logged.
//!
//! - Load once (`loaded`), tolerantly, through the schema migrations (`model::migrate`).
//! - `restore` applies it in `App::new`, before launch flags and scenes.
//! - `poll` compares what would be written with what was, and writes a change 0.4 s after it
//!   settles (debounced); closing the window, quitting and dropping the app write what is
//!   pending at once.
//! - Tooling runs keep nothing unless `KASSIBER_DESKTOP_PREFS_FILE` names a file.
//!
//! Left in their own canonical files (written by the daemon/CLI, read there): offline mode,
//! agent access and update checks (`KASSIBER_*_FILE`). Left out of the file because the web
//! keeps them out of `kb.ui` or the Rune app has no counterpart: see the field table in
//! `model.rs`'s docs and the pilot report.

pub mod file;
pub mod model;
pub mod window;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use rune::Tx;
use serde_json::{Map, Value, json};

use crate::app::App;
use model::{Sort, UiPrefs};
use window::Frame;

/// How long a change settles before it is written.
const DEBOUNCE: f64 = 0.4;

/// The file as this launch found it.
struct Loaded {
    path: Option<PathBuf>,
    prefs: UiPrefs,
    window: Option<Frame>,
    rewrite: bool,
    read_only: bool,
}

fn loaded() -> &'static Loaded {
    static LOADED: OnceLock<Loaded> = OnceLock::new();
    LOADED.get_or_init(|| {
        let launch = crate::daemon::Launch::from_args(rune::extra_args());
        let path = file::location(launch.data_root.as_deref());
        let m = model::migrate(path.as_deref().and_then(file::read));
        // No usable file (first launch, or one that is corrupt): write a good one at once.
        let fresh = m.doc.get("schemaVersion").is_none();
        Loaded {
            prefs: UiPrefs::read(&m.doc),
            window: Frame::read(&m.doc["window"]),
            path,
            rewrite: m.rewrite || fresh,
            read_only: m.read_only,
        }
    })
}

/// The saved frame for the window factory (see `window::run`).
pub fn launch_window() -> Option<Frame> {
    loaded().window
}

/// `app.persist`: the saver's state.
#[derive(Default)]
pub struct Store {
    path: Option<PathBuf>,
    read_only: bool,
    /// The book open now (`model::book_key` of the daemon's data root); it follows every root
    /// switch and book-set change, so a period is filed under the book it was chosen in.
    book: String,
    /// The demo was open at the last poll (leaving it brings the real book's period back).
    in_demo: bool,
    /// Per-book periods of every book, this one's updated as it changes.
    book_periods: std::collections::BTreeMap<String, String>,
    /// The chart's and the table's period as last seen (the web keeps one shared value; Rune
    /// has one per screen, so the one changed last is the book's).
    seen_periods: Option<(String, String)>,
    window: Option<Frame>,
    /// What the file holds (or would, for the first poll's state): writes happen on a change.
    written: Option<Value>,
    pending: Option<(Value, f64)>,
    /// A `--scene` run: the scene's state is synthetic, so the first poll takes it as saved
    /// instead of writing it (a later change, from a script, is saved as usual). Without this a
    /// scene's theme, language or layout leaked into every scene run after it.
    adopt_first: bool,
}

/// `App::new`: applies the saved preferences (before launch flags and scenes override them).
pub fn restore(app: &mut App) {
    let l = loaded();
    let book = model::book_key(app.launch.data_root.as_deref());
    let p = &l.prefs;
    apply(app, p);
    if let Some(period) = p.book_periods.get(&book) {
        apply_period(app, period);
    }
    if let Some(route) = p.last_route.as_deref().filter(|r| restorable(app, r)) {
        app.navigate(route);
        app.back.clear();
    }
    let mut store = Store::default();
    store.path = l.path.clone();
    store.read_only = l.read_only;
    store.book = book;
    store.book_periods = p.book_periods.clone();
    store.window = l.window;
    store.adopt_first = std::env::args().any(|a| a == "--scene");
    // What the file holds, normalized: a later state that differs is written (a migrated or
    // missing file at once).
    if !l.rewrite {
        let mut doc = Map::new();
        doc.insert("schemaVersion".into(), json!(model::SCHEMA_VERSION));
        p.write(&mut doc);
        if let Some(f) = l.window {
            doc.insert("window".into(), f.write());
        }
        store.written = Some(Value::Object(doc));
    }
    app.persist = store;
}

/// Whether a saved page can open at launch: a page some area renders, not gated behind
/// developer tools or the assistant switch, and not a redirect or the settings modal.
pub fn restorable(app: &App, route: &str) -> bool {
    let dev = app.prefs.developer_tools;
    if !dev
        && (crate::shell::DEV_LOCKED.contains(&route) || crate::shell::DEV_HIDDEN.contains(&route))
    {
        return false;
    }
    if route == "/assistant" && !app.prefs.ai_features {
        return false;
    }
    matches!(route, "/overview" | "/transactions")
        || crate::wallets::owns(route)
        || crate::review::owns(route)
        || crate::reports::owns(route)
        || crate::assistant::owns(route)
        || crate::custody::owns(route)
        || crate::provenance::owns(route)
}

/// Writes the saved preferences into the app's state.
fn apply(app: &mut App, p: &UiPrefs) {
    // The identity first: the saved page is only reachable with one.
    app.onboarding.identity = p.identity.clone();
    app.onboarding.first_sync_done = p.first_sync_done.clone();
    let a = &mut app.prefs;
    a.lang = p.lang;
    a.currency = p.currency;
    a.theme = p.theme;
    a.privacy = p.privacy;
    a.hide_on_unlock = p.hide_on_unlock;
    a.scale = p.scale;
    a.nav_collapsed = p.nav_collapsed;
    a.developer_tools = p.developer_tools;
    a.ai_features = p.ai_features;
    a.alpha_visible = p.alpha_visible;
    a.right_panel_open = p.right_panel_open;
    let s = &mut app.settings.prefs;
    s.clear_clipboard = p.clear_clipboard;
    s.desktop_notifications = p.desktop_notifications;
    s.hide_on_screen_share = p.hide_on_screen_share;
    s.exclude_unencrypted_backups = p.exclude_unencrypted_backups;
    s.explorer_fallbacks_allowed = p.explorer_fallbacks_allowed;
    s.auto_lock = p.lock.auto_lock;
    s.idle_minutes = p.lock.idle_minutes;
    s.require_on_launch = p.lock.require_on_launch;
    s.lock_on_close = p.lock.lock_on_close;
    s.touch_id_unlock = p.lock.touch_id_unlock;
    s.closed_bar = p.closed_bar;
    s.dock_position = p.dock_position;
    s.permission_mode = p.permission_mode;
    s.tool_profiles = p
        .tool_profiles
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    app.detail.width = p.side_panel_width;
    app.assistant.tab_open = p.assistant_panel_open;
    app.assistant.selection = p.model_selection.clone();
    app.assistant.picker.favorites = p.model_favorites.clone();
    // A new chat starts in the default mode (`assistantDefaultPermissionMode`).
    if let Some(m) = crate::assistant::PermissionMode::ALL
        .into_iter()
        .find(|m| m.id() == p.permission_mode)
    {
        app.assistant.permission = m;
    }
    app.assistant.chat_modes = p
        .chat_modes
        .iter()
        .filter_map(|(id, m)| {
            Some((
                id.clone(),
                crate::assistant::PermissionMode::ALL
                    .into_iter()
                    .find(|x| x.id() == *m)?,
            ))
        })
        .collect();
    for (table, sort) in &p.table_sorts {
        apply_sort(app, table, sort.as_ref());
    }
    app.ov.holdings = p.holdings_view.clone();
    app.ov.layout = p.overview_layout.clone();
}

fn apply_sort(app: &mut App, table: &str, sort: Option<&Sort>) {
    use crate::transactions::model::{Dir, SortKey as TxKey};
    use crate::wallets::list::SortKey as WKey;
    match table {
        "transactions" => {
            app.txs.filters.sort = sort.map(|s| {
                let key = if s.key == "amount" {
                    TxKey::Amount
                } else {
                    TxKey::Date
                };
                (key, if s.desc { Dir::Desc } else { Dir::Asc })
            });
        }
        "wallets" => {
            app.wallets.list.sort = sort.and_then(|s| {
                let key = match s.key.as_str() {
                    "label" => WKey::Label,
                    "kind" => WKey::Kind,
                    "transactions" => WKey::Transactions,
                    "last" => WKey::Last,
                    "taxFree" => WKey::TaxFree,
                    "balance" => WKey::Balance,
                    _ => return None,
                };
                Some((key, !s.desc))
            });
        }
        // The journal entries worklist (`ReviewDataTable`, table id "review"): date only; an
        // unsorted record falls back to its default (newest first).
        "review" => app.reports.ledger.journals.table.desc = sort.is_none_or(|s| s.desc),
        _ => {}
    }
}

fn apply_period(app: &mut App, key: &str) {
    use crate::chart::Period as C;
    use crate::transactions::model::{Period as T, Range};
    let Some(t) = T::parse(key) else { return };
    app.txs.period = t;
    app.chart.set_period(match t {
        T::Auto => C::Auto,
        T::Year(y) => C::Year(y),
        T::Range(Range::D30) => C::D30,
        T::Range(Range::Ytd) => C::Ytd,
        T::Range(Range::Y1) => C::Y1,
        T::Range(Range::Y4) => C::Y4,
        T::Range(Range::Y10) => C::Y10,
        T::Range(Range::All) => C::All,
    });
}

fn chart_period_key(app: &App) -> String {
    use crate::chart::Period as C;
    match app.chart.period {
        C::Auto => "auto".into(),
        C::D30 => "30days".into(),
        C::Ytd => "ytd".into(),
        C::Y1 => "1year".into(),
        C::Y4 => "4years".into(),
        C::Y10 => "10years".into(),
        C::All => "all".into(),
        C::Year(y) => y.to_string(),
    }
}

/// The app's state as the preferences to write.
fn capture(app: &App, store: &mut Store) -> UiPrefs {
    use crate::transactions::model::{Dir, SortKey as TxKey};
    use crate::wallets::list::SortKey as WKey;
    let a = &app.prefs;
    let s = &app.settings.prefs;
    // The book's period: whichever of the two screens changed it last. The demo is never
    // remembered, its period neither.
    let now = (chart_period_key(app), app.txs.period.param());
    match &store.seen_periods {
        None => {}
        Some(_) if app.demo.active.is_some() => {}
        Some((chart, table)) => {
            let changed = if &now.1 != table {
                Some(now.1.clone())
            } else if &now.0 != chart {
                Some(now.0.clone())
            } else {
                None
            };
            if let Some(p) = changed.and_then(|p| model::normalize_period(&p)) {
                store.book_periods.insert(store.book.clone(), p);
                while store.book_periods.len() > model::MAX_BOOKS {
                    let other = store
                        .book_periods
                        .keys()
                        .find(|k| **k != store.book)
                        .cloned();
                    match other {
                        Some(k) => store.book_periods.remove(&k),
                        None => break,
                    };
                }
            }
        }
    }
    store.seen_periods = Some(now);
    let mut table_sorts = std::collections::BTreeMap::new();
    table_sorts.insert(
        "transactions",
        app.txs.filters.sort.map(|(k, d)| Sort {
            key: if k == TxKey::Amount { "amount" } else { "date" }.into(),
            desc: d == Dir::Desc,
        }),
    );
    table_sorts.insert(
        "wallets",
        app.wallets.list.sort.map(|(k, asc)| Sort {
            key: match k {
                WKey::Label => "label",
                WKey::Kind => "kind",
                WKey::Transactions => "transactions",
                WKey::Last => "last",
                WKey::TaxFree => "taxFree",
                WKey::Balance => "balance",
            }
            .into(),
            desc: !asc,
        }),
    );
    table_sorts.insert(
        "review",
        Some(Sort {
            key: "date".into(),
            desc: app.reports.ledger.journals.table.desc,
        }),
    );
    UiPrefs {
        lang: a.lang,
        currency: a.currency,
        theme: a.theme,
        privacy: crate::osleaks::screen::saved_privacy(a.privacy),
        hide_on_unlock: a.hide_on_unlock,
        clear_clipboard: s.clear_clipboard,
        desktop_notifications: s.desktop_notifications,
        hide_on_screen_share: s.hide_on_screen_share,
        exclude_unencrypted_backups: s.exclude_unencrypted_backups,
        scale: a.scale,
        explorer_fallbacks_allowed: s.explorer_fallbacks_allowed,
        lock: model::LockPolicy {
            auto_lock: s.auto_lock,
            idle_minutes: s.idle_minutes.clamp(1, 1440),
            require_on_launch: s.require_on_launch,
            lock_on_close: s.lock_on_close,
            touch_id_unlock: s.touch_id_unlock,
        },
        ai_features: a.ai_features,
        developer_tools: a.developer_tools,
        alpha_visible: a.alpha_visible,
        nav_collapsed: a.nav_collapsed,
        model_selection: app.assistant.selection.clone(),
        model_favorites: app.assistant.picker.favorites.clone(),
        dock_position: s.dock_position,
        closed_bar: s.closed_bar,
        assistant_panel_open: app.assistant.tab_open,
        right_panel_open: a.right_panel_open,
        side_panel_width: app
            .detail
            .width
            .clamp(model::SIDE_PANEL_MIN_WIDTH, model::SIDE_PANEL_MAX_WIDTH),
        permission_mode: s.permission_mode,
        tool_profiles: s
            .tool_profiles
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        chat_modes: app
            .assistant
            .chat_modes
            .iter()
            .map(|(k, m)| (k.clone(), m.id()))
            .take(model::MAX_CHAT_MODES)
            .collect(),
        book_periods: store.book_periods.clone(),
        table_sorts,
        holdings_view: app.ov.holdings.clone(),
        overview_layout: app.ov.layout.clone(),
        // The demo book is never remembered (`crate::demo`): the real book's route and identity
        // stay while it is open.
        last_route: model::route_root(&crate::demo::persisted_route(app))
            .filter(|r| restorable(app, r)),
        identity: crate::demo::persisted_identity(app),
        first_sync_done: {
            let mut m = app.onboarding.first_sync_done.clone();
            while m.len() > model::MAX_BOOKS {
                let Some(k) = m.keys().next().cloned() else {
                    break;
                };
                m.remove(&k);
            }
            m
        },
    }
}

/// The whole document as it would be written now.
fn document(app: &App, store: &mut Store, clock: Option<f64>) -> Value {
    let prefs = capture(app, store);
    store.window = window::sample(store.window, clock).or(store.window);
    let mut doc = Map::new();
    doc.insert("schemaVersion".into(), json!(model::SCHEMA_VERSION));
    prefs.write(&mut doc);
    if let Some(f) = store.window {
        doc.insert("window".into(), f.write());
    }
    Value::Object(doc)
}

fn write_now(store: &mut Store, doc: Value) {
    store.pending = None;
    if let Some(path) = store.path.as_deref() {
        // A failed write (a read-only disk, say) is not retried every frame: the state stays
        // in memory and the next change, close or quit tries again.
        let _ = file::write(path, &doc);
    }
    store.written = Some(doc);
}

/// Every frame: notice changes and write them once they settle.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut store = std::mem::take(&mut app.persist);
    follow_book(app, &mut store);
    if store.path.is_none() || store.read_only {
        app.persist = store;
        return false;
    }
    let doc = document(app, &mut store, Some(app.clock));
    if std::mem::take(&mut store.adopt_first) {
        store.written = Some(doc);
        store.pending = None;
    } else if store.written.is_none() {
        write_now(&mut store, doc);
    } else if store.written.as_ref() != Some(&doc) {
        let due = match &store.pending {
            Some((p, due)) if *p == doc => *due,
            _ => app.clock + DEBOUNCE,
        };
        if app.clock >= due {
            write_now(&mut store, doc);
        } else {
            store.pending = Some((doc, due));
            tx.wake_after(Duration::from_secs_f64((due - app.clock).max(0.02)));
        }
    } else {
        store.pending = None;
    }
    app.persist = store;
    false
}

/// The open book changed (an imported book opened after launch, the demo, a book set picked
/// from the menu): its remembered period applies, and later changes are filed under it.
pub(crate) fn follow_book(app: &mut App, store: &mut Store) {
    if app.demo.active.is_some() {
        store.in_demo = true;
        return;
    }
    let key = model::book_key(app.daemon.launch.data_root.as_deref());
    let left_demo = std::mem::take(&mut store.in_demo);
    if key == store.book && !left_demo {
        return;
    }
    store.book = key;
    if let Some(period) = store.book_periods.get(&store.book).cloned() {
        apply_period(app, &period);
    }
    // What the screens show now is the new book's starting point, not a change to save.
    store.seen_periods = None;
}

/// Writes whatever changed, now (window close, quit).
pub fn flush(app: &mut App) {
    if app.persist.path.is_none() || app.persist.read_only {
        return;
    }
    let mut store = std::mem::take(&mut app.persist);
    let doc = document(app, &mut store, None);
    if store.written.as_ref() != Some(&doc) {
        write_now(&mut store, doc);
    }
    app.persist = store;
}

/// `App::on_close`.
pub fn on_close(app: &mut App, _: &mut Tx) {
    flush(app);
}

/// `App::on_quit`.
pub fn on_quit(app: &mut App, _: &mut Tx) {
    flush(app);
    crate::osleaks::on_quit();
}

impl Drop for Store {
    /// A run that ends without a close or quit (a headless script) still keeps its last
    /// settled change.
    fn drop(&mut self) {
        if self.read_only {
            return;
        }
        if let Some((doc, _)) = self.pending.take()
            && let Some(path) = self.path.as_deref()
        {
            let _ = file::write(path, &doc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transactions::model::{Period, Range};
    use std::path::Path;

    #[test]
    fn the_period_is_filed_under_the_book_that_is_open() {
        let mut app = crate::app::tests::quiet_app();
        let launch_key = model::book_key(None);
        let other = Path::new("/books/imported");
        let other_key = model::book_key(Some(other));
        let mut store = Store::default();
        store.book = launch_key.clone();
        store.book_periods.insert(other_key.clone(), "2024".into());
        // An imported book opens after launch (or a book set is picked): its own period applies.
        app.daemon.launch.data_root = Some(other.to_path_buf());
        follow_book(&mut app, &mut store);
        assert_eq!(app.txs.period, Period::Year(2024));
        let _ = capture(&app, &mut store);
        // A change is filed under the open book, never under the launch root.
        app.txs.period = Period::Range(Range::Ytd);
        let _ = capture(&app, &mut store);
        assert_eq!(
            store.book_periods.get(&other_key).map(String::as_str),
            Some("ytd")
        );
        assert!(!store.book_periods.contains_key(&launch_key));
    }

    #[test]
    fn the_demo_neither_files_a_period_nor_forgets_the_real_page() {
        let mut app = crate::app::tests::quiet_app();
        let key = model::book_key(None);
        let mut store = Store::default();
        store.book = key.clone();
        store.book_periods.insert(key.clone(), "1year".into());
        app.route = crate::app::Route("/reports".into());
        app.demo.active = Some(crate::demo::Session {
            data_root: "/d/data".into(),
            state_root: "/d".into(),
            env: Vec::new(),
        });
        follow_book(&mut app, &mut store);
        let _ = capture(&app, &mut store);
        app.txs.period = Period::Range(Range::All);
        let prefs = capture(&app, &mut store);
        assert_eq!(
            store.book_periods.get(&key).map(String::as_str),
            Some("1year"),
            "the demo's period is not the book's"
        );
        assert_eq!(store.book_periods.len(), 1);
        // No saved state (opened from Welcome): nothing is remembered rather than a demo page.
        assert_eq!(prefs.last_route, None);
        // Leaving brings the real book's period back.
        app.demo.active = None;
        follow_book(&mut app, &mut store);
        assert_eq!(app.txs.period, Period::Range(Range::Y1));
    }
}

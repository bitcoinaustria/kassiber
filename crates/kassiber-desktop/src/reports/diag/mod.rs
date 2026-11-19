//! Activity (`/activity`), Logs (`/logs`) and the network monitor (`/egress`).
//!
//! - `activity`: metadata provenance (`ui.activity.history`, `ui.activity.stale`), filters,
//!   the edit timeline with revert, and the stale-edits "Process" action.
//! - `logs` + `logstore` + `redact` + `regex`: the RAM-only log viewer, its ring and bridge,
//!   redaction tiers, filters, exports through the native Save dialog, the support bundle.
//! - `egress`: the outbound ledger (`ui.egress.snapshot`, polled every 4 s while open).
//!
//! - `surfaces` + `tail` + `changes`: the right panel's Logs and Changes tabs
//!   (assistant-panel/surfaces/**), the primary home of both; the pages reuse their rows.
//!
//! All three are developer-tools routes (`DEV_LOCKED` / `DEV_HIDDEN`); the shell's guard keeps
//! them unreachable while developer tools are off. Every read here is local to the daemon.

pub mod activity;
pub mod changes;
pub mod egress;
pub mod logs;
pub mod logstore;
mod ordjson;
pub mod redact;
pub mod regex;
pub mod replay;
pub mod surfaces;
pub mod tail;
mod time;
mod widgets;

use rune::{Cx, El, KeyEvent, Tx};

use crate::app::{App, Reply};
use crate::theme::Tk;

/// Which of this part's popups is open (one at a time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popup {
    ActDate,
    /// The Filters menu (its submenu is `act.sub`).
    ActFilters,
    /// An edit's "⋯" menu (which one is `act.row_menu`).
    ActRow,
    LogExport,
    LogFilter,
}

/// This part's UI state (`app.reports.diag`).
#[derive(Default)]
pub struct Ui {
    pub popup: Option<Popup>,
    pub act: activity::State,
    pub logs: logs::State,
    /// Logs → Export → Replay bundle.
    pub replay: replay::State,
    pub egress: egress::State,
    /// The panel's Logs tab (its ring is `logs.store`, shared with the page).
    pub tail: tail::State,
    /// The panel's Changes tab.
    pub chg: changes::State,
}

pub fn owns(path: &str) -> bool {
    matches!(path, "/activity" | "/logs" | "/egress")
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    match app.route.path() {
        "/activity" => activity::view(app, cx, tk, layer),
        "/logs" => logs::view(app, cx, tk, layer),
        _ => egress::view(app, cx, tk, layer),
    }
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    // The log ring collects the app's own transport records from launch, as the web's does.
    let mut changed = logs::collect(app);
    let path = app.route.path().to_owned();
    // The panel's surfaces stay mounted behind other tabs (filters, scroll and reads kept); the
    // Logs tab reads the daemon's ring like the page, and both drop their state once closed.
    use crate::assistant::{Surface, surface_open};
    if surface_open(app, Surface::Logs) {
        changed |= tail::poll(app, tx);
        if path != "/logs" {
            changed |= logs::bridge_tick(app, tx);
        }
    } else {
        app.reports.diag.tail = Default::default();
    }
    if surface_open(app, Surface::Changes) {
        changed |= changes::poll(app);
    } else if !app.reports.diag.chg.args_key.is_empty() {
        app.reports.diag.chg.close();
    }
    if path != "/egress" {
        // Leaving unmounts the monitor; coming back refetches at once.
        app.reports.diag.egress.unmount();
    }
    if !owns(&path) {
        if app.reports.diag.popup.is_some() {
            app.reports.diag.popup = None;
        }
        return changed;
    }
    changed |= match path.as_str() {
        "/activity" => activity::poll(app, tx),
        "/logs" => logs::poll(app, tx),
        _ => egress::poll(app, tx),
    };
    changed
}

/// Leaving one of this part's pages (`crate::reports::on_leave`).
pub fn leave(app: &mut App, from: &str) {
    app.reports.diag.popup = None;
    // The replay consent was given on this page, for this book.
    replay::close(app);
    match from {
        "/activity" => app.reports.diag.act.leave(),
        "/logs" => app.reports.diag.logs.leave(),
        "/egress" => app.reports.diag.egress.leave(),
        _ => {}
    }
}

/// Tags of this part start with `"reports.diag."`.
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag
        .strip_prefix("reports.diag.")
        .and_then(|t| t.split('.').next())
    {
        Some("act") => activity::on_reply(app, tag, r),
        Some("chg") => changes::on_reply(app, tag, r),
        Some("logs") => logs::on_reply(app, tag, r),
        Some("replay") => replay::on_reply(app, tag, r),
        Some("egress") => egress::on_reply(app, tag, r),
        _ => {}
    }
}

pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if matches!(ev.key, rune::Key::Escape) && app.reports.diag.popup.is_some() {
        app.reports.diag.popup = None;
        return true;
    }
    if matches!(ev.key, rune::Key::Escape) && app.reports.diag.chg.confirm.is_some() {
        app.reports.diag.chg.confirm = None;
        return true;
    }
    match app.route.path() {
        "/activity" => activity::on_key(app, ev, tx),
        "/logs" => logs::on_key(app, ev, tx),
        _ => false,
    }
}

/// Headless scenes: the panel surfaces' first reads.
pub fn prime_surfaces(app: &mut App) {
    use crate::assistant::{Surface, surface_open};
    if surface_open(app, Surface::Logs) {
        logs::prime(app);
    }
    if surface_open(app, Surface::Changes) {
        changes::poll(app);
        super::pump_until(app, |a| !a.reports.diag.chg.fetching);
    }
}

pub fn prime(app: &mut App) {
    match app.route.path() {
        "/activity" => activity::prime(app),
        "/logs" => logs::prime(app),
        "/egress" => egress::prime(app),
        _ => {}
    }
}

fn go(a: &mut App, path: &str) {
    a.prefs.developer_tools = true;
    // The web opens the sidebar group of the active route; the shell doesn't, so open it here.
    if path == "/egress" {
        a.shell.extras_open = true;
    }
    a.navigate(path);
    prime(a);
}

pub const SCENES: &[super::Scene] = &[
    ("reports-activity", |a| go(a, "/activity")),
    ("reports-activity-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        go(a, "/activity");
    }),
    ("reports-activity-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        go(a, "/activity");
    }),
    ("reports-activity-date", |a| {
        go(a, "/activity");
        a.reports.diag.popup = Some(Popup::ActDate);
    }),
    // SYNTHETIC: the demo book has no edit history, so this injects three events (one expanded,
    // one stale) to show the timeline. Not a parity scene.
    ("reports-activity-sample", |a| {
        go(a, "/activity");
        activity::inject_sample(a);
    }),
    // SYNTHETIC (as above), with the Filters menu open.
    ("reports-activity-filters", |a| {
        go(a, "/activity");
        activity::inject_sample(a);
        a.reports.diag.act.ai_only = true;
        a.reports.diag.popup = Some(Popup::ActFilters);
    }),
    // SYNTHETIC (as above), with the Filters menu's Source submenu open.
    ("reports-activity-filters-source", |a| {
        go(a, "/activity");
        activity::inject_sample(a);
        a.reports.diag.popup = Some(Popup::ActFilters);
        a.reports.diag.act.sub = Some("source");
    }),
    // SYNTHETIC (as above), with the first edit's "⋯" menu open.
    ("reports-activity-row-menu", |a| {
        go(a, "/activity");
        activity::inject_sample(a);
        a.reports.diag.act.row_menu = Some("ev-1".into());
        a.reports.diag.popup = Some(Popup::ActRow);
    }),
    ("reports-activity-sample-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        go(a, "/activity");
        activity::inject_sample(a);
    }),
    ("reports-activity-sample-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        go(a, "/activity");
        activity::inject_sample(a);
    }),
    ("reports-logs", |a| go(a, "/logs")),
    ("reports-logs-filtered", |a| {
        go(a, "/logs");
        logs::scene_filter(a);
    }),
    ("reports-logs-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        go(a, "/logs");
    }),
    ("reports-logs-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        go(a, "/logs");
    }),
    ("reports-logs-expanded", |a| {
        go(a, "/logs");
        logs::scene_expand_first(a);
    }),
    ("reports-logs-export-menu", |a| {
        go(a, "/logs");
        a.reports.diag.popup = Some(Popup::LogExport);
    }),
    ("reports-logs-filter-menu", |a| {
        go(a, "/logs");
        a.reports.diag.popup = Some(Popup::LogFilter);
    }),
    ("reports-logs-raw", |a| {
        go(a, "/logs");
        logs::set_redacted(a, false);
    }),
    ("reports-logs-support", |a| {
        go(a, "/logs");
        a.reports.diag.logs.support_open = true;
    }),
    ("reports-egress", |a| go(a, "/egress")),
    ("reports-egress-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        go(a, "/egress");
    }),
    ("reports-egress-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        go(a, "/egress");
    }),
    // SYNTHETIC: the disposable book records no egress, so this injects two ledger rows
    // (one unexpected, one expanded) to show the table. Not a parity scene.
    ("reports-egress-sample", |a| {
        go(a, "/egress");
        egress::inject_sample(a);
    }),
    ("reports-egress-sample-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        go(a, "/egress");
        egress::inject_sample(a);
    }),
    ("reports-egress-sample-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        go(a, "/egress");
        egress::inject_sample(a);
    }),
];

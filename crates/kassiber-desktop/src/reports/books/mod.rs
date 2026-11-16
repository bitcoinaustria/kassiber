//! Books (`/books`, routes/Books.tsx) and the bird's-eye view (`/books/<workspaceId>/birds-eye`,
//! routes/BirdsEye.tsx).
//!
//! - `model`: `ui.profiles.snapshot` parsing, the tax country registry, labels and the pure
//!   helpers of Books.tsx and books-profile-settings.ts (unit-tested).
//! - `list`: the Books page (book sets, book rows, the set actions menu).
//! - `dialogs`: New book, Rename book set and New book set, and their daemon calls.
//! - `settings` / `settings_model`: Book settings (every setting of any book, a dry-run preview
//!   before each change) and its pure logic.
//! - `birds_eye`: the book set rollup (metrics, refresh panel, chart, per-book rows, fiat rows,
//!   recent activity).
//! - `kit`: the shadcn pieces they draw with.
//!
//! The book-management calls (`ui.profiles.*`, `ui.workspace.*`) carry no `expected_scope`:
//! the web sends them outside any book boundary (they choose or create the book), and this
//! keeps the same contract. The exception is `ui.profiles.update` on the open book, which
//! passes it (the daemon refuses a scope naming another book than `profile_id`, so other books'
//! changes go without). Request tags are `reports.books.*`.
//!
//! On mount `/books` reads `ui.profiles.snapshot`, then `ui.profiles.settings` for each book (all
//! local): the rows show a tax year that doesn't start on the country's default day.

pub mod birds_eye;
pub mod dialogs;
pub mod kit;
pub mod list;
pub mod model;
pub mod settings;
pub mod settings_model;

use std::collections::{HashMap, HashSet};

use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::i18n::t;
use crate::theme::Tk;

/// The open dialog of the Books page.
#[derive(Clone, Debug, PartialEq)]
pub enum Dialog {
    /// New book in a set, optionally copying another book's settings.
    CreateProfile {
        ws: String,
        source: Option<String>,
        country: String,
        method: String,
    },
    /// Book settings for any book (`settings::Form` holds the fields and the preview step).
    Settings {
        ws: String,
        profile_id: String,
    },
    RenameWorkspace {
        ws: String,
    },
    CreateWorkspace,
}

/// This part's UI state (`app.reports.books`).
#[derive(Default)]
pub struct Ui {
    /// The book filter (shown above `BOOK_FILTER_THRESHOLD` books).
    pub query: TextEdit,
    /// The open select or menu: `ws:<id>` (set actions), `sel:<field>` (a dialog select).
    pub popup: Option<String>,
    pub dialog: Option<Dialog>,
    /// The name field of whichever dialog is open.
    pub name: TextEdit,
    pub error: Option<String>,
    /// A dialog's mutation is in flight.
    pub submitting: bool,
    /// After `ui.profiles.update`, the rename still to send: (profile id, label).
    pub then_rename: Option<(String, String)>,
    /// The book being opened (its row spins, the others lock).
    pub pending_open: Option<String>,
    pub switch_error: Option<String>,
    /// The Book settings form.
    pub form: settings::Form,
    /// Each book's settings (`ui.profiles.settings`), by profile id.
    pub cache: HashMap<String, settings_model::Settings>,
    /// Books whose settings reads were sent since the page mounted.
    pub requested: HashSet<String>,
    /// Books whose journals a change applied here left waiting for reprocessing, until opened.
    pub stale: HashSet<String>,
    /// An element to scroll into view on the next poll (headless scenes).
    pub reveal: Option<String>,
    /// The route last seen, to re-read on mount.
    pub mounted: String,
    pub be: birds_eye::State,
}

pub fn owns(path: &str) -> bool {
    path == "/books" || path.starts_with("/books/")
}

/// The workspace id of `/books/<id>/birds-eye`.
pub fn birds_eye_id(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/books/")?;
    let (id, tail) = rest.split_once('/')?;
    (tail == "birds-eye" && !id.is_empty()).then_some(id)
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let path = app.route.path();
    match birds_eye_id(path) {
        Some(id) => birds_eye::view(app, cx, tk, id, layer),
        None if path == "/books" => list::view(app, tk, layer),
        // `/books/<anything else>`: the router's not-found page.
        None => crate::ui::not_found_page(app, tk),
    }
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    if let Some(k) = app.reports.books.reveal.take() {
        tx.scroll_into_view(k);
    }
    step(app)
}

/// The work of `poll` that needs no `Tx`: the reads a route mount makes (local only).
fn step(app: &mut App) -> bool {
    let path = app.route.path().to_owned();
    if !owns(&path) {
        if !app.reports.books.mounted.is_empty() {
            app.reports.books.mounted.clear();
            app.reports.books.popup = None;
            app.reports.books.requested.clear();
        }
        return birds_eye::follow_refresh(app);
    }
    let mut changed = false;
    if app.reports.books.mounted != path {
        app.reports.books.mounted = path.clone();
        // `useDaemon("ui.profiles.snapshot")` on mount.
        if path == "/books" {
            app.send("ui.profiles.snapshot", Value::Null, "app.profiles");
            app.reports.books.requested.clear();
        }
        changed = true;
    }
    if path == "/books" {
        changed |= read_settings(app);
    }
    if let Some(id) = birds_eye_id(&path).map(str::to_owned) {
        changed |= birds_eye::step(app, &id);
    }
    changed | birds_eye::follow_refresh(app)
}

/// Each listed book's settings, once per mount (local reads): the snapshot doesn't carry the
/// tax calendar.
fn read_settings(app: &mut App) -> bool {
    let Some(p) = app.read("ui.profiles.snapshot") else {
        return false;
    };
    let ids: Vec<String> = model::parse(p)
        .workspaces
        .iter()
        .flat_map(|w| w.profiles.iter().map(|p| p.id.clone()))
        .filter(|id| !id.is_empty())
        .collect();
    let mut sent = false;
    for id in ids {
        if app.reports.books.requested.insert(id.clone()) {
            app.send(
                "ui.profiles.settings",
                json!({"profile_id": id}),
                "reports.books.settings_read",
            );
            sent = true;
        }
    }
    sent
}

/// Leaving `/books`: the filter, menus, dialogs and their errors are `Books`' `useState`s. A
/// mutation or book switch in flight keeps its dialog and row state until it answers.
pub fn leave(app: &mut App) {
    let ui = &mut app.reports.books;
    ui.query = TextEdit::new("");
    ui.popup = None;
    ui.switch_error = None;
    if !ui.submitting {
        ui.dialog = None;
        ui.name = TextEdit::new("");
        ui.error = None;
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if settings::on_reply(app, tag, r) {
        return;
    }
    match tag {
        "reports.books.switch" => list::on_switched(app, r),
        "reports.books.settings_read" => {
            if let (None, Some(s)) = (&r.error, settings_model::Settings::parse(&r.data)) {
                app.reports.books.cache.insert(s.profile_id.clone(), s);
            }
        }
        t if t.starts_with("reports.books.be.") => birds_eye::on_reply(app, t, r),
        t => dialogs::on_reply(app, t, r),
    }
}

/// Every reply (not only this part's tags), for reads the shell makes on this part's behalf.
pub fn on_any_reply(app: &mut App, tag: &str, r: &Reply) {
    let _ = (app, tag, r);
}

/// Stream progress of `ui.workspace.freshness.run` (tag `reports.books.be.refresh`), for a host
/// that forwards progress records to areas. Without that hook the panel follows the shell's
/// progress line (see `birds_eye::follow_refresh`).
pub fn on_progress(app: &mut App, tag: &str, env: &Value) {
    if tag == "reports.books.be.refresh" {
        birds_eye::on_progress(app, &env["data"]);
    }
}

/// Escape closes the innermost: a select or menu, then the dialog (not while it saves).
pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let _ = tx;
    if !matches!(ev.key, Key::Escape) {
        return false;
    }
    if app.reports.books.popup.is_none() && settings::on_key(app, ev) {
        return true;
    }
    let b = &mut app.reports.books;
    if b.popup.is_some() {
        b.popup = None;
        return true;
    }
    if b.dialog.is_some() {
        if !b.submitting {
            dialogs::close(app);
        }
        return true;
    }
    false
}

/// Headless: wait for the route's reads.
pub fn prime(app: &mut App) {
    step(app);
    super::pump_until(app, |a| a.pending.is_empty());
    if let Some(id) = birds_eye_id(app.route.path()).map(str::to_owned) {
        birds_eye::step(app, &id);
        super::pump_until(app, |a| a.pending.is_empty());
    }
}

/// The active book set.
fn active_ws(app: &App) -> String {
    app.read("ui.profiles.snapshot")
        .and_then(|p| p["activeWorkspaceId"].as_str())
        .unwrap_or_default()
        .to_owned()
}

fn snapshot(a: &App) -> model::Profiles {
    model::parse(a.read("ui.profiles.snapshot").unwrap_or(&json!({})))
}

fn scene_books(a: &mut App) {
    a.navigate("/books");
    prime(a);
}

fn scene_birds_eye(a: &mut App) {
    let ws = active_ws(a);
    a.navigate(&format!("/books/{ws}/birds-eye"));
    prime(a);
}

fn scene_create(a: &mut App) {
    scene_books(a);
    let p = snapshot(a);
    if let Some(w) = p.workspace(&p.active_workspace) {
        dialogs::request_create_profile(a, w, None);
    }
}

fn scene_settings(a: &mut App) {
    scene_books(a);
    let p = snapshot(a);
    if let Some(pr) = p.profile(&p.active_workspace, &p.active_profile) {
        settings::open(a, &p.active_workspace, &pr.id, &pr.name);
    }
    super::pump_until(a, |a| a.pending.is_empty());
}

/// Book settings scrolled to `section` (the dialog is taller than the window).
fn scene_settings_at(a: &mut App, section: &'static str) {
    scene_settings(a);
    a.reports.books.reveal = Some(section.into());
}

/// Book settings with the tax year set to 1 July, reviewed: the dry run's preview.
fn scene_settings_preview(a: &mut App) {
    scene_settings(a);
    settings::choose_year(a, "july");
    settings::review(a);
    super::pump_until(a, |a| a.pending.is_empty());
}

/// A book of `country` (first book of the open set, switched on a disposable book only by the
/// scene's own data root): Book settings as a locked-calendar country shows it. The switch
/// itself is a dry run; nothing is written.
fn scene_settings_switched(a: &mut App, country: &str) {
    scene_settings(a);
    settings::set_country(a, country);
    super::pump_until(a, |a| a.pending.is_empty());
}

pub const SCENES: &[super::Scene] = &[
    ("reports-books", scene_books),
    ("reports-books-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_books(a);
    }),
    ("reports-books-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_books(a);
    }),
    ("reports-books-menu", |a| {
        scene_books(a);
        a.reports.books.popup = Some(format!("ws:{}", active_ws(a)));
    }),
    ("reports-books-create", scene_create),
    ("reports-books-create-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_create(a);
    }),
    ("reports-books-create-region", |a| {
        scene_create(a);
        a.reports.books.popup = Some("sel:region".into());
    }),
    ("reports-books-create-copy", |a| {
        scene_books(a);
        let p = snapshot(a);
        if let Some(w) = p.workspace(&p.active_workspace) {
            let src = w.profiles.first().cloned();
            dialogs::request_create_profile(a, w, src.as_ref());
        }
    }),
    ("reports-books-settings", scene_settings),
    ("reports-books-settings-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_settings(a);
    }),
    ("reports-books-settings-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_settings(a);
    }),
    ("reports-books-settings-region", |a| {
        scene_settings(a);
        a.reports.books.popup = Some("sel:region".into());
    }),
    ("reports-books-settings-year", |a| {
        scene_settings_at(a, "books-set-sec-year");
        a.reports.books.popup = Some("sel:year".into());
    }),
    ("reports-books-settings-custom", |a| {
        scene_settings(a);
        settings::choose_year(a, "__custom__");
        a.reports.books.form.custom = TextEdit::new("02-29");
        settings::review(a);
        a.reports.books.reveal = Some("books-set-sec-year".into());
    }),
    ("reports-books-settings-basis", |a| {
        scene_settings_at(a, "books-set-sec-basis")
    }),
    ("reports-books-settings-review", |a| {
        scene_settings_at(a, "books-set-sec-review")
    }),
    ("reports-books-settings-preview", scene_settings_preview),
    ("reports-books-settings-preview-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_settings_preview(a);
    }),
    // A region switch shows the region's own choices (a dry run) and its legal, locked tax year.
    ("reports-books-settings-switch", |a| {
        scene_settings_switched(a, "gb");
        a.reports.books.reveal = Some("books-set-sec-year".into());
    }),
    // The US offers three pool scopes; the dry run's choices list them.
    ("reports-books-settings-us", |a| {
        scene_settings_switched(a, "us");
        a.reports.books.reveal = Some("books-set-sec-basis".into());
    }),
    // SYNTHETIC: a refusal the daemon would type for a stale request, shown in plain words.
    ("reports-books-settings-error", |a| {
        scene_settings_preview(a);
        a.reports.books.error = Some(t("onboarding:books.settings.errors.methodScopeUnsupported"));
    }),
    // Snap it at a narrow size (`--size 720x900`): the table keeps Book, status and actions.
    ("reports-books-narrow", scene_books),
    ("reports-books-row-menu", |a| {
        scene_books(a);
        let p = snapshot(a);
        if let Some(pr) = p
            .workspace(&p.active_workspace)
            .and_then(|w| w.profiles.first())
        {
            a.reports.books.popup = Some(format!("book:{}", pr.id));
        }
    }),
    ("reports-books-rename-set", |a| {
        scene_books(a);
        let p = snapshot(a);
        if let Some(w) = p.workspace(&p.active_workspace) {
            dialogs::request_rename_workspace(a, w);
        }
    }),
    ("reports-books-new-set", |a| {
        scene_books(a);
        dialogs::request_create_workspace(a);
    }),
    ("reports-books-new-set-error", |a| {
        scene_books(a);
        dialogs::request_create_workspace(a);
        // An empty name is refused before any daemon call.
        dialogs::submit(a);
    }),
    ("reports-birds-eye", scene_birds_eye),
    ("reports-birds-eye-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_birds_eye(a);
    }),
    ("reports-birds-eye-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_birds_eye(a);
    }),
    ("reports-birds-eye-hidden", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
        scene_birds_eye(a);
    }),
    // SYNTHETIC: the refresh panel as a finished run reports it (no refresh is sent: it's egress).
    ("reports-birds-eye-refresh", |a| {
        scene_birds_eye(a);
        birds_eye::scene_refresh_panel(a);
    }),
];

/// The text a failed mutation shows: the daemon's message, else the generic one.
pub(crate) fn error_text(r: &Reply, fallback_key: &str) -> String {
    r.error
        .clone()
        .filter(|e| !e.trim().is_empty())
        .unwrap_or_else(|| t(fallback_key))
}

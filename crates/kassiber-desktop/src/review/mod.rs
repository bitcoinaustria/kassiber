//! Review (`/review`, the quarantine cases) and Reconcile (`/reconcile`).
//!
//! Registered in `main.rs` (view, poll, keys, scenes) and `app.rs` (state, replies) through the
//! functions below; everything else for this area lives under `src/review/`.
//!
//! - `model`: the web's pure helpers (quarantine pages, causes, typed prices, candidates).
//! - `state`: what the Review screen reads and decides (reads, refetches, decisions, dialogs).
//! - `inbox` / `cards` / `dialogs`: the Review screen's view (Review.tsx, ReviewTransfers.tsx,
//!   the quarantine cause panel, the waiting list, the fix and proposal dialogs).
//! - `reconcile` / `tools`: Reconcile's two modes (Reconcile.tsx, ToolCrossCheck.tsx).

mod cards;
mod dialogs;
mod inbox;
mod kit;
mod links;
mod links_scenes;
pub mod model;
mod reconcile;
mod scenes;
mod state;
mod tools;

use rune::{Cx, El, Key, KeyEvent, Tx};

use crate::app::{App, Reply};
use crate::theme::Tk;

/// An open popover or menu of this area (one at a time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Popup {
    /// An `InfoNote` by its button key.
    Info(String),
    /// A cause card's unit select (by cause key).
    Unit(String),
    /// Reconcile's "Verify with" backend menu.
    VerifyBackend,
    /// The cross-check's year picker.
    Year,
    /// A suggested link's "More" menu (by candidate key).
    MoveMenu(String),
}

/// This area's UI state (`app.review`).
#[derive(Default)]
pub struct Ui {
    pub rv: state::Review,
    pub rc: reconcile::State,
    pub tools: tools::State,
    pub popup: Option<Popup>,
    /// The route path the reads were last started for (a new visit re-reads, as a mount does).
    seen: String,
    /// The book the reads were made in; a switch starts over.
    scope: String,
}

/// Whether this area renders `path` (the route without its query).
pub fn owns(path: &str) -> bool {
    matches!(path, "/review" | "/reconcile")
}

/// The page for a route this area owns. Overlays (menus, dialogs, sheets) go into `layer`.
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    match app.route.path() {
        "/reconcile" => reconcile::view(app, cx, tk, layer),
        _ => inbox::view(app, cx, tk, layer),
    }
}

/// Every frame: start the reads the current route needs (local reads only), advance timers.
/// True when anything changed.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let path = app.route.path().to_owned();
    let mut changed = false;
    // A book switch starts every list over (the web's queries are keyed by the session).
    let scope = app.expected_scope().to_string();
    if scope != app.review.scope {
        let first = app.review.scope.is_empty();
        app.review.scope = scope;
        if !first {
            app.review.rv = state::Review::default();
            app.review.tools = tools::State::default();
            app.review.seen.clear();
            changed = true;
        }
    }
    if !owns(&path) {
        app.review.seen.clear();
        app.review.popup = None;
    } else if app.review.seen != path {
        app.review.seen = path.clone();
        if path == "/review" {
            state::mount(app);
        } else {
            reconcile::mount(app);
        }
        changed = true;
    }
    changed |= state::poll(app, tx);
    changed |= tools::poll(app, tx);
    changed
}

/// Leaving `/review` or `/reconcile` (`crate::shell::on_leave`). The inbox remounts on its own
/// (`state::mount` on the next visit). Reconcile's entries, filter, results, expanded rows and
/// CSV are `Reconcile`'s `useState`s: they start over unless a check or verify is in flight
/// (its answer fills them as before). The cross-check keeps what the web keeps per session
/// (`keptCrossCheck`); its file error is the component's own.
pub fn on_leave(app: &mut App, from: &str) {
    app.review.popup = None;
    if from == "/reconcile" {
        if !app.review.rc.checking && !app.review.rc.verifying {
            app.review.rc = reconcile::State::default();
        }
        app.review.tools.file_error = None;
    }
}

/// Every daemon reply; act only on this area's own request tags (`"review.…"`).
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if !tag.starts_with("review.") {
        return;
    }
    if tag.starts_with("review.rc.") {
        reconcile::on_reply(app, tag, r);
    } else if tag.starts_with("review.tools.") {
        tools::on_reply(app, tag, r);
    } else {
        state::on_reply(app, tag, r);
    }
}

/// A stream's progress record for one of this area's requests (the on-chain verify).
pub fn on_progress(app: &mut App, tag: &str, env: &serde_json::Value) {
    if tag == "review.rc.verify" {
        reconcile::on_progress(app, env);
    }
}

/// Key presses before the shell sees them; true when handled.
pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if !owns(app.route.path()) {
        return false;
    }
    if ev.key == Key::Escape {
        // The innermost overlay first: a popover, then a dialog.
        if app.review.popup.take().is_some() {
            return true;
        }
        if dialogs::escape(app) {
            return true;
        }
    }
    if app.route.path() == "/reconcile" {
        return reconcile::on_key(app, ev, tx);
    }
    false
}

/// Headless runs: load what the current route needs before the first frame (block on the
/// daemon, as `transactions::prime` does). Called by the `route` scene after navigating.
pub fn prime(app: &mut App) {
    let path = app.route.path().to_owned();
    app.review.seen = path.clone();
    app.review.scope = app.expected_scope().to_string();
    if path == "/review" {
        state::mount(app);
    } else {
        reconcile::mount(app);
    }
    settle(app);
}

/// Pumps the daemon until this area has no request in flight (up to 20 s).
pub fn settle(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        app.pump();
        let busy = app
            .pending
            .values()
            .any(|(tag, _, _)| tag.starts_with("review."));
        if !busy || std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

// ---- Seams with the detail panel (links.rs) ----------------------------------------------------

/// The Review page's reading of a row it opened in the detail panel (`detailContextFor`).
pub fn detail_context(
    app: &App,
    id: &str,
    opened: Option<&crate::txdetail::QContext>,
) -> Option<crate::txdetail::QContext> {
    links::detail_context(app, id, opened)
}

/// The panel saved a row the Review page opened; `next` steps on after the lists re-read.
pub fn after_sheet_save(app: &mut App, id: &str, next: Option<crate::txdetail::ReviewHost>) {
    links::after_sheet_save(app, id, next)
}

/// The panel's Unpair while the Review page hosts it (reviewed plan/apply).
pub fn unpair_from_sheet(app: &mut App, relation_id: &str) {
    links::unpair_from_sheet(app, relation_id)
}

/// Whether the panel offers Unpair for its shown pair (`onUnpair` set: the pair is a case).
pub fn sheet_can_unpair(app: &App, record: &serde_json::Value) -> bool {
    links::sheet_pair_case(app, record).is_some()
}

/// `isUnpairing` of the Review page's sheet.
pub fn sheet_unpairing(app: &App) -> bool {
    links::unpairing(app)
}

/// The Review page's "Open pairing review" from its sheet.
pub fn open_pairing_review(app: &mut App) {
    links::open_pairing_review(app)
}

/// Toggles a popover of this area.
pub fn toggle(app: &mut App, p: Popup) {
    app.review.popup = if app.review.popup.as_ref() == Some(&p) {
        None
    } else {
        Some(p)
    };
}

/// Named states for `--scene` (headless snapshots and the parity harness); prefix names with
/// `review-`.
pub const SCENES: &[crate::app::Scene] = scenes::SCENES;

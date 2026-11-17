//! Transfers (`/transfers`; `/swaps` and `/custody-gaps` redirect here): every move between
//! the owner's wallets, what needs a decision, what they unlinked, and the developer-gated
//! custody tools (`routes/transfers/**`, `routes/transfers-custody/**`, `routes/CustodyGaps.tsx`).
//!
//! Registered in `main.rs` (view, poll, keys, scenes) and `app.rs` (state, replies) through the
//! functions below. The parts:
//!
//! - `route`: the URL state (`routeSearch.ts`) and the router's redirects.
//! - `model` / `link_model`: the pure model (`linkedMovesModel.ts`, `satsFormat.ts`, `linkModel.ts`).
//! - `kit`: shared building blocks (buttons, card header, stat tile, rails, menus, dialogs).
//! - `page`: `TransfersPage` / `TransfersOverview` — header, tabs, stats, the lists, notes.
//! - `suggestions`: `ReviewTransfers` as the page's framed suggestions card.
//! - `link`: "Link manually" (`LinkManuallyDialog`, `LinkCandidates`), panel or dialog.
//! - `unlink`: a move's details and unlinking (`UnlinkDialog`, `MoveEffects`, `MoveLeg`).
//! - `rules`: linking rules and saved views (`RulesSheet`).
//! - `gaps`: missing wallets and scan coverage (`CustodyGaps.tsx`).
//! - `components`: custody components (`CustodyComponentResolver`, the guided form).
//!
//! Every mutation is the owner's explicit press, bound to the book (`expected_scope`) and,
//! where the web does, to a server plan shown before confirmation. Nothing here touches the
//! network: all reads are local daemon reads.

pub mod components;
pub mod gaps;
pub mod kit;
pub mod link;
pub mod link_model;
pub mod model;
pub mod page;
pub mod route;
pub mod rules;
mod scenes;
pub mod suggestions;
pub mod unlink;

use rune::{Cx, El, Key, KeyEvent, Tx};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::theme::Tk;

/// This area's UI state (`app.custody`).
#[derive(Default)]
pub struct Ui {
    pub page: page::State,
    pub sugg: suggestions::State,
    pub link: link::State,
    pub unlink: unlink::State,
    pub rules: rules::State,
    pub gaps: gaps::State,
    pub comps: components::State,
    /// `useJournalRefreshAfterDecisions`: a refresh asked for, waiting to start (app clock).
    pub refresh_at: Option<f64>,
    /// Our own `ui.journals.process` is running.
    pub processing: bool,
    /// The route (path and query) the reads were last started for.
    seen: String,
    /// The book the reads were made in; a switch starts over.
    scope: String,
    /// `App::book_generation` the open page's reads belong to.
    generation: u64,
}

/// Whether this area renders `path` (the route without its query).
pub fn owns(path: &str) -> bool {
    path == "/transfers"
}

/// The router's redirects for `/transfers`, `/swaps` and `/custody-gaps` (called by
/// `shell::redirect`): the canonical target, or None to navigate as asked.
pub fn redirect(app: &App, path: &str, query: &str) -> Option<String> {
    route::redirect(app, path, query)
}

/// What the screen shows for the current URL (`transfersCustodyView`).
pub fn current_view(app: &App) -> route::View {
    let q = &app.route.0[app.route.path().len()..];
    route::view(&route::params(q), app.prefs.developer_tools)
}

/// Leaving `/transfers` (`crate::shell::on_leave`): the page's own state and its dialogs (link, unlink, rules, the component editor,
/// bulk dismiss) close, as the web unmounts them; one whose request is in flight stays until
/// it answers.
pub fn on_leave(app: &mut App) {
    page::leave(app);
    if !app.custody.link.preview.applying {
        link::close(app);
    }
    if !app.custody.unlink.unpairing {
        unlink::close(app);
    }
    if !app.custody.rules.creating {
        rules::close(app);
        rules::close_create(app);
    }
    let comps = &app.custody.comps;
    if comps.pending.is_none() && !comps.form.as_ref().is_some_and(|f| f.pending()) {
        components::close_form(app);
    }
    app.custody.comps.action_error = None;
    suggestions::close_bulk(app);
    suggestions::close_popup(app);
    let s = &mut app.custody.sugg;
    s.undo = None;
    s.error = None;
    s.selected.clear();
}

/// Navigates to `/transfers` with this search (`select`).
pub fn select(app: &mut App, s: route::Search) {
    app.navigate(&route::href(&s));
}

/// The page for a route this area owns. Overlays (menus, dialogs, sheets) go into `layer`.
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    page::view(app, cx, tk, layer)
}

/// Every frame: start the reads the current route needs (local reads only), advance timers.
/// True when anything changed.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    let scope = app.expected_scope().to_string();
    if scope != app.custody.scope {
        let first = app.custody.scope.is_empty();
        app.custody.scope = scope;
        if !first {
            let processing = app.custody.processing;
            let sugg = std::mem::take(&mut app.custody.sugg);
            app.custody = Ui {
                scope: app.custody.scope.clone(),
                processing,
                // Its requests still out are dropped when they answer, not taken as the new book's.
                sugg: suggestions::restart(sugg),
                ..Ui::default()
            };
            changed = true;
        }
    }
    let path = app.route.path().to_owned();
    if !owns(&path) {
        if !app.custody.seen.is_empty() {
            app.custody.seen.clear();
            page::leave(app);
        }
    } else if app.custody.seen != app.route.0 {
        let first_visit = app.custody.seen.is_empty();
        app.custody.seen = app.route.0.clone();
        app.custody.generation = app.book_generation;
        page::mount(app, first_visit);
        changed = true;
    }
    // Any book refresh re-reads the open page: an assistant's unlink, a sync or a rate rebuild
    // changes pairs, lineage and badges as much as a decision made here.
    if app.custody.generation != app.book_generation {
        app.custody.generation = app.book_generation;
        if !app.custody.seen.is_empty() {
            invalidate(app);
            changed = true;
        }
    }
    // A refresh asked for after a decision starts once the core is free (SETTLE_MS = 800).
    if let Some(at) = app.custody.refresh_at {
        if app.clock >= at {
            if app.busy.processing || app.busy.syncing {
                app.custody.refresh_at = Some(app.clock + 0.8);
            } else {
                app.custody.refresh_at = None;
                start_refresh(app);
            }
            changed = true;
        } else {
            tx.wake_at(at);
        }
    }
    changed |= page::poll(app, tx);
    changed |= suggestions::poll(app, tx);
    changed |= link::poll(app, tx);
    changed |= unlink::poll(app, tx);
    changed |= rules::poll(app, tx);
    changed |= gaps::poll(app, tx);
    changed |= components::poll(app, tx);
    changed
}

/// Every daemon reply; act only on this area's own request tags (`"custody.…"`).
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if !tag.starts_with("custody.") {
        // The shell's own journal run also leaves fresh journals behind.
        if tag == "app.journals_process" || tag == "app.freshness_run" {
            invalidate(app);
        }
        return;
    }
    if tag == "custody.journals" {
        app.custody.processing = false;
        app.busy.processing = false;
        app.busy.progress = None;
        if r.error.is_some() {
            // A failed refresh leaves the decision stored and the freshness barrier visible.
            app.send("ui.review.badges", Value::Null, "app.badges");
        }
        app.refresh_reads();
        invalidate(app);
        return;
    }
    match tag.split('.').nth(1).unwrap_or("") {
        "page" => page::on_reply(app, tag, r),
        "sugg" => suggestions::on_reply(app, tag, r),
        "link" => link::on_reply(app, tag, r),
        "unlink" => unlink::on_reply(app, tag, r),
        "rules" => rules::on_reply(app, tag, r),
        "gaps" => gaps::on_reply(app, tag, r),
        "comps" => components::on_reply(app, tag, r),
        _ => {}
    }
}

/// Key presses before the shell sees them; true when handled.
pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if !owns(app.route.path()) {
        return false;
    }
    if ev.key == Key::Escape {
        // The innermost overlay first: menus, then sheets and dialogs, then panels.
        return page::escape_menu(app)
            || suggestions::escape(app)
            || rules::escape(app)
            || gaps::escape(app)
            || components::escape(app)
            || unlink::escape(app)
            || link::escape(app);
    }
    let _ = tx;
    false
}

/// Headless runs: load what the current route needs before the first frame (block on the
/// daemon, as `transactions::prime` does). Called by the `route` scene after navigating.
pub fn prime(app: &mut App) {
    app.custody.scope = app.expected_scope().to_string();
    app.custody.seen = app.route.0.clone();
    page::mount(app, true);
    settle(app);
}

/// Pumps the daemon until this area has no request in flight (up to 20 s).
pub fn settle(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        app.pump();
        // Reads that start more reads (pages of holds) start them from poll.
        page::continue_reads(app);
        let busy = app
            .pending
            .values()
            .any(|(tag, _, _)| tag.starts_with("custody.") && *tag != "custody.journals");
        if !busy || std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

// ---- Journals after a decision (`useJournalRefreshAfterDecisions`) -----------------------------

/// `requestRefresh`: recalculate the journals shortly after a decision (800 ms, waiting for a
/// run in flight), without auto-pairing. The badges are read again at once.
pub fn request_refresh(app: &mut App) {
    if app.read("ui.profiles.snapshot").is_none() {
        return;
    }
    app.send("ui.review.badges", Value::Null, "app.badges");
    app.custody.refresh_at = Some(app.clock + 0.8);
}

fn start_refresh(app: &mut App) {
    let args = json!({"auto_pair": false, "expected_scope": app.expected_scope()});
    if app
        .send("ui.journals.process", args, "custody.journals")
        .is_some()
    {
        app.custody.processing = true;
        app.busy.processing = true;
        app.busy.progress = Some("Processing journals".into());
    }
}

/// `isProcessingJournals || refreshPending`: the journals are recalculating, or about to.
pub fn updating(app: &App) -> bool {
    app.busy.processing || app.custody.refresh_at.is_some()
}

/// `isProcessingJournals`.
pub fn processing(app: &App) -> bool {
    app.busy.processing
}

/// The journals were recalculated (ours or the shell's): every read on the screen is stale.
pub fn invalidate(app: &mut App) {
    if !owns(app.route.path()) {
        app.custody.seen.clear();
        return;
    }
    page::reload(app);
    suggestions::reload(app);
    link::reload(app);
    unlink::reload(app);
    gaps::reload(app);
    components::reload(app);
}

/// What every linked transaction has left outside its links (`useOpenParts`), from the pair
/// list the page reads.
pub fn open_parts(app: &App) -> Option<&link_model::Parts> {
    app.custody.page.parts.as_ref()
}

/// Opens a transaction on Transactions (`TransactionsMenuItems`' links).
pub fn show_transaction(app: &mut App, id: &str, tab: Option<&str>) {
    let mut to = format!("/transactions?tx={id}");
    if let Some(t) = tab {
        to.push_str(&format!("&tab={t}"));
    }
    app.navigate(&to);
}

/// Named states for `--scene` (headless snapshots and the parity harness).
pub const SCENES: &[crate::app::Scene] = scenes::SCENES;

#[cfg(test)]
mod refresh_tests {
    use super::*;

    /// A book refresh started elsewhere (an assistant's unlink) re-reads the open page's pairs,
    /// as a decision made on the page does.
    #[test]
    fn a_book_refresh_rereads_the_open_page() {
        let mut app = crate::app::tests::quiet_app();
        app.route = crate::app::Route("/transfers".into());
        let mut tx = Tx::default();
        poll(&mut app, &mut tx);
        let reads = |a: &App| -> usize {
            a.pending
                .values()
                .filter(|(tag, _, _)| *tag == "custody.page.pairs")
                .count()
        };
        let mounted = reads(&app);
        assert!(mounted >= 1, "mounting reads the pairs");
        poll(&mut app, &mut tx);
        assert_eq!(reads(&app), mounted, "an idle poll reads nothing");
        app.book_generation += 1;
        poll(&mut app, &mut tx);
        assert_eq!(reads(&app), mounted + 1, "the refresh re-reads them");
    }
}

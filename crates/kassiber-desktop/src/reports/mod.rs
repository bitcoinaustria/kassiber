//! Reports (`/reports`), Journals (`/journals`), Exit tax (`/exit-tax`), Activity (`/activity`), Logs (`/logs`), the network monitor (`/egress`) and Books (`/books`, `/books/<id>/birds-eye`).
//!
//! Registered in `main.rs` (view, poll, keys, scenes) and `app.rs` (state, replies) through the
//! functions below. Four parts, each with the same interface:
//! - `report`: the Reports screen;
//! - `ledger`: Journals and Exit tax;
//! - `diag`: Activity, Logs and the network monitor;
//! - `books`: Books and the bird's-eye view.
//!
//! Request tags are `"reports.<part>.…"`.

pub mod books;
pub mod diag;
pub mod ledger;
pub mod report;

use rune::{Cx, El, KeyEvent, Tx};

use crate::app::{App, Reply};
use crate::theme::Tk;

pub use crate::app::Scene;

/// This area's UI state (`app.reports`).
#[derive(Default)]
pub struct Ui {
    pub report: report::Ui,
    pub ledger: ledger::Ui,
    pub diag: diag::Ui,
    pub books: books::Ui,
}

/// Whether this area renders `path` (the route without its query).
pub fn owns(path: &str) -> bool {
    report::owns(path) || ledger::owns(path) || diag::owns(path) || books::owns(path)
}

/// The page for a route this area owns. Overlays (menus, dialogs, sheets) go into `layer`.
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let path = app.route.path();
    if report::owns(path) {
        report::view(app, cx, tk, layer)
    } else if ledger::owns(path) {
        ledger::view(app, cx, tk, layer)
    } else if diag::owns(path) {
        diag::view(app, cx, tk, layer)
    } else {
        books::view(app, cx, tk, layer)
    }
}

/// Every frame: start the reads the current route needs (local reads only), advance timers.
/// True when anything changed.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = report::poll(app, tx);
    changed |= ledger::poll(app, tx);
    changed |= diag::poll(app, tx);
    changed |= books::poll(app, tx);
    changed
}

/// Leaving one of this area's pages (`crate::shell::on_leave`): the state its route component
/// keeps in `useState` starts over.
pub fn on_leave(app: &mut App, from: &str) {
    if ledger::owns(from) {
        ledger::leave(app, from);
    } else if report::owns(from) {
        app.reports.report.leave();
    } else if diag::owns(from) {
        diag::leave(app, from);
    } else if books::owns(from) && !from.starts_with("/books/") {
        books::leave(app);
    }
}

/// Every daemon reply; act only on this area's own request tags (`"reports.…"`).
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if let Some(rest) = tag.strip_prefix("reports.") {
        match rest.split('.').next() {
            Some("report") => report::on_reply(app, tag, r),
            Some("ledger") => ledger::on_reply(app, tag, r),
            Some("diag") => diag::on_reply(app, tag, r),
            Some("books") => books::on_reply(app, tag, r),
            _ => {}
        }
    }
    // Parts that watch shared reads (book switches, profile snapshots) see every reply.
    books::on_any_reply(app, tag, r);
    report::on_any_reply(app, tag, r);
}

/// Key presses before the shell sees them; true when handled.
pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let path = app.route.path().to_owned();
    if report::owns(&path) {
        report::on_key(app, ev, tx)
    } else if ledger::owns(&path) {
        ledger::on_key(app, ev, tx)
    } else if diag::owns(&path) {
        diag::on_key(app, ev, tx)
    } else if books::owns(&path) {
        books::on_key(app, ev, tx)
    } else {
        false
    }
}

/// Headless runs: load what the current route needs before the first frame (block on the
/// daemon, as `transactions::prime` does). Called by the `route` scene after navigating.
pub fn prime(app: &mut App) {
    let path = app.route.path().to_owned();
    if report::owns(&path) {
        report::prime(app)
    } else if ledger::owns(&path) {
        ledger::prime(app)
    } else if diag::owns(&path) {
        diag::prime(app)
    } else if books::owns(&path) {
        books::prime(app)
    }
}

/// Blocks (headless only) until `done` holds or 20 s pass, pumping daemon replies.
pub fn pump_until(app: &mut App, done: impl Fn(&App) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !done(app) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.pump();
    }
}

const PARTS: &[&[Scene]] = &[report::SCENES, ledger::SCENES, diag::SCENES, books::SCENES];

const N: usize = {
    let mut n = 0;
    let mut i = 0;
    while i < PARTS.len() {
        n += PARTS[i].len();
        i += 1;
    }
    n
};

const fn concat() -> [Scene; N] {
    fn nop(_: &mut App) {}
    let mut out: [Scene; N] = [("", nop as fn(&mut App)); N];
    let (mut i, mut k) = (0, 0);
    while i < PARTS.len() {
        let mut j = 0;
        while j < PARTS[i].len() {
            out[k] = PARTS[i][j];
            k += 1;
            j += 1;
        }
        i += 1;
    }
    out
}

static ALL: [Scene; N] = concat();

/// Named states for `--scene` (headless snapshots and the parity harness); every name starts
/// with `reports-`.
pub static SCENES: &[Scene] = &ALL;

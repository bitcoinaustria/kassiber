//! Journals (`/journals`, routes/Journals.tsx with its Ledger entries tab,
//! components/kb/JournalReportableEntries.tsx) and Exit tax (`/exit-tax`, routes/ExitTax.tsx with
//! components/reports/ExitTaxFmvCard.tsx).
//!
//! - `journals`: the State tab (metrics, entry mix, latest rows);
//! - `reportable`: the Ledger entries tab (the `ReviewDataTable` "journal-events" worklist);
//! - `exit_tax`: the Wegzugsbesteuerung estimate, its date picker, destination select and the
//!   PDF/XLSX handoff with the native Save As;
//! - `common`: the page frame, tables, tabs, tones and skeleton both screens draw with.
//!
//! Reads (local only, on mount, as the web's `useDaemon` hooks): `ui.journals.snapshot`,
//! `ui.journals.events.list {limit: 500}` (Ledger entries tab only) and
//! `ui.reports.exit_tax_preview`. Writes, from a press only: `ui.journals.process` (through the
//! shared `App::process_journals`) and `ui.reports.export_exit_tax_{pdf,xlsx}`, which write into
//! the book's exports folder. Tags start with `"reports.ledger."`.

pub(crate) mod common;
mod exit_tax;
mod journals;
mod reportable;
mod verify;

use rune::{Cx, El, KeyEvent, Tx};
use serde_json::Value;

use crate::app::{App, Reply};
use crate::theme::Tk;

pub use exit_tax::ExitUi;
pub use journals::JournalsUi;

/// This part's UI state (`app.reports.ledger`).
#[derive(Default)]
pub struct Ui {
    pub journals: JournalsUi,
    pub exit: ExitUi,
    /// The book the cached reads belong to; a switch drops them.
    book: Option<String>,
    /// A sync or journal run was in flight at the last poll; its end re-reads (the web
    /// invalidates every daemon query when a mutation settles).
    busy: bool,
    /// `App::book_generation` at the last poll: a mutation's refresh cycle (an exclusion, a
    /// custody or classification change) re-reads too, not only a sync or journal run.
    generation: u64,
}

pub fn owns(path: &str) -> bool {
    matches!(path, "/journals" | "/exit-tax")
}

/// Leaving `/journals` or `/exit-tax` (`crate::reports::on_leave`): the route's `useState`s
/// start over (Journals' tab, entry filter, table search and paging, the verify dialog; Exit
/// tax's form and export strip). Cached reads and the remembered date order stay.
pub fn leave(app: &mut App, from: &str) {
    if from == "/exit-tax" {
        app.reports.ledger.exit.leave();
        return;
    }
    let j = &mut app.reports.ledger.journals;
    j.tab = Default::default();
    j.filter = None;
    j.table = reportable::TableUi {
        desc: j.table.desc,
        ..Default::default()
    };
    verify::close(app);
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    match app.route.path() {
        "/exit-tax" => exit_tax::view(app, cx, tk, layer),
        _ => journals::view(app, cx, tk, layer),
    }
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = invalidate_on_change(app);
    match app.route.path() {
        "/journals" => changed |= journals::step(app),
        "/exit-tax" => changed |= exit_tax::poll(app, tx),
        _ => {}
    }
    changed
}

/// Drops every cached read when the book changes, a sync / journal run finishes or the book's
/// reads are refreshed.
fn invalidate_on_change(app: &mut App) -> bool {
    let book = app
        .read("ui.profiles.snapshot")
        .and_then(|p| p.get("activeProfileId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let busy = app.busy.processing || app.busy.syncing;
    let generation = app.book_generation;
    let ui = &mut app.reports.ledger;
    let mut drop = false;
    if book.is_some() && ui.book.is_some() && book != ui.book {
        drop = true;
        // Per-book view state resets with the book, as the route remounts in the web.
        ui.journals = JournalsUi::default();
        ui.exit.switch_book();
    }
    if book.is_some() {
        ui.book = book;
    }
    if ui.busy && !busy {
        drop = true;
    }
    ui.busy = busy;
    if ui.generation != generation {
        drop = true;
        ui.generation = generation;
    }
    if drop {
        ui.journals.invalidate();
        ui.exit.invalidate();
    }
    drop
}

/// Tags of this part start with `"reports.ledger."`.
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if tag == verify::TAG {
        verify::on_reply(app, r);
    } else if tag.starts_with("reports.ledger.journals") || tag.starts_with("reports.ledger.events")
    {
        journals::on_reply(app, tag, r);
    } else if tag.starts_with("reports.ledger.exit") {
        exit_tax::on_reply(app, tag, r);
    }
}

pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    match app.route.path() {
        "/exit-tax" => exit_tax::on_key(app, ev, tx),
        "/journals" if ev.key == rune::Key::Escape && app.reports.ledger.journals.verify.open => {
            verify::close(app);
            true
        }
        _ => false,
    }
}

pub fn prime(app: &mut App) {
    match app.route.path() {
        "/exit-tax" => exit_tax::prime(app),
        _ => journals::prime(app),
    }
}

fn verify_different() -> Value {
    serde_json::json!({
        "status": "different",
        "deterministic": false,
        "differing": 3,
        "truncated": true,
        "by_wallet": {"Regtest Spending Wallet": 2, "Regtest Cold Storage": 1},
        "by_year": {"2024": 1, "2025": 2},
        "differences": [
            {"table": "journal_entries", "change": "changed", "key": "c0ffee:0", "columns": [{"column": "fiat_value", "stored": "1.00", "recomputed": "1.01"}, {"column": "gain_loss", "stored": "0", "recomputed": "0.01"}]},
            {"table": "journal_wallet_holdings", "change": "missing", "key": "spending:BTC"}
        ]
    })
}

/// Navigates and loads, for the scenes.
fn open(a: &mut App, route: &str) {
    if route == "/exit-tax" {
        a.prefs.developer_tools = true;
    }
    crate::shell::go(a, route);
    // The web's sidebar opens Extras while one of its routes is current.
    a.shell.extras_open = true;
    prime(a);
}

pub const SCENES: &[super::Scene] = &[
    ("reports-journals", |a| open(a, "/journals")),
    ("reports-journals-filtered", |a| {
        open(a, "/journals");
        journals::scene_filter_first(a);
    }),
    ("reports-journals-reportable", |a| {
        open(a, "/journals");
        a.reports.ledger.journals.tab = journals::Tab::Reportable;
        journals::prime(a);
    }),
    ("reports-journals-reportable-filtered", |a| {
        open(a, "/journals");
        a.reports.ledger.journals.tab = journals::Tab::Reportable;
        journals::prime(a);
        a.reports.ledger.journals.table.metric = Some("disposals".into());
        a.reports.ledger.journals.table.search = rune::TextEdit::new("spending");
    }),
    // The page's "⋯" menu (Reports, Check reproducibility).
    ("reports-journals-menu", |a| {
        open(a, "/journals");
        a.reports.ledger.journals.menu = true;
    }),
    // A journal run in flight: the inline status beside the tabs, the spinner on the action.
    ("reports-journals-processing", |a| {
        open(a, "/journals");
        a.busy.processing = true;
        a.busy.progress = Some("Building journal entries".into());
    }),
    // Synthetic: held rows and stale journals (the demo book has neither): the signal colours,
    // the quarantine tile as the way to Review, Process journals as the primary action.
    ("reports-journals-held", |a| {
        open(a, "/journals");
        journals::scene_status(a, 3.0, false);
    }),
    ("reports-journals-stale", |a| {
        open(a, "/journals");
        journals::scene_status(a, 0.0, true);
    }),
    ("reports-journals-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open(a, "/journals");
    }),
    ("reports-journals-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open(a, "/journals");
    }),
    ("reports-journals-hidden", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
        open(a, "/journals");
    }),
    // The reproducibility check on the disposable book (a real, local, read-only daemon call).
    ("certdelta-journals-verify", |a| {
        open(a, "/journals");
        verify::open(a);
        crate::reports::pump_until(a, |a| !a.reports.ledger.journals.verify.pending());
    }),
    ("certdelta-journals-verify-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open(a, "/journals");
        verify::open(a);
        crate::reports::pump_until(a, |a| !a.reports.ledger.journals.verify.pending());
    }),
    // Synthetic reports (the daemon's shape, made-up rows) for the states the demo book lacks.
    ("certdelta-journals-verify-different", |a| {
        open(a, "/journals");
        verify::scene_report(a, verify_different());
    }),
    ("certdelta-journals-verify-different-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open(a, "/journals");
        verify::scene_report(a, verify_different());
    }),
    ("certdelta-journals-verify-stale-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open(a, "/journals");
        verify::scene_report(a, serde_json::json!({"status": "stale"}));
    }),
    ("certdelta-journals-verify-refused", |a| {
        open(a, "/journals");
        verify::scene_report(
            a,
            serde_json::json!({"status": "refused", "error": {"code": "book_locked", "message": "the book is locked"}}),
        );
    }),
    ("reports-exit-tax", |a| open(a, "/exit-tax")),
    ("reports-exit-tax-filled", |a| {
        open(a, "/exit-tax");
        exit_tax::scene_set_date(a, "2025-11-10");
    }),
    ("reports-exit-tax-picker", |a| {
        open(a, "/exit-tax");
        exit_tax::scene_open_picker(a);
    }),
    ("reports-exit-tax-third", |a| {
        open(a, "/exit-tax");
        exit_tax::scene_set_destination(a, "third_country");
    }),
    ("reports-exit-tax-select", |a| {
        open(a, "/exit-tax");
        a.reports.ledger.exit.popup = Some(exit_tax::Popup::Destination);
    }),
    ("reports-exit-tax-exported", |a| {
        open(a, "/exit-tax");
        exit_tax::scene_exported(a);
    }),
    ("reports-exit-tax-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open(a, "/exit-tax");
    }),
    ("reports-exit-tax-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open(a, "/exit-tax");
    }),
];

#[cfg(test)]
mod refresh_tests {
    /// An exclusion or custody change refreshes the book's reads without a sync or journal run;
    /// Journals must read again rather than keep calling the old figures current.
    #[test]
    fn a_book_refresh_marks_cached_ledger_reads_stale() {
        let mut app = crate::app::tests::quiet_app();
        assert!(!super::invalidate_on_change(&mut app));
        app.book_generation += 1;
        assert!(super::invalidate_on_change(&mut app));
        assert!(!super::invalidate_on_change(&mut app), "once per refresh");
    }
}

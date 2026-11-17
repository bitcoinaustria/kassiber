//! Route-local state follows the web's component lifecycle.
//!
//! The web keeps a page's transient state (an export's outcome, filters typed into the page,
//! open menus and dialogs, a test's result) in `useState` inside the route component, so it is
//! gone once the user leaves the page: TanStack Router unmounts the route. The Rune port keeps
//! every area's state in `App`, which never unmounts. `on_leave` is the unmount: it follows
//! `app.route` (from `App::page_changed` and every frame) and has the area of the page that was
//! left reset what the web would have dropped. Dialogs and sheets reset on close, tabs on
//! switch, where the web unmounts them, in their own areas. Preferences, filters the web keeps in `kb.ui`, cached
//! reads (React Query) and URL state are not touched; a search-only change (`?year=`, `?tab=`)
//! keeps the same route component mounted, so it is not a leave.
//!
//! Also here: `dismiss`, the × of an outcome strip ("saved", "saved with notes", an error), an
//! owner-requested addition over the web.

use rune::{Color, El, row};

use crate::app::App;
use crate::theme::Tk;
use crate::ui::ic;

/// The page areas a route path belongs to, for the leave hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Transactions,
    Reports,
    Wallets,
    Custody,
    Review,
    Other,
}

/// Which area renders `path` (the route without its query), as `main::view` dispatches.
pub fn page_of(path: &str) -> Page {
    if path == "/transactions" {
        Page::Transactions
    } else if crate::reports::owns(path) {
        Page::Reports
    } else if crate::wallets::owns(path) {
        Page::Wallets
    } else if crate::custody::owns(path) {
        Page::Custody
    } else if crate::review::owns(path) {
        Page::Review
    } else {
        Page::Other
    }
}

/// Whether going `from` → `to` unmounts the page left: a different path. A search-only change
/// re-renders the same route component; `/connections/a` → `/connections/b` re-keys the detail
/// (its own id check starts over).
pub fn leaves(from: &str, to: &str) -> bool {
    from != to
}

/// Follows `app.route` and, when its path changed since the last call, resets the state the web
/// keeps in route-local `useState` for the page that was left. Called by `App::page_changed`
/// (navigate, back, forward: before the next frame renders) and every frame from `main::poll`
/// (routes a handler sets directly, e.g. after deleting a connection). Idempotent; true when a
/// page was left.
pub fn on_leave(app: &mut App) -> bool {
    let to = app.route.path().to_owned();
    if app.shell.page.is_empty() {
        app.shell.page = to;
        return false;
    }
    if !leaves(&app.shell.page, &to) {
        return false;
    }
    let from = std::mem::replace(&mut app.shell.page, to);
    leave_page(app, &from);
    true
}

fn leave_page(app: &mut App, from: &str) {
    match page_of(from) {
        Page::Transactions => crate::transactions::on_leave(app),
        Page::Reports => crate::reports::on_leave(app, from),
        Page::Wallets => crate::wallets::on_leave(app, from),
        Page::Custody => crate::custody::on_leave(app),
        Page::Review => crate::review::on_leave(app, from),
        // Overview and the assistant keep store-persisted state; Source of funds, Privacy
        // mirror and Chain analysis reset from their own polls once their route is gone.
        Page::Other => {}
    }
}

/// The × that dismisses an outcome strip (`size-7` ghost button, the strip's ink, an
/// accessible "Dismiss"). `key` must be unique on the page.
pub(crate) fn dismiss(
    tk: &Tk,
    key: impl Into<String>,
    ink: Color,
    on_press: impl Fn(&mut App) + 'static,
) -> El {
    let label = crate::i18n::t("chrome:notifications.dismissShort");
    row()
        .key(key.into())
        .size(tk.r(1.75), tk.r(1.75))
        .center()
        .radius(tk.r(0.375))
        .hover(tk.row_hover)
        .role("button")
        .label(label.clone())
        .help(label)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(on_press)
        .child(ic(&crate::icons::X, tk.r(0.875), ink).a11y_hidden())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_path_change_unmounts_the_page() {
        assert!(!leaves("/reports", "/reports"));
        assert!(leaves("/reports", "/overview"));
        assert!(leaves("/connections/a", "/connections/b"));
        // `Route::path` drops the search first: `?year=` keeps Reports mounted.
        let a = crate::app::Route("/reports?year=2024".into());
        let b = crate::app::Route("/reports?year=2025".into());
        assert!(!leaves(a.path(), b.path()));
    }

    fn app_on(path: &str) -> App {
        let mut app = App::new();
        // A book is open (tests restore no preferences, so the identity gate would send to `/`).
        app.onboarding.identity = Some(crate::onboarding::model::Identity {
            name: "Test".into(),
            ..Default::default()
        });
        app.navigate(path);
        on_leave(&mut app);
        app
    }

    fn saved_with_notes() -> crate::reports::report::Status {
        crate::reports::report::Status {
            year: 2025,
            tone: crate::reports::report::StatusTone::Warning,
            message: "kassiber-2025.xlsx saved with notes.".into(),
            notes: vec!["3 rows need review".into()],
            path: None,
            open_path: None,
            quarantine: None,
            actions: vec![],
        }
    }

    /// The owner's report: the amber "…xlsx saved with notes" strip outlived the page.
    #[test]
    fn leaving_reports_drops_its_export_strip() {
        let mut app = app_on("/reports");
        app.reports.report.status = Some(saved_with_notes());
        // A search-only change (`?year=`) keeps the route mounted: the strip stays.
        app.navigate("/reports?year=2024");
        assert!(app.reports.report.status.is_some());
        app.navigate("/overview");
        assert!(app.reports.report.status.is_none());
        // Back to Reports and away again by history: the same hook.
        app.go_back();
        app.reports.report.status = Some(saved_with_notes());
        app.go_forward();
        assert_eq!(app.route.path(), "/overview");
        assert!(app.reports.report.status.is_none());
    }

    /// A route a handler sets directly (deleting a connection) is caught by the per-frame call,
    /// once: a second call with the same route is a no-op.
    #[test]
    fn a_direct_route_change_is_a_leave_too() {
        let mut app = app_on("/reports");
        app.reports.report.status = Some(saved_with_notes());
        app.route = crate::app::Route("/overview".into());
        assert!(on_leave(&mut app));
        assert!(app.reports.report.status.is_none());
        app.reports.report.status = Some(saved_with_notes());
        assert!(
            !on_leave(&mut app),
            "nothing left: the route did not change"
        );
        assert!(app.reports.report.status.is_some());
    }

    #[test]
    fn leaving_a_wallet_drops_its_sync_error_but_not_the_reads() {
        let mut app = app_on("/connections/w1");
        app.wallets.detail.sync_error = Some("backend unreachable".into());
        app.navigate("/overview");
        assert!(app.wallets.detail.sync_error.is_none());
    }

    #[test]
    fn pages_map_to_their_areas() {
        assert_eq!(page_of("/transactions"), Page::Transactions);
        for p in [
            "/reports",
            "/journals",
            "/exit-tax",
            "/activity",
            "/logs",
            "/egress",
            "/books",
            "/books/x/birds-eye",
        ] {
            assert_eq!(page_of(p), Page::Reports, "{p}");
        }
        for p in ["/connections", "/connections/w1", "/imports"] {
            assert_eq!(page_of(p), Page::Wallets, "{p}");
        }
        assert_eq!(page_of("/transfers"), Page::Custody);
        assert_eq!(page_of("/overview"), Page::Other);
        assert_eq!(page_of("/review"), Page::Review);
        assert_eq!(page_of("/reconcile"), Page::Review);
        assert_eq!(page_of("/chain-analysis"), Page::Other);
    }
}

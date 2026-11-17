//! Crash recovery (components/AppErrorBoundary.tsx): a panic while drawing a frame, polling or
//! handling a key shows "Something went wrong" with Reload, Copy diagnostics and Go to overview
//! instead of ending the app. The daemon keeps running; Reload starts the UI state afresh.
//!
//! What Rune allows: the app's `view`, `poll` and `on_key` are ours, so they run under
//! `catch_unwind` (`guard`). Press, drag and other handler closures are called by Rune's input
//! dispatch inside the platform event loop, where a panic is not catchable from the app (Rune's
//! README: "Handler-closure panics are not caught"); those still end the process.
//!
//! The panic hook records the message and location in RAM only; nothing is written to disk.
//! The message passes the logs' secret floor before it is shown, logged or copied.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, PanicHookInfo, catch_unwind};
use std::sync::Mutex;

use rune::{El, Tx, col, row, text};

use crate::app::App;
use crate::i18n::t;
use crate::theme::Tk;

/// Most of a panic message the screen and the diagnostics keep.
const MAX_MESSAGE: usize = 600;
/// `DIAGNOSTIC_LOG_LIMIT`: the recent log records Copy diagnostics adds.
const DIAGNOSTIC_LOG_LIMIT: usize = 200;

/// A caught panic, as the recovery screen shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Crash {
    /// The redacted panic message.
    pub message: String,
    /// `file:line:column` of the panic.
    pub location: String,
    /// Which callback panicked: "view", "poll" or "key".
    pub stage: &'static str,
    /// The route's path (never its query, which can name a transaction).
    pub route: String,
}

/// The recovery screen's own state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recovery {
    pub crash: Option<Crash>,
    pub copied: bool,
    pub copy_error: bool,
    /// The shell itself failed to draw around the error card: draw the card alone.
    pub bare: bool,
    /// The crash went to the Logs ring already.
    pub logged: bool,
}

/// What the panic hook saw last: (message, location).
static LAST_PANIC: Mutex<Option<(String, String)>> = Mutex::new(None);

thread_local! {
    /// The recovery state: `view` only gets `&App`, so the screen's state lives beside it.
    static RECOVERY: RefCell<Recovery> = RefCell::new(Recovery::default());
}

/// Installs the hook that records each panic's message and location (then runs the previous
/// hook, which prints to the console in development).
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut last) = LAST_PANIC.lock() {
            *last = Some(describe(info));
        }
        previous(info);
    }));
}

fn describe(info: &PanicHookInfo<'_>) -> (String, String) {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "Box<dyn Any>".into());
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "unknown".into());
    (message, location)
}

/// The secret floor, one line, bounded.
pub fn redact_message(message: &str) -> String {
    let flat = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = crate::daemon::redact(&flat);
    if redacted.chars().count() > MAX_MESSAGE {
        format!(
            "{}…",
            redacted.chars().take(MAX_MESSAGE).collect::<String>()
        )
    } else {
        redacted
    }
}

/// The recovery state now (None while nothing crashed).
pub fn current() -> Option<Recovery> {
    RECOVERY.with(|r| r.borrow().crash.is_some().then(|| r.borrow().clone()))
}

fn update(f: impl FnOnce(&mut Recovery)) {
    RECOVERY.with(|r| f(&mut r.borrow_mut()));
}

/// Runs `f`; a panic in it becomes the recovery screen (the first crash is kept until the user
/// recovers). None when it panicked.
pub fn guard<T>(stage: &'static str, route: &str, f: impl FnOnce() -> T) -> Option<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(v) => Some(v),
        Err(payload) => {
            let (message, location) = LAST_PANIC
                .lock()
                .ok()
                .and_then(|mut l| l.take())
                .unwrap_or_else(|| {
                    let m = payload
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_owned())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_default();
                    (m, "unknown".into())
                });
            let route = route.split(['?', '#']).next().unwrap_or("").to_owned();
            update(|r| {
                if r.crash.is_none() {
                    *r = Recovery {
                        crash: Some(Crash {
                            message: redact_message(&message),
                            location,
                            stage,
                            route,
                        }),
                        ..Recovery::default()
                    };
                }
            });
            None
        }
    }
}

/// The shell could not draw around the card: the next frames show the card alone.
pub fn mark_bare() {
    update(|r| r.bare = true);
}

/// `componentDidCatch`'s `emitAppLog`: the crash joins the Logs ring once (RAM only).
pub fn log_once(app: &mut App) {
    let Some(rec) = current().filter(|r| !r.logged) else {
        return;
    };
    let Some(c) = rec.crash else { return };
    use crate::reports::diag::logstore::{Record, text as field};
    app.reports.diag.logs.store.emit(Record {
        id: String::new(),
        ts: String::new(),
        level: "error".into(),
        module: "ui".into(),
        file: "crash.rs".into(),
        line: 0,
        msg: c.message.clone(),
        fields: vec![
            ("location".into(), field(&c.location)),
            ("stage".into(), field(c.stage)),
            ("route".into(), field(&c.route)),
        ],
    });
    update(|r| r.logged = true);
}

/// The text Copy diagnostics puts on the clipboard: the crash, then the recent log records,
/// both redacted.
pub fn diagnostics(app: &App, c: &Crash) -> String {
    use crate::reports::diag::{logstore, redact::Opts};
    let generated = logstore::now_iso();
    let all: Vec<&logstore::Record> = app.reports.diag.logs.store.records.iter().collect();
    let records = &all[all.len().saturating_sub(DIAGNOSTIC_LOG_LIMIT)..];
    let version = app
        .daemon_ready
        .as_ref()
        .and_then(|d| d["version"].as_str())
        .unwrap_or("0.0.0");
    let header = logstore::Header {
        app_version: format!("{version} (unknown)"),
        os: std::env::consts::OS.into(),
        generated_at: generated.clone(),
        time_range: if records.is_empty() {
            "none".into()
        } else {
            logstore::time_range(records)
        },
        active_filter: "error-boundary-recovery".into(),
        redaction: "redacted".into(),
    };
    let opts = Opts::screen(true, false);
    let logs = logstore::export(records, "md", &opts, &header);
    // The panic line gets the records' redaction too (txids, addresses, amounts, paths, urls):
    // the screen shows the secret-floored message, the clipboard only the shareable form.
    let (message, location) = shareable(c, &opts);
    [
        "# Kassiber crash diagnostics".to_owned(),
        String::new(),
        format!("- Route: {}", c.route),
        format!("- Generated: {generated}"),
        format!("- Panic: {message}"),
        format!("- Location: {location} ({})", c.stage),
        "- The following records are redacted for clipboard sharing.".to_owned(),
        String::new(),
        logs.trim_end().to_owned(),
        String::new(),
    ]
    .join("\n")
}

/// The panic message and location as Copy diagnostics shares them: the log export's
/// operational redaction on top of the secret floor.
fn shareable(c: &Crash, opts: &crate::reports::diag::redact::Opts) -> (String, String) {
    use crate::reports::diag::redact::text_for_mode;
    (
        text_for_mode(&c.message, opts),
        text_for_mode(&c.location, opts),
    )
}

/// Reload: the UI state starts afresh (the web reloads the page); the daemon keeps running.
pub fn reload(app: &mut App) {
    RECOVERY.with(|r| *r.borrow_mut() = Recovery::default());
    app.reload_ui();
}

/// Forgets a crash without touching an app: the scene check builds a fresh app for every render
/// on one thread, and the recovery state is per thread (one app per process outside tests).
#[cfg(test)]
pub fn reset() {
    RECOVERY.with(|r| *r.borrow_mut() = Recovery::default());
}

/// "Go to overview" (the route-level boundary's way home).
pub fn go_overview(app: &mut App) {
    RECOVERY.with(|r| *r.borrow_mut() = Recovery::default());
    app.navigate("/overview");
}

fn copy(app: &mut App, tx: &mut Tx) {
    let Some(c) = current().and_then(|r| r.crash) else {
        return;
    };
    let text = diagnostics(app, &c);
    crate::settings::effects::copy(app, tx, text);
    update(|r| {
        r.copied = true;
        r.copy_error = false;
    });
}

/// The error card (`kb-surface w-full max-w-lg space-y-4 p-6 text-center`).
pub fn card(tk: &Tk, rec: &Recovery) -> El {
    let Some(c) = &rec.crash else { return col() };
    let go = t("chrome:errorBoundary.overview");
    let reload = t("chrome:errorBoundary.reload");
    let copy_label = t(if rec.copied {
        "chrome:errorBoundary.copied"
    } else {
        "chrome:errorBoundary.copy"
    });
    // The route-level boundary offers the way home; the app-level one (the shell failed too)
    // only Reload and Copy.
    let buttons = row()
        .gap(tk.r(0.5))
        .justify_center()
        .children((!rec.bare).then(|| {
            crate::ui::outline_button(tk, "crash-overview", None, &go).on_press(go_overview)
        }))
        .child(
            crate::ui::primary_button(tk, "crash-reload", None, &reload)
                .autofocus()
                .on_press(reload_pressed),
        )
        .child(
            row()
                .key("crash-copy")
                .h(tk.r(2.0))
                .px(tk.r(0.75))
                .center()
                .radius(tk.r(0.375))
                .fill(tk.muted)
                .hover(tk.accent)
                .role("button")
                .label(copy_label.clone())
                .focusable(true)
                .on_press_tx(copy)
                .child(text(copy_label).font(tk.tsm(500.0)).color(tk.fg)),
        );
    let mut body = col()
        .key("crash-card")
        .w_full()
        .max_w(tk.r(32.0))
        .padding(tk.r(1.5))
        .gap(tk.r(1.0))
        .items_center()
        .child(
            text(t("common:state.somethingWentWrong"))
                .font(tk.tlg(600.0))
                .color(tk.fg)
                .role("heading")
                .level(1)
                .align(rune::Align::Center),
        )
        .child(
            text(c.message.clone())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap_anywhere()
                .align(rune::Align::Center)
                .role("alert")
                .label(c.message.clone()),
        )
        .child(buttons)
        .child(
            text(t("chrome:errorBoundary.details"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .align(rune::Align::Center),
        );
    if rec.copy_error {
        body = body.child(
            text(t("chrome:errorBoundary.copyFailed"))
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .wrap()
                .align(rune::Align::Center),
        );
    }
    crate::ui::surface(tk)
        .w_full()
        .max_w(tk.r(32.0))
        .child(body)
}

fn reload_pressed(app: &mut App) {
    reload(app);
}

/// The card centred in the page (`flex h-full items-center justify-center bg-background p-6`).
pub fn page(tk: &Tk, rec: &Recovery, height: f32) -> El {
    col()
        .key("crash-page")
        .w_full()
        .h(height)
        .padding(tk.r(1.5))
        .items_center()
        .justify_center()
        .fill(tk.background)
        .child(card(tk, rec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_becomes_the_recovery_state_once() {
        install();
        RECOVERY.with(|r| *r.borrow_mut() = Recovery::default());
        assert_eq!(guard("view", "/x", || 7), Some(7));
        assert!(current().is_none());
        let caught: Option<()> = guard("view", "/transactions?tx=abc", || {
            panic!("row {} out of range", 3)
        });
        assert!(caught.is_none());
        let rec = current().expect("crashed");
        let c = rec.crash.unwrap();
        assert_eq!(c.message, "row 3 out of range");
        assert_eq!(c.route, "/transactions");
        assert_eq!(c.stage, "view");
        assert!(c.location.contains("crash.rs"), "{}", c.location);
        // A second panic keeps the first crash.
        let _: Option<()> = guard("poll", "/", || panic!("second"));
        assert_eq!(
            current().unwrap().crash.unwrap().message,
            "row 3 out of range"
        );
        RECOVERY.with(|r| *r.borrow_mut() = Recovery::default());
    }

    #[test]
    fn copied_diagnostics_redact_book_data_in_the_panic_line() {
        use crate::reports::diag::redact::Opts;
        let txid = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";
        let address = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";
        let c = Crash {
            message: redact_message(&format!(
                "no row for {txid} paying {address} 0.5 BTC in /Users/alice/books/data/kassiber.sqlite3"
            )),
            location: "src/transactions/model.rs:12:5".into(),
            stage: "view",
            route: "/transactions".into(),
        };
        let (message, location) = shareable(&c, &Opts::screen(true, false));
        for leak in [txid, address, "0.5 BTC", "/Users/alice"] {
            assert!(!message.contains(leak), "{leak} leaked: {message}");
        }
        assert!(
            message.contains("txid#")
                && message.contains("[redacted-address]")
                && message.contains("[redacted-path]"),
            "{message}"
        );
        assert!(location.contains("model.rs:12:5"), "{location}");
        let (_, abs) = shareable(
            &Crash {
                location: "/Users/alice/.cargo/registry/src/x/lib.rs:1:1".into(),
                ..c
            },
            &Opts::screen(true, false),
        );
        assert!(!abs.contains("alice"), "{abs}");
    }

    #[test]
    fn messages_are_flattened_bounded_and_pass_the_secret_floor() {
        assert_eq!(redact_message("a\n  b"), "a b");
        assert!(redact_message(&"x".repeat(2000)).chars().count() <= MAX_MESSAGE + 1);
        let xpub = "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz";
        assert!(!redact_message(&format!("bad key {xpub}")).contains(xpub));
    }
}

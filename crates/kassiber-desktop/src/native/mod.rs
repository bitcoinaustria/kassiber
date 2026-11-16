//! The Tauri shell's native features (ui-tauri/src-tauri/src/lib.rs and its modules), on Rune:
//!
//! - `menu`: the native menu bar with Tauri's items, accelerators and enabled state;
//! - `deeplink` + `intent`: `kassiber://` links and the menu's intents, with the web's route
//!   and developer-mode guards (`dispatchMenuIntent`);
//! - `instance`: one running app, a relaunch focusing it and handing over its links;
//! - `cli` + `terminal`: the app binary doubling as the `kassiber` CLI, and the terminal
//!   command it installs;
//! - `notify`: OS notifications through the platform's API, fixed texts only;
//! - `update`: the consent-gated update check through the CLI, its dialogs and the sidebar pill.
//!
//! `main` wires it: [`early_exit`] and [`claim_instance`] before anything starts, [`install`] on
//! the `rune::App`, [`setup`], [`poll`], [`overlay`] and [`on_key`] in the app's hooks.

pub mod cli;
pub mod deeplink;
pub mod instance;
pub mod intent;
pub mod menu;
pub mod notify;
pub mod terminal;
pub mod update;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use rune::{El, KeyEvent, Tx};

use crate::app::App;
use crate::i18n::Lang;
use crate::theme::Tk;
use intent::Intent;

/// RAM-only state (`app.native`).
#[derive(Default)]
pub struct Ui {
    /// The menu state last handed to Rune.
    pub menu: Option<menu::MenuState>,
    /// Intents from links that arrived before the app knew whether a book is open (a launch
    /// link arrives before the first reads settle), with the app clock of the first.
    pub pending: Vec<Intent>,
    pub pending_since: Option<f64>,
    pub update: update::State,
}

static HANDLE: OnceLock<rune::AppHandle<App>> = OnceLock::new();
/// Launches another process handed over before the app's handle existed.
static EARLY: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());
/// The language Rune's own menu labels take (`rune::set_labels`).
static LABELS_DE: AtomicBool = AtomicBool::new(false);

/// How long a link waits for the first reads before it is handled with what the app knows.
const PENDING_GRACE: f64 = 10.0;

/// A waker for background work (the scanner camera) that queues results for `poll`; None
/// before the app runs (headless runs included).
pub fn waker() -> Option<rune::Waker> {
    let h = HANDLE.get()?.clone();
    Some(std::sync::Arc::new(move || h.wake()))
}

/// Before anything starts: the helper flags and CLI forwarding exit here.
pub fn early_exit() -> Option<i32> {
    cli::early_exit()
}

/// False when a running Kassiber took this launch (`main` returns without a window).
pub fn claim_instance() -> bool {
    instance::claim()
}

/// The menu items, the link hook, and Rune's labels on the app.
pub fn install(app: rune::App<App>) -> rune::App<App> {
    let _ = HANDLE.set(app.handle());
    rune::set_labels(|label| {
        menu::rune_label(
            label,
            if LABELS_DE.load(Ordering::Relaxed) {
                Lang::De
            } else {
                Lang::En
            },
        )
    });
    // Built enabled, as Tauri builds them; the first poll applies the app's state.
    let initial = menu::MenuState {
        ai_features: true,
        developer_tools: true,
        has_workspace: true,
        locked: false,
        lang: Lang::En,
    };
    let mut app = app.on_open_url(on_open_url);
    for item in menu::items(&initial) {
        app = app.menu_item(item);
    }
    app
}

/// Shows, restores and focuses the window (Tauri's `show` + `unminimize` + `set_focus`).
pub fn bring_to_front() {
    if let Some(h) = HANDLE.get() {
        h.open();
    }
}

/// `App::on_open_url`: a link the system opened with Kassiber (macOS, Windows), or a later
/// launch's link (Windows, Linux single instance).
fn on_open_url(app: &mut App, url: &str, _tx: &mut Tx) {
    receive(app, url);
    bring_to_front();
}

/// A later launch's arguments (macOS single instance): its links, then the window comes
/// forward, as the single-instance callback does.
pub fn relaunched(args: Vec<String>) {
    match HANDLE.get() {
        Some(h) => h.update(move |app: &mut App, _tx: &mut Tx| {
            for link in deeplink::links_in(&args) {
                receive(app, &link);
            }
            bring_to_front();
        }),
        None => EARLY.lock().unwrap_or_else(|p| p.into_inner()).push(args),
    }
}

/// One incoming link: an allowlisted intent waits for the app's state, anything else is
/// refused (and only its size is said).
pub fn receive(app: &mut App, url: &str) {
    match deeplink::intent_for(url) {
        Some(i) => {
            app.native.pending.push(i);
            app.native.pending_since.get_or_insert(app.clock);
        }
        None => eprintln!(
            "kassiber: ignoring unrecognized deep link {}",
            deeplink::redacted(url)
        ),
    }
}

/// After the scene, before the first frame: launch links (the `--open-url` test path; Linux's
/// launch arguments, which Rune doesn't deliver there), and launches handed over early.
pub fn setup(app: &mut App) {
    let args = rune::extra_args();
    let mut links: Vec<String> = args
        .windows(2)
        .filter(|w| w[0] == "--open-url")
        .map(|w| w[1].clone())
        .collect();
    if cfg!(target_os = "linux") {
        links.extend(deeplink::links_in(&cli::process_args()));
    }
    for l in links {
        receive(app, &l);
    }
    let early = std::mem::take(&mut *EARLY.lock().unwrap_or_else(|p| p.into_inner()));
    for args in early {
        for l in deeplink::links_in(&args) {
            receive(app, &l);
        }
    }
    flush_pending(app);
}

/// Handles waiting links once the app knows whether a book is open.
fn flush_pending(app: &mut App) -> bool {
    if app.native.pending.is_empty() {
        return false;
    }
    let known = app.reads.contains_key("ui.profiles.snapshot") || !app.daemon.is_some();
    let waited = app
        .native
        .pending_since
        .is_some_and(|t| app.clock - t >= PENDING_GRACE);
    if !known && !waited {
        return false;
    }
    app.native.pending_since = None;
    for i in std::mem::take(&mut app.native.pending) {
        intent::dispatch(app, &i);
    }
    true
}

/// Every frame: the menu's state, waiting links, update checks.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = flush_pending(app);
    if !app.native.pending.is_empty() {
        tx.wake_after(std::time::Duration::from_millis(100));
    }
    let state = menu::MenuState::of(app);
    if app.native.menu != Some(state) {
        LABELS_DE.store(state.lang == Lang::De, Ordering::Relaxed);
        match HANDLE.get().map(|h| h.set_menu(menu::items(&state))) {
            // Not running yet: try again next frame.
            Some(Err(rune::MenuError::NotRunning)) | None => {
                tx.wake_after(std::time::Duration::from_millis(100))
            }
            Some(_) => app.native.menu = Some(state),
        }
    }
    changed |= update::poll(app, tx);
    changed
}

/// Overlays: the update dialog.
pub fn overlay(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    update::dialog(app, tk, layer);
}

/// The end of the key chain: menu accelerators nothing else took.
pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    menu::on_key(app, ev, tx)
}

/// Headless states. The update scenes are SYNTHETIC (no check runs headless): they show what a
/// check that found v0.99.0 leaves behind.
pub static SCENES: &[crate::app::Scene] = &[
    ("native-update-pill", |a| {
        consent_scene(a);
        a.native.update.update = Some(synthetic_update());
    }),
    ("native-update-dialog", |a| {
        consent_scene(a);
        a.native.update.update = Some(synthetic_update());
        a.native.update.dialog = Some(update::Dialog {
            body: crate::i18n::tr(
                "chrome:shell.version.availablePrompt",
                &[("version", "0.99.0")],
            ),
            error: false,
            open_url: Some(
                "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.99.0".into(),
            ),
        });
    }),
    ("native-update-disabled", |a| {
        a.native.update.dialog = Some(update::Dialog {
            body: crate::i18n::t("chrome:shell.version.disabled"),
            error: false,
            open_url: None,
        });
    }),
];

/// Consent as the scene's: effects' first-poll read of the consent file is skipped, so the
/// isolated preference file stays untouched.
fn consent_scene(a: &mut App) {
    a.settings.fx.started = true;
    a.settings.prefs.update_checks = true;
}

fn synthetic_update() -> update::AppUpdate {
    update::AppUpdate {
        current_version: "0.22.78".into(),
        latest_version: Some("0.99.0".into()),
        release_url: Some("https://github.com/bitcoinaustria/kassiber/releases/tag/v0.99.0".into()),
        update_available: true,
        prerelease: false,
        checked_at: 0,
    }
}

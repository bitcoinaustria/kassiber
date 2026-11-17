//! Welcome and setup (`routes/Welcome.tsx`, `components/kb/Onboarding/**`), and the rules for
//! when the app shows them instead of the books (`routeTree.tsx`'s `/` and app-layout guards,
//! AppShell's `resetLocalUiSession` and empty-book check, Settings' "Reset welcome").
//!
//! - **The gate is `identity`.** Without one, every app route lands on `/` (Welcome); with one,
//!   `/` lands on Overview. Setup, opening an existing folder and the regtest demo set it;
//!   "Reset welcome", deleting the books, "Open other books" on the lock screen, an app launch
//!   whose imported book can no longer be opened, and a book with no workspaces clear it.
//! - The identity is kept in the preferences file without paths (`model::Identity`); an imported
//!   book is named there by its book key, and its path lives only in the approved-projects
//!   store (`projects`).
//! - Nothing here reaches the network on its own: Welcome reads nothing from the book; setup
//!   reads the built-in server list from the local daemon; the only egress is the Electrum
//!   "Test connection" button, sent on its press, as the web does.
//! - The regtest demo tile is the web's dev-build feature: it shows only when the app was
//!   launched on an explicit data root (`--data-root` / `KASSIBER_DEMO_DATA_ROOT`, the dev
//!   bridge's `KASSIBER_DEV_DATA_ROOT` here) whose `status` names the regtest demo, and then it
//!   opens that book at once (`autoOpenedRegtestRef`). It starts no Docker or network service.

pub mod chain;
pub mod fields;
pub mod model;
pub mod projects;
pub mod scenes;
pub mod view;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rune::{Cx, El, KeyEvent, TextEdit, Tx};
use serde_json::{Value, json};

use crate::app::{App, Reply, Route};
use crate::i18n::t;
use model::{AiMode, BackendKind, BackendMode, DbMode, Form, Identity};
use projects::{Approved, Selection};

pub use scenes::SCENES;

/// The setup steps, in order (`DEFAULT_STEPS`).
pub const STEPS: [&str; 5] = ["essentials", "sync", "ai", "security", "review"];
pub const SECURITY_STEP: usize = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flow {
    #[default]
    Start,
    Setup,
}

/// A built-in server the daemon would add (`BackendPreviewRow`).
#[derive(Clone, Debug, PartialEq)]
pub struct BackendRow {
    pub name: String,
    pub kind: String,
    pub url: String,
    pub chain: String,
    pub liquid_opt_in: bool,
}

/// The Electrum probe's state (`testState`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Test {
    #[default]
    Idle,
    Testing,
    Ok,
    Fail,
}

/// Where `finish` is.
#[derive(Clone, Debug, PartialEq)]
enum Finish {
    Secrets,
    Complete,
    AiCreate(Value),
    AiUpdate(Value),
    AiDefault,
}

/// What the finished setup recorded, until the last request answers.
#[derive(Clone, Debug, PartialEq)]
struct Finished {
    stage: Finish,
    mode: BackendMode,
    ai_mode: AiMode,
    gains: String,
    days: u32,
}

#[derive(Default)]
pub struct Ui {
    /// `identity` (persisted, without paths).
    pub identity: Option<Identity>,
    /// `firstSyncDone`, by book identity key (persisted).
    pub first_sync_done: BTreeMap<String, bool>,
    pub flow: Flow,
    pub step: usize,
    pub form: Form,
    pub submitting: bool,
    pub finish_error: Option<String>,
    finishing: Option<Finished>,
    pub import_selection: Option<Selection>,
    /// The opened folder's `ui.profiles.snapshot`.
    pub import_snapshot: Option<Value>,
    pub importing: bool,
    pub loading_profiles: bool,
    pub import_error: Option<String>,
    pub open_error: Option<String>,
    pub opening_profile: Option<String>,
    /// The book being opened (workspace, profile) until `ui.profiles.switch` answers.
    pending_open: Option<(Value, Value)>,
    pub import_pass: TextEdit,
    pub import_reveal: bool,
    /// The launch book's `status` when it is the regtest demo.
    pub regtest: Option<Value>,
    pub loading_regtest: bool,
    pub opening_regtest: bool,
    auto_opened_regtest: bool,
    /// `ui.backends.public_defaults` (None until read).
    pub defaults: Option<Vec<BackendRow>>,
    defaults_asked: bool,
    pub test: Test,
    pub test_log: String,
    /// The open select menu (field key).
    pub select: Option<String>,
    pub lang_open: bool,
    /// "Existing backend credentials" (`<details>`).
    pub migrate_open: bool,
    /// Revealed password fields.
    pub reveal: HashSet<&'static str>,
    /// Fields that lost focus once (hints wait for that or for content).
    pub touched: HashSet<&'static str>,
    focused: Option<&'static str>,
    pub approved: Approved,
    /// The data root the launch opened (an explicit root), where closing a folder returns.
    pub seed: Option<PathBuf>,
    pub seed_explicit: bool,
    /// AppShell's empty-book check ran for this entry into the app.
    validated: bool,
    pub chain: chain::Loop,
    /// The shell's wordmark assembles after setup created the books (`celebrateBooksCreated`).
    pub celebrate: bool,
    /// Why the imported book this launch should open could not be opened (AppShell's
    /// `importRootError`): the restore screen shows it with "Back to setup".
    pub import_root_error: Option<String>,
}

/// Tags this module sends with.
const TAG_REGTEST: &str = "onboarding.regtest_status";
const TAG_DEFAULTS: &str = "onboarding.defaults";
const TAG_SECRETS: &str = "onboarding.secrets_init";
const TAG_COMPLETE: &str = "onboarding.complete";
const TAG_AI: &str = "onboarding.ai";
const TAG_UNLOCK: &str = "onboarding.import_unlock";
const TAG_PROFILES: &str = "onboarding.import_profiles";
const TAG_SWITCH: &str = "onboarding.import_switch";
const TAG_TEST: &str = "onboarding.electrum_test";

// ---- The gate ---------------------------------------------------------------------------------

/// Welcome is up: `/` without an identity.
pub fn shows(app: &App) -> bool {
    app.onboarding.identity.is_none() && app.route.path() == "/"
}

/// The route guards (`indexRoute` / `appLayoutRoute` `beforeLoad`): `/` with an identity goes to
/// Overview; every app route without one goes to `/`. The dev harness pages stay reachable.
pub fn redirect(app: &App, path: &str) -> Option<String> {
    let has = app.onboarding.identity.is_some();
    if path == "/" || path.is_empty() {
        return has.then(|| "/overview".to_owned());
    }
    (!has && !path.starts_with("/dev/")).then(|| "/".to_owned())
}

/// `App::new`, after the saved preferences: decides what the launch opens. True when the caller
/// should load the shell's reads (the app opens on its launch root); false for Welcome (which
/// reads nothing from the book) or when opening an imported book already loaded them.
pub fn boot(app: &mut App) -> bool {
    let ui = &mut app.onboarding;
    ui.approved = Approved::load();
    ui.seed_explicit = app.launch.explicit_root;
    ui.seed = if app.launch.explicit_root {
        app.launch.data_root.clone()
    } else {
        None
    };
    ui.chain = chain::Loop::new(0.5);
    match app.onboarding.identity.clone() {
        Some(id) => {
            app.onboarding.validated = false;
            if let Some(key) = id.imported.as_deref() {
                // `activateImportProject(identity.importedProject.dataRoot)`: the daemon starts
                // on that book (never on the default root first); the shell's reads follow once
                // it is ready.
                if let Err(e) = activate_imported(app, key) {
                    // The restore screen holds the shell, locked, until "Back to setup".
                    import_root_failed(app, e);
                }
                return false;
            }
            app.encrypted = id.is_encrypted();
            true
        }
        None => {
            enter_welcome(app);
            false
        }
    }
}

/// `activateImportProject` failed: the error (or "Could not open the selected local books
/// folder.") on the restore screen, and the app locked behind it (`setLocked(true)`).
pub fn import_root_failed(app: &mut App, error: String) {
    let error = if error.trim().is_empty() {
        crate::i18n::t("chrome:importRoot.couldNotOpen")
    } else {
        error
    };
    app.onboarding.import_root_error = Some(error);
    app.locked = true;
}

/// The book an identity's imported project names: approved and inspected now, opened once its
/// daemon is ready (a failure then shows the restore screen).
fn activate_imported(app: &mut App, key: &str) -> Result<(), String> {
    let root =
        app.onboarding.approved.by_key(key)?.ok_or(
            "Choose this Kassiber project with the native folder picker before opening it.",
        )?;
    let root = app.onboarding.approved.require(&root)?;
    let sel = projects::inspect(std::path::Path::new(&root))?;
    let encrypted = sel.encrypted;
    switch_to(
        app,
        &sel,
        move |app: &mut App, result: Result<(), String>| match result {
            Ok(()) => {
                app.encrypted = encrypted;
                if app.onboarding.identity.is_some() {
                    app.load_book();
                }
            }
            Err(e) => import_root_failed(app, e),
        },
    );
    Ok(())
}

/// Restarts the daemon on a chosen book folder (`set_data_root`) on the user's own preference
/// files; a book opened by path wants `require_existing_project` on its unlocks, as an explicit
/// root does. `done` runs once the daemon is decided.
fn switch_to(
    app: &mut App,
    sel: &Selection,
    done: impl FnOnce(&mut App, Result<(), String>) + 'static,
) {
    crate::app::switch_root(
        app,
        Some(PathBuf::from(&sel.data_root)),
        Vec::new(),
        move |app: &mut App, result: Result<(), String>| {
            if result.is_ok() {
                app.daemon.launch.explicit_root = true;
                app.launch.explicit_root = true;
            }
            done(app, result)
        },
    );
}

/// `clear_import_project`: back to the launch's own root (and the user's own preference files).
fn clear_import(app: &mut App, done: impl FnOnce(&mut App, Result<(), String>) + 'static) {
    let seed = app.onboarding.seed.clone();
    crate::app::switch_root(
        app,
        seed,
        Vec::new(),
        move |app: &mut App, result: Result<(), String>| {
            if result.is_ok() {
                let explicit = app.onboarding.seed_explicit;
                app.daemon.launch.explicit_root = explicit;
                app.launch.explicit_root = explicit;
            }
            done(app, result)
        },
    );
}

/// Mounts Welcome afresh at `/` (its component state starts over), and, on a dev-seeded launch,
/// asks the daemon whether its book is the regtest demo.
pub fn enter_welcome(app: &mut App) {
    enter_welcome_with(app, true);
}

pub fn enter_welcome_with(app: &mut App, probe: bool) {
    let keep_identity = app.onboarding.identity.take();
    let ui = std::mem::take(&mut app.onboarding);
    app.onboarding = Ui {
        identity: keep_identity,
        first_sync_done: ui.first_sync_done,
        approved: ui.approved,
        seed: ui.seed,
        seed_explicit: ui.seed_explicit,
        chain: chain::Loop {
            at: app.clock,
            ..chain::Loop::new(0.5)
        },
        ..Ui::default()
    };
    app.onboarding.identity = None;
    app.route = Route("/".into());
    app.back.clear();
    app.forward.clear();
    app.shell.close_popups();
    if app.locked {
        app.locked = false;
        app.lock = Default::default();
    }
    if probe {
        probe_regtest(app);
    }
}

/// `import.meta.env.DEV` + the regtest `status` read (`onboarding-regtest-status`). Headless
/// runs wait for it, so a scene sees what the first visible frame would.
fn probe_regtest(app: &mut App) {
    if !app.onboarding.seed_explicit {
        return;
    }
    app.onboarding.loading_regtest = true;
    let id = app.send("status", Value::Null, TAG_REGTEST);
    if crate::persist::file::tooling_run()
        && let Some(id) = id
    {
        pump_until(
            app,
            |a| !a.pending.contains_key(&id),
            Duration::from_secs(20),
        );
    }
}

pub fn pump_until(app: &mut App, done: impl Fn(&App) -> bool, limit: Duration) {
    let deadline = Instant::now() + limit;
    while (app.switching() || !done(app)) && Instant::now() < deadline {
        app.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    app.pump();
}

/// Enters the app on the book just chosen or created: Overview, the shell's reads, and the
/// empty-book check armed again.
pub(crate) fn enter_app(app: &mut App, identity: Identity) {
    app.encrypted = identity.is_encrypted();
    app.onboarding.identity = Some(identity);
    app.onboarding.validated = false;
    app.locked = false;
    app.lock = Default::default();
    app.daemon_auth_required = false;
    app.navigate("/overview");
    app.back.clear();
    app.send("status", Value::Null, "app.status");
    app.load_shell_reads();
}

/// `resetLocalUiSession` (the lock screen's "Open other books", the empty-book check): the
/// session's reads, notifications and hidden-amounts state go, an opened folder closes, and
/// Welcome takes over. No book is touched.
pub fn reset_local_session(app: &mut App) {
    drop_session(app);
    app.daemon_auth_required = false;
    app.prefs.privacy = crate::app::Privacy::Normal;
    app.shell.recent.clear();
    leave_to_welcome(app);
}

/// Settings' "Reset welcome" (after its confirmation) and a deleted book set.
pub fn reset_welcome(app: &mut App) {
    crate::settings::close(app);
    drop_session(app);
    leave_to_welcome(app);
}

/// Every cached read and screen state of the session goes (the lock's own reset, without
/// leaving the lock up).
fn drop_session(app: &mut App) {
    app.lock_session(false);
    app.locked = false;
    app.lock = Default::default();
}

/// Welcome on the launch's own root. Inside the demo this is the demo's one way out
/// (`demo::leave_then`): the copy's daemon, preference files and banner never outlive it, so a
/// book set up next lands in the user's own root.
fn leave_to_welcome(app: &mut App) {
    if app.demo.active.is_some() {
        crate::demo::leave_then(app, crate::demo::Then::Welcome);
        return;
    }
    if app
        .onboarding
        .identity
        .as_ref()
        .is_some_and(|i| i.imported.is_some())
    {
        // Errors are ignored here, as the web's `.catch(() => {})`.
        clear_import(app, |_, _| {});
    }
    app.onboarding.identity = None;
    enter_welcome(app);
}

// ---- Start choices ------------------------------------------------------------------------------

/// "Create new books".
pub fn begin_setup(app: &mut App) {
    let ui = &mut app.onboarding;
    ui.finish_error = None;
    ui.import_error = None;
    ui.flow = Flow::Setup;
    request_defaults(app);
}

/// `ui.backends.public_defaults` (`refetchOnMount: "always"` while setup is up): local.
fn request_defaults(app: &mut App) {
    app.onboarding.defaults_asked = true;
    app.send("ui.backends.public_defaults", Value::Null, TAG_DEFAULTS);
}

pub fn go_back(app: &mut App) {
    let ui = &mut app.onboarding;
    ui.select = None;
    if ui.step > 0 {
        ui.step -= 1;
        return;
    }
    ui.flow = Flow::Start;
}

pub fn jump(app: &mut App, index: usize) {
    let ui = &mut app.onboarding;
    ui.finish_error = None;
    ui.select = None;
    ui.step = index;
}

/// `handleSubmit`: the step's gate, then the next step or `finish`.
pub fn submit(app: &mut App) {
    let ui = &mut app.onboarding;
    if ui.submitting {
        return;
    }
    ui.finish_error = None;
    ui.select = None;
    if !ui.form.step_complete(ui.step) {
        return;
    }
    if ui.step + 1 < STEPS.len() {
        ui.step += 1;
        if ui.step == 0 {
            ui.form.workspace.focused = true;
        }
        if ui.step == SECURITY_STEP && ui.form.db_mode == DbMode::Sqlcipher {
            ui.form.passphrase.focused = true;
        }
        return;
    }
    ui.submitting = true;
    if let Err(e) = finish(app) {
        app.onboarding.finish_error = Some(e);
        app.onboarding.submitting = false;
    }
}

/// `finish`: the update-check answer, the encrypted database, the books, then the assistant.
fn finish(app: &mut App) -> Result<(), String> {
    let f = &app.onboarding.form;
    let mode = f
        .backend_mode
        .ok_or_else(|| t("onboarding:shell.finishError"))?;
    let offline = mode == BackendMode::Skip;
    let ai_mode = if offline { AiMode::Disabled } else { f.ai_mode };
    let allowed = model::gains_for(&f.tax_country);
    let gains = if allowed.contains(&f.gains) {
        f.gains.clone()
    } else {
        model::default_gains(&f.tax_country)
    };
    let days = if !model::uses_long_term_days(&f.tax_country) {
        0
    } else {
        model::parse_long_term_days(&f.long_term_days.text).unwrap_or(365)
    };
    let updates = !offline && f.update_checks;
    // `setAppUpdateChecksEnabled` first, before any book state; it fails closed. Only a tooling
    // run without a named consent file skips saving an "off", which is what "off" asks for.
    if updates || crate::settings::effects::update_preference_writable() {
        crate::settings::effects::set_update_checks(app, updates);
        if app.settings.prefs.update_checks != updates {
            app.toasts.retain(|t| t.key.is_some());
            return Err(t("settings:privacy.updatesSaveFailedBody"));
        }
    }
    let encrypted = app.onboarding.form.db_mode == DbMode::Sqlcipher;
    app.onboarding.finishing = Some(Finished {
        stage: if encrypted {
            Finish::Secrets
        } else {
            Finish::Complete
        },
        mode,
        ai_mode,
        gains,
        days,
    });
    if encrypted {
        let f = &app.onboarding.form;
        let args = json!({"auth_response": {"passphrase_secret": f.passphrase.text}, "migrate_credentials": f.migrate_credentials});
        app.send("ui.secrets.init", args, TAG_SECRETS);
    } else {
        send_complete(app);
    }
    Ok(())
}

fn send_complete(app: &mut App) {
    let Some(fin) = app.onboarding.finishing.as_mut() else {
        return;
    };
    fin.stage = Finish::Complete;
    let args = app
        .onboarding
        .form
        .complete_args(fin.mode, fin.days, &fin.gains.clone());
    app.send("ui.onboarding.complete", args, TAG_COMPLETE);
}

/// The provider the assistant step configured, or None to skip that part.
fn provider_args(app: &App, ai_mode: AiMode) -> Option<Value> {
    let f = &app.onboarding.form;
    if ai_mode == AiMode::Disabled || f.ai_url.text.trim().is_empty() {
        return None;
    }
    let name = model::or(f.ai_name.text.trim(), f.ai_kind.id()).to_owned();
    Some(
        json!({"name": name, "base_url": f.ai_url.text.trim(), "kind": f.ai_kind.id(), "acknowledged": f.ai_remote_ack}),
    )
}

fn finish_failed(app: &mut App, msg: String) {
    app.onboarding.finishing = None;
    app.onboarding.submitting = false;
    app.onboarding.finish_error = Some(msg);
}

/// The last step of `finish`: the identity, and Overview.
fn finish_done(app: &mut App) {
    let Some(fin) = app.onboarding.finishing.take() else {
        return;
    };
    let f = &app.onboarding.form;
    let name = model::or(f.profile.trim(), "Private").to_owned();
    let identity = Identity {
        name: name.clone(),
        workspace: model::or(f.workspace.text.trim(), "My Books").to_owned(),
        country: model::iso(&f.tax_country).unwrap_or("Generic").to_owned(),
        encrypted: f.db_mode == DbMode::Sqlcipher,
        profile: name,
        tax_country: Some(f.tax_country.clone()),
        fiat_currency: Some(f.fiat.clone()),
        tax_long_term_days: Some(fin.days),
        gains_algorithm: Some(fin.gains.clone()),
        database_mode: Some(
            if f.db_mode == DbMode::Sqlcipher {
                "sqlcipher"
            } else {
                "plaintext"
            }
            .into(),
        ),
        backend_setup_mode: Some(fin.mode.id().into()),
        ai_setup_mode: Some(fin.ai_mode.id().into()),
        imported: None,
    };
    // The passphrase leaves the UI's memory with the form.
    app.onboarding.form = Form::default();
    app.onboarding.submitting = false;
    app.onboarding.celebrate = true;
    enter_app(app, identity);
}

// ---- Opening an existing folder -----------------------------------------------------------------

/// "Open existing books": the native folder picker, then `select_import_project_directory`.
pub fn begin_import(app: &mut App, tx: &mut Tx) {
    let ui = &mut app.onboarding;
    if ui.importing || ui.submitting {
        return;
    }
    ui.finish_error = None;
    ui.import_error = None;
    ui.import_snapshot = None;
    ui.importing = true;
    let options = rune::FileDialogOptions {
        title: "Choose a Kassiber project folder".into(),
        directory: projects::picker_root(),
        ..Default::default()
    };
    tx.open_folder(
        options,
        |a: &mut App, r: rune::FileDialogResult, _tx: &mut Tx| match r {
            Ok(Some(path)) => picked(a, &path),
            Ok(None) => a.onboarding.importing = false,
            Err(e) => {
                a.onboarding.importing = false;
                a.onboarding.import_error = Some(e.0);
            }
        },
    );
}

/// A folder came back from the picker (or a test injected one): inspect, approve, open. The
/// "Open existing books" button keeps its spinner until the folder's daemon is ready.
pub fn picked(app: &mut App, path: &std::path::Path) {
    let failed = |app: &mut App, e: String| {
        app.onboarding.importing = false;
        app.onboarding.import_error = Some(if e.is_empty() {
            t("onboarding:shell.errorImportProject")
        } else {
            e
        });
    };
    let sel = match projects::inspect(path)
        .and_then(|sel| app.onboarding.approved.approve(&sel.data_root).map(|_| sel))
    {
        Ok(sel) => sel,
        Err(e) => return failed(app, e),
    };
    app.onboarding.importing = true;
    let opened = sel.clone();
    switch_to(
        app,
        &sel,
        move |app: &mut App, result: Result<(), String>| {
            if let Err(e) = result {
                return failed(app, e);
            }
            app.onboarding.importing = false;
            let encrypted = opened.encrypted;
            app.encrypted = encrypted;
            app.onboarding.import_selection = Some(opened);
            app.onboarding.import_pass = TextEdit::new("");
            app.onboarding.import_pass.focused = encrypted;
            if !encrypted {
                unlock_import(app, None);
            }
        },
    );
}

/// `unlockAndLoadImportedProfiles`: `daemon.unlock` on the opened folder, then its books.
pub fn unlock_import(app: &mut App, passphrase: Option<String>) {
    let ui = &mut app.onboarding;
    ui.loading_profiles = true;
    ui.import_error = None;
    ui.open_error = None;
    let mut args = json!({"require_existing_project": true});
    if let Some(p) = passphrase.filter(|p| !p.is_empty()) {
        args["auth_response"] = json!({"passphrase_secret": p});
    }
    app.send("daemon.unlock", args, TAG_UNLOCK);
}

pub fn submit_import_pass(app: &mut App) {
    let p = app.onboarding.import_pass.text.clone();
    if p.is_empty() || app.onboarding.loading_profiles {
        return;
    }
    // `.finally(() => setPassphrase(""))`.
    app.onboarding.import_pass = TextEdit::new("");
    app.onboarding.import_pass.focused = true;
    unlock_import(app, Some(p));
}

pub fn refresh_import_profiles(app: &mut App) {
    app.onboarding.loading_profiles = true;
    app.onboarding.import_error = None;
    app.send("ui.profiles.snapshot", Value::Null, TAG_PROFILES);
}

/// `openProfile`: switch to the book, then enter it.
pub fn open_profile(app: &mut App, workspace: Value, profile: Value) {
    if app.onboarding.opening_profile.is_some() {
        return;
    }
    let id = profile["id"].as_str().unwrap_or_default().to_owned();
    app.onboarding.open_error = None;
    app.onboarding.opening_profile = Some(id.clone());
    app.onboarding.pending_open = Some((workspace, profile));
    app.send("ui.profiles.switch", json!({"profile_id": id}), TAG_SWITCH);
}

/// "Back to new setup": the folder closes and Welcome starts over.
pub fn cancel_import(app: &mut App) {
    app.onboarding.import_error = None;
    app.onboarding.import_pass = TextEdit::new("");
    clear_import(
        app,
        |app: &mut App, result: Result<(), String>| match result {
            Ok(()) => {
                let ui = &mut app.onboarding;
                ui.import_selection = None;
                ui.import_snapshot = None;
                ui.loading_profiles = false;
                ui.flow = Flow::Start;
            }
            Err(e) => {
                app.onboarding.import_error = Some(if e.is_empty() {
                    t("onboarding:shell.errorReturnDefaultRoot")
                } else {
                    e
                });
            }
        },
    );
}

fn identity_from_profile(app: &App, workspace: &Value, profile: &Value) -> Identity {
    let sel = app.onboarding.import_selection.clone();
    let encrypted = sel.as_ref().is_some_and(|s| s.encrypted);
    let country = profile["taxCountry"]
        .as_str()
        .map(str::to_lowercase)
        .filter(|c| crate::reports::books::model::country(c).is_some())
        .unwrap_or_else(|| "generic".into());
    let days = profile["taxLongTermDays"]
        .as_f64()
        .filter(|d| d.is_finite())
        .map(|d| d.max(0.0) as u32)
        .unwrap_or(if model::uses_long_term_days(&country) {
            365
        } else {
            0
        });
    let gains = profile["gainsAlgorithm"]
        .as_str()
        .filter(|g| model::GAINS_ALGORITHMS.contains(g))
        .map(str::to_owned)
        .unwrap_or_else(|| model::default_gains(&country));
    let name = profile["name"].as_str().unwrap_or_default().to_owned();
    Identity {
        name: name.clone(),
        workspace: workspace["name"].as_str().unwrap_or_default().to_owned(),
        country: model::iso(&country).unwrap_or("Generic").to_owned(),
        encrypted,
        profile: name,
        fiat_currency: profile["fiatCurrency"]
            .as_str()
            .or(workspace["currency"].as_str())
            .map(str::to_owned),
        tax_country: Some(country),
        tax_long_term_days: Some(days),
        gains_algorithm: Some(gains),
        database_mode: Some(if encrypted { "sqlcipher" } else { "plaintext" }.into()),
        imported: sel
            .map(|s| crate::persist::model::book_key(Some(std::path::Path::new(&s.data_root)))),
        ..Identity::default()
    }
}

// ---- Regtest demo ------------------------------------------------------------------------------

/// "Open Regtest Demo" (and its automatic open): the launch book, as the demo's identity.
pub fn open_regtest(app: &mut App) {
    let Some(status) = app.onboarding.regtest.clone() else {
        return;
    };
    if app.onboarding.opening_regtest {
        return;
    }
    app.onboarding.opening_regtest = true;
    app.onboarding.finish_error = None;
    app.onboarding.import_error = None;
    let s = |k: &str, d: &str| {
        status[k]
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(d)
            .to_owned()
    };
    let profile = s("current_profile", "Full Accounting");
    let encrypted = status["database_encrypted"].as_bool().unwrap_or(false);
    // `regtestDemoImportedProject`: only a folder the picker approved and opened is imported.
    let imported = status["data_root"]
        .as_str()
        .filter(|r| {
            app.onboarding
                .import_selection
                .as_ref()
                .is_some_and(|s| &s.data_root == r)
        })
        .map(|r| crate::persist::model::book_key(Some(std::path::Path::new(r))));
    let identity = Identity {
        name: profile.clone(),
        workspace: s("current_workspace", "Regtest Demo"),
        country: "AT".into(),
        encrypted,
        profile,
        tax_country: Some("at".into()),
        fiat_currency: Some("EUR".into()),
        tax_long_term_days: Some(0),
        gains_algorithm: Some("MOVING_AVERAGE_AT".into()),
        database_mode: Some(if encrypted { "sqlcipher" } else { "plaintext" }.into()),
        backend_setup_mode: Some("custom".into()),
        imported,
        ..Identity::default()
    };
    if let Some(k) = model::book_identity_key(Some(&identity)) {
        app.onboarding.first_sync_done.insert(k, true);
    }
    app.onboarding.opening_regtest = false;
    enter_app(app, identity);
}

/// The automatic open (`autoOpenedRegtestRef`): once per Welcome mount.
fn maybe_auto_open_regtest(app: &mut App) {
    let ui = &app.onboarding;
    if ui.import_selection.is_some()
        || ui.flow != Flow::Start
        || ui.loading_regtest
        || ui.regtest.is_none()
        || ui.opening_regtest
        || ui.auto_opened_regtest
    {
        return;
    }
    app.onboarding.auto_opened_regtest = true;
    open_regtest(app);
}

// ---- Electrum test ------------------------------------------------------------------------------

/// "Test connection": the one setup control that reaches the network, on its press only.
pub fn run_electrum_test(app: &mut App) {
    let f = &app.onboarding.form;
    let url = model::electrum_url(&f.backend_host.text, &f.backend_port.text, f.use_ssl);
    let hint = model::endpoint_hint(f.backend_kind, &f.effective_backend_url());
    if hint.is_some() || url.is_empty() {
        app.onboarding.test = Test::Fail;
        app.onboarding.test_log = format!(
            "{}\n{}",
            t("onboarding:connections.validationFailed"),
            hint.map(str::to_owned)
                .unwrap_or_else(|| t("onboarding:connections.endpointRequired"))
        );
        return;
    }
    let (ph, pp) = (f.proxy_host.text.trim(), f.proxy_port.text.trim());
    let mut args = json!({"url": url, "trust_self_signed": f.use_ssl && f.trust_ssl});
    if f.use_ssl && !f.trust_ssl && !f.certificate.text.trim().is_empty() {
        args["certificate"] = json!(f.certificate.text.trim());
    }
    if f.use_proxy && !ph.is_empty() && !pp.is_empty() {
        args["proxy"] = json!(format!("{ph}:{pp}"));
    }
    app.onboarding.test = Test::Testing;
    app.onboarding.test_log.clear();
    app.send("ui.backends.electrum.test", args, TAG_TEST);
}

pub fn reset_test(app: &mut App) {
    app.onboarding.test = Test::Idle;
    app.onboarding.test_log.clear();
}

// ---- Replies -----------------------------------------------------------------------------------

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    // A book opened from the books menu or the lock screen becomes the identity
    // (`identityFromProject`), and its folder is approved (`approve_daemon_project_response`).
    if matches!(r.kind.as_str(), "ui.projects.select" | "ui.projects.create")
        && r.error.is_none()
        && let Some(root) = r.data["project"]["data_root"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
    {
        if let Err(e) = app.onboarding.approved.approve(root) {
            app.toast(
                crate::app::ToastTone::Error,
                "Choose the project again after checking app config permissions.",
                Some(e),
                None,
            );
        }
        if r.kind == "ui.projects.select" {
            project_selected(app, &r.data);
        }
    }
    match tag {
        "app.profiles" => empty_book_check(app, r),
        // A finished refresh: the book's first sync is behind it (`markFirstSyncDone`).
        "app.freshness_run" if r.error.is_none() => {
            if let Some(k) = model::book_identity_key(app.onboarding.identity.as_ref()) {
                app.onboarding.first_sync_done.insert(k, true);
            }
        }
        TAG_REGTEST => {
            app.onboarding.loading_regtest = false;
            let d = &r.data;
            let is_regtest = r.error.is_none()
                && (d["data_root"].as_str().is_some_and(is_regtest_demo_root)
                    || d["current_workspace"] == "Regtest Demo"
                    || d["default_backend"] == "core-regtest");
            app.onboarding.regtest = is_regtest.then(|| d.clone());
            maybe_auto_open_regtest(app);
        }
        TAG_DEFAULTS => {
            app.onboarding.defaults = Some(match &r.error {
                Some(_) => vec![],
                None => r.data["backends"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|b| {
                        let s =
                            |k: &str| b[k].as_str().map(str::trim).unwrap_or_default().to_owned();
                        BackendRow {
                            name: s("name"),
                            kind: s("kind"),
                            url: s("url"),
                            chain: if s("chain").is_empty() {
                                "bitcoin".into()
                            } else {
                                s("chain")
                            },
                            liquid_opt_in: b["liquid_opt_in"] == true,
                        }
                    })
                    .filter(|b| !b.name.is_empty() && !b.kind.is_empty() && !b.url.is_empty())
                    .collect(),
            });
        }
        TAG_SECRETS => match &r.error {
            Some(e) if r.error_detail["code"] == "auth_required" => {
                let _ = e;
                finish_failed(app, t("onboarding:shell.passphraseRequired"));
            }
            Some(_) => {
                let msg = r.error_detail["message"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| t("onboarding:shell.errorInitSqlcipher"));
                finish_failed(app, msg);
            }
            None => send_complete(app),
        },
        TAG_COMPLETE => match &r.error {
            Some(_) if r.error_detail["code"] == "auth_required" => {
                finish_failed(app, t("onboarding:shell.passphraseRequired"))
            }
            Some(_) => finish_failed(
                app,
                r.error_detail["message"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| t("onboarding:shell.finishError")),
            ),
            None => {
                let ai_mode = app
                    .onboarding
                    .finishing
                    .as_ref()
                    .map(|f| f.ai_mode)
                    .unwrap_or(AiMode::Disabled);
                match provider_args(app, ai_mode) {
                    Some(args) => {
                        if let Some(f) = app.onboarding.finishing.as_mut() {
                            f.stage = Finish::AiCreate(args.clone());
                        }
                        app.send("ai.providers.create", args, TAG_AI);
                    }
                    None => finish_done(app),
                }
            }
        },
        TAG_AI => {
            let stage = app.onboarding.finishing.as_ref().map(|f| f.stage.clone());
            let msg = r.error_detail["message"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| t("onboarding:shell.finishError"));
            match (stage, &r.error) {
                // "create or update": the seeded default provider may already exist.
                (Some(Finish::AiCreate(args)), Some(_)) if r.error_detail["code"] == "conflict" => {
                    if let Some(f) = app.onboarding.finishing.as_mut() {
                        f.stage = Finish::AiUpdate(args.clone());
                    }
                    app.send("ai.providers.update", args, TAG_AI);
                }
                (Some(Finish::AiCreate(args) | Finish::AiUpdate(args)), None) => {
                    if let Some(f) = app.onboarding.finishing.as_mut() {
                        f.stage = Finish::AiDefault;
                    }
                    app.send(
                        "ai.providers.set_default",
                        json!({"name": args["name"]}),
                        TAG_AI,
                    );
                }
                (Some(Finish::AiDefault), None) => finish_done(app),
                (Some(_), Some(_)) => finish_failed(app, msg),
                _ => {}
            }
        }
        TAG_UNLOCK => match &r.error {
            Some(_) => {
                app.onboarding.loading_profiles = false;
                let msg = if r.error_detail["code"] == "auth_required" {
                    t("onboarding:shell.passphraseRequired")
                } else {
                    let m = crate::app::unlock_failure(r);
                    if m.is_empty() {
                        t("onboarding:shell.errorOpenProject")
                    } else {
                        m
                    }
                };
                app.onboarding.import_error = Some(msg.clone());
                app.onboarding.open_error = Some(msg);
            }
            None => {
                if let Some(enc) = r.data["status"]["database_encrypted"].as_bool() {
                    app.encrypted = enc;
                }
                refresh_import_profiles(app);
            }
        },
        TAG_PROFILES => {
            app.onboarding.loading_profiles = false;
            match &r.error {
                Some(_) if r.error_detail["code"] == "auth_required" => {
                    app.onboarding.import_error = Some(t("onboarding:shell.passphraseRequired"))
                }
                Some(_) => {
                    app.onboarding.import_error = Some(
                        r.error_detail["message"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| t("onboarding:shell.errorLoadBooks")),
                    )
                }
                None => {
                    app.onboarding.import_snapshot = Some(if r.data.is_null() {
                        json!({"workspaces": [], "activeProfileId": ""})
                    } else {
                        r.data.clone()
                    })
                }
            }
        }
        TAG_SWITCH => {
            let pending = app.onboarding.pending_open.take();
            app.onboarding.opening_profile = None;
            match (&r.error, pending) {
                (Some(_), _) => {
                    app.onboarding.open_error =
                        Some(if r.error_detail["code"] == "auth_required" {
                            t("onboarding:import.passphraseRequired")
                        } else {
                            r.error_detail["message"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| t("onboarding:import.errorOpenBooks"))
                        });
                }
                (None, Some((w, p))) => {
                    let identity = identity_from_profile(app, &w, &p);
                    app.onboarding.import_selection = None;
                    app.onboarding.import_snapshot = None;
                    enter_app(app, identity);
                }
                (None, None) => {}
            }
        }
        TAG_TEST => match &r.error {
            Some(e) => {
                app.onboarding.test = Test::Fail;
                app.onboarding.test_log = if e.is_empty() {
                    t("onboarding:connections.testFailed")
                } else {
                    e.clone()
                };
            }
            None => {
                app.onboarding.test = if r.data["ok"] == true {
                    Test::Ok
                } else {
                    Test::Fail
                };
                app.onboarding.test_log = r.data["logs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n");
            }
        },
        _ => {}
    }
}

/// `isRegtestDemoDataRoot`: the harness's demo book (`KASSIBER_REGTEST_DEMO_HOME`, default
/// `~/.kassiber/regtest-demo`, its `data` folder).
fn is_regtest_demo_root(root: &str) -> bool {
    let home = std::env::var_os("KASSIBER_REGTEST_DEMO_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| crate::daemon::home_dir().map(|h| h.join(".kassiber").join("regtest-demo")));
    let norm = |s: &str| s.trim().trim_end_matches(['/', '\\']).to_owned();
    home.is_some_and(|h| norm(&h.join("data").to_string_lossy()) == norm(root))
}

/// AppShell's empty-book check: a set-up (not imported) identity whose daemon has no
/// workspaces at all goes back to Welcome. Once per entry into the app; a failed read is not
/// evidence.
fn empty_book_check(app: &mut App, r: &Reply) {
    let ui = &app.onboarding;
    if ui.validated
        || r.error.is_some()
        || ui.identity.as_ref().is_none_or(|i| i.imported.is_some())
    {
        return;
    }
    app.onboarding.validated = true;
    if r.data["workspaces"]
        .as_array()
        .is_some_and(|w| w.is_empty())
    {
        reset_local_session(app);
    }
}

/// `identityFromProject` after a successful `ui.projects.select`.
fn project_selected(app: &mut App, data: &Value) {
    let (project, status) = (&data["project"], &data["status"]);
    let encrypted = status["database_encrypted"]
        .as_bool()
        .or(project["encrypted"].as_bool())
        .unwrap_or(false);
    let pname = project["name"].as_str().unwrap_or_default();
    let profile = status["current_profile"]
        .as_str()
        .unwrap_or(pname)
        .to_owned();
    let root = project["data_root"].as_str().unwrap_or_default();
    app.onboarding.identity = Some(Identity {
        name: profile.clone(),
        workspace: status["current_workspace"]
            .as_str()
            .unwrap_or(pname)
            .to_owned(),
        profile,
        country: "Generic".into(),
        encrypted,
        database_mode: Some(if encrypted { "sqlcipher" } else { "plaintext" }.into()),
        imported: Some(crate::persist::model::book_key(Some(std::path::Path::new(
            root,
        )))),
        ..Identity::default()
    });
    app.onboarding.validated = true;
}

// ---- Every frame -------------------------------------------------------------------------------

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    if !shows(app) {
        return false;
    }
    let mut changed = false;
    // Welcome has no lock: a book's challenge while setup runs is the step's error
    // (`handleAuthRequired` + `passphraseRequired`).
    if app.locked {
        app.locked = false;
        app.lock = Default::default();
        app.daemon_auth_required = false;
        let msg = t("onboarding:shell.passphraseRequired");
        if app.onboarding.submitting {
            finish_failed(app, msg);
        } else if app.onboarding.loading_profiles {
            app.onboarding.loading_profiles = false;
            app.onboarding.import_error = Some(msg);
        }
        changed = true;
    }
    if app.onboarding.flow == Flow::Setup && !app.onboarding.defaults_asked {
        request_defaults(app);
    }
    changed |= track_touched(app);
    changed |= onion_effect(app);
    // The live chain: one beat every 0.9 s while the start page shows.
    if app.onboarding.flow == Flow::Start && app.onboarding.import_selection.is_none() {
        let c = &mut app.onboarding.chain;
        if app.clock - c.at >= chain::TICK {
            c.advance(app.clock);
            changed = true;
        }
        // The packets and slides step every 150 ms or so.
        tx.wake_after(Duration::from_millis(150));
    }
    changed
}

/// `onBlur → setTouched(true)`: a field that had focus and lost it.
fn track_touched(app: &mut App) -> bool {
    let f = &app.onboarding.form;
    let now = [
        ("workspace", f.workspace.focused),
        ("taxLongTermDays", f.long_term_days.focused),
        ("database-passphrase", f.passphrase.focused),
        ("database-passphrase-confirm", f.confirm.focused),
        ("backendName", f.backend_name.focused),
        ("backendHost", f.backend_host.focused),
        ("backendPort", f.backend_port.focused),
        ("backendUrl", f.backend_url.focused),
        ("backendCertificate", f.certificate.focused),
        ("backendProxyHost", f.proxy_host.focused),
        ("backendProxyPort", f.proxy_port.focused),
        ("aiProviderName", f.ai_name.focused),
        ("aiBaseUrl", f.ai_url.focused),
    ]
    .into_iter()
    .find(|(_, on)| *on)
    .map(|(k, _)| k);
    let ui = &mut app.onboarding;
    if ui.focused != now {
        if let Some(prev) = ui.focused {
            ui.touched.insert(prev);
        }
        ui.focused = now;
        return true;
    }
    false
}

/// ConnectionsFields' `.onion` effect: a custom `.onion` endpoint routes through the local Tor
/// proxy (filled in once), plain TCP for Electrum, and `http://` for a bare Esplora host.
fn onion_effect(app: &mut App) -> bool {
    let f = &mut app.onboarding.form;
    let custom = f.backend_mode == Some(BackendMode::Custom);
    let electrum = custom && f.backend_kind == BackendKind::Electrum;
    if !custom || !model::is_onion(&f.effective_backend_url()) {
        return false;
    }
    let mut changed = false;
    if !f.use_proxy {
        f.use_proxy = true;
        changed = true;
    }
    if f.proxy_host.text.trim().is_empty() {
        model::set_text(&mut f.proxy_host, model::DEFAULT_TOR_PROXY_HOST);
        changed = true;
    }
    if f.proxy_port.text.trim().is_empty() {
        model::set_text(&mut f.proxy_port, model::DEFAULT_TOR_PROXY_PORT);
        changed = true;
    }
    if electrum
        && f.use_ssl
        && (f.backend_port.text == model::DEFAULT_ELECTRUM_SSL_PORT
            || f.backend_port.text.trim().is_empty())
    {
        f.use_ssl = false;
        model::set_text(&mut f.backend_port, model::DEFAULT_ELECTRUM_TCP_PORT);
        f.trust_ssl = false;
        model::set_text(&mut f.certificate, "");
        changed = true;
    }
    if !electrum {
        let n = model::normalize_onion_http(&f.backend_url.text);
        if n != f.backend_url.text {
            model::set_text(&mut f.backend_url, &n);
            changed = true;
        }
    }
    if changed {
        reset_test(app);
    }
    changed
}

/// Escape closes an open select or the language menu first.
pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if !shows(app) {
        return false;
    }
    if ev.key == rune::Key::Escape && (app.onboarding.select.is_some() || app.onboarding.lang_open)
    {
        app.onboarding.select = None;
        app.onboarding.lang_open = false;
        return true;
    }
    false
}

/// The whole window while Welcome is up (`WindowFrame` + `Onboarding`).
pub fn root(app: &App, cx: &mut Cx<App>, layer: &mut Vec<El>) -> El {
    view::root(app, cx, layer)
}

/// Headless: what Welcome reads before its first frame (the built-in servers in setup).
pub fn prime(app: &mut App) {
    if app.onboarding.flow == Flow::Setup && app.onboarding.defaults.is_none() {
        if !app.onboarding.defaults_asked {
            request_defaults(app);
        }
        pump_until(
            app,
            |a| a.onboarding.defaults.is_some(),
            Duration::from_secs(20),
        );
    }
}

/// Whether this is the book's very first sync (the sync card's copy).
pub fn is_first_sync(app: &App) -> bool {
    model::book_identity_key(app.onboarding.identity.as_ref()).is_some_and(|k| {
        !app.onboarding
            .first_sync_done
            .get(&k)
            .copied()
            .unwrap_or(false)
    })
}

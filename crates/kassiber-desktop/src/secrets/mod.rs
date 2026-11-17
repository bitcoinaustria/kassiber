//! Secrets: the OS credential store, Touch ID, the AI-key bridge and the operator helper — the
//! Tauri shell's `secret_store.rs`, the supervisor's secret-store bridge, the `touch_id_*`
//! commands and `--operator-native-auth`, with the web's Touch ID flows (SettingsScreen.tsx,
//! AppShell.tsx's `unlockApp`/`unlockWithTouchId`, LockScreen.tsx, unlockNotifications.ts).
//!
//! - [`store`]: the store trait, the native store (same crates, versions and item attributes as
//!   Tauri, so its items keep working), the AI-key store policy.
//! - [`biometric`] + [`touch`]: LocalAuthentication and Touch ID credentials.
//! - [`desktop`]: the per-data-root commands and the CLI's stale guard.
//! - [`bridge`]: what the supervisor ([`crate::daemon`]) adds to AI requests and answers on the
//!   daemon's behalf.
//! - [`operator`]: the broker's helper, run from `main` before any window.
//!
//! Keychain work blocks (and Touch ID shows a system sheet), so every command runs on a worker
//! thread; the UI polls its outcome. Passphrases travel in `Zeroizing` buffers, are never put in
//! logs, toasts or scene names, and are not kept after the step that needed them.
//!
//! Signed-build-only: the protected (biometry-bound) Keychain items, operator Touch ID and the
//! production AI-key default need the Developer ID build with the provisioning profile and the
//! `6Q4R2C3GJK.at.bitcoinaustria.kassiber` Keychain group (docs/reference/macos-release.md).
//! Other builds take the paths the Tauri preview builds take: the app-gated login-Keychain copy
//! for desktop Touch ID, "unavailable" for operator Touch ID, SQLCipher for new AI keys.

pub mod biometric;
pub mod bridge;
pub mod desktop;
pub mod operator;
pub mod store;
pub mod touch;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tr};
use biometric::Biometric;
use store::SecretStore;
use touch::{Keychain, TouchIdPassphraseStatus, TouchIdProtection};

// ---- The platform ---------------------------------------------------------------------------

/// The stores the commands run against: the OS ones, or a scripted fake (scenes).
pub struct Backend {
    pub store: Arc<dyn SecretStore>,
    pub bio: Arc<dyn Biometric>,
    /// A Mac (Touch ID exists as a feature here at all).
    pub macos: bool,
}

impl Backend {
    fn keychain(&self) -> Keychain<'_> {
        Keychain {
            store: &*self.store,
            bio: &*self.bio,
            macos: self.macos,
        }
    }

    fn native() -> Backend {
        // Headless runs (snapshots, scripts, the eval tools) never reach the OS Keychain or
        // LocalAuthentication: without a scene's fake they see an empty store and no Touch ID.
        if rune::tooling::headless_run().is_some() {
            let mem = store::MemoryStore::new(store::SecretStoreAvailability::Unavailable {
                reason: "headless".into(),
            });
            return Backend {
                store: Arc::new(mem),
                bio: Arc::new(biometric::Unsupported),
                macos: cfg!(target_os = "macos"),
            };
        }
        #[cfg(target_os = "macos")]
        let bio: Arc<dyn Biometric> = Arc::new(biometric::Native);
        #[cfg(not(target_os = "macos"))]
        let bio: Arc<dyn Biometric> = Arc::new(biometric::Unsupported);
        Backend {
            store: Arc::new(store::NativeSecretStore),
            bio,
            macos: cfg!(target_os = "macos"),
        }
    }
}

static OVERRIDE: OnceLock<Mutex<Option<Arc<Backend>>>> = OnceLock::new();

fn backend() -> Arc<Backend> {
    OVERRIDE
        .get_or_init(Default::default)
        .lock()
        .expect("backend")
        .clone()
        .unwrap_or_else(|| Arc::new(Backend::native()))
}

/// Scenes: run the Touch ID commands against `b` instead of the OS (process-wide).
pub fn use_backend(b: Backend) {
    *OVERRIDE
        .get_or_init(Default::default)
        .lock()
        .expect("backend") = Some(Arc::new(b));
}

/// `canUseTouchIdPassphraseUnlock`: the desktop app on a Mac.
pub fn touch_id_supported() -> bool {
    OVERRIDE
        .get_or_init(Default::default)
        .lock()
        .expect("backend")
        .as_ref()
        .map(|b| b.macos)
        .unwrap_or(cfg!(target_os = "macos"))
}

// ---- State ----------------------------------------------------------------------------------

/// `app.secrets`.
pub struct Ui {
    /// The new passphrase of each `ui.secrets.change_passphrase` in flight, oldest first: the
    /// Touch ID update stores the one that encrypted the database, not the editable field.
    pub changing_to: std::collections::VecDeque<Zeroizing<String>>,
    /// The last Touch ID status (None: not asked, not encrypted, or not a Mac).
    pub status: Option<TouchIdPassphraseStatus>,
    pub status_pending: bool,
    /// The Settings enrollment runs (unlock, save, and on failure the cleanup).
    pub enroll_pending: bool,
    /// A Touch ID unlock's daemon request is out (`app.unlock`); what follows it.
    inflight: Option<Inflight>,
    /// A passphrase unlock that also saves the passphrase for Touch ID, once it succeeds.
    remember: Option<Remember>,
    /// The launch lock may still start Touch ID by itself (`touchIdAutoPromptPending`).
    pub auto_prompt_pending: bool,
    /// The app-level conditions (`touch-id.off`, `unlock.credential-remains`).
    pub touch_off: Option<String>,
    pub credential_remains: Vec<String>,
    /// "Forget all" collects the Touch ID removal's error before the CLI's.
    forget_all_errors: Option<Vec<String>>,
    /// (locked, settings open, encrypted) as last seen: status reads follow their changes.
    seen: Option<(bool, bool, bool)>,
    outbox: Arc<Mutex<Vec<Outcome>>>,
    running: usize,
}

impl Default for Ui {
    fn default() -> Self {
        Ui {
            changing_to: Default::default(),
            status: None,
            status_pending: false,
            enroll_pending: false,
            inflight: None,
            remember: None,
            auto_prompt_pending: true,
            touch_off: None,
            credential_remains: Vec::new(),
            forget_all_errors: None,
            seen: None,
            outbox: Arc::default(),
            running: 0,
        }
    }
}

struct Inflight {
    /// The passphrase came from the pre-migration account: re-save it under the current one.
    migrate: Option<(String, String, bool, Zeroizing<String>)>,
}

struct Remember {
    passphrase: Zeroizing<String>,
    /// The lock screen's "Use Touch ID next time" (None when it was not offered).
    remember: Option<bool>,
    stale_generation: Option<String>,
    data_root: Option<String>,
}

#[derive(Clone)]
enum StoreOrigin {
    Enroll,
    LockRemember(Option<bool>),
    ChangePassphrase,
}

#[derive(Clone)]
enum ForgetOrigin {
    /// Settings → Forget.
    Settings,
    /// Settings → Forget all saved unlock methods (the CLI's follows).
    ForgetAll,
    /// Cleanup after a failure; then the status is read again and `then` runs.
    Cleanup(Then),
}

#[derive(Clone)]
enum Then {
    Nothing,
    /// The enrollment failed with this text.
    EnrollError(String),
}

enum Job {
    Status,
    Store {
        passphrase: Zeroizing<String>,
        stale_generation: Option<String>,
        origin: StoreOrigin,
    },
    Forget {
        origin: ForgetOrigin,
    },
    Unlock,
    Migrate {
        account: String,
        legacy: String,
        cli_owns_legacy: bool,
        passphrase: Zeroizing<String>,
    },
}

enum Outcome {
    Status(Result<TouchIdPassphraseStatus, String>),
    Store(StoreOrigin, Result<TouchIdPassphraseStatus, String>),
    Forget(ForgetOrigin, Result<TouchIdPassphraseStatus, String>),
    Unlock(Result<desktop::UnlockFetch, String>),
    Migrate(Result<(), String>),
}

/// The data root the commands are for: a book waiting to open on the lock screen, else the
/// daemon's (`touchIdDataRoot`).
fn data_root(app: &App) -> Option<String> {
    app.lock
        .pending_project
        .as_ref()
        .and_then(|p| p["data_root"].as_str())
        .filter(|r| !r.trim().is_empty())
        .map(str::to_owned)
}

fn spawn(app: &mut App, job: Job) {
    spawn_for(app, data_root(app), job);
}

fn spawn_for(app: &mut App, root: Option<String>, job: Job) {
    let active: Option<PathBuf> = app.daemon.data_root().map(|p| p.to_path_buf());
    let outbox = app.secrets.outbox.clone();
    app.secrets.running += 1;
    let work = move || {
        let b = backend();
        let kc = b.keychain();
        let scope = desktop::scope_for_data_root(root.as_deref(), active.as_deref());
        let outcome = match job {
            Job::Status => Outcome::Status(desktop::status_command(&kc, &scope)),
            Job::Store {
                passphrase,
                stale_generation,
                origin,
            } => Outcome::Store(
                origin,
                desktop::store_command(&kc, &scope, &passphrase, stale_generation.as_deref()),
            ),
            Job::Forget { origin } => Outcome::Forget(origin, desktop::forget_command(&kc, &scope)),
            Job::Unlock => Outcome::Unlock(desktop::unlock_fetch(&kc, &scope)),
            Job::Migrate {
                account,
                legacy,
                cli_owns_legacy,
                passphrase,
            } => Outcome::Migrate(desktop::unlock_follow_migration(
                &kc,
                &account,
                &legacy,
                cli_owns_legacy,
                &passphrase,
            )),
        };
        outbox.lock().expect("outbox").push(outcome);
    };
    if std::thread::Builder::new()
        .name("kassiber-secrets".into())
        .spawn(work)
        .is_err()
    {
        app.secrets.running -= 1;
    }
}

// ---- Per frame ------------------------------------------------------------------------------

fn encrypted(app: &App) -> bool {
    app.encrypted || crate::settings::security::encrypted(app)
}

/// Applies finished commands, reads the status when a surface that shows it appears, and starts
/// the launch lock's Touch ID prompt.
pub fn poll(app: &mut App, tx: &mut rune::Tx) -> bool {
    let mut changed = apply_outcomes(app);
    let now = (app.locked, app.shell.settings_open, encrypted(app));
    if app.secrets.seen != Some(now) {
        let before = app.secrets.seen.replace(now);
        let (locked, open, enc) = now;
        let became = |was: Option<bool>, is: bool| is && was != Some(true);
        if became(before.map(|b| b.0), locked)
            || became(before.map(|b| b.1), open)
            || (before.is_some_and(|b| b.2 != enc) && (locked || open))
        {
            refresh_status(app);
            changed = true;
        }
    }
    // SettingsScreen: an "Offer Touch ID" preference without a saved passphrase turns off.
    if app.shell.settings_open
        && app.settings.prefs.touch_id_unlock
        && app.secrets.status.as_ref().is_some_and(|s| !s.configured)
    {
        app.settings.prefs.touch_id_unlock = false;
        changed = true;
    }
    changed |= auto_prompt(app);
    if app.secrets.running > 0 {
        tx.wake_after(Duration::from_millis(50));
    }
    changed
}

fn apply_outcomes(app: &mut App) -> bool {
    let done: Vec<Outcome> = std::mem::take(&mut *app.secrets.outbox.lock().expect("outbox"));
    let any = !done.is_empty();
    for o in done {
        app.secrets.running = app.secrets.running.saturating_sub(1);
        apply(app, o);
    }
    any
}

/// Scenes: waits for the running commands and applies them (up to `timeout`).
pub fn settle(app: &mut App, timeout: Duration) {
    let until = std::time::Instant::now() + timeout;
    while app.secrets.running > 0 && std::time::Instant::now() < until {
        if !apply_outcomes(app) {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    apply_outcomes(app);
}

fn apply(app: &mut App, o: Outcome) {
    match o {
        Outcome::Status(r) => {
            app.secrets.status_pending = false;
            app.secrets.status = Some(r.unwrap_or_else(failed_status));
        }
        Outcome::Store(origin, r) => stored(app, origin, r),
        Outcome::Forget(origin, r) => forgotten(app, origin, r),
        Outcome::Unlock(r) => fetched(app, r),
        Outcome::Migrate(r) => {
            if let Err(e) = r {
                // As the Tauri shell: reported, never fatal (the books are already open).
                eprintln!("kassiber: {e}");
            }
        }
    }
}

/// A status read that failed (`refreshTouchIdStatus`'s catch).
fn failed_status(reason: String) -> TouchIdPassphraseStatus {
    TouchIdPassphraseStatus {
        platform: store::SecretStorePlatform::Macos,
        available: false,
        configured: false,
        stale: false,
        protection: None,
        reason: Some(reason),
        stale_generation: None,
    }
}

/// `refreshTouchIdStatus`: nothing to ask unless the books are encrypted on a Mac.
pub fn refresh_status(app: &mut App) {
    if !encrypted(app) || !touch_id_supported() {
        app.secrets.status = None;
        app.secrets.status_pending = false;
        return;
    }
    app.secrets.status_pending = true;
    spawn(app, Job::Status);
}

// ---- Conditions -------------------------------------------------------------------------------

/// `recordTouchIdOff`.
fn record_touch_off(app: &mut App, reason: String) {
    app.secrets.touch_off = Some(reason);
    app.toast(
        ToastTone::Warning,
        &t("chrome:notifications.condition.touchIdOff.title"),
        None,
        Some("touch-id.off"),
    );
}

fn resolve_touch_off(app: &mut App) {
    app.secrets.touch_off = None;
}

/// `recordUnlockCredentialRemains`.
fn record_credential_remains(app: &mut App, reasons: Vec<String>) {
    app.secrets.credential_remains = reasons;
    app.toast(
        ToastTone::Warning,
        &t("chrome:notifications.condition.unlockCredentialRemains.title"),
        None,
        Some("unlock.credential-remains"),
    );
}

/// The notification center's app-level conditions: (code, title, body, details, action label).
pub fn conditions(app: &App) -> Vec<(&'static str, String, String, Vec<String>, String)> {
    let mut out = Vec::new();
    if let Some(reason) = &app.secrets.touch_off {
        out.push((
            "touch-id.off",
            t("chrome:notifications.condition.touchIdOff.title"),
            t("chrome:notifications.condition.touchIdOff.body"),
            vec![safe_message(reason)],
            t("chrome:notifications.condition.touchIdOff.action"),
        ));
    }
    if !app.secrets.credential_remains.is_empty() {
        out.push((
            "unlock.credential-remains",
            t("chrome:notifications.condition.unlockCredentialRemains.title"),
            t("chrome:notifications.condition.unlockCredentialRemains.body"),
            app.secrets
                .credential_remains
                .iter()
                .map(|r| safe_message(r))
                .collect(),
            t("chrome:notifications.action.openSecurity"),
        ));
    }
    out
}

/// The condition's one action: Settings → Security, at the Touch ID row.
pub fn open_security(app: &mut App) {
    crate::settings::open_section(app, "security#biometricUnlock");
}

/// `safeMessage`: details pass the secret floor and stay short.
fn safe_message(text: &str) -> String {
    let s = crate::daemon::redact(text.trim());
    if s.chars().count() > 240 {
        format!("{}…", s.chars().take(239).collect::<String>())
    } else {
        s
    }
}

// ---- Settings → Security --------------------------------------------------------------------

/// Settings' Refresh button.
pub fn settings_refresh(app: &mut App) {
    refresh_status(app);
}

/// Settings' Forget button (`forgetTouchIdUnlock`): no confirmation, as the web.
pub fn settings_forget(app: &mut App) {
    spawn(
        app,
        Job::Forget {
            origin: ForgetOrigin::Settings,
        },
    );
}

/// The enrollment page's submit (`onEnrollTouchId`), after the field check: the daemon verifies
/// the passphrase first.
pub fn enroll_submit(app: &mut App) {
    app.secrets.enroll_pending = true;
    app.settings.dialog.touch_error = None;
    let mut args =
        json!({"auth_response": {"passphrase_secret": app.settings.dialog.touch_pass.text}});
    if app.daemon.launch.explicit_root {
        args["require_existing_project"] = json!(true);
    }
    crate::settings::send(app, "daemon.unlock", args, "settings.touch_unlock");
}

/// The enrollment's daemon answer: save the verified passphrase for Touch ID.
pub fn enroll_unlocked(app: &mut App, r: &Reply) {
    if r.error.is_some() || r.kind != "daemon.unlock" {
        enroll_failed(app, t("settings:touchIdEnroll.errorUnlock"));
        return;
    }
    let passphrase = Zeroizing::new(app.settings.dialog.touch_pass.text.clone());
    let stale_generation = app
        .secrets
        .status
        .as_ref()
        .and_then(|s| s.stale_generation.clone());
    spawn(
        app,
        Job::Store {
            passphrase,
            stale_generation,
            origin: StoreOrigin::Enroll,
        },
    );
}

/// The catch of `onEnrollTouchId`: Touch ID off, the partial enrollment removed, status re-read.
fn enroll_failed(app: &mut App, error: String) {
    app.settings.prefs.touch_id_unlock = false;
    spawn(
        app,
        Job::Forget {
            origin: ForgetOrigin::Cleanup(Then::EnrollError(error)),
        },
    );
}

/// After `ui.secrets.change_passphrase` succeeded: an enrolled Touch ID follows the new
/// passphrase (`shouldRefreshTouchIdPassphrase`).
pub fn passphrase_changed(app: &mut App, r: &Reply) {
    // The passphrase this change submitted (taken in any case, so the queue stays in step).
    let Some(passphrase) = app.secrets.changing_to.pop_front() else {
        return;
    };
    if !(touch_id_supported() && app.secrets.status.as_ref().is_some_and(|s| s.configured)) {
        return;
    }
    let stale_generation = r.data["desktop_biometric_stale_generation"]
        .as_str()
        .map(str::to_owned);
    spawn(
        app,
        Job::Store {
            passphrase,
            stale_generation,
            origin: StoreOrigin::ChangePassphrase,
        },
    );
}

/// "Forget all saved unlock methods", confirmed: the desktop Touch ID entry first, then the
/// CLI's remembered unlock (`forgetAllSavedUnlockMethods`).
pub fn forget_all(app: &mut App) {
    app.secrets.forget_all_errors = Some(Vec::new());
    if touch_id_supported() {
        spawn(
            app,
            Job::Forget {
                origin: ForgetOrigin::ForgetAll,
            },
        );
    } else {
        forget_cli(app);
    }
}

fn forget_cli(app: &mut App) {
    crate::settings::send(
        app,
        "ui.secrets.forget_cli_unlock",
        json!({}),
        "settings.forget_cli",
    );
}

/// The CLI's answer to "Forget all": every collected error, or the success note.
pub fn forget_all_done(app: &mut App, r: &Reply) {
    let mut errors = app.secrets.forget_all_errors.take().unwrap_or_default();
    match &r.error {
        Some(e) => errors.push(e.clone()),
        None if r.data["cli_credential_deleted"].as_bool() == Some(false)
            || r.data["legacy_credential_deleted"].as_bool() == Some(false) =>
        {
            errors.push(t("settings:touchId.forgetAllCredentialWarning"))
        }
        None => {}
    }
    app.settings.prefs.touch_id_unlock = false;
    refresh_status(app);
    if !errors.is_empty() {
        record_credential_remains(app, errors);
        return;
    }
    // Nothing is saved any more: neither a leftover nor a Touch ID waiting to come back.
    app.secrets.credential_remains.clear();
    resolve_touch_off(app);
    app.toast(
        ToastTone::Success,
        &t("settings:touchId.forgetAllTitle"),
        Some(t("settings:touchId.forgetAllBody")),
        None,
    );
}

fn stored(app: &mut App, origin: StoreOrigin, r: Result<TouchIdPassphraseStatus, String>) {
    match origin {
        StoreOrigin::Enroll => {
            let status = match r {
                Ok(s) => s,
                Err(e) => {
                    return enroll_failed(
                        app,
                        if e.is_empty() {
                            t("settings:touchIdEnroll.errorGeneric")
                        } else {
                            e
                        },
                    );
                }
            };
            let configured = status.configured;
            let reason = status.reason.clone();
            app.secrets.status = Some(status);
            if !configured {
                return enroll_failed(app, not_set_up(reason));
            }
            app.settings.prefs.touch_id_unlock = true;
            app.secrets.enroll_pending = false;
            app.settings.dialog.touch_pass = rune::TextEdit::new("");
            app.settings.dialog.touch_open = false;
            app.settings.sub = None;
            resolve_touch_off(app);
            app.toast(
                ToastTone::Success,
                &t("settings:touchIdEnroll.notifyTitle"),
                Some(t("settings:touchIdEnroll.notifyBody")),
                None,
            );
        }
        StoreOrigin::ChangePassphrase => {
            let failure = match r {
                Ok(s) if s.configured => {
                    app.secrets.status = Some(s);
                    None
                }
                Ok(s) => {
                    let reason = s.reason.clone();
                    app.secrets.status = Some(s);
                    Some(not_set_up(reason))
                }
                Err(e) => Some(if e.is_empty() {
                    t("settings:touchId.disabledBody")
                } else {
                    e
                }),
            };
            if let Some(f) = failure {
                app.settings.prefs.touch_id_unlock = false;
                spawn(
                    app,
                    Job::Forget {
                        origin: ForgetOrigin::Cleanup(Then::Nothing),
                    },
                );
                record_touch_off(app, f);
            }
        }
        StoreOrigin::LockRemember(remember) => match r {
            Ok(s) => {
                let (configured, reason) = (s.configured, s.reason.clone());
                app.secrets.status = Some(s);
                if !configured {
                    app.settings.prefs.touch_id_unlock = false;
                    let why = match reason {
                        Some(reason) => {
                            tr("chrome:lock.touchIdNotSavedReason", &[("reason", &reason)])
                        }
                        None => t("chrome:lock.touchIdNotSavedKeychain"),
                    };
                    record_touch_off(app, why);
                    return;
                }
                resolve_touch_off(app);
                if remember == Some(true) {
                    app.settings.prefs.touch_id_unlock = true;
                }
            }
            Err(e) => {
                record_touch_off(
                    app,
                    if e.is_empty() {
                        t("chrome:lock.touchIdNotAcceptedKeychain")
                    } else {
                        e
                    },
                );
                refresh_status(app);
            }
        },
    }
}

fn not_set_up(reason: Option<String>) -> String {
    match reason {
        Some(reason) => tr(
            "settings:touchIdEnroll.errorNotSetUpReason",
            &[("reason", &reason)],
        ),
        None => t("settings:touchIdEnroll.errorNotSetUp"),
    }
}

fn forgotten(app: &mut App, origin: ForgetOrigin, r: Result<TouchIdPassphraseStatus, String>) {
    match origin {
        ForgetOrigin::Settings => match r {
            Ok(s) => {
                app.secrets.status = Some(s);
                app.settings.prefs.touch_id_unlock = false;
                app.secrets.credential_remains.clear();
            }
            Err(e) => record_credential_remains(
                app,
                vec![if e.is_empty() {
                    t("settings:touchId.removeFailedBody")
                } else {
                    e
                }],
            ),
        },
        ForgetOrigin::ForgetAll => {
            if let Err(e) = r {
                app.secrets
                    .forget_all_errors
                    .get_or_insert_with(Vec::new)
                    .push(e);
            }
            forget_cli(app);
        }
        ForgetOrigin::Cleanup(then) => {
            refresh_status(app);
            if let Then::EnrollError(error) = then {
                app.secrets.enroll_pending = false;
                app.settings.dialog.touch_error = Some(error);
            }
        }
    }
}

// ---- The lock screen ------------------------------------------------------------------------

/// `canEnrollTouchIdPassphrase`: the "Use Touch ID next time" switch is offered.
pub fn can_enroll(app: &App) -> bool {
    let s = app.secrets.status.as_ref();
    touch_id_supported()
        && app.passphrase_required()
        && s.is_none_or(|s| s.available)
        && (!app.settings.prefs.touch_id_unlock || s.is_some_and(|s| s.stale))
}

/// The switch's value: as set, else the default (`touchIdEnabled && canEnrollTouchId`, which
/// is false whenever the switch shows unless the saved passphrase went stale).
pub fn enroll_switch(app: &App) -> bool {
    can_enroll(app)
        && app
            .lock
            .enroll_touch_id
            .unwrap_or(app.settings.prefs.touch_id_unlock)
}

/// `canUseTouchId`: the Touch ID button shows.
pub fn can_use(app: &App) -> bool {
    app.settings.prefs.touch_id_unlock
        && touch_id_supported()
        && app.passphrase_required()
        && app
            .secrets
            .status
            .as_ref()
            .is_some_and(|s| s.available && s.configured)
}

/// Why Touch ID is not on offer although the user turned it on (`touchIdNote`).
pub fn lock_note(app: &App) -> Option<String> {
    if !(app.settings.prefs.touch_id_unlock && touch_id_supported() && app.passphrase_required()) {
        return None;
    }
    let s = app.secrets.status.as_ref()?;
    if !s.available {
        Some(match &s.reason {
            Some(r) => tr("chrome:lock.touchIdUnavailableReason", &[("reason", r)]),
            None => t("chrome:lock.touchIdUnavailable"),
        })
    } else if !s.configured {
        Some(match &s.reason {
            Some(r) => tr("chrome:lock.touchIdNotSetUpReason", &[("reason", r)]),
            None => t("chrome:lock.touchIdNotSetUp"),
        })
    } else {
        None
    }
}

/// The passphrase submit, just before `App::submit_unlock`: keeps the passphrase for Touch ID
/// when the unlock should also save it (`shouldStoreTouchIdPassphrase`).
pub fn before_passphrase_unlock(app: &mut App) {
    app.secrets.remember = None;
    if app.lock.submitting
        || app.lock.touch_submitting
        || !app.passphrase_required()
        || app.lock.passphrase.text.is_empty()
    {
        return;
    }
    let remember = can_enroll(app).then(|| enroll_switch(app));
    let configured = app.secrets.status.as_ref().is_some_and(|s| s.configured);
    let store =
        touch_id_supported() && remember != Some(false) && (remember == Some(true) || configured);
    if store {
        app.secrets.remember = Some(Remember {
            passphrase: Zeroizing::new(app.lock.passphrase.text.clone()),
            remember,
            stale_generation: app
                .secrets
                .status
                .as_ref()
                .and_then(|s| s.stale_generation.clone()),
            data_root: data_root(app),
        });
    }
}

/// "Unlock with Touch ID" (`submitTouchId` → `unlockWithTouchId`): read the saved passphrase
/// behind Touch ID, then unlock with it.
pub fn start_touch_unlock(app: &mut App) {
    if app.lock.touch_submitting || app.lock.submitting {
        return;
    }
    app.lock.error = None;
    app.lock.touch_submitting = true;
    app.lock.attempt += 1;
    app.session += 1;
    spawn(app, Job::Unlock);
}

fn fetched(app: &mut App, r: Result<desktop::UnlockFetch, String>) {
    if !app.locked || !app.lock.touch_submitting {
        // Unlocked another way meanwhile: the passphrase is dropped (and cleared).
        return;
    }
    match r {
        Err(_) => {
            // The native side refused (the sheet was cancelled, the Keychain said no): its text
            // is a raw Security framework message, so the user's language says it instead.
            app.lock.touch_submitting = false;
            app.lock.error = Some(t("chrome:lock.touchIdFailed"));
            app.lock.passphrase.focused = true;
        }
        Ok(desktop::UnlockFetch::Refused { code, .. }) => {
            app.lock.touch_submitting = false;
            if code == "touch_id_passphrase_not_found" {
                app.settings.prefs.touch_id_unlock = false;
                refresh_status(app);
            }
            app.lock.error = Some(t("chrome:lock.touchIdNotFound"));
            app.lock.passphrase.focused = true;
        }
        Ok(desktop::UnlockFetch::Passphrase {
            secret,
            migrated_legacy_account,
            account,
            cli_owns_legacy,
        }) => {
            let pending = app.lock.pending_project.clone();
            let project_id = pending
                .as_ref()
                .and_then(|p| p["id"].as_str())
                .map(str::to_owned);
            let require = pending.is_some() || app.daemon.launch.explicit_root;
            let (kind, args) = desktop::unlock_request(&secret, require, project_id.as_deref());
            app.secrets.inflight = Some(Inflight {
                migrate: migrated_legacy_account
                    .map(|legacy| (account, legacy, cli_owns_legacy, secret)),
            });
            app.send(kind, args, "app.unlock");
        }
    }
}

/// The lock screen's words for a refused Touch ID unlock (`unlockFailureKey("touch-id")`).
fn touch_failure(r: &Reply) -> String {
    match r.error_detail["code"].as_str() {
        Some("local_auth_denied") => t("chrome:lock.touchIdStale"),
        Some("touch_id_passphrase_not_found") => t("chrome:lock.touchIdNotFound"),
        Some("auth_required") | None => t("chrome:lock.touchIdFailed"),
        Some(_) => crate::app::unlock_failure(r),
    }
}

/// Starts Touch ID by itself on the launch lock, once its status says it can (LockScreen's
/// `autoTouchIdPrompt`), while the window has focus.
fn auto_prompt(app: &mut App) -> bool {
    if !app.secrets.auto_prompt_pending {
        return false;
    }
    if !app.locked {
        // Opened without a lock: nothing to prompt for at launch.
        if app.reads.contains_key("ui.overview.snapshot") {
            app.secrets.auto_prompt_pending = false;
        }
        return false;
    }
    if can_use(app) && app.settings.fx.window_focused != Some(false) && !app.lock.submitting {
        app.secrets.auto_prompt_pending = false;
        start_touch_unlock(app);
        return true;
    }
    false
}

/// Every reply; acts on the unlock and nothing else.
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if tag != "app.unlock" {
        return;
    }
    if let Some(inflight) = app.secrets.inflight.take() {
        app.lock.touch_submitting = false;
        if r.error.is_none() {
            app.secrets.auto_prompt_pending = false;
            if let Some((account, legacy, cli_owns_legacy, passphrase)) = inflight.migrate {
                spawn(
                    app,
                    Job::Migrate {
                        account,
                        legacy,
                        cli_owns_legacy,
                        passphrase,
                    },
                );
            }
            return;
        }
        match r.error_detail["code"].as_str() {
            Some("touch_id_passphrase_not_found") => {
                app.settings.prefs.touch_id_unlock = false;
                refresh_status(app);
            }
            Some("local_auth_denied") => {
                // The saved passphrase passed Touch ID but no longer opens these books: remove it
                // instead of offering a known-stale credential on every lock screen.
                app.settings.prefs.touch_id_unlock = false;
                spawn(
                    app,
                    Job::Forget {
                        origin: ForgetOrigin::Cleanup(Then::Nothing),
                    },
                );
            }
            _ => {}
        }
        app.lock.error = Some(touch_failure(r));
        return;
    }
    let remember = app.secrets.remember.take();
    if r.error.is_some() {
        return;
    }
    app.secrets.auto_prompt_pending = false;
    if let Some(rem) = remember {
        let Remember {
            passphrase,
            remember,
            stale_generation,
            data_root,
        } = rem;
        spawn_for(
            app,
            data_root,
            Job::Store {
                passphrase,
                stale_generation,
                origin: StoreOrigin::LockRemember(remember),
            },
        );
    }
}

// ---- Scenes ---------------------------------------------------------------------------------

/// A scripted Touch ID: `available` (else the system's reason), `entitled` (a production
/// build), an enrollment of `protection` already saved for the scene's data root.
fn fake(
    app: &mut App,
    available: Result<(), String>,
    entitled: bool,
    protection: Option<TouchIdProtection>,
) {
    let mem = store::MemoryStore::new(store::SecretStoreAvailability::Available {
        identity_strength: store::IdentityStrength::UnknownOrUnsigned,
    });
    let bio = biometric::Fake::new(available, entitled);
    let scope = desktop::scope_for_data_root(None, app.daemon.data_root());
    if let Some(p) = protection {
        let marker = p
            .marker_service()
            .unwrap_or(touch::LEGACY_SHARED_PASSPHRASE_SERVICE);
        let _ = mem.set(marker, &scope.account, b"1");
        // The scenes' saved passphrase is a placeholder; nothing unlocks with it.
        let _ = mem.set(
            touch::DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE,
            &scope.account,
            b"scene-placeholder",
        );
    }
    use_backend(Backend {
        store: Arc::new(mem),
        bio: Arc::new(bio),
        macos: true,
    });
}

/// The books as an encrypted one (the demo book is plaintext).
fn as_encrypted(app: &mut App) {
    app.encrypted = true;
    if let Some(r) = app.reads.get_mut("status") {
        r.data["database_encrypted"] = Value::Bool(true);
    } else {
        app.reads.insert(
            "status".into(),
            Reply {
                kind: "status".into(),
                data: json!({"database_encrypted": true}),
                error: None,
                error_detail: Value::Null,
                elapsed: Duration::ZERO,
            },
        );
    }
}

fn settings_scene(app: &mut App) {
    app.shell.settings_open = true;
    app.shell.settings_section = crate::settings::SECURITY;
    crate::settings::prime(app);
    // After Settings' own `status` read, which says the demo book is plaintext.
    as_encrypted(app);
    refresh_status(app);
    settle(app, Duration::from_secs(5));
    let now = (app.locked, app.shell.settings_open, encrypted(app));
    app.secrets.seen = Some(now);
}

fn lock_scene(app: &mut App, touch_pref: bool) {
    app.secrets.auto_prompt_pending = false;
    app.settings.prefs.touch_id_unlock = touch_pref;
    as_encrypted(app);
    app.lock_session(false);
    refresh_status(app);
    settle(app, Duration::from_secs(5));
    app.secrets.seen = Some((app.locked, app.shell.settings_open, encrypted(app)));
}

fn not_mac(app: &mut App) {
    use_backend(Backend {
        store: Arc::new(store::MemoryStore::new(
            store::SecretStoreAvailability::Unavailable {
                reason: "unsupported_platform".into(),
            },
        )),
        bio: Arc::new(biometric::Unsupported),
        macos: false,
    });
    let _ = app;
}

const NOT_ENROLLED_REASON: &str = "Biometry is not enrolled.";

/// Touch ID states in Settings → Security and on the lock screen (en; `--lang de` for German).
/// None of them touches the OS Keychain or shows a Touch ID sheet: they run against a fake.
pub const SCENES: &[crate::app::Scene] = &[
    // Not a Mac (or a browser build): "available in the macOS desktop app", buttons disabled.
    ("secrets-settings-unsupported", |a| {
        not_mac(a);
        settings_scene(a);
    }),
    // A Mac with Touch ID, nothing saved: Set up.
    ("secrets-settings-available", |a| {
        fake(a, Ok(()), true, None);
        settings_scene(a);
    }),
    // Saved in a production build (biometry-bound item), offered on the lock screen.
    ("secrets-settings-enrolled", |a| {
        fake(a, Ok(()), true, Some(TouchIdProtection::BiometryCurrentSet));
        a.settings.prefs.touch_id_unlock = true;
        settings_scene(a);
    }),
    // Saved by a preview build (the app-gated login-Keychain copy).
    ("secrets-settings-enrolled-preview", |a| {
        fake(
            a,
            Ok(()),
            false,
            Some(TouchIdProtection::ApplicationLocalAuthentication),
        );
        a.settings.prefs.touch_id_unlock = true;
        settings_scene(a);
    }),
    // Touch ID cannot be evaluated (no fingers enrolled): the system's reason.
    ("secrets-settings-failed", |a| {
        fake(a, Err(NOT_ENROLLED_REASON.into()), true, None);
        settings_scene(a);
    }),
    // The enrollment page after the Keychain refused the passphrase.
    ("secrets-settings-enroll-failed", |a| {
        fake(a, Ok(()), true, None);
        settings_scene(a);
        crate::settings::dialogs::open_touch_id(a);
        a.settings.dialog.touch_error = Some(tr(
            "settings:touchIdEnroll.errorNotSetUpReason",
            &[("reason", NOT_ENROLLED_REASON)],
        ));
    }),
    ("secrets-settings-unsupported-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        not_mac(a);
        settings_scene(a);
    }),
    ("secrets-settings-enrolled-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        fake(a, Ok(()), true, Some(TouchIdProtection::BiometryCurrentSet));
        a.settings.prefs.touch_id_unlock = true;
        settings_scene(a);
    }),
    ("secrets-settings-failed-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        fake(a, Err(NOT_ENROLLED_REASON.into()), true, None);
        settings_scene(a);
    }),
    // Lock screen, not a Mac: the passphrase alone.
    ("secrets-lock-unsupported", |a| {
        not_mac(a);
        lock_scene(a, false);
    }),
    // Lock screen, Touch ID available, nothing saved: "Use Touch ID next time".
    ("secrets-lock-available", |a| {
        fake(a, Ok(()), true, None);
        lock_scene(a, false);
    }),
    // Lock screen, enrolled: "Unlock with Touch ID".
    ("secrets-lock-enrolled", |a| {
        fake(a, Ok(()), true, Some(TouchIdProtection::BiometryCurrentSet));
        lock_scene(a, true);
    }),
    // Lock screen, Touch ID on but unavailable now: the note says why.
    ("secrets-lock-failed", |a| {
        fake(a, Err(NOT_ENROLLED_REASON.into()), true, None);
        lock_scene(a, true);
    }),
    // Lock screen after a refused Touch ID attempt.
    ("secrets-lock-refused", |a| {
        fake(a, Ok(()), true, Some(TouchIdProtection::BiometryCurrentSet));
        lock_scene(a, true);
        a.lock.attempt = 1;
        a.lock.error = Some(t("chrome:lock.touchIdFailed"));
    }),
    // Lock screen while the Touch ID sheet is up.
    ("secrets-lock-waiting", |a| {
        fake(a, Ok(()), true, Some(TouchIdProtection::BiometryCurrentSet));
        lock_scene(a, true);
        a.lock.touch_submitting = true;
        a.lock.attempt = 1;
    }),
    ("secrets-lock-available-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        fake(a, Ok(()), true, None);
        lock_scene(a, false);
    }),
    ("secrets-lock-enrolled-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        fake(a, Ok(()), true, Some(TouchIdProtection::BiometryCurrentSet));
        lock_scene(a, true);
    }),
    ("secrets-lock-failed-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        fake(a, Err(NOT_ENROLLED_REASON.into()), true, None);
        lock_scene(a, true);
    }),
    // The notification center with both Touch ID conditions raised.
    ("secrets-conditions", |a| {
        a.secrets.touch_off = Some(t("chrome:lock.touchIdNotSavedKeychain"));
        a.secrets.credential_remains = vec![t("settings:touchId.forgetAllCredentialWarning")];
        a.shell.toggle(crate::shell::Popup::Notifications);
    }),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_names_are_prefixed() {
        assert!(SCENES.iter().all(|(n, _)| n.starts_with("secrets-")));
    }

    #[test]
    fn safe_messages_are_bounded() {
        assert!(safe_message(&"x".repeat(500)).chars().count() <= 240);
    }
}

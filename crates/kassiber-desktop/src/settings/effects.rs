//! What the Settings preferences do outside the modal (the web's AppShell effects, lib/clipboard,
//! lib/appUpdate, lib/osNotifications):
//!
//! - auto-lock: an idle timer locks an encrypted workspace after `max(1, idle_minutes)` minutes
//!   without a press, key, wheel or touch (AppShell.tsx's idle effect);
//! - clipboard clearing: copies go through `copy`, which clears the clipboard 30 s later if it
//!   still holds the copy (`copyTextWithPolicy`);
//! - desktop notifications: new center results are offered to the OS through the web's
//!   eligibility rules (opt-in, unfocused window, dedupe, cooldown) with fixed texts;
//! - update checks: the choice is written to the owner-only preference file the CLI reads
//!   (`set_app_update_checks_enabled`, `<state root>/config/update-checks.json`), never checked
//!   from here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rune::Tx;
use serde_json::Value;

use crate::app::{App, Reply, ToastTone};
use crate::i18n::t;

/// `CLIPBOARD_CLEAR_DELAY_MS`.
pub const CLIPBOARD_CLEAR_DELAY: Duration = Duration::from_secs(30);

/// RAM-only state of these effects (`app.settings.fx`).
#[derive(Default)]
pub struct Effects {
    pub started: bool,
    /// App-clock time of the last activity the idle timer counts.
    pub idle_since: f64,
    /// `rune::last_input()` as of the last poll (a newer one is activity).
    pub seen_input: Option<rune::time::Instant>,
    /// The window's focus (None until the platform reports it: counts as focused).
    pub window_focused: Option<bool>,
    pub os: OsNotifications,
}

/// Whether the open workspace is encrypted (`identity.encrypted`, from `status`).
pub fn encrypted(app: &App) -> bool {
    app.read("status")
        .is_some_and(|s| s["database_encrypted"].as_bool() == Some(true))
}

/// Every frame, open or not: the first-frame reads and the idle lock.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    if !app.settings.fx.started {
        app.settings.fx.started = true;
        app.settings.fx.idle_since = app.clock;
        // The identity's encryption flag (a local read; the web has it from the open project).
        // Welcome reads nothing: no daemon starts on the default root before a book is chosen.
        if app.daemon.is_some() && !crate::onboarding::shows(app) {
            app.send("status", Value::Null, "settings.fx_status");
        }
        // The canonical consent file, failing closed (`syncAppUpdateChecksEnabled`).
        app.settings.prefs.update_checks = read_update_checks();
        changed = true;
    }
    // Platform input since the last frame (Rune's `last_input`: keys, presses, the wheel, and
    // also pointer moves, which the web's listener set does not count).
    let last = rune::last_input();
    if last.is_some() && last != app.settings.fx.seen_input {
        app.settings.fx.seen_input = last;
        app.settings.fx.idle_since = app.clock;
    }
    changed |= idle_lock(app, tx);
    changed |= crate::osleaks::poll(app, tx);
    changed
}

/// A key the app saw (scripts and keys no control consumed): activity.
pub fn activity(app: &mut App) {
    app.settings.fx.idle_since = app.clock;
}

/// AppShell's idle effect: armed only for an encrypted workspace with auto-lock on, while
/// unlocked; a lock (or turning it off) disarms it, and arming starts the countdown afresh.
fn idle_lock(app: &mut App, tx: &mut Tx) -> bool {
    let p = &app.settings.prefs;
    let armed = encrypted(app) && p.auto_lock && !app.locked;
    if !armed {
        app.settings.fx.idle_since = app.clock;
        return false;
    }
    let after = f64::from(p.idle_minutes.max(1)) * 60.0;
    let due = app.settings.fx.idle_since + after;
    if app.clock >= due {
        crate::shell::overlays::lock(app);
        app.settings.fx.idle_since = app.clock;
        return true;
    }
    tx.wake_after(Duration::from_secs_f64((due - app.clock).max(0.05)));
    false
}

/// `copyTextWithPolicy`: copy, and with "Clear clipboard after copy" on, clear it after 30 s if
/// it still holds this text. Every copy is also kept on this device and out of clipboard
/// histories (`crate::osleaks::clipboard`, which the web cannot ask for).
pub fn copy(app: &App, tx: &mut Tx, value: impl Into<String>) {
    crate::osleaks::clipboard::copy(app, tx, value);
}

// ---- Update checks ------------------------------------------------------------------------------

const UPDATE_PREFERENCE_SCHEMA: u64 = 1;
const MAX_PREFERENCE_BYTES: u64 = 1024;

/// The consent file, where the CLI and the Tauri shell keep it: `KASSIBER_UPDATE_PREFERENCE_FILE`
/// when set, else `<default state root>/config/update-checks.json` (`crate::settings::prefpath`).
/// Machine-wide: the demo has no copy of its own. None only in tooling runs without the
/// variable, which never touch the account's real state root.
pub fn update_preference_path() -> Option<PathBuf> {
    crate::settings::prefpath::path(crate::settings::prefpath::UPDATE_CHECKS, &[])
}

/// Whether the consent can be saved here (onboarding persists its answer whenever it can).
pub fn update_preference_writable() -> bool {
    update_preference_path().is_some()
}

fn env_disables_update_checks() -> bool {
    std::env::var("KASSIBER_DISABLE_UPDATE_CHECK")
        .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

/// `update_checks_enabled_at`: explicit consent only; anything unreadable is not consent.
pub fn read_update_checks() -> bool {
    if env_disables_update_checks() {
        return false;
    }
    update_preference_path().is_some_and(|p| update_checks_enabled_at(&p))
}

fn update_checks_enabled_at(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.file_type().is_file() || meta.len() > MAX_PREFERENCE_BYTES {
        return false;
    }
    let Ok(raw) = std::fs::read(path) else {
        return false;
    };
    let Ok(v) = serde_json::from_slice::<Value>(&raw) else {
        return false;
    };
    // `deny_unknown_fields`: exactly the two keys.
    v.as_object().is_some_and(|o| {
        o.len() == 2
            && o.get("schema_version").and_then(Value::as_u64) == Some(UPDATE_PREFERENCE_SCHEMA)
            && o.get("enabled") == Some(&Value::Bool(true))
    })
}

/// `write_update_checks_enabled_at`: owner-only folder and file, replaced atomically.
fn write_update_checks_at(path: &Path, enabled: bool) -> Result<(), String> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or("Update-check preference has no parent directory.")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("update-checks.json");
    let tmp = parent.join(format!(".{name}.{}.{nonce}.tmp", std::process::id()));
    // `UpdateCheckPreference { schema_version, enabled }` as serde writes the struct.
    let encoded =
        format!("{{\"schema_version\":{UPDATE_PREFERENCE_SCHEMA},\"enabled\":{enabled}}}")
            .into_bytes();
    let mut opts = std::fs::OpenOptions::new();
    opts.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp).map_err(|e| e.to_string())?;
    let written = f
        .write_all(&encoded)
        .and_then(|_| f.write_all(b"\n"))
        .and_then(|_| f.sync_all());
    drop(f);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// `setAppUpdateChecksEnabled`: persist the consent first, then show it; a failed write keeps
/// the old state and says so (`privacy.updatesSaveFailed*`). Nothing is checked.
pub fn set_update_checks(app: &mut App, enabled: bool) {
    let result = match update_preference_path() {
        Some(p) => write_update_checks_at(&p, enabled),
        None => Err("no preference file".into()),
    };
    match result {
        Ok(()) => app.settings.prefs.update_checks = enabled,
        Err(_) => {
            let title = t("settings:privacy.updatesSaveFailedTitle");
            app.toast(
                ToastTone::Error,
                &title,
                Some(t("settings:privacy.updatesSaveFailedBody")),
                None,
            );
        }
    }
}

// ---- Desktop (OS) notifications -----------------------------------------------------------------

/// `DEDUPE_WINDOW_MS`, `BURST_COOLDOWN_MS`.
const DEDUPE_WINDOW: f64 = 30.0 * 60.0;
const BURST_COOLDOWN: f64 = 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    NeedsAttention,
    JobFinished,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skip {
    Disabled,
    Focused,
    Duplicate,
    Cooldown,
}

/// RAM-only delivery record (`lastDeliveredAt`, `lastDeliveredByKey`).
#[derive(Default)]
pub struct OsNotifications {
    pub last_at: Option<f64>,
    pub by_key: HashMap<String, f64>,
    /// What was delivered (headless runs record instead of showing).
    pub delivered: Vec<&'static str>,
}

/// `osNotificationEligibility` for the categories this build raises (both allowlisted, both
/// new): opt-in, then an unfocused window (unknown counts as focused), dedupe, cooldown.
pub fn eligibility(
    os: &OsNotifications,
    enabled: bool,
    focused: Option<bool>,
    key: &str,
    now: f64,
) -> Result<(), Skip> {
    if !enabled {
        return Err(Skip::Disabled);
    }
    if focused != Some(false) {
        return Err(Skip::Focused);
    }
    if os
        .by_key
        .get(key)
        .is_some_and(|at| now - at < DEDUPE_WINDOW)
    {
        return Err(Skip::Duplicate);
    }
    if os.last_at.is_some_and(|at| now - at < BURST_COOLDOWN) {
        return Err(Skip::Cooldown);
    }
    Ok(())
}

/// `os_notifications.rs` `body`: the only texts an OS notification may carry.
pub fn body(category: Category, de: bool) -> &'static str {
    match (category, de) {
        (Category::NeedsAttention, false) => "Background work needs your attention.",
        (Category::JobFinished, false) => "Background work has finished.",
        (Category::NeedsAttention, true) => "Eine Hintergrundaufgabe braucht deine Aufmerksamkeit.",
        (Category::JobFinished, true) => "Eine Hintergrundaufgabe ist abgeschlossen.",
    }
}

/// The results the shell files in its center (book refresh, journal processing), offered to the
/// OS as `candidateForOccurrence` maps them: a failure needs attention, a finish is a job done.
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let (code, failed) = match tag {
        "app.freshness_run" => ("book-refresh", r.error.is_some()),
        "app.journals_process" => ("journals", r.error.is_some()),
        "settings.fx_status" => return,
        _ => return,
    };
    let category = if failed {
        Category::NeedsAttention
    } else {
        Category::JobFinished
    };
    let book = app.expected_scope().to_string();
    let key = if failed {
        format!("condition:{code}.failed:{book}")
    } else {
        format!("result:{code}.finished:{book}")
    };
    offer(app, category, &key);
}

fn offer(app: &mut App, category: Category, key: &str) {
    let now = app.clock;
    let fx = &app.settings.fx;
    if eligibility(
        &fx.os,
        app.settings.prefs.desktop_notifications,
        fx.window_focused,
        key,
        now,
    )
    .is_err()
    {
        return;
    }
    let text = body(category, app.prefs.lang == crate::i18n::Lang::De);
    let os = &mut app.settings.fx.os;
    os.last_at = Some(now);
    os.by_key.insert(key.to_owned(), now);
    os.delivered.push(text);
    deliver(text);
}

/// Shows the notification through the platform's API (`native::notify`). Headless runs never do.
fn deliver(text: &'static str) {
    crate::native::notify::show(text);
}

/// The window's activation, for the focus rule (wired by the root's `on_window_focus`).
#[allow(dead_code)]
pub fn window_focus(app: &mut App, focused: bool) {
    app.settings.fx.window_focused = Some(focused);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_follow_the_web_rules() {
        let mut os = OsNotifications::default();
        assert_eq!(
            eligibility(&os, false, Some(false), "a", 0.0),
            Err(Skip::Disabled)
        );
        assert_eq!(eligibility(&os, true, None, "a", 0.0), Err(Skip::Focused));
        assert_eq!(
            eligibility(&os, true, Some(true), "a", 0.0),
            Err(Skip::Focused)
        );
        assert_eq!(eligibility(&os, true, Some(false), "a", 0.0), Ok(()));
        os.last_at = Some(0.0);
        os.by_key.insert("a".into(), 0.0);
        assert_eq!(
            eligibility(&os, true, Some(false), "b", 30.0),
            Err(Skip::Cooldown)
        );
        assert_eq!(eligibility(&os, true, Some(false), "b", 61.0), Ok(()));
        assert_eq!(
            eligibility(&os, true, Some(false), "a", 61.0),
            Err(Skip::Duplicate)
        );
        assert_eq!(eligibility(&os, true, Some(false), "a", 1801.0), Ok(()));
    }

    #[test]
    fn update_consent_round_trips_and_fails_closed() {
        let dir = std::env::temp_dir().join(format!("kassiber-rune-upd-{}", std::process::id()));
        let path = dir.join("config").join("update-checks.json");
        assert!(!update_checks_enabled_at(&path));
        write_update_checks_at(&path, true).unwrap();
        assert!(update_checks_enabled_at(&path));
        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(raw, "{\"schema_version\":1,\"enabled\":true}\n");
        write_update_checks_at(&path, false).unwrap();
        assert!(!update_checks_enabled_at(&path));
        std::fs::write(&path, "{\"schema_version\":1,\"enabled\":true,\"x\":1}").unwrap();
        assert!(!update_checks_enabled_at(&path));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            write_update_checks_at(&path, true).unwrap();
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

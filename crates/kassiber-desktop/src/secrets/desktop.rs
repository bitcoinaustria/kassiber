//! The desktop's Touch ID commands (ui-tauri/src-tauri/src/lib.rs, `touch_id_*_command` and their
//! helpers): which Keychain account a data root uses, the managed-settings stale guard the CLI
//! writes when the passphrase changes outside the desktop, and the four commands the web calls
//! (`touchIdPassphraseStatus`, `storeTouchIdPassphrase`, `unlockTouchIdPassphrase`,
//! `forgetTouchIdPassphrase`). They block (Keychain, the Touch ID sheet), so the UI runs them on
//! a worker thread (`super::jobs`).

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::touch::{Keychain, TouchIdPassphraseStatus};

const DEFAULT_STATE_DIR: &str = ".kassiber";
const DEFAULT_PROJECTS_DIR: &str = "projects";
const DEFAULT_DATA_DIR: &str = "data";
const STATE_ROOT_MIGRATION_FILENAME: &str = ".state-root-migration.json";
const CLI_LEGACY_UNLOCK_QUARANTINED_SETTING: &str = "cli_legacy_unlock_quarantined";
const DESKTOP_BIOMETRIC_STALE_SETTING: &str = "desktop_biometric_stale";

/// The Keychain namespace of one data root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchIdScope {
    /// The canonical data-root path: the Keychain account.
    pub account: String,
    pub data_root: String,
    /// The account the same books had before the default state root moved (macOS).
    pub legacy_account: Option<String>,
}

pub fn scope_for_selected(selected: PathBuf) -> TouchIdScope {
    let normalized = fs::canonicalize(&selected).unwrap_or_else(|_| selected.clone());
    TouchIdScope {
        account: normalized.to_string_lossy().into_owned(),
        data_root: selected.to_string_lossy().into_owned(),
        legacy_account: migrated_legacy_touch_id_account(&selected),
    }
}

/// `touch_id_scope_for_data_root`: the explicit root, else the daemon's, else the default one.
/// The normalized path is the Keychain account; first-run roots that do not exist yet keep the
/// selected path.
pub fn scope_for_data_root(explicit: Option<&str>, active: Option<&Path>) -> TouchIdScope {
    let selected = if let Some(explicit) = explicit.filter(|v| !v.trim().is_empty()) {
        PathBuf::from(explicit)
    } else if let Some(active) = active {
        active.to_path_buf()
    } else {
        crate::daemon::default_state_data_root().unwrap_or_else(|| {
            PathBuf::from(DEFAULT_STATE_DIR)
                .join(DEFAULT_PROJECTS_DIR)
                .join("default")
                .join(DEFAULT_DATA_DIR)
        })
    };
    scope_for_selected(selected)
}

// ---- The managed-settings guard -------------------------------------------------------------

fn managed_settings_path(data_root: &str) -> PathBuf {
    let data_root = Path::new(data_root);
    let state_root = if data_root.file_name().and_then(|n| n.to_str()) == Some(DEFAULT_DATA_DIR) {
        data_root.parent().unwrap_or(data_root)
    } else {
        data_root
    };
    state_root.join("config").join("settings.json")
}

fn read_managed_settings(data_root: &str) -> Result<Option<Value>, String> {
    let raw = match fs::read_to_string(managed_settings_path(data_root)) {
        Ok(raw) => raw,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Managed settings could not be read: {e}")),
    };
    let value = serde_json::from_str::<Value>(&raw)
        .map_err(|e| format!("Managed settings JSON is invalid: {e}"))?;
    if !value.is_object() {
        return Err("Managed settings must contain a JSON object.".to_string());
    }
    Ok(Some(value))
}

fn stale_generation_from_settings(value: &Value) -> Option<String> {
    let raw = value.get(DESKTOP_BIOMETRIC_STALE_SETTING)?;
    if let Some(generation) = raw.as_str().filter(|v| !v.trim().is_empty()) {
        return Some(generation.to_string());
    }
    // Compatibility for short-lived development builds that wrote a boolean guard before
    // generation-bound compare-and-clear landed.
    raw.as_bool()
        .filter(|on| *on)
        .map(|_| "legacy-boolean-guard".to_string())
}

#[allow(dead_code)] // tests; the commands read both halves at once
pub fn desktop_biometric_stale_generation(data_root: &str) -> Result<Option<String>, String> {
    Ok(managed_unlock_state(data_root)?.1)
}

/// (the CLI owns the legacy shared item, the stale guard's generation).
pub fn managed_unlock_state(data_root: &str) -> Result<(bool, Option<String>), String> {
    let settings = read_managed_settings(data_root)?;
    let flag = |v: &Value, k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    let cli_owns_legacy = settings.as_ref().is_some_and(|v| {
        flag(v, "cli_remembered_unlock") || flag(v, CLI_LEGACY_UNLOCK_QUARANTINED_SETTING)
    });
    let stale_generation = settings.as_ref().and_then(stale_generation_from_settings);
    Ok((cli_owns_legacy, stale_generation))
}

pub fn status_with_managed_guard(
    kc: &Keychain,
    scope: &TouchIdScope,
) -> Result<TouchIdPassphraseStatus, String> {
    let (cli_owns_legacy, stale_generation) = managed_unlock_state(&scope.data_root)?;
    let mut status = kc.status(&scope.account, cli_owns_legacy);
    if !status.configured
        && !status.stale
        && let Some(legacy) = scope.legacy_account.as_deref()
    {
        let legacy_status = kc.status(legacy, cli_owns_legacy);
        if legacy_status.configured {
            status = legacy_status;
        }
    }
    let Some(generation) = stale_generation else {
        return Ok(status);
    };
    status.configured = false;
    status.stale = true;
    status.protection = None;
    status.stale_generation = Some(generation);
    Ok(status)
}

static LIFECYCLE: OnceLock<Mutex<()>> = OnceLock::new();

/// Serializes enrollment and removal (`touch_id_credential_lifecycle_lock`).
fn lifecycle_lock() -> Result<MutexGuard<'static, ()>, String> {
    LIFECYCLE
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "Touch ID credential lifecycle lock is poisoned.".to_string())
}

#[cfg(target_os = "macos")]
struct ManagedSettingsLock(fs::File);

#[cfg(target_os = "macos")]
impl Drop for ManagedSettingsLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // Closing the descriptor also releases the lock; a failed unlock is harmless.
        unsafe {
            flock(self.0.as_raw_fd(), LOCK_UN);
        }
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}
#[cfg(target_os = "macos")]
const LOCK_EX: i32 = 2;
#[cfg(target_os = "macos")]
const LOCK_UN: i32 = 8;

/// The CLI takes the same `settings.json.lock` before it writes the guard (macOS, as the Tauri
/// shell).
#[cfg(target_os = "macos")]
fn lock_managed_settings(settings_path: &Path) -> Result<ManagedSettingsLock, String> {
    use std::os::fd::AsRawFd;
    let lock_path = settings_path.with_file_name("settings.json.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| format!("Managed settings lock could not be opened: {e}"))?;
    if unsafe { flock(file.as_raw_fd(), LOCK_EX) } != 0 {
        return Err(format!(
            "Managed settings lock could not be acquired: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(ManagedSettingsLock(file))
}

#[cfg(not(target_os = "macos"))]
fn lock_managed_settings(_settings_path: &Path) -> Result<(), String> {
    Ok(())
}

fn write_managed_settings_payload(
    settings_path: &Path,
    parent: &Path,
    payload: &Value,
) -> Result<(), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("System clock could not create a settings nonce: {e}"))?
        .as_nanos();
    let temp_path = parent.join(format!(".settings.json.{}.{nonce}.tmp", std::process::id()));
    let encoded = serde_json::to_vec_pretty(payload)
        .map_err(|e| format!("Managed settings could not be encoded: {e}"))?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .map_err(|e| format!("Managed settings temporary file could not be opened: {e}"))?;
    let written = file
        .write_all(&encoded)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all());
    drop(file);
    if let Err(e) = written {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("Managed settings could not be written: {e}"));
    }
    #[cfg(target_os = "windows")]
    if settings_path.exists() {
        fs::remove_file(settings_path)
            .map_err(|e| format!("Managed settings could not be replaced: {e}"))?;
    }
    if let Err(e) = fs::rename(&temp_path, settings_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("Managed settings could not be replaced: {e}"));
    }
    Ok(())
}

/// Removes the stale guard, only when it still carries `expected` (Some) or unconditionally
/// (None). True when the guard is gone (or was never there).
fn update_stale_guard(data_root: &str, expected: Option<Option<&str>>) -> Result<bool, String> {
    let settings_path = managed_settings_path(data_root);
    if !settings_path.exists() {
        return Ok(matches!(expected, None | Some(None)));
    }
    let parent = settings_path
        .parent()
        .ok_or_else(|| "Managed settings path has no parent directory.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("Managed settings directory could not be created: {e}"))?;
    #[allow(clippy::let_unit_value)]
    let _settings_lock = lock_managed_settings(&settings_path)?;
    let mut payload = match fs::read_to_string(&settings_path) {
        Ok(raw) => serde_json::from_str::<Value>(&raw)
            .map_err(|e| format!("Managed settings JSON is invalid: {e}"))?,
        Err(e) if e.kind() == ErrorKind::NotFound => json!({}),
        Err(e) => return Err(format!("Managed settings could not be read: {e}")),
    };
    let current = stale_generation_from_settings(&payload);
    if let Some(expected) = expected
        && current.as_deref() != expected
    {
        return Ok(false);
    }
    if current.is_none() {
        return Ok(true);
    }
    payload
        .as_object_mut()
        .ok_or_else(|| "Managed settings must contain a JSON object.".to_string())?
        .remove(DESKTOP_BIOMETRIC_STALE_SETTING);
    write_managed_settings_payload(&settings_path, parent, &payload)?;
    Ok(true)
}

pub fn clear_stale_guard_if_matches(
    data_root: &str,
    expected: Option<&str>,
) -> Result<bool, String> {
    update_stale_guard(data_root, Some(expected))
}

pub fn clear_stale_guard(data_root: &str) -> Result<bool, String> {
    update_stale_guard(data_root, None)
}

// ---- The migrated default root's old account -----------------------------------------------

pub fn migrated_legacy_touch_id_account_for_platform(
    selected: &Path,
    platform: &str,
    home: &Path,
    xdg_data_home: Option<&Path>,
    local_app_data: Option<&Path>,
) -> Option<PathBuf> {
    let native = crate::daemon::native_state_root_for_platform(
        platform,
        Some(home),
        xdg_data_home,
        local_app_data,
    )?;
    let (selected, native) = match (fs::canonicalize(selected), fs::canonicalize(&native)) {
        (Ok(selected), Ok(native)) => (selected, native),
        _ => (selected.to_path_buf(), native),
    };
    let relative = selected.strip_prefix(&native).ok()?;
    let mut components = relative.components();
    if components.next()?.as_os_str() != DEFAULT_PROJECTS_DIR
        || !matches!(components.next()?, std::path::Component::Normal(_))
        || components.next()?.as_os_str() != DEFAULT_DATA_DIR
        || components.next().is_some()
    {
        return None;
    }
    let legacy = home.join(DEFAULT_STATE_DIR);
    let marker = native.join(STATE_ROOT_MIGRATION_FILENAME);
    if !fs::symlink_metadata(&marker)
        .ok()
        .is_some_and(|m| m.is_file() && !m.file_type().is_symlink())
    {
        return None;
    }
    let payload: Value = serde_json::from_slice(&fs::read(marker).ok()?).ok()?;
    if payload.get("schema_version").and_then(Value::as_u64) != Some(1)
        || payload
            .get("migrated_from_hidden_home")
            .and_then(Value::as_bool)
            != Some(true)
    {
        return None;
    }
    Some(legacy.join(relative))
}

fn migrated_legacy_touch_id_account(selected: &Path) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let home = crate::daemon::home_dir()?;
    migrated_legacy_touch_id_account_for_platform(selected, "macos", &home, None, None)
        .map(|p| p.to_string_lossy().into_owned())
}

// ---- The commands -----------------------------------------------------------------------------

/// `touch_id_passphrase_status_command`.
pub fn status_command(
    kc: &Keychain,
    scope: &TouchIdScope,
) -> Result<TouchIdPassphraseStatus, String> {
    status_with_managed_guard(kc, scope)
}

/// `touch_id_store_passphrase_command`: refuses when the stale guard moved since the status the
/// caller saw (a passphrase change elsewhere in between).
pub fn store_command(
    kc: &Keychain,
    scope: &TouchIdScope,
    passphrase: &str,
    stale_generation: Option<&str>,
) -> Result<TouchIdPassphraseStatus, String> {
    let expected = stale_generation.filter(|v| !v.trim().is_empty());
    let _lifecycle = lifecycle_lock()?;
    if managed_unlock_state(&scope.data_root)?.1.as_deref() != expected {
        return Err("Touch ID enrollment state changed; refresh status and verify the current passphrase again.".to_string());
    }
    kc.store_passphrase(&scope.account, passphrase)?;
    clear_stale_guard_if_matches(&scope.data_root, expected)?;
    status_with_managed_guard(kc, scope)
}

/// `touch_id_forget_passphrase_command`.
pub fn forget_command(
    kc: &Keychain,
    scope: &TouchIdScope,
) -> Result<TouchIdPassphraseStatus, String> {
    let _lifecycle = lifecycle_lock()?;
    let (cli_owns_legacy, _) = managed_unlock_state(&scope.data_root)?;
    kc.delete_passphrase(&scope.account, cli_owns_legacy)?;
    if let Some(legacy) = scope.legacy_account.as_deref() {
        kc.delete_passphrase(legacy, cli_owns_legacy)?;
    }
    clear_stale_guard(&scope.data_root)?;
    status_with_managed_guard(kc, scope)
}

/// The first half of `touch_id_unlock_passphrase_command`: the saved passphrase (after Touch ID),
/// or the error envelope the web gets instead.
pub enum UnlockFetch {
    Passphrase {
        secret: Zeroizing<String>,
        /// Read from the pre-migration account: re-saved under the current one after the unlock.
        migrated_legacy_account: Option<String>,
        account: String,
        cli_owns_legacy: bool,
    },
    /// The Tauri command's error envelope; the lock screen words it by `code`.
    #[allow(dead_code)]
    Refused {
        code: &'static str,
        message: &'static str,
        hint: &'static str,
    },
}

pub fn unlock_fetch(kc: &Keychain, scope: &TouchIdScope) -> Result<UnlockFetch, String> {
    let (cli_owns_legacy, stale_generation) = managed_unlock_state(&scope.data_root)?;
    if stale_generation.is_some() {
        return Ok(UnlockFetch::Refused {
            code: "touch_id_passphrase_not_found",
            message: "The saved Touch ID passphrase is stale for these books.",
            hint: "Unlock once with the current passphrase and re-enroll Touch ID.",
        });
    }
    let not_found = UnlockFetch::Refused {
        code: "touch_id_passphrase_not_found",
        message: "No Touch ID passphrase was found for these books.",
        hint: "Unlock once with the passphrase to save it again.",
    };
    if let Some(secret) = kc.get_passphrase(&scope.account, cli_owns_legacy)? {
        return Ok(UnlockFetch::Passphrase {
            secret,
            migrated_legacy_account: None,
            account: scope.account.clone(),
            cli_owns_legacy,
        });
    }
    let Some(legacy) = scope.legacy_account.as_deref() else {
        return Ok(not_found);
    };
    let Some(secret) = kc.get_passphrase(legacy, cli_owns_legacy)? else {
        return Ok(not_found);
    };
    Ok(UnlockFetch::Passphrase {
        secret,
        migrated_legacy_account: Some(legacy.to_string()),
        account: scope.account.clone(),
        cli_owns_legacy,
    })
}

/// The second half, after the daemon accepted the passphrase: a credential read from the
/// pre-migration account follows the books to the current one. Failures are reported, not
/// fatal (the unlock already happened).
pub fn unlock_follow_migration(
    kc: &Keychain,
    account: &str,
    legacy_account: &str,
    cli_owns_legacy: bool,
    passphrase: &str,
) -> Result<(), String> {
    kc.store_passphrase(account, passphrase)
        .map_err(|e| format!("Touch ID credential could not follow the migrated books: {e}"))?;
    kc.delete_passphrase(legacy_account, cli_owns_legacy)
        .map_err(|e| format!("migrated Touch ID credential cleanup failed: {e}"))
}

/// The unlock request the passphrase goes out with: `ui.projects.select` for a book that asked
/// on opening, else `daemon.unlock`.
pub fn unlock_request(
    passphrase: &str,
    require_existing_project: bool,
    project_id: Option<&str>,
) -> (&'static str, Value) {
    let mut args = json!({"auth_response": {"passphrase_secret": passphrase}, "require_existing_project": require_existing_project});
    match project_id.filter(|v| !v.trim().is_empty()) {
        Some(id) => {
            args["project_id"] = json!(id);
            ("ui.projects.select", args)
        }
        None => ("daemon.unlock", args),
    }
}

#[cfg(test)]
mod tests {
    use super::super::biometric::Fake;
    use super::super::store::{IdentityStrength, MemoryStore, SecretStoreAvailability};
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("rune-secrets-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("data")).unwrap();
        p
    }

    fn mem() -> MemoryStore {
        MemoryStore::new(SecretStoreAvailability::Available {
            identity_strength: IdentityStrength::Production,
        })
    }

    #[test]
    fn stale_generation_guard_is_compare_and_clear() {
        let root = temp("stale");
        let data_root = root.join("data").to_string_lossy().into_owned();
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(root.join("config/settings.json"), b"{\"cli_legacy_unlock_quarantined\":true,\"desktop_biometric_stale\":\"generation-new\",\"keep\":true}\n").unwrap();
        assert_eq!(
            desktop_biometric_stale_generation(&data_root)
                .unwrap()
                .as_deref(),
            Some("generation-new")
        );
        assert!(!clear_stale_guard_if_matches(&data_root, Some("generation-old")).unwrap());
        assert_eq!(
            desktop_biometric_stale_generation(&data_root)
                .unwrap()
                .as_deref(),
            Some("generation-new")
        );
        assert!(clear_stale_guard_if_matches(&data_root, Some("generation-new")).unwrap());
        assert_eq!(
            desktop_biometric_stale_generation(&data_root).unwrap(),
            None
        );
        let kept: Value =
            serde_json::from_str(&fs::read_to_string(root.join("config/settings.json")).unwrap())
                .unwrap();
        assert_eq!(kept["keep"], true);
        assert!(managed_unlock_state(&data_root).unwrap().0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_managed_settings_fail_closed() {
        let root = temp("invalid");
        let data_root = root.join("data").to_string_lossy().into_owned();
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(root.join("config/settings.json"), b"[1]").unwrap();
        assert!(desktop_biometric_stale_generation(&data_root).is_err());
        fs::write(root.join("config/settings.json"), b"{nope").unwrap();
        assert!(desktop_biometric_stale_generation(&data_root).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_boolean_guard_from_old_builds_still_counts() {
        assert_eq!(
            stale_generation_from_settings(&json!({"desktop_biometric_stale": true})).as_deref(),
            Some("legacy-boolean-guard")
        );
        assert_eq!(
            stale_generation_from_settings(&json!({"desktop_biometric_stale": false})),
            None
        );
        assert_eq!(
            stale_generation_from_settings(&json!({"desktop_biometric_stale": " "})),
            None
        );
    }

    #[test]
    fn store_is_refused_when_the_guard_moved_and_clears_it_when_it_matches() {
        let root = temp("store");
        let data_root = root.join("data");
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(
            root.join("config/settings.json"),
            b"{\"desktop_biometric_stale\":\"g2\"}",
        )
        .unwrap();
        let s = mem();
        let bio = Fake::new(Ok(()), true);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        let scope = scope_for_selected(data_root.clone());
        let before = status_command(&kc, &scope).unwrap();
        assert!(before.stale && before.stale_generation.as_deref() == Some("g2"));
        assert!(
            store_command(&kc, &scope, "pass", Some("g1"))
                .unwrap_err()
                .contains("enrollment state changed")
        );
        let after = store_command(&kc, &scope, "pass", Some("g2")).unwrap();
        assert!(after.configured && !after.stale);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unlock_fetch_refuses_a_stale_or_missing_passphrase_and_hands_out_a_saved_one() {
        let root = temp("unlock");
        let s = mem();
        let bio = Fake::new(Ok(()), true);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        let scope = scope_for_selected(root.join("data"));
        assert!(matches!(
            unlock_fetch(&kc, &scope).unwrap(),
            UnlockFetch::Refused {
                code: "touch_id_passphrase_not_found",
                ..
            }
        ));
        store_command(&kc, &scope, "open sesame", None).unwrap();
        match unlock_fetch(&kc, &scope).unwrap() {
            UnlockFetch::Passphrase {
                secret,
                migrated_legacy_account,
                ..
            } => {
                assert_eq!(secret.as_str(), "open sesame");
                assert_eq!(migrated_legacy_account, None);
            }
            UnlockFetch::Refused { .. } => panic!("refused"),
        }
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(
            root.join("config/settings.json"),
            b"{\"desktop_biometric_stale\":\"g\"}",
        )
        .unwrap();
        match unlock_fetch(&kc, &scope).unwrap() {
            UnlockFetch::Refused { message, .. } => assert!(message.contains("stale")),
            UnlockFetch::Passphrase { .. } => panic!("stale passphrase handed out"),
        }
        let forgotten = forget_command(&kc, &scope).unwrap();
        assert!(!forgotten.configured && !forgotten.stale);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unlock_request_matches_the_tauri_command() {
        let (kind, args) = unlock_request("p", true, Some("proj"));
        assert_eq!(kind, "ui.projects.select");
        assert_eq!(args["project_id"], "proj");
        assert_eq!(args["require_existing_project"], true);
        assert_eq!(args["auth_response"]["passphrase_secret"], "p");
        let (kind, args) = unlock_request("p", false, Some("  "));
        assert_eq!(kind, "daemon.unlock");
        assert!(args.get("project_id").is_none());
    }

    #[test]
    fn migrated_default_root_can_find_its_legacy_touch_id_account() {
        let home = std::env::temp_dir().join(format!("rune-secrets-home-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let native = home.join("Library/Application Support/at.bitcoinaustria.kassiber");
        let data_root = native.join("projects/default/data");
        fs::create_dir_all(&data_root).unwrap();
        assert_eq!(
            migrated_legacy_touch_id_account_for_platform(&data_root, "macos", &home, None, None),
            None
        );
        fs::write(
            native.join(STATE_ROOT_MIGRATION_FILENAME),
            b"{\"schema_version\":1,\"migrated_from_hidden_home\":true}",
        )
        .unwrap();
        let canonical_home = fs::canonicalize(&home).unwrap();
        let legacy =
            migrated_legacy_touch_id_account_for_platform(&data_root, "macos", &home, None, None)
                .unwrap();
        assert!(
            legacy == home.join(".kassiber/projects/default/data")
                || legacy == canonical_home.join(".kassiber/projects/default/data"),
            "{legacy:?}"
        );
        assert_eq!(
            migrated_legacy_touch_id_account_for_platform(
                &native.join("projects/default"),
                "macos",
                &home,
                None,
                None
            ),
            None
        );
        fs::write(
            native.join(STATE_ROOT_MIGRATION_FILENAME),
            b"{\"schema_version\":1,\"migrated_from_hidden_home\":false}",
        )
        .unwrap();
        assert_eq!(
            migrated_legacy_touch_id_account_for_platform(&data_root, "macos", &home, None, None),
            None
        );
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn scope_account_is_the_canonical_data_root() {
        let root = temp("scope");
        let scope = scope_for_data_root(Some(&root.join("data").to_string_lossy()), None);
        assert_eq!(
            scope.account,
            fs::canonicalize(root.join("data"))
                .unwrap()
                .to_string_lossy()
        );
        assert_eq!(scope.data_root, root.join("data").to_string_lossy());
        let _ = fs::remove_dir_all(root);
    }
}

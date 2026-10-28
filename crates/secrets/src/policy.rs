//! Python's DB-bound CLI enrollment and independent native invalidation guards.
use crate::service::storage_error;
use crate::{
    access_policy, remembered_unlock_account, valid_hex, CredentialStore, Passphrase,
    CLI_PASSPHRASE_SERVICE,
};
use kassiber_envelope::{dict, s, AppError};
use kassiber_ownership::OwnerNamespace;
use kassiber_paths::{Context, Platform};
use kassiber_pyjson::Value;
use serde_json::{json, Map, Value as Json};
use std::path::{Path, PathBuf};

pub const LEGACY_SERVICE: &str = "Kassiber Database Passphrase";
type Result<T> = std::result::Result<T, AppError>;

pub(crate) fn py(value: Json) -> Value {
    kassiber_pytext::loads(&value.to_string()).expect("JSON representation")
}

pub fn random_hex() -> Result<String> {
    #[cfg(feature = "test-hooks")]
    if let Ok(token) = std::env::var("KASSIBER_RUST_TEST_SECRETS_TOKEN") {
        if valid_hex(&token, 32) {
            return Ok(token);
        }
        return Err(AppError::new("validation", "invalid test secrets token"));
    }
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| AppError::new("internal_error", "OS randomness is unavailable"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub struct Policy {
    pub(crate) root: PathBuf,
    pub(crate) settings_path: PathBuf,
    pub(crate) project_identity: String,
    pub(crate) settings: Map<String, Json>,
}
impl Policy {
    pub fn load(namespace: &OwnerNamespace, root: &Path) -> Result<Self> {
        let project = namespace.canonical_project(root).map_err(storage_error)?;
        let root = project
            .database
            .parent()
            .expect("database parent")
            .to_owned();
        let settings_path = Context::from_env()
            .map_err(storage_error)?
            .state_root_for_data(&root)
            .map_err(storage_error)?
            .join("config/settings.json");
        let settings = kassiber_paths::load_settings(&settings_path).map_err(storage_error)?;
        Ok(Self {
            root,
            settings_path,
            project_identity: project.identity,
            settings,
        })
    }
    fn flag(&self, key: &str) -> bool {
        self.settings.get(key).and_then(Json::as_bool) == Some(true)
    }
    pub fn enabled(&self) -> bool {
        self.flag("cli_remembered_unlock")
    }
    pub fn quarantined(&self) -> bool {
        self.flag("cli_legacy_unlock_quarantined")
    }
    pub fn binding_state(&self) -> &'static str {
        let project = self
            .settings
            .get("operator_policy_project_identity")
            .and_then(Json::as_str);
        let db = self
            .settings
            .get("operator_policy_database_identity")
            .and_then(Json::as_str);
        if !project.is_some_and(|v| valid_hex(v, 64)) || !db.is_some_and(|v| valid_hex(v, 32)) {
            "missing"
        } else if project != Some(self.project_identity.as_str()) {
            "mismatch"
        } else {
            "valid"
        }
    }
    fn account(&self) -> Option<String> {
        remembered_unlock_account(
            self.settings
                .get("operator_policy_database_identity")?
                .as_str()?,
            self.settings
                .get("cli_remembered_unlock_enrollment")?
                .as_str()?,
        )
        .ok()
    }
    pub fn operator_status(&self) -> Result<Value> {
        let configured = self
            .settings
            .get("operator_unlock_mode")
            .unwrap_or(&Json::Null);
        if !configured.is_null()
            && !configured
                .as_str()
                .is_some_and(|v| ["manual", "brokered", "unattended"].contains(&v))
        {
            return Err(AppError::new(
                "invalid_operator_unlock_mode",
                "the configured operator unlock mode is invalid",
            )
            .with_hint("Choose manual, brokered, or unattended with `kassiber operator mode`.")
            .with_details(dict([(
                "configured",
                s(kassiber_envelope::py_str(&py(configured.clone()))),
            )])));
        }
        let effective = if self.binding_state() == "valid" {
            configured.as_str().unwrap_or("manual")
        } else {
            "manual"
        };
        Ok(dict([
            ("configured", py(configured.clone())),
            ("effective", s(effective)),
            ("legacy_inferred", Value::Bool(false)),
            ("binding_state", s(self.binding_state())),
        ]))
    }
    pub fn status(&self, store: &dyn CredentialStore) -> Value {
        let mut available = store.available();
        let mut configured = false;
        if available && self.enabled() && !self.quarantined() && self.binding_state() == "valid" {
            if let Some(account) = self.account() {
                match store.get_password(CLI_PASSPHRASE_SERVICE, &account) {
                    Ok(secret) => configured = secret.is_some(),
                    Err(_) => available = false,
                }
            } else {
                available = false;
            }
        }
        let platform = match Platform::current() {
            Platform::Linux => "linux",
            Platform::Macos => "macos",
            Platform::Windows => "windows",
        };
        dict([
            ("platform", s(platform)),
            ("access_policy", s(access_policy())),
            ("available", Value::Bool(available)),
            ("configured", Value::Bool(configured)),
            ("cli_enabled", Value::Bool(self.enabled())),
            ("legacy_quarantined", Value::Bool(self.quarantined())),
            ("binding_state", s(self.binding_state())),
        ])
    }
    pub fn disable(&self, quarantine: bool) -> Result<()> {
        kassiber_store::layout::mutate_settings(&self.settings_path, |settings| {
            settings.insert("operator_unlock_mode".into(), json!("manual"));
            for key in ["cli_remembered_unlock", "cli_remembered_unlock_enrollment"] {
                settings.remove(key);
            }
            if quarantine {
                settings.insert("cli_legacy_unlock_quarantined".into(), json!(true));
            } else {
                settings.remove("cli_legacy_unlock_quarantined");
            }
            Ok(())
        })
        .map_err(storage_error)
    }
    pub fn enable(&self, namespace: &OwnerNamespace, db_id: &str, enrollment: &str) -> Result<()> {
        remembered_unlock_account(db_id, enrollment).map_err(storage_error)?;
        let unchanged = || -> kassiber_paths::Result<()> {
            if namespace.canonical_project(&self.root)?.identity != self.project_identity {
                return Err(kassiber_paths::Error::new(
                    kassiber_paths::ErrorKind::OperatorProjectReplaced,
                    "the project changed before its unlock policy could be bound",
                ));
            }
            Ok(())
        };
        unchanged().map_err(storage_error)?;
        kassiber_store::layout::mutate_settings(&self.settings_path, |settings| {
            unchanged()?;
            let rebind = settings
                .get("operator_policy_project_identity")
                .and_then(Json::as_str)
                != Some(&self.project_identity)
                || settings
                    .get("operator_policy_database_identity")
                    .and_then(Json::as_str)
                    != Some(db_id);
            if rebind {
                for key in [
                    "operator_unlock_mode",
                    "cli_remembered_unlock",
                    "cli_remembered_unlock_enrollment",
                ] {
                    settings.remove(key);
                }
            }
            settings.insert(
                "operator_policy_project_identity".into(),
                json!(self.project_identity),
            );
            settings.insert("operator_policy_database_identity".into(), json!(db_id));
            settings.insert("cli_remembered_unlock".into(), json!(true));
            settings.insert("cli_remembered_unlock_enrollment".into(), json!(enrollment));
            settings.insert("operator_unlock_mode".into(), json!("unattended"));
            settings.remove("cli_legacy_unlock_quarantined");
            Ok(())
        })
        .map_err(storage_error)?;
        unchanged().map_err(storage_error)
    }
    fn delete(&self, store: &dyn CredentialStore, service: &str, account: Option<&str>) -> bool {
        account.is_some_and(|account| store.delete_password(service, account).is_ok())
    }
    fn delete_current(&self, store: &dyn CredentialStore) -> bool {
        self.delete(store, CLI_PASSPHRASE_SERVICE, self.account().as_deref())
    }
    fn delete_legacy(&self, store: &dyn CredentialStore, service: &str) -> bool {
        self.delete(store, service, self.root.to_str())
    }
}

pub fn enroll(
    namespace: &OwnerNamespace,
    root: &Path,
    secret: &Passphrase,
    store: &dyn CredentialStore,
) -> Result<Value> {
    enroll_observed(namespace, root, secret, store, &mut |_| Ok(()))
}
pub fn enroll_observed(
    namespace: &OwnerNamespace,
    root: &Path,
    secret: &Passphrase,
    store: &dyn CredentialStore,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Value> {
    let admitted = namespace.canonical_project(root).map_err(storage_error)?;
    let book = kassiber_store::Book::open_migrating(
        root,
        kassiber_store::OpenOptions {
            passphrase: Some(secret.expose()),
            expected_project: Some(&admitted),
            ..Default::default()
        },
        false,
    )
    .map_err(storage_error)?;
    let db_id = book.instance_id().to_owned();
    book.close()
        .map_err(|failure| storage_error(failure.error))?;
    let policy = Policy::load(namespace, root)?;
    if policy.project_identity != admitted.identity {
        return Err(AppError::new(
            "operator_project_replaced",
            "the project changed during remembered-unlock enrollment",
        ));
    }
    let enrollment = random_hex()?;
    let account = remembered_unlock_account(&db_id, &enrollment).map_err(storage_error)?;
    if store
        .set_password(CLI_PASSPHRASE_SERVICE, &account, secret)
        .is_err()
    {
        return Err(AppError::new("remembered_unlock_unavailable", "the OS credential store is unavailable or rejected the passphrase")
            .with_hint("Unlock the platform credential store and retry, or keep using --db-passphrase-fd. Kassiber will not use a plaintext fallback.")
            .with_details(policy.status(store)).retryable(true));
    }
    observer("remember.after_item_write")?;
    let prior = !policy.enabled() || policy.account().is_none() || policy.delete_current(store);
    observer("remember.after_prior_delete")?;
    let legacy_cli = policy.delete_legacy(store, CLI_PASSPHRASE_SERVICE);
    observer("remember.after_legacy_cli_delete")?;
    let legacy_shared = policy.delete_legacy(store, LEGACY_SERVICE);
    observer("remember.after_legacy_shared_delete")?;
    if !prior || !legacy_cli || !legacy_shared {
        let deleted = policy.delete(store, CLI_PASSPHRASE_SERVICE, Some(&account));
        return Err(AppError::new("remembered_unlock_legacy_cleanup_failed", "a legacy unlock credential could not be removed")
            .with_hint("Remove the legacy Kassiber CLI/shared database passphrase in the OS credential manager and retry enrollment.")
            .with_details(py(json!({"cli_credential_deleted":deleted,"prior_v2_credential_deleted":prior,"legacy_cli_credential_deleted":legacy_cli,"legacy_shared_credential_deleted":legacy_shared}))).retryable(true));
    }
    if let Err(error) = policy.enable(namespace, &db_id, &enrollment) {
        let deleted = policy.delete(store, CLI_PASSPHRASE_SERVICE, Some(&account));
        if error.code == "storage_io_error" {
            return Err(AppError::new(
                "remembered_unlock_settings_failed",
                "the passphrase was stored, but the CLI policy could not be enabled",
            )
            .with_hint(if deleted {
                "Fix permissions on the managed config directory and retry enrollment."
            } else {
                "Fix config permissions, remove the OS credential manually, and retry."
            })
            .with_details(dict([
                ("settings_error", s(error.message)),
                ("credential_deleted", Value::Bool(deleted)),
            ]))
            .retryable(true));
        }
        return Err(error);
    }
    observer("remember.after_policy_publish")?;
    Ok(dict([
        (
            "database",
            s(kassiber_paths::resolve_database_path(root)
                .to_string_lossy()
                .as_ref()),
        ),
        (
            "remembered_unlock",
            Policy::load(namespace, root)?.status(store),
        ),
    ]))
}

pub fn forget(
    namespace: &OwnerNamespace,
    root: &Path,
    store: &dyn CredentialStore,
    desktop: bool,
) -> Result<Value> {
    forget_observed(namespace, root, store, desktop, &mut |_| Ok(()))
}
pub fn forget_observed(
    namespace: &OwnerNamespace,
    root: &Path,
    store: &dyn CredentialStore,
    desktop: bool,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Value> {
    let policy = Policy::load(namespace, root)?;
    let owned = policy.enabled() || policy.quarantined();
    let deleted = policy.delete_current(store);
    observer("forget.after_item_delete")?;
    let legacy_cli = !owned || policy.delete_legacy(store, CLI_PASSPHRASE_SERVICE);
    observer("forget.after_legacy_cli_delete")?;
    let legacy_shared = !owned || policy.delete_legacy(store, LEGACY_SERVICE);
    observer("forget.after_legacy_shared_delete")?;
    let quarantine = !legacy_cli || !legacy_shared;
    let marker_error = policy.disable(quarantine).err().map(|e| e.message);
    observer("forget.after_policy_publish")?;
    let deletion_key = if desktop {
        "cli_credential_deleted"
    } else {
        "credential_deleted"
    };
    let mut details = dict([
        ("cli_marker_cleared", Value::Bool(marker_error.is_none())),
        (deletion_key, Value::Bool(deleted)),
        ("legacy_cli_credential_deleted", Value::Bool(legacy_cli)),
        (
            "legacy_shared_credential_deleted",
            Value::Bool(legacy_shared),
        ),
    ]);
    if quarantine {
        kassiber_envelope::set(
            &mut details,
            "legacy_quarantined",
            Value::Bool(marker_error.is_none()),
        );
        kassiber_envelope::set(
            &mut details,
            "quarantine_error",
            marker_error.map_or(Value::Null, s),
        );
        return Err(AppError::new("remembered_unlock_legacy_cleanup_failed", "the CLI-owned legacy unlock credential could not be deleted")
            .with_hint("Remove `Kassiber Database Passphrase` in the OS credential manager, then retry. Kassiber quarantined the leftover from both CLI and desktop use when managed settings allowed it.")
            .with_details(details).retryable(true));
    }
    if let Some(error) = marker_error {
        kassiber_envelope::set(&mut details, "settings_error", s(error));
        return Err(AppError::new(
            "remembered_unlock_settings_failed",
            "the CLI remembered-unlock marker could not be cleared",
        )
        .with_hint(if deleted {
            "Fix permissions on the managed config directory and retry."
        } else if desktop {
            "Fix config permissions, remove the remaining OS credential manually, and retry."
        } else {
            "Fix config permissions, remove the OS credential manually, and retry."
        })
        .with_details(details)
        .retryable(true));
    }
    kassiber_envelope::set(
        &mut details,
        "remembered_unlock",
        Policy::load(namespace, root)?.status(store),
    );
    if !desktop && !deleted {
        kassiber_envelope::set(&mut details, "warning", s("The CLI opt-in marker was cleared, but the OS credential could not be deleted. Remove it in the platform credential manager."));
    }
    Ok(details)
}

pub fn invalidate(
    namespace: &OwnerNamespace,
    root: &Path,
) -> kassiber_paths::Result<(Option<String>, String)> {
    invalidate_observed(namespace, root, &mut |_| Ok(()))
}
pub fn invalidate_observed(
    namespace: &OwnerNamespace,
    root: &Path,
    observer: &mut impl FnMut(&str) -> kassiber_paths::Result<()>,
) -> kassiber_paths::Result<(Option<String>, String)> {
    let policy = Policy::load(namespace, root).map_err(|_| {
        kassiber_paths::Error::new(
            kassiber_paths::ErrorKind::UnsafeConfiguration,
            "could not read native credential guards",
        )
    })?;
    let generation = random_hex().map_err(|_| {
        kassiber_paths::Error::new(
            kassiber_paths::ErrorKind::Io,
            "OS randomness is unavailable",
        )
    })?;
    // Python uses token_urlsafe(32) for the macOS desktop guard.
    #[cfg(target_os = "macos")]
    let desktop = {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| {
            kassiber_paths::Error::new(
                kassiber_paths::ErrorKind::Io,
                "OS randomness is unavailable",
            )
        })?;
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        let mut bits = 0u32;
        let mut count = 0;
        for byte in bytes {
            bits = (bits << 8) | u32::from(byte);
            count += 8;
            while count >= 6 {
                count -= 6;
                out.push(ALPHABET[((bits >> count) & 63) as usize] as char);
            }
        }
        if count != 0 {
            out.push(ALPHABET[((bits << (6 - count)) & 63) as usize] as char);
        }
        Some(out)
    };
    #[cfg(not(target_os = "macos"))]
    let desktop: Option<String> = None;
    if let Some(desktop) = &desktop {
        kassiber_store::layout::mutate_settings(&policy.settings_path, |settings| {
            settings.insert("desktop_biometric_stale".into(), json!(desktop));
            Ok(())
        })?;
        observer("rotation.after_desktop_guard")?;
    }
    kassiber_store::layout::mutate_settings(&policy.settings_path, |settings| {
        settings.insert("operator_native_auth_generation".into(), json!(generation));
        Ok(())
    })?;
    observer("rotation.after_operator_guard")?;
    Ok((desktop, generation))
}

pub fn refresh(
    namespace: &OwnerNamespace,
    root: &Path,
    secret: &Passphrase,
    store: &dyn CredentialStore,
) -> Result<Option<Value>> {
    refresh_observed(namespace, root, secret, store, &mut |_| Ok(()))
}
pub fn refresh_observed(
    namespace: &OwnerNamespace,
    root: &Path,
    secret: &Passphrase,
    store: &dyn CredentialStore,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Option<Value>> {
    let policy = Policy::load(namespace, root)?;
    if !policy.enabled() {
        return Ok(None);
    }
    if policy.binding_state() == "valid"
        && policy.account().is_some_and(|account| {
            store
                .set_password(CLI_PASSPHRASE_SERVICE, &account, secret)
                .is_ok()
        })
    {
        observer("rotation.after_item_refresh")?;
        return Ok(None);
    }
    let deleted = policy.delete_current(store);
    observer("rotation.after_item_delete")?;
    let legacy_cli = policy.delete_legacy(store, CLI_PASSPHRASE_SERVICE);
    observer("rotation.after_legacy_cli_delete")?;
    let legacy_shared = policy.delete_legacy(store, LEGACY_SERVICE);
    observer("rotation.after_legacy_shared_delete")?;
    let quarantine = !(legacy_cli && legacy_shared);
    let cleared = policy.disable(quarantine).is_ok();
    observer("rotation.after_policy_disable")?;
    Ok(Some(dict([("code", s("remembered_unlock_update_failed")), ("message", s("The database passphrase changed, but the CLI remembered-unlock copy could not be updated safely. Review the cleanup fields and re-enroll after removing any retained legacy credential.")), ("credential_deleted", Value::Bool(deleted)), ("legacy_cli_credential_deleted", Value::Bool(legacy_cli)), ("legacy_shared_credential_deleted", Value::Bool(legacy_shared)), ("legacy_quarantined", Value::Bool(quarantine)), ("cli_marker_cleared", Value::Bool(cleared))])))
}

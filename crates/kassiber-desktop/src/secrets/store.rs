//! The OS credential store and its policy (ui-tauri/src-tauri/src/secret_store.rs, the store half).
//!
//! The native store is the Tauri shell's: `keyring-core` over the platform keyring crates at the
//! same versions, with the same per-platform entry modifiers and search specs, so every item the
//! Tauri app wrote (AI provider keys, Touch ID markers, the preview-build Touch ID passphrase) is
//! found by the same `(service, account)` pair. The AI-key store policy (which store a new key
//! goes to by default) is byte-for-byte the same decision table.

use keyring_core::Entry;
use keyring_core::api::CredentialStoreApi;
use keyring_core::error::Error as KeyringError;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)] // the full Tauri set; `Unsigned` is never reported on its own
pub enum IdentityStrength {
    Unsigned,
    Adhoc,
    Production,
    UnknownOrUnsigned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[allow(dead_code)] // `LockedNeedsUnlock` comes from the Linux Secret Service
pub enum SecretStoreAvailability {
    Available { identity_strength: IdentityStrength },
    LockedNeedsUnlock,
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecretStoreEntryRef {
    pub service: String,
    pub account: String,
}

/// One generic-password store. `get` hands back the secret's bytes; callers zeroize them.
pub trait SecretStore: Send + Sync {
    fn availability(&self) -> SecretStoreAvailability;
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String>;
    fn exists(&self, service: &str, account: &str) -> Result<bool, String>;
    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), String>;
    fn delete(&self, service: &str, account: &str) -> Result<(), String>;
    fn list(&self, service: &str) -> Result<Vec<SecretStoreEntryRef>, String>;
}

/// A store that refuses everything (`ProbeSecretStore`).
#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // the Tauri shell's probe; kept for the contract and its test
pub struct ProbeSecretStore;

#[allow(dead_code)]
const PROBE_REFUSAL: &str = "native secret storage is unavailable in this build";

impl SecretStore for ProbeSecretStore {
    fn availability(&self) -> SecretStoreAvailability {
        default_secret_store_availability()
    }
    fn get(&self, _service: &str, _account: &str) -> Result<Option<Vec<u8>>, String> {
        Err(PROBE_REFUSAL.to_string())
    }
    fn exists(&self, _service: &str, _account: &str) -> Result<bool, String> {
        Err(PROBE_REFUSAL.to_string())
    }
    fn set(&self, _service: &str, _account: &str, _secret: &[u8]) -> Result<(), String> {
        Err(PROBE_REFUSAL.to_string())
    }
    fn delete(&self, _service: &str, _account: &str) -> Result<(), String> {
        Err(PROBE_REFUSAL.to_string())
    }
    fn list(&self, _service: &str) -> Result<Vec<SecretStoreEntryRef>, String> {
        Err(PROBE_REFUSAL.to_string())
    }
}

/// The OS store: macOS login Keychain, Windows Credential Manager (user scope), Linux Secret
/// Service.
#[derive(Debug, Clone, Default)]
pub struct NativeSecretStore;

impl SecretStore for NativeSecretStore {
    fn availability(&self) -> SecretStoreAvailability {
        default_secret_store_availability()
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        let entry = native_entry(service, account)?;
        match entry.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(keyring_error_for_user(error)),
        }
    }

    fn exists(&self, service: &str, account: &str) -> Result<bool, String> {
        Ok(self
            .list(service)?
            .iter()
            .any(|entry| entry.account == account))
    }

    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), String> {
        let entry = native_entry(service, account)?;
        entry.set_secret(secret).map_err(keyring_error_for_user)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        let entry = native_entry(service, account)?;
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(keyring_error_for_user(error)),
        }?;
        // apple-native-keyring-store 1.0.0's classic-Keychain backend drops
        // SecKeychainItemDelete's OSStatus. Verify absence so an ACL/signing failure cannot be
        // reported as successful credential revocation.
        match entry.get_secret() {
            Err(KeyringError::NoEntry) => Ok(()),
            Ok(mut left) => {
                zero(&mut left);
                Err("The credential remained in the native store after deletion.".to_string())
            }
            Err(error) => Err(keyring_error_for_user(error)),
        }
    }

    fn list(&self, service: &str) -> Result<Vec<SecretStoreEntryRef>, String> {
        native_search_by_service(service)
    }
}

/// Secrets by (service, account).
type Entries = BTreeMap<(String, String), Vec<u8>>;

/// An in-memory store (`MockSecretStore`): unit tests and the Touch ID scenes. Never touches
/// the OS.
#[derive(Debug, Clone)]
pub struct MemoryStore {
    availability: Arc<Mutex<SecretStoreAvailability>>,
    entries: Arc<Mutex<Entries>>,
    fail_next_set: Arc<Mutex<Option<String>>>,
    fail_next_delete: Arc<Mutex<Option<String>>>,
}

impl MemoryStore {
    pub fn new(availability: SecretStoreAvailability) -> Self {
        MemoryStore {
            availability: Arc::new(Mutex::new(availability)),
            entries: Arc::default(),
            fail_next_set: Arc::default(),
            fail_next_delete: Arc::default(),
        }
    }

    #[allow(dead_code)] // tests
    pub fn fail_next_set(&self, message: &str) {
        *self.fail_next_set.lock().expect("fail lock") = Some(message.to_string());
    }

    #[allow(dead_code)] // tests
    pub fn fail_next_delete(&self, message: &str) {
        *self.fail_next_delete.lock().expect("fail lock") = Some(message.to_string());
    }
}

impl SecretStore for MemoryStore {
    fn availability(&self) -> SecretStoreAvailability {
        self.availability.lock().expect("availability lock").clone()
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self
            .entries
            .lock()
            .expect("entries lock")
            .get(&(service.to_string(), account.to_string()))
            .cloned())
    }

    fn exists(&self, service: &str, account: &str) -> Result<bool, String> {
        Ok(self
            .entries
            .lock()
            .expect("entries lock")
            .contains_key(&(service.to_string(), account.to_string())))
    }

    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), String> {
        if let Some(message) = self.fail_next_set.lock().expect("fail lock").take() {
            return Err(message);
        }
        if let Some(mut old) = self
            .entries
            .lock()
            .expect("entries lock")
            .insert((service.to_string(), account.to_string()), secret.to_vec())
        {
            zero(&mut old);
        }
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        if let Some(message) = self.fail_next_delete.lock().expect("fail lock").take() {
            return Err(message);
        }
        if let Some(mut old) = self
            .entries
            .lock()
            .expect("entries lock")
            .remove(&(service.to_string(), account.to_string()))
        {
            zero(&mut old);
        }
        Ok(())
    }

    fn list(&self, service: &str) -> Result<Vec<SecretStoreEntryRef>, String> {
        Ok(self
            .entries
            .lock()
            .expect("entries lock")
            .keys()
            .filter(|(stored, _)| stored == service)
            .map(|(service, account)| SecretStoreEntryRef {
                service: service.clone(),
                account: account.clone(),
            })
            .collect())
    }
}

/// Overwrites a secret buffer before it is freed (`secret.fill(0)` in the Tauri shell).
pub fn zero(buf: &mut Vec<u8>) {
    use zeroize::Zeroize;
    buf.zeroize();
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretStorePlatform {
    Macos,
    Windows,
    Linux,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretStoreSelectionReason {
    Requested,
    ProductionDefault,
    UnsignedMacosDefault,
    PlatformDefault,
    NativeUnavailable,
    UnsupportedPlatform,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecretStorePolicySelection {
    pub store_id: String,
    pub reason: SecretStoreSelectionReason,
    pub native_store_id: Option<String>,
    pub native_available: bool,
    pub warning: Option<String>,
}

pub const STORE_ID_MACOS_KEYCHAIN: &str = "macos_keychain";
pub const STORE_ID_WINDOWS_DPAPI: &str = "windows_dpapi";
pub const STORE_ID_LINUX_SECRET_SERVICE: &str = "linux_secret_service";
pub const STORE_ID_SQLCIPHER_INLINE: &str = "sqlcipher_inline";

pub fn default_secret_store_availability() -> SecretStoreAvailability {
    if cfg!(target_os = "macos") {
        SecretStoreAvailability::Available {
            identity_strength: macos_identity_strength(),
        }
    } else if cfg!(target_os = "windows") {
        SecretStoreAvailability::Available {
            identity_strength: IdentityStrength::UnknownOrUnsigned,
        }
    } else if cfg!(target_os = "linux") {
        linux_secret_service_availability()
    } else {
        SecretStoreAvailability::Unavailable {
            reason: "unsupported_platform".to_string(),
        }
    }
}

pub fn current_secret_store_platform() -> SecretStorePlatform {
    if cfg!(target_os = "macos") {
        SecretStorePlatform::Macos
    } else if cfg!(target_os = "windows") {
        SecretStorePlatform::Windows
    } else if cfg!(target_os = "linux") {
        SecretStorePlatform::Linux
    } else {
        SecretStorePlatform::Unsupported
    }
}

/// The release build sets `KASSIBER_PRODUCTION_SIGNED` (or `KASSIBER_ADHOC_SIGNED`) at compile
/// time, exactly as for the Tauri shell; a plain build is unknown/unsigned.
fn macos_identity_strength() -> IdentityStrength {
    if option_env!("KASSIBER_PRODUCTION_SIGNED").is_some() {
        IdentityStrength::Production
    } else if option_env!("KASSIBER_ADHOC_SIGNED").is_some() {
        IdentityStrength::Adhoc
    } else {
        IdentityStrength::UnknownOrUnsigned
    }
}

#[cfg(target_os = "linux")]
fn linux_secret_service_availability() -> SecretStoreAvailability {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        return SecretStoreAvailability::Unavailable {
            reason: "dbus_session_bus_missing".to_string(),
        };
    }
    match linux_secret_service_store() {
        Ok(_) => SecretStoreAvailability::Available {
            identity_strength: IdentityStrength::UnknownOrUnsigned,
        },
        Err(KeyringError::NoStorageAccess(_)) => SecretStoreAvailability::LockedNeedsUnlock,
        Err(error) => SecretStoreAvailability::Unavailable {
            reason: keyring_error_code(&error).to_string(),
        },
    }
}

#[cfg(not(target_os = "linux"))]
fn linux_secret_service_availability() -> SecretStoreAvailability {
    SecretStoreAvailability::Unavailable {
        reason: "not_linux".to_string(),
    }
}

pub fn native_store_id_for_platform(platform: &SecretStorePlatform) -> Option<&'static str> {
    match platform {
        SecretStorePlatform::Macos => Some(STORE_ID_MACOS_KEYCHAIN),
        SecretStorePlatform::Windows => Some(STORE_ID_WINDOWS_DPAPI),
        SecretStorePlatform::Linux => Some(STORE_ID_LINUX_SECRET_SERVICE),
        SecretStorePlatform::Unsupported => None,
    }
}

/// Which store a new AI provider key goes to (`select_ai_provider_secret_store`).
pub fn select_ai_provider_secret_store(
    platform: SecretStorePlatform,
    availability: SecretStoreAvailability,
    requested_store_id: Option<&str>,
) -> SecretStorePolicySelection {
    let native_store_id = native_store_id_for_platform(&platform).map(str::to_string);
    let native_available = matches!(availability, SecretStoreAvailability::Available { .. });
    let pick = |store_id: &str, reason, warning| SecretStorePolicySelection {
        store_id: store_id.to_string(),
        reason,
        native_store_id: native_store_id.clone(),
        native_available,
        warning,
    };

    if let Some(requested) = requested_store_id.filter(|value| !value.trim().is_empty()) {
        if requested == STORE_ID_SQLCIPHER_INLINE {
            return pick(
                STORE_ID_SQLCIPHER_INLINE,
                SecretStoreSelectionReason::Requested,
                None,
            );
        }
        if native_store_id.as_deref() == Some(requested) && native_available {
            return pick(
                requested,
                SecretStoreSelectionReason::Requested,
                native_warning_for(&platform, &availability),
            );
        }
        return pick(
            STORE_ID_SQLCIPHER_INLINE,
            SecretStoreSelectionReason::NativeUnavailable,
            Some(native_unavailable_message(&availability)),
        );
    }

    match platform {
        SecretStorePlatform::Macos => match availability {
            SecretStoreAvailability::Available { identity_strength: IdentityStrength::Production } => pick(STORE_ID_MACOS_KEYCHAIN, SecretStoreSelectionReason::ProductionDefault, None),
            SecretStoreAvailability::Available { .. } => pick(
                STORE_ID_SQLCIPHER_INLINE,
                SecretStoreSelectionReason::UnsignedMacosDefault,
                Some("This build is not signed, so AI keys are kept in the book database. Moving them to the Keychain is experimental: macOS may ask for access again after updates.".to_string()),
            ),
            other => pick(STORE_ID_SQLCIPHER_INLINE, SecretStoreSelectionReason::NativeUnavailable, Some(native_unavailable_message(&other))),
        },
        SecretStorePlatform::Windows => {
            if native_available {
                pick(STORE_ID_WINDOWS_DPAPI, SecretStoreSelectionReason::PlatformDefault, None)
            } else {
                pick(STORE_ID_SQLCIPHER_INLINE, SecretStoreSelectionReason::NativeUnavailable, Some(native_unavailable_message(&availability)))
            }
        }
        SecretStorePlatform::Linux => {
            if native_available {
                pick(STORE_ID_LINUX_SECRET_SERVICE, SecretStoreSelectionReason::PlatformDefault, None)
            } else {
                pick(STORE_ID_SQLCIPHER_INLINE, SecretStoreSelectionReason::NativeUnavailable, Some(native_unavailable_message(&availability)))
            }
        }
        SecretStorePlatform::Unsupported => pick(
            STORE_ID_SQLCIPHER_INLINE,
            SecretStoreSelectionReason::UnsupportedPlatform,
            Some("This system has no supported keychain, so AI keys stay in the book database.".to_string()),
        ),
    }
}

pub fn current_ai_provider_secret_store_policy(
    requested_store_id: Option<&str>,
) -> SecretStorePolicySelection {
    select_ai_provider_secret_store(
        current_secret_store_platform(),
        default_secret_store_availability(),
        requested_store_id,
    )
}

/// What `ai.providers.list` answers carry as `secret_store_policy` (`secret_store_policy_status`).
pub fn secret_store_policy_status() -> serde_json::Value {
    let platform = current_secret_store_platform();
    let availability = default_secret_store_availability();
    let default_selection =
        select_ai_provider_secret_store(platform.clone(), availability.clone(), None);
    serde_json::json!({
        "platform": platform,
        "availability": availability,
        "default": default_selection,
        "policy": platform_policy_summary(),
    })
}

fn native_warning_for(
    platform: &SecretStorePlatform,
    availability: &SecretStoreAvailability,
) -> Option<String> {
    match (platform, availability) {
        (
            SecretStorePlatform::Macos,
            SecretStoreAvailability::Available { identity_strength: IdentityStrength::Unsigned | IdentityStrength::Adhoc | IdentityStrength::UnknownOrUnsigned },
        ) => Some("Keychain storage is experimental in unsigned builds: macOS may ask for access again after updates.".to_string()),
        _ => None,
    }
}

fn native_unavailable_message(availability: &SecretStoreAvailability) -> String {
    match availability {
        SecretStoreAvailability::LockedNeedsUnlock => "The system keychain is locked, so AI keys stay in the book database until it is unlocked.".to_string(),
        SecretStoreAvailability::Unavailable { reason } => format!("The system keychain is unavailable ({reason}), so AI keys stay in the book database."),
        SecretStoreAvailability::Available { .. } => "That keychain is not available on this system.".to_string(),
    }
}

pub fn platform_policy_summary() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        (
            "macos_unsigned_default",
            "sqlcipher_inline; Keychain opt-in remains experimental until production signing",
        ),
        (
            "windows_scope",
            "user-scope Credential Manager/DPAPI only; no machine-scope secrets",
        ),
        (
            "linux_fallback",
            "Secret Service when available; sqlcipher_inline when missing, locked, headless, or no D-Bus",
        ),
    ])
}

// ---- The platform keyrings ------------------------------------------------------------------

fn native_entry(service: &str, account: &str) -> Result<Entry, String> {
    if service.trim().is_empty() || account.trim().is_empty() {
        return Err("secret store service and account must be non-empty".to_string());
    }
    native_platform_entry(service, account).map_err(keyring_error_for_user)
}

fn native_search_by_service(service: &str) -> Result<Vec<SecretStoreEntryRef>, String> {
    if service.trim().is_empty() {
        return Err("secret store service must be non-empty".to_string());
    }
    native_platform_search(service).map_err(keyring_error_for_user)
}

#[cfg(target_os = "macos")]
fn native_platform_entry(service: &str, account: &str) -> keyring_core::Result<Entry> {
    apple_native_keyring_store::keychain::Store::new()?.build(service, account, None)
}

#[cfg(target_os = "macos")]
fn native_platform_search(service: &str) -> keyring_core::Result<Vec<SecretStoreEntryRef>> {
    let mut spec = HashMap::new();
    spec.insert("service", service);
    Ok(apple_native_keyring_store::keychain::Store::new()?
        .search(&spec)?
        .into_iter()
        .filter_map(|entry| {
            entry
                .get_specifiers()
                .map(|(service, account)| SecretStoreEntryRef { service, account })
        })
        .collect())
}

#[cfg(target_os = "windows")]
fn native_platform_entry(service: &str, account: &str) -> keyring_core::Result<Entry> {
    let mut modifiers = HashMap::new();
    modifiers.insert("persistence", "Local");
    windows_native_keyring_store::Store::new()?.build(service, account, Some(&modifiers))
}

#[cfg(target_os = "windows")]
fn native_platform_search(service: &str) -> keyring_core::Result<Vec<SecretStoreEntryRef>> {
    let mut spec = HashMap::new();
    spec.insert("pattern", service);
    Ok(windows_native_keyring_store::Store::new()?
        .search(&spec)?
        .into_iter()
        .filter_map(|entry| {
            entry
                .get_specifiers()
                .map(|(service, account)| SecretStoreEntryRef { service, account })
        })
        .filter(|entry| entry.service == service)
        .collect())
}

#[cfg(target_os = "linux")]
fn linux_secret_service_store()
-> keyring_core::Result<Arc<zbus_secret_service_keyring_store::Store>> {
    zbus_secret_service_keyring_store::Store::new()
}

#[cfg(target_os = "linux")]
fn native_platform_entry(service: &str, account: &str) -> keyring_core::Result<Entry> {
    let mut modifiers = HashMap::new();
    let label = format!("Kassiber AI provider {account}");
    modifiers.insert("label", label.as_str());
    linux_secret_service_store()?.build(service, account, Some(&modifiers))
}

#[cfg(target_os = "linux")]
fn native_platform_search(service: &str) -> keyring_core::Result<Vec<SecretStoreEntryRef>> {
    let mut spec = HashMap::new();
    spec.insert("service", service);
    Ok(linux_secret_service_store()?
        .search(&spec)?
        .into_iter()
        .filter_map(|entry| {
            entry
                .get_specifiers()
                .map(|(service, account)| SecretStoreEntryRef { service, account })
        })
        .collect())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn native_platform_entry(_service: &str, _account: &str) -> keyring_core::Result<Entry> {
    Err(KeyringError::NotSupportedByStore(
        "unsupported platform".to_string(),
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn native_platform_search(_service: &str) -> keyring_core::Result<Vec<SecretStoreEntryRef>> {
    Err(KeyringError::NotSupportedByStore(
        "unsupported platform".to_string(),
    ))
}

fn keyring_error_code(error: &KeyringError) -> &'static str {
    match error {
        KeyringError::PlatformFailure(_) => "platform_failure",
        KeyringError::NoStorageAccess(_) => "no_storage_access",
        KeyringError::NoEntry => "no_entry",
        KeyringError::BadEncoding(_) => "bad_encoding",
        KeyringError::BadDataFormat(_, _) => "bad_data_format",
        KeyringError::BadStoreFormat(_) => "bad_store_format",
        KeyringError::TooLong(_, _) => "too_long",
        KeyringError::Invalid(_, _) => "invalid",
        KeyringError::Ambiguous(_) => "ambiguous",
        KeyringError::NoDefaultStore => "no_default_store",
        KeyringError::NotSupportedByStore(_) => "not_supported",
        _ => "unknown",
    }
}

fn keyring_error_for_user(error: KeyringError) -> String {
    match error {
        KeyringError::NoEntry => "secret_ref_missing".to_string(),
        KeyringError::NoStorageAccess(_) => "secret_store_locked_or_denied".to_string(),
        other => format!("{}: {other}", keyring_error_code(&other)),
    }
}

/// The platform keyring compiled into this build (`compiled_keyring_backend_marker`).
#[allow(dead_code)] // tests
pub fn compiled_keyring_backend_marker() -> &'static str {
    let _ = std::any::type_name::<KeyringError>();
    compiled_platform_backend_marker()
}

#[cfg(target_os = "macos")]
fn compiled_platform_backend_marker() -> &'static str {
    std::any::type_name::<apple_native_keyring_store::keychain::Store>()
}

#[cfg(target_os = "windows")]
fn compiled_platform_backend_marker() -> &'static str {
    std::any::type_name::<windows_native_keyring_store::Store>()
}

#[cfg(target_os = "linux")]
fn compiled_platform_backend_marker() -> &'static str {
    std::any::type_name::<zbus_secret_service_keyring_store::Store>()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn compiled_platform_backend_marker() -> &'static str {
    "unsupported"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_store_is_not_production_storage() {
        let store = ProbeSecretStore;
        assert!(store.set("service", "account", b"secret").is_err());
        assert!(store.get("service", "account").is_err());
        assert!(store.delete("service", "account").is_err());
        assert!(store.list("service").is_err());
    }

    #[test]
    fn keyring_backend_marker_is_compiled_for_this_target() {
        assert!(!compiled_keyring_backend_marker().is_empty());
    }

    #[test]
    fn policy_summary_keeps_expected_defaults() {
        let policy = platform_policy_summary();
        assert!(policy["macos_unsigned_default"].contains("sqlcipher_inline"));
        assert!(policy["windows_scope"].contains("user-scope"));
        assert!(policy["linux_fallback"].contains("Secret Service"));
    }

    #[test]
    fn policy_selects_sqlcipher_for_unsigned_macos_by_default() {
        let selection = select_ai_provider_secret_store(
            SecretStorePlatform::Macos,
            SecretStoreAvailability::Available {
                identity_strength: IdentityStrength::UnknownOrUnsigned,
            },
            None,
        );
        assert_eq!(selection.store_id, STORE_ID_SQLCIPHER_INLINE);
        assert_eq!(
            selection.reason,
            SecretStoreSelectionReason::UnsignedMacosDefault
        );
        assert!(selection.warning.is_some());
    }

    #[test]
    fn policy_selects_keychain_for_production_macos() {
        let selection = select_ai_provider_secret_store(
            SecretStorePlatform::Macos,
            SecretStoreAvailability::Available {
                identity_strength: IdentityStrength::Production,
            },
            None,
        );
        assert_eq!(selection.store_id, STORE_ID_MACOS_KEYCHAIN);
        assert_eq!(
            selection.reason,
            SecretStoreSelectionReason::ProductionDefault
        );
        assert_eq!(selection.warning, None);
    }

    #[test]
    fn policy_honours_an_explicit_keychain_move_with_the_unsigned_warning() {
        let selection = select_ai_provider_secret_store(
            SecretStorePlatform::Macos,
            SecretStoreAvailability::Available {
                identity_strength: IdentityStrength::UnknownOrUnsigned,
            },
            Some(STORE_ID_MACOS_KEYCHAIN),
        );
        assert_eq!(selection.store_id, STORE_ID_MACOS_KEYCHAIN);
        assert_eq!(selection.reason, SecretStoreSelectionReason::Requested);
        assert!(selection.warning.unwrap().contains("experimental"));
        // A store of another platform falls back to SQLCipher.
        let other = select_ai_provider_secret_store(
            SecretStorePlatform::Macos,
            SecretStoreAvailability::Available {
                identity_strength: IdentityStrength::Production,
            },
            Some(STORE_ID_WINDOWS_DPAPI),
        );
        assert_eq!(other.store_id, STORE_ID_SQLCIPHER_INLINE);
        assert_eq!(other.reason, SecretStoreSelectionReason::NativeUnavailable);
    }

    #[test]
    fn policy_selects_windows_user_store_when_available() {
        let selection = select_ai_provider_secret_store(
            SecretStorePlatform::Windows,
            SecretStoreAvailability::Available {
                identity_strength: IdentityStrength::UnknownOrUnsigned,
            },
            None,
        );
        assert_eq!(selection.store_id, STORE_ID_WINDOWS_DPAPI);
        assert_eq!(
            selection.reason,
            SecretStoreSelectionReason::PlatformDefault
        );
    }

    #[test]
    fn policy_selects_sqlcipher_when_linux_secret_service_is_unavailable() {
        let selection = select_ai_provider_secret_store(
            SecretStorePlatform::Linux,
            SecretStoreAvailability::Unavailable {
                reason: "dbus_session_bus_missing".to_string(),
            },
            None,
        );
        assert_eq!(selection.store_id, STORE_ID_SQLCIPHER_INLINE);
        assert_eq!(
            selection.reason,
            SecretStoreSelectionReason::NativeUnavailable
        );
        assert!(
            selection
                .warning
                .unwrap()
                .contains("dbus_session_bus_missing")
        );
    }

    #[test]
    fn policy_status_has_the_tauri_json_shape() {
        let status = secret_store_policy_status();
        assert!(status["platform"].is_string());
        assert!(status["availability"]["state"].is_string());
        assert!(status["default"]["store_id"].is_string());
        assert!(status["default"]["reason"].is_string());
        assert!(status["policy"]["linux_fallback"].is_string());
        // This dev build is not production-signed: SQLCipher stays the default on macOS.
        if cfg!(target_os = "macos") && option_env!("KASSIBER_PRODUCTION_SIGNED").is_none() {
            assert_eq!(status["default"]["store_id"], STORE_ID_SQLCIPHER_INLINE);
            assert_eq!(
                status["availability"]["identity_strength"],
                "unknown_or_unsigned"
            );
        }
    }

    #[test]
    fn memory_store_roundtrips_without_host_keychain() {
        let store = MemoryStore::new(SecretStoreAvailability::Available {
            identity_strength: IdentityStrength::Production,
        });
        assert_eq!(store.get("svc", "acct").unwrap(), None);
        store.set("svc", "acct", b"secret").unwrap();
        assert_eq!(store.get("svc", "acct").unwrap(), Some(b"secret".to_vec()));
        assert_eq!(store.list("svc").unwrap().len(), 1);
        assert!(store.exists("svc", "acct").unwrap());
        store.delete("svc", "acct").unwrap();
        assert_eq!(store.get("svc", "acct").unwrap(), None);
    }

    #[test]
    fn zero_clears_the_buffer() {
        let mut secret = b"hunter2".to_vec();
        zero(&mut secret);
        assert!(secret.is_empty() || secret.iter().all(|b| *b == 0));
    }

    /// Writes, reads, lists and deletes one item in the real login Keychain under a test-only
    /// service. Ignored by default; run explicitly on a Mac to prove the native backend (the
    /// one the Tauri shell used) works from an unsigned build.
    #[test]
    #[ignore]
    fn native_store_roundtrip_with_a_test_only_service() {
        let service = "Kassiber Rune Desktop Test Only";
        let account = "rune-secret-store-roundtrip";
        let store = NativeSecretStore;
        let _ = store.delete(service, account);
        assert!(!store.exists(service, account).unwrap());
        store
            .set(service, account, b"test-value-not-a-secret")
            .unwrap();
        assert!(store.exists(service, account).unwrap());
        let mut got = store.get(service, account).unwrap().expect("item");
        assert_eq!(got, b"test-value-not-a-secret");
        zero(&mut got);
        store.delete(service, account).unwrap();
        assert!(!store.exists(service, account).unwrap());
        assert_eq!(store.get(service, account).unwrap(), None);
    }
}

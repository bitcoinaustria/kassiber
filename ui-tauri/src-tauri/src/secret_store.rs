use keyring_core::api::CredentialStoreApi;
use keyring_core::error::Error as KeyringError;
use keyring_core::Entry;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
#[cfg(test)]
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityStrength {
    Unsigned,
    Adhoc,
    Production,
    UnknownOrUnsigned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
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

pub trait SecretStore: Send + Sync {
    fn availability(&self) -> SecretStoreAvailability;
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String>;
    fn exists(&self, service: &str, account: &str) -> Result<bool, String>;
    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), String>;
    fn delete(&self, service: &str, account: &str) -> Result<(), String>;
    fn list(&self, service: &str) -> Result<Vec<SecretStoreEntryRef>, String>;
}

#[derive(Debug, Clone, Default)]
pub struct ProbeSecretStore;

impl SecretStore for ProbeSecretStore {
    fn availability(&self) -> SecretStoreAvailability {
        default_secret_store_availability()
    }

    fn get(&self, _service: &str, _account: &str) -> Result<Option<Vec<u8>>, String> {
        Err("native secret storage is unavailable in this build".to_string())
    }

    fn exists(&self, _service: &str, _account: &str) -> Result<bool, String> {
        Err("native secret storage is unavailable in this build".to_string())
    }

    fn set(&self, _service: &str, _account: &str, _secret: &[u8]) -> Result<(), String> {
        Err("native secret storage is unavailable in this build".to_string())
    }

    fn delete(&self, _service: &str, _account: &str) -> Result<(), String> {
        Err("native secret storage is unavailable in this build".to_string())
    }

    fn list(&self, _service: &str) -> Result<Vec<SecretStoreEntryRef>, String> {
        Err("native secret storage is unavailable in this build".to_string())
    }
}

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
        }
    }

    fn list(&self, service: &str) -> Result<Vec<SecretStoreEntryRef>, String> {
        native_search_by_service(service)
    }
}

#[cfg(test)]
type MockSecretKey = (String, String);
#[cfg(test)]
type MockSecretEntries = Arc<Mutex<BTreeMap<MockSecretKey, Vec<u8>>>>;

#[cfg(test)]
#[derive(Debug, Clone)]
pub struct MockSecretStore {
    availability: Arc<Mutex<SecretStoreAvailability>>,
    entries: MockSecretEntries,
    fail_next_set: Arc<Mutex<Option<String>>>,
}

#[cfg(test)]
impl MockSecretStore {
    pub fn new(availability: SecretStoreAvailability) -> Self {
        Self {
            availability: Arc::new(Mutex::new(availability)),
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            fail_next_set: Arc::new(Mutex::new(None)),
        }
    }

    pub fn fail_next_set(&self, message: &str) {
        *self.fail_next_set.lock().expect("mock fail lock") = Some(message.to_string());
    }
}

#[cfg(test)]
impl SecretStore for MockSecretStore {
    fn availability(&self) -> SecretStoreAvailability {
        self.availability
            .lock()
            .expect("mock availability lock")
            .clone()
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self
            .entries
            .lock()
            .expect("mock entries lock")
            .get(&(service.to_string(), account.to_string()))
            .cloned())
    }

    fn exists(&self, service: &str, account: &str) -> Result<bool, String> {
        Ok(self
            .entries
            .lock()
            .expect("mock entries lock")
            .contains_key(&(service.to_string(), account.to_string())))
    }

    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), String> {
        if let Some(message) = self.fail_next_set.lock().expect("mock fail lock").take() {
            return Err(message);
        }
        self.entries
            .lock()
            .expect("mock entries lock")
            .insert((service.to_string(), account.to_string()), secret.to_vec());
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        self.entries
            .lock()
            .expect("mock entries lock")
            .remove(&(service.to_string(), account.to_string()));
        Ok(())
    }

    fn list(&self, service: &str) -> Result<Vec<SecretStoreEntryRef>, String> {
        Ok(self
            .entries
            .lock()
            .expect("mock entries lock")
            .keys()
            .filter(|(stored_service, _)| stored_service == service)
            .map(|(stored_service, account)| SecretStoreEntryRef {
                service: stored_service.clone(),
                account: account.clone(),
            })
            .collect())
    }
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

pub fn select_ai_provider_secret_store(
    platform: SecretStorePlatform,
    availability: SecretStoreAvailability,
    requested_store_id: Option<&str>,
) -> SecretStorePolicySelection {
    let native_store_id = native_store_id_for_platform(&platform).map(str::to_string);
    let native_available = matches!(availability, SecretStoreAvailability::Available { .. });

    if let Some(requested) = requested_store_id.filter(|value| !value.trim().is_empty()) {
        if requested == STORE_ID_SQLCIPHER_INLINE {
            return SecretStorePolicySelection {
                store_id: STORE_ID_SQLCIPHER_INLINE.to_string(),
                reason: SecretStoreSelectionReason::Requested,
                native_store_id,
                native_available,
                warning: None,
            };
        }
        if native_store_id.as_deref() == Some(requested) && native_available {
            return SecretStorePolicySelection {
                store_id: requested.to_string(),
                reason: SecretStoreSelectionReason::Requested,
                native_store_id,
                native_available,
                warning: native_warning_for(&platform, &availability),
            };
        }
        return SecretStorePolicySelection {
            store_id: STORE_ID_SQLCIPHER_INLINE.to_string(),
            reason: SecretStoreSelectionReason::NativeUnavailable,
            native_store_id,
            native_available,
            warning: Some(native_unavailable_message(&availability)),
        };
    }

    match platform {
        SecretStorePlatform::Macos => match availability {
            SecretStoreAvailability::Available {
                identity_strength: IdentityStrength::Production,
            } => SecretStorePolicySelection {
                store_id: STORE_ID_MACOS_KEYCHAIN.to_string(),
                reason: SecretStoreSelectionReason::ProductionDefault,
                native_store_id,
                native_available,
                warning: None,
            },
            SecretStoreAvailability::Available { .. } => SecretStorePolicySelection {
                store_id: STORE_ID_SQLCIPHER_INLINE.to_string(),
                reason: SecretStoreSelectionReason::UnsignedMacosDefault,
                native_store_id,
                native_available,
                warning: Some(
                    "Unsigned or ad-hoc macOS builds keep AI keys in SQLCipher by default; Keychain is an explicit experimental move because prompts may appear again after rebuilds or identity changes."
                        .to_string(),
                ),
            },
            other => SecretStorePolicySelection {
                store_id: STORE_ID_SQLCIPHER_INLINE.to_string(),
                reason: SecretStoreSelectionReason::NativeUnavailable,
                native_store_id,
                native_available,
                warning: Some(native_unavailable_message(&other)),
            },
        },
        SecretStorePlatform::Windows => {
            if native_available {
                SecretStorePolicySelection {
                    store_id: STORE_ID_WINDOWS_DPAPI.to_string(),
                    reason: SecretStoreSelectionReason::PlatformDefault,
                    native_store_id,
                    native_available,
                    warning: None,
                }
            } else {
                SecretStorePolicySelection {
                    store_id: STORE_ID_SQLCIPHER_INLINE.to_string(),
                    reason: SecretStoreSelectionReason::NativeUnavailable,
                    native_store_id,
                    native_available,
                    warning: Some(native_unavailable_message(&availability)),
                }
            }
        }
        SecretStorePlatform::Linux => {
            if native_available {
                SecretStorePolicySelection {
                    store_id: STORE_ID_LINUX_SECRET_SERVICE.to_string(),
                    reason: SecretStoreSelectionReason::PlatformDefault,
                    native_store_id,
                    native_available,
                    warning: None,
                }
            } else {
                SecretStorePolicySelection {
                    store_id: STORE_ID_SQLCIPHER_INLINE.to_string(),
                    reason: SecretStoreSelectionReason::NativeUnavailable,
                    native_store_id,
                    native_available,
                    warning: Some(native_unavailable_message(&availability)),
                }
            }
        }
        SecretStorePlatform::Unsupported => SecretStorePolicySelection {
            store_id: STORE_ID_SQLCIPHER_INLINE.to_string(),
            reason: SecretStoreSelectionReason::UnsupportedPlatform,
            native_store_id,
            native_available,
            warning: Some("This platform has no supported OS credential store; AI keys stay in SQLCipher.".to_string()),
        },
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
            SecretStoreAvailability::Available {
                identity_strength:
                    IdentityStrength::Unsigned
                    | IdentityStrength::Adhoc
                    | IdentityStrength::UnknownOrUnsigned,
            },
        ) => Some(
            "Keychain storage is experimental for unsigned or ad-hoc macOS builds; rebuilding or app identity changes can trigger access prompts."
                .to_string(),
        ),
        _ => None,
    }
}

fn native_unavailable_message(availability: &SecretStoreAvailability) -> String {
    match availability {
        SecretStoreAvailability::LockedNeedsUnlock => {
            "The OS credential store is locked; AI keys stay in SQLCipher until it is unlocked."
                .to_string()
        }
        SecretStoreAvailability::Unavailable { reason } => {
            format!("The OS credential store is unavailable ({reason}); AI keys stay in SQLCipher.")
        }
        SecretStoreAvailability::Available { .. } => {
            "The requested OS credential store is not available on this platform.".to_string()
        }
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
fn linux_secret_service_store(
) -> keyring_core::Result<Arc<zbus_secret_service_keyring_store::Store>> {
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

pub fn compiled_keyring_backend_marker() -> &'static str {
    let _ = std::any::type_name::<keyring_core::Error>();
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
    fn availability_has_platform_policy_shape() {
        let availability = default_secret_store_availability();
        match availability {
            SecretStoreAvailability::Available { .. }
            | SecretStoreAvailability::LockedNeedsUnlock
            | SecretStoreAvailability::Unavailable { .. } => {}
        }
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
        assert!(selection
            .warning
            .unwrap()
            .contains("dbus_session_bus_missing"));
    }

    #[test]
    fn mock_store_roundtrips_without_host_keychain() {
        let store = MockSecretStore::new(SecretStoreAvailability::Available {
            identity_strength: IdentityStrength::Production,
        });
        assert_eq!(store.get("svc", "acct").unwrap(), None);
        store.set("svc", "acct", b"secret").unwrap();
        assert_eq!(store.get("svc", "acct").unwrap(), Some(b"secret".to_vec()));
        assert_eq!(store.list("svc").unwrap().len(), 1);
        store.delete("svc", "acct").unwrap();
        assert_eq!(store.get("svc", "acct").unwrap(), None);
    }
}

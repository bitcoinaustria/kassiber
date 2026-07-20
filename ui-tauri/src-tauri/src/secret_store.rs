use keyring_core::api::CredentialStoreApi;
use keyring_core::error::Error as KeyringError;
use keyring_core::Entry;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
#[cfg(any(target_os = "linux", test))]
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;

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
        }?;
        // apple-native-keyring-store 1.0.0's classic-Keychain backend drops
        // SecKeychainItemDelete's OSStatus. Verify absence so an ACL/signing
        // failure cannot be reported as successful credential revocation.
        match entry.get_secret() {
            Err(KeyringError::NoEntry) => Ok(()),
            Ok(_) => Err("The credential remained in the native store after deletion.".to_string()),
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
    fail_next_delete: Arc<Mutex<Option<String>>>,
}

#[cfg(test)]
impl MockSecretStore {
    pub fn new(availability: SecretStoreAvailability) -> Self {
        Self {
            availability: Arc::new(Mutex::new(availability)),
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            fail_next_set: Arc::new(Mutex::new(None)),
            fail_next_delete: Arc::new(Mutex::new(None)),
        }
    }

    pub fn fail_next_set(&self, message: &str) {
        *self.fail_next_set.lock().expect("mock fail lock") = Some(message.to_string());
    }

    pub fn fail_next_delete(&self, message: &str) {
        *self.fail_next_delete.lock().expect("mock delete fail lock") = Some(message.to_string());
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
        if let Some(message) = self
            .fail_next_delete
            .lock()
            .expect("mock delete fail lock")
            .take()
        {
            return Err(message);
        }
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
pub const LEGACY_SHARED_PASSPHRASE_SERVICE: &str = "Kassiber Database Passphrase";
pub const CLI_REMEMBERED_PASSPHRASE_SERVICE: &str = "Kassiber CLI Database Passphrase";
pub const DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE: &str = "Kassiber Desktop Biometric Passphrase";
pub const DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE: &str = "Kassiber Desktop Biometric Invalidated";
pub const OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE: &str = "Kassiber Operator Biometric Passphrase";
const OPERATOR_BIOMETRIC_MARKER_SERVICE: &str = "Kassiber Operator Biometric Enrollment";
const DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE: &str =
    "Kassiber Desktop Biometric Enrollment (Current Set)";
const DESKTOP_APPLICATION_GATE_MARKER_SERVICE: &str =
    "Kassiber Desktop Biometric Enrollment (Application Gate)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchIdProtection {
    BiometryCurrentSet,
    ApplicationLocalAuthentication,
    LegacyShared,
}

impl TouchIdProtection {
    fn marker_service(self) -> Option<&'static str> {
        match self {
            Self::BiometryCurrentSet => Some(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE),
            Self::ApplicationLocalAuthentication => Some(DESKTOP_APPLICATION_GATE_MARKER_SERVICE),
            Self::LegacyShared => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TouchIdPassphraseStatus {
    pub platform: SecretStorePlatform,
    pub available: bool,
    pub configured: bool,
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protection: Option<TouchIdProtection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "staleGeneration", skip_serializing_if = "Option::is_none")]
    pub stale_generation: Option<String>,
}

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

pub fn touch_id_passphrase_status(
    account: &str,
    cli_remembered_unlock_enabled: bool,
) -> TouchIdPassphraseStatus {
    let platform = current_secret_store_platform();
    if !cfg!(target_os = "macos") {
        return TouchIdPassphraseStatus {
            platform,
            available: false,
            configured: false,
            stale: false,
            protection: None,
            reason: Some(
                "Touch ID passphrase unlock is only available in the macOS desktop app."
                    .to_string(),
            ),
            stale_generation: None,
        };
    }

    match touch_id_biometrics_available() {
        Ok(()) => match desktop_biometric_passphrase_is_stale(account) {
            Ok(true) => TouchIdPassphraseStatus {
                platform,
                available: true,
                configured: false,
                stale: true,
                protection: None,
                reason: None,
                stale_generation: None,
            },
            Ok(false) => match desktop_biometric_enrollment(account, cli_remembered_unlock_enabled)
            {
                Ok(protection) => TouchIdPassphraseStatus {
                    platform,
                    available: true,
                    configured: protection.is_some(),
                    stale: false,
                    protection,
                    reason: None,
                    stale_generation: None,
                },
                Err(reason) => TouchIdPassphraseStatus {
                    platform,
                    available: true,
                    configured: false,
                    stale: false,
                    protection: None,
                    reason: Some(reason),
                    stale_generation: None,
                },
            },
            Err(reason) => TouchIdPassphraseStatus {
                platform,
                available: true,
                configured: false,
                stale: false,
                protection: None,
                reason: Some(reason),
                stale_generation: None,
            },
        },
        Err(reason) => TouchIdPassphraseStatus {
            platform,
            available: false,
            configured: false,
            stale: false,
            protection: None,
            reason: Some(reason),
            stale_generation: None,
        },
    }
}

pub fn touch_id_store_passphrase(account: &str, passphrase: &str) -> Result<(), String> {
    validate_touch_id_account(account)?;
    if passphrase.is_empty() {
        return Err("database passphrase must not be empty".to_string());
    }
    touch_id_biometrics_available()?;
    #[cfg(target_os = "macos")]
    let existing_marker = desktop_biometric_marker(account)?;

    #[cfg(target_os = "macos")]
    let protection = match store_biometry_current_set_passphrase(account, passphrase.as_bytes()) {
        Ok(()) => {
            if let Err(error) =
                NativeSecretStore.delete(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account)
            {
                return Err(rollback_desktop_passphrase_write(
                    account,
                    TouchIdProtection::BiometryCurrentSet,
                    format!("The preview Keychain fallback could not be removed: {error}"),
                ));
            }
            TouchIdProtection::BiometryCurrentSet
        }
        Err(error) if error.code() == ERR_SEC_MISSING_ENTITLEMENT => {
            if !application_fallback_allowed(existing_marker) {
                return Err(
                    "This preview build cannot replace an existing protected Touch ID item; \
                     reopen a production-entitled build to update or remove it."
                        .to_string(),
                );
            }
            NativeSecretStore.set(
                DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE,
                account,
                passphrase.as_bytes(),
            )?;
            TouchIdProtection::ApplicationLocalAuthentication
        }
        Err(error) => return Err(security_framework_error_for_user(error)),
    };
    #[cfg(not(target_os = "macos"))]
    let protection = TouchIdProtection::ApplicationLocalAuthentication;

    if let Err(error) = write_desktop_biometric_marker(account, protection) {
        return Err(rollback_desktop_passphrase_write(
            account, protection, error,
        ));
    }
    if let Err(error) = NativeSecretStore.delete(DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE, account) {
        let cleanup_error = rollback_desktop_passphrase_write(account, protection, error);
        let _ = delete_desktop_biometric_markers(account);
        return Err(cleanup_error);
    }
    Ok(())
}

pub fn touch_id_get_passphrase(
    account: &str,
    cli_remembered_unlock_enabled: bool,
) -> Result<Option<String>, String> {
    validate_touch_id_account(account)?;
    if desktop_biometric_passphrase_is_stale(account)? {
        return Ok(None);
    }
    let Some(protection) = desktop_biometric_enrollment(account, cli_remembered_unlock_enabled)?
    else {
        return Ok(None);
    };
    let secret = read_desktop_passphrase_for_protection(account, protection)?;
    secret
        .map(|value| {
            String::from_utf8(value)
                .map_err(|_| "stored database passphrase is not UTF-8".to_string())
        })
        .transpose()
}

pub fn touch_id_delete_passphrase(
    account: &str,
    cli_remembered_unlock_enabled: bool,
) -> Result<(), String> {
    validate_touch_id_account(account)?;
    let enrollment = desktop_biometric_enrollment(account, cli_remembered_unlock_enabled)?;
    delete_desktop_passphrase_copies(account, enrollment == Some(TouchIdProtection::LegacyShared))?;
    delete_desktop_biometric_markers(account)?;
    NativeSecretStore.delete(DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE, account)
}

#[cfg(target_os = "macos")]
pub fn operator_touch_id_store_passphrase(account: &str, passphrase: &[u8]) -> Result<(), String> {
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::passwords::{set_generic_password_options, AccessControlOptions};

    validate_touch_id_account(account)?;
    if passphrase.is_empty() {
        return Err("database passphrase must not be empty".to_string());
    }
    touch_id_biometrics_available()?;
    let mut options = protected_password_options(OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE, account);
    let access_control = SecAccessControl::create_with_protection(
        Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
        AccessControlOptions::BIOMETRY_CURRENT_SET.bits(),
    )
    .map_err(security_framework_error_for_user)?;
    options.set_access_control(access_control);
    set_generic_password_options(passphrase, options).map_err(security_framework_error_for_user)?;
    if let Err(error) = NativeSecretStore.set(OPERATOR_BIOMETRIC_MARKER_SERVICE, account, b"1") {
        let _ = operator_touch_id_delete_passphrase(account);
        return Err(error);
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn operator_touch_id_store_passphrase(
    _account: &str,
    _passphrase: &[u8],
) -> Result<(), String> {
    Err("Touch ID operator unlock is only available in the macOS desktop app.".to_string())
}

#[cfg(target_os = "macos")]
pub fn operator_touch_id_get_passphrase(account: &str) -> Result<Option<Vec<u8>>, String> {
    use security_framework::passwords::generic_password;

    validate_touch_id_account(account)?;
    touch_id_biometrics_available()?;
    match generic_password(protected_password_options(
        OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE,
        account,
    )) {
        Ok(secret) => Ok(Some(secret)),
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
        Err(error) => Err(security_framework_error_for_user(error)),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn operator_touch_id_get_passphrase(_account: &str) -> Result<Option<Vec<u8>>, String> {
    Err("Touch ID operator unlock is only available in the macOS desktop app.".to_string())
}

#[cfg(target_os = "macos")]
pub fn operator_touch_id_delete_passphrase(account: &str) -> Result<(), String> {
    use security_framework::passwords::delete_generic_password_options;

    validate_touch_id_account(account)?;
    match delete_generic_password_options(protected_password_options(
        OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE,
        account,
    )) {
        Ok(()) => {}
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => {}
        Err(error) => return Err(security_framework_error_for_user(error)),
    }
    NativeSecretStore.delete(OPERATOR_BIOMETRIC_MARKER_SERVICE, account)
}

#[cfg(not(target_os = "macos"))]
pub fn operator_touch_id_delete_passphrase(_account: &str) -> Result<(), String> {
    Err("Touch ID operator unlock is only available in the macOS desktop app.".to_string())
}

pub fn operator_touch_id_configured(account: &str) -> Result<bool, String> {
    validate_touch_id_account(account)?;
    if !cfg!(target_os = "macos") {
        return Err(
            "Touch ID operator unlock is only available in the macOS desktop app.".to_string(),
        );
    }
    operator_touch_id_configured_with_store(&NativeSecretStore, account)
}

fn operator_touch_id_configured_with_store(
    store: &dyn SecretStore,
    account: &str,
) -> Result<bool, String> {
    store.exists(OPERATOR_BIOMETRIC_MARKER_SERVICE, account)
}

fn validate_touch_id_account(account: &str) -> Result<(), String> {
    if account.trim().is_empty() {
        return Err("Touch ID account is missing.".to_string());
    }
    Ok(())
}

fn desktop_biometric_enrollment(
    account: &str,
    cli_remembered_unlock_enabled: bool,
) -> Result<Option<TouchIdProtection>, String> {
    validate_touch_id_account(account)?;
    if let Some(protection) = desktop_biometric_marker(account)? {
        return Ok(Some(protection));
    }
    if NativeSecretStore.exists(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account)? {
        return Ok(Some(TouchIdProtection::ApplicationLocalAuthentication));
    }
    if legacy_shared_item_belongs_to_desktop(cli_remembered_unlock_enabled)
        && NativeSecretStore.exists(LEGACY_SHARED_PASSPHRASE_SERVICE, account)?
    {
        return Ok(Some(TouchIdProtection::LegacyShared));
    }
    Ok(None)
}

fn desktop_biometric_marker(account: &str) -> Result<Option<TouchIdProtection>, String> {
    if NativeSecretStore.exists(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE, account)? {
        return Ok(Some(TouchIdProtection::BiometryCurrentSet));
    }
    if NativeSecretStore.exists(DESKTOP_APPLICATION_GATE_MARKER_SERVICE, account)? {
        return Ok(Some(TouchIdProtection::ApplicationLocalAuthentication));
    }
    Ok(None)
}

fn application_fallback_allowed(existing_marker: Option<TouchIdProtection>) -> bool {
    existing_marker != Some(TouchIdProtection::BiometryCurrentSet)
}

fn legacy_shared_item_belongs_to_desktop(cli_remembered_unlock_enabled: bool) -> bool {
    !cli_remembered_unlock_enabled
}

fn desktop_biometric_passphrase_is_stale(account: &str) -> Result<bool, String> {
    NativeSecretStore.exists(DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE, account)
}

fn write_desktop_biometric_marker(
    account: &str,
    protection: TouchIdProtection,
) -> Result<(), String> {
    delete_desktop_biometric_markers(account)?;
    let service = protection
        .marker_service()
        .ok_or_else(|| "legacy enrollment cannot be written as a current marker".to_string())?;
    NativeSecretStore.set(service, account, b"1")
}

fn delete_desktop_biometric_markers(account: &str) -> Result<(), String> {
    NativeSecretStore.delete(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE, account)?;
    NativeSecretStore.delete(DESKTOP_APPLICATION_GATE_MARKER_SERVICE, account)
}

fn read_desktop_passphrase_for_protection(
    account: &str,
    protection: TouchIdProtection,
) -> Result<Option<Vec<u8>>, String> {
    match protection {
        TouchIdProtection::BiometryCurrentSet => read_biometry_current_set_passphrase(account),
        TouchIdProtection::ApplicationLocalAuthentication => {
            authenticate_touch_id_for_passphrase()?;
            NativeSecretStore.get(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account)
        }
        TouchIdProtection::LegacyShared => {
            authenticate_touch_id_for_passphrase()?;
            let Some(secret) = NativeSecretStore.get(LEGACY_SHARED_PASSPHRASE_SERVICE, account)?
            else {
                return Ok(None);
            };
            let decoded = String::from_utf8(secret.clone())
                .map_err(|_| "stored database passphrase is not UTF-8".to_string())?;
            touch_id_store_passphrase(account, &decoded)?;
            if let Err(error) = NativeSecretStore.delete(LEGACY_SHARED_PASSPHRASE_SERVICE, account)
            {
                // Do not let a new marker hide a retained shared credential.
                // Roll the new enrollment back so the next attempt can retry
                // ownership transfer instead of silently orphaning the legacy
                // passphrase in Keychain.
                let rollback = touch_id_delete_passphrase(account, true);
                return Err(match rollback {
                    Ok(()) => format!(
                        "The legacy shared credential could not be removed; the new desktop enrollment was rolled back: {error}"
                    ),
                    Err(rollback_error) => format!(
                        "The legacy shared credential could not be removed ({error}), and the new desktop enrollment rollback was incomplete ({rollback_error})."
                    ),
                });
            }
            Ok(Some(secret))
        }
    }
}

fn delete_desktop_passphrase_copies(
    account: &str,
    delete_legacy_shared: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let delete_protected = delete_biometry_current_set_passphrase;
    #[cfg(not(target_os = "macos"))]
    let delete_protected = |_account: &str| Ok(());

    delete_desktop_passphrase_copies_with_store(
        &NativeSecretStore,
        account,
        delete_legacy_shared,
        delete_protected,
    )
}

fn delete_desktop_passphrase_copies_with_store<F>(
    store: &dyn SecretStore,
    account: &str,
    delete_legacy_shared: bool,
    mut delete_protected: F,
) -> Result<(), String>
where
    F: FnMut(&str) -> Result<(), String>,
{
    // Marker state cannot prove that the other Keychain domain is empty: an
    // interrupted mode transition may have left both copies behind. Always
    // attempt both current desktop stores, then report every partial failure.
    // The caller removes enrollment markers only after all cleanup succeeds.
    let mut failures = Vec::new();
    if let Err(error) = store.delete(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account) {
        failures.push(format!("preview fallback: {error}"));
    }
    if let Err(error) = delete_protected(account) {
        failures.push(format!("protected biometric item: {error}"));
    }
    if delete_legacy_shared {
        if let Err(error) = store.delete(LEGACY_SHARED_PASSPHRASE_SERVICE, account) {
            failures.push(format!("legacy shared item: {error}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Desktop credential cleanup is incomplete ({}). Enrollment markers were preserved; retry Forget Touch ID from a build that can access both stores.",
            failures.join("; ")
        ))
    }
}

fn rollback_desktop_passphrase_write(
    account: &str,
    protection: TouchIdProtection,
    error: String,
) -> String {
    let cleanup = match protection {
        TouchIdProtection::BiometryCurrentSet => delete_biometry_current_set_passphrase(account),
        TouchIdProtection::ApplicationLocalAuthentication => {
            NativeSecretStore.delete(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account)
        }
        TouchIdProtection::LegacyShared => {
            NativeSecretStore.delete(LEGACY_SHARED_PASSPHRASE_SERVICE, account)
        }
    };
    match cleanup {
        Ok(()) => error,
        Err(cleanup_error) => {
            format!("{error} The partially written credential also could not be removed: {cleanup_error}")
        }
    }
}

#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25_300;
#[cfg(target_os = "macos")]
const ERR_SEC_MISSING_ENTITLEMENT: i32 = -34_018;

#[cfg(target_os = "macos")]
fn protected_password_options(
    service: &str,
    account: &str,
) -> security_framework::passwords::PasswordOptions {
    use security_framework::passwords::PasswordOptions;

    let mut options = PasswordOptions::new_generic_password(service, account);
    options.use_protected_keychain();
    options
}

#[cfg(target_os = "macos")]
fn store_biometry_current_set_passphrase(
    account: &str,
    passphrase: &[u8],
) -> Result<(), security_framework::base::Error> {
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::passwords::{set_generic_password_options, AccessControlOptions};

    let mut options = protected_password_options(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account);
    let access_control = SecAccessControl::create_with_protection(
        Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
        AccessControlOptions::BIOMETRY_CURRENT_SET.bits(),
    )?;
    options.set_access_control(access_control);
    set_generic_password_options(passphrase, options)
}

#[cfg(target_os = "macos")]
fn read_biometry_current_set_passphrase(account: &str) -> Result<Option<Vec<u8>>, String> {
    use security_framework::passwords::generic_password;

    match generic_password(protected_password_options(
        DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE,
        account,
    )) {
        Ok(secret) => Ok(Some(secret)),
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => {
            let _ = delete_desktop_biometric_markers(account);
            Ok(None)
        }
        Err(error) => Err(security_framework_error_for_user(error)),
    }
}

#[cfg(not(target_os = "macos"))]
fn read_biometry_current_set_passphrase(_account: &str) -> Result<Option<Vec<u8>>, String> {
    Err("Touch ID passphrase unlock is only available in the macOS desktop app.".to_string())
}

#[cfg(target_os = "macos")]
fn delete_biometry_current_set_passphrase(account: &str) -> Result<(), String> {
    use security_framework::passwords::delete_generic_password_options;

    match delete_generic_password_options(protected_password_options(
        DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE,
        account,
    )) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
        Err(error) => Err(security_framework_error_for_user(error)),
    }
}

#[cfg(not(target_os = "macos"))]
fn delete_biometry_current_set_passphrase(_account: &str) -> Result<(), String> {
    Err("Touch ID passphrase unlock is only available in the macOS desktop app.".to_string())
}

#[cfg(target_os = "macos")]
fn security_framework_error_for_user(error: security_framework::base::Error) -> String {
    match error.code() {
        ERR_SEC_MISSING_ENTITLEMENT => {
            "This build cannot access the protected Keychain item; ".to_string()
                + "reopen a production-entitled build to manage it."
        }
        ERR_SEC_ITEM_NOT_FOUND => "No saved desktop biometric passphrase was found.".to_string(),
        code => format!("macOS protected Keychain rejected the operation ({code})."),
    }
}

#[cfg(target_os = "macos")]
fn authenticate_touch_id_for_passphrase() -> Result<(), String> {
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::{AnyObject, Bool};
    use objc2_foundation::NSString;
    use std::sync::mpsc;

    let context = touch_id_context_if_available()?;

    let reason = NSString::from_str("Unlock Kassiber with Touch ID");
    let (sender, receiver) = mpsc::channel();
    let reply = RcBlock::new(move |success: Bool, _error: *mut AnyObject| {
        let _ = sender.send(success.as_bool());
    });
    unsafe {
        let _: () = msg_send![
            &*context,
            evaluatePolicy: LAPOLICY_DEVICE_OWNER_AUTHENTICATION_WITH_BIOMETRICS,
            localizedReason: &*reason,
            reply: &*reply
        ];
    }

    match receiver.recv() {
        Ok(true) => Ok(()),
        Ok(false) => Err("Touch ID authentication was cancelled or failed.".to_string()),
        Err(_) => Err("Touch ID authentication did not return a result.".to_string()),
    }
}

#[cfg(target_os = "macos")]
fn touch_id_biometrics_available() -> Result<(), String> {
    touch_id_context_if_available().map(|_| ())
}

#[cfg(target_os = "macos")]
fn touch_id_context_if_available() -> Result<objc2::rc::Retained<objc2::runtime::AnyObject>, String>
{
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};

    let class = AnyClass::get(c"LAContext")
        .ok_or_else(|| "LocalAuthentication is unavailable on this Mac".to_string())?;
    let context: Retained<AnyObject> = unsafe { msg_send![class, new] };
    let mut can_evaluate_error: *mut AnyObject = std::ptr::null_mut();
    let can_evaluate: Bool = unsafe {
        msg_send![
            &*context,
            canEvaluatePolicy: LAPOLICY_DEVICE_OWNER_AUTHENTICATION_WITH_BIOMETRICS,
            error: &mut can_evaluate_error
        ]
    };
    if can_evaluate.as_bool() {
        Ok(context)
    } else {
        Err(
            ns_error_localized_description(can_evaluate_error).unwrap_or_else(|| {
                "Touch ID is not available or not enrolled for this Mac user.".to_string()
            }),
        )
    }
}

#[cfg(target_os = "macos")]
fn ns_error_localized_description(error: *mut objc2::runtime::AnyObject) -> Option<String> {
    if error.is_null() {
        return None;
    }
    use objc2::msg_send;
    use objc2_foundation::NSString;

    let description: *mut NSString = unsafe { msg_send![error, localizedDescription] };
    if description.is_null() {
        return None;
    }
    Some(unsafe { &*description }.to_string())
}

#[cfg(target_os = "macos")]
// LocalAuthentication.framework LAPolicy.deviceOwnerAuthenticationWithBiometrics.
const LAPOLICY_DEVICE_OWNER_AUTHENTICATION_WITH_BIOMETRICS: i64 = 1;

#[cfg(not(target_os = "macos"))]
fn touch_id_biometrics_available() -> Result<(), String> {
    Err("Touch ID passphrase unlock is only available in the macOS desktop app.".to_string())
}

#[cfg(not(target_os = "macos"))]
fn authenticate_touch_id_for_passphrase() -> Result<(), String> {
    Err("Touch ID passphrase unlock is only available in the macOS desktop app.".to_string())
}

#[cfg(not(target_os = "macos"))]
fn touch_id_passphrase_search(_account: &str) -> Result<bool, String> {
    Err("Touch ID passphrase unlock is only available in the macOS desktop app.".to_string())
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

    #[test]
    fn desktop_and_cli_passphrase_namespaces_are_distinct() {
        assert_eq!(
            LEGACY_SHARED_PASSPHRASE_SERVICE,
            "Kassiber Database Passphrase"
        );
        assert_eq!(
            CLI_REMEMBERED_PASSPHRASE_SERVICE,
            "Kassiber CLI Database Passphrase"
        );
        assert_eq!(
            DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE,
            "Kassiber Desktop Biometric Passphrase"
        );
        assert_eq!(
            DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE,
            "Kassiber Desktop Biometric Invalidated"
        );
        assert_ne!(
            CLI_REMEMBERED_PASSPHRASE_SERVICE,
            DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE
        );
        assert_ne!(
            OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE,
            DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE
        );
        assert_ne!(
            OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE,
            CLI_REMEMBERED_PASSPHRASE_SERVICE
        );
        assert_ne!(
            OPERATOR_BIOMETRIC_MARKER_SERVICE,
            OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE
        );
    }

    #[test]
    fn operator_touch_id_status_uses_a_non_prompting_marker() {
        let store = MockSecretStore::new(SecretStoreAvailability::Available {
            identity_strength: IdentityStrength::Production,
        });
        let account = "operator-project";
        assert!(!operator_touch_id_configured_with_store(&store, account).unwrap());
        store
            .set(OPERATOR_BIOMETRIC_MARKER_SERVICE, account, b"1")
            .unwrap();
        assert!(operator_touch_id_configured_with_store(&store, account).unwrap());
    }

    #[test]
    fn biometric_marker_services_are_mode_specific() {
        assert_eq!(
            TouchIdProtection::BiometryCurrentSet.marker_service(),
            Some(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE)
        );
        assert_eq!(
            TouchIdProtection::ApplicationLocalAuthentication.marker_service(),
            Some(DESKTOP_APPLICATION_GATE_MARKER_SERVICE)
        );
        assert_eq!(TouchIdProtection::LegacyShared.marker_service(), None);
        assert_ne!(
            DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE,
            DESKTOP_APPLICATION_GATE_MARKER_SERVICE
        );
    }

    #[test]
    fn legacy_shared_item_is_assigned_conservatively() {
        assert!(legacy_shared_item_belongs_to_desktop(false));
        assert!(!legacy_shared_item_belongs_to_desktop(true));
    }

    #[test]
    fn preview_fallback_never_replaces_a_protected_enrollment() {
        assert!(!application_fallback_allowed(Some(
            TouchIdProtection::BiometryCurrentSet
        )));
        assert!(application_fallback_allowed(Some(
            TouchIdProtection::ApplicationLocalAuthentication
        )));
        assert!(application_fallback_allowed(None));
    }

    #[test]
    fn protected_deletion_removes_preview_copy_before_protected_item() {
        let store = MockSecretStore::new(SecretStoreAvailability::Available {
            identity_strength: IdentityStrength::Production,
        });
        let account = "book";
        store
            .set(
                DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE,
                account,
                b"preview-copy",
            )
            .unwrap();
        let mut protected_deleted = false;

        delete_desktop_passphrase_copies_with_store(&store, account, false, |_| {
            protected_deleted = true;
            Ok(())
        })
        .unwrap();

        assert!(protected_deleted);
        assert_eq!(
            store
                .get(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account)
                .unwrap(),
            None
        );
    }

    #[test]
    fn protected_deletion_is_still_attempted_when_preview_cleanup_fails() {
        let store = MockSecretStore::new(SecretStoreAvailability::Available {
            identity_strength: IdentityStrength::Production,
        });
        let account = "book";
        store
            .set(
                DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE,
                account,
                b"preview-copy",
            )
            .unwrap();
        store.fail_next_delete("preview cleanup failed");
        let mut protected_deleted = false;

        let result = delete_desktop_passphrase_copies_with_store(&store, account, false, |_| {
            protected_deleted = true;
            Ok(())
        });

        assert!(result
            .unwrap_err()
            .contains("preview fallback: preview cleanup failed"));
        assert!(protected_deleted);
        assert_eq!(
            store
                .get(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account)
                .unwrap(),
            Some(b"preview-copy".to_vec())
        );
    }
}

use serde::Serialize;
use std::collections::BTreeMap;

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

pub trait SecretStore {
    fn availability(&self) -> SecretStoreAvailability;
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, String>;
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
        Err("native secret storage is probe-only in this build".to_string())
    }

    fn set(&self, _service: &str, _account: &str, _secret: &[u8]) -> Result<(), String> {
        Err("native secret storage is probe-only in this build".to_string())
    }

    fn delete(&self, _service: &str, _account: &str) -> Result<(), String> {
        Err("native secret storage is probe-only in this build".to_string())
    }

    fn list(&self, _service: &str) -> Result<Vec<SecretStoreEntryRef>, String> {
        Err("native secret storage is probe-only in this build".to_string())
    }
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

fn macos_identity_strength() -> IdentityStrength {
    if option_env!("KASSIBER_PRODUCTION_SIGNED").is_some() {
        IdentityStrength::Production
    } else if option_env!("KASSIBER_ADHOC_SIGNED").is_some() {
        IdentityStrength::Adhoc
    } else {
        IdentityStrength::UnknownOrUnsigned
    }
}

fn linux_secret_service_availability() -> SecretStoreAvailability {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        return SecretStoreAvailability::Unavailable {
            reason: "dbus_session_bus_missing".to_string(),
        };
    }
    SecretStoreAvailability::Available {
        identity_strength: IdentityStrength::UnknownOrUnsigned,
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
}

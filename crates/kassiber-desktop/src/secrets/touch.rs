//! Touch ID credentials (secret_store.rs's `touch_id_*` and `operator_touch_id_*`), over a
//! [`Keychain`]: the generic store plus [`Biometric`]. The logic, service names, markers, the
//! preview fallback, legacy-item migration and rollback rules are the Tauri shell's, so items it
//! wrote are read, migrated and removed the same way.
//!
//! Desktop Touch ID (per data root, the canonical data-root path is the Keychain account):
//! - Production-entitled builds keep the passphrase as a data-protection item bound to the
//!   current biometry set (`BiometryCurrentSet`); the marker service records that mode.
//! - Builds without the Keychain access group (unsigned, ad-hoc, or signed without the
//!   provisioning profile) get `errSecMissingEntitlement` and fall back to a login-Keychain
//!   copy that the app reads only after its own LocalAuthentication check
//!   (`ApplicationLocalAuthentication`), never replacing a protected enrollment.
//! - A legacy item shared with the CLI (`Kassiber Database Passphrase`) belongs to the desktop
//!   only while the CLI's remembered unlock is off; its first use migrates it.
//! - A stale marker (the passphrase changed elsewhere) hides any enrollment until re-enrolled.

use serde::Serialize;
use zeroize::Zeroizing;

use super::biometric::{Biometric, ProtectedError};
use super::store::{SecretStore, SecretStorePlatform, current_secret_store_platform, zero};

pub const LEGACY_SHARED_PASSPHRASE_SERVICE: &str = "Kassiber Database Passphrase";
#[allow(dead_code)] // the CLI's own item: named so the namespaces stay distinct (tests)
pub const CLI_REMEMBERED_PASSPHRASE_SERVICE: &str = "Kassiber CLI Database Passphrase";
pub const DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE: &str = "Kassiber Desktop Biometric Passphrase";
pub const DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE: &str = "Kassiber Desktop Biometric Invalidated";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
pub const OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE: &str = "Kassiber Operator Biometric Passphrase";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
pub const OPERATOR_BIOMETRIC_MARKER_SERVICE: &str = "Kassiber Operator Biometric Enrollment";
pub const DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE: &str =
    "Kassiber Desktop Biometric Enrollment (Current Set)";
pub const DESKTOP_APPLICATION_GATE_MARKER_SERVICE: &str =
    "Kassiber Desktop Biometric Enrollment (Application Gate)";

#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
const OPERATOR_DESKTOP_ONLY: &str =
    "Touch ID operator unlock is only available in the macOS desktop app.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchIdProtection {
    BiometryCurrentSet,
    ApplicationLocalAuthentication,
    LegacyShared,
}

impl TouchIdProtection {
    pub fn marker_service(self) -> Option<&'static str> {
        match self {
            Self::BiometryCurrentSet => Some(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE),
            Self::ApplicationLocalAuthentication => Some(DESKTOP_APPLICATION_GATE_MARKER_SERVICE),
            Self::LegacyShared => None,
        }
    }
}

/// `TouchIdPassphraseStatus`, field for field (the web's `TouchIdPassphraseStatus`).
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

impl TouchIdPassphraseStatus {
    fn new(platform: SecretStorePlatform, available: bool) -> Self {
        TouchIdPassphraseStatus {
            platform,
            available,
            configured: false,
            stale: false,
            protection: None,
            reason: None,
            stale_generation: None,
        }
    }
}

/// The stores Touch ID works against. `macos` is false off a Mac (and in tests that say so).
pub struct Keychain<'a> {
    pub store: &'a dyn SecretStore,
    pub bio: &'a dyn Biometric,
    pub macos: bool,
}

impl Keychain<'_> {
    // ---- Desktop Touch ID -------------------------------------------------------------------

    pub fn status(
        &self,
        account: &str,
        cli_remembered_unlock_enabled: bool,
    ) -> TouchIdPassphraseStatus {
        let platform = if self.macos {
            SecretStorePlatform::Macos
        } else {
            current_secret_store_platform()
        };
        if !self.macos {
            let mut s = TouchIdPassphraseStatus::new(platform, false);
            s.reason = Some(
                "Touch ID passphrase unlock is only available in the macOS desktop app."
                    .to_string(),
            );
            return s;
        }
        match self.bio.available() {
            Ok(()) => match self.is_stale(account) {
                Ok(true) => {
                    let mut s = TouchIdPassphraseStatus::new(platform, true);
                    s.stale = true;
                    s
                }
                Ok(false) => match self.enrollment(account, cli_remembered_unlock_enabled) {
                    Ok(protection) => {
                        let mut s = TouchIdPassphraseStatus::new(platform, true);
                        s.configured = protection.is_some();
                        s.protection = protection;
                        s
                    }
                    Err(reason) => {
                        let mut s = TouchIdPassphraseStatus::new(platform, true);
                        s.reason = Some(reason);
                        s
                    }
                },
                Err(reason) => {
                    let mut s = TouchIdPassphraseStatus::new(platform, true);
                    s.reason = Some(reason);
                    s
                }
            },
            Err(reason) => {
                let mut s = TouchIdPassphraseStatus::new(platform, false);
                s.reason = Some(reason);
                s
            }
        }
    }

    /// `touch_id_store_passphrase`.
    pub fn store_passphrase(&self, account: &str, passphrase: &str) -> Result<(), String> {
        validate_touch_id_account(account)?;
        if passphrase.is_empty() {
            return Err("database passphrase must not be empty".to_string());
        }
        self.bio.available()?;
        let protection = if self.macos {
            let existing_marker = self.marker(account)?;
            match self.bio.protected_set(
                DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE,
                account,
                passphrase.as_bytes(),
            ) {
                Ok(()) => {
                    if let Err(error) = self
                        .store
                        .delete(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account)
                    {
                        return Err(self.rollback_write(
                            account,
                            TouchIdProtection::BiometryCurrentSet,
                            format!("The preview Keychain fallback could not be removed: {error}"),
                        ));
                    }
                    TouchIdProtection::BiometryCurrentSet
                }
                Err(ProtectedError::MissingEntitlement) => {
                    if !application_fallback_allowed(existing_marker) {
                        return Err("This preview build cannot replace an existing protected Touch ID item; reopen a production-entitled build to update or remove it.".to_string());
                    }
                    self.store.set(
                        DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE,
                        account,
                        passphrase.as_bytes(),
                    )?;
                    TouchIdProtection::ApplicationLocalAuthentication
                }
                Err(error) => return Err(error.for_user()),
            }
        } else {
            TouchIdProtection::ApplicationLocalAuthentication
        };

        if let Err(error) = self.write_marker(account, protection) {
            return Err(self.rollback_write(account, protection, error));
        }
        if let Err(error) = self
            .store
            .delete(DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE, account)
        {
            let cleanup_error = self.rollback_write(account, protection, error);
            let _ = self.delete_markers(account);
            return Err(cleanup_error);
        }
        Ok(())
    }

    /// `touch_id_get_passphrase`: None when nothing (current) is saved.
    pub fn get_passphrase(
        &self,
        account: &str,
        cli_remembered_unlock_enabled: bool,
    ) -> Result<Option<Zeroizing<String>>, String> {
        validate_touch_id_account(account)?;
        if self.is_stale(account)? {
            return Ok(None);
        }
        let Some(protection) = self.enrollment(account, cli_remembered_unlock_enabled)? else {
            return Ok(None);
        };
        let Some(secret) = self.read_for_protection(account, protection)? else {
            return Ok(None);
        };
        utf8(secret).map(Some)
    }

    /// `touch_id_delete_passphrase`.
    pub fn delete_passphrase(
        &self,
        account: &str,
        cli_remembered_unlock_enabled: bool,
    ) -> Result<(), String> {
        validate_touch_id_account(account)?;
        let enrollment = self.enrollment(account, cli_remembered_unlock_enabled)?;
        self.delete_copies(account, enrollment == Some(TouchIdProtection::LegacyShared))?;
        self.delete_markers(account)?;
        self.store
            .delete(DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE, account)
    }

    fn enrollment(
        &self,
        account: &str,
        cli_remembered_unlock_enabled: bool,
    ) -> Result<Option<TouchIdProtection>, String> {
        validate_touch_id_account(account)?;
        if let Some(protection) = self.marker(account)? {
            return Ok(Some(protection));
        }
        if self
            .store
            .exists(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account)?
        {
            return Ok(Some(TouchIdProtection::ApplicationLocalAuthentication));
        }
        if legacy_shared_item_belongs_to_desktop(cli_remembered_unlock_enabled)
            && self
                .store
                .exists(LEGACY_SHARED_PASSPHRASE_SERVICE, account)?
        {
            return Ok(Some(TouchIdProtection::LegacyShared));
        }
        Ok(None)
    }

    fn marker(&self, account: &str) -> Result<Option<TouchIdProtection>, String> {
        if self
            .store
            .exists(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE, account)?
        {
            return Ok(Some(TouchIdProtection::BiometryCurrentSet));
        }
        if self
            .store
            .exists(DESKTOP_APPLICATION_GATE_MARKER_SERVICE, account)?
        {
            return Ok(Some(TouchIdProtection::ApplicationLocalAuthentication));
        }
        Ok(None)
    }

    fn is_stale(&self, account: &str) -> Result<bool, String> {
        self.store
            .exists(DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE, account)
    }

    fn write_marker(&self, account: &str, protection: TouchIdProtection) -> Result<(), String> {
        self.delete_markers(account)?;
        let service = protection
            .marker_service()
            .ok_or_else(|| "legacy enrollment cannot be written as a current marker".to_string())?;
        self.store.set(service, account, b"1")
    }

    fn delete_markers(&self, account: &str) -> Result<(), String> {
        self.store
            .delete(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE, account)?;
        self.store
            .delete(DESKTOP_APPLICATION_GATE_MARKER_SERVICE, account)
    }

    fn read_for_protection(
        &self,
        account: &str,
        protection: TouchIdProtection,
    ) -> Result<Option<Vec<u8>>, String> {
        match protection {
            TouchIdProtection::BiometryCurrentSet => self.read_biometry_current_set(account),
            TouchIdProtection::ApplicationLocalAuthentication => {
                self.bio.authenticate()?;
                self.store
                    .get(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account)
            }
            TouchIdProtection::LegacyShared => {
                self.bio.authenticate()?;
                let Some(secret) = self.store.get(LEGACY_SHARED_PASSPHRASE_SERVICE, account)?
                else {
                    return Ok(None);
                };
                let decoded = utf8(secret.clone())?;
                self.store_passphrase(account, &decoded)?;
                drop(decoded);
                if let Err(error) = self.store.delete(LEGACY_SHARED_PASSPHRASE_SERVICE, account) {
                    // Do not let a new marker hide a retained shared credential. Roll the new
                    // enrollment back so the next attempt can retry ownership transfer instead of
                    // silently orphaning the legacy passphrase in Keychain.
                    let mut secret = secret;
                    zero(&mut secret);
                    let rollback = self.delete_passphrase(account, true);
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

    fn read_biometry_current_set(&self, account: &str) -> Result<Option<Vec<u8>>, String> {
        if !self.macos {
            return Err(ProtectedError::Unsupported.for_user());
        }
        match self
            .bio
            .protected_get(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account)
        {
            Ok(secret) => Ok(Some(secret)),
            Err(ProtectedError::NotFound) => {
                let _ = self.delete_markers(account);
                Ok(None)
            }
            Err(error) => Err(error.for_user()),
        }
    }

    fn delete_biometry_current_set(&self, account: &str) -> Result<(), String> {
        if !self.macos {
            return Err(ProtectedError::Unsupported.for_user());
        }
        match self
            .bio
            .protected_delete(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account)
        {
            Ok(()) | Err(ProtectedError::NotFound) => Ok(()),
            Err(error) => Err(error.for_user()),
        }
    }

    fn delete_copies(&self, account: &str, delete_legacy_shared: bool) -> Result<(), String> {
        let macos = self.macos;
        delete_desktop_passphrase_copies_with_store(
            self.store,
            account,
            delete_legacy_shared,
            |account| {
                if macos {
                    self.delete_biometry_current_set(account)
                } else {
                    Ok(())
                }
            },
        )
    }

    fn rollback_write(
        &self,
        account: &str,
        protection: TouchIdProtection,
        error: String,
    ) -> String {
        let cleanup = match protection {
            TouchIdProtection::BiometryCurrentSet => self.delete_biometry_current_set(account),
            TouchIdProtection::ApplicationLocalAuthentication => self
                .store
                .delete(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account),
            TouchIdProtection::LegacyShared => {
                self.store.delete(LEGACY_SHARED_PASSPHRASE_SERVICE, account)
            }
        };
        match cleanup {
            Ok(()) => error,
            Err(cleanup_error) => format!(
                "{error} The partially written credential also could not be removed: {cleanup_error}"
            ),
        }
    }

    // ---- Operator Touch ID (the broker's helper) --------------------------------------------

    /// `operator_touch_id_store_passphrase`: no preview fallback, ever.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
    pub fn operator_store(&self, account: &str, passphrase: &[u8]) -> Result<(), String> {
        if !self.macos {
            return Err(OPERATOR_DESKTOP_ONLY.to_string());
        }
        validate_operator_touch_id_account(account)?;
        if passphrase.is_empty() {
            return Err("database passphrase must not be empty".to_string());
        }
        self.bio.available()?;
        self.bio
            .protected_set(OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE, account, passphrase)
            .map_err(|e| e.for_user())?;
        if let Err(error) = self
            .store
            .set(OPERATOR_BIOMETRIC_MARKER_SERVICE, account, b"1")
        {
            let _ = self.operator_delete(account);
            return Err(error);
        }
        Ok(())
    }

    /// `operator_touch_id_get_passphrase`.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
    pub fn operator_get(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        if !self.macos {
            return Err(OPERATOR_DESKTOP_ONLY.to_string());
        }
        validate_operator_touch_id_account(account)?;
        self.bio.available()?;
        match self
            .bio
            .protected_get(OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE, account)
        {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(ProtectedError::NotFound) => Ok(None),
            Err(error) => Err(error.for_user()),
        }
    }

    /// `operator_touch_id_delete_passphrase`.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
    pub fn operator_delete(&self, account: &str) -> Result<(), String> {
        if !self.macos {
            return Err(OPERATOR_DESKTOP_ONLY.to_string());
        }
        validate_operator_touch_id_account(account)?;
        match self
            .bio
            .protected_delete(OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE, account)
        {
            Ok(()) | Err(ProtectedError::NotFound) => {}
            Err(error) => return Err(error.for_user()),
        }
        self.store
            .delete(OPERATOR_BIOMETRIC_MARKER_SERVICE, account)
    }

    /// `operator_touch_id_configured`: a non-prompting marker check.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
    pub fn operator_configured(&self, account: &str) -> Result<bool, String> {
        validate_operator_touch_id_account(account)?;
        if !self.macos {
            return Err(OPERATOR_DESKTOP_ONLY.to_string());
        }
        self.store
            .exists(OPERATOR_BIOMETRIC_MARKER_SERVICE, account)
    }
}

/// A stored passphrase as text; the bytes are cleared either way.
fn utf8(mut secret: Vec<u8>) -> Result<Zeroizing<String>, String> {
    match std::str::from_utf8(&secret) {
        Ok(text) => {
            let out = Zeroizing::new(text.to_owned());
            zero(&mut secret);
            Ok(out)
        }
        Err(_) => {
            zero(&mut secret);
            Err("stored database passphrase is not UTF-8".to_string())
        }
    }
}

pub fn validate_touch_id_account(account: &str) -> Result<(), String> {
    if account.trim().is_empty() {
        return Err("Touch ID account is missing.".to_string());
    }
    Ok(())
}

/// The broker names an operator item by a lowercase SHA-256 digest.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
pub fn validate_operator_touch_id_account(account: &str) -> Result<(), String> {
    if account.len() != 64
        || !account
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("Touch ID operator account is invalid.".to_string());
    }
    Ok(())
}

fn application_fallback_allowed(existing_marker: Option<TouchIdProtection>) -> bool {
    existing_marker != Some(TouchIdProtection::BiometryCurrentSet)
}

fn legacy_shared_item_belongs_to_desktop(cli_remembered_unlock_enabled: bool) -> bool {
    !cli_remembered_unlock_enabled
}

/// Marker state cannot prove that the other Keychain domain is empty: an interrupted mode
/// transition may have left both copies behind. Always attempt both current desktop stores,
/// then report every partial failure. The caller removes enrollment markers only after all
/// cleanup succeeds.
fn delete_desktop_passphrase_copies_with_store<F>(
    store: &dyn SecretStore,
    account: &str,
    delete_legacy_shared: bool,
    mut delete_protected: F,
) -> Result<(), String>
where
    F: FnMut(&str) -> Result<(), String>,
{
    let mut failures = Vec::new();
    if let Err(error) = store.delete(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, account) {
        failures.push(format!("preview fallback: {error}"));
    }
    if let Err(error) = delete_protected(account) {
        failures.push(format!("protected biometric item: {error}"));
    }
    if delete_legacy_shared
        && let Err(error) = store.delete(LEGACY_SHARED_PASSPHRASE_SERVICE, account)
    {
        failures.push(format!("legacy shared item: {error}"));
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

#[cfg(test)]
mod tests {
    use super::super::biometric::Fake;
    use super::super::store::{IdentityStrength, MemoryStore, SecretStoreAvailability};
    use super::*;

    fn store() -> MemoryStore {
        MemoryStore::new(SecretStoreAvailability::Available {
            identity_strength: IdentityStrength::Production,
        })
    }

    const ACCOUNT: &str = "/books/demo/data";

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
        assert_eq!(
            OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE,
            "Kassiber Operator Biometric Passphrase"
        );
        assert_eq!(
            OPERATOR_BIOMETRIC_MARKER_SERVICE,
            "Kassiber Operator Biometric Enrollment"
        );
        assert_eq!(
            DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE,
            "Kassiber Desktop Biometric Enrollment (Current Set)"
        );
        assert_eq!(
            DESKTOP_APPLICATION_GATE_MARKER_SERVICE,
            "Kassiber Desktop Biometric Enrollment (Application Gate)"
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
            OPERATOR_BIOMETRIC_MARKER_SERVICE,
            OPERATOR_BIOMETRIC_PASSPHRASE_SERVICE
        );
    }

    #[test]
    fn off_a_mac_touch_id_is_unavailable() {
        let s = store();
        let bio = Fake::new(Ok(()), true);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: false,
        };
        let status = kc.status(ACCOUNT, false);
        assert!(!status.available && !status.configured);
        assert!(status.reason.unwrap().contains("macOS desktop app"));
        assert!(kc.operator_configured(&"a".repeat(64)).is_err());
    }

    #[test]
    fn unavailable_biometrics_report_the_system_reason() {
        let s = store();
        let bio = Fake::new(Err("Biometry is not enrolled.".into()), true);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        let status = kc.status(ACCOUNT, false);
        assert_eq!((status.available, status.configured), (false, false));
        assert_eq!(status.reason.as_deref(), Some("Biometry is not enrolled."));
        assert_eq!(
            kc.store_passphrase(ACCOUNT, "pass"),
            Err("Biometry is not enrolled.".into())
        );
    }

    #[test]
    fn entitled_build_enrolls_a_biometry_bound_item_and_reads_it_back() {
        let s = store();
        let bio = Fake::new(Ok(()), true);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        assert!(!kc.status(ACCOUNT, false).configured);
        kc.store_passphrase(ACCOUNT, "correct horse").unwrap();
        let status = kc.status(ACCOUNT, false);
        assert!(status.available && status.configured);
        assert_eq!(
            status.protection,
            Some(TouchIdProtection::BiometryCurrentSet)
        );
        // The secret lives only in the protected item; the login store holds the marker alone.
        assert_eq!(
            s.get(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, ACCOUNT)
                .unwrap(),
            None
        );
        assert!(
            s.exists(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE, ACCOUNT)
                .unwrap()
        );
        assert_eq!(
            kc.get_passphrase(ACCOUNT, false)
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("correct horse")
        );
        kc.delete_passphrase(ACCOUNT, false).unwrap();
        let status = kc.status(ACCOUNT, false);
        assert!(!status.configured);
        assert!(bio.items.lock().unwrap().is_empty());
        assert!(
            s.list(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn unentitled_build_falls_back_to_the_app_gated_login_item() {
        let s = store();
        let bio = Fake::new(Ok(()), false);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        kc.store_passphrase(ACCOUNT, "preview pass").unwrap();
        let status = kc.status(ACCOUNT, false);
        assert_eq!(
            status.protection,
            Some(TouchIdProtection::ApplicationLocalAuthentication)
        );
        assert!(
            s.exists(DESKTOP_APPLICATION_GATE_MARKER_SERVICE, ACCOUNT)
                .unwrap()
        );
        assert_eq!(
            kc.get_passphrase(ACCOUNT, false)
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("preview pass")
        );
    }

    #[test]
    fn app_gated_read_requires_a_successful_touch_id_check() {
        let s = store();
        let mut bio = Fake::new(Ok(()), false);
        bio.authenticate = Err("Touch ID authentication was cancelled or failed.".into());
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        kc.store_passphrase(ACCOUNT, "preview pass").unwrap();
        assert_eq!(
            kc.get_passphrase(ACCOUNT, false).map(|o| o.is_some()),
            Err("Touch ID authentication was cancelled or failed.".into())
        );
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
        // A production enrollment, then a preview build tries to re-enroll.
        let s = store();
        s.set(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE, ACCOUNT, b"1")
            .unwrap();
        let bio = Fake::new(Ok(()), false);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        assert!(
            kc.store_passphrase(ACCOUNT, "x")
                .unwrap_err()
                .contains("cannot replace an existing protected Touch ID item")
        );
        assert_eq!(
            s.get(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, ACCOUNT)
                .unwrap(),
            None
        );
    }

    #[test]
    fn a_stale_marker_hides_the_enrollment_until_re_enrolled() {
        let s = store();
        let bio = Fake::new(Ok(()), true);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        kc.store_passphrase(ACCOUNT, "old").unwrap();
        s.set(DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE, ACCOUNT, b"1")
            .unwrap();
        let status = kc.status(ACCOUNT, false);
        assert!(status.stale && !status.configured);
        assert_eq!(kc.get_passphrase(ACCOUNT, false).unwrap(), None);
        kc.store_passphrase(ACCOUNT, "new").unwrap();
        assert!(
            !s.exists(DESKTOP_BIOMETRIC_STALE_MARKER_SERVICE, ACCOUNT)
                .unwrap()
        );
        assert!(kc.status(ACCOUNT, false).configured);
    }

    #[test]
    fn legacy_shared_item_is_assigned_conservatively_and_migrated_on_use() {
        assert!(legacy_shared_item_belongs_to_desktop(false));
        assert!(!legacy_shared_item_belongs_to_desktop(true));
        let s = store();
        s.set(LEGACY_SHARED_PASSPHRASE_SERVICE, ACCOUNT, b"shared")
            .unwrap();
        let bio = Fake::new(Ok(()), true);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        // While the CLI remembers the unlock, the shared item is the CLI's.
        assert!(!kc.status(ACCOUNT, true).configured);
        assert_eq!(
            kc.status(ACCOUNT, false).protection,
            Some(TouchIdProtection::LegacyShared)
        );
        assert_eq!(
            kc.get_passphrase(ACCOUNT, false)
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("shared")
        );
        assert_eq!(
            s.get(LEGACY_SHARED_PASSPHRASE_SERVICE, ACCOUNT).unwrap(),
            None
        );
        assert_eq!(
            kc.status(ACCOUNT, false).protection,
            Some(TouchIdProtection::BiometryCurrentSet)
        );
    }

    #[test]
    fn a_retained_legacy_item_rolls_the_migration_back() {
        let s = store();
        s.set(LEGACY_SHARED_PASSPHRASE_SERVICE, ACCOUNT, b"shared")
            .unwrap();
        let bio = Fake::new(Ok(()), true);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        // The Keychain refuses to delete the legacy item.
        let failing = FailLegacyDelete { inner: &s };
        let kc2 = Keychain {
            store: &failing,
            bio: kc.bio,
            macos: true,
        };
        let err = kc2.get_passphrase(ACCOUNT, false).unwrap_err();
        assert!(
            err.contains("legacy shared credential could not be removed"),
            "{err}"
        );
        assert!(s.exists(LEGACY_SHARED_PASSPHRASE_SERVICE, ACCOUNT).unwrap());
        assert!(
            !s.exists(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE, ACCOUNT)
                .unwrap()
        );
    }

    struct FailLegacyDelete<'a> {
        inner: &'a MemoryStore,
    }

    impl SecretStore for FailLegacyDelete<'_> {
        fn availability(&self) -> super::super::store::SecretStoreAvailability {
            self.inner.availability()
        }
        fn get(&self, s: &str, a: &str) -> Result<Option<Vec<u8>>, String> {
            self.inner.get(s, a)
        }
        fn exists(&self, s: &str, a: &str) -> Result<bool, String> {
            self.inner.exists(s, a)
        }
        fn set(&self, s: &str, a: &str, v: &[u8]) -> Result<(), String> {
            self.inner.set(s, a, v)
        }
        fn delete(&self, s: &str, a: &str) -> Result<(), String> {
            if s == LEGACY_SHARED_PASSPHRASE_SERVICE {
                return Err("denied".into());
            }
            self.inner.delete(s, a)
        }
        fn list(&self, s: &str) -> Result<Vec<super::super::store::SecretStoreEntryRef>, String> {
            self.inner.list(s)
        }
    }

    #[test]
    fn a_missing_protected_item_clears_its_markers() {
        let s = store();
        s.set(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE, ACCOUNT, b"1")
            .unwrap();
        let bio = Fake::new(Ok(()), true);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        assert_eq!(kc.get_passphrase(ACCOUNT, false).unwrap(), None);
        assert!(
            !s.exists(DESKTOP_BIOMETRY_CURRENT_SET_MARKER_SERVICE, ACCOUNT)
                .unwrap()
        );
    }

    #[test]
    fn protected_deletion_removes_preview_copy_before_protected_item() {
        let s = store();
        s.set(
            DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE,
            "book",
            b"preview-copy",
        )
        .unwrap();
        let mut protected_deleted = false;
        delete_desktop_passphrase_copies_with_store(&s, "book", false, |_| {
            protected_deleted = true;
            Ok(())
        })
        .unwrap();
        assert!(protected_deleted);
        assert_eq!(
            s.get(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, "book").unwrap(),
            None
        );
    }

    #[test]
    fn protected_deletion_is_still_attempted_when_preview_cleanup_fails() {
        let s = store();
        s.set(
            DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE,
            "book",
            b"preview-copy",
        )
        .unwrap();
        s.fail_next_delete("preview cleanup failed");
        let mut protected_deleted = false;
        let result = delete_desktop_passphrase_copies_with_store(&s, "book", false, |_| {
            protected_deleted = true;
            Ok(())
        });
        assert!(
            result
                .unwrap_err()
                .contains("preview fallback: preview cleanup failed")
        );
        assert!(protected_deleted);
        assert_eq!(
            s.get(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, "book").unwrap(),
            Some(b"preview-copy".to_vec())
        );
    }

    #[test]
    fn a_failed_marker_write_rolls_the_secret_back() {
        let s = store();
        let bio = Fake::new(Ok(()), false);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        // The fallback write succeeds; the marker write is the next set.
        let failing = FailNthSet {
            inner: &s,
            n: std::sync::atomic::AtomicUsize::new(0),
        };
        let kc2 = Keychain {
            store: &failing,
            bio: kc.bio,
            macos: true,
        };
        assert_eq!(
            kc2.store_passphrase(ACCOUNT, "p"),
            Err("marker denied".into())
        );
        assert_eq!(
            s.get(DESKTOP_BIOMETRIC_PASSPHRASE_SERVICE, ACCOUNT)
                .unwrap(),
            None
        );
    }

    struct FailNthSet<'a> {
        inner: &'a MemoryStore,
        n: std::sync::atomic::AtomicUsize,
    }

    impl SecretStore for FailNthSet<'_> {
        fn availability(&self) -> super::super::store::SecretStoreAvailability {
            self.inner.availability()
        }
        fn get(&self, s: &str, a: &str) -> Result<Option<Vec<u8>>, String> {
            self.inner.get(s, a)
        }
        fn exists(&self, s: &str, a: &str) -> Result<bool, String> {
            self.inner.exists(s, a)
        }
        fn set(&self, s: &str, a: &str, v: &[u8]) -> Result<(), String> {
            if self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                return Err("marker denied".into());
            }
            self.inner.set(s, a, v)
        }
        fn delete(&self, s: &str, a: &str) -> Result<(), String> {
            self.inner.delete(s, a)
        }
        fn list(&self, s: &str) -> Result<Vec<super::super::store::SecretStoreEntryRef>, String> {
            self.inner.list(s)
        }
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
    }

    #[test]
    fn operator_touch_id_status_uses_a_non_prompting_marker() {
        let s = store();
        let bio = Fake::new(Ok(()), true);
        let kc = Keychain {
            store: &s,
            bio: &bio,
            macos: true,
        };
        let account = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(!kc.operator_configured(account).unwrap());
        s.set(OPERATOR_BIOMETRIC_MARKER_SERVICE, account, b"1")
            .unwrap();
        assert!(kc.operator_configured(account).unwrap());
    }

    #[test]
    fn operator_items_have_no_preview_fallback() {
        let s = store();
        let account = "a".repeat(64);
        let unentitled = Fake::new(Ok(()), false);
        let kc = Keychain {
            store: &s,
            bio: &unentitled,
            macos: true,
        };
        assert!(
            kc.operator_store(&account, b"pass")
                .unwrap_err()
                .contains("production-entitled")
        );
        assert!(
            !s.exists(OPERATOR_BIOMETRIC_MARKER_SERVICE, &account)
                .unwrap()
        );
        let entitled = Fake::new(Ok(()), true);
        let kc = Keychain {
            store: &s,
            bio: &entitled,
            macos: true,
        };
        kc.operator_store(&account, b"pass").unwrap();
        assert!(kc.operator_configured(&account).unwrap());
        assert_eq!(
            kc.operator_get(&account)
                .unwrap()
                .as_deref()
                .map(Vec::as_slice),
            Some(&b"pass"[..])
        );
        kc.operator_delete(&account).unwrap();
        assert!(!kc.operator_configured(&account).unwrap());
        assert_eq!(kc.operator_get(&account).unwrap(), None);
    }

    #[test]
    fn operator_touch_id_account_is_a_lowercase_sha256_digest() {
        assert!(validate_operator_touch_id_account(&"a".repeat(64)).is_ok());
        assert!(validate_operator_touch_id_account(&"A".repeat(64)).is_err());
        assert!(validate_operator_touch_id_account("short").is_err());
        assert!(validate_operator_touch_id_account(&format!("{}\n", "a".repeat(64))).is_err());
    }
}

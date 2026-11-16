//! What Touch ID needs beyond a generic store (secret_store.rs's macOS half): LocalAuthentication
//! (`LAContext`) and the data-protection Keychain's biometry-bound items.
//!
//! The native implementation is the Tauri shell's, call for call: `canEvaluatePolicy` /
//! `evaluatePolicy` with `LAPolicyDeviceOwnerAuthenticationWithBiometrics` through objc2, and
//! generic passwords in the data-protection Keychain (`kSecUseDataProtectionKeychain`) with a
//! `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` + `biometryCurrentSet` access control, built
//! with `security-framework`'s `PasswordOptions` exactly as the Tauri shell builds them (no
//! explicit access group: the item lands in the app's default group, the first entry of its
//! `keychain-access-groups` entitlement, `6Q4R2C3GJK.at.bitcoinaustria.kassiber` in releases).
//!
//! Data-protection items need that entitlement, which only a build signed with the Developer ID
//! provisioning profile has; any other build gets `errSecMissingEntitlement` (-34018), and the
//! Touch ID logic then falls back to the app-gated login-Keychain copy (`touch`).

/// Why a data-protection Keychain operation failed (`security_framework::base::Error` codes the
/// Tauri shell tells apart).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtectedError {
    /// `errSecMissingEntitlement` (-34018): the build has no Keychain access group.
    MissingEntitlement,
    /// `errSecItemNotFound` (-25300).
    NotFound,
    /// Any other OSStatus (a cancelled or failed biometric prompt among them).
    Status(i32),
    /// Not a Mac.
    Unsupported,
}

pub const ERR_SEC_ITEM_NOT_FOUND: i32 = -25_300;
pub const ERR_SEC_MISSING_ENTITLEMENT: i32 = -34_018;

impl ProtectedError {
    pub fn from_code(code: i32) -> ProtectedError {
        match code {
            ERR_SEC_MISSING_ENTITLEMENT => ProtectedError::MissingEntitlement,
            ERR_SEC_ITEM_NOT_FOUND => ProtectedError::NotFound,
            code => ProtectedError::Status(code),
        }
    }

    /// `security_framework_error_for_user`.
    pub fn for_user(&self) -> String {
        match self {
            ProtectedError::MissingEntitlement => "This build cannot access the protected Keychain item; reopen a production-entitled build to manage it.".to_string(),
            ProtectedError::NotFound => "No saved desktop biometric passphrase was found.".to_string(),
            ProtectedError::Status(code) => format!("macOS protected Keychain rejected the operation ({code})."),
            ProtectedError::Unsupported => TOUCH_ID_DESKTOP_ONLY.to_string(),
        }
    }
}

pub const TOUCH_ID_DESKTOP_ONLY: &str =
    "Touch ID passphrase unlock is only available in the macOS desktop app.";

/// LocalAuthentication and the protected Keychain.
pub trait Biometric: Send + Sync {
    /// Touch ID can be evaluated now (`touch_id_biometrics_available`); the error is the system's
    /// localized reason.
    fn available(&self) -> Result<(), String>;
    /// Shows the Touch ID sheet and waits for it (`authenticate_touch_id_for_passphrase`).
    fn authenticate(&self) -> Result<(), String>;
    /// Writes a biometry-current-set item (`store_biometry_current_set_passphrase`).
    fn protected_set(
        &self,
        service: &str,
        account: &str,
        secret: &[u8],
    ) -> Result<(), ProtectedError>;
    /// Reads one (the system asks for Touch ID).
    fn protected_get(&self, service: &str, account: &str) -> Result<Vec<u8>, ProtectedError>;
    fn protected_delete(&self, service: &str, account: &str) -> Result<(), ProtectedError>;
}

/// Not a Mac: everything says so.
pub struct Unsupported;

impl Biometric for Unsupported {
    fn available(&self) -> Result<(), String> {
        Err(TOUCH_ID_DESKTOP_ONLY.to_string())
    }
    fn authenticate(&self) -> Result<(), String> {
        Err(TOUCH_ID_DESKTOP_ONLY.to_string())
    }
    fn protected_set(&self, _: &str, _: &str, _: &[u8]) -> Result<(), ProtectedError> {
        Err(ProtectedError::Unsupported)
    }
    fn protected_get(&self, _: &str, _: &str) -> Result<Vec<u8>, ProtectedError> {
        Err(ProtectedError::Unsupported)
    }
    fn protected_delete(&self, _: &str, _: &str) -> Result<(), ProtectedError> {
        Err(ProtectedError::Unsupported)
    }
}

/// A scripted Touch ID for tests and scenes: no sheet, no Keychain.
pub struct Fake {
    /// `available()`'s answer.
    pub available: Result<(), String>,
    /// `authenticate()`'s answer.
    pub authenticate: Result<(), String>,
    /// The build has the Keychain access group (else protected writes fail with -34018).
    pub entitled: bool,
    pub items: std::sync::Mutex<std::collections::BTreeMap<(String, String), Vec<u8>>>,
}

impl Fake {
    pub fn new(available: Result<(), String>, entitled: bool) -> Fake {
        Fake {
            available,
            authenticate: Ok(()),
            entitled,
            items: Default::default(),
        }
    }
}

impl Biometric for Fake {
    fn available(&self) -> Result<(), String> {
        self.available.clone()
    }
    fn authenticate(&self) -> Result<(), String> {
        self.available.clone()?;
        self.authenticate.clone()
    }
    fn protected_set(
        &self,
        service: &str,
        account: &str,
        secret: &[u8],
    ) -> Result<(), ProtectedError> {
        if !self.entitled {
            return Err(ProtectedError::MissingEntitlement);
        }
        self.items
            .lock()
            .expect("fake items")
            .insert((service.into(), account.into()), secret.to_vec());
        Ok(())
    }
    fn protected_get(&self, service: &str, account: &str) -> Result<Vec<u8>, ProtectedError> {
        if !self.entitled {
            return Err(ProtectedError::MissingEntitlement);
        }
        self.authenticate()
            .map_err(|_| ProtectedError::Status(-128))?;
        self.items
            .lock()
            .expect("fake items")
            .get(&(service.into(), account.into()))
            .cloned()
            .ok_or(ProtectedError::NotFound)
    }
    fn protected_delete(&self, service: &str, account: &str) -> Result<(), ProtectedError> {
        if !self.entitled {
            return Err(ProtectedError::MissingEntitlement);
        }
        self.items
            .lock()
            .expect("fake items")
            .remove(&(service.into(), account.into()))
            .map(|_| ())
            .ok_or(ProtectedError::NotFound)
    }
}

#[cfg(target_os = "macos")]
pub use macos::Native;

#[cfg(target_os = "macos")]
mod macos {
    use super::{Biometric, ProtectedError};

    // `LAContext` lives in LocalAuthentication.framework (the Tauri shell links it from build.rs).
    #[link(name = "LocalAuthentication", kind = "framework")]
    unsafe extern "C" {}

    /// LocalAuthentication.framework `LAPolicy.deviceOwnerAuthenticationWithBiometrics`.
    const LAPOLICY_DEVICE_OWNER_AUTHENTICATION_WITH_BIOMETRICS: i64 = 1;

    pub struct Native;

    fn protected_password_options(
        service: &str,
        account: &str,
    ) -> security_framework::passwords::PasswordOptions {
        use security_framework::passwords::PasswordOptions;
        let mut options = PasswordOptions::new_generic_password(service, account);
        options.use_protected_keychain();
        options
    }

    fn context_if_available() -> Result<objc2::rc::Retained<objc2::runtime::AnyObject>, String> {
        use objc2::msg_send;
        use objc2::rc::Retained;
        use objc2::runtime::{AnyClass, AnyObject, Bool};

        let class = AnyClass::get(c"LAContext")
            .ok_or_else(|| "LocalAuthentication is unavailable on this Mac".to_string())?;
        let context: Retained<AnyObject> = unsafe { msg_send![class, new] };
        let mut can_evaluate_error: *mut AnyObject = std::ptr::null_mut();
        let can_evaluate: Bool = unsafe {
            msg_send![&*context, canEvaluatePolicy: LAPOLICY_DEVICE_OWNER_AUTHENTICATION_WITH_BIOMETRICS, error: &mut can_evaluate_error]
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

    impl Biometric for Native {
        fn available(&self) -> Result<(), String> {
            context_if_available().map(|_| ())
        }

        fn authenticate(&self) -> Result<(), String> {
            use block2::RcBlock;
            use objc2::msg_send;
            use objc2::runtime::{AnyObject, Bool};
            use objc2_foundation::NSString;
            use std::sync::mpsc;

            let context = context_if_available()?;
            let reason = NSString::from_str("Unlock Kassiber with Touch ID");
            let (sender, receiver) = mpsc::channel();
            let reply = RcBlock::new(move |success: Bool, _error: *mut AnyObject| {
                let _ = sender.send(success.as_bool());
            });
            unsafe {
                let _: () = msg_send![&*context, evaluatePolicy: LAPOLICY_DEVICE_OWNER_AUTHENTICATION_WITH_BIOMETRICS, localizedReason: &*reason, reply: &*reply];
            }
            match receiver.recv() {
                Ok(true) => Ok(()),
                Ok(false) => Err("Touch ID authentication was cancelled or failed.".to_string()),
                Err(_) => Err("Touch ID authentication did not return a result.".to_string()),
            }
        }

        fn protected_set(
            &self,
            service: &str,
            account: &str,
            secret: &[u8],
        ) -> Result<(), ProtectedError> {
            use security_framework::access_control::{ProtectionMode, SecAccessControl};
            use security_framework::passwords::{
                AccessControlOptions, set_generic_password_options,
            };

            let mut options = protected_password_options(service, account);
            let access_control = SecAccessControl::create_with_protection(
                Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
                AccessControlOptions::BIOMETRY_CURRENT_SET.bits(),
            )
            .map_err(|e| ProtectedError::from_code(e.code()))?;
            options.set_access_control(access_control);
            set_generic_password_options(secret, options)
                .map_err(|e| ProtectedError::from_code(e.code()))
        }

        fn protected_get(&self, service: &str, account: &str) -> Result<Vec<u8>, ProtectedError> {
            security_framework::passwords::generic_password(protected_password_options(
                service, account,
            ))
            .map_err(|e| ProtectedError::from_code(e.code()))
        }

        fn protected_delete(&self, service: &str, account: &str) -> Result<(), ProtectedError> {
            security_framework::passwords::delete_generic_password_options(
                protected_password_options(service, account),
            )
            .map_err(|e| ProtectedError::from_code(e.code()))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Asks LocalAuthentication whether Touch ID could be evaluated (no sheet, no Keychain).
        /// Ignored by default; run explicitly to see this Mac's answer.
        #[test]
        #[ignore]
        fn native_availability_answers_without_a_prompt() {
            match Native.available() {
                Ok(()) => println!("touch id: available"),
                Err(reason) => println!("touch id: unavailable ({reason})"),
            }
        }

        /// An unsigned build has no Keychain access group: a protected write under a test-only
        /// service must fail with errSecMissingEntitlement (the trigger of the preview fallback)
        /// and leave nothing behind.
        #[test]
        #[ignore]
        fn unsigned_build_has_no_protected_keychain() {
            let service = "Kassiber Rune Desktop Test Only (Protected)";
            let r =
                Native.protected_set(service, "rune-protected-probe", b"test-value-not-a-secret");
            println!("protected_set: {r:?}");
            if r.is_ok() {
                let _ = Native.protected_delete(service, "rune-protected-probe");
            }
            assert_eq!(r, Err(ProtectedError::MissingEntitlement));
        }
    }
}

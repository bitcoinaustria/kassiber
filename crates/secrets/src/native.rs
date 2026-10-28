use super::{store_error, Passphrase, Result};
use keyring_core::{api::CredentialStoreApi, Entry};
use zeroize::Zeroizing;

pub(super) fn available() -> bool {
    // Explicit secrets status/enrollment only; never called by startup or an
    // ordinary display read. Construction can contact the platform session bus.
    #[cfg(target_os = "linux")]
    {
        zbus_secret_service_keyring_store::Store::new().is_ok()
    }
    #[cfg(target_os = "macos")]
    {
        apple_native_keyring_store::keychain::Store::new().is_ok()
    }
    #[cfg(target_os = "windows")]
    {
        windows_native_keyring_store::Store::new().is_ok()
    }
}

#[cfg(not(target_os = "windows"))]
pub(super) fn set_password(service: &str, account: &str, secret: &Passphrase) -> Result<()> {
    compatible_entry(service, account)
        .map_err(store_error)?
        .set_password(secret.expose())
        .map_err(store_error)
}

#[cfg(target_os = "windows")]
fn write_python_item(entry: &Entry, account: &str, secret: &str) -> Result<()> {
    use std::collections::HashMap;
    let attributes = HashMap::from([
        ("username", account),
        ("comment", "Stored using python-keyring"),
        ("target_alias", ""),
    ]);
    // This adapter preserves an existing target's username on set_password.
    // Python overwrites it. Rebind only after the prior occupant has been
    // copied to its compound target; never place the new secret under that
    // prior occupant's username, even transiently.
    match entry.get_attributes() {
        Ok(_) => entry.update_attributes(&attributes).map_err(store_error)?,
        Err(keyring_core::Error::NoEntry) => (),
        Err(error) => return Err(store_error(error)),
    }
    entry.set_password(secret).map_err(store_error)?;
    entry.update_attributes(&attributes).map_err(store_error)
}

#[cfg(target_os = "windows")]
pub(super) fn set_password(service: &str, account: &str, secret: &Passphrase) -> Result<()> {
    use std::collections::HashMap;
    let store = windows_native_keyring_store::Store::new().map_err(store_error)?;
    let bare = store
        .build(
            service,
            account,
            Some(&HashMap::from([("target", service)])),
        )
        .map_err(store_error)?;
    match bare.get_attributes() {
        Ok(attributes) => {
            let username = attributes
                .get("username")
                .ok_or_else(|| store_error(keyring_core::Error::NoEntry))?;
            {
                // Python moves the prior bare-service occupant to its compound
                // target before writing the new one. Password APIs use UTF-16.
                let old = Zeroizing::new(bare.get_password().map_err(store_error)?);
                let target = format!("{username}@{service}");
                let displaced = store
                    .build(
                        service,
                        username,
                        Some(&HashMap::from([("target", target.as_str())])),
                    )
                    .map_err(store_error)?;
                write_python_item(&displaced, username, &old)?;
            }
        }
        Err(keyring_core::Error::NoEntry) => (),
        Err(e) => return Err(store_error(e)),
    }
    write_python_item(&bare, account, secret.expose())
}

#[cfg(not(target_os = "windows"))]
pub(super) fn delete_password(service: &str, account: &str) -> Result<()> {
    let entry = compatible_entry(service, account).map_err(store_error)?;
    // Deletion errors are never proof of absence: verify by reading afterwards.
    let _ = entry.delete_credential();
    match get_password(service, account)? {
        None => Ok(()),
        Some(_) => Err(store_error(keyring_core::Error::NoEntry)),
    }
}

#[cfg(target_os = "windows")]
pub(super) fn delete_password(service: &str, account: &str) -> Result<()> {
    use std::collections::HashMap;
    let store = windows_native_keyring_store::Store::new().map_err(store_error)?;
    for target in [service.to_owned(), format!("{account}@{service}")] {
        let entry = store
            .build(
                service,
                account,
                Some(&HashMap::from([("target", target.as_str())])),
            )
            .map_err(store_error)?;
        match entry.get_attributes() {
            Ok(attributes) if attributes.get("username").map(String::as_str) == Some(account) => {
                let _ = entry.delete_credential();
            }
            Ok(_) | Err(keyring_core::Error::NoEntry) => (),
            Err(e) => return Err(store_error(e)),
        }
    }
    match get_password(service, account)? {
        None => Ok(()),
        Some(_) => Err(store_error(keyring_core::Error::NoEntry)),
    }
}

pub(super) fn update_existing(service: &str, account: &str, secret: &Passphrase) -> Result<()> {
    let entry = compatible_entry(service, account).map_err(store_error)?;
    // Resolve a unique existing wrapper first. This must never create an item
    // at a guessed target or overwrite another Windows account.
    let existing = entry.get_credential().map_err(store_error)?;
    existing.set_password(secret.expose()).map_err(store_error)
}

pub(super) fn get_password(service: &str, account: &str) -> Result<Option<Passphrase>> {
    let entry = match compatible_entry(service, account) {
        Ok(entry) => entry,
        Err(keyring_core::Error::NoEntry) => return Ok(None),
        Err(e) => return Err(store_error(e)),
    };
    match entry.get_password() {
        Ok(secret) if secret.is_empty() => Ok(None),
        Ok(secret) => Passphrase::new(Zeroizing::new(secret)).map(Some),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(e) => Err(store_error(e)),
    }
}
#[cfg(target_os = "linux")]
fn compatible_entry(service: &str, account: &str) -> keyring_core::Result<Entry> {
    // Python matches service/username only. Adding a target attribute here
    // would make existing Python items invisible. Extra application retained.
    zbus_secret_service_keyring_store::Store::new()?.build(service, account, None)
}
#[cfg(target_os = "macos")]
fn compatible_entry(service: &str, account: &str) -> keyring_core::Result<Entry> {
    apple_native_keyring_store::keychain::Store::new()?.build(service, account, None)
}
#[cfg(target_os = "windows")]
fn compatible_entry(service: &str, account: &str) -> keyring_core::Result<Entry> {
    use std::collections::HashMap;
    let store = windows_native_keyring_store::Store::new()?;
    for target in [service.to_owned(), format!("{account}@{service}")] {
        let modifiers = HashMap::from([("target", target.as_str())]);
        let entry = store.build(service, account, Some(&modifiers))?;
        match entry.get_attributes() {
            Ok(attributes) if attributes.get("username").map(String::as_str) == Some(account) => {
                return Ok(entry)
            }
            Ok(_) | Err(keyring_core::Error::NoEntry) => (),
            Err(error) => return Err(error),
        }
    }
    Err(keyring_core::Error::NoEntry)
}

use super::{store_error, Passphrase, Result};
use keyring_core::{api::CredentialStoreApi, Entry};
use zeroize::Zeroizing;

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

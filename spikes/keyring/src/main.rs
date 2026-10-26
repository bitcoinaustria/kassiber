use keyring_core::{Entry, api::CredentialStoreApi};
use std::collections::HashMap;

#[cfg(target_os = "linux")]
fn compatible_entry(service: &str, account: &str) -> keyring_core::Result<Entry> {
    // Do not set target: Python's items have no target attribute.
    let label = format!("Password for '{account}' on '{service}'");
    let modifiers = HashMap::from([("label", label.as_str())]);
    zbus_secret_service_keyring_store::Store::new()?.build(service, account, Some(&modifiers))
}

#[cfg(target_os = "macos")]
fn compatible_entry(service: &str, account: &str) -> keyring_core::Result<Entry> {
    // Classic generic-password store, not the protected-data store.
    apple_native_keyring_store::keychain::Store::new()?.build(service, account, None)
}

#[cfg(target_os = "windows")]
fn compatible_entry(service: &str, account: &str) -> keyring_core::Result<Entry> {
    let store = windows_native_keyring_store::Store::new()?;
    for target in [service.to_string(), format!("{account}@{service}")] {
        let mods = HashMap::from([("target", target.as_str())]);
        let candidate = store.build(service, account, Some(&mods))?;
        match candidate.get_attributes() {
            Ok(attrs) if attrs.get("username").map(String::as_str) == Some(account) => {
                let persistence = attrs
                    .get("persistence")
                    .map(String::as_str)
                    .unwrap_or("Enterprise");
                return store.build(
                    service,
                    account,
                    Some(&HashMap::from([
                        ("target", target.as_str()),
                        ("persistence", persistence),
                    ])),
                );
            }
            Ok(_) | Err(keyring_core::Error::NoEntry) => {}
            Err(e) => return Err(e),
        }
    }
    // This spike updates existing Python items only. Never turn a username
    // mismatch into a new entry at the same target and then read its secret.
    Err(keyring_core::Error::NoEntry)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let [_, service, account] = args.as_slice() else {
        return Err("service account".into());
    };
    assert!(
        service.starts_with("Kassiber R0 synthetic "),
        "synthetic namespace required"
    );
    let entry = compatible_entry(service, account)?;
    // Password APIs matter: Windows stores strings as UTF-16LE, not UTF-8 bytes.
    assert_eq!(entry.get_password()?, "Python R0 '🔑");
    entry.set_password("Rust R0 '🔐")?;
    assert_eq!(entry.get_password()?, "Rust R0 '🔐");
    println!("PASS: read Python password and update matching native item from Rust");
    Ok(())
}

//! No native store construction, dependencies, probes or file fallbacks.
use super::{Error, ErrorKind, Passphrase, Result};
fn unavailable() -> Error {
    Error::new(
        ErrorKind::SecretStoreUnavailable,
        "native credential storage is not enabled in this build",
    )
}
pub(super) fn available() -> bool {
    false
}
pub(super) fn set_password(_: &str, _: &str, _: &Passphrase) -> Result<()> {
    Err(unavailable())
}
pub(super) fn delete_password(_: &str, _: &str) -> Result<()> {
    Err(unavailable())
}
pub(super) fn get_password(_: &str, _: &str) -> Result<Option<Passphrase>> {
    Err(unavailable())
}
pub(super) fn update_existing(_: &str, _: &str, _: &Passphrase) -> Result<()> {
    Err(unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_store_never_grants_or_changes_a_credential() {
        assert_eq!(
            get_password("synthetic", "synthetic").unwrap_err().kind,
            ErrorKind::SecretStoreUnavailable
        );
        let secret = crate::read_passphrase(b"synthetic".as_slice()).unwrap();
        assert_eq!(
            update_existing("synthetic", "synthetic", &secret)
                .unwrap_err()
                .kind,
            ErrorKind::SecretStoreUnavailable
        );
    }
}

//! Database authorization and explicit encryption services. Native stores are
//! constructed lazily for explicit unlock/lifecycle actions, never by ordinary
//! display reads.
pub mod credentials;
pub mod modes;
#[cfg(feature = "native-keyring")]
mod native;
#[cfg(not(feature = "native-keyring"))]
#[path = "native_disabled.rs"]
mod native;
pub mod policy;
pub mod service;
#[cfg(test)]
#[path = "tests.rs"]
mod service_tests;
pub mod terminal;
use kassiber_ownership::OwnerNamespace;
use kassiber_paths::{load_settings, Platform};
pub use kassiber_paths::{Error, ErrorKind, Result};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{self, IsTerminal, Read},
    path::Path,
};
#[cfg(feature = "native-keyring")]
use zeroize::Zeroize;
use zeroize::Zeroizing;

pub const CLI_PASSPHRASE_SERVICE: &str = "Kassiber CLI Database Passphrase";
pub const MAX_PASSPHRASE_BYTES: usize = 8192;
/// Debug never prints the value. Application-owned values are zeroized on
/// drop and borrowed from the session cache for reopens.
pub struct Passphrase(Zeroizing<String>);
impl Passphrase {
    pub fn new(value: Zeroizing<String>) -> Result<Self> {
        if value.is_empty() || value.contains('\0') {
            return Err(invalid("passphrase must not be empty or contain NUL bytes"));
        }
        Ok(Self(value))
    }
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}
impl fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Passphrase([REDACTED])")
    }
}
fn invalid(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidPassphrase, message)
}
/// Read to EOF (or the limit+1 refusal byte); strip one LF, then one CR. The
/// owned reader is dropped on every path, including I/O/UTF-8/length failures.
pub fn read_passphrase(reader: impl Read) -> Result<Passphrase> {
    let mut raw = Zeroizing::new(Vec::with_capacity(MAX_PASSPHRASE_BYTES + 2));
    reader
        .take(MAX_PASSPHRASE_BYTES as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|_| invalid("could not read passphrase descriptor"))?;
    if raw.len() > MAX_PASSPHRASE_BYTES {
        return Err(invalid("passphrase exceeds 8192-byte limit"));
    }
    // Wipe removed bytes too; Vec::pop does not overwrite its spare capacity.
    for ending in *b"\n\r" {
        if raw.last() == Some(&ending) {
            if let Some(last) = raw.last_mut() {
                *last = 0;
            }
            raw.pop();
        }
    }
    let text = std::str::from_utf8(&raw).map_err(|_| invalid("passphrase is not valid UTF-8"))?;
    Passphrase::new(Zeroizing::new(text.into()))
}
/// Descriptors a broker child drained before dispatch, by number
/// (`prime_db_passphrase`): a later read of the same number returns the
/// cached passphrase instead of reading a closed pipe.
static PRIMED: std::sync::Mutex<Vec<(i32, Zeroizing<String>)>> = std::sync::Mutex::new(Vec::new());

/// Drain and cache `fd` now, so a parent writing several secret pipes in
/// order cannot block on this one while the child waits on another.
pub fn prime_passphrase_descriptor(fd: i32) -> Result<()> {
    let passphrase = read_descriptor_passphrase(fd)?;
    let mut primed = PRIMED.lock().unwrap_or_else(|poison| poison.into_inner());
    primed.retain(|(number, _)| *number != fd);
    primed.push((fd, passphrase.0.clone()));
    Ok(())
}

/// Read a passphrase from a descriptor the caller named, once; a primed
/// descriptor answers from its cache.
pub fn read_passphrase_from_fd(fd: i32) -> Result<Passphrase> {
    let cached = PRIMED
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .iter()
        .find(|(number, _)| *number == fd)
        .map(|(_, passphrase)| passphrase.clone());
    match cached {
        Some(passphrase) => Passphrase::new(passphrase),
        None => read_descriptor_passphrase(fd),
    }
}

#[cfg(unix)]
fn read_descriptor_passphrase(fd: i32) -> Result<Passphrase> {
    if fd < 0 {
        return Err(invalid("passphrase descriptor must be non-negative"));
    }
    // Use raw OS calls so even an invalid caller-supplied integer is an input
    // error, not a violation of Rust's OwnedFd validity contract.
    struct Descriptor(i32);
    impl Read for Descriptor {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            // SAFETY: OS validates the fd; buffer is live for its stated length.
            let count = unsafe { libc::read(self.0, buffer.as_mut_ptr().cast(), buffer.len()) };
            if count < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(count as usize)
            }
        }
    }
    impl Drop for Descriptor {
        fn drop(&mut self) {
            // SAFETY: close accepts any integer, and this object consumes it once.
            unsafe {
                libc::close(self.0);
            }
        }
    }
    read_passphrase(Descriptor(fd))
}
#[cfg(windows)]
fn read_descriptor_passphrase(fd: i32) -> Result<Passphrase> {
    unsafe extern "C" {
        fn _read(fd: i32, buffer: *mut u8, count: u32) -> i32;
        fn _close(fd: i32) -> i32;
    }
    if fd < 0 {
        return Err(invalid("passphrase descriptor must be non-negative"));
    }
    // A broker child is handed OS handles, not CRT descriptors
    // (`msvcrt.open_osfhandle` in Python).
    if std::env::var_os("KASSIBER_OPERATOR_CHILD").is_some_and(|v| v == "1") {
        use std::os::windows::io::{FromRawHandle, OwnedHandle, RawHandle};
        // SAFETY: the broker listed this handle for the child to own; it is
        // consumed exactly once here and closed when the file drops.
        let handle = unsafe { OwnedHandle::from_raw_handle(fd as usize as RawHandle) };
        return read_passphrase_from_inherited_handle(handle);
    }
    struct Descriptor(i32);
    impl Read for Descriptor {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            // SAFETY: CRT validates fd; buffer is writable, count fits u32.
            let result = unsafe {
                _read(
                    self.0,
                    buffer.as_mut_ptr(),
                    buffer.len().min(u32::MAX as usize) as u32,
                )
            };
            if result < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(result as usize)
            }
        }
    }
    impl Drop for Descriptor {
        fn drop(&mut self) {
            // SAFETY: consumes this CRT descriptor exactly once.
            unsafe {
                _close(self.0);
            }
        }
    }
    read_passphrase(Descriptor(fd))
}
/// Windows broker pipes are OS handles, whereas ordinary CLI fd arguments are
/// CRT descriptors. The shell selects this only for an authenticated child.
#[cfg(windows)]
pub fn read_passphrase_from_inherited_handle(
    handle: std::os::windows::io::OwnedHandle,
) -> Result<Passphrase> {
    read_passphrase(std::fs::File::from(handle))
}

pub trait CredentialReader {
    fn get_password(&self, service: &str, account: &str) -> Result<Option<Passphrase>>;
}
/// Explicit credential lifecycle actions. Production adapters never use files
/// as a fallback; tests inject an isolated memory store or disposable bus.
pub trait CredentialStore: CredentialReader {
    fn available(&self) -> bool;
    fn set_password(&self, service: &str, account: &str, secret: &Passphrase) -> Result<()>;
    fn delete_password(&self, service: &str, account: &str) -> Result<()>;
}
#[derive(Default)]
pub struct NativeCredentialReader;
impl NativeCredentialReader {
    /// Explicit maintenance action, separate from ordinary unlock reads. Finds
    /// a unique existing item; never creates an enrollment or edits settings.
    pub fn update_existing_password(
        &self,
        service: &str,
        account: &str,
        secret: &Passphrase,
    ) -> Result<()> {
        native::update_existing(service, account, secret)
    }
}
impl CredentialReader for NativeCredentialReader {
    fn get_password(&self, service: &str, account: &str) -> Result<Option<Passphrase>> {
        native::get_password(service, account)
    }
}
impl CredentialStore for NativeCredentialReader {
    fn available(&self) -> bool {
        native::available()
    }
    fn set_password(&self, service: &str, account: &str, secret: &Passphrase) -> Result<()> {
        native::set_password(service, account, secret)
    }
    fn delete_password(&self, service: &str, account: &str) -> Result<()> {
        native::delete_password(service, account)
    }
}
/// Public-safe metadata: this function does not contact a store or session bus.
pub fn access_policy() -> &'static str {
    match Platform::current() {
        Platform::Macos => "macos_keychain_application_acl",
        Platform::Windows => "windows_dpapi_user_scope",
        Platform::Linux => "linux_secret_service_session",
    }
}
fn valid_hex(value: &str, count: usize) -> bool {
    value.len() == count
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn remembered_unlock_account(database_identity: &str, enrollment: &str) -> Result<String> {
    if !valid_hex(database_identity, 32) || !valid_hex(enrollment, 32) {
        return Err(Error::new(
            ErrorKind::OperatorPolicyBindingRequired,
            "remembered credential is not bound to a project database",
        ));
    }
    Ok(Sha256::digest(
        format!("kassiber-cli-remembered-unlock-v2:{database_identity}:{enrollment}").as_bytes(),
    )
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect())
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnlockNotice {
    RememberedUnlockStale,
}
#[derive(Default)]
pub struct UnlockOptions<'a> {
    pub non_interactive: bool,
    pub expected_database_identity: Option<&'a str>,
}
#[derive(Default)]
pub struct UnlockSession {
    cached: Option<Passphrase>,
    cached_identity: Option<String>,
    notices: Vec<UnlockNotice>,
}
impl UnlockSession {
    /// Consume once before dispatch (also safe for broker pipe draining).
    pub fn prime_fd(&mut self, fd: i32) -> Result<()> {
        self.clear();
        self.cached = Some(read_passphrase_from_fd(fd)?);
        Ok(())
    }
    pub fn cache(&mut self, passphrase: Passphrase) {
        self.clear();
        self.cached = Some(passphrase);
    }
    pub fn clear(&mut self) {
        self.cached = None;
        self.cached_identity = None;
    }
    pub fn take_notices(&mut self) -> Vec<UnlockNotice> {
        std::mem::take(&mut self.notices)
    }
    /// Callback receives passphrase and the database ID it MUST bind before any
    /// writes. Shells normally call Book::open with those two arguments.
    pub fn open<T>(
        &mut self,
        data_root: &Path,
        options: UnlockOptions<'_>,
        reader: &dyn CredentialReader,
        open: impl FnMut(Option<&str>, Option<&str>) -> Result<T>,
    ) -> Result<T> {
        self.open_in_namespace(data_root, options, reader, &OwnerNamespace::system()?, open)
    }
    /// Embedders/tests inject the ownership rendezvous explicitly. Product
    /// callers use `open`, which always selects the fixed account namespace.
    pub fn open_in_namespace<T>(
        &mut self,
        data_root: &Path,
        options: UnlockOptions<'_>,
        reader: &dyn CredentialReader,
        namespace: &OwnerNamespace,
        mut open: impl FnMut(Option<&str>, Option<&str>) -> Result<T>,
    ) -> Result<T> {
        if let Some(cached) = &self.cached {
            if let (Some(cached), Some(expected)) = (
                self.cached_identity.as_deref(),
                options.expected_database_identity,
            ) {
                if cached != expected {
                    return Err(Error::new(
                        ErrorKind::OperatorPolicyBindingMismatch,
                        "cached unlock is bound to a different project database",
                    ));
                }
            }
            return open(
                Some(cached.expose()),
                options
                    .expected_database_identity
                    .or(self.cached_identity.as_deref()),
            );
        }
        match open(None, options.expected_database_identity) {
            Ok(value) => return Ok(value),
            Err(e) if e.kind == ErrorKind::PassphraseRequired => (),
            Err(e) => return Err(e),
        }
        let project = namespace.canonical_project(data_root)?;
        let settings_path = project.database.parent().expect("database parent");
        let context = kassiber_paths::Context::from_env()?;
        let canonical_state = context.state_root_for_data(settings_path)?;
        let settings = load_settings(&canonical_state.join("config/settings.json"))?;
        let mode = match settings.get("operator_unlock_mode") {
            None | Some(serde_json::Value::Null) => "manual",
            Some(v)
                if v.as_str()
                    .is_some_and(|s| ["manual", "brokered", "unattended"].contains(&s)) =>
            {
                v.as_str().expect("checked mode")
            }
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidOperatorUnlockMode,
                    "configured operator unlock mode is invalid",
                ))
            }
        };
        let project_binding = settings
            .get("operator_policy_project_identity")
            .and_then(|v| v.as_str());
        let database_binding = settings
            .get("operator_policy_database_identity")
            .and_then(|v| v.as_str());
        let bound = project_binding == Some(project.identity.as_str())
            && database_binding.is_some_and(|s| valid_hex(s, 32));
        if mode == "unattended"
            && bound
            && settings
                .get("cli_remembered_unlock")
                .and_then(|v| v.as_bool())
                == Some(true)
            && settings
                .get("cli_legacy_unlock_quarantined")
                .and_then(|v| v.as_bool())
                != Some(true)
        {
            let db_id = database_binding.expect("validated binding");
            if options
                .expected_database_identity
                .is_some_and(|id| id != db_id)
            {
                return Err(Error::new(
                    ErrorKind::OperatorPolicyBindingMismatch,
                    "remembered unlock is bound to a different project database",
                ));
            }
            let enrollment = settings
                .get("cli_remembered_unlock_enrollment")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Ok(account) = remembered_unlock_account(db_id, enrollment) {
                // Store unavailability is not permission to fall back to files.
                if let Ok(Some(secret)) = reader.get_password(CLI_PASSPHRASE_SERVICE, &account) {
                    match open(Some(secret.expose()), Some(db_id)) {
                        Ok(value) => {
                            self.cached = Some(secret);
                            self.cached_identity = Some(db_id.into());
                            return Ok(value);
                        }
                        Err(e) if e.kind == ErrorKind::UnlockFailed => {
                            self.notices.push(UnlockNotice::RememberedUnlockStale)
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        if !options.non_interactive && io::stdin().is_terminal() {
            let value = Zeroizing::new(
                rpassword::prompt_password("Database passphrase: ")
                    .map_err(|_| invalid("no TTY available for passphrase prompt"))?,
            );
            let secret = Passphrase::new(value)?;
            let result = open(Some(secret.expose()), options.expected_database_identity)?;
            self.cached = Some(secret);
            return Ok(result);
        }
        Err(Error::new(ErrorKind::InteractionRequired, "database authorization requires local interaction")
            .hint("The database is encrypted. For agent or scripted work, ask the user to run `kassiber operator unlock` in their own terminal (same --project/--data-root selection), then retry; never ask for the passphrase itself. A controlling process that already holds the secret may pass --db-passphrase-fd instead.")
            .details(serde_json::json!({"reason":"database_passphrase","unlock_mode":if bound {mode} else {"manual"},"user_command":"kassiber operator unlock"})))
    }
}
/// Error adapters must erase secret bytes attached to native decode errors.
#[cfg(feature = "native-keyring")]
fn store_error(mut error: keyring_core::Error) -> Error {
    match &mut error {
        keyring_core::Error::BadEncoding(bytes) | keyring_core::Error::BadDataFormat(bytes, _) => {
            bytes.zeroize()
        }
        _ => (),
    }
    Error::new(
        ErrorKind::SecretStoreUnavailable,
        "native credential store is unavailable or ambiguous",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fd_text_rules_and_bounds() {
        for (raw, expected) in [
            (b"x\r\n".as_slice(), "x"),
            (b"x\n\n", "x\n"),
            (b"x\r", "x"),
            (b" x ", " x "),
        ] {
            assert_eq!(read_passphrase(raw).unwrap().expose(), expected);
        }
        for raw in [b"".as_slice(), b"\r\n", b"\xff", b"x\0"] {
            assert_eq!(
                read_passphrase(raw).unwrap_err().kind,
                ErrorKind::InvalidPassphrase
            );
        }
        assert!(read_passphrase(vec![b'x'; 8192].as_slice()).is_ok());
        assert!(read_passphrase(vec![b'x'; 8193].as_slice()).is_err());
        assert_eq!(
            read_passphrase_from_fd(-1).unwrap_err().kind,
            ErrorKind::InvalidPassphrase
        );
        #[cfg(unix)]
        assert!(read_passphrase_from_fd(999999).is_err());
    }
    #[test]
    fn cache_reopens_without_store_reads_and_redacts_debug() {
        struct Never;
        impl CredentialReader for Never {
            fn get_password(&self, _: &str, _: &str) -> Result<Option<Passphrase>> {
                panic!("unexpected store read")
            }
        }
        let mut session = UnlockSession::default();
        let secret = read_passphrase(b"synthetic".as_slice()).unwrap();
        assert!(!format!("{secret:?}").contains("synthetic"));
        session.cache(secret);
        for _ in 0..2 {
            assert_eq!(
                session
                    .open(
                        Path::new("/not-used"),
                        UnlockOptions::default(),
                        &Never,
                        |pass, _| Ok(pass.unwrap().len())
                    )
                    .unwrap(),
                9
            );
        }
        session.clear();
    }
}

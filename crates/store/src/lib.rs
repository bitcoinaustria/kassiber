//! Shared storage. Runtime admission remains read-only; creation, semantic
//! migration and encryption maintenance are explicit operations.
#[cfg(any(test, feature = "test-hooks"))]
mod compat;
mod creation;
mod durable;
pub mod encryption;
pub mod layout;
pub mod retired;
mod schema;
mod schema_structure;
pub mod semantics;
pub mod state_root;

/// Development-only deterministic sources and file-layer fault injection.
/// No product command reads test environment variables or enables this feature.
#[cfg(feature = "test-hooks")]
pub mod test_hooks {
    use super::*;
    pub fn graph(root: &Path) -> Result<()> {
        let mut conn = Connection::open_with_flags(
            kassiber_paths::resolve_database_path(root),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(map_sqlite_error)?;
        configure_pragmas(&conn, false)?;
        crate::compat::install_graph(&mut conn)
    }
    pub fn msat(root: &Path, observer: &mut impl FnMut(&str) -> Result<()>) -> Result<()> {
        let mut conn = Connection::open_with_flags(
            kassiber_paths::resolve_database_path(root),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(map_sqlite_error)?;
        configure_pragmas(&conn, false)?;
        crate::compat::install_base(&conn)?;
        crate::compat::migrate_msat(&conn, observer)?;
        crate::compat::finish_msat_columns(&conn)?;
        crate::compat::install_graph(&mut conn)
    }
    pub fn migrate_root(
        context: &kassiber_paths::Context,
        observer: &mut impl FnMut(&str) -> Result<()>,
    ) -> Result<PathBuf> {
        crate::state_root::migrate_with_observer(context, observer)
    }
    pub fn create(
        root: &Path,
        passphrase: Option<&str>,
        identity: &str,
        timestamp: &str,
        observer: &mut impl FnMut(&str) -> Result<()>,
    ) -> Result<Book> {
        crate::creation::create(root, passphrase, Some(identity), Some(timestamp), observer)
    }
    pub fn semantics(
        root: &Path,
        passphrase: Option<&str>,
        timestamp: &str,
        observer: &mut impl FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        let database = kassiber_paths::resolve_database_path(root);
        let mut conn = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(map_sqlite_error)?;
        if let Some(pass) = passphrase {
            apply_keying(&conn, pass)?;
        }
        configure_pragmas(&conn, passphrase.is_some())?;
        let timestamp = timestamp.to_owned();
        conn.create_scalar_function(
            "strftime",
            2,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| Ok(timestamp.clone()),
        )
        .map_err(map_sqlite_error)?;
        crate::semantics::migrate_with_observer(&mut conn, observer)
    }
    pub fn encrypt(
        namespace: &OwnerNamespace,
        root: &Path,
        passphrase: &str,
        observer: &mut impl FnMut(&str) -> Result<()>,
    ) -> Result<crate::encryption::EncryptionResult> {
        crate::encryption::encrypt_with_observer(namespace, root, passphrase, None, observer)
    }
    pub fn rekey(
        namespace: &OwnerNamespace,
        root: &Path,
        current: &str,
        new: &str,
        observer: &mut impl FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        crate::encryption::rekey_with_observer(
            namespace,
            root,
            current,
            new,
            None,
            || Ok(()),
            observer,
        )
    }
    pub fn retired(
        root: &Path,
        passphrase: Option<&str>,
        timestamp: &str,
        observer: &mut impl FnMut(&str) -> Result<()>,
    ) -> Result<()> {
        let database = kassiber_paths::resolve_database_path(root);
        let mut conn = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(map_sqlite_error)?;
        if let Some(pass) = passphrase {
            apply_keying(&conn, pass)?;
        }
        configure_pragmas(&conn, passphrase.is_some())?;
        let timestamp = timestamp.to_owned();
        conn.create_scalar_function(
            "strftime",
            2,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| Ok(timestamp.clone()),
        )
        .map_err(map_sqlite_error)?;
        crate::retired::drop_device_sync_with_observer(&mut conn, observer)?;
        crate::retired::retire_ledger_with_observer(&mut conn, observer)?;
        Ok(())
    }
}
use kassiber_ownership::{CanonicalProject, OwnerNamespace};
use kassiber_paths::{canonicalize_allow_missing, looks_like_plaintext, resolve_database_path};
pub use kassiber_paths::{Error, ErrorKind, Result};
pub use rusqlite;
use rusqlite::{
    backup::{Backup, StepResult},
    Connection, OpenFlags, OptionalExtension, Savepoint, Transaction, TransactionBehavior,
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub const BUSY_TIMEOUT: Duration = Duration::from_secs(30);
#[derive(Default)]
pub struct OpenOptions<'a> {
    pub passphrase: Option<&'a str>,
    pub expected_database_identity: Option<&'a str>,
    pub expected_project: Option<&'a CanonicalProject>,
}
/// Owns its connection; callers holding an ownership lease must drop the book
/// before releasing the lease. Use `OwnedBook` to enforce that drop order.
pub struct Book {
    connection: Connection,
    database: PathBuf,
    instance_id: String,
    encrypted: bool,
}
/// Failed closes retain the live resource. Runtime retirement queues keep it
/// until close succeeds, so a lease cannot be released before the connection.
pub struct CloseFailure<T> {
    pub resource: Box<T>,
    pub error: Error,
}
impl Book {
    pub fn close(self) -> std::result::Result<(), CloseFailure<Self>> {
        let Self {
            connection,
            database,
            instance_id,
            encrypted,
        } = self;
        connection
            .close()
            .map_err(|(connection, error)| CloseFailure {
                resource: Box::new(Self {
                    connection,
                    database,
                    instance_id,
                    encrypted,
                }),
                error: map_sqlite_error(error),
            })
    }
    pub fn open(data_root: &Path, options: OpenOptions<'_>) -> Result<Self> {
        let effective = kassiber_paths::Context::from_env()?.effective_data_root(data_root)?;
        let child_project = verify_operator_child_project(&effective)?;
        let database = canonicalize_allow_missing(&resolve_database_path(&effective))?;
        if !database.is_file() || database.metadata().map_err(Error::io)?.len() == 0 {
            return Err(Error::new(
                ErrorKind::NotInitialized,
                "an existing initialized book is required",
            ));
        }
        kassiber_ownership::validate_database_file(&database)?;
        verify_project(
            &database,
            options.expected_project.or(child_project.as_ref()),
        )?;
        let encrypted = !looks_like_plaintext(&database)?;
        match (encrypted, options.passphrase) {
            (true, None) => {
                return Err(Error::new(
                    ErrorKind::PassphraseRequired,
                    "database is encrypted; supply a passphrase via --db-passphrase-fd",
                ))
            }
            (false, Some(_)) => {
                return Err(Error::new(
                    ErrorKind::PlaintextDatabase,
                    "database file at this path is plaintext SQLite",
                ))
            }
            _ => (),
        }
        let connection = Connection::open_with_flags(
            &database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(map_sqlite_error)?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(map_sqlite_error)?;
        if let Some(passphrase) = options.passphrase {
            apply_keying(&connection, passphrase)?;
            connection
                .query_row("SELECT count(*) FROM sqlite_master", [], |r| {
                    r.get::<_, i64>(0)
                })
                .map_err(|e| {
                    if sqlite_is_busy(&e) {
                        map_sqlite_error(e)
                    } else {
                        Error::new(
                            ErrorKind::UnlockFailed,
                            "wrong passphrase or unsupported database format",
                        )
                        .retryable(true)
                    }
                })?;
        }
        // A child may never silently discard its environment binding, even if
        // its caller did not supply explicit OpenOptions.
        let child = std::env::var_os("KASSIBER_OPERATOR_CHILD").is_some_and(|v| v == "1");
        let env_identity = std::env::var("KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY").ok();
        if child && env_identity.is_none() {
            return Err(Error::new(
                ErrorKind::OperatorProjectBindingInvalid,
                "operator child database binding is missing",
            ));
        }
        let expected = options
            .expected_database_identity
            .or(env_identity.as_deref());
        let identity = read_instance_id(&connection).map_err(|e| {
            if expected.is_some() {
                Error::new(
                    ErrorKind::OperatorProjectReplaced,
                    "the opened database does not match the operator lease",
                )
            } else {
                e
            }
        })?;
        if expected.is_some_and(|id| id != identity) {
            return Err(Error::new(
                ErrorKind::OperatorProjectReplaced,
                "the opened database does not match the operator lease",
            ));
        }
        if let (Some(explicit), Some(environment)) =
            (options.expected_database_identity, env_identity.as_deref())
        {
            if explicit != environment {
                return Err(Error::new(
                    ErrorKind::OperatorProjectReplaced,
                    "database identity bindings disagree",
                ));
            }
        }
        // The gate takes a consistent snapshot before any journal mutation.
        connection
            .execute_batch("BEGIN DEFERRED")
            .map_err(map_sqlite_error)?;
        let gate = schema::verify(&connection);
        connection
            .execute_batch("ROLLBACK")
            .map_err(map_sqlite_error)?;
        gate?;
        verify_project(
            &database,
            options.expected_project.or(child_project.as_ref()),
        )?;
        configure_pragmas(&connection, encrypted)?;
        Ok(Self {
            connection,
            database,
            instance_id: identity,
            encrypted,
        })
    }
    pub fn connection(&self) -> &Connection {
        &self.connection
    }
    pub fn database(&self) -> &Path {
        &self.database
    }
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }
    pub fn encrypted(&self) -> bool {
        self.encrypted
    }
    pub fn schema_fingerprint(&self) -> Result<String> {
        schema::fingerprint(&self.connection)
    }
    /// BEGIN IMMEDIATE, commit on success, rollback on error or unwind. Nested
    /// operations use `with_savepoint` on the supplied transaction.
    pub fn transaction<T>(
        &mut self,
        f: impl FnOnce(&mut Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let value = f(&mut tx)?;
        tx.commit().map_err(map_sqlite_error)?;
        Ok(value)
    }
    /// Hot-copy into an already initialized disposable destination. Creation,
    /// atomic publish/restore and rekey are deliberately R3 responsibilities.
    /// Cipher-to-plaintext copies are refused so this cannot decrypt a book.
    pub fn hot_backup(&self, destination: &mut Book) -> Result<()> {
        if self.encrypted != destination.encrypted {
            return Err(Error::new(
                ErrorKind::Validation,
                "backup requires matching encryption modes",
            ));
        }
        if self.database == destination.database {
            return Err(Error::new(
                ErrorKind::Validation,
                "backup destination must be a different database",
            ));
        }
        {
            let backup = Backup::new(&self.connection, &mut destination.connection)
                .map_err(map_sqlite_error)?;
            let deadline = Instant::now() + BUSY_TIMEOUT;
            loop {
                match backup.step(128).map_err(map_sqlite_error)? {
                    StepResult::Done => break,
                    _ if Instant::now() >= deadline => {
                        return Err(Error::new(
                            ErrorKind::DatabaseBusy,
                            "database backup did not finish before its deadline",
                        )
                        .retryable(true))
                    }
                    _ => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        }
        destination.instance_id = read_instance_id(&destination.connection)?;
        schema::verify(&destination.connection)?;
        Ok(())
    }
}
/// Connection field is dropped first, then the owner handles.
pub struct OwnedBook {
    book: Book,
    lease: kassiber_ownership::OwnerLease,
}
impl OwnedBook {
    pub fn close(self) -> std::result::Result<(), CloseFailure<Self>> {
        let Self { book, lease } = self;
        match book.close() {
            Ok(()) => {
                drop(lease);
                Ok(())
            }
            Err(failure) => Err(CloseFailure {
                resource: Box::new(Self {
                    book: *failure.resource,
                    lease,
                }),
                error: failure.error,
            }),
        }
    }
    /// Attach the lease retained across UnlockSession's attempts, preserving
    /// connection-before-lease drop order after successful authorization.
    pub fn from_open_book(book: Book, lease: kassiber_ownership::OwnerLease) -> Result<Self> {
        if book.database != lease.project().database {
            return Err(Error::new(
                ErrorKind::ProjectOwnerMismatch,
                "opened book does not match its ownership lease",
            ));
        }
        verify_project(&book.database, Some(lease.project()))?;
        Ok(Self { book, lease })
    }
    pub fn open(lease: kassiber_ownership::OwnerLease, options: OpenOptions<'_>) -> Result<Self> {
        let book = Book::open(
            lease
                .project()
                .database
                .parent()
                .expect("canonical database parent"),
            OpenOptions {
                passphrase: options.passphrase,
                expected_database_identity: options.expected_database_identity,
                expected_project: Some(lease.project()),
            },
        )?;
        Ok(Self { book, lease })
    }
    pub fn book(&self) -> &Book {
        &self.book
    }
    pub fn book_mut(&mut self) -> &mut Book {
        &mut self.book
    }
    pub fn lease(&self) -> &kassiber_ownership::OwnerLease {
        &self.lease
    }
}
pub fn with_savepoint<T>(
    transaction: &mut Transaction<'_>,
    f: impl FnOnce(&mut Savepoint<'_>) -> Result<T>,
) -> Result<T> {
    let mut savepoint = transaction.savepoint().map_err(map_sqlite_error)?;
    let value = f(&mut savepoint)?;
    savepoint.commit().map_err(map_sqlite_error)?;
    Ok(value)
}
fn verify_project(database: &Path, expected: Option<&CanonicalProject>) -> Result<()> {
    if let Some(expected) = expected {
        let current = OwnerNamespace::system()?
            .canonical_project(database.parent().expect("canonical database parent"))?;
        if current.identity != expected.identity || current.database != expected.database {
            return Err(Error::new(
                ErrorKind::OperatorProjectReplaced,
                "the project database was replaced",
            ));
        }
    }
    Ok(())
}
fn verify_operator_child_project(data_root: &Path) -> Result<Option<CanonicalProject>> {
    let child = std::env::var_os("KASSIBER_OPERATOR_CHILD").is_some_and(|v| v == "1");
    let expected = std::env::var_os("KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY");
    match (child, expected) {
        (false, None) => Ok(None),
        (true, Some(expected)) => {
            let project = OwnerNamespace::system()?.canonical_project(data_root)?;
            if expected.to_str() != Some(project.identity.as_str()) {
                return Err(Error::new(
                    ErrorKind::OperatorProjectReplaced,
                    "the operator project changed before the database was opened",
                )
                .details(json!({"project":project.public_id})));
            }
            Ok(Some(project))
        }
        (true, None) => Err(Error::new(
            ErrorKind::OperatorProjectBindingInvalid,
            "operator child project binding is missing",
        )),
        (false, Some(_)) => Err(Error::new(
            ErrorKind::OperatorProjectBindingInvalid,
            "operator project binding is only valid in a broker child",
        )),
    }
}
fn read_instance_id(conn: &Connection) -> Result<String> {
    let id: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key='database_instance_id'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| {
            if sqlite_is_busy(&e) {
                map_sqlite_error(e)
            } else {
                Error::migration("database_instance_id")
            }
        })?;
    id.filter(|s| valid_hex(s, 32))
        .ok_or_else(|| Error::migration("database_instance_id"))
}
pub fn valid_hex(text: &str, length: usize) -> bool {
    text.len() == length
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn apply_keying(conn: &Connection, passphrase: &str) -> Result<()> {
    if passphrase.is_empty() || passphrase.contains('\0') {
        return Err(Error::new(
            ErrorKind::InvalidPassphrase,
            "passphrase must not be empty or contain NUL bytes",
        ));
    }
    let sql = key_statement(passphrase);
    conn.execute_batch(&sql).map_err(map_sqlite_error)?;
    conn.execute_batch("PRAGMA cipher_compatibility = 4; PRAGMA kdf_iter = 256000; PRAGMA cipher_page_size = 4096;").map_err(map_sqlite_error)
}
fn key_statement(passphrase: &str) -> Zeroizing<String> {
    // Reserve the complete worst-case escaped statement before adding any
    // secret bytes; reallocating a growing String would free unwiped copies.
    let mut sql = Zeroizing::new(String::with_capacity(2 * passphrase.len() + 16));
    sql.push_str("PRAGMA key = '");
    for c in passphrase.chars() {
        sql.push(c);
        if c == '\'' {
            sql.push('\'');
        }
    }
    sql.push('\'');
    sql
}
/// Query the connection's own engine version. Unknown/vendor suffixes use
/// DELETE/FULL, including SQLCipher versions differing from Python's sqlite3.
pub fn journal_settings(version: &str) -> (&'static str, &'static str) {
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return ("delete", "FULL");
    }
    let parsed: Option<Vec<u64>> = parts.into_iter().map(|p| p.parse().ok()).collect();
    let Some(n) = parsed else {
        return ("delete", "FULL");
    };
    if n.as_slice() >= [3, 51, 3].as_slice()
        || (n[..2] == [3, 50] && n[2] >= 7)
        || (n[..2] == [3, 44] && n[2] >= 6)
    {
        ("wal", "NORMAL")
    } else {
        ("delete", "FULL")
    }
}
fn configure_pragmas(conn: &Connection, encrypted: bool) -> Result<()> {
    let version: String = conn
        .query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .map_err(map_sqlite_error)?;
    // During Python coexistence a newer embedded SQLite must never promote a
    // file to WAL behind a live, older Python connection. SQLCipher's shipped
    // Python driver requires DELETE/FULL. Plaintext preserves its existing
    // mode unless WAL is unsafe for this connection's own SQLite.
    let current: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .map_err(map_sqlite_error)?;
    let (journal, synchronous) = transition_journal_settings(&version, encrypted, &current);
    let actual: String = if current == journal {
        current.clone()
    } else {
        conn.query_row(&format!("PRAGMA journal_mode = {journal}"), [], |r| {
            r.get(0)
        })
        .map_err(|e| {
            Error::new(
                ErrorKind::DatabaseJournalModeUnavailable,
                "could not configure a safe database journal mode",
            )
            .retryable(sqlite_is_busy(&e))
            .details(json!({"required_journal_mode":journal}))
        })?
    };
    if actual.to_lowercase() != journal {
        return Err(Error::new(
            ErrorKind::DatabaseJournalModeUnavailable,
            "database did not accept the required safe journal mode",
        )
        .retryable(true)
        .details(json!({"required_journal_mode":journal,"actual_journal_mode":actual})));
    }
    conn.execute_batch(&format!("PRAGMA synchronous = {synchronous}; PRAGMA temp_store = MEMORY; PRAGMA cache_size = -16000; PRAGMA foreign_keys = ON;")).map_err(map_sqlite_error)?;
    conn.execute_batch(if encrypted {
        "PRAGMA mmap_size = 0;"
    } else {
        "PRAGMA mmap_size = 268435456;"
    })
    .map_err(map_sqlite_error)
}
fn transition_journal_settings<'a>(
    version: &str,
    encrypted: bool,
    current: &'a str,
) -> (&'a str, &'static str) {
    if encrypted
        || (current == "wal" && journal_settings(version).0 != "wal")
        || !["delete", "wal", "truncate", "persist"].contains(&current)
    {
        ("delete", "FULL")
    } else if current == "wal" {
        ("wal", "NORMAL")
    } else {
        (current, "FULL")
    }
}
fn sqlite_is_busy(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(code, _) if matches!(code.extended_code & 255, 5 | 6))
}
pub fn map_sqlite_error(error: rusqlite::Error) -> Error {
    let mut result = if sqlite_is_busy(&error) {
        Error::new(
            ErrorKind::DatabaseBusy,
            "the database is busy in another process",
        )
        .retryable(true)
    } else {
        Error::new(ErrorKind::DatabaseError, "database operation failed")
    };
    if let rusqlite::Error::SqliteFailure(code, _) = error {
        result.details = json!({"sqlite_error_code":code.extended_code});
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_literal_reserves_all_escaped_bytes_before_adding_the_secret() {
        let secret = "'".repeat(8191);
        let sql = key_statement(&secret);
        assert_eq!(sql.capacity(), 2 * secret.len() + 16);
        assert_eq!(sql.len(), 2 * secret.len() + 15);
        assert!(sql.starts_with("PRAGMA key = '"));
        assert_eq!(
            key_statement("synthetic '🔑").as_str(),
            "PRAGMA key = 'synthetic ''🔑'"
        );
    }
    #[test]
    fn transition_never_promotes_live_python_books_to_wal() {
        for version in ["3.51.1", "3.51.3", "unknown"] {
            assert_eq!(
                transition_journal_settings(version, true, "delete"),
                ("delete", "FULL")
            );
            assert_eq!(
                transition_journal_settings(version, true, "wal"),
                ("delete", "FULL")
            );
            assert_eq!(
                transition_journal_settings(version, false, "delete"),
                ("delete", "FULL")
            );
        }
        assert_eq!(
            transition_journal_settings("3.51.3", false, "wal"),
            ("wal", "NORMAL")
        );
        assert_eq!(
            transition_journal_settings("3.51.1", false, "wal"),
            ("delete", "FULL")
        );
    }
    #[test]
    fn wal_fix_versions_fail_closed() {
        for v in ["3.51.3", "3.50.7", "3.44.6", "3.52.0", "4.0.0"] {
            assert_eq!(journal_settings(v), ("wal", "NORMAL"));
        }
        for v in [
            "3.51.1",
            "3.50.6",
            "3.44.5",
            "3.51.3-vendor",
            "3.51",
            "3.51.٣",
            "999999999999999999999.0.0",
            "",
        ] {
            assert_eq!(journal_settings(v), ("delete", "FULL"));
        }
    }
    #[test]
    fn immediate_rollback_and_savepoints() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE test(value INTEGER)")
            .unwrap();
        let mut book = Book {
            connection: conn,
            database: PathBuf::new(),
            instance_id: String::new(),
            encrypted: false,
        };
        let failure: Result<()> = book.transaction(|tx| {
            tx.execute("INSERT INTO test VALUES(1)", [])
                .map_err(map_sqlite_error)?;
            Err(Error::migration("test"))
        });
        assert!(failure.is_err());
        assert_eq!(
            book.connection
                .query_row("SELECT count(*) FROM test", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        book.transaction(|tx| {
            tx.execute("INSERT INTO test VALUES(2)", [])
                .map_err(map_sqlite_error)?;
            let nested: Result<()> = with_savepoint(tx, |sp| {
                sp.execute("INSERT INTO test VALUES(3)", [])
                    .map_err(map_sqlite_error)?;
                Err(Error::migration("test"))
            });
            assert!(nested.is_err());
            Ok(())
        })
        .unwrap();
        assert_eq!(
            book.connection
                .query_row("SELECT sum(value) FROM test", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}

#[cfg(test)]
mod close_tests {
    use super::*;
    #[test]
    fn failed_close_retains_owner_until_connection_can_close() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("kassiber.sqlite3"),
            b"synthetic ownership identity",
        )
        .unwrap();
        let namespace = OwnerNamespace::isolated(temp.path().join("run/owners"));
        std::fs::create_dir(temp.path().join("run")).unwrap();
        let project = namespace.canonical_project(temp.path()).unwrap();
        let lease = namespace
            .acquire(
                project.clone(),
                kassiber_ownership::OwnerKind::Desktop,
                "close-test",
            )
            .unwrap();
        let connection = Connection::open_in_memory().unwrap();
        let mut statement = std::ptr::null_mut();
        let sql = std::ffi::CString::new("SELECT 1").unwrap();
        // SAFETY: the connection and SQL live through prepare. This intentional
        // raw outstanding statement injects SQLITE_BUSY into sqlite3_close;
        // it is finalized below before the retained resource is retried.
        let status = unsafe {
            rusqlite::ffi::sqlite3_prepare_v2(
                connection.handle(),
                sql.as_ptr(),
                -1,
                &mut statement,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(status, rusqlite::ffi::SQLITE_OK);
        let book = Book {
            connection,
            database: project.database.clone(),
            instance_id: "a".repeat(32),
            encrypted: false,
        };
        let failed = match (OwnedBook { book, lease }).close() {
            Ok(()) => panic!("expected close failure"),
            Err(failed) => failed,
        };
        assert_eq!(failed.error.kind, ErrorKind::DatabaseBusy);
        assert!(namespace
            .acquire(
                project.clone(),
                kassiber_ownership::OwnerKind::Desktop,
                "must-block"
            )
            .is_err());
        // SAFETY: statement was created above and its retained connection is live.
        assert_eq!(
            unsafe { rusqlite::ffi::sqlite3_finalize(statement) },
            rusqlite::ffi::SQLITE_OK
        );
        assert!(failed.resource.close().is_ok());
        assert!(namespace
            .acquire(project, kassiber_ownership::OwnerKind::Desktop, "released")
            .is_ok());
    }
}

//! Explicit SQLCipher maintenance. Callers must stop their own live connection
//! before entering; compatible ownership leases exclude other long-lived roles.
use crate::{
    apply_keying, durable, map_sqlite_error, read_instance_id, sqlite_is_busy, Error, ErrorKind,
    Result, BUSY_TIMEOUT,
};
use kassiber_ownership::{OwnerKind, OwnerLease, OwnerNamespace};
use rusqlite::{types::Value, Connection, OpenFlags};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const MARKERS: &[&[u8]] = &[
    b"slip77(",
    b"xprv9",
    b"tprv8",
    b"_TOKEN=",
    b"_PASSWORD=",
    b"Authorization:",
];

/// Create Python's empty SQLCipher container. Schema initialization and book
/// identity are deferred until the first explicit open, as in projects create.
pub fn create_empty_database(database: &Path, passphrase: &str) -> Result<()> {
    create_empty_with_observer(database, passphrase, &mut |_| Ok(()))
}
pub(crate) fn create_empty_with_observer(
    database: &Path,
    passphrase: &str,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    if passphrase.is_empty() || passphrase.contains('\0') {
        return Err(Error::new(
            ErrorKind::InvalidPassphrase,
            "passphrase must not be empty or contain NUL bytes",
        ));
    }
    // The data root resolves like Python's; links at the database and its
    // candidate still refuse.
    durable::refuse_symlinks(&durable::beside(database)?)?;
    if database.exists() {
        validate(database)?;
    }
    if database.exists() && database.metadata().map_err(Error::io)?.len() > 0 {
        return Err(Error::new(
            ErrorKind::DatabaseExists,
            "refusing to overwrite existing database",
        ));
    }
    if let Some(parent) = durable::beside(database)?.parent() {
        durable::directory(parent)?;
    }
    let state = find_resumable_state(database);
    if state.encrypted_temp {
        return Err(Error::new(
            ErrorKind::ResumeRequired,
            "refusing to overwrite an existing encrypted temporary database",
        ));
    }
    observer("empty.before_candidate_create")?;
    drop(durable::create_new(&durable::beside(
        &state.encrypted_temp_path,
    )?)?);
    observer("empty.after_candidate_create")?;
    let conn = Connection::open(&state.encrypted_temp_path).map_err(map_sqlite_error)?;
    apply_keying(&conn, passphrase)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS kassiber_init_marker(x INTEGER); DROP TABLE kassiber_init_marker;")
        .map_err(map_sqlite_error)?;
    conn.close().map_err(|(_, e)| map_sqlite_error(e))?;
    observer("empty.after_candidate_close")?;
    // This new empty file has no database identity yet. The caller's role
    // reservations protect publication; do not apply an existing-book binding
    // to a candidate that intentionally has no application schema.
    validate(&state.encrypted_temp_path)?;
    if state
        .encrypted_temp_path
        .metadata()
        .map_err(Error::io)?
        .len()
        == 0
    {
        return Err(verification());
    }
    let fresh = Connection::open_with_flags(
        &state.encrypted_temp_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(map_sqlite_error)?;
    apply_keying(&fresh, passphrase)?;
    integrity(&fresh)?;
    fresh.close().map_err(|(_, e)| map_sqlite_error(e))?;
    let parent = database.parent().expect("database parent");
    observer("empty.before_candidate_sync")?;
    sync(&state.encrypted_temp_path)?;
    observer("empty.after_candidate_file_sync")?;
    observer("empty.before_candidate_directory_sync")?;
    durable::sync_directory(parent)?;
    observer("empty.after_candidate_sync")?;
    // Only the previously validated empty placeholder may be replaced. A
    // competing creator's populated file must survive even under a race.
    if database.exists() {
        validate(database)?;
        if database.metadata().map_err(Error::io)?.len() != 0 {
            return Err(Error::new(
                ErrorKind::DatabaseExists,
                "refusing to overwrite existing database",
            ));
        }
        observer("empty.before_placeholder_remove")?;
        std::fs::remove_file(database).map_err(Error::io)?;
        observer("empty.before_placeholder_directory_sync")?;
        durable::sync_directory(parent)?;
        observer("empty.after_placeholder_remove")?;
    }
    observer("empty.before_publish_rename")?;
    kassiber_paths::durable::rename_noreplace(&state.encrypted_temp_path, database)
        .map_err(Error::io)?;
    observer("empty.after_publish_rename")?;
    observer("empty.before_publish_directory_sync")?;
    durable::sync_directory(parent)?;
    observer("empty.after_publish_directory_sync")
}

#[derive(Debug)]
pub struct EncryptionResult {
    pub encrypted_path: PathBuf,
    pub backup_path: PathBuf,
    pub plaintext_user_version: i64,
    pub plaintext_auto_vacuum: i64,
}
#[derive(Debug)]
pub struct ResumableState {
    pub plaintext: bool,
    pub encrypted_temp: bool,
    pub backup: bool,
    pub plaintext_path: PathBuf,
    pub encrypted_temp_path: PathBuf,
    pub backup_path: PathBuf,
}
pub fn find_resumable_state(path: &Path) -> ResumableState {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .replace(".sqlite3", "");
    let temporary = path.with_file_name(format!("{name}.encrypted.sqlite3"));
    let backup = path.with_file_name(format!("{name}.pre-encryption.sqlite3.bak"));
    ResumableState {
        plaintext: path.exists(),
        encrypted_temp: temporary.exists(),
        backup: backup.exists(),
        plaintext_path: path.into(),
        encrypted_temp_path: temporary,
        backup_path: backup,
    }
}
fn maintenance(
    namespace: &OwnerNamespace,
    root: &Path,
    active: Option<OwnerKind>,
) -> Result<Vec<OwnerLease>> {
    crate::verify_operator_child_project(root)?;
    let project = namespace.canonical_project(root)?;
    let mut leases = Vec::new();
    for kind in [OwnerKind::Broker, OwnerKind::Desktop] {
        if active != Some(kind) {
            leases.push(namespace.acquire(
                project.clone(),
                kind,
                &format!("maintenance-{}", std::process::id()),
            )?);
        }
    }
    Ok(leases)
}
fn validate(path: &Path) -> Result<()> {
    durable::refuse_symlinks(&durable::beside(path)?)?;
    kassiber_ownership::validate_database_file(path)
}
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
fn quote_secret(value: &str) -> Zeroizing<String> {
    let mut result = Zeroizing::new(String::with_capacity(2 * value.len() + 2));
    result.push('\'');
    for c in value.chars() {
        result.push(c);
        if c == '\'' {
            result.push('\'');
        }
    }
    result.push('\'');
    result
}
/// Explicit maintenance connection, authenticating before schema access without
/// initializing or migrating it. Callers must not treat it as an admitted book.
pub fn open_container(path: &Path, pass: Option<&str>) -> Result<Connection> {
    open_container_inner(path, pass, false)
}

/// Desktop preview authenticates an isolated incoming payload. It neither
/// adopts the active operator binding nor prints SQLCipher unlock diagnostics.
pub fn open_backup_container(path: &Path, pass: &str) -> Result<Connection> {
    open_container_inner(path, Some(pass), true)
}

fn open_container_inner(path: &Path, pass: Option<&str>, backup: bool) -> Result<Connection> {
    validate(path)?;
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(map_sqlite_error)?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(map_sqlite_error)?;
    if let Some(pass) = pass {
        apply_keying(&conn, pass)?;
    }
    let logging: Option<String> = if backup {
        let previous = conn
            .query_row("PRAGMA cipher_log_level", [], |r| r.get(0))
            .map_err(map_sqlite_error)?;
        conn.execute_batch("PRAGMA cipher_log_level=NONE")
            .map_err(map_sqlite_error)?;
        Some(previous)
    } else {
        None
    };
    let authenticated = conn
        .query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|e| {
            if pass.is_some() && !sqlite_is_busy(&e) {
                Error::new(
                    ErrorKind::UnlockFailed,
                    "wrong passphrase or unsupported database format",
                )
                .retryable(true)
            } else {
                map_sqlite_error(e)
            }
        });
    if let Some(level) = logging {
        let level = match level.as_str() {
            "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE" => level.as_str(),
            _ => "NONE",
        };
        conn.execute_batch(&format!("PRAGMA cipher_log_level={level}"))
            .map_err(map_sqlite_error)?;
    }
    authenticated?;
    if !backup && std::env::var_os("KASSIBER_OPERATOR_CHILD").is_some_and(|v| v == "1") {
        let expected =
            std::env::var("KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY").map_err(|_| {
                Error::new(
                    ErrorKind::OperatorProjectBindingInvalid,
                    "operator child database binding is missing",
                )
            })?;
        if read_instance_id(&conn)? != expected {
            return Err(Error::new(
                ErrorKind::OperatorProjectReplaced,
                "the opened database does not match the operator lease",
            ));
        }
    }
    Ok(conn)
}
fn open(path: &Path, pass: Option<&str>) -> Result<Connection> {
    open_container(path, pass)
}

/// Hot-copy an authenticated SQLCipher container without a schema migration.
/// The archive owner supplies a new private temporary destination. Keeping the
/// codec and online-copy loop here prevents a service-specific SQLite policy.
pub fn backup_encrypted(source: &Path, destination: &Path, passphrase: &str) -> Result<()> {
    if kassiber_paths::looks_like_plaintext(source)? {
        return Err(Error::new(
            ErrorKind::PlaintextDatabase,
            "database file at this path is plaintext SQLite",
        ));
    }
    let source = open(source, Some(passphrase))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(destination).map_err(Error::io)?;
    let mut target = Connection::open(destination).map_err(map_sqlite_error)?;
    apply_keying(&target, passphrase)?;
    crate::backup_connection(&source, &mut target)?;
    target.close().map_err(|(_, e)| map_sqlite_error(e))?;
    source.close().map_err(|(_, e)| map_sqlite_error(e))?;
    Ok(())
}
fn integrity(conn: &Connection) -> Result<()> {
    let mut statement = conn
        .prepare("PRAGMA integrity_check")
        .map_err(map_sqlite_error)?;
    let results: Vec<String> = statement
        .query_map([], |r| r.get(0))
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<_>>()
        .map_err(map_sqlite_error)?;
    if results != ["ok"] {
        return Err(verification());
    }
    let mut cipher = conn
        .prepare("PRAGMA cipher_integrity_check")
        .map_err(map_sqlite_error)?;
    if cipher
        .query([])
        .map_err(map_sqlite_error)?
        .next()
        .map_err(map_sqlite_error)?
        .is_some()
    {
        return Err(verification());
    }
    Ok(())
}
/// Inspect an existing keyed container without initializing or migrating its
/// schema. This is deliberately independent of `Book`'s current-schema gate:
/// `secrets verify` must also work on a newly created empty SQLCipher file.
pub fn verify_encrypted(database: &Path, passphrase: &str) -> Result<Verification> {
    if database.metadata().map_err(Error::io)?.len() == 0 {
        return Err(Error::new(
            ErrorKind::MissingDatabase,
            "database does not exist",
        ));
    }
    let conn = open(database, Some(passphrase))?;
    let integrity_check = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(map_sqlite_error)?;
    let cipher_integrity_check = conn
        .prepare("PRAGMA cipher_integrity_check")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .ok()
        .map(|rows| {
            if rows.is_empty() {
                "ok".into()
            } else {
                rows.join("; ")
            }
        });
    let sqlite_master_rows = conn
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
        .map_err(map_sqlite_error)?;
    conn.close().map_err(|(_, e)| map_sqlite_error(e))?;
    Ok(Verification {
        integrity_check,
        cipher_integrity_check,
        sqlite_master_rows,
    })
}

#[derive(Debug)]
pub struct Verification {
    pub integrity_check: String,
    pub cipher_integrity_check: Option<String>,
    pub sqlite_master_rows: i64,
}
fn verification() -> Error {
    Error::new(
        ErrorKind::MigrationVerificationFailed,
        "database migration verification failed",
    )
}
fn pragma(conn: &Connection, name: &str) -> Result<i64> {
    conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
        .map_err(map_sqlite_error)
}
fn sync(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(Error::io)
}
fn no_sidecars(path: &Path) -> Result<()> {
    if ["-wal", "-shm"]
        .iter()
        .any(|suffix| PathBuf::from(format!("{}{suffix}", path.display())).exists())
    {
        return Err(Error::new(
            ErrorKind::DatabaseBusy,
            "database sidecars remain after closing the migration source",
        )
        .retryable(true));
    }
    Ok(())
}
fn marker_scan(path: &Path) -> Result<()> {
    let mut file = File::open(path).map_err(Error::io)?;
    let overlap = MARKERS.iter().map(|v| v.len()).max().expect("markers") - 1;
    let mut buffer = vec![0; 4 * 1024 * 1024 + overlap];
    let mut tail = 0;
    loop {
        let n = file.read(&mut buffer[tail..]).map_err(Error::io)?;
        if n == 0 {
            break;
        }
        let total = tail + n;
        if MARKERS
            .iter()
            .any(|m| buffer[..total].windows(m.len()).any(|window| window == *m))
        {
            return Err(Error::new(
                ErrorKind::MigrationLeaksPlaintext,
                "encrypted database contains plaintext credential markers",
            ));
        }
        tail = overlap.min(total);
        buffer.copy_within(total - tail..total, 0);
    }
    Ok(())
}

/// Preserve schema SQL and every row, including duplicate rows in tables with
/// no primary key. This comparison is independent of the schema admission gate.
fn same_contents(left: &Connection, right: &Connection) -> Result<()> {
    type SchemaObject = (String, String, String, Option<String>);
    fn strings(conn: &Connection) -> Result<Vec<SchemaObject>> {
        conn.prepare("SELECT type,name,tbl_name,sql FROM sqlite_master ORDER BY type,name")
            .map_err(map_sqlite_error)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(map_sqlite_error)?
            .collect::<rusqlite::Result<_>>()
            .map_err(map_sqlite_error)
    }
    let objects = strings(left)?;
    if objects != strings(right)? {
        return Err(verification());
    }
    for name in ["user_version", "auto_vacuum"] {
        if pragma(left, name)? != pragma(right, name)? {
            return Err(verification());
        }
    }
    for (_, name, _, _) in objects.iter().filter(|v| v.0 == "table") {
        let table = format!("\"{}\"", name.replace('"', "\"\""));
        let columns: Vec<String> = left
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(map_sqlite_error)?
            .query_map([], |r| r.get(1))
            .map_err(map_sqlite_error)?
            .collect::<rusqlite::Result<_>>()
            .map_err(map_sqlite_error)?;
        let order = columns
            .iter()
            .map(|c| format!("\"{}\"", c.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT * FROM {table} ORDER BY {order}");
        let mut l = left.prepare(&sql).map_err(map_sqlite_error)?;
        let mut r = right.prepare(&sql).map_err(map_sqlite_error)?;
        let mut lrows = l.query([]).map_err(map_sqlite_error)?;
        let mut rrows = r.query([]).map_err(map_sqlite_error)?;
        loop {
            match (
                lrows.next().map_err(map_sqlite_error)?,
                rrows.next().map_err(map_sqlite_error)?,
            ) {
                (None, None) => break,
                (Some(a), Some(b)) => {
                    for i in 0..columns.len() {
                        if a.get::<_, Value>(i).map_err(map_sqlite_error)?
                            != b.get::<_, Value>(i).map_err(map_sqlite_error)?
                        {
                            return Err(verification());
                        }
                    }
                }
                _ => return Err(verification()),
            }
        }
    }
    Ok(())
}

/// Export to the Python-compatible sibling, verify fresh keyed pages, then
/// retain the original at Python's rollback path before publishing the sibling.
pub fn encrypt_plaintext(
    namespace: &OwnerNamespace,
    root: &Path,
    passphrase: &str,
    active_owner: Option<OwnerKind>,
) -> Result<EncryptionResult> {
    encrypt_with_observer(namespace, root, passphrase, active_owner, &mut |_| Ok(()))
}
pub(crate) fn encrypt_with_observer(
    namespace: &OwnerNamespace,
    root: &Path,
    passphrase: &str,
    active_owner: Option<OwnerKind>,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<EncryptionResult> {
    if passphrase.is_empty() || passphrase.contains('\0') {
        return Err(Error::new(
            ErrorKind::InvalidPassphrase,
            "passphrase must not be empty or contain NUL bytes",
        ));
    }
    let path = kassiber_paths::resolve_database_path(root);
    if !path.exists() {
        return Err(Error::new(
            ErrorKind::MissingDatabase,
            "plaintext database was not found",
        ));
    }
    validate(&path)?;
    let _leases = maintenance(namespace, root, active_owner)?;
    if !kassiber_paths::looks_like_plaintext(&path)? {
        return Err(Error::new(
            ErrorKind::NotPlaintextDatabase,
            "file does not look like plaintext SQLite",
        ));
    }
    let state = find_resumable_state(&path);
    for artifact in [&state.encrypted_temp_path, &state.backup_path] {
        durable::refuse_symlinks(&durable::beside(artifact)?)?;
    }
    if state.encrypted_temp {
        return Err(Error::new(
            ErrorKind::ResumeRequired,
            "refusing to overwrite an existing encrypted temporary database",
        ));
    }
    if state.backup {
        return Err(Error::new(
            ErrorKind::BackupExists,
            "refusing to overwrite an existing rollback backup",
        ));
    }
    let source = open(&path, None)?;
    let checkpoint: i64 = source
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
        .map_err(map_sqlite_error)?;
    if checkpoint != 0 {
        return Err(Error::new(
            ErrorKind::DatabaseBusy,
            "plaintext database WAL checkpoint is busy",
        )
        .retryable(true));
    }
    let mode: String = source
        .query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))
        .map_err(map_sqlite_error)?;
    if mode.to_lowercase() != "delete" {
        return Err(Error::new(
            ErrorKind::DatabaseBusy,
            "plaintext database remains in WAL mode",
        )
        .retryable(true));
    }
    let version = pragma(&source, "user_version")?;
    let vacuum = pragma(&source, "auto_vacuum")?;
    observer("encryption.before_export")?;
    durable::create_new(&durable::beside(&state.encrypted_temp_path)?)?;
    observer("encryption.after_candidate_create")?;
    let path_text = state
        .encrypted_temp_path
        .to_str()
        .ok_or_else(|| Error::new(ErrorKind::Validation, "database path is not valid Unicode"))?;
    let secret_literal = quote_secret(passphrase);
    let attach = Zeroizing::new(format!(
        "ATTACH DATABASE {} AS encrypted KEY {}",
        quote(path_text),
        secret_literal.as_str()
    ));
    source.execute_batch(&attach).map_err(map_sqlite_error)?;
    source.execute_batch(&format!("PRAGMA encrypted.cipher_compatibility=4; PRAGMA encrypted.kdf_iter=256000; PRAGMA encrypted.cipher_page_size=4096; PRAGMA encrypted.auto_vacuum={vacuum};")).map_err(map_sqlite_error)?;
    source
        .query_row("SELECT sqlcipher_export('encrypted')", [], |_| Ok(()))
        .map_err(map_sqlite_error)?;
    source
        .execute_batch(&format!(
            "PRAGMA encrypted.user_version={version}; DETACH DATABASE encrypted;"
        ))
        .map_err(map_sqlite_error)?;
    source.close().map_err(|(_, e)| map_sqlite_error(e))?;
    observer("encryption.after_export")?;
    no_sidecars(&path)?;
    let fresh = open(&state.encrypted_temp_path, Some(passphrase))?;
    integrity(&fresh)?;
    same_contents(&open(&path, None)?, &fresh)?;
    fresh.close().map_err(|(_, e)| map_sqlite_error(e))?;
    marker_scan(&state.encrypted_temp_path)?;
    observer("encryption.before_verified_sync")?;
    sync(&state.encrypted_temp_path)?;
    durable::sync_directory(root)?;
    observer("encryption.after_verified_sync")?;
    publish(&state, observer)?;
    Ok(EncryptionResult {
        encrypted_path: path,
        backup_path: state.backup_path,
        plaintext_user_version: version,
        plaintext_auto_vacuum: vacuum,
    })
}
fn publish(state: &ResumableState, observer: &mut impl FnMut(&str) -> Result<()>) -> Result<()> {
    let parent = state.plaintext_path.parent().expect("database parent");
    if state.plaintext {
        observer("encryption.before_backup_rename")?;
        kassiber_paths::durable::rename_noreplace(&state.plaintext_path, &state.backup_path)
            .map_err(Error::io)?;
        observer("encryption.after_backup_rename")?;
        observer("encryption.before_backup_directory_sync")?;
        durable::sync_directory(parent)?;
        observer("encryption.after_backup_directory_sync")?;
    }
    observer("encryption.before_publish_rename")?;
    kassiber_paths::durable::rename_noreplace(&state.encrypted_temp_path, &state.plaintext_path)
        .map_err(Error::io)?;
    observer("encryption.after_publish_rename")?;
    observer("encryption.before_publish_directory_sync")?;
    durable::sync_directory(parent)?;
    observer("encryption.after_publish_directory_sync")
}

/// Explicit recovery only. A candidate must match the retained plaintext
/// source/backup completely; unknown or conflicting states are refused.
pub fn resume_encryption(namespace: &OwnerNamespace, root: &Path, passphrase: &str) -> Result<()> {
    let path = kassiber_paths::resolve_database_path(root);
    let state = find_resumable_state(&path);
    let _leases = maintenance(namespace, root, None)?;
    if !state.encrypted_temp {
        return Err(Error::new(
            ErrorKind::ResumeRequired,
            "no encrypted candidate is available to resume",
        ));
    }
    if state.plaintext == state.backup {
        return Err(Error::new(
            ErrorKind::ResumeRequired,
            "ambiguous encryption recovery state",
        ));
    }
    let source = if state.plaintext {
        &state.plaintext_path
    } else {
        &state.backup_path
    };
    if !kassiber_paths::looks_like_plaintext(source)? {
        return Err(verification());
    }
    let candidate = open(&state.encrypted_temp_path, Some(passphrase))?;
    integrity(&candidate)?;
    same_contents(&open(source, None)?, &candidate)?;
    candidate.close().map_err(|(_, e)| map_sqlite_error(e))?;
    marker_scan(&state.encrypted_temp_path)?;
    sync(&state.encrypted_temp_path)?;
    publish(&state, &mut |_| Ok(()))
}

/// Authenticate first, arm the caller's native-credential stale guards, rekey
/// in place, then verify on a fresh connection. Never clears guards on failure.
pub fn change_passphrase(
    namespace: &OwnerNamespace,
    root: &Path,
    current: &str,
    new: &str,
    active_owner: Option<OwnerKind>,
    before_rekey: impl FnOnce() -> Result<()>,
) -> Result<()> {
    rekey_with_observer(
        namespace,
        root,
        current,
        new,
        active_owner,
        before_rekey,
        &mut |_| Ok(()),
    )
}
/// Explicit observation seam for maintenance lifecycle crash tests. Observers
/// are supplied by the caller, never selected through product environment.
pub fn rekey_with_observer(
    namespace: &OwnerNamespace,
    root: &Path,
    current: &str,
    new: &str,
    active_owner: Option<OwnerKind>,
    before_rekey: impl FnOnce() -> Result<()>,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    if new.is_empty() || new.contains('\0') {
        return Err(Error::new(
            ErrorKind::InvalidPassphrase,
            "passphrase must not be empty or contain NUL bytes",
        ));
    }
    let path = kassiber_paths::resolve_database_path(root);
    let _leases = maintenance(namespace, root, active_owner)?;
    if kassiber_paths::looks_like_plaintext(&path)? {
        return Err(Error::new(
            ErrorKind::PlaintextDatabase,
            "database is plaintext; initialize encryption first",
        ));
    }
    let conn = open(&path, Some(current))?;
    before_rekey()?;
    observer("rekey.before_rekey")?;
    let literal = quote_secret(new);
    let sql = Zeroizing::new(format!("PRAGMA rekey={}", literal.as_str()));
    conn.execute_batch(&sql).map_err(map_sqlite_error)?;
    observer("rekey.after_rekey")?;
    conn.close().map_err(|(_, e)| map_sqlite_error(e))?;
    let verify = open(&path, Some(new))?;
    integrity(&verify).map_err(|e| {
        if e.kind == ErrorKind::MigrationVerificationFailed {
            Error::new(
                ErrorKind::RekeyVerificationFailed,
                "rekeyed database verification failed",
            )
        } else {
            e
        }
    })?;
    verify.close().map_err(|(_, e)| map_sqlite_error(e))?;
    sync(&path)?;
    durable::sync_directory(root)?;
    observer("rekey.after_verified_sync")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_verification_never_recreates_a_missing_or_zero_byte_candidate() {
        for remove in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("kassiber.sqlite3");
            std::fs::write(&path, []).unwrap();
            let candidate = find_resumable_state(&path).encrypted_temp_path;
            let result = create_empty_with_observer(&path, "synthetic key", &mut |label| {
                if label == "empty.after_candidate_close" {
                    if remove {
                        std::fs::remove_file(&candidate)
                    } else {
                        std::fs::write(&candidate, [])
                    }
                    .map_err(Error::io)?;
                }
                Ok(())
            });
            assert!(result.is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"");
            assert_eq!(candidate.exists(), !remove);
            if !remove {
                assert_eq!(
                    verify_encrypted(&candidate, "synthetic key")
                        .unwrap_err()
                        .kind,
                    ErrorKind::MissingDatabase
                );
            }
        }
    }
    #[test]
    fn corrupt_empty_candidate_never_replaces_the_live_placeholder() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("kassiber.sqlite3");
        std::fs::write(&path, []).unwrap();
        let candidate = find_resumable_state(&path).encrypted_temp_path;
        let result = create_empty_with_observer(&path, "synthetic key", &mut |label| {
            if label == "empty.after_candidate_close" {
                std::fs::write(&candidate, b"corrupt candidate").map_err(Error::io)?;
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"");
        assert_eq!(std::fs::read(&candidate).unwrap(), b"corrupt candidate");
    }
    #[cfg(unix)]
    #[test]
    fn data_roots_resolve_but_linked_databases_and_candidates_refuse() {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir().unwrap();
        let (alias, real) = crate::durable::tests::aliased(temporary.path());
        let path = alias.join("empty/data/kassiber.sqlite3");
        create_empty_database(&path, "synthetic key").unwrap();
        assert!(
            verify_encrypted(&real.join("empty/data/kassiber.sqlite3"), "synthetic key").is_ok()
        );
        let namespace = OwnerNamespace::isolated(temporary.path().join("owners"));
        let data = alias.join("plain/data");
        drop(crate::Book::create(&data, None).unwrap());
        let result = encrypt_plaintext(&namespace, &data, "synthetic key", None).unwrap();
        assert_eq!(result.encrypted_path, data.join("kassiber.sqlite3"));
        assert!(real
            .join("plain/data/kassiber.pre-encryption.sqlite3.bak")
            .is_file());
        let refused = |result: Result<()>| {
            assert_eq!(result.unwrap_err().kind, ErrorKind::UnsafeConfiguration);
        };
        let linked = real.join("linked/data");
        std::fs::create_dir_all(&linked).unwrap();
        symlink("elsewhere.sqlite3", linked.join("kassiber.sqlite3")).unwrap();
        refused(create_empty_database(
            &alias.join("linked/data/kassiber.sqlite3"),
            "key",
        ));
        assert!(!linked.join("elsewhere.sqlite3").exists());
        let data = alias.join("candidate/data");
        drop(crate::Book::create(&data, None).unwrap());
        symlink(
            "elsewhere.sqlite3",
            real.join("candidate/data/kassiber.encrypted.sqlite3"),
        )
        .unwrap();
        refused(encrypt_plaintext(&namespace, &data, "synthetic key", None).map(drop));
        assert!(!real.join("candidate/data/elsewhere.sqlite3").exists());
    }
    #[test]
    fn empty_container_is_verified_private_and_never_overwrites_a_book_or_candidate() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("data/kassiber.sqlite3");
        create_empty_database(&path, "synthetic '🔑").unwrap();
        let result = verify_encrypted(&path, "synthetic '🔑").unwrap();
        assert_eq!(result.sqlite_master_rows, 0);
        assert_eq!(result.integrity_check, "ok");
        assert_eq!(result.cipher_integrity_check.as_deref(), Some("ok"));
        assert_eq!(
            verify_encrypted(&path, "wrong key").unwrap_err().kind,
            ErrorKind::UnlockFailed
        );
        let before = std::fs::read(&path).unwrap();
        assert_eq!(
            create_empty_database(&path, "other key").unwrap_err().kind,
            ErrorKind::DatabaseExists
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(&path).unwrap();
        let candidate = find_resumable_state(&path).encrypted_temp_path;
        std::fs::write(&candidate, b"retained interrupted candidate").unwrap();
        assert_eq!(
            create_empty_database(&path, "other key").unwrap_err().kind,
            ErrorKind::ResumeRequired
        );
        assert_eq!(
            std::fs::read(&candidate).unwrap(),
            b"retained interrupted candidate"
        );
        assert!(!path.exists());
    }

    #[test]
    fn verification_accepts_historical_schema_without_migrating_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("kassiber.sqlite3");
        create_empty_database(&path, "synthetic key").unwrap();
        let conn = open(&path, Some("synthetic key")).unwrap();
        conn.execute_batch("CREATE TABLE historical(value TEXT); INSERT INTO historical VALUES('exact value'); PRAGMA user_version=37").unwrap();
        conn.close().unwrap();
        let before = std::fs::read(&path).unwrap();
        let result = verify_encrypted(&path, "synthetic key").unwrap();
        assert_eq!(result.sqlite_master_rows, 1);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let conn = open(&path, Some("synthetic key")).unwrap();
        assert_eq!(pragma(&conn, "user_version").unwrap(), 37);
        assert_eq!(
            conn.query_row("SELECT value FROM historical", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "exact value"
        );
    }

    #[test]
    fn failed_empty_publication_retains_an_inspectable_candidate() {
        for point in [
            "empty.before_candidate_create",
            "empty.after_candidate_create",
            "empty.after_candidate_close",
            "empty.before_candidate_sync",
            "empty.after_candidate_sync",
            "empty.before_placeholder_remove",
            "empty.after_placeholder_remove",
            "empty.before_publish_rename",
            "empty.after_publish_rename",
            "empty.after_publish_directory_sync",
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("kassiber.sqlite3");
            std::fs::write(&path, []).unwrap();
            assert!(
                create_empty_with_observer(&path, "synthetic key", &mut |label| {
                    if point == label {
                        Err(Error::new(ErrorKind::Io, "injected failure"))
                    } else {
                        Ok(())
                    }
                })
                .is_err()
            );
            let state = find_resumable_state(&path);
            let complete = if path.metadata().is_ok_and(|m| m.len() > 0) {
                &path
            } else {
                &state.encrypted_temp_path
            };
            if complete.metadata().is_ok_and(|m| m.len() > 0) {
                assert_eq!(
                    verify_encrypted(complete, "synthetic key")
                        .unwrap()
                        .sqlite_master_rows,
                    0
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    assert_eq!(
                        complete.metadata().unwrap().permissions().mode() & 0o777,
                        0o600
                    );
                }
            }
        }
    }
    #[test]
    fn native_guard_callback_runs_only_after_authentication_and_stays_armed_on_ambiguity() {
        use std::cell::Cell;
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        drop(crate::Book::create(&data, Some("current key")).unwrap());
        let namespace = OwnerNamespace::isolated(root.path().join("owners"));
        let armed = Cell::new(false);
        assert!(
            change_passphrase(&namespace, &data, "wrong key", "new key", None, || {
                armed.set(true);
                Ok(())
            })
            .is_err()
        );
        assert!(!armed.get());
        assert!(rekey_with_observer(
            &namespace,
            &data,
            "current key",
            "new key",
            None,
            || {
                armed.set(true);
                Ok(())
            },
            &mut |label| if label == "rekey.after_rekey" {
                Err(Error::new(ErrorKind::Io, "injected post-rekey failure"))
            } else {
                Ok(())
            }
        )
        .is_err());
        assert!(armed.get());
        integrity(&open(&data.join("kassiber.sqlite3"), Some("new key")).unwrap()).unwrap();
    }
    #[test]
    fn encryption_and_rekey_preserve_identity_and_rollback_copy() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let book = crate::Book::create(&data, None).unwrap();
        let identity = book.instance_id().to_owned();
        book.connection().execute_batch("PRAGMA user_version=37; INSERT INTO settings VALUES('synthetic','_TOKEN=keep in cipher only')").unwrap();
        drop(book);
        let namespace = OwnerNamespace::isolated(root.path().join("owners"));
        let result = encrypt_plaintext(&namespace, &data, "synthetic key", None).unwrap();
        assert_eq!(result.plaintext_user_version, 37);
        assert!(kassiber_paths::looks_like_plaintext(&result.backup_path).unwrap());
        assert!(!kassiber_paths::looks_like_plaintext(&result.encrypted_path).unwrap());
        assert_eq!(
            read_instance_id(&open(&result.encrypted_path, Some("synthetic key")).unwrap())
                .unwrap(),
            identity
        );
        change_passphrase(
            &namespace,
            &data,
            "synthetic key",
            "rotated key",
            None,
            || Ok(()),
        )
        .unwrap();
        assert_eq!(
            read_instance_id(&open(&result.encrypted_path, Some("rotated key")).unwrap()).unwrap(),
            identity
        );
    }
    #[test]
    fn every_publication_failure_retains_verified_recoverable_state() {
        for point in [
            "encryption.after_export",
            "encryption.after_verified_sync",
            "encryption.before_backup_rename",
            "encryption.after_backup_rename",
            "encryption.after_backup_directory_sync",
            "encryption.before_publish_rename",
            "encryption.after_publish_rename",
            "encryption.after_publish_directory_sync",
        ] {
            let root = tempfile::tempdir().unwrap();
            let data = root.path().join("data");
            drop(crate::Book::create(&data, None).unwrap());
            let namespace = OwnerNamespace::isolated(root.path().join("owners"));
            assert!(encrypt_with_observer(
                &namespace,
                &data,
                "synthetic key",
                None,
                &mut |label| {
                    if label == point {
                        Err(Error::new(ErrorKind::Io, "injected migration failure"))
                    } else {
                        Ok(())
                    }
                }
            )
            .is_err());
            let state = find_resumable_state(&data.join("kassiber.sqlite3"));
            if state.encrypted_temp {
                resume_encryption(&namespace, &data, "synthetic key").unwrap();
            }
            let conn = open(&data.join("kassiber.sqlite3"), Some("synthetic key")).unwrap();
            integrity(&conn).unwrap();
        }
    }
}

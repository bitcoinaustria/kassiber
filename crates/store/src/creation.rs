//! Empty-book creation, kept separate from the R1 admission gate.
use crate::{
    apply_keying, configure_pragmas, durable, map_sqlite_error, read_instance_id, schema, Book,
    Error, ErrorKind, Result, BUSY_TIMEOUT,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::path::Path;

/// Install only into an empty schema. A failed statement rolls back all DDL,
/// audit sentinels and the graph change-feed seed together.
pub(crate) fn install(
    conn: &mut Connection,
    identity: Option<&str>,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let empty: bool = conn
        .query_row("SELECT NOT EXISTS(SELECT 1 FROM sqlite_master)", [], |r| {
            r.get(0)
        })
        .map_err(map_sqlite_error)?;
    if !empty {
        return Err(Error::new(
            ErrorKind::DatabaseExists,
            "schema creation requires an empty database",
        ));
    }
    let transcript: Value = serde_json::from_str(include_str!("../fixtures/creation.json"))
        .expect("checked creation transcript");
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_sqlite_error)?;
    for (index, statement) in transcript["statements"]
        .as_array()
        .expect("statements")
        .iter()
        .enumerate()
    {
        observer(&format!("schema.before_statement.{index}"))?;
        tx.execute_batch(statement.as_str().expect("SQL string"))
            .map_err(map_sqlite_error)?;
        observer(&format!("schema.after_statement.{index}"))?;
    }
    let identity = match identity {
        Some(value) if crate::valid_hex(value, 32) => value.to_owned(),
        Some(_) => {
            return Err(Error::new(
                ErrorKind::Validation,
                "invalid database identity",
            ))
        }
        None => {
            let mut bytes: Vec<u8> = tx
                .query_row("SELECT randomblob(16)", [], |r| r.get(0))
                .map_err(map_sqlite_error)?;
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            bytes.iter().map(|b| format!("{b:02x}")).collect()
        }
    };
    tx.execute(
        "INSERT INTO settings(key,value) VALUES('database_instance_id',?)",
        [&identity],
    )
    .map_err(map_sqlite_error)?;
    schema::verify(&tx)?;
    observer("schema.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("schema.after_commit")
}

impl Book {
    /// Create an owner-only book, or resume an interrupted empty-schema create.
    /// Existing nonempty schemas are refused. Layout publication is separate.
    pub fn create(data_root: &Path, passphrase: Option<&str>) -> Result<Self> {
        create(data_root, passphrase, None, None, &mut |_| Ok(()))
    }

    /// Packaging supplies the shipped Python plaintext SQLite version while
    /// Python remains supported. Unknown/older runtimes retain DELETE/FULL.
    /// SQLCipher always uses DELETE/FULL during coexistence.
    pub fn create_for_python_runtime(
        data_root: &Path,
        passphrase: Option<&str>,
        python_plaintext_sqlite_version: &str,
    ) -> Result<Self> {
        let book = Self::create(data_root, passphrase)?;
        if passphrase.is_none()
            && crate::journal_settings(python_plaintext_sqlite_version).0 == "wal"
        {
            let version: String = book
                .connection
                .query_row("SELECT sqlite_version()", [], |r| r.get(0))
                .map_err(map_sqlite_error)?;
            if crate::journal_settings(&version).0 != "wal" {
                return Err(Error::new(
                    ErrorKind::DatabaseJournalModeUnavailable,
                    "reference runtime WAL policy is unsafe for the Rust SQLite build",
                ));
            }
            let mode: String = book
                .connection
                .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
                .map_err(map_sqlite_error)?;
            if mode != "wal" {
                return Err(Error::new(
                    ErrorKind::DatabaseJournalModeUnavailable,
                    "new book did not accept the reference runtime journal mode",
                ));
            }
            book.connection
                .execute_batch("PRAGMA synchronous=NORMAL")
                .map_err(map_sqlite_error)?;
        }
        Ok(book)
    }
}

pub(crate) fn create(
    root: &Path,
    passphrase: Option<&str>,
    identity: Option<&str>,
    timestamp: Option<&str>,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Book> {
    crate::verify_operator_child_project(root)?;
    if std::env::var_os("KASSIBER_OPERATOR_CHILD").is_some_and(|v| v == "1") {
        // A newly generated database cannot satisfy an existing broker
        // database-identity lease. Refuse before creating even an empty file.
        return Err(Error::new(
            ErrorKind::OperatorProjectReplaced,
            "an operator child cannot create a replacement database",
        ));
    }
    if passphrase.is_some_and(|v| v.is_empty() || v.contains('\0')) {
        return Err(Error::new(
            ErrorKind::InvalidPassphrase,
            "passphrase must not be empty or contain NUL bytes",
        ));
    }
    if identity.is_some_and(|v| !crate::valid_hex(v, 32)) {
        return Err(Error::new(
            ErrorKind::Validation,
            "invalid database identity",
        ));
    }
    // Python resolves the data root itself; a link at the database still refuses.
    let root = &durable::under(root, root)?;
    durable::directory(root)?;
    let database = root.join(kassiber_paths::DB_FILENAME);
    durable::refuse_symlinks(&database)?;
    let file = if database.exists() {
        kassiber_ownership::validate_database_file(&database)?;
        if database.metadata().map_err(Error::io)?.len() > 0 {
            let plain = kassiber_paths::looks_like_plaintext(&database)?;
            if plain && passphrase.is_some() {
                return Err(Error::new(
                    ErrorKind::PlaintextDatabase,
                    "database file at this path is plaintext SQLite",
                ));
            }
            if !plain && passphrase.is_none() {
                return Err(Error::new(
                    ErrorKind::PassphraseRequired,
                    "database is encrypted; supply a passphrase",
                ));
            }
        }
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&database)
            .map_err(Error::io)?
    } else {
        durable::create_new(&database)?
    };
    observer("schema.after_file_create")?;
    crate::seed_test_prng();
    let mut conn = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(map_sqlite_error)?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(map_sqlite_error)?;
    if let Some(pass) = passphrase {
        apply_keying(&conn, pass)?;
    }
    if conn
        .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master)", [], |r| {
            r.get::<_, bool>(0)
        })
        .map_err(map_sqlite_error)?
    {
        return Err(Error::new(
            ErrorKind::DatabaseExists,
            "schema creation requires an empty database",
        ));
    }
    if let Some(timestamp) = timestamp {
        let timestamp = timestamp.to_owned();
        conn.create_scalar_function(
            "strftime",
            2,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| Ok(timestamp.clone()),
        )
        .map_err(map_sqlite_error)?;
    }
    configure_pragmas(&conn, passphrase.is_some())?;
    install(&mut conn, identity, observer)?;
    let checkpoint: i64 = conn
        .query_row("PRAGMA wal_checkpoint(FULL)", [], |r| r.get(0))
        .map_err(map_sqlite_error)?;
    if checkpoint != 0 {
        return Err(
            Error::new(ErrorKind::DatabaseBusy, "new database checkpoint is busy").retryable(true),
        );
    }
    // SQLCipher and SQLite flush their journal at commit. Explicitly flush
    // the file and parent too, so publication survives directory-entry loss.
    file.sync_all().map_err(Error::io)?;
    durable::sync_directory(root)?;
    observer("schema.after_directory_sync")?;
    let instance_id = read_instance_id(&conn)?;
    let database = kassiber_paths::canonicalize(&database).map_err(Error::io)?;
    Ok(Book {
        connection: conn,
        database,
        instance_id,
        encrypted: passphrase.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn data_roots_resolve_but_a_linked_database_is_refused() {
        let temporary = tempfile::tempdir().unwrap();
        let (alias, real) = crate::durable::tests::aliased(temporary.path());
        let book = Book::create(&alias.join("book/data"), None).unwrap();
        assert_eq!(book.database, real.join("book/data/kassiber.sqlite3"));
        let linked = real.join("linked/data");
        std::fs::create_dir_all(&linked).unwrap();
        std::os::unix::fs::symlink("elsewhere.sqlite3", linked.join("kassiber.sqlite3")).unwrap();
        let error = Book::create(&alias.join("linked/data"), None)
            .err()
            .unwrap();
        assert_eq!(error.code, "unsafe_configuration");
        assert!(!linked.join("elsewhere.sqlite3").exists());
        // Python creates the schema through such a link; this store refuses.
        std::fs::write(linked.join("elsewhere.sqlite3"), b"").unwrap();
        let error = Book::create(&alias.join("linked/data"), None)
            .err()
            .unwrap();
        assert_eq!(error.code, "unsafe_configuration");
        assert_eq!(
            std::fs::read(linked.join("elsewhere.sqlite3")).unwrap(),
            b""
        );
    }
    #[test]
    fn new_books_follow_explicit_shipped_python_policy() {
        let root = tempfile::tempdir().unwrap();
        for (name, pass, version, expected) in [
            ("old", None, "3.51.1", "delete"),
            ("fixed", None, "3.53.1", "wal"),
            ("cipher", Some("synthetic"), "3.53.1", "delete"),
        ] {
            let book =
                Book::create_for_python_runtime(&root.path().join(name), pass, version).unwrap();
            assert_eq!(
                book.connection
                    .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                expected
            );
        }
    }
    #[test]
    fn initial_schema_is_transactional_and_open_returns_ready() {
        let root = tempfile::tempdir().unwrap();
        for (name, pass) in [("plain", None), ("cipher", Some("synthetic key"))] {
            let book = Book::create(&root.path().join(name), pass).unwrap();
            assert!(book.connection().is_autocommit());
            assert_eq!(book.instance_id().len(), 32);
            assert_eq!(
                book.connection()
                    .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                book.connection()
                    .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "ok"
            );
            assert!(book
                .connection()
                .prepare("PRAGMA foreign_key_check")
                .unwrap()
                .query([])
                .unwrap()
                .next()
                .unwrap()
                .is_none());
            schema::verify(book.connection()).unwrap();
        }
    }
    #[test]
    fn failure_at_every_schema_statement_rolls_back_all_objects() {
        let statements: Value =
            serde_json::from_str(include_str!("../fixtures/creation.json")).unwrap();
        for index in 0..statements["statements"].as_array().unwrap().len() {
            for side in ["before", "after"] {
                let mut conn = Connection::open_in_memory().unwrap();
                let target = format!("schema.{side}_statement.{index}");
                let result = install(&mut conn, None, &mut |label| {
                    if label == target {
                        Err(Error::new(ErrorKind::Io, "injected write failure"))
                    } else {
                        Ok(())
                    }
                });
                assert!(result.is_err(), "{target}");
                assert!(conn.is_autocommit());
                assert_eq!(
                    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r
                        .get::<_, i64>(0))
                        .unwrap(),
                    0,
                    "{target}"
                );
            }
        }
    }
}

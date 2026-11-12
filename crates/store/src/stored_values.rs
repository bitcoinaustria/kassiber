//! Stored values Kassiber does not convert: NaN or infinity, integers beyond
//! 64 bits and text with an unpaired surrogate, where a migration step or the
//! admission gate has to read them exactly. Python's open interpreted some of
//! them with CPython semantics and raised on others; Kassiber refuses the
//! book instead (owner decision 2026-11-11, plan 22), and refuses it before
//! anything is written: a book that still needs migrating is first migrated
//! on a private in-memory copy, and a stored-value refusal there refuses the
//! open with the book file untouched.
use crate::{compat, custody_hooks::MigrationClock, map_sqlite_error, schema, Error, ErrorKind};
use rusqlite::{backup::Backup, Connection};
use serde_json::json;

/// The shape of a stored value Kassiber refuses, when the site knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    NonFinite,
    BeyondI64,
    LoneSurrogate,
    /// A collection, an unparsable or too-deep document, or a value whose
    /// conversion has no exact result.
    Other,
}

pub(crate) const REASON: &str = "unsupported_stored_value";

/// The refusal every such site returns: `unsupported_schema_layout`, reason
/// `unsupported_stored_value` and, where the site knows it, the value's shape.
pub(crate) fn refusal(shape: Shape) -> Error {
    let (message, value) = match shape {
        Shape::NonFinite => (
            "this book stores NaN or infinity where Kassiber needs an exact number",
            Some("non_finite_number"),
        ),
        Shape::BeyondI64 => (
            "this book stores an integer beyond 64 bits that Kassiber cannot convert exactly",
            Some("integer_beyond_64_bits"),
        ),
        Shape::LoneSurrogate => (
            "this book stores text with an unpaired surrogate that Kassiber cannot convert exactly",
            Some("lone_surrogate"),
        ),
        Shape::Other => (
            "this book stores a value Kassiber cannot convert exactly",
            None,
        ),
    };
    let details = match value {
        Some(value) => json!({"reason": REASON, "value": value}),
        None => json!({"reason": REASON}),
    };
    Error::new(ErrorKind::UnsupportedSchemaLayout, message)
        .hint("Kassiber does not open books that hold such values.")
        .details(details)
}

fn is_refusal(error: &Error) -> bool {
    error.kind == ErrorKind::UnsupportedSchemaLayout && error.details["reason"] == REASON
}

/// Whether the ordered pass may have layout work to do: an object of the
/// current schema (or one it retires) differs by type and name, or a required
/// audit is missing. Archived `gl_*` objects are ignored; a book whose names
/// and audits are current (historical `ALTER` layouts included) skips the
/// rehearsal, and the gate's own stored-value checks still refuse it before
/// the pass changes anything.
pub(crate) fn layout_pending(conn: &Connection) -> crate::Result<bool> {
    let manifest: serde_json::Value =
        serde_json::from_str(schema::MANIFEST).expect("checked-in schema manifest");
    let expected: std::collections::BTreeSet<(String, String)> = manifest["objects"]
        .as_array()
        .expect("schema objects")
        .iter()
        .map(|object| {
            (
                object[0].as_str().expect("type").to_owned(),
                object[1].as_str().expect("name").to_owned(),
            )
        })
        .collect();
    let mut statement = conn
        .prepare(
            r"SELECT type,name FROM sqlite_master WHERE name NOT GLOB 'sqlite_*' AND NOT (name LIKE 'gl\_%' ESCAPE '\' OR (type='index' AND tbl_name LIKE 'gl\_%' ESCAPE '\'))",
        )
        .map_err(map_sqlite_error)?;
    let actual: std::collections::BTreeSet<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<_>>()
        .map_err(map_sqlite_error)?;
    if actual != expected {
        return Ok(true);
    }
    let mut statement = conn
        .prepare("SELECT id FROM schema_migration_audits")
        .map_err(map_sqlite_error)?;
    let audits: std::collections::BTreeSet<String> = statement
        .query_map([], |row| row.get(0))
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<_>>()
        .map_err(map_sqlite_error)?;
    Ok(manifest["required_audits"]
        .as_array()
        .expect("audit list")
        .iter()
        .any(|audit| !audits.contains(audit[0].as_str().expect("audit id"))))
}

/// Copy the book into memory (a SQLCipher book through an attached plaintext
/// memory database, never a file) and run the ordered pass and the admission
/// gate there. Only a stored-value refusal is returned; any other outcome is
/// left to the real pass, which reaches it the documented way.
pub(crate) fn rehearse(
    conn: &Connection,
    encrypted: bool,
    clock: MigrationClock<'_>,
) -> crate::Result<()> {
    let mut copy = Connection::open_in_memory().map_err(map_sqlite_error)?;
    if encrypted {
        conn.execute_batch(
            "ATTACH DATABASE ':memory:' AS kassiber_rehearsal KEY '';
             SELECT sqlcipher_export('kassiber_rehearsal');",
        )
        .map_err(map_sqlite_error)?;
        let copied = Backup::new_with_names(conn, "kassiber_rehearsal", &mut copy, "main")
            .and_then(|backup| backup.run_to_completion(1024, std::time::Duration::ZERO, None));
        conn.execute_batch("DETACH DATABASE kassiber_rehearsal")
            .map_err(map_sqlite_error)?;
        copied.map_err(map_sqlite_error)?;
    } else {
        Backup::new(conn, &mut copy)
            .and_then(|backup| backup.run_to_completion(1024, std::time::Duration::ZERO, None))
            .map_err(map_sqlite_error)?;
    }
    if let Some(timestamp) = clock.pinned_timestamp() {
        let timestamp = timestamp.to_owned();
        copy.create_scalar_function(
            "strftime",
            2,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| Ok(timestamp.clone()),
        )
        .map_err(map_sqlite_error)?;
    }
    copy.execute_batch("PRAGMA foreign_keys = ON")
        .map_err(map_sqlite_error)?;
    // Identities from a counter: the rehearsal draws nothing from SQLite's
    // random generator, so the real pass generates the ids it always would.
    let mut counter = 0u64;
    let outcome = compat::ordered::migrate_with_clock(
        &mut copy,
        &mut |_| {
            counter += 1;
            Ok(format!("{counter:016x}4000800000000000"))
        },
        clock,
        &mut |_| Ok(()),
    )
    .and_then(|()| schema::verify(&copy));
    match outcome {
        Err(error) if is_refusal(&error) => Err(error),
        _ => Ok(()),
    }
}

/// The shape of the first value in a stored JSON document that a strict
/// reader cannot hold exactly, read with Python's own JSON rules.
pub(crate) fn json_shape(raw: &str) -> Shape {
    use kassiber_pyjson::{BigInt, Key, Value};
    fn key(key: &Key) -> Option<Shape> {
        match key {
            Key::Str(text) if text.as_str().is_none() => Some(Shape::LoneSurrogate),
            _ => None,
        }
    }
    fn walk(value: &Value) -> Option<Shape> {
        match value {
            Value::Float(number) if !number.is_finite() => Some(Shape::NonFinite),
            Value::Int(number)
                if number < &BigInt::from(i64::MIN) || number > &BigInt::from(u64::MAX) =>
            {
                Some(Shape::BeyondI64)
            }
            Value::Str(text) if text.as_str().is_none() => Some(Shape::LoneSurrogate),
            Value::List(items) => items.iter().find_map(walk),
            Value::Dict(items) => items
                .iter()
                .find_map(|(name, value)| key(name).or_else(|| walk(value))),
            _ => None,
        }
    }
    compat::python_json::loads(raw)
        .ok()
        .and_then(|value| walk(&value))
        .unwrap_or(Shape::Other)
}

#[cfg(test)]
mod tests {
    use crate::{encryption, Book, ErrorKind, OpenOptions};
    use std::path::{Path, PathBuf};

    const PASSPHRASE: &str = "Synthetic passphrase 1";

    /// A current book with one wallet, closed, then `sql`: the stored value
    /// and, for a legacy layout, a table the ordered pass recreates before it
    /// reaches the value.
    fn book(cipher: bool, sql: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let passphrase = cipher.then_some(PASSPHRASE);
        let book = Book::create(&data, passphrase).unwrap();
        let database = book.database().to_owned();
        book.close().map_err(|f| f.error).unwrap();
        let conn = encryption::open_container(&database, passphrase).unwrap();
        conn.execute_batch(
            "INSERT INTO workspaces VALUES('ws','Synthetic','fixed');
             INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed');
             INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal','ws','pf','Wallet','custom','fixed');",
        )
        .unwrap();
        conn.execute_batch(sql).unwrap();
        drop(conn);
        (root, data, database)
    }

    /// A table the base schema recreates first thing: without the rehearsal
    /// the pass would write it before it reaches the value.
    const LEGACY: &str = "DROP TABLE ai_provider_checks;";

    /// The legacy gap projection table whose input version `gap_inputs`
    /// converts to an integer.
    fn projection(version: &str) -> String {
        format!(
            "{LEGACY}CREATE TABLE custody_gap_candidate_projections(profile_id TEXT, producer_kind TEXT, version_json TEXT, ignored_ids_json TEXT, accounting_ignored_ids_json TEXT, created_at TEXT);
             INSERT INTO custody_gap_candidate_projections VALUES('pf','journal','{version}','[]','[]','fixed');"
        )
    }

    fn wallet_config(config: &str) -> String {
        format!("UPDATE wallets SET config_json='{config}';")
    }

    /// Every file beside the book, by name and bytes.
    fn files(database: &Path) -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<_> = std::fs::read_dir(database.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .map(|path| {
                (
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read(&path).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    }

    /// Open the book migrating; require the refusal with nothing written.
    fn refused(cipher: bool, sql: &str, value: &str, message: &str) {
        let (_root, data, database) = book(cipher, sql);
        let before = files(&database);
        let error = Book::open_migrating(
            &data,
            OpenOptions {
                passphrase: cipher.then_some(PASSPHRASE),
                ..Default::default()
            },
            true,
        )
        .err()
        .unwrap_or_else(|| panic!("opened: {sql}"));
        assert_eq!(error.kind, ErrorKind::UnsupportedSchemaLayout, "{sql}");
        assert_eq!(error.code, "unsupported_schema_layout");
        assert_eq!(error.message, message, "{sql}");
        assert_eq!(
            error.hint.as_deref(),
            Some("Kassiber does not open books that hold such values.")
        );
        assert_eq!(
            error.details,
            serde_json::json!({"reason": "unsupported_stored_value", "value": value}),
            "{sql}"
        );
        assert!(!format!("{error:?}").to_lowercase().contains("python"));
        assert!(files(&database) == before, "the book changed: {sql}");
    }

    #[test]
    fn nan_is_refused_before_anything_is_written() {
        let message = "this book stores NaN or infinity where Kassiber needs an exact number";
        // A migration input (the legacy gap projection's input version),
        // plaintext and SQLCipher.
        for cipher in [false, true] {
            refused(cipher, &projection("[NaN]"), "non_finite_number", message);
        }
        refused(
            false,
            &projection("[-Infinity]"),
            "non_finite_number",
            message,
        );
        // The admission gate, in a current and in a legacy layout.
        for layout in ["", LEGACY] {
            let sql =
                layout.to_owned() + &wallet_config(r#"{"ownership_history": null, "fee": NaN}"#);
            refused(false, &sql, "non_finite_number", message);
        }
    }

    #[test]
    fn integers_beyond_64_bits_are_refused_before_anything_is_written() {
        let message =
            "this book stores an integer beyond 64 bits that Kassiber cannot convert exactly";
        for cipher in [false, true] {
            refused(
                cipher,
                &projection("[123456789012345678901234567890]"),
                "integer_beyond_64_bits",
                message,
            );
        }
        refused(
            false,
            &projection("[9223372036854775808]"),
            "integer_beyond_64_bits",
            message,
        );
        for layout in ["", LEGACY] {
            let sql = layout.to_owned()
                + &wallet_config(
                    r#"{"ownership_history": null, "gap_limit": 18446744073709551616}"#,
                );
            refused(false, &sql, "integer_beyond_64_bits", message);
        }
    }

    #[test]
    fn lone_surrogates_are_refused_before_anything_is_written() {
        let message =
            "this book stores text with an unpaired surrogate that Kassiber cannot convert exactly";
        // A migration input: an inline ownership policy's text field.
        for cipher in [false, true] {
            let sql = LEGACY.to_owned()
                + &wallet_config(
                    r#"{"ownership_history": [{"descriptor": "wpkh(x)", "chain": "\ud800"}]}"#,
                );
            refused(cipher, &sql, "lone_surrogate", message);
        }
        for layout in ["", LEGACY] {
            let sql = layout.to_owned()
                + &wallet_config(r#"{"ownership_history": null, "label": "a\udc00b"}"#);
            refused(false, &sql, "lone_surrogate", message);
        }
    }

    /// The rehearsal leaves a book it does not refuse to the real pass, which
    /// migrates it as before; a current book is not copied at all.
    #[test]
    fn books_without_such_values_migrate_as_before() {
        let (_root, data, database) = book(false, &projection("[3]"));
        let conn = encryption::open_container(&database, None).unwrap();
        assert!(super::layout_pending(&conn).unwrap());
        drop(conn);
        let book = Book::open_migrating(&data, OpenOptions::default(), true).unwrap();
        let version: i64 = book
            .connection()
            .query_row(
                "SELECT input_version FROM journal_custody_gap_inputs",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, 3);
        assert!(!super::layout_pending(book.connection()).unwrap());
    }
}

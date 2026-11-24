//! Exchange sync outcomes keep only an error class (Rust only;
//! docs/reference/desktop.md#sources-live-sync-and-file-snapshots).
//!
//! Builds before classes stored the envelope error of a failed exchange API
//! sync in `wallets.config_json.exchange_sync.error`, including the
//! provider's message and hint, which can carry response text. Each open
//! rewrites any such record to its class, with `secure_delete` on so the old
//! text does not survive in freed pages, then checkpoints the WAL into the
//! database file and truncates it. Only that truncate makes the cleanup
//! done; until then a settings marker keeps it pending and each migrating
//! open retries it. A book without legacy records or a pending marker is
//! only read.
use crate::{map_sqlite_error, Result};
use kassiber_pyjson::{dumps, Key, Options, Value};
use rusqlite::{Connection, TransactionBehavior};

/// The error classes a stored exchange sync may carry.
pub const EXCHANGE_ERROR_CLASSES: [&str; 8] = [
    "auth",
    "rate_limited",
    "offline",
    "network",
    "protocol",
    "validation",
    "config",
    "other",
];

/// The class of an envelope error code.
pub fn exchange_error_class(code: &str) -> &'static str {
    match code {
        "auth_error" | "auth_required" | "unauthorized" | "permission_denied" => "auth",
        "rate_limited" => "rate_limited",
        "network_egress_disabled" | "offline" => "offline",
        "backend_timeout" | "network_error" | "backend_unreachable" | "timeout" => "network",
        "protocol_error" => "protocol",
        "validation" => "validation",
        "config_error" => "config",
        _ => "other",
    }
}

fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Dict(pairs) => pairs
            .iter()
            .find(|(k, _)| *k == Key::from(key))
            .map(|(_, v)| v),
        _ => None,
    }
}

fn text(value: Option<&Value>) -> Option<&str> {
    match value {
        Some(Value::Str(text)) => text.as_str(),
        _ => None,
    }
}

/// The record's error reduced to its class, or `None` when it already is.
fn classed(record: &Value) -> Option<Value> {
    let error = get(record, "error")?;
    let only_class = matches!(error, Value::Dict(pairs)
        if pairs.len() == 1
            && text(get(error, "class")).is_some_and(|c| EXCHANGE_ERROR_CLASSES.contains(&c)));
    if only_class {
        return None;
    }
    let class = text(get(error, "class"))
        .and_then(|class| {
            EXCHANGE_ERROR_CLASSES
                .iter()
                .find(|c| **c == class)
                .copied()
        })
        .unwrap_or_else(|| text(get(error, "code")).map_or("other", exchange_error_class));
    let Value::Dict(pairs) = record else {
        return None;
    };
    let mut pairs = pairs.clone();
    for (key, value) in pairs.iter_mut() {
        if *key == Key::from("error") {
            *value = Value::Dict(vec![(Key::from("class"), Value::from(class))]);
        }
    }
    Some(Value::Dict(pairs))
}

/// The wallet config with its exchange sync error reduced to a class, when
/// it held more.
fn rewritten(raw: &str) -> Option<String> {
    let Ok(Value::Dict(mut config)) = kassiber_pytext::loads(raw) else {
        return None;
    };
    let mut changed = false;
    for (key, value) in config.iter_mut() {
        if *key == Key::from("exchange_sync") {
            if let Some(record) = classed(value) {
                *value = record;
                changed = true;
            }
        }
    }
    if !changed {
        return None;
    }
    dumps(&Value::Dict(config), &Options::new().sort_keys(true)).ok()
}

/// The settings key that marks a rewrite whose old text may still be in the
/// database or its WAL.
const CLEANUP_KEY: &str = "exchange_sync_errors_cleanup";

/// Whether a rewrite's cleanup has not finished yet: its old text may
/// still be in the book's files. The next migrating open retries it.
pub fn cleanup_pending(conn: &Connection) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM settings WHERE key = ?1)",
        [CLEANUP_KEY],
        |r| r.get(0),
    )
    .map_err(map_sqlite_error)
}

/// Every wallet whose stored exchange sync still holds more than its error
/// class, with its rewritten config.
fn pending(conn: &Connection) -> Result<Vec<(String, String)>> {
    let mut statement = conn
        .prepare(
            "SELECT id, config_json FROM wallets \
             WHERE instr(config_json, '\"exchange_sync\"') > 0 ORDER BY id",
        )
        .map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<(String, String)>>>()
        .map_err(map_sqlite_error)?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, raw)| rewritten(&raw).map(|updated| (id, updated)))
        .collect())
}

/// Whether a migrating open would write here: a record to rewrite or a
/// cleanup to finish. The open's preflight rehearses such a book first, so a
/// stored-value refusal still leaves it untouched.
pub(crate) fn work_pending(conn: &Connection) -> Result<bool> {
    Ok(!pending(conn)?.is_empty() || cleanup_pending(conn)?)
}

pub(crate) fn migrate(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    if pending(conn)?.is_empty() {
        // A rewrite an earlier open could not finish cleaning up.
        return if cleanup_pending(conn)? {
            finish(conn, observer)
        } else {
            Ok(())
        };
    }
    let secure: i64 = conn
        .query_row("PRAGMA secure_delete", [], |r| r.get(0))
        .map_err(map_sqlite_error)?;
    conn.execute_batch("PRAGMA secure_delete = ON")
        .map_err(map_sqlite_error)?;
    let result = (|| {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        observer("exchange_sync_errors.before")?;
        for (id, updated) in pending(&tx)? {
            tx.execute(
                "UPDATE wallets SET config_json = ?1 WHERE id = ?2",
                rusqlite::params![updated, id],
            )
            .map_err(map_sqlite_error)?;
        }
        // Cleared only once no file of the book holds the old text.
        tx.execute(
            "INSERT INTO settings(key, value) VALUES(?1, 'pending') \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [CLEANUP_KEY],
        )
        .map_err(map_sqlite_error)?;
        observer("exchange_sync_errors.before_commit")?;
        tx.commit().map_err(map_sqlite_error)
    })();
    conn.execute_batch(&format!("PRAGMA secure_delete = {secure}"))
        .map_err(map_sqlite_error)?;
    result?;
    observer("exchange_sync_errors.after_commit")?;
    finish(conn, observer)
}

/// Moves every committed page into the database file and empties the WAL,
/// so neither keeps a page image with the old text, then clears the
/// marker. A reader holding an older snapshot keeps those pages alive: after
/// a few short tries the cleanup stays pending (the book stays usable) and
/// the next migrating open tries again. A DELETE-mode book wrote the pages
/// in place at commit and has no WAL to empty.
fn finish(conn: &mut Connection, observer: &mut impl FnMut(&str) -> Result<()>) -> Result<()> {
    if !truncate_wal(conn)? {
        return observer("exchange_sync_errors.cleanup_pending");
    }
    conn.execute("DELETE FROM settings WHERE key = ?1", [CLEANUP_KEY])
        .map_err(map_sqlite_error)?;
    observer("exchange_sync_errors.removed")
}

/// `PRAGMA wal_checkpoint(TRUNCATE)`, a few times with a short busy wait;
/// whether it completed.
fn truncate_wal(conn: &Connection) -> Result<bool> {
    conn.busy_timeout(std::time::Duration::from_millis(250))
        .map_err(map_sqlite_error)?;
    let mut done = Ok(false);
    for _ in 0..3 {
        match conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            r.get::<_, i64>(0)
        }) {
            Ok(0) => {
                done = Ok(true);
                break;
            }
            Ok(_) => {}
            Err(error) if crate::sqlite_is_busy(&error) => {}
            Err(error) => {
                done = Err(map_sqlite_error(error));
                break;
            }
        }
    }
    conn.busy_timeout(crate::BUSY_TIMEOUT)
        .map_err(map_sqlite_error)?;
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_keep_only_their_class() {
        let raw = r#"{"backend": "kraken", "exchange_sync": {"at": "t", "error": {"code": "auth_error", "hint": "provider says rotate ABC", "message": "HTTP 401 EAPI:Invalid key ABC"}, "status": "error"}, "last_synced_at": "s"}"#;
        let updated = rewritten(raw).unwrap();
        assert_eq!(
            updated,
            r#"{"backend": "kraken", "exchange_sync": {"at": "t", "error": {"class": "auth"}, "status": "error"}, "last_synced_at": "s"}"#
        );
        // Already classed, or nothing to class: left alone.
        assert_eq!(rewritten(&updated), None);
        assert_eq!(
            rewritten(r#"{"exchange_sync": {"at": "t", "status": "ok"}}"#),
            None
        );
        assert_eq!(rewritten("not json"), None);
        // An unknown stored class falls back to the code's.
        assert!(rewritten(
            r#"{"exchange_sync": {"error": {"class": "nope", "code": "rate_limited"}}}"#
        )
        .unwrap()
        .contains(r#""class": "rate_limited""#));
    }

    const SECRET: &[u8] = b"SECRET-PROVIDER-TEXT";

    /// A seeded pre-class book, in WAL (as books Python created are) or in
    /// DELETE mode (as fresh Rust books are).
    fn seeded(root: &std::path::Path, wal: bool) {
        let book = crate::Book::create(root, None).unwrap();
        book.connection
            .execute_batch(
                "INSERT INTO workspaces VALUES('ws','W','fixed');
                 INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','P','fixed');
                 INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('w','ws','p','Kraken','kraken',
                   '{\"backend\": \"kraken\", \"exchange_sync\": {\"at\": \"t\", \"error\": {\"code\": \"rate_limited\", \"hint\": \"x\", \"message\": \"429 body: SECRET-PROVIDER-TEXT\"}, \"status\": \"error\"}}','fixed');",
            )
            .unwrap();
        drop(book);
        if wal {
            let conn = Connection::open(root.join("kassiber.sqlite3")).unwrap();
            let mode: String = conn
                .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
        }
    }

    /// The files of the book that still hold the provider text.
    fn holding(root: &std::path::Path) -> Vec<String> {
        ["", "-wal", "-journal"]
            .into_iter()
            .filter(|suffix| {
                let path = root.join(format!("kassiber.sqlite3{suffix}"));
                std::fs::read(path)
                    .unwrap_or_default()
                    .windows(SECRET.len())
                    .any(|w| w == SECRET)
            })
            .map(|suffix| format!("kassiber.sqlite3{suffix}"))
            .collect()
    }

    fn stored(book: &crate::Book) -> String {
        book.connection
            .query_row("SELECT config_json FROM wallets WHERE id='w'", [], |r| {
                r.get(0)
            })
            .unwrap()
    }

    /// A book a pre-class build wrote: once the migrating open returns, with
    /// the book still open, no file of it holds the provider text.
    #[test]
    fn a_migrating_open_removes_stored_provider_text_from_the_book() {
        for wal in [false, true] {
            let root = tempfile::tempdir().unwrap();
            seeded(root.path(), wal);
            assert_eq!(holding(root.path()), ["kassiber.sqlite3"], "wal={wal}");
            let book =
                crate::Book::open_migrating(root.path(), crate::OpenOptions::default(), true)
                    .unwrap();
            assert!(
                stored(&book).contains(r#""error": {"class": "rate_limited"}"#),
                "{}",
                stored(&book)
            );
            assert!(!cleanup_pending(&book.connection).unwrap());
            assert_eq!(holding(root.path()), Vec::<String>::new(), "wal={wal}");
        }
    }

    /// Third review, finding 2: a reader holding a snapshot keeps the old
    /// pages alive, so the cleanup is pending, not claimed. Once the reader
    /// is gone, the next open finishes it.
    #[test]
    fn a_busy_reader_leaves_the_cleanup_pending_until_the_next_open() {
        let root = tempfile::tempdir().unwrap();
        seeded(root.path(), true);
        let reader = Connection::open(root.path().join("kassiber.sqlite3")).unwrap();
        reader
            .execute_batch("BEGIN; SELECT COUNT(*) FROM wallets;")
            .unwrap();
        let book =
            crate::Book::open_migrating(root.path(), crate::OpenOptions::default(), true).unwrap();
        // Rewritten for every new read, and the book stays usable.
        assert!(stored(&book).contains(r#""error": {"class": "rate_limited"}"#));
        assert!(cleanup_pending(&book.connection).unwrap());
        assert!(
            holding(root.path()).contains(&"kassiber.sqlite3".to_owned()),
            "the reader's snapshot still needs the old page"
        );
        reader.execute_batch("COMMIT").unwrap();
        drop(reader);
        // The first connection stays open: nothing relies on a last close.
        let again =
            crate::Book::open_migrating(root.path(), crate::OpenOptions::default(), true).unwrap();
        assert!(!cleanup_pending(&again.connection).unwrap());
        assert_eq!(holding(root.path()), Vec::<String>::new());
        drop(book);
        drop(again);
        assert_eq!(holding(root.path()), Vec::<String>::new());
    }
}

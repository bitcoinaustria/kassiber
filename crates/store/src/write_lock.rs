//! Write transactions hold SQLite's writer lock before their first read.
//!
//! A transaction begun deferred (a bare `SAVEPOINT` outside a transaction,
//! `BEGIN`, rusqlite's `transaction()`/`savepoint()`/`unchecked_transaction()`)
//! takes a shared lock with its reads. When another connection holds the
//! writer lock (a daemon worker, another Kassiber process), the first write's
//! upgrade fails with SQLITE_BUSY at once: SQLite skips the busy handler
//! because waiting could deadlock, so `busy_timeout` never applies and the
//! request ends as `database_busy`. Asking for the writer lock with the
//! transaction's first statement waits like any write. This mirrors Python's
//! `kassiber.db.begin_write_savepoint` and `BEGIN IMMEDIATE`; the inventory
//! is docs/reference/rust-write-transactions.md and
//! `crates/store/tests/write_transactions.rs` fails on a new deferred opener.

use rusqlite::{
    CachedStatement, Connection, Savepoint, Transaction, TransactionBehavior, TransactionState,
};

/// Changes nothing; requesting the writer lock is its only effect. No row
/// matches, so no trigger fires, no page is dirtied and commit writes nothing.
const RESERVE: &str = "UPDATE settings SET value = value WHERE 0";

/// The reservation statement, prepared while no transaction of ours is open,
/// or `None` when this connection needs none: it already holds the writer,
/// cannot write at all (read-only or `query_only`, so no upgrade can follow),
/// or has no `settings` table (partial test schemas; every Kassiber book has
/// one, as Python checks).
fn reservation(conn: &Connection) -> rusqlite::Result<Option<CachedStatement<'_>>> {
    if conn.transaction_state(Some("main"))? == TransactionState::Write
        || conn.is_readonly("main")?
        || conn.query_row("PRAGMA query_only", [], |row| row.get::<_, bool>(0))?
    {
        return Ok(None);
    }
    match conn.prepare_cached(RESERVE) {
        Ok(statement) => Ok(Some(statement)),
        Err(rusqlite::Error::SqliteFailure(_, Some(message)))
            if message.starts_with("no such table") =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Open `SAVEPOINT name` for a write, holding the writer lock from the start.
/// RELEASE of an outermost savepoint still commits; nested in a caller's
/// transaction it behaves as a plain savepoint. A refused reservation leaves
/// no savepoint open. Read-only snapshots keep a plain `SAVEPOINT`.
pub fn begin_write_savepoint(conn: &Connection, name: &str) -> rusqlite::Result<()> {
    let reservation = reservation(conn)?;
    conn.execute_batch(&format!("SAVEPOINT {name}"))?;
    if let Some(mut statement) = reservation {
        if let Err(error) = statement.execute([]) {
            let _ = conn.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT {name}; RELEASE SAVEPOINT {name}"
            ));
            return Err(error);
        }
    }
    Ok(())
}

/// [`begin_write_savepoint`] as a rusqlite [`Savepoint`]: committed by
/// `commit()`, rolled back when dropped.
pub fn write_savepoint(conn: &mut Connection) -> rusqlite::Result<Savepoint<'_>> {
    let reserve = reservation(conn)?.is_some();
    let savepoint = conn.savepoint()?;
    if reserve {
        // Dropping the savepoint on error rolls it back and releases it.
        savepoint.execute(RESERVE, [])?;
    }
    Ok(savepoint)
}

/// An unchecked transaction for a write: `BEGIN IMMEDIATE`, or `BEGIN
/// DEFERRED` on a connection that cannot write (its writes fail anyway, and
/// `BEGIN IMMEDIATE` would be refused there before any read).
pub fn write_transaction(conn: &Connection) -> rusqlite::Result<Transaction<'_>> {
    let behavior = if conn.is_readonly("main")?
        || conn.query_row("PRAGMA query_only", [], |row| row.get::<_, bool>(0))?
    {
        TransactionBehavior::Deferred
    } else {
        TransactionBehavior::Immediate
    };
    Transaction::new_unchecked(conn, behavior)
}

/// A read-only snapshot that began the transaction (`outermost`) must not
/// have written: a write there would upgrade a shared lock and fail at once
/// against another writer. Debug and test builds check it where the snapshot
/// ends; release builds do nothing.
#[inline]
pub fn debug_assert_no_write(conn: &Connection, outermost: bool, snapshot: &str) {
    debug_assert!(
        !outermost
            || !conn
                .transaction_state(Some("main"))
                .is_ok_and(|state| state == TransactionState::Write),
        "read-only snapshot {snapshot} wrote; open it with begin_write_savepoint"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Book, OpenOptions};
    use std::path::Path;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    const HOLD: Duration = Duration::from_millis(700);

    fn book(root: &Path, passphrase: Option<&str>) -> Book {
        drop(Book::create(root, passphrase).unwrap());
        open(root, passphrase)
    }

    fn open(root: &Path, passphrase: Option<&str>) -> Book {
        Book::open(
            root,
            OpenOptions {
                passphrase,
                ..Default::default()
            },
        )
        .unwrap()
    }

    /// Another connection takes the writer and keeps it for `HOLD`.
    fn hold_writer(root: &Path, passphrase: Option<&str>) -> thread::JoinHandle<()> {
        let (root, passphrase) = (root.to_owned(), passphrase.map(str::to_owned));
        let (held, wait) = mpsc::channel();
        let holder = thread::spawn(move || {
            let other = open(&root, passphrase.as_deref());
            other.connection().execute_batch("BEGIN IMMEDIATE").unwrap();
            held.send(()).unwrap();
            thread::sleep(HOLD);
            other.connection().execute_batch("COMMIT").unwrap();
        });
        wait.recv().unwrap();
        holder
    }

    fn probe(conn: &Connection) -> Option<String> {
        conn.query_row("SELECT value FROM settings WHERE key='probe'", [], |r| {
            r.get(0)
        })
        .ok()
    }

    fn busy(error: &rusqlite::Error) -> bool {
        matches!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        )
    }

    #[test]
    fn write_openers_wait_for_a_held_writer_and_deferred_ones_fail_at_once() {
        let scratch = tempfile::tempdir().unwrap();
        for passphrase in [None, Some("synthetic key")] {
            let root = scratch.path().join(if passphrase.is_some() {
                "cipher"
            } else {
                "plain"
            });
            let mut book = book(&root, passphrase);
            let conn = book.connection();

            // The race: a deferred savepoint that reads first is refused at once.
            let holder = hold_writer(&root, passphrase);
            conn.execute_batch("SAVEPOINT bare").unwrap();
            conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get::<_, i64>(0))
                .unwrap();
            let started = Instant::now();
            let error = conn
                .execute("INSERT INTO settings(key,value) VALUES('probe','bare')", [])
                .unwrap_err();
            assert!(busy(&error), "{error}");
            assert!(started.elapsed() < HOLD / 2);
            conn.execute_batch("ROLLBACK TO bare; RELEASE bare")
                .unwrap();
            holder.join().unwrap();

            let holder = hold_writer(&root, passphrase);
            let started = Instant::now();
            begin_write_savepoint(conn, "reserved").unwrap();
            assert!(started.elapsed() >= HOLD / 2);
            conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get::<_, i64>(0))
                .unwrap();
            conn.execute(
                "INSERT INTO settings(key,value) VALUES('probe','savepoint')",
                [],
            )
            .unwrap();
            conn.execute_batch("RELEASE reserved").unwrap();
            assert!(conn.is_autocommit());
            holder.join().unwrap();
            assert_eq!(
                probe(open(&root, passphrase).connection()).as_deref(),
                Some("savepoint")
            );

            let holder = hold_writer(&root, passphrase);
            let started = Instant::now();
            let tx = write_transaction(conn).unwrap();
            assert!(started.elapsed() >= HOLD / 2);
            tx.execute(
                "UPDATE settings SET value='transaction' WHERE key='probe'",
                [],
            )
            .unwrap();
            tx.commit().unwrap();
            holder.join().unwrap();
            assert_eq!(
                probe(open(&root, passphrase).connection()).as_deref(),
                Some("transaction")
            );

            let holder = hold_writer(&root, passphrase);
            let started = Instant::now();
            let savepoint = write_savepoint(book.connection_mut()).unwrap();
            assert!(started.elapsed() >= HOLD / 2);
            savepoint
                .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get::<_, i64>(0))
                .unwrap();
            savepoint
                .execute("UPDATE settings SET value='rusqlite' WHERE key='probe'", [])
                .unwrap();
            savepoint.commit().unwrap();
            holder.join().unwrap();
            assert_eq!(
                probe(open(&root, passphrase).connection()).as_deref(),
                Some("rusqlite")
            );
        }
    }

    #[test]
    fn a_refused_reservation_leaves_no_savepoint_open() {
        let scratch = tempfile::tempdir().unwrap();
        let mut book = book(scratch.path(), None);
        book.connection().busy_timeout(Duration::ZERO).unwrap();
        let holder = hold_writer(scratch.path(), None);
        let error = begin_write_savepoint(book.connection(), "refused").unwrap_err();
        assert!(busy(&error), "{error}");
        assert!(book.connection().is_autocommit());
        assert!(busy(
            &write_savepoint(book.connection_mut())
                .map(drop)
                .unwrap_err()
        ));
        assert!(book.connection().is_autocommit());
        holder.join().unwrap();
    }

    #[test]
    fn the_reservation_writes_nothing_and_nests_as_a_plain_savepoint() {
        let scratch = tempfile::tempdir().unwrap();
        let book = book(scratch.path(), None);
        let conn = book.connection();
        let database = book.database().to_owned();
        let before = std::fs::read(&database).unwrap();
        let changes = conn.total_changes();
        begin_write_savepoint(conn, "outer").unwrap();
        assert_eq!(
            conn.transaction_state(Some("main")).unwrap(),
            TransactionState::Write
        );
        begin_write_savepoint(conn, "inner").unwrap();
        conn.execute_batch("RELEASE inner; RELEASE outer").unwrap();
        assert!(conn.is_autocommit());
        assert_eq!(conn.total_changes(), changes);
        assert_eq!(std::fs::read(&database).unwrap(), before);

        // Inside a caller's transaction a refused inner write rolls back only itself.
        conn.execute_batch("BEGIN IMMEDIATE; INSERT INTO settings(key,value) VALUES('kept','1')")
            .unwrap();
        begin_write_savepoint(conn, "nested").unwrap();
        conn.execute_batch("ROLLBACK TO nested; RELEASE nested; COMMIT")
            .unwrap();
        assert_eq!(
            conn.query_row("SELECT value FROM settings WHERE key='kept'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "1"
        );
    }

    #[test]
    fn connections_that_cannot_write_and_partial_schemas_take_no_reservation() {
        let scratch = tempfile::tempdir().unwrap();
        let book = book(scratch.path(), None);
        let conn = book.connection();
        conn.execute_batch("PRAGMA query_only = ON").unwrap();
        let holder = hold_writer(scratch.path(), None);
        let started = Instant::now();
        begin_write_savepoint(conn, "pinned").unwrap();
        conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get::<_, i64>(0))
            .unwrap();
        conn.execute_batch("RELEASE pinned").unwrap();
        let tx = write_transaction(conn).unwrap();
        tx.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get::<_, i64>(0))
            .unwrap();
        drop(tx);
        assert!(
            started.elapsed() < HOLD / 2,
            "a read-only connection must not wait"
        );
        holder.join().unwrap();
        conn.execute_batch("PRAGMA query_only = OFF").unwrap();

        let partial = Connection::open_in_memory().unwrap();
        partial.execute_batch("CREATE TABLE other(x)").unwrap();
        begin_write_savepoint(&partial, "partial").unwrap();
        assert_eq!(
            partial.transaction_state(Some("main")).unwrap(),
            TransactionState::None
        );
        partial
            .execute_batch("INSERT INTO other VALUES(1); RELEASE partial")
            .unwrap();
    }
}

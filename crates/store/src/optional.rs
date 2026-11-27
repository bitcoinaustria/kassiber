//! Opt-in schema: objects a book gets only when the user first uses the
//! feature that needs them (`fixtures/optional-schema.json`, one key per
//! group). A book that never used a feature carries none of its objects and
//! keeps exactly the layout the manifest describes; once created, a group's
//! objects are part of the expected schema and must match their definitions
//! exactly, so a partial or drifted set fails the structural gate.
//!
//! Older Rust builds refuse a book that has a group they do not know (their
//! admission refuses unknown objects). The released Python app (v0.22.78)
//! has no such gate: it opens the book and ignores the group, as it ignores
//! every correction since its release; see
//! `docs/reference/database-compatibility.md#opt-in-schema-groups`.
//!
//! Groups: `bank_statements` (imported bank statements,
//! `docs/reference/bank-statements.md`; their tables hold personal financial
//! data, so a book that never imported a statement carries none of them) and
//! `provider_trade_holds` (provider trades an importer holds for review,
//! [Bisq](../../../docs/exchanges/bisq.md)).
use crate::{map_sqlite_error, Result};
use rusqlite::Connection;
use serde_json::Value;
use std::sync::OnceLock;

const FIXTURE: &str = include_str!("../fixtures/optional-schema.json");

/// Imported bank statements (`bank_statements`).
pub const BANK_STATEMENTS: &str = "bank_statements";
/// Provider trades held for review (`provider_trade_holds`).
pub const PROVIDER_TRADE_HOLDS: &str = "provider_trade_holds";
/// Imported Bark wallet operations and source labels.
pub const BTCPAY_BARK_ACTIVITY: &str = "btcpay_bark_activity";
/// Every group and the table whose presence marks it started.
const GROUPS: &[(&str, &str)] = &[
    (BANK_STATEMENTS, "bank_statement_lines"),
    (PROVIDER_TRADE_HOLDS, "provider_trade_holds"),
    (BTCPAY_BARK_ACTIVITY, "btcpay_bark_activity"),
];

fn fixture() -> &'static Value {
    static FIXTURE_VALUE: OnceLock<Value> = OnceLock::new();
    FIXTURE_VALUE.get_or_init(|| serde_json::from_str(FIXTURE).expect("optional schema fixture"))
}

/// A group's objects as `sqlite_master` stores them: `[type, name, tbl_name,
/// sql]`, sorted by type and name.
pub(crate) fn objects(group: &str) -> &'static [Value] {
    fixture()[group].as_array().expect("optional schema group")
}

fn presence_table(group: &str) -> &'static str {
    GROUPS
        .iter()
        .find(|(name, _)| *name == group)
        .map(|(_, table)| *table)
        .expect("known optional schema group")
}

/// Whether this book has started using `group`.
pub fn present(conn: &Connection, group: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
        [presence_table(group)],
        |r| r.get(0),
    )
    .map_err(map_sqlite_error)
}

/// Create `group`'s objects if absent. Call inside the caller's write
/// transaction, so a failed operation leaves no objects behind.
pub fn ensure(conn: &Connection, group: &str) -> Result<()> {
    for kind in ["table", "index"] {
        for object in objects(group).iter().filter(|o| o[0] == kind) {
            let sql = object[3].as_str().expect("sql");
            let create = match kind {
                "table" => sql.replacen("CREATE TABLE ", "CREATE TABLE IF NOT EXISTS ", 1),
                _ => sql.replacen("CREATE INDEX ", "CREATE INDEX IF NOT EXISTS ", 1),
            };
            conn.execute_batch(&create).map_err(map_sqlite_error)?;
        }
    }
    Ok(())
}

/// The bank-statement table names.
pub fn bank_statement_tables() -> Vec<&'static str> {
    objects(BANK_STATEMENTS)
        .iter()
        .filter(|o| o[0] == "table")
        .map(|o| o[1].as_str().expect("name"))
        .collect()
}

/// Whether this book has started storing bank statements.
pub fn bank_statements_present(conn: &Connection) -> Result<bool> {
    present(conn, BANK_STATEMENTS)
}

/// Create the bank-statement objects if absent. Call inside the caller's
/// write transaction, so a failed import leaves no tables behind.
pub fn ensure_bank_statement_schema(conn: &Connection) -> Result<()> {
    ensure(conn, BANK_STATEMENTS)
}

/// Names of every optional object, whether or not this book has them.
pub(crate) fn optional_names() -> impl Iterator<Item = (&'static str, &'static str)> {
    GROUPS.iter().flat_map(|(group, _)| {
        objects(group)
            .iter()
            .map(|o| (o[0].as_str().expect("type"), o[1].as_str().expect("name")))
    })
}

/// The manifest's objects plus the objects of every group this book has
/// started using (any of the group's objects present), in `sqlite_master`
/// order (type, then name).
pub(crate) fn expected_objects(conn: &Connection, manifest: &Value) -> Result<Value> {
    let mut objects_list = manifest["objects"].as_array().expect("objects").clone();
    let mut extended = false;
    for (group, _) in GROUPS {
        let names = objects(group)
            .iter()
            .map(|o| format!("'{}'", o[1].as_str().expect("name")))
            .collect::<Vec<_>>()
            .join(",");
        let started: bool = conn
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name IN ({names}))"),
                [],
                |r| r.get(0),
            )
            .map_err(map_sqlite_error)?;
        if started {
            objects_list.extend(objects(group).iter().cloned());
            extended = true;
        }
    }
    if extended {
        objects_list
            .sort_by(|a, b| (a[0].as_str(), a[1].as_str()).cmp(&(b[0].as_str(), b[1].as_str())));
    }
    let mut expected = manifest.clone();
    expected["objects"] = Value::Array(objects_list);
    Ok(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> (tempfile::TempDir, crate::Book) {
        let root = tempfile::tempdir().unwrap();
        let book = crate::Book::create(root.path(), None).unwrap();
        (root, book)
    }

    #[test]
    fn fresh_books_have_no_bank_tables_and_creation_is_idempotent_and_verified() {
        let (_root, book) = book();
        let conn = book.connection();
        assert!(!bank_statements_present(conn).unwrap());
        crate::schema::verify(conn).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        ensure_bank_statement_schema(conn).unwrap();
        ensure_bank_statement_schema(conn).unwrap();
        conn.execute_batch("COMMIT").unwrap();
        assert!(bank_statements_present(conn).unwrap());
        assert_eq!(
            bank_statement_tables(),
            [
                "bank_statement_imports",
                "bank_statement_line_decisions",
                "bank_statement_lines"
            ]
        );
        crate::schema::verify(conn).unwrap();
        // The stored definitions are exactly the fixture's.
        for object in objects(BANK_STATEMENTS) {
            let sql: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type=? AND name=?",
                    [object[0].as_str().unwrap(), object[1].as_str().unwrap()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(sql, object[3].as_str().unwrap());
        }
    }

    #[test]
    fn a_partial_or_drifted_bank_set_fails_closed() {
        let (_root, book) = book();
        let conn = book.connection();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        ensure_bank_statement_schema(conn).unwrap();
        conn.execute_batch("COMMIT").unwrap();
        conn.execute_batch("DROP INDEX idx_bank_statement_lines_profile_date")
            .unwrap();
        assert!(crate::schema::verify(conn).is_err());
        conn.execute_batch("CREATE INDEX idx_bank_statement_lines_profile_date ON bank_statement_lines(profile_id)")
            .unwrap();
        assert!(crate::schema::verify(conn).is_err());
        conn.execute_batch("DROP INDEX idx_bank_statement_lines_profile_date")
            .unwrap();
        ensure_bank_statement_schema(conn).unwrap();
        crate::schema::verify(conn).unwrap();
        conn.execute_batch("ALTER TABLE bank_statement_lines ADD COLUMN extra TEXT")
            .unwrap();
        assert!(crate::schema::verify(conn).is_err());
    }

    /// Each group is expected on its own: a book that uses both verifies,
    /// and one group's objects never make the other's expected.
    #[test]
    fn groups_are_expected_independently() {
        let (_root, book) = book();
        let conn = book.connection();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        ensure(conn, PROVIDER_TRADE_HOLDS).unwrap();
        conn.execute_batch("COMMIT").unwrap();
        assert!(!bank_statements_present(conn).unwrap());
        crate::schema::verify(conn).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        ensure_bank_statement_schema(conn).unwrap();
        conn.execute_batch("COMMIT").unwrap();
        crate::schema::verify(conn).unwrap();
        conn.execute_batch("DROP INDEX idx_provider_trade_holds_profile")
            .unwrap();
        assert!(crate::schema::verify(conn).is_err());
    }

    #[test]
    fn fresh_books_have_no_holds_table_and_creation_is_idempotent_and_verified() {
        let (_root, book) = book();
        let conn = book.connection();
        assert!(!present(conn, PROVIDER_TRADE_HOLDS).unwrap());
        crate::schema::verify(conn).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        ensure(conn, PROVIDER_TRADE_HOLDS).unwrap();
        ensure(conn, PROVIDER_TRADE_HOLDS).unwrap();
        conn.execute_batch("COMMIT").unwrap();
        assert!(present(conn, PROVIDER_TRADE_HOLDS).unwrap());
        crate::schema::verify(conn).unwrap();
        for object in objects(PROVIDER_TRADE_HOLDS) {
            let sql: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type=? AND name=?",
                    [object[0].as_str().unwrap(), object[1].as_str().unwrap()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(sql, object[3].as_str().unwrap());
        }
    }

    #[test]
    fn a_partial_or_drifted_holds_set_fails_closed() {
        let (_root, book) = book();
        let conn = book.connection();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        ensure(conn, PROVIDER_TRADE_HOLDS).unwrap();
        conn.execute_batch("COMMIT").unwrap();
        conn.execute_batch("DROP INDEX idx_provider_trade_holds_profile")
            .unwrap();
        assert!(crate::schema::verify(conn).is_err());
    }

    /// A Rust build that predates the group verifies against the manifest
    /// alone: it refuses a book that holds trades. (The released Python app
    /// does not verify objects; see the stamp test below.)
    #[test]
    fn a_rust_build_without_the_group_refuses_a_book_that_uses_it() {
        let (_root, book) = book();
        let conn = book.connection();
        let manifest: Value = serde_json::from_str(crate::schema::MANIFEST).unwrap();
        crate::schema_structure::verify(conn, &manifest).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        ensure(conn, PROVIDER_TRADE_HOLDS).unwrap();
        conn.execute_batch("COMMIT").unwrap();
        assert!(crate::schema_structure::verify(conn, &manifest).is_err());
    }

    /// Older Rust builds stay out of books with groups they do not know:
    /// admission refuses a table this build does not know, as a Rust build
    /// that predates a group does with that group's table. A refactor that
    /// makes admission tolerate unknown tables would let an older Rust build
    /// write to a book with holds it cannot see; this test fails first.
    #[test]
    fn opening_refuses_a_table_this_build_does_not_know() {
        let root = tempfile::tempdir().unwrap();
        {
            let book = crate::Book::create(root.path(), None).unwrap();
            let conn = book.connection();
            conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            ensure(conn, PROVIDER_TRADE_HOLDS).unwrap();
            conn.execute_batch("COMMIT").unwrap();
        }
        // This build knows the holds group, so the book opens.
        drop(crate::Book::open(root.path(), Default::default()).unwrap());
        {
            let book = crate::Book::open(root.path(), Default::default()).unwrap();
            book.connection()
                .execute_batch(
                    "CREATE TABLE future_trade_holds (id TEXT PRIMARY KEY, profile_id TEXT NOT NULL);
                     CREATE INDEX idx_future_trade_holds_profile ON future_trade_holds(profile_id);",
                )
                .unwrap();
        }
        let refused = crate::Book::open(root.path(), Default::default())
            .err()
            .expect("an unknown table refuses the book");
        assert_eq!(
            refused.kind,
            crate::ErrorKind::RequiresPythonMigration,
            "{refused:?}"
        );
    }

    /// The released Python app (v0.22.78) opens a book with holds and
    /// ignores them, but cannot leave its journals current: publishing them
    /// updates the processing markers, and `trg_journal_semantics_unstamped`
    /// clears the semantics stamp, so the current app rebuilds with the
    /// holds on its next open.
    #[test]
    fn a_v0_22_78_rebuild_leaves_holds_journals_stale() {
        let (_root, book) = book();
        let conn = book.connection();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        ensure(conn, PROVIDER_TRADE_HOLDS).unwrap();
        conn.execute_batch(
            "INSERT INTO workspaces(id,label,created_at) VALUES('ws','Main','2026-01-01');
             INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Book','2026-01-01');
             INSERT INTO provider_trade_holds(id,workspace_id,profile_id,provider,trade_id,trade_status,reason,state,created_at,updated_at)
               VALUES('h','ws','p','bisq','d15e0001','Mediated','not_completed','held','2026-01-01','2026-01-01');",
        )
        .unwrap();
        // The current writer publishes its markers, then stamps them.
        conn.execute(
            "UPDATE profiles SET last_processed_at='2026-01-02',last_processed_tx_count=1,last_processed_input_version=journal_input_version WHERE id='p'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE profiles SET journal_semantics_version=?1,required_journal_semantics_version=MAX(required_journal_semantics_version,?1) WHERE id='p'",
            [crate::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION],
        )
        .unwrap();
        conn.execute_batch("COMMIT").unwrap();
        assert!(crate::semantics::journal_current(conn, "p").unwrap());
        // v0.22.78's publication (kassiber/core/custody_journal.py at the
        // tag), verbatim: it knows nothing of the semantics columns.
        conn.execute(
            "UPDATE profiles
        SET last_processed_at = ?, last_processed_tx_count = ?,
            last_processed_input_version = journal_input_version,
            ownership_review_counts_json = ?
        WHERE id = ?",
            rusqlite::params!["2026-01-03T00:00:00Z", 1, "{}", "p"],
        )
        .unwrap();
        let (stamp, required): (i64, i64) = conn
            .query_row(
                "SELECT journal_semantics_version,required_journal_semantics_version FROM profiles WHERE id='p'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(stamp, 0);
        assert!(required >= 1);
        assert!(!crate::semantics::journal_current(conn, "p").unwrap());
    }
}

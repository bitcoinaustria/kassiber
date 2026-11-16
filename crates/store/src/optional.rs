//! Opt-in schema: objects a book gets only when the user first uses the
//! feature that needs them (`fixtures/optional-schema.json`).
//!
//! Bank statements (`docs/reference/bank-statements.md`) are the only such
//! feature. Their tables hold personal financial data, so a book that never
//! imported a statement carries none of them, and every existing or fresh
//! book keeps exactly the layout the manifest describes. Once created, the
//! objects are part of the expected schema and must match their definitions
//! exactly; a partial or drifted set fails the structural gate.
use crate::{map_sqlite_error, Result};
use rusqlite::Connection;
use serde_json::Value;
use std::sync::OnceLock;

const FIXTURE: &str = include_str!("../fixtures/optional-schema.json");

/// The bank-statement objects as `sqlite_master` stores them:
/// `[type, name, tbl_name, sql]`, sorted by type and name.
pub(crate) fn bank_statement_objects() -> &'static [Value] {
    static OBJECTS: OnceLock<Vec<Value>> = OnceLock::new();
    OBJECTS.get_or_init(|| {
        let fixture: Value = serde_json::from_str(FIXTURE).expect("optional schema fixture");
        fixture["bank_statements"]
            .as_array()
            .expect("bank statement objects")
            .clone()
    })
}

/// The bank-statement table names.
pub fn bank_statement_tables() -> Vec<&'static str> {
    bank_statement_objects()
        .iter()
        .filter(|o| o[0] == "table")
        .map(|o| o[1].as_str().expect("name"))
        .collect()
}

/// Whether this book has started storing bank statements.
pub fn bank_statements_present(conn: &Connection) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='bank_statement_lines')",
        [],
        |r| r.get(0),
    )
    .map_err(map_sqlite_error)
}

/// Create the bank-statement objects if absent. Call inside the caller's
/// write transaction, so a failed import leaves no tables behind.
pub fn ensure_bank_statement_schema(conn: &Connection) -> Result<()> {
    for kind in ["table", "index"] {
        for object in bank_statement_objects().iter().filter(|o| o[0] == kind) {
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

/// Names of every optional object, whether or not this book has them.
pub(crate) fn optional_names() -> impl Iterator<Item = (&'static str, &'static str)> {
    bank_statement_objects()
        .iter()
        .map(|o| (o[0].as_str().expect("type"), o[1].as_str().expect("name")))
}

/// The manifest's objects plus the optional objects this book has started
/// using, in `sqlite_master` order (type, then name).
pub(crate) fn expected_objects(conn: &Connection, manifest: &Value) -> Result<Value> {
    let mut objects = manifest["objects"].as_array().expect("objects").clone();
    let started: bool = conn
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name IN ({}))",
                optional_names()
                    .map(|(_, name)| format!("'{name}'"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            [],
            |r| r.get(0),
        )
        .map_err(map_sqlite_error)?;
    if started {
        objects.extend(bank_statement_objects().iter().cloned());
        objects.sort_by(|a, b| (a[0].as_str(), a[1].as_str()).cmp(&(b[0].as_str(), b[1].as_str())));
    }
    let mut expected = manifest.clone();
    expected["objects"] = Value::Array(objects);
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
    fn fresh_books_have_none_and_creation_is_idempotent_and_verified() {
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
        for object in bank_statement_objects() {
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
    fn a_partial_or_drifted_set_fails_closed() {
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
}

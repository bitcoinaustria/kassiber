//! `pinned_context_scope` and `PRAGMA query_only` for one MCP read
//! (`kassiber/mcp/tools.py` `run_tool`, `kassiber/db.py`).
//!
//! Python overrides `get_setting` for the two context keys while a read
//! runs. Here a temporary view shadows `settings` for the connection's
//! unqualified reads, answering `context_workspace` and `context_profile`
//! with the pin and every other key from the stored table. Nothing is
//! written to the book, and `query_only` refuses any write the read
//! attempts.

use rusqlite::Connection;

/// The statements that pin `(workspace, profile)` read-only, or lift it.
pub fn read_only_scope_sql(scope: Option<(&str, &str)>) -> String {
    match scope {
        Some((workspace, profile)) => {
            let quote = |text: &str| format!("'{}'", text.replace('\'', "''"));
            format!(
                "CREATE TEMP VIEW settings AS \
                 SELECT key, value FROM main.settings \
                 WHERE key NOT IN ('context_workspace', 'context_profile') \
                 UNION ALL SELECT 'context_workspace', {} \
                 UNION ALL SELECT 'context_profile', {}; \
                 PRAGMA query_only = ON;",
                quote(workspace),
                quote(profile)
            )
        }
        None => "PRAGMA query_only = OFF; DROP VIEW IF EXISTS temp.settings;".to_owned(),
    }
}

/// Apply [`read_only_scope_sql`] to `conn`.
pub fn pin_read_only_scope(conn: &Connection, scope: Option<(&str, &str)>) -> rusqlite::Result<()> {
    conn.execute_batch(&read_only_scope_sql(scope))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT);
             INSERT INTO settings VALUES
               ('context_workspace', 'stored-w'), ('context_profile', 'stored-p'), ('unrelated', 'kept');
             CREATE TABLE notes(id INTEGER PRIMARY KEY, text TEXT);",
        )
        .unwrap();
        conn
    }

    fn setting(conn: &Connection, key: &str) -> Option<String> {
        conn.query_row("SELECT value FROM settings WHERE key=?", [key], |row| {
            row.get(0)
        })
        .ok()
    }

    #[test]
    fn a_pin_answers_the_context_and_never_writes_it() {
        let conn = book();
        pin_read_only_scope(&conn, Some(("ws", "it's-a-book"))).unwrap();
        assert_eq!(setting(&conn, "context_workspace").as_deref(), Some("ws"));
        assert_eq!(
            setting(&conn, "context_profile").as_deref(),
            Some("it's-a-book")
        );
        assert_eq!(setting(&conn, "unrelated").as_deref(), Some("kept"));
        // The read runs read-only: no write reaches the book.
        assert!(conn
            .execute("INSERT INTO notes(text) VALUES ('x')", [])
            .is_err());
        assert!(conn
            .execute(
                "UPDATE main.settings SET value='w2' WHERE key='context_workspace'",
                []
            )
            .is_err());
        let query_only: i64 = conn
            .query_row("PRAGMA query_only", [], |row| row.get(0))
            .unwrap();
        assert_eq!(query_only, 1);
        pin_read_only_scope(&conn, None).unwrap();
        // The stored context is untouched, and the book is writable again.
        assert_eq!(
            setting(&conn, "context_workspace").as_deref(),
            Some("stored-w")
        );
        assert_eq!(
            setting(&conn, "context_profile").as_deref(),
            Some("stored-p")
        );
        conn.execute("INSERT INTO notes(text) VALUES ('x')", [])
            .unwrap();
        // A second call pins again on the same connection.
        pin_read_only_scope(&conn, Some(("a", "b"))).unwrap();
        assert_eq!(setting(&conn, "context_profile").as_deref(), Some("b"));
        pin_read_only_scope(&conn, None).unwrap();
    }
}

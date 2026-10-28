use super::*;
use kassiber_store::rusqlite::{self, types::ValueRef, Params};
use rusqlite::ffi;

pub fn sql_error(error: rusqlite::Error) -> AppError {
    if matches!(&error, rusqlite::Error::SqliteFailure(code, _) if matches!(code.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
    {
        AppError::new(
            "database_busy",
            "the database is busy in another Kassiber process",
        )
        .with_hint("Retry when the other process finishes its write.")
        .retryable(true)
    } else {
        AppError::new("internal_error", error.to_string())
    }
}
pub fn rows(conn: &Connection, query: &str, args: impl Params) -> Result<Vec<Value>> {
    raw_rows(conn, query, args).map_err(sql_error)
}

/// Only Python sqlite3.OperationalError makes an optional quote cache absent.
/// Invalid stored values and corruption must still fail the report closed.
pub fn optional_rows(conn: &Connection, query: &str, args: impl Params) -> Result<Vec<Value>> {
    match raw_rows(conn, query, args) {
        Err(error) if optional_cache_error(&error) => Ok(Vec::new()),
        result => result.map_err(sql_error),
    }
}

fn optional_cache_error(error: &rusqlite::Error) -> bool {
    // CPython 3.11 sqlite3's primary-code mapping to OperationalError.
    matches!(error, rusqlite::Error::SqliteFailure(code, _) if matches!(code.extended_code & 0xff,
        ffi::SQLITE_ERROR | ffi::SQLITE_PERM | ffi::SQLITE_ABORT | ffi::SQLITE_BUSY |
        ffi::SQLITE_LOCKED | ffi::SQLITE_READONLY | ffi::SQLITE_INTERRUPT |
        ffi::SQLITE_IOERR | ffi::SQLITE_FULL | ffi::SQLITE_CANTOPEN |
        ffi::SQLITE_PROTOCOL | ffi::SQLITE_EMPTY | ffi::SQLITE_SCHEMA |
        ffi::SQLITE_NOLFS | ffi::SQLITE_AUTH))
}

fn raw_rows(conn: &Connection, query: &str, args: impl Params) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare(query)?;
    let names: Vec<_> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let result = stmt
        .query_map(args, |row| {
            let mut fields = Vec::new();
            for (i, name) in names.iter().enumerate() {
                let v = match row.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => Value::Int(n.into()),
                    ValueRef::Real(n) => Value::Float(n),
                    ValueRef::Text(t) => s(std::str::from_utf8(t).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            i,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?),
                    ValueRef::Blob(_) => return Err(rusqlite::Error::InvalidQuery),
                };
                fields.push((name.clone(), v));
            }
            Ok(dict(fields))
        })?
        .collect::<std::result::Result<Vec<_>, _>>();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_quote_errors() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(optional_rows(&conn, "SELECT * FROM absent_cache", [])
            .unwrap()
            .is_empty());
        assert!(optional_rows(&conn, "SELECT x'aa' AS rate", []).is_err());
        assert!(rows(&conn, "SELECT * FROM absent_cache", []).is_err());
        assert!(!optional_cache_error(&rusqlite::Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_CORRUPT),
            None,
        )));
    }
}

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
        .with_sqlite_origin()
    } else {
        AppError::new("internal_error", error.to_string()).with_sqlite_origin()
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

pub(super) fn optional_cache_error(error: &rusqlite::Error) -> bool {
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
        .query_map(args, |row| row_value(row, &names))?
        .collect::<std::result::Result<Vec<_>, _>>();
    result
}
fn row_value(row: &rusqlite::Row<'_>, names: &[String]) -> rusqlite::Result<Value> {
    let mut fields = Vec::with_capacity(names.len());
    for (i, name) in names.iter().enumerate() {
        let v = column_value(row, i)?;
        fields.push((name.clone(), v));
    }
    Ok(dict(fields))
}

fn column_value(row: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Value> {
    Ok(match row.get_ref(i)? {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => Value::Int(n.into()),
        ValueRef::Real(n) => Value::Float(n),
        ValueRef::Text(t) => s(std::str::from_utf8(t).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e))
        })?),
        ValueRef::Blob(_) => return Err(rusqlite::Error::InvalidQuery),
    })
}

/// Stream rows without retaining an unused table projection. As with rows()
/// followed by a loop, SQL/codec errors anywhere take precedence over errors
/// from interpreting the first rows. Keep the first interpretation error while
/// finishing SQL conversion, then return it after the query succeeds.
pub fn visit_rows(
    conn: &Connection,
    query: &str,
    args: impl Params,
    mut visit: impl FnMut(&Value) -> Result<()>,
) -> Result<()> {
    let mut stmt = conn.prepare(query).map_err(sql_error)?;
    let names: Vec<_> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let mut query = stmt.query(args).map_err(sql_error)?;
    let mut interpreted = Ok(());
    let mut converted = dict(names.iter().map(|name| (name.as_str(), Value::Null)));
    while let Some(row) = query.next().map_err(sql_error)? {
        if let Value::Dict(fields) = &mut converted {
            for (i, (_, value)) in fields.iter_mut().enumerate() {
                if let ValueRef::Text(raw) = row.get_ref(i).map_err(sql_error)? {
                    let raw = std::str::from_utf8(raw).map_err(|error| {
                        sql_error(rusqlite::Error::FromSqlConversionFailure(
                            i,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        ))
                    })?;
                    if !matches!(value, Value::Str(old) if old.as_str() == Some(raw)) {
                        *value = s(raw);
                    }
                } else {
                    *value = column_value(row, i).map_err(sql_error)?;
                }
            }
        }
        if interpreted.is_ok() {
            interpreted = visit(&converted);
        }
    }
    interpreted
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
    #[test]
    fn streaming_reuses_keys_without_retaining_previous_column_values() {
        let conn = Connection::open_in_memory().unwrap();
        let query = "SELECT 'BTC' AS asset, 1 AS amount UNION ALL SELECT 'BTC',NULL UNION ALL SELECT 'LBTC',2.5 UNION ALL SELECT NULL,'text'";
        let expected = rows(&conn, query, []).unwrap();
        let mut actual = Vec::new();
        visit_rows(&conn, query, [], |row| {
            actual.push(row.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(actual, expected);
    }
    #[test]
    fn streaming_preserves_late_sql_error_precedence() {
        let conn = Connection::open_in_memory().unwrap();
        let invalid = || AppError::new("app_error", "invalid decimal");
        let query = "SELECT 'bad' AS rate UNION ALL SELECT x'aa'";
        let expected = rows(&conn, query, []).unwrap_err();
        let actual = visit_rows(&conn, query, [], |_| Err(invalid())).unwrap_err();
        assert_eq!(
            (actual.code, actual.message),
            (expected.code, expected.message)
        );
        let actual = visit_rows(&conn, "SELECT 'bad' AS rate UNION ALL SELECT 1", [], |_| {
            Err(invalid())
        })
        .unwrap_err();
        assert_eq!(actual.code, "app_error");
        assert_eq!(actual.message, "invalid decimal");
    }
}

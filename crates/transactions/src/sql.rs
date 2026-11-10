use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{types::ValueRef, Connection, Params};

pub type Result<T> = std::result::Result<T, AppError>;

pub fn string(value: &Value) -> Option<&str> {
    match value {
        Value::Str(text) => text.as_str(),
        _ => None,
    }
}

/// Scope is resolved by the book adapter before entering a domain service.
pub struct Scope<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
}

pub fn error(error: kassiber_store::rusqlite::Error) -> AppError {
    let e = kassiber_store::map_sqlite_error(error);
    AppError::new(e.code, e.message).retryable(e.retryable)
}

pub fn rows(conn: &Connection, sql: &str, params: impl Params) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql).map_err(error)?;
    let names: Vec<_> = stmt.column_names().iter().map(|x| x.to_string()).collect();
    let mut cursor = stmt.query(params).map_err(error)?;
    let mut result = Vec::new();
    while let Some(row) = cursor.next().map_err(error)? {
        let mut pairs = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let value = match row.get_ref(i).map_err(error)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => Value::Int(n.into()),
                ValueRef::Real(n) => Value::Float(n),
                ValueRef::Text(t) => s(std::str::from_utf8(t)
                    .map_err(|_| AppError::new("validation", "stored text is not valid UTF-8"))?),
                ValueRef::Blob(raw) => Value::Bytes(raw.to_vec()),
            };
            pairs.push((name.as_str(), value));
        }
        result.push(dict(pairs));
    }
    Ok(result)
}

pub fn now(conn: &Connection) -> Result<String> {
    conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')", [], |r| {
        r.get(0)
    })
    .map_err(error)
}

pub fn uuid(conn: &Connection) -> Result<String> {
    let mut bytes: Vec<u8> = conn
        .query_row("SELECT randomblob(16)", [], |r| r.get(0))
        .map_err(error)?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

pub fn atomic<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    kassiber_store::begin_write_savepoint(conn, "transaction_service").map_err(error)?;
    match f() {
        Ok(value) => {
            if let Err(e) = conn.execute_batch("RELEASE SAVEPOINT transaction_service") {
                let _ = conn.execute_batch("ROLLBACK TO SAVEPOINT transaction_service; RELEASE SAVEPOINT transaction_service");
                return Err(error(e));
            }
            Ok(value)
        }
        Err(e) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT transaction_service; RELEASE SAVEPOINT transaction_service",
            );
            Err(e)
        }
    }
}

pub fn invalidate(conn: &Connection, profile_id: &str) -> Result<()> {
    conn.execute("UPDATE profiles SET last_processed_at=NULL, last_processed_tx_count=0, journal_input_version=journal_input_version+1, ownership_review_counts_json=NULL WHERE id=?", [profile_id]).map_err(error)?;
    Ok(())
}

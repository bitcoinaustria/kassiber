use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{types::ValueRef, Connection, Params};

pub type Result<T> = std::result::Result<T, AppError>;

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
                ValueRef::Blob(_) => {
                    return Err(AppError::new("validation", "unexpected binary field"))
                }
            };
            pairs.push((name.as_str(), value));
        }
        result.push(dict(pairs));
    }
    Ok(result)
}

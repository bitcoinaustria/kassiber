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

/// Injectable UUID/clock source for Python row oracles; production has no overrides.
pub trait Sources {
    fn uuid(&self, conn: &Connection) -> Result<String>;
    fn now(&self, conn: &Connection) -> Result<String>;
}
pub struct SystemSources;
impl Sources for SystemSources {
    fn uuid(&self, conn: &Connection) -> Result<String> {
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
    fn now(&self, conn: &Connection) -> Result<String> {
        conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')", [], |r| {
            r.get(0)
        })
        .map_err(error)
    }
}
/// Own the operation savepoint; never commit another service's partial writes.
pub fn atomic<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT transfers_service")
        .map_err(error)?;
    match f() {
        Ok(value) => {
            if let Err(e) = conn.execute_batch("RELEASE SAVEPOINT transfers_service") {
                let _ = conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT transfers_service; RELEASE SAVEPOINT transfers_service",
                );
                return Err(error(e));
            }
            Ok(value)
        }
        Err(e) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT transfers_service; RELEASE SAVEPOINT transfers_service",
            );
            Err(e)
        }
    }
}

pub fn bind(value: &Value, index: usize) -> Result<kassiber_store::rusqlite::types::Value> {
    use kassiber_store::rusqlite::types::Value as Sql;
    match value {
        Value::Null => Ok(Sql::Null),
        Value::Bool(v) => Ok(Sql::Integer(i64::from(*v))),
        Value::Int(n) => n.to_string().parse().map(Sql::Integer).map_err(|_| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        }),
        Value::Float(v) => Ok(Sql::Real(*v)),
        Value::Str(v) => v.as_str().map(|s| Sql::Text(s.into())).ok_or_else(|| {
            AppError::new("internal_error", "surrogates not allowed in SQLite text")
        }),
        _ => Err(AppError::new(
            "internal_error",
            format!(
                "Error binding parameter {index}: type '{}' is not supported",
                kassiber_envelope::type_name(value)
            ),
        )),
    }
}

//! Local book setup shared by the CLI and desktop daemon. No transport clients.
pub mod accounts;
pub mod agent_access;
mod backend_runtime;
pub mod backends;
pub mod context;
pub mod core_credentials;
pub mod networks;
pub mod offline;
mod profile_desktop;
pub mod profile_reset;
pub mod profile_snapshot;
pub mod profiles;
pub mod projects;
pub mod references;
pub mod sensitive;
pub mod views;
mod workspace_desktop;
pub mod workspaces;

use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{self, types::ValueRef, Connection, OptionalExtension, Params, Row};

pub type Result<T> = std::result::Result<T, AppError>;

fn storage_error(error: kassiber_paths::Error) -> AppError {
    AppError {
        code: error.code.to_owned(),
        message: error.message.to_owned(),
        hint: error.hint.map(Box::from),
        details: if error
            .details
            .as_object()
            .is_some_and(|fields| fields.is_empty())
        {
            None
        } else {
            kassiber_pytext::loads(&error.details.to_string()).ok()
        },
        retryable: error.retryable,
        message_codepoints: None,
        sqlite_origin: false,
    }
}

/// Python's outer CLI/daemon exception mapping; never include bound parameters.
fn sql_error(error: rusqlite::Error) -> AppError {
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
fn row_value(row: &Row<'_>) -> rusqlite::Result<Value> {
    let mut fields = Vec::with_capacity(row.as_ref().column_count());
    for (index, name) in row.as_ref().column_names().iter().enumerate() {
        let value = column_value(row, index)?;
        fields.push(((*name).to_owned(), value));
    }
    Ok(dict(fields))
}
fn column_value(row: &Row<'_>, index: usize) -> rusqlite::Result<Value> {
    Ok(match row.get_ref(index)? {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => Value::Int(n.into()),
        ValueRef::Real(n) => Value::Float(n),
        ValueRef::Text(v) => s(std::str::from_utf8(v).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?),
        ValueRef::Blob(_) => return Err(rusqlite::Error::InvalidQuery),
    })
}
/// Decode every SQL value in order, but retain only one row dictionary.
fn visit_rows(
    conn: &Connection,
    sql: &str,
    params: impl Params,
    mut visit: impl FnMut(&Value),
) -> Result<()> {
    let mut stmt = conn.prepare(sql).map_err(sql_error)?;
    let mut converted = dict(stmt.column_names().iter().map(|name| (*name, Value::Null)));
    let mut query = stmt.query(params).map_err(sql_error)?;
    while let Some(row) = query.next().map_err(sql_error)? {
        if let Value::Dict(fields) = &mut converted {
            for (index, (_, value)) in fields.iter_mut().enumerate() {
                if let ValueRef::Text(raw) = row.get_ref(index).map_err(sql_error)? {
                    let raw = std::str::from_utf8(raw).map_err(|error| {
                        sql_error(rusqlite::Error::FromSqlConversionFailure(
                            index,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        ))
                    })?;
                    if !matches!(value, Value::Str(old) if old.as_str() == Some(raw)) {
                        *value = s(raw);
                    }
                } else {
                    *value = column_value(row, index).map_err(sql_error)?;
                }
            }
        }
        visit(&converted);
    }
    Ok(())
}

fn rows(conn: &Connection, sql: &str, params: impl Params) -> Result<Vec<Value>> {
    conn.prepare(sql)
        .map_err(sql_error)?
        .query_map(params, row_value)
        .map_err(sql_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_error)
}
fn one(conn: &Connection, sql: &str, params: impl Params) -> Result<Value> {
    conn.query_row(sql, params, row_value).map_err(sql_error)
}
fn setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key=?", [key], |r| {
        r.get(0)
    })
    .optional()
    .map_err(sql_error)
}
fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [key,value]).map_err(sql_error)?;
    Ok(())
}
fn text(value: &Value, key: &str) -> String {
    get(value, key).map(py_str).unwrap_or_else(|| "None".into())
}
fn optional<'a>(args: &'a Value, key: &str) -> Option<&'a Value> {
    get(args, key).filter(|v| !matches!(v, Value::Null))
}
fn flag(value: &Value, key: &str) -> bool {
    get(value, key).is_some_and(truthy)
}
fn replace(value: &mut Value, key: &str, replacement: Value) {
    if let Value::Dict(fields) = value {
        if let Some((_, slot)) = fields.iter_mut().find(|(k, _)| *k == key.into()) {
            *slot = replacement;
        } else {
            fields.push((key.into(), replacement));
        }
    }
}
fn integrity(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::ConstraintViolation)
}

/// Injectable identity/clock source; default timestamps match `time_utils.now_iso`.
pub trait Sources {
    fn uuid(&self, conn: &Connection) -> Result<String>;
    fn now(&self, conn: &Connection) -> Result<String>;
}
pub struct SystemSources;
impl Sources for SystemSources {
    fn uuid(&self, conn: &Connection) -> Result<String> {
        let mut bytes: Vec<u8> = conn
            .query_row("SELECT randomblob(16)", [], |r| r.get(0))
            .map_err(sql_error)?;
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
        .map_err(sql_error)
    }
}

/// Executes one admitted operation atomically. Error/unwind drops the transaction.
/// The shell selects supported operations before opening a book.
pub fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let tx = conn.unchecked_transaction().map_err(sql_error)?;
    let result = if command.starts_with("ui.workspace.") {
        workspace_desktop::execute(&tx, command, args, sources)
    } else if command.starts_with("workspaces.") {
        workspaces::execute(&tx, command, args, sources)
    } else if command.starts_with("profiles.") || command.starts_with("ui.profiles.") {
        profiles::execute(&tx, command, args, sources)
    } else if command.starts_with("accounts.") {
        accounts::execute(&tx, command, args, sources)
    } else if command.starts_with("context.") {
        context::execute(&tx, command, args)
    } else if command.starts_with("networks.") || command.starts_with("ui.networks.") {
        networks::execute(&tx, command, args, sources)
    } else if command.starts_with("views.") || command.starts_with("ui.saved_views.") {
        views::execute(&tx, command, args, sources)
    } else {
        Err(AppError::new(
            "not_implemented",
            "setup operation is not implemented in the Rust backend yet",
        ))
    }?;
    tx.commit().map_err(sql_error)?;
    Ok(result)
}

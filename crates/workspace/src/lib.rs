//! Workspace shell reads shared by the CLI and the desktop daemon: journal
//! readiness, next actions, review badges and the book/book-set overviews.
//! Every read is local: none processes journals, runs a matcher, infers
//! custody, writes the book or initiates network traffic.
pub mod overview;
pub mod readiness;

use kassiber_envelope::{get, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{types::ValueRef, Connection, Params};

pub type Result<T> = std::result::Result<T, AppError>;

/// Python's outer CLI/daemon mapping of SQLite failures.
pub(crate) fn sql_error(error: kassiber_store::rusqlite::Error) -> AppError {
    use kassiber_store::rusqlite::{Error, ErrorCode};
    if matches!(&error, Error::SqliteFailure(code, _) if matches!(code.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked))
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

/// Rows as Python's `sqlite3.Row` mappings, in column order.
pub(crate) fn rows(conn: &Connection, sql: &str, params: impl Params) -> Result<Vec<Value>> {
    let mut statement = conn.prepare(sql).map_err(sql_error)?;
    let names: Vec<String> = statement
        .column_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let mut cursor = statement.query(params).map_err(sql_error)?;
    let mut output = Vec::new();
    while let Some(row) = cursor.next().map_err(sql_error)? {
        let mut fields = Vec::with_capacity(names.len());
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index).map_err(sql_error)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => Value::Int(n.into()),
                ValueRef::Real(n) => Value::Float(n),
                ValueRef::Text(bytes) => Value::Str(
                    std::str::from_utf8(bytes)
                        .map_err(|error| AppError::new("internal_error", error.to_string()))?
                        .into(),
                ),
                ValueRef::Blob(_) => {
                    return Err(AppError::new(
                        "internal_error",
                        "unexpected BLOB value in a workspace read",
                    ))
                }
            };
            fields.push((name.clone(), value));
        }
        output.push(kassiber_envelope::dict(fields));
    }
    Ok(output)
}

pub(crate) fn val(row: &Value, key: &str) -> Value {
    get(row, key).cloned().unwrap_or(Value::Null)
}

/// Text of a stored column; non-text values render as Python's `str()`.
pub(crate) fn text(row: &Value, key: &str) -> String {
    match get(row, key) {
        None | Some(Value::Null) => String::new(),
        Some(value) => kassiber_envelope::py_str(value),
    }
}

/// `int(value or 0)` for counts SQLite always answers as integers.
pub(crate) fn count(row: &Value, key: &str) -> i64 {
    match get(row, key) {
        Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap_or_default(),
        Some(Value::Bool(b)) => i64::from(*b),
        _ => 0,
    }
}

/// Python's `int(value)` for a stored SQLite value.
pub(crate) fn py_int(value: &Value) -> Result<kassiber_pyjson::BigInt> {
    match value {
        Value::Int(n) => Ok(n.clone()),
        Value::Bool(n) => Ok(u8::from(*n).into()),
        Value::Str(v) => kassiber_pytext::parse_int(v.as_str().unwrap_or_default()).map_err(|_| {
            AppError::new(
                "internal_error",
                format!(
                    "invalid literal for int() with base 10: {}",
                    kassiber_envelope::py_repr(value)
                ),
            )
        }),
        Value::Float(n) if n.is_finite() => format!("{:.0}", n.trunc())
            .parse()
            .map_err(|_| AppError::new("internal_error", "cannot convert float to integer")),
        Value::Float(n) => Err(AppError::new(
            "internal_error",
            if n.is_nan() {
                "cannot convert float NaN to integer"
            } else {
                "cannot convert float infinity to integer"
            },
        )),
        _ => Err(AppError::new(
            "internal_error",
            format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                kassiber_envelope::type_name(value)
            ),
        )),
    }
}

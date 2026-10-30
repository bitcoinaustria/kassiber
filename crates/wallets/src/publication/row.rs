//! `sqlite3.Row`-shaped reads for the publication writers. Columns become
//! Python values so the ported predicates can keep Python's field semantics.
use crate::database_error;
use kassiber_envelope::{get, py_str, truthy, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_store::rusqlite::{types::ValueRef, Connection, Params, Row};

pub(crate) type Result<T> = std::result::Result<T, AppError>;

/// Every publication writer belongs to the coordinator's wallet transaction.
pub(crate) fn require_transaction(conn: &Connection) -> Result<()> {
    if conn.is_autocommit() {
        return Err(AppError::new(
            "observer_apply_outside_transaction",
            "Publication requires the caller's wallet transaction",
        ));
    }
    Ok(())
}

fn column(value: ValueRef<'_>) -> Result<Value> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => Value::Int(n.into()),
        ValueRef::Real(n) => Value::Float(n),
        ValueRef::Text(text) => Value::from(
            std::str::from_utf8(text)
                .map_err(|_| AppError::new("database_error", "Stored text is not valid UTF-8"))?,
        ),
        // Python would surface `bytes`; no publication column stores a blob.
        ValueRef::Blob(_) => {
            return Err(AppError::new(
                "observer_state_invalid",
                "Publication rows cannot contain binary values",
            ))
        }
    })
}

fn convert(row: &Row<'_>) -> Result<Value> {
    let statement = row.as_ref();
    let mut pairs = Vec::with_capacity(statement.column_count());
    for index in 0..statement.column_count() {
        let name = statement
            .column_name(index)
            .map_err(database_error)?
            .to_owned();
        pairs.push((
            Key::from(name.as_str()),
            column(row.get_ref(index).map_err(database_error)?)?,
        ));
    }
    Ok(Value::Dict(pairs))
}

/// All rows of one query, each as an insertion-ordered column dict.
pub(crate) fn rows(conn: &Connection, sql: &str, params: impl Params) -> Result<Vec<Value>> {
    let mut statement = conn.prepare(sql).map_err(database_error)?;
    let mut cursor = statement.query(params).map_err(database_error)?;
    let mut output = Vec::new();
    while let Some(row) = cursor.next().map_err(database_error)? {
        output.push(convert(row)?);
    }
    Ok(output)
}

pub(crate) fn first(conn: &Connection, sql: &str, params: impl Params) -> Result<Option<Value>> {
    let mut statement = conn.prepare(sql).map_err(database_error)?;
    let mut cursor = statement.query(params).map_err(database_error)?;
    cursor
        .next()
        .map_err(database_error)?
        .map(convert)
        .transpose()
}

/// `row[key]` that reads a missing column as `None`, like `_row_field`.
pub(crate) fn field<'a>(row: &'a Value, key: &str) -> &'a Value {
    get(row, key).unwrap_or(&Value::Null)
}

/// `str(value or "")`.
pub(crate) fn or_text(value: &Value) -> String {
    if truthy(value) {
        py_str(value)
    } else {
        String::new()
    }
}

/// `value in (None, "")`.
pub(crate) fn blank(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Str(text) => text.code_points().next().is_none(),
        _ => false,
    }
}

/// Python `==` for JSON-shaped and SQLite values, including numeric towers.
pub(crate) fn py_eq(a: &Value, b: &Value) -> bool {
    use num_cmp::numeric;
    match (numeric(a), numeric(b)) {
        (Some(x), Some(y)) => x == y,
        _ => match (a, b) {
            (Value::List(x), Value::List(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(x, y)| py_eq(x, y))
            }
            (Value::Dict(x), Value::Dict(y)) => {
                x.len() == y.len()
                    && x.iter().all(|(key, value)| {
                        y.iter()
                            .rev()
                            .find(|(other, _)| other == key)
                            .is_some_and(|(_, other)| py_eq(value, other))
                    })
            }
            _ => a == b,
        },
    }
}

mod num_cmp {
    use kassiber_pyjson::Value;

    /// Exact rational comparison key for bool/int/float (NaN never equal).
    #[derive(PartialEq)]
    pub(super) enum Numeric {
        Int(kassiber_pyjson::BigInt),
        Float(f64),
    }

    pub(super) fn numeric(value: &Value) -> Option<Numeric> {
        match value {
            Value::Bool(b) => Some(Numeric::Int(i64::from(*b).into())),
            Value::Int(n) => Some(Numeric::Int(n.clone())),
            Value::Float(f) => {
                if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.2e18 {
                    Some(Numeric::Int((*f as i64).into()))
                } else {
                    Some(Numeric::Float(*f))
                }
            }
            _ => None,
        }
    }
}

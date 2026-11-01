//! Authored investigation operations. These claims never grant ownership or
//! accounting authority. The transport resolves scope; this service owns writes.
use crate::analysis_index::Text;
use crate::database_error;
use crate::publication::row::{field, first, py_eq, Result};
use crate::publication::wallet::{Sources, SystemSources};
use kassiber_envelope::{dict, get, s, set, truthy, AppError};
use kassiber_pyjson::{dumps, Key, Options, Value};
use kassiber_pytext::{lower, strip_py};
use kassiber_store::rusqlite::{params, types::Value as Sql, Connection};

mod cases;
mod datasets;
#[cfg(any(test, feature = "test-harness"))]
pub mod fixture;
mod labels;

pub struct Scope<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
}

pub fn execute(
    conn: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
) -> Result<Value> {
    execute_with_sources(conn, scope, operation, args, &mut SystemSources::default())
}

pub fn execute_with_sources(
    conn: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    let mut args = match args {
        Value::Null => dict::<&str>([]),
        Value::Dict(_) => args.clone(),
        _ => return Err(invalid("Chain analysis arguments must be an object")),
    };
    if let Some(expected) = get(&args, "expected_scope") {
        if expected != &Value::Null
            && !py_eq(
                expected,
                &dict([
                    ("workspace_id", s(scope.workspace_id)),
                    ("profile_id", s(scope.profile_id)),
                ]),
            )
        {
            return Err(AppError::new(
                "scope_changed",
                "The active book changed; reopen the investigation in its original book",
            ));
        }
    }
    remove(&mut args, "expected_scope");
    match operation {
        "labels.upsert" => labels::upsert(conn, scope.profile_id, &args, sources),
        "labels.delete" => labels::delete(conn, scope.profile_id, &args, sources),
        "labels.import" => labels::import(conn, scope.profile_id, &args, sources),
        "cases.save" => cases::save(conn, scope.profile_id, &args, sources),
        "cases.delete" => cases::delete(conn, scope.profile_id, &args),
        "datasets.revoke" => datasets::revoke(conn, scope.profile_id, &args, sources),
        "datasets.discard" => datasets::discard(conn, scope.profile_id, &args),
        _ => Err(AppError::new(
            "not_implemented",
            "Unsupported chain analysis operation",
        )),
    }
}

fn invalid(message: &str) -> AppError {
    AppError::new("validation", message)
}

fn stale(message: &str) -> AppError {
    AppError::new("chain_analysis_stale", message).retryable(true)
}

fn arguments(args: &Value, allowed: &[&str], required: &[&str]) -> Result<()> {
    let Value::Dict(pairs) = args else {
        return Err(invalid("Unsupported or missing chain analysis arguments"));
    };
    if pairs.iter().any(|(key, _)| {
        !matches!(key, Key::Str(text) if text.as_str().is_some_and(|k| allowed.contains(&k)))
    }) || required.iter().any(|key| get(args, key).is_none()) {
        return Err(invalid("Unsupported or missing chain analysis arguments"));
    }
    Ok(())
}

fn text_value(value: &Value, field: &str, limit: usize) -> Result<kassiber_pyjson::PyStr> {
    if let Value::Str(text) = value {
        if text.code_points().count() <= limit && !text.code_points().any(|c| c < 32) {
            let text = strip_py(text);
            if text.code_points().next().is_some() {
                return Ok(text);
            }
        }
    }
    Err(invalid(&format!(
        "{field} must be nonempty text up to {limit} characters"
    )))
}

fn remove(value: &mut Value, name: &str) {
    if let Value::Dict(pairs) = value {
        pairs.retain(|(k, _)| k != &Key::from(name));
    }
}

fn canonical(value: &Value) -> Result<String> {
    dumps(
        value,
        &Options::new().sort_keys(true).compact().allow_nan(false),
    )
    .map_err(|error| AppError::new("internal_error", error.to_string()))
}

fn sql(value: &Value, index: usize) -> Result<Sql> {
    Ok(match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Int(n) => Sql::Integer(i64::try_from(n.clone()).map_err(|_| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        })?),
        Value::Float(n) => Sql::Real(*n),
        Value::Str(t) => Sql::Text(t.as_str().ok_or_else(|| encoding_error(t))?.to_owned()),
        other => {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "Error binding parameter {index}: type '{}' is not supported",
                    kassiber_envelope::type_name(other)
                ),
            ))
        }
    })
}

fn encoding_error(text: &kassiber_pyjson::PyStr) -> AppError {
    let points: Vec<_> = text.code_points().collect();
    let start = points
        .iter()
        .position(|p| (0xd800..=0xdfff).contains(p))
        .unwrap();
    let end = start
        + points[start..]
            .iter()
            .take_while(|p| (0xd800..=0xdfff).contains(*p))
            .count();
    let position = if end == start + 1 {
        format!("character '\\u{:04x}' in position {start}", points[start])
    } else {
        format!("characters in position {start}-{}", end - 1)
    };
    AppError::new(
        "internal_error",
        format!("'utf-8' codec can't encode {position}: surrogates not allowed"),
    )
}

fn atomic<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT chain_analysis_ops")
        .map_err(database_error)?;
    match f() {
        Ok(value) => match conn.execute_batch("RELEASE SAVEPOINT chain_analysis_ops") {
            Ok(()) => Ok(value),
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK TO SAVEPOINT chain_analysis_ops; RELEASE SAVEPOINT chain_analysis_ops");
                Err(database_error(e))
            }
        },
        Err(e) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT chain_analysis_ops; RELEASE SAVEPOINT chain_analysis_ops",
            );
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests;

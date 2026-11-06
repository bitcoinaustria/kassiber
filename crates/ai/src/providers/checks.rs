//! `kassiber/ai/provider_checks.py`: the last explicit check of each AI
//! provider, kept across restarts so Settings shows what it found instead of
//! "Not checked yet". Only a check the user started writes here; reading it
//! contacts nothing.
//!
//! A result belongs to the endpoint it was found at: it is stored with the
//! provider's base URL and read back only while the provider still has that
//! URL. A new API key or address clears it. Model lists are capped so one
//! large catalog cannot bloat the book.

use kassiber_envelope::{get, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::{params, Connection, OptionalExtension};

use super::{database_error, value_of};
use crate::discovery::Snapshot;

/// `CHECK_KIND_RUNTIME`: a CLI runtime status check.
pub const CHECK_KIND_RUNTIME: &str = "runtime";
/// `CHECK_KIND_MODELS`: a model listing.
pub const CHECK_KIND_MODELS: &str = "models";
/// `MAX_SAVED_MODELS`.
pub const MAX_SAVED_MODELS: usize = 1000;

/// `_bounded(kind, value)`.
fn bounded(kind: &str, value: &Value) -> Value {
    match (kind, value) {
        (CHECK_KIND_MODELS, Value::List(models)) => {
            Value::List(models.iter().take(MAX_SAVED_MODELS).cloned().collect())
        }
        (CHECK_KIND_RUNTIME, Value::List(statuses)) => Value::List(
            statuses
                .iter()
                .map(|status| match (status, get(status, "models")) {
                    (Value::Dict(pairs), Some(Value::List(models))) => {
                        let models =
                            Value::List(models.iter().take(MAX_SAVED_MODELS).cloned().collect());
                        Value::Dict(
                            pairs
                                .iter()
                                .map(|(key, item)| {
                                    if *key == "models".into() {
                                        (key.clone(), models.clone())
                                    } else {
                                        (key.clone(), item.clone())
                                    }
                                })
                                .collect(),
                        )
                    }
                    _ => status.clone(),
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// `json.dumps(value, default=str)`.
fn text(value: &Value) -> Result<String, AppError> {
    dumps(value, &Options::new().default_str(true))
        .map_err(|error| AppError::new("internal_error", error.to_string()))
}

/// `save_provider_check(conn, provider_name, kind, base_url, snapshot)`.
pub fn save(
    conn: &Connection,
    provider: &str,
    kind: &str,
    base_url: &str,
    snapshot: &Snapshot,
) -> Result<(), AppError> {
    if ![CHECK_KIND_RUNTIME, CHECK_KIND_MODELS].contains(&kind) {
        return Err(AppError::new(
            "internal_error",
            format!("unknown provider check kind: {kind}"),
        ));
    }
    let error = match &snapshot.error {
        Some(error) if kassiber_envelope::truthy(error) => Some(text(error)?),
        _ => None,
    };
    conn.execute(
        "INSERT INTO ai_provider_checks(provider_name, check_kind, base_url, checked_at, stale, error_json, value_json) VALUES(?, ?, ?, ?, ?, ?, ?) ON CONFLICT(provider_name, check_kind) DO UPDATE SET base_url = excluded.base_url, checked_at = excluded.checked_at, stale = excluded.stale, error_json = excluded.error_json, value_json = excluded.value_json",
        params![
            provider,
            kind,
            base_url,
            snapshot.checked_at,
            i64::from(snapshot.stale),
            error,
            text(&bounded(kind, &snapshot.value))?
        ],
    )
    .map_err(database_error)?;
    super::commit(conn)
}

/// `load_provider_check(conn, provider_name, kind, base_url)`: the last
/// check, if it was made at `base_url`.
pub fn load(
    conn: &Connection,
    provider: &str,
    kind: &str,
    base_url: &str,
) -> Result<Option<Snapshot>, AppError> {
    let row = conn
        .query_row(
            "SELECT base_url, checked_at, stale, error_json, value_json FROM ai_provider_checks WHERE provider_name = ? AND check_kind = ?",
            params![provider, kind],
            |row| {
                Ok([
                    value_of(row.get_ref(0)?),
                    value_of(row.get_ref(1)?),
                    value_of(row.get_ref(2)?),
                    value_of(row.get_ref(3)?),
                    value_of(row.get_ref(4)?),
                ])
            },
        )
        .optional()
        .map_err(database_error)?;
    let Some([stored_url, checked_at, stale, error_json, value_json]) = row else {
        return Ok(None);
    };
    if stored_url != kassiber_envelope::s(base_url) {
        return Ok(None);
    }
    // `json.loads` of a non-text column raises TypeError: no snapshot.
    let parse = |value: &Value| match value {
        Value::Str(text) => kassiber_pytext::loads(&crate::pyval::lossy(text)).ok(),
        _ => None,
    };
    let Some(value) = parse(&value_json) else {
        return Ok(None);
    };
    let error = if kassiber_envelope::truthy(&error_json) {
        match parse(&error_json) {
            Some(error) => Some(error),
            None => return Ok(None),
        }
    } else {
        None
    };
    Ok(Some(Snapshot {
        value,
        checked_at: kassiber_envelope::py_str(&checked_at),
        stale: kassiber_envelope::truthy(&stale),
        error: error.filter(|error| matches!(error, Value::Dict(_))),
    }))
}

/// `clear_provider_checks(conn, provider_name)` without its commit: a key,
/// address or row change forgets what the old one found.
pub fn clear(conn: &Connection, provider: &str) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM ai_provider_checks WHERE provider_name = ?",
        params![provider],
    )
    .map_err(database_error)?;
    Ok(())
}

/// `_last_checked_at(conn, provider)`: when the provider was last checked at
/// its current base URL, if ever.
pub fn last_checked_at(
    conn: &Connection,
    provider: &str,
    base_url: &Value,
) -> Result<Value, AppError> {
    conn.query_row(
        "SELECT MAX(checked_at) AS checked_at FROM ai_provider_checks WHERE provider_name = ? AND base_url = ?",
        params![provider, kassiber_envelope::py_str(base_url)],
        |row| Ok(value_of(row.get_ref(0)?)),
    )
    .map_err(database_error)
}

//! Stored review facts are distinct from observation, matching and journal authority.
mod component_reads;
mod component_review;
mod component_rows;
mod component_store;
mod coverage;
pub mod custody_ai_audit;
mod dismissals;
mod gap_candidates;
mod gap_review;
mod gap_review_store;
mod gap_snapshot;
mod history;
pub mod legacy_refresh;
mod pair_authoring;
mod pair_spec;
pub mod pairing;
mod pairing_commands;
mod payout_create;
mod payout_delete;
mod records;
pub mod review_artifact;
pub mod review_cases;
mod review_context;
pub mod review_operations;
pub mod review_reading;
mod rules;
#[cfg(not(feature = "test-harness"))]
mod sql;
#[cfg(feature = "test-harness")]
#[doc(hidden)]
pub mod sql;

pub use component_store::authored_active_components;
use kassiber_envelope::{dict, get, py_str, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
pub use records::pair_review_records;
use sql::{rows, Result};
pub use sql::{Scope, Sources, SystemSources};

fn value(row: &Value, key: &str) -> Value {
    get(row, key).cloned().unwrap_or(Value::Null)
}
fn text(row: &Value, key: &str) -> String {
    get(row, key).map(py_str).unwrap_or_default()
}
fn project(row: &Value, keys: &[&str]) -> Value {
    dict(keys.iter().map(|key| (*key, value(row, key))))
}
fn parse_json(text: &str) -> Result<Value> {
    kassiber_pytext::loads(text).map_err(|e| AppError::new("internal_error", e.to_string()))
}

pub fn execute(
    conn: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
) -> Result<Value> {
    execute_with_sources(conn, scope, operation, args, &SystemSources)
}

pub fn execute_with_sources(
    conn: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    match operation {
        "transfers.components.list" | "transfers.components.show" => {
            component_reads::execute(conn, scope, operation, args)
        }
        "transfers.suggest" | "transfers.bulk-pair" | "transfers.rules.apply" => {
            pairing_commands::execute(conn, scope, operation, args, sources)
        }
        "transfers.pair" | "transfers.unpair" | "transfers.update" => {
            pair_authoring::execute(conn, scope, operation, args, sources)
        }
        "transfers.payouts.create" => payout_create::execute(conn, scope, args, sources),
        "transfers.gaps.plan" | "transfers.gaps.apply" => {
            gap_review::execute(conn, scope, operation, args, sources)
        }
        "transfers.gaps.list" | "transfers.gaps.review" => {
            gap_snapshot::execute(conn, scope, operation, args)
        }
        "transfers.components.plan" | "transfers.components.apply" => {
            component_review::execute(conn, scope, operation, args, sources)
        }
        "transfers.payouts.delete" => payout_delete::execute(conn, scope, args, sources),
        "transfers.dismiss" => dismissals::dismiss(conn, scope, args, sources),
        "transfers.dismissals.list" => dismissals::list(conn, scope, args, sources),
        "transfers.dismissals.delete" => dismissals::delete(conn, scope, args),
        "transfers.rules.create"
        | "transfers.rules.delete"
        | "transfers.rules.enable"
        | "transfers.rules.disable" => rules::write(conn, scope, operation, args, sources),
        "custody.coverage" => coverage::snapshot(conn, scope),
        "transfers.review-context" => review_context::execute(conn, scope, args),
        "transfers.list" => records::pairs(conn, scope),
        "transfers.payouts.list" => records::payouts(conn, scope),
        "transfers.gaps.history" => history::list(conn, scope, args),
        "transfers.rules.list" => rules::list(conn, scope),
        "review.receipt" => receipt(conn, scope, args),
        _ => Err(AppError::new(
            "not_implemented",
            format!("Transfer service '{operation}' is not implemented"),
        )),
    }
}

fn receipt(conn: &Connection, scope: &Scope<'_>, args: &Value) -> Result<Value> {
    let id = get(args, "receipt_id").filter(|v| !matches!(v, Value::Null));
    let key = get(args, "idempotency_key").filter(|v| !matches!(v, Value::Null));
    let (field, selector) = match (id, key) {
        (Some(id), None) => ("id", id),
        (None, Some(key)) => ("idempotency_key", key),
        _ => {
            return Err(AppError::new(
                "validation",
                "Specify exactly one receipt_id or idempotency_key",
            ))
        }
    };
    let row = rows(
        conn,
        &format!(
            "SELECT receipt_json FROM review_workflow_receipts WHERE profile_id=? AND {field}=?"
        ),
        kassiber_store::rusqlite::params![scope.profile_id, sql::bind(selector, 2)?],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "Review receipt was not found"))?;
    parse_json(&text(&row, "receipt_json"))
}

fn integer(value: &Value) -> Result<kassiber_pyjson::BigInt> {
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

#[doc(hidden)]
pub mod review_support {
    /// The review workflow's row and binding helpers, without exposing the module.
    pub mod sql {
        pub use crate::sql::{bind, error, rows};
    }
    pub fn parse_json(text: &str) -> super::Result<super::Value> {
        super::parse_json(text)
    }
    use super::*;
    pub fn value(row: &Value, key: &str) -> Value {
        super::value(row, key)
    }
    pub fn text(row: &Value, key: &str) -> String {
        super::text(row, key)
    }
    pub fn reference_bind(
        value: &Value,
        index: usize,
    ) -> Result<kassiber_store::rusqlite::types::Value> {
        component_reads::reference_bind(value, index)
    }
    pub fn require_input_version(
        conn: &Connection,
        scope: &Scope<'_>,
        expected: &Value,
    ) -> Result<i64> {
        component_rows::require_input_version(conn, scope, expected)
    }
    pub fn input_version(profile: &Value) -> i64 {
        component_rows::input_version(profile)
    }
}

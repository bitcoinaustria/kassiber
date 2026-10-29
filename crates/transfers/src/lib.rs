//! Stored review facts are distinct from observation, matching and journal authority.
mod history;
mod records;
mod rules;
mod sql;

use kassiber_envelope::{dict, get, py_str, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
pub use sql::Scope;
use sql::{rows, Result};

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
    match operation {
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
        [scope.profile_id, &py_str(selector)],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "Review receipt was not found"))?;
    parse_json(&text(&row, "receipt_json"))
}

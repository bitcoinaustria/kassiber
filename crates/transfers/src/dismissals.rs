//! Dismissal is an advisory hint. It does not pair, own, or carry any value.
use super::*;
use crate::sql::{atomic, bind, error, Sources};
use kassiber_envelope::truthy;
use kassiber_store::rusqlite::params;

fn resolve(conn: &Connection, scope: &Scope<'_>, reference: &Value) -> Result<Value> {
    kassiber_transactions::lookup::resolve_transaction(conn, scope.profile_id, reference, None)
        .map(|row| value(&row, "id"))
}
pub(super) fn dismiss(
    conn: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    atomic(conn, || {
        let source = resolve(conn, scope, &value(args, "tx_out"))?;
        let target = resolve(conn, scope, &value(args, "tx_in"))?;
        let days = get(args, "expires_in_days").cloned().unwrap_or(90.into());
        let days = if truthy(&days) {
            integer(&days)?
        } else {
            0.into()
        };
        let now = sources.now(conn)?;
        let expires = if days > 0.into() {
            let days = days.to_string().parse::<i32>().map_err(|_| {
                AppError::new("internal_error", "Python int too large to convert to C int")
            })?;
            // datetime.timedelta rejects more than 999999999 days; datetime
            // addition may overflow even below that bound.
            if days > 999999999 {
                return Err(AppError::new(
                    "internal_error",
                    format!("days={days}; must have magnitude <= 999999999"),
                ));
            }
            let expiry: Option<String> = conn
                .query_row(
                    "SELECT strftime('%Y-%m-%dT%H:%M:%SZ',?,?)",
                    params![now, format!("+{days} days")],
                    |r| r.get(0),
                )
                .map_err(error)?;
            expiry
                .ok_or_else(|| AppError::new("internal_error", "date value out of range"))?
                .into()
        } else {
            None
        };
        conn.execute("INSERT INTO transaction_pair_dismissals(id,workspace_id,profile_id,out_transaction_id,in_transaction_id,reason,created_at,expires_at) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(profile_id,out_transaction_id,in_transaction_id) DO UPDATE SET reason=COALESCE(excluded.reason,transaction_pair_dismissals.reason),expires_at=excluded.expires_at",params![sources.uuid(conn)?,scope.workspace_id,scope.profile_id,bind(&source,4)?,bind(&target,5)?,bind(&value(args,"reason"),6)?,now,expires]).map_err(error)?;
        rows(conn,"SELECT id,workspace_id,profile_id,out_transaction_id,in_transaction_id,reason,created_at,expires_at FROM transaction_pair_dismissals WHERE profile_id=? AND out_transaction_id=? AND in_transaction_id=?",params![scope.profile_id,bind(&source,2)?,bind(&target,3)?])?.into_iter().next().ok_or_else(|| AppError::new("internal_error","Dismissal row was not stored"))
    })
}

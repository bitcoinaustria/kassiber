//! Dismissal is an advisory hint. It does not pair, own, or carry any value.
use super::*;
use crate::sql::{atomic, bind, error, Sources};
use kassiber_envelope::{s, set, truthy};
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
            // Python stamps `created_at` with `now_iso()` but counts the
            // expiry from `datetime.now()` itself, outside the clock seam
            // test references bind.
            let wall = super::sql::wall_clock(conn, sources)?;
            let expiry: Option<String> = conn
                .query_row(
                    "SELECT strftime('%Y-%m-%dT%H:%M:%SZ',?,?)",
                    params![wall, format!("+{days} days")],
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
        // Journal builds read dismissals (they stop native transitions), so
        // the book's journals are stale after one.
        super::component_store::invalidate(conn, scope.profile_id)?;
        rows(conn,"SELECT id,workspace_id,profile_id,out_transaction_id,in_transaction_id,reason,created_at,expires_at FROM transaction_pair_dismissals WHERE profile_id=? AND out_transaction_id=? AND in_transaction_id=?",params![scope.profile_id,bind(&source,2)?,bind(&target,3)?])?.into_iter().next().ok_or_else(|| AppError::new("internal_error","Dismissal row was not stored"))
    })
}

const DISMISSAL_LIST_MAX: i64 = 500;
const DISMISSAL_COLUMNS: [&str; 8] = [
    "id",
    "workspace_id",
    "profile_id",
    "out_transaction_id",
    "in_transaction_id",
    "reason",
    "created_at",
    "expires_at",
];

fn dismissal_leg(row: &Value, side: &str) -> Result<Value> {
    let field = |name: &str| value(row, &format!("{side}_{name}"));
    Ok(dict([
        ("transaction_id", field("transaction_id")),
        ("external_id", field("external_id")),
        ("wallet_id", field("wallet_id")),
        ("wallet", field("wallet")),
        ("direction", field("direction")),
        ("asset", field("asset")),
        (
            "amount_msat",
            kassiber_transactions::lookup::python_int(field("amount"))?,
        ),
        ("occurred_at", field("occurred_at")),
    ]))
}

/// `list_transfer_dismissals`: the "not a transfer" vetoes the matcher
/// applies, read back with full leg ids. Activity uses the matcher's own
/// expiry rule; expired rows are listed only on request.
pub(super) fn list(
    conn: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let limit = get(args, "limit")
        .cloned()
        .unwrap_or(Value::Int(200.into()));
    let limit = match &limit {
        Value::Int(n) => i64::try_from(n.clone())
            .ok()
            .filter(|n| (1..=DISMISSAL_LIST_MAX).contains(n)),
        _ => None,
    }
    .ok_or_else(|| {
        AppError::new(
            "validation",
            format!("Dismissal list limit must be an integer between 1 and {DISMISSAL_LIST_MAX}"),
        )
    })?;
    let include_expired = get(args, "include_expired").is_some_and(truthy);
    let found = rows(
        conn,
        "SELECT d.*, tout.external_id AS out_external_id, tout.wallet_id AS out_wallet_id, tout.direction AS out_direction, tout.asset AS out_asset, tout.amount AS out_amount, tout.occurred_at AS out_occurred_at, tin.external_id AS in_external_id, tin.wallet_id AS in_wallet_id, tin.direction AS in_direction, tin.asset AS in_asset, tin.amount AS in_amount, tin.occurred_at AS in_occurred_at, wout.label AS out_wallet, win.label AS in_wallet FROM transaction_pair_dismissals d LEFT JOIN transactions tout ON tout.id = d.out_transaction_id AND tout.profile_id = d.profile_id LEFT JOIN wallets wout ON wout.id = tout.wallet_id LEFT JOIN transactions tin ON tin.id = d.in_transaction_id AND tin.profile_id = d.profile_id LEFT JOIN wallets win ON win.id = tin.wallet_id WHERE d.profile_id = ? ORDER BY d.created_at DESC, d.id DESC",
        [scope.profile_id],
    )?;
    let now = sources.now(conn)?;
    let (mut active_count, mut expired_count, mut permanent_count) = (0usize, 0usize, 0usize);
    let mut dismissals = Vec::new();
    for row in &found {
        let expires_at = value(row, "expires_at");
        let active = pairing::dismissal_is_active(&expires_at, &now);
        if active {
            active_count += 1;
            if !truthy(&expires_at) {
                permanent_count += 1;
            }
        } else {
            expired_count += 1;
        }
        if !active && !include_expired {
            continue;
        }
        let mut item = project(row, &DISMISSAL_COLUMNS);
        set(&mut item, "active", Value::Bool(active));
        set(&mut item, "permanent", Value::Bool(!truthy(&expires_at)));
        set(&mut item, "out", dismissal_leg(row, "out")?);
        set(&mut item, "in", dismissal_leg(row, "in")?);
        dismissals.push(item);
    }
    let count = |n: usize| Value::Int(n.into());
    let has_more = dismissals.len() as i64 > limit;
    dismissals.truncate(limit as usize);
    Ok(dict([
        ("dismissals", Value::List(dismissals)),
        (
            "counts",
            dict([
                ("active", count(active_count)),
                ("expired", count(expired_count)),
                ("permanent", count(permanent_count)),
            ]),
        ),
        ("has_more", Value::Bool(has_more)),
        ("limit", Value::Int(limit.into())),
        ("include_expired", Value::Bool(include_expired)),
        ("as_of", s(now)),
    ]))
}

/// `str(value)` bound as SQL text; surrogates fail as in Python.
fn text_reference(reference: &Value) -> Result<String> {
    match reference {
        Value::Str(raw) => raw.as_str().map(str::to_owned).ok_or_else(|| {
            AppError::new(
                "internal_error",
                kassiber_pytext::utf8_error(raw).unwrap_or_default(),
            )
        }),
        other => Ok(py_str(other)),
    }
}

/// `delete_transfer_dismissal`: undo one dismissal, by its id or by both
/// legs, so the matcher may suggest that pair again. Journals read
/// dismissals, so they are stale afterwards.
pub(super) fn delete(conn: &Connection, scope: &Scope<'_>, args: &Value) -> Result<Value> {
    let id = value(args, "dismissal_id");
    let (out_ref, in_ref) = (value(args, "tx_out"), value(args, "tx_in"));
    atomic(conn, || {
        let found = if truthy(&id) {
            if truthy(&out_ref) || truthy(&in_ref) {
                return Err(AppError::new(
                    "validation",
                    "Pass a dismissal id or both legs, not both",
                ));
            }
            rows(
                conn,
                "SELECT * FROM transaction_pair_dismissals WHERE profile_id = ? AND id = ?",
                params![scope.profile_id, text_reference(&id)?],
            )?
        } else {
            if !truthy(&out_ref) || !truthy(&in_ref) {
                return Err(AppError::new(
                    "validation",
                    "Deleting a dismissal needs its id or both legs",
                )
                .with_hint("Read ids with `kassiber transfers dismissals list`."));
            }
            let source = resolve(conn, scope, &out_ref)?;
            let target = resolve(conn, scope, &in_ref)?;
            rows(
                conn,
                "SELECT * FROM transaction_pair_dismissals WHERE profile_id = ? AND out_transaction_id = ? AND in_transaction_id = ?",
                params![scope.profile_id, bind(&source, 2)?, bind(&target, 3)?],
            )?
        };
        let row = found
            .into_iter()
            .next()
            .ok_or_else(|| AppError::new("not_found", "Dismissal not found"))?;
        conn.execute(
            "DELETE FROM transaction_pair_dismissals WHERE id = ?",
            params![bind(&value(&row, "id"), 1)?],
        )
        .map_err(error)?;
        super::component_store::invalidate(conn, scope.profile_id)?;
        Ok(dict([
            ("deleted", value(&row, "id")),
            (
                "dismissal",
                project(
                    &row,
                    &[
                        "id",
                        "out_transaction_id",
                        "in_transaction_id",
                        "reason",
                        "created_at",
                        "expires_at",
                    ],
                ),
            ),
        ]))
    })
}

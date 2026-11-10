//! Python `core/loans.py` and the loan handlers; marks do not rewrite observations.
use std::collections::BTreeMap;

use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::Value;
use kassiber_pytext::strip;
use kassiber_store::rusqlite::{params, Connection};

use crate::sql::{atomic, error, invalidate, now, rows, string, uuid, Result, Scope};

pub const ROLES: &[&str] = &[
    "collateral_lock",
    "collateral_release",
    "loan_principal_received",
    "loan_principal_repaid",
];
pub const ROLE_LABELS: &[(&str, &str)] = &[
    (
        "collateral_lock",
        "BTC collateral posted for fiat loan (out)",
    ),
    ("collateral_release", "BTC collateral returned (in)"),
    (
        "loan_principal_received",
        "BTC loan principal received (in)",
    ),
    ("loan_principal_repaid", "BTC loan principal repaid (out)"),
    ("channel_open", "BTC funded into a Lightning channel (out)"),
    (
        "channel_close",
        "BTC returned from a Lightning channel close (in)",
    ),
];

pub enum Command {
    Mark {
        txid: String,
        mark_as: String,
        note: Value,
        loan_id: Option<String>,
    },
    Unmark {
        txid: String,
    },
    Link {
        txids: Vec<String>,
        loan_id: Option<String>,
    },
    List {
        include_roles: bool,
    },
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    get(value, key).and_then(string).unwrap_or_default()
}

fn resolve(conn: &Connection, profile_id: &str, txid: &str) -> Result<String> {
    // Preserve the reference loan handler's lookup, separate from the stricter
    // attachment/metadata transaction resolver.
    rows(
        conn,
        "SELECT id FROM transactions WHERE profile_id=? AND (id=? OR external_id=?)",
        [profile_id, txid, txid],
    )?
    .first()
    .map(|row| text(row, "id").to_owned())
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            format!("Transaction '{txid}' not found in this book"),
        )
        .with_details(dict([("txid", s(txid))]))
    })
}

fn validate_direction(role: &str, direction: &str) -> Result<()> {
    if !ROLES.contains(&role) {
        return Err(AppError::new(
            "validation",
            format!("Invalid loan role '{role}'. Allowed: {}", ROLES.join(", ")),
        )
        .with_details(dict([("field", s("role")), ("value", s(role))])));
    }
    let expected = match role {
        "collateral_lock" | "loan_principal_repaid" => "outbound",
        _ => "inbound",
    };
    if direction != expected {
        return Err(AppError::new(
            "validation",
            format!("Loan role '{role}' requires an {expected} transaction"),
        )
        .with_details(dict([
            ("field", s("role")),
            ("role", s(role)),
            ("expected_direction", s(expected)),
            ("actual_direction", s(direction)),
        ])));
    }
    Ok(())
}

pub fn execute(conn: &Connection, scope: &Scope<'_>, command: &Command) -> Result<Value> {
    match command {
        Command::List { include_roles } => {
            let marks = rows(conn, "SELECT ll.transaction_id, ll.loan_id, ll.role, ll.note, ll.created_at, t.direction, t.asset, t.amount, t.occurred_at, t.description FROM loan_legs ll JOIN transactions t ON t.id=ll.transaction_id WHERE ll.profile_id=? AND ll.deleted_at IS NULL ORDER BY t.occurred_at DESC, ll.created_at DESC", [scope.profile_id])?;
            let open = open_locks(&marks);
            let mut payload = vec![
                ("marks", Value::List(marks)),
                ("open_locks", Value::List(open)),
            ];
            if *include_roles {
                payload.push((
                    "roles",
                    Value::List(ROLES.iter().map(|role| s(*role)).collect()),
                ));
                payload.push((
                    "role_labels",
                    dict(ROLE_LABELS.iter().map(|(role, label)| (*role, s(*label)))),
                ));
            }
            Ok(dict(payload))
        }
        Command::Mark {
            txid,
            mark_as,
            note,
            loan_id,
        } => {
            let role = match mark_as.as_str() {
                "collateral" => "collateral_lock",
                "returned" => "collateral_release",
                "principal-received" => "loan_principal_received",
                "principal-repaid" => "loan_principal_repaid",
                _ => return Err(AppError::new("validation", format!("Invalid --as '{mark_as}'. Use one of: collateral, returned, principal-received, principal-repaid"))
                    .with_details(dict([("field", s("as")), ("value", s(mark_as.as_str()))]))),
            };
            let id = resolve(conn, scope.profile_id, txid)?;
            let direction: String = conn
                .query_row(
                    "SELECT direction FROM transactions WHERE id=? AND profile_id=?",
                    [id.as_str(), scope.profile_id],
                    |row| row.get(0),
                )
                .map_err(error)?;
            validate_direction(role, &direction)?;
            let existing = rows(conn, "SELECT loan_id FROM loan_legs WHERE profile_id=? AND transaction_id=? AND deleted_at IS NULL", [scope.profile_id, &id])?;
            let chosen = loan_id
                .as_deref()
                .map(strip)
                .filter(|x| !x.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    existing
                        .first()
                        .map(|r| text(r, "loan_id"))
                        .filter(|x| !x.is_empty())
                        .map(str::to_owned)
                });
            let leg_id = uuid(conn)?;
            let timestamp = now(conn)?;
            let note = match note {
                Value::Null => None,
                Value::Str(t) => Some(
                    t.as_str()
                        .ok_or_else(|| AppError::new("validation", "note is not valid UTF-8"))?,
                ),
                _ => return Err(AppError::new("validation", "note must be a string or null")),
            };
            atomic(conn, || {
                conn.execute("UPDATE loan_legs SET deleted_at=? WHERE profile_id=? AND transaction_id=? AND deleted_at IS NULL", [&timestamp, scope.profile_id, &id]).map_err(error)?;
                conn.execute("INSERT INTO loan_legs(id,workspace_id,profile_id,transaction_id,loan_id,role,note,deleted_at,created_at) VALUES(?,?,?,?,?,?,?,NULL,?)", params![leg_id, scope.workspace_id, scope.profile_id, id, chosen, role, note, timestamp]).map_err(error)?;
                invalidate(conn, scope.profile_id)?;
                Ok(())
            })?;
            Ok(rows(conn, "SELECT * FROM loan_legs WHERE id=?", [&leg_id])?.remove(0))
        }
        Command::Unmark { txid } => {
            let id = resolve(conn, scope.profile_id, txid)?;
            let existing = rows(conn, "SELECT id FROM loan_legs WHERE profile_id=? AND transaction_id=? AND deleted_at IS NULL", [scope.profile_id, &id])?;
            let Some(row) = existing.first() else {
                return Err(AppError::new(
                    "not_found",
                    format!("No loan mark on transaction '{id}'"),
                )
                .with_details(dict([("transaction_id", s(id))])));
            };
            atomic(conn, || {
                conn.execute(
                    "UPDATE loan_legs SET deleted_at=? WHERE id=?",
                    [now(conn)?.as_str(), text(row, "id")],
                )
                .map_err(error)?;
                invalidate(conn, scope.profile_id)
            })?;
            Ok(dict([("unmarked", s(id))]))
        }
        Command::Link { txids, loan_id } => {
            let mut unique = Vec::new();
            for txid in txids {
                let id = resolve(conn, scope.profile_id, txid)?;
                if !unique.contains(&id) {
                    unique.push(id);
                }
            }
            let ids = Value::List(unique.iter().map(|id| s(id.as_str())).collect());
            if unique.len() < 2 {
                return Err(AppError::new(
                    "validation",
                    "At least two marked loan transactions are required to link a loan",
                )
                .with_details(dict([("transaction_ids", ids)])));
            }
            let marks = rows(conn, "SELECT transaction_id,loan_id FROM loan_legs WHERE profile_id=? AND deleted_at IS NULL", [scope.profile_id])?;
            let missing: Vec<_> = unique
                .iter()
                .filter(|id| !marks.iter().any(|r| text(r, "transaction_id") == *id))
                .map(|id| s(id.as_str()))
                .collect();
            if !missing.is_empty() {
                return Err(AppError::new(
                    "not_found",
                    "All linked loan transactions must already have a loan mark",
                )
                .with_details(dict([
                    ("transaction_ids", ids),
                    ("missing", Value::List(missing)),
                ])));
            }
            let chosen = loan_id
                .as_deref()
                .map(strip)
                .filter(|x| !x.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    unique.iter().find_map(|id| {
                        marks
                            .iter()
                            .find(|r| text(r, "transaction_id") == id)
                            .map(|r| text(r, "loan_id"))
                            .filter(|x| !x.is_empty())
                            .map(str::to_owned)
                    })
                })
                .map_or_else(|| uuid(conn), Ok)?;
            atomic(conn, || {
                for id in &unique {
                    conn.execute("UPDATE loan_legs SET loan_id=? WHERE profile_id=? AND transaction_id=? AND deleted_at IS NULL", [&chosen, scope.profile_id, id]).map_err(error)?;
                }
                Ok(())
            })?;
            Ok(dict([("loan_id", s(chosen)), ("transaction_ids", ids)]))
        }
    }
}

fn open_locks(marks: &[Value]) -> Vec<Value> {
    let mut releases: BTreeMap<&str, usize> = BTreeMap::new();
    for mark in marks {
        if text(mark, "role") == "collateral_release" {
            *releases.entry(text(mark, "asset")).or_default() += 1;
        }
    }
    let mut ordered: Vec<_> = marks.iter().collect();
    ordered.sort_by_key(|m| (text(m, "occurred_at"), text(m, "created_at")));
    ordered
        .into_iter()
        .filter(|mark| {
            if text(mark, "role") != "collateral_lock" {
                return false;
            }
            let count = releases.entry(text(mark, "asset")).or_default();
            if *count > 0 {
                *count -= 1;
                false
            } else {
                true
            }
        })
        .cloned()
        .collect()
}

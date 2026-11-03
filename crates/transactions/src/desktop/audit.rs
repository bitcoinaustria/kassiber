//! `build_audit_changes_since_last_answer_snapshot`: counts of stored rows
//! created after an answer's baseline. Reading it computes nothing new.
use super::*;

/// Audit tables and their timestamp columns; no other table is read.
const PROFILE_TABLES: &[(&str, &str)] = &[
    ("transactions", "created_at"),
    ("journal_entries", "created_at"),
    ("journal_quarantines", "created_at"),
    ("transaction_edit_events", "changed_at"),
    ("wallets", "created_at"),
];

fn scalar(conn: &Connection, sql: &str, bind: Vec<SqlValue>) -> Result<Value> {
    let found = rows(conn, sql, params_from_iter(bind))?;
    Ok(found
        .first()
        .and_then(|row| match row {
            Value::Dict(pairs) => pairs.first().map(|(_, v)| v.clone()),
            _ => None,
        })
        .unwrap_or(Value::Null))
}
fn count(value: Value) -> i64 {
    match value {
        Value::Int(n) => i64::try_from(n).unwrap_or_default(),
        _ => 0,
    }
}
fn latest(value: Value) -> Value {
    if truthy(&value) {
        value
    } else {
        Value::Null
    }
}

pub(super) fn changes(conn: &Connection, scope: &Scope<'_>, args: Option<&Value>) -> Result<Value> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    if !matches!(args, Value::Dict(_)) {
        return Err(invalid("ui snapshot args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    unknown(args, "ui.audit.changes_since_last_answer", &["since"])?;
    let since = match get(args, "since") {
        None | Some(Value::Null) => None,
        Some(raw) => match crate::sql::string(raw).filter(|t| !strip(t).is_empty()) {
            Some(raw) => Some(time::timestamp(Some(raw), "since")?),
            None => return Err(invalid("since must be an RFC3339 timestamp")),
        },
    };
    let baseline = dict([("since", since.as_deref().map_or(Value::Null, s))]);
    let profile = if scope.workspace_id.is_empty() || scope.profile_id.is_empty() {
        None
    } else {
        rows(
            conn,
            "SELECT p.last_processed_at,w.label AS workspace_label,p.label AS profile_label,(SELECT COUNT(*) FROM transactions t WHERE t.profile_id=p.id AND t.excluded=0) AS active,(SELECT COUNT(*) FROM journal_quarantines q WHERE q.profile_id=p.id) AS quarantines FROM profiles p LEFT JOIN workspaces w ON w.id=? WHERE p.id=?",
            [scope.workspace_id, scope.profile_id],
        )?
        .into_iter()
        .next()
    };
    let Some(profile) = profile else {
        let status = if since.is_some() {
            "no_active_profile"
        } else {
            "baseline_required"
        };
        return Ok(dict([
            ("status", s(status)),
            (
                "changed",
                if since.is_some() {
                    Value::Bool(false)
                } else {
                    Value::Null
                },
            ),
            ("baseline", baseline),
            ("workspace", Value::Null),
            ("profile", Value::Null),
            ("counts_since", dict::<&str>([])),
            ("latest", dict::<&str>([])),
        ]));
    };
    let label = |key| latest(val(&profile, key));
    let processed = val(&profile, "last_processed_at");
    let current = dict([
        ("generated_at", s(crate::sql::now(conn)?)),
        (
            "active_transactions",
            Value::Int(num(&profile, "active").into()),
        ),
        ("journals_processed_at", processed.clone()),
        (
            "quarantines",
            Value::Int(num(&profile, "quarantines").into()),
        ),
    ]);
    let Some(since) = since else {
        let mut empty_latest: Vec<(&str, Value)> = PROFILE_TABLES
            .iter()
            .map(|(table, _)| (*table, Value::Null))
            .collect();
        empty_latest.push(("rates", Value::Null));
        empty_latest.push(("journals_processed_at", processed));
        return Ok(dict([
            ("status", s("baseline_required")),
            ("changed", Value::Null),
            (
                "baseline",
                dict([("since", Value::Null), ("required", Value::Bool(true))]),
            ),
            ("workspace", label("workspace_label")),
            ("profile", label("profile_label")),
            ("counts_since", dict::<&str>([])),
            ("latest", dict(empty_latest)),
            ("current", current),
        ]));
    };
    let mut counts = Vec::new();
    let mut newest = Vec::new();
    for (table, column) in PROFILE_TABLES {
        let predicate = time::predicate(conn, column, ">")?;
        counts.push((
            *table,
            count(scalar(
                conn,
                &format!(
                    "SELECT COUNT(*) AS count FROM {table} WHERE profile_id=? AND {predicate}"
                ),
                vec![
                    SqlValue::Text(scope.profile_id.into()),
                    SqlValue::Text(since.clone()),
                ],
            )?),
        ));
        newest.push((
            *table,
            latest(scalar(
                conn,
                &format!("SELECT MAX({column}) AS latest FROM {table} WHERE profile_id=?"),
                vec![SqlValue::Text(scope.profile_id.into())],
            )?),
        ));
    }
    let predicate = time::predicate(conn, "fetched_at", ">")?;
    counts.push((
        "rates",
        count(scalar(
            conn,
            &format!("SELECT COUNT(*) AS count FROM rates_cache WHERE {predicate}"),
            vec![SqlValue::Text(since.clone())],
        )?),
    ));
    newest.push((
        "rates",
        latest(scalar(
            conn,
            "SELECT MAX(fetched_at) AS latest FROM rates_cache",
            vec![],
        )?),
    ));
    newest.push(("journals_processed_at", processed));
    let changed = counts.iter().any(|(_, n)| *n > 0);
    Ok(dict([
        ("status", s("compared")),
        ("changed", Value::Bool(changed)),
        ("baseline", baseline),
        ("workspace", label("workspace_label")),
        ("profile", label("profile_label")),
        (
            "counts_since",
            dict(counts.into_iter().map(|(k, n)| (k, Value::Int(n.into())))),
        ),
        ("latest", dict(newest)),
        ("current", current),
    ]))
}

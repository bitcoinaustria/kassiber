//! `build_custody_lineage_snapshot` over `custody_decision_rows`: canonical
//! internal-custody edges with custody finality kept apart from basis state.
//! Observation commitments and slice offsets stay private.
use sha2::{Digest, Sha256};

use super::*;

const QUALIFICATION: &str = "Derived locally from canonical evidence in the active project and selected book. Custody finality is separate from tax-basis eligibility: a verified custody edge may remain basis-blocked by an earlier unresolved gap. State counters describe the returned rows when the result is truncated.";
const KEYS: &[&str] = &["decision_id", "occurred_at", "version"];

fn cursor_error() -> AppError {
    invalid("Invalid cursor").with_hint(
        "Pass the exact next_cursor value from the previous response; do not modify it or change filters.",
    )
}
fn label(value: Value) -> Value {
    if truthy(&value) {
        value
    } else {
        Value::Null
    }
}
fn or_unknown(row: &Value, key: &str) -> String {
    let value = val(row, key);
    if truthy(&value) {
        py_str(&value)
    } else {
        "unknown".into()
    }
}
fn joined(a: &str, b: &str, separator: &str) -> String {
    if a == b {
        a.into()
    } else {
        format!("{a}{separator}{b}")
    }
}

pub(super) fn snapshot(
    conn: &Connection,
    scope: &Scope<'_>,
    args: Option<&Value>,
) -> Result<Value> {
    let kind = "ui.custody.lineage.snapshot";
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    if !matches!(args, Value::Dict(_)) {
        return Err(invalid("ui snapshot args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    let Value::Dict(pairs) = args else {
        unreachable!()
    };
    let mut extra: Vec<_> = pairs
        .iter()
        .filter_map(|(key, _)| match key {
            Key::Str(t)
                if !["cursor", "limit", "transaction_id"]
                    .contains(&t.as_str().unwrap_or_default()) =>
            {
                Some(Value::Str(t.clone()))
            }
            _ => None,
        })
        .collect();
    extra.sort_by_key(py_str);
    if !extra.is_empty() {
        return Err(invalid(format!("{kind} received unsupported arguments"))
            .with_details(dict([("unknown", Value::List(extra))])));
    }
    let limit = limit(args, 100, 500)?;
    let cursor = match get(args, "cursor") {
        None | Some(Value::Null) => None,
        Some(raw) => match crate::sql::string(raw).filter(|t| !t.is_empty()) {
            Some(raw) => Some(raw.to_owned()),
            None => return Err(invalid(format!("{kind} cursor must be a non-empty string"))),
        },
    };
    let transaction = match get(args, "transaction_id") {
        None | Some(Value::Null) => None,
        Some(raw) => match crate::sql::string(raw).map(strip).filter(|t| !t.is_empty()) {
            Some(raw) => Some(raw.to_owned()),
            None => {
                return Err(invalid(format!(
                    "{kind} transaction_id must be a non-empty string"
                )))
            }
        },
    };
    let context = rows(
        conn,
        "SELECT (SELECT label FROM workspaces WHERE id=?) AS workspace_label,(SELECT label FROM profiles WHERE id=?) AS profile_label",
        [scope.workspace_id, scope.profile_id],
    )?
    .remove(0);
    let scope_payload = dict([
        ("workspace_id", nullable(scope.workspace_id)),
        ("workspace_label", label(val(&context, "workspace_label"))),
        ("profile_id", nullable(scope.profile_id)),
        ("profile_label", label(val(&context, "profile_label"))),
    ]);
    let summary =
        |total: i64, returned: i64, truncated: bool, counts: &BTreeMap<(bool, String), i64>| {
            let n = |custody: bool, state: &str| {
                Value::Int(
                    counts
                        .get(&(custody, state.into()))
                        .copied()
                        .unwrap_or(0)
                        .into(),
                )
            };
            dict([
                ("total_count", Value::Int(total.into())),
                ("returned_count", Value::Int(returned.into())),
                ("truncated", Value::Bool(truncated)),
                ("internal_verified", n(true, "internal_verified")),
                ("internal_reviewed", n(true, "internal_reviewed")),
                ("basis_eligible", n(false, "eligible")),
                ("basis_blocked", n(false, "blocked_by_prior_custody_basis")),
                ("qualification", s(QUALIFICATION)),
            ])
        };
    let profile_exists = !scope.workspace_id.is_empty()
        && !scope.profile_id.is_empty()
        && !rows(
            conn,
            "SELECT 1 FROM profiles WHERE id=?",
            [scope.profile_id],
        )?
        .is_empty();
    if !profile_exists {
        return Ok(dict([
            ("scope", scope_payload),
            ("items", Value::List(vec![])),
            ("next_cursor", Value::Null),
            ("summary", summary(0, 0, false, &BTreeMap::new())),
            ("observation_commitments_included", Value::Bool(false)),
            ("replicated", Value::Bool(false)),
        ]));
    }
    // `custody_decision_rows(conn, profile_id, limit=..., transaction_ids=..., cursor=...)`.
    let mut condition = "d.profile_id=?".to_owned();
    let mut bind = vec![SqlValue::Text(scope.profile_id.into())];
    let selected: Option<Vec<String>> = transaction.map(|id| vec![id]);
    if let Some(selected) = &selected {
        let terms = vec!["?"; selected.len()].join(",");
        condition.push_str(&format!(
            " AND (d.source_transaction_id IN ({terms}) OR d.target_transaction_id IN ({terms}))"
        ));
        for _ in 0..2 {
            bind.extend(selected.iter().cloned().map(SqlValue::Text));
        }
    }
    let scope_condition = condition.clone();
    let scope_bind = bind.clone();
    let hash: String = Sha256::digest(scope.profile_id.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let filters = dict([
        ("profile_scope_hash", s(hash)),
        (
            "transaction_ids",
            selected.as_ref().map_or(Value::Null, |ids| {
                Value::List(ids.iter().map(|i| s(i.as_str())).collect())
            }),
        ),
    ]);
    if let Some(raw) = cursor.as_deref() {
        let row = cursor::decode(Some(raw), &filters, &[], false).map_err(|_| cursor_error())?;
        let row = row.ok_or_else(cursor_error)?;
        let version_ok = cursor::equivalent(&val(&row, "version"), &Value::Int(1.into()));
        let decision = val(&row, "decision_id");
        let occurred = val(&row, "occurred_at");
        if !version_ok
            || !crate::sql::string(&decision).is_some_and(|d| !d.is_empty())
            || !matches!(occurred, Value::Null | Value::Str(_))
        {
            return Err(cursor_error());
        }
        if occurred == Value::Null {
            condition.push_str(" AND d.occurred_at IS NULL AND d.decision_id < ?");
            bind.push(SqlValue::Text(py_str(&decision)));
        } else {
            condition.push_str(" AND (d.occurred_at < ? OR d.occurred_at IS NULL OR (d.occurred_at = ? AND d.decision_id < ?))");
            bind.push(SqlValue::Text(py_str(&occurred)));
            bind.push(SqlValue::Text(py_str(&occurred)));
            bind.push(SqlValue::Text(py_str(&decision)));
        }
    }
    let total = rows(
        conn,
        &format!(
            "SELECT COUNT(*) AS count FROM journal_custody_decisions d WHERE {scope_condition}"
        ),
        params_from_iter(scope_bind),
    )?;
    let total = num(&total[0], "count");
    bind.push(SqlValue::Integer(limit + 1));
    let mut page = rows(
        conn,
        &format!("SELECT d.decision_id AS _cursor_decision_id,d.source_transaction_id,d.target_transaction_id,d.source_wallet_id,source_wallet.label AS source_wallet_label,d.target_wallet_id,target_wallet.label AS target_wallet_label,d.source_network,d.target_network,d.source_rail,d.target_rail,d.source_asset,d.target_asset,d.source_end_msat-d.source_start_msat AS amount_msat,d.state AS custody_state,d.basis_state,d.basis_barrier_at,d.reason,d.atomic_group_id,d.component_id,d.occurred_at FROM journal_custody_decisions d LEFT JOIN wallets source_wallet ON source_wallet.id=d.source_wallet_id LEFT JOIN wallets target_wallet ON target_wallet.id=d.target_wallet_id WHERE {condition} ORDER BY d.occurred_at DESC,d.decision_id DESC LIMIT ?"),
        params_from_iter(bind),
    )?;
    let more = page.len() > limit as usize;
    page.truncate(limit as usize);
    let next = match (more, page.last()) {
        (true, Some(last)) => cursor::encode(
            &dict([
                ("decision_id", val(last, "_cursor_decision_id")),
                ("occurred_at", val(last, "occurred_at")),
                ("version", Value::Int(1.into())),
            ]),
            &filters,
            KEYS,
        )?,
        _ => Value::Null,
    };
    let mut counts: BTreeMap<(bool, String), i64> = BTreeMap::new();
    let mut items = Vec::new();
    for record in &page {
        let custody = or_unknown(record, "custody_state");
        let basis = or_unknown(record, "basis_state");
        *counts.entry((true, custody.clone())).or_default() += 1;
        *counts.entry((false, basis.clone())).or_default() += 1;
        let asset = ["source_asset", "target_asset"]
            .into_iter()
            .map(|k| val(record, k))
            .find(truthy)
            .map(|v| py_str(&v))
            .unwrap_or_default();
        let (source_network, target_network) = (
            or_unknown(record, "source_network"),
            or_unknown(record, "target_network"),
        );
        let (source_rail, target_rail) = (
            or_unknown(record, "source_rail"),
            or_unknown(record, "target_rail"),
        );
        items.push(dict([
            ("out_transaction_id", val(record, "source_transaction_id")),
            ("in_transaction_id", val(record, "target_transaction_id")),
            ("occurred_at", val(record, "occurred_at")),
            ("asset", s(asset)),
            ("source_asset", val(record, "source_asset")),
            ("target_asset", val(record, "target_asset")),
            ("amount_msat", s(num(record, "amount_msat").to_string())),
            ("from_wallet_id", val(record, "source_wallet_id")),
            ("from_wallet_label", val(record, "source_wallet_label")),
            ("to_wallet_id", val(record, "target_wallet_id")),
            ("to_wallet_label", val(record, "target_wallet_label")),
            ("custody_state", s(custody)),
            ("basis_state", s(basis)),
            ("basis_barrier_at", val(record, "basis_barrier_at")),
            ("evidence_reason", val(record, "reason")),
            ("network", s(joined(&source_network, &target_network, "->"))),
            ("source_network", s(source_network)),
            ("target_network", s(target_network)),
            ("rail", s(joined(&source_rail, &target_rail, "->"))),
            ("source_rail", s(source_rail)),
            ("target_rail", s(target_rail)),
            ("atomic_bundle_id", val(record, "atomic_group_id")),
            ("component_id", val(record, "component_id")),
        ]));
    }
    let returned = items.len() as i64;
    Ok(dict([
        ("scope", scope_payload),
        ("items", Value::List(items)),
        ("next_cursor", next),
        ("summary", summary(total, returned, more, &counts)),
        ("observation_commitments_included", Value::Bool(false)),
        ("replicated", Value::Bool(false)),
    ]))
}

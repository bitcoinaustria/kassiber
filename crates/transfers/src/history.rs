use super::*;
use kassiber_envelope::{s, truthy};
use kassiber_store::rusqlite::params;

pub(super) fn list(conn: &Connection, scope: &Scope<'_>, args: &Value) -> Result<Value> {
    let limit = get(args, "limit")
        .cloned()
        .unwrap_or(Value::Int(100.into()));
    let limit = match &limit {
        Value::Int(n) => n.to_string().parse::<i64>().ok(),
        _ => None,
    }
    .filter(|n| (1..=500).contains(n))
    .ok_or_else(|| {
        AppError::new(
            "custody_gap_review_validation",
            "Custody review history limit must be between 1 and 500",
        )
    })?;
    let gap_id = text(args, "gap_id");
    let stored = rows(conn,"SELECT r.*,c.revision AS component_revision,(SELECT COUNT(*) FROM custody_filed_report_impacts i WHERE i.review_id=r.id) AS filed_report_impact_count FROM custody_gap_reviews r LEFT JOIN custody_components c ON c.id=r.component_id WHERE r.profile_id=? AND r.gap_id=? ORDER BY r.revision DESC,r.created_at DESC,r.id DESC LIMIT ?",params![scope.profile_id,gap_id,limit])?;
    let mut history = Vec::new();
    for row in stored.iter().rev() {
        let event = get(row, "event_kind")
            .filter(|v| truthy(v))
            .cloned()
            .unwrap_or_else(|| s("review_decision"));
        let status = if event == s("bridge_reopened") {
            "needs_review"
        } else if text(row, "action") == "resolved" {
            "resolved"
        } else {
            "dismissed"
        };
        let snapshot = parse_json(&text(row, "snapshot_json")).unwrap_or_else(|_| dict::<&str>([]));
        if !matches!(snapshot, Value::Dict(_)) {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "'{}' object has no attribute 'get'",
                    kassiber_envelope::type_name(&snapshot)
                ),
            ));
        }
        let residual = get(&snapshot, "residual_classification")
            .and_then(|v| {
                if matches!(v, Value::Dict(_)) {
                    get(v, "classification")
                } else {
                    None
                }
            })
            .filter(|v| truthy(v));
        let mut projected = project(row, &["revision"]);
        if let Value::Dict(fields) = &mut projected {
            fields.extend([
                ("event_kind".into(), event),
                ("status".into(), s(status)),
                ("component_id".into(), value(row, "component_id")),
                (
                    "component_revision".into(),
                    value(row, "component_revision"),
                ),
                ("authored_source".into(), value(row, "authored_source")),
                ("created_at".into(), value(row, "created_at")),
                ("reason".into(), value(row, "reason")),
                (
                    "retained_msat".into(),
                    Value::Int(integer(
                        &get(&snapshot, "retained_msat")
                            .filter(|v| truthy(v))
                            .cloned()
                            .unwrap_or(0.into()),
                    )?),
                ),
                (
                    "residual_msat".into(),
                    Value::Int(integer(
                        &get(&snapshot, "residual_msat")
                            .filter(|v| truthy(v))
                            .cloned()
                            .unwrap_or(0.into()),
                    )?),
                ),
                (
                    "residual_classification".into(),
                    residual.map(|v| s(py_str(v))).unwrap_or(Value::Null),
                ),
                (
                    "filed_report_impact_count".into(),
                    value(row, "filed_report_impact_count"),
                ),
            ]);
        }
        history.push(projected);
    }
    Ok(dict([
        ("gap_id", s(gap_id)),
        ("count", Value::Int(history.len().into())),
        ("history", Value::List(history)),
    ]))
}

use super::*;
fn object(v: &Value) -> Value {
    match v {
        Value::Str(t) => t
            .as_str()
            .and_then(|s| kassiber_pytext::loads(s).ok())
            .filter(|v| matches!(v, Value::Dict(_)))
            .unwrap_or_else(|| dict::<&str>([])),
        _ => dict::<&str>([]),
    }
}
pub fn list(conn: &Connection, profile: &str) -> Result<Vec<Value>> {
    rows(conn,"SELECT * FROM filed_report_snapshots WHERE profile_id=? ORDER BY period_start_year,period_end_year,created_at,id",[profile])?.iter().map(|row| {
        Ok(dict([
            ("id",value(row,"id").clone()),("workspace_id",value(row,"workspace_id").clone()),("profile_id",value(row,"profile_id").clone()),
            ("report_kind",value(row,"report_kind").clone()),("report_state",value(row,"report_state").clone()),
            ("period_start_year",value(row,"period_start_year").clone()),("period_end_year",value(row,"period_end_year").clone()),
            ("content_sha256",value(row,"content_sha256").clone()),("classification_summary",object(value(row,"classification_summary_json"))),
            ("gain_summary",object(value(row,"gain_summary_json"))),("report_scope",object(value(row,"report_scope_json"))),
            ("authored_source",value(row,"authored_source").clone()),("notes",value(row,"notes").clone()),("created_at",value(row,"created_at").clone()),
        ]))
    }).collect()
}

pub(super) fn get_snapshot(conn: &Connection, id: &str, profile: &str) -> Result<Value> {
    list(conn, profile)?
        .into_iter()
        .find(|row| text(row, "id") == id)
        .ok_or_else(|| AppError::new("not_found", "filed report snapshot was not found"))
}

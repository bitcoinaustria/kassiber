use super::*;

pub fn report(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    redacted: bool,
) -> Result<Value> {
    let profile_id = text(profile, "id");
    let mut payload = if redacted {
        kassiber_wallets::analysis_index::mirror::build_redacted(conn, &profile_id)?
    } else {
        kassiber_wallets::analysis_index::mirror::build_local(conn, &profile_id)?
    };
    replace(
        &mut payload,
        "scope",
        dict([
            ("workspace", s("active")),
            ("profile", s("active")),
            (
                "workspace_selected",
                Value::Bool(truthy(value(workspace, "id"))),
            ),
            (
                "profile_selected",
                Value::Bool(truthy(value(profile, "id"))),
            ),
        ]),
    );
    Ok(payload)
}

pub fn table(payload: &Value) -> Value {
    let mut rows = Vec::new();
    if let Value::List(findings) = value(payload, "findings") {
        for finding in findings {
            rows.push(dict([
                ("surface", s("finding")),
                ("severity", value(finding, "severity").clone()),
                ("item", value(finding, "code").clone()),
                ("evidence_level", value(finding, "authority").clone()),
                ("detail", value(finding, "detail").clone()),
            ]));
        }
    }
    if let Value::List(checks) = value(value(payload, "coverage"), "checks") {
        for check in checks {
            let mut detail = format!(
                "{}/{} evaluated",
                text(check, "evaluated"),
                text(check, "eligible")
            );
            if truthy(value(check, "reason")) {
                detail.push_str("; ");
                detail.push_str(&text(check, "reason"));
            }
            rows.push(dict([
                ("surface", s("coverage")),
                ("severity", s("info")),
                ("item", value(check, "code").clone()),
                ("evidence_level", value(check, "status").clone()),
                ("detail", s(detail)),
            ]));
        }
    }
    Value::List(rows)
}

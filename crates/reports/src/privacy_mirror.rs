use super::*;

struct IndexSources {
    identity: String,
    timestamp: String,
}
impl kassiber_wallets::publication::wallet::Sources for IndexSources {
    fn id(&mut self) -> String {
        self.identity.clone()
    }
    fn now(&mut self) -> String {
        self.timestamp.clone()
    }
}

pub(super) fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let mut index_sources = IndexSources {
        identity: sources.uuid(conn)?,
        timestamp: sources.now(conn)?,
    };
    execute_with_index(conn, command, args, &mut index_sources)
}

/// The mirror report, synchronizing the analysis index with `index`, the same
/// identity and clock source every other index read uses (Python draws them
/// all from `projection_store`'s `uuid4` and `now_iso`).
pub fn execute_with_index(
    conn: &Connection,
    command: &str,
    args: &Value,
    index: &mut dyn kassiber_wallets::publication::wallet::Sources,
) -> Result<Value> {
    conn.execute_batch("SAVEPOINT mirror_report")
        .map_err(sql_error)?;
    let result = (|| {
        let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
        kassiber_wallets::analysis_index::durable::synchronize_with_sources(
            conn,
            &text(&profile, "id"),
            false,
            index,
        )?;
        let report = report(
            conn,
            &workspace,
            &profile,
            command == "reports.privacy-mirror",
        )?;
        Ok(
            if truthy(value(args, "text")) || truthy(value(args, "csv")) {
                table(&report)
            } else {
                report
            },
        )
    })();
    match result {
        Ok(payload) => {
            conn.execute_batch("RELEASE mirror_report")
                .map_err(sql_error)?;
            Ok(payload)
        }
        Err(error) => {
            conn.execute_batch("ROLLBACK TO mirror_report; RELEASE mirror_report")
                .map_err(sql_error)?;
            Err(error)
        }
    }
}

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

use super::*;
use kassiber_setup::Sources;
use kassiber_store::rusqlite::params;
use sha2::{Digest, Sha256};

pub struct FrozenCase<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
    pub target_transaction_id: &'a str,
    pub target_amount_msat: i64,
    pub asset: &'a str,
    pub reveal_mode: &'a str,
    pub status: &'a str,
    pub snapshot: &'a Value,
    pub label: Option<&'a str>,
    pub target_external_id: Option<&'a str>,
    pub recipient_id: Option<&'a str>,
    pub recipient_label_snapshot: Option<&'a str>,
    pub recipient_kind_snapshot: Option<&'a str>,
    pub recipient_reveal_mode_snapshot: Option<&'a str>,
}

struct Write<'a> {
    conn: &'a Connection,
    owns_transaction: bool,
    committed: bool,
}

impl Drop for Write<'_> {
    fn drop(&mut self) {
        if !self.committed {
            if self.owns_transaction {
                let _ = self.conn.execute_batch("ROLLBACK");
            } else {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO source_funds_case_write; RELEASE source_funds_case_write",
                );
            }
        }
    }
}

pub fn save_snapshot(
    conn: &Connection,
    case: &FrozenCase<'_>,
    sources: &dyn Sources,
) -> Result<Value> {
    let case_id = sources.uuid(conn)?;
    let snapshot_id = sources.uuid(conn)?;
    let created_at = sources.now(conn)?;
    let snapshot = kassiber_envelope::json_ready(case.snapshot.clone());
    let stored =
        kassiber_pyjson::dumps(&snapshot, &kassiber_pyjson::Options::new().sort_keys(true))
            .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let compact = kassiber_pyjson::dumps(
        &snapshot,
        &kassiber_pyjson::Options::new().sort_keys(true).compact(),
    )
    .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let digest = format!("{:x}", Sha256::digest(compact.as_bytes()));
    let owns_transaction = conn.is_autocommit();
    kassiber_store::begin_write_savepoint(conn, "source_funds_case_write").map_err(sql_error)?;
    let mut write = Write {
        conn,
        owns_transaction,
        committed: false,
    };
    conn.execute(
        "INSERT INTO source_funds_cases(id,workspace_id,profile_id,target_transaction_id,target_external_id,target_amount,asset,label,reveal_mode,status,snapshot_hash,snapshot_json,created_at,updated_at,recipient_id,recipient_label_snapshot,recipient_kind_snapshot,recipient_reveal_mode_snapshot) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![case_id,case.workspace_id,case.profile_id,case.target_transaction_id,case.target_external_id,case.target_amount_msat,case.asset,case.label,case.reveal_mode,case.status,digest,stored,created_at,created_at,case.recipient_id,case.recipient_label_snapshot,case.recipient_kind_snapshot,case.recipient_reveal_mode_snapshot],
    ).map_err(sql_error)?;
    conn.execute(
        "INSERT INTO source_funds_snapshots(id,case_id,snapshot_hash,snapshot_json,created_at) VALUES(?,?,?,?,?)",
        params![snapshot_id,case_id,digest,stored,created_at],
    ).map_err(sql_error)?;
    conn.execute_batch("RELEASE source_funds_case_write")
        .map_err(sql_error)?;
    write.committed = true;
    Ok(dict([
        ("id", s(case_id)),
        ("snapshot_id", s(snapshot_id)),
        ("snapshot_hash", s(digest)),
        ("status", s(case.status)),
        ("created_at", s(created_at)),
    ]))
}

pub fn handles(command: &str, args: &Value) -> bool {
    command == "ui.source_funds.cases.save"
        || (command == "reports.source-funds" && truthy(value(args, "save_case")))
}

pub fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    if command.starts_with("ui.") {
        if !matches!(args, Value::Null | Value::Dict(_)) {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(kassiber_envelope::type_name(args)))])));
        }
        if !matches!(value(args,"target_transaction"),Value::Str(raw) if raw.as_str().is_some_and(|raw|!kassiber_pytext::strip(raw).is_empty()))
        {
            return Err(AppError::new(
                "validation",
                "ui.source_funds.cases.save requires args.target_transaction",
            ));
        }
        if !matches!(value(args, "case_label"), Value::Null | Value::Str(_)) {
            return Err(AppError::new(
                "validation",
                "ui.source_funds.cases.save case_label must be a string",
            ));
        }
        if get(args, "expected_review_fingerprint").is_some() {
            let (_, profile) = kassiber_setup::references::scope(conn, &dict::<&str>([]))?;
            return super::source_funds_review::save_reviewed_case(
                conn,
                &profile,
                args,
                label(args),
                sources,
            );
        }
    }
    crate::export::admitted(conn, || save_unreviewed(conn, command, args, sources))
}

fn label(args: &Value) -> Option<&str> {
    match value(args, "case_label") {
        Value::Str(raw) => raw.as_str(),
        _ => None,
    }
}

fn save_unreviewed(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let (workspace, profile, recipe) = super::source_funds_report::prepare(conn, command, args)?;
    let report = super::source_funds_report::build(conn, &workspace, &profile, &recipe, None)?;
    let report = save_report(conn, &workspace, &profile, report, label(args), sources)?;
    super::source_funds_report::render(command, args, report)
}

pub(super) fn save_report(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    mut report: Value,
    label: Option<&str>,
    sources: &dyn Sources,
) -> Result<Value> {
    let target_id = text(value(&report, "target"), "transaction_id");
    let target = super::source_funds_report::transaction(conn, &text(profile, "id"), &target_id)?
        .ok_or_else(|| AppError::new("internal_error", "transaction wallet missing"))?;
    let external = if truthy(value(&target, "external_id")) {
        text(&target, "external_id")
    } else {
        String::new()
    };
    let recipient = value(&report, "recipient");
    let optional = |key| {
        if matches!(value(recipient, key), Value::Null) {
            None
        } else {
            Some(text(recipient, key))
        }
    };
    let recipient_id = optional("id");
    let recipient_label = optional("label");
    let recipient_kind = optional("kind");
    let recipient_mode = optional("default_reveal_mode");
    let target_amount = i64::try_from(super::source_funds_projection::integer(value(
        value(&report, "allocations"),
        "target_amount_msat",
    ))?)
    .map_err(|_| {
        AppError::new(
            "internal_error",
            "Python int too large to convert to SQLite INTEGER",
        )
    })?;
    let receipt = save_snapshot(
        conn,
        &FrozenCase {
            workspace_id: &text(workspace, "id"),
            profile_id: &text(profile, "id"),
            target_transaction_id: &target_id,
            target_amount_msat: target_amount,
            asset: &text(value(&report, "allocations"), "asset"),
            reveal_mode: &text(&report, "reveal_mode"),
            status: if truthy(value(value(&report, "explain_gates"), "exportable")) {
                "exportable"
            } else {
                "blocked"
            },
            snapshot: &report,
            label,
            target_external_id: Some(&external),
            recipient_id: recipient_id.as_deref(),
            recipient_label_snapshot: recipient_label.as_deref(),
            recipient_kind_snapshot: recipient_kind.as_deref(),
            recipient_reveal_mode_snapshot: recipient_mode.as_deref(),
        },
        sources,
    )?;
    replace(&mut report, "case", receipt);
    Ok(report)
}

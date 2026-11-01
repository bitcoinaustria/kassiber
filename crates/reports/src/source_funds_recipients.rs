use super::*;
use kassiber_pytext::{lower, strip};
use kassiber_setup::Sources;
use kassiber_store::rusqlite::{params, ErrorCode};

const KINDS: &[&str] = &[
    "tax_authority",
    "exchange",
    "bank",
    "lawyer",
    "accountant",
    "other",
];
const REVEAL_MODES: &[&str] = &["labels_only", "minimal", "standard", "full"];

pub fn handles(command: &str) -> bool {
    matches!(
        command,
        "source-funds.recipients.create"
            | "source-funds.recipients.update"
            | "source-funds.recipients.delete"
            | "ui.source_funds.recipients.create"
            | "ui.source_funds.recipients.update"
            | "ui.source_funds.recipients.delete"
    )
}

fn normalize(raw: &str, kind: bool) -> Result<String> {
    let input = if !kind && raw.is_empty() {
        "standard"
    } else {
        raw
    };
    let normalized = lower(strip(input));
    let normalized = if kind {
        normalized.replace('-', "_")
    } else {
        normalized
    };
    let choices = if kind { KINDS } else { REVEAL_MODES };
    if !choices.contains(&normalized.as_str()) {
        let field = if kind {
            "recipient kind"
        } else {
            "reveal mode"
        };
        return Err(
            AppError::new("validation", format!("Unsupported {field} '{raw}'"))
                .with_hint(format!("Choose one of: {}", choices.join(", "))),
        );
    }
    Ok(normalized)
}

fn get_recipient(conn: &Connection, profile: &str, id: &str) -> Result<Value> {
    rows(
        conn,
        "SELECT * FROM source_funds_recipients WHERE profile_id=? AND id=?",
        [profile, id],
    )?
    .first()
    .map(super::source_funds_reads::recipient)
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            format!("Recipient '{id}' not found in this profile."),
        )
    })
}

pub fn resolve(conn: &Connection, profile: &str, reference: &str) -> Result<Value> {
    let reference = strip(reference);
    if reference.is_empty() {
        return Err(AppError::new("validation", "Recipient ref is required"));
    }
    if let Some(row) = rows(
        conn,
        "SELECT * FROM source_funds_recipients WHERE profile_id=? AND id=?",
        [profile, reference],
    )?
    .first()
    {
        return Ok(super::source_funds_reads::recipient(row));
    }
    rows(
        conn,
        "SELECT * FROM source_funds_recipients WHERE profile_id=? AND label=? AND active=1",
        [profile, reference],
    )?
    .first()
    .map(super::source_funds_reads::recipient)
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            format!("Recipient '{reference}' not found in this profile."),
        )
    })
}

fn write_error(conn: &Connection, error: kassiber_store::rusqlite::Error, label: &str) -> AppError {
    if error.sqlite_error_code() == Some(ErrorCode::ConstraintViolation)
        && rows(conn, "PRAGMA cipher_salt", []).is_ok_and(|rows| rows.is_empty())
    {
        AppError::new(
            "validation",
            format!("A recipient labelled '{label}' already exists in this profile."),
        )
    } else {
        sql_error(error)
    }
}

pub fn execute(
    conn: &Connection,
    command: &str,
    raw: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let desktop = command.starts_with("ui.");
    let empty = dict::<&str>([]);
    let args = if desktop {
        match raw {
            Value::Null => &empty,
            Value::Dict(_) => raw,
            _ => {
                return Err(AppError::new("validation", "daemon args must be an object")
                    .with_details(dict([("type", s(kassiber_envelope::type_name(raw)))])))
            }
        }
    } else {
        raw
    };
    let (workspace, profile) =
        kassiber_setup::references::scope(conn, if desktop { &empty } else { args })?;
    let profile = text(&profile, "id");
    crate::export::admitted(conn, || {
        if command.ends_with(".create") {
            let field = |key, fallback: &str| {
                let v = value(args, key);
                if !truthy(v) {
                    fallback.into()
                } else {
                    py_str(v)
                }
            };
            let label = field("label", "");
            let label = strip(&label);
            if label.is_empty() {
                return Err(AppError::new("validation", "Recipient label is required"));
            }
            let kind = normalize(&field("kind", ""), true)?;
            let reveal = normalize(&field("default_reveal_mode", "standard"), false)?;
            let notes = if matches!(value(args, "notes"), Value::Str(_)) {
                text(args, "notes")
            } else {
                String::new()
            };
            let notes = strip(&notes);
            let id = sources.uuid(conn)?;
            let now = sources.now(conn)?;
            conn.execute("INSERT INTO source_funds_recipients(id,workspace_id,profile_id,label,kind,default_reveal_mode,notes,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)",
                params![id,text(&workspace,"id"),profile,label,kind,reveal,(!notes.is_empty()).then_some(notes),now,now])
                .map_err(|e| write_error(conn, e, label))?;
            return get_recipient(conn, &profile, &id);
        }
        let reference = match value(args, "recipient") {
            Value::Str(raw) => raw.as_str().unwrap_or_default(),
            _ => "",
        };
        if desktop && strip(reference).is_empty() {
            return Err(AppError::new(
                "validation",
                format!("{command} requires args.recipient"),
            ));
        }
        let existing = resolve(conn, &profile, reference)?;
        let id = text(&existing, "id");
        if command.ends_with(".delete") {
            let now = sources.now(conn)?;
            conn.execute("UPDATE source_funds_recipients SET active=0,updated_at=? WHERE profile_id=? AND id=?", params![now,profile,id]).map_err(sql_error)?;
        } else {
            let field = |key| match value(args, key) {
                Value::Str(_) => strip(&text(args, key)).to_owned(),
                _ => text(&existing, key),
            };
            let label = field("label");
            if label.is_empty() {
                return Err(AppError::new(
                    "validation",
                    "Recipient label cannot be empty",
                ));
            }
            let kind = if matches!(value(args, "kind"), Value::Str(_)) {
                normalize(&text(args, "kind"), true)?
            } else {
                text(&existing, "kind")
            };
            let reveal = if matches!(value(args, "default_reveal_mode"), Value::Str(_)) {
                normalize(&text(args, "default_reveal_mode"), false)?
            } else {
                text(&existing, "default_reveal_mode")
            };
            let notes = field("notes");
            let now = sources.now(conn)?;
            conn.execute("UPDATE source_funds_recipients SET label=?,kind=?,default_reveal_mode=?,notes=?,updated_at=? WHERE profile_id=? AND id=?",
                params![label,kind,reveal,(!notes.is_empty()).then_some(notes),now,profile,id])
                .map_err(|e| write_error(conn, e, &label))?;
        }
        get_recipient(conn, &profile, &id)
    })
}

//! External evidence registration. Documents confer no accounting authority.
use crate::sql::{atomic, now, rows, string, uuid, Result, Scope};
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pydecimal::Decimal;
use kassiber_pyjson::Value;
use kassiber_pytext::{lower, strip, upper};
use kassiber_store::rusqlite::{params, types::Value as SqlValue, Connection};
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

pub const TYPES: &[&str] = &[
    "invoice",
    "receipt",
    "contract",
    "statement",
    "tax_report",
    "other",
];
pub enum Command {
    List { limit: i64 },
    Create(Value),
    Attach(Value),
    ImportReport(Value),
}

// The reference wraps unexpected SQLite/I/O exceptions as internal_error;
// explicit domain refusals and lock contention retain their own codes.
pub(crate) fn reference_failure(mut error: AppError) -> AppError {
    if matches!(
        error.code.as_str(),
        "database_error" | "filesystem_error" | "storage_io_error"
    ) {
        error.code = "internal_error".into();
    }
    error
}
pub(crate) fn error(error: kassiber_store::rusqlite::Error) -> AppError {
    if matches!(&error, kassiber_store::rusqlite::Error::SqliteFailure(code, _) if matches!(code.extended_code & 255, 5 | 6))
    {
        return crate::sql::error(error);
    }
    let message = match &error {
        kassiber_store::rusqlite::Error::SqliteFailure(_, Some(message)) => message.clone(),
        _ => error.to_string(),
    };
    AppError::new("internal_error", message)
}

pub(crate) fn value(v: &Value, key: &str) -> Value {
    get(v, key).cloned().unwrap_or(Value::Null)
}
pub(crate) fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    get(v, key).and_then(string).unwrap_or_default()
}
pub(crate) fn present(v: &Value) -> bool {
    !matches!(v, Value::Null) && v != &s("")
}
fn surrogate(v: &Value) -> bool {
    matches!(v, Value::Str(t) if t.has_surrogates())
}
fn storable(v: &Value) -> Result<()> {
    if surrogate(v) {
        return Err(AppError::new(
            "internal_error",
            "text cannot be encoded as UTF-8",
        ));
    }
    Ok(())
}
pub(crate) fn or_empty(v: &Value, key: &str) -> Value {
    let v = value(v, key);
    if truthy(&v) {
        v
    } else {
        s("")
    }
}
pub(crate) fn sql(v: &Value) -> Result<SqlValue> {
    Ok(match v {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Int(n) => SqlValue::Integer(i64::try_from(n.clone()).map_err(|_| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        })?),
        Value::Float(n) => SqlValue::Real(*n),
        Value::Str(t) => SqlValue::Text(
            t.as_str()
                .ok_or_else(|| AppError::new("internal_error", "text cannot be encoded as UTF-8"))?
                .into(),
        ),
        _ => {
            return Err(AppError::new(
                "internal_error",
                "unsupported SQLite parameter type",
            ))
        }
    })
}
pub(crate) fn choice(v: &Value, allowed: &[&str], label: &str) -> Result<String> {
    let raw = if truthy(v) { py_str(v) } else { String::new() };
    let normalized = lower(strip(&raw)).replace('-', "_");
    if surrogate(v) || !allowed.contains(&normalized.as_str()) {
        return Err(
            AppError::new("validation", format!("Unsupported {label} '{}'", py_str(v)))
                .with_hint(format!("Choose one of: {}.", allowed.join(", "))),
        );
    }
    Ok(normalized)
}
pub(crate) fn decimal(v: &Value) -> Result<Decimal> {
    let raw = if present(v) { py_str(v) } else { "0".into() };
    if surrogate(v) {
        return Err(AppError::new("app_error", "Invalid decimal value"));
    }
    Decimal::from_str(strip(&raw))
        .map_err(|_| AppError::new("app_error", format!("Invalid decimal value: {}", py_str(v))))
}
pub(crate) fn exact(v: &Value) -> Result<Option<String>> {
    if !present(v) {
        return Ok(None);
    }
    if surrogate(v) {
        return Err(AppError::new(
            "validation",
            "commercial price must be a decimal number",
        ));
    }
    kassiber_pydecimal::finite_price(&py_str(v))
        .map_err(|e| AppError::new("validation", e.message("commercial price")))?
        .to_fixed_string()
        .map(Some)
        .map_err(|_| {
            AppError::new(
                "validation",
                "commercial price is outside the supported decimal range",
            )
        })
}
pub(crate) fn timestamp(v: &Value) -> Result<Option<String>> {
    if !present(v) {
        return Ok(None);
    }
    if !truthy(v) {
        return Err(AppError::new("app_error", "Missing occurred_at/date value"));
    }
    let raw = py_str(v);
    let trimmed = strip(&raw);
    let expanded = if trimmed.chars().count() == 10 {
        format!("{trimmed}T00:00:00+00:00")
    } else {
        trimmed.into()
    };
    let us = (!surrogate(v))
        .then(|| crate::metadata::time::normalized(&expanded))
        .flatten()
        .ok_or_else(|| {
            AppError::new(
                "validation",
                format!("Invalid occurred_at/date value '{raw}'"),
            )
            .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z")
        })?;
    if !(0..315_537_897_600_000_000).contains(&us) {
        return Err(AppError::new("internal_error", "date value out of range"));
    }
    let normalized = crate::metadata::time::format_timestamp(us / 1_000_000 * 1_000_000)?;
    Ok((normalized != "1970-01-01T00:00:00Z").then_some(normalized))
}
fn expanded(path: &str) -> Result<PathBuf> {
    kassiber_paths::Context::from_env()
        .map_err(|e| AppError::new(e.code, e.message))?
        .expand_user(Path::new(path))
        .map_err(|e| AppError::new(e.code, e.message))
}
fn file_arg(args: &Value) -> Value {
    let a = value(args, "file_path");
    if truthy(&a) {
        a
    } else {
        value(args, "file")
    }
}
fn report_fields(args: &Value) -> Result<(PathBuf, Option<String>, String, String)> {
    let provider = value(args, "provider");
    storable(&provider)?;
    let provider = if matches!(provider, Value::Null) {
        String::new()
    } else {
        strip(&py_str(&provider)).to_owned()
    };
    if provider.is_empty() {
        return Err(AppError::new(
            "validation",
            "A prior report provider is required",
        ));
    }
    let file = file_arg(args);
    let raw = if truthy(&file) {
        py_str(&file)
    } else {
        String::new()
    };
    let path = expanded(&raw)?;
    if surrogate(&file) || !path.is_file() {
        return Err(AppError::new(
            "not_found",
            format!("Prior tax report '{}' not found", py_str(&file)),
        ));
    }
    let year = value(args, "tax_year");
    if surrogate(&year) {
        return Err(AppError::new("validation", "Tax year must be four digits"));
    }
    let year = present(&year).then(|| strip(&py_str(&year)).to_owned());
    if year
        .as_ref()
        .is_some_and(|s| !s.is_empty() && (s.chars().count() != 4 || !s.chars().all(digit)))
    {
        return Err(AppError::new("validation", "Tax year must be four digits"));
    }
    let label = value(args, "label");
    storable(&label)?;
    let label = if matches!(label, Value::Null) {
        String::new()
    } else {
        strip(&py_str(&label)).into()
    };
    let label = if label.is_empty() {
        [
            Some(provider.as_str()),
            year.as_deref().filter(|s| !s.is_empty()),
            Some("tax report"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
    } else {
        label
    };
    Ok((path, year, provider, label))
}
/// Python validates these inputs before it attempts to resolve book scope.
pub fn validate_before_scope(command: &Command) -> Result<()> {
    match command {
        Command::Attach(args) if truthy(&file_arg(args)) == truthy(&value(args, "url")) => Err(
            AppError::new("validation", "Provide exactly one of --file or --url"),
        ),
        Command::ImportReport(args) => report_fields(args).map(|_| ()).map_err(reference_failure),
        _ => Ok(()),
    }
}
pub fn execute(
    conn: &Connection,
    scope: &Scope<'_>,
    root: &Path,
    command: &Command,
) -> Result<Value> {
    validate_before_scope(command)?;
    let mut copied = Vec::new();
    let result = atomic(conn, || {
        match command {
        Command::List{limit} => Ok(Value::List(rows(conn,"SELECT * FROM external_documents WHERE profile_id=? ORDER BY created_at DESC, id DESC LIMIT ?",params![scope.profile_id,if *limit==0 {100} else {(*limit).clamp(1,1000)}])?.iter().map(payload).collect())),
        Command::Create(args) => create(conn,scope,args,&dict::<&str>([])),
        Command::Attach(args) => attach(conn,scope,root,args,&mut copied),
        Command::ImportReport(args) => {
            let (path,year,provider,label)=report_fields(args)?;
            let document=create(conn,scope,&dict([
                ("document_type",s("tax_report")),("label",s(label)),("issuer",s(provider.clone())),
                ("issued_at",year.as_ref().filter(|s| !s.is_empty()).map_or(Value::Null,|y|s(format!("{y}-12-31")))),
                ("notes",s("Imported prior tax report. Preserved as evidence only; Kassiber does not treat another calculator's totals or lot assignments as ledger facts.")),
            ]), &dict([("provider",s(provider)),("tax_year",year.as_deref().map_or(Value::Null,s))]))?;
            let attachment=attach(conn,scope,root,&dict([("document",value(&document,"id")),("file_path",s(path.to_string_lossy().as_ref())),("label",s(path.file_name().and_then(|s|s.to_str()).unwrap_or_default()))]),&mut copied)?;
            Ok(dict([("document",document),("attachment",attachment),("tax_year",year.map_or(Value::Null,s))]))
        }
    }
    });
    if result.is_err() {
        for path in copied {
            let _ = fs::remove_file(path);
        }
    }
    result.map_err(reference_failure)
}
fn create(conn: &Connection, scope: &Scope<'_>, args: &Value, raw: &Value) -> Result<Value> {
    let kind = choice(&value(args, "document_type"), TYPES, "document type")?;
    let external = value(args, "external_ref");
    if truthy(&external)
        && !rows(
            conn,
            "SELECT id FROM external_documents WHERE profile_id=? AND external_ref=?",
            params![scope.profile_id, sql(&external)?],
        )?
        .is_empty()
    {
        return Err(AppError::new(
            "conflict",
            format!(
                "External document reference '{}' already exists",
                py_str(&external)
            ),
        )
        .with_hint("Use the existing document id or choose a unique external reference."));
    }
    let id = uuid(conn)?;
    let now = now(conn)?;
    let issued = timestamp(&value(args, "issued_at"))?;
    let due = timestamp(&value(args, "due_at"))?;
    let currency = value(args, "fiat_currency");
    storable(&currency)?;
    let currency = truthy(&currency).then(|| upper(&py_str(&currency)));
    let fiat = exact(&value(args, "fiat_value"))?;
    let raw = kassiber_pyjson::dumps(
        raw,
        &kassiber_pyjson::Options {
            sort_keys: true,
            ..Default::default()
        },
    )
    .map_err(|e| AppError::new("validation", e.to_string()))?;
    conn.execute("INSERT INTO external_documents(id,workspace_id,profile_id,document_type,label,external_ref,issuer,counterparty,issued_at,due_at,fiat_currency,fiat_value_exact,notes,raw_json,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",params![id,scope.workspace_id,scope.profile_id,kind,sql(&value(args,"label"))?,sql(&external)?,sql(&value(args,"issuer"))?,sql(&value(args,"counterparty"))?,issued,due,currency,fiat,sql(&value(args,"notes"))?,raw,now,now]).map_err(error)?;
    get_document(conn, scope.profile_id, &id)
}
pub fn get_document(conn: &Connection, profile: &str, reference: &str) -> Result<Value> {
    let exact = rows(
        conn,
        "SELECT * FROM external_documents WHERE profile_id=? AND id=?",
        [profile, reference],
    )?;
    if let Some(row) = exact.first() {
        return Ok(payload(row));
    }
    let found=rows(conn,"SELECT * FROM external_documents WHERE profile_id=? AND (external_ref=? OR label=?) ORDER BY created_at DESC",[profile,reference,reference])?;
    if found.len() > 1 {
        return Err(AppError::new(
            "ambiguous",
            format!("External document reference '{reference}' is ambiguous"),
        )
        .with_hint("Use the document id instead of a label or external reference.")
        .with_details(dict([("matches", Value::Int(found.len().into()))])));
    }
    found.first().map(payload).ok_or_else(|| {
        AppError::new(
            "not_found",
            format!("External document '{reference}' not found"),
        )
    })
}
fn payload(row: &Value) -> Value {
    dict(
        [
            "id",
            "document_type",
            "label",
            "external_ref",
            "issuer",
            "counterparty",
            "issued_at",
            "due_at",
            "fiat_currency",
            "fiat_value_exact",
            "review_state",
            "notes",
            "created_at",
            "updated_at",
        ]
        .map(|key| {
            (
                key,
                if matches!(
                    key,
                    "id" | "document_type" | "label" | "review_state" | "created_at" | "updated_at"
                ) {
                    value(row, key)
                } else {
                    or_empty(row, key)
                },
            )
        }),
    )
}
fn attach(
    conn: &Connection,
    scope: &Scope<'_>,
    root: &Path,
    args: &Value,
    copied: &mut Vec<PathBuf>,
) -> Result<Value> {
    storable(&value(args, "document"))?;
    let document = get_document(conn, scope.profile_id, text(args, "document"))?;
    let id = uuid(conn)?;
    let now = now(conn)?;
    fs::create_dir_all(root).map_err(|e| AppError::new("filesystem_error", e.to_string()))?;
    let file = file_arg(args);
    let url = value(args, "url");
    let mut label = value(args, "label");
    let mut media = value(args, "media_type");
    let (kind, name, relative, source, size, sha) = if truthy(&file) {
        let file = string(&file).ok_or_else(|| {
            if surrogate(&file) {
                AppError::new("not_found", "Attachment file not found")
            } else {
                AppError::new("internal_error", "Attachment path must be a string")
            }
        })?;
        let path = expanded(file)?;
        if !path.is_file() {
            return Err(AppError::new(
                "not_found",
                format!("Attachment file '{file}' not found"),
            ));
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let (destination, relative) =
            crate::attachments::storage(root, scope.profile_id, &id, name)?;
        copied.push(destination.clone());
        let (size, sha) = crate::attachments::hash(&path, Some(&destination))?;
        if !truthy(&media) {
            media = s(crate::mime::guess(name));
        }
        if !truthy(&label) {
            label = s(name);
        }
        (
            "file",
            Some(name.to_owned()),
            Some(relative),
            None,
            Some(size),
            Some(sha),
        )
    } else {
        storable(&url)?;
        let source = py_str(&url);
        if !truthy(&label) {
            label = s(source.as_str());
        }
        if !truthy(&media) {
            media = s("text/uri-list");
        }
        ("url", None, None, Some(source), None, None)
    };
    conn.execute("INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,original_filename,stored_relpath,source_url,media_type,size_bytes,sha256,created_at) VALUES(?,?,?,NULL,?,?,?,?,?,?,?,?,?)",params![id,scope.workspace_id,scope.profile_id,kind,sql(&label)?,name,relative,source,sql(&media)?,size,sha,now]).map_err(error)?;
    conn.execute("INSERT INTO external_document_attachments(document_id,attachment_id,created_at) VALUES(?,?,?)",params![text(&document,"id"),id,now]).map_err(error)?;
    Ok(dict([
        ("document_id", value(&document, "id")),
        ("attachment_id", s(id)),
        ("label", label),
    ]))
}

// CPython 3.11 Unicode 14 digit characters outside decimal (Nd).
fn digit(c: char) -> bool {
    kassiber_pytext::decimal_value(c as u32).is_some()
        || matches!(c as u32, 0xb2..=0xb3 | 0xb9 | 0x1369..=0x1371 | 0x19da | 0x2070 | 0x2074..=0x2079 | 0x2080..=0x2089 | 0x2460..=0x2468 | 0x2474..=0x247c | 0x2488..=0x2490 | 0x24ea | 0x24f5..=0x24fd | 0x24ff | 0x2776..=0x277e | 0x2780..=0x2788 | 0x278a..=0x2792 | 0x10a40..=0x10a43 | 0x10e60..=0x10e68 | 0x11052..=0x1105a | 0x1f100..=0x1f10a)
}

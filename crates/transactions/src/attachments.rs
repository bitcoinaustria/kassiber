//! Managed local evidence. URL references are never dereferenced.
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use fancy_regex::Regex;
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_pytext::{is_space, strip};
use kassiber_store::rusqlite::{params, Connection, Params};
use sha2::{Digest, Sha256};

use crate::sql::{atomic, error, now, rows, string, uuid, Result, Scope};

pub enum Command {
    Add {
        transaction: String,
        file: Option<String>,
        url: Option<String>,
        label: Option<String>,
        media_type: Option<String>,
    },
    List {
        transaction: Option<String>,
        limit: Option<i64>,
        offset: i64,
    },
    Rename {
        attachment: String,
        label: String,
    },
    Remove {
        attachment: String,
    },
    Verify {
        transaction: Option<String>,
    },
    Gc {
        dry_run: bool,
    },
    Copy {
        transaction: String,
        source_transaction: String,
        attachments: Vec<Value>,
    },
    Open {
        attachment: String,
    },
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    get(v, key).and_then(string).unwrap_or_default()
}
fn value(v: &Value, key: &str) -> Value {
    get(v, key).cloned().unwrap_or(Value::Null)
}
fn set(v: &mut Value, key: &str, value: Value) {
    if let Value::Dict(pairs) = v {
        if let Some((_, old)) = pairs.iter_mut().find(|(k, _)| *k == Key::from(key)) {
            *old = value;
        } else {
            pairs.push((Key::from(key), value));
        }
    }
}
fn io(error: std::io::Error) -> AppError {
    AppError::new("filesystem_error", error.to_string())
}
fn clean(label: &str) -> String {
    label
        .split(|c| is_space(c as u32))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
fn label(raw: Option<&str>, required: bool) -> Result<Option<String>> {
    let label = clean(raw.unwrap_or_default());
    if required && label.is_empty() {
        return Err(AppError::new(
            "validation",
            "Attachment label cannot be empty",
        ));
    }
    if label.chars().count() > 200 {
        return Err(AppError::new(
            "validation",
            "Attachment label must be 200 characters or fewer",
        )
        .with_details(dict([("max_length", Value::Int(200.into()))])));
    }
    Ok((!label.is_empty()).then_some(label))
}

fn filename(name: &str) -> String {
    static WORD: OnceLock<Regex> = OnceLock::new();
    let word =
        WORD.get_or_init(|| Regex::new(kassiber_pytext::REGEX_WORD).expect("CPython word class"));
    let name = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let sanitized: String = strip(name)
        .chars()
        .map(|c| {
            if matches!(c, '.' | '_' | '-') || word.is_match(&c.to_string()).unwrap_or(false) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = sanitized.trim_matches(['.', '_']);
    if name.is_empty() {
        "attachment.bin".into()
    } else {
        name.into()
    }
}

pub(crate) fn storage(
    root: &Path,
    profile: &str,
    id: &str,
    name: &str,
) -> Result<(PathBuf, String)> {
    let dir = root.join(profile);
    fs::create_dir_all(&dir).map_err(io)?;
    let relative = format!("{profile}/{id}-{}", filename(name));
    Ok((root.join(&relative), relative))
}

/// `_resolve_stored_path`: a managed file inside the attachments root, or
/// `(None, false)` for an absolute or escaping stored path.
pub fn stored_path(root: &Path, relative: &str) -> (Option<PathBuf>, bool) {
    stored(root, relative)
}

/// `attachment_display_label(row)`.
pub fn display_label(row: &Value) -> String {
    let explicit = clean(text(row, "label"));
    if !explicit.is_empty() {
        explicit
    } else if text(row, "attachment_type") == "url" {
        url_display_label(text(row, "source_url"))
    } else {
        let name = clean(text(row, "original_filename"));
        if name.is_empty() {
            "File attachment".into()
        } else {
            name
        }
    }
}

fn stored(root: &Path, relative: &str) -> (Option<PathBuf>, bool) {
    let relative = strip(relative);
    if relative.is_empty() {
        return (None, true);
    }
    if Path::new(relative).is_absolute() {
        return (None, false);
    }
    let Ok(root) = kassiber_paths::canonicalize_allow_missing(root) else {
        return (None, false);
    };
    let Ok(path) = kassiber_paths::canonicalize_allow_missing(&root.join(relative)) else {
        return (None, false);
    };
    if !path.starts_with(&root) {
        return (None, false);
    }
    (Some(path), true)
}

pub(crate) fn hash(path: &Path, destination: Option<&Path>) -> Result<(i64, String)> {
    let mut input = fs::File::open(path).map_err(io)?;
    let mut output = destination.map(fs::File::create).transpose().map_err(io)?;
    let mut digest = Sha256::new();
    let mut bytes = vec![0; 1024 * 1024];
    let mut size = 0;
    loop {
        let n = input.read(&mut bytes).map_err(io)?;
        if n == 0 {
            break;
        }
        if let Some(file) = output.as_mut() {
            file.write_all(&bytes[..n]).map_err(io)?;
        }
        digest.update(&bytes[..n]);
        size += n as i64;
    }
    Ok((
        size,
        digest
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    ))
}

// Parse the presentation parts only, retaining the exact stored URL. No URL
// client or network library is linked to this service.
fn url_parts(raw: &str) -> (String, String, String) {
    let raw = raw
        .trim_start_matches(|c: char| c as u32 <= 32)
        .replace(['\t', '\r', '\n'], "");
    let Some((scheme, rest)) = raw.split_once(':') else {
        return (String::new(), String::new(), raw);
    };
    if scheme.is_empty()
        || !scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return (String::new(), String::new(), raw);
    }
    let scheme = scheme.to_ascii_lowercase();
    let Some(rest) = rest.strip_prefix("//") else {
        return (scheme, String::new(), rest.into());
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = if let Some(v) = host.strip_prefix('[') {
        v.split(']').next().unwrap_or_default()
    } else {
        host.split(':').next().unwrap_or_default()
    };
    let host = if host.is_empty() {
        authority.to_owned()
    } else {
        host.to_lowercase()
    };
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    let path = rest[end..].split(['?', '#']).next().unwrap_or_default();
    let last_segment = path.rfind('/').map_or(0, |index| index + 1);
    let path_end = path[last_segment..]
        .find(';')
        .map_or(path.len(), |index| last_segment + index);
    let path = path[..path_end].to_owned();
    (scheme, host, path)
}

fn validate_literal_url(url: &str) -> Result<()> {
    // urllib.urlparse refuses malformed bracketed hosts and authorities whose
    // NFKC normalization introduces URL delimiters before storing a reference.
    // These are the Unicode 14 / CPython 3.11 non-ASCII characters that normalize
    // to a string containing '/', '?', '#', '@', or ':'. No normalization is
    // applied to the stored literal reference.
    if let Some((_, rest)) = url.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        if authority.contains(|c: char| "⁇⁈⁉℀℁℅℆⩴︓︖﹕﹖﹟﹫＃／：？＠".contains(c))
        {
            return Err(AppError::new("validation", "Invalid URL authority"));
        }
        if authority.contains(['[', ']']) {
            let host = authority.rsplit('@').next().unwrap_or_default();
            let valid = host
                .strip_prefix('[')
                .and_then(|host| host.split_once(']'))
                .is_some_and(|(host, suffix)| {
                    let address = host.split('%').next().unwrap_or_default();
                    let ipv_future = address
                        .strip_prefix('v')
                        .and_then(|v| v.split_once('.'))
                        .is_some_and(|(version, address)| {
                            !version.is_empty()
                                && version.chars().all(|c| c.is_ascii_hexdigit())
                                && !address.is_empty()
                        });
                    (suffix.is_empty() || suffix.starts_with(':'))
                        && (address.parse::<std::net::Ipv6Addr>().is_ok() || ipv_future)
                });
            if !valid {
                return Err(AppError::new("validation", "Invalid bracketed URL host"));
            }
        }
    }
    Ok(())
}

fn unquote(text: &str) -> String {
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if text.as_bytes()[i] == b'%' && i + 2 < text.len() {
            let hex = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
            if let (Some(a), Some(b)) = (hex(text.as_bytes()[i + 1]), hex(text.as_bytes()[i + 2])) {
                let n = a * 16 + b;
                bytes.push(n);
                i += 3;
                continue;
            }
        }
        bytes.push(text.as_bytes()[i]);
        i += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn url_display_label(raw: &str) -> String {
    let (scheme, host, path) = url_parts(&clean(raw));
    if !matches!(scheme.as_str(), "http" | "https") {
        return "Link attachment".into();
    }
    if matches!(host.as_str(), "docs.google.com" | "drive.google.com") {
        for (route, label) in [
            ("/document/d/", "Google Doc"),
            ("/spreadsheets/d/", "Google Sheet"),
            ("/presentation/d/", "Google Slides deck"),
            ("/forms/d/", "Google Form"),
            ("/drawings/d/", "Google Drawing"),
            ("/file/d/", "Google Drive file"),
            ("/drive/folders/", "Google Drive folder"),
        ] {
            if path.starts_with(route) {
                return label.into();
            }
        }
        return if host == "drive.google.com" {
            "Google Drive link"
        } else {
            "Google Workspace link"
        }
        .into();
    }
    let last = path
        .split('/')
        .map(strip)
        .rfind(|x| !x.is_empty())
        .unwrap_or_default();
    if !last.is_empty()
        && !(last.chars().count() >= 16 && last.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
    {
        let decoded = unquote(last);
        let title = if let Some((prefix, suffix)) = decoded.rsplit_once('.') {
            if (2..=5).contains(&suffix.chars().count())
                && suffix
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, 'İ' | 'ı' | 'ſ' | 'K'))
            {
                prefix
            } else {
                &decoded
            }
        } else {
            &decoded
        };
        let title = clean(&title.replace(['-', '_', '+'], " "));
        if !title.is_empty() && title.to_lowercase() != host.to_lowercase() {
            return format!("{host} - {title}");
        }
    }
    if host.is_empty() {
        "Link attachment".into()
    } else {
        host
    }
}

const SELECT: &str="SELECT a.*,t.external_id,t.occurred_at,t.asset,w.label AS wallet FROM attachments a LEFT JOIN transactions t ON t.id=a.transaction_id LEFT JOIN wallets w ON w.id=t.wallet_id";
// Wallet projection seam for joining the parallel wallet slice. This is the
// service's only wallet lookup; it exposes a display label, never config.
fn attachment_rows(conn: &Connection, suffix: &str, params: impl Params) -> Result<Vec<Value>> {
    rows(conn, &format!("{SELECT} {suffix}"), params)
}
fn selected(conn: &Connection, profile: &str, id: &str) -> Result<Value> {
    attachment_rows(conn, "WHERE a.profile_id=? AND a.id=?", [profile, id])?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::new("not_found", format!("Attachment '{id}' not found")))
}
fn payload(row: &Value, root: &Path) -> Value {
    let relative = text(row, "stored_relpath");
    let (path, valid) = stored(root, relative);
    let exists =
        path.map(|p| Value::Bool(p.exists()))
            .unwrap_or(if !relative.is_empty() && !valid {
                Value::Bool(false)
            } else {
                Value::Null
            });
    let display = display_label(row);
    dict([
        ("id", value(row, "id")),
        ("transaction_id", value(row, "transaction_id")),
        ("external_id", s(text(row, "external_id"))),
        ("wallet", s(text(row, "wallet"))),
        ("occurred_at", s(text(row, "occurred_at"))),
        ("asset", s(text(row, "asset"))),
        ("attachment_type", value(row, "attachment_type")),
        ("label", value(row, "label")),
        ("display_label", s(display)),
        ("original_filename", s(text(row, "original_filename"))),
        ("url", s(text(row, "source_url"))),
        ("media_type", s(text(row, "media_type"))),
        ("size_bytes", value(row, "size_bytes")),
        ("sha256", s(text(row, "sha256"))),
        ("stored_relpath", s(relative)),
        (
            "copied_from_attachment_id",
            s(text(row, "copied_from_attachment_id")),
        ),
        (
            "copied_from_transaction_id",
            s(text(row, "copied_from_transaction_id")),
        ),
        ("exists", exists),
        ("created_at", value(row, "created_at")),
    ])
}

pub(crate) fn resolve_transaction(
    conn: &Connection,
    profile: &str,
    reference: &str,
) -> Result<String> {
    let exact = rows(
        conn,
        "SELECT id FROM transactions WHERE profile_id=? AND id=?",
        [profile, reference],
    )?;
    if let Some(row) = exact.first() {
        return Ok(text(row, "id").into());
    }
    let found=rows(conn,"SELECT id FROM transactions WHERE profile_id=? AND external_id=? ORDER BY occurred_at DESC,created_at DESC,id DESC LIMIT 2",[profile,reference])?;
    if found.len() > 1 {
        return Err(AppError::new("ambiguous_reference",format!("Transaction external_id '{reference}' matches multiple transactions")).with_hint("Use the Kassiber transaction id from `transactions list` or narrow the command to an unambiguous row."));
    }
    found
        .first()
        .map(|r| text(r, "id").to_owned())
        .ok_or_else(|| AppError::new("app_error", format!("Transaction '{reference}' not found")))
}

fn list(
    conn: &Connection,
    scope: &Scope<'_>,
    root: &Path,
    transaction: Option<&str>,
    limit: Option<i64>,
    offset: i64,
) -> Result<Vec<Value>> {
    let tx = transaction
        .filter(|t| !t.is_empty())
        .map(|tx| resolve_transaction(conn, scope.profile_id, tx))
        .transpose()?;
    let sql="WHERE a.profile_id=? AND (? IS NULL OR a.transaction_id=?) ORDER BY a.created_at DESC,a.id DESC LIMIT ? OFFSET ?";
    let found = attachment_rows(
        conn,
        sql,
        params![
            scope.profile_id,
            tx,
            tx,
            limit.unwrap_or(-1),
            if limit.is_some() { offset } else { 0 }
        ],
    )?;
    fs::create_dir_all(root).map_err(io)?;
    Ok(found.iter().map(|r| payload(r, root)).collect())
}

fn prune(root: &Path, file: &Path) {
    let mut current = file.parent();
    while let Some(dir) = current {
        if dir == root || !dir.starts_with(root) || fs::remove_dir(dir).is_err() {
            break;
        }
        current = dir.parent();
    }
}

pub fn execute(
    conn: &Connection,
    scope: Option<&Scope<'_>>,
    root: &Path,
    command: &Command,
) -> Result<Value> {
    if let Command::Gc { dry_run } = command {
        return gc(conn, root, *dry_run);
    }
    let scope = scope.ok_or_else(|| AppError::new("internal_error", "missing attachment scope"))?;
    match command {
        Command::List {
            transaction,
            limit,
            offset,
        } => Ok(Value::List(list(
            conn,
            scope,
            root,
            transaction.as_deref(),
            *limit,
            *offset,
        )?)),
        Command::Add {
            transaction,
            file,
            url,
            label: raw_label,
            media_type,
        } => {
            let file = file.as_deref().filter(|t| !t.is_empty());
            let url = url.as_deref().filter(|t| !t.is_empty());
            if file.is_some() == url.is_some() {
                return Err(AppError::new(
                    "validation",
                    "Provide exactly one of --file or --url",
                ));
            }
            let tx = resolve_transaction(conn, scope.profile_id, transaction)?;
            let id = uuid(conn)?;
            let created_at = now(conn)?;
            fs::create_dir_all(root).map_err(io)?;
            let mut destination = None;
            let (kind, label, name, relative, source_url, media, size, digest) =
                if let Some(file) = file {
                    let source = kassiber_paths::Context::from_env()
                        .map_err(|e| AppError::new(e.code, e.message))?
                        .expand_user(Path::new(file))
                        .map_err(|e| AppError::new(e.code, e.message))?;
                    if !source.exists() {
                        return Err(AppError::new(
                            "not_found",
                            format!("Attachment file '{file}' not found"),
                        ));
                    }
                    if !source.is_file() {
                        return Err(AppError::new(
                            "validation",
                            format!("Attachment path '{file}' is not a file"),
                        ));
                    }
                    let name = source
                        .file_name()
                        .and_then(|v| v.to_str())
                        .unwrap_or_default()
                        .to_owned();
                    let label = label(
                        raw_label
                            .as_deref()
                            .filter(|t| !t.is_empty())
                            .or(Some(&name)),
                        true,
                    )?;
                    let (path, relative) = storage(root, scope.profile_id, &id, &name)?;
                    let result = hash(&source, Some(&path));
                    if result.is_err() {
                        let _ = fs::remove_file(&path);
                    }
                    let (size, digest) = result?;
                    destination = Some(path);
                    let media = media_type
                        .as_deref()
                        .filter(|t| !t.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| crate::mime::guess(&name));
                    (
                        "file",
                        label,
                        Some(name),
                        Some(relative),
                        None,
                        media,
                        Some(size),
                        Some(digest),
                    )
                } else {
                    let url = clean(url.unwrap_or_default());
                    validate_literal_url(&url)?;
                    if url_parts(&url).0.is_empty() {
                        return Err(AppError::new(
                            "validation",
                            "--url must include a scheme such as https://",
                        ));
                    }
                    (
                        "url",
                        label(raw_label.as_deref(), false)?,
                        None,
                        None,
                        Some(url),
                        media_type
                            .as_deref()
                            .filter(|t| !t.is_empty())
                            .unwrap_or("text/uri-list")
                            .to_owned(),
                        None,
                        None,
                    )
                };
            let outcome = atomic(conn, || {
                conn.execute("INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,original_filename,stored_relpath,source_url,media_type,size_bytes,sha256,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",params![id,scope.workspace_id,scope.profile_id,tx,kind,label,name,relative,source_url,media,size,digest,created_at]).map_err(error)?;
                Ok(())
            });
            if outcome.is_err() {
                if let Some(path) = destination {
                    let _ = fs::remove_file(path);
                }
            }
            outcome?;
            Ok(payload(&selected(conn, scope.profile_id, &id)?, root))
        }
        Command::Rename {
            attachment,
            label: raw_label,
        } => {
            let label = label(Some(raw_label), true)?;
            let row = selected(conn, scope.profile_id, attachment)?;
            if text(&row, "attachment_type") != "url" {
                return Err(AppError::new(
                    "validation",
                    "Only URL attachment link text can be renamed",
                ));
            }
            atomic(conn, || {
                conn.execute(
                    "UPDATE attachments SET label=? WHERE profile_id=? AND id=?",
                    params![label, scope.profile_id, attachment],
                )
                .map_err(error)?;
                Ok(())
            })?;
            fs::create_dir_all(root).map_err(io)?;
            Ok(payload(
                &selected(conn, scope.profile_id, attachment)?,
                root,
            ))
        }
        Command::Remove { attachment } => {
            let row = selected(conn, scope.profile_id, attachment)?;
            fs::create_dir_all(root).map_err(io)?;
            let mut result = payload(&row, root);
            let (path, _) = stored(root, text(&row, "stored_relpath"));
            let mut deleted = false;
            atomic(conn, || {
                conn.execute("DELETE FROM attachments WHERE id=?", [attachment])
                    .map_err(error)?;
                if let Some(path) = path.as_ref().filter(|p| p.exists()) {
                    fs::remove_file(path).map_err(|e| {
                        AppError::new(
                            "filesystem_error",
                            format!(
                                "Could not delete attachment file '{}': {e}",
                                text(&row, "stored_relpath")
                            ),
                        )
                        .with_hint("Fix the local file permissions and retry `attachments remove`.")
                        .retryable(true)
                    })?;
                    deleted = true;
                    prune(root, path);
                }
                Ok(())
            })?;
            set(&mut result, "removed", Value::Bool(true));
            set(&mut result, "deleted_file", Value::Bool(deleted));
            Ok(result)
        }
        Command::Verify { transaction } => {
            fs::create_dir_all(root).map_err(io)?;
            let mut found = list(conn, scope, root, transaction.as_deref(), None, 0)?;
            let mut broken = 0;
            for row in &mut found {
                let mut issues = Vec::new();
                if text(row, "attachment_type") != "url" {
                    let (path, valid) = stored(root, text(row, "stored_relpath"));
                    if !valid {
                        issues.push(s("invalid_storage_path"));
                    } else if let Some(path) = path.filter(|p| p.exists()) {
                        let (size, digest) = hash(&path, None)?;
                        if get(row, "size_bytes").is_some_and(|v| {
                            !matches!(v, Value::Null) && v != &Value::Int(size.into())
                        }) {
                            issues.push(s("size_mismatch"));
                        }
                        if !text(row, "sha256").is_empty() && text(row, "sha256") != digest {
                            issues.push(s("sha256_mismatch"));
                        }
                    } else {
                        issues.push(s("missing_file"));
                    }
                }
                if !issues.is_empty() {
                    broken += 1;
                }
                set(
                    row,
                    "status",
                    s(if issues.is_empty() { "ok" } else { "broken" }),
                );
                set(row, "issues", Value::List(issues));
            }
            Ok(dict([
                ("checked", Value::Int(found.len().into())),
                ("broken", Value::Int(broken.into())),
                ("ok", Value::Int((found.len() - broken).into())),
                ("results", Value::List(found)),
            ]))
        }
        Command::Copy {
            transaction,
            source_transaction,
            attachments,
        } => copy(
            conn,
            scope,
            root,
            transaction,
            source_transaction,
            attachments,
        ),
        Command::Open { attachment } => {
            let found = list(conn, scope, root, None, None, 0)?;
            let row = found
                .into_iter()
                .find(|r| text(r, "id") == attachment)
                .ok_or_else(|| {
                    AppError::new("not_found", format!("Attachment '{attachment}' not found"))
                })?;
            if text(&row, "attachment_type") == "url" {
                let url = value(&row, "url");
                return Ok(dict([
                    ("attachment", row),
                    ("target_type", s("url")),
                    ("url", url),
                ]));
            }
            let relative = text(&row, "stored_relpath");
            if relative.is_empty() {
                return Err(AppError::new(
                    "not_found",
                    "Attachment has no stored file path",
                ));
            }
            let (path, valid) = stored(root, relative);
            if !valid {
                return Err(AppError::new(
                    "validation",
                    "Attachment has an invalid managed storage path",
                ));
            }
            let path = path.filter(|p| p.exists()).ok_or_else(|| {
                AppError::new("not_found", "Attachment file is missing")
                    .with_details(dict([("stored_relpath", s(relative))]))
            })?;
            Ok(dict([
                ("attachment", row),
                ("target_type", s("file")),
                ("path", s(path.to_string_lossy().as_ref())),
            ]))
        }
        Command::Gc { .. } => unreachable!(),
    }
}

fn copy(
    conn: &Connection,
    scope: &Scope<'_>,
    root: &Path,
    transaction: &str,
    source_transaction: &str,
    attachments: &[Value],
) -> Result<Value> {
    if attachments.is_empty() {
        return Err(AppError::new(
            "validation",
            "Select at least one attachment to copy",
        ));
    }
    let target = resolve_transaction(conn, scope.profile_id, transaction)?;
    let source = resolve_transaction(conn, scope.profile_id, source_transaction)?;
    if source == target {
        return Err(AppError::new(
            "validation",
            "Choose a different source transaction for evidence reuse",
        ));
    }
    let mut ids = Vec::new();
    for id in attachments {
        let id = string(id)
            .map(strip)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AppError::new("validation", "attachment_ids must be non-empty strings")
            })?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let mut source_rows = Vec::new();
    let mut missing = Vec::new();
    for id in ids {
        match selected(conn, scope.profile_id, id) {
            Ok(row) if text(&row, "transaction_id") == source => source_rows.push(row),
            _ => missing.push(s(id)),
        }
    }
    if !missing.is_empty() {
        return Err(
            AppError::new("not_found", "One or more source attachments were not found")
                .with_details(dict([("attachment_ids", Value::List(missing))])),
        );
    }
    fs::create_dir_all(root).map_err(io)?;
    let mut paths = Vec::new();
    let mut new_ids = Vec::new();
    let timestamp = now(conn)?;
    let outcome = atomic(conn, || {
        for row in &source_rows {
            let id = uuid(conn)?;
            let mut relative = None;
            let mut size = value(row, "size_bytes");
            let mut digest = value(row, "sha256");
            if text(row, "attachment_type") == "file" {
                let (source_path, valid) = stored(root, text(row, "stored_relpath"));
                let fail = |code: &str, message: &str| {
                    AppError::new(code, message)
                        .with_details(dict([("attachment_id", value(row, "id"))]))
                };
                if !valid || source_path.is_none() {
                    return Err(fail(
                        "validation",
                        "Source attachment has an invalid managed storage path",
                    ));
                }
                let source_path = source_path.expect("checked");
                if !source_path.exists() {
                    return Err(fail("not_found", "Source attachment file is missing"));
                }
                let name = if text(row, "original_filename").is_empty() {
                    text(row, "label")
                } else {
                    text(row, "original_filename")
                };
                let (path, rel) = storage(root, scope.profile_id, &id, name)?;
                paths.push(path.clone());
                let (n, sha) = hash(&source_path, Some(&path))?;
                if !matches!(size, Value::Null) && size != Value::Int(n.into()) {
                    return Err(fail(
                        "validation",
                        "Source attachment size does not match its stored metadata",
                    ));
                }
                if !text(row, "sha256").is_empty() && text(row, "sha256") != sha {
                    return Err(fail(
                        "validation",
                        "Source attachment hash does not match its stored metadata",
                    ));
                }
                size = Value::Int(n.into());
                digest = s(sha);
                relative = Some(rel);
            }
            let size = match size {
                Value::Int(n) => i64::try_from(n).ok(),
                _ => None,
            };
            conn.execute("INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,original_filename,stored_relpath,source_url,media_type,size_bytes,sha256,copied_from_attachment_id,copied_from_transaction_id,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",params![id,scope.workspace_id,scope.profile_id,target,text(row,"attachment_type"),string(&value(row,"label")),string(&value(row,"original_filename")),relative,string(&value(row,"source_url")),string(&value(row,"media_type")),size,string(&digest),text(row,"id"),text(row,"transaction_id"),timestamp]).map_err(error)?;
            new_ids.push(id);
        }
        Ok(())
    });
    if outcome.is_err() {
        for path in &paths {
            let _ = fs::remove_file(path);
            prune(root, path);
        }
    }
    outcome?;
    let copied = new_ids
        .iter()
        .map(|id| selected(conn, scope.profile_id, id).map(|r| payload(&r, root)))
        .collect::<Result<Vec<_>>>()?;
    Ok(dict([
        ("source_transaction_id", s(source)),
        ("target_transaction_id", s(target)),
        ("copied", Value::Int(copied.len().into())),
        ("attachments", Value::List(copied)),
    ]))
}

fn gc(conn: &Connection, root: &Path, dry_run: bool) -> Result<Value> {
    fs::create_dir_all(root).map_err(io)?;
    let live=rows(conn,"SELECT stored_relpath FROM attachments WHERE stored_relpath IS NOT NULL AND stored_relpath!=''",[])?;
    let mut files = Vec::new();
    let mut paths = Vec::new();
    let mut size = 0;
    // pathlib.rglob does not recurse into directory symlinks.
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let mut child_dirs = Vec::new();
        for entry in fs::read_dir(dir).map_err(io)? {
            let entry = entry.map_err(io)?;
            let path = entry.path();
            if entry.file_type().map_err(io)?.is_dir() {
                child_dirs.push(path);
                continue;
            }
            if !path.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("managed root")
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            if live.iter().any(|r| text(r, "stored_relpath") == relative) {
                continue;
            }
            let bytes = path.metadata().map_err(io)?.len();
            size += bytes;
            files.push(dict([
                ("stored_relpath", s(relative)),
                ("size_bytes", Value::Int(bytes.into())),
            ]));
            paths.push(path);
        }
        dirs.extend(child_dirs.into_iter().rev());
    }
    if !dry_run {
        for path in &paths {
            fs::remove_file(path).map_err(io)?;
            prune(root, path);
        }
    }
    Ok(dict([
        ("attachments_root", s(root.to_string_lossy().as_ref())),
        ("dry_run", Value::Bool(dry_run)),
        ("orphaned_files", Value::Int(files.len().into())),
        ("orphaned_bytes", Value::Int(size.into())),
        (
            "removed_files",
            Value::Int(if dry_run {
                0.into()
            } else {
                files.len().into()
            }),
        ),
        (
            "removed_bytes",
            Value::Int(if dry_run { 0.into() } else { size.into() }),
        ),
        ("files", Value::List(files)),
    ]))
}

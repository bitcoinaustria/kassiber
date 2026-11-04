//! `build_evidence_summary` (`ui.audit.evidence.summary`): the audit
//! package's evidence surface without copying files or writing an export.
//! Observed transactions, their direct evidence, reviewed source-of-funds
//! links, journal state, custody quantity status, authored custody evidence
//! hashes and review history stay in separate sections; none of them grants
//! another. Managed attachment files are only checked for existence, and URL
//! references are parsed for redaction, never fetched.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use bitcoin_hashes::{sha256, Hash};
use kassiber_envelope::{dict, get, py_str, s, set, truthy, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql, Connection};

use crate::report_blockers::{custody_quantity_readiness, int, query, rows, sql_error, val};

type Result<T> = std::result::Result<T, AppError>;

const SCHEMA_VERSION: i64 = 2;
const SENSITIVE_MATERIAL_EXCLUSIONS: &[&str] = &[
    "wallet descriptors",
    "xpubs",
    "backend credentials",
    "backend URLs",
    "raw wallet files",
    "raw wallet config",
    "environment files",
    "logs",
    "AI settings",
    "raw AI custody proposals (cryptographic hashes and consent outcomes only)",
    "unrelated books",
    "raw technical wallet evidence (cryptographic hashes only)",
    "chain observer state and derivation coverage",
];
const DECISION_KEYWORDS: &[&str] = &[
    "board",
    "decision",
    "management",
    "approval",
    "resolution",
    "director",
];
const SECRET_URL_QUERY_HINTS: &[&str] = &[
    "access_token",
    "api_key",
    "auth",
    "bearer",
    "key",
    "password",
    "secret",
    "signature",
    "sig",
    "token",
];

/// The request options `_audit_package_options` passes through.
#[derive(Default)]
pub struct Options {
    pub transaction_refs: Option<Vec<String>>,
    pub source_funds_case: Option<String>,
    pub include_journal_state: bool,
    pub include_review_state: bool,
    pub include_edit_history: bool,
}

fn optional_str(args: &Value, key: &str) -> Result<Option<String>> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(text)) => {
            let stripped = kassiber_pytext::strip_py(text);
            Ok(stripped
                .code_points()
                .next()
                .is_some()
                .then(|| py_str(&Value::Str(stripped))))
        }
        Some(other) => Err(
            AppError::new("validation", format!("{key} must be a string"))
                .with_details(dict([("type", s(type_name(other)))])),
        ),
    }
}

fn optional_bool(args: &Value, key: &str, default: bool) -> Result<bool> {
    match get(args, key) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(other) => Err(
            AppError::new("validation", format!("{key} must be a boolean"))
                .with_details(dict([("type", s(type_name(other)))])),
        ),
    }
}

/// `_audit_package_options(args)`, refusing in Python's order. Copied
/// attachments and URL references are validated though the summary ignores
/// them, as the package export shares the parser.
pub fn options(args: &Value) -> Result<Options> {
    let transaction = optional_str(args, "transaction")?;
    let transactions = get(args, "transactions").filter(|v| **v != Value::Null);
    if transaction.is_some() && transactions.is_some() {
        return Err(AppError::new(
            "validation",
            "Use either transaction or transactions, not both",
        ));
    }
    let transaction_refs = match (transaction, transactions) {
        (Some(transaction), _) => Some(vec![transaction]),
        (None, None) => None,
        (None, Some(value)) => {
            let invalid = || {
                AppError::new(
                    "validation",
                    "transactions must be a non-empty array of non-empty strings",
                )
            };
            let Value::List(items) = value else {
                return Err(invalid());
            };
            if items.is_empty() {
                return Err(invalid());
            }
            let mut refs = Vec::new();
            for item in items {
                let Value::Str(text) = item else {
                    return Err(invalid());
                };
                let stripped = kassiber_pytext::strip_py(text);
                if stripped.code_points().next().is_none() {
                    return Err(invalid());
                }
                refs.push(py_str(&Value::Str(stripped)));
            }
            Some(refs)
        }
    };
    let source_funds_case = optional_str(args, "source_funds_case")?;
    if transaction_refs.is_some() && source_funds_case.is_some() {
        return Err(AppError::new(
            "validation",
            "Use either transaction(s) or source_funds_case, not both",
        ));
    }
    optional_bool(args, "include_copied_attachments", true)?;
    optional_bool(args, "include_url_references", true)?;
    Ok(Options {
        transaction_refs,
        source_funds_case,
        include_journal_state: optional_bool(args, "include_journal_state", true)?,
        include_review_state: optional_bool(args, "include_review_state", true)?,
        include_edit_history: optional_bool(args, "include_edit_history", false)?,
    })
}

/// `row[key] or ""`.
fn or_empty(row: &Value, key: &str) -> Value {
    match get(row, key) {
        Some(value) if truthy(value) => value.clone(),
        _ => s(""),
    }
}

fn text(row: &Value, key: &str) -> String {
    match get(row, key) {
        None | Some(Value::Null) => String::new(),
        Some(value) => py_str(value),
    }
}

fn count(value: &Value) -> Result<i64> {
    Ok(i64::try_from(int(value)?).unwrap_or(i64::MAX))
}

/// `float(msat_to_btc(msat))`, or `None`.
fn btc(value: &Value) -> Result<Value> {
    if *value == Value::Null {
        return Ok(Value::Null);
    }
    Ok(format!("{}e-11", int(value)?)
        .parse::<f64>()
        .map_or(Value::Null, Value::Float))
}

fn warning(code: &str, severity: &str, message: impl Into<String>, action: Option<&str>) -> Value {
    let mut item = dict([
        ("code", s(code)),
        ("severity", s(severity)),
        ("message", s(message.into())),
    ]);
    if let Some(action) = action {
        set(&mut item, "action", s(action));
    }
    item
}

/// `json.loads(str(value or "{}"))`, or `{}` when it is not an object.
fn json_object(value: &Value) -> Value {
    let raw = if truthy(value) {
        py_str(value)
    } else {
        "{}".into()
    };
    kassiber_pytext::loads(&raw)
        .ok()
        .filter(|v| matches!(v, Value::Dict(_)))
        .unwrap_or_else(|| dict::<&str>([]))
}

fn in_list(column: &str, ids: &[String]) -> String {
    format!("{column} IN ({})", vec!["?"; ids.len()].join(","))
}

fn bind<'a>(head: &[&str], ids: impl IntoIterator<Item = &'a String>) -> Vec<Sql> {
    head.iter()
        .map(|v| Sql::Text((*v).to_owned()))
        .chain(ids.into_iter().map(|id| Sql::Text(id.clone())))
        .collect()
}

fn sha256_hex(text: &str) -> String {
    sha256::Hash::hash(text.as_bytes()).to_string()
}

fn canonical(value: &Value) -> Result<String> {
    kassiber_pyjson::dumps(
        value,
        &kassiber_pyjson::Options::new().sort_keys(true).compact(),
    )
    .map_err(|error| AppError::new("internal_error", error.to_string()))
}

// ---------------------------------------------------------------- URLs

/// `urlparse(url)` parts the audit summary reads: scheme, netloc, query and
/// fragment, with CPython's refusals for malformed authorities.
fn split_url(raw: &str) -> Result<(String, String, String, String)> {
    let mut url: String = raw
        .trim_start_matches(|c: char| c as u32 <= 0x20)
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let mut scheme = String::new();
    if let Some(i) = url.find(':') {
        let head = &url[..i];
        if i > 0
            && head.starts_with(|c: char| c.is_ascii_alphabetic())
            && head
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            scheme = head.to_ascii_lowercase();
            url = url[i + 1..].to_owned();
        }
    }
    let mut netloc = String::new();
    if let Some(rest) = url.strip_prefix("//") {
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        netloc = rest[..end].to_owned();
        url = rest[end..].to_owned();
        if netloc.contains('[') != netloc.contains(']') {
            return Err(AppError::new("internal_error", "Invalid IPv6 URL"));
        }
        if netloc.contains('[') {
            let host = netloc.rsplit('@').next().unwrap_or_default();
            let bracketed = host
                .split_once('[')
                .map(|(_, rest)| rest.split(']').next().unwrap_or_default())
                .unwrap_or_default();
            if let Some(future) = bracketed.strip_prefix('v') {
                let valid = future.split_once('.').is_some_and(|(version, address)| {
                    !version.is_empty()
                        && version.chars().all(|c| c.is_ascii_hexdigit())
                        && !address.is_empty()
                });
                if !valid {
                    return Err(AppError::new(
                        "internal_error",
                        "IPvFuture address is invalid",
                    ));
                }
            } else if bracketed.parse::<std::net::Ipv4Addr>().is_ok() {
                return Err(AppError::new(
                    "internal_error",
                    "An IPv4 address cannot be in brackets",
                ));
            } else if bracketed
                .split('%')
                .next()
                .unwrap_or_default()
                .parse::<std::net::Ipv6Addr>()
                .is_err()
            {
                return Err(AppError::new(
                    "internal_error",
                    format!(
                        "{} does not appear to be an IPv4 or IPv6 address",
                        kassiber_pytext::repr_str(bracketed)
                    ),
                ));
            }
        }
    }
    let mut fragment = String::new();
    if let Some((before, after)) = url.split_once('#') {
        fragment = after.to_owned();
        url = before.to_owned();
    }
    let mut query = String::new();
    if let Some((_, after)) = url.split_once('?') {
        query = after.to_owned();
    }
    if !netloc.is_ascii() && netloc.contains(|c: char| "⁇⁈⁉℀℁℅℆⩴︓︖﹕﹖﹟﹫＃／：？＠".contains(c))
    {
        return Err(AppError::new(
            "internal_error",
            format!("netloc '{netloc}' contains invalid characters under NFKC normalization"),
        ));
    }
    Ok((scheme, netloc, query, fragment))
}

/// `urllib.parse.unquote` with UTF-8 and replacement errors.
fn unquote(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
            if let (Some(a), Some(b)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                output.push(a * 16 + b);
                i += 3;
                continue;
            }
        }
        output.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

/// `parse_qsl(text, keep_blank_values=True)` keys.
fn query_keys(text: &str) -> Vec<String> {
    text.split('&')
        .filter(|piece| !piece.is_empty())
        .map(|piece| {
            unquote(
                &piece
                    .split('=')
                    .next()
                    .unwrap_or_default()
                    .replace('+', " "),
            )
        })
        .collect()
}

fn secret_like(key: &str) -> bool {
    let lower = kassiber_pytext::lower(key);
    SECRET_URL_QUERY_HINTS
        .iter()
        .any(|hint| lower.contains(hint))
}

/// `_url_safety(url)` as the summary reads it: whether the reference carries
/// credentials or secret-like parameters, and its host and scheme.
fn url_safety(raw: &str) -> Result<(bool, String, String, String)> {
    let raw = kassiber_pytext::strip(raw);
    if raw.is_empty() {
        return Ok((false, String::new(), String::new(), String::new()));
    }
    let (scheme, mut netloc, query, fragment) = split_url(raw)?;
    let mut redacted = false;
    let mut reason = String::new();
    let (userinfo, hostinfo) = match netloc.rsplit_once('@') {
        Some((userinfo, hostinfo)) => (Some(userinfo.to_owned()), hostinfo.to_owned()),
        None => (None, netloc.clone()),
    };
    let credentials = userinfo.as_deref().is_some_and(|userinfo| {
        let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
        !user.is_empty() || !password.is_empty()
    });
    if credentials {
        redacted = true;
        reason = "embedded credentials".into();
        let host = match hostinfo.split_once('[') {
            Some((_, bracketed)) => bracketed.split(']').next().unwrap_or_default().to_owned(),
            None => hostinfo.split(':').next().unwrap_or_default().to_owned(),
        };
        netloc = match host.split_once('%') {
            Some((host, zone)) => format!("{}%{zone}", kassiber_pytext::lower(host)),
            None => kassiber_pytext::lower(&host),
        };
    }
    if query_keys(&query).iter().any(|key| secret_like(key)) {
        redacted = true;
        if reason.is_empty() {
            reason = "secret-like query parameter".into();
        }
    }
    let fragment_keys = query_keys(fragment.trim_start_matches('?'));
    let fragment_secret = if fragment_keys.is_empty() {
        !fragment.is_empty() && secret_like(&fragment)
    } else {
        fragment_keys.iter().any(|key| secret_like(key))
    };
    if fragment_secret {
        redacted = true;
        if reason.is_empty() {
            reason = "secret-like URL fragment".into();
        }
    }
    Ok((redacted, reason, netloc, scheme))
}

// --------------------------------------------------------- attachments

fn attachment_summary(row: &Value, root: &Path) -> Result<Value> {
    let relative = text(row, "stored_relpath");
    let (path, valid) = kassiber_transactions::attachments::stored_path(root, &relative);
    let exists = match path {
        Some(path) => Value::Bool(path.exists()),
        None if truthy(&val(row, "stored_relpath")) && !valid => Value::Bool(false),
        None => Value::Null,
    };
    let (redacted, reason, host, scheme) = url_safety(&text(row, "source_url"))?;
    let size = val(row, "size_bytes");
    Ok(dict([
        ("id", val(row, "id")),
        ("attachment_type", val(row, "attachment_type")),
        (
            "label",
            s(kassiber_transactions::attachments::display_label(row)),
        ),
        ("original_filename", or_empty(row, "original_filename")),
        ("media_type", or_empty(row, "media_type")),
        (
            "size_bytes",
            if size == Value::Null {
                Value::Null
            } else {
                Value::Int(int(&size)?)
            },
        ),
        ("sha256", or_empty(row, "sha256")),
        ("exists", exists),
        ("url_host", s(host)),
        ("url_scheme", s(scheme)),
        ("url_redacted", Value::Bool(redacted)),
        ("url_redaction_reason", s(reason)),
        (
            "copied_from_attachment_id",
            or_empty(row, "copied_from_attachment_id"),
        ),
        (
            "copied_from_transaction_id",
            or_empty(row, "copied_from_transaction_id"),
        ),
        ("created_at", val(row, "created_at")),
    ]))
}

fn attachments_by_ids(
    conn: &Connection,
    profile: &str,
    ids: &[String],
    root: &Path,
) -> Result<Vec<Value>> {
    let unique: Vec<String> = ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if unique.is_empty() {
        return Ok(Vec::new());
    }
    rows(
        conn,
        &format!(
            "SELECT * FROM attachments WHERE profile_id = ? AND {} ORDER BY created_at ASC, id ASC",
            in_list("id", &unique)
        ),
        params_from_iter(bind(&[profile], &unique)),
    )?
    .iter()
    .map(|row| attachment_summary(row, root))
    .collect()
}

fn join_attachment_ids(
    conn: &Connection,
    table: &str,
    column: &str,
    id: &str,
) -> Result<Vec<String>> {
    Ok(rows(
        conn,
        &format!("SELECT attachment_id FROM {table} WHERE {column} = ? ORDER BY created_at ASC, attachment_id ASC"),
        [id],
    )?
    .iter()
    .map(|row| text(row, "attachment_id"))
    .collect())
}

// -------------------------------------------------------- transactions

fn transaction_summary(row: &Value, tags: &[String]) -> Result<Value> {
    Ok(dict([
        ("id", val(row, "id")),
        ("external_id", or_empty(row, "external_id")),
        ("occurred_at", or_empty(row, "occurred_at")),
        ("direction", val(row, "direction")),
        ("asset", val(row, "asset")),
        ("amount", btc(&val(row, "amount"))?),
        ("amount_msat", val(row, "amount")),
        ("fee", btc(&val(row, "fee"))?),
        ("fee_msat", val(row, "fee")),
        ("fiat_currency", or_empty(row, "fiat_currency")),
        ("fiat_value", val(row, "fiat_value")),
        ("fiat_price_source", or_empty(row, "fiat_price_source")),
        ("pricing_source_kind", or_empty(row, "pricing_source_kind")),
        ("pricing_provider", or_empty(row, "pricing_provider")),
        ("pricing_quality", or_empty(row, "pricing_quality")),
        ("kind", or_empty(row, "kind")),
        ("kind_override", or_empty(row, "kind_override")),
        ("description", or_empty(row, "description")),
        ("counterparty", or_empty(row, "counterparty")),
        ("note", or_empty(row, "note")),
        ("excluded", Value::Bool(truthy(&val(row, "excluded")))),
        (
            "tags",
            Value::List(tags.iter().map(|t| s(t.as_str())).collect()),
        ),
    ]))
}

fn tags(conn: &Connection, id: &str) -> Result<Vec<String>> {
    Ok(rows(
        conn,
        "SELECT tg.label FROM transaction_tags tt JOIN tags tg ON tg.id = tt.tag_id WHERE tt.transaction_id = ? ORDER BY tg.label ASC, tg.id ASC",
        [id],
    )?
    .iter()
    .map(|row| text(row, "label"))
    .collect())
}

/// `resolve_transaction(conn, profile_id, ref)`: an id, then a unique
/// external id.
fn resolve_transaction(conn: &Connection, profile: &str, reference: &str) -> Result<Value> {
    if let Some(row) = rows(
        conn,
        "SELECT * FROM transactions WHERE profile_id = ? AND id = ?",
        [profile, reference],
    )?
    .into_iter()
    .next()
    {
        return Ok(row);
    }
    let mut found = rows(
        conn,
        "SELECT * FROM transactions WHERE profile_id = ? AND external_id = ? ORDER BY occurred_at DESC, created_at DESC, id DESC LIMIT 2",
        [profile, reference],
    )?;
    if found.len() > 1 {
        return Err(AppError::new(
            "ambiguous_reference",
            format!("Transaction external_id '{reference}' matches multiple transactions"),
        )
        .with_hint("Use the Kassiber transaction id from `transactions list` or narrow the command to an unambiguous row."));
    }
    found
        .pop()
        .ok_or_else(|| AppError::new("app_error", format!("Transaction '{reference}' not found")))
}

fn source_summary(conn: &Connection, profile: &str, id: &str, root: &Path) -> Result<Value> {
    let Some(source) = rows(
        conn,
        "SELECT * FROM source_funds_sources WHERE profile_id = ? AND id = ?",
        [profile, id],
    )?
    .into_iter()
    .next() else {
        return Ok(Value::Null);
    };
    let ids = join_attachment_ids(conn, "source_funds_source_attachments", "source_id", id)?;
    Ok(dict([
        ("id", val(&source, "id")),
        ("source_type", val(&source, "source_type")),
        ("label", val(&source, "label")),
        ("asset", val(&source, "asset")),
        ("amount", btc(&val(&source, "amount"))?),
        ("amount_msat", val(&source, "amount")),
        ("fiat_currency", or_empty(&source, "fiat_currency")),
        ("fiat_value", val(&source, "fiat_value")),
        ("acquired_at", or_empty(&source, "acquired_at")),
        ("review_state", val(&source, "review_state")),
        ("description", or_empty(&source, "description")),
        (
            "attachments",
            Value::List(attachments_by_ids(conn, profile, &ids, root)?),
        ),
    ]))
}

/// `_source_funds_links_for_tx`: suggested and reviewed links into the
/// transaction. Only reviewed links reveal amounts, evidence and parents.
fn source_links(conn: &Connection, profile: &str, id: &str, root: &Path) -> Result<Vec<Value>> {
    let mut output = Vec::new();
    for row in rows(
        conn,
        "SELECT * FROM source_funds_links WHERE profile_id = ? AND to_transaction_id = ? AND state != 'rejected' ORDER BY state ASC, created_at ASC, id ASC",
        [profile, id],
    )? {
        let reviewed = text(&row, "state") == "reviewed";
        let attachments = if reviewed {
            let ids =
                join_attachment_ids(conn, "source_funds_link_attachments", "link_id", &text(&row, "id"))?;
            attachments_by_ids(conn, profile, &ids, root)?
        } else {
            Vec::new()
        };
        let mut parent = Value::Null;
        if reviewed && truthy(&val(&row, "from_transaction_id")) {
            if let Some(tx) = rows(
                conn,
                "SELECT * FROM transactions WHERE profile_id = ? AND id = ?",
                [profile, &text(&row, "from_transaction_id")],
            )?
            .into_iter()
            .next()
            {
                parent = transaction_summary(&tx, &[])?;
            }
        }
        let from_source = if reviewed && truthy(&val(&row, "from_source_id")) {
            source_summary(conn, profile, &text(&row, "from_source_id"), root)?
        } else {
            Value::Null
        };
        let reviewed_or = |value: Result<Value>, other: Value| -> Result<Value> {
            if reviewed {
                value
            } else {
                Ok(other)
            }
        };
        output.push(dict([
            ("id", val(&row, "id")),
            ("link_type", val(&row, "link_type")),
            ("state", val(&row, "state")),
            ("confidence", val(&row, "confidence")),
            ("method", val(&row, "method")),
            ("asset", val(&row, "asset")),
            (
                "allocation_amount",
                reviewed_or(btc(&val(&row, "allocation_amount")), Value::Null)?,
            ),
            (
                "allocation_amount_msat",
                reviewed_or(Ok(val(&row, "allocation_amount")), Value::Null)?,
            ),
            (
                "from_asset",
                reviewed_or(
                    Ok(if truthy(&val(&row, "from_asset")) {
                        val(&row, "from_asset")
                    } else {
                        val(&row, "asset")
                    }),
                    Value::Null,
                )?,
            ),
            (
                "from_allocation_amount",
                reviewed_or(btc(&val(&row, "from_allocation_amount")), Value::Null)?,
            ),
            (
                "from_allocation_amount_msat",
                reviewed_or(Ok(val(&row, "from_allocation_amount")), Value::Null)?,
            ),
            (
                "allocation_policy",
                reviewed_or(Ok(val(&row, "allocation_policy")), s(""))?,
            ),
            (
                "explanation",
                reviewed_or(Ok(or_empty(&row, "explanation")), s(""))?,
            ),
            (
                "uses_chain_observation",
                Value::Bool(truthy(&val(&row, "uses_chain_observation"))),
            ),
            (
                "chain_data_confirmed",
                Value::Bool(reviewed && truthy(&val(&row, "chain_data_confirmed"))),
            ),
            ("review_details_redacted", Value::Bool(!reviewed)),
            ("attachments", Value::List(attachments)),
            ("from_source", from_source),
            ("from_transaction", parent),
            ("created_at", val(&row, "created_at")),
            ("updated_at", val(&row, "updated_at")),
        ]));
    }
    Ok(output)
}

fn journal_entries(conn: &Connection, profile: &str, id: &str) -> Result<Vec<Value>> {
    rows(
        conn,
        "SELECT * FROM journal_entries WHERE profile_id = ? AND transaction_id = ? ORDER BY occurred_at ASC, entry_type ASC, id ASC",
        [profile, id],
    )?
    .iter()
    .map(|row| {
        Ok(dict([
            ("id", val(row, "id")),
            ("occurred_at", val(row, "occurred_at")),
            ("entry_type", val(row, "entry_type")),
            ("asset", val(row, "asset")),
            ("quantity", btc(&val(row, "quantity"))?),
            ("quantity_msat", val(row, "quantity")),
            ("fiat_value", val(row, "fiat_value")),
            ("gain_loss", val(row, "gain_loss")),
            ("at_category", val(row, "at_category")),
            ("at_kennzahl", val(row, "at_kennzahl")),
            ("pricing_source_kind", val(row, "pricing_source_kind")),
            ("pricing_quality", val(row, "pricing_quality")),
            ("description", val(row, "description")),
        ]))
    })
    .collect()
}

fn quarantine(conn: &Connection, profile: &str, id: &str) -> Result<Value> {
    Ok(rows(
        conn,
        "SELECT * FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        [profile, id],
    )?
    .first()
    .map_or(Value::Null, |row| {
        dict([
            ("reason", val(row, "reason")),
            ("details", or_empty(row, "detail_json")),
            ("created_at", val(row, "created_at")),
        ])
    }))
}

fn has_pricing_evidence(tx: &Value, quarantine: &Value) -> bool {
    if truthy(quarantine)
        && kassiber_pytext::lower(&text_or(quarantine, "reason")).contains("price")
    {
        return false;
    }
    if val(tx, "fiat_rate") == Value::Null && val(tx, "fiat_value") == Value::Null {
        return false;
    }
    [
        "pricing_source_kind",
        "fiat_price_source",
        "pricing_provider",
        "pricing_external_ref",
        "fiat_rate_exact",
        "fiat_value_exact",
    ]
    .iter()
    .any(|key| truthy(&val(tx, key)))
}

/// `str(row.get(key) or "")`.
fn text_or(row: &Value, key: &str) -> String {
    match get(row, key) {
        Some(value) if truthy(value) => py_str(value),
        _ => String::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn evidence_warnings(
    tx: &Value,
    tags: &[String],
    direct: &[Value],
    links: &[Value],
    freshness: &Value,
    quarantine: &Value,
    include_review_state: bool,
) -> Vec<Value> {
    let mut warnings = Vec::new();
    if truthy(&val(tx, "excluded")) {
        warnings.push(warning(
            "transaction_excluded",
            "warning",
            "This transaction is excluded from accounting reports.",
            Some("Review whether the exclusion is intended before handoff."),
        ));
    }
    if direct.is_empty() {
        warnings.push(warning(
            "receipt_missing",
            "blocker",
            "No direct receipt, invoice, note, or URL reference is attached to this transaction.",
            Some("Attach a local receipt file or a URL reference from transaction detail."),
        ));
    }
    if direct.iter().any(|item| {
        text(item, "attachment_type") == "file" && val(item, "exists") == Value::Bool(false)
    }) {
        warnings.push(warning(
            "receipt_file_missing",
            "blocker",
            "A copied attachment row points to a missing managed file.",
            Some("Reattach the local file or remove the stale attachment row."),
        ));
    }
    let mut haystack = ["kind", "description", "counterparty", "note"]
        .iter()
        .map(|key| text_or(tx, key))
        .collect::<Vec<_>>()
        .join(" ");
    haystack.push(' ');
    haystack.push_str(&tags.join(" "));
    let haystack = kassiber_pytext::lower(&haystack);
    if DECISION_KEYWORDS.iter().any(|k| haystack.contains(k))
        && !direct.iter().any(|item| {
            let label = kassiber_pytext::lower(&text(item, "label"));
            DECISION_KEYWORDS.iter().any(|k| label.contains(k))
        })
    {
        warnings.push(warning(
            "decision_evidence_missing",
            "blocker",
            "This transaction looks decision-backed, but no board or management decision evidence is attached.",
            Some("Attach the decision document as a URL reference or local file."),
        ));
    }
    if include_review_state {
        let reviewed: Vec<_> = links
            .iter()
            .filter(|l| text(l, "state") == "reviewed")
            .collect();
        if reviewed.is_empty() {
            warnings.push(warning(
                "source_link_missing",
                "blocker",
                "No reviewed source-of-funds link or root source is connected to this transaction.",
                Some("Review or create a source-of-funds link before exporting."),
            ));
        }
        if links.iter().any(|l| text(l, "state") == "suggested") {
            warnings.push(warning(
                "source_link_unreviewed",
                "blocker",
                "At least one source-of-funds suggestion is still unreviewed.",
                Some("Accept, edit, or reject the suggested link."),
            ));
        }
        let empty = |v: &Value| !matches!(v, Value::List(items) if !items.is_empty());
        if reviewed.iter().any(|link| {
            empty(&val(link, "attachments"))
                && empty(&val(&val(link, "from_source"), "attachments"))
        }) {
            warnings.push(warning(
                "source_evidence_missing",
                "warning",
                "A reviewed source-of-funds link has no attached supporting evidence.",
                Some("Attach evidence to the link or its root source."),
            ));
        }
    }
    if truthy(&val(freshness, "needs_processing")) {
        warnings.push(warning(
            "journal_stale",
            "blocker",
            format!(
                "Journal state is {}: {}.",
                py_str(&val(freshness, "status")),
                py_str(&val(freshness, "reason"))
            ),
            Some("Run journal processing before handing this transaction to an auditor."),
        ));
    }
    if !has_pricing_evidence(tx, quarantine) {
        warnings.push(warning(
            "pricing_evidence_missing",
            "blocker",
            "No persisted pricing source is available for this transaction.",
            Some("Add a manual price or refresh rates so the pricing source is recorded."),
        ));
    }
    if truthy(quarantine) {
        warnings.push(warning(
            "journal_quarantined",
            "blocker",
            format!(
                "Journal processing quarantined this transaction: {}.",
                py_str(&val(quarantine, "reason"))
            ),
            Some("Resolve the journal quarantine before export."),
        ));
    }
    warnings.push(warning(
        "sensitive_material_excluded",
        "info",
        "Descriptors, xpubs, backend URLs, credentials, wallet files, logs, AI settings, and technical wallet evidence are excluded from this audit surface.",
        None,
    ));
    warnings
}

fn custody_warning(tx: &Value, summary: &Value, bounded: bool) -> Option<Value> {
    if text(summary, "status") != "known_custody_gaps" {
        return None;
    }
    if bounded {
        return Some(warning(
            "custody_quantity_unresolved",
            "blocker",
            "Known custody gaps may affect final tax basis; profile-wide quantity and timing details were excluded from this bounded package.",
            None,
        ));
    }
    let blocked_from = text_or(summary, "blocked_from");
    let occurred_at = text_or(tx, "occurred_at");
    if !blocked_from.is_empty() && !occurred_at.is_empty() && occurred_at < blocked_from {
        return None;
    }
    let totals = match val(summary, "unresolved_by_asset") {
        Value::List(items) => items
            .iter()
            .map(|item| {
                format!(
                    "{} msat {}",
                    py_str(&val(item, "amount_msat")),
                    py_str(&val(item, "asset"))
                )
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    };
    let suffix = if totals.is_empty() {
        String::new()
    } else {
        format!(" ({totals})")
    };
    Some(warning(
        "custody_quantity_unresolved",
        "blocker",
        format!("Known custody gaps withhold final tax basis from this point onward{suffix}."),
        None,
    ))
}

// ------------------------------------------------- profile-wide sections

/// `authored_evidence_hash_summary`.
fn authored_evidence(conn: &Connection, profile: &str, ids: Option<&[String]>) -> Result<Value> {
    let mut sql = "SELECT s.subject_kind, s.subject_id, s.detail_hash, s.quantity_hash, s.created_at FROM custody_authored_evidence_snapshots s WHERE s.profile_id = ?".to_owned();
    let mut bound = vec![Sql::Text(profile.into())];
    if let Some(ids) = ids {
        let selected: Vec<String> = ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if selected.is_empty() {
            return Ok(dict([
                ("snapshot_count", 0.into()),
                ("subject_count", 0.into()),
                ("hashes", Value::List(vec![])),
            ]));
        }
        sql.push_str(&format!(" AND s.subject_kind = 'custody_component' AND EXISTS (SELECT 1 FROM custody_component_legs l WHERE l.profile_id = s.profile_id AND l.component_id = s.subject_id AND {})", in_list("COALESCE(l.anchor_transaction_id, l.transaction_id)", &selected)));
        bound.extend(selected.into_iter().map(Sql::Text));
    }
    sql.push_str(
        " ORDER BY s.subject_kind, s.subject_id, s.created_at, s.detail_hash, s.quantity_hash",
    );
    let hashes = match query(conn, &sql, params_from_iter(bound)) {
        Ok(rows) => rows,
        Err(error) if crate::report_blockers::operational(&error) => Vec::new(),
        Err(error) => return Err(sql_error(error)),
    };
    let subjects: BTreeSet<(String, String)> = hashes
        .iter()
        .map(|h| (text(h, "subject_kind"), text(h, "subject_id")))
        .collect();
    Ok(dict([
        ("snapshot_count", hashes.len().into()),
        ("subject_count", subjects.len().into()),
        ("hashes", Value::List(hashes)),
    ]))
}

/// `complete_selected_review_ids`: reviews whose exact, non-empty authored
/// boundary lies inside the selection.
fn complete_reviews(
    conn: &Connection,
    profile: &str,
    selected: &BTreeSet<String>,
    review_ids: &[String],
) -> Result<BTreeSet<String>> {
    let candidates: Vec<String> = review_ids
        .iter()
        .filter(|v| !v.is_empty())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if selected.is_empty() || candidates.is_empty() {
        return Ok(BTreeSet::new());
    }
    let found = rows(
        conn,
        &format!("SELECT r.id, s.expected_source_count, s.expected_return_count, x.role, x.transaction_id FROM custody_gap_reviews r JOIN custody_gap_review_relation_sets s ON s.review_id = r.id LEFT JOIN custody_gap_review_transactions x ON x.review_id = r.id WHERE r.profile_id = ? AND {} ORDER BY r.id, x.role, x.transaction_id, x.id", in_list("r.id", &candidates)),
        params_from_iter(bind(&[profile], &candidates)),
    )?;
    let mut state: BTreeMap<String, (i64, i64, BTreeSet<String>, BTreeSet<String>)> =
        BTreeMap::new();
    for row in &found {
        let entry = match state.entry(text(row, "id")) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => entry.insert((
                count(&val(row, "expected_source_count"))?,
                count(&val(row, "expected_return_count"))?,
                BTreeSet::new(),
                BTreeSet::new(),
            )),
        };
        if val(row, "role") != Value::Null && val(row, "transaction_id") != Value::Null {
            match text(row, "role").as_str() {
                "source" => entry.2.insert(text(row, "transaction_id")),
                "return" => entry.3.insert(text(row, "transaction_id")),
                other => {
                    return Err(AppError::new("internal_error", format!("'{other}'")));
                }
            };
        }
    }
    Ok(state
        .into_iter()
        .filter(|(_, (sources, returns, source, ret))| {
            *sources > 0
                && *returns > 0
                && source.len() as i64 == *sources
                && ret.len() as i64 == *returns
                && source.iter().chain(ret).all(|id| selected.contains(id))
        })
        .map(|(id, _)| id)
        .collect())
}

fn review_history_row(row: &Value, wide: bool) -> Result<Value> {
    let event_kind = match get(row, "event_kind") {
        Some(v) if truthy(v) => py_str(v),
        _ => "review_decision".into(),
    };
    let status = if event_kind == "bridge_reopened" {
        "needs_review"
    } else if text(row, "action") == "resolved" {
        "resolved"
    } else {
        "dismissed"
    };
    let component_revision = val(row, "component_revision");
    let mut output = dict([
        ("revision", Value::Int(int(&val(row, "revision"))?)),
        ("event_kind", s(event_kind)),
        ("status", s(status)),
        ("component_id", val(row, "component_id")),
        (
            "component_revision",
            if component_revision == Value::Null {
                Value::Null
            } else {
                Value::Int(int(&component_revision)?)
            },
        ),
        ("authored_source", val(row, "authored_source")),
        ("created_at", val(row, "created_at")),
    ]);
    if wide {
        let raw = text_or(row, "snapshot_json");
        let snapshot = kassiber_pytext::loads(if raw.is_empty() { "{}" } else { &raw })
            .unwrap_or_else(|_| dict::<&str>([]));
        if !matches!(snapshot, Value::Dict(_)) {
            return Err(AppError::new(
                "internal_error",
                format!("'{}' object has no attribute 'get'", type_name(&snapshot)),
            ));
        }
        let residual = val(&snapshot, "residual_classification");
        let classification = match &residual {
            Value::Dict(_) if truthy(&val(&residual, "classification")) => {
                s(py_str(&val(&residual, "classification")))
            }
            _ => Value::Null,
        };
        set(&mut output, "reason", val(row, "reason"));
        set(
            &mut output,
            "retained_msat",
            Value::Int(int(&val(&snapshot, "retained_msat"))?),
        );
        set(
            &mut output,
            "residual_msat",
            Value::Int(int(&val(&snapshot, "residual_msat"))?),
        );
        set(&mut output, "residual_classification", classification);
        set(
            &mut output,
            "filed_report_impact_count",
            Value::Int(int(&val(row, "filed_report_impact_count"))?),
        );
    } else {
        set(
            &mut output,
            "candidate_wide_payload_excluded",
            Value::Bool(true),
        );
        set(
            &mut output,
            "excluded_fields",
            Value::List(
                [
                    "reason",
                    "retained_msat",
                    "residual_msat",
                    "residual_classification",
                    "filed_report_impact_count",
                ]
                .into_iter()
                .map(s)
                .collect(),
            ),
        );
    }
    set(&mut output, "gap_id", val(row, "gap_id"));
    Ok(output)
}

/// `list_audit_review_history(limit=500)`.
fn review_history(conn: &Connection, profile: &str, ids: Option<&[String]>) -> Result<Value> {
    let mut incomplete = BTreeSet::new();
    for row in rows(
        conn,
        "SELECT r.id, s.expected_source_count, s.expected_return_count, SUM(CASE WHEN x.role = 'source' THEN 1 ELSE 0 END) AS actual_source_count, SUM(CASE WHEN x.role = 'return' THEN 1 ELSE 0 END) AS actual_return_count FROM custody_gap_reviews r LEFT JOIN custody_gap_review_relation_sets s ON s.review_id = r.id LEFT JOIN custody_gap_review_transactions x ON x.review_id = r.id WHERE r.profile_id = ? GROUP BY r.id, s.expected_source_count, s.expected_return_count ORDER BY r.created_at, r.id",
        [profile],
    )? {
        let expected = [val(&row, "expected_source_count"), val(&row, "expected_return_count")];
        let complete = if expected.contains(&Value::Null) {
            false
        } else {
            let sources = count(&expected[0])?;
            let returns = count(&expected[1])?;
            sources > 0
                && returns > 0
                && sources == count(&val(&row, "actual_source_count"))?
                && returns == count(&val(&row, "actual_return_count"))?
        };
        if !complete {
            incomplete.insert(text(&row, "id"));
        }
    }
    let result = |total: usize, records: Vec<Value>, bounded: bool| {
        let mut output = dict([
            ("count", total.into()),
            ("returned", records.len().into()),
            ("truncated", Value::Bool(total > records.len())),
            (
                "scope_completeness",
                s(if incomplete.is_empty() {
                    "complete"
                } else {
                    "legacy_unscoped_history_present"
                }),
            ),
            ("records", Value::List(records)),
        ]);
        if !bounded {
            set(
                &mut output,
                "legacy_unscoped_review_count",
                incomplete.len().into(),
            );
        }
        output
    };
    let mut condition = "r.profile_id = ?".to_owned();
    let mut bound = vec![Sql::Text(profile.into())];
    let mut selected = None;
    if let Some(ids) = ids {
        let chosen: BTreeSet<String> = ids.iter().filter(|v| !v.is_empty()).cloned().collect();
        if chosen.is_empty() {
            return Ok(result(0, Vec::new(), true));
        }
        let list: Vec<String> = chosen.iter().cloned().collect();
        condition.push_str(&format!(" AND (EXISTS (SELECT 1 FROM custody_gap_review_transactions x WHERE x.review_id = r.id AND x.profile_id = r.profile_id AND {}) OR (NOT EXISTS (SELECT 1 FROM custody_gap_review_transactions x WHERE x.review_id = r.id) AND r.component_id IS NOT NULL AND EXISTS (SELECT 1 FROM custody_component_legs l WHERE l.profile_id = r.profile_id AND l.component_id = r.component_id AND {})))", in_list("x.transaction_id", &list), in_list("COALESCE(l.anchor_transaction_id, l.transaction_id)", &list)));
        bound.extend(list.iter().cloned().map(Sql::Text));
        bound.extend(list.iter().cloned().map(Sql::Text));
        selected = Some(chosen);
    }
    let total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM custody_gap_reviews r WHERE {condition}"),
            params_from_iter(bound.iter()),
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let mut page_bind = bound;
    page_bind.push(Sql::Integer(500));
    let found = rows(
        conn,
        &format!("SELECT r.*, c.revision AS component_revision, (SELECT COUNT(*) FROM custody_filed_report_impacts i WHERE i.review_id = r.id) AS filed_report_impact_count FROM custody_gap_reviews r LEFT JOIN custody_components c ON c.id = r.component_id WHERE {condition} ORDER BY r.created_at, r.id LIMIT ?"),
        params_from_iter(page_bind),
    )?;
    let complete = match &selected {
        Some(chosen) if !found.is_empty() => complete_reviews(
            conn,
            profile,
            chosen,
            &found.iter().map(|r| text(r, "id")).collect::<Vec<_>>(),
        )?,
        _ => BTreeSet::new(),
    };
    let records = found
        .iter()
        .map(|row| {
            review_history_row(
                row,
                selected.is_none() || complete.contains(&text(row, "id")),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(result(total as usize, records, selected.is_some()))
}

fn filed_snapshots(conn: &Connection, profile: &str) -> Result<Vec<Value>> {
    rows(
        conn,
        "SELECT * FROM filed_report_snapshots WHERE profile_id = ? ORDER BY period_start_year, period_end_year, created_at, id",
        [profile],
    )?
    .iter()
    .map(|row| {
        Ok(dict([
            ("id", val(row, "id")),
            ("workspace_id", val(row, "workspace_id")),
            ("profile_id", val(row, "profile_id")),
            ("report_kind", val(row, "report_kind")),
            ("report_state", val(row, "report_state")),
            ("period_start_year", Value::Int(int(&val(row, "period_start_year"))?)),
            ("period_end_year", Value::Int(int(&val(row, "period_end_year"))?)),
            ("content_sha256", val(row, "content_sha256")),
            (
                "classification_summary",
                json_object(&val(row, "classification_summary_json")),
            ),
            ("gain_summary", json_object(&val(row, "gain_summary_json"))),
            ("report_scope", json_object(&val(row, "report_scope_json"))),
            ("authored_source", val(row, "authored_source")),
            ("notes", val(row, "notes")),
            ("created_at", val(row, "created_at")),
        ]))
    })
    .collect()
}

fn custody_impacts(conn: &Connection, profile: &str, ids: Option<&[String]>) -> Result<Vec<Value>> {
    let mut condition = "profile_id = ?".to_owned();
    let mut bound = vec![Sql::Text(profile.into())];
    if let Some(ids) = ids {
        let selected: Vec<String> = ids
            .iter()
            .filter(|v| !v.is_empty())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if selected.is_empty() {
            return Ok(Vec::new());
        }
        condition.push_str(&format!(" AND EXISTS (SELECT 1 FROM custody_component_legs l WHERE l.profile_id = custody_filed_report_impacts.profile_id AND l.component_id = custody_filed_report_impacts.component_id AND {})", in_list("COALESCE(l.anchor_transaction_id, l.transaction_id)", &selected)));
        bound.extend(selected.into_iter().map(Sql::Text));
    }
    let impacts = rows(
        conn,
        &format!("SELECT * FROM custody_filed_report_impacts WHERE {condition} ORDER BY affected_period_start_year, created_at, id"),
        params_from_iter(bound),
    )?;
    let mut resolutions = BTreeMap::new();
    for row in rows(
        conn,
        "SELECT * FROM custody_filed_report_impact_resolutions WHERE profile_id = ? ORDER BY rebuilt_at, created_at, id",
        [profile],
    )? {
        resolutions.insert(
            text(&row, "impact_id"),
            dict([
                ("id", val(&row, "id")),
                ("impact_id", val(&row, "impact_id")),
                ("rebuilt_at", val(&row, "rebuilt_at")),
                (
                    "after_classification_summary",
                    json_object(&val(&row, "after_classification_summary_json")),
                ),
                (
                    "after_gain_summary",
                    json_object(&val(&row, "after_gain_summary_json")),
                ),
                (
                    "classification_changed",
                    Value::Bool(truthy(&val(&row, "classification_changed"))),
                ),
                ("gain_changed", Value::Bool(truthy(&val(&row, "gain_changed")))),
                ("amendment_status", val(&row, "amendment_status")),
                ("created_at", val(&row, "created_at")),
            ]),
        );
    }
    impacts
        .iter()
        .map(|row| {
            Ok(dict([
                ("id", val(row, "id")),
                (
                    "filed_report_snapshot_id",
                    val(row, "filed_report_snapshot_id"),
                ),
                ("component_id", val(row, "component_id")),
                ("review_id", val(row, "review_id")),
                ("gap_id", val(row, "gap_id")),
                (
                    "affected_period_start_year",
                    Value::Int(int(&val(row, "affected_period_start_year"))?),
                ),
                (
                    "affected_period_end_year",
                    Value::Int(int(&val(row, "affected_period_end_year"))?),
                ),
                (
                    "before_classification_summary",
                    json_object(&val(row, "before_classification_summary_json")),
                ),
                (
                    "after_classification_summary",
                    json_object(&val(row, "after_classification_summary_json")),
                ),
                (
                    "before_gain_summary",
                    json_object(&val(row, "before_gain_summary_json")),
                ),
                (
                    "after_gain_summary",
                    json_object(&val(row, "after_gain_summary_json")),
                ),
                ("amendment_warning", val(row, "amendment_warning")),
                (
                    "resolution",
                    resolutions
                        .get(&text(row, "id"))
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                ("created_at", val(row, "created_at")),
            ]))
        })
        .collect()
}

fn bounded_snapshot(snapshot: &Value) -> Value {
    dict([
        ("id", val(snapshot, "id")),
        ("report_kind", val(snapshot, "report_kind")),
        ("report_state", val(snapshot, "report_state")),
        ("period_start_year", val(snapshot, "period_start_year")),
        ("period_end_year", val(snapshot, "period_end_year")),
        ("content_sha256", val(snapshot, "content_sha256")),
        ("authored_source", val(snapshot, "authored_source")),
        ("created_at", val(snapshot, "created_at")),
        ("report_wide_payload_excluded", Value::Bool(true)),
        (
            "excluded_fields",
            Value::List(
                [
                    "classification_summary",
                    "gain_summary",
                    "report_scope",
                    "notes",
                ]
                .into_iter()
                .map(s)
                .collect(),
            ),
        ),
    ])
}

fn bounded_impact(impact: &Value) -> Value {
    let resolution = match val(impact, "resolution") {
        resolution @ Value::Dict(_) => dict([
            ("id", val(&resolution, "id")),
            ("impact_id", val(&resolution, "impact_id")),
            ("rebuilt_at", val(&resolution, "rebuilt_at")),
            (
                "classification_changed",
                val(&resolution, "classification_changed"),
            ),
            ("gain_changed", val(&resolution, "gain_changed")),
            ("amendment_status", val(&resolution, "amendment_status")),
            ("created_at", val(&resolution, "created_at")),
            ("report_wide_payload_excluded", Value::Bool(true)),
            (
                "excluded_fields",
                Value::List(
                    ["after_classification_summary", "after_gain_summary"]
                        .into_iter()
                        .map(s)
                        .collect(),
                ),
            ),
        ]),
        _ => Value::Null,
    };
    dict([
        ("id", val(impact, "id")),
        (
            "filed_report_snapshot_id",
            val(impact, "filed_report_snapshot_id"),
        ),
        ("component_id", val(impact, "component_id")),
        ("review_id", val(impact, "review_id")),
        ("gap_id", val(impact, "gap_id")),
        (
            "affected_period_start_year",
            val(impact, "affected_period_start_year"),
        ),
        (
            "affected_period_end_year",
            val(impact, "affected_period_end_year"),
        ),
        ("amendment_warning", val(impact, "amendment_warning")),
        ("resolution", resolution),
        ("created_at", val(impact, "created_at")),
        ("report_wide_payload_excluded", Value::Bool(true)),
        (
            "excluded_fields",
            Value::List(
                [
                    "before_classification_summary",
                    "after_classification_summary",
                    "before_gain_summary",
                    "after_gain_summary",
                ]
                .into_iter()
                .map(s)
                .collect(),
            ),
        ),
    ])
}

fn excluded_section() -> Value {
    dict([
        ("status", s("excluded_from_bounded_scope")),
        ("reason", s("complete_custody_review_boundary_required")),
    ])
}

/// `custody_ai_audit.redacted_audit_summary(limit=500)`: hash-only history.
fn ai_assistance(conn: &Connection, profile: &str, ids: Option<&[String]>) -> Result<Value> {
    let mut condition = "profile_id = ?".to_owned();
    let mut bound = vec![Sql::Text(profile.into())];
    if let Some(ids) = ids {
        let selected: Vec<String> = ids
            .iter()
            .filter(|v| !v.is_empty())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if selected.is_empty() {
            return Ok(dict([
                ("records", Value::List(vec![])),
                ("count", 0.into()),
                ("truncated", Value::Bool(false)),
                ("raw_proposals_included", Value::Bool(false)),
                ("replicated", Value::Bool(false)),
                ("chat_history_required", Value::Bool(false)),
            ]));
        }
        condition.push_str(&format!(" AND component_id IN (SELECT DISTINCT component_id FROM custody_component_legs WHERE {})", in_list("anchor_transaction_id", &selected)));
        bound.extend(selected.into_iter().map(Sql::Text));
    }
    bound.push(Sql::Integer(501));
    let mut found = rows(
        conn,
        &format!("SELECT id, tool_name, daemon_kind, provider_kind, gap_id, candidate_fingerprint, facts_sha256, user_edited, consent_decision, consent_requested_at, consent_decided_at, execution_status, execution_code, result_sha256, review_id, component_id, created_at FROM custody_ai_assistance_audits WHERE {condition} ORDER BY created_at DESC, id DESC LIMIT ?"),
        params_from_iter(bound),
    )?;
    let truncated = found.len() > 500;
    found.truncate(500);
    for row in &mut found {
        let edited = truthy(&val(row, "user_edited"));
        set(row, "user_edited", Value::Bool(edited));
    }
    let count = found.len();
    Ok(dict([
        ("records", Value::List(found)),
        ("count", count.into()),
        ("truncated", Value::Bool(truncated)),
        ("raw_proposals_included", Value::Bool(false)),
        ("replicated", Value::Bool(false)),
        ("chat_history_required", Value::Bool(false)),
    ]))
}

/// `review_workflow.audit_receipt_summary(limit=500)`.
fn receipts(conn: &Connection, profile: &str, ids: Option<&[String]>) -> Result<Value> {
    let mut condition = "profile_id = ?".to_owned();
    let mut bound = vec![Sql::Text(profile.into())];
    if let Some(ids) = ids {
        let selected: Vec<String> = ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if selected.is_empty() {
            return Ok(dict([
                ("records", Value::List(vec![])),
                ("count", 0.into()),
                ("truncated", Value::Bool(false)),
                ("raw_proposals_included", Value::Bool(false)),
            ]));
        }
        condition.push_str(&format!(
            " AND EXISTS (SELECT 1 FROM json_each(receipt_json, '$.transaction_ids') WHERE {})",
            in_list("value", &selected)
        ));
        bound.extend(selected.into_iter().map(Sql::Text));
    }
    bound.push(Sql::Integer(501));
    let found = rows(
        conn,
        &format!("SELECT id,artifact_digest,receipt_json,created_at FROM review_workflow_receipts WHERE {condition} ORDER BY created_at DESC,id DESC LIMIT ?"),
        params_from_iter(bound),
    )?;
    let mut records = Vec::new();
    for row in found.iter().take(500) {
        let receipt = kassiber_pytext::loads(&text(row, "receipt_json"))
            .map_err(|error| AppError::new("internal_error", error.to_string()))?;
        let field = |key: &str| {
            get(&receipt, key)
                .cloned()
                .ok_or_else(|| AppError::new("internal_error", format!("'{key}'")))
        };
        records.push(dict([
            ("id", val(row, "id")),
            ("artifact_digest", val(row, "artifact_digest")),
            ("receipt_digest", s(sha256_hex(&canonical(&receipt)?))),
            ("created_at", val(row, "created_at")),
            ("status", field("status")?),
            ("base_input_version", field("base_input_version")?),
            ("result_input_version", field("result_input_version")?),
        ]));
    }
    let count = records.len();
    Ok(dict([
        ("records", Value::List(records)),
        ("count", count.into()),
        ("truncated", Value::Bool(found.len() > 500)),
        ("raw_proposals_included", Value::Bool(false)),
    ]))
}

fn schema_migration_audits(conn: &Connection) -> Result<Vec<Value>> {
    rows(
        conn,
        "SELECT migration_name, schema_version, impact_json, created_at FROM schema_migration_audits ORDER BY created_at, migration_name",
        [],
    )?
    .iter()
    .map(|row| {
        let raw = match val(row, "impact_json") {
            Value::Str(text) => kassiber_pytext::loads(text.as_str().unwrap_or_default())
                .unwrap_or_else(|_| dict::<&str>([])),
            _ => dict::<&str>([]),
        };
        let (version, migration) = if matches!(raw, Value::Dict(_)) {
            let migration: String = text_or(&raw, "migration").chars().take(64).collect();
            (Value::Int(int(&val(&raw, "schema_version"))?), s(migration))
        } else {
            (0.into(), s(""))
        };
        Ok(dict([
            ("migration_name", val(row, "migration_name")),
            ("schema_version", Value::Int(int(&val(row, "schema_version"))?)),
            ("scope", s("database_schema_only")),
            (
                "impact",
                dict([("schema_version", version), ("migration", migration)]),
            ),
            ("created_at", val(row, "created_at")),
        ]))
    })
    .collect()
}

/// `_transaction_refs_from_case`: the transactions of a saved source-of-funds
/// case snapshot, refusing a snapshot whose hash no longer matches.
fn case_scope(conn: &Connection, profile: &str, case_ref: &str) -> Result<(Vec<String>, Value)> {
    let Some(row) = rows(
        conn,
        "SELECT * FROM source_funds_cases WHERE profile_id = ? AND id = ?",
        [profile, case_ref],
    )?
    .into_iter()
    .next() else {
        return Err(AppError::new(
            "not_found",
            format!("Source-funds case '{case_ref}' not found"),
        ));
    };
    let snapshot = kassiber_pytext::loads(&text(&row, "snapshot_json"))
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    if sha256_hex(&canonical(&snapshot)?) != text(&row, "snapshot_hash") {
        return Err(AppError::new(
            "snapshot_integrity_error",
            "Source-funds case snapshot hash does not match stored payload",
        ));
    }
    let mut refs = BTreeSet::new();
    if let Value::List(nodes) = val(&val(&snapshot, "graph"), "nodes") {
        for node in &nodes {
            if matches!(node, Value::Dict(_))
                && text(node, "node_type") == "transaction"
                && truthy(&val(node, "transaction_id"))
            {
                refs.insert(text(node, "transaction_id"));
            }
        }
    }
    let target = val(&snapshot, "target");
    if matches!(target, Value::Dict(_)) && truthy(&val(&target, "transaction_id")) {
        refs.insert(text(&target, "transaction_id"));
    }
    let or_text = |v: Value| if truthy(&v) { v } else { s("") };
    let case = dict([
        ("id", s(case_ref)),
        (
            "snapshot_hash",
            or_text(val(&val(&snapshot, "case"), "snapshot_hash")),
        ),
        ("reveal_mode", or_text(val(&snapshot, "reveal_mode"))),
        (
            "target_transaction_id",
            or_text(val(&target, "transaction_id")),
        ),
        ("target_label", or_text(val(&target, "label"))),
        (
            "exportable",
            Value::Bool(truthy(&val(&val(&snapshot, "explain_gates"), "exportable"))),
        ),
    ]);
    Ok((refs.into_iter().collect(), case))
}

/// `build_evidence_summary(conn, data_root, None, None, hooks, ...)` for the
/// active book. `attachments_root` must already exist.
pub fn summary(conn: &Connection, attachments_root: &Path, options: &Options) -> Result<Value> {
    let (workspace, profile) = kassiber_setup::references::scope(conn, &dict::<&str>([]))?;
    let profile_id = text(&profile, "id");
    let mut scope = dict([("type", s("active_profile"))]);
    let mut refs = options.transaction_refs.clone();
    if let Some(case) = &options.source_funds_case {
        let (case_refs, case) = case_scope(conn, &profile_id, case)?;
        refs = Some(case_refs);
        scope = dict([("type", s("source_funds_case")), ("case", case)]);
    } else if let Some(refs) = &options.transaction_refs {
        scope = dict([
            ("type", s("transactions")),
            ("transaction_count", refs.len().into()),
        ]);
    }
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0",
            [&profile_id],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let profile_freshness = kassiber_workspace::readiness::evaluate_freshness(&profile, active)?;
    let bounded = refs.is_some();
    let freshness = if bounded {
        dict([
            ("status", s("excluded_from_bounded_scope")),
            ("needs_processing", Value::Bool(false)),
            ("reason", s("Profile-wide journal freshness excluded")),
            ("scope", s("selected_transactions")),
        ])
    } else {
        profile_freshness.clone()
    };
    let mut readiness_freshness = profile_freshness.clone();
    if !options.include_journal_state {
        set(
            &mut readiness_freshness,
            "needs_processing",
            Value::Bool(false),
        );
        set(
            &mut readiness_freshness,
            "reason",
            s("journal state excluded by export options"),
        );
    }
    let profile_custody = custody_quantity_readiness(
        conn,
        &profile_id,
        &py_str(&val(&profile_freshness, "status")),
        options.include_journal_state,
    )?;
    let custody_quantity = if bounded {
        dict([
            ("status", s("excluded_from_bounded_scope")),
            ("status_text", s("Profile-wide custody totals excluded")),
            ("scope", s("selected_transactions")),
            (
                "qualification",
                s("Only custody facts durably anchored to the selected transactions are included in this package."),
            ),
        ])
    } else {
        profile_custody.clone()
    };
    let tx_rows = match &refs {
        Some(refs) => {
            let mut selected: Vec<Value> = Vec::new();
            for reference in refs {
                let stripped = kassiber_pytext::strip(reference);
                if stripped.is_empty() {
                    return Err(AppError::new(
                        "validation",
                        "transaction references must be non-empty strings",
                    ));
                }
                let row = resolve_transaction(conn, &profile_id, stripped)?;
                if !selected.iter().any(|r| val(r, "id") == val(&row, "id")) {
                    selected.push(row);
                }
            }
            selected.sort_by(|a, b| {
                (text_or(a, "occurred_at"), text_or(a, "created_at"), text(a, "id")).cmp(&(
                    text_or(b, "occurred_at"),
                    text_or(b, "created_at"),
                    text(b, "id"),
                ))
            });
            selected
        }
        None => rows(
            conn,
            "SELECT * FROM transactions WHERE profile_id = ? ORDER BY occurred_at ASC, created_at ASC, id ASC",
            [&profile_id],
        )?,
    };
    let tx_ids: Vec<String> = tx_rows.iter().map(|r| text(r, "id")).collect();
    let processed = count(&val(&profile, "last_processed_input_version"))?;
    let history = if options.include_edit_history {
        kassiber_transactions::metadata::history_for_transaction_ids(
            conn,
            &profile_id,
            processed,
            &tx_ids,
        )?
    } else {
        Vec::new()
    };
    let mut edit_count: i64 = history.iter().map(|(_, events)| events.len() as i64).sum();
    if !options.include_edit_history && !tx_ids.is_empty() {
        edit_count = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM transaction_edit_events WHERE profile_id = ? AND {}",
                    in_list("transaction_id", &tx_ids)
                ),
                params_from_iter(bind(&[&profile_id], &tx_ids)),
                |row| row.get(0),
            )
            .map_err(sql_error)?;
    }
    let mut transactions = Vec::new();
    let mut warning_counts: BTreeMap<String, i64> = BTreeMap::new();
    for tx in &tx_rows {
        let id = text(tx, "id");
        let tags = tags(conn, &id)?;
        let direct = rows(
            conn,
            "SELECT * FROM attachments WHERE profile_id = ? AND transaction_id = ? ORDER BY created_at ASC, id ASC",
            [&profile_id, &id],
        )?
        .iter()
        .map(|row| attachment_summary(row, attachments_root))
        .collect::<Result<Vec<_>>>()?;
        let links = if options.include_review_state {
            source_links(conn, &profile_id, &id, attachments_root)?
        } else {
            Vec::new()
        };
        let quarantine = if options.include_journal_state {
            quarantine(conn, &profile_id, &id)?
        } else {
            Value::Null
        };
        let mut warnings = evidence_warnings(
            tx,
            &tags,
            &direct,
            &links,
            &readiness_freshness,
            &quarantine,
            options.include_review_state,
        );
        if let Some(item) = custody_warning(tx, &profile_custody, bounded) {
            warnings.push(item);
        }
        if !options.include_review_state {
            warnings.push(warning(
                "review_state_excluded",
                "info",
                "Source-of-funds review state was excluded by export options.",
                None,
            ));
        }
        if !options.include_journal_state {
            warnings.push(warning(
                "journal_state_excluded",
                "info",
                "Journal and quarantine state was excluded by export options.",
                None,
            ));
        }
        for item in &warnings {
            *warning_counts.entry(text(item, "code")).or_default() += 1;
        }
        let severity = |level: &str| warnings.iter().any(|w| text(w, "severity") == level);
        let status = if severity("blocker") {
            "blocked"
        } else if severity("warning") {
            "warning"
        } else {
            "ready"
        };
        let mut item = dict([
            ("transaction", transaction_summary(tx, &tags)?),
            (
                "readiness",
                dict([("status", s(status)), ("warnings", Value::List(warnings))]),
            ),
            ("direct_attachments", Value::List(direct)),
            ("source_funds_links", Value::List(links)),
        ]);
        if options.include_journal_state {
            set(
                &mut item,
                "journal",
                dict([
                    (
                        "entries",
                        Value::List(journal_entries(conn, &profile_id, &id)?),
                    ),
                    ("quarantine", quarantine),
                ]),
            );
        }
        if options.include_edit_history {
            let events = history
                .iter()
                .find(|(key, _)| *key == id)
                .map(|(_, events)| events.clone())
                .unwrap_or_default();
            set(&mut item, "edit_history", Value::List(events));
        }
        transactions.push(item);
    }
    let selected_ids = bounded.then_some(tx_ids.as_slice());
    let authored = authored_evidence(conn, &profile_id, selected_ids)?;
    let history_section = review_history(conn, &profile_id, selected_ids)?;
    let snapshots = filed_snapshots(conn, &profile_id)?;
    let impacts = custody_impacts(conn, &profile_id, selected_ids)?;
    let mut excluded = bounded
        && (truthy(&val(&history_section, "truncated"))
            || match val(&history_section, "records") {
                Value::List(records) => records
                    .iter()
                    .any(|r| truthy(&val(r, "candidate_wide_payload_excluded"))),
                _ => false,
            });
    if bounded && !impacts.is_empty() {
        let candidates: BTreeSet<String> = impacts
            .iter()
            .map(|i| py_str(&val(i, "review_id")))
            .collect();
        let selected: BTreeSet<String> = tx_ids.iter().filter(|v| !v.is_empty()).cloned().collect();
        let complete = complete_reviews(
            conn,
            &profile_id,
            &selected,
            &candidates.iter().cloned().collect::<Vec<_>>(),
        )?;
        excluded = excluded || complete != candidates;
    }
    let (snapshot_section, impact_section, resolution_section, counts) = if excluded {
        (
            excluded_section(),
            excluded_section(),
            excluded_section(),
            None,
        )
    } else {
        let (snapshot_records, impact_records) = if bounded {
            let referenced: BTreeSet<String> = impacts
                .iter()
                .map(|i| py_str(&val(i, "filed_report_snapshot_id")))
                .collect();
            (
                snapshots
                    .iter()
                    .filter(|s| referenced.contains(&py_str(&val(s, "id"))))
                    .map(bounded_snapshot)
                    .collect::<Vec<_>>(),
                impacts.iter().map(bounded_impact).collect::<Vec<_>>(),
            )
        } else {
            (snapshots, impacts)
        };
        let resolutions: Vec<Value> = impact_records
            .iter()
            .filter_map(|i| get(i, "resolution").filter(|r| **r != Value::Null).cloned())
            .collect();
        let counts = (
            snapshot_records.len(),
            impact_records.len(),
            resolutions.len(),
        );
        (
            Value::List(snapshot_records),
            Value::List(impact_records),
            Value::List(resolutions),
            Some(counts),
        )
    };
    let ai = ai_assistance(conn, &profile_id, refs.as_ref().map(|_| tx_ids.as_slice()))?;
    let receipt_section = receipts(conn, &profile_id, selected_ids)?;
    let migrations = schema_migration_audits(conn)?;
    let readiness_count = |status: &str| {
        transactions
            .iter()
            .filter(|item| text(&val(item, "readiness"), "status") == status)
            .count()
    };
    let mut summary = dict([
        ("transaction_count", transactions.len().into()),
        ("ready_count", readiness_count("ready").into()),
        ("blocked_count", readiness_count("blocked").into()),
        ("warning_count", readiness_count("warning").into()),
        (
            "warning_codes",
            dict(
                warning_counts
                    .into_iter()
                    .map(|(k, v)| (k, Value::Int(v.into()))),
            ),
        ),
        ("edit_history_event_count", Value::Int(edit_count.into())),
        (
            "edit_history_included",
            Value::Bool(options.include_edit_history),
        ),
        ("custody_gap_status", val(&custody_quantity, "status")),
        ("custody_issue_count", val(&custody_quantity, "issue_count")),
        (
            "authored_custody_evidence_hash_count",
            val(&authored, "snapshot_count"),
        ),
        (
            "custody_gap_review_event_count",
            val(&history_section, "returned"),
        ),
        ("custody_ai_assistance_count", val(&ai, "count")),
        (
            "review_workflow_receipt_count",
            val(&receipt_section, "count"),
        ),
        ("schema_migration_audit_count", migrations.len().into()),
    ]);
    if let Some((snapshots, impacts, resolutions)) = counts {
        set(
            &mut summary,
            "filed_report_snapshot_count",
            snapshots.into(),
        );
        set(
            &mut summary,
            "custody_filed_report_impact_count",
            impacts.into(),
        );
        set(
            &mut summary,
            "custody_filed_report_impact_resolution_count",
            resolutions.into(),
        );
    }
    let pool = match get(&profile, "cost_basis_pool_scope") {
        Some(v) if truthy(v) => py_str(v),
        _ => "global".into(),
    };
    Ok(dict([
        ("schema_version", SCHEMA_VERSION.into()),
        (
            "workspace",
            dict([
                ("id", val(&workspace, "id")),
                ("label", val(&workspace, "label")),
            ]),
        ),
        (
            "profile",
            dict([
                ("id", val(&profile, "id")),
                ("label", val(&profile, "label")),
                ("cost_basis_pool_scope", s(pool)),
            ]),
        ),
        ("scope", scope),
        ("journal_freshness", freshness),
        ("custody_quantity", custody_quantity),
        ("authored_custody_evidence", authored),
        ("filed_report_snapshots", snapshot_section),
        ("custody_filed_report_impacts", impact_section),
        (
            "custody_filed_report_impact_resolutions",
            resolution_section,
        ),
        ("custody_gap_review_history", history_section),
        ("custody_ai_assistance", ai),
        ("review_workflow_receipts", receipt_section),
        ("schema_migration_audits", Value::List(migrations)),
        ("transactions", Value::List(transactions)),
        ("summary", summary),
        (
            "excluded_sensitive_material",
            Value::List(
                SENSITIVE_MATERIAL_EXCLUSIONS
                    .iter()
                    .map(|v| s(*v))
                    .collect(),
            ),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_safety_matches_urllib_for_credentials_queries_and_fragments() {
        let facts = |raw: &str| url_safety(raw).unwrap();
        assert_eq!(
            facts("https://user:pw@Example.COM:8443/x?a=1"),
            (
                true,
                "embedded credentials".into(),
                "example.com".into(),
                "https".into()
            )
        );
        assert_eq!(
            facts("https://Example.COM:8443/x?Api%5FKey=1"),
            (
                true,
                "secret-like query parameter".into(),
                "Example.COM:8443".into(),
                "https".into()
            )
        );
        assert_eq!(
            facts("https://h/x#?sig=1"),
            (
                true,
                "secret-like URL fragment".into(),
                "h".into(),
                "https".into()
            )
        );
        assert_eq!(
            facts("https://h/x#token"),
            (
                true,
                "secret-like URL fragment".into(),
                "h".into(),
                "https".into()
            )
        );
        assert_eq!(
            facts("https://:@h/"),
            (false, String::new(), ":@h".into(), "https".into())
        );
        assert_eq!(
            facts("  "),
            (false, String::new(), String::new(), String::new())
        );
        assert!(url_safety("https://[::1/x").is_err());
    }

    #[test]
    fn unquote_decodes_percent_runs_with_replacement() {
        assert_eq!(unquote("a%20b%zz%4"), "a b%zz%4");
        assert_eq!(unquote("%E2%82%AC%FF"), "€\u{FFFD}");
    }
}

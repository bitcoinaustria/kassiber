//! The AI read paths over native services (`_execute_read_only_ai_tool`'s
//! branches) and the read-side projections: the evidence and source-of-funds
//! key filters, the wallet UTXO and profile projections, the loan and
//! capital-gains trims, and the AI-safe egress snapshot.

use std::sync::OnceLock;

use fancy_regex::Regex;
use kassiber_envelope::{dict, get, s, set, AppError};
use kassiber_pyjson::{dumps, BigInt, Key, Options, PyStr, Value};

use crate::catalog::ToolEntry;
use crate::executor::Book;
use crate::pyre;
use crate::redact::redact_ai_tool_result;
use crate::validate::py_eq;
use crate::ToolError;

/// Tools whose Python builder takes no arguments: the native read gets none.
const ARGUMENTLESS: [&str; 9] = [
    "status",
    "ui.wallets.list",
    "ui.backends.list",
    "ui.profiles.snapshot",
    "ui.reports.capital_gains",
    "ui.reports.balance_sheet",
    "ui.reports.portfolio_summary",
    "ui.rates.summary",
    "ui.maintenance.settings",
];

/// `_SOURCE_FUNDS_READ_AI_DAEMON_KINDS`.
const SOURCE_FUNDS_READS: [&str; 8] = [
    "ui.source_funds.preview",
    "ui.source_funds.review_context",
    "ui.source_funds.request_input",
    "ui.source_funds.sources.list",
    "ui.source_funds.links.list",
    "ui.source_funds.evidence.list",
    "ui.source_funds.coverage",
    "ui.source_funds.cases.list",
];

/// The commercial reads whose payloads pass the evidence filter.
const COMMERCIAL_READS: [&str; 5] = [
    "ui.transactions.commercial_context",
    "ui.btcpay.provenance.list",
    "ui.btcpay.provenance.suggest",
    "ui.btcpay.provenance.links",
    "ui.documents.list",
];

/// `_SOURCE_FUNDS_AI_REDACTED_KEYS`.
const SOURCE_FUNDS_REDACTED_KEYS: [&str; 5] =
    ["source_url", "stored_relpath", "file", "dir", "path"];

/// `_EVIDENCE_AI_REDACTED_KEYS`.
const EVIDENCE_REDACTED_KEYS: [&str; 8] = [
    "source_url",
    "stored_relpath",
    "file",
    "dir",
    "file_path",
    "path",
    "url",
    "manifest",
];

/// `_wallet_utxo_row_for_ai`: address, script and derivation fields.
const UTXO_PRIVATE_FIELDS: [&str; 9] = [
    "address",
    "address_label",
    "script_pubkey",
    "branch_label",
    "branch_index",
    "address_index",
    "derivation_path",
    "derivation_paths",
    "anon_history",
];

fn suffix_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| pyre::compile(r"(?:^|_)(?:url|path|file|dir)$"))
}

fn scheme_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| pyre::compile(r"(?i)^[a-z][a-z0-9+.-]*://"))
}

fn drive_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| pyre::compile(r"^[A-Za-z]:[\\/]"))
}

/// `_evidence_key_is_sensitive(key)`.
pub fn evidence_key_is_sensitive(key: &Key) -> bool {
    let Key::Str(key) = key else {
        return false;
    };
    if EVIDENCE_REDACTED_KEYS.iter().any(|name| key.is(name)) {
        return true;
    }
    let text = pyre::lossy(key);
    let lowered = kassiber_pytext::lower(&text);
    suffix_pattern().is_match(&lowered).unwrap_or(true)
        || ["Url", "URL", "Path", "File", "Dir"]
            .iter()
            .any(|suffix| text.ends_with(suffix))
}

/// `_screen_context_contains_path_or_url(value)`.
pub fn contains_path_or_url(value: &Value) -> bool {
    match value {
        Value::Dict(pairs) => pairs
            .iter()
            .any(|(key, item)| evidence_key_is_sensitive(key) || contains_path_or_url(item)),
        Value::List(items) => items.iter().any(contains_path_or_url),
        Value::Str(text) => {
            let text = pyre::lossy(text);
            let stripped = kassiber_pytext::strip(&text);
            scheme_pattern().is_match(stripped).unwrap_or(true)
                || ["/", "~/", "\\\\"]
                    .iter()
                    .any(|prefix| stripped.starts_with(prefix))
                || drive_pattern().is_match(stripped).unwrap_or(true)
        }
        _ => false,
    }
}

fn filter_keys(value: &Value, drop: &dyn Fn(&Key) -> bool) -> Value {
    match value {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .iter()
                .filter(|(key, _)| !drop(key))
                .map(|(key, item)| (key.clone(), filter_keys(item, drop)))
                .collect(),
        ),
        Value::List(items) => {
            Value::List(items.iter().map(|item| filter_keys(item, drop)).collect())
        }
        Value::Str(_) if contains_path_or_url(value) => s("<redacted>"),
        other => other.clone(),
    }
}

/// `_redact_evidence_payload_for_ai(value)`: local paths and URL targets
/// go; evidence labels and state stay.
pub fn redact_evidence_for_ai(value: &Value) -> Value {
    filter_keys(value, &evidence_key_is_sensitive)
}

/// `_redact_source_funds_payload_for_ai(value)`.
pub fn redact_source_funds_for_ai(value: &Value) -> Value {
    filter_keys(value, &|key| {
        matches!(key, Key::Str(name) if SOURCE_FUNDS_REDACTED_KEYS.iter().any(|drop| name.is(drop)))
            || evidence_key_is_sensitive(key)
    })
}

/// `_wallet_utxo_row_for_ai` over `build_wallet_utxos_snapshot_for_ai`.
pub fn wallet_utxos_for_ai(payload: Value) -> Value {
    let rows = match get(&payload, "utxos") {
        Some(Value::List(rows)) => rows
            .iter()
            .filter_map(|row| match row {
                Value::Dict(fields) => Some(Value::Dict(
                    fields
                        .iter()
                        .filter(|(key, _)| !matches!(key, Key::Str(name) if UTXO_PRIVATE_FIELDS.iter().any(|f| name.is(f))))
                        .cloned()
                        .collect(),
                )),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let mut payload = payload;
    set(&mut payload, "utxos", Value::List(rows));
    payload
}

/// `_profiles_snapshot_for_ai`'s checks before the snapshot is read.
fn profiles_scope(state: &Value) -> Result<PyStr, ToolError> {
    if get(state, "cross_book_read_allowed") != Some(&Value::Bool(true)) {
        return Err(AppError::new(
            "validation",
            "A profile list requires an explicit all-books request",
        )
        .into());
    }
    match get(state, "scope_workspace_id") {
        Some(Value::Str(workspace)) if workspace.code_points().next().is_some() => {
            Ok(workspace.clone())
        }
        _ => Err(AppError::new(
            "stale_context",
            "The AI profile list is missing its original workspace scope",
        )
        .into()),
    }
}

/// `_profiles_snapshot_for_ai`: only the workspace frozen for the turn.
fn profiles_for_ai(snapshot: &Value, workspace: &PyStr) -> Result<Value, ToolError> {
    let workspaces: Vec<Value> = match get(snapshot, "workspaces") {
        Some(Value::List(items)) => items
            .iter()
            .filter(|item| matches!(item, Value::Dict(_)))
            .filter(|item| matches!(get(item, "id"), Some(Value::Str(id)) if id == workspace))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    if workspaces.len() != 1 {
        return Err(AppError::new(
            "stale_context",
            "The chat's original workspace is no longer available",
        )
        .into());
    }
    Ok(dict([
        ("workspaces", Value::List(workspaces)),
        ("activeWorkspaceId", Value::Str(workspace.clone())),
        (
            "activeProfileId",
            get(snapshot, "activeProfileId")
                .cloned()
                .unwrap_or(Value::Null),
        ),
    ]))
}

/// `int(raw)` as `_coerce_positive_int` calls it; `None` where Python
/// raises `TypeError`/`ValueError`, `Err` where it raises anything else.
fn python_int(raw: &Value) -> Result<Option<BigInt>, ToolError> {
    Ok(match raw {
        Value::Int(n) => Some(n.clone()),
        Value::Bool(b) => Some(BigInt::from(u8::from(*b))),
        Value::Float(f) if f.is_nan() || f.is_infinite() => return Err(ToolError::Unexpected),
        Value::Float(f) => Some(
            format!("{:.0}", f.trunc())
                .parse()
                .map_err(|_| ToolError::Unexpected)?,
        ),
        Value::Str(text) => text
            .as_str()
            .and_then(|text| kassiber_pytext::parse_int(text).ok()),
        _ => None,
    })
}

/// `_coerce_positive_int(raw, label, maximum=...)`.
pub fn coerce_positive_int(raw: &Value, label: &str, maximum: i64) -> Result<i64, ToolError> {
    let Some(value) = python_int(raw)? else {
        return Err(
            AppError::new("validation", format!("{label} must be an integer"))
                .with_details(dict([("value", raw.clone())]))
                .into(),
        );
    };
    if value < BigInt::from(1) {
        return Err(
            AppError::new("validation", format!("{label} must be positive"))
                .with_details(dict([("value", raw.clone())]))
                .into(),
        );
    }
    Ok(i64::try_from(value.min(BigInt::from(maximum))).expect("bounded by maximum"))
}

fn list_field(payload: &Value, key: &str) -> Result<Vec<Value>, ToolError> {
    match get(payload, key) {
        Some(Value::List(items)) => Ok(items.clone()),
        // `full_snapshot["marks"]` on a missing key, then slicing it.
        _ => Err(ToolError::Unexpected),
    }
}

/// The AI loan list: its own row limit and summary.
fn loans_for_ai(snapshot: Value, limit: i64) -> Result<Value, ToolError> {
    let marks = list_field(&snapshot, "marks")?;
    let open_locks = list_field(&snapshot, "open_locks")?;
    let cap = usize::try_from(limit).unwrap_or(usize::MAX);
    let truncated = marks.len() > cap || open_locks.len() > cap;
    let summary = dict([
        ("marks", Value::from(marks.len())),
        ("open_locks", Value::from(open_locks.len())),
        ("row_limit", Value::from(limit)),
        ("truncated", Value::Bool(truncated)),
    ]);
    let mut payload = snapshot;
    set(
        &mut payload,
        "marks",
        Value::List(marks.into_iter().take(cap).collect()),
    );
    set(
        &mut payload,
        "open_locks",
        Value::List(open_locks.into_iter().take(cap).collect()),
    );
    set(&mut payload, "summary", summary);
    Ok(payload)
}

/// `lot.pop("explanationReference", None)` for every capital-gains lot.
fn capital_gains_for_ai(mut payload: Value) -> Result<Value, ToolError> {
    if let Value::Dict(fields) = &mut payload {
        if let Some((_, Value::List(lots))) = fields
            .iter_mut()
            .rev()
            .find(|(key, _)| *key == Key::from("lots"))
        {
            for lot in lots {
                let Value::Dict(lot) = lot else {
                    return Err(ToolError::Unexpected);
                };
                lot.retain(|(key, _)| *key != Key::from("explanationReference"));
            }
        }
    }
    Ok(payload)
}

fn unknown_fields(arguments: &Value, allowed: &[&str]) -> Vec<Value> {
    let mut unknown: Vec<PyStr> = match arguments {
        Value::Dict(pairs) => pairs
            .iter()
            .filter_map(|(key, _)| match key {
                Key::Str(name) if !allowed.iter().any(|allowed| name.is(allowed)) => {
                    Some(name.clone())
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    unknown.sort();
    unknown.into_iter().map(Value::Str).collect()
}

fn refuse_unknown(arguments: &Value, allowed: &[&str], kind: &str) -> Result<(), ToolError> {
    let unknown = unknown_fields(arguments, allowed);
    if unknown.is_empty() {
        return Ok(());
    }
    Err(
        AppError::new("validation", format!("{kind} received unsupported fields"))
            .with_details(dict([("unknown", Value::List(unknown))]))
            .into(),
    )
}

fn has_text(arguments: &Value, key: &str) -> bool {
    matches!(get(arguments, key), Some(Value::Str(text))
        if kassiber_pytext::strip(&pyre::lossy(text)).chars().next().is_some())
}

/// `len(json.dumps({"source_funds_recipe": recipe}))`.
fn recipe_chars(recipe: &Value) -> Result<usize, ToolError> {
    dumps(
        &kassiber_envelope::json_ready(dict([("source_funds_recipe", recipe.clone())])),
        &Options::new(),
    )
    .map(|text| text.len())
    .map_err(|_| ToolError::Unexpected)
}

const HISTORY_FIELDS: [&str; 12] = [
    "transaction",
    "source",
    "field_family",
    "field",
    "pricing_only",
    "ai_only",
    "stale_only",
    "start",
    "end",
    "cursor",
    "limit",
    "include_stale",
];

const ACTIVITY_FIELDS: [&str; 13] = [
    "transaction",
    "wallet",
    "source",
    "field_family",
    "field",
    "pricing_only",
    "ai_only",
    "stale_only",
    "start",
    "end",
    "cursor",
    "limit",
    "include_stale",
];

/// One read tool over its native service, with the AI path's own checks
/// and projections, in Python's order.
pub fn read_for_ai(
    entry: &ToolEntry,
    kind: &str,
    arguments: &Value,
    state: &Value,
    book: &mut dyn Book,
) -> Result<Value, ToolError> {
    let name = entry.name.as_str();
    let empty = Value::Dict(Vec::new());
    if ARGUMENTLESS.contains(&name) {
        let workspace = if name == "ui.profiles.snapshot" {
            Some(profiles_scope(state)?)
        } else {
            None
        };
        let payload = book.read(kind, &empty, "gui")?;
        return match name {
            "ui.profiles.snapshot" => profiles_for_ai(&payload, &workspace.expect("checked above")),
            "ui.reports.capital_gains" => capital_gains_for_ai(payload),
            _ => Ok(payload),
        };
    }
    match name {
        "ui.wallets.utxos" => Ok(wallet_utxos_for_ai(book.read(kind, arguments, "gui")?)),
        "ui.transactions.history" => {
            refuse_unknown(arguments, &HISTORY_FIELDS, "ui.transactions.history")?;
            if !has_text(arguments, "transaction") {
                return Err(AppError::new(
                    "validation",
                    "ui.transactions.history requires args.transaction",
                )
                .into());
            }
            book.read(kind, arguments, "gui")
        }
        "ui.activity.history" => {
            refuse_unknown(arguments, &ACTIVITY_FIELDS, "ui.activity.history")?;
            book.read(kind, arguments, "gui")
        }
        "ui.activity.stale" => {
            if matches!(arguments, Value::Dict(pairs) if !pairs.is_empty()) {
                return Err(AppError::new(
                    "validation",
                    "ui.activity.stale does not accept arguments",
                )
                .into());
            }
            book.read(kind, arguments, "gui")
        }
        "ui.attachments.list" => {
            let mut arguments = arguments.clone();
            if get(&arguments, "limit").is_none() {
                set(&mut arguments, "limit", Value::from(100));
            }
            Ok(redact_evidence_for_ai(&book.read(kind, &arguments, "gui")?))
        }
        "ui.loans.list" => {
            let limit = coerce_positive_int(
                get(arguments, "limit").unwrap_or(&Value::from(100)),
                "limit",
                200,
            )?;
            loans_for_ai(book.read(kind, &empty, "gui")?, limit)
        }
        "ui.review.receipt" => {
            let receipt = book.read(kind, arguments, "ai_tool")?;
            let overrides = match get(&receipt, "proposed_operations") {
                Some(Value::List(operations)) => operations
                    .iter()
                    .any(|operation| matches!(get(operation, "type"), Some(Value::Str(t)) if t.is("kind_override"))),
                _ => false,
            };
            if overrides {
                return Err(AppError::new(
                    "interaction_required",
                    "Acquisition effect receipts are local desktop/CLI only",
                )
                .into());
            }
            Ok(receipt)
        }
        _ if COMMERCIAL_READS.contains(&kind) => {
            Ok(redact_evidence_for_ai(&book.read(kind, arguments, "gui")?))
        }
        _ if SOURCE_FUNDS_READS.contains(&kind) => {
            let mut arguments = arguments.clone();
            if kind == "ui.source_funds.request_input" {
                if let Some(explanation) = get(&arguments, "explanation").cloned() {
                    set(
                        &mut arguments,
                        "explanation",
                        redact_ai_tool_result(&redact_source_funds_for_ai(&explanation)),
                    );
                }
            }
            let payload = book.read(kind, &arguments, "gui")?;
            if matches!(
                kind,
                "ui.source_funds.review_context" | "ui.source_funds.request_input"
            ) {
                let recipe = get(&payload, "recipe")
                    .cloned()
                    .ok_or(ToolError::Unexpected)?;
                if recipe_chars(&recipe)? > 4096 {
                    return Err(AppError::new("source_funds_recipe_too_large", "This source-funds recipe is too large for a chat continuation")
                        .with_hint("Narrow the disclosure options or shorten the note, or continue in the local UI or CLI.")
                        .into());
                }
                if !py_eq(
                    &redact_ai_tool_result(&redact_source_funds_for_ai(&recipe)),
                    &recipe,
                ) {
                    return Err(AppError::new("source_funds_recipe_private", "This source-funds recipe cannot be returned unchanged to an AI provider")
                        .with_hint("Use a destination label and non-sensitive note, or review the full recipe in the local UI or CLI.")
                        .into());
                }
            }
            Ok(redact_source_funds_for_ai(&payload))
        }
        _ if kind.starts_with("ui.transfers.")
            || kind.starts_with("ui.saved_views.")
            || matches!(
                kind,
                "ui.custody.gaps.list"
                    | "ui.custody.gaps.review_context"
                    | "ui.custody.gaps.history"
                    | "ui.custody.review.plan"
            ) =>
        {
            book.read(kind, arguments, "ai_tool")
        }
        _ => book.read(kind, arguments, "gui"),
    }
}

/// `_ai_safe_egress_snapshot(args)` over a ledger snapshot
/// (`{"records", "last_id", "gap"}`): counts per subsystem only; hosts,
/// ports, paths and bodies never reach the model.
pub fn ai_safe_egress_snapshot(after_id: i64, snapshot: &Value) -> Value {
    let records: Vec<&Value> = match get(snapshot, "records") {
        Some(Value::List(records)) => records.iter().collect(),
        _ => Vec::new(),
    };
    let mut by_subsystem: Vec<(Key, Value)> = Vec::new();
    for record in records
        .iter()
        .filter(|record| matches!(record, Value::Dict(_)))
    {
        let subsystem = match get(record, "subsystem") {
            Some(subsystem) if kassiber_envelope::truthy(subsystem) => {
                kassiber_envelope::py_str(subsystem)
            }
            _ => "unknown".to_owned(),
        };
        let bytes = match get(record, "bytes_out") {
            Some(Value::Int(n)) => n.clone(),
            _ => BigInt::from(0),
        };
        let key = Key::from(subsystem.as_str());
        match by_subsystem.iter_mut().find(|(name, _)| *name == key) {
            Some((_, row)) => {
                let count = match get(row, "records") {
                    Some(Value::Int(n)) => n.clone() + 1,
                    _ => BigInt::from(1),
                };
                let total = match get(row, "bytes_out") {
                    Some(Value::Int(n)) => n.clone() + bytes,
                    _ => bytes,
                };
                set(row, "records", Value::Int(count));
                set(row, "bytes_out", Value::Int(total));
            }
            None => by_subsystem.push((
                key,
                dict([
                    ("records", Value::from(1)),
                    ("bytes_out", Value::Int(bytes)),
                ]),
            )),
        }
    }
    dict([
        ("after_id", Value::from(after_id)),
        ("last_id", get(snapshot, "last_id").cloned().unwrap_or(Value::Null)),
        ("gap", Value::Bool(get(snapshot, "gap").is_some_and(kassiber_envelope::truthy))),
        ("records_returned", Value::from(records.len())),
        ("by_subsystem", Value::Dict(by_subsystem)),
        (
            "privacy_note",
            s("Hosts, ports, paths, query strings, headers, request bodies, and configured backend identities are intentionally omitted from the AI-facing view."),
        ),
    ])
}

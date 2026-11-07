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
const ARGUMENTLESS: [&str; 13] = [
    "status",
    "ui.overview.snapshot",
    "ui.workspace.health",
    "ui.next_actions",
    "ui.journals.snapshot",
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

/// `core_ownership.redact_result_for_ai(item)`: the ownership verdict, without
/// scripts, derivation paths or address indexes.
pub fn identify_result_for_ai(item: &Value) -> Value {
    let field = |key: &str| get(item, key).cloned();
    let text_or = |key: &str, default: &str| field(key).unwrap_or_else(|| s(default));
    let kind = match get(item, "type") {
        Some(value) if kassiber_envelope::truthy(value) => kassiber_envelope::py_str(value),
        _ => String::new(),
    };
    let shown_input = match kind.as_str() {
        "extended_public_key" => s("extended public key (not shown)"),
        "lightning_invoice" => s("Lightning invoice (not shown)"),
        _ => text_or("input", ""),
    };
    let mut redacted = vec![
        (Key::from("input"), shown_input),
        (Key::from("type"), text_or("type", "")),
        (Key::from("chain"), text_or("chain", "")),
        (Key::from("status"), text_or("status", "")),
        (Key::from("classification"), text_or("classification", "")),
        (
            Key::from("ownership_ambiguous"),
            Value::Bool(get(item, "ownership_ambiguous").is_some_and(kassiber_envelope::truthy)),
        ),
        (Key::from("note"), text_or("note", "")),
    ];
    let mut wallets: Vec<Value> = Vec::new();
    match (get(item, "matches"), get(item, "wallets")) {
        (Some(matches), _) if kassiber_envelope::truthy(matches) => {
            let mut names: Vec<Value> = Vec::new();
            if let Value::List(matches) = matches {
                for found in matches {
                    if let Some(wallet) =
                        get(found, "wallet").filter(|w| kassiber_envelope::truthy(w))
                    {
                        if !names.iter().any(|name| py_eq(name, wallet)) {
                            names.push(wallet.clone());
                        }
                    }
                }
            }
            names.sort_by(|a, b| match (a, b) {
                (Value::Str(a), Value::Str(b)) => a.cmp(b),
                _ => std::cmp::Ordering::Equal,
            });
            wallets = names;
        }
        (_, Some(listed)) if kassiber_envelope::truthy(listed) => {
            if let Value::List(listed) = listed {
                wallets = listed
                    .iter()
                    .filter(|wallet| kassiber_envelope::truthy(wallet))
                    .cloned()
                    .collect();
            }
        }
        _ => {}
    }
    if !wallets.is_empty() {
        redacted.push((Key::from("wallets"), Value::List(wallets.clone())));
    }
    if get(item, "classification") == Some(&s("owned_address")) {
        let owners = wallets
            .iter()
            .map(|wallet| format!("'{}'", kassiber_envelope::py_str(wallet)))
            .collect::<Vec<_>>()
            .join(", ");
        let owners = if owners.is_empty() {
            "a wallet in this profile".to_owned()
        } else {
            owners
        };
        set_pair(&mut redacted, "note", s(format!("Owned by {owners}.")));
    }
    for key in [
        "owned_inputs",
        "owned_outputs",
        "external_outputs",
        "match_source",
    ] {
        if let Some(value) = get(item, key).filter(|value| !matches!(value, Value::Null)) {
            redacted.push((Key::from(key), value.clone()));
        }
    }
    Value::Dict(redacted)
}

fn set_pair(pairs: &mut [(Key, Value)], key: &str, value: Value) {
    if let Some((_, slot)) = pairs.iter_mut().find(|(name, _)| *name == Key::from(key)) {
        *slot = value;
    }
}

/// `build_wallet_identify_snapshot_for_ai`: never a bulk CSV harvest, and
/// every result reduced to its ownership verdict.
fn wallet_identify_for_ai(arguments: &Value, book: &mut dyn Book) -> Result<Value, ToolError> {
    let arguments = match arguments {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .iter()
                .filter(|(key, _)| *key != Key::from("csv_text"))
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    };
    let mut payload = book.read("ui.wallets.identify", &arguments, "gui")?;
    let results = match get(&payload, "results") {
        Some(Value::List(rows)) => rows
            .iter()
            .filter(|row| matches!(row, Value::Dict(_)))
            .map(identify_result_for_ai)
            .collect(),
        // `payload.get("results", [])` on a missing key.
        None => Vec::new(),
        // Iterating anything else is a Python TypeError.
        Some(_) => return Err(ToolError::Unexpected),
    };
    set(&mut payload, "results", Value::List(results));
    Ok(payload)
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
    auto_sync: Option<&Value>,
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
        "ui.workspace.overview.snapshot" => {
            if get(state, "cross_book_read_allowed") == Some(&Value::Bool(false)) {
                return Err(AppError::new(
                    "validation",
                    "A book-set overview requires an explicit all-books request",
                )
                .into());
            }
            let requested = match get(arguments, "workspace_id") {
                Some(Value::Str(text)) => Some(kassiber_pytext::strip_py(text)),
                _ => None,
            };
            let outside = match (&requested, get(state, "scope_workspace_id")) {
                (None, _) => true,
                (Some(requested), _) if requested.code_points().next().is_none() => true,
                (Some(requested), Some(Value::Str(frozen))) => requested != frozen,
                _ => false,
            };
            if outside {
                return Err(AppError::new(
                    "validation",
                    "The AI book-set overview is limited to the chat's original workspace",
                )
                .into());
            }
            let requested = requested.expect("checked above");
            book.read(
                kind,
                &dict([("workspace_id", Value::Str(requested))]),
                "gui",
            )
        }
        "ui.report.blockers" => book.report_blockers(auto_sync),
        "ui.wallets.request_input" => crate::import_runs::wallet_input_request(book, arguments),
        "ui.imports.list" => {
            crate::import_runs::list_for_ai(book, arguments, crate::executor::full_access(state))
        }
        "ui.imports.rollback.plan" => crate::import_runs::rollback_for_ai(
            book,
            arguments,
            crate::executor::full_access(state),
            false,
        ),
        "ui.review.badges" => {
            if matches!(arguments, Value::Dict(pairs) if !pairs.is_empty()) {
                return Err(AppError::new(
                    "validation",
                    "ui.review.badges does not accept arguments",
                )
                .into());
            }
            book.read(kind, &empty, "gui")
        }
        "ui.transactions.graph" => {
            refuse_unknown(arguments, &["transaction"], "AI transaction graph")?;
            let transaction = match get(arguments, "transaction") {
                Some(Value::Str(text)) => kassiber_pytext::strip_py(text),
                _ => PyStr::new(),
            };
            if transaction.code_points().next().is_none() {
                return Err(AppError::new(
                    "validation",
                    "ui.transactions.graph requires args.transaction",
                )
                .into());
            }
            book.read(
                kind,
                &dict([("transaction", Value::Str(transaction))]),
                "gui",
            )
        }
        "ui.audit.evidence.summary" => {
            Ok(redact_evidence_for_ai(&book.read(kind, arguments, "gui")?))
        }
        "ui.wallets.identify" => wallet_identify_for_ai(arguments, book),
        "ui.wallets.analyze_file" => {
            crate::file_analysis::analyze_attached_file(arguments, state, book)
        }
        "ui.reports.privacy_mirror" => {
            let unknown = unknown_fields(arguments, &[]);
            if !unknown.is_empty() {
                return Err(AppError::new(
                    "validation",
                    "ui.reports.privacy_mirror received unsupported arguments",
                )
                .with_details(dict([("unknown", Value::List(unknown))]))
                .into());
            }
            // `report_privacy_mirror(..., redacted=True)`: the CLI's audience.
            book.read("reports.privacy-mirror", &empty, "gui")
        }
        "ui.egress.snapshot" => egress_view(arguments, &mut |after_id, limit| {
            book.egress_ledger(after_id, limit)
        }),
        "ui.reports.lightning_profitability" => {
            // `report.to_ai_envelope_payload()`: the routing aggregate only,
            // never the connection block or per-channel peer rows.
            let payload = book.read(kind, arguments, "gui")?;
            let field = |key: &str| get(&payload, key).cloned().ok_or(ToolError::Unexpected);
            Ok(dict([
                ("windowLabel", field("windowLabel")?),
                ("summary", field("summary")?),
            ]))
        }
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
        // `_review_workflow_payload(..., authored_source="ai_tool")`: the
        // native route applies the AI rules (redacted explanations, the
        // operation authority checks, plans that must reach the provider
        // unchanged, no acquisition-effect receipts).
        "ui.review.cases" | "ui.review.request_input" | "ui.review.plan" | "ui.review.receipt" => {
            book.review(kind, arguments, crate::executor::full_access(state))
        }
        _ if kind.starts_with("ui.chain_analysis.") => {
            chain_analysis_for_ai(kind, arguments, state, book)
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

/// `_chain_analysis_ai_payload(conn, runtime, kind, args)`: on-device
/// providers get the curated result; any other provider gets the
/// `ca-ref:` projection, and errors without their details, because a
/// saved-query failure can carry public subjects.
pub(crate) fn chain_analysis_for_ai(
    kind: &str,
    arguments: &Value,
    state: &Value,
    book: &mut dyn Book,
) -> Result<Value, ToolError> {
    let profile = book.active_profile()?;
    // Every tool and unprojected result with full agent access (#678).
    let on_device = crate::executor::full_access(state);
    if crate::analysis::on_device_kind(kind) && !on_device {
        return Err(AppError::new(
            "agent_access_restricted",
            "Source acquisition and dataset imports are unavailable while this provider's agent access is restricted",
        )
        .with_hint("Set the provider's agent access to full in Settings → Assistant, or use the desktop workflow.")
        .into());
    }
    let result = (|| {
        let decoded = book.decode_analysis_references(&profile, arguments)?;
        if matches!(
            kind,
            "ui.chain_analysis.datasets.import" | "ui.chain_analysis.datasets.import.start"
        ) {
            // Consent was bound to this exact local validation.
            book.dataset_preview(&decoded)?;
        }
        if kind.starts_with("ui.chain_analysis.datasets.") && kind.ends_with(".start") {
            // Background dataset work needs the chat's job starter, which
            // this executor state never carries.
            return Err(ToolError::App(AppError::new(
                "daemon_required",
                "Background dataset work requires a running daemon; use the synchronous CLI operation",
            )));
        }
        book.read(kind, &decoded, "gui")
    })();
    let result = match result {
        Ok(result) => result,
        Err(ToolError::App(error)) if !on_device => {
            return Err(AppError::new(
                error.code,
                "Local chain investigation could not complete; inspect the error code and refresh the case or query",
            )
            .retryable(error.retryable)
            .into());
        }
        Err(error) => return Err(error),
    };
    if on_device {
        return Ok(result);
    }
    crate::analysis::project_ai_result(&result, &mut |value| {
        book.issue_analysis_reference(&profile, value)
    })
}

/// `_ai_safe_egress_snapshot(args)`: the argument checks, then the counts
/// over `ledger(after_id, limit)`.
pub fn egress_view(
    arguments: &Value,
    ledger: &mut dyn FnMut(i64, i64) -> Result<Value, ToolError>,
) -> Result<Value, ToolError> {
    refuse_unknown(arguments, &["after_id", "limit"], "ui.egress.snapshot")?;
    let after_id = match get(arguments, "after_id") {
        Some(raw) if kassiber_envelope::truthy(raw) => {
            coerce_positive_int(raw, "after_id", i64::MAX)?
        }
        _ => 0,
    };
    let limit = coerce_positive_int(
        get(arguments, "limit").unwrap_or(&Value::from(100)),
        "limit",
        500,
    )?;
    let snapshot = ledger(after_id, limit)?;
    Ok(ai_safe_egress_snapshot(after_id, &snapshot))
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

/// `_AI_PROCESS_JOURNALS_REMEDIATION`.
fn process_journals_remediation() -> Value {
    dict([
        ("action", s("process_journals")),
        ("tool", s("ui.journals.process")),
        ("target", Value::Dict(Vec::new())),
        ("desktop_only", Value::Bool(false)),
    ])
}

/// `_ai_journal_unavailable(reason, as_of=..., freshness=...)`: an explicit
/// stand-in for journal figures the model must not quote. Zeros and empty
/// lists read as "nothing booked"; `null` figures with a status, reason and
/// remediation cannot be mistaken for that.
fn journal_unavailable(reason: &str, as_of: Value) -> Vec<(Key, Value)> {
    let message = if reason == "journals_never_processed" {
        "Journals have never been processed for this book, so there are no figures yet."
    } else {
        "Journal inputs changed since the last processing; these figures are withheld until journals are processed again."
    };
    vec![
        (Key::from("status"), s("unavailable")),
        (Key::from("reason"), s(reason)),
        (Key::from("message"), s(message)),
        (Key::from("as_of"), as_of),
        (Key::from("journal_freshness"), Value::Null),
        (Key::from("remediation"), process_journals_remediation()),
    ]
}

/// `_ai_journal_stale(as_of)`: quarantine is the last processing's verdict;
/// say so instead of hiding it.
fn journal_stale(as_of: Value) -> Value {
    dict([
        ("status", s("stale")),
        ("reason", s("journals_stale")),
        ("as_of", as_of),
        (
            "message",
            s("These rows are what the last journal processing held. Inputs changed since; process journals before treating the counts as current."),
        ),
        ("remediation", process_journals_remediation()),
    ])
}

/// `payload.get(key) or {}`, as a dict to read and spread.
fn section(payload: &Value, key: &str) -> Value {
    match get(payload, key) {
        Some(found @ Value::Dict(_)) => found.clone(),
        _ => Value::Dict(Vec::new()),
    }
}

fn field(value: &Value, key: &str) -> Value {
    get(value, key).cloned().unwrap_or(Value::Null)
}

fn flag(value: &Value, key: &str) -> bool {
    get(value, key).is_some_and(kassiber_envelope::truthy)
}

/// `{**base, key: None, ...}`.
fn nulled(mut base: Value, keys: &[&str]) -> Value {
    for key in keys {
        set(&mut base, key, Value::Null);
    }
    base
}

/// `{**marker, **rest}`.
fn marked(mut marker: Vec<(Key, Value)>, rest: Vec<(&str, Value)>) -> Value {
    for (key, value) in rest {
        marker.retain(|(existing, _)| *existing != Key::from(key));
        marker.push((Key::from(key), value));
    }
    Value::Dict(marker)
}

/// `_project_stale_journal_figures_for_ai(kind, payload)` (#681): stale
/// journal-derived figures in an AI read become `null` with an explicit
/// state. The desktop pages read the same builders and show their own stale
/// state, so this lives only in the AI projection. It runs after the read's
/// own journal refresh: figures stay stale only when that refresh failed or
/// was disabled (an MCP read).
pub fn stale_journal_figures_for_ai(kind: &str, payload: Value) -> Value {
    if !matches!(payload, Value::Dict(_)) {
        return payload;
    }
    match kind {
        "ui.journals.transfers.list" => {
            let summary = section(&payload, "summary");
            if get(&summary, "projection_status") != Some(&s("stale")) {
                return payload;
            }
            let mut kept = Value::Dict(Vec::new());
            for key in ["workspace", "profile", "limit"] {
                set(&mut kept, key, field(&summary, key));
            }
            let kept = nulled(
                kept,
                &[
                    "manual_pairs",
                    "same_asset_pairs",
                    "cross_asset_pairs",
                    "journal_transfer_entries",
                ],
            );
            marked(
                journal_unavailable("journals_stale", Value::Null),
                vec![("summary", kept), ("pairs", Value::Null)],
            )
        }
        "ui.reports.capital_gains" | "ui.journals.events.list" | "ui.journals.snapshot" => {
            let status = section(
                &payload,
                if kind == "ui.journals.events.list" {
                    "summary"
                } else {
                    "status"
                },
            );
            if !flag(&status, "needsJournals") {
                return payload;
            }
            let as_of = field(&status, "lastProcessedAt");
            let reason = if kassiber_envelope::truthy(&as_of) || kind == "ui.reports.capital_gains"
            {
                "journals_stale"
            } else {
                "journals_never_processed"
            };
            let marker = journal_unavailable(reason, as_of);
            match kind {
                "ui.reports.capital_gains" => marked(
                    marker,
                    vec![
                        ("year", field(&payload, "year")),
                        ("availableYears", field(&payload, "availableYears")),
                        ("journal_status", status),
                        ("totals", Value::Null),
                        ("kennzahlRows", Value::Null),
                        ("rows", Value::Null),
                    ],
                ),
                "ui.journals.events.list" => marked(
                    marker,
                    vec![
                        (
                            "summary",
                            nulled(status, &["count", "reportableCount", "entryTypes"]),
                        ),
                        ("events", Value::Null),
                    ],
                ),
                _ => marked(
                    marker,
                    vec![
                        ("journal_status", status),
                        ("entryTypes", Value::Null),
                        ("recent", Value::Null),
                        ("recentByType", Value::Null),
                    ],
                ),
            }
        }
        "ui.overview.snapshot" => {
            let Some(fiat @ Value::Dict(_)) = get(&payload, "fiat").cloned() else {
                return payload;
            };
            let stale = matches!(get(&fiat, "completeness"), Some(completeness @ Value::Dict(_))
                if get(completeness, "state") == Some(&s("stale")));
            if !stale {
                return payload;
            }
            let mut fiat = nulled(fiat, &["eurCostBasis", "eurUnrealized", "eurRealizedYTD"]);
            set(
                &mut fiat,
                "availability",
                Value::Dict(journal_unavailable("journals_stale", Value::Null)),
            );
            let mut payload = payload;
            set(&mut payload, "fiat", fiat);
            payload
        }
        "ui.journals.quarantine" => {
            let summary = section(&payload, "summary");
            let freshness = section(&summary, "freshness");
            if !flag(&freshness, "needs_processing") {
                return payload;
            }
            let as_of = field(&freshness, "last_processed_at");
            if kassiber_envelope::truthy(&as_of) {
                let mut payload = payload;
                set(&mut payload, "projection", journal_stale(as_of));
                return payload;
            }
            marked(
                journal_unavailable("journals_never_processed", Value::Null),
                vec![
                    (
                        "summary",
                        nulled(
                            summary,
                            &[
                                "count",
                                "by_reason",
                                "attention_count",
                                "waiting_count",
                                "blocking_count",
                                "scope_count",
                                "groups",
                                "group_count",
                            ],
                        ),
                    ),
                    ("items", Value::Null),
                ],
            )
        }
        "ui.journals.quarantine.groups" => {
            if !flag(&payload, "needs_processing") {
                return payload;
            }
            let as_of = field(&payload, "last_processed_at");
            if kassiber_envelope::truthy(&as_of) {
                let mut payload = payload;
                set(&mut payload, "projection", journal_stale(as_of));
                return payload;
            }
            marked(
                journal_unavailable("journals_never_processed", Value::Null),
                vec![("definitions", field(&payload, "definitions"))],
            )
        }
        _ => payload,
    }
}

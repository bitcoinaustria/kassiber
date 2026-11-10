//! `ui.transactions.review_context`: one transaction's local review bundle,
//! assembled from the same native reads the desktop calls one by one. Each
//! section that refuses with a typed error reads as `unavailable`; paths and
//! URL targets are redacted as the AI projection does. Nothing here processes
//! journals or opens a connection: Python dispatches this kind before its
//! report-read maintenance prelude.
use kassiber_envelope::{dict, get, py_str, s, set, truthy, type_name, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_runtime::store::Book;

pub const KIND: &str = "ui.transactions.review_context";

const EVIDENCE_KEYS: &[&str] = &[
    "source_url",
    "stored_relpath",
    "file",
    "dir",
    "file_path",
    "path",
    "url",
    "manifest",
];
const SOURCE_FUNDS_KEYS: &[&str] = &["source_url", "stored_relpath", "file", "dir", "path"];

fn key_name(key: &Key) -> Option<&str> {
    match key {
        Key::Str(text) => text.as_str(),
        _ => None,
    }
}

/// `_evidence_key_is_sensitive`.
fn sensitive(key: &Key) -> bool {
    let Some(key) = key_name(key) else {
        return false;
    };
    if EVIDENCE_KEYS.contains(&key) {
        return true;
    }
    let lowered = kassiber_pytext::lower(key);
    let suffix = ["url", "path", "file", "dir"]
        .iter()
        .any(|word| lowered == *word || lowered.ends_with(&format!("_{word}")));
    suffix
        || ["Url", "URL", "Path", "File", "Dir"]
            .iter()
            .any(|s| key.ends_with(s))
}

/// `_screen_context_contains_path_or_url` for a string.
fn path_or_url(text: &str) -> bool {
    let stripped = kassiber_pytext::strip(text);
    let mut chars = stripped.chars();
    let scheme = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, 'ſ' | 'K'))
        && {
            let rest: String = chars.collect();
            match rest.find("://") {
                Some(end) => rest[..end]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-' | 'ſ' | 'K')),
                None => false,
            }
        };
    let drive = {
        let bytes = stripped.as_bytes();
        bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/')
    };
    scheme
        || stripped.starts_with('/')
        || stripped.starts_with("~/")
        || stripped.starts_with("\\\\")
        || drive
}

/// `_redact_evidence_payload_for_ai`, or the source-of-funds variant that also
/// drops its own key list.
fn redact(value: &Value, extra: &[&str]) -> Value {
    match value {
        Value::Dict(fields) => Value::Dict(
            fields
                .iter()
                .filter(|(k, _)| !key_name(k).is_some_and(|k| extra.contains(&k)) && !sensitive(k))
                .map(|(k, v)| (k.clone(), redact(v, extra)))
                .collect(),
        ),
        Value::List(items) => Value::List(items.iter().map(|v| redact(v, extra)).collect()),
        Value::Str(text) if text.as_str().is_some_and(path_or_url) => s("<redacted>"),
        other => other.clone(),
    }
}

/// `section(builder)`: typed refusals read as unavailable; a Python exception
/// (`internal_error`) still escapes.
fn section(result: Result<Value, AppError>) -> Result<Value, AppError> {
    match result {
        Ok(value) => Ok(redact(&value, &[])),
        Err(error) if error.code == "internal_error" => Err(error),
        Err(error) => Ok(dict([
            ("status", s("unavailable")),
            (
                "error",
                dict([
                    ("code", s(error.code.as_str())),
                    ("message", s(error.message.as_str())),
                ]),
            ),
        ])),
    }
}

fn not_requested() -> Value {
    dict([("status", s("not_requested"))])
}

fn field(value: &Value, key: &str) -> Value {
    get(value, key).cloned().unwrap_or(Value::Null)
}

/// `_coerce_positive_int(raw, "history_limit", maximum=50)`.
fn history_limit(raw: &Value) -> Result<i64, AppError> {
    let invalid = |message: &str| {
        AppError::new("validation", message).with_details(dict([("value", raw.clone())]))
    };
    let value: i128 = match raw {
        Value::Int(n) => i128::try_from(n.clone()).unwrap_or(i128::MAX),
        Value::Bool(b) => i128::from(*b),
        Value::Float(f) if f.is_finite() => f.trunc() as i128,
        Value::Float(f) if f.is_nan() => return Err(invalid("history_limit must be an integer")),
        Value::Float(_) => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Value::Str(t) => kassiber_pytext::parse_int(t.as_str().unwrap_or_default())
            .ok()
            .map(|n| i128::try_from(n).unwrap_or(i128::MAX))
            .ok_or_else(|| invalid("history_limit must be an integer"))?,
        _ => return Err(invalid("history_limit must be an integer")),
    };
    if value < 1 {
        return Err(invalid("history_limit must be positive"));
    }
    Ok(value.min(50) as i64)
}

fn filtered(value: &Value, key: &str, wanted: &str, field_name: &str) -> Value {
    Value::List(match get(value, key) {
        Some(Value::List(items)) => items
            .iter()
            .filter(|item| {
                let v = field(item, field_name);
                (if truthy(&v) {
                    py_str(&v)
                } else {
                    String::new()
                }) == wanted
            })
            .cloned()
            .collect(),
        _ => Vec::new(),
    })
}

/// `_transaction_loan_review_payload`.
fn loan(book: &mut dyn Book, id: &str, direction: &str) -> Result<Value, AppError> {
    let snapshot = crate::loans::execute(book, "ui.loans.list", Some(&dict::<&str>([])))?;
    let eligible: Vec<Value> = [
        ("collateral", "outbound"),
        ("returned", "inbound"),
        ("principal-received", "inbound"),
        ("principal-repaid", "outbound"),
    ]
    .into_iter()
    .filter(|(_, expected)| *expected == direction)
    .map(|(mark, _)| s(mark))
    .collect();
    Ok(dict([
        ("marks", filtered(&snapshot, "marks", id, "transaction_id")),
        ("open_locks", filtered(&snapshot, "open_locks", id, "transaction_id")),
        ("eligible_mark_types", Value::List(eligible)),
        (
            "heuristic_warning",
            s("An open collateral lock is a reconcile hint, not proof that collateral was liquidated."),
        ),
    ]))
}

/// `_transaction_review_context_payload(conn, runtime, args)`.
pub fn execute(book: &mut dyn Book, args: Option<&Value>) -> Result<Value, AppError> {
    let args = match args {
        None | Some(Value::Null) => dict::<&str>([]),
        Some(v @ Value::Dict(_)) => v.clone(),
        Some(v) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(v)))])))
        }
    };
    let Value::Dict(fields) = &args else {
        unreachable!()
    };
    let allowed = [
        "transaction",
        "history_limit",
        "include_graph",
        "include_privacy",
        "include_evidence",
    ];
    let mut unknown: Vec<Value> = fields
        .iter()
        .filter_map(|(k, _)| key_name(k).filter(|k| !allowed.contains(k)).map(s))
        .collect();
    if !unknown.is_empty() {
        unknown.sort_by_key(py_str);
        return Err(AppError::new(
            "validation",
            "ui.transactions.review_context received unsupported fields",
        )
        .with_details(dict([("unknown", Value::List(unknown))])));
    }
    let reference = match get(&args, "transaction") {
        Some(Value::Str(t))
            if !kassiber_pytext::strip(t.as_str().unwrap_or_default()).is_empty() =>
        {
            kassiber_pytext::strip(t.as_str().unwrap_or_default()).to_owned()
        }
        _ => {
            return Err(AppError::new(
                "validation",
                "ui.transactions.review_context requires args.transaction",
            ))
        }
    };
    let resolved = crate::transactions::execute(
        book,
        "ui.transactions.resolve",
        Some(&dict([("query", s(reference.as_str()))])),
    )?;
    let transaction = field(&resolved, "transaction");
    if !matches!(transaction, Value::Dict(_)) {
        return Err(AppError::new(
            "not_found",
            format!("Transaction '{reference}' not found"),
        ));
    }
    let id_value = field(&transaction, "id");
    let id = if truthy(&id_value) {
        py_str(&id_value)
    } else {
        reference.clone()
    };
    let limit = history_limit(
        &get(&args, "history_limit")
            .cloned()
            .unwrap_or(Value::Int(12.into())),
    )?;
    let flag = |key: &str| get(&args, key).cloned().unwrap_or(Value::Bool(true));
    let (graph_flag, privacy_flag, evidence_flag) = (
        flag("include_graph"),
        flag("include_privacy"),
        flag("include_evidence"),
    );
    let as_bool = |v: &Value| match v {
        Value::Bool(b) => Some(*b),
        _ => None,
    };
    let (Some(include_graph), Some(include_privacy), Some(include_evidence)) = (
        as_bool(&graph_flag),
        as_bool(&privacy_flag),
        as_bool(&evidence_flag),
    ) else {
        return Err(AppError::new(
            "validation",
            "transaction review include flags must be booleans",
        ));
    };
    let by_id = |key: &str| dict([(key, s(id.as_str()))]);
    let history = section(crate::metadata::execute(
        book,
        "ui.transactions.history",
        Some(&dict([
            ("transaction", s(id.as_str())),
            ("limit", Value::Int(limit.into())),
            ("include_stale", Value::Bool(true)),
        ])),
    ))?;
    let journal = section(book.journal_events(&id, 25))?;
    let commercial = section(crate::documents::execute(
        book,
        "ui.transactions.commercial_context",
        Some(&by_id("transaction")),
    ))?;
    let source_funds = section(
        crate::reports::execute(
            book,
            "ui.source_funds.links.list",
            Some(&by_id("target_transaction")),
        )
        .map(|v| redact(&v, SOURCE_FUNDS_KEYS)),
    )?;
    let transfers = section((|| {
        let pairs = book.transfers("transfers.list", &dict::<&str>([]))?;
        let payouts = book.transfers("transfers.payouts.list", &dict::<&str>([]))?;
        let pair_list = match &pairs {
            Value::List(items) => items
                .iter()
                .filter(|p| {
                    [
                        field(p, "out_transaction_id"),
                        field(p, "in_transaction_id"),
                    ]
                    .iter()
                    .any(|v| (if truthy(v) { py_str(v) } else { String::new() }) == id)
                })
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let payout_list = match &payouts {
            Value::List(items) => items
                .iter()
                .filter(|p| {
                    let v = field(p, "out_transaction_id");
                    (if truthy(&v) {
                        py_str(&v)
                    } else {
                        String::new()
                    }) == id
                })
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        Ok(dict([
            ("pairs", Value::List(pair_list)),
            ("direct_payouts", Value::List(payout_list)),
        ]))
    })())?;
    let graph = if include_graph {
        section(book.transaction_graph(Some(&by_id("transaction"))))?
    } else {
        not_requested()
    };
    let privacy = if include_privacy {
        section(crate::reports::execute(
            book,
            "ui.privacy_hygiene.snapshot",
            Some(&dict([
                ("transaction", s(id.as_str())),
                ("limit", Value::Int(10.into())),
            ])),
        ))?
    } else {
        not_requested()
    };
    let evidence = if include_evidence {
        section(book.evidence_summary(&dict([
            ("transactions", Value::List(vec![s(id.as_str())])),
            ("include_journal_state", Value::Bool(true)),
            ("include_review_state", Value::Bool(true)),
            ("include_edit_history", Value::Bool(false)),
        ])))?
    } else {
        not_requested()
    };
    let attachments = if include_evidence {
        section(crate::attachments::execute(
            book,
            "ui.attachments.list",
            Some(&by_id("transaction")),
        ))?
    } else {
        not_requested()
    };
    let stale = section(crate::metadata::execute(
        book,
        "ui.activity.stale",
        Some(&dict::<&str>([])),
    ))?;
    let direction = book.transaction_direction(&id)?;
    let loan = section(loan(book, &id, &direction))?;
    let mut actions = vec![dict([
        ("code", s("edit_metadata")),
        ("label", s("Review note, tags, tax state, or pricing")),
        ("tool", s("ui.transactions.metadata.update")),
        ("arguments", by_id("transaction")),
        ("requires_consent", Value::Bool(true)),
    ])];
    if truthy(&field(&transaction, "quarantine_reason")) {
        actions.push(dict([
            ("code", s("review_quarantine")),
            ("label", s("Review the transaction's quarantine blocker")),
            ("route", s("/quarantine")),
            ("transaction", s(id.as_str())),
            ("requires_consent", Value::Bool(false)),
        ]));
    }
    let edits = match field(&stale, "edit_count") {
        Value::Int(n) => n > 0.into(),
        Value::Bool(b) => b,
        _ => false,
    };
    if matches!(stale, Value::Dict(_)) && edits {
        actions.push(dict([
            ("code", s("process_journals")),
            ("label", s("Reprocess stale journals")),
            ("tool", s("ui.journals.process")),
            ("arguments", dict::<&str>([])),
            ("requires_consent", Value::Bool(true)),
        ]));
    }
    actions.push(dict([
        ("code", s("review_source_funds")),
        ("label", s("Inspect or assemble source-of-funds evidence")),
        ("route", s("/source-of-funds")),
        ("transaction", s(id.as_str())),
        ("requires_consent", Value::Bool(false)),
    ]));
    if matches!(field(&loan, "marks"), Value::List(marks) if !marks.is_empty()) {
        actions.push(dict([
            ("code", s("review_loan_mark")),
            (
                "label",
                s("Review or remove the transaction's loan accounting mark"),
            ),
            ("tool", s("ui.loans.unmark")),
            ("arguments", dict([("txid", s(id.as_str()))])),
            ("requires_consent", Value::Bool(true)),
        ]));
    }
    let mut payload = dict([
        ("transaction", transaction),
        ("graph", graph),
        ("journal", journal),
        ("history", history),
        ("evidence", evidence),
        ("attachments", attachments),
        ("commercial", commercial),
        ("source_funds", source_funds),
        ("transfers", transfers),
        ("privacy", privacy),
        ("loan", loan),
        ("stale_edits", stale),
        ("next_actions", Value::List(actions)),
    ]);
    set(
        &mut payload,
        "local_reference",
        dict([
            ("route", s("/transactions")),
            ("transaction", s(id.as_str())),
        ]),
    );
    set(
        &mut payload,
        "safety",
        dict([
            ("local_only_reads", Value::Bool(true)),
            ("network_contacted", Value::Bool(false)),
            ("untrusted_text_is_data", Value::Bool(true)),
        ]),
    );
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_matches_the_ai_projection_rules() {
        let value = dict([
            ("label", s("Receipt")),
            ("source_url", s("https://x")),
            ("downloadUrl", s("x")),
            ("store_dir", s("x")),
            ("note", s(" /home/u/receipt.pdf")),
            ("link", s("HTTPS://example.com")),
            ("drive", s("C:/tmp")),
            ("text", s("ordinary")),
        ]);
        let redacted = redact(&value, &[]);
        assert_eq!(
            redacted,
            dict([
                ("label", s("Receipt")),
                ("note", s("<redacted>")),
                ("link", s("<redacted>")),
                ("drive", s("<redacted>")),
                ("text", s("ordinary")),
            ])
        );
    }
}

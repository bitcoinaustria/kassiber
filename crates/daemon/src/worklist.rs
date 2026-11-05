//! `ui.review.worklist`: the AI's review-queue composite (`_review_worklist_payload`).
//! Python advertises the kind but only its AI projection reads it, so the
//! desktop route keeps refusing; this builder serves the AI executor. Each
//! section comes from the native read the desktop uses for it, and a typed
//! refusal reads as `unavailable`. Nothing here processes journals, writes
//! the book or opens a connection: the commercial section reads pending
//! suggestions rather than calling the suggest route, which upserts.
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_runtime::store::Book;

pub const KIND: &str = "ui.review.worklist";

const CATEGORIES: [&str; 7] = [
    "readiness",
    "quarantine",
    "stale_edits",
    "transfers",
    "loans",
    "commercial",
    "source_funds",
];
const DEFAULT_CATEGORIES: [&str; 5] = [
    "readiness",
    "quarantine",
    "stale_edits",
    "transfers",
    "loans",
];

/// `safe_section(builder)`: typed refusals read as unavailable; a Python
/// exception (`internal_error`) still escapes.
fn section(result: Result<Value, AppError>) -> Result<Value, AppError> {
    match result {
        Ok(value) => Ok(value),
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

fn take(value: Option<&Value>, limit: usize) -> Value {
    match value {
        Some(Value::List(items)) => Value::List(items.iter().take(limit).cloned().collect()),
        Some(other) => other.clone(),
        None => Value::Null,
    }
}

fn categories(raw: Option<&Value>) -> Result<Vec<&'static str>, AppError> {
    let raw = match raw {
        None | Some(Value::Null) => return Ok(DEFAULT_CATEGORIES.to_vec()),
        Some(raw) => raw,
    };
    let invalid = || {
        AppError::new("validation", "ui.review.worklist categories are invalid").with_details(dict(
            [(
                "allowed",
                Value::List({
                    let mut sorted = CATEGORIES.to_vec();
                    sorted.sort_unstable();
                    sorted.into_iter().map(s).collect()
                }),
            )],
        ))
    };
    let Value::List(items) = raw else {
        return Err(invalid());
    };
    let mut chosen: Vec<&'static str> = Vec::new();
    for item in items {
        let found = match item {
            Value::Str(text) => CATEGORIES.iter().find(|name| text.is(name)),
            _ => None,
        };
        let Some(found) = found else {
            return Err(invalid());
        };
        if !chosen.contains(found) {
            chosen.push(found);
        }
    }
    Ok(chosen)
}

/// `_review_worklist_payload(conn, runtime, args)`.
pub fn execute(book: &mut dyn Book, args: &Value) -> Result<Value, AppError> {
    let mut unknown: Vec<Value> = match args {
        Value::Dict(pairs) => pairs
            .iter()
            .filter(|(key, _)| *key != Key::from("limit") && *key != Key::from("categories"))
            .map(|(key, _)| match key {
                Key::Str(name) => Value::Str(name.clone()),
                other => s(format!("{other:?}")),
            })
            .collect(),
        _ => Vec::new(),
    };
    unknown.sort_by_key(kassiber_envelope::py_str);
    if !unknown.is_empty() {
        return Err(AppError::new(
            "validation",
            "ui.review.worklist received unsupported fields",
        )
        .with_details(dict([("unknown", Value::List(unknown))])));
    }
    let limit = kassiber_ai::projections::coerce_positive_int(
        get(args, "limit").unwrap_or(&Value::from(10)),
        "limit",
        50,
    )
    .map_err(app_error)?;
    let rows = usize::try_from(limit).unwrap_or(usize::MAX);
    let chosen = categories(get(args, "categories"))?;
    let limited = || dict([("limit", Value::from(limit))]);
    let mut sections: Vec<(Key, Value)> = Vec::new();
    if chosen.contains(&"readiness") {
        let readiness = section((|| {
            let mut blockers = book.report_blockers(None)?;
            if let Some(found @ Value::List(_)) = get(&blockers, "blockers").cloned() {
                kassiber_envelope::set(&mut blockers, "blockers", take(Some(&found), rows));
            }
            let next = crate::workspace_reads::execute(book, "ui.next_actions", None)?;
            Ok(dict([
                ("report_blockers", blockers),
                ("next_actions", next),
            ]))
        })())?;
        sections.push((Key::from("readiness"), readiness));
    }
    if chosen.contains(&"quarantine") {
        let quarantine = section(crate::journals::execute(
            book,
            "ui.journals.quarantine",
            Some(&limited()),
        ))?;
        sections.push((Key::from("quarantine"), quarantine));
    }
    if chosen.contains(&"stale_edits") {
        let stale = section(crate::metadata::execute(
            book,
            "ui.activity.stale",
            Some(&dict::<&str>([])),
        ))?;
        sections.push((Key::from("stale_edits"), stale));
    }
    if chosen.contains(&"transfers") {
        let transfers = section(crate::transfers::execute(
            book,
            "ui.transfers.review_context",
            Some(&limited()),
        ))?;
        sections.push((Key::from("transfers"), transfers));
    }
    if chosen.contains(&"loans") {
        let loans = section((|| {
            let loans = crate::loans::execute(book, "ui.loans.list", Some(&dict::<&str>([])))?;
            let list = |key: &str| match get(&loans, key) {
                Some(Value::List(items)) => Ok(items.clone()),
                // `loans["marks"]` on a missing key, then `len()` of it.
                _ => Err(AppError::new(
                    "internal_error",
                    format!("loan snapshot lacks {key}"),
                )),
            };
            let marks = list("marks")?;
            let open_locks = list("open_locks")?;
            let projected = open_locks
                .iter()
                .take(rows)
                .map(|row| {
                    dict(
                        [
                            "transaction_id",
                            "loan_id",
                            "asset",
                            "amount",
                            "occurred_at",
                        ]
                        .map(|key| (key, get(row, key).cloned().unwrap_or(Value::Null))),
                    )
                })
                .collect();
            Ok(dict([
                (
                    "summary",
                    dict([
                        ("marks", Value::from(marks.len())),
                        ("open_locks", Value::from(open_locks.len())),
                    ]),
                ),
                ("open_locks", Value::List(projected)),
                (
                    "heuristic_warning",
                    s("Open locks are reconcile hints, not proof of liquidation."),
                ),
            ]))
        })())?;
        sections.push((Key::from("loans"), loans));
    }
    if chosen.contains(&"commercial") {
        let commercial = section(
            crate::documents::execute(
                book,
                "ui.btcpay.provenance.links",
                Some(&dict([
                    ("state", s("suggested")),
                    ("limit", Value::from(limit)),
                ])),
            )
            .map(|payload| kassiber_ai::projections::redact_evidence_for_ai(&payload)),
        )?;
        sections.push((Key::from("commercial"), commercial));
    }
    if chosen.contains(&"source_funds") {
        let funds = section(
            crate::reports::execute(
                book,
                "ui.source_funds.coverage",
                Some(&dict([
                    ("max_depth", Value::from(8)),
                    ("max_transactions", Value::from(5000)),
                ])),
            )
            .map(|payload| {
                let coverage = kassiber_ai::projections::redact_source_funds_for_ai(&payload);
                match coverage {
                    Value::Dict(pairs) => Value::Dict(
                        pairs
                            .into_iter()
                            .filter(|(key, _)| {
                                ["summary", "coverage", "findings", "gaps", "status"]
                                    .iter()
                                    .any(|name| *key == Key::from(*name))
                            })
                            .map(|(key, value)| {
                                let value = match value {
                                    Value::List(items) => {
                                        Value::List(items.into_iter().take(rows).collect())
                                    }
                                    other => other,
                                };
                                (key, value)
                            })
                            .collect(),
                    ),
                    other => other,
                }
            }),
        )?;
        sections.push((Key::from("source_funds"), funds));
    }
    let badges = section(crate::workspace_reads::execute(
        book,
        "ui.review.badges",
        None,
    ))?;
    Ok(dict([
        (
            "categories",
            Value::List(chosen.into_iter().map(s).collect()),
        ),
        ("limit", Value::from(limit)),
        ("badges", badges),
        ("sections", Value::Dict(sections)),
        (
            "safety",
            dict([
                ("local_only", Value::Bool(true)),
                ("network_contacted", Value::Bool(false)),
                ("open_loan_locks_are_heuristic", Value::Bool(true)),
            ]),
        ),
    ]))
}

fn app_error(error: kassiber_ai::ToolError) -> AppError {
    match error {
        kassiber_ai::ToolError::App(error) => error,
        _ => AppError::new("internal_error", "limit could not be read"),
    }
}

//! What the user sees before consenting to a tool call
//! (`ai.chat.tool_consent_required`): the summary, the redacted arguments,
//! and for three families a server-side recomputation instead of the model's
//! own claims (`_ai_review_consent_preview`,
//! `_chain_analysis_acquisition_consent_preview`,
//! `_chain_analysis_dataset_consent_preview`). Every preview runs on the
//! request loop behind the frozen project and book check, writes nothing and
//! contacts nothing, and is redacted like a tool result.

use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{dumps, Key, Options, Value};

use crate::catalog::ToolEntry;
use crate::executor::{assert_scope, Book, ParsedCall, ReadHost, Runtime};
use crate::redact::{redact_ai_tool_result, redact_tool_arguments};
use crate::summary::summarize_tool_call;
use crate::ToolError;

/// The consent card for `call`: `call_id`, `name`, `summary`,
/// `arguments_preview` and, for `ui.review.apply`, `review_preview`. An
/// error where Python's summary raises.
pub fn consent_request(
    entry: &ToolEntry,
    call: &ParsedCall,
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
) -> Result<Value, kassiber_envelope::AppError> {
    let (arguments_preview, summary) = if PLAN_BOUND_CONSENT_TOOLS.contains(&entry.name.as_str())
        && call.argument_error.is_none()
    {
        // The card shows the daemon's own plan, not the model's figures.
        plan_consent_preview(entry, &call.arguments, runtime, host)
    } else {
        (
            preview_arguments(entry, &call.arguments, runtime, host),
            None,
        )
    };
    let summary = match summary {
        Some(summary) => s(summary),
        None => Value::Str(summarize_tool_call(entry, &call.arguments)?),
    };
    let mut card = vec![
        (Key::from("call_id"), Value::Str(call.call_id.clone())),
        (Key::from("name"), s(entry.name.as_str())),
        (Key::from("summary"), summary),
        (Key::from("arguments_preview"), arguments_preview),
    ];
    if entry.name == "ui.review.apply" {
        card.push((
            Key::from("review_preview"),
            review_preview(&call.arguments, runtime, host),
        ));
    }
    Ok(Value::Dict(card))
}

/// `_AI_PLAN_BOUND_CONSENT_TOOLS`: applies bound to a plan the daemon
/// recomputes for the card.
pub const PLAN_BOUND_CONSENT_TOOLS: [&str; 3] = [
    "ui.wallets.import_attachment.apply",
    "ui.wallets.import_document.apply",
    "ui.imports.rollback",
];

/// `_ai_plan_consent_preview(runtime, entry, arguments)`: recompute an import
/// or rollback plan for the consent card, and a summary built from it. An
/// unavailable or stale plan is shown as such; the apply re-checks
/// everything when it runs.
pub fn plan_consent_preview(
    entry: &ToolEntry,
    arguments: &Value,
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
) -> (Value, Option<String>) {
    let name = entry.name.as_str();
    let plan = scoped(runtime, host, &mut |book, state| {
        let full_access = crate::executor::full_access(state);
        match name {
            "ui.imports.rollback" => {
                crate::import_runs::rollback_for_ai(book, arguments, full_access, false)
            }
            // The import plans are not native yet: shown as unavailable, after
            // the checks Python makes first (the chat's export; a chat at all).
            "ui.wallets.import_attachment.apply" => {
                crate::chat::files::ai_chat_file(
                    state,
                    arguments,
                    Some(&crate::chat::files::CHAT_TABLE_KINDS),
                )?;
                Err(
                    crate::executor::not_native(name, crate::executor::ATTACHMENT_IMPORT_OWNER)
                        .into(),
                )
            }
            _ => {
                if get(state, "chat_files").is_none() {
                    return Err(AppError::new(
                        "ai_attachment_missing",
                        "Documents can only be read in a desktop or CLI chat",
                    )
                    .into());
                }
                Err(
                    crate::executor::not_native(name, crate::executor::DOCUMENT_IMPORT_OWNER)
                        .into(),
                )
            }
        }
    });
    let plan = match plan {
        Ok(plan) => plan,
        Err(ToolError::App(error)) => {
            let code = if error.code.is_empty() {
                "tool_error".to_owned()
            } else {
                error.code
            };
            return (
                dict([
                    ("status", s("unavailable")),
                    ("kind", s(name)),
                    ("code", s(code)),
                ]),
                None,
            );
        }
        Err(_) => {
            return (
                dict([
                    ("status", s("unavailable")),
                    ("kind", s(name)),
                    ("code", s("tool_error")),
                ]),
                None,
            )
        }
    };
    let status = if !crate::validate::py_eq(
        get(&plan, "plan_digest").unwrap_or(&Value::Null),
        get(arguments, "plan_digest").unwrap_or(&Value::Null),
    ) {
        "stale"
    } else if get(&plan, "ready").is_some_and(kassiber_envelope::truthy) {
        "ready"
    } else {
        "blocked"
    };
    let count = |value: Option<&Value>| match value {
        Some(Value::Null) | None => "0".to_owned(),
        Some(value) => kassiber_envelope::py_str(value),
    };
    let summary = if name == "ui.imports.rollback" {
        format!(
            "Delete {} row(s) created by an import run",
            count(get(&plan, "transactions_to_delete"))
        )
    } else {
        let empty = Value::Dict(Vec::new());
        let target = get(&plan, "target")
            .filter(|t| kassiber_envelope::truthy(t))
            .unwrap_or(&empty);
        let rows = get(&plan, "rows")
            .filter(|r| kassiber_envelope::truthy(r))
            .unwrap_or(&empty);
        let noun = if get(target, "action") == Some(&s("create")) {
            "new wallet"
        } else {
            "wallet"
        };
        let what = if name == "ui.wallets.import_document.apply" {
            "document"
        } else {
            "file"
        };
        let label = get(target, "label")
            .filter(|label| kassiber_envelope::truthy(label))
            .map_or_else(|| "?".to_owned(), kassiber_envelope::py_str);
        format!(
            "Import {} row(s) from the attached {what} into {noun} {label}",
            count(get(rows, "to_import"))
        )
    };
    (
        dict([("status", s(status)), ("kind", s(name)), ("plan", plan)]),
        Some(summary),
    )
}

/// `preview_arguments`: the redacted arguments, or for acquisition and
/// dataset consent the recomputed plan or validation they are bound to.
pub fn preview_arguments(
    entry: &ToolEntry,
    arguments: &Value,
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
) -> Value {
    match entry.name.as_str() {
        "ui.chain_analysis.acquire.apply" => acquisition_preview(arguments, false, runtime, host),
        "ui.chain_analysis.sources.authorize" => {
            acquisition_preview(arguments, true, runtime, host)
        }
        "ui.chain_analysis.datasets.import" | "ui.chain_analysis.datasets.import.start" => {
            dataset_preview(arguments, runtime, host)
        }
        _ => redact_tool_arguments(arguments),
    }
}

/// `_run_scoped_ai_operation(runtime, preview)` for a preview.
fn scoped(
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
    preview: &mut dyn FnMut(&mut dyn Book, &Value) -> Result<Value, ToolError>,
) -> Result<Value, ToolError> {
    let data_root = runtime.data_root.to_owned();
    let state = &*runtime.state;
    let mut work = |book: &mut dyn Book| -> Result<Value, ToolError> {
        assert_scope(book, &data_root, state)?;
        preview(book, state)
    };
    Ok(redact_ai_tool_result(&host.on_request_loop(&mut work)?))
}

fn blocked(code: &str) -> Value {
    dict([("status", s("blocked")), ("code", s(code))])
}

/// Every refusal reads as `{"status": "blocked", "code": ...}`; anything else
/// escapes as it does from Python's preview.
fn blocked_on(error: ToolError) -> Result<Value, ToolError> {
    match error {
        ToolError::App(error) => Ok(blocked(&error.code)),
        other => Err(other),
    }
}

/// `chain_analysis_cases.canonical(value)`.
fn canonical(value: &Value) -> Result<String, ToolError> {
    dumps(
        value,
        &Options::new().sort_keys(true).compact().allow_nan(false),
    )
    .map_err(|error| AppError::new("validation", error.to_string()).into())
}

fn on_device(state: &Value) -> bool {
    crate::executor::full_access(state)
}

/// `_chain_analysis_acquisition_consent_preview(runtime, args, recurring=...)`:
/// only a plan the book recomputes identically may be shown and consented.
fn acquisition_preview(
    arguments: &Value,
    recurring: bool,
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
) -> Value {
    let result = scoped(runtime, host, &mut |book, state| {
        if !on_device(state) {
            return Ok(blocked("agent_access_restricted"));
        }
        book.active_profile()?;
        let field = if recurring { "spec" } else { "args" };
        let supplied = match get(arguments, "plan") {
            Some(plan @ Value::Dict(_)) if matches!(get(plan, field), Some(Value::Dict(_))) => plan,
            _ => return Ok(blocked("validation")),
        };
        let kind = if recurring {
            "ui.chain_analysis.sources.plan"
        } else {
            "ui.chain_analysis.acquire.plan"
        };
        let current = match book.read(kind, get(supplied, field).expect("checked above"), "gui") {
            Ok(current) => current,
            Err(error) => return blocked_on(error),
        };
        let same = match (canonical(&current), canonical(supplied)) {
            (Ok(current), Ok(supplied)) => current == supplied,
            (Err(error), _) | (_, Err(error)) => return blocked_on(error),
        };
        if !same {
            return Ok(blocked("chain_analysis_stale"));
        }
        Ok(dict([("status", s("ready")), ("plan", current)]))
    });
    result
        .or_else(blocked_on)
        .unwrap_or_else(|_| blocked("tool_error"))
}

/// `_chain_analysis_dataset_consent_preview(runtime, args)`: consent binds to
/// the local source bytes the user already validated.
fn dataset_preview(arguments: &Value, runtime: &mut Runtime<'_>, host: &mut dyn ReadHost) -> Value {
    let result = scoped(runtime, host, &mut |book, state| {
        if !on_device(state) {
            return Ok(blocked("agent_access_restricted"));
        }
        let preview = match book.dataset_preview(arguments) {
            Ok(preview) => preview,
            Err(error) => return blocked_on(error),
        };
        let field = |key: &str| get(&preview, key).cloned().unwrap_or(Value::Null);
        Ok(dict([
            ("status", s("ready")),
            ("manifest", field("manifest")),
            ("row_count", field("row_count")),
            ("byte_count", field("byte_count")),
            ("sha256", field("sha256")),
        ]))
    });
    result
        .or_else(blocked_on)
        .unwrap_or_else(|_| blocked("tool_error"))
}

/// `_ai_review_consent_preview(runtime, arguments)`: the accounting effects
/// are recomputed, never taken from the model.
fn review_preview(arguments: &Value, runtime: &mut Runtime<'_>, host: &mut dyn ReadHost) -> Value {
    let result = scoped(runtime, host, &mut |book, state| {
        book.review_consent_preview(arguments, crate::executor::full_access(state))
    });
    match result {
        Ok(preview) => preview,
        Err(ToolError::App(error)) => dict([
            ("status", s("unavailable")),
            ("code", s(error.code.as_str())),
        ]),
        Err(_) => dict([("status", s("unavailable")), ("code", s("tool_error"))]),
    }
}

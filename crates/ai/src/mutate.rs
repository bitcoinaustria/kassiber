//! The mutating executor (`_execute_mutating_ai_tool`, `kassiber/daemon.py`)
//! and the custody AI audit around consented custody writes.
//!
//! Gates run in Python's order: argument errors, the catalog and its
//! `kind_class`, the local-provider custody gate, the per-turn advertised
//! set. Then, unlike Python, which leaves it to its chat loop, the call must
//! carry a consent bound to its call id (`consented_call_ids`): nothing here
//! writes the book or contacts the network on a request the user did not
//! approve. Then the schema and the route. Every write reaches the native
//! service the desktop and CLI use, authored `ai_tool` where Python authors
//! it so, on the daemon's request loop after the frozen project and book
//! check, and every result is redacted before it leaves this module.

use kassiber_envelope::{dict, get, s, set, success, AppError};
use kassiber_pyjson::{Key, PyStr, Value};

use crate::catalog::{get_tool_py, KindClass, ToolEntry};
use crate::executor::{
    assert_scope, consented, is_advertised, tool_result_denied, Book, ReadHost, Runtime,
};
use crate::projections::{redact_evidence_for_ai, redact_source_funds_for_ai};
use crate::redact::redact_ai_tool_result;
use crate::validate::validate_arguments;
use crate::ToolError;

/// `_CUSTODY_GAP_MUTATING_DAEMON_KINDS`.
pub const CUSTODY_GAP_MUTATING_KINDS: [&str; 1] = ["ui.custody.review.apply"];

/// `_SOURCE_FUNDS_MUTATING_AI_DAEMON_KINDS`.
const SOURCE_FUNDS_MUTATIONS: [&str; 11] = [
    "ui.source_funds.sources.create",
    "ui.source_funds.links.create",
    "ui.source_funds.links.review",
    "ui.source_funds.suggest",
    "ui.source_funds.links.bulk_review",
    "ui.source_funds.sources.attach",
    "ui.source_funds.links.attach",
    "ui.source_funds.assemble",
    "ui.source_funds.cases.save",
    "ui.source_funds.export_pdf",
    "ui.source_funds.export_bundle",
];

/// The commercial writes whose payloads pass the evidence filter.
const COMMERCIAL_MUTATIONS: [&str; 3] = [
    "ui.btcpay.provenance.review",
    // Seeding suggestions upserts link rows, so it is declared mutating.
    "ui.btcpay.provenance.suggest",
    "ui.documents.create",
];

/// Export targets whose renderers are not native yet (the PDF stage and the
/// audit package): refused after argument validation, before the book is
/// touched.
pub const REFUSED_EXPORTS: [&str; 7] = [
    "ui.reports.export_pdf",
    "ui.reports.export_summary_pdf",
    "ui.reports.export_austrian_e1kv_pdf",
    "ui.reports.export_exit_tax_pdf",
    "ui.reports.export_audit_package",
    "ui.source_funds.export_pdf",
    "ui.source_funds.export_bundle",
];

/// Mutating tools whose service is not native yet, with the work that owns it.
pub const REFUSED_MUTATIONS: [(&str, &str); 1] =
    [("ui.btcpay.provenance.review", "commercial evidence review")];

fn refuse_export(kind: &str) -> Result<(), ToolError> {
    if REFUSED_EXPORTS.contains(&kind) {
        return Err(crate::executor::not_native(kind, "the PDF and audit-package stage").into());
    }
    Ok(())
}

/// `CustodyAiConsentAudit`: the consent facts a custody write is audited with.
#[derive(Clone, Debug, PartialEq)]
pub struct CustodyAudit {
    pub provider_kind: String,
    pub model: String,
    pub consent_decision: String,
    pub consent_requested_at: String,
    pub consent_decided_at: String,
}

/// `_execute_mutating_ai_tool(call, runtime, custody_audit=...)`.
pub fn execute_mutating(
    call: &crate::executor::ParsedCall,
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
    custody_audit: Option<&CustodyAudit>,
) -> Value {
    if let Some(error) = call.argument_error {
        return tool_result_denied(error, None);
    }
    let Some(entry) =
        get_tool_py(&call.name).filter(|entry| entry.kind_class == KindClass::Mutating)
    else {
        return tool_result_denied("tool_not_allowed", None);
    };
    if entry
        .daemon_kind
        .as_deref()
        .is_some_and(|kind| CUSTODY_GAP_MUTATING_KINDS.contains(&kind))
        && !matches!(get(runtime.state, "provider_kind"), Some(Value::Str(kind)) if kind.is("local"))
    {
        return tool_result_denied(
            "local_provider_required",
            Some(&PyStr::from(
                "Custody-gap linkage is available to local AI providers only.",
            )),
        );
    }
    if !is_advertised(entry, runtime.state) {
        return tool_result_denied("tool_not_advertised", None);
    }
    if !consented(runtime.state, &call.call_id) {
        // The chat loop records the user's decision for this call id; a
        // write that reaches here without it is a request nobody approved.
        return tool_result_denied("consent_required", None);
    }
    match mutate(entry, call, runtime, host, custody_audit) {
        Ok(result) => result,
        Err(ToolError::App(error)) => {
            let code = if error.code.is_empty() {
                "tool_error"
            } else {
                error.code.as_str()
            };
            let message = match &error.message_codepoints {
                Some(message) => (**message).clone(),
                None => PyStr::from(error.message.as_str()),
            };
            tool_result_denied(code, Some(&message))
        }
        // The mutating executor has no lock-contention branch: Python's
        // `except Exception` answers every other failure the same way.
        Err(ToolError::Busy | ToolError::Unexpected) => tool_result_denied(
            "tool_error",
            Some(&PyStr::from("AI tool execution failed unexpectedly")),
        ),
    }
}

/// `{"ok": True, "envelope": build_envelope(kind, payload)}`.
fn envelope(kind: &str, payload: Value) -> Value {
    dict([
        ("ok", Value::Bool(true)),
        ("envelope", success(kind, payload)),
    ])
}

/// `_run_scoped_ai_mutation(runtime, callback)`: the frozen project and book
/// check, then the write, then the result redaction.
fn scoped(
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
    write: &mut dyn FnMut(&mut dyn Book, &mut Value) -> Result<Value, ToolError>,
) -> Result<Value, ToolError> {
    let data_root = runtime.data_root.to_owned();
    let state = &mut *runtime.state;
    let mut work = |book: &mut dyn Book| -> Result<Value, ToolError> {
        assert_scope(book, &data_root, state)?;
        write(book, state)
    };
    Ok(redact_ai_tool_result(&host.on_request_loop(&mut work)?))
}

fn unknown_keys(arguments: &Value, allowed: &[&str]) -> Vec<Value> {
    let mut unknown: Vec<PyStr> = match arguments {
        Value::Dict(pairs) => pairs
            .iter()
            .filter_map(|(key, _)| match key {
                Key::Str(name) if !allowed.iter().any(|known| name.is(known)) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    unknown.sort();
    unknown.into_iter().map(Value::Str).collect()
}

/// `{**arguments, key: value}`.
fn with(arguments: &Value, key: &str, value: Value) -> Value {
    let mut arguments = arguments.clone();
    set(&mut arguments, key, value);
    arguments
}

/// `_ai_report_export_target(arguments)`: the export kind and the arguments
/// it may receive.
pub fn report_export_target(arguments: &Value) -> Result<(&'static str, Value), AppError> {
    let (Some(Value::Str(report)), Some(Value::Str(format))) =
        (get(arguments, "report"), get(arguments, "format"))
    else {
        return Err(AppError::new(
            "validation",
            "ui.reports.export requires report and format",
        ));
    };
    let mapping: [(&str, &str, &'static str); 11] = [
        ("full", "pdf", "ui.reports.export_pdf"),
        ("full", "xlsx", "ui.reports.export_xlsx"),
        ("full", "csv", "ui.reports.export_csv"),
        ("summary", "pdf", "ui.reports.export_summary_pdf"),
        (
            "capital_gains",
            "csv",
            "ui.reports.export_capital_gains_csv",
        ),
        (
            "austrian_e1kv",
            "pdf",
            "ui.reports.export_austrian_e1kv_pdf",
        ),
        (
            "austrian_e1kv",
            "xlsx",
            "ui.reports.export_austrian_e1kv_xlsx",
        ),
        (
            "austrian_e1kv",
            "csv",
            "ui.reports.export_austrian_e1kv_csv",
        ),
        ("exit_tax", "pdf", "ui.reports.export_exit_tax_pdf"),
        ("exit_tax", "xlsx", "ui.reports.export_exit_tax_xlsx"),
        (
            "audit_package",
            "package",
            "ui.reports.export_audit_package",
        ),
    ];
    let Some(kind) = mapping
        .iter()
        .find(|(r, f, _)| report.is(r) && format.is(f))
        .map(|(_, _, kind)| *kind)
    else {
        return Err(AppError::new(
            "validation",
            format!(
                "Unsupported {} / {} export combination",
                kassiber_envelope::py_str(&Value::Str(report.clone())),
                kassiber_envelope::py_str(&Value::Str(format.clone()))
            ),
        )
        .with_details(dict([
            ("report", Value::Str(report.clone())),
            ("format", Value::Str(format.clone())),
        ])));
    };
    let allowed: &[&str] = match kind {
        "ui.reports.export_pdf" | "ui.reports.export_csv" | "ui.reports.export_summary_pdf" => {
            &["wallet"]
        }
        "ui.reports.export_xlsx" => &["wallet", "verify"],
        "ui.reports.export_capital_gains_csv"
        | "ui.reports.export_austrian_e1kv_pdf"
        | "ui.reports.export_austrian_e1kv_xlsx"
        | "ui.reports.export_austrian_e1kv_csv" => &["year"],
        "ui.reports.export_exit_tax_pdf" | "ui.reports.export_exit_tax_xlsx" => {
            &["departure_date", "destination"]
        }
        _ => &["transaction", "transactions", "source_funds_case"],
    };
    let mut forwarded: Vec<(Key, Value)> = match arguments {
        Value::Dict(pairs) => pairs
            .iter()
            .filter(|(key, _)| matches!(key, Key::Str(name) if allowed.iter().any(|a| name.is(a))))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    if kind == "ui.reports.export_summary_pdf" {
        if let Some(index) = forwarded
            .iter()
            .position(|(key, _)| *key == Key::from("wallet"))
        {
            let (_, wallet) = forwarded.remove(index);
            if matches!(wallet, Value::Str(_)) {
                forwarded.push((
                    Key::from("wallets"),
                    Value::List(vec![get(arguments, "wallet")
                        .cloned()
                        .unwrap_or(Value::Null)]),
                ));
            }
        }
    }
    let forwarded = Value::Dict(forwarded);
    if report.is("exit_tax")
        && (!matches!(get(&forwarded, "departure_date"), Some(Value::Str(_)))
            || !matches!(get(&forwarded, "destination"), Some(Value::Str(d)) if d.is("eu_eea") || d.is("third_country")))
    {
        return Err(AppError::new(
            "validation",
            "Exit-tax export requires departure_date and destination",
        ));
    }
    Ok((kind, forwarded))
}

/// `_coerce_wallets_sync_args(raw_args, strict=True)`.
pub fn strict_wallets_sync_args(arguments: &Value) -> Result<Value, AppError> {
    let unknown = unknown_keys(
        arguments,
        &["wallet", "all", "force_full", "process_journals"],
    );
    if !unknown.is_empty() {
        return Err(AppError::new(
            "validation",
            "ui.wallets.sync received unsupported arguments",
        )
        .with_details(dict([("unknown", Value::List(unknown))])));
    }
    let type_details = |value: &Value| dict([("type", s(kassiber_envelope::type_name(value)))]);
    let wallet = match get(arguments, "wallet") {
        None | Some(Value::Null) => None,
        Some(Value::Str(text))
            if kassiber_pytext::strip_py(text)
                .code_points()
                .next()
                .is_some() =>
        {
            Some(kassiber_pytext::strip_py(text))
        }
        Some(other) => {
            return Err(AppError::new(
                "validation",
                "ui.wallets.sync wallet must be a non-empty string",
            )
            .with_details(type_details(other)))
        }
    };
    let boolean = |key: &str| -> Result<Option<bool>, AppError> {
        match get(arguments, key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(value)) => Ok(Some(*value)),
            Some(other) => Err(AppError::new(
                "validation",
                format!("ui.wallets.sync {key} must be a boolean"),
            )
            .with_details(type_details(other))),
        }
    };
    let all = boolean("all")?.unwrap_or(wallet.is_none());
    if all && wallet.is_some() {
        return Err(AppError::new(
            "validation",
            "ui.wallets.sync wallet and all are mutually exclusive",
        ));
    }
    let force_full = boolean("force_full")?.unwrap_or(false);
    let process_journals = boolean("process_journals")? != Some(false);
    Ok(dict([
        ("wallet", wallet.map_or(Value::Null, Value::Str)),
        ("all", Value::Bool(all)),
        ("force_full", Value::Bool(force_full)),
        ("process_journals", Value::Bool(process_journals)),
    ]))
}

/// Records an automatic report-read refresh in the turn's state, as
/// `_run_auto_sync_wallets` does: the busy answer comes before Python's
/// execution wrapper marks the turn as attempted.
pub fn record_auto_sync(state: &mut Value, payload: &Value) {
    if get(payload, "reason") != Some(&s("project_operation_in_progress")) {
        set(state, "auto_sync_attempted", Value::Bool(true));
    }
    set(state, "auto_sync", payload.clone());
}

fn mutate(
    entry: &'static ToolEntry,
    call: &crate::executor::ParsedCall,
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
    custody_audit: Option<&CustodyAudit>,
) -> Result<Value, ToolError> {
    validate_arguments(entry, &call.arguments)?;
    if let Some((_, owner)) = REFUSED_MUTATIONS
        .iter()
        .find(|(name, _)| *name == entry.name)
    {
        return Err(crate::executor::not_native(&entry.name, owner).into());
    }
    let arguments = &call.arguments;
    let kind = entry.daemon_kind.as_deref();
    if let Some(kind) = kind.filter(|kind| kind.starts_with("ui.chain_analysis.")) {
        return scoped(runtime, host, &mut |book, state| {
            let payload = crate::projections::chain_analysis_for_ai(kind, arguments, state, book)?;
            Ok(envelope(kind, payload))
        });
    }
    if entry.name == "ui.reports.export" {
        let (export_kind, export_args) = report_export_target(arguments)?;
        refuse_export(export_kind)?;
        return scoped(runtime, host, &mut |book, _| {
            let mut payload =
                redact_evidence_for_ai(&book.mutate(export_kind, &export_args, "gui")?);
            set(&mut payload, "artifact_kind", s(export_kind));
            set(&mut payload, "saved_locally", Value::Bool(true));
            Ok(envelope(export_kind, payload))
        });
    }
    if entry.name == "ui.source_funds.export" {
        let format = get(arguments, "format");
        if !matches!(format, Some(Value::Str(f)) if f.is("pdf") || f.is("bundle")) {
            return Err(AppError::new(
                "validation",
                "ui.source_funds.export format must be pdf or bundle",
            )
            .into());
        }
        let case = match get(arguments, "case") {
            Some(Value::Str(text)) => kassiber_pytext::strip_py(text),
            _ => PyStr::new(),
        };
        if case.code_points().next().is_none() {
            return Err(AppError::new(
                "validation",
                "ui.source_funds.export requires a saved case",
            )
            .into());
        }
        let export_kind = if format == Some(&s("pdf")) {
            "ui.source_funds.export_pdf"
        } else {
            "ui.source_funds.export_bundle"
        };
        let export_args = dict([("case", Value::Str(case))]);
        refuse_export(export_kind)?;
        return scoped(runtime, host, &mut |book, _| {
            let mut payload =
                redact_source_funds_for_ai(&book.mutate(export_kind, &export_args, "gui")?);
            set(&mut payload, "artifact_kind", s(export_kind));
            set(&mut payload, "saved_locally", Value::Bool(true));
            Ok(envelope(export_kind, payload))
        });
    }
    let Some(kind) = kind else {
        return Ok(tool_result_denied("tool_not_allowed", None));
    };
    match kind {
        "ui.wallets.sync" => {
            let args = strict_wallets_sync_args(arguments)?;
            // The assistant's consent covers refreshing sources; it does not
            // author transfer pairs, so the journal step only rebuilds.
            scoped(runtime, host, &mut |book, _| {
                Ok(envelope(kind, book.mutate(kind, &args, "ai_tool")?))
            })
        }
        "ui.journals.process" => {
            let unknown = unknown_keys(arguments, &[]);
            if !unknown.is_empty() {
                return Err(AppError::new(
                    "validation",
                    "ui.journals.process received unsupported arguments",
                )
                .with_details(dict([("unknown", Value::List(unknown))]))
                .into());
            }
            // "Process journals" consent rebuilds derived state only.
            scoped(runtime, host, &mut |book, _| {
                Ok(envelope(
                    kind,
                    book.mutate(kind, &Value::Dict(Vec::new()), "ai_tool")?,
                ))
            })
        }
        "ui.review.apply" | "ui.journals.quarantine.resolve" => {
            scoped(runtime, host, &mut |book, _| {
                Ok(envelope(kind, book.mutate(kind, arguments, "ai_tool")?))
            })
        }
        "ui.rates.rebuild" | "ui.rates.latest" | "ui.maintenance.configure" => {
            scoped(runtime, host, &mut |book, _| {
                Ok(envelope(kind, book.mutate(kind, arguments, "gui")?))
            })
        }
        "ui.maintenance.run" => scoped(runtime, host, &mut |book, state| {
            let attempted =
                get(state, "auto_sync_attempted").is_some_and(kassiber_envelope::truthy);
            let (payload, sync) = book.maintenance_run(arguments, attempted)?;
            if let Some(sync) = sync {
                record_auto_sync(state, &sync);
            }
            Ok(envelope(kind, payload))
        }),
        "ui.transactions.metadata.update" | "ui.transactions.history.revert" => {
            let args = with(arguments, "source", s("ai_tool"));
            scoped(runtime, host, &mut |book, _| {
                Ok(envelope(kind, book.mutate(kind, &args, "ai_tool")?))
            })
        }
        "ui.loans.mark" | "ui.loans.link" | "ui.loans.unmark" => {
            scoped(runtime, host, &mut |book, _| {
                Ok(envelope(kind, book.mutate(kind, arguments, "gui")?))
            })
        }
        "ui.attachments.copy" => scoped(runtime, host, &mut |book, _| {
            Ok(envelope(
                kind,
                redact_evidence_for_ai(&book.mutate(kind, arguments, "gui")?),
            ))
        }),
        _ if COMMERCIAL_MUTATIONS.contains(&kind) => scoped(runtime, host, &mut |book, _| {
            Ok(envelope(
                kind,
                redact_evidence_for_ai(&book.mutate(kind, arguments, "gui")?),
            ))
        }),
        _ if SOURCE_FUNDS_MUTATIONS.contains(&kind) => scoped(runtime, host, &mut |book, _| {
            Ok(envelope(
                kind,
                redact_source_funds_for_ai(&book.mutate(kind, arguments, "gui")?),
            ))
        }),
        _ if CUSTODY_GAP_MUTATING_KINDS.contains(&kind) => {
            let call_id = call.call_id.clone();
            scoped(runtime, host, &mut |book, state| {
                let Some(audit) = custody_audit else {
                    return Err(AppError::new(
                        "custody_ai_audit_required",
                        "Custody AI write is missing its consent audit context",
                    )
                    .into());
                };
                let (Some(workspace @ Value::Str(_)), Some(profile @ Value::Str(_))) = (
                    get(state, "scope_workspace_id"),
                    get(state, "scope_profile_id"),
                ) else {
                    return Err(AppError::new(
                        "stale_context",
                        "Custody AI write has no frozen book scope",
                    )
                    .retryable(true)
                    .into());
                };
                let record = audit_record(
                    entry, kind, &call_id, arguments, workspace, profile, audit, "executed", None,
                );
                let payload = book.custody_assisted_write(kind, arguments, &record)?;
                Ok(envelope(kind, payload))
            })
        }
        _ if kind.starts_with("ui.transfers.") || kind.starts_with("ui.saved_views.") => {
            if kind == "ui.transfers.update"
                && !["kind", "policy", "notes"]
                    .iter()
                    .any(|key| get(arguments, key).is_some())
            {
                return Err(AppError::new(
                    "validation",
                    "ui.transfers.update requires kind, policy, or notes",
                )
                .into());
            }
            scoped(runtime, host, &mut |book, _| {
                Ok(envelope(kind, book.mutate(kind, arguments, "ai_tool")?))
            })
        }
        _ => Ok(tool_result_denied("tool_not_allowed", None)),
    }
}

/// The arguments of `core_custody_ai_audit.append_assistance_record` the
/// executor knows; the host adds the outcome and writes the row.
#[allow(clippy::too_many_arguments)]
pub fn audit_record(
    entry: &ToolEntry,
    kind: &str,
    call_id: &PyStr,
    arguments: &Value,
    workspace: &Value,
    profile: &Value,
    audit: &CustodyAudit,
    execution_status: &str,
    execution_code: Option<&str>,
) -> Value {
    dict([
        ("workspace_id", workspace.clone()),
        ("profile_id", profile.clone()),
        ("tool_name", s(entry.name.as_str())),
        ("daemon_kind", s(kind)),
        ("call_id", Value::Str(call_id.clone())),
        ("provider_kind", s(audit.provider_kind.as_str())),
        ("model", s(audit.model.as_str())),
        ("model_proposal", arguments.clone()),
        ("final_proposal", arguments.clone()),
        ("consent_decision", s(audit.consent_decision.as_str())),
        (
            "consent_requested_at",
            s(audit.consent_requested_at.as_str()),
        ),
        ("consent_decided_at", s(audit.consent_decided_at.as_str())),
        ("execution_status", s(execution_status)),
        ("execution_code", execution_code.map_or(Value::Null, s)),
    ])
}

/// `_record_nonexecuted_custody_ai_call`: a denied, timed-out or cancelled
/// custody proposal is audited in the book even though nothing ran. Failures
/// are logged by Python and never reach the chat.
#[allow(clippy::too_many_arguments)]
pub fn record_nonexecuted_custody_call(
    call: &crate::executor::ParsedCall,
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
    audit: &CustodyAudit,
    execution_status: &str,
    execution_code: &str,
) -> Result<(), ToolError> {
    let Some(entry) = get_tool_py(&call.name) else {
        return Ok(());
    };
    let Some(kind) = entry
        .daemon_kind
        .as_deref()
        .filter(|kind| CUSTODY_GAP_MUTATING_KINDS.contains(kind))
    else {
        return Ok(());
    };
    let data_root = runtime.data_root.to_owned();
    let state = &*runtime.state;
    let mut work = |book: &mut dyn Book| -> Result<Value, ToolError> {
        assert_scope(book, &data_root, state)?;
        let (Some(workspace @ Value::Str(_)), Some(profile @ Value::Str(_))) = (
            get(state, "scope_workspace_id"),
            get(state, "scope_profile_id"),
        ) else {
            return Err(AppError::new(
                "stale_context",
                "Custody AI audit has no frozen book scope",
            )
            .retryable(true)
            .into());
        };
        let record = audit_record(
            entry,
            kind,
            &call.call_id,
            &call.arguments,
            workspace,
            profile,
            audit,
            execution_status,
            Some(execution_code),
        );
        book.append_custody_audit(&record)
    };
    host.on_request_loop(&mut work).map(|_| ())
}

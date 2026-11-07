//! The read executor (`_execute_read_only_ai_tool`, `kassiber/daemon.py:6569`)
//! and the call parsing around it.
//!
//! Gates run in Python's order: argument errors, the catalog and its
//! `kind_class`, the local-provider custody gate, the per-turn advertised
//! set, the schema, then the read. Every read runs on the daemon's request
//! loop through [`ReadHost`], first checking that the open book is still the
//! project and book frozen for the turn (`_run_scoped_ai_operation`). Every
//! result is redacted before it leaves this module. A tool whose service is
//! not native yet is refused with `not_implemented` before the book is
//! touched, and an egressing tool is refused unless a consent bound to this
//! call is supplied.

use kassiber_envelope::{dict, get, s, success, AppError};
use kassiber_pyjson::{Key, PyStr, Value};

use crate::catalog::{get_tool_py, KindClass, ToolEntry};
use crate::projections;
use crate::redact::redact_ai_tool_result;
use crate::skills::read_skill_reference;
use crate::validate::validate_arguments;
use crate::ToolError;

/// `_AI_AUTO_JOURNAL_REFRESH_TOOL_NAMES`: reads that rebuild stale journals
/// first, unless the caller disabled read maintenance (MCP).
pub const AUTO_JOURNAL_REFRESH_TOOLS: [&str; 20] = [
    "ui.audit.changes_since_last_answer",
    "ui.audit.evidence.summary",
    "ui.journals.quarantine",
    "ui.journals.snapshot",
    "ui.journals.transfers.list",
    "ui.next_actions",
    "ui.overview.snapshot",
    "ui.rates.coverage",
    "ui.report.blockers",
    "ui.reports.balance_history",
    "ui.reports.balance_sheet",
    "ui.reports.capital_gains",
    "ui.reports.exit_tax_preview",
    "ui.reports.portfolio_summary",
    "ui.reports.summary",
    "ui.reports.tax_summary",
    "ui.review.worklist",
    "ui.transactions.review_context",
    "ui.transfers.review_context",
    "ui.workspace.health",
];

/// `_LOCAL_CUSTODY_READ_DAEMON_KINDS`: custody lineage reads, for the local
/// desktop workflow and local providers only.
pub const LOCAL_CUSTODY_READ_KINDS: [&str; 6] = [
    "ui.custody.coverage.snapshot",
    "ui.custody.gaps.history",
    "ui.custody.gaps.list",
    "ui.custody.gaps.review_context",
    "ui.custody.lineage.snapshot",
    "ui.custody.review.plan",
];

/// Read tools refused until the service behind them is native, with the
/// work that owns it (design doc §2).
pub const REFUSED_READS: [(&str, &str); 1] = [(
    "ui.connections.node.snapshot",
    "the Lightning node snapshot route",
)];

/// Reads whose payload carries a `safety` block that reports whether the
/// read prelude contacted the network.
const SAFETY_COMPOSITES: [&str; 2] = ["ui.transactions.review_context", "ui.review.worklist"];

/// `ParsedAiToolCall`.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedCall {
    pub call_id: PyStr,
    pub name: PyStr,
    pub arguments: Value,
    pub argument_error: Option<&'static str>,
}

impl ParsedCall {
    /// A call the daemon planned itself (auto-reads) or a test supplied.
    pub fn new(call_id: impl Into<PyStr>, name: impl Into<PyStr>, arguments: Value) -> Self {
        ParsedCall {
            call_id: call_id.into(),
            name: name.into(),
            arguments,
            argument_error: None,
        }
    }
}

/// `_parse_ai_tool_call(raw, index)`: a provider's tool call, with
/// `arguments` as a JSON object string, an object, or nothing.
pub fn parse_tool_call(raw: &Value, index: usize) -> ParsedCall {
    let call_id = match get(raw, "id") {
        Some(Value::Str(id)) if id.code_points().next().is_some() => id.clone(),
        _ => PyStr::from(format!("call_{index}")),
    };
    let failed = |call_id: PyStr, name: PyStr, error| ParsedCall {
        call_id,
        name,
        arguments: Value::Dict(Vec::new()),
        argument_error: Some(error),
    };
    let Some(function @ Value::Dict(_)) = get(raw, "function") else {
        return failed(call_id, PyStr::new(), "invalid_tool_call");
    };
    let name = match get(function, "name") {
        Some(Value::Str(name)) => name.clone(),
        _ => PyStr::new(),
    };
    match get(function, "arguments") {
        None | Some(Value::Null) => ParsedCall::new(call_id, name, Value::Dict(Vec::new())),
        Some(Value::Str(text)) if text.code_points().next().is_none() => {
            ParsedCall::new(call_id, name, Value::Dict(Vec::new()))
        }
        Some(arguments @ Value::Dict(_)) => ParsedCall::new(call_id, name, arguments.clone()),
        Some(Value::Str(text)) => match kassiber_pytext::loads_py(text) {
            Ok(arguments @ Value::Dict(_)) => ParsedCall::new(call_id, name, arguments),
            _ => failed(call_id, name, "invalid_arguments"),
        },
        Some(_) => failed(call_id, name, "invalid_arguments"),
    }
}

/// `_tool_result_denied(reason, message=...)`.
pub fn tool_result_denied(reason: &str, message: Option<&PyStr>) -> Value {
    let mut result = vec![
        (Key::from("ok"), Value::Bool(false)),
        (Key::from("reason"), s(reason)),
    ];
    if let Some(message) = message.filter(|message| message.code_points().next().is_some()) {
        result.push((
            Key::from("message"),
            redact_ai_tool_result(&Value::Str(message.clone())),
        ));
    }
    Value::Dict(result)
}

fn message_of(error: &AppError) -> PyStr {
    match &error.message_codepoints {
        Some(message) => (**message).clone(),
        None => PyStr::from(error.message.as_str()),
    }
}

/// The denial Python's executor returns for a failure.
pub fn denial_for(error: ToolError) -> Value {
    match error {
        ToolError::App(error) => {
            let code = if error.code.is_empty() {
                "tool_error"
            } else {
                &error.code
            };
            tool_result_denied(code, Some(&message_of(&error)))
        }
        ToolError::Busy => tool_result_denied(
            "database_busy",
            Some(&PyStr::from(
                "The database is busy in another Kassiber process; retry shortly.",
            )),
        ),
        ToolError::Unexpected => tool_result_denied(
            "tool_error",
            Some(&PyStr::from("AI tool execution failed unexpectedly")),
        ),
    }
}

/// What `_auto_maintain_for_read` did before an AI read.
#[derive(Debug, Clone, Default)]
pub struct Maintenance {
    /// The `ui.wallets.sync` payload of the automatic refresh, if attempted.
    pub auto_sync: Option<Value>,
    /// The `ui.journals.process` payload, if stale journals were rebuilt.
    pub journals: Option<Value>,
}

/// What a read needs from the open book. Implemented by the daemon; every
/// method runs on its request loop.
pub trait Book {
    /// `_assert_ai_runtime_database_scope`: whether the open database is the
    /// one `data_root` names.
    fn database_in_scope(&mut self, data_root: &str) -> Result<bool, ToolError>;
    /// `current_context_snapshot(conn)`: the active workspace and profile ids.
    fn context(&mut self) -> Result<(Value, Value), ToolError>;
    /// `_auto_maintain_for_read` for an AI read: the automatic wallet refresh
    /// payload when the book's report-read policy ran one (never twice in a
    /// turn), and the journal-processing payload when stale journals were
    /// rebuilt.
    fn maintain_for_read(&mut self, sync_attempted: bool) -> Result<Maintenance, ToolError>;
    /// One native read: the kind's payload, with `authored_source` passed to
    /// the services that record it.
    fn read(
        &mut self,
        kind: &str,
        arguments: &Value,
        authored_source: &'static str,
    ) -> Result<Value, ToolError>;
    /// `build_report_blockers_snapshot(conn)` with an automatic refresh's
    /// failure applied (`_apply_sync_failure_blocker`).
    fn report_blockers(&mut self, _auto_sync: Option<&Value>) -> Result<Value, ToolError> {
        Err(not_native("ui.report.blockers", "this book").into())
    }
    /// `core_file_analysis.analyze_file(source_file, column_map=...,
    /// sample_rows=...)` over the chat's staged attachment.
    fn analyze_file(&mut self, _source_file: &str, _arguments: &Value) -> Result<Value, ToolError> {
        Err(not_native("ui.wallets.analyze_file", "this book").into())
    }
    /// `get_egress_ledger().snapshot(after_id=..., limit=...)`.
    fn egress_ledger(&mut self, _after_id: i64, _limit: i64) -> Result<Value, ToolError> {
        Err(not_native("ui.egress.snapshot", "this book").into())
    }
    /// One native write for an AI call, with `authored_source` passed to the
    /// services that record it: the route the desktop uses, with the AI-path
    /// parameters Python passes (strict wallet sync without journal pairing,
    /// journal processing without pairing, `ai_tool` review and quarantine
    /// authorship).
    fn mutate(
        &mut self,
        kind: &str,
        _arguments: &Value,
        _authored_source: &'static str,
    ) -> Result<Value, ToolError> {
        Err(not_native(kind, "this book").into())
    }
    /// `_maintenance_run_payload(..., state=...)`: the payload, and the
    /// automatic refresh it attempted, if any. With `sync_attempted`, an
    /// `if_enabled` refresh is skipped as the turn already ran one.
    fn maintenance_run(
        &mut self,
        _arguments: &Value,
        _sync_attempted: bool,
    ) -> Result<(Value, Option<Value>), ToolError> {
        Err(not_native("ui.maintenance.run", "this book").into())
    }
    /// The consented custody write inside `SAVEPOINT
    /// custody_ai_assisted_write`, committed with its `executed` audit row; on
    /// failure rolled back, with a `failed` row committed instead.
    fn custody_assisted_write(
        &mut self,
        kind: &str,
        _arguments: &Value,
        _audit: &Value,
    ) -> Result<Value, ToolError> {
        Err(not_native(kind, "this book").into())
    }
    /// `SOURCES.preview(...)` over the normalized recipe: the completed local
    /// validation a dataset import is bound to.
    fn dataset_preview(&mut self, _arguments: &Value) -> Result<Value, ToolError> {
        Err(not_native("ui.chain_analysis.datasets.import", "this book").into())
    }
    /// `_ai_review_consent_preview`'s scoped body.
    fn review_consent_preview(&mut self, _arguments: &Value) -> Result<Value, ToolError> {
        Err(not_native("ui.review.apply", "this book").into())
    }
    /// `append_assistance_record(...)` then `commit()`.
    fn append_custody_audit(&mut self, _audit: &Value) -> Result<Value, ToolError> {
        Err(not_native("the custody AI audit", "this book").into())
    }
    /// `resolve_scope(conn, None, None)`: the active profile's id.
    fn active_profile(&mut self) -> Result<String, ToolError> {
        Err(not_native("the active profile", "this book").into())
    }
    /// `decode_ai_args`: chain-analysis handles in arguments back to subjects.
    fn decode_analysis_references(
        &mut self,
        _profile: &str,
        _arguments: &Value,
    ) -> Result<Value, ToolError> {
        Err(not_native("chain-analysis references", "this book").into())
    }
    /// `_reference(scope, value)`: one process- and book-scoped handle.
    fn issue_analysis_reference(
        &mut self,
        _profile: &str,
        _value: &str,
    ) -> Result<String, ToolError> {
        Err(not_native("chain-analysis references", "this book").into())
    }
}

/// Runs database work on the daemon's request loop.
pub trait ReadHost {
    fn on_request_loop(
        &mut self,
        work: &mut dyn FnMut(&mut dyn Book) -> Result<Value, ToolError>,
    ) -> Result<Value, ToolError>;
}

/// `AiToolRuntime`: the frozen data root and the turn's `maintenance_state`.
pub struct Runtime<'a> {
    pub data_root: &'a str,
    pub state: &'a mut Value,
}

impl Runtime<'_> {
    fn field(&self, key: &str) -> Option<&Value> {
        get(self.state, key)
    }

    fn is(&self, key: &str, text: &str) -> bool {
        matches!(self.field(key), Some(Value::Str(value)) if value.is(text))
    }
}

/// `_ai_tool_is_advertised(entry, runtime)`: without an advertised list
/// (direct callers outside a chat turn) the catalog allowlist applies.
pub fn is_advertised(entry: &ToolEntry, state: &Value) -> bool {
    match get(state, "advertised_tools") {
        Some(Value::List(advertised)) => advertised
            .iter()
            .any(|name| matches!(name, Value::Str(name) if name.is(entry.provider_name()))),
        _ => true,
    }
}

/// The owning work of a refused read tool, if it is refused.
pub fn refused_read(name: &str) -> Option<&'static str> {
    REFUSED_READS
        .iter()
        .find(|(tool, _)| *tool == name)
        .map(|(_, owner)| *owner)
}

/// Whether `state` holds a consent bound to this call (`consented_call_ids`).
/// The chat loop (R6-7) records it after the user's decision; nothing else
/// may run an egressing tool.
pub(crate) fn consented(state: &Value, call_id: &PyStr) -> bool {
    matches!(get(state, "consented_call_ids"), Some(Value::List(ids))
        if ids.iter().any(|id| matches!(id, Value::Str(id) if id == call_id)))
}

/// `_execute_read_only_ai_tool(call, runtime, planned_auto_read=...)`.
pub fn execute_read_only(
    call: &ParsedCall,
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
    planned_auto_read: bool,
) -> Value {
    if let Some(error) = call.argument_error {
        return tool_result_denied(error, None);
    }
    let Some(entry) =
        get_tool_py(&call.name).filter(|entry| entry.kind_class == KindClass::ReadOnly)
    else {
        return tool_result_denied("tool_not_allowed", None);
    };
    if entry
        .daemon_kind
        .as_deref()
        .is_some_and(|kind| LOCAL_CUSTODY_READ_KINDS.contains(&kind))
        && !runtime.is("provider_kind", "local")
    {
        return tool_result_denied(
            "local_provider_required",
            Some(&PyStr::from(
                "Custody lineage is available in the local desktop workflow and to local AI providers only.",
            )),
        );
    }
    if !planned_auto_read && !is_advertised(entry, runtime.state) {
        return tool_result_denied("tool_not_advertised", None);
    }
    match read(entry, call, runtime, host) {
        Ok(result) => result,
        Err(error) => denial_for(error),
    }
}

fn read(
    entry: &'static ToolEntry,
    call: &ParsedCall,
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
) -> Result<Value, ToolError> {
    validate_arguments(entry, &call.arguments)?;
    if entry.name == "read_skill_reference" {
        let Some(Value::Str(name)) = get(&call.arguments, "name") else {
            return Err(
                AppError::new("validation", "read_skill_reference requires a name string").into(),
            );
        };
        let reference = read_skill_reference(name)?;
        return Ok(redact_ai_tool_result(&dict([
            ("ok", Value::Bool(true)),
            ("envelope", success("read_skill_reference", reference)),
        ])));
    }
    let Some(kind) = entry.daemon_kind.as_deref() else {
        return Ok(tool_result_denied("tool_not_allowed", None));
    };
    if entry.egresses && !consented(runtime.state, &call.call_id) {
        // Auto-reads skip egressing tools, and the chat loop asks first:
        // reaching here without a bound consent is a request nobody made.
        return Ok(tool_result_denied("consent_required", None));
    }
    if let Some(owner) = refused_read(&entry.name) {
        return Err(not_native(&entry.name, owner).into());
    }
    let data_root = runtime.data_root.to_owned();
    // Models call tools by wire name (`ui_reports_summary`); decide on the
    // catalog name the refresh set uses
    // (tests/rust_port/repro_ai_refresh_wire_name.py).
    let refresh = AUTO_JOURNAL_REFRESH_TOOLS.contains(&entry.name.as_str())
        && !runtime.is("read_maintenance", "disabled");
    let state = &mut *runtime.state;
    let mut work = |book: &mut dyn Book| -> Result<Value, ToolError> {
        assert_scope(book, &data_root, state)?;
        let mut metadata: Vec<(Key, Value)> = Vec::new();
        let mut auto_sync_data = None;
        if refresh {
            let attempted =
                get(state, "auto_sync_attempted").is_some_and(kassiber_envelope::truthy);
            let maintenance = book.maintain_for_read(attempted)?;
            if let Some(auto_sync) = maintenance.auto_sync {
                // The busy answer comes before Python's execution wrapper
                // marks the turn as attempted.
                if get(&auto_sync, "reason")
                    != Some(&Value::Str("project_operation_in_progress".into()))
                {
                    kassiber_envelope::set(state, "auto_sync_attempted", Value::Bool(true));
                }
                kassiber_envelope::set(state, "auto_sync", auto_sync.clone());
                metadata.push((
                    Key::from("auto_sync"),
                    success("ui.wallets.sync", auto_sync.clone()),
                ));
                auto_sync_data = Some(auto_sync);
            }
            if let Some(processed) = maintenance.journals {
                kassiber_envelope::set(state, "auto_journal_process", processed.clone());
                metadata.push((
                    Key::from("auto_journal_process"),
                    success("ui.journals.process", processed),
                ));
            }
        }
        let mut payload = projections::read_for_ai(
            entry,
            kind,
            &call.arguments,
            state,
            auto_sync_data.as_ref(),
            book,
        )?;
        if SAFETY_COMPOSITES.contains(&kind) {
            if let Some(mut safety @ Value::Dict(_)) = get(&payload, "safety").cloned() {
                let contacted = auto_sync_data.is_some();
                kassiber_envelope::set(&mut safety, "network_contacted", Value::Bool(contacted));
                kassiber_envelope::set(
                    &mut safety,
                    "network_contact_reason",
                    if contacted {
                        s("opt_in_freshness_sync")
                    } else {
                        Value::Null
                    },
                );
                kassiber_envelope::set(&mut payload, "safety", safety);
            }
        }
        let mut result = vec![
            (Key::from("ok"), Value::Bool(true)),
            (Key::from("envelope"), success(kind, payload)),
        ];
        if kind != "ui.report.blockers" && sync_payload_has_errors(auto_sync_data.as_ref()) {
            let blockers = book.report_blockers(auto_sync_data.as_ref())?;
            result.push((
                Key::from("auto_report_blockers"),
                success("ui.report.blockers", blockers),
            ));
        }
        result.extend(metadata);
        Ok(Value::Dict(result))
    };
    let result = host.on_request_loop(&mut work)?;
    Ok(redact_ai_tool_result(&result))
}

/// `_sync_payload_has_errors(payload)`: the refresh failed, or a source did.
pub fn sync_payload_has_errors(payload: Option<&Value>) -> bool {
    let Some(payload @ Value::Dict(_)) = payload else {
        return false;
    };
    if get(payload, "ok") == Some(&Value::Bool(false)) {
        return true;
    }
    matches!(get(payload, "results"), Some(Value::List(rows)) if rows.iter().any(|row| {
        matches!(row, Value::Dict(_))
            && match get(row, "status") {
                Some(status) if kassiber_envelope::truthy(status) => {
                    kassiber_pytext::lower(&kassiber_envelope::py_str(status)) == "error"
                }
                _ => false,
            }
    }))
}

/// `_assert_ai_runtime_database_scope` and the frozen book check.
pub(crate) fn assert_scope(
    book: &mut dyn Book,
    data_root: &str,
    state: &Value,
) -> Result<(), ToolError> {
    if !book.database_in_scope(data_root)? {
        return Err(AppError::new(
            "stale_context",
            "The active project changed while the AI turn was running",
        )
        .with_hint("Ask the assistant again in the current project.")
        .retryable(true)
        .into());
    }
    let (Some(expected_workspace), Some(expected_profile)) = (
        get(state, "scope_workspace_id"),
        get(state, "scope_profile_id"),
    ) else {
        return Ok(());
    };
    let (workspace, profile) = book.context()?;
    if &workspace != expected_workspace || &profile != expected_profile {
        return Err(AppError::new(
            "stale_context",
            "The active book changed while the AI turn was running",
        )
        .with_hint("Ask the assistant again in the current book.")
        .with_details(dict([
            ("expected_workspace_id", expected_workspace.clone()),
            ("expected_profile_id", expected_profile.clone()),
            ("current_workspace_id", workspace),
            ("current_profile_id", profile),
        ]))
        .retryable(true)
        .into());
    }
    Ok(())
}

/// `_CUSTODY_GAP_MUTATING_DAEMON_KINDS`: custody linkage, for local
/// providers only.
pub const CUSTODY_GAP_MUTATING_KINDS: [&str; 1] = ["ui.custody.review.apply"];

/// Who owns the mutating executor that is not native yet.
pub const MUTATING_EXECUTOR_OWNER: &str = "the R6-5b mutating executor";

/// `_execute_mutating_ai_tool(call, runtime)` up to the point where Python
/// would write: its gates run in Python's order, then the call is refused
/// with `not_implemented` because the mutating executor is R6-5b's. Nothing
/// reaches the book.
pub fn execute_mutating(call: &ParsedCall, state: &Value) -> Value {
    if let Some(error) = call.argument_error {
        return tool_result_denied(error, None);
    }
    let Some(entry) =
        get_tool_py(&call.name).filter(|entry| entry.kind_class == KindClass::Mutating)
    else {
        return tool_result_denied("tool_not_allowed", None);
    };
    let local = matches!(get(state, "provider_kind"), Some(Value::Str(kind)) if kind.is("local"));
    if entry
        .daemon_kind
        .as_deref()
        .is_some_and(|kind| CUSTODY_GAP_MUTATING_KINDS.contains(&kind))
        && !local
    {
        return tool_result_denied(
            "local_provider_required",
            Some(&PyStr::from(
                "Custody-gap linkage is available to local AI providers only.",
            )),
        );
    }
    if !is_advertised(entry, state) {
        return tool_result_denied("tool_not_advertised", None);
    }
    if let Err(error) = validate_arguments(entry, &call.arguments) {
        return denial_for(error);
    }
    denial_for(not_native(&entry.name, MUTATING_EXECUTOR_OWNER).into())
}

/// The refusal for a tool whose service is not native yet.
pub fn not_native(name: &str, owner: &str) -> AppError {
    AppError::new(
        "not_implemented",
        format!("AI tool {name} is not implemented in the Rust backend yet ({owner})"),
    )
}

/// The model-visible bytes of a result: `json.dumps(json_ready(
/// redact_ai_tool_result(result)), sort_keys=True, separators=(",", ":"))`.
pub fn model_output(result: &Value) -> String {
    crate::planner::model_json(&redact_ai_tool_result(result))
}

/// `_run_auto_read_tools`: run the planner's reads that are local, read-only
/// and advertised this turn, emitting the `ai.chat.status`,
/// `ai.chat.tool_call` and `ai.chat.tool_result` records (without
/// `request_id`), and return the context message for the model, if any.
pub fn run_auto_reads(
    planned: &[crate::planner::AutoRead],
    runtime: &mut Runtime<'_>,
    host: &mut dyn ReadHost,
    emit: &mut dyn FnMut(Value),
    cancelled: &dyn Fn() -> bool,
) -> Option<String> {
    if planned.is_empty() {
        return None;
    }
    emit(success(
        "ai.chat.status",
        dict([
            ("phase", s("reading_local_context")),
            ("label", s("Reading local context")),
        ]),
    ));
    let advertised: Vec<PyStr> = match get(runtime.state, "advertised_tools") {
        Some(Value::List(names)) => names
            .iter()
            .filter_map(|name| match name {
                Value::Str(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let mut context = Vec::new();
    for (index, read) in planned.iter().enumerate() {
        if cancelled() {
            return None;
        }
        let call = ParsedCall::new(
            format!("auto_read_{}", index + 1),
            read.name,
            read.arguments.clone(),
        );
        let Some(entry) = crate::catalog::get_tool(read.name)
            .filter(|entry| entry.kind_class == KindClass::ReadOnly)
        else {
            continue;
        };
        // Auto-reads run without consent, so they never leave the machine.
        if entry.egresses || !advertised.iter().any(|name| name.is(entry.provider_name())) {
            continue;
        }
        let arguments = crate::redact::redact_tool_arguments(&read.arguments);
        emit(success(
            "ai.chat.tool_call",
            dict([
                ("call_id", Value::Str(call.call_id.clone())),
                ("name", s(entry.name.as_str())),
                ("arguments", arguments.clone()),
                ("kind_class", s(entry.kind_class.as_str())),
                ("needs_consent", Value::Bool(false)),
            ]),
        ));
        let result = execute_read_only(&call, runtime, host, true);
        crate::provenance::record_tool_usage(
            runtime.state,
            &PyStr::from(entry.name.as_str()),
            &result,
        );
        emit(tool_result_record(&call.call_id, &result));
        context.push(crate::planner::context_record(
            &entry.name,
            &read.arguments,
            &result,
        ));
    }
    if context.is_empty() || cancelled() {
        return None;
    }
    Some(crate::planner::auto_tool_context_for_model(&context))
}

/// The `ai.chat.tool_result` record: `{"call_id", **redacted_result}`.
pub fn tool_result_record(call_id: &PyStr, result: &Value) -> Value {
    let mut data = vec![(Key::from("call_id"), Value::Str(call_id.clone()))];
    if let Value::Dict(pairs) = redact_ai_tool_result(result) {
        for (key, value) in pairs {
            match data.iter_mut().find(|(existing, _)| *existing == key) {
                Some((_, slot)) => *slot = value,
                None => data.push((key, value)),
            }
        }
    }
    success("ai.chat.tool_result", Value::Dict(data))
}

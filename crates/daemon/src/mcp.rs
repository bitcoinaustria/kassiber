//! One MCP tool call against an open book (`kassiber/mcp/tools.py`
//! `run_tool`): `kassiber mcp serve` on a direct book, `kassiber mcp call`
//! and a broker's `mcp call` child all run here.
//!
//! The call stays within the user's External agents grant (#686). Reads go
//! through the AI read executor as an off-device provider's would, under
//! SQLite `query_only`; from the operator grant up a read may first refresh
//! stale journals locally (never the opt-in sync) and publish the local
//! analysis index. Writes within the grant run through the assistant's
//! mutating executor on a pinned book; the host's tool approval is the
//! consent, and custody writes are audited as `mcp_host`. The book is pinned
//! to the resolved workspace and profile without writing the stored context,
//! and the result is redacted again at this boundary.

use std::sync::Arc;

use kassiber_ai::executor::{execute_read_only, Book as AiBook, ParsedCall, ReadHost, Runtime};
use kassiber_ai::redact::redact_ai_tool_result;
use kassiber_ai::ToolError;
use kassiber_envelope::{dict, get, s, truthy, AppError};
use kassiber_pyjson::{PyStr, Value};
use kassiber_runtime::store::{Book, BookStore};

use crate::ai_tools::HostBook;
use crate::{Daemon, DaemonConfig};

/// What one call names.
pub struct ToolRequest<'a> {
    /// The MCP name (`transactions_list`).
    pub name: &'a PyStr,
    pub arguments: &'a Value,
    pub workspace: Option<&'a str>,
    pub profile: Option<&'a str>,
    /// The catalog project's name, shown in the result's `book`.
    pub project_name: Option<&'a str>,
    /// The user's grant, possibly narrowed (`read` by default).
    pub grant: &'a str,
    /// `full` or `restricted` (`restricted` by default).
    pub agent_access: &'a str,
    /// The host's self-reported name, cleaned; display only.
    pub client_name: Option<&'a str>,
}

/// The book as the executor's request loop: the caller already owns the
/// connection on this thread, so work runs in place (`_InlineMainThread`).
struct Inline<'a> {
    daemon: &'a mut Daemon,
}

impl ReadHost for Inline<'_> {
    fn on_request_loop(
        &mut self,
        work: &mut dyn FnMut(&mut dyn AiBook) -> Result<Value, ToolError>,
    ) -> Result<Value, ToolError> {
        work(&mut HostBook {
            daemon: &mut *self.daemon,
        })
    }
}

/// `mcp-<uuid4 hex>`: the call id the executor sees (never shown).
fn call_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut text = String::from("mcp-");
    for round in 0..2u8 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u8(round);
        text.push_str(&format!("{:016x}", hasher.finish()));
    }
    text
}

/// `run_tool(conn, ...)`: the redacted structured result, or the
/// executor's refusal as an `AppError` with its reason code. The book comes
/// back in every case, for the caller to close.
pub fn run_tool(
    store: Arc<dyn BookStore>,
    config: DaemonConfig,
    book: Box<dyn Book>,
    request: &ToolRequest<'_>,
) -> (Box<dyn Book>, Result<Value, AppError>) {
    let data_root = config.bootstrap.paths.data_root.clone();
    let mut daemon = Daemon::new(
        store,
        config,
        crate::output::Output::new(Box::new(std::io::sink())),
    );
    daemon.book = Some(book);
    let result = run(&mut daemon, &data_root, request);
    let book = daemon
        .book
        .take()
        .expect("the book stays open for the call");
    (book, result)
}

/// `_ANALYSIS_INDEX_READ_KINDS`: reads served from the local chain-analysis
/// index, a derived projection published lazily when it is read.
const ANALYSIS_INDEX_READ_KINDS: [&str; 7] = [
    "ui.chain_analysis.query",
    "ui.chain_analysis.entropy",
    "ui.chain_analysis.cases.compare",
    "ui.chain_analysis.watches.preview",
    "ui.chain_analysis.acquire.plan",
    "ui.chain_analysis.sources.plan",
    "ui.reports.privacy_mirror",
];

fn unknown_tool(name: &PyStr) -> AppError {
    let text = name
        .code_points()
        .filter_map(char::from_u32)
        .collect::<String>();
    AppError::new("unknown_tool", format!("unknown Kassiber MCP tool: {text}")).with_python_message(
        PyStr::from_code_points(
            "unknown Kassiber MCP tool: "
                .chars()
                .map(u32::from)
                .chain(name.code_points()),
        )
        .unwrap_or_default(),
    )
}

/// `_is_readonly_refusal(exc)`: SQLite refused a write under `query_only`.
fn readonly_refusal(error: &AppError) -> bool {
    matches!(
        error.details.as_ref().and_then(|details| get(details, "sqlite_error_code")),
        Some(Value::Int(code)) if i64::try_from(code.clone()).is_ok_and(|code| code & 0xFF == 8)
    )
}

/// Run `work` with the book pinned (`pinned_context_scope`), writable or
/// read-only, and always lift the pin.
fn pinned<T>(
    daemon: &mut Daemon,
    scope: (&str, &str),
    writable: bool,
    work: &mut dyn FnMut(&mut Daemon) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let pin = |daemon: &mut Daemon, scope| {
        let book = daemon.book_mut()?;
        if writable {
            book.pin_write_scope(scope)
        } else {
            book.pin_read_only_scope(scope)
        }
    };
    pin(daemon, Some(scope))?;
    let result = work(daemon);
    let released = pin(daemon, None);
    let result = result?;
    released?;
    Ok(result)
}

/// `_sync_analysis_index(conn, runtime, refresh=...)`: from the operator
/// grant up a stale index is published; at the read grant a stale one is
/// reported instead of rebuilt.
fn sync_analysis_index(
    daemon: &mut Daemon,
    scope: (&str, &str),
    refresh: bool,
) -> Result<(), AppError> {
    let mut revision = |daemon: &mut Daemon| {
        daemon
            .book_mut()?
            .chain_analysis_ops(None, None, "index.revision", &Value::Null)
            .map(|_| ())
    };
    if refresh {
        return pinned(daemon, scope, true, &mut revision);
    }
    match pinned(daemon, scope, false, &mut revision) {
        Ok(()) => Ok(()),
        Err(error) if error.code == "database_busy" => Err(AppError::new(
            "database_busy",
            "The database is busy in another Kassiber process; retry shortly.",
        )
        .retryable(true)),
        Err(error) if readonly_refusal(&error) => Err(AppError::new(
            "derived_state_stale",
            "Kassiber's local analysis index is out of date for this book",
        )
        .with_hint("At the read grant agents cannot refresh it. Ask the user to open the investigation or Privacy Mirror in Kassiber, or to grant operator access, then retry.")
        .retryable(true)),
        Err(error) => Err(error),
    }
}

fn run(daemon: &mut Daemon, data_root: &str, request: &ToolRequest<'_>) -> Result<Value, AppError> {
    use kassiber_mcp::tools::{available_tools, entry_for, refusal_for};
    let Some(tool) = entry_for(request.name) else {
        return Err(unknown_tool(request.name));
    };
    if !matches!(request.arguments, Value::Dict(_)) {
        return Err(AppError::new(
            "validation",
            "tool arguments must be a JSON object",
        ));
    }
    if let Some(refusal) = refusal_for(tool, request.grant, request.agent_access) {
        return Err(refusal);
    }
    if !tool.read_only() && !(request.workspace.is_some() && request.profile.is_some()) {
        // Unpinned, each call follows whichever book is active when it runs,
        // so a task could read one book and change another.
        return Err(AppError::new("book_pin_required", "changing a book needs a server pinned to it")
            .with_hint("Ask the user to start the server with `kassiber mcp serve --workspace ... --profile ...` (the desktop's command pins it)."));
    }
    let workspace = request.workspace.map(s);
    let profile = request.profile.map(s);
    let (workspace_id, profile_id) = daemon
        .book_mut()?
        .resolve_scope(workspace.as_ref(), profile.as_ref())?;
    let call = ParsedCall::new(
        call_id(),
        tool.entry.name.as_str(),
        request.arguments.clone(),
    );
    let mut state = dict([
        // The daemon's allowlist for this call: exactly what the grant lists.
        (
            "advertised_tools",
            Value::List(
                available_tools(request.grant, request.agent_access)
                    .map(|tool| s(tool.entry.provider_name()))
                    .collect(),
            ),
        ),
        // The agent's model is outside Kassiber's trust boundary: never on
        // device, so on-device gates stay shut unless access is full.
        ("provider_kind", s("mcp")),
        ("provider_on_device", Value::Bool(false)),
        (
            "provider_full_access",
            Value::Bool(request.agent_access == "full"),
        ),
        ("cross_book_read_allowed", Value::Bool(false)),
        // Reads never run the opt-in freshness sync; the journal refresh
        // (operator grant and up) runs before the read, below.
        ("read_maintenance", s("disabled")),
        ("scope_workspace_id", s(workspace_id.as_str())),
        ("scope_profile_id", s(profile_id.as_str())),
    ]);
    let scope = (workspace_id.as_str(), profile_id.as_str());
    // The resolved rows' labels, before the tool runs (a write may rename).
    let context = pinned(daemon, scope, false, &mut |daemon| {
        daemon.book_mut()?.context()
    })?;
    // The host's approval of this call is the consent, for a write and for
    // a read that leaves the machine (the operator grant and up); custody
    // writes record it in the custody AI audit as `mcp_host`.
    kassiber_envelope::set(
        &mut state,
        "consented_call_ids",
        Value::List(vec![Value::Str(call.call_id.clone())]),
    );
    let outcome = if !tool.read_only() {
        let approved_at = kassiber_ai::clock::now_iso();
        let audit = kassiber_ai::mutate::CustodyAudit {
            provider_kind: "mcp".into(),
            model: request.client_name.unwrap_or("unknown").to_owned(),
            consent_decision: "mcp_host".into(),
            consent_requested_at: approved_at.clone(),
            consent_decided_at: approved_at,
        };
        let outcome = pinned(daemon, scope, true, &mut |daemon| {
            Ok(kassiber_ai::mutate::execute_mutating(
                &call,
                &mut Runtime {
                    data_root,
                    state: &mut state,
                },
                &mut Inline { daemon },
                Some(&audit),
            ))
        })?;
        let ok = get(&outcome, "ok") == Some(&Value::Bool(true));
        daemon.log_ring.append(
            "info",
            line!(),
            "mcp write",
            vec![
                ("tool", s(tool.name.as_str())),
                ("client", s(request.client_name.unwrap_or("unknown"))),
                ("status", s(if ok { "executed" } else { "refused" })),
                (
                    "code",
                    if ok {
                        Value::Null
                    } else {
                        get(&outcome, "reason").cloned().unwrap_or(Value::Null)
                    },
                ),
            ],
            None,
        );
        outcome
    } else {
        kassiber_envelope::set(&mut state, "auto_sync_attempted", Value::Bool(true));
        let refresh = kassiber_setup::agent_access::grant_allows(request.grant, "operator");
        if refresh
            && kassiber_ai::executor::AUTO_JOURNAL_REFRESH_TOOLS.contains(&tool.entry.name.as_str())
        {
            // The read-triggered journal refresh, without the opt-in sync:
            // local, writing derived journal state only.
            pinned(daemon, scope, true, &mut |daemon| {
                daemon.book_mut()?.maintain_for_ai_read(true).map(|_| ())
            })?;
        }
        if tool
            .entry
            .daemon_kind
            .as_deref()
            .is_some_and(|kind| ANALYSIS_INDEX_READ_KINDS.contains(&kind))
        {
            sync_analysis_index(daemon, scope, refresh)?;
        }
        // SQLite enforces the read-only contract for the read itself.
        pinned(daemon, scope, false, &mut |daemon| {
            Ok(execute_read_only(
                &call,
                &mut Runtime {
                    data_root,
                    state: &mut state,
                },
                &mut Inline { daemon },
                false,
            ))
        })?
    };
    // Second redaction pass at the external boundary, after the executor's.
    let outcome = redact_ai_tool_result(&outcome);
    if !matches!(get(&outcome, "ok"), Some(Value::Bool(true))) {
        let reason = get(&outcome, "reason");
        let code = match reason {
            Some(reason) if truthy(reason) => kassiber_envelope::py_str(reason),
            _ => "tool_error".to_owned(),
        };
        let retryable = matches!(reason, Some(Value::Str(reason)) if reason.is("stale_context") || reason.is("database_busy"));
        let mut error = match get(&outcome, "message") {
            Some(Value::Str(message)) if message.code_points().next().is_some() => {
                AppError::new(code, String::new()).with_python_message(message.clone())
            }
            Some(message) if truthy(message) => {
                AppError::new(code, kassiber_envelope::py_str(message))
            }
            _ => AppError::new(code, "The Kassiber tool did not complete"),
        };
        if let Some(Value::Str(hint)) = get(&outcome, "hint") {
            error = error.with_hint(kassiber_envelope::py_str(&Value::Str(hint.clone())));
        }
        if let Some(details @ Value::Dict(_)) = get(&outcome, "details") {
            error = error.with_details(details.clone());
        }
        return Err(error.retryable(retryable));
    }
    let Some(envelope @ Value::Dict(_)) = get(&outcome, "envelope") else {
        return Err(AppError::new(
            "tool_error",
            "The Kassiber tool returned no result",
        ));
    };
    let book = redact_ai_tool_result(&dict([
        ("project", request.project_name.map_or(Value::Null, s)),
        ("workspace", s(context.workspace_label.as_str())),
        ("profile", s(context.profile_label.as_str())),
    ]));
    let field = |key: &str| get(envelope, key).cloned().unwrap_or(Value::Null);
    Ok(dict([
        ("tool", Value::Str(request.name.clone())),
        ("book", book),
        ("kind", field("kind")),
        ("schema_version", field("schema_version")),
        ("data", field("data")),
    ]))
}

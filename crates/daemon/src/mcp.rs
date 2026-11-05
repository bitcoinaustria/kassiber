//! One MCP tool call against an open book (`kassiber/mcp/tools.py`
//! `run_tool`): `kassiber mcp serve` on a direct book, `kassiber mcp call`
//! and a broker's `mcp call` child all run here.
//!
//! The call goes through the AI read executor as a remote provider would:
//! only the 44 advertised read tools, no journal refresh or sync, no
//! cross-book reads, local-only gates shut. SQLite `query_only` enforces the
//! read-only contract, and the book is pinned to the resolved workspace and
//! profile without writing the stored context. The result is redacted again
//! at this boundary.

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

fn run(daemon: &mut Daemon, data_root: &str, request: &ToolRequest<'_>) -> Result<Value, AppError> {
    let Some(entry) = kassiber_mcp::tools::entry_for(request.name) else {
        let name = request
            .name
            .code_points()
            .filter_map(char::from_u32)
            .collect::<String>();
        return Err(
            AppError::new("unknown_tool", format!("unknown Kassiber MCP tool: {name}"))
                .with_python_message(
                    PyStr::from_code_points(
                        "unknown Kassiber MCP tool: "
                            .chars()
                            .map(u32::from)
                            .chain(request.name.code_points()),
                    )
                    .unwrap_or_default(),
                ),
        );
    };
    if !matches!(request.arguments, Value::Dict(_)) {
        return Err(AppError::new(
            "validation",
            "tool arguments must be a JSON object",
        ));
    }
    let workspace = request.workspace.map(s);
    let profile = request.profile.map(s);
    let (workspace_id, profile_id) = daemon
        .book_mut()?
        .resolve_scope(workspace.as_ref(), profile.as_ref())?;
    let mut state = dict([
        (
            "advertised_tools",
            Value::List(
                kassiber_mcp::tools::mcp_entries()
                    .map(|entry| s(entry.provider_name()))
                    .collect(),
            ),
        ),
        // The agent's model is outside Kassiber's trust boundary: treat it as
        // a remote provider so local-only and on-device-only gates stay shut.
        ("provider_kind", s("remote")),
        ("provider_on_device", Value::Bool(false)),
        ("cross_book_read_allowed", Value::Bool(false)),
        ("read_maintenance", s("disabled")),
        ("auto_sync_attempted", Value::Bool(true)),
        ("scope_workspace_id", s(workspace_id.as_str())),
        ("scope_profile_id", s(profile_id.as_str())),
    ]);
    let call = ParsedCall::new(call_id(), entry.name.as_str(), request.arguments.clone());
    // SQLite enforces the read-only contract for the tool itself; opening
    // the book (schema compatibility) happened before, as for any CLI read.
    daemon
        .book_mut()?
        .pin_read_only_scope(Some((&workspace_id, &profile_id)))?;
    let executed = (|| {
        let context = daemon.book_mut()?.context()?;
        let outcome = execute_read_only(
            &call,
            &mut Runtime {
                data_root,
                state: &mut state,
            },
            &mut Inline {
                daemon: &mut *daemon,
            },
            false,
        );
        Ok::<_, AppError>((context, outcome))
    })();
    let released = daemon
        .book_mut()
        .and_then(|book| book.pin_read_only_scope(None));
    let (context, outcome) = executed?;
    released?;
    // Second redaction pass at the external boundary, after the executor's.
    let outcome = redact_ai_tool_result(&outcome);
    if !matches!(get(&outcome, "ok"), Some(Value::Bool(true))) {
        let reason = get(&outcome, "reason");
        let code = match reason {
            Some(reason) if truthy(reason) => kassiber_envelope::py_str(reason),
            _ => "tool_error".to_owned(),
        };
        let retryable = matches!(reason, Some(Value::Str(reason)) if reason.is("stale_context") || reason.is("database_busy"));
        let error = match get(&outcome, "message") {
            Some(Value::Str(message)) if message.code_points().next().is_some() => {
                AppError::new(code, String::new()).with_python_message(message.clone())
            }
            Some(message) if truthy(message) => {
                AppError::new(code, kassiber_envelope::py_str(message))
            }
            _ => AppError::new(code, "The Kassiber tool did not complete"),
        };
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

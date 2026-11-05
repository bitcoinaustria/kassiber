//! `kassiber mcp serve|tools|status|enable|disable|call`
//! (`kassiber/mcp/cli.py` and `kassiber/agent_access.py`).
//!
//! Every tool call is a finite, self-contained operation, which keeps the
//! server stateless and still lets it read encrypted books:
//!
//! - plaintext and `unattended` books open like an ordinary non-interactive
//!   CLI command, never prompting, and close again after the call;
//! - `brokered` books submit `kassiber mcp call` to the operator broker, so
//!   the lease's capability and scope checks apply and this process never
//!   sees the passphrase; the server never starts a broker;
//! - `manual` encrypted books answer `interaction_required`.
//!
//! Launch does nothing: no path resolution, database open, lock or socket
//! until the first `tools/call`. External agents are off unless the user
//! turned them on, and only the user's own terminal can turn them on.

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kassiber_daemon::mcp::ToolRequest;
use kassiber_envelope::{dict, get, s, truthy, AppError};
use kassiber_mcp::protocol::{
    Cancelled, Internal, McpServer, StdioLoop, ToolOutcome, ToolProvider,
};
use kassiber_mcp::tools::{entry_for, error_outcome, finish, INSTRUCTIONS};
use kassiber_operator::client::{
    AgentSessionHandle, BrokerClient, Host, SubmitOptions, TERMINAL_OPERATION_STATES,
};
use kassiber_operator::values::{as_str, py_str_value};
use kassiber_pyjson::{dumps, Options, PyStr, Value};
use kassiber_runtime::store::{BookStore, Selection, Unlock, UnlockMode};
use kassiber_runtime::BuildInfo;

use crate::argparse::Namespace;
use crate::run::Environment;

/// `_BROKER_POLL_SECONDS`.
pub(crate) const BROKER_POLL: Duration = Duration::from_millis(100);

/// The leaves that run tools, registered with the `native-mcp` feature
/// until every one of the 44 tools is native (design §7 item 1).
#[cfg_attr(not(feature = "native-mcp"), allow(dead_code))]
pub(crate) const GATED_COMMANDS: [&str; 2] = ["mcp.serve", "mcp.call"];

/// The leaves answered before state migration, path resolution or broker
/// routing: MCP clients may launch probe copies, and launch must not
/// migrate state, create a project layout or write to a stderr the client
/// may persist.
pub(crate) const EARLY_COMMANDS: [&str; 5] = [
    "mcp.serve",
    "mcp.tools",
    "mcp.status",
    "mcp.enable",
    "mcp.disable",
];

/// `_require_complete_pin(args)`: a half pin resolves against whichever
/// workspace or profile is active at call time, so it could silently read
/// a different book.
pub(crate) fn require_complete_pin(args: &Namespace) -> Result<(), AppError> {
    let set = |name: &str| args.text(name).is_some_and(|value| !value.is_empty());
    if set("workspace") != set("profile") {
        return Err(AppError::new(
            "validation",
            "pin a book with both --workspace and --profile, or with neither",
        )
        .with_hint("Use `kassiber workspaces list` and `kassiber profiles list` to find both."));
    }
    Ok(())
}

fn not_initialized(message: &str) -> AppError {
    AppError::new("not_initialized", message)
        .with_hint("Ask the user to set Kassiber up first (desktop app or `kassiber init`).")
}

/// `require_mcp_access()`: off unless the user turned external agents on
/// and the desktop's AI master switch is not off.
fn require_mcp_access(store: &dyn BookStore) -> Result<(), AppError> {
    kassiber_setup::agent_access::require_mcp_access(&store.agent_access_path()?)
}

/// `_require_existing_book_for_agent_read(args)`: `mcp call` reads; unlike
/// other CLI reads it never creates a project or a book.
pub(crate) fn require_existing_book(
    store: &dyn BookStore,
    selection: &Selection,
) -> Result<(), AppError> {
    require_mcp_access(store)?;
    if selection.data_root.is_none()
        && selection.project.is_none()
        && !store.catalog_has_projects()?
    {
        return Err(not_initialized(
            "No Kassiber project exists on this machine yet",
        ));
    }
    let paths = store.runtime_paths(selection)?;
    if !database_exists(&paths.database) {
        return Err(not_initialized(
            "No Kassiber book exists in the selected project yet",
        ));
    }
    Ok(())
}

fn database_exists(database: &str) -> bool {
    std::fs::metadata(database).is_ok_and(|metadata| metadata.len() > 0)
}

/// `mcp tools`, `status`, `enable` and `disable`: the payload to emit.
pub(crate) fn configure(
    store: &dyn BookStore,
    command: &str,
    non_interactive: bool,
    stdin_is_tty: bool,
    stdin: &mut dyn BufRead,
    stderr: &mut dyn Write,
) -> Result<Value, AppError> {
    use kassiber_setup::agent_access;
    match command {
        "mcp.tools" => Ok(kassiber_mcp::tools::describe_tools()),
        "mcp.status" => Ok(agent_access::status(&store.agent_access_path()?)),
        "mcp.disable" => agent_access::set(&store.agent_access_path()?, Some(false), None),
        _ => {
            // Turning disclosure on is the user's decision, never an agent's:
            // it needs their own terminal (or the desktop switch).
            if non_interactive || !stdin_is_tty {
                return Err(AppError::new(
                    "interaction_required",
                    "enabling external agents needs the user's own confirmation",
                )
                .with_hint(
                    "Ask the user to turn on External agents in Kassiber Settings > AI, or to run `kassiber mcp enable` in their own terminal.",
                )
                .with_details(dict([("reason", s("user_consent"))])));
            }
            let _ = stderr.write_all(
                b"External agents you connect will read this machine's Kassiber books (read-only), and their model provider receives what they read.\nAllow? [y/N] ",
            );
            let _ = stderr.flush();
            let mut line = String::new();
            let _ = stdin.read_line(&mut line);
            let answer = kassiber_pytext::lower(kassiber_pytext::strip(&line));
            let path = store.agent_access_path()?;
            if matches!(answer.as_str(), "y" | "yes") {
                agent_access::set(&path, Some(true), None)
            } else {
                Ok(agent_access::status(&path))
            }
        }
    }
}

/// `_parse_arguments(args.arguments)`: a JSON object, refusing `NaN` and
/// `Infinity`; the flag defaults to `{}`.
pub(crate) fn parse_arguments(raw: Option<&Value>) -> Result<Value, AppError> {
    let raw = match raw {
        Some(Value::Str(raw)) => raw.clone(),
        _ => PyStr::from("{}"),
    };
    match kassiber_pytext::loads_py_without_constants(&raw) {
        Ok(value @ Value::Dict(_)) => Ok(value),
        _ => Err(AppError::new(
            "validation",
            "--arguments must be a JSON object",
        )),
    }
}

/// How a direct read opens its book.
pub(crate) struct Open<'a> {
    pub selection: Selection,
    pub passphrase_fd: Option<i64>,
    pub allow_prompt: bool,
    pub env: &'a Environment,
}

/// Bootstrap, open, account the project, run one tool and close: the
/// direct path of the server and the body of `mcp call`. `arguments` runs
/// once the book is open (`mcp call` checks its pin and parses its
/// arguments there, as Python's dispatch does).
pub(crate) fn read_book(
    store: &Arc<dyn BookStore>,
    build: BuildInfo,
    open: Open<'_>,
    name: &PyStr,
    arguments: &dyn Fn() -> Result<Value, AppError>,
    workspace: Option<&str>,
    profile: Option<&str>,
) -> Result<Value, AppError> {
    let bootstrap = store.bootstrap(&open.selection)?;
    let mut runtime_config = bootstrap.runtime_config.clone();
    let expected = crate::run::operator_expected_database_identity(open.env)?;
    let mut book = store.open_book(
        &bootstrap.paths.data_root,
        Unlock::Cli {
            passphrase_fd: open.passphrase_fd,
            allow_prompt: open.allow_prompt,
            expected_database_identity: expected.as_deref(),
            initialize_version: None,
        },
        &mut runtime_config,
    )?;
    let opened = match &bootstrap.paths.project_id {
        Some(project_id) => store.mark_project_opened(
            project_id,
            &bootstrap.paths.data_root,
            !bootstrap.selection_explicit,
        ),
        None => Ok(()),
    }
    .and_then(|()| arguments());
    let arguments = match opened {
        Ok(arguments) => arguments,
        Err(error) => {
            let _ = book.close();
            return Err(error);
        }
    };
    let project_name = bootstrap.paths.project_name.clone();
    let mut bootstrap = bootstrap;
    bootstrap.runtime_config = runtime_config;
    let (mut book, outcome) = kassiber_daemon::mcp::run_tool(
        Arc::clone(store),
        kassiber_daemon::DaemonConfig { build, bootstrap },
        book,
        &ToolRequest {
            name,
            arguments: &arguments,
            workspace,
            profile,
            project_name: project_name.as_deref(),
        },
    );
    let closed = book.close();
    let result = outcome?;
    closed?;
    Ok(result)
}

/// One process's agent session for a book, shared by its calls.
struct Session {
    handle: Mutex<AgentSessionHandle>,
    token: PyStr,
}

impl Session {
    fn alive(&self) -> bool {
        self.handle
            .lock()
            .map(|handle| handle.alive())
            .unwrap_or(false)
    }

    fn close(&self) {
        if let Ok(mut handle) = self.handle.lock() {
            handle.close();
        }
    }
}

/// `BookToolProvider`: calls against the configured project, one at a time.
pub(crate) struct BookTools<H: Host + 'static> {
    pub store: Arc<dyn BookStore>,
    pub build: BuildInfo,
    pub env: Environment,
    pub client: Box<dyn Fn() -> BrokerClient<H> + Send + Sync>,
    pub selection: Selection,
    pub workspace: Option<String>,
    pub profile: Option<String>,
    /// Desktop agent sessions this server process holds, per data root.
    sessions: Mutex<Vec<(String, Arc<Session>)>>,
}

impl<H: Host + 'static> BookTools<H> {
    pub(crate) fn new(
        store: Arc<dyn BookStore>,
        build: BuildInfo,
        env: Environment,
        client: Box<dyn Fn() -> BrokerClient<H> + Send + Sync>,
        selection: Selection,
        workspace: Option<String>,
        profile: Option<String>,
    ) -> Self {
        BookTools {
            store,
            build,
            env,
            client,
            selection,
            workspace,
            profile,
            sessions: Mutex::new(Vec::new()),
        }
    }

    fn call(
        &self,
        name: &PyStr,
        arguments: &Value,
        cancelled: &Cancelled,
    ) -> Result<Value, AppError> {
        // First, before any project resolution: off unless the user turned
        // external agents on and the AI master switch is not off.
        require_mcp_access(self.store.as_ref())?;
        if self.selection.data_root.is_none() && !self.store.catalog_has_projects()? {
            // Resolving the default project would create one; an agent's read
            // must not set up Kassiber on a machine where nothing exists yet.
            return Err(not_initialized(
                "No Kassiber project exists on this machine yet",
            ));
        }
        let paths = self.store.runtime_paths(&self.selection)?;
        if !database_exists(&paths.database) {
            // Never let an agent's read create a new (plaintext) book.
            return Err(not_initialized(
                "No Kassiber book exists in the selected project yet",
            ));
        }
        let root = Selection {
            data_root: Some(paths.data_root.clone()),
            ..Selection::default()
        };
        if self.store.effective_unlock_mode(&root)? == UnlockMode::Brokered {
            return self.call_brokered(&paths.data_root, name, arguments, cancelled);
        }
        read_book(
            &self.store,
            self.build,
            Open {
                selection: Selection {
                    data_root: Some(paths.data_root),
                    env_file: self.selection.env_file.clone(),
                    project: None,
                },
                passphrase_fd: None,
                allow_prompt: false,
                env: &self.env,
            },
            name,
            &|| Ok(arguments.clone()),
            self.workspace.as_deref(),
            self.profile.as_deref(),
        )
    }

    /// `_agent_session(client, data_root, replacing=...)`: this process's
    /// registration for the book, opened when needed. Parallel callers that
    /// found the same stale session replace it only once.
    fn agent_session(
        &self,
        client: &BrokerClient<H>,
        data_root: &str,
        replacing: Option<&Arc<Session>>,
    ) -> Result<Arc<Session>, AppError> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let slot = sessions.iter().position(|(root, _)| root == data_root);
        if let Some(index) = slot {
            let existing = &sessions[index].1;
            if existing.alive() && !replacing.is_some_and(|stale| Arc::ptr_eq(stale, existing)) {
                return Ok(Arc::clone(existing));
            }
            existing.close();
        }
        let handle = client
            .open_agent_session(data_root)
            .map_err(crate::operator_cli::app_error)?;
        let session = Arc::new(Session {
            token: handle.token.clone(),
            handle: Mutex::new(handle),
        });
        match slot {
            Some(index) => sessions[index].1 = Arc::clone(&session),
            None => sessions.push((data_root.to_owned(), Arc::clone(&session))),
        }
        Ok(session)
    }

    fn call_brokered(
        &self,
        data_root: &str,
        name: &PyStr,
        arguments: &Value,
        cancelled: &Cancelled,
    ) -> Result<Value, AppError> {
        // Validate here, in this build, before anything is queued: the
        // broker child runs the broker's build, which must not be the only
        // check.
        let Some(entry) = entry_for(name) else {
            return Err(AppError::new("unknown_tool", "unknown Kassiber MCP tool")
                .with_python_message(join("unknown Kassiber MCP tool: ", name)));
        };
        kassiber_ai::validate::validate_arguments(entry, arguments).map_err(tool_error)?;
        let client = (self.client)();
        // Status never starts a broker; without a lease there is nothing to
        // submit to, so do not spawn one just to be refused.
        let status = client
            .status(Some(data_root))
            .map_err(crate::operator_cli::app_error)?;
        if !matches!(get(&status, "lease"), Some(Value::Str(lease)) if lease.is("unlocked")) {
            return Err(AppError::new(
                "interaction_required",
                "this project has no active operator lease",
            )
            .with_hint(
                "Ask the user to unlock this book for agents in Kassiber (Settings, AI, External agents), or to run `kassiber operator unlock --capability read` in their own terminal, then retry. Never ask for the passphrase.",
            )
            .with_details(dict([
                ("reason", s("operator_lease_required")),
                ("unlock_mode", s("brokered")),
                ("user_command", s("kassiber operator unlock --capability read")),
            ]))
            .retryable(true));
        }
        let default_scope = match get(&status, "default_scope") {
            Some(scope @ Value::Dict(_)) => scope.clone(),
            _ => Value::Dict(Vec::new()),
        };
        let pick = |pinned: &Option<String>, key: &str| -> Option<PyStr> {
            match pinned.as_deref().filter(|value| !value.is_empty()) {
                Some(value) => Some(PyStr::from(value)),
                None => get(&default_scope, key)
                    .filter(|value| truthy(value))
                    .map(py_str_value),
            }
        };
        let (Some(workspace), Some(profile)) = (
            pick(&self.workspace, "workspace"),
            pick(&self.profile, "profile"),
        ) else {
            return Err(AppError::new(
                "operator_scope_required",
                "a brokered book needs an explicit book scope",
            )
            .with_hint(
                "Start the server with `kassiber mcp serve --workspace ... --profile ...`.",
            ));
        };
        let text = |value: &PyStr| {
            value
                .code_points()
                .map(|point| char::from_u32(point).unwrap_or('\u{fffd}'))
                .collect::<String>()
        };
        let mut argv = vec!["--data-root".to_owned(), data_root.to_owned()];
        if let Some(env_file) = self.selection.env_file.as_deref() {
            // Absolute, so the path cannot change meaning in a worker.
            argv.extend(["--env-file".to_owned(), absolute(env_file)]);
        }
        argv.extend([
            "--machine".to_owned(),
            "mcp".to_owned(),
            "call".to_owned(),
            "--tool".to_owned(),
            text(name),
            "--arguments".to_owned(),
            dumps(arguments, &Options::new().compact())
                .map_err(|error| AppError::new("validation", error.to_string()))?,
            "--workspace".to_owned(),
            text(&workspace),
            "--profile".to_owned(),
            text(&profile),
        ]);
        let agent_scope = matches!(get(&status, "agent_scope"), Some(Value::Bool(true)));
        // The desktop's "Unlock for agents" session admits only processes
        // the user allowed; register this one so it can be allowed.
        let mut session = if agent_scope {
            Some(self.agent_session(&client, data_root, None)?)
        } else {
            None
        };
        let submit = |session: Option<&Arc<Session>>| -> Result<Value, AppError> {
            let mut prepared = prepare(&argv)?;
            let token = session.map(|session| session.token.clone());
            let token = token.as_ref().map(|token| text(token));
            let accepted = client.submit(
                data_root,
                &prepared,
                None,
                &SubmitOptions {
                    // The lease check above did not start a broker; a broker
                    // that has since exited must not be restarted by an
                    // agent's read.
                    start_broker: false,
                    // Its warning would be invisible (stderr is silenced), so
                    // an older broker that ignores caller context is refused.
                    require_caller_context: true,
                    // Agent reads run the broker's code; only this build's.
                    require_same_build: true,
                    agent_session_token: token.as_deref(),
                },
            );
            prepared.wipe();
            accepted.map_err(crate::operator_cli::app_error)
        };
        let accepted = match submit(session.as_ref()) {
            Ok(accepted) => accepted,
            Err(error)
                if agent_scope
                    && error.details.as_ref().is_some_and(|details| {
                        matches!(get(details, "reason"), Some(Value::Str(reason)) if reason.is("agent_session_required"))
                    }) =>
            {
                // The broker dropped this process's session (its lease
                // changed): register again, which the user must allow again.
                session = Some(self.agent_session(&client, data_root, session.as_ref())?);
                submit(session.as_ref())?
            }
            Err(error) => return Err(error),
        };
        let Some(operation_id) = as_str(get(&accepted, "operation_id")).cloned() else {
            return Err(AppError::new(
                "operator_protocol_error",
                "broker did not return an operation id",
            ));
        };
        let terminal = |status: &Value| matches!(get(status, "state"), Some(Value::Str(state)) if TERMINAL_OPERATION_STATES.iter().any(|t| state.is(t)));
        let mut completed = accepted;
        let mut polled = false;
        while !terminal(&completed) {
            if cancelled.is_set() {
                // Queued work is cancelled; a running read simply finishes.
                client
                    .cancel(&operation_id)
                    .map_err(crate::operator_cli::app_error)?;
                return Err(AppError::new("cancelled", "the tool call was cancelled"));
            }
            std::thread::sleep(BROKER_POLL);
            completed = client
                .operation_status(&operation_id)
                .map_err(crate::operator_cli::app_error)?;
            polled = true;
        }
        if !polled
            && get(&completed, "stdout").is_none()
            && get(&completed, "output_error").is_none()
        {
            // A replayed id can be answered terminal at admission, without
            // output.
            completed = client
                .operation_status(&operation_id)
                .map_err(crate::operator_cli::app_error)?;
        }
        brokered_result(&operation_id, &completed)
    }
}

fn join(prefix: &str, text: &PyStr) -> PyStr {
    PyStr::from_code_points(prefix.chars().map(u32::from).chain(text.code_points()))
        .unwrap_or_default()
}

/// The executor's validation failure as the AppError Python raises.
fn tool_error(error: kassiber_ai::ToolError) -> AppError {
    match error {
        kassiber_ai::ToolError::App(error) => error,
        kassiber_ai::ToolError::Busy | kassiber_ai::ToolError::Unexpected => unexpected(),
    }
}

fn unexpected() -> AppError {
    AppError::new("tool_error", "The Kassiber tool failed unexpectedly")
}

/// `prepare_arguments(argv, stdin=io.BytesIO())`: no secret flags are
/// expected, and none can read this process's stdin.
fn prepare(argv: &[String]) -> Result<kassiber_operator::client::PreparedArguments, AppError> {
    let mut stdin = std::io::empty();
    let open_fd = |_fd: i32| -> Box<dyn std::io::Read> { Box::new(std::io::empty()) };
    let token = || kassiber_operator::protocol::token_hex(16);
    kassiber_operator::arguments::prepare_arguments(
        argv,
        kassiber_operator::arguments::SecretSources {
            stdin: &mut stdin,
            open_fd: &open_fd,
            token: &token,
        },
    )
    .map_err(crate::operator_cli::app_error)
}

/// `os.path.abspath(path)`.
fn absolute(path: &str) -> String {
    let joined = if path.starts_with('/') {
        path.to_owned()
    } else {
        let cwd = std::env::current_dir()
            .map(|dir| dir.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{cwd}/{path}")
    };
    // posixpath.normpath for an absolute path.
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let leading = if joined.starts_with("//") && !joined.starts_with("///") {
        "//"
    } else {
        "/"
    };
    format!("{leading}{}", parts.join("/"))
}

/// `str(value or default)`.
fn text_or(value: Option<&Value>, default: &str) -> PyStr {
    match value {
        Some(value) if truthy(value) => py_str_value(value),
        _ => PyStr::from(default),
    }
}

fn app_error(code: PyStr, message: PyStr) -> AppError {
    let code = code
        .code_points()
        .map(|point| char::from_u32(point).unwrap_or('\u{fffd}'))
        .collect::<String>();
    AppError::new(code, String::new()).with_python_message(message)
}

/// `_brokered_result(operation_id, completed)`.
fn brokered_result(operation_id: &PyStr, completed: &Value) -> Result<Value, AppError> {
    if let Some(output_error @ Value::Dict(_)) = get(completed, "output_error") {
        return Err(app_error(
            text_or(get(output_error, "code"), "operator_output_unavailable"),
            text_or(
                get(output_error, "message"),
                "The operator result output is unavailable.",
            ),
        )
        .with_hint("Narrow the request with the tool's limit or filter arguments."));
    }
    let envelope = match get(completed, "stdout") {
        Some(Value::Str(stdout))
            if kassiber_pytext::strip_py(stdout)
                .code_points()
                .next()
                .is_some() =>
        {
            kassiber_pytext::loads_py(stdout).ok()
        }
        _ => None,
    };
    if let Some(envelope @ Value::Dict(_)) = &envelope {
        if matches!(get(envelope, "kind"), Some(Value::Str(kind)) if kind.is("mcp.call")) {
            if let Some(data @ Value::Dict(_)) = get(envelope, "data") {
                // Redact again at this process's boundary, as the direct path does.
                return Ok(kassiber_ai::redact::redact_ai_tool_result(data));
            }
        }
        if let Some(error @ Value::Dict(_)) = get(envelope, "error") {
            let mut failure = app_error(
                text_or(get(error, "code"), "tool_error"),
                text_or(get(error, "message"), "The Kassiber tool did not complete"),
            )
            .retryable(get(error, "retryable").is_some_and(truthy));
            if let Some(Value::Str(hint)) = get(error, "hint") {
                failure.hint = Some(
                    hint.code_points()
                        .map(|point| char::from_u32(point).unwrap_or('\u{fffd}'))
                        .collect::<String>()
                        .into_boxed_str(),
                );
            }
            if let Some(details @ Value::Dict(_)) = get(error, "details") {
                failure = failure.with_details(details.clone());
            }
            return Err(failure);
        }
    }
    let state = get(completed, "state").cloned().unwrap_or(Value::Null);
    let code = match &state {
        Value::Str(state) => join("operator_operation_", state),
        _ => PyStr::from("operator_error"),
    };
    Err(app_error(
        code,
        join("the brokered tool call ended as ", &py_str_value(&state)),
    )
    .with_hint(
        "Check `kassiber operator status`; if the lease ended, ask the user to unlock again.",
    )
    .with_details(dict([
        ("operation_id", Value::Str(operation_id.clone())),
        ("state", state.clone()),
    ]))
    .retryable(matches!(&state, Value::Str(state) if state.is("cancelled"))))
}

impl<H: Host + 'static> ToolProvider for BookTools<H> {
    fn instructions(&self) -> &str {
        INSTRUCTIONS
    }

    fn tool_definitions(&self) -> Vec<Value> {
        kassiber_mcp::tools::tool_definitions()
    }

    fn has_tool(&self, name: &PyStr) -> bool {
        entry_for(name).is_some()
    }

    fn call_tool(
        &self,
        name: &PyStr,
        arguments: &Value,
        cancelled: &Cancelled,
    ) -> Result<ToolOutcome, Internal> {
        let called = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.call(name, arguments, cancelled)
        }));
        match called {
            Ok(Ok(structured)) => finish(structured),
            // Python's executor reports anything but an AppError as an
            // unexpected failure and never echoes raw exception text.
            Ok(Err(error)) if error.code == "internal_error" => Ok(error_outcome(&unexpected())),
            Ok(Err(error)) => Ok(error_outcome(&error)),
            Err(_) => Ok(error_outcome(&unexpected())),
        }
    }
}

/// `run_server(args)`: the stdio server until end of input; exit code 0.
pub(crate) fn serve<H: Host + 'static>(
    tools: BookTools<H>,
    version: &str,
    input: Box<dyn std::io::Read + Send>,
    output: Box<dyn Write + Send>,
    claim: bool,
) -> i32 {
    let (input, output): (Box<dyn std::io::Read + Send>, Box<dyn Write + Send>) = if claim {
        // Nothing else in the process can then read protocol input or
        // corrupt protocol output, and nothing reaches a stderr the client
        // may persist (logs stay in RAM).
        match kassiber_ipc::stdio::claim() {
            Ok(claimed) => (Box::new(claimed.input), Box::new(claimed.output)),
            Err(_) => return 1,
        }
    } else {
        (input, output)
    };
    let server = McpServer::new(Arc::new(tools), "kassiber", version);
    StdioLoop::new(Arc::new(server), input, output).run();
    0
}

#[cfg(test)]
mod tests;

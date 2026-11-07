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
//! turned them on, and only the user's own terminal can turn them on or
//! widen their grant (#686). The listing, instructions and every call follow
//! the recorded choice when they run, so a changed grant applies to the next
//! call.

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kassiber_daemon::mcp::ToolRequest;
use kassiber_envelope::{dict, get, s, truthy, AppError};
use kassiber_mcp::protocol::{
    Cancelled, Internal, McpServer, StdioLoop, ToolOutcome, ToolProvider,
};
use kassiber_mcp::tools::{
    call_capability, client_label, entry_for, error_outcome, finish, instructions_for, refusal_for,
    McpTool,
};
use kassiber_operator::client::{
    AgentSessionHandle, BrokerClient, Host, SubmitOptions, TERMINAL_OPERATION_STATES,
};
use kassiber_operator::values::{as_str, py_str_value};
use kassiber_pyjson::{dumps, Options, PyStr, Value};
use kassiber_runtime::store::{BookStore, Selection, Unlock, UnlockMode};
use kassiber_runtime::BuildInfo;
use kassiber_setup::agent_access::{grant_allows, narrower_grant, Access, GRANTS};

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
/// and the desktop's AI master switch is not off; the recorded choice.
fn require_mcp_access(store: &dyn BookStore) -> Result<Access, AppError> {
    kassiber_setup::agent_access::require_mcp_access(&store.agent_access_path()?)
}

/// `agent_access_status()`: never refuses; anything unreadable is the
/// defaults.
fn recorded(store: &dyn BookStore) -> Access {
    store
        .agent_access_path()
        .map(|path| kassiber_setup::agent_access::read(&path))
        .unwrap_or_default()
}

/// `_call_access(args)`: the grant one `mcp call` runs with, the user's
/// only ever narrowed. Inside an operator broker worker the call may use no
/// more than the broker admitted for it (`call_capability`, which the lease
/// had to cover); an explicit `--grant` is a ceiling everywhere.
pub(crate) fn call_access(
    store: &dyn BookStore,
    tool: Option<&str>,
    claimed: Option<&str>,
    operator_child: bool,
) -> Result<(&'static str, &'static str), AppError> {
    let status = require_mcp_access(store)?;
    let mut grant = status.grant;
    let known = |grant: &str| GRANTS.iter().copied().find(|name| *name == grant);
    if operator_child {
        let admitted = call_capability(tool, claimed).as_str();
        // Admin is above every grant: the narrower one stays the user's.
        if let Some(admitted) = known(admitted) {
            grant = known(narrower_grant(grant, admitted)).expect("a grant");
        }
    }
    if let Some(claimed) = claimed.and_then(known) {
        grant = known(narrower_grant(grant, claimed)).expect("a grant");
    }
    Ok((grant, status.agent_access))
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

/// `_GRANT_CONSENT_TEXT`.
fn grant_consent_text(grant: &str) -> &'static str {
    match grant {
        "operator" => "read and operate this machine's Kassiber books: process journals, export, record source-of-funds evidence, documents and analysis cases, and sync wallets and rates, which contacts your configured backends and rate sources",
        "accounting_decisions" => "read, operate and make accounting decisions in this machine's Kassiber books: everything operator allows (including wallet and rate syncs that contact your backends) plus resolving quarantine, pairing and dismissing transfers, custody, loan and source-of-funds reviews, and editing transaction notes, tags, exclusions and classification",
        _ => "read this machine's Kassiber books (read-only, no network)",
    }
}

/// `_ACCESS_CONSENT_TEXT`.
fn access_consent_text(access: &str) -> &'static str {
    match access {
        "full" => "with full access: unprojected results (txids, outpoints, labels) and the custody-gap, lineage and chain-analysis source tools",
        _ => "with restricted access: projected results only",
    }
}

/// `mcp tools`, `status`, `enable` and `disable`: the payload to emit.
/// `grant` and `agent_access` are `mcp enable`'s options.
#[allow(clippy::too_many_arguments)]
pub(crate) fn configure(
    store: &dyn BookStore,
    command: &str,
    non_interactive: bool,
    stdin_is_tty: bool,
    stdin: &mut dyn BufRead,
    stderr: &mut dyn Write,
    grant: Option<&str>,
    agent_access: Option<&str>,
) -> Result<Value, AppError> {
    use kassiber_setup::agent_access;
    match command {
        "mcp.tools" => {
            let status = recorded(store);
            Ok(kassiber_mcp::tools::describe_tools(
                status.grant,
                status.agent_access,
            ))
        }
        "mcp.status" => Ok(agent_access::status(&store.agent_access_path()?)),
        "mcp.disable" => agent_access::set_switches(&store.agent_access_path()?, Some(false), None),
        _ => {
            // Turning disclosure on, or widening what agents may do, is the
            // user's decision, never an agent's: it needs their own terminal
            // (or the desktop switch).
            if non_interactive || !stdin_is_tty {
                return Err(AppError::new(
                    "interaction_required",
                    "enabling external agents or changing their grant needs the user's own confirmation",
                )
                .with_hint(
                    "Ask the user to turn on External agents in Kassiber Settings > AI, or to run `kassiber mcp enable` in their own terminal.",
                )
                .with_details(dict([("reason", s("user_consent"))])));
            }
            let path = store.agent_access_path()?;
            let current = agent_access::read(&path);
            let grant = grant.unwrap_or(current.grant);
            let access = agent_access.unwrap_or(current.agent_access);
            let prompt = format!(
                "External agents you connect will {}, {}. Their model provider receives what they read, and your agent host's own tool approval is the only prompt before a change. Admin work (secrets, backups, deletes, passphrases) is never available to agents.\nAllow? [y/N] ",
                grant_consent_text(grant),
                access_consent_text(access),
            );
            let _ = stderr.write_all(prompt.as_bytes());
            let _ = stderr.flush();
            let mut line = String::new();
            let _ = stdin.read_line(&mut line);
            let answer = kassiber_pytext::lower(kassiber_pytext::strip(&line));
            if matches!(answer.as_str(), "y" | "yes") {
                agent_access::set(
                    &path,
                    agent_access::Changes {
                        mcp_enabled: Some(true),
                        grant: Some(s(grant)),
                        agent_access: Some(s(access)),
                        ..agent_access::Changes::default()
                    },
                )
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

/// The user's choice one call runs with.
pub(crate) struct Grant {
    pub grant: &'static str,
    pub agent_access: &'static str,
    /// The host's cleaned self-reported name (display only).
    pub client_name: Option<String>,
}

/// Bootstrap, open, account the project, run one tool and close: the
/// direct path of the server and the body of `mcp call`. `arguments` runs
/// once the book is open (`mcp call` checks its pin and parses its
/// arguments there, as Python's dispatch does); `grant` reads the user's
/// choice then too.
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_book(
    store: &Arc<dyn BookStore>,
    build: BuildInfo,
    open: Open<'_>,
    name: &PyStr,
    arguments: &dyn Fn() -> Result<Value, AppError>,
    workspace: Option<&str>,
    profile: Option<&str>,
    grant: &dyn Fn() -> Result<Grant, AppError>,
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
    .and_then(|()| Ok((arguments()?, grant()?)));
    let (arguments, grant) = match opened {
        Ok(opened) => opened,
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
            grant: grant.grant,
            agent_access: grant.agent_access,
            client_name: grant.client_name.as_deref(),
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
        client_name: Option<String>,
    ) -> Result<Value, AppError> {
        // First, before any project resolution: off unless the user turned
        // external agents on and the AI master switch is not off, and
        // refused when the tool is above the user's grant.
        let status = require_mcp_access(self.store.as_ref())?;
        let (grant, agent_access) = (status.grant, status.agent_access);
        let Some(tool) = entry_for(name) else {
            return Err(AppError::new("unknown_tool", "unknown Kassiber MCP tool")
                .with_python_message(join("unknown Kassiber MCP tool: ", name)));
        };
        if let Some(refusal) = refusal_for(tool, grant, agent_access) {
            return Err(refusal);
        }
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
            return self.call_brokered(
                &paths.data_root,
                tool,
                arguments,
                cancelled,
                grant,
                client_name.as_deref(),
            );
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
            &|| {
                Ok(Grant {
                    grant,
                    agent_access,
                    client_name: client_name.clone(),
                })
            },
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
        tool: &McpTool,
        arguments: &Value,
        cancelled: &Cancelled,
        grant: &str,
        client_name: Option<&str>,
    ) -> Result<Value, AppError> {
        // Validate here, in this build, before anything is queued: the
        // broker child runs the broker's build, which must not be the only
        // check.
        let name = &PyStr::from(tool.name.as_str());
        kassiber_ai::validate::validate_arguments(tool.entry, arguments).map_err(tool_error)?;
        let client = (self.client)();
        // Status never starts a broker; without a lease there is nothing to
        // submit to, so do not spawn one just to be refused.
        let status = client
            .status(Some(data_root))
            .map_err(crate::operator_cli::app_error)?;
        let unlock_command = format!("kassiber operator unlock --capability {grant}");
        if !matches!(get(&status, "lease"), Some(Value::Str(lease)) if lease.is("unlocked")) {
            return Err(AppError::new(
                "interaction_required",
                "this project has no active operator lease",
            )
            .with_hint(format!(
                "Ask the user to unlock this book for agents in Kassiber (Settings, AI, External agents), or to run `{unlock_command}` in their own terminal, then retry. Never ask for the passphrase."
            ))
            .with_details(dict([
                ("reason", s("operator_lease_required")),
                ("unlock_mode", s("brokered")),
                ("user_command", s(unlock_command.as_str())),
            ]))
            .retryable(true));
        }
        // The lease bounds what this call may do as much as the grant does:
        // a read under a `read` lease runs without the journal refresh, and a
        // write needs a lease that covers it.
        let leased = get(&status, "capability").cloned().unwrap_or(Value::Null);
        let effective = match &leased {
            Value::Str(leased) => GRANTS
                .iter()
                .copied()
                .find(|name| leased.is(name))
                .map_or("read", |leased| narrower_grant(grant, leased)),
            _ => "read",
        };
        let required = tool.required.as_str();
        if !grant_allows(effective, required) {
            let required_command = format!("kassiber operator unlock --capability {required}");
            return Err(AppError::new(
                "interaction_required",
                format!("the operator lease does not cover {}", tool.name),
            )
            .with_hint(format!(
                "Ask the user to lock and unlock this book for agents in Kassiber (Settings, AI, External agents), or to lock the lease and run `{required_command}` in their own terminal, then retry. Never ask for the passphrase."
            ))
            .with_details(dict([
                ("reason", s("operator_capability_insufficient")),
                ("required", s(required)),
                ("leased", leased.clone()),
                ("user_command", s(required_command.as_str())),
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
            // The broker requires its lease to cover this grant as well.
            "--grant".to_owned(),
            effective.to_owned(),
            "--workspace".to_owned(),
            text(&workspace),
            "--profile".to_owned(),
            text(&profile),
        ]);
        if let Some(client_name) = client_name.filter(|name| !name.is_empty()) {
            // One token, so a name can never read as another option.
            argv.push(format!("--client={client_name}"));
        }
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
            let token = token.as_ref().map(text);
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
                // Queued work is cancelled; a running call (a write too)
                // simply finishes.
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
    fn instructions(&self) -> String {
        instructions_for(recorded(self.store.as_ref()).grant)
    }

    fn tool_definitions(&self) -> Vec<Value> {
        let status = recorded(self.store.as_ref());
        kassiber_mcp::tools::tool_definitions(status.grant, status.agent_access)
    }

    fn has_tool(&self, name: &PyStr) -> bool {
        // Known at some grant: a call above the user's grant gets a tool
        // error naming the grant it needs. Admin-class and chat-only tools
        // stay unknown.
        entry_for(name).is_some()
    }

    fn call_tool(
        &self,
        name: &PyStr,
        arguments: &Value,
        cancelled: &Cancelled,
        client_name: Option<&Value>,
    ) -> Result<ToolOutcome, Internal> {
        let called = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.call(name, arguments, cancelled, client_label(client_name))
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

//! `kassiber operator ...` and brokered routing (`kassiber/operator/cli.py`).
//!
//! The seven password-authenticated leaves talk to the per-user broker
//! through `kassiber_operator::client`; `operator touch-id ...` and
//! `operator unlock --auth touch-id` past its interaction check are R6-4.
//! [`route_brokered`] hands an ordinary command of a `brokered` book to the
//! broker: it never starts one, sends work only to this build's broker,
//! names the accepted operation on stderr, waits for the result, cancels on
//! Ctrl-C (exit 130) and maps a result without child output to a typed
//! error. Secrets reach the broker only in `S` frames; the command line it
//! receives carries opaque labels in their place.

use std::io::Write;

use kassiber_envelope::{dict, s, AppError};
use kassiber_operator::client::{BrokerClient, Host, PreparedArguments, SubmitOptions};
use kassiber_operator::values::{as_str, get, py_eq};
use kassiber_pyjson::{PyStr, Value};
use kassiber_runtime::capabilities::{cli_capability, Capability};
use kassiber_runtime::store::{BookStore, Selection, UnlockMode};
use zeroize::Zeroizing;

use crate::argparse::Namespace;
use crate::render::{emit_error, Format, Sink};

/// `_DIRECT_COMMANDS`: never routed through a project's broker.
pub(crate) const DIRECT_COMMANDS: [&str; 8] = [
    "commands",
    "daemon",
    "chat",
    "offline",
    "operator",
    "projects",
    "update",
    "verify-download",
];

/// How the process may ask for a secret.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Interaction {
    pub non_interactive: bool,
    pub stdin_is_tty: bool,
}

/// The process's output, for routed results.
pub(crate) struct Output<'a> {
    pub stdout: &'a mut dyn Write,
    pub stderr: &'a mut dyn Write,
    pub output: Option<&'a str>,
    pub home: Option<&'a str>,
    pub format: Format,
    pub machine: bool,
}

pub(crate) use kassiber_operator::error::into_app_error as app_error;

/// `dispatch_operator(args)`: the payload `emit` prints.
pub(crate) fn dispatch<H: Host>(
    client: &BrokerClient<H>,
    store: &dyn BookStore,
    args: &Namespace,
    selection: &Selection,
    interaction: Interaction,
) -> Result<Value, AppError> {
    let command = args.text("operator_command").unwrap_or_default();
    if command == "operation" {
        let operation_id = PyStr::from(args.text("operation_id").unwrap_or_default());
        return if args.text("operator_operation_command") == Some("status") {
            client.operation_status(&operation_id)
        } else {
            client.cancel(&operation_id)
        }
        .map_err(app_error);
    }
    if command == "stop" {
        return client.stop().map_err(app_error);
    }
    let data_root = store.selected_data_root(selection)?;
    match command {
        "status" => {
            let all = args.flag("all");
            let mut broker = client
                .status((!all).then_some(data_root.as_str()))
                .map_err(app_error)?;
            if !all {
                let mode = store.unlock_mode_status(&data_root)?;
                kassiber_envelope::set(&mut broker, "mode", mode);
            }
            if let Some(broker_build) = get(&broker, "broker_build").cloned() {
                // A lease serves every build, but runs the broker's own code.
                let this_build = client.host().build_identity();
                let same = py_eq(&broker_build, &this_build);
                kassiber_envelope::set(&mut broker, "this_build", this_build);
                kassiber_envelope::set(&mut broker, "same_build", Value::Bool(same));
            }
            Ok(broker)
        }
        "lock" => client.lock(&data_root, None, false).map_err(app_error),
        "unlock" => {
            let capability = args.text("capability").unwrap_or("accounting_decisions");
            if args.text("auth") == Some("touch-id") {
                require_interactive_auth(interaction, None)?;
                return Err(kassiber_runtime::store::not_implemented(
                    "operator",
                    "Touch ID unlock",
                ));
            }
            let passphrase = password_secret(
                fd_value(args.get("passphrase_fd")),
                interaction,
                "Database passphrase: ",
                "--passphrase-fd",
            )?;
            let duration = duration_seconds(args.text("duration"))?;
            client
                .unlock(
                    &data_root,
                    &passphrase,
                    duration,
                    capability,
                    "password",
                    false,
                    None,
                    None,
                )
                .map_err(app_error)
        }
        "mode" => {
            let authentication = password_secret(
                fd_value(args.get("passphrase_fd")),
                interaction,
                "Fresh database passphrase: ",
                "--passphrase-fd",
            )?;
            client
                .set_mode(
                    &data_root,
                    args.text("mode").unwrap_or_default(),
                    &authentication,
                )
                .map_err(app_error)
        }
        _ => Err(AppError::new("unknown_command", "unknown operator command")),
    }
}

/// `parse_duration(args.duration) if args.duration else None`.
fn duration_seconds(duration: Option<&str>) -> Result<Option<kassiber_pyjson::BigInt>, AppError> {
    match duration.filter(|text| !text.is_empty()) {
        None => Ok(None),
        Some(text) => kassiber_operator::arguments::parse_duration(text)
            .map(Some)
            .map_err(app_error),
    }
}

/// An argparse `type=int` descriptor flag.
fn fd_value(value: Option<&Value>) -> Option<kassiber_pyjson::BigInt> {
    match value {
        Some(Value::Int(fd)) => Some(fd.clone()),
        _ => None,
    }
}

/// `_password_secret(fd, non_interactive=..., label=..., fd_flag=...)`.
fn password_secret(
    fd: Option<kassiber_pyjson::BigInt>,
    interaction: Interaction,
    label: &str,
    fd_flag: &str,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let passphrase = match fd {
        Some(fd) => {
            let fd = i32::try_from(fd).map_err(|_| {
                AppError::new(
                    "invalid_passphrase",
                    "passphrase descriptor must be non-negative",
                )
            })?;
            kassiber_secrets::read_passphrase_from_fd(fd)
                .map_err(kassiber_secrets::service::storage_error)?
        }
        None => {
            require_interactive_auth(interaction, Some(fd_flag))?;
            kassiber_secrets::service::prompt_passphrase(label)?
        }
    };
    Ok(Zeroizing::new(passphrase.expose().as_bytes().to_vec()))
}

/// `_require_interactive_auth(non_interactive, fd_flag=...)`: agents reach
/// this path too, so the hint is the user's own terminal, never a
/// passphrase relayed through the agent.
fn require_interactive_auth(
    interaction: Interaction,
    fd_flag: Option<&str>,
) -> Result<(), AppError> {
    if !interaction.non_interactive && interaction.stdin_is_tty {
        return Ok(());
    }
    let alternative = fd_flag
        .map(|flag| format!(", or have a controlling process pass it through {flag}"))
        .unwrap_or_default();
    Err(
        AppError::new("interaction_required", "fresh local authentication is required")
            .with_hint(format!(
                "Ask the user to run this command in their own terminal{alternative}. Never ask for the passphrase itself."
            ))
            .with_details(dict([("reason", s("fresh_local_authentication"))])),
    )
}

/// `_pin_project_arguments(argv, data_root)`: the canonical root first,
/// without the caller's `--project`, `--data-root` or `--operator-auth-fd`.
pub(crate) fn pin_project_arguments(argv: &[String], data_root: &str) -> Vec<String> {
    let mut pinned = vec!["--data-root".to_owned(), data_root.to_owned()];
    let mut index = 0;
    while index < argv.len() {
        let token = &argv[index];
        if ["--project", "--data-root", "--operator-auth-fd"].contains(&token.as_str()) {
            index += 2;
            continue;
        }
        if token.starts_with("--project=")
            || token.starts_with("--data-root=")
            || token.starts_with("--operator-auth-fd=")
        {
            index += 1;
            continue;
        }
        pinned.push(token.clone());
        index += 1;
    }
    pinned
}

/// What [`route_brokered`] needs from the command line.
pub(crate) struct Routed<'a> {
    /// The raw command line, without the program name.
    pub argv: &'a [String],
    /// The selected leaf's path (`transactions.list`).
    pub path: &'a str,
    pub args: &'a Namespace,
    pub selection: &'a Selection,
    pub interaction: Interaction,
    /// `KASSIBER_OPERATOR_DIRECT=1`.
    pub direct: bool,
}

/// The broker client a routed command uses, and Ctrl-C while it waits.
pub(crate) trait Waiting {
    /// Whether Ctrl-C arrived.
    fn interrupted(&self) -> bool;
    /// Sleep up to `duration`, less when Ctrl-C arrives.
    fn sleep(&self, duration: std::time::Duration);
}

/// Ctrl-C while waiting: the process-wide interrupt flag, installed when
/// the wait starts. Without a handler the wait just sleeps.
pub(crate) fn ctrl_c() -> Box<dyn Waiting> {
    struct CtrlC(Option<kassiber_ipc::signal::Interrupt>);
    impl Waiting for CtrlC {
        fn interrupted(&self) -> bool {
            self.0.is_some_and(|interrupt| interrupt.is_set())
        }
        fn sleep(&self, duration: std::time::Duration) {
            match self.0 {
                Some(interrupt) => {
                    interrupt.sleep(duration);
                }
                None => std::thread::sleep(duration),
            }
        }
    }
    Box::new(CtrlC(kassiber_ipc::signal::install_interrupt().ok()))
}

/// `route_brokered_command(args, argv)`: `None` when the command runs
/// directly, else the exit code of the brokered run.
pub(crate) fn route_brokered<H: Host>(
    client: &dyn Fn() -> BrokerClient<H>,
    waiting: &dyn Fn() -> Box<dyn Waiting>,
    store: &dyn BookStore,
    routed: &Routed<'_>,
    out: &mut Output<'_>,
) -> Result<Option<i32>, AppError> {
    if routed.direct {
        return Ok(None);
    }
    let command = routed.path.split('.').next().unwrap_or_default();
    if command == "chat" {
        let data_root = store.selected_data_root(routed.selection)?;
        if store.effective_unlock_mode(&root_selection(&data_root))? == UnlockMode::Brokered {
            return Err(AppError::new(
                "operator_chat_not_supported",
                "CLI chat is not available while this project is in brokered mode",
            )
            .with_hint(
                "Use brokered CLI commands directly, or lock the broker and select manual mode before starting `kassiber chat`.",
            ));
        }
        return Ok(None);
    }
    if DIRECT_COMMANDS.contains(&command) {
        return Ok(None);
    }
    if command == "mcp" && routed.args.text("mcp_command") != Some("call") {
        // The server routes each finite `mcp call` itself; describing tools
        // needs no project.
        return Ok(None);
    }
    let data_root = store.selected_data_root(routed.selection)?;
    if store.effective_unlock_mode(&root_selection(&data_root))? != UnlockMode::Brokered {
        return Ok(None);
    }
    let capability = cli_capability(routed.path).ok_or_else(|| {
        AppError::new(
            "internal_error",
            format!("\"unclassified CLI command '{}'\"", routed.path),
        )
    })?;
    let pinned = pin_project_arguments(routed.argv, &data_root);
    let (pinned, _ignored_database_secrets) =
        kassiber_operator::runner::strip_database_passphrase_arguments(&pinned);
    let admin_authentication = if capability == Capability::Admin {
        Some(password_secret(
            fd_value(routed.args.get("operator_auth_fd")),
            routed.interaction,
            "Fresh database passphrase for admin operation: ",
            "--operator-auth-fd",
        )?)
    } else {
        None
    };
    let mut prepared = prepare(&pinned)?;
    let result = submit_and_wait(
        &client(),
        waiting,
        &data_root,
        &prepared,
        admin_authentication.as_deref().map(Vec::as_slice),
        out,
    );
    prepared.wipe();
    result.map(Some)
}

fn root_selection(data_root: &str) -> Selection {
    Selection {
        data_root: Some(data_root.to_owned()),
        ..Selection::default()
    }
}

/// `prepare_arguments(pinned_argv)`: each `--*-fd N` and the one
/// `--*-stdin` become opaque labels, their values read now.
fn prepare(argv: &[String]) -> Result<PreparedArguments, AppError> {
    let mut stdin = std::io::stdin();
    let open_fd = |fd: i32| -> Box<dyn std::io::Read> {
        Box::new(kassiber_ipc::descriptor::Descriptor::adopt(fd))
    };
    let token = || kassiber_operator::protocol::token_hex(16);
    kassiber_operator::arguments::prepare_arguments(
        argv,
        kassiber_operator::arguments::SecretSources {
            stdin: &mut stdin,
            open_fd: &open_fd,
            token: &token,
        },
    )
    .map_err(app_error)
}

fn submit_and_wait<H: Host>(
    client: &BrokerClient<H>,
    waiting: &dyn Fn() -> Box<dyn Waiting>,
    data_root: &str,
    prepared: &PreparedArguments,
    admin_authentication: Option<&[u8]>,
    out: &mut Output<'_>,
) -> Result<i32, AppError> {
    // Ordinary commands never start a broker: without a lease it could not
    // run them, and a stray broker would serve every other build. Brokered
    // work runs the broker's code, and its command line can carry secrets:
    // only this build's broker may take it.
    let accepted = client
        .submit(
            data_root,
            prepared,
            admin_authentication,
            &SubmitOptions {
                start_broker: false,
                require_same_build: true,
                ..SubmitOptions::default()
            },
        )
        .map_err(app_error)?;
    let Some(operation_id) = as_str(get(&accepted, "operation_id")).cloned() else {
        return Err(AppError::new(
            "operator_protocol_error",
            "broker did not return an operation id",
        ));
    };
    // Ctrl-C from here on cancels the accepted work instead of ending the
    // process with it still queued.
    let waiting = waiting();
    if out.machine {
        // One JSON line on stderr keeps stdout a single envelope while still
        // telling a caller that may time out which work to inspect.
        let event = kassiber_envelope::success(
            "operator.operation.accepted",
            dict([
                ("operation_id", Value::Str(operation_id.clone())),
                (
                    "state",
                    get(&accepted, "state").cloned().unwrap_or(Value::Null),
                ),
                (
                    "broker_build",
                    get(&accepted, "broker_build")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
            ]),
        );
        let line = kassiber_envelope::daemon_line(&event)
            .map_err(|error| AppError::new("internal_error", error.to_string()))?;
        let _ = out.stderr.write_all(line.as_bytes());
    } else {
        let _ = writeln!(
            out.stderr,
            "operator operation accepted: {}",
            text(&operation_id)
        );
    }
    let _ = out.stderr.flush();
    let completed = match wait(client, waiting.as_ref(), &operation_id)? {
        Some(completed) => completed,
        None => {
            let cancelled = client.cancel(&operation_id).map_err(app_error)?;
            let state = get(&cancelled, "state")
                .map(kassiber_envelope::py_str)
                .unwrap_or_else(|| "unknown".to_owned());
            let _ = writeln!(
                out.stderr,
                "operator cancellation requested: {} ({state})",
                text(&operation_id)
            );
            let _ = out.stderr.flush();
            return Ok(130);
        }
    };
    Ok(deliver(&completed, &operation_id, out))
}

/// `client.wait(operation_id)` that gives up when Ctrl-C arrives (`None`):
/// Python's `KeyboardInterrupt` cancels instead of reporting a state the
/// caller never saw.
fn wait<H: Host>(
    client: &BrokerClient<H>,
    waiting: &dyn Waiting,
    operation_id: &PyStr,
) -> Result<Option<Value>, AppError> {
    let mut delay = 0.05f64;
    loop {
        if waiting.interrupted() {
            return Ok(None);
        }
        let status = client.operation_status(operation_id).map_err(app_error)?;
        if waiting.interrupted() {
            return Ok(None);
        }
        if as_str(get(&status, "state")).is_some_and(|state| {
            kassiber_operator::client::TERMINAL_OPERATION_STATES
                .iter()
                .any(|terminal| state.is(terminal))
        }) {
            return Ok(Some(status));
        }
        waiting.sleep(std::time::Duration::from_secs_f64(delay));
        delay = (delay * 1.4).min(0.5);
    }
}

fn text(value: &PyStr) -> String {
    kassiber_envelope::py_str(&Value::Str(value.clone()))
}

/// `isinstance(value, int)`: bools count, as in Python.
fn python_int(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Int(number)) => Some(i64::try_from(number.clone()).unwrap_or(i64::MAX)),
        Some(Value::Bool(flag)) => Some(i64::from(*flag)),
        _ => None,
    }
}

/// The rest of `route_brokered_command`: pass the child's output through,
/// or report a result without it as a typed error, and pick the exit code.
pub(crate) fn deliver(completed: &Value, operation_id: &PyStr, out: &mut Output<'_>) -> i32 {
    let stdout = as_str(get(completed, "stdout"));
    let stderr = as_str(get(completed, "stderr"));
    let child_wrote_stdout = stdout.is_some_and(|text| text.code_points().next().is_some());
    if let Some(stdout) = stdout.filter(|_| child_wrote_stdout) {
        let _ = out.stdout.write_all(text(stdout).as_bytes());
        let _ = out.stdout.flush();
    }
    if let Some(stderr) = stderr.filter(|text| text.code_points().next().is_some()) {
        let _ = out.stderr.write_all(text(stderr).as_bytes());
        let _ = out.stderr.flush();
    }
    let state = get(completed, "state");
    let state_is = |name: &str| as_str(state).is_some_and(|state| state.is(name));
    let exit_code = python_int(get(completed, "exit_code"));
    if let Some(output_error @ Value::Dict(_)) = get(completed, "output_error") {
        let field = |key: &str, default: &str| {
            get(output_error, key)
                .filter(|value| kassiber_envelope::truthy(value))
                .map(kassiber_envelope::py_str)
                .unwrap_or_else(|| default.to_owned())
        };
        let error = outcome_error(
            &field("code", "operator_output_unavailable"),
            &field("message", "The operator result output is unavailable."),
            operation_id,
            completed,
            "Narrow the query or write a file export; reconcile a mutation before retrying it."
                .to_owned(),
            false,
        );
        emit(out, &error);
        return 1;
    }
    if state_is("result_unknown") {
        let _ = writeln!(
            out.stderr,
            "operator result is unknown; inspect operation {} before retrying",
            text(operation_id)
        );
        let _ = out.stderr.flush();
        if !child_wrote_stdout {
            let error = outcome_error(
                "operator_result_unknown",
                "the broker cannot prove whether the operation ran",
                operation_id,
                completed,
                format!(
                    "Run `kassiber operator operation status {}` and reconcile the book before retrying.",
                    text(operation_id)
                ),
                false,
            );
            emit(out, &error);
        }
        return 1;
    }
    for terminal in ["cancelled", "failed"] {
        if state_is(terminal) && !child_wrote_stdout {
            // Queued work cancelled by lock/expiry never started; say so on
            // stdout instead of leaving a machine caller with empty output.
            let error = outcome_error(
                &format!("operator_operation_{terminal}"),
                &format!("the operator operation was {terminal}"),
                operation_id,
                completed,
                "Check `kassiber operator status`; if the lease ended, ask the user to run `kassiber operator unlock` and retry."
                    .to_owned(),
                terminal == "cancelled",
            );
            emit(out, &error);
            // A signal-killed child reports a negative code; exit plainly.
            return exit_code.filter(|code| *code > 0).map_or(1, exit_status);
        }
    }
    match exit_code {
        Some(code) => exit_status(code),
        None => i32::from(!state_is("completed")),
    }
}

/// `sys.exit(code)`: the code as the process exit status.
fn exit_status(code: i64) -> i32 {
    i32::try_from(code).unwrap_or(1)
}

fn emit(out: &mut Output<'_>, error: &AppError) {
    let mut sink = Sink {
        output: out.output,
        home: out.home,
        stdout: &mut *out.stdout,
        stderr: &mut *out.stderr,
    };
    emit_error(&mut sink, out.format, error);
}

/// `_operation_outcome_error(...)`.
fn outcome_error(
    code: &str,
    message: &str,
    operation_id: &PyStr,
    completed: &Value,
    hint: String,
    retryable: bool,
) -> AppError {
    let mut details = vec![
        ("operation_id", Value::Str(operation_id.clone())),
        (
            "state",
            get(completed, "state").cloned().unwrap_or(Value::Null),
        ),
    ];
    if let Some(exit_code @ (Value::Int(_) | Value::Bool(_))) = get(completed, "exit_code") {
        details.push(("exit_code", exit_code.clone()));
    }
    if let Some(reason) = get(completed, "reason")
        .filter(|reason| matches!(reason, Value::Str(text) if text.code_points().next().is_some()))
    {
        details.push(("reason", reason.clone()));
    }
    AppError::new(code, message)
        .with_hint(hint)
        .with_details(dict(details))
        .retryable(retryable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    struct Captured {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        code: i32,
    }

    fn deliver_machine(completed: &str) -> Captured {
        let completed = kassiber_pytext::loads(completed).unwrap();
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let code = deliver(
            &completed,
            &PyStr::from("generation.operation"),
            &mut Output {
                stdout: &mut stdout,
                stderr: &mut stderr,
                output: None,
                home: None,
                format: Format::Json,
                machine: true,
            },
        );
        Captured {
            stdout,
            stderr,
            code,
        }
    }

    fn envelope(bytes: &[u8]) -> Value {
        kassiber_pytext::loads(std::str::from_utf8(bytes).unwrap()).unwrap()
    }

    fn error_field<'a>(envelope: &'a Value, key: &str) -> Option<&'a Value> {
        get(get(envelope, "error").unwrap(), key)
    }

    #[test]
    fn pinning_drops_locators_and_the_admin_fd() {
        assert_eq!(
            pin_project_arguments(
                &argv(&[
                    "--project",
                    "p",
                    "--machine",
                    "--data-root=/x",
                    "--operator-auth-fd",
                    "7",
                    "--operator-auth-fd=8",
                    "--project=q",
                    "status",
                ]),
                "/canonical"
            ),
            argv(&["--data-root", "/canonical", "--machine", "status"])
        );
        // The database fd never reaches the broker; the lease is the key.
        let (stripped, removed) = kassiber_operator::runner::strip_database_passphrase_arguments(
            &pin_project_arguments(&argv(&["--db-passphrase-fd", "9", "status"]), "/root"),
        );
        assert_eq!(stripped, argv(&["--data-root", "/root", "status"]));
        assert!(removed.contains("9"));
    }

    #[test]
    fn global_and_direct_commands_are_never_routed() {
        for command in [
            "update",
            "verify-download",
            "commands",
            "daemon",
            "offline",
            "projects",
        ] {
            assert!(DIRECT_COMMANDS.contains(&command), "{command}");
        }
        assert!(!DIRECT_COMMANDS.contains(&"status"));
        assert!(!DIRECT_COMMANDS.contains(&"mcp"));
    }

    #[test]
    fn machine_cancelled_operation_writes_typed_envelope() {
        let run = deliver_machine(
            r#"{"operation_id": "generation.operation", "state": "cancelled", "exit_code": 1, "stdout": "", "stderr": "operator lease expired\n"}"#,
        );
        assert_eq!(run.code, 1);
        let payload = envelope(&run.stdout);
        assert_eq!(
            error_field(&payload, "code"),
            Some(&s("operator_operation_cancelled"))
        );
        let details = error_field(&payload, "details").unwrap();
        assert_eq!(
            get(details, "operation_id"),
            Some(&s("generation.operation"))
        );
        assert_eq!(get(details, "state"), Some(&s("cancelled")));
        assert_eq!(get(details, "exit_code"), Some(&Value::from(1)));
        assert_eq!(error_field(&payload, "retryable"), Some(&Value::Bool(true)));
        assert_eq!(run.stderr, b"operator lease expired\n");
    }

    #[test]
    fn machine_failed_operation_exits_with_a_positive_child_code() {
        let run = deliver_machine(r#"{"state": "failed", "exit_code": 3}"#);
        assert_eq!(run.code, 3);
        let payload = envelope(&run.stdout);
        assert_eq!(
            error_field(&payload, "code"),
            Some(&s("operator_operation_failed"))
        );
        assert_eq!(
            error_field(&payload, "retryable"),
            Some(&Value::Bool(false))
        );
        // A signal-killed child exits plainly.
        assert_eq!(
            deliver_machine(r#"{"state": "failed", "exit_code": -9}"#).code,
            1
        );
        assert_eq!(deliver_machine(r#"{"state": "failed"}"#).code, 1);
    }

    #[test]
    fn machine_unknown_result_without_child_output_writes_envelope() {
        let run = deliver_machine(
            r#"{"operation_id": "generation.operation", "state": "result_unknown", "reason": "broker_generation_changed"}"#,
        );
        assert_eq!(run.code, 1);
        let payload = envelope(&run.stdout);
        assert_eq!(
            error_field(&payload, "code"),
            Some(&s("operator_result_unknown"))
        );
        let details = error_field(&payload, "details").unwrap();
        assert_eq!(
            get(details, "reason"),
            Some(&s("broker_generation_changed"))
        );
        assert!(get(details, "exit_code").is_none());
        assert!(
            kassiber_envelope::py_str(error_field(&payload, "hint").unwrap())
                .contains("operator operation status generation.operation")
        );
        assert_eq!(
            run.stderr,
            b"operator result is unknown; inspect operation generation.operation before retrying\n"
        );
    }

    #[test]
    fn machine_unknown_result_keeps_the_childs_single_envelope() {
        let run = deliver_machine(
            r#"{"state": "result_unknown", "exit_code": 1, "stdout": "{\"kind\": \"error\"}\n", "stderr": ""}"#,
        );
        assert_eq!(run.code, 1);
        assert_eq!(run.stdout, b"{\"kind\": \"error\"}\n");
    }

    #[test]
    fn machine_oversized_result_writes_typed_envelope() {
        let run = deliver_machine(
            r#"{"state": "completed", "exit_code": 0, "output_available": false, "output_error": {"code": "operator_result_too_large", "message": "The operator result is too large to return."}}"#,
        );
        assert_eq!(run.code, 1);
        let payload = envelope(&run.stdout);
        assert_eq!(
            error_field(&payload, "code"),
            Some(&s("operator_result_too_large"))
        );
        assert_eq!(
            error_field(&payload, "message"),
            Some(&s("The operator result is too large to return."))
        );
        let details = error_field(&payload, "details").unwrap();
        assert_eq!(get(details, "state"), Some(&s("completed")));
        assert_eq!(get(details, "exit_code"), Some(&Value::from(0)));
        // An empty code or message falls back.
        let run = deliver_machine(r#"{"state": "completed", "output_error": {"code": ""}}"#);
        let payload = envelope(&run.stdout);
        assert_eq!(
            error_field(&payload, "code"),
            Some(&s("operator_output_unavailable"))
        );
        assert_eq!(
            error_field(&payload, "message"),
            Some(&s("The operator result output is unavailable."))
        );
    }

    #[test]
    fn completed_results_pass_the_child_through_with_its_exit_code() {
        let run = deliver_machine(
            r#"{"state": "completed", "exit_code": 0, "stdout": "{}\n", "stderr": "note\n"}"#,
        );
        assert_eq!(
            (run.code, run.stdout, run.stderr),
            (0, b"{}\n".to_vec(), b"note\n".to_vec())
        );
        assert_eq!(deliver_machine(r#"{"state": "completed"}"#).code, 0);
        assert_eq!(deliver_machine(r#"{"state": "running"}"#).code, 1);
        // `isinstance(True, int)`: a bool exit code counts.
        assert_eq!(
            deliver_machine(r#"{"state": "completed", "exit_code": true}"#).code,
            1
        );
        // A child that wrote output and then failed keeps its own envelope.
        let run = deliver_machine(r#"{"state": "failed", "exit_code": 2, "stdout": "x\n"}"#);
        assert_eq!((run.code, run.stdout), (2, b"x\n".to_vec()));
    }

    #[test]
    fn interactive_auth_is_refused_to_machines_and_pipes() {
        for interaction in [
            Interaction {
                non_interactive: true,
                stdin_is_tty: true,
            },
            Interaction {
                non_interactive: false,
                stdin_is_tty: false,
            },
        ] {
            let error =
                require_interactive_auth(interaction, Some("--operator-auth-fd")).unwrap_err();
            assert_eq!(error.code, "interaction_required");
            assert_eq!(error.message, "fresh local authentication is required");
            assert_eq!(
                error.hint.as_deref(),
                Some("Ask the user to run this command in their own terminal, or have a controlling process pass it through --operator-auth-fd. Never ask for the passphrase itself.")
            );
            assert_eq!(
                error.details,
                Some(dict([("reason", s("fresh_local_authentication"))]))
            );
            assert_eq!(
                require_interactive_auth(interaction, None)
                    .unwrap_err()
                    .hint
                    .as_deref(),
                Some("Ask the user to run this command in their own terminal. Never ask for the passphrase itself.")
            );
        }
        require_interactive_auth(
            Interaction {
                non_interactive: false,
                stdin_is_tty: true,
            },
            None,
        )
        .unwrap();
    }

    #[test]
    fn os_errors_read_as_python_s() {
        use kassiber_operator::error::os_error_text;
        let refused = std::io::Error::from_raw_os_error(111);
        if cfg!(target_os = "linux") {
            assert_eq!(os_error_text(&refused), "[Errno 111] Connection refused");
            let error = app_error(kassiber_operator::Error::Io(refused));
            assert_eq!(
                (error.code.as_str(), error.message.as_str()),
                ("internal_error", "[Errno 111] Connection refused")
            );
        }
        assert_eq!(
            os_error_text(&std::io::Error::from(std::io::ErrorKind::TimedOut)),
            "timed out"
        );
    }
}

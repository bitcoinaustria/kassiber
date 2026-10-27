//! `kassiber.cli.main.main(argv)`: the steps before and around a command.
//!
//! In Python's order: the two hidden argv forms; argparse, with usage
//! errors as an envelope when `--machine` appears anywhere; `--machine`
//! implying `--non-interactive`; the output format; the operator-child
//! bindings; the default state-root migration flag; broker routing; and
//! the command. A leaf the port lacks fails `not_implemented` right after
//! the format is known, before anything is read, written, or bootstrapped;
//! nothing is ever handed to Python.

use std::io::{Read, Write};
use std::sync::Arc;

use kassiber_envelope::{build_error_envelope, cli_text, derive_kind, dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::status::status_payload;
use kassiber_runtime::store::{not_implemented, BookStore, Selection, Unlock, UnlockMode};
use kassiber_runtime::BuildInfo;

use crate::argparse::{parse_args, Context, Namespace, Stop};
use crate::describe::IMPLEMENTED;
use crate::help::{format_usage, width_from_columns};
use crate::render::{emit, emit_error, Format, Sink};
use crate::spec::{parsers, ParserId};

/// The environment variables the shell reads.
#[derive(Clone, Debug, Default)]
pub struct Environment {
    /// `COLUMNS`, for argparse's wrapping width.
    pub columns: Option<String>,
    /// Current-user home for `--output ~/...` (Windows ignores `HOME`).
    pub home: Option<String>,
    /// `KASSIBER_OPERATOR_CHILD`.
    pub operator_child: Option<String>,
    /// `KASSIBER_OPERATOR_WORKING_DIRECTORY`.
    pub operator_working_directory: Option<String>,
    /// `KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY`.
    pub operator_expected_project_identity: Option<String>,
    /// `KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY`.
    pub operator_expected_database_identity: Option<String>,
    /// `KASSIBER_OPERATOR_DIRECT`.
    pub operator_direct: Option<String>,
}

impl Environment {
    /// Reads the variables from this process.
    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        Environment {
            columns: var("COLUMNS"),
            #[cfg(not(windows))]
            home: var("HOME"),
            #[cfg(windows)]
            home: var("USERPROFILE").or_else(|| {
                Some(format!(
                    "{}{}",
                    var("HOMEDRIVE").unwrap_or_default(),
                    var("HOMEPATH")?
                ))
            }),
            operator_child: var("KASSIBER_OPERATOR_CHILD"),
            operator_working_directory: var("KASSIBER_OPERATOR_WORKING_DIRECTORY"),
            operator_expected_project_identity: var("KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY"),
            operator_expected_database_identity: var(
                "KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY",
            ),
            operator_direct: var("KASSIBER_OPERATOR_DIRECT"),
        }
    }

    fn operator_child(&self) -> bool {
        self.operator_child.as_deref() == Some("1")
    }
}

/// The process's standard streams.
pub struct Streams {
    pub stdin: Box<dyn Read + Send>,
    pub stdin_is_tty: bool,
    pub stdout: Box<dyn Write + Send>,
    pub stderr: Box<dyn Write + Send>,
}

/// `USAGE_HINT` of the `--machine` usage-error envelope.
const USAGE_HINT: &str = "Global flags such as --machine, --format, --output, --project, and --data-root go before the subcommand. Check `kassiber --machine commands describe <command path>` for the exact arguments.";

/// The global flags after parsing.
struct Globals {
    selection: Selection,
    format: Option<String>,
    machine: bool,
    non_interactive: bool,
    output: Option<String>,
    diagnostics_out: Option<String>,
    db_passphrase_fd: Option<i64>,
}

fn globals(namespace: &Namespace) -> Globals {
    let fd = |name: &str| match namespace.get(name) {
        Some(Value::Int(n)) => Some(i64::try_from(n.clone()).unwrap_or(i64::MAX)),
        _ => None,
    };
    let text = |name: &str| match namespace.get(name) {
        Some(Value::Str(text)) => text.as_str().map(str::to_owned),
        _ => None,
    };
    let machine = namespace.flag("machine");
    Globals {
        selection: Selection {
            project: text("project"),
            data_root: text("data_root"),
            env_file: text("env_file"),
        },
        format: text("format"),
        machine,
        non_interactive: namespace.flag("non_interactive") || machine,
        output: text("output"),
        diagnostics_out: text("diagnostics_out"),
        db_passphrase_fd: fd("db_passphrase_fd"),
    }
}

/// The leaf the namespace selected: its path and parser.
fn selected_leaf(namespace: &Namespace) -> (Vec<&'static str>, ParserId) {
    let mut path = Vec::new();
    let mut pid = 0;
    while let Some(action) = parsers()[pid]
        .actions
        .iter()
        .find(|a| a.subparsers.is_some())
    {
        let (Some(dest), Some(sub)) = (action.dest, action.subparsers) else {
            break;
        };
        let Some(name) = namespace.text(dest) else {
            break;
        };
        let Some(&(name, child)) = sub.names.iter().find(|(n, _)| *n == name) else {
            break;
        };
        path.push(name);
        pid = usize::from(child);
    }
    (path, pid)
}

/// Runs one command line (without the program name); returns the exit code.
pub fn run(
    argv: &[String],
    env: &Environment,
    store: Arc<dyn BookStore>,
    build: BuildInfo,
    streams: Streams,
) -> i32 {
    let Streams {
        stdin,
        stdin_is_tty,
        mut stdout,
        mut stderr,
    } = streams;
    // The hidden forms the desktop and the broker spawn; Python reports an
    // AppError from them as `<code>: <message>` on stderr.
    let hidden = match argv {
        [only] if only == "--operator-broker-server" => {
            Some(not_implemented("operator", "the operator broker server"))
        }
        [only] if only == "--migrate-default-state-root" => {
            match store.migrate_default_state_root() {
                Ok(()) => return 0,
                Err(error) => Some(error),
            }
        }
        _ => None,
    };
    if let Some(error) = hidden {
        let _ = writeln!(stderr, "{}: {}", error.code, error.message);
        return 1;
    }

    let width = width_from_columns(env.columns.as_deref());
    let version = build.version_text();
    let namespace = match parse_args(
        argv,
        Context {
            width,
            version: &version,
        },
    ) {
        Ok(namespace) => namespace,
        Err(Stop::Exit { stdout: text }) => {
            let _ = stdout.write_all(text.as_bytes());
            let _ = stdout.flush();
            return 0;
        }
        Err(Stop::Error { parser, message }) => {
            let usage = format_usage(parser, width);
            if argv.iter().any(|a| a == "--machine") {
                let envelope = build_error_envelope(
                    "usage_error",
                    &message,
                    Some(dict([("usage", s(kassiber_pytext::strip(&usage)))])),
                    Some(USAGE_HINT),
                    false,
                    None,
                );
                if let Ok(text) = cli_text(&envelope) {
                    let _ = stdout.write_all(text.as_bytes());
                    let _ = stdout.flush();
                }
            } else {
                let _ = writeln!(
                    stderr,
                    "{usage}{}: error: {message}",
                    parsers()[parser].prog
                );
                let _ = stderr.flush();
            }
            return 2;
        }
    };
    let globals = globals(&namespace);

    // resolve_output_format: a bad combination is reported in table form.
    let format = if globals.machine {
        match globals.format.as_deref() {
            None | Some("json") => Format::Json,
            Some(other) => {
                let error = AppError::new(
                    "invalid_flag_combination",
                    format!("--machine requires --format json, got --format {other}"),
                );
                let mut sink = Sink {
                    output: globals.output.as_deref(),
                    home: env.home.as_deref(),
                    stdout: &mut stdout,
                    stderr: &mut stderr,
                };
                emit_error(&mut sink, Format::Table, &error);
                return 1;
            }
        }
    } else {
        globals
            .format
            .as_deref()
            .and_then(Format::parse)
            .unwrap_or(Format::Table)
    };

    let (path, _) = selected_leaf(&namespace);
    let dotted = path.join(".");
    let kind = derive_kind(|name| namespace.text(name), None);
    let output = globals.output.clone();
    let fail = |stdout: &mut Box<dyn Write + Send>,
                stderr: &mut Box<dyn Write + Send>,
                error: AppError| {
        let mut sink = Sink {
            output: output.as_deref(),
            home: env.home.as_deref(),
            stdout,
            stderr,
        };
        emit_error(&mut sink, format, &error);
        1
    };

    let prepared = (|| -> Result<(), AppError> {
        if !IMPLEMENTED.contains(&dotted.as_str()) {
            let mut error = not_implemented("cli", &format!("the command `{}`", path.join(" ")));
            error.details = Some(dict([
                ("component", s("cli")),
                ("command", s(dotted.as_str())),
            ]));
            return Err(error);
        }
        check_operator_bindings(env)?;
        if globals.diagnostics_out.is_some() {
            return Err(not_implemented("diagnostics", "--diagnostics-out"));
        }
        Ok(())
    })();
    if let Err(error) = prepared {
        return fail(&mut stdout, &mut stderr, error);
    }
    // _maybe_migrate_default_state_root, before broker routing and
    // bootstrap. `commands describe` never migrates (nor bootstraps).
    let migrate = globals.selection.data_root.is_none()
        && globals.selection.env_file.is_none()
        && !matches!(path.first(), Some(&"commands") | Some(&"verify-download"));
    if migrate {
        if let Err(error) = store.migrate_default_state_root() {
            return fail(&mut stdout, &mut stderr, error);
        }
    }

    if dotted == "daemon" {
        // The daemon owns stdin and stdout from here on.
        return match store.bootstrap(&globals.selection) {
            Ok(bootstrap) => kassiber_daemon::serve(
                Arc::clone(&store),
                kassiber_daemon::DaemonConfig { build, bootstrap },
                stdin,
                stdout,
            ),
            Err(error) => fail(&mut stdout, &mut stderr, error),
        };
    }
    drop(stdin);

    let mut sink = Sink {
        output: output.as_deref(),
        home: env.home.as_deref(),
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let result = match dotted.as_str() {
        "commands.describe" => describe_command(&namespace, &globals.selection)
            .and_then(|catalog| emit(&mut sink, format, &kind, catalog)),
        _ => status_command(
            env,
            store.as_ref(),
            build,
            &globals,
            stdin_is_tty,
            dotted == "init",
            |payload| emit(&mut sink, format, &kind, payload),
        ),
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            emit_error(&mut sink, format, &error);
            1
        }
    }
}

/// `commands describe [path...]`. Pure: no project is resolved or created
/// (Python bootstraps the default project here; see
/// docs/reference/rust-shell.md), but the flag conflict resolution would
/// raise still fails.
fn describe_command(namespace: &Namespace, selection: &Selection) -> Result<Value, AppError> {
    check_project_selection(selection)?;
    let query: Vec<String> = match namespace.get("path") {
        Some(Value::List(items)) => items.iter().map(kassiber_envelope::py_str).collect(),
        _ => Vec::new(),
    };
    crate::describe::describe(&query)
}

/// `status`: broker routing, `bootstrap_runtime(needs_db=True)`, and
/// `cmd_status`.
fn status_command(
    env: &Environment,
    store: &dyn BookStore,
    build: BuildInfo,
    globals: &Globals,
    stdin_is_tty: bool,
    initialize: bool,
    emit: impl FnOnce(Value) -> Result<(), AppError>,
) -> Result<(), AppError> {
    route_brokered(env, store, &globals.selection)?;
    if env.operator_child() {
        return Err(not_implemented("operator", "running as a broker child"));
    }
    let bootstrap = store.bootstrap(&globals.selection)?;
    let mut runtime_config = bootstrap.runtime_config.clone();
    let expected = operator_expected_database_identity(env)?;
    let allow_prompt =
        globals.db_passphrase_fd.is_none() && stdin_is_tty && !globals.non_interactive;
    let mut book = store.open_book(
        &bootstrap.paths.data_root,
        Unlock::Cli {
            passphrase_fd: globals.db_passphrase_fd,
            allow_prompt,
            expected_database_identity: expected.as_deref(),
            initialize_version: initialize.then_some(build.version),
        },
        &mut runtime_config,
    )?;
    let outcome = (|| {
        if let Some(project_id) = &bootstrap.paths.project_id {
            store.mark_project_opened(
                project_id,
                &bootstrap.paths.data_root,
                !bootstrap.selection_explicit,
            )?;
        }
        if initialize {
            let p = &bootstrap.paths;
            let text = |v: &Option<String>| v.as_deref().map_or(Value::Null, s);
            return emit(dict([
                ("version", s(build.version)),
                ("project_id", text(&p.project_id)),
                ("project_root", text(&p.project_root)),
                ("state_root", s(p.state_root.as_str())),
                ("data_root", s(p.data_root.as_str())),
                ("database", s(p.database.as_str())),
                ("config_root", s(p.config_root.as_str())),
                ("settings_file", s(p.settings_file.as_str())),
                ("exports_root", s(p.exports_root.as_str())),
                ("attachments_root", s(p.attachments_root.as_str())),
                ("env_file", s(p.env_file.as_str())),
            ]));
        }
        if !book.is_initialized()? {
            return Err(
                AppError::new("not_initialized", "Kassiber is not initialized yet")
                    .with_hint("Run `kassiber init`, then run `kassiber status` again.")
                    .with_details(dict([("data_root", s(bootstrap.paths.data_root.as_str()))])),
            );
        }
        let facts = book.status_facts()?;
        emit(status_payload(build.version, &facts, &runtime_config))
    })();
    let closed = book.close();
    outcome?;
    closed
}

/// `resolve_runtime_paths`'s first check, which needs no store.
fn check_project_selection(selection: &Selection) -> Result<(), AppError> {
    if selection.data_root.is_some() && selection.project.is_some() {
        return Err(AppError::new(
            "invalid_flag_combination",
            "--project and --data-root cannot be used together",
        )
        .with_hint(
            "Use --project for a catalog project or --data-root for an explicit project data directory, not both.",
        ));
    }
    Ok(())
}

/// `_enter_operator_caller_directory` and `_verify_operator_child_project`:
/// a broker child must carry its bindings, and only a broker child may.
/// Broker children themselves are not ported yet.
fn check_operator_bindings(env: &Environment) -> Result<(), AppError> {
    let invalid = |message: &str| AppError::new("operator_project_binding_invalid", message);
    if env.operator_working_directory.is_some() {
        if !env.operator_child() {
            return Err(invalid(
                "operator working directory is only valid in a broker child",
            ));
        }
        return Err(not_implemented("operator", "running as a broker child"));
    }
    match &env.operator_expected_project_identity {
        None if env.operator_child() => Err(invalid("operator child project binding is missing")),
        None => Ok(()),
        Some(_) if !env.operator_child() => Err(invalid(
            "operator project binding is only valid in a broker child",
        )),
        Some(_) => Err(not_implemented("operator", "running as a broker child")),
    }
}

/// `_operator_expected_database_identity()`.
fn operator_expected_database_identity(env: &Environment) -> Result<Option<String>, AppError> {
    if env.operator_child() && env.operator_expected_database_identity.is_none() {
        return Err(AppError::new(
            "operator_project_binding_invalid",
            "operator child database binding is missing",
        ));
    }
    Ok(env.operator_expected_database_identity.clone())
}

/// `route_brokered_command`: a brokered project's commands go through the
/// operator broker, which is not ported yet.
fn route_brokered(
    env: &Environment,
    store: &dyn BookStore,
    selection: &Selection,
) -> Result<(), AppError> {
    if env.operator_direct.as_deref() == Some("1") {
        return Ok(());
    }
    if store.effective_unlock_mode(selection)? == UnlockMode::Brokered {
        return Err(not_implemented(
            "operator",
            "running commands through the operator broker",
        ));
    }
    Ok(())
}

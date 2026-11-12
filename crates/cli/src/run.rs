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
use kassiber_runtime::store::{not_implemented, BookStore, Secret, Selection, Unlock};
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
    /// Where the operator broker listens; `None` reads this process's
    /// account and environment (`Runtime::from_process`). Tests name a
    /// disposable directory here so no client ever probes the account's
    /// endpoint.
    pub operator_runtime: Option<kassiber_operator::runtime::Runtime>,
    /// Whether the streams are this process's own descriptors, which
    /// `mcp serve` moves onto private copies before serving. Callers that
    /// pass other streams (tests) leave it off.
    pub claim_stdio: bool,
    /// `KASSIBER_RUST_TEST_PROBE=1`: parse the command line, print the
    /// selected command path (`wallets.list`), then exit 0 when this binary
    /// implements the command or 3 when it does not, without touching any
    /// state. Test builds only; the hybrid test candidate
    /// (`tests/rust_port/hybrid_cli.py`) routes each call on it.
    #[cfg(feature = "test-probe")]
    pub test_probe: bool,
    #[cfg(feature = "test-probe")]
    pub test_sync_handoff: bool,
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
            operator_runtime: None,
            claim_stdio: true,
            #[cfg(feature = "test-probe")]
            test_probe: var("KASSIBER_RUST_TEST_PROBE").as_deref() == Some("1"),
            #[cfg(feature = "test-probe")]
            test_sync_handoff: var("KASSIBER_RUST_TEST_SYNC_HANDOFF").as_deref() == Some("1"),
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
    pub stdout_is_tty: bool,
    pub stderr: Box<dyn Write + Send>,
    pub stderr_is_tty: bool,
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

/// Exit code of a `KASSIBER_RUST_TEST_PROBE` run for a command this binary
/// does not implement. Distinct from every exit code a real run uses.
#[cfg(feature = "test-probe")]
pub const PROBE_NOT_IMPLEMENTED: i32 = 3;

// Platform transaction formats remain unported; refuse before opening a book.
pub(crate) fn implemented(command: &str, args: &Namespace) -> bool {
    IMPLEMENTED.contains(&command)
        && (command != "transactions.export"
            || matches!(
                args.text("export_format"),
                Some("csv" | "xlsx" | "cointracking-csv" | "blockpit-csv")
            ))
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
        mut stdin,
        stdin_is_tty,
        mut stdout,
        stdout_is_tty,
        mut stderr,
        stderr_is_tty,
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
    let mut namespace = match parse_args(
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

    namespace.set("non_interactive", Value::Bool(globals.non_interactive));
    namespace.set(
        "format",
        s(if globals.machine {
            "json"
        } else {
            globals.format.as_deref().unwrap_or("table")
        }),
    );
    let (path, _) = selected_leaf(&namespace);
    let dotted = path.join(".");
    #[cfg(feature = "test-probe")]
    if env.test_probe {
        let _ = writeln!(stdout, "{dotted}");
        let _ = stdout.flush();
        return if implemented(&dotted, &namespace) {
            0
        } else {
            PROBE_NOT_IMPLEMENTED
        };
    }
    // `maintenance purge-legacy-ledger` without --confirm only plans.
    let plan_only = dotted == "maintenance.purge-legacy-ledger" && !namespace.flag("confirm");
    let kind = derive_kind(
        |name| namespace.text(name),
        crate::reports::kind(&dotted)
            .or(plan_only.then_some("maintenance.purge-legacy-ledger.plan")),
    );
    // A broker child enters its caller's directory and proves its project
    // before anything else; a vanished directory reports on stdout, never
    // to a relative --output in the broker's directory.
    let bindings = check_operator_bindings(env, store.as_ref(), &globals.selection);
    let output = match &bindings {
        Err(error) if error.code == "operator_working_directory_unavailable" => None,
        _ => globals.output.clone(),
    };
    let fail = |stdout: &mut Box<dyn Write + Send>,
                stderr: &mut Box<dyn Write + Send>,
                error: AppError| {
        crate::diagnostics::write_error(
            &crate::diagnostics::arguments(&namespace, None),
            build.version,
            &error,
            None,
            stderr.as_mut(),
        );
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
        bindings?;
        if !implemented(&dotted, &namespace) {
            let mut error = not_implemented("cli", &format!("the command `{}`", path.join(" ")));
            error.details = Some(dict([
                ("component", s("cli")),
                ("command", s(dotted.as_str())),
            ]));
            return Err(error);
        }
        Ok(())
    })();
    if let Err(error) = prepared {
        return fail(&mut stdout, &mut stderr, error);
    }
    let interaction = crate::operator_cli::Interaction {
        non_interactive: globals.non_interactive,
        stdin_is_tty,
    };
    if crate::mcp::EARLY_COMMANDS.contains(&dotted.as_str()) {
        // MCP clients may launch probe copies: these never migrate state,
        // resolve a project or route through a broker.
        if dotted == "mcp.serve" {
            if let Err(error) = crate::mcp::require_complete_pin(&namespace) {
                return fail(&mut stdout, &mut stderr, error);
            }
            let client_env = env.clone();
            let tools = crate::mcp::BookTools::new(
                Arc::clone(&store),
                build,
                env.clone(),
                Box::new(move || operator_client(build, &client_env)),
                globals.selection.clone(),
                namespace.text("workspace").map(str::to_owned),
                namespace.text("profile").map(str::to_owned),
            );
            return crate::mcp::serve(tools, build.version, stdin, stdout, env.claim_stdio);
        }
        let mut input = std::io::BufReader::new(&mut stdin);
        let result = crate::mcp::configure(
            store.as_ref(),
            &dotted,
            interaction.non_interactive,
            stdin_is_tty,
            &mut input,
            &mut stderr,
            namespace.text("grant"),
            namespace.text("agent_access"),
        )
        .and_then(|payload| {
            let mut sink = Sink {
                output: output.as_deref(),
                home: env.home.as_deref(),
                stdout: &mut stdout,
                stderr: &mut stderr,
            };
            emit(&mut sink, format, &kind, payload)
        });
        return match result {
            Ok(()) => 0,
            Err(error) => fail(&mut stdout, &mut stderr, error),
        };
    }
    if dotted == "mcp.call" {
        // Before path resolution or broker routing, both of which would
        // create a default project on a machine with no Kassiber state.
        if let Err(error) = crate::mcp::require_existing_book(store.as_ref(), &globals.selection) {
            return fail(&mut stdout, &mut stderr, error);
        }
    }
    #[cfg(feature = "test-probe")]
    let journal_invocation_guard = (dotted == "journals.process"
        && std::env::var("KASSIBER_RUST_TEST_JOURNAL_FALLBACK").as_deref() == Ok("1"))
    .then(|| store.journal_replay_guard(&globals.selection))
    .flatten();
    // _maybe_migrate_default_state_root, before broker routing and
    // bootstrap. `commands describe` never migrates (nor bootstraps).
    let migrate = globals.selection.data_root.is_none()
        && globals.selection.env_file.is_none()
        && !matches!(path.first(), Some(&"commands") | Some(&"verify-download"))
        && !(path.first() == Some(&"operator")
            && matches!(
                path.get(1),
                Some(&"status") | Some(&"lock") | Some(&"operation")
            ));
    if migrate {
        if let Err(error) = store.migrate_default_state_root() {
            return fail(&mut stdout, &mut stderr, error);
        }
    }
    if path.first() == Some(&"operator") {
        let result = crate::operator_cli::dispatch(
            &operator_client(build, env),
            store.as_ref(),
            &namespace,
            &globals.selection,
            interaction,
        )
        .and_then(|payload| {
            let mut sink = Sink {
                output: output.as_deref(),
                home: env.home.as_deref(),
                stdout: &mut stdout,
                stderr: &mut stderr,
            };
            emit(&mut sink, format, &kind, payload)
        });
        return match result {
            Ok(()) => 0,
            Err(error) => fail(&mut stdout, &mut stderr, error),
        };
    }
    let routed = crate::operator_cli::route_brokered(
        &|| operator_client(build, env),
        &crate::operator_cli::ctrl_c,
        store.as_ref(),
        &crate::operator_cli::Routed {
            argv,
            path: &dotted,
            args: &namespace,
            selection: &globals.selection,
            interaction,
            direct: env.operator_direct.as_deref() == Some("1"),
        },
        &mut crate::operator_cli::Output {
            stdout: &mut stdout,
            stderr: &mut stderr,
            output: output.as_deref(),
            home: env.home.as_deref(),
            format,
            machine: globals.machine,
        },
    );
    match routed {
        Ok(Some(code)) => return code,
        Ok(None) => {}
        Err(error) => return fail(&mut stdout, &mut stderr, error),
    }
    if env.operator_child() && path.first() != Some(&"projects") {
        if let Err(error) =
            prepare_operator_child(env, store.as_ref(), &globals, &dotted, &namespace)
        {
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
    if dotted == "chat" {
        // Brokered routing above already refused a brokered project.
        let outcome = store.bootstrap(&globals.selection).and_then(|bootstrap| {
            let mut args = crate::chat::Args::from_namespace(&namespace);
            // Only this process's own terminal is watched for Ctrl-C.
            let mut terminal =
                crate::chat::TerminalInput::new(&mut stdin, env.claim_stdio && stdin_is_tty);
            let mut input = std::io::BufReader::new(&mut terminal);
            let mut io = crate::chat::Io {
                stdin: &mut input,
                stdin_is_tty,
                stdout: &mut stdout,
                stdout_is_tty,
                stderr: &mut stderr,
                stderr_is_tty,
                columns: env.columns.clone(),
                interrupt: None,
            };
            let data_root = bootstrap.paths.data_root.clone();
            let mut passphrase = |allow_prompt: bool| {
                // `resolve_db_passphrase_for_bypass`: the chain ordinary
                // commands unlock with; the book closes again at once.
                let mut config = bootstrap.runtime_config.clone();
                let mut book = store.open_book(
                    &data_root,
                    Unlock::Cli {
                        passphrase_fd: globals.db_passphrase_fd,
                        allow_prompt,
                        expected_database_identity: None,
                        initialize_version: None,
                    },
                    &mut config,
                )?;
                let secret = book.unlock_passphrase();
                book.close()?;
                Ok(secret)
            };
            crate::chat::run(
                &mut args,
                namespace.text("prompt").map(str::to_owned),
                namespace.text("prompt_text").map(str::to_owned),
                &mut io,
                crate::chat::Start {
                    data_root: &data_root,
                    env_file: Some(bootstrap.paths.env_file.as_str()),
                    passphrase: &mut passphrase,
                },
            )
        });
        return match outcome {
            Ok(Some(payload)) => {
                let mut sink = Sink {
                    output: output.as_deref(),
                    home: env.home.as_deref(),
                    stdout: &mut stdout,
                    stderr: &mut stderr,
                };
                match emit(&mut sink, format, "chat", payload) {
                    Ok(()) => 0,
                    Err(error) => fail(&mut stdout, &mut stderr, error),
                }
            }
            Ok(None) => 0,
            Err(error) if error.code == crate::chat::INTERRUPTED => {
                // An uncaught KeyboardInterrupt, after the daemon closed.
                let _ = stdout.flush();
                let _ = stderr.flush();
                kassiber_ipc::signal::exit_by_interrupt()
            }
            Err(error) => fail(&mut stdout, &mut stderr, error),
        };
    }
    let mut warnings = Vec::new();
    let mut prepare = |args: &Namespace| {
        if matches!(dotted.as_str(), "backends.create" | "backends.update") {
            crate::backends::prepare(args, &mut stdin, &mut warnings)
        } else {
            Ok(args.clone())
        }
    };

    let mut sink = Sink {
        output: output.as_deref(),
        home: env.home.as_deref(),
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let mut diagnostics = crate::diagnostics::FailureContext {
        args: crate::diagnostics::arguments(&namespace, None),
        report: None,
        unhandled_type: None,
        journal_replay_shape: None,
    };
    #[cfg(feature = "test-probe")]
    let mut sync_handoff = false;
    let result = match dotted.as_str() {
        command if crate::backup::COMMANDS.contains(&command) => {
            store
                .bootstrap(&globals.selection)
                .and_then(|bootstrap| {
                    diagnostics.args = crate::diagnostics::arguments(&namespace, Some(&bootstrap));
                    crate::backup::execute(store.as_ref(), &bootstrap.paths, command, &namespace)
                })
                .and_then(|payload| emit(&mut sink, format, &kind, payload))
        }
        command if crate::secrets::COMMANDS.contains(&command) => {
            store
                .bootstrap(&globals.selection)
                .and_then(|bootstrap| {
                    diagnostics.args = crate::diagnostics::arguments(&namespace, Some(&bootstrap));
                    crate::secrets::execute(store.as_ref(), &bootstrap.paths, command, &namespace)
                })
                .and_then(|payload| {
                    if kassiber_envelope::get(&payload, "remembered_unlock_warning").is_some() {
                        let _ = sink.stderr.write_all(b"warning: remembered_unlock_update_failed: the database passphrase changed, but the CLI credential-store copy could not be updated safely; inspect `kassiber secrets status`, remove any retained legacy credential, then re-enroll.\n");
                    }
                    emit(&mut sink, format, &kind, payload)
                })
        }
        command if crate::offline::COMMANDS.contains(&command) => store
            .bootstrap(&globals.selection)
            .and_then(|bootstrap| {
                diagnostics.args = crate::diagnostics::arguments(&namespace, Some(&bootstrap));
                crate::offline::execute(store.as_ref(), command, &namespace)
            })
            .and_then(|payload| emit(&mut sink, format, command, payload)),
        command if crate::update::COMMANDS.contains(&command) => {
            let table = format == crate::render::Format::Table && output.is_none();
            crate::update::execute(store.as_ref(), command, &namespace, &build, table).and_then(
                |outcome| match outcome {
                    crate::update::Outcome::Emit(kind, payload) => {
                        emit(&mut sink, format, kind, payload)
                    }
                    crate::update::Outcome::Text(result) => {
                        // `supports_color(sys.stdout)`.
                        use std::io::IsTerminal;
                        let color = std::io::stdout().is_terminal()
                            && std::env::var("TERM").as_deref() != Ok("dumb")
                            && std::env::var_os("NO_COLOR").is_none()
                            && std::env::var("CLICOLOR").as_deref() != Ok("0");
                        let text = kassiber_release::update::render(&result, color);
                        sink.stdout
                            .write_all(text.as_bytes())
                            .and_then(|_| sink.stdout.flush())
                            .map_err(|e| AppError::new("internal_error", e.to_string()))
                    }
                },
            )
        }
        command if crate::projects::COMMANDS.contains(&command) => {
            crate::projects::execute(store.as_ref(), command, &namespace)
                .and_then(|payload| emit(&mut sink, format, command, payload))
        }
        "commands.describe" => describe_command(&namespace, &globals.selection)
            .and_then(|catalog| emit(&mut sink, format, &kind, catalog)),
        // Static catalogs on no-database paths: the runtime layout is
        // bootstrapped as in Python, but no book is opened or created.
        "wallets.kinds" | "backends.kinds" => store
            .bootstrap(&globals.selection)
            .and_then(|bootstrap| {
                diagnostics.args = crate::diagnostics::arguments(&namespace, Some(&bootstrap));
                let catalog = if dotted == "wallets.kinds" {
                    kassiber_wallets::connections::wallet_kinds()
                } else {
                    kassiber_setup::backends::kinds()
                };
                emit(&mut sink, format, &kind, catalog)
            }),
        // `command_needs_database`: a guide without `--backend` reads no book.
        "btcpay.key-url" if !namespace.get("backend").is_some_and(kassiber_envelope::truthy) => store
            .bootstrap(&globals.selection)
            .and_then(|bootstrap| {
                diagnostics.args = crate::diagnostics::arguments(&namespace, Some(&bootstrap));
                kassiber_wallets::btcpay::key_url(&crate::connections::arguments(&namespace), |_| {
                    Err(AppError::new("internal_error", "no saved backend without a book"))
                })
            })
            .and_then(|payload| emit(&mut sink, format, &kind, payload)),
        "wallets.import-ledger" if namespace.flag("dry_run") => crate::wallets::preview_ledger(&namespace)
            .and_then(|payload| emit(&mut sink,format,&kind,payload)),
        "wallets.ledger-template" => crate::wallets::ledger_template(&namespace)
            .and_then(|payload| emit(&mut sink, format, &kind, payload)),
        // No book: the runtime layout is bootstrapped as in Python.
        "wallets.analyze-file" => store
            .bootstrap(&globals.selection)
            .and_then(|bootstrap| {
                diagnostics.args = crate::diagnostics::arguments(&namespace, Some(&bootstrap));
                crate::wallets::analyze_file(&namespace)
            })
            .and_then(|payload| emit(&mut sink, format, "wallets.file_analysis", payload)),
        _ if dotted.starts_with("wallets.") => with_book(
            env,
            store.as_ref(),
            &globals,
            stdin_is_tty,
            None,
            (&namespace, build.version, &mut diagnostics),
            |book, _| {
                #[cfg(feature = "test-probe")]
                if dotted == "wallets.sync" && env.test_sync_handoff {
                    let (_, profile) = book.resolve_scope(
                        namespace.get("workspace"),
                        namespace.get("profile"),
                    )?;
                    if !book.wallet_sync_supported(
                        &profile,
                        namespace.text("wallet"),
                        namespace.flag("all"),
                        namespace.flag("force_full"),
                    )? {
                        sync_handoff = true;
                        return Ok(());
                    }
                }
                let payload =
                    crate::wallets::execute(&namespace, book, stdin.as_mut(), sink.stdout, sink.stderr, stdin_is_tty)?;
                if dotted == "wallets.reveal-descriptor" && namespace.flag("material_only") {
                    let material = crate::wallets::material_text(&namespace, &payload)?;
                    return sink
                        .write_text(&material)
                        .map_err(|error| AppError::new("internal_error", error.to_string()));
                }
                let payload = if dotted == "wallets.identify" {
                    crate::wallets::identify_output(format, payload)
                } else {
                    payload
                };
                emit(&mut sink, format, &kind, payload)
            },
        ),
        command if crate::chats::COMMANDS.contains(&command) => with_book(
            env,
            store.as_ref(),
            &globals,
            stdin_is_tty,
            None,
            (&namespace, build.version, &mut diagnostics),
            |book, bootstrap| {
                let encrypted = || store.database_is_encrypted(&bootstrap.paths.data_root);
                let payload = crate::chats::execute(book, command, &namespace, &encrypted)?;
                emit(&mut sink, format, &kind, payload)
            },
        ),
        command if crate::ai::COMMANDS.contains(&command) => with_book(
            env,
            store.as_ref(),
            &globals,
            stdin_is_tty,
            None,
            (&namespace, build.version, &mut diagnostics),
            |book, _| {
                let payload =
                    crate::ai::execute(book, command, &namespace, stdin.as_mut(), &mut warnings)?;
                emit(&mut sink, format, &kind, payload)
            },
        ),
        "mcp.call" => crate::mcp::read_book(
            &store,
            build,
            crate::mcp::Open {
                selection: globals.selection.clone(),
                passphrase_fd: globals.db_passphrase_fd,
                allow_prompt: globals.db_passphrase_fd.is_none()
                    && stdin_is_tty
                    && !globals.non_interactive,
                env,
            },
            &kassiber_pyjson::PyStr::from(namespace.text("tool").unwrap_or_default()),
            &|| {
                crate::mcp::require_complete_pin(&namespace)?;
                crate::mcp::parse_arguments(namespace.get("arguments"))
            },
            namespace.text("workspace"),
            namespace.text("profile"),
            &|| {
                let (grant, agent_access) = crate::mcp::call_access(
                    store.as_ref(),
                    namespace.text("tool"),
                    namespace.text("grant"),
                    env.operator_child(),
                )?;
                Ok(crate::mcp::Grant {
                    grant,
                    agent_access,
                    client_name: namespace.text("client").map(str::to_owned),
                })
            },
        )
        .and_then(|payload| emit(&mut sink, format, &kind, payload)),
        _ if dotted.starts_with("imports.") => with_book(
            env,
            store.as_ref(),
            &globals,
            stdin_is_tty,
            None,
            (&namespace, build.version, &mut diagnostics),
            |book, _| {
                let (kind, payload) = crate::imports::execute(&namespace, book)?;
                emit(&mut sink, format, kind, payload)
            },
        ),
        _ => book_command(
            env,
            store.as_ref(),
            build,
            &globals,
            (&dotted, &namespace, stdin_is_tty, &mut diagnostics),
            &mut prepare,
            |payload| {
                if matches!(
                    dotted.as_str(),
                    "metadata.bip329.list" | "transactions.list" | "journals.list"
                ) {
                    let data = kassiber_envelope::get(&payload, "records")
                        .cloned()
                        .unwrap_or(Value::Null);
                    let keys: &[&str] = if dotted == "transactions.list" {
                        &[
                            "next_cursor",
                            "has_more",
                            "count",
                            "total",
                            "limit",
                            "sort",
                            "order",
                        ]
                    } else {
                        &["next_cursor", "has_more", "limit"]
                    };
                    let meta = keys
                        .iter()
                        .map(|key| {
                            (
                                (*key).to_owned(),
                                kassiber_envelope::get(&payload, key)
                                    .cloned()
                                    .unwrap_or(Value::Null),
                            )
                        })
                        .collect();
                    crate::render::emit_with_meta(&mut sink, format, &kind, data, meta)
                } else {
                    emit(&mut sink, format, &kind, payload)
                }
            },
        ),
    };
    let _ = sink.stderr.write_all(&warnings);
    match result {
        Ok(()) => {
            #[cfg(feature = "test-probe")]
            if sync_handoff {
                return 4;
            }
            0
        }
        Err(mut error) => {
            #[cfg(feature = "test-probe")]
            if journal_invocation_guard
                .as_ref()
                .is_some_and(|check| check())
            {
                if let Some(shape) = &diagnostics.journal_replay_shape {
                    let _ = writeln!(sink.stdout, "{shape}");
                    let _ = sink.stdout.flush();
                    return PROBE_NOT_IMPLEMENTED;
                }
            }
            // Python wraps layout I/O exceptions before these local services
            // as internal_error, just as it does their evidence-file failures.
            if error.code == "storage_io_error"
                && (dotted.starts_with("documents.")
                    || matches!(
                        dotted.as_str(),
                        "btcpay.provenance.list"
                            | "btcpay.provenance.links"
                            | "btcpay.provenance.suggest"
                            | "btcpay.provenance.review"
                    ))
            {
                error.code = "internal_error".into();
            }
            crate::diagnostics::write_error(
                &diagnostics.args,
                build.version,
                &error,
                diagnostics.report,
                sink.stderr,
            );
            let debug = kassiber_diagnostics::report::native_debug(
                diagnostics.unhandled_type.is_some(),
                namespace.flag("debug"),
            );
            crate::render::emit_error_with_debug(&mut sink, format, &error, debug);
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

/// Shared book bootstrap/unlock for runtime commands and registered services.
fn book_command(
    env: &Environment,
    store: &dyn BookStore,
    build: BuildInfo,
    globals: &Globals,
    command: (
        &str,
        &Namespace,
        bool,
        &mut crate::diagnostics::FailureContext,
    ),
    prepare: &mut dyn FnMut(&Namespace) -> Result<Namespace, AppError>,
    emit: impl FnOnce(Value) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let (operation, namespace, stdin_is_tty, diagnostics) = command;
    let initialize = operation == "init";
    let bootstrap = store.bootstrap(&globals.selection)?;
    diagnostics.args = crate::diagnostics::arguments(namespace, Some(&bootstrap));
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
        if operation == "diagnostics.collect" {
            return emit(crate::diagnostics::execute(
                book.as_mut(),
                namespace,
                &bootstrap,
                build.version,
                diagnostics,
            )?);
        }
        if namespace.text("command") == Some("rates") {
            return emit(crate::rates::run(book.as_mut(), namespace)?);
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
        if operation == "transactions.list" {
            return emit(crate::transactions::execute(book.as_mut(), namespace)?);
        }
        if operation == "maintenance.purge-legacy-ledger" {
            let confirm = namespace.flag("confirm");
            let payload = book.purge_legacy_ledger(confirm)?;
            return emit(payload);
        }
        if operation == "transactions.verify" {
            return emit(book.verify_transaction(
                namespace.get("workspace"),
                namespace.get("profile"),
                namespace.text("transaction").unwrap_or_default(),
            )?);
        }
        if operation.starts_with("journals.") && operation != "journals.process" {
            let args = dict(
                namespace
                    .items()
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.clone())),
            );
            return emit(book.journal_view(operation, Some(&args))?);
        }
        if operation == "journals.process" {
            let auto_pair = namespace.flag("auto_pair");
            let result = book
                .process_journals(
                    namespace.get("workspace"),
                    namespace.get("profile"),
                    auto_pair,
                    &mut |_| {},
                )
                .map_err(|failure| {
                    diagnostics.journal_replay_shape = failure.replay_shape;
                    *failure.error
                })?;
            if auto_pair {
                emit(result.payload().clone())?;
                return result.after_reply();
            }
            // A pair or dismissal made since the last run leaves the shared
            // review count's cached moves behind otherwise.
            let payload = result.payload().clone();
            result.after_reply()?;
            book.recount_transfer_candidates(namespace.get("workspace"), namespace.get("profile"))?;
            return emit(payload);
        }
        if crate::chain_analysis_ops::COMMANDS.contains(&operation) {
            return emit(crate::chain_analysis_ops::execute(
                book.as_mut(),
                operation,
                namespace,
            )?);
        }
        if operation.starts_with("chain-analysis.") {
            return emit(crate::chain_analysis_reads::execute(
                book.as_mut(),
                operation,
                namespace,
            )?);
        }
        if operation.starts_with("metadata.") {
            return emit(crate::metadata::execute(
                book.as_mut(),
                operation,
                namespace,
            )?);
        }
        if operation.starts_with("review.") {
            return emit(crate::review::execute(book.as_mut(), operation, namespace)?);
        }
        if operation.starts_with("transfers.") {
            return emit(crate::transfers::execute(
                book.as_mut(),
                operation,
                namespace,
            )?);
        }
        if operation.starts_with("loans.") {
            return emit(crate::loans::execute(book.as_mut(), operation, namespace)?);
        }
        if operation.starts_with("documents.") {
            return emit(crate::documents::execute(
                book.as_mut(),
                operation,
                namespace,
            )?);
        }
        if [
            "btcpay.provenance.suggest",
            "btcpay.provenance.links",
            "btcpay.provenance.list",
            "btcpay.provenance.review",
        ]
        .contains(&operation)
        {
            return emit(crate::commercial::execute(
                book.as_mut(),
                operation,
                namespace,
            )?);
        }
        if operation.starts_with("attachments.") {
            return emit(crate::attachments::execute(
                book.as_mut(),
                operation,
                namespace,
            )?);
        }
        if operation != "status" {
            let namespace = prepare(namespace)?;
            return emit(crate::services::execute(
                book.as_mut(),
                operation,
                &namespace,
            )?);
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
    if let Err(ref error) = outcome {
        if globals.diagnostics_out.is_some() {
            diagnostics.report = Some(
                book.diagnostics(&diagnostics.args, build.version, Some(error))
                    .map(|mut report| {
                        diagnostics.annotate(&mut report);
                        report
                    }),
            );
        }
    }
    let closed = book.close();
    outcome?;
    closed
}

/// Shared admission, unlock, project accounting and close for service families.
fn with_book(
    env: &Environment,
    store: &dyn BookStore,
    globals: &Globals,
    stdin_is_tty: bool,
    initialize_version: Option<&str>,
    diagnostics: (&Namespace, &str, &mut crate::diagnostics::FailureContext),
    execute: impl FnOnce(
        &mut dyn kassiber_runtime::store::Book,
        &kassiber_runtime::store::Bootstrap,
    ) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let (args, version, diagnostics) = diagnostics;
    let mut bootstrap = store.bootstrap(&globals.selection)?;
    diagnostics.args = crate::diagnostics::arguments(args, Some(&bootstrap));
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
            initialize_version,
        },
        &mut runtime_config,
    )?;
    bootstrap.runtime_config = runtime_config;
    let outcome = (|| {
        if let Some(project_id) = &bootstrap.paths.project_id {
            store.mark_project_opened(
                project_id,
                &bootstrap.paths.data_root,
                !bootstrap.selection_explicit,
            )?;
        }
        execute(book.as_mut(), &bootstrap)
    })();
    if let Err(ref error) = outcome {
        if globals.diagnostics_out.is_some() {
            diagnostics.report = Some(book.diagnostics(&diagnostics.args, version, Some(error)));
        }
    }
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
/// a broker child must carry its bindings, and only a broker child may. The
/// child enters its caller's directory, then proves the selected project is
/// the file the broker admitted.
fn check_operator_bindings(
    env: &Environment,
    store: &dyn BookStore,
    selection: &Selection,
) -> Result<(), AppError> {
    let invalid = |message: &str| AppError::new("operator_project_binding_invalid", message);
    if let Some(directory) = &env.operator_working_directory {
        if !env.operator_child() {
            return Err(invalid(
                "operator working directory is only valid in a broker child",
            ));
        }
        if std::env::set_current_dir(directory).is_err() {
            return Err(AppError::new(
                "operator_working_directory_unavailable",
                "the caller's working directory is no longer available",
            )
            .with_hint("Run the command from an existing directory or pass absolute paths."));
        }
    }
    let expected = match &env.operator_expected_project_identity {
        None if env.operator_child() => {
            return Err(invalid("operator child project binding is missing"))
        }
        None => return Ok(()),
        Some(_) if !env.operator_child() => {
            return Err(invalid(
                "operator project binding is only valid in a broker child",
            ))
        }
        Some(expected) => expected,
    };
    check_project_selection(selection)?;
    let project = store.canonical_project(&store.resolve_data_root(selection)?)?;
    if project.identity != *expected {
        return Err(AppError::new(
            "operator_project_replaced",
            "the operator project changed before the database was opened",
        )
        .with_details(dict([("project", s(project.public_id.as_str()))])));
    }
    Ok(())
}

/// `command_needs_database(args)`.
fn command_needs_database(dotted: &str, namespace: &Namespace) -> bool {
    if !crate::describe::needs_database(dotted) {
        return false;
    }
    if dotted == "wallets.import-ledger" && namespace.flag("dry_run") {
        return false;
    }
    if dotted == "btcpay.key-url" && namespace.text("backend").is_none() {
        // A link from --server-url is pure; only --backend reads a saved instance.
        return false;
    }
    true
}

/// A broker child, before dispatch: drain the lease pipe eagerly
/// (`prime_db_passphrase`) so the broker can feed the next secret pipe, and
/// for a command that bootstraps no database, authenticate the bound
/// database with it first (`_verify_operator_child_database`).
fn prepare_operator_child(
    env: &Environment,
    store: &dyn BookStore,
    globals: &Globals,
    dotted: &str,
    namespace: &Namespace,
) -> Result<(), AppError> {
    let passphrase = match globals.db_passphrase_fd {
        Some(fd) => {
            let fd = i32::try_from(fd).map_err(|_| {
                AppError::new(
                    "invalid_passphrase",
                    "passphrase descriptor must be non-negative",
                )
            })?;
            kassiber_secrets::prime_passphrase_descriptor(fd)
                .map_err(kassiber_secrets::service::storage_error)?;
            Some(
                kassiber_secrets::read_passphrase_from_fd(fd)
                    .map_err(kassiber_secrets::service::storage_error)?,
            )
        }
        None => None,
    };
    if command_needs_database(dotted, namespace) {
        return Ok(());
    }
    if env.operator_expected_database_identity.is_none() {
        return Err(AppError::new(
            "operator_project_binding_invalid",
            "operator child database binding is missing",
        ));
    }
    let Some(passphrase) = passphrase else {
        return Err(AppError::new(
            "operator_project_binding_invalid",
            "operator child database passphrase is missing",
        ));
    };
    let data_root = store.resolve_data_root(&globals.selection)?;
    store.verify_operator_child_database(&data_root, &Secret::new(passphrase.expose().to_owned()))
}

/// `_operator_expected_database_identity()`.
pub(crate) fn operator_expected_database_identity(
    env: &Environment,
) -> Result<Option<String>, AppError> {
    if env.operator_child() && env.operator_expected_database_identity.is_none() {
        return Err(AppError::new(
            "operator_project_binding_invalid",
            "operator child database binding is missing",
        ));
    }
    Ok(env.operator_expected_database_identity.clone())
}

/// The broker client for this binary's build identity.
pub(crate) fn operator_client(
    build: BuildInfo,
    env: &Environment,
) -> kassiber_operator::client::BrokerClient<kassiber_operator::client::SystemHost> {
    let mut host = kassiber_operator::client::SystemHost::new(
        kassiber_operator::build::current_build_identity(kassiber_operator::build::BuildInputs {
            version: build.version,
            channel: build.channel,
            commit: build.commit,
        }),
    );
    if let Some(runtime) = &env.operator_runtime {
        host.runtime = runtime.clone();
    }
    kassiber_operator::client::BrokerClient::new(host)
}

#[cfg(test)]
mod export_tests {
    use super::*;

    #[test]
    fn transaction_export_admission_registers_all_native_formats() {
        let mut invalid = Namespace::default();
        invalid.set("export_format", kassiber_envelope::s("unknown"));
        assert!(!implemented("transactions.export", &invalid));
        for format in ["csv", "xlsx", "cointracking-csv", "blockpit-csv"] {
            let argv = [
                "transactions",
                "export",
                "--export-format",
                format,
                "--file",
                "unused",
            ]
            .map(String::from);
            let args = parse_args(
                &argv,
                Context {
                    width: 78,
                    version: "test",
                },
            )
            .unwrap();
            assert_eq!(
                implemented("transactions.export", &args),
                ["csv", "xlsx", "cointracking-csv", "blockpit-csv"].contains(&format)
            );
        }
    }
}

//! Kassiber's AI provider broker ([plan 23]): the process the daemon starts
//! for every CLI-provider request, served by the `kassiber` binary as
//! `kassiber --provider-broker`. It adapts the installed agent CLIs (Codex,
//! Claude, OpenCode, ACP agents such as GitHub Copilot) to the daemon's
//! JSONL broker protocol, which `kassiber_ai::broker` speaks on the other
//! end.
//!
//! # Protocol
//!
//! The first stdin line is the request ([`protocol`]):
//!
//! - `status` probes the scoped providers in parallel and writes one
//!   `result` event with their [`ProviderStatus`] list;
//! - `models` writes one provider's models as a `result`, or fails;
//! - `chat` streams `status`, `delta`, `tool_call` and `done` events. When
//!   the request advertises tools, a [`ToolBridge`] starts first, and later
//!   stdin lines `{"command": "tool_results", "results": [...]}` answer its
//!   calls.
//!
//! A failure becomes one `error` event (redacted by
//! [`protocol::safe_error_message`]) and exit code 1. Every probe and turn
//! runs in a fresh private working directory ([`workdir`]); SIGTERM, SIGINT
//! and SIGHUP kill tracked children and remove tracked directories before
//! exit ([`cleanup`]).
//!
//! # Adapters
//!
//! Each provider is one [`Adapter`] in its own module, and [`adapter`] maps
//! a [`ProviderId`] to it. An adapter module owns everything specific to
//! its CLI and nothing else touches it:
//!
//! | Provider | Module | Static |
//! | --- | --- | --- |
//! | `codex` | `src/codex.rs` (and `src/codex/`) | `codex::CODEX` |
//! | `claude` | `src/claude.rs` (and `src/claude/`) | `claude::CLAUDE` |
//! | `opencode` | `src/opencode.rs` (and `src/opencode/`, both generations) | `opencode::OPENCODE` |
//! | `copilot` | `src/acp.rs` (and `src/acp/`) | `acp::COPILOT` |
//!
//! Submodules go in the module's own directory (`src/codex/app_server.rs`
//! declared from `codex.rs`), and tests beside them, so adapters can be
//! written in parallel without editing this file.
//!
//! An adapter uses the shared pieces and never reimplements them:
//! [`executables::resolve_executable`] to find its CLI,
//! [`executables::provider_command`] for every process it starts (the only
//! reviewed process launch in the crate; it applies
//! [`env::provider_environment`]), [`executables::run_provider`] for short
//! probes, [`cleanup::track_child`] for long-lived children,
//! [`protocol`]'s option checks ([`protocol::sensitive_context`],
//! [`protocol::stateless_session`], [`protocol::requested_fast_mode`],
//! [`protocol::safe_session_cursor`]), [`prompt`] for prompt text, and
//! [`bridge::mcp_command`] (or [`ToolBridge::request`] for a CLI that calls
//! tools itself) for Kassiber's tools. Errors are [`BrokerFailure`]s with
//! the raw message; the dispatcher redacts them.

pub mod bridge;
pub mod cleanup;
pub mod env;
pub mod executables;
pub mod mcp;
pub mod prompt;
pub mod protocol;
pub mod workdir;

pub mod acp;
pub mod claude;
pub mod codex;
pub mod opencode;

use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use serde_json::{Map, Value};

pub use bridge::ToolBridge;
pub use protocol::{BrokerFailure, ChatRequest, EventSink, ProviderId, ProviderStatus};

use protocol::{error_event, status_scope, Event, ProviderState};
use workdir::with_working_directory;

/// One provider runtime.
///
/// Both calls run inside a fresh, private working directory `cwd` that is
/// removed afterwards; nothing else in it is Kassiber's. Return failures
/// with their raw message: the dispatcher redacts and classifies them
/// (`not installed`/`not found` read as a missing CLI, `auth`/`login` as a
/// login prompt).
pub trait Adapter: Sync {
    /// The provider's status and models. A probe that cannot run reports
    /// `missing_executable` or `error` in the status; an `Err` fails the
    /// whole status request, as a rejected probe does in TypeScript.
    fn status(&self, cwd: &Path) -> Result<ProviderStatus, BrokerFailure>;

    /// One chat turn: write `status`, `delta` and (through `bridge`)
    /// `tool_call` events to `events`, ending with exactly one `done` on
    /// success. `bridge` is present when the request advertised tools.
    fn chat(
        &self,
        request: &ChatRequest,
        cwd: &Path,
        bridge: Option<&ToolBridge>,
        events: &EventSink,
    ) -> Result<(), BrokerFailure>;
}

/// The adapter for `provider` (`PROVIDERS` in `index.ts`).
pub fn adapter(provider: ProviderId) -> &'static dyn Adapter {
    match provider {
        ProviderId::Codex => &codex::CODEX,
        ProviderId::Claude => &claude::CLAUDE,
        ProviderId::OpenCode => &opencode::OPENCODE,
        ProviderId::Copilot => &acp::COPILOT,
    }
}

/// Which adapter serves a provider; tests swap in fakes.
pub type Adapters<'a> = &'a (dyn Sync + Fn(ProviderId) -> &'static dyn Adapter);

/// `status(provider)` in `index.ts`: one probe in its own directory.
fn probe(adapters: Adapters<'_>, provider: ProviderId) -> Result<ProviderStatus, BrokerFailure> {
    with_working_directory(provider, |cwd| adapters(provider).status(cwd))
}

/// The broker's stdin: the request line, then (for a chat with tools) the
/// daemon's `tool_results` lines.
pub type Input = Box<dyn BufRead + Send>;

/// One stdin line, decoded as Node's readline does (invalid UTF-8 becomes
/// U+FFFD, a trailing `\r` is dropped); `None` at end of input.
fn read_line(input: &mut Input) -> Option<String> {
    let mut line = Vec::new();
    match input.read_until(b'\n', &mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => {
            if line.last() == Some(&b'\n') {
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
            }
            Some(String::from_utf8_lossy(&line).into_owned())
        }
    }
}

/// `main()` in `index.ts` over explicit streams and adapters.
pub fn dispatch(
    mut input: Input,
    events: &EventSink,
    adapters: Adapters<'_>,
) -> Result<(), BrokerFailure> {
    let Some(first) = read_line(&mut input) else {
        return Err(BrokerFailure::new(
            "Kassiber provider broker received no request.",
        ));
    };
    let request: Map<String, Value> = serde_json::from_str(&first)
        .map_err(|_| BrokerFailure::new("Kassiber provider broker received an invalid request."))?;
    match request.get("command").and_then(Value::as_str) {
        Some("status") => {
            let scope = status_scope(&request)?;
            let statuses = std::thread::scope(|threads| {
                let probes: Vec<_> = scope
                    .iter()
                    .map(|provider| threads.spawn(move || probe(adapters, *provider)))
                    .collect();
                probes
                    .into_iter()
                    .map(|probe| {
                        probe.join().unwrap_or_else(|_| {
                            Err(BrokerFailure::new("Provider status probe failed."))
                        })
                    })
                    .collect::<Vec<_>>()
            });
            let statuses = statuses.into_iter().collect::<Result<Vec<_>, _>>()?;
            events.write(&Event::result(&statuses)?);
            Ok(())
        }
        Some("models") => {
            let provider = request
                .get("provider")
                .and_then(ProviderId::parse)
                .ok_or_else(|| BrokerFailure::new("Unknown provider."))?;
            let snapshot = probe(adapters, provider)?;
            if snapshot.state == ProviderState::Error {
                return Err(BrokerFailure::new(snapshot.message));
            }
            events.write(&Event::result(&snapshot.models)?);
            Ok(())
        }
        Some("chat") => {
            if request
                .get("provider")
                .and_then(ProviderId::parse)
                .is_none()
            {
                return Err(BrokerFailure::new("Unknown provider."));
            }
            let request: ChatRequest = serde_json::from_str(&first).map_err(|_| {
                BrokerFailure::new("Kassiber provider broker received an invalid chat request.")
            })?;
            chat(request, input, events, adapters)
        }
        _ => Err(BrokerFailure::new(
            "Kassiber provider broker received an unknown command.",
        )),
    }
}

/// A chat turn: the bridge (when tools were advertised) and its reply
/// reader, then the adapter in a working directory, then the bridge closed.
fn chat(
    request: ChatRequest,
    mut input: Input,
    events: &EventSink,
    adapters: Adapters<'_>,
) -> Result<(), BrokerFailure> {
    let bridge = if request.tools.is_empty() {
        None
    } else {
        Some(std::sync::Arc::new(ToolBridge::start(
            &request.tools,
            events.clone(),
        )?))
    };
    if let Some(bridge) = &bridge {
        // Reads until stdin ends; the process exits without waiting for it.
        let bridge = std::sync::Arc::clone(bridge);
        let _ = std::thread::Builder::new()
            .name("tool-results".into())
            .spawn(move || {
                while let Some(line) = read_line(&mut input) {
                    let Ok(message) = serde_json::from_str::<Map<String, Value>>(&line) else {
                        continue;
                    };
                    if message.get("command").and_then(Value::as_str) != Some("tool_results") {
                        continue;
                    }
                    if let Some(Value::Array(results)) = message.get("results") {
                        bridge.resolve(results);
                    }
                }
            });
    }
    let turn = with_working_directory(request.provider, |cwd| {
        adapters(request.provider).chat(&request, cwd, bridge.as_deref(), events)
    });
    let closed = bridge.as_ref().map_or(Ok(()), |bridge| bridge.close());
    closed?;
    turn
}

/// `kassiber --provider-broker`: install termination cleanup, serve one
/// request from stdin to stdout, and return the exit code.
pub fn serve() -> i32 {
    let events = EventSink::stdout();
    // Without it a signalled broker would leave its children and
    // directories behind, or ignore the signal: refuse to start.
    if cleanup::clean_up_on_termination().is_err() {
        events.write(&error_event(&BrokerFailure::new(
            "Kassiber could not install the broker's termination cleanup.",
        )));
        return 1;
    }
    let input: Input = Box::new(BufReader::new(std::io::stdin()));
    match dispatch(input, &events, &adapter) {
        Ok(()) => 0,
        Err(failure) => {
            events.write(&error_event(&failure));
            1
        }
    }
}

/// `kassiber --provider-broker-mcp <socket> <manifest>`.
pub fn serve_mcp(socket_path: &str, manifest: &str) -> i32 {
    mcp::serve_mcp(socket_path, manifest)
}

/// Read all of `input` (tests).
#[doc(hidden)]
pub fn input_from(text: impl Into<Vec<u8>>) -> Input {
    Box::new(BufReader::new(std::io::Cursor::new(text.into())))
}

#[doc(hidden)]
pub fn input_reader(reader: impl Read + Send + 'static) -> Input {
    Box::new(BufReader::new(reader))
}

#[cfg(test)]
mod tests;

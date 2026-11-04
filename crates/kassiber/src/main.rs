//! The `kassiber` binary: the CLI and `kassiber daemon` (plan 21, R1).
//!
//! It arms the egress gate before anything else, so a test environment
//! that sets `KASSIBER_TEST_NO_EGRESS` on a build without the attempt
//! observer stops with exit code 78 instead of running unguarded.

use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::Arc;

use kassiber_cli::{run, Environment, Streams};
use kassiber_net::{Gate, Ledger, Offline};
use kassiber_runtime::store::BookStore;
use kassiber_runtime::BuildInfo;
mod real_store;

const BUILD: BuildInfo = BuildInfo {
    version: env!("CARGO_PKG_VERSION"),
    channel: option_env!("KASSIBER_BUILD_CHANNEL"),
    commit: option_env!("KASSIBER_BUILD_COMMIT"),
};

/// Production always uses the real, fail-closed storage adapter.
#[cfg(not(feature = "test-harness"))]
fn store() -> Arc<dyn BookStore> {
    Arc::new(real_store::RealStore::default())
}

/// Test builds run against the in-memory store when
/// `KASSIBER_RUST_TEST_FAKE_STORE` holds its JSON configuration.
#[cfg(feature = "test-harness")]
fn store() -> Arc<dyn BookStore> {
    use kassiber_runtime::fake::{FakeConfig, FakeStore};
    match std::env::var("KASSIBER_RUST_TEST_FAKE_STORE") {
        Ok(config) => match kassiber_pytext::loads(&config) {
            Ok(config) => Arc::new(FakeStore::new(FakeConfig::from_value(&config))),
            Err(error) => {
                eprintln!("KASSIBER_RUST_TEST_FAKE_STORE is not valid JSON: {error}");
                std::process::exit(78);
            }
        },
        Err(_) => Arc::new(real_store::RealStore::default()),
    }
}

fn main() -> ExitCode {
    // Arm the process-wide attempt observer before dispatch. Service clients
    // supply their own fresh offline preference reader and exact-action grant.
    let _gate = Gate::new(|| Offline::Unavailable, Ledger::process());

    let mut argv = Vec::new();
    for argument in std::env::args_os().skip(1) {
        match argument.into_string() {
            Ok(argument) => argv.push(argument),
            Err(_) => {
                // Python would carry undecodable bytes as surrogate escapes;
                // a path or name the port cannot represent is refused.
                eprintln!("kassiber: error: command-line arguments must be valid UTF-8");
                return ExitCode::from(2);
            }
        }
    }
    let stdin = std::io::stdin();
    let streams = Streams {
        stdin_is_tty: stdin.is_terminal(),
        stdin: Box::new(stdin),
        stdout: Box::new(std::io::stdout()),
        stderr: Box::new(std::io::stderr()),
    };
    let code = run(&argv, &Environment::from_process(), store(), BUILD, streams);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

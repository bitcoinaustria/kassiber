//! `update` and `verify-download`: no book, no bootstrap. The update check
//! contacts GitHub only for the explicit command, after consent.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::{store::BookStore, BuildInfo};

pub const COMMANDS: &[&str] = &["update", "verify-download"];

/// What the shell emits: an envelope kind and payload, or (for a table
/// update check without `--output`) the human status text.
pub enum Outcome {
    Emit(&'static str, Value),
    Text(Value),
}

pub fn execute(
    store: &dyn BookStore,
    command: &str,
    args: &Namespace,
    build: &BuildInfo,
    table_to_stdout: bool,
) -> Result<Outcome, AppError> {
    if command == "verify-download" {
        let field = |name: &str| args.get(name).cloned().unwrap_or(Value::Null);
        let payload = store.local_service(
            "verify-download",
            &dict([
                ("artifact", field("artifact")),
                ("manifest", field("manifest")),
                ("signature", field("signature")),
                ("public_key", field("public_key")),
                ("fingerprint", field("fingerprint")),
            ]),
        )?;
        return Ok(Outcome::Emit("verify-download", payload));
    }
    let build_args = dict([
        ("version", s(build.version)),
        ("channel", build.channel.map_or(Value::Null, s)),
    ]);
    if args.flag("disable_checks") {
        store.local_service("update.disable", &build_args)?;
        return Ok(Outcome::Emit(
            "update.preference",
            dict([
                ("enabled", Value::Bool(false)),
                ("contacts_github", Value::Bool(false)),
            ]),
        ));
    }
    if args.flag("enable_checks") {
        store.local_service("update.enable", &build_args)?;
    }
    if args.flag("status") {
        return Ok(Outcome::Emit(
            "update.preference",
            store.local_service("update.status", &build_args)?,
        ));
    }
    let result = store.local_service("update.check", &build_args)?;
    Ok(if table_to_stdout {
        Outcome::Text(result)
    } else {
        Outcome::Emit("update", result)
    })
}

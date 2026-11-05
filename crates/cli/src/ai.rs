//! `ai providers ...` and `ai models` (R6-6a): provider configuration and
//! the two leaves that contact a provider, served by `kassiber-ai` on the
//! open book. The API key comes from `--api-key-fd`, `--api-key-stdin` or
//! the deprecated argv shim, read after the book is unlocked, as Python
//! reads it; it is never echoed.

use crate::argparse::Namespace;
use kassiber_ai::discovery::Cache;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::{AiSurface, Book};

pub const COMMANDS: &[&str] = kassiber_ai::service::COMMANDS;

/// The CLI lends no keychain: native keys are desktop-only.
struct Host {
    cache: Cache,
}

impl kassiber_ai::service::Host for Host {
    fn secret_store(
        &mut self,
        _op: &str,
        _provider_name: &str,
        _store_id: &str,
        _service: &str,
        _account: &str,
        _secret: Option<&str>,
    ) -> Result<Value, AppError> {
        Err(AppError::new(
            "secret_store_unavailable",
            "native AI provider secret storage is available only in the desktop app",
        )
        .retryable(true))
    }

    fn discovery(&self) -> &Cache {
        &self.cache
    }

    fn gate(&self) -> kassiber_ai::service::Gate {
        kassiber_ai::service::system_gate()
    }
}

/// `read_secret_from_args(args, "api-key", legacy_attr="api_key")`.
fn api_key(
    args: &Namespace,
    stdin: &mut dyn std::io::Read,
    warnings: &mut dyn std::io::Write,
) -> Result<Option<String>, AppError> {
    if let Some(Value::Int(fd)) = args.get("api_key_fd") {
        let fd = i32::try_from(fd.clone()).map_err(|_| {
            AppError::new(
                "invalid_passphrase",
                "passphrase descriptor must be non-negative",
            )
        })?;
        return kassiber_secrets::read_passphrase_from_fd(fd)
            .map(|secret| Some(secret.expose().to_owned()))
            .map_err(|e| AppError::new(e.code, e.message));
    }
    if args.flag("api_key_stdin") {
        let mut raw = String::new();
        stdin.read_to_string(&mut raw).map_err(|_| {
            AppError::new("invalid_secret_input", "could not read secret from stdin")
        })?;
        if raw.ends_with('\n') {
            raw.pop();
        }
        if raw.ends_with('\r') {
            raw.pop();
        }
        if raw.is_empty() {
            return Err(AppError::new(
                "invalid_secret_input",
                "api key value read from stdin is empty",
            ));
        }
        if raw.contains('\0') {
            return Err(AppError::new(
                "invalid_secret_input",
                "api key value contains NUL bytes",
            ));
        }
        return Ok(Some(raw));
    }
    match args.text("api_key") {
        Some(value) if !value.is_empty() => {
            let _ = writeln!(
                warnings,
                "warning: --api-key <value> exposes secrets in shell history; prefer --api-key-stdin or --api-key-fd FD."
            );
            Ok(Some(value.to_owned()))
        }
        _ => Ok(None),
    }
}

pub fn execute(
    book: &mut dyn Book,
    command: &str,
    args: &Namespace,
    stdin: &mut dyn std::io::Read,
    warnings: &mut dyn std::io::Write,
) -> Result<Value, AppError> {
    let mut payload = dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone())));
    if matches!(command, "ai.providers.create" | "ai.providers.update") {
        let key = api_key(args, stdin, warnings)?;
        kassiber_envelope::set(&mut payload, "api_key", key.map_or(Value::Null, s));
    }
    let mut host = Host {
        cache: Cache::default(),
    };
    book.ai_providers(AiSurface::Cli, command, Some(&payload), &mut host)
}

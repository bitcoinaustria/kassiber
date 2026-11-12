//! Backend configuration, shared with desktop services.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
pub const COMMANDS: &[&str] = &[
    "backends.list",
    "backends.kinds",
    "backends.get",
    "backends.create",
    "backends.update",
    "backends.delete",
    "backends.set-default",
    "backends.clear-default",
    "backends.reveal-token",
];
pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    book.service(
        command,
        &dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone()))),
    )
}

/// Resolve safe CLI input channels after book unlock. Warnings never contain
/// submitted values; descriptors are consumed/closed by the secrets crate.
pub fn prepare(
    args: &Namespace,
    stdin: &mut dyn std::io::Read,
    warnings: &mut dyn std::io::Write,
) -> Result<Namespace, AppError> {
    use kassiber_envelope::{get, s};
    let mut prepared = args.clone();
    let mut config = Vec::new();
    for key in ["auth_header", "token", "username", "password"] {
        let base = key.replace('_', "-");
        let v = if let Some(Value::Int(fd)) = args.get(&format!("{key}_fd")) {
            let fd = i32::try_from(fd.clone()).map_err(|_| {
                AppError::new(
                    "invalid_passphrase",
                    "passphrase descriptor must be non-negative",
                )
            })?;
            Some(
                kassiber_secrets::read_passphrase_from_fd(fd)
                    .map_err(|e| AppError::new(e.code, e.message))?
                    .expose()
                    .to_owned(),
            )
        } else if args.flag(&format!("{key}_stdin")) {
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
            let label = base.replace('-', " ");
            if raw.is_empty() {
                return Err(AppError::new(
                    "invalid_secret_input",
                    format!("{label} value read from stdin is empty"),
                ));
            }
            if raw.contains('\0') {
                return Err(AppError::new(
                    "invalid_secret_input",
                    format!("{label} value contains NUL bytes"),
                ));
            }
            Some(raw)
        } else if let Some(v) = args.text(key) {
            let _=writeln!(warnings,"warning: --{base} <value> exposes secrets in shell history; prefer --{base}-stdin or --{base}-fd FD.");
            Some(v.to_owned())
        } else {
            None
        };
        if ["username", "password"].contains(&key) {
            if let Some(v) = v {
                config.push((key.to_owned(), s(v)));
            }
        } else {
            prepared.set(key, v.map_or(Value::Null, s));
        }
        // Python checks shared stdin after auth_header/token evaluation, before
        // username/password are read by _backend_extra_config.
        if key == "token" {
            let active: Vec<_> = ["token", "auth_header", "username", "password"]
                .into_iter()
                .filter(|k| args.flag(&format!("{k}_stdin")))
                .collect();
            if active.len() > 1 {
                return Err(AppError::new(
                    "invalid_secret_input",
                    "only one --*-stdin option may be used per invocation",
                )
                .with_details(dict([(
                    "conflicting",
                    Value::List(active.into_iter().map(s).collect()),
                )]))
                .with_hint("Use --<name>-fd FD for additional secret inputs."));
            }
        }
    }
    for key in [
        "insecure",
        "cookiefile",
        "certificate",
        "lightning_cli",
        "lightning_dir",
        "rpc_file",
        "commando_peer_id",
        "display_name",
        "silent_payments",
        "silent_payment_scan_file",
        "silent_payment_scan_path",
        "wallet_prefix",
    ] {
        if let Some(v) = args.get(key).filter(|v| !matches!(v, Value::Null)) {
            config.push((
                if key == "wallet_prefix" {
                    "walletprefix"
                } else {
                    key
                }
                .to_owned(),
                v.clone(),
            ));
        }
    }
    prepared.set(
        "config",
        if config.is_empty() {
            Value::Null
        } else {
            dict(config)
        },
    );
    let payload = dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone())));
    if let Some(Value::List(fields)) = get(&payload, "clear") {
        prepared.set(
            "clear",
            Value::List(
                fields
                    .iter()
                    .map(|v| {
                        let k = kassiber_envelope::py_str(v);
                        s(if k == "wallet-prefix" {
                            "walletprefix".to_owned()
                        } else {
                            k.replace('-', "_")
                        })
                    })
                    .collect(),
            ),
        );
    }
    Ok(prepared)
}

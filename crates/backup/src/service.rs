//! CLI adapter for the archive service. It deliberately bypasses the ordinary
//! persistent runtime connection, as backup/cli.py does.
use crate::*;
use kassiber_ownership::OwnerNamespace;
use kassiber_secrets::{NativeCredentialReader, Passphrase, UnlockOptions, UnlockSession};
use zeroize::Zeroizing;

fn fd(args: &Value, key: &str) -> Result<Option<i32>> {
    match get(args, key) {
        Some(Value::Int(n)) => i32::try_from(n.clone()).map(Some).map_err(|_| {
            AppError::new(
                "invalid_passphrase",
                "passphrase descriptor must be non-negative",
            )
        }),
        _ => Ok(None),
    }
}
fn optional(args: &Value, key: &str) -> Option<String> {
    get(args, key)
        .filter(|v| !matches!(v, Value::Null))
        .map(py_str)
}
fn expand(raw: &str) -> Result<PathBuf> {
    kassiber_paths::Context::from_env()
        .and_then(|c| c.expand_user(Path::new(raw)))
        .map_err(kassiber_secrets::service::storage_error)
}
fn outer_passphrase(args: &Value, export: bool) -> Result<Passphrase> {
    if let Some(fd) = fd(args, "backup_passphrase_fd")? {
        return kassiber_secrets::read_passphrase_from_fd(fd)
            .map_err(kassiber_secrets::service::storage_error);
    }
    if get(args, "non_interactive").is_some_and(truthy) || get(args, "machine").is_some_and(truthy)
    {
        return Err(AppError::new("interaction_required", "backup passphrase input is required in non-interactive mode")
            .with_hint("Pass --backup-passphrase-fd, or use --recipient for backup export and --identity-file for backup import."));
    }
    let prompt = |label| {
        rpassword::prompt_password(label)
            .map(Zeroizing::new)
            .map_err(|_| {
                AppError::new(
                    "invalid_passphrase",
                    "no TTY available for passphrase prompt",
                )
                .with_hint(
                    "Run interactively or pass --db-passphrase-fd from a controlling process.",
                )
            })
    };
    let pass = Passphrase::new(prompt("Backup passphrase: ")?)
        .map_err(kassiber_secrets::service::storage_error)?;
    if export && pass.expose() != prompt("Confirm backup passphrase: ")?.as_str() {
        return Err(
            AppError::new("invalid_passphrase", "passphrases do not match")
                .with_hint("Re-run the command and re-enter the passphrase carefully."),
        );
    }
    Ok(pass)
}

pub fn execute(
    namespace: &OwnerNamespace,
    paths: &RuntimePaths,
    operation: &str,
    args: &Value,
    version: &str,
) -> Result<Value> {
    if operation == "backup.export" {
        let output = expand(&optional(args, "file").unwrap_or_default())?;
        let mut unlock = UnlockSession::default();
        if let Some(fd) = fd(args, "db_passphrase_fd")? {
            unlock
                .prime_fd(fd)
                .map_err(kassiber_secrets::service::storage_error)?;
        }
        let (book, password) = unlock
            .open_in_namespace(
                &paths.data_root,
                UnlockOptions {
                    non_interactive: get(args, "machine").is_some_and(truthy)
                        || get(args, "non_interactive").is_some_and(truthy),
                    ..Default::default()
                },
                &NativeCredentialReader,
                namespace,
                |pass, expected| {
                    kassiber_store::Book::open_migrating(
                        &paths.data_root,
                        kassiber_store::OpenOptions {
                            passphrase: pass,
                            expected_database_identity: expected,
                            ..Default::default()
                        },
                        true,
                    )
                    .map(|book| (book, pass.map(|p| Zeroizing::new(p.to_owned()))))
                },
            )
            .map_err(kassiber_secrets::service::storage_error)?;
        book.close()
            .map_err(|f| kassiber_secrets::service::storage_error(f.error))?;
        let recipients = match get(args, "recipient") {
            Some(Value::List(items)) if !items.is_empty() => {
                Some(items.iter().map(py_str).collect::<Vec<_>>())
            }
            _ => None,
        };
        let outer = if recipients.is_none() {
            Some(outer_passphrase(args, true)?)
        } else {
            None
        };
        let result = export(
            paths,
            &output,
            password.as_ref().map(|p| p.as_str()),
            outer.as_ref().map(Passphrase::expose),
            recipients.as_deref(),
            version,
        )?;
        return Ok(dict([
            ("output", text(&result.output)),
            (
                "size_bytes",
                Value::from(fs::metadata(&result.output).map_err(io_error)?.len()),
            ),
            ("manifest", result.manifest),
            ("age_backend", s(result.age_backend)),
        ]));
    }
    if operation == "backup.import" {
        // Installation stays unavailable until the cross-runtime crash-recovery
        // protocol has been implemented and tested. Staging never mutates the
        // target book.
        if get(args, "install").is_some_and(truthy) {
            return Err(AppError::new(
                "not_implemented",
                "backup installation awaits cross-runtime restore recovery",
            )
            .with_details(dict([
                ("component", s("backup")),
                ("command", s("backup.import --install")),
            ])));
        }
        let archive = expand(&optional(args, "archive").unwrap_or_default())?;
        let identity = optional(args, "identity_file")
            .filter(|x| !x.is_empty())
            .map(|p| expand(&p))
            .transpose()?;
        let pass = if identity.is_none() {
            Some(outer_passphrase(args, false)?)
        } else {
            None
        };
        let result = import(
            &archive,
            pass.as_ref().map(Passphrase::expose),
            identity.as_deref(),
        )?;
        let staging = result.staging();
        let unavailable = if result.unavailable.is_empty() {
            Value::Null
        } else {
            dict([
                ("code", s("secret_ref_unavailable")),
                ("hint", s(SECRET_REF_HINT)),
                ("details", dict([("refs", Value::List(result.unavailable))])),
            ])
        };
        let _ = result.workspace.keep();
        return Ok(dict([
            ("archive", text(&archive)),
            ("staging_path", text(&staging)),
            ("installed_data_root", Value::Null),
            ("pre_restore_backup", Value::Null),
            ("temporary_artifacts_cleaned", Value::Bool(false)),
            ("manifest", result.manifest),
            ("secret_ref_unavailable", unavailable),
        ]));
    }
    Err(AppError::new(
        "not_implemented",
        "backup operation is not implemented",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unverified_installation_never_decrypts_or_creates_the_target() {
        let root = tempfile::tempdir().unwrap();
        let paths = kassiber_paths::Context::from_env()
            .unwrap()
            .resolve_for_bootstrap(&kassiber_paths::ResolveOptions {
                data_root: Some(root.path().join("target/data")),
                ..Default::default()
            })
            .unwrap();
        let args = dict([
            ("install", Value::Bool(true)),
            ("machine", Value::Bool(true)),
        ]);
        assert_eq!(
            execute(
                &OwnerNamespace::isolated(root.path().join("owners")),
                &paths,
                "backup.import",
                &args,
                "test"
            )
            .unwrap_err()
            .code,
            "not_implemented"
        );
        assert!(!paths.data_root.exists());
    }
}

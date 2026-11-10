//! CLI adapter for the archive service. It deliberately bypasses the ordinary
//! persistent runtime connection, as backup/cli.py does.
use crate::*;
use kassiber_ownership::OwnerNamespace;
use kassiber_secrets::{NativeCredentialReader, Passphrase, UnlockOptions, UnlockSession};
use zeroize::Zeroizing;

pub fn restore_error(failure: kassiber_paths::Error) -> AppError {
    let mut error = kassiber_secrets::service::storage_error(failure);
    if error.code == "project_in_use" {
        if let Some(details) = &error.details {
            error.details = Some(dict([
                (
                    "project",
                    get(details, "project").cloned().unwrap_or(Value::Null),
                ),
                (
                    "owner",
                    get(details, "owner")
                        .cloned()
                        .unwrap_or_else(|| s("unknown")),
                ),
            ]));
        }
    } else if error.code == "restore_rollback_failed" {
        let recovery = error
            .details
            .as_ref()
            .and_then(|d| get(d, "recovery_path"))
            .map(py_str)
            .unwrap_or_else(|| "None".into());
        error.hint = Some(
            format!(
                "Keep the recovery copy at {recovery}; restore it before reopening this container."
            )
            .into_boxed_str(),
        );
    }
    error
}

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
        let install = get(args, "install").is_some_and(truthy);
        let destination = optional(args, "target_data_root")
            .filter(|s| !s.is_empty())
            .map(|s| expand(&s))
            .transpose()?
            .unwrap_or_else(|| paths.data_root.clone());
        let _maintenance = if install {
            fs::create_dir_all(&destination).map_err(io_error)?;
            let lease = kassiber_store::restore::maintenance(namespace, &destination, None)
                .map_err(restore_error)?;
            kassiber_store::restore::recover(&destination, &mut |_| Ok(()))
                .map_err(restore_error)?;
            Some(lease)
        } else {
            None
        };
        let result = import(
            &archive,
            pass.as_ref().map(Passphrase::expose),
            identity.as_deref(),
        )?;
        let staging = result.staging();
        let recovery = if install {
            kassiber_store::restore::install(&staging, &destination).map_err(restore_error)?
        } else {
            None
        };
        let unavailable = if result.unavailable.is_empty() {
            Value::Null
        } else {
            dict([
                ("code", s("secret_ref_unavailable")),
                ("hint", s(SECRET_REF_HINT)),
                ("details", dict([("refs", Value::List(result.unavailable))])),
            ])
        };
        if install {
            let temporary = result.workspace.path().to_owned();
            result.workspace.close().map_err(|e| {
                AppError::new("restore_cleanup_failed", "backup was installed but the decrypted temporary restore workspace could not be removed")
                    .with_hint(format!("Remove the temporary restore workspace manually: {}", temporary.display()))
                    .with_details(dict([("temporary_path", text(&temporary)), ("error", s(e.to_string()))]))
            })?;
        } else {
            let _ = result.workspace.keep();
        }
        return Ok(dict([
            ("archive", text(&archive)),
            (
                "staging_path",
                if install { Value::Null } else { text(&staging) },
            ),
            (
                "installed_data_root",
                if install {
                    text(&destination)
                } else {
                    Value::Null
                },
            ),
            (
                "pre_restore_backup",
                recovery.as_deref().map_or(Value::Null, text),
            ),
            ("temporary_artifacts_cleaned", Value::Bool(install)),
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
    use kassiber_ownership::OwnerKind;

    #[test]
    fn installation_requires_both_owner_roles_before_decryption_and_preserves_live_files() {
        for role in [OwnerKind::Broker, OwnerKind::Desktop] {
            let root = tempfile::tempdir().unwrap();
            let data = root.path().join("target/data");
            fs::create_dir_all(&data).unwrap();
            let namespace = OwnerNamespace::isolated(root.path().join("owners"));
            let project = namespace.canonical_project(&data).unwrap();
            let lease = namespace.acquire(project, role, "held").unwrap();
            let mut context = kassiber_paths::Context::from_env().unwrap();
            context.home = root.path().join("home");
            context.environ.clear();
            let paths = context
                .resolve_for_bootstrap(&kassiber_paths::ResolveOptions {
                    data_root: Some(data.clone()),
                    ..Default::default()
                })
                .unwrap();
            let failure = execute(
                &namespace,
                &paths,
                "backup.import",
                &dict([
                    ("install", Value::Bool(true)),
                    ("identity_file", s("/missing.identity")),
                    ("archive", s("/missing.kassiber")),
                ]),
                "test",
            )
            .unwrap_err();
            assert_eq!(failure.code, "project_in_use");
            assert!(failure.retryable);
            assert_eq!(
                failure.message,
                "database maintenance requires exclusive project access"
            );
            assert!(!data.join(DATABASE).exists());
            drop(lease);
            let _maintenance =
                kassiber_store::restore::maintenance(&namespace, &data, None).unwrap();
            for role in [OwnerKind::Broker, OwnerKind::Desktop] {
                assert_eq!(
                    namespace
                        .acquire(namespace.canonical_project(&data).unwrap(), role, "late")
                        .err()
                        .unwrap()
                        .code,
                    "project_in_use"
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn staging_only_import_never_requires_exclusive_ownership() {
        use std::os::fd::IntoRawFd;
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("book/data");
        fs::create_dir_all(&data).unwrap();
        let namespace = OwnerNamespace::isolated(root.path().join("owners"));
        let project = namespace.canonical_project(&data).unwrap();
        let _owner = namespace
            .acquire(project, OwnerKind::Desktop, "held")
            .unwrap();
        let mut context = kassiber_paths::Context::from_env().unwrap();
        context.home = root.path().join("home");
        context.environ.clear();
        let paths = context
            .resolve_for_bootstrap(&kassiber_paths::ResolveOptions {
                data_root: Some(data),
                ..Default::default()
            })
            .unwrap();
        let input = root.path().join("secret-input");
        fs::write(&input, "synthetic outer passphrase").unwrap();
        let fd = File::open(input).unwrap().into_raw_fd();
        let failure = execute(
            &namespace,
            &paths,
            "backup.import",
            &dict([
                ("archive", text(&root.path().join("absent.kassiber"))),
                ("backup_passphrase_fd", Value::from(fd)),
            ]),
            "test",
        )
        .unwrap_err();
        assert_eq!(failure.code, "missing_backup");
        assert!(!paths.database.exists());
    }
}

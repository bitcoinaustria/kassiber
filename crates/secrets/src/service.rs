//! Explicit encryption commands. They bypass ordinary schema bootstrap, just
//! like Python: verification and artifact inspection must never migrate a book.
use crate::{credentials, policy, CredentialStore, NativeCredentialReader};
use crate::{read_passphrase_from_fd, Passphrase};
use kassiber_envelope::{dict, get, s, truthy, AppError};
use kassiber_ownership::{OwnerKind, OwnerNamespace};
use kassiber_paths::RuntimePaths;
use kassiber_pyjson::Value;
use kassiber_store::encryption;
use std::path::Path;
use zeroize::Zeroizing;

pub fn storage_error(error: kassiber_paths::Error) -> AppError {
    AppError {
        code: error.code.into(),
        message: error.message.into(),
        hint: error.hint.map(Box::from),
        details: kassiber_pytext::loads(&error.details.to_string()).ok(),
        retryable: error.retryable,
        message_codepoints: None,
        sqlite_origin: false,
    }
}

/// Secret channels are a descriptor or a restoring no-echo controlling TTY.
/// The transport passes flags only; secret values never enter an argument map.
fn passphrase(
    args: &Value,
    field: &str,
    label: &str,
    confirm: bool,
) -> Result<Passphrase, AppError> {
    if let Some(Value::Int(fd)) = get(args, field) {
        let fd = i32::try_from(fd.clone()).map_err(|_| {
            AppError::new(
                "invalid_passphrase",
                "passphrase descriptor must be non-negative",
            )
        })?;
        return read_passphrase_from_fd(fd).map_err(storage_error);
    }
    if get(args, "non_interactive").is_some_and(truthy) || get(args, "machine").is_some_and(truthy)
    {
        return Err(AppError::new(
            "interaction_required",
            "passphrase input is required in non-interactive mode",
        )
        .with_hint(format!(
            "Pass the secret through --{} from a controlling process.",
            field.replace('_', "-")
        )));
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
    let first = Passphrase::new(prompt(label)?).map_err(storage_error)?;
    if confirm && first.expose() != prompt("Confirm passphrase: ")?.as_str() {
        return Err(
            AppError::new("invalid_passphrase", "passphrases do not match")
                .with_hint("Re-run the command and re-enter the passphrase carefully."),
        );
    }
    Ok(first)
}

fn resumable(database: &Path) -> Value {
    let state = encryption::find_resumable_state(database);
    dict([
        ("plaintext", Value::Bool(state.plaintext)),
        ("encrypted_temp", Value::Bool(state.encrypted_temp)),
        ("backup", Value::Bool(state.backup)),
        (
            "plaintext_path",
            s(state.plaintext_path.to_string_lossy().as_ref()),
        ),
        (
            "encrypted_temp_path",
            s(state.encrypted_temp_path.to_string_lossy().as_ref()),
        ),
        (
            "backup_path",
            s(state.backup_path.to_string_lossy().as_ref()),
        ),
    ])
}

fn enforce_new_passphrase(secret: &Passphrase) -> Result<(), AppError> {
    if secret.expose().chars().count() < 8 {
        return Err(AppError::new(
            "invalid_passphrase",
            "passphrase must be at least 8 characters long",
        )
        .with_hint("Pick a long passphrase from a password manager."));
    }
    Ok(())
}

pub fn execute(
    namespace: &OwnerNamespace,
    paths: &RuntimePaths,
    operation: &str,
    args: &Value,
) -> Result<Value, AppError> {
    execute_with_store(namespace, paths, operation, args, &NativeCredentialReader)
}

pub fn execute_with_store(
    namespace: &OwnerNamespace,
    paths: &RuntimePaths,
    operation: &str,
    args: &Value,
    store: &dyn CredentialStore,
) -> Result<Value, AppError> {
    let database = &paths.database;
    let database_text = || s(database.to_string_lossy().as_ref());
    if operation == "secrets.init-resume" {
        return Ok(dict([
            ("database", database_text()),
            ("state", resumable(database)),
            ("hint", s("If `encrypted_temp` is present and trustworthy, you can rename it to the database path manually after verifying with `kassiber secrets verify`.")),
        ]));
    }
    let exists = database.metadata().is_ok_and(|meta| meta.len() != 0);
    let plaintext =
        exists && kassiber_paths::looks_like_plaintext(database).map_err(storage_error)?;
    let db_error = |code, message| {
        AppError::new(code, message).with_details(dict([("database", database_text())]))
    };
    match operation {
        "secrets.status" => {
            let mut result = dict([
                ("path", database_text()),
                ("exists", Value::Bool(exists)),
                ("encrypted", Value::Bool(exists && !plaintext)),
                ("plaintext", Value::Bool(plaintext)),
            ]);
            if exists {
                kassiber_envelope::set(
                    &mut result,
                    "size_bytes",
                    Value::from(
                        database
                            .metadata()
                            .map_err(|e| storage_error(kassiber_paths::Error::io(e)))?
                            .len(),
                    ),
                );
            }
            let findings = credentials::scan(&paths.env_file)?;
            let policy = policy::Policy::load(namespace, &paths.data_root)?;
            for (key, value) in [
                ("resumable", resumable(database)),
                ("sqlcipher_available", Value::Bool(true)),
                ("dotenv_path", s(paths.env_file.to_string_lossy().as_ref())),
                ("dotenv_plaintext_secrets", findings.clone()),
                ("remembered_unlock", policy.status(store)),
                ("operator_unlock_mode", policy.operator_status()?),
            ] {
                kassiber_envelope::set(&mut result, key, value);
            }
            if exists && !plaintext && truthy(&findings) {
                kassiber_envelope::set(&mut result, "dotenv_warning", s("Encrypted database is in use but the bootstrap dotenv still contains plaintext secrets. Run `kassiber secrets migrate-credentials` to lift them into the encrypted backends table and sanitize the file."));
            }
            Ok(result)
        }
        "secrets.change-passphrase" | "secrets.remember-unlock" | "secrets.migrate-credentials" => {
            let rotation = operation == "secrets.change-passphrase";
            if !exists {
                return Err(db_error(
                    "missing_database",
                    if rotation {
                        "database does not exist"
                    } else {
                        "database does not exist; run `kassiber secrets init` first"
                    },
                ));
            }
            if plaintext {
                return Err(db_error("plaintext_database", match operation {
                "secrets.change-passphrase" => "database is plaintext; run `kassiber secrets init` first",
                "secrets.remember-unlock" => "database is plaintext; encrypt it with `kassiber secrets init` first",
                _ => "database is plaintext; encrypt it with `kassiber secrets init` before migrating credentials into it",
            }));
            }
            if operation == "secrets.migrate-credentials" {
                let findings = credentials::scan(&paths.env_file)?;
                if !truthy(&findings) {
                    return Ok(dict([
                        ("dotenv_path", s(paths.env_file.to_string_lossy().as_ref())),
                        ("migrated", Value::List(vec![])),
                        ("skipped", Value::List(vec![])),
                        ("backup_path", Value::Null),
                        ("rewritten", Value::Bool(false)),
                        ("note", s("dotenv has no plaintext secret-shaped entries")),
                    ]));
                }
                if get(args, "dry_run").is_some_and(truthy) {
                    return Ok(dict([
                        ("dotenv_path", s(paths.env_file.to_string_lossy().as_ref())),
                        ("dry_run", Value::Bool(true)),
                        ("would_migrate", findings),
                        ("rewritten", Value::Bool(false)),
                    ]));
                }
                let secret = passphrase(args, "db_passphrase_fd", "Database passphrase: ", false)?;
                let conn = encryption::open_container(database, Some(secret.expose()))
                    .map_err(storage_error)?;
                return credentials::migrate(&conn, &paths.env_file);
            }
            if rotation {
                let current = passphrase(args, "db_passphrase_fd", "Current passphrase: ", false)?;
                let new = passphrase(args, "new_passphrase_fd", "New passphrase: ", true)?;
                rotate(
                    namespace,
                    paths,
                    &current,
                    &new,
                    store,
                    broker_child_owner(),
                )
            } else {
                let secret = passphrase(args, "passphrase_fd", "Database passphrase: ", false)?;
                policy::enroll(namespace, &paths.data_root, &secret, store)
            }
        }
        "secrets.forget-unlock" => {
            let secret = passphrase(
                args,
                "db_passphrase_fd",
                "Fresh database passphrase: ",
                false,
            )?;
            let book = kassiber_store::Book::open_migrating(
                &paths.data_root,
                kassiber_store::OpenOptions {
                    passphrase: Some(secret.expose()),
                    ..Default::default()
                },
                true,
            )
            .map_err(storage_error)?;
            book.close()
                .map_err(|failure| storage_error(failure.error))?;
            policy::forget(namespace, &paths.data_root, store, false)
        }
        "secrets.init" => {
            if exists && !plaintext {
                return Err(
                    db_error("already_encrypted", "database is already encrypted").with_hint(
                        "Use `kassiber secrets change-passphrase` to rotate the passphrase.",
                    ),
                );
            }
            let new = passphrase(args, "new_passphrase_fd", "New database passphrase: ", true)?;
            enforce_new_passphrase(&new)?;
            if plaintext {
                let result = encryption::encrypt_plaintext(
                    namespace,
                    &paths.data_root,
                    new.expose(),
                    broker_child_owner(),
                )
                .map_err(storage_error)?;
                return Ok(dict([
                    ("mode", s("migrated")),
                    (
                        "database",
                        s(result.encrypted_path.to_string_lossy().as_ref()),
                    ),
                    ("backup", s(result.backup_path.to_string_lossy().as_ref())),
                    ("user_version", Value::from(result.plaintext_user_version)),
                    ("auto_vacuum", Value::from(result.plaintext_auto_vacuum)),
                    ("integrity_check", s("ok")),
                    ("cipher_integrity_check", s("ok")),
                    ("credential_marker_clean", Value::Bool(true)),
                ]));
            }
            // An empty/new container still needs both role reservations. No
            // long-lived desktop/broker may create the same file concurrently.
            let project = namespace
                .canonical_project(&paths.data_root)
                .map_err(storage_error)?;
            let _broker = namespace
                .acquire(project.clone(), OwnerKind::Broker, "secrets-init")
                .map_err(storage_error)?;
            let _desktop = namespace
                .acquire(project, OwnerKind::Desktop, "secrets-init")
                .map_err(storage_error)?;
            encryption::create_empty_database(database, new.expose()).map_err(storage_error)?;
            Ok(dict([
                ("mode", s("created")),
                ("database", database_text()),
            ]))
        }
        "secrets.verify" => {
            if !exists {
                return Err(db_error("missing_database", "database does not exist"));
            }
            if plaintext {
                return Err(db_error(
                    "plaintext_database",
                    "database is plaintext; nothing to verify",
                ));
            }
            let secret = passphrase(args, "db_passphrase_fd", "Database passphrase: ", false)?;
            let result =
                encryption::verify_encrypted(database, secret.expose()).map_err(storage_error)?;
            Ok(dict([
                ("database", database_text()),
                ("integrity_check", s(result.integrity_check)),
                (
                    "cipher_integrity_check",
                    result.cipher_integrity_check.map_or(Value::Null, s),
                ),
                ("sqlite_master_rows", Value::from(result.sqlite_master_rows)),
            ]))
        }
        _ => Err(AppError::new(
            "not_implemented",
            "secrets operation is not implemented in the Rust backend yet",
        )),
    }
}

/// `active_owner_kind`: a broker child already holds the broker role by
/// inheritance, so project maintenance reserves only the other one.
fn broker_child_owner() -> Option<OwnerKind> {
    std::env::var_os("KASSIBER_OPERATOR_CHILD")
        .is_some_and(|value| value == "1")
        .then_some(OwnerKind::Broker)
}

pub fn rotate(
    namespace: &OwnerNamespace,
    paths: &RuntimePaths,
    current: &Passphrase,
    new: &Passphrase,
    store: &dyn CredentialStore,
    active_owner: Option<OwnerKind>,
) -> Result<Value, AppError> {
    rotate_observed(
        namespace,
        paths,
        current,
        new,
        store,
        active_owner,
        &mut |_| Ok(()),
    )
}
pub fn rotate_observed(
    namespace: &OwnerNamespace,
    paths: &RuntimePaths,
    current: &Passphrase,
    new: &Passphrase,
    store: &dyn CredentialStore,
    active_owner: Option<OwnerKind>,
    observer: &mut impl FnMut(&str) -> kassiber_paths::Result<()>,
) -> Result<Value, AppError> {
    enforce_new_passphrase(new)?;
    let mut guards = None;
    let observer = std::cell::RefCell::new(observer);
    encryption::rekey_with_observer(
        namespace,
        &paths.data_root,
        current.expose(),
        new.expose(),
        active_owner,
        || {
            guards = Some(policy::invalidate_observed(
                namespace,
                &paths.data_root,
                &mut **observer.borrow_mut(),
            )?);
            Ok(())
        },
        &mut |label| (*observer.borrow_mut())(label),
    )
    .map_err(storage_error)?;
    let (desktop_generation, operator_generation) = guards.expect("authenticated rekey callback");
    let mut result = dict([
        ("database", s(paths.database.to_string_lossy().as_ref())),
        ("integrity_check", s("ok")),
        ("cipher_integrity_check", s("ok")),
        (
            "desktop_biometric_invalidated",
            Value::Bool(desktop_generation.is_some()),
        ),
        (
            "desktop_biometric_stale_generation",
            desktop_generation.map_or(Value::Null, s),
        ),
        ("operator_native_auth_invalidated", Value::Bool(true)),
        (
            "operator_native_auth_stale_generation",
            s(operator_generation),
        ),
    ]);
    let warning =
        policy::refresh_observed(namespace, &paths.data_root, new, store, &mut |label| {
            (*observer.borrow_mut())(label).map_err(storage_error)
        })?;
    if active_owner != Some(OwnerKind::Desktop) {
        kassiber_envelope::set(
            &mut result,
            "remembered_unlock",
            policy::Policy::load(namespace, &paths.data_root)?.status(store),
        );
    }
    if let Some(warning) = warning {
        kassiber_envelope::set(&mut result, "remembered_unlock_warning", warning);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_passphrase_minimum_counts_characters_and_preserves_spaces() {
        for text in ["1234567", "🔑🔑🔑🔑🔑🔑🔑"] {
            let secret = Passphrase::new(Zeroizing::new(text.into())).unwrap();
            assert_eq!(
                enforce_new_passphrase(&secret).unwrap_err().code,
                "invalid_passphrase"
            );
        }
        for text in ["12345678", "🔑🔑🔑🔑🔑🔑🔑🔑", "        "] {
            let secret = Passphrase::new(Zeroizing::new(text.into())).unwrap();
            enforce_new_passphrase(&secret).unwrap();
        }
    }

    #[test]
    fn machine_and_noninteractive_input_never_prompt_or_accept_inline_secrets() {
        for flag in ["machine", "non_interactive"] {
            let args = dict([
                (flag, Value::Bool(true)),
                ("new_passphrase", s("synthetic inline")),
            ]);
            let error = passphrase(&args, "new_passphrase_fd", "unused", true).unwrap_err();
            assert_eq!(error.code, "interaction_required");
            assert_eq!(
                error.hint.as_deref(),
                Some("Pass the secret through --new-passphrase-fd from a controlling process.")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_input_closes_and_bypasses_confirmation_in_machine_mode() {
        use std::io::Write;
        use std::os::fd::{FromRawFd, IntoRawFd};
        let mut descriptors = [-1; 2];
        // SAFETY: the array contains space for both descriptors. Each end is
        // transferred once to its owner; passphrase consumes the read end.
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let read = unsafe { std::fs::File::from_raw_fd(descriptors[0]) };
        let mut write = unsafe { std::fs::File::from_raw_fd(descriptors[1]) };
        write.write_all(" synthetic '🔑 \r\n".as_bytes()).unwrap();
        drop(write);
        let fd = read.into_raw_fd();
        // SAFETY: fstat only reads metadata of the still-open pipe end.
        let identity = |fd| unsafe {
            let mut stat = std::mem::zeroed::<libc::stat>();
            (libc::fstat(fd, &mut stat) == 0).then_some((stat.st_dev, stat.st_ino))
        };
        let pipe = identity(fd).unwrap();
        let args = dict([
            ("machine", Value::Bool(true)),
            ("new_passphrase_fd", Value::from(fd)),
        ]);
        let secret = passphrase(&args, "new_passphrase_fd", "unused", true).unwrap();
        assert_eq!(secret.expose(), " synthetic '🔑 ");
        // A concurrently running test may reuse the closed number, so the
        // pipe is closed when the number is free or names another file.
        assert_ne!(identity(fd), Some(pipe));
    }
}

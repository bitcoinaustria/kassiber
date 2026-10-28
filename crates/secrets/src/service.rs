//! Explicit encryption commands. They bypass ordinary schema bootstrap, just
//! like Python: verification and artifact inspection must never migrate a book.
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
        hint: error.hint.map(str::to_owned),
        details: kassiber_pytext::loads(&error.details.to_string()).ok(),
        retryable: error.retryable,
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
                let result =
                    encryption::encrypt_plaintext(namespace, &paths.data_root, new.expose(), None)
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
        let args = dict([
            ("machine", Value::Bool(true)),
            ("new_passphrase_fd", Value::from(fd)),
        ]);
        let secret = passphrase(&args, "new_passphrase_fd", "unused", true).unwrap();
        assert_eq!(secret.expose(), " synthetic '🔑 ");
        // SAFETY: fcntl validates this now-closed integer descriptor.
        assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
    }
}

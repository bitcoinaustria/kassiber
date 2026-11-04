//! Desktop encryption lifecycle; secret values use the dedicated store seam.
use crate::output::{with_request_id, RequestId};
use crate::{error_envelope, locked_envelope, Daemon, Handled};
use kassiber_envelope::{dict, get, s, success, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Secret;

pub const KINDS: &[&str] = &[
    "ui.secrets.init",
    "ui.secrets.change_passphrase",
    "ui.secrets.forget_cli_unlock",
];

pub(super) fn handle(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request_id: &RequestId,
) -> Handled {
    if kind == "ui.secrets.forget_cli_unlock" {
        let result = daemon.store.forget_cli_unlock(&daemon.data_root)?;
        return Ok((
            Some(with_request_id(success(kind, result), request_id)),
            false,
        ));
    }
    if kind == "ui.secrets.init" {
        return init(daemon, args, request_id);
    }
    let empty = dict::<&str>([]);
    let args = match args {
        None | Some(Value::Null) => &empty,
        Some(value @ Value::Dict(_)) => value,
        Some(value) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(value)))]))
                .into())
        }
    };
    let current = get(args, "auth_response")
        .and_then(|auth| get(auth, "passphrase_secret"))
        .and_then(|v| match v {
            Value::Str(text) if text.code_points().next().is_some() => Some(text),
            _ => None,
        });
    let Some(current) = current else {
        return Ok((
            Some(locked_envelope(
                "change_database_passphrase",
                "Enter the current SQLCipher database passphrase.",
                request_id,
            )),
            false,
        ));
    };
    let new = get(args, "new_passphrase_secret")
        .and_then(|v| match v {
            Value::Str(text) if text.code_points().next().is_some() => Some(text),
            _ => None,
        })
        .ok_or_else(|| {
            AppError::new(
                "validation",
                "ui.secrets.change_passphrase requires a new passphrase",
            )
            .with_hint("Ask the user to enter and confirm a new database passphrase.")
        })?;
    if new.code_points().count() < 12 {
        return Err(AppError::new(
            "invalid_passphrase",
            "database passphrase must be at least 12 characters long",
        )
        .with_hint("Pick a long passphrase from a password manager.")
        .into());
    }
    if !daemon.store.database_is_encrypted(&daemon.data_root)? {
        return Err(AppError::new(
            "plaintext_database",
            "database is plaintext; initialize SQLCipher before changing passphrase",
        )
        .into());
    }
    let denied = || {
        error_envelope(
            "local_auth_denied",
            "passphrase verification failed",
            request_id,
            None,
            None,
            true,
        )
    };
    let Some(current) = current.as_str() else {
        return Ok((Some(denied()), false));
    };
    let current = Secret::new(current.into());
    if !daemon
        .store
        .verify_passphrase(&daemon.data_root, "change_database_passphrase", &current)?
    {
        return Ok((Some(denied()), false));
    }
    let new = new
        .as_str()
        .ok_or_else(|| AppError::new("invalid_passphrase", "passphrase is not valid UTF-8"))?;
    let new = Secret::new(new.into());
    let acquired = daemon.owner.is_none();
    daemon.ensure_owner()?;
    daemon.reap_freshness(true)?;
    // Authentication and owner validation precede connection retirement. A
    // failed close preserves the connection and its lease for a later retry.
    daemon.stop_watch_worker(true)?;
    if let Some(mut book) = daemon.book.take() {
        if let Err(error) = book.close() {
            daemon.book = Some(book);
            return Err(error.into());
        }
    }
    daemon.unlocked_passphrase = None;
    let result = daemon.store.rotate_passphrase(
        &daemon.data_root,
        &daemon.runtime_config.env_file,
        &current,
        &new,
    );
    let result = result.and_then(|result| {
        daemon.open(Some(&new), false)?;
        Ok(result)
    });
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            if acquired && daemon.book.is_none() {
                daemon.release_owner()?;
            }
            return Err(error.into());
        }
    };
    Ok((
        Some(with_request_id(success(kind, result), request_id)),
        false,
    ))
}

/// `ui.secrets.init`: encrypt this project's book with a new passphrase, or
/// unlock it when it is already encrypted. Credentials never reach a log.
fn init(daemon: &mut Daemon, args: Option<&Value>, request_id: &RequestId) -> Handled {
    let args = match args {
        None | Some(Value::Null) => dict::<&str>([]),
        Some(value @ Value::Dict(_)) => value.clone(),
        Some(value) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(value)))]))
                .into())
        }
    };
    let Some(supplied) = crate::projects::passphrase(&args).cloned() else {
        return Ok((
            Some(locked_envelope(
                "init_database_encryption",
                "Choose a SQLCipher database passphrase.",
                request_id,
            )),
            false,
        ));
    };
    let secret = crate::projects::new_passphrase(&supplied)?;
    let database = kassiber_paths::resolve_database_path(std::path::Path::new(&daemon.data_root));
    let result = if daemon.store.database_is_encrypted(&daemon.data_root)? {
        if !daemon.store.verify_passphrase(
            &daemon.data_root,
            "init_database_encryption",
            &secret,
        )? {
            return Ok((
                Some(error_envelope(
                    "local_auth_denied",
                    "passphrase verification failed",
                    request_id,
                    None,
                    None,
                    true,
                )),
                false,
            ));
        }
        daemon.open(Some(&secret), false)?;
        dict([
            ("encrypted", Value::Bool(true)),
            ("already_encrypted", Value::Bool(true)),
            ("database", s(database.to_string_lossy().as_ref())),
        ])
    } else {
        daemon.ensure_owner()?;
        // Exclusive maintenance precedes retiring the open connection.
        let maintenance = daemon.store.restore_maintenance(&daemon.data_root)?;
        daemon.reap_freshness(true)?;
        if let Some(mut book) = daemon.book.take() {
            if let Err(error) = book.close() {
                daemon.book = Some(book);
                return Err(error.into());
            }
            daemon.unlocked_passphrase = None;
        }
        drop(maintenance);
        let mut result = daemon.store.init_encryption(&daemon.data_root, &secret)?;
        daemon.release_owner()?;
        daemon.open(Some(&secret), false)?;
        if get(&args, "migrate_credentials") != Some(&Value::Bool(false)) {
            let env_file = daemon.runtime_config.env_file.clone();
            let credentials = daemon.book_mut()?.service(
                "ui.secrets.migrate_credentials",
                &dict([("env_file", s(env_file.as_str()))]),
            )?;
            kassiber_envelope::set(&mut result, "credentials", credentials);
            daemon.runtime_config = daemon.store.runtime_config_for(&daemon.data_root)?;
        }
        result
    };
    Ok((
        Some(with_request_id(
            success("ui.secrets.init", result),
            request_id,
        )),
        false,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{output::Output, DaemonConfig};
    use kassiber_runtime::{
        fake::{FakeConfig, FakeStore},
        store::{
            Book, BookStore, Bootstrap, OwnerKind, ProjectOwner, ProjectRef, RuntimeConfig,
            Selection, Unlock, UnlockMode,
        },
        BuildInfo,
    };
    use std::sync::Arc;

    struct ReopenFailure(FakeStore);
    impl BookStore for ReopenFailure {
        fn migrate_default_state_root(&self) -> Result<(), AppError> {
            self.0.migrate_default_state_root()
        }
        fn bootstrap(&self, selection: &Selection) -> Result<Bootstrap, AppError> {
            self.0.bootstrap(selection)
        }
        fn effective_unlock_mode(&self, selection: &Selection) -> Result<UnlockMode, AppError> {
            self.0.effective_unlock_mode(selection)
        }
        fn database_is_encrypted(&self, root: &str) -> Result<bool, AppError> {
            self.0.database_is_encrypted(root)
        }
        fn canonical_project(&self, root: &str) -> Result<ProjectRef, AppError> {
            self.0.canonical_project(root)
        }
        fn acquire_ownership(
            &self,
            project: &ProjectRef,
            kind: OwnerKind,
            generation: &str,
        ) -> Result<Box<dyn ProjectOwner>, AppError> {
            self.0.acquire_ownership(project, kind, generation)
        }
        fn open_book(
            &self,
            _: &str,
            _: Unlock<'_>,
            _: &mut RuntimeConfig,
        ) -> Result<Box<dyn Book>, AppError> {
            Err(AppError::new("storage_io_error", "injected reopen failure"))
        }
        fn verify_passphrase(
            &self,
            root: &str,
            scope: &str,
            secret: &Secret,
        ) -> Result<bool, AppError> {
            self.0.verify_passphrase(root, scope, secret)
        }
        fn mark_project_opened(&self, id: &str, root: &str, select: bool) -> Result<(), AppError> {
            self.0.mark_project_opened(id, root, select)
        }
        fn rotate_passphrase(
            &self,
            _: &str,
            _: &str,
            _: &Secret,
            _: &Secret,
        ) -> Result<Value, AppError> {
            Ok(dict::<&str>([]))
        }
    }

    #[test]
    fn failed_rotation_reopen_releases_only_a_newly_acquired_owner() {
        for already_owned in [false, true] {
            let fake = FakeStore::new(FakeConfig {
                encrypted: true,
                passphrase: Some("synthetic current".into()),
                ..Default::default()
            });
            let bootstrap = fake.bootstrap(&Selection::default()).unwrap();
            let mut daemon = Daemon::new(
                Arc::new(ReopenFailure(fake.clone())),
                DaemonConfig {
                    build: BuildInfo {
                        version: "test",
                        channel: None,
                        commit: None,
                    },
                    bootstrap,
                },
                Output::new(Box::new(std::io::sink())),
            );
            if already_owned {
                daemon.ensure_owner().unwrap();
            }
            let args = dict([
                (
                    "auth_response",
                    dict([("passphrase_secret", s("synthetic current"))]),
                ),
                ("new_passphrase_secret", s("synthetic replacement")),
            ]);
            let result = handle(
                &mut daemon,
                "ui.secrets.change_passphrase",
                Some(&args),
                &RequestId::Missing,
            );
            assert!(
                matches!(result, Err(crate::Failure::App(error)) if error.message == "injected reopen failure")
            );
            assert!(daemon.book.is_none());
            assert_eq!(daemon.owner.is_some(), already_owned);
            assert_eq!(
                fake.events().iter().any(|event| event == "release fake"),
                !already_owned
            );
        }
    }
}

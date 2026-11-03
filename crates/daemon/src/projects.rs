//! Desktop project catalog and switching (`ui.projects.*`). The catalog is
//! answered without opening a book; creation and selection replace the
//! daemon's book, ownership and passphrase only after the target is open.
use crate::output::{with_request_id, RequestId};
use crate::{error_envelope, Daemon, Handled, Retired};
use kassiber_envelope::{dict, get, s, set, success, truthy, type_name, AppError};
use kassiber_pyjson::{PyStr, Value};
use kassiber_pytext::{repr_py, strip_py};
use kassiber_runtime::store::{OwnerKind, ProjectCatalog, ProjectOwner, Secret, Unlock};

pub const KINDS: &[&str] = &["ui.projects.list", "ui.projects.create", "ui.projects.select"];

/// `MIN_DATABASE_PASSPHRASE_CHARS`.
const MIN_PASSPHRASE_CHARS: usize = 12;

pub(super) fn handle(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request_id: &RequestId,
) -> Handled {
    let data = match kind {
        "ui.projects.list" => list(daemon)?,
        "ui.projects.create" => create(daemon, &coerce(args)?)?,
        _ => return select(daemon, &coerce(args)?, request_id),
    };
    Ok((Some(with_request_id(success(kind, data), request_id)), false))
}

/// `_coerce_args_dict`.
fn coerce(args: Option<&Value>) -> Result<Value, AppError> {
    match args {
        None | Some(Value::Null) => Ok(dict::<&str>([])),
        Some(args @ Value::Dict(_)) => Ok(args.clone()),
        Some(other) => Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(other)))]))),
    }
}

/// Catalog text must be encodable; Python would store lone surrogates as JSON
/// escapes, which the Rust catalog cannot represent, so they are refused.
fn utf8<'a>(text: &'a PyStr, what: &str) -> Result<&'a str, AppError> {
    text.as_str().ok_or_else(|| {
        AppError::new(
            "validation",
            format!("{what} must not contain unpaired surrogates"),
        )
    })
}

/// `_passphrase_from_auth`: a non-empty `auth_response.passphrase_secret`.
pub(crate) fn passphrase(args: &Value) -> Option<&PyStr> {
    match get(args, "auth_response").and_then(|auth| get(auth, "passphrase_secret")) {
        Some(Value::Str(text)) if text.code_points().next().is_some() => Some(text),
        _ => None,
    }
}

/// `_validate_new_database_passphrase`, then SQLCipher's UTF-8 encoding.
pub(crate) fn new_passphrase(text: &PyStr) -> Result<Secret, AppError> {
    if text.code_points().count() < MIN_PASSPHRASE_CHARS {
        return Err(AppError::new(
            "invalid_passphrase",
            format!(
                "database passphrase must be at least {MIN_PASSPHRASE_CHARS} characters long"
            ),
        )
        .with_hint("Pick a long passphrase from a password manager."));
    }
    let text = text
        .as_str()
        .ok_or_else(|| AppError::new("invalid_passphrase", "passphrase is not valid UTF-8"))?;
    Ok(Secret::new(text.to_owned()))
}

fn field(payload: &Value, key: &str) -> String {
    kassiber_envelope::py_str(get(payload, key).expect("project payload field"))
}

/// `_projects_list_payload(ctx)`: selection is the daemon's, not the catalog's.
fn list(daemon: &mut Daemon) -> Result<Value, AppError> {
    let Value::List(entries) = daemon.store.project_catalog(ProjectCatalog::List)? else {
        unreachable!("project list")
    };
    let projects = entries
        .into_iter()
        .map(|mut entry| {
            let selected = daemon.project_id.as_deref() == Some(field(&entry, "id").as_str());
            set(&mut entry, "selected", Value::Bool(selected));
            entry
        })
        .collect();
    Ok(dict([
        (
            "selected_project_id",
            daemon.project_id.as_deref().map_or(Value::Null, s),
        ),
        ("projects", Value::List(projects)),
    ]))
}

fn create(daemon: &mut Daemon, args: &Value) -> Result<Value, AppError> {
    let name = match get(args, "name") {
        Some(Value::Str(name)) if strip_py(name).code_points().next().is_some() => name,
        _ => {
            return Err(AppError::new(
                "validation",
                "ui.projects.create requires a non-empty name",
            ))
        }
    };
    let project_id = match get(args, "project_id") {
        None | Some(Value::Null) => None,
        Some(Value::Str(id)) => Some(id),
        Some(_) => {
            return Err(AppError::new(
                "validation",
                "ui.projects.create project_id must be a string",
            ))
        }
    };
    let select = get(args, "select") != Some(&Value::Bool(false));
    let secret = passphrase(args).map(new_passphrase).transpose()?;
    let name = utf8(name, "project name")?;
    let project_id = project_id.map(|id| utf8(id, "project id")).transpose()?;
    let mut entry = daemon.store.project_catalog(ProjectCatalog::Create {
        name,
        project_id,
        select,
    })?;
    if let Some(secret) = &secret {
        daemon
            .store
            .create_encrypted_database(&field(&entry, "database"), secret)?;
        entry = daemon.store.project_catalog(ProjectCatalog::Opened {
            project_id: &field(&entry, "id"),
            data_root: &field(&entry, "data_root"),
            stamp: select,
            select,
        })?;
    }
    if select {
        daemon.close_project_for_switch()?;
        daemon.project_id = Some(field(&entry, "id"));
        daemon.project_root = Some(field(&entry, "path"));
        daemon.data_root = field(&entry, "data_root");
        daemon.select_project_on_open = true;
        daemon.runtime_config = daemon.store.runtime_config_for(&daemon.data_root)?;
        daemon.open(secret.as_ref(), false)?;
    }
    let unlocked = daemon.book.is_some() && daemon.project_id == Some(field(&entry, "id"));
    Ok(dict([
        ("project", entry),
        (
            "selected_project_id",
            daemon.project_id.as_deref().map_or(Value::Null, s),
        ),
        ("unlocked", Value::Bool(unlocked)),
    ]))
}

/// `_select_project_payload(ctx, args, request_id)`.
fn select(daemon: &mut Daemon, args: &Value, request_id: &RequestId) -> Handled {
    daemon.retry_retired();
    let reference = match get(args, "project_id").filter(|v| truthy(v)) {
        Some(value) => Some(value),
        None => get(args, "id"),
    };
    let reference = match reference {
        Some(Value::Str(id)) if strip_py(id).code_points().next().is_some() => id,
        _ => {
            return Err(
                AppError::new("validation", "ui.projects.select requires project_id").into(),
            )
        }
    };
    let mut entry = daemon
        .store
        .project_catalog(ProjectCatalog::Get(utf8(reference, "project id")?))?;
    let entry_id = field(&entry, "id");
    let switching = daemon.project_id.as_deref() != Some(entry_id.as_str())
        || std::path::Path::new(&daemon.data_root)
            != std::path::Path::new(&field(&entry, "data_root"));
    if !switching && daemon.book.is_some() {
        let entry = daemon.store.project_catalog(ProjectCatalog::Opened {
            project_id: &entry_id,
            data_root: &daemon.data_root.clone(),
            stamp: true,
            select: true,
        })?;
        return selected(daemon, entry, request_id);
    }
    let supplied = passphrase(args).cloned();
    let target = daemon
        .store
        .canonical_project(&field(&entry, "data_root"))?;
    let transferred = daemon
        .owner
        .as_ref()
        .is_some_and(|owner| owner.project().identity == target.identity);
    let mut target_owner: Option<Box<dyn ProjectOwner>> = if transferred {
        None
    } else {
        Some(daemon.store.acquire_ownership(
            &target,
            OwnerKind::Desktop,
            &daemon.ownership_generation,
        )?)
    };
    let mut target_book = None;
    let outcome = (|| -> Handled {
        // The desktop role lock precedes even a passphrase probe.
        let encrypted = daemon.store.database_is_encrypted(&target.data_root)?;
        let secret = if encrypted {
            let Some(supplied) = &supplied else {
                let record = success(
                    "auth_required",
                    dict([
                        ("scope", s("unlock_project")),
                        (
                            "label",
                            s(format!(
                                "Enter the SQLCipher passphrase for project {}.",
                                repr_py(&PyStr::from(field(&entry, "name").as_str()))
                            )),
                        ),
                        ("project", entry.clone()),
                    ]),
                );
                return Ok((Some(with_request_id(record, request_id)), false));
            };
            let verified = match supplied.as_str() {
                Some(text) => {
                    let root = if switching {
                        target.data_root.clone()
                    } else {
                        daemon.data_root.clone()
                    };
                    daemon.store.verify_passphrase(
                        &root,
                        "unlock_project",
                        &Secret::new(text.to_owned()),
                    )?
                }
                None => false,
            };
            if !verified {
                let record = error_envelope(
                    "local_auth_denied",
                    "passphrase verification failed",
                    request_id,
                    None,
                    None,
                    true,
                );
                return Ok((Some(record), false));
            }
            supplied.as_str().map(|text| Secret::new(text.to_owned()))
        } else {
            None
        };
        let require_existing_schema = get(args, "require_existing_project").is_some_and(truthy);
        if !switching {
            if let Some(owner) = target_owner.take() {
                daemon.owner = Some(owner);
            }
            if let Err(error) = daemon.open(secret.as_ref(), require_existing_schema) {
                if !transferred {
                    target_owner = daemon.owner.take();
                }
                daemon.unlocked_passphrase = None;
                return Err(error.into());
            }
            entry = daemon.store.project_catalog(ProjectCatalog::Get(&entry_id))?;
        } else {
            let mut runtime_config = daemon.store.runtime_config_for(&target.data_root)?;
            target_book = Some(daemon.store.open_book(
                &target.data_root,
                Unlock::Daemon {
                    passphrase: secret.as_ref(),
                    require_existing_schema,
                    expected_project_identity: &target.identity,
                },
                &mut runtime_config,
            )?);
            if let Some(owner) = target_owner.as_mut() {
                owner.refresh_after_open()?;
            }
            daemon.reap_freshness(true)?;
            entry = daemon
                .store
                .project_catalog(ProjectCatalog::Select(&entry_id))?;
            let old_book = daemon.book.take();
            let old_owner = if transferred {
                None
            } else {
                daemon.owner.take()
            };
            daemon.project_id = Some(field(&entry, "id"));
            daemon.project_root = Some(field(&entry, "path"));
            daemon.data_root = target.data_root.clone();
            daemon.select_project_on_open = true;
            daemon.runtime_config = runtime_config;
            if let Some(owner) = target_owner.take() {
                daemon.owner = Some(owner);
            }
            daemon.book = target_book.take();
            daemon.unlocked_passphrase = secret;
            if old_book.is_some() || old_owner.is_some() {
                daemon.retired.push(Retired {
                    book: old_book,
                    owner: old_owner,
                });
                daemon.retry_retired();
            }
        }
        Ok((None, false))
    })();
    // Roll back whatever the target acquired but did not commit; close
    // precedes release, and a failed close keeps both for a later retry.
    let mut cleanup = None;
    if let Some(mut book) = target_book.take() {
        if let Err(error) = book.close() {
            cleanup = Some(error);
            daemon.retired.push(Retired {
                book: Some(book),
                owner: target_owner.take(),
            });
        }
    }
    if let Some(mut owner) = target_owner.take() {
        if let Err(error) = owner.release() {
            daemon.retired.push(Retired {
                book: None,
                owner: Some(owner),
            });
            cleanup.get_or_insert(error);
        }
    }
    match outcome {
        Ok((Some(record), shutdown)) => match cleanup {
            Some(error) => Err(error.into()),
            None => Ok((Some(record), shutdown)),
        },
        Ok((None, _)) => match cleanup {
            Some(error) => Err(error.into()),
            None => selected(daemon, entry, request_id),
        },
        Err(failure) => Err(failure),
    }
}

fn selected(daemon: &mut Daemon, entry: Value, request_id: &RequestId) -> Handled {
    let status = daemon.status_payload()?;
    let data = dict([("project", entry), ("status", status)]);
    Ok((
        Some(with_request_id(success("ui.projects.select", data), request_id)),
        false,
    ))
}

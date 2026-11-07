//! Explicit project unlock modes and their authenticated policy binding
//! (`kassiber/operator/modes.py`, `kassiber/operator/policy.py`).
//!
//! A reusable mode (`brokered`, `unattended`) counts only while the managed
//! settings beside the book name this exact file identity and a database
//! identity read from an authenticated open; otherwise the effective mode is
//! `manual`. Binding to a new database clears every permissive key, so a
//! moved or replaced file never inherits a policy.

use crate::service::storage_error;
use kassiber_envelope::{dict, s, AppError};
use kassiber_ownership::OwnerNamespace;
use kassiber_paths::Context;
use kassiber_pyjson::Value;
use serde_json::{json, Map, Value as Json};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, AppError>;

pub const OPERATOR_UNLOCK_MODE_SETTING: &str = "operator_unlock_mode";
pub const OPERATOR_POLICY_PROJECT_IDENTITY_SETTING: &str = "operator_policy_project_identity";
pub const OPERATOR_POLICY_DATABASE_IDENTITY_SETTING: &str = "operator_policy_database_identity";
/// Sorted, as `details.allowed` lists them.
pub const UNLOCK_MODES: [&str; 3] = ["brokered", "manual", "unattended"];
const PERMISSIVE_POLICY_KEYS: [&str; 3] = [
    "operator_unlock_mode",
    "cli_remembered_unlock",
    "cli_remembered_unlock_enrollment",
];

/// `resolve_canonical_project_data_root` and the settings file beside it.
struct Located {
    root: PathBuf,
    settings: PathBuf,
    identity: String,
}

fn locate(namespace: &OwnerNamespace, data_root: &Path) -> Result<Located> {
    let project = namespace
        .canonical_project(data_root)
        .map_err(storage_error)?;
    let root = project
        .database
        .parent()
        .expect("database parent")
        .to_owned();
    let settings = Context::from_env()
        .and_then(|context| context.state_root_for_data(&root))
        .map_err(storage_error)?
        .join("config/settings.json");
    Ok(Located {
        root,
        settings,
        identity: project.identity,
    })
}

fn load(path: &Path) -> Map<String, Json> {
    // load_managed_settings: an unreadable file is an empty object.
    kassiber_paths::load_settings(path).unwrap_or_default()
}

fn valid_project_identity(value: Option<&Json>) -> bool {
    value
        .and_then(Json::as_str)
        .is_some_and(|text| kassiber_store::valid_hex(text, 64))
}

fn valid_database_identity(value: Option<&Json>) -> bool {
    value
        .and_then(Json::as_str)
        .is_some_and(|text| kassiber_store::valid_hex(text, 32))
}

/// `configured_unlock_mode(data_root)`: the explicit mode, or `None`.
pub fn configured_unlock_mode(
    namespace: &OwnerNamespace,
    data_root: &Path,
) -> Result<Option<String>> {
    kassiber_store::restore::recover_pending(namespace, data_root).map_err(storage_error)?;
    let located = locate(namespace, data_root)?;
    match load(&located.settings).get(OPERATOR_UNLOCK_MODE_SETTING) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(mode)) if UNLOCK_MODES.contains(&mode.as_str()) => Ok(Some(mode.clone())),
        Some(other) => Err(AppError::new(
            "invalid_operator_unlock_mode",
            "the configured operator unlock mode is invalid",
        )
        .with_hint("Choose manual, brokered, or unattended with `kassiber operator mode`.")
        .with_details(dict([(
            "configured",
            s(kassiber_envelope::py_str(&crate::policy::py(other.clone())).as_str()),
        )]))),
    }
}

/// `require_project_policy_binding(data_root)`: the bound database
/// identity, while the binding still names this file.
pub fn require_project_policy_binding(
    namespace: &OwnerNamespace,
    data_root: &Path,
) -> Result<(String, String)> {
    let located = locate(namespace, data_root)?;
    let settings = load(&located.settings);
    let project = settings.get(OPERATOR_POLICY_PROJECT_IDENTITY_SETTING);
    let database = settings.get(OPERATOR_POLICY_DATABASE_IDENTITY_SETTING);
    if !valid_project_identity(project) || !valid_database_identity(database) {
        return Err(AppError::new(
            "operator_policy_binding_required",
            "the project unlock policy is not bound to an authenticated database",
        )
        .with_hint(
            "Authenticate with `kassiber operator mode` or re-enroll remembered unlock before using a reusable unlock policy.",
        ));
    }
    let project = project.and_then(Json::as_str).unwrap_or_default();
    if project != located.identity {
        return Err(AppError::new(
            "operator_policy_binding_mismatch",
            "the project database no longer matches its unlock policy",
        )
        .with_hint(
            "Use manual password authorization, then select the intended operator mode again for this database.",
        ));
    }
    Ok((
        project.to_owned(),
        database
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned(),
    ))
}

/// The managed setting whose rotation retires every operator Touch ID
/// credential (`OPERATOR_NATIVE_AUTH_GENERATION_SETTING`).
pub const OPERATOR_NATIVE_AUTH_GENERATION_SETTING: &str = "operator_native_auth_generation";

/// `operator_touch_id_account(data_root)` (`native_auth.py`): the opaque
/// keychain account of this book's operator Touch ID credential, usable only
/// while the policy stays bound to this file. It hashes the project identity
/// with the native-auth generation, `"initial"` when unset and `"invalid"`
/// when not a string, so neither the path nor the identity leaves.
pub fn operator_touch_id_account(namespace: &OwnerNamespace, data_root: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let located = locate(namespace, data_root)?;
    require_project_policy_binding(namespace, &located.root)?;
    let generation = match load(&located.settings).get(OPERATOR_NATIVE_AUTH_GENERATION_SETTING) {
        None => "initial".to_owned(),
        Some(Json::String(generation)) => generation.clone(),
        Some(_) => "invalid".to_owned(),
    };
    let digest = Sha256::digest(
        format!(
            "kassiber-operator-touch-id-v1:{}:{generation}",
            located.identity
        )
        .as_bytes(),
    );
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// `effective_unlock_mode(data_root)`: the configured mode, `manual` when
/// unset or when a reusable mode lost its binding.
pub fn effective_unlock_mode(namespace: &OwnerNamespace, data_root: &Path) -> Result<String> {
    let Some(configured) = configured_unlock_mode(namespace, data_root)? else {
        return Ok("manual".into());
    };
    if configured != "manual" {
        if let Err(error) = require_project_policy_binding(namespace, data_root) {
            if matches!(
                error.code.as_str(),
                "operator_policy_binding_required" | "operator_policy_binding_mismatch"
            ) {
                return Ok("manual".into());
            }
            return Err(error);
        }
    }
    Ok(configured)
}

/// `project_policy_state(data_root).binding_state`: `missing`, `mismatch`
/// or `valid`, without exposing either identity.
pub fn policy_binding_state(namespace: &OwnerNamespace, data_root: &Path) -> Result<&'static str> {
    let located = locate(namespace, data_root)?;
    let settings = load(&located.settings);
    let project = settings.get(OPERATOR_POLICY_PROJECT_IDENTITY_SETTING);
    let database = settings.get(OPERATOR_POLICY_DATABASE_IDENTITY_SETTING);
    Ok(
        if !valid_project_identity(project) || !valid_database_identity(database) {
            "missing"
        } else if project.and_then(Json::as_str) != Some(located.identity.as_str()) {
            "mismatch"
        } else {
            "valid"
        },
    )
}

/// `unlock_mode_status(data_root)`: what `kassiber operator status` shows
/// as `mode`.
pub fn unlock_mode_status(namespace: &OwnerNamespace, data_root: &Path) -> Result<Value> {
    let configured = configured_unlock_mode(namespace, data_root)?;
    let binding_state = policy_binding_state(namespace, data_root)?;
    let effective = effective_unlock_mode(namespace, data_root)?;
    Ok(dict([
        ("configured", configured.as_deref().map_or(Value::Null, s)),
        ("effective", s(effective.as_str())),
        ("legacy_inferred", Value::Bool(false)),
        ("binding_state", s(binding_state)),
    ]))
}

/// `bind_project_policy(data_root, database_identity, ...)`: bind after an
/// authenticated open, clearing permissive keys when the binding changes.
pub fn bind_project_policy(
    namespace: &OwnerNamespace,
    data_root: &Path,
    database_identity: &str,
    expected_project_identity: Option<&str>,
    updates: &[(&str, Json)],
) -> Result<()> {
    if !kassiber_store::valid_hex(database_identity, 32) {
        return Err(AppError::new(
            "invalid_project_database",
            "the authenticated database identity is invalid",
        ));
    }
    let located = locate(namespace, data_root)?;
    if expected_project_identity.is_some_and(|expected| expected != located.identity) {
        return Err(AppError::new(
            "operator_project_replaced",
            "the project changed before its unlock policy could be bound",
        ));
    }
    kassiber_store::layout::mutate_settings(&located.settings, |payload| {
        let rebinding = payload
            .get(OPERATOR_POLICY_PROJECT_IDENTITY_SETTING)
            .and_then(Json::as_str)
            != Some(located.identity.as_str())
            || payload
                .get(OPERATOR_POLICY_DATABASE_IDENTITY_SETTING)
                .and_then(Json::as_str)
                != Some(database_identity);
        if rebinding {
            for key in PERMISSIVE_POLICY_KEYS {
                payload.remove(key);
            }
        }
        payload.insert(
            OPERATOR_POLICY_PROJECT_IDENTITY_SETTING.into(),
            json!(located.identity),
        );
        payload.insert(
            OPERATOR_POLICY_DATABASE_IDENTITY_SETTING.into(),
            json!(database_identity),
        );
        for (key, value) in updates {
            payload.insert((*key).to_owned(), value.clone());
        }
        Ok(())
    })
    .map_err(storage_error)?;
    if locate(namespace, &located.root)?.identity != located.identity {
        return Err(AppError::new(
            "operator_project_replaced",
            "the project changed while its unlock policy was being bound",
        ));
    }
    Ok(())
}

/// `set_unlock_mode(data_root, mode, database_identity=...,
/// expected_project_identity=...)`. A reusable mode needs an authenticated
/// database identity; `manual` can always be written.
pub fn set_unlock_mode(
    namespace: &OwnerNamespace,
    data_root: &Path,
    mode: &str,
    database_identity: Option<&str>,
    expected_project_identity: Option<&str>,
) -> Result<String> {
    if !UNLOCK_MODES.contains(&mode) {
        return Err(AppError::new(
            "invalid_operator_unlock_mode",
            format!("unsupported operator unlock mode: {mode}"),
        )
        .with_details(dict([(
            "allowed",
            Value::List(UNLOCK_MODES.iter().map(|mode| s(*mode)).collect()),
        )])));
    }
    let located = locate(namespace, data_root)?;
    if let Some(identity) = database_identity {
        bind_project_policy(
            namespace,
            &located.root,
            identity,
            expected_project_identity,
            &[(OPERATOR_UNLOCK_MODE_SETTING, json!(mode))],
        )?;
        return Ok(mode.to_owned());
    }
    if mode != "manual" {
        return Err(AppError::new(
            "operator_policy_binding_required",
            "a reusable unlock mode requires an authenticated database binding",
        ));
    }
    // A restrictive manual policy remains a safe recovery state when old
    // path-adjacent settings cannot be authenticated and rebound yet.
    kassiber_store::layout::mutate_settings(&located.settings, |payload| {
        payload.insert(OPERATOR_UNLOCK_MODE_SETTING.into(), json!("manual"));
        Ok(())
    })
    .map_err(storage_error)?;
    Ok(mode.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATABASE: &str = "11111111111111111111111111111111";
    const ENROLLMENT: &str = "22222222222222222222222222222222";

    struct Book {
        _owners: tempfile::TempDir,
        namespace: OwnerNamespace,
    }

    fn book() -> Book {
        let owners = tempfile::tempdir().unwrap();
        Book {
            namespace: OwnerNamespace::isolated(owners.path().to_owned()),
            _owners: owners,
        }
    }

    impl Book {
        fn effective(&self, root: &Path) -> String {
            effective_unlock_mode(&self.namespace, root).unwrap()
        }
        fn configured(&self, root: &Path) -> Option<String> {
            configured_unlock_mode(&self.namespace, root).unwrap()
        }
        /// `set_cli_remembered_unlock_enabled(root, True, ...)`.
        fn remember(&self, root: &Path) -> Result<()> {
            crate::policy::Policy::load(&self.namespace, root)?.enable(
                &self.namespace,
                DATABASE,
                ENROLLMENT,
            )
        }
        fn binding_state(&self, root: &Path) -> &'static str {
            let policy = crate::policy::Policy::load(&self.namespace, root).unwrap();
            policy.binding_state()
        }
    }

    #[test]
    fn test_new_project_defaults_to_manual() {
        let book = book();
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(book.configured(tmp.path()), None);
        assert_eq!(book.effective(tmp.path()), "manual");
    }

    #[test]
    fn test_complete_remembered_enrollment_is_explicitly_unattended() {
        let book = book();
        let tmp = tempfile::tempdir().unwrap();
        book.remember(tmp.path()).unwrap();
        assert_eq!(book.effective(tmp.path()), "unattended");
        assert_eq!(book.configured(tmp.path()).as_deref(), Some("unattended"));
    }

    #[test]
    fn test_explicit_brokered_mode_never_uses_remembered_store() {
        let book = book();
        let tmp = tempfile::tempdir().unwrap();
        book.remember(tmp.path()).unwrap();
        set_unlock_mode(
            &book.namespace,
            tmp.path(),
            "brokered",
            Some(DATABASE),
            None,
        )
        .unwrap();
        assert_eq!(book.configured(tmp.path()).as_deref(), Some("brokered"));
        assert_eq!(book.effective(tmp.path()), "brokered");
    }

    #[test]
    fn test_explicit_manual_mode_overrides_legacy_marker() {
        let book = book();
        let tmp = tempfile::tempdir().unwrap();
        book.remember(tmp.path()).unwrap();
        set_unlock_mode(&book.namespace, tmp.path(), "manual", None, None).unwrap();
        assert_eq!(book.effective(tmp.path()), "manual");
    }

    #[cfg(unix)]
    #[test]
    fn test_mode_and_remembered_state_share_canonical_project_alias() {
        let book = book();
        let parent = tempfile::tempdir().unwrap();
        let project = parent.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let alias = parent.path().join("alias");
        std::os::unix::fs::symlink(&project, &alias).unwrap();
        book.remember(&alias).unwrap();
        assert_eq!(book.effective(&project), "unattended");
        set_unlock_mode(&book.namespace, &project, "brokered", Some(DATABASE), None).unwrap();
        assert_eq!(book.configured(&alias).as_deref(), Some("brokered"));
        set_unlock_mode(&book.namespace, &alias, "manual", None, None).unwrap();
        assert_eq!(book.configured(&project).as_deref(), Some("manual"));
    }

    #[test]
    fn test_hardlink_aliases_cannot_select_divergent_unlock_policies() {
        let book = book();
        let parent = tempfile::tempdir().unwrap();
        let first = parent.path().join("first");
        let second = parent.path().join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        std::fs::write(first.join("kassiber.sqlite3"), b"database").unwrap();
        std::fs::hard_link(
            first.join("kassiber.sqlite3"),
            second.join("kassiber.sqlite3"),
        )
        .unwrap();
        for (root, mode) in [(&first, "brokered"), (&second, "unattended")] {
            let error =
                set_unlock_mode(&book.namespace, root, mode, Some(DATABASE), None).unwrap_err();
            assert_eq!(error.code, "unsafe_project_database");
            assert_eq!(
                error.details,
                Some(dict([("link_count", Value::Int(2.into()))]))
            );
            assert_eq!(
                book.remember(root).unwrap_err().code,
                "unsafe_project_database"
            );
        }
    }

    #[test]
    fn test_moved_database_cannot_inherit_destination_unattended_policy() {
        let book = book();
        let parent = tempfile::tempdir().unwrap();
        let source = parent.path().join("source");
        let destination = parent.path().join("destination");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(source.join("kassiber.sqlite3"), b"source").unwrap();
        std::fs::write(destination.join("kassiber.sqlite3"), b"destination").unwrap();
        let source_identity = book.namespace.canonical_project(&source).unwrap().identity;
        set_unlock_mode(
            &book.namespace,
            &source,
            "brokered",
            Some(&"a".repeat(32)),
            Some(&source_identity),
        )
        .unwrap();
        book.remember(&destination).unwrap();
        std::fs::remove_file(destination.join("kassiber.sqlite3")).unwrap();
        std::fs::rename(
            source.join("kassiber.sqlite3"),
            destination.join("kassiber.sqlite3"),
        )
        .unwrap();
        assert_eq!(book.configured(&destination).as_deref(), Some("unattended"));
        assert_eq!(book.effective(&destination), "manual");
        assert_eq!(book.binding_state(&destination), "mismatch");
    }

    #[test]
    fn test_bindings_refuse_invalid_identities_and_replaced_projects() {
        let book = book();
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            set_unlock_mode(&book.namespace, tmp.path(), "brokered", Some("xyz"), None)
                .unwrap_err()
                .code,
            "invalid_project_database"
        );
        assert_eq!(
            set_unlock_mode(
                &book.namespace,
                tmp.path(),
                "brokered",
                Some(DATABASE),
                Some(&"f".repeat(64))
            )
            .unwrap_err()
            .code,
            "operator_project_replaced"
        );
        let error =
            set_unlock_mode(&book.namespace, tmp.path(), "brokered", None, None).unwrap_err();
        assert_eq!(error.code, "operator_policy_binding_required");
        let error = set_unlock_mode(&book.namespace, tmp.path(), "auto", None, None).unwrap_err();
        assert_eq!(error.code, "invalid_operator_unlock_mode");
        assert_eq!(error.message, "unsupported operator unlock mode: auto");
        assert_eq!(
            error.details,
            Some(dict([(
                "allowed",
                Value::List(vec![s("brokered"), s("manual"), s("unattended")])
            )]))
        );
    }

    #[test]
    fn test_rebinding_to_another_database_clears_permissive_keys() {
        let book = book();
        let tmp = tempfile::tempdir().unwrap();
        book.remember(tmp.path()).unwrap();
        bind_project_policy(&book.namespace, tmp.path(), &"b".repeat(32), None, &[]).unwrap();
        let settings = load(&locate(&book.namespace, tmp.path()).unwrap().settings);
        for key in PERMISSIVE_POLICY_KEYS {
            assert!(!settings.contains_key(key), "{key}");
        }
        assert_eq!(book.effective(tmp.path()), "manual");
        // Binding the same database again keeps the mode.
        set_unlock_mode(
            &book.namespace,
            tmp.path(),
            "brokered",
            Some(&"b".repeat(32)),
            None,
        )
        .unwrap();
        bind_project_policy(&book.namespace, tmp.path(), &"b".repeat(32), None, &[]).unwrap();
        assert_eq!(book.effective(tmp.path()), "brokered");
    }

    #[test]
    fn test_an_invalid_configured_mode_is_a_typed_error() {
        let book = book();
        let tmp = tempfile::tempdir().unwrap();
        let settings = locate(&book.namespace, tmp.path()).unwrap().settings;
        kassiber_store::layout::mutate_settings(&settings, |payload| {
            payload.insert(OPERATOR_UNLOCK_MODE_SETTING.into(), json!("auto"));
            Ok(())
        })
        .unwrap();
        let error = effective_unlock_mode(&book.namespace, tmp.path()).unwrap_err();
        assert_eq!(error.code, "invalid_operator_unlock_mode");
        assert_eq!(error.details, Some(dict([("configured", s("auto"))])));
    }
}

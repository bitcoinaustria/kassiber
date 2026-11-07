use crate::{
    policy::{self, Policy},
    service, CredentialReader, CredentialStore, Error, ErrorKind, Passphrase, Result,
    CLI_PASSPHRASE_SERVICE,
};
use kassiber_envelope::{dict, get, s};
use kassiber_ownership::OwnerNamespace;
use kassiber_paths::{Context, ResolveOptions};
use kassiber_pyjson::Value;
use kassiber_store::{Book, OpenOptions};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};

const KEY: &str = "Synthetic old 🔑";
const NEW: &str = "Synthetic new 🔐";
#[test]
fn credential_account_matches_the_python_v2_digest() {
    assert_eq!(
        crate::remembered_unlock_account(&"a".repeat(32), &"b".repeat(32)).unwrap(),
        "79febfb6165f61c1c885f4716cdaabe16aaae9b1240e220f0f404dbf46a3ab19"
    );
    assert!(crate::remembered_unlock_account(&"A".repeat(32), &"b".repeat(32)).is_err());
}
fn secret(text: &str) -> Passphrase {
    crate::read_passphrase(text.as_bytes()).unwrap()
}
#[derive(Default)]
struct MemoryStore {
    items: RefCell<BTreeMap<(String, String), String>>,
    reads: Cell<usize>,
    writes: Cell<usize>,
    deletes: Cell<usize>,
    reject_write: Cell<bool>,
    reject_legacy_delete: Cell<bool>,
}
impl CredentialReader for MemoryStore {
    fn get_password(&self, service: &str, account: &str) -> Result<Option<Passphrase>> {
        self.reads.set(self.reads.get() + 1);
        Ok(self
            .items
            .borrow()
            .get(&(service.into(), account.into()))
            .map(|v| secret(v)))
    }
}
impl CredentialStore for MemoryStore {
    fn available(&self) -> bool {
        true
    }
    fn set_password(&self, service: &str, account: &str, value: &Passphrase) -> Result<()> {
        self.writes.set(self.writes.get() + 1);
        if self.reject_write.get() {
            return Err(Error::new(
                ErrorKind::SecretStoreUnavailable,
                "synthetic rejection",
            ));
        }
        self.items
            .borrow_mut()
            .insert((service.into(), account.into()), value.expose().into());
        Ok(())
    }
    fn delete_password(&self, service: &str, account: &str) -> Result<()> {
        self.deletes.set(self.deletes.get() + 1);
        if self.reject_legacy_delete.get() && account.contains(std::path::MAIN_SEPARATOR) {
            return Err(Error::new(
                ErrorKind::SecretStoreUnavailable,
                "synthetic rejection",
            ));
        }
        self.items
            .borrow_mut()
            .remove(&(service.into(), account.into()));
        Ok(())
    }
}
fn fixture() -> (
    tempfile::TempDir,
    OwnerNamespace,
    kassiber_paths::RuntimePaths,
) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("book/data");
    drop(Book::create(&root, Some(KEY)).unwrap());
    let namespace = OwnerNamespace::isolated(tmp.path().join("owners"));
    let paths = Context::from_env()
        .unwrap()
        .resolve_for_bootstrap(&ResolveOptions {
            data_root: Some(root),
            ..Default::default()
        })
        .unwrap();
    (tmp, namespace, paths)
}

#[test]
fn enrollment_identity_and_cleanup_are_python_compatible() {
    let (_tmp, ns, paths) = fixture();
    let store = MemoryStore::default();
    let old = Policy::load(&ns, &paths.data_root).unwrap();
    let legacy = old.root.to_str().unwrap();
    for service in [CLI_PASSPHRASE_SERVICE, policy::LEGACY_SERVICE] {
        store.set_password(service, legacy, &secret(KEY)).unwrap();
    }
    let result = policy::enroll(&ns, &paths.data_root, &secret(KEY), &store).unwrap();
    assert_eq!(
        get(get(&result, "remembered_unlock").unwrap(), "configured"),
        Some(&Value::Bool(true))
    );
    let policy = Policy::load(&ns, &paths.data_root).unwrap();
    assert_eq!(policy.binding_state(), "valid");
    assert_eq!(policy.settings["operator_unlock_mode"], "unattended");
    let db = Book::open(
        &paths.data_root,
        OpenOptions {
            passphrase: Some(KEY),
            ..Default::default()
        },
    )
    .unwrap();
    let enrollment = policy.settings["cli_remembered_unlock_enrollment"]
        .as_str()
        .unwrap();
    let account = crate::remembered_unlock_account(db.instance_id(), enrollment).unwrap();
    assert_eq!(store.items.borrow().len(), 1);
    assert_eq!(
        store
            .items
            .borrow()
            .get(&(CLI_PASSPHRASE_SERVICE.into(), account)),
        Some(&KEY.into())
    );
    assert!(!std::fs::read_to_string(&paths.settings_file)
        .unwrap()
        .contains(KEY));
    policy::forget(&ns, &paths.data_root, &store, false).unwrap();
    assert!(store.items.borrow().is_empty());
    let disabled = Policy::load(&ns, &paths.data_root).unwrap();
    assert!(!disabled.enabled());
    assert_eq!(disabled.settings["operator_unlock_mode"], "manual");
    assert!(!disabled
        .settings
        .contains_key("cli_remembered_unlock_enrollment"));
}

#[test]
fn wrong_enrollment_key_never_writes_or_deletes_credentials() {
    let (_tmp, ns, paths) = fixture();
    let store = MemoryStore::default();
    assert_eq!(
        policy::enroll(&ns, &paths.data_root, &secret("wrong key"), &store)
            .unwrap_err()
            .code,
        "unlock_failed"
    );
    assert_eq!(
        (store.writes.get(), store.deletes.get(), store.reads.get()),
        (0, 0, 0)
    );
    assert!(!paths.settings_file.exists());
}

#[test]
fn status_never_reads_unbound_quarantined_or_unenrolled_items() {
    let (_tmp, ns, paths) = fixture();
    let store = MemoryStore::default();
    let policy = Policy::load(&ns, &paths.data_root).unwrap();
    policy.status(&store);
    assert_eq!(store.reads.get(), 0);
    policy::enroll(&ns, &paths.data_root, &secret(KEY), &store).unwrap();
    store.reads.set(0);
    Policy::load(&ns, &paths.data_root)
        .unwrap()
        .disable(true)
        .unwrap();
    let policy = Policy::load(&ns, &paths.data_root).unwrap();
    policy.status(&store);
    assert_eq!(store.reads.get(), 0);
    assert!(policy.quarantined());
    kassiber_store::layout::mutate_settings(&paths.settings_file, |settings| {
        settings.insert("cli_remembered_unlock".into(), serde_json::json!(true));
        settings.remove("cli_legacy_unlock_quarantined");
        settings.insert(
            "operator_policy_project_identity".into(),
            serde_json::json!("0".repeat(64)),
        );
        Ok(())
    })
    .unwrap();
    let policy = Policy::load(&ns, &paths.data_root).unwrap();
    assert_eq!(policy.binding_state(), "mismatch");
    policy.status(&store);
    assert_eq!(store.reads.get(), 0);
}

#[test]
fn failed_legacy_cleanup_rolls_back_new_enrollment_and_quarantines_revocation() {
    let (_tmp, ns, paths) = fixture();
    let store = MemoryStore::default();
    store.reject_legacy_delete.set(true);
    assert_eq!(
        policy::enroll(&ns, &paths.data_root, &secret(KEY), &store)
            .unwrap_err()
            .code,
        "remembered_unlock_legacy_cleanup_failed"
    );
    assert!(store.items.borrow().is_empty());
    assert!(!Policy::load(&ns, &paths.data_root).unwrap().enabled());
    store.reject_legacy_delete.set(false);
    policy::enroll(&ns, &paths.data_root, &secret(KEY), &store).unwrap();
    store.reject_legacy_delete.set(true);
    assert_eq!(
        policy::forget(&ns, &paths.data_root, &store, true)
            .unwrap_err()
            .code,
        "remembered_unlock_legacy_cleanup_failed"
    );
    let policy = Policy::load(&ns, &paths.data_root).unwrap();
    assert!(!policy.enabled());
    assert!(policy.quarantined());
    assert_eq!(policy.settings["operator_unlock_mode"], "manual");
}

#[test]
fn rotation_authenticates_before_guards_and_refreshes_only_the_bound_item() {
    let (_tmp, ns, paths) = fixture();
    let store = MemoryStore::default();
    policy::enroll(&ns, &paths.data_root, &secret(KEY), &store).unwrap();
    let before = std::fs::read(&paths.settings_file).unwrap();
    let writes = store.writes.get();
    assert_eq!(
        service::rotate(
            &ns,
            &paths,
            &secret("wrong current"),
            &secret(NEW),
            &store,
            None,
        )
        .unwrap_err()
        .code,
        "unlock_failed"
    );
    assert_eq!(std::fs::read(&paths.settings_file).unwrap(), before);
    assert_eq!(store.writes.get(), writes);
    let result = service::rotate(&ns, &paths, &secret(KEY), &secret(NEW), &store, None).unwrap();
    assert_eq!(
        get(&result, "operator_native_auth_invalidated"),
        Some(&Value::Bool(true))
    );
    let policy = Policy::load(&ns, &paths.data_root).unwrap();
    assert_eq!(
        Some(&policy::py(
            policy.settings["operator_native_auth_generation"].clone()
        )),
        get(&result, "operator_native_auth_stale_generation")
    );
    assert_eq!(
        store.items.borrow().values().collect::<Vec<_>>(),
        vec![&NEW.to_owned()]
    );
    assert!(Book::open(
        &paths.data_root,
        OpenOptions {
            passphrase: Some(KEY),
            ..Default::default()
        }
    )
    .is_err());
    assert!(Book::open(
        &paths.data_root,
        OpenOptions {
            passphrase: Some(NEW),
            ..Default::default()
        }
    )
    .is_ok());
}

#[test]
fn failed_rotation_refresh_disables_policy_and_keeps_the_successful_new_key() {
    let (_tmp, ns, paths) = fixture();
    let store = MemoryStore::default();
    policy::enroll(&ns, &paths.data_root, &secret(KEY), &store).unwrap();
    store.reject_write.set(true);
    store.reject_legacy_delete.set(true);
    let result = service::rotate(&ns, &paths, &secret(KEY), &secret(NEW), &store, None).unwrap();
    let warning = get(&result, "remembered_unlock_warning").unwrap();
    assert_eq!(
        get(warning, "code"),
        Some(&s("remembered_unlock_update_failed"))
    );
    let policy = Policy::load(&ns, &paths.data_root).unwrap();
    assert!(!policy.enabled());
    assert!(policy.quarantined());
    assert!(Book::open(
        &paths.data_root,
        OpenOptions {
            passphrase: Some(NEW),
            ..Default::default()
        }
    )
    .is_ok());
}

#[cfg(unix)]
#[test]
fn forget_requires_fresh_authentication_before_any_native_or_policy_effect() {
    use std::os::fd::IntoRawFd;
    let (_tmp, ns, paths) = fixture();
    let store = MemoryStore::default();
    policy::enroll(&ns, &paths.data_root, &secret(KEY), &store).unwrap();
    let before = std::fs::read(&paths.settings_file).unwrap();
    let deletes = store.deletes.get();
    let mut input = tempfile::tempfile().unwrap();
    use std::io::{Seek, Write};
    input.write_all(b"wrong key").unwrap();
    input.rewind().unwrap();
    let args = dict([
        ("machine", Value::Bool(true)),
        ("db_passphrase_fd", Value::from(input.into_raw_fd())),
    ]);
    assert_eq!(
        service::execute_with_store(&ns, &paths, "secrets.forget-unlock", &args, &store)
            .unwrap_err()
            .code,
        "unlock_failed"
    );
    assert_eq!(store.deletes.get(), deletes);
    assert_eq!(std::fs::read(&paths.settings_file).unwrap(), before);
}

/// `test_rotation_generation_changes_opaque_native_account`: the operator
/// Touch ID account needs the binding and changes with the generation.
#[test]
fn operator_touch_id_account_follows_the_binding_and_the_generation() {
    use crate::modes::{bind_project_policy, operator_touch_id_account};
    use sha2::{Digest, Sha256};
    let (tmp, ns, paths) = fixture();
    let root = &paths.data_root;
    assert_eq!(
        operator_touch_id_account(&ns, root).unwrap_err().code,
        "operator_policy_binding_required"
    );
    bind_project_policy(&ns, root, &"d".repeat(32), None, &[]).unwrap();
    let identity = ns.canonical_project(root).unwrap().identity;
    let expected = |generation: &str| -> String {
        Sha256::digest(format!("kassiber-operator-touch-id-v1:{identity}:{generation}").as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    };
    let before = operator_touch_id_account(&ns, root).unwrap();
    assert_eq!(before, expected("initial"));
    assert_eq!(before.len(), 64);
    assert!(!before.contains(tmp.path().to_str().unwrap()));
    for (stored, generation) in [
        (serde_json::json!("ab".repeat(16)), "ab".repeat(16)),
        (serde_json::json!(7), "invalid".to_owned()),
        (serde_json::Value::Null, "invalid".to_owned()),
    ] {
        kassiber_store::layout::mutate_settings(&paths.settings_file, |settings| {
            settings.insert("operator_native_auth_generation".into(), stored.clone());
            Ok(())
        })
        .unwrap();
        let after = operator_touch_id_account(&ns, root).unwrap();
        assert_eq!(after, expected(&generation));
        assert_ne!(after, before);
    }
}

use super::*;
use kassiber_ownership::{OwnerKind, OwnerLease, OwnerNamespace};

struct Target {
    _root: tempfile::TempDir,
    data: PathBuf,
    conn: Option<Box<Connection>>,
    namespace: OwnerNamespace,
    owner: Option<OwnerLease>,
    late_write: bool,
    fail_close: bool,
    fail_install: bool,
    fail_invalidate: bool,
    invalidated: bool,
}
impl Drop for Target {
    fn drop(&mut self) {
        // Release inode reservations before TempDir removes their last paths.
        self.conn = None;
        self.owner = None;
    }
}
impl Target {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("book/data");
        fs::create_dir_all(&data).unwrap();
        let conn = Connection::open(data.join(DATABASE)).unwrap();
        conn.execute_batch("CREATE TABLE workspaces(label TEXT); CREATE TABLE profiles(label TEXT); INSERT INTO workspaces VALUES('Target set'); INSERT INTO profiles VALUES('Target book');").unwrap();
        Self {
            namespace: OwnerNamespace::isolated(root.path().join("owners")),
            _root: root,
            data,
            conn: Some(Box::new(conn)),
            owner: None,
            late_write: false,
            fail_close: false,
            fail_install: false,
            fail_invalidate: false,
            invalidated: false,
        }
    }
    fn preview(&self) -> Sessions {
        let workspace = tempfile::Builder::new()
            .prefix("kassiber-restore-test-")
            .tempdir()
            .unwrap();
        let stage = workspace.path().join("stage");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join(DATABASE), b"incoming database").unwrap();
        fs::create_dir(stage.join("attachments")).unwrap();
        fs::write(stage.join("attachments/proof"), b"evidence").unwrap();
        let imported = Import {
            workspace,
            manifest: dict::<&str>([]),
            unavailable: vec![s("os-ref")],
        };
        Sessions {
            preview: Some(Preview {
                token: "capability".into(),
                target: self.fingerprint().unwrap(),
                digest: digest_tree(&stage).unwrap(),
                imported,
                expires: Instant::now() + Duration::from_secs(600),
                incoming: dict::<&str>([]),
                unavailable: true,
            }),
        }
    }
    fn apply(&mut self, sessions: &mut Sessions) -> Result<Value> {
        sessions.handle(
            self,
            "ui.backup.apply",
            &dict([("confirm", s("RESTORE")), ("token", s("capability"))]),
        )
    }
}
impl Host for Target {
    fn data_root(&self) -> &Path {
        &self.data
    }
    fn unlocked(&self) -> bool {
        self.conn.is_some()
    }
    fn fingerprint(&self) -> Result<Fingerprint> {
        fingerprint(self.conn.as_ref().unwrap(), &self.data, Some("target"))
    }
    fn summary(&self) -> Result<Value> {
        summary(self.conn.as_ref().unwrap())
    }
    fn export(&mut self, output: &Path, _: &str) -> Result<Value> {
        Ok(text(output))
    }
    fn ensure_owner(&mut self) -> Result<()> {
        if self.owner.is_none() {
            self.owner = Some(
                self.namespace
                    .acquire(
                        self.namespace.canonical_project(&self.data).unwrap(),
                        OwnerKind::Desktop,
                        "test",
                    )
                    .map_err(service::restore_error)?,
            );
        }
        Ok(())
    }
    fn maintenance(&self) -> Result<Box<dyn Send>> {
        Ok(Box::new(
            kassiber_store::restore::maintenance(
                &self.namespace,
                &self.data,
                Some(OwnerKind::Desktop),
            )
            .map_err(service::restore_error)?,
        ))
    }
    fn close_database(&mut self, expected: &Fingerprint) -> Result<()> {
        assert!(self.owner.is_some());
        assert_eq!(
            self.namespace
                .acquire(
                    self.namespace.canonical_project(&self.data).unwrap(),
                    OwnerKind::Broker,
                    "late"
                )
                .err()
                .unwrap()
                .code,
            "project_in_use"
        );
        if self.late_write {
            self.conn
                .as_ref()
                .unwrap()
                .execute("UPDATE profiles SET label='Late write'", [])
                .unwrap();
        }
        check_close(
            self.conn.as_ref().unwrap(),
            &self.data,
            Some("target"),
            expected,
        )?;
        if self.fail_close {
            return Err(AppError::new("project_in_use", "injected close failure"));
        }
        self.conn.take().unwrap().close().unwrap();
        if self.fail_install {
            fs::create_dir(self.data.parent().unwrap().join("exports")).unwrap();
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(
                    "/missing",
                    self.data.parent().unwrap().join("exports/link"),
                )
                .unwrap();
            }
        }
        Ok(())
    }
    fn invalidate_credentials(&mut self) -> Result<()> {
        self.invalidated = true;
        if self.fail_invalidate {
            return Err(io_error("injected invalidation failure"));
        }
        invalidate_credentials(&self.namespace, &self.data)
    }
    fn release_owner(&mut self) -> Result<()> {
        self.owner = None;
        Ok(())
    }
}
fn empty() -> Value {
    dict::<&str>([])
}
fn code(result: Result<Value>, expected: &str) {
    assert_eq!(result.unwrap_err().code, expected);
}

#[test]
fn cancel_capabilities_expiry_and_cleanup_match_reference() {
    let mut host = Target::new();
    let mut sessions = host.preview();
    let stage = sessions.preview.as_ref().unwrap().imported.staging();
    for token in [Value::Null, s("wrong"), Value::from(1)] {
        assert_eq!(
            sessions
                .handle(&mut host, "ui.backup.cancel", &dict([("token", token)]))
                .unwrap(),
            dict([("cancelled", Value::Bool(true))])
        );
        assert!(stage.exists());
    }
    host.conn = None;
    sessions
        .handle(
            &mut host,
            "ui.backup.cancel",
            &dict([("token", s("capability"))]),
        )
        .unwrap();
    assert!(!stage.exists());
    assert!(sessions.preview.is_none());
    let host = Target::new();
    let mut sessions = host.preview();
    let workspace = sessions
        .preview
        .as_ref()
        .unwrap()
        .imported
        .workspace
        .path()
        .to_owned();
    assert!(sessions.remaining().unwrap() <= Duration::from_secs(600));
    sessions.preview.as_mut().unwrap().expires = Instant::now();
    sessions.expire();
    assert!(sessions.preview.is_none());
    assert!(!workspace.exists());
    let sessions = host.preview();
    let workspace = sessions
        .preview
        .as_ref()
        .unwrap()
        .imported
        .workspace
        .path()
        .to_owned();
    drop(sessions);
    assert!(!workspace.exists());
}

#[test]
fn apply_confirmation_tokens_target_and_stage_guards_match_reference() {
    let mut host = Target::new();
    let mut sessions = host.preview();
    code(
        sessions.handle(
            &mut host,
            "ui.backup.apply",
            &dict([("token", s("capability"))]),
        ),
        "backup_confirmation_required",
    );
    assert!(sessions.preview.is_some());
    code(
        sessions.handle(
            &mut host,
            "ui.backup.apply",
            &dict([("confirm", s("RESTORE")), ("token", s("wrong"))]),
        ),
        "stale_backup_preview",
    );
    assert!(sessions.preview.is_none());
    code(host.apply(&mut sessions), "stale_backup_preview");
    for change in ["database", "sidecar", "staging", "renamed"] {
        let mut host = Target::new();
        let mut sessions = host.preview();
        let stage = sessions.preview.as_ref().unwrap().imported.staging();
        match change {
            "database" => {
                host.conn
                    .as_ref()
                    .unwrap()
                    .execute("UPDATE profiles SET label='new'", [])
                    .unwrap();
            }
            "sidecar" => {
                fs::create_dir(host.data.parent().unwrap().join("attachments")).unwrap();
            }
            "staging" => {
                fs::write(stage.join("attachments/proof"), b"tampered").unwrap();
            }
            _ => {
                fs::rename(
                    stage.join("attachments/proof"),
                    stage.join("attachments/other"),
                )
                .unwrap();
            }
        }
        let error = host.apply(&mut sessions).unwrap_err();
        assert_eq!(error.code, "stale_backup_preview");
        if ["database", "sidecar"].contains(&change) {
            assert_eq!(error, stale());
        }
        assert!(host.owner.is_none());
        assert!(sessions.preview.is_none());
        assert!(!stage.exists());
        assert!(host.conn.is_some());
    }
    #[cfg(unix)]
    {
        let mut host = Target::new();
        let mut sessions = host.preview();
        let stage = sessions.preview.as_ref().unwrap().imported.staging();
        std::os::unix::fs::symlink("proof", stage.join("attachments/link")).unwrap();
        let error = host.apply(&mut sessions).unwrap_err();
        assert_eq!(error.message, "Restore staging changed");
        // Python leaves the preview intact when digesting itself raises.
        assert!(sessions.preview.is_some());
    }
}

#[test]
fn apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference() {
    for mode in ["owner", "late", "close", "install", "warning", "success"] {
        let mut host = Target::new();
        let mut sessions = host.preview();
        let stage = sessions.preview.as_ref().unwrap().imported.staging();
        let held = if mode == "owner" {
            Some(
                host.namespace
                    .acquire(
                        host.namespace.canonical_project(&host.data).unwrap(),
                        OwnerKind::Broker,
                        "other",
                    )
                    .unwrap(),
            )
        } else {
            None
        };
        host.late_write = mode == "late";
        host.fail_close = mode == "close";
        host.fail_install = mode == "install";
        host.fail_invalidate = mode == "warning";
        let result = host.apply(&mut sessions);
        match mode {
            "owner" => {
                code(result, "project_in_use");
                assert!(sessions.preview.is_some());
                assert!(stage.exists());
                assert!(host.conn.is_some());
            }
            "late" | "close" => {
                let e = result.unwrap_err();
                assert_eq!(
                    e.code,
                    if mode == "late" {
                        "stale_backup_preview"
                    } else {
                        "project_in_use"
                    }
                );
                assert!(e.details.is_none());
                assert!(sessions.preview.is_none());
                assert!(!stage.exists());
                assert!(host.conn.is_some());
            }
            "install" => {
                #[cfg(unix)]
                {
                    let e = result.unwrap_err();
                    assert_eq!(
                        get(e.details.as_ref().unwrap(), "locked"),
                        Some(&Value::Bool(true))
                    );
                    assert!(host.conn.is_none());
                    assert!(host.owner.is_none());
                    assert!(!stage.exists());
                }
            }
            _ => {
                let data = result.unwrap();
                assert_eq!(get(&data, "restored"), Some(&Value::Bool(true)));
                assert_eq!(get(&data, "locked"), Some(&Value::Bool(true)));
                assert_eq!(get(&data, "unavailable_secrets"), Some(&Value::Bool(true)));
                assert_eq!(
                    get(&data, "warning"),
                    Some(&if mode == "warning" {
                        s("restore_unlock_settings_failed")
                    } else {
                        Value::Null
                    })
                );
                assert!(host.invalidated);
                assert!(host.conn.is_none());
                assert!(host.owner.is_none());
                assert!(!stage.exists());
                assert_eq!(
                    fs::read(host.data.join(DATABASE)).unwrap(),
                    b"incoming database"
                );
                let recovery = PathBuf::from(py_str(get(&data, "pre_restore_backup").unwrap()));
                assert!(recovery.join(DATABASE).exists());
            }
        }
        drop(held);
    }
}

#[test]
fn fingerprint_pins_connection_project_database_sidecars_and_sqlite_generations() {
    let host = Target::new();
    let conn = host.conn.as_ref().unwrap();
    let before = host.fingerprint().unwrap();
    assert_eq!(before.root, fs::canonicalize(&host.data).unwrap());
    assert_eq!(before.project, Some("target".into()));
    assert_eq!(
        before.connection,
        conn.as_ref() as *const Connection as usize
    );
    assert_eq!(before.changes, conn.total_changes());
    assert_eq!(
        before.version,
        conn.query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
            .unwrap()
    );
    assert_ne!(
        fingerprint(conn, &host.data, Some("other")).unwrap(),
        before
    );
    conn.execute_batch("BEGIN; UPDATE profiles SET label='ephemeral'; ROLLBACK;")
        .unwrap();
    let after = host.fingerprint().unwrap();
    assert!(after.changes > before.changes);
    assert_ne!(after, before);
    let before = after;
    let external = Connection::open(host.data.join(DATABASE)).unwrap();
    external
        .execute("UPDATE profiles SET label='external'", [])
        .unwrap();
    let after = host.fingerprint().unwrap();
    assert!(after.version > before.version);
    let attachments = host.data.parent().unwrap().join("attachments");
    fs::create_dir(&attachments).unwrap();
    fs::write(attachments.join("proof"), b"a").unwrap();
    let before = host.fingerprint().unwrap();
    assert_eq!(before.files.len(), 3);
    let entry = before
        .files
        .iter()
        .find(|(p, _)| p.ends_with("proof"))
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::symlink_metadata(&entry.0).unwrap();
        assert_eq!(
            entry.1,
            vec![
                m.dev() as u128,
                m.ino() as u128,
                m.len() as u128,
                m.mtime() as u128,
                m.mtime_nsec() as u128
            ]
        );
    }

    fs::write(attachments.join("proof"), b"changed").unwrap();
    assert_ne!(host.fingerprint().unwrap(), before);
}

#[test]
fn restore_credentials_revoke_remembered_unlock_without_native_access() {
    let host = Target::new();
    let settings = host.data.parent().unwrap().join("config/settings.json");
    kassiber_store::layout::mutate_settings(&settings, |settings| {
        settings.insert("cli_remembered_unlock".into(), true.into());
        settings.insert("operator_unlock_mode".into(), "unattended".into());
        settings.insert(
            "cli_remembered_unlock_enrollment".into(),
            "synthetic".into(),
        );
        settings.insert("operator_native_auth_generation".into(), "old".into());
        Ok(())
    })
    .unwrap();
    invalidate_credentials(&host.namespace, &host.data).unwrap();
    let settings = kassiber_paths::load_settings(&settings).unwrap();
    assert_eq!(settings.get("operator_unlock_mode"), Some(&"manual".into()));
    assert_eq!(
        settings.get("cli_legacy_unlock_quarantined"),
        Some(&true.into())
    );
    assert!(!settings.contains_key("cli_remembered_unlock"));
    assert!(!settings.contains_key("cli_remembered_unlock_enrollment"));
    assert_ne!(
        settings.get("operator_native_auth_generation"),
        Some(&"old".into())
    );
}

#[test]
fn export_paths_plaintext_and_secret_refusals_match_reference() {
    let mut host = Target::new();
    let mut sessions = Sessions::default();
    for path in [
        Value::Null,
        Value::from(1),
        s("relative.kassiber"),
        s("/tmp/file.tar"),
        text(&host.data.parent().unwrap().join("archive.kassiber")),
    ] {
        code(
            sessions.handle(&mut host, "ui.backup.export", &dict([("path", path)])),
            "invalid_backup_destination",
        );
    }
    let paths = kassiber_paths::Context::from_env()
        .unwrap()
        .resolve_for_bootstrap(&kassiber_paths::ResolveOptions {
            data_root: Some(host.data.clone()),
            ..Default::default()
        })
        .unwrap();
    code(
        export(
            &paths,
            &host._root.path().join("out.kassiber"),
            None,
            "",
            "test",
        ),
        "plaintext_database",
    );
    code(
        export(
            &paths,
            &host._root.path().join("out.kassiber"),
            Some("synthetic"),
            "",
            "test",
        ),
        "backup_passphrase_required",
    );
    for kind in ["ui.backup.export", "ui.backup.preview", "ui.backup.apply"] {
        host.conn = None;
        code(
            sessions.handle(&mut host, kind, &empty()),
            "backup_target_locked",
        );
    }
}

fn bundle(root: &Path, encrypted: bool, corrupt: bool) -> PathBuf {
    let db = root.join(if encrypted {
        if corrupt {
            "source-corrupt.db"
        } else {
            "source-encrypted.db"
        }
    } else {
        "source-plain.db"
    });
    if encrypted {
        kassiber_store::encryption::create_empty_database(&db, "inner-secret").unwrap();
    }
    let conn = if encrypted {
        kassiber_store::encryption::open_container(&db, Some("inner-secret")).unwrap()
    } else {
        Connection::open(&db).unwrap()
    };
    conn.execute_batch("CREATE TABLE workspaces(label TEXT); CREATE TABLE profiles(label TEXT); CREATE TABLE unused(label TEXT);").unwrap();
    for n in (0..120).rev() {
        conn.execute("INSERT INTO profiles VALUES(?)", [format!("Book {n:03}")])
            .unwrap();
        conn.execute("INSERT INTO workspaces VALUES(?)", [format!("Set {n:03}")])
            .unwrap();
    }
    if corrupt {
        conn.execute_batch("PRAGMA writable_schema=ON; UPDATE sqlite_master SET rootpage=(SELECT rootpage FROM sqlite_master WHERE name='profiles') WHERE name='unused';").unwrap();
    }
    conn.close().unwrap();
    let tarball = root.join("bundle.tar");
    let mut tar = tar::Builder::new(File::create(&tarball).unwrap());
    archive::append_file(&mut tar, &db, DATABASE).unwrap();
    archive::append_bytes(&mut tar, "attachments/proof", b"count-only evidence").unwrap();
    archive::append_bytes(&mut tar,"manifest.json",br#"{"schema_version":1,"entries":{"database":"kassiber.sqlite3","attachments_files":1},"secret_refs":{"ai_provider_refs":[{"provider_name":"test","store_id":"native"}]}}"#).unwrap();
    tar.finish().unwrap();
    drop(tar);
    let output = root.join(if encrypted {
        if corrupt {
            "corrupt.kassiber"
        } else {
            "encrypted.kassiber"
        }
    } else {
        "plain.kassiber"
    });
    let mut file = File::create(&output).unwrap();
    crate::encryption::encrypt(
        &crate::encryption::backend(true).unwrap(),
        &tarball,
        &mut file,
        Some("outer-secret"),
        None,
    )
    .unwrap();
    output
}
fn preview_args(archive: &Path) -> Value {
    dict([
        ("path", text(archive)),
        ("backup_passphrase_secret", s("outer-secret")),
        ("database_passphrase_secret", s("inner-secret")),
    ])
}

#[test]
fn preview_validates_encryption_integrity_summaries_counts_and_redaction() {
    let mut host = Target::new();
    let mut sessions = host.preview();
    let root = tempfile::tempdir().unwrap();
    let archive = bundle(root.path(), true, false);
    let data = sessions
        .handle(&mut host, "ui.backup.preview", &preview_args(&archive))
        .unwrap();
    assert_eq!(
        get(&data, "target"),
        Some(&dict([
            ("workspaces", Value::List(vec![s("Target set")])),
            ("books", Value::List(vec![s("Target book")])),
            ("book_count", Value::from(1))
        ]))
    );
    let incoming = get(&data, "incoming").unwrap();
    assert_eq!(get(incoming, "book_count"), Some(&Value::from(120)));
    assert_eq!(
        get(incoming, "books"),
        Some(&Value::List(
            (0..100).map(|i| s(format!("Book {i:03}"))).collect()
        ))
    );
    assert_eq!(
        get(incoming, "workspaces"),
        Some(&Value::List(
            (0..100).map(|i| s(format!("Set {i:03}"))).collect()
        ))
    );
    assert_eq!(get(&data, "attachments_files"), Some(&Value::from(1)));
    assert_eq!(get(&data, "unavailable_secrets"), Some(&Value::Bool(true)));
    assert_eq!(get(&data, "expires_in_seconds"), Some(&Value::from(600)));
    assert_eq!(
        get(&data, "replaces"),
        Some(&Value::List(vec![s(DATABASE)]))
    );
    assert_eq!(get(&data, "target_data_root"), Some(&text(&host.data)));
    assert!(sessions.remaining().unwrap() > Duration::from_secs(599));
    assert_ne!(super::token().unwrap(), super::token().unwrap());
    let token = py_str(get(&data, "token").unwrap());
    assert_eq!(token.len(), 43);
    assert!(token
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'));
    let wire = dumps(&data, &Options::default()).unwrap();
    assert!(!wire.contains("outer-secret"));
    assert!(!wire.contains("inner-secret"));
    assert!(!wire.contains("hash"));
    assert!(!wire.contains("stage"));
    let old_stage = sessions.preview.as_ref().unwrap().imported.staging();
    code(
        sessions.handle(&mut host, "ui.backup.preview", &empty()),
        "missing_backup",
    );
    assert!(!old_stage.exists());
    assert!(sessions.preview.is_none());
    for (archive, expected) in [
        (bundle(root.path(), false, false), "invalid_backup"),
        (bundle(root.path(), true, true), "invalid_backup"),
    ] {
        code(
            sessions.handle(&mut host, "ui.backup.preview", &preview_args(&archive)),
            expected,
        );
        assert!(sessions.preview.is_none());
    }
}

#[test]
fn preview_secret_path_and_validation_error_contracts_match_reference() {
    let mut host = Target::new();
    let mut sessions = Sessions::default();
    let root = tempfile::tempdir().unwrap();
    let archive = bundle(root.path(), true, false);
    for (key, value, expected) in [
        ("path", s("relative"), "missing_backup"),
        ("path", Value::from(1), "missing_backup"),
        (
            "backup_passphrase_secret",
            Value::Null,
            "backup_passphrase_required",
        ),
        (
            "backup_passphrase_secret",
            s(""),
            "backup_passphrase_required",
        ),
        (
            "database_passphrase_secret",
            s(""),
            "backup_passphrase_required",
        ),
        ("database_passphrase_secret", s("wrong"), "invalid_backup"),
        ("backup_passphrase_secret", s("wrong"), "invalid_backup"),
    ] {
        let mut args = preview_args(&archive);
        kassiber_envelope::set(&mut args, key, value);
        let e = sessions
            .handle(&mut host, "ui.backup.preview", &args)
            .unwrap_err();
        assert_eq!(e.code, expected);
        if expected == "invalid_backup" {
            assert_eq!(e, invalid_backup());
        }
        assert!(sessions.preview.is_none());
    }
    for key in ["backup_passphrase_secret", "database_passphrase_secret"] {
        let mut args = preview_args(&archive);
        kassiber_envelope::set(
            &mut args,
            key,
            Value::Str(kassiber_pyjson::PyStr::from_code_points(vec![0xd800]).unwrap()),
        );
        assert_eq!(
            sessions
                .handle(&mut host, "ui.backup.preview", &args)
                .unwrap_err(),
            invalid_backup()
        );
    }
    let args = preview_args(&root.path().join("missing.kassiber"));
    code(
        sessions.handle(&mut host, "ui.backup.preview", &args),
        "missing_backup",
    );
    fs::write(&archive, b"not an age archive").unwrap();
    let e = sessions
        .handle(&mut host, "ui.backup.preview", &preview_args(&archive))
        .unwrap_err();
    assert_eq!(e, invalid_backup());
}

#[test]
fn close_refuses_busy_wal_readers_before_retiring_connection() {
    let host = Target::new();
    let conn = host.conn.as_ref().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    conn.busy_timeout(Duration::ZERO).unwrap();
    let reader = Connection::open(host.data.join(DATABASE)).unwrap();
    reader
        .execute_batch("BEGIN; SELECT * FROM profiles;")
        .unwrap();
    conn.execute("UPDATE profiles SET label='WAL generation'", [])
        .unwrap();
    let fingerprint = host.fingerprint().unwrap();
    let e = check_close(conn, &host.data, Some("target"), &fingerprint).unwrap_err();
    assert_eq!(e.code, "project_in_use");
    assert_eq!(e.message, "Close other database readers before restoring");
    assert_eq!(
        conn.query_row("SELECT label FROM profiles", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "WAL generation"
    );
}

#[test]
fn incoming_authentication_is_quiet_and_ignores_active_operator_binding() {
    const CHILD: &str = "KASSIBER_BACKUP_QUIET_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("incoming.db");
        kassiber_store::encryption::create_empty_database(&db, "inner-secret").unwrap();
        assert!(kassiber_store::encryption::open_backup_container(&db, "wrong").is_err());
        kassiber_store::encryption::open_backup_container(&db, "inner-secret")
            .unwrap()
            .close()
            .unwrap();
        return;
    }
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "desktop::tests::incoming_authentication_is_quiet_and_ignores_active_operator_binding",
        ])
        .env(CHILD, "1")
        .env("KASSIBER_OPERATOR_CHILD", "1")
        .env("KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY", "unrelated")
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stdout)
    );
    assert!(
        child.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
}

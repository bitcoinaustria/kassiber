//! The Python backup format: a SQLCipher hot copy and count-checked sidecars
//! in a tar archive inside an age envelope. No provider or Python dispatch.
pub mod archive;
pub mod desktop;
mod encryption;
pub mod service;

use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_paths::RuntimePaths;
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::Connection;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const DATABASE: &str = "kassiber.sqlite3";
pub const SECRET_REF_HINT: &str =
    "Open Settings -> AI providers and re-enter or repair the provider API key.";
pub type Result<T> = std::result::Result<T, AppError>;

pub(crate) fn io_error(error: impl std::fmt::Display) -> AppError {
    AppError::new("internal_error", error.to_string())
}
pub(crate) fn sql_error(error: kassiber_store::rusqlite::Error) -> AppError {
    kassiber_secrets::service::storage_error(kassiber_store::map_sqlite_error(error))
}
pub(crate) fn text(path: &Path) -> Value {
    s(path.to_string_lossy().as_ref())
}

pub struct Export {
    pub output: PathBuf,
    pub manifest: Value,
    pub age_backend: &'static str,
}

/// `Some(passphrase)` and `Some(recipients)` are mutually exclusive even when
/// the recipient list is empty, matching the Python call contract.
pub fn export(
    paths: &RuntimePaths,
    output: &Path,
    db_passphrase: Option<&str>,
    passphrase: Option<&str>,
    recipients: Option<&[String]>,
    version: &str,
) -> Result<Export> {
    if passphrase.is_some() == recipients.is_some() {
        return Err(AppError::new(
            "invalid_backup_call",
            "export_backup requires exactly one of `backup_passphrase` or `recipients`",
        ));
    }
    let backend = encryption::backend(passphrase.is_some())?;
    if !paths.database.exists() {
        return Err(AppError::new(
            "missing_database",
            format!("database not found at {}", paths.database.display()),
        ));
    }
    fs::create_dir_all(
        output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )
    .map_err(io_error)?;
    if kassiber_paths::looks_like_plaintext(&paths.database)
        .map_err(kassiber_secrets::service::storage_error)?
    {
        return Err(AppError::new(
            "plaintext_database",
            format!(
                "refusing to back up plaintext database at {}",
                paths.database.display()
            ),
        )
        .with_hint("Run `kassiber secrets init` first."));
    }
    let pass = db_passphrase.ok_or_else(|| {
        AppError::new(
            "passphrase_required",
            "database is encrypted; supply a passphrase via --db-passphrase-fd",
        )
    })?;
    let stage = tempfile::Builder::new()
        .prefix("kassiber-backup-")
        .tempdir()
        .map_err(io_error)?;
    let copied = stage.path().join(DATABASE);
    // The store owns SQLCipher settings and the bounded online-backup loop.
    kassiber_store::encryption::backup_encrypted(&paths.database, &copied, pass)
        .map_err(kassiber_secrets::service::storage_error)?;
    let conn = kassiber_store::encryption::open_container(&copied, Some(pass))
        .map_err(kassiber_secrets::service::storage_error)?;
    let refs = collect_secret_refs(&conn)?;
    let created: String = conn
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')", [], |r| {
            r.get(0)
        })
        .map_err(sql_error)?;
    conn.close().map_err(|(_, e)| sql_error(e))?;
    let tarball = stage.path().join("bundle.tar");
    let mut tar = tar::Builder::new(File::create(&tarball).map_err(io_error)?);
    archive::append_file(&mut tar, &copied, DATABASE)?;
    let count = archive::append_tree(&mut tar, &paths.attachments_root, "attachments")?;
    // Backup paths are project-local even when bootstrap used --env-file.
    let env = paths.config_root.join("backends.env");
    let settings = &paths.settings_file;
    for (source, name) in [
        (&env, "config/backends.env"),
        (settings, "config/settings.json"),
    ] {
        if source.exists() {
            archive::append_file(&mut tar, source, name)?;
        }
    }
    let manifest = manifest(
        paths,
        created,
        count,
        env.exists(),
        settings.exists(),
        refs,
        version,
    )?;
    let bytes = dumps(
        &manifest,
        &Options {
            indent: Some(2.into()),
            sort_keys: true,
            ..Default::default()
        },
    )
    .map_err(io_error)?
        + "\n";
    archive::append_bytes(&mut tar, "manifest.json", bytes.as_bytes())?;
    archive::finish(tar)?;
    let mut pending = tempfile::Builder::new()
        .prefix(&format!(
            ".{}.",
            output.file_name().unwrap_or_default().to_string_lossy()
        ))
        .suffix(".tmp")
        .tempfile_in(
            output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
        .map_err(io_error)?;
    encryption::encrypt(
        &backend,
        &tarball,
        pending.as_file_mut(),
        passphrase,
        recipients,
    )?;
    publish(pending, output)?;
    Ok(Export {
        output: output.to_owned(),
        manifest,
        age_backend: backend.flavor(),
    })
}

/// Flush the complete archive, rename it into place and make the rename
/// durable, as Python's `backup.pack` does.
fn publish(mut pending: tempfile::NamedTempFile, output: &Path) -> Result<()> {
    pending.flush().map_err(io_error)?;
    pending.as_file().sync_all().map_err(io_error)?;
    pending.persist(output).map_err(|e| io_error(e.error))?;
    sync_parent_directory(output)
}

/// Make the rename that published `path` durable (`F_FULLFSYNC` on macOS
/// through `sync_all`), as Python's `backup.pack` does. A no-op on Windows.
fn sync_parent_directory(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    #[cfg(test)]
    tests::SYNCED_DIRECTORIES.with(|synced| synced.borrow_mut().push(parent.to_owned()));
    #[cfg(not(windows))]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error)?;
    Ok(())
}

fn manifest(
    paths: &RuntimePaths,
    created: String,
    count: usize,
    env: bool,
    settings: bool,
    refs: Vec<Value>,
    version: &str,
) -> Result<Value> {
    let context =
        kassiber_paths::Context::from_env().map_err(kassiber_secrets::service::storage_error)?;
    let catalog_path = context
        .default_state_root()
        .map_err(kassiber_secrets::service::storage_error)?
        .join("config/projects.json");
    let catalog = kassiber_paths::load_catalog(&catalog_path, &context)
        .map_err(kassiber_secrets::service::storage_error)?;
    let canonical_data = kassiber_paths::canonicalize_allow_missing(&paths.data_root)
        .map_err(kassiber_secrets::service::storage_error)?;
    let project = catalog
        .projects
        .into_iter()
        .find(|p| {
            kassiber_paths::canonicalize_allow_missing(&p.path.join("data"))
                .ok()
                .as_ref()
                == Some(&canonical_data)
        })
        .map(|p| {
            dict([
                ("id", s(p.id)),
                ("name", s(p.name)),
                ("path", text(&p.path)),
                // Python refreshes catalog encryption metadata from disk.
                ("encrypted", Value::Bool(true)),
                ("last_opened_at", p.last_opened_at.map_or(Value::Null, s)),
            ])
        })
        .unwrap_or(Value::Null);
    Ok(dict([
        ("schema_version", Value::from(1)),
        ("kassiber_version", s(version)),
        ("created_at", s(created)),
        ("project", project),
        (
            "paths",
            dict([
                ("state_root", text(&paths.state_root)),
                ("data_root", text(&paths.data_root)),
                ("attachments_root", text(&paths.attachments_root)),
                (
                    "backends_env",
                    text(&paths.config_root.join("backends.env")),
                ),
                ("settings_json", text(&paths.settings_file)),
                ("exports_root", text(&paths.exports_root)),
            ]),
        ),
        (
            "entries",
            dict([
                ("database", s(DATABASE)),
                ("attachments_files", Value::from(count)),
                ("backends_env", Value::Bool(env)),
                ("settings_json", Value::Bool(settings)),
            ]),
        ),
        (
            "notes",
            dict([
                ("inner_db_encrypted", Value::Bool(true)),
                ("inner_db_passphrase_required", Value::Bool(true)),
                ("scope", s("single_project_container")),
                (
                    "plaintext_sidecars",
                    Value::List(vec![
                        s("attachments"),
                        s("config/backends.env"),
                        s("config/settings.json"),
                    ]),
                ),
                ("exports_included", Value::Bool(false)),
                ("logs_included", Value::Bool(false)),
            ]),
        ),
        (
            "secret_refs",
            dict([("ai_provider_refs", Value::List(refs))]),
        ),
    ]))
}

fn collect_secret_refs(conn: &Connection) -> Result<Vec<Value>> {
    let exists: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='ai_provider_secret_refs'", [], |r| r.get(0)).map_err(sql_error)?;
    if exists == 0 {
        return Ok(vec![]);
    }
    let mut query = conn.prepare("SELECT r.provider_name,r.store_id,r.service,r.account,r.state,p.api_key IS NOT NULL AND p.api_key != '' FROM ai_provider_secret_refs r LEFT JOIN ai_providers p ON p.name=r.provider_name WHERE r.store_id != 'sqlcipher_inline' ORDER BY r.provider_name").map_err(sql_error)?;
    let rows = query
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })
        .map_err(sql_error)?;
    rows.map(|r| {
        let (provider, store, service, account, state, inline) = r.map_err(sql_error)?;
        if inline {
            return Err(AppError::new(
                "secret_ref_inline_secret",
                "refusing to export an OS-backed AI provider ref that still has an inline API key",
            )
            .with_hint("Repair the provider secret state before exporting the backup.")
            .with_details(dict([
                ("provider_name", s(provider)),
                ("store_id", s(store)),
            ])));
        }
        Ok(dict([
            ("provider_name", s(provider)),
            ("store_id", s(store)),
            ("service", service.map_or(Value::Null, s)),
            ("account", account.map_or(Value::Null, s)),
            ("state", s(state)),
        ]))
    })
    .collect()
}

/// RAII cleanup until the caller deliberately transfers a staging-only result.
pub struct Import {
    pub workspace: tempfile::TempDir,
    pub manifest: Value,
    pub unavailable: Vec<Value>,
}
impl Import {
    pub fn staging(&self) -> PathBuf {
        self.workspace.path().join("stage")
    }
}

pub fn import(
    archive: &Path,
    passphrase: Option<&str>,
    identity_file: Option<&Path>,
) -> Result<Import> {
    import_observed(archive, passphrase, identity_file, &mut |_| Ok(()))
}

#[cfg(feature = "test-hooks")]
pub fn import_with_observer(
    archive: &Path,
    passphrase: Option<&str>,
    identity_file: Option<&Path>,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Import> {
    import_observed(archive, passphrase, identity_file, observer)
}

fn import_observed(
    archive: &Path,
    passphrase: Option<&str>,
    identity_file: Option<&Path>,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Import> {
    if passphrase.is_some() == identity_file.is_some() {
        return Err(AppError::new(
            "invalid_backup_call",
            "import_backup requires exactly one of `backup_passphrase` or `identity_file`",
        ));
    }
    let backend = encryption::backend(passphrase.is_some())?;
    if !archive.exists() {
        return Err(AppError::new(
            "missing_backup",
            format!("backup file not found at {}", archive.display()),
        ));
    }
    let workspace = tempfile::Builder::new()
        .prefix("kassiber-restore-")
        .tempdir()
        .map_err(io_error)?;
    observer("import.workspace")?;
    let decrypted = workspace.path().join("bundle.tar");
    let mut dst = File::create(&decrypted).map_err(io_error)?;
    encryption::decrypt(&backend, archive, &mut dst, passphrase, identity_file)?;
    drop(dst);
    observer("import.decrypted")?;
    let staging = workspace.path().join("stage");
    archive::extract(&decrypted, &staging)?;
    observer("import.extracted")?;
    fs::remove_file(decrypted).map_err(io_error)?;
    let manifest = validate(&staging)?;
    let unavailable = unavailable_refs(&manifest);
    observer("import.validated")?;
    Ok(Import {
        workspace,
        manifest,
        unavailable,
    })
}

pub fn validate(staging: &Path) -> Result<Value> {
    let fail = |message: String| AppError::new("invalid_backup", message);
    let file = staging.join("manifest.json");
    if !file.exists() {
        return Err(fail("backup is missing manifest.json".into()));
    }
    let manifest =
        kassiber_pytext::loads(&fs::read_to_string(file).map_err(io_error)?).map_err(io_error)?;
    let schema = get(&manifest, "schema_version").unwrap_or(&Value::Null);
    if !matches!(schema, Value::Bool(true))
        && schema != &Value::from(1)
        && schema != &Value::Float(1.0)
    {
        return Err(fail(format!(
            "unsupported backup manifest schema_version {}",
            kassiber_envelope::py_repr(schema)
        )));
    }
    let entries = get(&manifest, "entries").unwrap_or(&Value::Null);
    let database = get(entries, "database").unwrap_or(&Value::Null);
    if database != &s(DATABASE) {
        return Err(fail(format!(
            "manifest declares unexpected database path {}",
            kassiber_envelope::py_repr(database)
        )));
    }
    if !staging.join(DATABASE).exists() {
        return Err(fail(
            "decrypted backup is missing the SQLCipher database".into(),
        ));
    }
    for (key, name) in [
        ("backends_env", "config/backends.env"),
        ("settings_json", "config/settings.json"),
    ] {
        if get(entries, key).is_some_and(truthy) && !staging.join(name).exists() {
            return Err(fail(format!(
                "manifest declares {key} but the file is missing from the archive"
            )));
        }
    }
    let count = get(entries, "attachments_files").unwrap_or(&Value::Null);
    if !matches!(count, Value::Int(n) if n >= &0.into()) {
        return Err(fail(
            "manifest declares an invalid attachments_files count".into(),
        ));
    }
    let actual = archive::regular_count(&staging.join("attachments"))?;
    if count != &Value::from(actual) {
        return Err(fail(
            "manifest attachments_files count does not match the restored archive".into(),
        )
        .with_details(dict([
            ("declared", count.clone()),
            ("actual", Value::from(actual)),
        ])));
    }
    Ok(manifest)
}

fn unavailable_refs(manifest: &Value) -> Vec<Value> {
    let Some(Value::List(refs)) =
        get(manifest, "secret_refs").and_then(|v| get(v, "ai_provider_refs"))
    else {
        return vec![];
    };
    refs.iter()
        .filter_map(|r| {
            if !matches!(r, Value::Dict(_)) {
                return None;
            }
            let string = |key| {
                get(r, key)
                    .filter(|v| truthy(v))
                    .map(py_str)
                    .unwrap_or_default()
            };
            let store = string("store_id");
            if store.is_empty() || store == "sqlcipher_inline" {
                return None;
            }
            let provider = string("provider_name");
            let account = string("account");
            Some(dict([
                ("provider_name", s(provider.clone())),
                ("store_id", s(store)),
                ("service", s(string("service"))),
                (
                    "account",
                    s(if account.is_empty() {
                        provider
                    } else {
                        account
                    }),
                ),
                ("state", s("unavailable")),
            ]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Directories `sync_parent_directory` flushed on this test thread.
        pub(super) static SYNCED_DIRECTORIES: std::cell::RefCell<Vec<PathBuf>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    #[test]
    fn published_archives_sync_their_directory_after_the_rename() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("out/backup.kassiber");
        fs::create_dir(root.path().join("out")).unwrap();
        let mut pending = tempfile::Builder::new()
            .prefix(".backup.kassiber.")
            .tempfile_in(root.path().join("out"))
            .unwrap();
        pending.write_all(b"age-encryption.org/v1").unwrap();
        SYNCED_DIRECTORIES.with(|synced| synced.borrow_mut().clear());
        publish(pending, &output).unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"age-encryption.org/v1");
        assert_eq!(
            SYNCED_DIRECTORIES.with(|synced| synced.take()),
            vec![root.path().join("out")]
        );
    }

    #[test]
    fn provider_refs_never_materialize_external_secrets_or_hide_inline_conflicts() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(collect_secret_refs(&conn).unwrap().is_empty());
        conn.execute_batch("CREATE TABLE ai_provider_secret_refs(provider_name,store_id,service,account,state); CREATE TABLE ai_providers(name,api_key); INSERT INTO ai_provider_secret_refs VALUES('inline','sqlcipher_inline',NULL,NULL,'available'),('os','native','service','account','available'); INSERT INTO ai_providers VALUES('inline','synthetic inline secret'),('os','');").unwrap();
        let refs = collect_secret_refs(&conn).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(get(&refs[0], "provider_name"), Some(&s("os")));
        let m = dict([(
            "secret_refs",
            dict([("ai_provider_refs", Value::List(refs))]),
        )]);
        let unavailable = unavailable_refs(&m);
        assert_eq!(
            unavailable,
            vec![dict([
                ("provider_name", s("os")),
                ("store_id", s("native")),
                ("service", s("service")),
                ("account", s("account")),
                ("state", s("unavailable"))
            ])]
        );
        conn.execute(
            "UPDATE ai_providers SET api_key='synthetic conflict' WHERE name='os'",
            [],
        )
        .unwrap();
        let failure = collect_secret_refs(&conn).unwrap_err();
        assert_eq!(failure.code, "secret_ref_inline_secret");
        assert_eq!(
            failure.details,
            Some(dict([
                ("provider_name", s("os")),
                ("store_id", s("native"))
            ]))
        );
        assert!(!format!("{failure:?}").contains("synthetic conflict"));
    }
}

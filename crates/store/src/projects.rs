//! Explicit R3 legacy-container migration. No workspace or network splitting.
//! Read-only resolution and admission-only Book::open never call this module.
use crate::{durable, layout, map_sqlite_error, Error, ErrorKind, Result};
use kassiber_paths::{durable::rename_noreplace, Catalog, Context, ProjectEntry, DB_FILENAME};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const MARKER: &str = "legacy-project-migration.json";
const BACKUP: &str = "pre-project-migration-";
const DIRECTORIES: [&str; 4] = ["data", "config", "exports", "attachments"];

// The project-copy protocol predates role leases. Respect an existing local
// compatibility reservation without changing the reference's published tree.
// This is not a replacement for the remaining full concurrent-copy matrix.
fn owner_guard(data: &Path) -> Result<Option<fs::File>> {
    let path = data.join(".operator-owner.lock");
    #[cfg(windows)]
    {
        if path.exists() && kassiber_ownership::legacy_owner_lock_is_active(&path)? {
            return Err(copy_in_use());
        }
        Ok(None)
    }
    #[cfg(not(windows))]
    crate::state_root::owner_lock(data, &path, false, false)
}
fn copy_in_use() -> Error {
    Error::new(
        ErrorKind::StateRootMigrationInUse,
        "legacy project migration requires a closed, checkpointed source database",
    )
    .retryable(true)
    .hint("Close other Kassiber processes and open then close this book with the Python build before retrying.")
}
fn require_no_sidecar(database: &Path) -> Result<()> {
    for suffix in ["-wal", "-journal"] {
        let sidecar = database.with_file_name(format!(
            "{}{suffix}",
            database.file_name().unwrap_or_default().to_string_lossy()
        ));
        match fs::metadata(sidecar) {
            Ok(info) if info.len() > 0 => return Err(copy_in_use()),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(Error::io(e)),
        }
    }
    Ok(())
}

struct Legacy {
    state: PathBuf,
    data: PathBuf,
    database: PathBuf,
}
impl Legacy {
    fn sources(&self) -> [(&'static str, PathBuf); 5] {
        [
            ("attachments", self.state.join("attachments")),
            ("exports", self.state.join("exports")),
            ("backends_env", self.state.join("config/backends.env")),
            ("settings_json", self.state.join("config/settings.json")),
            ("database", self.database.clone()),
        ]
    }
}
fn candidates(context: &Context, global: &Path) -> [PathBuf; 3] {
    [
        global.join("data"),
        context.home.join(".local/share/kassiber"),
        context.home.join(".local/share/satbooks"),
    ]
}
fn discover(context: &Context, global: &Path) -> Result<Option<Legacy>> {
    for data in candidates(context, global) {
        for filename in [DB_FILENAME, "satbooks.sqlite3"] {
            let database = data.join(filename);
            if database.exists() {
                return Ok(Some(Legacy {
                    state: context.state_root_for_data(&data)?,
                    data,
                    database,
                }));
            }
        }
    }
    Ok(None)
}
fn read_marker(path: &Path) -> Result<Value> {
    let invalid = || {
        Error::new(
            ErrorKind::StateRootMigrationFailed,
            "default state migration marker is invalid",
        )
        .retryable(true)
        .details(json!({"path":path}))
    };
    if !fs::symlink_metadata(path).map_err(|_| invalid())?.is_file() {
        return Err(invalid());
    }
    let value: Value =
        serde_json::from_slice(&fs::read(path).map_err(|_| invalid())?).map_err(|_| invalid())?;
    if !value.is_object() {
        return Err(invalid());
    }
    Ok(value)
}
/// `root` is the global or project state root holding the marker.
fn write_marker(root: &Path, path: &Path, marker: &Value) -> Result<()> {
    durable::atomic_write(
        &durable::under(root, path)?,
        (layout::python_json(marker, true)? + "\n").as_bytes(),
        &mut |_| Ok(()),
    )
}
fn workspace_count(path: &Path) -> Result<Option<i64>> {
    if !path.exists() || !kassiber_paths::looks_like_plaintext(path)? {
        return Ok(None);
    }
    // This probe never keys, migrates, or sets journal_mode on the source.
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(map_sqlite_error)?;
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='workspaces')",
            [],
            |r| r.get(0),
        )
        .map_err(map_sqlite_error)?;
    Ok(Some(if exists {
        conn.query_row("SELECT COUNT(*) FROM workspaces", [], |r| r.get(0))
            .map_err(map_sqlite_error)?
    } else {
        0
    }))
}
fn initial_marker(legacy: &Legacy, count: Option<i64>, timestamp: &str) -> Value {
    json!({"schema_version":1,"source_state_root":legacy.state,"source_data_root":legacy.data,
        "source_database":legacy.database,"created_at":timestamp,"requires_single_workspace_validation":count.is_none(),"legacy_workspace_count":count})
}
fn copy(source: &Path, target: &Path) -> Result<bool> {
    if !source.exists() {
        return Ok(false);
    }
    let metadata = fs::metadata(source).map_err(Error::io)?;
    if metadata.is_dir() {
        fs::create_dir_all(target).map_err(Error::io)?;
        for item in fs::read_dir(source).map_err(Error::io)? {
            let item = item.map_err(Error::io)?;
            copy(&item.path(), &target.join(item.file_name()))?;
        }
        fs::set_permissions(target, metadata.permissions()).map_err(Error::io)?;
        durable::sync_directory(target)?;
    } else if metadata.is_file() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(Error::io)?;
        }
        fs::copy(source, target).map_err(Error::io)?;
        durable::sync_file(target)?;
    } else {
        return Err(Error::new(
            ErrorKind::UnsupportedSchemaLayout,
            "legacy artifact is not a regular file or directory",
        ));
    }
    Ok(true)
}
fn report(
    legacy_state: &Path,
    source_db: Option<&Path>,
    target: &Path,
    timestamp: &str,
    status: &str,
    details: Value,
) -> Result<PathBuf> {
    let directory = legacy_state.join("config/migration-reports");
    fs::create_dir_all(&directory).map_err(Error::io)?;
    let path = directory.join(format!(
        "{}-legacy-project-migration.json",
        timestamp.replace(':', "")
    ));
    let payload = json!({"schema_version":1,"status":status,"created_at":timestamp,
        "source":{"state_root":legacy_state,"database":source_db},
        "target":{"project_id":"default","project_root":target,"data_root":target.join("data")},
        "split_policy":serde_json::from_str::<Value>(include_str!("../fixtures/project-policy.json")).expect("policy fixture"),"details":details});
    // Match Python's ordinary report-file mode, including an existing report.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .map_err(Error::io)?;
    file.write_all((layout::python_json(&payload, true)? + "\n").as_bytes())
        .map_err(Error::io)?;
    file.sync_all().map_err(Error::io)?;
    durable::sync_directory(&directory)?;
    Ok(path)
}
fn invalid_intent() -> Error {
    Error::new(
        ErrorKind::Validation,
        "invalid legacy project migration intent",
    )
}
fn encrypted_at(data: &Path) -> Result<bool> {
    let database = kassiber_paths::resolve_database_path(data);
    Ok(database.exists()
        && fs::metadata(&database).map_err(Error::io)?.len() > 0
        && !kassiber_paths::looks_like_plaintext(&database)?)
}
fn resume(
    context: &Context,
    global: &Path,
    path: &Path,
    marker: &mut Value,
    timestamp: &str,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let field = |name| {
        marker[name]
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(invalid_intent)
    };
    let legacy = Legacy {
        state: field("source_state_root")?,
        data: field("source_data_root")?,
        database: field("source_database")?,
    };
    let pending = &marker["pending_legacy_artifacts"];
    let backup = PathBuf::from(pending["backup_root"].as_str().ok_or_else(invalid_intent)?);
    let sources = pending["sources"].as_object().ok_or_else(invalid_intent)?;
    if !candidates(context, global).contains(&legacy.data)
        || context.state_root_for_data(&legacy.data)? != legacy.state
        || legacy.database.parent() != Some(legacy.data.as_path())
        || ![DB_FILENAME, "satbooks.sqlite3"]
            .iter()
            .any(|n| legacy.database.file_name().is_some_and(|name| name == *n))
        || backup.parent() != Some(legacy.state.as_path())
        || !backup
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(BACKUP))
        || sources.iter().any(|(key, value)| {
            !legacy
                .sources()
                .iter()
                .any(|(k, p)| k == key && value.as_str() == p.to_str())
        })
    {
        return Err(invalid_intent());
    }
    let _target_owner = owner_guard(&global.join("projects/default/data"))?;
    let _source_owner = owner_guard(&legacy.data)?;
    require_no_sidecar(&legacy.database)?;
    let mut moved = serde_json::Map::new();
    for (key, source) in legacy.sources() {
        if !sources.contains_key(key) {
            continue;
        }
        let target = backup.join(
            source
                .strip_prefix(&legacy.state)
                .map_err(|_| invalid_intent())?,
        );
        if source.exists() {
            fs::create_dir_all(target.parent().expect("backup parent")).map_err(Error::io)?;
            observer(&format!("legacy.before_move.{key}"))?;
            rename_noreplace(&source, &target).map_err(Error::io)?;
            observer(&format!("legacy.after_move.{key}"))?;
        } else if !target.exists() {
            return Err(Error::new(ErrorKind::Io, "legacy artifact is missing"));
        }
        durable::sync_directory(source.parent().expect("source parent"))?;
        durable::sync_directory(target.parent().expect("target parent"))?;
        moved.insert(key.into(), json!(target));
    }
    marker["legacy_artifacts"] = json!({"moved":moved,"backup_root":backup});
    marker["legacy_artifacts_moved_at"] = json!(timestamp);
    marker
        .as_object_mut()
        .expect("marker object")
        .remove("pending_legacy_artifacts");
    write_marker(global, path, marker)?;
    observer("legacy.complete")
}
fn resume_default(
    context: &Context,
    global: &Path,
    timestamp: &str,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let path = global.join("projects/default/config").join(MARKER);
    if path.exists() {
        let mut marker = read_marker(&path)?;
        if marker.get("pending_legacy_artifacts").is_some() {
            resume(context, global, &path, &mut marker, timestamp, observer)?;
        }
    }
    Ok(())
}
fn move_aside(
    context: &Context,
    global: &Path,
    legacy: &Legacy,
    target: &Path,
    timestamp: &str,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let path = target.join("config").join(MARKER);
    fs::create_dir_all(path.parent().expect("marker parent")).map_err(Error::io)?;
    let mut marker = if path.exists() {
        read_marker(&path)?
    } else {
        initial_marker(legacy, workspace_count(&legacy.database)?, timestamp)
    };
    if marker.get("pending_legacy_artifacts").is_none() {
        let base = legacy
            .state
            .join(format!("{BACKUP}{}", timestamp.replace(['-', ':'], "")));
        let mut backup = base.clone();
        let mut suffix = 1;
        while backup.exists() {
            suffix += 1;
            backup = PathBuf::from(format!("{}-{suffix}", base.to_string_lossy()));
        }
        let sources: serde_json::Map<String, Value> = legacy
            .sources()
            .into_iter()
            .filter(|(_, p)| p.exists())
            .map(|(k, p)| (k.into(), json!(p)))
            .collect();
        marker["pending_legacy_artifacts"] = json!({"backup_root":backup,"sources":sources});
        write_marker(global, &path, &marker)?;
        observer("legacy.intent")?;
    }
    resume(context, global, &path, &mut marker, timestamp, observer)
}
fn register(
    context: &Context,
    global: &Path,
    target: &Path,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<ProjectEntry> {
    for directory in DIRECTORIES {
        fs::create_dir_all(target.join(directory)).map_err(Error::io)?;
    }
    let entry = ProjectEntry {
        id: "default".into(),
        name: "Default".into(),
        path: target.into(),
        encrypted: encrypted_at(&target.join("data"))?,
        last_opened_at: None,
    };
    let path = global.join("config/projects.json");
    let mut catalog = kassiber_paths::load_catalog(&path, context)?;
    if let Some(old) = catalog.projects.iter_mut().find(|p| p.id == entry.id) {
        *old = entry.clone();
    } else {
        catalog.projects.push(entry.clone());
    }
    catalog.selected_project_id = Some(entry.id.clone());
    publish_catalog(&path, &catalog)?;
    observer("legacy.catalog")?;
    Ok(entry)
}
fn publish_catalog(path: &Path, catalog: &Catalog) -> Result<()> {
    let parent = path.parent().expect("catalog parent");
    fs::create_dir_all(parent).map_err(Error::io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(Error::io)?;
    }
    layout::write_catalog(path, catalog)
}

pub fn ensure_default(
    context: &Context,
    explicit_state_root: Option<&Path>,
    timestamp: &str,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<ProjectEntry> {
    let global = match explicit_state_root {
        Some(path) => context.expand_user(path)?,
        None => context.default_state_root()?,
    };
    if explicit_state_root.is_none() {
        resume_default(context, &global, timestamp, observer)?;
    }
    let catalog_path = global.join("config/projects.json");
    let mut catalog = kassiber_paths::load_catalog(&catalog_path, context)?;
    if !catalog.projects.is_empty() {
        if let Some(selected) = &catalog.selected_project_id {
            // Python get_project calls list_projects, refreshing the encryption
            // metadata for every entry even when the selected id is missing.
            let mut changed = false;
            for entry in &mut catalog.projects {
                let encrypted = encrypted_at(&entry.path.join("data"))?;
                changed |= encrypted != entry.encrypted;
                entry.encrypted = encrypted;
            }
            if changed {
                publish_catalog(&catalog_path, &catalog)?;
            }
            return catalog
                .projects
                .into_iter()
                .find(|p| &p.id == selected)
                .ok_or_else(|| {
                    Error::new(ErrorKind::ProjectNotFound, "project is not registered")
                });
        }
        let first = catalog.projects[0].clone();
        catalog.selected_project_id = Some(first.id.clone());
        publish_catalog(&catalog_path, &catalog)?;
        return Ok(first);
    }
    let target = global.join("projects/default");
    if explicit_state_root.is_none() {
        if let Some(legacy) = discover(context, &global)? {
            let _source_owner = owner_guard(&legacy.data)?;
            let _target_owner = owner_guard(&target.join("data"))?;
            require_no_sidecar(&legacy.database)?;
            if !target.join("data").join(DB_FILENAME).exists() {
                let count = workspace_count(&legacy.database)?;
                let staging = target.with_file_name("default.migrating");
                if staging.exists() {
                    fs::remove_dir_all(&staging).map_err(Error::io)?;
                }
                let staged = (|| {
                    fs::create_dir_all(&staging).map_err(Error::io)?;
                    for directory in DIRECTORIES {
                        fs::create_dir_all(staging.join(directory)).map_err(Error::io)?;
                    }
                    observer("legacy.staging")?;
                    copy(&legacy.database, &staging.join("data").join(DB_FILENAME))?;
                    observer("legacy.copy.database")?;
                    let mut copied = json!({"database":true});
                    for (key, source) in legacy
                        .sources()
                        .into_iter()
                        .filter(|(k, _)| *k != "database")
                    {
                        copied[key] = json!(copy(
                            &source,
                            &staging.join(
                                source
                                    .strip_prefix(&legacy.state)
                                    .map_err(|_| invalid_intent())?
                            )
                        )?);
                        observer(&format!("legacy.copy.{key}"))?;
                    }
                    let mut marker = initial_marker(&legacy, count, timestamp);
                    let marker_path = staging.join("config").join(MARKER);
                    write_marker(&global, &marker_path, &marker)?;
                    observer("legacy.marker")?;
                    let multi = count.is_some_and(|n| n > 1);
                    let report = report(
                        &legacy.state,
                        Some(&legacy.database),
                        &target,
                        timestamp,
                        if multi {
                            "staged_multi_workspace_single_project_copy"
                        } else {
                            "staged_single_project_copy"
                        },
                        json!({"copied":copied,"workspace_count":count,"reason":if multi { "legacy DB has multiple workspaces; migrated as one default project container because a project may contain multiple books" } else { "legacy DB migrated as one default project container" }}),
                    )?;
                    observer("legacy.report")?;
                    marker["report"] = json!(report);
                    write_marker(&global, &marker_path, &marker)?;
                    observer("legacy.before_promote")?;
                    rename_noreplace(&staging, &target).map_err(Error::io)?;
                    durable::sync_directory(target.parent().expect("projects parent"))?;
                    observer("legacy.after_promote")
                })();
                if let Err(error) = staged {
                    if staging.exists() {
                        let _ = fs::remove_dir_all(&staging);
                    }
                    return Err(error);
                }
            }
            drop(_target_owner);
            drop(_source_owner);
            move_aside(context, &global, &legacy, &target, timestamp, observer)?;
        }
    }
    register(context, &global, &target, observer)
}

pub fn validate_after_unlock(
    context: &Context,
    data: &Path,
    conn: &Connection,
    timestamp: &str,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let target = context.state_root_for_data(data)?;
    let path = target.join("config").join(MARKER);
    if !path.exists() {
        return Ok(());
    }
    let mut marker = read_marker(&path)?;
    if !marker["requires_single_workspace_validation"]
        .as_bool()
        .unwrap_or(false)
    {
        return Ok(());
    }
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM workspaces", [], |r| r.get(0))
        .map_err(map_sqlite_error)?;
    let source_state = marker["source_state_root"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or(context.default_state_root()?);
    let database = marker["source_database"].as_str().map(PathBuf::from);
    if count > 1 {
        marker["report_after_unlock"] = json!(report(
            &source_state,
            database.as_deref(),
            &target,
            timestamp,
            "validated_multi_workspace_single_project",
            json!({"workspace_count":count,"reason":"encrypted legacy DB has multiple workspaces; it remains a single project container until the user chooses an explicit split"})
        )?);
        observer("legacy.unlock_report")?;
    }
    marker["requires_single_workspace_validation"] = json!(false);
    marker["legacy_workspace_count"] = json!(count);
    marker["validated_at"] = json!(timestamp);
    write_marker(&target, &path, &marker)?;
    observer("legacy.validated")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn legacy_books_under_linked_parents_migrate_and_markers_refuse_links_below() {
        let temporary = tempfile::tempdir().unwrap();
        let (alias, real) = crate::durable::tests::aliased(temporary.path());
        let mut context = Context::from_env().unwrap();
        context.home = alias.join("home");
        context.environ.clear();
        let global = context.native_state_root();
        drop(crate::Book::create(&global.join("data"), None).unwrap());
        let entry =
            ensure_default(&context, None, "2026-01-01T00:00:00Z", &mut |_| Ok(())).unwrap();
        assert_eq!(entry.path, global.join("projects/default"));
        // The platform's own state root under the resolved alias, not Linux's.
        let projects = real
            .join(global.strip_prefix(&alias).unwrap())
            .join("projects");
        let marker = read_marker(&projects.join("default/config").join(MARKER)).unwrap();
        assert!(marker.get("legacy_artifacts").is_some());
        // Kassiber's own project tree is below the resolved root and still refuses links.
        let other = alias.join("other");
        fs::create_dir_all(real.join("other/projects-real")).unwrap();
        std::os::unix::fs::symlink("projects-real", real.join("other/projects")).unwrap();
        let root = other.join("projects/default");
        let error =
            write_marker(&other, &root.join("config").join(MARKER), &json!({})).unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnsafeConfiguration);
        assert!(!real.join("other/projects-real/default").exists());
    }
}

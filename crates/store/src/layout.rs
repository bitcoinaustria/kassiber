//! Explicit layout writes; resolving or opening a book never calls these.
use crate::{durable, Error, ErrorKind, Result};
use kassiber_paths::{Catalog, RuntimePaths};
use kassiber_pyjson::{dumps, Indent, Key, Options, Value as PyValue};
use serde_json::{json, Value};
use std::{fs::OpenOptions, path::Path};

/// Publish a machine preference exactly as `private_files.atomic_write_private`.
/// Unlike book metadata writes, this replaces a final symlink without following it.
pub fn write_private_preference(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut directories = std::fs::DirBuilder::new();
    directories.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        directories.mode(0o700);
        directories.create(parent)?;
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    directories.create(parent)?;
    let prefix = format!(
        ".{}.",
        path.file_name().unwrap_or_default().to_string_lossy()
    );
    let mut temporary = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub(crate) fn python_json(value: &Value, pretty: bool) -> Result<String> {
    dumps(
        &python_value(value),
        &Options {
            sort_keys: true,
            indent: pretty.then_some(Indent::Spaces(2)),
            ..Default::default()
        },
    )
    .map_err(|_| Error::new(ErrorKind::Validation, "invalid persisted JSON"))
}

pub(crate) fn python_value(value: &Value) -> PyValue {
    match value {
        Value::Null => PyValue::Null,
        Value::Bool(v) => PyValue::Bool(*v),
        Value::String(v) => PyValue::Str(v.as_str().into()),
        Value::Number(v) => {
            if let Some(n) = v.as_i64() {
                PyValue::Int(n.into())
            } else if let Some(n) = v.as_u64() {
                PyValue::Int(n.into())
            } else {
                PyValue::Float(v.as_f64().expect("JSON number"))
            }
        }
        Value::Array(v) => PyValue::List(v.iter().map(python_value).collect()),
        Value::Object(v) => PyValue::Dict(
            v.iter()
                .map(|(k, v)| (Key::Str(k.as_str().into()), python_value(v)))
                .collect(),
        ),
    }
}

/// Merge the managed path manifest, preserving unrelated preferences.
pub fn initialize_layout(paths: &RuntimePaths) -> Result<()> {
    if crate::restore::pending(&paths.data_root)? {
        return Err(Error::new(
            ErrorKind::RestoreRecoveryRequired,
            "Restore recovery must finish before writing the runtime layout",
        ));
    }
    for directory in [
        &paths.data_root,
        &paths.config_root,
        &paths.exports_root,
        &paths.attachments_root,
    ] {
        durable::directory(directory)?;
    }
    if let Some(parent) = paths.env_file.parent() {
        durable::directory(parent)?;
    }
    let settings = &paths.settings_file;
    durable::refuse_symlinks(settings)?;
    let lock_path = settings.with_file_name(format!(
        "{}.lock",
        settings
            .file_name()
            .expect("settings name")
            .to_string_lossy()
    ));
    durable::refuse_symlinks(&lock_path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(lock_path).map_err(Error::io)?;
    lock.lock().map_err(Error::io)?;
    let mut payload = Value::Object(kassiber_paths::load_settings(settings)?);
    payload["schema_version"] = json!(1);
    payload["app"] = json!("kassiber");
    if !payload["paths"].is_object() {
        payload["paths"] = json!({});
    }
    let additions = json!({"state_root":paths.state_root,"data_root":paths.data_root,
        "database":paths.database,"config_root":paths.config_root,"settings_file":paths.settings_file,
        "env_file":paths.env_file,"exports_root":paths.exports_root,"attachments_root":paths.attachments_root});
    payload["paths"]
        .as_object_mut()
        .expect("paths object")
        .extend(additions.as_object().expect("path fields").clone());
    let encoded = python_json(&payload, true)? + "\n";
    if std::fs::read(settings).ok().as_deref() != Some(encoded.as_bytes()) {
        durable::atomic_write(settings, encoded.as_bytes(), &mut |_| Ok(()))?;
    }
    Ok(())
}

/// Publish a complete catalog. Callers serialize read/modify/write operations
/// using their project-management ownership lease.
pub fn write_catalog(path: &Path, catalog: &Catalog) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    let mut projects = Vec::new();
    for entry in &catalog.projects {
        let mut entry = entry.clone();
        entry.id = kassiber_paths::sanitize_project_id(&entry.id)?;
        entry.name = entry.name.trim().to_owned();
        if entry.name.is_empty()
            || entry.path.as_os_str().is_empty()
            || !seen.insert(entry.id.clone())
        {
            continue;
        }
        projects.push(serde_json::to_value(entry).expect("project entry"));
    }
    let payload = json!({"schema_version":1,"selected_project_id":catalog.selected_project_id,"projects":projects});
    durable::atomic_write(
        path,
        (python_json(&payload, true)? + "\n").as_bytes(),
        &mut |_| Ok(()),
    )
}

/// Bounded private metadata publication for callers such as auth backoff.
pub fn mutate_settings(
    path: &Path,
    mutate: impl FnOnce(&mut serde_json::Map<String, Value>) -> Result<()>,
) -> Result<()> {
    let parent = path.parent().expect("settings parent");
    durable::directory(parent)?;
    durable::refuse_symlinks(path)?;
    let lock_path = path.with_file_name("settings.json.lock");
    durable::refuse_symlinks(&lock_path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(lock_path).map_err(Error::io)?;
    lock.lock().map_err(Error::io)?;
    let mut payload = kassiber_paths::load_settings(path)?;
    mutate(&mut payload)?;
    payload.insert("schema_version".into(), json!(1));
    write_private_json(path, &Value::Object(payload))
}

/// Bounded private metadata publication for callers such as auth backoff.
pub fn write_private_json(path: &Path, value: &Value) -> Result<()> {
    durable::atomic_write(
        path,
        (python_json(value, true)? + "\n").as_bytes(),
        &mut |_| Ok(()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_replace_preserves_old_or_complete_new_bytes_at_each_boundary() {
        for point in [
            "file.before_sync",
            "file.after_sync",
            "file.before_replace",
            "file.after_replace",
            "file.after_directory_sync",
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("settings.json");
            std::fs::write(&path, b"old").unwrap();
            assert!(durable::atomic_write(&path, b"new", &mut |label| {
                if label == point {
                    Err(Error::new(ErrorKind::Io, "injected failure"))
                } else {
                    Ok(())
                }
            })
            .is_err());
            let bytes = std::fs::read(&path).unwrap();
            assert!(bytes == b"old" || bytes == b"new");
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        }
    }
    #[cfg(unix)]
    #[test]
    fn catalog_is_private_and_refuses_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config/projects.json");
        write_catalog(&path, &Catalog::default()).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let link = root.path().join("link");
        symlink(&path, &link).unwrap();
        assert_eq!(
            write_catalog(&link, &Catalog::default()).unwrap_err().kind,
            ErrorKind::UnsafeConfiguration
        );
    }
}

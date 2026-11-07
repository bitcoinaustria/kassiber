//! Explicit layout writes; resolving or opening a book never calls these.
use crate::{durable, Error, ErrorKind, Result};
use kassiber_paths::{Catalog, RuntimePaths};
use kassiber_pyjson::{dumps, Indent, Key, Options, Value as PyValue};
use serde_json::{json, Value};
use std::{fs::OpenOptions, path::Path};

/// Publish report-owned CSV bytes atomically, retaining Python's file modes.
/// Unlike private exports, a new CSV uses 0666 subject to the process umask;
/// overwriting a file retains its permissions. The caller expands `~` first.
/// Final symlinks are followed, including dangling links with existing parents.
pub fn write_report_csv(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_report_csv_with(path, bytes, &mut || Ok(()))
}

fn write_report_csv_with(
    path: &Path,
    bytes: &[u8],
    before_replace: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
    use std::io::Write;
    // Python creates the requested parent before opening/following the file.
    std::fs::create_dir_all(parent_directory(path))?;
    let destination = report_csv_destination(path)?;
    // Replacement needs directory authority, not write access to the old inode.
    // The Python reference stats the target and carries its mode to the sibling.
    let permissions = match std::fs::metadata(&destination) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let parent = parent_directory(&destination);
    let mut builder = tempfile::Builder::new();
    let prefix = format!(
        ".{}.",
        destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    );
    builder.prefix(&prefix).suffix(".tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // tempfile applies these at creation, so the kernel applies the umask.
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    let mut staged = builder.tempfile_in(parent)?;
    staged.write_all(bytes)?;
    if let Some(permissions) = permissions {
        staged.as_file().set_permissions(permissions)?;
    }
    staged.as_file().sync_all()?;
    before_replace()?;
    staged.persist(&destination).map_err(|e| e.error)?;
    #[cfg(not(windows))]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn report_csv_destination(path: &Path) -> std::io::Result<std::path::PathBuf> {
    let mut destination = path.to_owned();
    // Bound dangling-link traversal too: canonicalize catches existing cycles.
    for _ in 0..40 {
        match std::fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                match destination.canonicalize() {
                    Ok(resolved) => return Ok(resolved),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        let link = std::fs::read_link(&destination)?;
                        destination = if link.is_absolute() {
                            link
                        } else {
                            parent_directory(&destination).join(link)
                        };
                        // Following a dangling link does not create its parents.
                        parent_directory(&destination).canonicalize()?;
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(_) => return Ok(destination),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(destination),
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::other("Too many levels of symbolic links"))
}

/// Publish a machine preference exactly as `private_files.atomic_write_private`.
/// Unlike book metadata writes, this replaces a final symlink without following it.
pub fn write_private_preference(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_atomic_file(path, bytes, true, &mut || Ok(()))
}

/// Publish an explicit export without changing the permissions of its parent.
/// The complete owner-only temporary file is synced before atomic replacement.
pub fn write_export(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // Atomic replacement must not bypass an existing file's write refusal.
    match OpenOptions::new().write(true).open(path) {
        Ok(_) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error),
    }
    write_atomic_file(path, bytes, false, &mut || Ok(()))?;
    #[cfg(not(windows))]
    std::fs::File::open(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?
    .sync_all()?;
    Ok(())
}

fn write_atomic_file(
    path: &Path,
    bytes: &[u8],
    _private_parent: bool,
    before_publish: &mut impl FnMut() -> std::io::Result<()>,
) -> std::io::Result<()> {
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
        if _private_parent {
            directories.mode(0o700);
        }
        directories.create(parent)?;
        if _private_parent {
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
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
    before_publish()?;
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
    // The data and state roots resolve like Python's; the managed directories
    // and files below the state root keep refusing links.
    let state = &paths.state_root;
    durable::directory(&durable::under(&paths.data_root, &paths.data_root)?)?;
    for directory in [
        &paths.config_root,
        &paths.exports_root,
        &paths.attachments_root,
    ] {
        durable::directory(&durable::under(state, directory)?)?;
    }
    if let Some(parent) = paths.env_file.parent() {
        // An explicit --env-file directory is the user's own root.
        let root = if parent.starts_with(state) {
            state
        } else {
            parent
        };
        durable::directory(&durable::under(root, parent)?)?;
    }
    let settings = &durable::under(state, &paths.settings_file)?;
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

/// Managed metadata lives in `<state root>/config/`; the state root resolves.
fn config_file(path: &Path) -> Result<std::path::PathBuf> {
    durable::under(
        path.parent()
            .and_then(Path::parent)
            .unwrap_or(Path::new("")),
        path,
    )
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
        &config_file(path)?,
        (python_json(&payload, true)? + "\n").as_bytes(),
        &mut |_| Ok(()),
    )
}

/// Bounded private metadata publication for callers such as auth backoff.
pub fn mutate_settings(
    path: &Path,
    mutate: impl FnOnce(&mut serde_json::Map<String, Value>) -> Result<()>,
) -> Result<()> {
    let path = &config_file(path)?;
    durable::directory(path.parent().expect("settings parent"))?;
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
        &config_file(path)?,
        (python_json(value, true)? + "\n").as_bytes(),
        &mut |_| Ok(()),
    )
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn report_csv_replaces_read_only_target_preserving_mode() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("report.csv");
        std::fs::write(&path, b"old\r\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o440)).unwrap();
        super::write_report_csv(&path, b"new\r\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new\r\n");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o440
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn report_csv_publication_failure_keeps_old_bytes_and_mode() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("report.csv");
        std::fs::write(&path, b"prior\r\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
        }
        assert!(
            super::write_report_csv_with(&path, b"replacement\r\n", &mut || {
                assert_eq!(std::fs::read(&path).unwrap(), b"prior\r\n");
                Err(std::io::Error::other("injected before publication"))
            })
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"prior\r\n");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        super::write_report_csv(&path, b"replacement\r\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement\r\n");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o640
            );
            assert_eq!(
                std::fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
                0o750
            );
        }
        let absent = root.path().join("absent.csv");
        assert!(super::write_report_csv_with(&absent, b"new", &mut || {
            assert!(!absent.exists());
            Err(std::io::Error::other("injected new publication failure"))
        })
        .is_err());
        assert!(!absent.exists());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn report_csv_new_file_uses_python_creation_mode() {
        let root = tempfile::tempdir().unwrap();
        // Compare to an ordinary create under the same inherited umask. Never
        // change the process umask in a multithreaded test binary.
        let control = root.path().join("control");
        std::fs::write(&control, b"control").unwrap();
        let path = root.path().join("nested/report.csv");
        super::write_report_csv(&path, b"new\r\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new\r\n");
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                std::fs::metadata(control).unwrap().permissions().mode() & 0o777
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn report_csv_follows_symlinks_and_refuses_missing_link_parents() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target.csv");
        let link = root.path().join("report.csv");
        symlink("target.csv", &link).unwrap();
        super::write_report_csv(&link, b"new\r\n").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new\r\n");
        super::write_report_csv(&link, b"updated\r\n").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"updated\r\n");
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("target.csv"));
        let missing = root.path().join("missing.csv");
        symlink("not-created/target.csv", &missing).unwrap();
        assert!(super::write_report_csv(&missing, b"no").is_err());
        assert!(!root.path().join("not-created").exists());
        let chained = root.path().join("chain.csv");
        symlink("report.csv", &chained).unwrap();
        super::write_report_csv(&chained, b"chained").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"chained");
        let cycle = root.path().join("cycle.csv");
        symlink("cycle.csv", &cycle).unwrap();
        assert!(super::write_report_csv(&cycle, b"no").is_err());
        assert!(super::write_report_csv(root.path(), b"no").is_err());
    }
    use super::*;
    #[test]
    fn export_publication_is_atomic_and_preserves_parent_permissions() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("report.csv");
        std::fs::write(&path, b"prior\r\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
        }
        assert!(write_atomic_file(&path, b"complete\r\n", false, &mut || {
            // The old artifact remains visible until the complete file is published.
            assert_eq!(std::fs::read(&path).unwrap(), b"prior\r\n");
            Err(std::io::Error::other("injected publication failure"))
        })
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"prior\r\n");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        write_export(&path, b"complete\r\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"complete\r\n");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
                0o750
            );
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let absent = root.path().join("nested/new.csv");
        write_export(&absent, b"new\r\n").unwrap();
        assert_eq!(std::fs::read(absent).unwrap(), b"new\r\n");
    }
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
    fn layout_resolves_its_roots_and_refuses_links_below_them() {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir().unwrap();
        let (alias, real) = crate::durable::tests::aliased(temporary.path());
        let mut context = kassiber_paths::Context::from_env().unwrap();
        context.home = temporary.path().join("home");
        context.environ.clear();
        let resolve = |data: &Path| {
            context
                .resolve_for_recovery(&kassiber_paths::ResolveOptions {
                    data_root: Some(data.into()),
                    ..Default::default()
                })
                .unwrap()
        };
        let paths = resolve(&alias.join("book/data"));
        initialize_layout(&paths).unwrap();
        let settings = real.join("book/config/settings.json");
        let manifest: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
        // The manifest keeps Python's lexical paths.
        assert_eq!(
            manifest["paths"]["settings_file"],
            json!(paths.settings_file)
        );
        for name in [
            "data",
            "exports",
            "attachments",
            "config/settings.json.lock",
        ] {
            assert!(real.join("book").join(name).exists(), "{name}");
        }
        mutate_settings(&paths.settings_file, |settings| {
            settings.insert("marker".into(), json!(true));
            Ok(())
        })
        .unwrap();
        write_catalog(
            &alias.join("book/config/projects.json"),
            &Catalog::default(),
        )
        .unwrap();
        write_private_json(&alias.join("book/config/auth_backoff.json"), &json!({})).unwrap();
        for name in ["projects.json", "auth_backoff.json"] {
            assert!(real.join("book/config").join(name).is_file(), "{name}");
        }
        // A user-chosen data directory may itself be a link, as in Python.
        std::fs::create_dir(real.join("data-real")).unwrap();
        symlink("data-real", real.join("linked-data")).unwrap();
        let mut paths = resolve(&alias.join("linked-data"));
        initialize_layout(&paths).unwrap();
        // Directories Kassiber manages below a state root still refuse links.
        let state = real.join("managed");
        std::fs::create_dir_all(state.join("config-real")).unwrap();
        symlink("config-real", state.join("config")).unwrap();
        paths = resolve(&alias.join("managed/data"));
        assert_eq!(
            initialize_layout(&paths).unwrap_err().kind,
            ErrorKind::UnsafeConfiguration
        );
        assert!(!state.join("config-real/settings.json").exists());
        for write in [
            write_catalog(
                &paths.config_root.join("projects.json"),
                &Catalog::default(),
            ),
            write_private_json(&paths.config_root.join("auth_backoff.json"), &json!({})),
            mutate_settings(&paths.settings_file, |_| Ok(())),
        ] {
            assert_eq!(write.unwrap_err().kind, ErrorKind::UnsafeConfiguration);
        }
        assert_eq!(
            std::fs::read_dir(state.join("config-real"))
                .unwrap()
                .count(),
            0
        );
        let state = real.join("exported");
        std::fs::create_dir_all(state.join("exports-real")).unwrap();
        symlink("exports-real", state.join("exports")).unwrap();
        paths = resolve(&alias.join("exported/data"));
        assert_eq!(
            initialize_layout(&paths).unwrap_err().kind,
            ErrorKind::UnsafeConfiguration
        );
        assert!(!state.join("config/settings.json").exists());
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

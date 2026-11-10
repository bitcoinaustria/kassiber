//! Explicit, inode-preserving hidden-home cutover. Resolution never calls it.
use crate::{durable, layout::python_json, Error, ErrorKind, Result};
use kassiber_paths::{durable::rename_noreplace, Context};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    path::{Component, Path, PathBuf},
};

const PROOF: &str = ".state-root-migration.json";
fn proof() -> Value {
    json!({"schema_version":1,"migrated_from_hidden_home":true})
}
fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}
fn has_state(root: &Path) -> bool {
    match fs::read_dir(root) {
        Ok(entries) => entries.into_iter().any(|entry| {
            entry
                .map(|e| !["bin", "run", PROOF].contains(&e.file_name().to_string_lossy().as_ref()))
                .unwrap_or(true)
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => exists(root),
    }
}
fn read_json(path: &Path) -> Result<Value> {
    if !fs::symlink_metadata(path).map_err(Error::io)?.is_file() {
        return Err(failed());
    }
    serde_json::from_slice(&fs::read(path).map_err(Error::io)?).map_err(|_| failed())
}
fn valid_proof(root: &Path) -> bool {
    read_json(&root.join(PROOF)).is_ok_and(|value| value == proof())
}
fn failed() -> Error {
    Error::new(
        ErrorKind::StateRootMigrationFailed,
        "default state migration failed",
    )
    .retryable(true)
}
fn conflict(legacy: &Path, preferred: &Path) -> Error {
    Error::new(ErrorKind::StateRootConflict,"both legacy and native default state roots contain Kassiber state").retryable(true).hint("Use --data-root to inspect and choose a root, then reconcile or move one root before retrying.").details(json!({"legacy":legacy,"native":preferred}))
}
fn chmod_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(Error::io)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
/// `root` is the app-data parent or legacy root the lock lives in; see
/// `durable::under`. Links from the root down are refused.
pub(crate) fn owner_lock(
    root: &Path,
    path: &Path,
    create: bool,
    blocking: bool,
) -> Result<Option<File>> {
    if !exists(path) && !create {
        return Ok(None);
    }
    let path = &durable::under(root, path)?;
    durable::refuse_symlinks(path)?;
    if exists(path) && !fs::symlink_metadata(path).map_err(Error::io)?.is_file() {
        return Err(failed());
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(create)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(Error::io)?;
    kassiber_ownership::validate_database_file(path)?;
    #[cfg(windows)]
    if blocking && file.metadata().map_err(Error::io)?.len() == 0 {
        use std::io::Write;
        let mut writer = &file;
        writer.write_all(&[0]).map_err(Error::io)?;
        writer.flush().map_err(Error::io)?;
    }
    if blocking {
        file.lock().map_err(Error::io)?;
    } else {
        file.try_lock().map_err(|_| {
            Error::new(
                ErrorKind::StateRootMigrationInUse,
                "default state migration requires exclusive access",
            )
            .retryable(true)
        })?;
    }
    Ok(Some(file))
}
fn reject_symlinks(root: &Path, top: bool) -> Result<()> {
    if root.is_symlink() || !root.is_dir() {
        return Err(failed());
    }
    for entry in fs::read_dir(root).map_err(Error::io)? {
        let entry = entry.map_err(Error::io)?;
        if top && ["bin", "run"].contains(&entry.file_name().to_string_lossy().as_ref()) {
            continue;
        }
        let kind = entry.file_type().map_err(Error::io)?;
        if kind.is_symlink() {
            return Err(failed());
        }
        if kind.is_dir() {
            reject_symlinks(&entry.path(), false)?;
        }
    }
    Ok(())
}
fn collect_owner_locks(legacy: &Path, root: &Path, handles: &mut Vec<File>) -> Result<()> {
    for entry in fs::read_dir(root).map_err(Error::io)? {
        let entry = entry.map_err(Error::io)?;
        let kind = entry.file_type().map_err(Error::io)?;
        if entry.file_name() == ".operator-owner.lock" {
            #[cfg(windows)]
            if kassiber_ownership::legacy_owner_lock_is_active(&entry.path())? {
                return Err(Error::new(
                    ErrorKind::StateRootMigrationInUse,
                    "default state migration requires exclusive access",
                )
                .retryable(true));
            }
            #[cfg(not(windows))]
            if let Some(handle) = owner_lock(legacy, &entry.path(), false, false)? {
                handles.push(handle);
            }
        } else if kind.is_dir() {
            collect_owner_locks(legacy, &entry.path(), handles)?;
        }
    }
    Ok(())
}
fn lexical_normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => (),
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(part.as_os_str()),
        }
    }
    out
}
fn rebase_catalog(
    preferred: &Path,
    legacy: &Path,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let path = preferred.join("config/projects.json");
    if !path.is_file() {
        return Ok(());
    }
    let path = durable::under(preferred, &path)?;
    durable::refuse_symlinks(&path)?;
    let mut payload = read_json(&path)?;
    if !payload.is_object() {
        return Err(failed());
    }
    let mut changed = false;
    if let Some(projects) = payload["projects"].as_array_mut() {
        for entry in projects {
            if let Some(text) = entry["path"].as_str() {
                let candidate = lexical_normal(Path::new(text));
                if candidate.is_absolute() {
                    if let Ok(relative) = candidate.strip_prefix(legacy) {
                        entry["path"] = json!(preferred.join(relative));
                        changed = true;
                    }
                }
            }
        }
    }
    if changed {
        let permissions = fs::metadata(&path).map_err(Error::io)?.permissions();
        durable::atomic_write_with_permissions(
            &path,
            (python_json(&payload, true)? + "\n").as_bytes(),
            Some(permissions),
            &mut |label| observer(&format!("catalog.{label}")),
        )?;
    }
    Ok(())
}

pub fn migrate_hidden_home(context: &Context) -> Result<PathBuf> {
    migrate_with_observer(context, &mut |_| Ok(()))
}
pub(crate) fn migrate_with_observer(
    context: &Context,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<PathBuf> {
    migrate_with_rename(context, observer, &mut |source, target| {
        rename_noreplace(source, target)
    })
}
fn migrate_with_rename(
    context: &Context,
    observer: &mut impl FnMut(&str) -> Result<()>,
    rename: &mut impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<PathBuf> {
    let legacy = context.home.join(".kassiber");
    let preferred = context.native_state_root();
    if preferred == legacy {
        return Ok(preferred);
    }
    let mut moved = false;
    let result = (|| {
        let parent = preferred.parent().ok_or_else(failed)?;
        durable::directory(&durable::under(parent, parent)?)?;
        let lock = parent.join(format!(
            ".{}-state-migration.lock",
            preferred.file_name().unwrap_or_default().to_string_lossy()
        ));
        let _lock = owner_lock(parent, &lock, true, true)?;
        let legacy_state = has_state(&legacy);
        let legacy_proof = exists(&legacy.join(PROOF));
        let preferred_proof = valid_proof(&preferred);
        moved = preferred_proof;
        if exists(&preferred) {
            if preferred.is_symlink() {
                return Ok(if legacy_state || legacy_proof {
                    legacy.clone()
                } else {
                    preferred.clone()
                });
            }
            if preferred_proof {
                if legacy_state || legacy_proof {
                    return Err(conflict(&legacy, &preferred));
                }
            } else {
                return Ok(if legacy_state || legacy_proof {
                    legacy.clone()
                } else {
                    preferred.clone()
                });
            }
        } else {
            if !legacy_state {
                return Ok(preferred.clone());
            }
            if legacy_proof && !valid_proof(&legacy) {
                return Ok(legacy.clone());
            }
            if reject_symlinks(&legacy, true).is_err() {
                return Ok(legacy.clone());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if fs::metadata(&legacy).map_err(Error::io)?.dev()
                    != fs::metadata(parent).map_err(Error::io)?.dev()
                {
                    if valid_proof(&legacy) {
                        fs::remove_file(legacy.join(PROOF)).map_err(Error::io)?;
                        durable::sync_directory(&legacy)?;
                    }
                    return Ok(legacy.clone());
                }
            }
            let mut locks = Vec::new();
            #[cfg(not(windows))]
            {
                let startup = legacy.join("run/operator-v1.start.lock");
                durable::directory(&durable::under(
                    &legacy,
                    startup.parent().expect("startup parent"),
                )?)?;
                match owner_lock(&legacy, &startup, true, false) {
                    Ok(Some(file)) => locks.push(file),
                    Ok(None) => (),
                    Err(_) => return Ok(legacy.clone()),
                }
            }
            if collect_owner_locks(&legacy, &legacy, &mut locks).is_err() {
                return Ok(legacy.clone());
            }
            if exists(&preferred) {
                return Ok(legacy.clone());
            }
            if !valid_proof(&legacy) {
                durable::atomic_write(
                    &durable::under(&legacy, &legacy.join(PROOF))?,
                    (python_json(&proof(), true)? + "\n").as_bytes(),
                    &mut |label| observer(&format!("proof.{label}")),
                )?;
            }
            observer("state-root.before_rename")?;
            if let Err(error) = rename(&legacy, &preferred) {
                if error.kind() == std::io::ErrorKind::CrossesDevices && valid_proof(&legacy) {
                    fs::remove_file(legacy.join(PROOF)).map_err(Error::io)?;
                    durable::sync_directory(&legacy)?;
                }
                return Ok(legacy.clone());
            }
            moved = true;
            observer("state-root.after_rename")?;
            durable::sync_directory(legacy.parent().ok_or_else(failed)?)?;
            durable::sync_directory(parent)?;
            observer("state-root.after_parent_sync")?;
        }
        chmod_private(&preferred)?;
        if legacy.is_symlink() || (exists(&legacy) && !legacy.is_dir()) {
            return Err(failed());
        }
        durable::directory(&durable::under(&legacy, &legacy)?)?;
        chmod_private(&legacy)?;
        for fixed in ["bin", "run"] {
            let source = preferred.join(fixed);
            let target = legacy.join(fixed);
            if !exists(&source) {
                continue;
            }
            if exists(&target) {
                return Err(failed());
            }
            observer(&format!("state-root.before_restore.{fixed}"))?;
            rename(&source, &target).map_err(Error::io)?;
            observer(&format!("state-root.after_restore.{fixed}"))?;
            durable::sync_directory(&preferred)?;
            durable::sync_directory(&legacy)?;
        }
        if has_state(&legacy) {
            return Err(conflict(&legacy, &preferred));
        }
        rebase_catalog(&preferred, &legacy, observer)?;
        durable::sync_directory(&preferred)?;
        durable::sync_directory(&legacy)?;
        observer("state-root.after_final_sync")?;
        Ok(preferred.clone())
    })();
    match result {
        Err(error)
            if !moved && has_state(&legacy) && error.kind != ErrorKind::StateRootConflict =>
        {
            Ok(legacy)
        }
        Err(error) if error.kind == ErrorKind::Io => {
            Err(failed().details(json!({"source":legacy,"target":preferred})))
        }
        value => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_device_failure_removes_prepared_proof_and_preserves_legacy() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join(".kassiber");
        durable::directory(&legacy.join("data")).unwrap();
        fs::write(legacy.join("data/kassiber.sqlite3"), b"retained").unwrap();
        let ctx = context(root.path());
        let mut attempted = false;
        let result = migrate_with_rename(&ctx, &mut |_| Ok(()), &mut |_, _| {
            attempted = true;
            Err(std::io::Error::new(
                std::io::ErrorKind::CrossesDevices,
                "injected EXDEV",
            ))
        })
        .unwrap();
        assert!(attempted);
        assert_eq!(result, legacy);
        assert_eq!(
            fs::read(legacy.join("data/kassiber.sqlite3")).unwrap(),
            b"retained"
        );
        assert!(!legacy.join(PROOF).exists());
        assert!(!ctx.native_state_root().exists());
    }
    #[cfg(unix)]
    #[test]
    fn linked_home_parents_migrate_but_linked_legacy_layouts_stay() {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir().unwrap();
        let (alias, real) = durable::tests::aliased(temporary.path());
        let home = alias.join("home");
        let legacy = real.join("home/.kassiber");
        durable::directory(&legacy.join("data")).unwrap();
        durable::directory(&legacy.join("run")).unwrap();
        fs::write(legacy.join("data/kassiber.sqlite3"), b"moved").unwrap();
        let ctx = context(&home);
        let moved = migrate_hidden_home(&ctx).unwrap();
        assert_eq!(moved, home.join(".local/share/kassiber"));
        assert_eq!(
            fs::read(real.join("home/.local/share/kassiber/data/kassiber.sqlite3")).unwrap(),
            b"moved"
        );
        assert!(legacy.join("run").is_dir());
        assert!(!legacy.join("data").exists());
        // Python's _reject_unsafe_migration_symlinks keeps these layouts legacy.
        for inner in [false, true] {
            let home = alias.join(format!("home-{inner}"));
            let target = real.join(format!("home-{inner}/target"));
            durable::directory(&target.join("data")).unwrap();
            fs::write(target.join("data/kassiber.sqlite3"), b"kept").unwrap();
            let legacy = home.join(".kassiber");
            let real_legacy = real.join(format!("home-{inner}/.kassiber"));
            if inner {
                durable::directory(&real_legacy).unwrap();
                symlink(target.join("data"), real_legacy.join("data")).unwrap();
            } else {
                symlink(&target, &real_legacy).unwrap();
            }
            let ctx = context(&home);
            assert_eq!(migrate_hidden_home(&ctx).unwrap(), legacy);
            assert!(!ctx.native_state_root().exists());
            assert_eq!(
                fs::read(legacy.join("data/kassiber.sqlite3")).unwrap(),
                b"kept"
            );
        }
    }
    fn context(home: &Path) -> Context {
        Context {
            home: home.into(),
            platform: kassiber_paths::Platform::Linux,
            environ: std::collections::BTreeMap::new(),
        }
    }
    #[test]
    fn atomic_cutover_keeps_inode_and_resumes_after_every_root_step() {
        for point in [
            "state-root.before_rename",
            "state-root.after_rename",
            "state-root.after_parent_sync",
            "state-root.after_restore.bin",
            "state-root.after_restore.run",
            "state-root.after_final_sync",
        ] {
            let root = tempfile::tempdir().unwrap();
            let legacy = root.path().join(".kassiber");
            durable::directory(&legacy.join("data")).unwrap();
            durable::directory(&legacy.join("bin")).unwrap();
            durable::directory(&legacy.join("run")).unwrap();
            let path = legacy.join("data/kassiber.sqlite3");
            fs::write(&path, b"synthetic inode marker").unwrap();
            #[cfg(unix)]
            let identity = {
                use std::os::unix::fs::MetadataExt;
                fs::metadata(&path).unwrap().ino()
            };
            let ctx = context(root.path());
            let _ = migrate_with_observer(&ctx, &mut |label| {
                if label == point {
                    Err(Error::new(ErrorKind::Io, "injected state-root failure"))
                } else {
                    Ok(())
                }
            });
            let moved = migrate_hidden_home(&ctx).unwrap();
            assert_eq!(moved, ctx.native_state_root());
            assert_eq!(
                fs::read(moved.join("data/kassiber.sqlite3")).unwrap(),
                b"synthetic inode marker"
            );
            assert!(valid_proof(&moved));
            assert!(!has_state(&legacy));
            assert!(legacy.join("bin").is_dir());
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_eq!(
                    fs::metadata(moved.join("data/kassiber.sqlite3"))
                        .unwrap()
                        .ino(),
                    identity
                );
            }
        }
    }
}

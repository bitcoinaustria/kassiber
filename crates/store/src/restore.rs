//! Restore transaction format shared with backup/install.py. Callers close all
//! connections and hold maintenance ownership before install/recover. An intent
//! precedes live moves; pending transactions roll back, committed ones retain
//! the complete incoming tree and pre-restore copy. Never infer a commit from
//! a partially moved tree. Observers are explicit, never environment selected.
use crate::{durable, Error, ErrorKind, Result};
use kassiber_ownership::{OwnerKind, OwnerLease, OwnerNamespace};
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

pub const JOURNAL: &str = ".restore-pending.json";
pub const NAMES: [&str; 8] = [
    "kassiber.sqlite3",
    "kassiber.sqlite3-wal",
    "kassiber.sqlite3-shm",
    "kassiber.sqlite3-journal",
    "attachments",
    "config/backends.env",
    "config/settings.json",
    "exports",
];
pub const INCOMING: [&str; 4] = [NAMES[0], NAMES[4], NAMES[5], NAMES[6]];

fn invalid() -> Error {
    Error::new(
        ErrorKind::RestoreRecoveryRequired,
        "Restore recovery journal is invalid; keep the restore artifacts",
    )
}
fn safe(path: &Path) -> Result<()> {
    durable::refuse_symlinks(path).map_err(|error| {
        if error.kind == ErrorKind::UnsafeConfiguration {
            // Python's `_unsafe()` carries no details: the envelope says null.
            Error::new(
                ErrorKind::UnsafeRestoreTarget,
                "Restore destination contains a symbolic link",
            )
            .details(Value::Null)
        } else {
            error
        }
    })
}
pub fn targets(data: &Path) -> Result<Vec<(&'static str, PathBuf)>> {
    let data = kassiber_paths::canonicalize_allow_missing(data)?;
    let state = if data.file_name().is_some_and(|n| n == "data") {
        data.parent().ok_or_else(invalid)?
    } else {
        &data
    };
    Ok(NAMES
        .iter()
        .map(|name| {
            (
                *name,
                if name.starts_with("kassiber.sqlite3") {
                    data.join(name)
                } else {
                    state.join(name)
                },
            )
        })
        .collect())
}
fn state(data: &Path) -> Result<PathBuf> {
    Ok(targets(data)?[4].1.parent().ok_or_else(invalid)?.to_owned())
}
pub fn pending(data: &Path) -> Result<bool> {
    let marker = state(data)?.join(JOURNAL);
    Ok(marker.try_exists().map_err(Error::io)? || marker.is_symlink())
}
/// Retain the same path and file reservations across every rename. The active
/// desktop role can be supplied only by a caller already retaining that lease.
pub struct Maintenance {
    _leases: Vec<OwnerLease>,
}
pub fn maintenance(
    namespace: &OwnerNamespace,
    data: &Path,
    active: Option<OwnerKind>,
) -> Result<Maintenance> {
    let project = namespace.canonical_project(data)?;
    let mut leases = Vec::new();
    for role in [OwnerKind::Broker, OwnerKind::Desktop] {
        if Some(role) == active {
            continue;
        }
        leases.push(namespace.acquire(project.clone(), role, "backup-restore").map_err(|failure| {
            if failure.kind != ErrorKind::ProjectInUse { return failure; }
            Error::new(ErrorKind::ProjectInUse, "database maintenance requires exclusive project access")
                .hint("Lock the operator broker lease and close the desktop project, then retry.")
                .retryable(true)
                .details(json!({"project":project.public_id,"owner":failure.details.get("owner").unwrap_or(&Value::String("unknown".into()))}))
        })?);
    }
    Ok(Maintenance { _leases: leases })
}
pub fn recover_pending(namespace: &OwnerNamespace, data: &Path) -> Result<()> {
    if pending(data)? {
        let _maintenance = maintenance(namespace, data, None)?;
        recover(data, &mut |_| Ok(()))?;
    }
    Ok(())
}

/// Recovery precedes local configuration admission as well as layout writes.
/// Resolve again afterwards: the transaction may have replaced backends.env.
pub fn resolve_for_bootstrap(
    context: &kassiber_paths::Context,
    namespace: &OwnerNamespace,
    options: &kassiber_paths::ResolveOptions,
) -> Result<kassiber_paths::RuntimePaths> {
    let located = context.resolve_for_recovery(options)?;
    recover_pending(namespace, &located.data_root)?;
    context.resolve_for_bootstrap(options)
}

fn entries(path: &Path) -> Result<Vec<PathBuf>> {
    let mut items = fs::read_dir(path)
        .map_err(Error::io)?
        .map(|e| e.map(|e| e.path()).map_err(Error::io))
        .collect::<Result<Vec<_>>>()?;
    items.sort();
    Ok(items)
}
fn walk(path: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for entry in entries(path)? {
        safe(&entry)?;
        output.push(entry.clone());
        if entry.is_dir() {
            walk(&entry, output)?;
        }
    }
    output.sort();
    Ok(())
}
fn copy(source: &Path, target: &Path) -> Result<()> {
    safe(source)?;
    if source.is_dir() {
        durable::directory(target)?;
        for entry in entries(source)? {
            copy(&entry, &target.join(entry.file_name().ok_or_else(invalid)?))?;
        }
    } else {
        if !source.is_file() {
            return Err(invalid());
        }
        if let Some(parent) = target.parent() {
            durable::directory(parent)?;
        }
        fs::copy(source, target).map_err(Error::io)?;
    }
    let metadata = fs::metadata(source).map_err(Error::io)?;
    fs::set_permissions(target, metadata.permissions()).map_err(Error::io)?;
    let times = fs::FileTimes::new()
        .set_accessed(metadata.accessed().map_err(Error::io)?)
        .set_modified(metadata.modified().map_err(Error::io)?);
    File::open(target)
        .and_then(|f| f.set_times(times))
        .map_err(Error::io)
}
fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .expect("restore child")
        .to_string_lossy()
        .replace('\\', "/")
}
fn sync_tree(path: &Path, observer: &mut impl FnMut(&str) -> Result<()>) -> Result<()> {
    let mut items = Vec::new();
    walk(path, &mut items)?;
    for item in items.iter().filter(|p| p.is_file()) {
        File::open(item)
            .and_then(|f| f.sync_all())
            .map_err(Error::io)?;
        observer(&format!("restore.copy_sync.{}", relative(item, path)))?;
    }
    for item in items.iter().rev().filter(|p| p.is_dir()) {
        durable::sync_directory(item)?;
        observer(&format!("restore.copy_dir_sync.{}", relative(item, path)))?;
    }
    durable::sync_directory(path)?;
    observer("restore.copy_dir_sync.root")
}
fn publish(
    state: &Path,
    journal: &Value,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let phase = journal["phase"].as_str().ok_or_else(invalid)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".restore-journal-")
        .tempfile_in(state)
        .map_err(Error::io)?;
    temp.write_all((journal.to_string() + "\n").as_bytes())
        .map_err(Error::io)?;
    temp.flush().map_err(Error::io)?;
    temp.as_file().sync_all().map_err(Error::io)?;
    observer(&format!("restore.{phase}.journal_sync"))?;
    temp.persist(state.join(JOURNAL))
        .map_err(|e| Error::io(e.error))?;
    observer(&format!("restore.{phase}.journal_replace"))?;
    durable::sync_directory(state)?;
    observer(&format!("restore.{phase}.journal_dir_sync"))
}
fn names(journal: &Value, field: &str, allowed: &[&str]) -> Result<Vec<String>> {
    let names = journal[field]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|v| v.as_str().map(str::to_owned).ok_or_else(invalid))
        .collect::<Result<Vec<_>>>()?;
    if names
        != allowed
            .iter()
            .filter(|n| names.iter().any(|s| s == **n))
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    {
        return Err(invalid());
    }
    Ok(names)
}
fn read(state: &Path, data: &Path) -> Result<Value> {
    let marker = state.join(JOURNAL);
    safe(&marker)?;
    let info = fs::metadata(&marker).map_err(Error::io)?;
    if !info.is_file() || info.len() > 65536 {
        return Err(invalid());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // validate_database_file checks current ownership without consulting NSS.
        kassiber_ownership::validate_database_file(&marker).map_err(|_| invalid())?;
        if info.mode() & 0o022 != 0 {
            return Err(invalid());
        }
    }
    let journal: Value =
        serde_json::from_slice(&fs::read(&marker).map_err(Error::io)?).map_err(|_| invalid())?;
    if [
        "version",
        "data_root",
        "prepared",
        "recovery",
        "original",
        "incoming",
        "phase",
    ]
    .iter()
    .any(|key| journal.get(key).is_none())
    {
        return Err(invalid());
    }
    let prepared = journal["prepared"].as_str().ok_or_else(invalid)?;
    let token = prepared
        .strip_prefix(".restore-install-")
        .ok_or_else(invalid)?;
    let recovery = journal["recovery"].as_str();
    if journal["version"].as_u64() != Some(1)
        || journal["data_root"] != json!(kassiber_paths::canonicalize_allow_missing(data)?)
        || token.is_empty()
        || !token
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        || (!journal["recovery"].is_null()
            && recovery != Some(format!("pre-restore-{token}").as_str()))
        || !matches!(journal["phase"].as_str(), Some("pending" | "committed"))
    {
        return Err(invalid());
    }
    let original = names(&journal, "original", &NAMES)?;
    names(&journal, "incoming", &INCOMING)?;
    if original.is_empty() != recovery.is_none() {
        return Err(invalid());
    }
    for name in [Some(prepared), recovery].into_iter().flatten() {
        let root = state.join(name);
        safe(&root)?;
        walk(&root, &mut Vec::new())?;
    }
    Ok(journal)
}
fn remove(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(Error::io)
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::io(e)),
        }
    }
}
fn move_entry(
    source: &Path,
    target: &Path,
    label: &str,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    durable::directory(target.parent().ok_or_else(invalid)?)?;
    observer(&format!("restore.before_{label}"))?;
    fs::rename(source, target).map_err(Error::io)?;
    observer(&format!("restore.after_{label}"))?;
    durable::sync_directory(source.parent().ok_or_else(invalid)?)?;
    observer(&format!("restore.source_sync.{label}"))?;
    durable::sync_directory(target.parent().ok_or_else(invalid)?)?;
    observer(&format!("restore.target_sync.{label}"))
}

pub fn recover(data: &Path, observer: &mut impl FnMut(&str) -> Result<()>) -> Result<()> {
    let targets = targets(data)?;
    let state = state(data)?;
    for (_, target) in &targets {
        safe(target)?;
    }
    if !pending(data)? {
        return Ok(());
    }
    let journal = read(&state, data)?;
    let original = names(&journal, "original", &NAMES)?;
    let incoming = names(&journal, "incoming", &INCOMING)?;
    let prepared = state.join(journal["prepared"].as_str().ok_or_else(invalid)?);
    let recovery = journal["recovery"].as_str().map(|n| state.join(n));
    let pending = journal["phase"] == "pending";
    if pending {
        for (name, target) in targets.iter().rev() {
            let saved = recovery.as_ref().map(|r| r.join(name));
            if let Some(saved) = saved.filter(|p| p.exists()) {
                remove(target)?;
                observer(&format!("restore.rollback_remove.{name}"))?;
                move_entry(&saved, target, &format!("rollback.{name}"), observer)?;
            } else if !original.iter().any(|n| n == name) && incoming.iter().any(|n| n == name) {
                remove(target)?;
                if let Some(parent) = target.parent().filter(|p| p.exists()) {
                    durable::sync_directory(parent)?;
                }
                observer(&format!("restore.rollback_absent.{name}"))?;
            } else if original.iter().any(|n| n == name) && !target.exists() {
                return Err(invalid());
            }
        }
    }
    for (_, target) in &targets {
        if let Some(parent) = target.parent().filter(|p| p.exists()) {
            durable::sync_directory(parent)?;
        }
    }
    observer("restore.recovery_dir_sync")?;
    fs::remove_file(state.join(JOURNAL)).map_err(Error::io)?;
    observer("restore.journal_unlink")?;
    durable::sync_directory(&state)?;
    observer("restore.journal_unlink_sync")?;
    let _ = fs::remove_dir_all(prepared);
    if let Some(recovery) = recovery.filter(|_| pending) {
        fs::remove_dir_all(recovery).map_err(Error::io)?;
    }
    observer("restore.cleanup")
}

pub fn install(staging: &Path, data: &Path) -> Result<Option<PathBuf>> {
    install_observed(staging, data, &mut |_| Ok(()))
}
fn install_observed(
    staging: &Path,
    data: &Path,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Option<PathBuf>> {
    let targets = targets(data)?;
    let state = state(data)?;
    for (_, target) in &targets {
        safe(target)?;
    }
    recover(data, observer)?;
    durable::directory(&state)?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".restore-install-");
    // Deterministic wire-oracle path, never enabled in a shipping binary.
    #[cfg(feature = "test-hooks")]
    let fixed_prefix = std::env::var("KASSIBER_RUST_TEST_RESTORE_SUFFIX")
        .ok()
        .filter(|s| !s.is_empty() && s.len() <= 32 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        .map(|s| format!(".restore-install-{s}"));
    #[cfg(feature = "test-hooks")]
    if let Some(prefix) = fixed_prefix.as_deref() {
        builder.prefix(prefix).rand_bytes(0);
    }
    let prepared = builder.tempdir_in(&state).map_err(Error::io)?.keep();
    let mut recovery = None;
    let mut published = false;
    let mut committed = false;
    let result: Result<Option<PathBuf>> = (|| {
        observer("restore.prepared")?;
        let mut incoming = Vec::new();
        for name in INCOMING {
            let source = staging.join(name);
            if source.exists() {
                copy(&source, &prepared.join(name))?;
                incoming.push(name);
            }
        }
        sync_tree(&prepared, observer)?;
        for (_, target) in &targets {
            durable::directory(target.parent().ok_or_else(invalid)?)?;
            durable::sync_directory(target.parent().ok_or_else(invalid)?)?;
        }
        durable::sync_directory(&state)?;
        observer("restore.destinations_dir_sync")?;
        let original = targets
            .iter()
            .filter(|(_, target)| target.exists())
            .map(|(name, _)| *name)
            .collect::<Vec<_>>();
        if !original.is_empty() {
            let saved = state.join(format!(
                "pre-restore-{}",
                prepared
                    .file_name()
                    .ok_or_else(invalid)?
                    .to_string_lossy()
                    .trim_start_matches(".restore-install-")
            ));
            // Exclusive creation: never reuse or replace a retained recovery tree.
            fs::create_dir(&saved).map_err(Error::io)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&saved, fs::Permissions::from_mode(0o700))
                    .map_err(Error::io)?;
            }
            durable::directory(&saved.join("config"))?;
            durable::sync_directory(&saved.join("config"))?;
            durable::sync_directory(&saved)?;
            recovery = Some(saved);
        }
        durable::sync_directory(&state)?;
        observer("restore.artifacts_dir_sync")?;
        let mut journal = json!({"version":1,"data_root":kassiber_paths::canonicalize_allow_missing(data)?,"prepared":prepared.file_name().and_then(|n|n.to_str()),"recovery":recovery.as_ref().and_then(|p|p.file_name()).and_then(|n|n.to_str()),"original":original,"incoming":incoming,"phase":"pending"});
        publish(&state, &journal, observer)?;
        published = true;
        for (name, target) in &targets {
            if original.contains(name) {
                move_entry(
                    target,
                    &recovery.as_ref().ok_or_else(invalid)?.join(name),
                    &format!("save.{name}"),
                    observer,
                )?;
            }
        }
        for (name, target) in &targets {
            if incoming.contains(name) {
                move_entry(
                    &prepared.join(name),
                    target,
                    &format!("install.{name}"),
                    observer,
                )?;
            }
        }
        journal["phase"] = json!("committed");
        publish(&state, &journal, observer)?;
        committed = true;
        recover(data, observer)?;
        Ok(recovery.clone())
    })();
    let result = match result {
        Err(failure) if failure.kind == ErrorKind::Io => {
            if pending(data)? {
                (|| {
                    committed = read(&state, data)?["phase"] == "committed";
                    recover(data, &mut |_| Ok(()))
                })()
                .map_err(|_| {
                    let recovery_path = recovery
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "None".into());
                    Error::new(
                        ErrorKind::RestoreRollbackFailed,
                        "Restore failed and automatic recovery could not finish",
                    )
                    .details(json!({"recovery_path":recovery_path}))
                })?;
            }
            if committed {
                Ok(recovery.clone())
            } else {
                Err(Error::new(
                    ErrorKind::RestoreInstallFailed,
                    if published {
                        "Restore could not be installed; the original data was restored"
                    } else {
                        "Restore preparation failed; the original data is unchanged"
                    },
                )
                .hint("Check destination permissions and available space, then preview again."))
            }
        }
        other => other,
    };
    if !pending(data)? {
        let _ = fs::remove_dir_all(prepared);
        if let Some(recovery) = recovery.filter(|_| !committed) {
            let _ = fs::remove_dir_all(recovery);
        }
    }
    result
}

#[cfg(feature = "test-hooks")]
pub fn install_with_observer(
    staging: &Path,
    data: &Path,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<Option<PathBuf>> {
    install_observed(staging, data, observer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    const NEW_NAMES: [&str; 4] = [
        "kassiber.sqlite3",
        "attachments",
        "config/backends.env",
        "config/settings.json",
    ];
    fn expected_targets(data: &Path) -> Vec<(&'static str, PathBuf)> {
        [
            "kassiber.sqlite3",
            "kassiber.sqlite3-wal",
            "kassiber.sqlite3-shm",
            "kassiber.sqlite3-journal",
            "attachments",
            "config/backends.env",
            "config/settings.json",
            "exports",
        ]
        .into_iter()
        .map(|name| {
            (
                name,
                if name.starts_with("kassiber.sqlite3") {
                    data.join(name)
                } else {
                    data.parent().unwrap().join(name)
                },
            )
        })
        .collect()
    }
    fn setup(root: &Path, populated: bool) -> (PathBuf, PathBuf) {
        let data = root.join("book/data");
        let stage = root.join("stage");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&stage).unwrap();
        if populated {
            for (name, mut target) in expected_targets(&data) {
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                if ["attachments", "exports"].contains(&name) {
                    fs::create_dir(&target).unwrap();
                    target = target.join("old");
                }
                fs::write(target, format!("old {name}")).unwrap();
            }
        }
        for name in NEW_NAMES {
            let mut target = stage.join(name);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            if name == "attachments" {
                fs::create_dir(&target).unwrap();
                target = target.join("new");
            }
            fs::write(target, format!("new {name}")).unwrap();
        }
        (data, stage)
    }
    fn assert_tree(data: &Path, populated: bool, committed: bool) {
        for (name, mut target) in expected_targets(data) {
            let present = if committed {
                NEW_NAMES.contains(&name)
            } else {
                populated
            };
            if !present {
                assert!(!target.exists(), "unexpected {}", target.display());
                continue;
            }
            if ["attachments", "exports"].contains(&name) {
                target = target.join(if committed { "new" } else { "old" });
            }
            assert_eq!(
                fs::read_to_string(&target).unwrap(),
                format!("{} {name}", if committed { "new" } else { "old" }),
                "{}",
                target.display()
            );
        }
        assert!(!pending(data).unwrap());
        if populated && committed {
            let recovery = entries(data.parent().unwrap())
                .unwrap()
                .into_iter()
                .find(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("pre-restore-")
                })
                .unwrap();
            for (name, _) in expected_targets(data) {
                let mut saved = recovery.join(name);
                if ["attachments", "exports"].contains(&name) {
                    saved = saved.join("old");
                }
                assert_eq!(fs::read_to_string(saved).unwrap(), format!("old {name}"));
            }
        }
    }
    #[test]
    fn durable_install_boundaries_recover_original_or_complete_and_retain_copy() {
        for populated in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let (data, stage) = setup(root.path(), populated);
            let mut points = Vec::new();
            install_observed(&stage, &data, &mut |p| {
                points.push(p.to_owned());
                Ok(())
            })
            .unwrap();
            let commit = points
                .iter()
                .position(|p| p == "restore.committed.journal_replace")
                .unwrap();
            let mut expected = vec!["restore.prepared".to_owned()];
            expected.extend(
                [
                    "attachments/new",
                    "config/backends.env",
                    "config/settings.json",
                    "kassiber.sqlite3",
                ]
                .map(|name| format!("restore.copy_sync.{name}")),
            );
            expected.extend(
                [
                    "restore.copy_dir_sync.config",
                    "restore.copy_dir_sync.attachments",
                    "restore.copy_dir_sync.root",
                    "restore.destinations_dir_sync",
                    "restore.artifacts_dir_sync",
                    "restore.pending.journal_sync",
                    "restore.pending.journal_replace",
                    "restore.pending.journal_dir_sync",
                ]
                .map(str::to_owned),
            );
            let saves = expected_targets(&data)
                .into_iter()
                .filter(|_| populated)
                .map(|(name, _)| format!("save.{name}"));
            for entry in saves.chain(NEW_NAMES.map(|name| format!("install.{name}"))) {
                expected.extend([
                    format!("restore.before_{entry}"),
                    format!("restore.after_{entry}"),
                    format!("restore.source_sync.{entry}"),
                    format!("restore.target_sync.{entry}"),
                ]);
            }
            expected.extend(
                [
                    "restore.committed.journal_sync",
                    "restore.committed.journal_replace",
                    "restore.committed.journal_dir_sync",
                    "restore.recovery_dir_sync",
                    "restore.journal_unlink",
                    "restore.journal_unlink_sync",
                    "restore.cleanup",
                ]
                .map(str::to_owned),
            );
            assert_eq!(points, expected);
            assert!(points.iter().any(|p| p == "restore.pending.journal_sync"));
            assert!(
                points
                    .iter()
                    .position(|p| p == "restore.pending.journal_dir_sync")
                    .unwrap()
                    < points
                        .iter()
                        .position(|p| p == "restore.before_install.kassiber.sqlite3")
                        .unwrap()
            );
            for (index, point) in points.iter().enumerate() {
                let root = tempfile::tempdir().unwrap();
                let (data, stage) = setup(root.path(), populated);
                let killed = catch_unwind(AssertUnwindSafe(|| {
                    install_observed(&stage, &data, &mut |p| {
                        if p == point {
                            panic!("simulate process death: {p}");
                        }
                        Ok(())
                    })
                }));
                assert!(killed.is_err(), "unreached {point}");
                recover(&data, &mut |_| Ok(())).unwrap();
                recover(&data, &mut |_| Ok(())).unwrap();
                assert_tree(&data, populated, index >= commit);
            }
            for (index, point) in points.iter().enumerate() {
                let root = tempfile::tempdir().unwrap();
                let (data, stage) = setup(root.path(), populated);
                let result = install_observed(&stage, &data, &mut |p| {
                    if p == point {
                        return Err(Error::new(ErrorKind::Io, "injected"));
                    }
                    Ok(())
                });
                if index < commit {
                    assert_eq!(
                        result.unwrap_err().code,
                        "restore_install_failed",
                        "{point}"
                    );
                } else {
                    result.unwrap();
                }
                assert_tree(&data, populated, index >= commit);
            }
        }
    }
    fn interrupted(root: &Path, populated: bool) -> PathBuf {
        let (data, stage) = setup(root, populated);
        let killed = catch_unwind(AssertUnwindSafe(|| {
            install_observed(&stage, &data, &mut |p| {
                if p == "restore.committed.journal_sync" {
                    panic!("pending publication");
                }
                Ok(())
            })
        }));
        assert!(killed.is_err());
        data
    }
    #[test]
    fn rollback_is_repeatable_at_every_recovery_boundary() {
        for populated in [true, false] {
            let trace = tempfile::tempdir().unwrap();
            let data = interrupted(trace.path(), populated);
            let mut points = Vec::new();
            recover(&data, &mut |p| {
                points.push(p.to_owned());
                Ok(())
            })
            .unwrap();
            assert!(points.iter().any(|p| p == "restore.recovery_dir_sync"));
            for point in points {
                let root = tempfile::tempdir().unwrap();
                let data = interrupted(root.path(), populated);
                assert!(catch_unwind(AssertUnwindSafe(|| recover(&data, &mut |p| {
                    if p == point {
                        panic!("kill recovery");
                    }
                    Ok(())
                })))
                .is_err());
                recover(&data, &mut |_| Ok(())).unwrap();
                assert_tree(&data, populated, false);
            }
        }
    }
    #[cfg(unix)]
    #[test]
    fn failed_rollback_preserves_intent_and_reports_recovery_location() {
        use std::os::unix::fs::PermissionsExt;
        for populated in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let (data, stage) = setup(root.path(), populated);
            let marker = state(&data).unwrap().join(JOURNAL);
            let failure = install_observed(&stage, &data, &mut |point| {
                if point == "restore.after_install.kassiber.sqlite3" {
                    // An inadmissible intent cannot be replayed after an I/O
                    // failure. Retain it and every copy for manual recovery.
                    fs::set_permissions(&marker, fs::Permissions::from_mode(0o666)).unwrap();
                    return Err(Error::new(ErrorKind::Io, "injected failure"));
                }
                Ok(())
            })
            .unwrap_err();
            assert_eq!(failure.code, "restore_rollback_failed");
            let journal: Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
            assert_eq!(journal["phase"], "pending");
            assert_eq!(
                failure.details["recovery_path"],
                journal["recovery"].as_str().map_or_else(
                    || json!("None"),
                    |name| json!(state(&data).unwrap().join(name)),
                )
            );
            assert_eq!(
                fs::read_to_string(data.join(NAMES[0])).unwrap(),
                "new kassiber.sqlite3"
            );
            assert!(state(&data)
                .unwrap()
                .join(journal["prepared"].as_str().unwrap())
                .exists());
            fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();
            recover(&data, &mut |_| Ok(())).unwrap();
            assert_tree(&data, populated, false);
        }
    }
    #[test]
    fn invalid_intents_and_target_links_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let data = interrupted(root.path(), true);
        let marker = state(&data).unwrap().join(JOURNAL);
        let journal: Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
        assert!(read(&state(&data).unwrap(), &data).is_ok());
        let cases = [
            ("version", json!(true)),
            ("data_root", json!("/elsewhere")),
            ("prepared", json!("../outside")),
            ("prepared", json!(".restore-install-")),
            ("recovery", json!("pre-restore-wrong")),
            ("incoming", json!(["exports"])),
            ("original", json!(["kassiber.sqlite3", "kassiber.sqlite3"])),
            ("phase", json!("unknown")),
        ];
        for (key, value) in cases {
            let mut bad = journal.clone();
            bad[key] = value;
            fs::write(&marker, bad.to_string()).unwrap();
            assert_eq!(
                recover(&data, &mut |_| Ok(())).unwrap_err().code,
                "restore_recovery_required",
                "{key}"
            );
        }
        let mut escaped = journal.clone();
        escaped["prepared"] = json!(format!("../{}", journal["prepared"].as_str().unwrap()));
        fs::write(&marker, escaped.to_string()).unwrap();
        assert_eq!(
            recover(&data, &mut |_| Ok(())).unwrap_err().code,
            "restore_recovery_required"
        );
        let mut oversized = journal.clone();
        oversized["padding"] = json!("x".repeat(65536));
        fs::write(&marker, oversized.to_string()).unwrap();
        assert_eq!(
            recover(&data, &mut |_| Ok(())).unwrap_err().code,
            "restore_recovery_required"
        );
        fs::write(&marker, journal.to_string()).unwrap();
        assert_eq!(
            crate::Book::open_migrating(&data, Default::default(), false)
                .err()
                .unwrap()
                .code,
            "restore_recovery_required"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let original = data.join("kassiber.sqlite3-journal");
            let other = root.path().join("other");
            fs::write(&other, "outside").unwrap();
            symlink(&other, original).unwrap();
            let error = recover(&data, &mut |_| Ok(())).unwrap_err();
            assert_eq!(
                (error.code, error.details),
                ("unsafe_restore_target", Value::Null)
            );
            assert_eq!(fs::read_to_string(other).unwrap(), "outside");
        }
    }

    #[test]
    fn maintenance_retains_path_reservations_across_database_replacement() {
        let root = tempfile::tempdir().unwrap();
        let (data, stage) = setup(root.path(), true);
        let namespace = OwnerNamespace::isolated(root.path().join("owners"));
        let guard = maintenance(&namespace, &data, None).unwrap();
        install(&stage, &data).unwrap();
        let current = namespace.canonical_project(&data).unwrap();
        for role in [OwnerKind::Broker, OwnerKind::Desktop] {
            assert!(namespace.acquire(current.clone(), role, "late").is_err());
        }
        drop(guard);
        for role in [OwnerKind::Broker, OwnerKind::Desktop] {
            let lease = namespace
                .acquire(current.clone(), role, "released")
                .unwrap();
            drop(lease);
        }
    }

    #[test]
    fn startup_recovers_before_reading_partial_sidecar_configuration() {
        let root = tempfile::tempdir().unwrap();
        let (data, stage) = setup(root.path(), true);
        fs::write(
            data.parent().unwrap().join("config/settings.json"),
            "{\"schema_version\":1}",
        )
        .unwrap();
        fs::write(
            stage.join("config/settings.json"),
            "invalid incoming settings",
        )
        .unwrap();
        assert!(catch_unwind(AssertUnwindSafe(|| install_observed(
            &stage,
            &data,
            &mut |point| {
                if point == "restore.after_install.config/settings.json" {
                    panic!("interrupt partial settings");
                }
                Ok(())
            }
        )))
        .is_err());
        let mut context = kassiber_paths::Context::from_env().unwrap();
        context.home = root.path().join("home");
        context.environ.clear();
        let options = kassiber_paths::ResolveOptions {
            data_root: Some(data.clone()),
            ..Default::default()
        };
        assert!(context.resolve_for_bootstrap(&options).is_err());
        let located = context.resolve_for_recovery(&options).unwrap();
        assert_eq!(
            crate::layout::initialize_layout(&located).unwrap_err().code,
            "restore_recovery_required"
        );
        let namespace = OwnerNamespace::isolated(root.path().join("owners"));
        let resolved = resolve_for_bootstrap(&context, &namespace, &options).unwrap();
        assert_eq!(resolved.data_root, data);
        assert_eq!(
            fs::read_to_string(resolved.settings_file).unwrap(),
            "{\"schema_version\":1}"
        );
        assert!(!pending(&data).unwrap());
    }
}

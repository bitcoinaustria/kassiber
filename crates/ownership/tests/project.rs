//! `tests/test_operator_project.py`, ported. Every namespace is a disposable
//! directory; no test selects the account home.
//!
//! Not ported here, with where their behaviour lives:
//! - the two close-error tests (`release`/`ProjectOwnerChildHandles.close`
//!   attempting every handle): Rust closes files in `Drop`, which cannot fail
//!   or be retried, so there is no partial release to test;
//! - the daemon and broker-service cases (`test_broker_first_allows_desktop_owner`,
//!   `test_desktop_first_allows_broker_unlock`,
//!   `test_broker_owner_allows_daemon_database_open`,
//!   `test_daemon_open_uses_the_owned_projects_canonical_root`): the role
//!   coexistence they rely on is pinned below; the daemon's use is covered
//!   by its own crate, and the broker service's by R6-2;
//! - `test_database_maintenance_excludes_the_other_live_role`: maintenance
//!   exclusion lives in `kassiber-store` (`restore.rs`, `encryption.rs`).

use kassiber_ownership::{prune_owner_locks, OwnerKind, OwnerNamespace, PRUNE_MIN_AGE_SECONDS};
use kassiber_paths::ErrorKind;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A child forked by one test holds copies of every descriptor until it
/// execs, so another test's released lock could look held for that moment.
/// Tests in this file run one at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Scratch {
    _serial: std::sync::MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
    root: PathBuf,
    owners: OwnerNamespace,
}

fn scratch() -> Scratch {
    let serial = SERIAL.lock().unwrap_or_else(|poison| poison.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    fs::create_dir(root.join("run")).unwrap();
    let owners = OwnerNamespace::isolated(root.join("run/owners"));
    Scratch {
        _serial: serial,
        _dir: dir,
        root,
        owners,
    }
}

fn book(path: &Path) -> PathBuf {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("kassiber.sqlite3"), b"database").unwrap();
    path.to_path_buf()
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

fn age(path: &Path, seconds: f64) {
    let when = SystemTime::now() - Duration::from_secs_f64(seconds);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_accessed(when).set_modified(when))
        .unwrap();
}

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    path.with_file_name(format!(
        "{}.{suffix}",
        path.file_name().unwrap().to_string_lossy()
    ))
}

#[cfg(unix)]
#[test]
fn symlink_aliases_share_existing_database_identity() {
    let s = scratch();
    let real = book(&s.root.join("real"));
    let alias = s.root.join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    assert_eq!(
        s.owners.canonical_project(&real).unwrap().identity,
        s.owners.canonical_project(&alias).unwrap().identity
    );
}

/// std's canonicalize spells a Windows path verbatim (`\\?\C:\...`); a data
/// root given that way is the same project as its plain spelling.
#[cfg(windows)]
#[test]
fn verbatim_windows_paths_name_the_same_project() {
    let s = scratch();
    let data = book(&s.root.join("verbatim"));
    let verbatim = fs::canonicalize(&data).unwrap();
    assert!(verbatim.to_string_lossy().starts_with(r"\\?\"));
    let plain = s.owners.canonical_project(&data).unwrap();
    let spelled = s.owners.canonical_project(&verbatim).unwrap();
    assert_eq!(spelled.identity, plain.identity);
    assert_eq!(spelled.database, plain.database);
    assert!(!spelled.database.to_string_lossy().starts_with(r"\\?\"));
}

#[test]
fn hardlink_aliases_are_rejected_before_project_ownership() {
    let s = scratch();
    let first = book(&s.root.join("first"));
    let second = s.root.join("second");
    fs::create_dir(&second).unwrap();
    fs::hard_link(
        first.join("kassiber.sqlite3"),
        second.join("kassiber.sqlite3"),
    )
    .unwrap();
    for data_root in [&first, &second] {
        let error = s.owners.canonical_project(data_root).unwrap_err();
        assert_eq!(error.code, "unsafe_project_database");
        assert_eq!(
            error.message,
            "the project database has multiple filesystem links"
        );
        assert_eq!(error.details, serde_json::json!({"link_count": 2}));
        assert!(error
            .hint
            .unwrap()
            .starts_with("Remove every hard-link alias"));
    }
}

#[test]
fn competing_owner_gets_public_safe_owner_state() {
    let s = scratch();
    let data = book(&s.root.join("data"));
    let project = s.owners.canonical_project(&data).unwrap();
    let _lease = s
        .owners
        .acquire(project.clone(), OwnerKind::Broker, "broker-generation")
        .unwrap();
    let error = s
        .owners
        .acquire(project.clone(), OwnerKind::Broker, "broker-test")
        .err()
        .unwrap();
    assert_eq!(error.code, "project_in_use");
    assert!(error.retryable);
    assert_eq!(error.details["project"], project.public_id.as_str());
    assert_eq!(
        error.details["owner"],
        if cfg!(windows) { "unknown" } else { "broker" }
    );
    assert!(!error.details.to_string().contains(&*data.to_string_lossy()));
}

#[test]
fn desktop_and_broker_can_coexist() {
    let s = scratch();
    let data = book(&s.root.join("data"));
    let _desktop = s
        .owners
        .acquire(
            s.owners.canonical_project(&data).unwrap(),
            OwnerKind::Desktop,
            "desktop",
        )
        .unwrap();
    s.owners
        .acquire(
            s.owners.canonical_project(&data).unwrap(),
            OwnerKind::Broker,
            "broker",
        )
        .unwrap();
}

#[test]
fn simultaneous_desktop_and_broker_admission_both_succeed() {
    for round in 0..20 {
        let s = scratch();
        let data = book(&s.root.join(format!("data-{round}")));
        let project = s.owners.canonical_project(&data).unwrap();
        let barrier = std::sync::Barrier::new(2);
        let leases: Vec<_> = std::thread::scope(|scope| {
            [OwnerKind::Desktop, OwnerKind::Broker]
                .map(|role| {
                    let (owners, project, barrier) = (&s.owners, project.clone(), &barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        owners.acquire(project, role, role.as_str())
                    })
                })
                .into_iter()
                .map(|worker| worker.join().unwrap().unwrap())
                .collect()
        });
        assert_eq!(leases.len(), 2);
    }
}

#[test]
fn release_allows_next_owner_and_lock_is_private() {
    let s = scratch();
    let data = book(&s.root.join("data"));
    let project = s.owners.canonical_project(&data).unwrap();
    s.owners
        .acquire(project.clone(), OwnerKind::Broker, "one")
        .unwrap()
        .release();
    s.owners
        .acquire(project.clone(), OwnerKind::Desktop, "two")
        .unwrap()
        .release();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [
            &project.lock_path,
            &project.alias_lock_path,
            &project.local_lock_path,
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

#[test]
fn prune_drops_dead_records_and_spares_live_ones() {
    let s = scratch();
    let dead = s
        .owners
        .canonical_project(&book(&s.root.join("dead")))
        .unwrap();
    s.owners
        .acquire(dead.clone(), OwnerKind::Desktop, "dead")
        .unwrap()
        .release();
    let live = s
        .owners
        .canonical_project(&book(&s.root.join("live")))
        .unwrap();
    let lease = s
        .owners
        .acquire(live.clone(), OwnerKind::Desktop, "live")
        .unwrap();
    // Age everything, so only the held lock, not the floor, spares the live
    // records.
    let root = s.owners.root().to_path_buf();
    for entry in fs::read_dir(&root).unwrap() {
        age(&entry.unwrap().path(), PRUNE_MIN_AGE_SECONDS + 60.0);
    }
    prune_owner_locks(&root, now()).unwrap();
    assert!(!dead.lock_path.exists());
    assert!(!dead.alias_lock_path.exists());
    assert!(!suffixed(&dead.lock_path, "desktop").exists());
    assert!(suffixed(&dead.lock_path, "admission").exists());
    assert!(live.lock_path.exists());
    assert!(suffixed(&live.lock_path, "desktop").exists());
    drop(lease);
}

#[test]
fn prune_spares_records_younger_than_the_age_floor() {
    let s = scratch();
    let project = s
        .owners
        .canonical_project(&book(&s.root.join("data")))
        .unwrap();
    s.owners
        .acquire(project.clone(), OwnerKind::Broker, "recent")
        .unwrap()
        .release();
    let root = s.owners.root().to_path_buf();
    // The acquisition above already claimed today's sweep, so drop the stamp
    // or this asserts against a prune that never ran.
    fs::remove_file(root.join("prune-stamp")).unwrap();
    prune_owner_locks(&root, now()).unwrap();
    assert!(project.lock_path.exists());
    assert!(suffixed(&project.lock_path, "broker").exists());
    // Just inside the floor is still spared; just past it is swept.
    for entry in fs::read_dir(&root).unwrap() {
        age(&entry.unwrap().path(), PRUNE_MIN_AGE_SECONDS - 60.0);
    }
    fs::remove_file(root.join("prune-stamp")).unwrap();
    prune_owner_locks(&root, now()).unwrap();
    assert!(project.lock_path.exists());
    fs::remove_file(root.join("prune-stamp")).unwrap();
    prune_owner_locks(&root, now() + 120.0).unwrap();
    assert!(!project.lock_path.exists());
}

#[test]
fn prune_runs_at_most_once_a_day() {
    let s = scratch();
    let project = s
        .owners
        .canonical_project(&book(&s.root.join("data")))
        .unwrap();
    s.owners
        .acquire(project.clone(), OwnerKind::Broker, "dead")
        .unwrap()
        .release();
    let root = s.owners.root().to_path_buf();
    for entry in fs::read_dir(&root).unwrap() {
        age(&entry.unwrap().path(), PRUNE_MIN_AGE_SECONDS + 60.0);
    }
    // A stamp touched an hour ago blocks the sweep; a day-old one does not.
    age(&root.join("prune-stamp"), 3600.0);
    prune_owner_locks(&root, now()).unwrap();
    assert!(project.lock_path.exists());
    age(&root.join("prune-stamp"), 24.0 * 3600.0 + 1.0);
    prune_owner_locks(&root, now()).unwrap();
    assert!(!project.lock_path.exists());
    let stamp = fs::metadata(root.join("prune-stamp"))
        .unwrap()
        .modified()
        .unwrap();
    assert!(SystemTime::now().duration_since(stamp).unwrap() < Duration::from_secs(60));
}

#[cfg(unix)]
#[test]
fn owner_namespace_uses_account_home_not_runtime_environment() {
    // system() reads the passwd entry, never HOME/XDG/the broker override.
    let home = nix::unistd::User::from_uid(nix::unistd::getuid())
        .unwrap()
        .unwrap()
        .dir;
    assert_eq!(
        OwnerNamespace::system().unwrap().root(),
        home.join(".kassiber/run/operator-owners")
    );
}

#[cfg(unix)]
#[test]
fn owner_namespace_keeps_shared_broker_runtime_private() {
    use std::os::unix::fs::PermissionsExt;
    let s = scratch();
    let account = s.root.join("account");
    fs::create_dir(&account).unwrap();
    let owners = OwnerNamespace::for_account_home(&account);
    let data = book(&s.root.join("data"));
    owners
        .acquire(
            owners.canonical_project(&data).unwrap(),
            OwnerKind::Broker,
            "b",
        )
        .unwrap();
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&account.join(".kassiber/run")), 0o700);
    assert_eq!(mode(&account.join(".kassiber/run/operator-owners")), 0o700);
}

#[test]
fn desktop_and_broker_can_own_a_project_before_its_database_exists() {
    let s = scratch();
    let data = s.root.join("data");
    fs::create_dir(&data).unwrap();
    let project = s.owners.canonical_project(&data).unwrap();
    let desktop = s
        .owners
        .acquire(project.clone(), OwnerKind::Desktop, "desktop")
        .unwrap();
    let broker = s
        .owners
        .acquire(project, OwnerKind::Broker, "broker")
        .unwrap();
    drop(desktop);
    drop(broker);
}

#[cfg(unix)]
#[test]
fn inherited_owner_handles_exclude_a_second_owner_after_parent_release() {
    use kassiber_ipc::process::{ChildCommand, Stdio};
    let s = scratch();
    let data = book(&s.root.join("data"));
    let project = s.owners.canonical_project(&data).unwrap();
    let lease = s
        .owners
        .acquire(project.clone(), OwnerKind::Broker, "broker")
        .unwrap();
    let inherited = lease.duplicate_for_child().unwrap();
    let mut command = ChildCommand::new("/bin/sh");
    command.arg("-c").arg("read line").stdin(Stdio::Pipe);
    for token in inherited.tokens() {
        command.inherit(token);
    }
    let mut child = command.spawn().unwrap();
    inherited.close();
    lease.release();
    let error = s
        .owners
        .acquire(project.clone(), OwnerKind::Broker, "replacement-broker")
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::ProjectInUse);
    child.wait().unwrap();
    s.owners
        .acquire(project, OwnerKind::Broker, "replacement-broker")
        .unwrap();
}

#[cfg(unix)]
#[test]
fn uninherited_children_hold_nothing() {
    use kassiber_ipc::process::{ChildCommand, Stdio};
    let s = scratch();
    let data = book(&s.root.join("data"));
    let project = s.owners.canonical_project(&data).unwrap();
    let lease = s
        .owners
        .acquire(project.clone(), OwnerKind::Broker, "broker")
        .unwrap();
    let duplicates = lease.duplicate_for_child().unwrap();
    let mut command = ChildCommand::new("/bin/sh");
    command.arg("-c").arg("read line").stdin(Stdio::Pipe);
    let mut child = command.spawn().unwrap();
    duplicates.close();
    lease.release();
    s.owners
        .acquire(project, OwnerKind::Broker, "next")
        .unwrap()
        .release();
    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn move_preserves_identity_and_owner_exclusion() {
    let s = scratch();
    let first = book(&s.root.join("first"));
    let second = s.root.join("second");
    fs::create_dir(&second).unwrap();
    let project = s.owners.canonical_project(&first).unwrap();
    let _lease = s
        .owners
        .acquire(project.clone(), OwnerKind::Broker, "broker")
        .unwrap();
    fs::rename(
        first.join("kassiber.sqlite3"),
        second.join("kassiber.sqlite3"),
    )
    .unwrap();
    let moved = s.owners.canonical_project(&second).unwrap();
    assert_eq!(project.identity, moved.identity);
    let error = s
        .owners
        .acquire(moved, OwnerKind::Broker, "replacement-broker")
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::ProjectInUse);
}

#[test]
fn path_lock_blocks_replacement_inode() {
    let s = scratch();
    let data = book(&s.root.join("data"));
    let _lease = s
        .owners
        .acquire(
            s.owners.canonical_project(&data).unwrap(),
            OwnerKind::Broker,
            "broker",
        )
        .unwrap();
    fs::rename(data.join("kassiber.sqlite3"), data.join("old.sqlite3")).unwrap();
    fs::write(data.join("kassiber.sqlite3"), b"replacement").unwrap();
    let error = s
        .owners
        .acquire(
            s.owners.canonical_project(&data).unwrap(),
            OwnerKind::Desktop,
            "desktop",
        )
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::ProjectInUse);
}

#[cfg(unix)]
#[test]
fn legacy_exclusive_owner_blocks_new_role_owner() {
    let s = scratch();
    let data = book(&s.root.join("data"));
    let project = s.owners.canonical_project(&data).unwrap();
    fs::create_dir_all(project.lock_path.parent().unwrap()).unwrap();
    // A legacy single owner: an exclusive flock on the identity lock and its
    // record in the file.
    let legacy = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&project.lock_path)
        .unwrap();
    legacy.try_lock().unwrap();
    use std::io::Write;
    (&legacy)
        .write_all(br#"{"owner": "desktop", "generation": "legacy-desktop"}"#)
        .unwrap();
    let error = s
        .owners
        .acquire(project, OwnerKind::Desktop, "desktop")
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::ProjectInUse);
    assert!(error.message.contains("desktop app or preview"));
    assert!(error.hint.unwrap().contains("second desktop"));
}

#[cfg(unix)]
#[test]
fn new_role_owner_blocks_legacy_exclusive_owner() {
    let s = scratch();
    let data = book(&s.root.join("data"));
    let project = s.owners.canonical_project(&data).unwrap();
    let _owner = s
        .owners
        .acquire(project.clone(), OwnerKind::Broker, "broker")
        .unwrap();
    let legacy = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&project.lock_path)
        .unwrap();
    assert!(matches!(
        legacy.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
}

#[cfg(unix)]
#[test]
fn conflict_names_the_holding_process() {
    let s = scratch();
    let data = book(&s.root.join("data"));
    let project = s.owners.canonical_project(&data).unwrap();
    let lease = s
        .owners
        .acquire(project.clone(), OwnerKind::Desktop, "desktop-one")
        .unwrap();
    let error = s
        .owners
        .acquire(project.clone(), OwnerKind::Desktop, "desktop-two")
        .err()
        .unwrap();
    drop(lease);
    let pid = std::process::id();
    assert_eq!(error.details["pid"], pid);
    assert_eq!(
        error.hint.as_deref(),
        Some(
            format!(
                "Reuse or close the existing desktop app or preview. It is held by pid {pid}. \
A CLI broker can coexist, but a second desktop cannot."
            )
            .as_str()
        )
    );
    assert_eq!(error.details["generation"], "desktop-one");
    assert!(!error.details.to_string().contains(&*data.to_string_lossy()));
    // A broker conflict and an unknown owner name the pid the same way.
    let _broker = s
        .owners
        .acquire(project.clone(), OwnerKind::Broker, "b-one")
        .unwrap();
    let error = s
        .owners
        .acquire(project, OwnerKind::Broker, "b-two")
        .err()
        .unwrap();
    assert_eq!(error.message, "another CLI broker owns this project");
    assert_eq!(
        error.hint.as_deref(),
        Some(
            format!("Reuse or lock the existing CLI broker, then retry. It is held by pid {pid}.")
                .as_str()
        )
    );
}

#[test]
fn ownership_lock_errors_match_python() {
    let s = scratch();
    let data = s.root.join("data");
    fs::create_dir(&data).unwrap();
    fs::create_dir(data.join("kassiber.sqlite3")).unwrap();
    let error = s.owners.canonical_project(&data).unwrap_err();
    assert_eq!(
        (error.code, error.message),
        (
            "unsafe_project_database",
            "the project database is not a regular file"
        )
    );
    #[cfg(unix)]
    {
        // A directory where a lock file belongs.
        let data = book(&s.root.join("locked"));
        let project = s.owners.canonical_project(&data).unwrap();
        fs::create_dir(&project.local_lock_path).unwrap();
        let error = s
            .owners
            .acquire(project.clone(), OwnerKind::Broker, "b")
            .err()
            .unwrap();
        assert_eq!(error.code, "project_owner_lock_unavailable");
        assert_eq!(error.message, "the project ownership lock is unavailable");
        assert_eq!(
            error.details,
            serde_json::json!({"project": project.public_id})
        );
        assert!(error.retryable);
    }
}

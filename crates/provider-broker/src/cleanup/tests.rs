//! Ports of `cleanup.test.ts`, on private registries.

use super::*;

fn registry() -> &'static Cleanup {
    Box::leak(Box::new(Cleanup::new()))
}

#[cfg(unix)]
fn sleeper() -> Child {
    crate::executables::provider_command(
        crate::protocol::ProviderId::Codex,
        Path::new("/bin/sh"),
        &["-c", "exec sleep 60"],
        None,
        None,
    )
    .spawn()
    .unwrap()
}

#[cfg(unix)]
#[test]
fn stops_agents_and_removes_per_turn_directories() {
    use std::os::unix::process::ExitStatusExt;
    // What a signalled broker leaves behind otherwise: a working directory
    // whose agent home holds the session log.
    let registry = registry();
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("kassiber-ai-cleanup");
    std::fs::create_dir(&cwd).unwrap();
    std::fs::write(cwd.join("events.jsonl"), "{}\n").unwrap();
    let _tracked = registry.track_directory(&cwd);
    let child = registry.track_child(sleeper());

    registry.clean_up_now();

    let status = child
        .wait_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(status.signal(), Some(9));
    assert!(!cwd.exists());
}

#[test]
fn forgets_a_directory_once_its_turn_removed_it() {
    let registry = registry();
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("kassiber-ai-cleanup");
    std::fs::create_dir(&cwd).unwrap();
    registry.track_directory(&cwd).untrack();
    registry.clean_up_now();
    assert!(cwd.exists());
}

#[cfg(unix)]
#[test]
fn forgets_an_exited_child_and_never_signals_it_again() {
    let registry = registry();
    let child = crate::executables::provider_command(
        crate::protocol::ProviderId::Codex,
        Path::new("/bin/sh"),
        &["-c", "exit 3"],
        None,
        None,
    )
    .spawn()
    .unwrap();
    let child = registry.track_child(child);
    assert_eq!(child.wait().unwrap().code(), Some(3));
    assert!(registry.entries().children.is_empty());
    // Killing or terminating a reaped child is a no-op, not a stray signal.
    child.kill().unwrap();
    child.terminate().unwrap();
}

#[cfg(unix)]
#[test]
fn a_dropped_running_child_stays_tracked_until_cleanup() {
    use std::os::unix::process::ExitStatusExt;
    let registry = registry();
    let child = registry.track_child(sleeper());
    let pid = child.id();
    drop(child);
    assert_eq!(registry.entries().children.len(), 1);
    let tracked = Arc::clone(&registry.entries().children[0].1);
    registry.clean_up_now();
    let status = tracked.lock().unwrap().wait().unwrap();
    assert_eq!(status.signal(), Some(9), "pid {pid}");
}

#[cfg(unix)]
#[test]
fn terminate_sends_sigterm() {
    use std::os::unix::process::ExitStatusExt;
    let registry = registry();
    let child = registry.track_child(sleeper());
    child.terminate().unwrap();
    let status = child
        .wait_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(status.signal(), Some(15));
}

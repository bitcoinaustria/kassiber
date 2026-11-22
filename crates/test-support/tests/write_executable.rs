#![cfg(unix)]

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use kassiber_test_support::write_executable;

#[test]
fn a_written_script_runs_while_other_threads_fork() {
    let dir = tempfile::tempdir().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let forkers: Vec<_> = (0..8)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    Command::new("/bin/sh").arg("-c").arg(":").status().unwrap();
                }
            })
        })
        .collect();
    let runs = (0..300).map(|n| {
        let script = dir.path().join(format!("script-{n}"));
        write_executable(&script, format!("#!/bin/sh\necho {n}\n")).unwrap();
        Command::new(&script).output()
    });
    let failures: Vec<_> = runs.filter_map(Result::err).collect();
    stop.store(true, Ordering::Relaxed);
    for forker in forkers {
        forker.join().unwrap();
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn rewriting_a_script_replaces_it_with_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("script");
    write_executable(&script, "#!/bin/sh\necho first\n").unwrap();
    write_executable(&script, "#!/bin/sh\necho second\n").unwrap();
    let output = Command::new(&script).output().unwrap();
    assert_eq!(output.stdout, b"second\n");
    let mode = std::fs::metadata(&script).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

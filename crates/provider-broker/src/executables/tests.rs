//! Ports of the executable cases in `broker.test.ts`.

use super::*;

#[test]
fn finds_a_windows_cli_by_its_exe_name_never_by_a_shell_script() {
    let names = |pathext| executable_names("copilot", true, pathext);
    assert_eq!(
        names(Some(".COM;.EXE;.BAT;.CMD;.VBS;.JS;.PS1")),
        ["copilot.com", "copilot.exe"]
    );
    assert_eq!(names(None), ["copilot.com", "copilot.exe"]);
    assert_eq!(names(Some(".CMD;.BAT")), ["copilot.exe"]);
    assert_eq!(names(Some("")), ["copilot.exe"]);
    assert_eq!(executable_names("copilot.exe", true, None), ["copilot.exe"]);
}

#[test]
fn uses_the_bare_name_elsewhere() {
    assert_eq!(executable_names("copilot", false, None), ["copilot"]);
    assert_eq!(
        executable_names("copilot", false, Some(".EXE")),
        ["copilot"]
    );
}

#[test]
fn an_extension_is_a_dot_and_a_tail_without_separators() {
    assert!(has_extension("a.exe"));
    assert!(!has_extension("a."));
    assert!(!has_extension("dir.d/copilot"));
    assert!(!has_extension(r"dir.d\copilot"));
    assert!(!has_extension("copilot"));
}

#[cfg(unix)]
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn detects_an_executable_through_path_and_reports_missing_binaries() {
    let root = tempfile::tempdir().unwrap();
    let command = script(root.path(), "kassiber-test-provider", "exit 0\n");
    let path = Some(root.path().as_os_str());
    assert_eq!(
        resolve_in("kassiber-test-provider", path, &[]),
        Some(command.clone())
    );
    assert_eq!(resolve_in("definitely-not-a-provider", path, &[]), None);
    // A file without the execute bit is not a candidate.
    std::fs::write(root.path().join("plain"), "x").unwrap();
    assert_eq!(resolve_in("plain", path, &[]), None);
    // The fallback directories come after PATH, each directory once.
    let other = tempfile::tempdir().unwrap();
    let fallback = script(other.path(), "kassiber-test-fallback", "exit 0\n");
    assert_eq!(
        resolve_in(
            "kassiber-test-fallback",
            path,
            &[other.path().to_path_buf(), other.path().to_path_buf()]
        ),
        Some(fallback)
    );
    let both = std::env::join_paths([other.path(), root.path()]).unwrap();
    let shadow = script(other.path(), "kassiber-test-provider", "exit 0\n");
    assert_eq!(
        resolve_in("kassiber-test-provider", Some(&both), &[]),
        Some(shadow)
    );
    // Empty PATH entries are skipped, not read as the current directory.
    assert_eq!(
        resolve_in("kassiber-test-provider", Some(OsStr::new("::")), &[]),
        None
    );
}

#[cfg(unix)]
#[test]
fn captures_a_providers_full_stdout_even_when_it_exits_immediately() {
    // Regression: settling on the child's exit loses stdout still queued
    // behind it, so a probe that succeeded looks like an unconfigured
    // provider.
    let root = tempfile::tempdir().unwrap();
    // Write a payload far larger than one pipe buffer, then exit at once.
    let command = script(
        root.path(),
        "kassiber-test-chatty",
        "awk 'BEGIN{for(i=0;i<8000;i++)print \"0123456789012345678901234\"}'\n",
    );
    for _ in 0..5 {
        let output = run_provider(
            ProviderId::Codex,
            &command,
            &[] as &[&str],
            RunOptions::default(),
        )
        .unwrap();
        assert_eq!(output.code, Some(0));
        assert_eq!(output.stdout.len(), 8000 * 26);
    }
}

#[cfg(unix)]
#[test]
fn runs_with_the_provider_environment_and_discards_stderr() {
    let root = tempfile::tempdir().unwrap();
    let command = script(
        root.path(),
        "kassiber-test-env",
        "echo secret-on-stderr >&2\nenv | sort\nexit 7\n",
    );
    let env = vec![
        (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
        (OsString::from("KASSIBER_AI_BROKER"), OsString::from("1")),
    ];
    let output = run_provider(
        ProviderId::Codex,
        &command,
        &[] as &[&str],
        RunOptions {
            cwd: Some(root.path()),
            env: Some(&env),
            ..RunOptions::default()
        },
    )
    .unwrap();
    assert_eq!(output.code, Some(7));
    assert!(
        output.stdout.contains("KASSIBER_AI_BROKER=1"),
        "{}",
        output.stdout
    );
    assert!(!output.stdout.contains("secret-on-stderr"));
    assert!(!output.stdout.contains("HOME="), "{}", output.stdout);
}

#[cfg(unix)]
#[test]
fn keeps_stdout_up_to_the_limit_and_drains_the_rest() {
    let root = tempfile::tempdir().unwrap();
    let command = script(
        root.path(),
        "kassiber-test-flood",
        "awk 'BEGIN{for(i=0;i<100000;i++)print \"0123456789\"}'\n",
    );
    let output = run_provider(
        ProviderId::Claude,
        &command,
        &[] as &[&str],
        RunOptions {
            limit: Some(10),
            ..RunOptions::default()
        },
    )
    .unwrap();
    assert_eq!(output.code, Some(0));
    // The first chunk is kept whole, then nothing more.
    assert!(output.stdout.len() >= 10 && output.stdout.len() < 1_100_000);
}

#[cfg(unix)]
#[test]
fn a_failed_spawn_reads_as_node_names_it() {
    let missing = Path::new("/nonexistent/kassiber-provider");
    let failure = run_provider(
        ProviderId::Codex,
        missing,
        &[] as &[&str],
        RunOptions::default(),
    )
    .unwrap_err();
    assert_eq!(
        failure.message(),
        "spawn /nonexistent/kassiber-provider ENOENT"
    );
    assert_eq!(
        crate::protocol::error_code(failure.message()),
        crate::protocol::ErrorCode::MissingExecutable
    );
    let root = tempfile::tempdir().unwrap();
    let plain = root.path().join("not-executable");
    std::fs::write(&plain, "x").unwrap();
    let failure = run_provider(
        ProviderId::Codex,
        &plain,
        &[] as &[&str],
        RunOptions::default(),
    )
    .unwrap_err();
    assert!(failure.message().ends_with(" EACCES"), "{failure}");
}

#[cfg(unix)]
#[test]
fn provider_clis_still_stop_on_the_daemons_signals() {
    // The broker catches SIGTERM, SIGINT and SIGHUP for its cleanup. A
    // provider CLI must not inherit that: handlers reset at exec, and no
    // signal may be blocked (std does not reset an inherited mask, which is
    // why the broker does not block them).
    if !Path::new("/proc/self/status").exists() {
        return;
    }
    kassiber_ipc::signal::install_termination_signals().unwrap();
    let output = run_provider(
        ProviderId::Codex,
        Path::new("/bin/sh"),
        &["-c", "exec grep -E '^Sig(Blk|Ign|Cgt)' /proc/self/status"],
        RunOptions::default(),
    )
    .unwrap();
    let mask = |name: &str| {
        let line = output
            .stdout
            .lines()
            .find(|line| line.starts_with(name))
            .unwrap();
        u64::from_str_radix(line.split('\t').nth(1).unwrap(), 16).unwrap()
    };
    let termination = (1 << (15 - 1)) | (1 << (2 - 1)) | (1 << (1 - 1));
    assert_eq!(mask("SigBlk") & termination, 0, "{}", output.stdout);
    assert_eq!(mask("SigIgn") & termination, 0, "{}", output.stdout);
    assert_eq!(mask("SigCgt") & termination, 0, "{}", output.stdout);
}

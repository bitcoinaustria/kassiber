//! The short-lived direct CLI child each dispatched operation runs in
//! (`kassiber/operator/runner.py`).
//!
//! The child is this executable again, started from its own directory (never
//! the caller's), with the caller's directory and bindings in its
//! environment. The lease passphrase and every command-specific secret cross
//! anonymous pipes the child inherits by number; argv carries only those
//! numbers. Every pipe is fed on its own thread while stdout and stderr are
//! drained, so a full pipe cannot deadlock a later secret handoff. Launches
//! go through `kassiber_ipc::process::ChildCommand`, which on Windows passes
//! only the listed handles.

use crate::error::{Error, Result};
use crate::service::{Backend, Dispatch, OperationResult, Runner};
use kassiber_ipc::process::{ChildCommand, Inheritable, Stdio};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

pub const CALLER_WORKING_DIRECTORY_ENV: &str = "KASSIBER_OPERATOR_WORKING_DIRECTORY";

/// `run_cli_operation`.
pub struct CliRunner {
    backend: Arc<dyn Backend>,
    program: PathBuf,
    launch_directory: PathBuf,
}

impl CliRunner {
    /// Children run `program`, started in `launch_directory`
    /// (`trusted_launch_directory()`).
    pub fn new(backend: Arc<dyn Backend>, program: PathBuf, launch_directory: PathBuf) -> Self {
        CliRunner {
            backend,
            program,
            launch_directory,
        }
    }

    /// This executable, started from its own directory.
    pub fn current(backend: Arc<dyn Backend>) -> Result<Self> {
        let program = std::env::current_exe()?;
        let launch_directory = program
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| Error::Value("the executable has no directory".into()))?;
        Ok(Self::new(backend, program, launch_directory))
    }
}

/// `strip_database_passphrase_arguments`: remove the caller's database
/// descriptors so the lease is authoritative; report the removed values.
pub fn strip_database_passphrase_arguments(argv: &[String]) -> (Vec<String>, HashSet<String>) {
    let mut prepared = Vec::new();
    let mut removed = HashSet::new();
    let mut index = 0;
    while index < argv.len() {
        let token = &argv[index];
        if token == "--db-passphrase-fd" {
            if let Some(value) = argv.get(index + 1) {
                removed.insert(value.clone());
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(value) = token.strip_prefix("--db-passphrase-fd=") {
            removed.insert(value.to_owned());
            index += 1;
            continue;
        }
        prepared.push(token.clone());
        index += 1;
    }
    (prepared, removed)
}

/// The number a child is told for an inherited pipe end.
fn token_text(token: Inheritable) -> String {
    #[cfg(unix)]
    {
        token.to_string()
    }
    #[cfg(windows)]
    {
        (token as usize).to_string()
    }
}

#[cfg(unix)]
fn token_of(reader: &std::io::PipeReader) -> Inheritable {
    use std::os::fd::AsRawFd;
    reader.as_raw_fd()
}

#[cfg(windows)]
fn token_of(reader: &std::io::PipeReader) -> Inheritable {
    use std::os::windows::io::AsRawHandle;
    reader.as_raw_handle()
}

/// `process.returncode`: a signal is its negative number.
fn exit_code(status: std::process::ExitStatus) -> i64 {
    if let Some(code) = status.code() {
        return i64::from(code);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return -i64::from(signal);
        }
    }
    -1
}

impl Runner for CliRunner {
    fn run(&self, operation: &Dispatch, passphrase: &[u8]) -> Result<OperationResult> {
        if self
            .backend
            .canonical_project(&operation.data_root)?
            .identity
            != operation.project_identity
        {
            return Err(Error::Value(
                "operator project changed before child launch".into(),
            ));
        }
        if operation
            .working_directory
            .as_deref()
            .is_some_and(|directory| !std::path::Path::new(directory).is_dir())
        {
            // Never fall back to the broker's directory: a relative output
            // or proposal path would silently point at a different file.
            return Err(Error::Value(
                "operator caller working directory changed before child launch".into(),
            ));
        }
        let (stripped, removed) = strip_database_passphrase_arguments(&operation.argv);
        let mut readers = Vec::new();
        let mut writers: Vec<(std::io::PipeWriter, &[u8])> = Vec::new();
        let (db_reader, db_writer) = std::io::pipe()?;
        let db_token = token_of(&db_reader);
        writers.push((db_writer, passphrase));
        readers.push(db_reader);
        let mut argv = vec!["--db-passphrase-fd".to_owned(), token_text(db_token)];
        argv.extend(stripped);
        for (marker, secret) in &operation.secret_arguments {
            if removed.contains(marker) && !argv.contains(marker) {
                continue;
            }
            let (reader, writer) = std::io::pipe()?;
            let token = token_text(token_of(&reader));
            writers.push((writer, secret.as_slice()));
            readers.push(reader);
            for value in &mut argv {
                if value == marker {
                    *value = token.clone();
                }
            }
        }
        let mut command = ChildCommand::new(&self.program);
        command
            .args(&argv)
            .stdin(Stdio::Null)
            .stdout(Stdio::Pipe)
            .stderr(Stdio::Pipe)
            .env("KASSIBER_OPERATOR_DIRECT", "1")
            .env("KASSIBER_OPERATOR_CHILD", "1")
            .env(
                "KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY",
                &operation.project_identity,
            )
            .env(
                "KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY",
                &operation.database_identity,
            )
            // Starting in the caller's directory would let it shadow the
            // install; the child changes there once it runs.
            .current_dir(&self.launch_directory);
        if operation.no_egress {
            // The caller's kill switch can only tighten the broker's own.
            command.env("KASSIBER_NO_EGRESS", "1");
        }
        if let Some(directory) = &operation.working_directory {
            command.env(CALLER_WORKING_DIRECTORY_ENV, directory);
        }
        for reader in &readers {
            command.inherit(token_of(reader));
        }
        for token in &operation.owner_tokens {
            command.inherit(*token);
        }
        let mut child = command.spawn()?;
        drop(readers);
        let outcome = std::thread::scope(|scope| -> Result<OperationResult> {
            let feeders: Vec<_> = writers
                .drain(..)
                .map(|(mut writer, secret)| {
                    scope.spawn(move || -> std::io::Result<()> {
                        let written = writer.write_all(secret);
                        drop(writer);
                        written
                    })
                })
                .collect();
            let mut stderr_pipe = child.stderr.take();
            let stderr = scope.spawn(move || -> std::io::Result<Vec<u8>> {
                let mut buffer = Vec::new();
                if let Some(pipe) = stderr_pipe.as_mut() {
                    pipe.read_to_end(&mut buffer)?;
                }
                Ok(buffer)
            });
            let mut stdout = Vec::new();
            let read = match child.stdout.take() {
                Some(mut pipe) => pipe.read_to_end(&mut stdout).map(|_| ()),
                None => Ok(()),
            };
            let stderr = stderr
                .join()
                .unwrap_or_else(|_| Err(std::io::Error::other("stderr reader panicked")));
            let status = child.wait();
            let mut first_feed_error = None;
            for feeder in feeders {
                if let Err(error) = feeder
                    .join()
                    .unwrap_or_else(|_| Err(std::io::Error::other("secret feeder panicked")))
                {
                    first_feed_error.get_or_insert(error);
                }
            }
            read?;
            let stderr = stderr?;
            let status = status?;
            if let Some(error) = first_feed_error {
                return Err(Error::Io(error));
            }
            Ok(OperationResult::new(
                exit_code(status),
                String::from_utf8_lossy(&stdout).into_owned(),
                String::from_utf8_lossy(&stderr).into_owned(),
            ))
        });
        if outcome.is_err() && matches!(child.try_wait(), Ok(None)) {
            let _ = child.kill();
            let _ = child.wait();
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn caller_database_descriptors_are_stripped() {
        let (prepared, removed) = strip_database_passphrase_arguments(&argv(&[
            "--db-passphrase-fd",
            "broker-secret-a",
            "--machine",
            "--db-passphrase-fd=7",
            "status",
            "--db-passphrase-fd",
        ]));
        assert_eq!(prepared, argv(&["--machine", "status"]));
        assert_eq!(
            removed,
            ["broker-secret-a".to_owned(), "7".to_owned()]
                .into_iter()
                .collect()
        );
    }

    /// A stand-in child: prints its argv, cwd and broker variables, and
    /// each inherited secret descriptor's contents. It reads them through
    /// `/dev/fd/N`: dash, Ubuntu's `/bin/sh`, refuses `<&N` above 9.
    #[cfg(unix)]
    fn script(directory: &std::path::Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("child.sh");
        std::fs::write(
            &path,
            "#!/bin/sh\n\
             echo \"argv=$*\"\n\
             echo \"cwd=$(pwd -P)\"\n\
             echo \"caller=$KASSIBER_OPERATOR_WORKING_DIRECTORY\"\n\
             echo \"egress=$KASSIBER_NO_EGRESS\"\n\
             echo \"child=$KASSIBER_OPERATOR_CHILD direct=$KASSIBER_OPERATOR_DIRECT\"\n\
             echo \"project=$KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY\"\n\
             echo \"database=$KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY\"\n\
             echo \"lease=$(cat /dev/fd/$2)\"\n\
             while [ $# -gt 0 ]; do if [ \"$1\" = --token-fd ]; then echo \"secret=$(cat /dev/fd/$2)\"; fi; shift; done\n\
             echo oops >&2\n\
             exit 3\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    fn dispatch(
        fake: &crate::service::tests::Fake,
        data_root: &str,
        working_directory: Option<String>,
    ) -> Dispatch {
        let project = fake.canonical_project(data_root).unwrap();
        Dispatch {
            id: "generation.caller-context".into(),
            project_identity: project.identity,
            database_identity: "d".repeat(32),
            data_root: data_root.into(),
            argv: argv(&[
                "--db-passphrase-fd",
                "broker-secret-caller",
                "--machine",
                "status",
                "--token-fd",
                "broker-secret-x",
            ]),
            command_path: "status".into(),
            capability: crate::service::Capability::Read,
            secret_arguments: vec![
                (
                    "broker-secret-caller".into(),
                    zeroize::Zeroizing::new(b"caller".to_vec()),
                ),
                (
                    "broker-secret-x".into(),
                    zeroize::Zeroizing::new(vec![b'x'; 70_000]),
                ),
            ],
            working_directory,
            no_egress: true,
            owner_tokens: Vec::new(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn worker_child_gets_caller_directory_and_tightened_egress() {
        use crate::service::tests::Fake;
        let book = tempfile::tempdir().unwrap();
        let caller = tempfile::tempdir().unwrap();
        let launch = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let runner = CliRunner::new(
            Arc::clone(&fake) as Arc<dyn Backend>,
            script(launch.path()),
            launch.path().to_owned(),
        );
        let data_root = book.path().to_string_lossy().into_owned();
        let operation = dispatch(
            &fake,
            &data_root,
            Some(caller.path().to_string_lossy().into_owned()),
        );
        let result = runner.run(&operation, b"lease-passphrase").unwrap();
        assert_eq!(result.exit_code, 3);
        assert_eq!(result.stderr, "oops\n");
        let lines: Vec<&str> = result.stdout.lines().collect();
        // The caller's own database descriptor is gone; the lease's comes first.
        assert!(
            lines[0].starts_with("argv=--db-passphrase-fd "),
            "{lines:?}"
        );
        assert!(!lines[0].contains("broker-secret"), "{lines:?}");
        assert!(
            lines[0].contains(" --machine status --token-fd "),
            "{lines:?}"
        );
        // The launch directory is the broker's, never the caller's.
        assert_eq!(
            lines[1],
            format!(
                "cwd={}",
                std::fs::canonicalize(launch.path()).unwrap().display()
            )
        );
        assert_eq!(lines[2], format!("caller={}", caller.path().display()));
        assert_eq!(lines[3], "egress=1");
        assert_eq!(lines[4], "child=1 direct=1");
        assert_eq!(lines[5], format!("project={}", operation.project_identity));
        assert_eq!(lines[6], format!("database={}", "d".repeat(32)));
        assert_eq!(lines[7], "lease=lease-passphrase");
        // A secret larger than a pipe's buffer arrives whole.
        assert_eq!(lines[8], format!("secret={}", "x".repeat(70_000)));
    }

    #[cfg(unix)]
    #[test]
    fn worker_refuses_when_the_callers_directory_disappeared() {
        use crate::service::tests::Fake;
        let book = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let launch = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let program = script(launch.path());
        let runner = CliRunner::new(
            Arc::clone(&fake) as Arc<dyn Backend>,
            program,
            launch.path().to_owned(),
        );
        let data_root = book.path().to_string_lossy().into_owned();
        let gone = root.path().join("gone").to_string_lossy().into_owned();
        let error = runner
            .run(&dispatch(&fake, &data_root, Some(gone)), b"lease")
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "operator caller working directory changed before child launch"
        );
        // A replaced project is refused before launch too.
        let mut moved = dispatch(&fake, &data_root, None);
        moved.project_identity = "f".repeat(64);
        let error = runner.run(&moved, b"lease").unwrap_err();
        assert_eq!(
            error.to_string(),
            "operator project changed before child launch"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_signalled_child_reports_the_negative_signal() {
        use crate::service::tests::Fake;
        use std::os::unix::fs::PermissionsExt;
        let book = tempfile::tempdir().unwrap();
        let launch = tempfile::tempdir().unwrap();
        let program = launch.path().join("killed.sh");
        // It takes its lease first: a child that dies before the broker
        // has written it fails the handoff with a broken pipe instead.
        std::fs::write(
            &program,
            "#!/bin/sh\ncat /dev/fd/$2 >/dev/null\nkill -9 $$\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let fake = Fake::new();
        let runner = CliRunner::new(
            Arc::clone(&fake) as Arc<dyn Backend>,
            program,
            launch.path().to_owned(),
        );
        let data_root = book.path().to_string_lossy().into_owned();
        let mut operation = dispatch(&fake, &data_root, None);
        operation.secret_arguments.clear();
        let result = runner.run(&operation, b"lease").unwrap();
        assert_eq!(result.exit_code, -9);
    }
}

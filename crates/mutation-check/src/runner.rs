//! Running a suite in a scratch copy of the working tree.
//!
//! The scratch tree lives at `target/mutation-scratch/<suite>`; every suite
//! builds into the one shared `target/scratch-build`, so third-party crates
//! compile once. Copies get the current time as their modification time, so
//! cargo never mistakes the previous run's last mutant for a fresh build.

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crate::{is_pytest_id, problems, Error, Mutation, Suite};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Passed,
    Failed,
    /// It selected no test, could not run, or timed out.
    Error,
}

pub struct Outcome {
    pub status: Status,
    pub output: String,
}

/// `cargo test` result lines: (passed, failed) per test binary.
pub fn cargo_counts(stdout: &str) -> Vec<(u64, u64)> {
    stdout
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("test result: ")?;
            let (_, rest) = rest.split_once(". ")?;
            let mut numbers = rest
                .split("; ")
                .map(|part| part.split(' ').next()?.parse::<u64>().ok());
            Some((numbers.next()??, numbers.next()??))
        })
        .collect()
}

/// A `package[/binary]:name` filter selects its own target.
pub fn package_filter(test: &str) -> Option<(String, Option<String>, String)> {
    if test.contains("::") {
        return None;
    }
    let (target, name) = test.split_once(':')?;
    let (package, binary) = match target.split_once('/') {
        Some((package, binary)) => (package, Some(binary.to_owned())),
        None => (target, None),
    };
    let valid = !package.is_empty()
        && package
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| (package.to_owned(), binary, name.to_owned()))
}

/// Files git would carry: tracked, plus untracked but not ignored.
pub fn worktree_files(root: &Path) -> Result<Vec<String>, Error> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .map_err(|e| Error(format!("git ls-files: {e}")))?;
    if !output.status.success() {
        return Err(Error(format!(
            "git ls-files: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let mut names: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

pub fn copy_worktree(root: &Path, scratch: &Path) -> Result<(), Error> {
    for name in worktree_files(root)? {
        let source = root.join(&name);
        let Ok(meta) = source.symlink_metadata() else {
            continue; // deleted in the worktree, still in the index
        };
        let target = scratch.join(&name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| Error(format!("{}: {e}", parent.display())))?;
        }
        if meta.file_type().is_symlink() {
            let link = fs::read_link(&source).map_err(|e| Error(format!("{name}: {e}")))?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link, &target)
                .map_err(|e| Error(format!("{name}: {e}")))?;
            #[cfg(not(unix))]
            return Err(Error(format!(
                "{name}: symlink {} in the tree",
                link.display()
            )));
        } else if meta.is_file() {
            fs::copy(&source, &target).map_err(|e| Error(format!("{name}: {e}")))?;
        }
    }
    for name in locked_test_data(root) {
        let source = root.join(&name);
        if !source.is_file() {
            return Err(Error(format!(
                "{name} is missing: run scripts/fetch-test-data.sh first"
            )));
        }
        let target = scratch.join(&name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| Error(format!("{}: {e}", parent.display())))?;
        }
        fs::copy(&source, &target).map_err(|e| Error(format!("{name}: {e}")))?;
    }
    Ok(())
}

/// The generated test data `tests/test-data.lock` pins. Git ignores it, so
/// the scratch tree gets it from the worktree's fetched copy.
pub fn locked_test_data(root: &Path) -> Vec<String> {
    let Ok(lock) = fs::read_to_string(root.join("tests/test-data.lock")) else {
        return vec![];
    };
    lock.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let (blob, path) = line.split_once(' ')?;
            (blob.len() == 40 && blob.chars().all(|c| c.is_ascii_hexdigit()))
                .then(|| path.to_owned())
        })
        .collect()
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| match e.file_type() {
            Ok(kind) if kind.is_dir() => dir_size(&e.path()),
            Ok(kind) if kind.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

pub struct Runner<'a> {
    suite: &'a Suite,
    root: PathBuf,
    scratch: PathBuf,
    build: PathBuf,
}

fn run_command(mut command: Command, timeout: Duration) -> (Option<i32>, String, String) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return (None, String::new(), format!("could not start: {error}")),
    };
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let out = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stdout.read_to_end(&mut text);
        String::from_utf8_lossy(&text).into_owned()
    });
    let err = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stderr.read_to_end(&mut text);
        String::from_utf8_lossy(&text).into_owned()
    });
    let (done_tx, done_rx) = mpsc::channel();
    let id = child.id();
    let waiter = std::thread::spawn(move || {
        let status = child.wait();
        let _ = done_tx.send(());
        status
    });
    let timed_out = done_rx.recv_timeout(timeout).is_err();
    if timed_out {
        // Our own child, by PID.
        let _ = Command::new("kill").arg("-9").arg(id.to_string()).status();
    }
    let status = waiter
        .join()
        .ok()
        .and_then(|s| s.ok())
        .and_then(|s| s.code());
    let stdout = out.join().unwrap_or_default();
    let mut stderr = err.join().unwrap_or_default();
    if timed_out {
        stderr.push_str(&format!("\ntimed out after {}s", timeout.as_secs()));
        return (None, stdout, stderr);
    }
    (status, stdout, stderr)
}

fn tail(text: &str, bytes: usize) -> String {
    let start = text.len().saturating_sub(bytes);
    let start = (start..=text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(text.len());
    text[start..].to_owned()
}

impl<'a> Runner<'a> {
    /// `target` must be the worktree's resolved `target/` directory.
    pub fn new(suite: &'a Suite, root: &Path, target: &Path) -> Self {
        Self {
            suite,
            root: root.to_path_buf(),
            // One tree path for every suite: cargo keys the workspace crates'
            // artifacts on their path, so per-suite trees would each keep a
            // full copy in the shared build dir.
            scratch: target.join("mutation-scratch").join("tree"),
            build: target.join("scratch-build"),
        }
    }

    pub fn scratch(&self) -> &Path {
        &self.scratch
    }

    fn cargo(&self, args: &[String]) -> Command {
        let mut command = Command::new("cargo");
        command.current_dir(&self.scratch);
        command.args(args);
        // Tests run on production; nothing points them at another engine.
        for (key, _) in std::env::vars() {
            if key.starts_with("KASSIBER_TEST_TAX_") || key.starts_with("KASSIBER_TEST_CUSTODY_") {
                command.env_remove(key);
            }
        }
        command
            .env("CARGO_TARGET_DIR", self.build.join("target"))
            .env("CARGO_BUILD_BUILD_DIR", &self.build)
            .env("TMPDIR", self.scratch.join("tmp"))
            // Test processes never resolve the account's app-data root.
            .env("XDG_DATA_HOME", self.scratch.join("tmp/xdg-data"))
            .env(
                "KASSIBER_OFFLINE_PREFERENCE_FILE",
                self.scratch.join("tmp/offline-preference.json"),
            )
            .envs(&self.suite.env);
        command
    }

    fn base_args(&self, verb: &str, cargo: &[String]) -> Vec<String> {
        let mut args = vec![verb.to_owned()];
        args.extend(cargo.iter().cloned());
        for flag in ["--offline", "--locked"] {
            if !args.iter().any(|a| a == flag) {
                args.push(flag.to_owned());
            }
        }
        args
    }

    fn resolve_target(&self, test: &str) -> Result<Vec<String>, Error> {
        let Some(resolve) = &self.suite.resolve_target else {
            return Ok(vec![]);
        };
        let name = test.rsplit("::").next().unwrap_or(test);
        let defines = |text: &str| {
            text.match_indices("fn ").any(|(at, _)| {
                let rest = &text[at + 3..];
                rest.strip_prefix(name)
                    .is_some_and(|after| after.trim_start().starts_with('('))
            })
        };
        let mut owners = Vec::new();
        let dir = self.scratch.join(&resolve.tests_dir);
        let mut files: Vec<PathBuf> = fs::read_dir(&dir)
            .map_err(|e| Error(format!("{}: {e}", dir.display())))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "rs"))
            .collect();
        files.sort();
        for file in files {
            if defines(&fs::read_to_string(&file).unwrap_or_default()) {
                owners.push(file.file_stem().unwrap().to_string_lossy().into_owned());
            }
        }
        for (target, file) in &resolve.extra {
            if defines(&fs::read_to_string(self.scratch.join(file)).unwrap_or_default()) {
                owners.push(target.clone());
            }
        }
        match owners.as_slice() {
            [] => Ok(vec!["--lib".into()]),
            [one] => Ok(vec!["--test".into(), one.clone()]),
            _ => Err(Error(format!("ambiguous test filter {test}: {owners:?}"))),
        }
    }

    /// The `cargo test` arguments that run one filter.
    pub fn test_command(&self, mutation: &Mutation, test: &str) -> Result<Vec<String>, Error> {
        let mut args = if let Some((package, binary, name)) = package_filter(test) {
            let mut args = self.base_args("test", &["-p".into(), package]);
            match binary {
                Some(binary) => args.extend(["--test".into(), binary]),
                None => args.push("--lib".into()),
            }
            args.extend(["--".into(), name]);
            return Ok(args);
        } else {
            self.base_args("test", &self.suite.cargo_args(mutation)?)
        };
        args.extend(self.resolve_target(test)?);
        args.push("--".into());
        args.push(test.to_owned());
        args.extend(self.suite.test_args.iter().cloned());
        Ok(args)
    }

    pub fn outcome(&self, mutation: &Mutation, test: &str) -> Outcome {
        if is_pytest_id(test) {
            return Outcome {
                status: Status::Error,
                output: format!("{test} is a pytest node id; give the entry a Rust killer"),
            };
        }
        let args = match self.test_command(mutation, test) {
            Ok(args) => args,
            Err(error) => {
                return Outcome {
                    status: Status::Error,
                    output: error.0,
                }
            }
        };
        let (code, stdout, stderr) = run_command(
            self.cargo(&args),
            Duration::from_secs(self.suite.timeout_secs),
        );
        let selected: u64 = cargo_counts(&stdout).iter().map(|(p, f)| p + f).sum();
        let status = match (code, selected) {
            (_, 0) | (None, _) => Status::Error,
            (Some(0), _) => Status::Passed,
            (Some(_), _) => Status::Failed,
        };
        Outcome {
            status,
            output: tail(
                &format!("{} {}\n{stdout}{stderr}", "cargo", args.join(" ")),
                3000,
            ),
        }
    }

    /// Whether the mutated tree builds every test target the entry uses.
    pub fn builds(&self, mutation: &Mutation) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for test in &mutation.expect_failing {
            let mut args = self.test_command(mutation, test).map_err(|e| e.0)?;
            if let Some(split) = args.iter().position(|a| a == "--") {
                args.truncate(split);
            }
            args.push("--no-run".into());
            if !seen.insert(args.clone()) {
                continue;
            }
            let (code, stdout, stderr) = run_command(
                self.cargo(&args),
                Duration::from_secs(self.suite.timeout_secs),
            );
            if code != Some(0) {
                return Err(tail(&format!("{stdout}{stderr}"), 3000));
            }
        }
        Ok(())
    }

    pub fn prepare(&self) -> Result<(), Error> {
        if self.scratch.exists() {
            fs::remove_dir_all(&self.scratch)
                .map_err(|e| Error(format!("{}: {e}", self.scratch.display())))?;
        }
        fs::create_dir_all(self.scratch.join("tmp/xdg-data"))
            .map_err(|e| Error(format!("{}: {e}", self.scratch.display())))?;
        copy_worktree(&self.root, &self.scratch)
    }

    pub fn cleanup(&self) {
        if self
            .scratch
            .ends_with(Path::new("mutation-scratch").join("tree"))
        {
            let _ = fs::remove_dir_all(&self.scratch);
        }
    }

    /// Clear the shared build dir when it has grown past `limit` bytes, so
    /// many suites' feature sets cannot fill the build disk.
    pub fn bound_build_dir(&self, limit: u64) -> u64 {
        let size = dir_size(&self.build);
        if size > limit && self.build.ends_with("scratch-build") {
            let _ = fs::remove_dir_all(&self.build);
        }
        size
    }

    /// Baseline, then one entry at a time. Returns the failures.
    pub fn run(&self, mutations: &[Mutation], out: &mut dyn Write) -> Result<Vec<String>, Error> {
        let trouble = problems(mutations, &self.scratch);
        if !trouble.is_empty() {
            return Ok(trouble);
        }
        let mut failures = Vec::new();
        let mut baseline: Vec<(&Mutation, &str)> = Vec::new();
        let mut seen = BTreeSet::new();
        for mutation in mutations {
            for test in &mutation.expect_failing {
                let command = self
                    .test_command(mutation, test)
                    .unwrap_or_else(|e| vec![e.0]);
                if seen.insert(command) {
                    baseline.push((mutation, test));
                }
            }
        }
        for (mutation, test) in &baseline {
            let outcome = self.outcome(mutation, test);
            if outcome.status != Status::Passed {
                failures.push(format!(
                    "baseline: {test} {:?} before any mutation\n{}",
                    outcome.status, outcome.output
                ));
            }
        }
        if !failures.is_empty() {
            return Ok(failures);
        }
        let _ = writeln!(out, "baseline: {} tests pass unmutated", baseline.len());
        for mutation in mutations {
            let path = self.scratch.join(&mutation.file);
            let original =
                fs::read_to_string(&path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
            fs::write(
                &path,
                original.replacen(&mutation.find, &mutation.replace, 1),
            )
            .map_err(|e| Error(format!("{}: {e}", path.display())))?;
            let result = (|| {
                self.builds(mutation)?;
                Ok::<_, String>(
                    mutation
                        .expect_failing
                        .iter()
                        .map(|test| (test.clone(), self.outcome(mutation, test)))
                        .collect::<Vec<_>>(),
                )
            })();
            fs::write(&path, &original).map_err(|e| Error(format!("{}: {e}", path.display())))?;
            match result {
                Err(output) => {
                    let _ = writeln!(out, "ERROR    {} (does not build)", mutation.id);
                    failures.push(format!(
                        "{}: the mutated tree does not build\n{output}",
                        mutation.id
                    ));
                }
                Ok(outcomes) => {
                    let mut bad = Vec::new();
                    for (test, outcome) in &outcomes {
                        match outcome.status {
                            Status::Failed => {}
                            Status::Passed => {
                                bad.push(format!("{}: {test} still passes", mutation.id))
                            }
                            Status::Error => bad.push(format!(
                                "{}: {test} could not run\n{}",
                                mutation.id, outcome.output
                            )),
                        }
                    }
                    if bad.is_empty() {
                        let _ =
                            writeln!(out, "killed   {} ({} failing)", mutation.id, outcomes.len());
                    } else {
                        let survived = outcomes.iter().any(|(_, o)| o.status == Status::Passed);
                        let _ = writeln!(
                            out,
                            "{} {}",
                            if survived { "SURVIVED" } else { "ERROR   " },
                            mutation.id
                        );
                        failures.extend(bad);
                    }
                }
            }
        }
        Ok(failures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_result_lines_are_counted() {
        let out = "running 2 tests\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.1s\n\
                   test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.0s\n";
        assert_eq!(cargo_counts(out), vec![(2, 0), (0, 1)]);
        assert!(cargo_counts("no tests").is_empty());
    }

    #[test]
    fn package_filters_name_their_target() {
        assert_eq!(
            package_filter("kassiber-wallets/sync:classes_match"),
            Some((
                "kassiber-wallets".into(),
                Some("sync".into()),
                "classes_match".into()
            ))
        );
        assert_eq!(
            package_filter("kassiber-daemon:a_test"),
            Some(("kassiber-daemon".into(), None, "a_test".into()))
        );
        assert_eq!(package_filter("tests::module::name"), None);
        assert_eq!(package_filter("plain_name"), None);
    }

    #[test]
    fn commands_time_out() {
        let mut command = Command::new("sleep");
        command.arg("5");
        let (code, _, stderr) = run_command(command, Duration::from_millis(200));
        assert_eq!(code, None);
        assert!(stderr.contains("timed out"));
    }
}

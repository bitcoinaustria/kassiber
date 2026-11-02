//! Child launches that inherit exactly the descriptors or handles listed.
//!
//! The broker hands its children owner-lock duplicates and lease pipes by
//! number, as Python's `subprocess.Popen(pass_fds=...)` on POSIX and
//! `STARTUPINFO.lpAttributeList["handle_list"]` on Windows do:
//!
//! - POSIX: std creates every descriptor close-on-exec; the child clears
//!   `FD_CLOEXEC` on the listed ones after fork and, when detached, calls
//!   `setsid` (`start_new_session=True`).
//! - Windows: `CreateProcessW` with `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, so
//!   only the standard handles and the listed ones cross. Listed handles
//!   keep their values (the child is told them by number), so they are made
//!   inheritable for the launch and reset afterwards; launches through this
//!   module are serialized so two of them never see each other's handles.
//!   std's own `Command` inherits every inheritable handle, so code that
//!   launches while such a window is open must use this module too.
//!
//! Every place that constructs a [`ChildCommand`] is listed in the
//! workspace chokepoint scanner's reviewed process calls.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

/// Where a standard stream of the child goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stdio {
    /// The null device (`subprocess.DEVNULL`).
    Null,
    /// This process's own stream (`None`).
    Inherit,
    /// A new pipe whose other end the parent keeps (`subprocess.PIPE`).
    Pipe,
}

/// A descriptor (POSIX) or handle (Windows) the child inherits under the
/// same number.
#[cfg(unix)]
pub type Inheritable = std::os::fd::RawFd;
/// A descriptor (POSIX) or handle (Windows) the child inherits under the
/// same number.
#[cfg(windows)]
pub type Inheritable = std::os::windows::io::RawHandle;

/// A child process description, like `std::process::Command` with explicit
/// inheritance and detachment.
pub struct ChildCommand {
    pub(crate) program: OsString,
    pub(crate) args: Vec<OsString>,
    env_clear: bool,
    env: Vec<(OsString, Option<OsString>)>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) stdin: Stdio,
    pub(crate) stdout: Stdio,
    pub(crate) stderr: Stdio,
    pub(crate) inherit: Vec<Inheritable>,
    pub(crate) detach: bool,
}

impl ChildCommand {
    /// A command running `program`, which is also the child's `argv[0]`.
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        ChildCommand {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            env_clear: false,
            env: Vec::new(),
            cwd: None,
            stdin: Stdio::Inherit,
            stdout: Stdio::Inherit,
            stderr: Stdio::Inherit,
            inherit: Vec::new(),
            detach: false,
        }
    }

    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
        self
    }

    /// Start from an empty environment rather than this process's.
    pub fn env_clear(&mut self) -> &mut Self {
        self.env_clear = true;
        self.env.clear();
        self
    }

    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.env
            .push((key.as_ref().to_owned(), Some(value.as_ref().to_owned())));
        self
    }

    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.env.push((key.as_ref().to_owned(), None));
        self
    }

    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.cwd = Some(dir.as_ref().to_owned());
        self
    }

    pub fn stdin(&mut self, stdio: Stdio) -> &mut Self {
        self.stdin = stdio;
        self
    }

    pub fn stdout(&mut self, stdio: Stdio) -> &mut Self {
        self.stdout = stdio;
        self
    }

    pub fn stderr(&mut self, stdio: Stdio) -> &mut Self {
        self.stderr = stdio;
        self
    }

    /// Let the child inherit `handle` under the same number. The caller
    /// keeps ownership and must keep it open until [`ChildCommand::spawn`]
    /// returns.
    pub fn inherit(&mut self, handle: Inheritable) -> &mut Self {
        if !self.inherit.contains(&handle) {
            self.inherit.push(handle);
        }
        self
    }

    /// Detach the child: a new session on POSIX (`start_new_session=True`),
    /// `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` on Windows.
    pub fn detach(&mut self, on: bool) -> &mut Self {
        self.detach = on;
        self
    }

    /// The environment the child will see, `os.environ` plus the changes.
    pub(crate) fn environment(&self) -> Vec<(OsString, OsString)> {
        let mut environment: Vec<(OsString, OsString)> = if self.env_clear {
            Vec::new()
        } else {
            std::env::vars_os().collect()
        };
        for (key, value) in &self.env {
            environment.retain(|(name, _)| !same_key(name, key));
            if let Some(value) = value {
                environment.push((key.clone(), value.clone()));
            }
        }
        environment
    }

    /// Start the child.
    pub fn spawn(&mut self) -> io::Result<Child> {
        imp::spawn(self)
    }
}

#[cfg(unix)]
fn same_key(left: &OsStr, right: &OsStr) -> bool {
    left == right
}

#[cfg(windows)]
fn same_key(left: &OsStr, right: &OsStr) -> bool {
    left.to_string_lossy().to_uppercase() == right.to_string_lossy().to_uppercase()
}

/// A started child. Dropping it neither waits for nor kills the process.
pub struct Child {
    pub stdin: Option<File>,
    pub stdout: Option<File>,
    pub stderr: Option<File>,
    pub(crate) inner: imp::Process,
}

impl Child {
    /// The child's process id.
    pub fn id(&self) -> u32 {
        self.inner.id()
    }

    /// Wait for the child to exit.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        drop(self.stdin.take());
        self.inner.wait()
    }

    /// The exit status if the child has exited.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }

    /// `Popen.terminate()`: SIGTERM on POSIX, `TerminateProcess` on Windows.
    pub fn terminate(&mut self) -> io::Result<()> {
        self.inner.terminate()
    }

    /// `Popen.kill()`: SIGKILL on POSIX, `TerminateProcess` on Windows.
    pub fn kill(&mut self) -> io::Result<()> {
        self.inner.kill()
    }
}

/// `subprocess.list2cmdline` over UTF-16 units: the MS C runtime quoting
/// Python uses for every Windows launch.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn list2cmdline(arguments: &[Vec<u16>]) -> Vec<u16> {
    const SPACE: u16 = b' ' as u16;
    const TAB: u16 = b'\t' as u16;
    const QUOTE: u16 = b'"' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    let mut result = Vec::new();
    for argument in arguments {
        let mut backslashes = 0usize;
        if !result.is_empty() {
            result.push(SPACE);
        }
        let quote = argument.is_empty() || argument.contains(&SPACE) || argument.contains(&TAB);
        if quote {
            result.push(QUOTE);
        }
        for unit in argument {
            match *unit {
                BACKSLASH => backslashes += 1,
                QUOTE => {
                    result.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2));
                    backslashes = 0;
                    result.extend([BACKSLASH, QUOTE]);
                }
                other => {
                    result.extend(std::iter::repeat_n(BACKSLASH, backslashes));
                    backslashes = 0;
                    result.push(other);
                }
            }
        }
        result.extend(std::iter::repeat_n(BACKSLASH, backslashes));
        if quote {
            result.extend(std::iter::repeat_n(BACKSLASH, backslashes));
            result.push(QUOTE);
        }
    }
    result
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::process::CommandExt;

    pub(crate) struct Process(std::process::Child);

    impl Process {
        pub(super) fn id(&self) -> u32 {
            self.0.id()
        }
        pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
            self.0.wait()
        }
        pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            self.0.try_wait()
        }
        pub(super) fn terminate(&mut self) -> io::Result<()> {
            if self.0.try_wait()?.is_some() {
                return Ok(());
            }
            let pid = libc::pid_t::try_from(self.0.id())
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            // SAFETY: the pid belongs to an unreaped child of this process.
            if unsafe { libc::kill(pid, libc::SIGTERM) } == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        pub(super) fn kill(&mut self) -> io::Result<()> {
            self.0.kill()
        }
    }

    fn stdio(stdio: Stdio) -> std::process::Stdio {
        match stdio {
            Stdio::Null => std::process::Stdio::null(),
            Stdio::Inherit => std::process::Stdio::inherit(),
            Stdio::Pipe => std::process::Stdio::piped(),
        }
    }

    pub(super) fn spawn(command: &ChildCommand) -> io::Result<Child> {
        // Below 3 the number would be replaced by the child's stdio.
        if command.inherit.iter().any(|fd| *fd <= 2) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "inherited descriptors must be above the standard streams",
            ));
        }
        let mut child = std::process::Command::new(&command.program);
        child.args(&command.args);
        child.env_clear();
        child.envs(command.environment());
        if let Some(cwd) = &command.cwd {
            child.current_dir(cwd);
        }
        child
            .stdin(stdio(command.stdin))
            .stdout(stdio(command.stdout))
            .stderr(stdio(command.stderr));
        let inherit = command.inherit.clone();
        let detach = command.detach;
        // SAFETY: the closure runs between fork and exec and calls only
        // fcntl(2) and setsid(2), which are async-signal-safe; it allocates
        // nothing (the vector was moved in before fork).
        unsafe {
            child.pre_exec(move || {
                for fd in &inherit {
                    let flags = libc::fcntl(*fd, libc::F_GETFD);
                    if flags == -1
                        || libc::fcntl(*fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1
                    {
                        return Err(io::Error::last_os_error());
                    }
                }
                if detach && libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut process = child.spawn()?;
        Ok(Child {
            stdin: process.stdin.take().map(owned_file),
            stdout: process.stdout.take().map(owned_file),
            stderr: process.stderr.take().map(owned_file),
            inner: Process(process),
        })
    }

    fn owned_file(stream: impl Into<std::os::fd::OwnedFd>) -> File {
        File::from(stream.into())
    }
}

#[cfg(windows)]
mod imp {
    pub(super) use crate::windows::process::{spawn, Process};
}

#[cfg(test)]
mod quoting_tests {
    use super::list2cmdline;

    #[test]
    fn command_lines_match_python_list2cmdline() {
        // crates/ipc/scripts/gen_list2cmdline.py writes these from CPython.
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/list2cmdline.json")).unwrap();
        let vectors = vectors.as_array().unwrap();
        assert!(vectors.len() >= 13);
        for vector in vectors {
            let argv: Vec<Vec<u16>> = vector["argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap().encode_utf16().collect())
                .collect();
            let expected: Vec<u16> = vector["command_line"]
                .as_str()
                .unwrap()
                .encode_utf16()
                .collect();
            assert_eq!(list2cmdline(&argv), expected, "{vector}");
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;

    fn shell(script: &str) -> ChildCommand {
        let mut command = ChildCommand::new("/bin/sh");
        command.arg("-c").arg(script);
        command
    }

    fn descriptor_open_in_child(inherit: bool) -> bool {
        let file = tempfile::tempfile().unwrap();
        let fd = file.as_raw_fd();
        let mut command = shell(&format!(
            "(: <&{fd}) 2>/dev/null && echo open || echo closed"
        ));
        command.stdout(Stdio::Pipe).stdin(Stdio::Null);
        if inherit {
            command.inherit(fd);
        }
        let mut child = command.spawn().unwrap();
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert!(child.wait().unwrap().success());
        drop(file);
        output.trim() == "open"
    }

    #[test]
    fn only_listed_descriptors_cross_exec() {
        assert!(descriptor_open_in_child(true));
        assert!(!descriptor_open_in_child(false));
    }

    #[test]
    fn standard_stream_numbers_are_refused() {
        let error = shell("true").inherit(2).spawn().err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn detached_children_lead_a_new_session() {
        let mut command = shell("ps -o sid= -p $$");
        command.stdout(Stdio::Pipe).detach(true);
        let mut child = command.spawn().unwrap();
        let pid = child.id();
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        child.wait().unwrap();
        // getsid of the exited child is gone; the shell reported its own.
        let reported: u32 = output.trim().parse().unwrap_or(0);
        assert_eq!(reported, pid, "the detached child is its session leader");
    }

    #[test]
    fn environment_and_directory_are_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = shell("printf '%s|%s|%s' \"$KEEP\" \"${DROP-unset}\" \"$PWD\"");
        command
            .env_clear()
            .env("KEEP", "yes")
            .env("DROP", "no")
            .env_remove("DROP")
            .current_dir(dir.path())
            .stdout(Stdio::Pipe);
        let mut child = command.spawn().unwrap();
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        child.wait().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        assert_eq!(output, format!("yes|unset|{}", canonical.display()));
    }

    #[test]
    fn piped_stdin_closes_on_wait_and_terminate_signals() {
        let mut command = shell("cat >/dev/null; exit 3");
        command.stdin(Stdio::Pipe);
        let mut child = command.spawn().unwrap();
        child.stdin.as_mut().unwrap().write_all(b"data").unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(3));
        let mut sleeper = shell("exec sleep 30").spawn().unwrap();
        sleeper.terminate().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(sleeper.wait().unwrap().signal(), Some(libc::SIGTERM));
        sleeper.terminate().unwrap();
    }
}

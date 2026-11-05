//! The AI provider broker's process route (`kassiber/ai/broker_client.py`).
//!
//! The bundled Node broker starts the provider CLIs (Claude, Codex, OpenCode,
//! ACP agents), which reach their vendors, so a launch is network work: it
//! needs a [`Route`] for [`Request::ai_broker`], which passes the same online
//! gate as Python's `require_online` (offline mode refuses it; the product
//! `KASSIBER_NO_EGRESS` switch leaves AI alone, as in Python) and an
//! explicit-trigger consent. An armed test observer records every launch
//! (`Operation::Process`); it admits only the fake broker a harness named.
//! The product ledger records none, as Python records none.
//!
//! The broker runs as `[node, script]` with the full environment plus
//! `KASSIBER_AI_BROKER_TMPDIR`, a private per-turn root this process owns,
//! in RAM where [`broker_temp_parent`] finds a safe place for it. On Unix it
//! leads a new process group, so a cancel or a timeout can reach the CLIs it
//! started; Windows has no group kill and ends only the broker.

use crate::{remaining, Endpoint, Error, Gate, Observer, Operation, Request, Route, Subsystem};
use crate::{TestEgressBlocked, TlsTrust};
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The variable naming the broker's per-turn root, written to its environment.
pub const BROKER_TMPDIR_ENV: &str = "KASSIBER_AI_BROKER_TMPDIR";
/// The Node executable override.
pub const BROKER_NODE_ENV: &str = "KASSIBER_AI_BROKER_NODE";
/// The broker script override.
pub const BROKER_SCRIPT_ENV: &str = "KASSIBER_AI_PROVIDER_BROKER";
/// How long a terminated broker gets before its group is killed.
pub const BROKER_GRACE: Duration = Duration::from_secs(2);

const BROKER_LABEL: &str = "provider-broker";
const NODE_FALLBACKS: &[&str] = &[
    "/opt/homebrew/bin/node",
    "/usr/local/bin/node",
    "~/.local/bin/node",
    "/opt/local/bin/node",
];
const POLL: Duration = Duration::from_millis(5);

/// What a broker launch runs: `_node_executable()` and `_broker_script()`.
#[derive(Clone, PartialEq, Eq)]
pub struct BrokerProgram {
    node: Option<PathBuf>,
    script: PathBuf,
}

impl fmt::Debug for BrokerProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BrokerProgram(<redacted>)")
    }
}

impl BrokerProgram {
    pub fn new(node: Option<PathBuf>, script: PathBuf) -> Self {
        Self { node, script }
    }

    /// Read afresh from the environment, as Python does for every request:
    /// `KASSIBER_AI_BROKER_NODE` if it names an executable file, else `node`
    /// on `PATH`, else the fixed fallbacks; `KASSIBER_AI_PROVIDER_BROKER`,
    /// else `bundled`.
    pub fn locate(bundled: &Path) -> Self {
        Self {
            node: node_executable(),
            script: broker_script(bundled),
        }
    }

    pub fn script(&self) -> &Path {
        &self.script
    }

    /// `node and script.is_file()`.
    pub fn available(&self) -> bool {
        self.node.is_some() && self.script.is_file()
    }
}

fn env_os(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

/// `_node_executable()`.
pub fn node_executable() -> Option<PathBuf> {
    if let Some(configured) = env_os(BROKER_NODE_ENV) {
        let candidate = expanduser(Path::new(&configured));
        if executable_file(&candidate) {
            return Some(candidate);
        }
    }
    if let Some(found) = which("node") {
        return Some(found);
    }
    NODE_FALLBACKS
        .iter()
        .map(|raw| expanduser(Path::new(raw)))
        .find(|candidate| executable_file(candidate))
}

/// `_broker_script()`.
pub fn broker_script(bundled: &Path) -> PathBuf {
    match env_os(BROKER_SCRIPT_ENV) {
        Some(configured) => expanduser(Path::new(&configured)),
        None => bundled.to_path_buf(),
    }
}

/// `candidate.is_file() and os.access(candidate, os.X_OK)`.
fn executable_file(candidate: &Path) -> bool {
    candidate.is_file() && access_x(candidate)
}

#[cfg(unix)]
fn access_x(path: &Path) -> bool {
    nix::unistd::access(path, nix::unistd::AccessFlags::X_OK).is_ok()
}

#[cfg(not(unix))]
fn access_x(path: &Path) -> bool {
    // Windows has no execute bit: os.access(X_OK) only asks whether it exists.
    path.exists()
}

/// `Path.expanduser()`: a leading `~` or `~user`; anything else unchanged.
#[cfg(unix)]
fn expanduser(path: &Path) -> PathBuf {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let raw = path.as_os_str().as_bytes();
    if raw.first() != Some(&b'~') {
        return path.to_path_buf();
    }
    let end = raw[1..]
        .iter()
        .position(|byte| *byte == b'/')
        .map_or(raw.len(), |index| index + 1);
    let home: Vec<u8> = if end == 1 {
        match std::env::var_os("HOME") {
            Some(home) => home.into_vec(),
            None => match nix::unistd::User::from_uid(nix::unistd::getuid()) {
                Ok(Some(user)) => user.dir.into_os_string().into_vec(),
                _ => return path.to_path_buf(),
            },
        }
    } else {
        let Ok(name) = std::str::from_utf8(&raw[1..end]) else {
            return path.to_path_buf();
        };
        match nix::unistd::User::from_name(name) {
            Ok(Some(user)) => user.dir.into_os_string().into_vec(),
            _ => return path.to_path_buf(),
        }
    };
    let mut expanded = home;
    while expanded.last() == Some(&b'/') {
        expanded.pop();
    }
    expanded.extend_from_slice(&raw[end..]);
    if expanded.is_empty() {
        expanded.push(b'/');
    }
    PathBuf::from(OsString::from_vec(expanded))
}

/// `ntpath.expanduser`: `~` is `USERPROFILE`, else `HOMEDRIVE` + `HOMEPATH`.
/// `~user` stays unchanged (Python guesses a sibling of the current home).
#[cfg(not(unix))]
fn expanduser(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let Some(rest) = text.strip_prefix('~') else {
        return path.to_path_buf();
    };
    if !(rest.is_empty() || rest.starts_with(['/', '\\'])) {
        return path.to_path_buf();
    }
    let home = match std::env::var_os("USERPROFILE") {
        Some(home) => PathBuf::from(home),
        None => match std::env::var_os("HOMEPATH") {
            Some(home_path) => {
                let mut home = std::env::var_os("HOMEDRIVE").unwrap_or_default();
                home.push(home_path);
                PathBuf::from(home)
            }
            None => return path.to_path_buf(),
        },
    };
    PathBuf::from(format!("{}{rest}", home.to_string_lossy()))
}

/// `shutil.which(command)` as Python 3.11 resolves it.
pub fn which(command: &str) -> Option<PathBuf> {
    let found = |candidate: &Path| candidate.exists() && access_x(candidate) && !candidate.is_dir();
    if Path::new(command)
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        return found(Path::new(command)).then(|| PathBuf::from(command));
    }
    let path = match std::env::var_os("PATH") {
        Some(path) => path,
        None if cfg!(windows) => ".;C:\\bin".into(),
        None if cfg!(target_os = "macos") => "/usr/bin:/bin:/usr/sbin:/sbin".into(),
        None => "/bin:/usr/bin".into(),
    };
    if path.is_empty() {
        return None;
    }
    let mut directories: Vec<PathBuf> = split_path(&path);
    let mut files = vec![command.to_owned()];
    if cfg!(windows) {
        // The current directory takes precedence on Windows.
        if !directories.iter().any(|dir| dir.as_os_str() == ".") {
            directories.insert(0, PathBuf::from("."));
        }
        let source = std::env::var("PATHEXT")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD;.VBS;.JS;.WSF;.MSC".into());
        let extensions: Vec<&str> = source.split(';').filter(|ext| !ext.is_empty()).collect();
        let lower = command.to_lowercase();
        if !extensions
            .iter()
            .any(|ext| lower.ends_with(&ext.to_lowercase()))
        {
            files = extensions
                .iter()
                .map(|ext| format!("{command}{ext}"))
                .collect();
        }
    }
    let mut seen: Vec<OsString> = Vec::new();
    for directory in directories {
        let normalized = if cfg!(windows) {
            directory.to_string_lossy().to_lowercase().into()
        } else {
            directory.as_os_str().to_owned()
        };
        if seen.contains(&normalized) {
            continue;
        }
        seen.push(normalized);
        for file in &files {
            let candidate = if directory.as_os_str().is_empty() {
                PathBuf::from(file)
            } else {
                directory.join(file)
            };
            if found(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(unix)]
fn split_path(path: &std::ffi::OsStr) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    path.as_bytes()
        .split(|byte| *byte == b':')
        .map(|part| PathBuf::from(std::ffi::OsStr::from_bytes(part)))
        .collect()
}

#[cfg(not(unix))]
fn split_path(path: &std::ffi::OsStr) -> Vec<PathBuf> {
    path.to_string_lossy()
        .split(';')
        .map(PathBuf::from)
        .collect()
}

/// `str.splitlines()`'s line boundaries.
fn line_boundary(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r'
            | '\u{0b}'
            | '\u{0c}'
            | '\u{1c}'
            | '\u{1d}'
            | '\u{1e}'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// `str.splitlines()`.
pub fn splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        if !line_boundary(c) {
            continue;
        }
        lines.push(&text[start..index]);
        start = index + c.len_utf8();
        if c == '\r' {
            if let Some((next, '\n')) = chars.peek().copied() {
                chars.next();
                start = next + 1;
            }
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// `str.split()`: runs of Unicode whitespace.
fn split_fields(line: &str) -> impl Iterator<Item = &str> {
    line.split(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .filter(|field| !field.is_empty())
}

/// `\ooo` octal escapes in a mounts-table field, each one code point.
fn unescape_mount_point(field: &str) -> String {
    let chars: Vec<char> = field.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\\'
            && index + 3 < chars.len()
            && chars[index + 1..=index + 3]
                .iter()
                .all(|c| ('0'..='7').contains(c))
        {
            let code = chars[index + 1..=index + 3]
                .iter()
                .fold(0, |value, c| value * 8 + c.to_digit(8).unwrap_or(0));
            out.push(char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER));
            index += 4;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

/// `_mount_fstype(path, mounts_path)`: the filesystem type of the mount that
/// holds `path`; the last entry for the longest mount point is in effect.
pub fn mount_fstype(path: &str, mounts_path: &Path) -> Option<String> {
    let raw = fs::read(mounts_path).ok()?;
    let text = String::from_utf8_lossy(&raw);
    let mut best: Option<(usize, String)> = None;
    for line in splitlines(&text) {
        let fields: Vec<&str> = split_fields(line).collect();
        if fields.len() < 3 {
            continue;
        }
        let mount_point = unescape_mount_point(fields[1]);
        let inside = format!("{}/", mount_point.trim_end_matches('/'));
        if path != mount_point && !path.starts_with(&inside) {
            continue;
        }
        let length = mount_point.chars().count();
        if best.as_ref().is_none_or(|(longest, _)| length >= *longest) {
            best = Some((length, fields[2].to_owned()));
        }
    }
    best.map(|(_, fstype)| fstype)
}

/// `_broker_temp_parent()`: where the per-turn root goes, `None` for the
/// system temporary directory (disk-backed).
///
/// On Linux the per-user `$XDG_RUNTIME_DIR` is normally a tmpfs only that
/// user can enter, so Copilot's session file never reaches a disk. It is
/// used only when it is an absolute, real directory owned by this user,
/// writable by no one else, and mounted as tmpfs or ramfs.
pub fn broker_temp_parent() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        temp_parent_for(
            cfg!(any(target_os = "linux", target_os = "android")),
            std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
            Path::new("/proc/self/mounts"),
            nix::unistd::getuid().as_raw(),
        )
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// [`broker_temp_parent`] with its inputs explicit, for tests.
#[cfg(unix)]
pub fn temp_parent_for(
    linux: bool,
    runtime_dir: Option<&std::ffi::OsStr>,
    mounts_path: &Path,
    uid: u32,
) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    if !linux {
        return None;
    }
    let candidate = Path::new(runtime_dir?);
    if !candidate.is_absolute() {
        return None;
    }
    let info = fs::symlink_metadata(candidate).ok()?;
    if !info.file_type().is_dir() || info.uid() != uid {
        return None;
    }
    if info.mode() & 0o022 != 0 {
        return None;
    }
    // Mount points are real paths; an ancestor may be a symlink (/var/run).
    let path = fs::canonicalize(candidate).ok()?;
    match mount_fstype(&path.to_string_lossy(), mounts_path)?.as_str() {
        "tmpfs" | "ramfs" => Some(path),
        _ => None,
    }
}

/// `tempfile.mkdtemp(prefix="kassiber-ai-broker-", dir=parent)`.
fn make_temp_root(parent: Option<&Path>) -> io::Result<PathBuf> {
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_";
    let parent = match parent {
        Some(parent) => parent.to_path_buf(),
        None => std::path::absolute(std::env::temp_dir())?,
    };
    for _ in 0..10_000 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos()),
        );
        hasher.write_u32(std::process::id());
        let mut seed = hasher.finish();
        let name: String = (0..8)
            .map(|_| {
                let letter = LETTERS[(seed % LETTERS.len() as u64) as usize];
                seed /= LETTERS.len() as u64;
                letter as char
            })
            .collect();
        let root = parent.join(format!("kassiber-ai-broker-{name}"));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&root) {
            Ok(()) => return Ok(root),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no usable temporary name",
    ))
}

impl Request {
    /// A broker launch. Consent binds the exact program.
    pub fn ai_broker(program: BrokerProgram) -> Self {
        let mut request = Self::new(
            Subsystem::Ai,
            Endpoint::new(BROKER_LABEL, 1).expect("fixed process label"),
            TlsTrust::Plaintext,
        );
        request.broker = Some(program);
        request
    }
}

impl Gate {
    /// The one fake broker script an armed loopback observer admits.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_broker_fixture(mut self, script: PathBuf) -> Self {
        self.broker_fixture = Some(script);
        self
    }
}

#[derive(Debug)]
pub enum BrokerError {
    /// A gate refusal: offline, consent, or a non-broker route.
    Gate(Error),
    /// No Node, or no script file.
    Missing,
    /// The temporary root or the launch failed.
    Io,
}

impl From<Error> for BrokerError {
    fn from(value: Error) -> Self {
        Self::Gate(value)
    }
}

impl Route {
    /// Start the broker. The gate is checked first, as `require_online` is;
    /// a missing Node or script starts nothing.
    pub fn spawn_broker(self) -> Result<BrokerProcess, BrokerError> {
        self.check()?;
        let program = self.request.broker.as_ref().ok_or(Error::InvalidEndpoint)?;
        let (Some(node), true) = (program.node.clone(), program.script.is_file()) else {
            return Err(BrokerError::Missing);
        };
        if self.gate.observer != Observer::Disabled {
            self.gate
                .attempts
                .record(&self.request, &self.request.endpoint, Operation::Process, 0);
            if self.gate.observer == Observer::Strict
                || self.gate.broker_fixture.as_ref() != Some(&program.script)
            {
                std::panic::panic_any(TestEgressBlocked);
            }
        }
        let root = make_temp_root(broker_temp_parent().as_deref()).map_err(|_| BrokerError::Io)?;
        let mut command = Command::new(node);
        command
            .arg(&program.script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env(BROKER_TMPDIR_ENV, &root);
        // The broker leads its own group, so signals reach the CLIs it starts.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let spawned = self
            .check()
            .map_err(BrokerError::Gate)
            .and_then(|()| command.spawn().map_err(|_| BrokerError::Io));
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => {
                let _ = fs::remove_dir_all(&root);
                return Err(error);
            }
        };
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().map(BrokerOutput::new);
        let signal = BrokerSignal {
            #[cfg(unix)]
            pgid: i32::try_from(child.id()).unwrap_or(0),
            child: Arc::new(Mutex::new(child)),
        };
        Ok(BrokerProcess {
            signal,
            stdin,
            stdout,
            root: Some(root),
            route: self,
        })
    }
}

/// Signals the broker from any thread, without its pipes.
#[derive(Clone)]
pub struct BrokerSignal {
    child: Arc<Mutex<Child>>,
    /// Recorded at start: once the broker has exited, its group can only be
    /// found by this id, and CLIs that ignored SIGTERM may still be in it.
    #[cfg(unix)]
    pgid: i32,
}

impl BrokerSignal {
    fn child(&self) -> std::sync::MutexGuard<'_, Child> {
        self.child.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// `process.poll() is None`.
    pub fn running(&self) -> bool {
        matches!(self.child().try_wait(), Ok(None))
    }

    /// SIGTERM to the group (`process.terminate()` on Windows).
    pub fn terminate(&self) {
        #[cfg(unix)]
        self.group(nix::sys::signal::Signal::SIGTERM);
        #[cfg(not(unix))]
        let _ = self.child().kill();
    }

    /// SIGKILL to the group (`process.terminate()` on Windows).
    pub fn kill(&self) {
        #[cfg(unix)]
        self.group(nix::sys::signal::Signal::SIGKILL);
        #[cfg(not(unix))]
        let _ = self.child().kill();
    }

    #[cfg(unix)]
    fn group(&self, signal: nix::sys::signal::Signal) {
        if self.pgid > 0 {
            let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(self.pgid), signal);
        }
    }

    /// Wait for the broker to exit, at most `timeout`; whether it did.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if !matches!(self.child().try_wait(), Ok(None)) {
                return true;
            }
            match remaining(deadline) {
                Ok(left) => std::thread::sleep(left.min(POLL)),
                Err(_) => return false,
            }
        }
    }

    /// `process.wait()`.
    pub fn wait(&self) {
        while matches!(self.child().try_wait(), Ok(None)) {
            std::thread::sleep(POLL);
        }
    }
}

/// The broker's stdout, read as Python's text-mode pipe: `\r\n` and a bare
/// `\r` end a line as `\n` does.
pub struct BrokerOutput {
    reader: BufReader<ChildStdout>,
}

impl BrokerOutput {
    fn new(stdout: ChildStdout) -> Self {
        Self {
            reader: BufReader::new(stdout),
        }
    }

    /// The next line without its end, `None` at EOF. A line longer than
    /// `limit` is `InvalidData`.
    pub fn read_line(&mut self, limit: usize) -> io::Result<Option<Vec<u8>>> {
        let mut line = Vec::new();
        loop {
            let buffer = match self.reader.fill_buf() {
                Ok(buffer) => buffer,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            if buffer.is_empty() {
                return Ok((!line.is_empty()).then_some(line));
            }
            match buffer.iter().position(|byte| matches!(byte, b'\n' | b'\r')) {
                Some(index) => {
                    let carriage = buffer[index] == b'\r';
                    line.extend_from_slice(&buffer[..index]);
                    self.reader.consume(index + 1);
                    if line.len() > limit {
                        return Err(too_long());
                    }
                    if carriage {
                        // A CR LF pair is one line end; this waits for the next
                        // byte as Python's newline decoder does.
                        if let Ok(next) = self.reader.fill_buf() {
                            if next.first() == Some(&b'\n') {
                                self.reader.consume(1);
                            }
                        }
                    }
                    return Ok(Some(line));
                }
                None => {
                    let length = buffer.len();
                    line.extend_from_slice(buffer);
                    self.reader.consume(length);
                    if line.len() > limit {
                        return Err(too_long());
                    }
                }
            }
        }
    }

    /// Everything until EOF, at most `limit` bytes.
    pub fn read_to_end(&mut self, limit: usize) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        (&mut self.reader)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(too_long());
        }
        Ok(bytes)
    }
}

fn too_long() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "broker output over the limit")
}

/// A started broker. Dropping it ends the process and its group and removes
/// its root, as `_finish_process` does.
pub struct BrokerProcess {
    signal: BrokerSignal,
    stdin: Option<ChildStdin>,
    stdout: Option<BrokerOutput>,
    root: Option<PathBuf>,
    route: Route,
}

impl BrokerProcess {
    pub fn signal(&self) -> BrokerSignal {
        self.signal.clone()
    }

    /// The route's gate, rechecked: offline mode or a revoked consent.
    pub fn check(&self) -> Result<(), Error> {
        self.route.check()
    }

    /// The per-turn root in the broker's environment.
    pub fn temp_root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Write one request line; the gate is rechecked first.
    pub fn write_line(&mut self, line: &[u8]) -> io::Result<()> {
        self.route.check().map_err(io::Error::other)?;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?;
        stdin.write_all(line)?;
        stdin.flush()
    }

    pub fn close_stdin(&mut self) {
        self.stdin = None;
    }

    pub fn output(&mut self) -> Option<&mut BrokerOutput> {
        self.stdout.as_mut()
    }

    pub fn take_output(&mut self) -> Option<BrokerOutput> {
        self.stdout.take()
    }

    /// `_finish_process`: SIGTERM while it runs, SIGKILL after the grace
    /// period, then the root is removed after anything left in the group.
    pub fn finish(mut self) {
        self.finish_in_place();
    }

    fn finish_in_place(&mut self) {
        if self.root.is_none() && self.stdin.is_none() && self.stdout.is_none() {
            return;
        }
        if self.signal.running() {
            self.signal.terminate();
        }
        if !self.signal.wait_timeout(BROKER_GRACE) {
            self.signal.kill();
            self.signal.wait();
        }
        self.stdin = None;
        self.stdout = None;
        self.discard_root();
    }

    /// `_discard_broker_root`: a provider CLI that ignored SIGTERM can outlive
    /// the broker in its group and still be writing under the root.
    fn discard_root(&mut self) {
        self.signal.kill();
        if let Some(root) = self.root.take() {
            let _ = fs::remove_dir_all(root);
        }
    }
}

impl Drop for BrokerProcess {
    fn drop(&mut self) {
        self.finish_in_place();
    }
}

#[cfg(all(test, unix))]
mod tests {
    //! `BrokerTempParentTest` (`tests/test_ai_broker.py`) and the route's
    //! own gates.
    use super::*;
    use crate::{Consent, Ledger, Offline};
    use std::os::unix::fs::PermissionsExt;

    struct Parent {
        _dir: tempfile::TempDir,
        runtime: PathBuf,
        mounts: PathBuf,
    }

    impl Parent {
        fn new() -> Self {
            let dir = tempfile::Builder::new()
                .prefix("kassiber-broker-parent-")
                .tempdir()
                .unwrap();
            let runtime = dir.path().join("run user");
            fs::create_dir(&runtime).unwrap();
            fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
            let mounts = dir.path().join("mounts");
            let parent = Self {
                _dir: dir,
                runtime,
                mounts,
            };
            parent.write_mounts("tmpfs", "");
            parent
        }

        fn write_mounts(&self, fstype: &str, extra: &str) {
            let escaped = self.runtime.to_str().unwrap().replace(' ', "\\040");
            fs::write(
                &self.mounts,
                format!(
                    "/dev/sda1 / ext4 rw,relatime 0 0\ntmpfs {escaped} {fstype} rw,nosuid,nodev,mode=700 0 0\n{extra}"
                ),
            )
            .unwrap();
        }

        fn parent_for(&self, linux: bool, runtime: Option<&Path>, uid: u32) -> Option<PathBuf> {
            temp_parent_for(linux, runtime.map(Path::as_os_str), &self.mounts, uid)
        }

        fn parent(&self) -> Option<PathBuf> {
            self.parent_for(true, Some(&self.runtime), uid())
        }
    }

    fn uid() -> u32 {
        nix::unistd::getuid().as_raw()
    }

    #[test]
    fn uses_a_private_tmpfs_runtime_dir_on_linux() {
        let parent = Parent::new();
        let real = fs::canonicalize(&parent.runtime).unwrap();
        assert_eq!(parent.parent(), Some(real.clone()));
        // A path with a trailing slash names the same directory.
        let slashed = PathBuf::from(format!("{}/", parent.runtime.display()));
        assert_eq!(parent.parent_for(true, Some(&slashed), uid()), Some(real));
    }

    #[test]
    fn keeps_the_disk_temp_dir_when_the_runtime_dir_is_not_ram() {
        let parent = Parent::new();
        parent.write_mounts("ext4", "");
        assert_eq!(parent.parent(), None);
        // A later mount over the same point is the one in effect.
        let escaped = parent.runtime.to_str().unwrap().replace(' ', "\\040");
        parent.write_mounts("tmpfs", &format!("/dev/sdb1 {escaped} ext4 rw 0 0\n"));
        assert_eq!(parent.parent(), None);
        // No mounts table at all is not proof of RAM either.
        fs::remove_file(&parent.mounts).unwrap();
        assert_eq!(parent.parent(), None);
    }

    #[test]
    fn refuses_a_runtime_dir_others_can_write() {
        let parent = Parent::new();
        for mode in [0o720, 0o702, 0o777] {
            fs::set_permissions(&parent.runtime, fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(parent.parent(), None, "{mode:o}");
        }
        fs::set_permissions(&parent.runtime, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(parent.parent().is_some());
    }

    #[test]
    fn refuses_a_runtime_dir_owned_by_someone_else() {
        let parent = Parent::new();
        assert_eq!(
            parent.parent_for(true, Some(&parent.runtime), uid() + 1),
            None
        );
    }

    #[test]
    fn refuses_missing_relative_or_symlinked_runtime_dirs() {
        let parent = Parent::new();
        let base = parent.runtime.parent().unwrap().to_path_buf();
        assert_eq!(parent.parent_for(true, None, uid()), None);
        assert_eq!(parent.parent_for(true, Some(Path::new("")), uid()), None);
        assert_eq!(
            parent.parent_for(true, Some(Path::new("run/user/1000")), uid()),
            None
        );
        assert_eq!(
            parent.parent_for(true, Some(&base.join("absent")), uid()),
            None
        );
        let link = base.join("link");
        std::os::unix::fs::symlink(&parent.runtime, &link).unwrap();
        assert_eq!(parent.parent_for(true, Some(&link), uid()), None);
        let regular = base.join("file");
        fs::write(&regular, "").unwrap();
        assert_eq!(parent.parent_for(true, Some(&regular), uid()), None);
    }

    #[test]
    fn macos_and_windows_keep_the_system_temp_dir() {
        let parent = Parent::new();
        assert_eq!(parent.parent_for(false, Some(&parent.runtime), uid()), None);
    }

    #[test]
    fn mount_points_unescape_octal_and_the_longest_match_wins() {
        let dir = tempfile::tempdir().unwrap();
        let mounts = dir.path().join("mounts");
        fs::write(
            &mounts,
            "none / ext4 rw 0 0\nnone /run tmpfs rw 0 0\nnone /run/user\\0401 ramfs rw 0 0\nshort\n",
        )
        .unwrap();
        assert_eq!(
            mount_fstype("/run/user 1/x", &mounts).as_deref(),
            Some("ramfs")
        );
        assert_eq!(mount_fstype("/run/user", &mounts).as_deref(), Some("tmpfs"));
        assert_eq!(mount_fstype("/running", &mounts).as_deref(), Some("ext4"));
        assert_eq!(mount_fstype("/x", &dir.path().join("absent")), None);
    }

    #[test]
    fn splitlines_matches_python() {
        assert_eq!(
            splitlines("a\nb\r\nc\rd\u{2028}e\u{85}f\x0bg\x1ch\n"),
            ["a", "b", "c", "d", "e", "f", "g", "h"]
        );
        assert_eq!(splitlines("\n\nx"), ["", "", "x"]);
        assert!(splitlines("").is_empty());
    }

    #[test]
    fn temp_roots_are_private_and_unique() {
        let dir = tempfile::tempdir().unwrap();
        let first = make_temp_root(Some(dir.path())).unwrap();
        let second = make_temp_root(Some(dir.path())).unwrap();
        assert_ne!(first, second);
        for root in [&first, &second] {
            let name = root.file_name().unwrap().to_str().unwrap();
            assert!(
                name.starts_with("kassiber-ai-broker-") && name.len() == 27,
                "{name}"
            );
            let mode = fs::metadata(root).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
    }

    #[test]
    fn a_broker_route_never_opens_a_socket_and_needs_the_online_gate() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("broker.py");
        fs::write(&script, "raise SystemExit('must not run')\n").unwrap();
        let program = BrokerProgram::new(which("python3"), script.clone());
        let online = Gate::new(|| Offline::Online, Ledger::default());
        let request = Request::ai_broker(program.clone());
        let route = online
            .authorize(request.clone(), Some(Consent::explicit_trigger(&request)))
            .unwrap();
        assert!(matches!(
            route.connect(Duration::from_secs(1)),
            Err(Error::InvalidEndpoint)
        ));
        // Without consent, offline, or with consent for another program.
        assert!(matches!(
            online.authorize(request.clone(), None),
            Err(Error::ConsentMissing)
        ));
        let offline = Gate::new(|| Offline::Offline, Ledger::default());
        assert!(matches!(
            offline.authorize(request.clone(), Some(Consent::explicit_trigger(&request))),
            Err(Error::Offline)
        ));
        let other = Request::ai_broker(BrokerProgram::new(which("python3"), dir.path().join("x")));
        assert!(matches!(
            online.authorize(request, Some(Consent::explicit_trigger(&other))),
            Err(Error::ConsentScope)
        ));
        // A revoked consent refuses the launch itself.
        let request = Request::ai_broker(program);
        let consent = Consent::explicit_trigger(&request);
        let route = online.authorize(request, Some(consent.clone())).unwrap();
        consent.revoke();
        assert!(matches!(
            route.spawn_broker(),
            Err(BrokerError::Gate(Error::ConsentRevoked))
        ));
    }

    #[test]
    fn a_missing_node_or_script_starts_nothing_and_counts_no_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let mut gate = Gate::new(|| Offline::Online, Ledger::default());
        gate.observer = Observer::LoopbackAllowed;
        for program in [
            BrokerProgram::new(None, dir.path().join("broker.py")),
            BrokerProgram::new(which("python3"), dir.path().join("absent.py")),
        ] {
            let request = Request::ai_broker(program);
            let route = gate
                .authorize(request.clone(), Some(Consent::explicit_trigger(&request)))
                .unwrap();
            assert!(matches!(route.spawn_broker(), Err(BrokerError::Missing)));
        }
        assert!(gate.attempts().records.is_empty());
    }

    #[test]
    fn expanduser_and_which_follow_python() {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        if let Some(home) = home {
            let trimmed = home.to_str().unwrap().trim_end_matches('/').to_owned();
            assert_eq!(
                expanduser(Path::new("~/x")),
                PathBuf::from(format!("{trimmed}/x"))
            );
        }
        assert_eq!(expanduser(Path::new("/a/~b")), PathBuf::from("/a/~b"));
        assert_eq!(
            expanduser(Path::new("~no-such-user-kassiber/x")),
            PathBuf::from("~no-such-user-kassiber/x")
        );
        assert!(which("sh").is_some_and(|path| path.is_absolute()));
        assert_eq!(which("/nonexistent/node"), None);
        assert_eq!(which("no-such-command-kassiber"), None);
    }
}

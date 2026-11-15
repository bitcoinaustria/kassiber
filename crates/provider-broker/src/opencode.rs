//! The OpenCode adapter (`opencode.ts`, `opencode2.ts`): both OpenCode
//! generations behind one version switch.
//!
//! - OpenCode 1 ([`v1`]) runs `opencode serve` on a free loopback port and
//!   speaks the API the `@opencode-ai/sdk` client spoke, with sharing always
//!   off, deny-all session permissions plus the advertised Kassiber tools,
//!   and an isolated in-memory store for Incognito and selected-data turns.
//! - OpenCode 2 ([`v2`]) runs `opencode serve --stdio` with a private
//!   password and a configuration built in RAM from the user's provider
//!   settings only, so user plugins, MCP servers and instructions never load.
//!
//! Both talk HTTP only to the loopback server the adapter itself started
//! ([`http`]). Every probe and turn starts with `opencode --version`; an
//! unknown future major or an OpenCode 2 prerelease is refused before any
//! configuration is read.

use std::ffi::{OsStr, OsString};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::cleanup::{track_child, TrackedChild};
use crate::env::environment_from;
use crate::executables::{fallback_dirs, provider_command, resolve_in, run_provider, RunOptions};
use crate::protocol::{
    js_space, js_trim, provider_status, safe_error_message, sensitive_context, stateless_session,
    BrokerFailure, ChatRequest, ProviderId, ProviderState, ProviderStatus,
};
use crate::{Adapter, EventSink, ToolBridge};

pub mod http;
pub mod js;
pub mod v1;
pub mod v2;

#[cfg(test)]
mod tests;

/// The adapter [`crate::adapter`] returns for `opencode`.
pub static OPENCODE: OpenCode = OpenCode;

/// The OpenCode adapter over this process's environment.
pub struct OpenCode;

const DISPLAY_NAME: &str = "OpenCode";
const EXECUTABLE: &str = "opencode";
const PROVIDER: ProviderId = ProviderId::OpenCode;

/// What the TypeScript adapter read from `process`: the broker's
/// environment, platform, home and the free-port lookup. The adapter reads
/// nothing else of its surroundings, so tests give it their own.
#[derive(Clone)]
pub struct Host {
    /// The broker's environment (`process.env`).
    pub vars: Vec<(OsString, OsString)>,
    /// Directories searched after `PATH` for the CLI.
    pub fallbacks: Vec<PathBuf>,
    /// `process.platform === "win32"`.
    pub windows: bool,
    /// `process.platform === "darwin"`.
    pub macos: bool,
    /// System-managed OpenCode configuration locations OpenCode 2 refuses;
    /// `None` for this platform's.
    pub managed: Option<Vec<PathBuf>>,
    /// `availablePort()`: a free loopback port for OpenCode 1's server.
    pub free_port: Arc<dyn Fn() -> std::io::Result<u16> + Send + Sync>,
    /// How long an OpenCode 2 event stream gets to send its first frame
    /// (30 seconds).
    pub event_ready: Duration,
}

impl Host {
    /// This process.
    pub fn process() -> Self {
        Self {
            vars: std::env::vars_os().collect(),
            fallbacks: fallback_dirs(),
            windows: cfg!(windows),
            macos: cfg!(target_os = "macos"),
            managed: None,
            free_port: Arc::new(kassiber_net::free_loopback_port),
            event_ready: Duration::from_secs(30),
        }
    }

    /// `process.env[name]` (present, possibly empty).
    pub fn var_os(&self, name: &str) -> Option<&OsStr> {
        self.vars
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_os_str())
    }

    /// `process.env[name]` as JavaScript reads it.
    pub fn var(&self, name: &str) -> Option<String> {
        self.var_os(name)
            .map(|value| value.to_string_lossy().into_owned())
    }

    /// `providerEnvironment("opencode")`.
    pub fn provider_env(&self) -> Env {
        Env(environment_from(PROVIDER, self.vars.iter().cloned()))
    }

    /// `resolveExecutable("opencode")`.
    pub fn executable(&self) -> Option<PathBuf> {
        resolve_in(EXECUTABLE, self.var_os("PATH"), &self.fallbacks)
    }

    /// `os.homedir()`: `HOME` when set.
    pub fn home(&self) -> PathBuf {
        let name = if self.windows { "USERPROFILE" } else { "HOME" };
        match self.var_os(name) {
            Some(home) => PathBuf::from(home),
            None => std::env::home_dir().unwrap_or_default(),
        }
    }
}

/// A provider environment built up as the TypeScript adapter assigns into
/// its `env` object: a name keeps its place when reassigned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env(pub Vec<(OsString, OsString)>);

impl Env {
    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_os_str())
    }

    /// The value as JavaScript reads it.
    pub fn get_str(&self, name: &str) -> Option<String> {
        self.get(name)
            .map(|value| value.to_string_lossy().into_owned())
    }

    pub fn set(&mut self, name: &str, value: impl Into<OsString>) {
        let value = value.into();
        match self.0.iter_mut().find(|(key, _)| key == name) {
            Some(slot) => slot.1 = value,
            None => self.0.push((name.into(), value)),
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.0.retain(|(key, _)| key != name);
    }

    pub fn names(&self) -> impl Iterator<Item = String> + '_ {
        self.0
            .iter()
            .map(|(key, _)| key.to_string_lossy().into_owned())
    }

    pub fn as_slice(&self) -> &[(OsString, OsString)] {
        &self.0
    }
}

/// The adapter with an explicit [`Host`].
pub struct Adapted {
    pub host: Host,
}

impl Adapter for OpenCode {
    fn status(&self, cwd: &Path) -> Result<ProviderStatus, BrokerFailure> {
        Ok(status(&Host::process(), cwd))
    }

    fn chat(
        &self,
        request: &ChatRequest,
        cwd: &Path,
        bridge: Option<&ToolBridge>,
        events: &EventSink,
    ) -> Result<(), BrokerFailure> {
        chat(&Host::process(), request, cwd, bridge, events)
    }
}

impl Adapter for Adapted {
    fn status(&self, cwd: &Path) -> Result<ProviderStatus, BrokerFailure> {
        Ok(status(&self.host, cwd))
    }

    fn chat(
        &self,
        request: &ChatRequest,
        cwd: &Path,
        bridge: Option<&ToolBridge>,
        events: &EventSink,
    ) -> Result<(), BrokerFailure> {
        chat(&self.host, request, cwd, bridge, events)
    }
}

/// `(?:opencode\s+v?)?` stripped, when it is there: the remainders the
/// version patterns are tried against, in the order JavaScript tries them.
fn version_bodies(version: &str) -> Vec<&str> {
    let mut bodies = Vec::new();
    if let Some(rest) = version.strip_prefix("opencode") {
        let trimmed = rest.trim_start_matches(js_space);
        if trimmed.len() < rest.len() {
            bodies.push(trimmed.strip_prefix('v').unwrap_or(trimmed));
            if trimmed.starts_with('v') {
                bodies.push(trimmed);
            }
        }
    }
    bodies.push(version);
    bodies
}

/// `(\d+)\.\d+\.\d+(?:[-+][\w.+-]+)?$` from the start of `body`: the major.
fn version_major(body: &str) -> Option<&str> {
    let digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
    let major = digits(body);
    let mut rest = &body[major..];
    if major == 0 {
        return None;
    }
    for _ in 0..2 {
        rest = rest.strip_prefix('.')?;
        let count = digits(rest);
        if count == 0 {
            return None;
        }
        rest = &rest[count..];
    }
    if rest.is_empty() {
        return Some(&body[..major]);
    }
    let suffix = rest.strip_prefix(['-', '+'])?;
    (!suffix.is_empty()
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.+-".contains(&byte)))
    .then(|| &body[..major])
}

/// An OpenCode 2 version string (`/^(?:opencode\s+v?)?2\./`).
pub fn is_v2(version: &str) -> bool {
    version_bodies(version)
        .iter()
        .any(|body| body.starts_with("2."))
}

/// `checkOpenCodeVersion`: the matched version, `None` when the CLI did not
/// print one or failed; an error for a version this build refuses.
pub fn check_version(
    host: &Host,
    executable: &Path,
    cwd: &Path,
) -> Result<Option<String>, BrokerFailure> {
    // Version checks carry no prompt. On POSIX discard the CLI's automatic
    // log before even this first invocation; Windows retains the legacy v1
    // path.
    let env = if host.windows {
        host.provider_env()
    } else {
        v2::discard_environment(host, &cwd.join("version"))?
    };
    let result = run_provider(
        PROVIDER,
        executable,
        &["--version"],
        RunOptions {
            cwd: Some(cwd),
            env: Some(env.as_slice()),
            ..RunOptions::default()
        },
    )?;
    let version = js_trim(&result.stdout);
    let major = version_bodies(version)
        .into_iter()
        .find_map(version_major)
        .map(|major| major.parse::<f64>().unwrap_or(f64::INFINITY));
    if major.is_some_and(|major| major >= 3.0) {
        return Err(BrokerFailure::new(
            "This OpenCode major version is not supported by this build.",
        ));
    }
    if major == Some(2.0) && version.contains(['-', '+']) {
        return Err(BrokerFailure::new(
            "OpenCode 2 prerelease builds are not supported. Use a stable OpenCode 2 release.",
        ));
    }
    Ok((result.code == Some(0) && major.is_some()).then(|| version.to_owned()))
}

/// `openCodeStatus(cwd)`.
pub fn status(host: &Host, cwd: &Path) -> ProviderStatus {
    let Some(executable) = host.executable() else {
        return provider_status(
            PROVIDER,
            DISPLAY_NAME,
            ProviderState::MissingExecutable,
            "Install OpenCode, then run `opencode auth login` outside Kassiber.",
        );
    };
    let probe = || -> Result<_, BrokerFailure> {
        // Check compatibility before commands whose flags changed in v2.
        // Keep the probes sequential: a lost config read mislabels local
        // models.
        let version = check_version(host, &executable, cwd)?;
        let models = if version.as_deref().is_some_and(is_v2) {
            v2::models(host, &executable, cwd)?
        } else {
            v1::models(host, &executable, cwd)?
        };
        Ok((version, models))
    };
    match probe() {
        Ok((version, models)) => {
            let ready = !models.is_empty();
            let status = provider_status(
                PROVIDER,
                DISPLAY_NAME,
                if ready {
                    ProviderState::Ready
                } else {
                    ProviderState::AuthenticationRequired
                },
                if ready {
                    "Ready using the existing OpenCode configuration."
                } else {
                    "Run `opencode auth login` outside Kassiber."
                },
            )
            .with_executable(&executable.to_string_lossy())
            .with_models(models);
            match version {
                Some(version) => status.with_version(version),
                None => status,
            }
        }
        Err(failure) => provider_status(
            PROVIDER,
            DISPLAY_NAME,
            ProviderState::Error,
            safe_error_message(failure.message()),
        )
        .with_executable(&executable.to_string_lossy()),
    }
}

/// `openCodeChat(request, cwd, toolBridge)`.
pub fn chat(
    host: &Host,
    request: &ChatRequest,
    cwd: &Path,
    bridge: Option<&ToolBridge>,
    events: &EventSink,
) -> Result<(), BrokerFailure> {
    let sensitive = sensitive_context(request)?;
    let stateless = stateless_session(request)?;
    if stateless && !sensitive {
        v1::check_incognito_platform(host.windows)?;
    }
    let executable = host
        .executable()
        .ok_or_else(|| BrokerFailure::new("OpenCode is not installed."))?;
    let version = check_version(host, &executable, cwd)?;
    if version.as_deref().is_some_and(is_v2) {
        return v2::chat(host, &executable, request, cwd, bridge, events);
    }
    v1::chat(
        host,
        &executable,
        request,
        cwd,
        bridge,
        events,
        sensitive,
        stateless,
    )
}

/// `model.indexOf("/")` when it splits `provider/model`.
pub fn model_separator(model: &str) -> Result<usize, BrokerFailure> {
    match model.find('/') {
        Some(at) if at > 0 && at + 1 < model.len() => Ok(at),
        _ => Err(BrokerFailure::new(
            "OpenCode models must use provider/model format.",
        )),
    }
}

/// How much of one output line is kept; the rest of the line is read and
/// dropped, so a CLI cannot grow the broker's memory without bound.
const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Read `stream` line by line as Node's `readline` splits it (`\n`, `\r\n`
/// or `\r`), sending each line until the receiver goes; then keep draining
/// so the CLI never blocks on a full pipe.
fn forward_lines(stream: impl Read + Send + 'static, lines: mpsc::Sender<String>) {
    let _ = std::thread::Builder::new()
        .name("opencode-output".into())
        .spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut pending = Vec::new();
            let mut buffer = [0u8; 8192];
            let mut skip_newline = false;
            let send = |pending: &mut Vec<u8>| {
                let line = String::from_utf8_lossy(pending).into_owned();
                pending.clear();
                let _ = lines.send(line);
            };
            loop {
                let read = match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                };
                for &byte in &buffer[..read] {
                    if skip_newline {
                        skip_newline = false;
                        if byte == b'\n' {
                            continue;
                        }
                    }
                    match byte {
                        b'\n' => send(&mut pending),
                        b'\r' => {
                            send(&mut pending);
                            skip_newline = true;
                        }
                        _ if pending.len() < MAX_LINE_BYTES => pending.push(byte),
                        _ => {}
                    }
                }
            }
            if !pending.is_empty() {
                send(&mut pending);
            }
        });
}

/// A running `opencode serve`.
pub struct ServerProcess {
    child: TrackedChild,
    stdin: Option<ChildStdin>,
}

/// How a server start ended without being ready.
pub enum NotReady {
    /// The launch itself failed.
    Spawn(std::io::Error),
    /// It exited first, with this code (`None` after a signal).
    Exited(Option<i32>),
    TimedOut,
}

/// How long a server gets to announce itself.
const STARTUP: Duration = Duration::from_secs(30);

impl ServerProcess {
    /// Start `opencode <args>` with piped standard streams and wait until
    /// `ready` accepts a line of its stdout (and of its stderr when
    /// `stderr_lines`), returning what it accepted. A server that is not
    /// ready is killed.
    pub fn start<T>(
        executable: &Path,
        args: &[String],
        cwd: &Path,
        env: &Env,
        stderr_lines: bool,
        mut ready: impl FnMut(&str) -> Option<T>,
    ) -> Result<(Self, T), NotReady> {
        let mut command =
            provider_command(PROVIDER, executable, args, Some(cwd), Some(env.as_slice()));
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child =
            crate::executables::spawn_provider(&mut command).map_err(NotReady::Spawn)?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let child = track_child(child);
        let (sender, lines) = mpsc::channel();
        if let Some(stdout) = stdout {
            forward_lines(stdout, sender.clone());
        }
        if let Some(stderr) = stderr {
            if stderr_lines {
                forward_lines(stderr, sender.clone());
            } else {
                // Drain bounded process output without retaining diagnostics
                // or secrets.
                let (discard, _) = mpsc::channel();
                forward_lines(stderr, discard);
            }
        }
        drop(sender);
        let server = Self { child, stdin };
        match server.wait_ready(&lines, &mut ready) {
            Ok(accepted) => Ok((server, accepted)),
            Err(error) => {
                let _ = server.child.kill();
                Err(error)
            }
        }
    }

    fn wait_ready<T>(
        &self,
        lines: &Receiver<String>,
        ready: &mut impl FnMut(&str) -> Option<T>,
    ) -> Result<T, NotReady> {
        let deadline = Instant::now() + STARTUP;
        let mut open = true;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(NotReady::TimedOut);
            }
            if open {
                match lines.recv_timeout(left.min(Duration::from_millis(20))) {
                    Ok(line) => match ready(&line) {
                        Some(accepted) => return Ok(accepted),
                        None => continue,
                    },
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => open = false,
                }
            } else {
                std::thread::sleep(left.min(Duration::from_millis(20)));
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                // Lines it printed before exiting still count.
                while let Ok(line) = lines.try_recv() {
                    if let Some(accepted) = ready(&line) {
                        return Ok(accepted);
                    }
                }
                return Err(NotReady::Exited(status.code()));
            }
        }
    }

    /// Whether it has exited.
    pub fn exited(&self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    /// `child.kill("SIGTERM")`.
    pub fn terminate(&self) {
        #[cfg(unix)]
        let _ = self.child.terminate();
        #[cfg(not(unix))]
        let _ = self.child.kill();
    }

    /// `child.kill("SIGKILL")`.
    pub fn kill(&self) {
        let _ = self.child.kill();
    }

    /// End its stdin.
    pub fn close_stdin(&mut self) {
        self.stdin.take();
    }

    /// Wait at most `timeout` for it to exit.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        matches!(self.child.wait_timeout(timeout), Ok(Some(_)))
    }

    pub fn wait(&self) {
        let _ = self.child.wait();
    }
}

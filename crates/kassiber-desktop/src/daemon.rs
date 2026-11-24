//! The `kassiber daemon` supervisor: the same process, protocol and failure rules as the Tauri
//! shell's (ui-tauri/src-tauri/src/supervisor.rs), driven from the UI thread without blocking it.
//!
//! - **Launch.** The binary resolves as Tauri's does: `--kassiber-bin` / `KASSIBER_BIN` (a
//!   debugging override), the bundled sidecar (next to the app binary, or the bundle's
//!   `Resources/binaries/kassiber-cli/`), then the source checkout's `target/{release,debug}`.
//!   Without an explicit data root the CLI's `--migrate-default-state-root` runs first, as Tauri's
//!   setup does; a failed migration fails closed (no daemon starts).
//! - **Ready handshake.** A spawned process must print `daemon.ready` within 30 s; a watchdog
//!   kills it otherwise. Requests written before then wait in the pipe (the daemon answers them
//!   only after its ready record).
//! - **Records.** A reader thread classifies every stdout line ([`classify_line`]); the UI thread
//!   routes request records ([`route_record`]): the terminal record of a request is its exact
//!   kind, `error` or `auth_required`; streaming kinds may send progress records first. A record
//!   before `daemon.ready`, a post-ready record without a string `request_id` (other than an
//!   `event: true` record), an event record carrying one, invalid JSON, or a non-terminal record
//!   for a non-streaming request breaks the process (docs/reference/daemon.md, request routing).
//! - **Exit.** When a process exits or breaks, every request it had pending fails with an error
//!   envelope (`daemon_exited`, …). Nothing restarts it by itself: the next request respawns it,
//!   behind a short backoff only when the previous process died before serving anything.
//! - **Diagnostics.** A 16 KB stderr tail per process and a 64-entry lifecycle ring stay in RAM,
//!   secret-floor redacted at insert; the Logs screen folds the ring in ([`Supervisor::lifecycle`]).
//! - **Data root.** `ui.projects.select`/`create` move the root later respawns use (as Tauri's
//!   `note_project_data_root_from_response`); [`Supervisor::begin_switch`] restarts against a new
//!   root and preference environment, warming the replacement (polled, never awaited on the UI
//!   thread) before the old process goes.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// A cold first launch (the OS still validating a freshly installed sidecar) can take far longer
/// than a warm one (supervisor.rs).
pub const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(30);
pub const STDERR_TAIL_LIMIT: usize = 16 * 1024;
pub const LIFECYCLE_RING_CAPACITY: usize = 64;
/// A process that dies this soon after starting, without answering anything, counts as a failed
/// start for the respawn backoff.
const QUICK_EXIT: Duration = Duration::from_secs(5);
const BACKOFF_FIRST: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

use crate::secrets::bridge::SECRET_STORE_CONTROL_REQUEST_KIND;

// ---- Errors ---------------------------------------------------------------------------------

/// `SupervisorError`: what a request gets instead of a daemon answer.
#[derive(Clone, Debug, PartialEq)]
pub struct DaemonError {
    pub code: &'static str,
    pub message: String,
    pub hint: Option<&'static str>,
    pub details: Option<Value>,
    pub retryable: bool,
}

impl DaemonError {
    pub fn new(code: &'static str, message: impl Into<String>) -> DaemonError {
        DaemonError {
            code,
            message: message.into(),
            hint: None,
            details: None,
            retryable: false,
        }
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    pub fn hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// Details pass the secret floor (sensitive keys and secret-shaped strings), as Tauri's do.
    pub fn details(mut self, details: Value) -> Self {
        self.details = Some(redact_value(details, 0));
        self
    }

    pub fn with_stderr_tail(mut self, tail: &str) -> Self {
        let tail = redact(tail);
        if tail.is_empty() {
            return self;
        }
        let mut map = match self.details.take() {
            Some(Value::Object(map)) => map,
            Some(other) => Map::from_iter([("details".to_owned(), other)]),
            None => Map::new(),
        };
        map.insert("stderr_tail".into(), Value::String(tail));
        self.details = Some(Value::Object(map));
        self
    }

    /// The standard error envelope, echoing `request_id`.
    pub fn envelope(&self, request_id: &str) -> Value {
        let mut error =
            json!({"code": self.code, "message": self.message, "retryable": self.retryable});
        if let Some(h) = self.hint {
            error["hint"] = json!(h);
        }
        if let Some(d) = &self.details {
            error["details"] = d.clone();
        }
        json!({"kind": "error", "schema_version": 1, "request_id": request_id, "error": error})
    }
}

/// The shared secret floor (the Logs screen's port of the backend's `redact_secret_text`).
pub fn redact(text: &str) -> String {
    crate::reports::diag::redact::secret_floor_text(text)
}

/// Keys whose values never leave redacted (docs/reference/daemon.md, reveal kinds).
const SENSITIVE_KEYS: &[&str] = &[
    "passphrase_secret",
    "passphrase",
    "token",
    "descriptor",
    "change_descriptor",
    "blinding_key",
    "auth_header",
    "password",
    "api_key",
    "secret",
    "xpub",
    "mnemonic",
];

/// `redact_sensitive_value`: sensitive keys (the backend's `is_sensitive_key` plus the reveal
/// kinds' keys above) become `[redacted]`, strings pass the secret floor,
/// nesting past eight levels becomes `[truncated]`.
fn redact_value(value: Value, depth: usize) -> Value {
    if depth > 8 {
        return Value::String("[truncated]".into());
    }
    match value {
        Value::String(s) => Value::String(redact(&s)),
        Value::Array(a) => {
            Value::Array(a.into_iter().map(|v| redact_value(v, depth + 1)).collect())
        }
        Value::Object(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| {
                    let lower = k.to_ascii_lowercase();
                    if crate::secret_floor::floor::is_sensitive_key(&k)
                        || SENSITIVE_KEYS
                            .iter()
                            .any(|s| lower == *s || lower.ends_with(&format!("_{s}")))
                    {
                        (k, Value::String("[redacted]".into()))
                    } else {
                        (k, redact_value(v, depth + 1))
                    }
                })
                .collect(),
        ),
        other => other,
    }
}

// ---- Line classification (reader thread) ----------------------------------------------------

/// One stdout line, classified before it reaches the UI.
#[derive(Debug, PartialEq)]
pub enum Line {
    Ready(Value),
    Record {
        request_id: String,
        envelope: Value,
    },
    Event(Value),
    /// The daemon asks the desktop's secret store something (answered on the reader thread).
    Control(Value),
    /// A protocol violation: the process is broken.
    Fatal(DaemonError),
}

pub fn classify_line(line: &str, ready_seen: bool) -> Line {
    let envelope: Value = match serde_json::from_str(line.trim()) {
        Ok(v) => v,
        Err(e) => {
            let sample: String = line.trim().chars().take(200).collect();
            return Line::Fatal(
                DaemonError::new(
                    "daemon_protocol_error",
                    format!("Daemon emitted invalid JSON: {e}"),
                )
                .details(json!({"line": sample})),
            );
        }
    };
    let kind = envelope.get("kind").and_then(Value::as_str).unwrap_or("");
    let request_id = envelope.get("request_id");
    if kind == "daemon.ready" && request_id.is_none() && !ready_seen {
        return Line::Ready(envelope);
    }
    if !ready_seen {
        return Line::Fatal(
            DaemonError::new(
                "daemon_protocol_error",
                "Daemon emitted a request response before daemon.ready",
            )
            .details(json!({"kind": kind})),
        );
    }
    if kind == SECRET_STORE_CONTROL_REQUEST_KIND {
        return Line::Control(envelope);
    }
    if envelope.get("event").and_then(Value::as_bool) == Some(true) {
        if request_id.is_some() {
            return Line::Fatal(
                DaemonError::new(
                    "daemon_protocol_error",
                    "Daemon emitted an event record with a request_id",
                )
                .details(json!({"kind": kind})),
            );
        }
        if kind.trim().is_empty() {
            return Line::Fatal(DaemonError::new(
                "daemon_protocol_error",
                "Daemon emitted an event record without a kind",
            ));
        }
        return Line::Event(envelope);
    }
    match request_id {
        Some(Value::String(id)) => Line::Record {
            request_id: id.clone(),
            envelope,
        },
        Some(other) => Line::Fatal(
            DaemonError::new(
                "daemon_request_id_mismatch",
                "Daemon response request_id did not match the active request",
            )
            .details(json!({"actual": other, "kind": kind})),
        ),
        None => Line::Fatal(
            DaemonError::new(
                "daemon_protocol_error",
                "Daemon emitted a response without request_id",
            )
            .details(json!({"kind": kind})),
        ),
    }
}

/// What a record means for the request it answers.
#[derive(Debug, PartialEq)]
pub enum Route {
    /// The request's last record.
    Terminal,
    /// A streaming request's intermediate record.
    Progress,
    /// The pre-streaming `progress` passthrough non-streaming kinds ignore.
    Ignore,
    Violation(DaemonError),
}

pub fn route_record(request_kind: &str, streaming: bool, record_kind: &str) -> Route {
    if record_kind == request_kind || record_kind == "error" || record_kind == "auth_required" {
        Route::Terminal
    } else if streaming {
        Route::Progress
    } else if record_kind == "progress" {
        Route::Ignore
    } else {
        Route::Violation(
            DaemonError::new(
                "daemon_protocol_error",
                "Daemon emitted a non-terminal record for a non-streaming request",
            )
            .details(json!({"kind": record_kind, "request_kind": request_kind})),
        )
    }
}

// ---- Launch ---------------------------------------------------------------------------------

/// How the daemon is started: the data root and the binary override.
#[derive(Clone, Debug)]
pub struct Launch {
    /// `--data-root` given to the daemon; None lets the CLI open its default state root.
    pub data_root: Option<PathBuf>,
    /// The root came from the command line or the dev env (not the default state root).
    pub explicit_root: bool,
    /// `--kassiber-bin`.
    pub bin_flag: Option<PathBuf>,
    /// The binary the launch resolved to (display and the terminal command's target).
    pub binary: PathBuf,
    /// Extra environment for the daemon process: the demo book's own preference files
    /// (`crate::demo`), empty otherwise.
    pub env: Vec<(String, String)>,
}

impl Launch {
    /// `--data-root`, `--kassiber-bin`. Tooling builds also take `KASSIBER_DEMO_DATA_ROOT` (the
    /// headless harness's disposable book); a release build ignores it, so a leftover variable
    /// can never point the shipped app at another data root.
    pub fn from_args(args: &[String]) -> Launch {
        let flag = |name: &str| {
            args.iter()
                .position(|a| a == name)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        let env_root = || {
            cfg!(feature = "tooling")
                .then(|| std::env::var("KASSIBER_DEMO_DATA_ROOT").ok())
                .flatten()
                .filter(|v| !v.is_empty())
        };
        let data_root = flag("--data-root").or_else(env_root).map(PathBuf::from);
        let mut l = Launch {
            explicit_root: data_root.is_some(),
            data_root,
            bin_flag: flag("--kassiber-bin").map(PathBuf::from),
            binary: PathBuf::from("kassiber"),
            env: Vec::new(),
        };
        if let Ok(cmd) = l.command() {
            l.binary = cmd.program;
        }
        l
    }

    pub fn command(&self) -> Result<DaemonCommand, DaemonError> {
        let env_bin = self
            .bin_flag
            .clone()
            .map(|p| p.into_os_string())
            .or_else(|| std::env::var_os("KASSIBER_BIN"));
        // Shipped builds never run a binary from the build checkout: a missing sidecar is a clear
        // spawn error instead.
        resolve_command(
            env_bin,
            &resource_dirs(),
            cfg!(feature = "tooling").then(repo_root),
        )
    }
}

/// A resolved CLI invocation.
#[derive(Clone, Debug, PartialEq)]
pub struct DaemonCommand {
    pub program: PathBuf,
    pub cwd: PathBuf,
    /// `env_bin`, `bundled_sidecar` or `checkout_build` (lifecycle records name it).
    pub source: &'static str,
}

impl DaemonCommand {
    fn failure_hint(&self) -> &'static str {
        match self.source {
            "bundled_sidecar" => {
                "The bundled Kassiber CLI sidecar failed to start; reinstall the desktop package or set KASSIBER_BIN to override it."
            }
            "env_bin" => "Set KASSIBER_BIN to a working kassiber binary, or unset it.",
            _ => {
                "Rebuild the kassiber binary with `cargo build -p kassiber --bin kassiber`, or set KASSIBER_BIN."
            }
        }
    }
}

/// `resolve_kassiber_command`: the override, the bundled sidecar, the checkout build.
pub fn resolve_command(
    env_bin: Option<std::ffi::OsString>,
    resource_dirs: &[PathBuf],
    checkout_root: Option<PathBuf>,
) -> Result<DaemonCommand, DaemonError> {
    if let Some(bin) = env_bin.filter(|v| !v.is_empty()) {
        return Ok(DaemonCommand {
            program: PathBuf::from(bin),
            cwd: std::env::temp_dir(),
            source: "env_bin",
        });
    }
    // Packaged macOS builds inherit the bundle's read-only Contents/Resources as cwd; the sidecar
    // gets a writable scratch directory instead.
    if let Some(sidecar) = resource_dirs.iter().find_map(|d| bundled_sidecar(d)) {
        return Ok(DaemonCommand {
            program: sidecar,
            cwd: std::env::temp_dir(),
            source: "bundled_sidecar",
        });
    }
    if let Some(root) = checkout_root
        && let Some(bin) = checkout_build(&root)
    {
        return Ok(DaemonCommand {
            program: bin,
            cwd: root,
            source: "checkout_build",
        });
    }
    Err(DaemonError::new(
        "daemon_binary_missing",
        "Kassiber daemon binary not found: the bundled kassiber sidecar is missing",
    )
    .hint("Reinstall the desktop package, or set KASSIBER_BIN to a kassiber binary."))
}

/// Where a packaged app keeps its resources (`resource_dir_before_tauri`, plus the directory of
/// the app binary itself for a plain install on macOS and Windows). Never the app binary's own
/// directory on Linux: an installed package puts the `kassiber` forwarder (`exec kassiber-ui
/// --cli`) beside `kassiber-ui` in `/usr/bin`, and the daemon must start the sidecar itself, as
/// `native::cli::resource_dirs` does for terminal forwarding.
pub fn resource_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(app_dir) = std::env::var_os("APPDIR").filter(|v| !v.is_empty()) {
        out.push(
            PathBuf::from(app_dir)
                .join("usr")
                .join("lib")
                .join("Kassiber"),
        );
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        if cfg!(target_os = "macos")
            && let Some(contents) = parent.parent()
        {
            out.push(contents.join("Resources"));
        }
        if !cfg!(target_os = "linux") {
            out.push(parent.to_path_buf());
        }
    }
    if cfg!(target_os = "linux") {
        out.push(PathBuf::from("/usr/lib/Kassiber"));
    }
    out
}

/// `bundled_sidecar`: the one-dir sidecar, the flat `binaries/` one, the bare one, then a plain
/// `kassiber` next to the app binary.
pub fn bundled_sidecar(dir: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(name) = sidecar_filename() {
        candidates.push(dir.join("binaries").join("kassiber-cli").join(&name));
        candidates.push(dir.join("binaries").join(&name));
        candidates.push(dir.join(&name));
    }
    candidates.push(dir.join(format!("kassiber{}", std::env::consts::EXE_SUFFIX)));
    // `is_file`, not `exists`: a directory only fails later, as a confusing spawn error.
    candidates.into_iter().find(|p| p.is_file())
}

pub fn sidecar_filename() -> Option<String> {
    let triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return None,
    };
    Some(format!(
        "kassiber-cli-{triple}{}",
        std::env::consts::EXE_SUFFIX
    ))
}

/// `target/release/kassiber`, then `target/debug/kassiber`, under the checkout.
pub fn checkout_build(root: &Path) -> Option<PathBuf> {
    let exe = format!("kassiber{}", std::env::consts::EXE_SUFFIX);
    ["release", "debug"]
        .into_iter()
        .map(|p| root.join("target").join(p).join(&exe))
        .find(|p| p.is_file())
}

/// `KASSIBER_REPO_ROOT`, else the checkout this app was built in (`crates/kassiber-desktop/../..`).
fn repo_root() -> PathBuf {
    if let Ok(p) = std::env::var("KASSIBER_REPO_ROOT") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// ---- Default state root (lib.rs `default_state_root`) -----------------------------------------

#[allow(dead_code)]
const DEFAULT_STATE_DIR: &str = ".kassiber";
#[allow(dead_code)]
const NATIVE_STATE_DIR: &str = "at.bitcoinaustria.kassiber";
#[allow(dead_code)]
const DEFAULT_PROJECTS_DIR: &str = "projects";
#[allow(dead_code)]
const DEFAULT_PROJECT_ID: &str = "default";
#[allow(dead_code)]
const DEFAULT_DATA_DIR: &str = "data";
#[allow(dead_code)]
const STATE_ROOT_MIGRATION_FILENAME: &str = ".state-root-migration.json";

#[allow(dead_code)]
pub fn home_dir() -> Option<PathBuf> {
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))
    } else {
        std::env::var_os("HOME")
    };
    #[allow(deprecated)]
    home.map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::home_dir().filter(|p| p.is_absolute()))
}

#[allow(dead_code)]
fn state_root_has_user_state(path: &Path) -> bool {
    match path.read_dir() {
        Ok(mut entries) => entries.any(|e| match e {
            Ok(e) => match e.file_name().to_str() {
                Some("bin" | "run") => false,
                Some(n) if n == STATE_ROOT_MIGRATION_FILENAME => false,
                _ => true,
            },
            Err(_) => true,
        }),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

#[allow(dead_code)]
pub fn native_state_root_for_platform(
    platform: &str,
    home: Option<&Path>,
    xdg_data_home: Option<&Path>,
    local_app_data: Option<&Path>,
) -> Option<PathBuf> {
    match platform {
        "linux" => xdg_data_home
            .filter(|p| p.is_absolute())
            .map(|p| p.join("kassiber"))
            .or_else(|| home.map(|p| p.join(".local").join("share").join("kassiber"))),
        "macos" => home.map(|p| {
            p.join("Library")
                .join("Application Support")
                .join(NATIVE_STATE_DIR)
        }),
        "windows" => local_app_data
            .filter(|p| p.is_absolute())
            .map(|p| p.join(NATIVE_STATE_DIR))
            .or_else(|| home.map(|p| p.join("AppData").join("Local").join(NATIVE_STATE_DIR))),
        _ => home.map(|p| p.join(DEFAULT_STATE_DIR)),
    }
}

/// The native per-user directory, unless the legacy `~/.kassiber` still holds user state.
#[allow(dead_code)]
pub fn state_root_for_platform(
    platform: &str,
    home: Option<&Path>,
    xdg_data_home: Option<&Path>,
    local_app_data: Option<&Path>,
) -> Option<PathBuf> {
    let preferred = native_state_root_for_platform(platform, home, xdg_data_home, local_app_data)?;
    if let Some(legacy) = home.map(|p| p.join(DEFAULT_STATE_DIR))
        && preferred != legacy
        && state_root_has_user_state(&legacy)
    {
        return Some(legacy);
    }
    Some(preferred)
}

#[allow(dead_code)]
fn platform() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "other"
    }
}

/// The default state root exactly as the Tauri shell resolves it (`default_state_root`).
#[allow(dead_code)] // the onboarding / welcome screens use it
pub fn default_state_root() -> Option<PathBuf> {
    let home = home_dir();
    let xdg = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let lad = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    state_root_for_platform(platform(), home.as_deref(), xdg.as_deref(), lad.as_deref())
}

/// `default_state_data_root`: the default project's data directory.
#[allow(dead_code)] // the onboarding / welcome screens use it
pub fn default_state_data_root() -> Option<PathBuf> {
    default_state_root().map(|r| {
        r.join(DEFAULT_PROJECTS_DIR)
            .join(DEFAULT_PROJECT_ID)
            .join(DEFAULT_DATA_DIR)
    })
}

/// The pre-launch `kassiber --migrate-default-state-root` (lib.rs setup): quiet on success, a
/// typed error on stderr otherwise.
pub fn migrate_default_state_root(cmd: &DaemonCommand) -> Result<(), DaemonError> {
    let mut c = Command::new(&cmd.program);
    c.arg("--migrate-default-state-root")
        .current_dir(&cmd.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    hide_console_window(&mut c);
    let out = c.output().map_err(|e| {
        DaemonError::new(
            "daemon_spawn_failed",
            format!("Could not start Kassiber CLI with {:?}: {e}", cmd.program),
        )
        .hint(cmd.failure_hint())
    })?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    Err(DaemonError::new(
        "state_root_migration_failed",
        format!(
            "default state-root migration failed with status {}",
            out.status.code().unwrap_or(1)
        ),
    )
    .with_stderr_tail(&stderr))
}

// ---- One process ----------------------------------------------------------------------------

#[cfg(windows)]
fn hide_console_window(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    c.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console_window(_c: &mut Command) {}

#[derive(Clone, Default)]
struct StderrTail(Arc<Mutex<Vec<u8>>>, Arc<AtomicBool>);

impl StderrTail {
    fn append(&self, chunk: &[u8]) {
        let mut b = self.0.lock().unwrap();
        b.extend_from_slice(chunk);
        if b.len() > STDERR_TAIL_LIMIT {
            let over = b.len() - STDERR_TAIL_LIMIT;
            b.drain(..over);
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }

    /// The pipe reached its end: everything the process wrote is in the tail.
    fn close(&self) {
        self.1.store(true, Ordering::SeqCst);
    }

    /// Waits up to `wait` for the pipe's end, so a dead process's last words are in the tail.
    fn settle(&self, wait: Duration) {
        let deadline = Instant::now() + wait;
        while !self.1.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// What the reader thread hands the UI thread.
#[derive(Debug)]
pub enum Incoming {
    Ready(Value),
    /// Any record carrying a `request_id`: progress records of a stream, then its terminal one.
    Record {
        request_id: String,
        envelope: Value,
    },
    Event(Value),
    /// The process is gone or broken; every request it had pending fails with this.
    Exited(DaemonError),
}

type Queue = Arc<Mutex<Vec<(u64, Incoming)>>>;

struct Process {
    pgen: u64,
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<ChildStdin>>,
    queue: Queue,
    stderr: StderrTail,
    broken: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    spawned_at: Instant,
    /// A terminal record arrived: the process served something.
    answered: bool,
    source: &'static str,
}

fn push(queue: &Queue, wake: &Arc<Mutex<Option<Wake>>>, pgen: u64, item: Incoming) {
    queue.lock().unwrap().push((pgen, item));
    if let Some(w) = wake.lock().unwrap().as_ref() {
        w();
    }
}

/// Marks the process broken; true for the first caller only (who then reports it).
fn break_once(broken: &AtomicBool) -> bool {
    !broken.swap(true, Ordering::SeqCst)
}

fn kill(child: &Mutex<Child>) {
    if let Ok(mut c) = child.lock() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

impl Process {
    fn spawn(
        pgen: u64,
        cmd: &DaemonCommand,
        data_root: Option<&Path>,
        env: &[(String, String)],
        wake: &Arc<Mutex<Option<Wake>>>,
        ready_timeout: Duration,
    ) -> Result<Process, DaemonError> {
        let mut c = Command::new(&cmd.program);
        c.envs(env.iter().map(|(k, v)| (k, v)));
        if let Some(root) = data_root {
            c.arg("--data-root").arg(root);
        }
        c.arg("daemon")
            .current_dir(&cmd.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_console_window(&mut c);
        let mut busy_retries = 0u8;
        let mut child = loop {
            match c.spawn() {
                Ok(child) => break child,
                // Linux can briefly return ETXTBSY while an atomically installed sidecar is still
                // being released by the file system; retry only that.
                Err(e) if cfg!(unix) && e.raw_os_error() == Some(26) && busy_retries < 3 => {
                    busy_retries += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    return Err(DaemonError::new(
                        "daemon_spawn_failed",
                        format!(
                            "Could not start Kassiber daemon with {:?}: {e}",
                            cmd.program
                        ),
                    )
                    .hint(cmd.failure_hint())
                    .details(json!({"source": cmd.source}))
                    .retryable());
                }
            }
        };
        let stdin = Arc::new(Mutex::new(child.stdin.take().ok_or_else(|| {
            DaemonError::new("daemon_spawn_failed", "Daemon stdin was not captured")
        })?));
        let stdout = child.stdout.take().ok_or_else(|| {
            DaemonError::new("daemon_spawn_failed", "Daemon stdout was not captured")
        })?;
        let stderr_pipe = child.stderr.take().ok_or_else(|| {
            DaemonError::new("daemon_spawn_failed", "Daemon stderr was not captured")
        })?;
        let child = Arc::new(Mutex::new(child));
        let queue: Queue = Arc::default();
        let stderr = StderrTail::default();
        let broken = Arc::new(AtomicBool::new(false));
        let ready = Arc::new(AtomicBool::new(false));
        let thread_err = |e: std::io::Error| DaemonError::new("daemon_spawn_failed", e.to_string());

        // stderr: a bounded tail, never logged as such.
        {
            let tail = stderr.clone();
            std::thread::Builder::new()
                .name("kassiber-daemon-stderr".into())
                .spawn(move || {
                    let mut r = BufReader::new(stderr_pipe);
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = r.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        tail.append(&buf[..n]);
                    }
                    tail.close();
                })
                .map_err(thread_err)?;
        }

        // stdout: classify each line.
        {
            let (queue, wake, broken, ready, child, stdin) = (
                queue.clone(),
                wake.clone(),
                broken.clone(),
                ready.clone(),
                child.clone(),
                stdin.clone(),
            );
            std::thread::Builder::new()
                .name("kassiber-daemon-reader".into())
                .spawn(move || {
                    let mut r = BufReader::new(stdout);
                    let fail = |err: DaemonError| {
                        if break_once(&broken) {
                            kill(&child);
                            push(&queue, &wake, pgen, Incoming::Exited(err));
                        }
                    };
                    loop {
                        let mut line = String::new();
                        match r.read_line(&mut line) {
                            Ok(0) => {
                                return fail(
                                    DaemonError::new("daemon_exited", "Daemon closed stdout")
                                        .retryable(),
                                );
                            }
                            Err(e) => {
                                return fail(
                                    DaemonError::new(
                                        "daemon_read_failed",
                                        format!("Could not read from daemon stdout: {e}"),
                                    )
                                    .retryable(),
                                );
                            }
                            Ok(_) => {}
                        }
                        if line.trim().is_empty() {
                            continue;
                        }
                        match classify_line(&line, ready.load(Ordering::SeqCst)) {
                            Line::Ready(env) => {
                                ready.store(true, Ordering::SeqCst);
                                push(&queue, &wake, pgen, Incoming::Ready(env));
                            }
                            Line::Record {
                                request_id,
                                mut envelope,
                            } => {
                                crate::secrets::bridge::attach_policy_status(&mut envelope);
                                push(
                                    &queue,
                                    &wake,
                                    pgen,
                                    Incoming::Record {
                                        request_id,
                                        envelope,
                                    },
                                )
                            }
                            Line::Event(env) => push(&queue, &wake, pgen, Incoming::Event(env)),
                            Line::Control(req) => {
                                if let Err(e) = answer_control(&stdin, req) {
                                    return fail(e);
                                }
                            }
                            Line::Fatal(err) => return fail(err),
                        }
                    }
                })
                .map_err(thread_err)?;
        }

        // The ready watchdog.
        {
            let (queue, wake, broken, ready, child) = (
                queue.clone(),
                wake.clone(),
                broken.clone(),
                ready.clone(),
                child.clone(),
            );
            std::thread::Builder::new()
                .name("kassiber-daemon-ready".into())
                .spawn(move || {
                    let deadline = Instant::now() + ready_timeout;
                    while Instant::now() < deadline {
                        if ready.load(Ordering::SeqCst) || broken.load(Ordering::SeqCst) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    if !ready.load(Ordering::SeqCst) && break_once(&broken) {
                        kill(&child);
                        let err = DaemonError::new(
                            "daemon_timeout",
                            format!(
                                "Daemon did not answer within {} seconds",
                                ready_timeout.as_secs()
                            ),
                        )
                        .retryable();
                        push(&queue, &wake, pgen, Incoming::Exited(err));
                    }
                })
                .map_err(thread_err)?;
        }

        Ok(Process {
            pgen,
            child,
            stdin,
            queue,
            stderr,
            broken,
            ready,
            spawned_at: Instant::now(),
            answered: false,
            source: cmd.source,
        })
    }

    fn write(&self, line: &Value) -> Result<(), DaemonError> {
        let mut s = self.stdin.lock().map_err(|_| {
            DaemonError::new("daemon_lock_poisoned", "daemon stdin lock is poisoned").retryable()
        })?;
        writeln!(s, "{line}").and_then(|_| s.flush()).map_err(|e| {
            DaemonError::new(
                "daemon_write_failed",
                format!("Could not write to daemon stdin: {e}"),
            )
            .retryable()
        })
    }

    fn is_broken(&self) -> bool {
        self.broken.load(Ordering::SeqCst)
    }

    /// The exit status once the process is gone (waits up to `wait` for it).
    fn status(&self, wait: Duration) -> Option<String> {
        let deadline = Instant::now() + wait;
        loop {
            if let Ok(Some(s)) = self.child.lock().unwrap().try_wait() {
                return Some(s.to_string());
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if !self.is_broken() {
            let _ = self.write(&json!({"request_id": "shutdown", "kind": "daemon.shutdown"}));
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                if let Ok(Some(_)) = self.child.lock().unwrap().try_wait() {
                    self.broken.store(true, Ordering::SeqCst);
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        self.broken.store(true, Ordering::SeqCst);
        kill(&self.child);
    }
}

/// The daemon asks the OS credential store for an AI provider key (`crate::secrets::bridge`):
/// answered on stdin, never logged; the request (a `set` carries the key) and the answer (a
/// `get` carries it) are cleared once written.
fn answer_control(stdin: &Mutex<ChildStdin>, req: Value) -> Result<(), DaemonError> {
    let reply =
        crate::secrets::bridge::control_response_line(&req, &*crate::secrets::bridge::store());
    crate::secrets::bridge::scrub(req);
    let mut s = stdin
        .lock()
        .map_err(|_| DaemonError::new("daemon_lock_poisoned", "daemon stdin lock is poisoned"))?;
    writeln!(s, "{}", reply.as_str())
        .and_then(|_| s.flush())
        .map_err(|e| {
            DaemonError::new(
                "secret_store_bridge_failed",
                format!("Could not write secret-store bridge response: {e}"),
            )
            .retryable()
        })
}

// ---- Lifecycle ring -------------------------------------------------------------------------

/// A daemon lifecycle event for the Logs screen (`LifecycleRecord`); redacted at insert.
#[derive(Clone, Debug)]
pub struct LifecycleRecord {
    pub id: u64,
    pub ts_ms: u64,
    pub event: &'static str,
    pub detail: String,
    pub stderr_tail: String,
    pub source: &'static str,
}

impl LifecycleRecord {
    /// `lifecycleLevel` (daemonLogBridge.ts): spawns and clean exits are info, the rest errors.
    pub fn level(&self) -> &'static str {
        if self.event == "spawned" || (self.event == "exited" && clean_status(&self.detail)) {
            "info"
        } else {
            "error"
        }
    }
}

/// `/\bstatus:?\s*0\b/i`.
fn clean_status(detail: &str) -> bool {
    let d = detail.to_ascii_lowercase();
    d.match_indices("status").any(|(i, _)| {
        let rest = d[i + 6..].trim_start_matches(':').trim_start();
        rest.starts_with('0') && !rest[1..].starts_with(|c: char| c.is_ascii_alphanumeric())
    })
}

// ---- Backoff --------------------------------------------------------------------------------

/// Respawn pacing: only a process that died before serving anything delays the next start
/// (1 s, doubling, up to 30 s); anything it answered resets it.
#[derive(Clone, Debug, Default)]
pub struct Backoff {
    pub failures: u32,
    pub until: Option<Instant>,
}

impl Backoff {
    pub fn fail(&mut self, now: Instant) {
        self.failures += 1;
        let factor = 1u32 << (self.failures - 1).min(5);
        self.until = Some(now + (BACKOFF_FIRST * factor).min(BACKOFF_MAX));
    }

    pub fn reset(&mut self) {
        *self = Backoff::default();
    }

    /// Time left before another start, if any.
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.until.filter(|u| *u > now).map(|u| u - now)
    }
}

// ---- Supervisor -----------------------------------------------------------------------------

pub struct Supervisor {
    pub launch: Launch,
    process: Option<Process>,
    /// Records of a broken process retired before they were drained.
    carry: Vec<(u64, Incoming)>,
    /// That process, kept until `drain` recorded its exit.
    retired: Option<Process>,
    wake: Arc<Mutex<Option<Wake>>>,
    lifecycle: VecDeque<LifecycleRecord>,
    next_lifecycle: u64,
    next_request: u64,
    next_gen: u64,
    pub backoff: Backoff,
    /// The launch failed closed (state-root migration): no daemon starts.
    blocked: Option<DaemonError>,
    pub ready_timeout: Duration,
    /// A data-root switch warming its replacement ([`Supervisor::begin_switch`]).
    switching: Option<PendingSwitch>,
}

/// The replacement process of a data-root switch, until its ready record (or failure).
struct PendingSwitch {
    next: Process,
    root: Option<PathBuf>,
    env: Vec<(String, String)>,
    deadline: Instant,
}

impl Supervisor {
    /// Resolves the launch and runs the pre-launch migration; starts nothing yet.
    pub fn new(launch: Launch) -> Supervisor {
        let mut s = Supervisor {
            launch,
            process: None,
            carry: Vec::new(),
            retired: None,
            wake: Arc::default(),
            lifecycle: VecDeque::new(),
            next_lifecycle: 1,
            next_request: 0,
            next_gen: 0,
            backoff: Backoff::default(),
            blocked: None,
            ready_timeout: DAEMON_READY_TIMEOUT,
            switching: None,
        };
        // Unit tests never start a daemon on the default state root (the owner's real books):
        // without an explicit data root the supervisor is blocked and every request fails.
        if cfg!(test) && !s.launch.explicit_root {
            s.blocked = Some(DaemonError::new(
                "daemon_spawn_failed",
                "Tests need an explicit data root (KASSIBER_DEMO_DATA_ROOT)",
            ));
            return s;
        }
        // Tauri runs the migration before anything else; an explicit data root (command line,
        // dev env) bypasses the default state root, so nothing there is migrated.
        if !s.launch.explicit_root {
            match s.launch.command() {
                Ok(cmd) => {
                    if let Err(e) = migrate_default_state_root(&cmd) {
                        let tail = e
                            .details
                            .as_ref()
                            .and_then(|d| d["stderr_tail"].as_str())
                            .unwrap_or("")
                            .to_owned();
                        s.record("migration_failed", &e.message, &tail, cmd.source);
                        s.blocked = Some(e);
                    }
                }
                Err(e) => s.record("spawn_failed", &e.message, "", "missing"),
            }
        }
        s
    }

    pub fn set_waker(&self, wake: Wake) {
        *self.wake.lock().unwrap() = Some(wake);
    }

    /// Whether requests can be served at all (false only when the launch failed closed).
    pub fn is_some(&self) -> bool {
        self.blocked.is_none()
    }

    /// A live, unbroken process exists.
    #[allow(dead_code)]
    pub fn running(&self) -> bool {
        self.process.as_ref().is_some_and(|p| !p.is_broken())
    }

    /// The configured data root (what a respawn opens).
    #[allow(dead_code)]
    pub fn data_root(&self) -> Option<&Path> {
        self.launch.data_root.as_deref()
    }

    /// The live process's id (tests and diagnostics).
    pub fn pid(&self) -> Option<u32> {
        self.process.as_ref().map(|p| p.child.lock().unwrap().id())
    }

    pub fn alloc_id(&mut self) -> String {
        self.next_request += 1;
        format!("rune-{}", self.next_request)
    }

    fn record(
        &mut self,
        event: &'static str,
        detail: &str,
        stderr_tail: &str,
        source: &'static str,
    ) {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if self.lifecycle.len() == LIFECYCLE_RING_CAPACITY {
            self.lifecycle.pop_front();
        }
        self.lifecycle.push_back(LifecycleRecord {
            id: self.next_lifecycle,
            ts_ms,
            event,
            detail: redact(detail),
            stderr_tail: redact(stderr_tail),
            source,
        });
        self.next_lifecycle += 1;
    }

    /// `lifecycle_snapshot(after_id)`: records newer than `after`, and the newest id.
    pub fn lifecycle(&self, after: u64) -> (Vec<LifecycleRecord>, u64) {
        let last = self.lifecycle.back().map(|r| r.id).unwrap_or(0);
        (
            self.lifecycle
                .iter()
                .filter(|r| r.id > after)
                .cloned()
                .collect(),
            last,
        )
    }

    fn spawn(
        &mut self,
        root: Option<PathBuf>,
        env: &[(String, String)],
    ) -> Result<Process, DaemonError> {
        if let Some(e) = &self.blocked {
            return Err(e.clone());
        }
        let cmd = match self.launch.command() {
            Ok(c) => c,
            Err(e) => {
                self.record("spawn_failed", &e.message, "", "missing");
                return Err(e);
            }
        };
        self.next_gen += 1;
        match Process::spawn(
            self.next_gen,
            &cmd,
            root.as_deref(),
            env,
            &self.wake,
            self.ready_timeout,
        ) {
            Ok(p) => Ok(p),
            Err(e) => {
                self.record("spawn_failed", &e.message, "", cmd.source);
                Err(e)
            }
        }
    }

    /// `ensure_process`: the live process, or a new one (unless the backoff says wait).
    fn ensure(&mut self) -> Result<&mut Process, DaemonError> {
        if self.process.as_ref().is_some_and(|p| p.is_broken()) {
            self.retire();
        }
        if self.process.is_none() {
            // A launch that failed closed stays closed (and is not a failed start to pace).
            if let Some(e) = &self.blocked {
                return Err(e.clone());
            }
            if let Some(left) = self.backoff.wait(Instant::now()) {
                return Err(DaemonError::new(
                    "daemon_unavailable",
                    format!("The Kassiber daemon stopped right after starting; Kassiber tries again in {} s.", left.as_secs().max(1)),
                )
                .retryable());
            }
            let env = self.launch.env.clone();
            match self.spawn(self.launch.data_root.clone(), &env) {
                Ok(p) => self.process = Some(p),
                Err(e) => {
                    self.backoff.fail(Instant::now());
                    return Err(e);
                }
            }
        }
        Ok(self.process.as_mut().expect("process"))
    }

    /// Drops a broken process; its undrained records (its exit among them) reach the next
    /// `drain`, so its pending requests still fail. The thread that broke it may still be between
    /// marking it broken and queueing its exit (the window of `kill()`); that late record would
    /// land in a queue nobody drains any more, so the exit is queued here instead, and the
    /// process's queue is never read again: every pending request fails exactly once.
    fn retire(&mut self) {
        let Some(p) = self.process.take() else { return };
        let left = std::mem::take(&mut *p.queue.lock().unwrap());
        let has_exit = left
            .iter()
            .any(|(g, m)| *g == p.pgen && matches!(m, Incoming::Exited(_)));
        self.carry.extend(left);
        if !has_exit {
            self.carry.push((
                p.pgen,
                Incoming::Exited(DaemonError::new("daemon_exited", "Daemon exited").retryable()),
            ));
        }
        // `drain` records the exit; keep the process until then for its tail and status.
        self.retired = Some(p);
    }

    /// Starts the daemon now, without waiting for its ready record (tests; the app starts it
    /// with its first request).
    #[cfg(test)]
    #[cfg_attr(not(unix), allow(dead_code))] // the unix-only stub-daemon tests
    pub fn start(&mut self) -> Result<(), DaemonError> {
        self.ensure().map(|_| ())
    }

    /// Writes one request; returns the process generation that answers it.
    pub fn request(
        &mut self,
        request_id: &str,
        kind: &str,
        args: &Value,
    ) -> Result<u64, DaemonError> {
        let p = self.ensure()?;
        // AI provider kinds may keep keys in the OS store (`augment_ai_provider_secret_args`).
        let bridged = crate::secrets::bridge::augment_args(kind, args);
        let args = bridged.as_ref().unwrap_or(args);
        let line = if args.is_null() {
            json!({"request_id": request_id, "kind": kind})
        } else {
            json!({"request_id": request_id, "kind": kind, "args": args})
        };
        match p.write(&line) {
            Ok(()) => Ok(p.pgen),
            Err(e) => {
                // The pipe is gone: the process is broken, and every request it had fails.
                let (pgen, tail) = (p.pgen, p.stderr.text());
                if break_once(&p.broken) {
                    kill(&p.child);
                    p.queue
                        .lock()
                        .unwrap()
                        .push((pgen, Incoming::Exited(e.clone())));
                }
                Err(e.with_stderr_tail(&tail))
            }
        }
    }

    /// The process served a request: a later exit is not a failed start.
    pub fn note_answer(&mut self, pgen: u64) {
        if let Some(p) = self.process.as_mut().filter(|p| p.pgen == pgen) {
            p.answered = true;
        }
        self.backoff.reset();
    }

    /// Everything the processes said since the last call. An exit is recorded in the lifecycle
    /// ring with its stderr tail, and the process dropped.
    pub fn drain(&mut self) -> Vec<(u64, Incoming)> {
        let mut raw: Vec<(u64, Incoming)> = std::mem::take(&mut self.carry);
        if let Some(p) = &self.process {
            raw.extend(std::mem::take(&mut *p.queue.lock().unwrap()));
        }
        let mut out = Vec::with_capacity(raw.len());
        for (pgen, m) in raw {
            match m {
                Incoming::Ready(env) => {
                    let source = self
                        .process
                        .as_ref()
                        .filter(|p| p.pgen == pgen)
                        .map(|p| p.source)
                        .unwrap_or("");
                    self.record("spawned", "daemon ready", "", source);
                    out.push((pgen, Incoming::Ready(env)));
                }
                Incoming::Exited(err) => {
                    let err = self.on_exit(pgen, err);
                    out.push((pgen, Incoming::Exited(err)));
                }
                other => out.push((pgen, other)),
            }
        }
        out
    }

    /// Records a process's exit and drops it; the error its pending requests fail with.
    fn on_exit(&mut self, pgen: u64, mut err: DaemonError) -> DaemonError {
        let p = match self.process.take() {
            Some(p) if p.pgen == pgen => Some(p),
            other => {
                self.process = other;
                self.retired.take().filter(|p| p.pgen == pgen)
            }
        };
        let Some(p) = p else { return err };
        // Let the stderr thread catch the last words.
        let status = p.status(Duration::from_millis(200));
        p.stderr.settle(Duration::from_millis(if status.is_some() {
            500
        } else {
            20
        }));
        let tail = p.stderr.text();
        let was_ready = p.ready.load(Ordering::SeqCst);
        let quick = !p.answered && p.spawned_at.elapsed() < QUICK_EXIT;
        let event = if !was_ready {
            "spawn_failed"
        } else if matches!(
            err.code,
            "daemon_exited" | "daemon_read_failed" | "daemon_write_failed"
        ) {
            "exited"
        } else {
            "killed"
        };
        let detail = match (event, &status) {
            ("exited", Some(s)) => s.clone(),
            _ => err.message.clone(),
        };
        self.record(event, &detail, &tail, "");
        if let (Some(s), "daemon_exited") = (&status, err.code) {
            err.message = format!("Daemon exited before answering: {s}");
        }
        if quick || !was_ready {
            self.backoff.fail(Instant::now());
        }
        drop(p);
        err.with_stderr_tail(&tail)
    }

    /// A protocol violation found on the UI thread: kill the process, record it, and return the
    /// error its pending requests fail with. The next request respawns.
    pub fn break_process(&mut self, pgen: u64, err: DaemonError) -> DaemonError {
        let Some(p) = self.process.as_ref().filter(|p| p.pgen == pgen) else {
            return err;
        };
        let tail = p.stderr.text();
        break_once(&p.broken);
        kill(&p.child);
        self.record("killed", err.code, &tail, "");
        // Whatever else it queued is moot.
        self.process = None;
        err.with_stderr_tail(&tail)
    }

    /// The live process's stderr tail, redacted (attached to `internal_error` envelopes).
    pub fn stderr_tail(&self, pgen: u64) -> String {
        self.process
            .as_ref()
            .filter(|p| p.pgen == pgen)
            .map(|p| redact(&p.stderr.text()))
            .unwrap_or_default()
    }

    /// `note_project_data_root_from_response`: later respawns open the selected project.
    pub fn note_project_root(&mut self, root: PathBuf) {
        self.launch.data_root = Some(root);
    }

    /// `set_data_root` / `clear_data_root`: restart against another data root and preference
    /// environment, without blocking the UI thread. The replacement starts now and is warmed
    /// (its ready record awaited, up to the ready timeout) by [`Supervisor::poll_switch`]; until
    /// then the current process keeps serving. Ok(false) when nothing would change. A switch
    /// still warming is abandoned for the new one.
    pub fn begin_switch(
        &mut self,
        root: Option<PathBuf>,
        env: Vec<(String, String)>,
    ) -> Result<bool, DaemonError> {
        self.cancel_switch();
        if self.launch.data_root == root && self.launch.env == env {
            return Ok(false);
        }
        let next = self.spawn(root.clone(), &env)?;
        self.switching = Some(PendingSwitch {
            next,
            root,
            env,
            deadline: Instant::now() + self.ready_timeout,
        });
        Ok(true)
    }

    /// A switch is warming its replacement.
    #[cfg(test)]
    #[cfg_attr(not(unix), allow(dead_code))] // the unix-only stub-daemon tests
    pub fn switching(&self) -> bool {
        self.switching.is_some()
    }

    /// Abandons a warming switch (its process is killed); the current one stays.
    pub fn cancel_switch(&mut self) {
        if let Some(sw) = self.switching.take() {
            break_once(&sw.next.broken);
            kill(&sw.next.child);
        }
    }

    /// The warming switch's outcome once there is one: Ok(Some(old pgen)) when a process was
    /// replaced (its pending requests are the caller's to fail), Ok(None) when none ran; on an
    /// error the current process stays. None while it still warms.
    pub fn poll_switch(&mut self) -> Option<Result<Option<u64>, DaemonError>> {
        let sw = self.switching.as_ref()?;
        let first = sw
            .next
            .queue
            .lock()
            .unwrap()
            .first()
            .map(|(_, m)| matches!(m, Incoming::Ready(_)));
        match first {
            Some(true) => {}
            Some(false) => {
                // Gone before ready (a timeout, an early exit, a protocol error).
                let sw = self.switching.take().expect("switching");
                let items = std::mem::take(&mut *sw.next.queue.lock().unwrap());
                let err = items
                    .into_iter()
                    .find_map(|(_, m)| {
                        if let Incoming::Exited(e) = m {
                            Some(e)
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| {
                        DaemonError::new(
                            "daemon_protocol_error",
                            "Daemon did not emit daemon.ready on startup",
                        )
                    });
                let tail = sw.next.stderr.text();
                self.record("spawn_failed", &err.message, &tail, sw.next.source);
                return Some(Err(err.with_stderr_tail(&tail)));
            }
            None if Instant::now() >= sw.deadline => {
                let sw = self.switching.take().expect("switching");
                let tail = sw.next.stderr.text();
                break_once(&sw.next.broken);
                kill(&sw.next.child);
                let err = DaemonError::new(
                    "daemon_timeout",
                    format!(
                        "Daemon did not answer within {} seconds",
                        self.ready_timeout.as_secs()
                    ),
                )
                .retryable();
                self.record("spawn_failed", &err.message, &tail, sw.next.source);
                return Some(Err(err.with_stderr_tail(&tail)));
            }
            None => return None,
        }
        // The ready record stays queued for `drain` (which records "spawned").
        let sw = self.switching.take().expect("switching");
        let old = self.process.replace(sw.next);
        self.launch.data_root = sw.root;
        self.launch.env = sw.env;
        self.backoff.reset();
        Some(Ok(old.map(|p| {
            let tail = p.stderr.text();
            break_once(&p.broken);
            kill(&p.child);
            self.record("replaced", "data root changed", &tail, "");
            p.pgen
        })))
    }
}

#[cfg(test)]
pub(crate) mod tests;

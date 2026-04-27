use serde_json::{json, Map, Value};
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(5);
const DAEMON_INVOKE_TIMEOUT: Duration = Duration::from_secs(15);
/// Per-record inactivity timeout for streaming kinds. The recv clock resets
/// every time a delta arrives, so a long-running stream stays alive as long
/// as the daemon keeps producing output within the window.
const DAEMON_STREAM_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(90);
const STDERR_TAIL_LIMIT: usize = 16 * 1024;

pub struct DaemonSupervisor {
    process: Option<DaemonProcess>,
    resource_dir: Option<PathBuf>,
    next_request_id: u64,
}

struct DaemonProcess {
    child: Child,
    stdin: ChildStdin,
    stdout_rx: mpsc::Receiver<Result<Value, SupervisorError>>,
    stderr_tail: StderrTail,
    broken: Arc<AtomicBool>,
}

#[derive(Clone)]
struct StderrTail {
    bytes: Arc<Mutex<Vec<u8>>>,
}

#[derive(Debug)]
pub struct SupervisorError {
    pub code: &'static str,
    pub message: String,
    pub hint: Option<&'static str>,
    pub details: Option<Value>,
    pub retryable: bool,
}

impl SupervisorError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
            details: None,
            retryable: false,
        }
    }

    fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    fn hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint);
        self
    }

    fn details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    fn with_stderr_tail(mut self, stderr_tail: String) -> Self {
        if stderr_tail.is_empty() {
            return self;
        }

        let mut map = match self.details.take() {
            Some(Value::Object(map)) => map,
            Some(details) => {
                let mut map = Map::new();
                map.insert("details".to_string(), details);
                map
            }
            None => Map::new(),
        };
        map.insert("stderr_tail".to_string(), Value::String(stderr_tail));
        self.details = Some(Value::Object(map));
        self
    }
}

impl DaemonSupervisor {
    pub fn new(resource_dir: Option<PathBuf>) -> Self {
        Self {
            process: None,
            resource_dir,
            next_request_id: 1,
        }
    }

    pub fn invoke(
        &mut self,
        kind: &str,
        args: Option<Value>,
        app: &AppHandle,
        streaming: bool,
    ) -> Result<Value, SupervisorError> {
        let request_id = self.allocate_request_id();
        let process = self.ensure_process()?;
        let mut request = json!({
            "request_id": request_id,
            "kind": kind,
        });
        if let Some(args) = args {
            request["args"] = args;
        }

        process.write_json_line(&request)?;

        // For streaming kinds we use a per-record inactivity timeout so a slow
        // model that keeps emitting tokens stays alive past the 15s
        // total-budget. Non-streaming kinds keep the original total deadline.
        let stream_prefix = format!("{kind}.");
        let deadline = if streaming {
            None
        } else {
            Some(Instant::now() + DAEMON_INVOKE_TIMEOUT)
        };

        loop {
            let remaining = if let Some(deadline) = deadline {
                deadline.saturating_duration_since(Instant::now())
            } else {
                DAEMON_STREAM_INACTIVITY_TIMEOUT
            };
            let mut response = process.read_json_value(remaining)?;

            let response_kind = response
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            // Treat any record sharing this request's id and a kind that
            // starts with "<request_kind>." (e.g. ai.chat.delta) as a
            // mid-stream record: forward to the webview as a Tauri event,
            // reset the inactivity clock, keep reading.
            let is_stream_record = streaming
                && (response_kind == "progress" || response_kind.starts_with(&stream_prefix));

            if is_stream_record {
                if response.get("request_id").and_then(Value::as_str) != Some(request_id.as_str()) {
                    process.mark_broken();
                    process.kill();
                    return Err(request_id_mismatch(&request_id, &response)
                        .with_stderr_tail(process.stderr_tail()));
                }
                // Single channel per app; the webview filters by request_id
                // from the payload so we don't need a per-stream listener.
                if let Err(error) = app.emit("daemon://stream", &response) {
                    eprintln!("kassiber: failed to emit stream event: {error}");
                }
                continue;
            }

            // Pre-streaming "progress" passthrough kept for the existing
            // protocol surface; non-streaming kinds still ignore them.
            if !streaming && response_kind == "progress" {
                if response.get("request_id").and_then(Value::as_str) != Some(request_id.as_str()) {
                    process.mark_broken();
                    process.kill();
                    return Err(request_id_mismatch(&request_id, &response)
                        .with_stderr_tail(process.stderr_tail()));
                }
                continue;
            }

            if response.get("request_id").and_then(Value::as_str) != Some(request_id.as_str()) {
                process.mark_broken();
                process.kill();
                return Err(request_id_mismatch(&request_id, &response)
                    .with_stderr_tail(process.stderr_tail()));
            }
            attach_stderr_tail_to_internal_error(&mut response, process.stderr_tail());
            return Ok(response);
        }
    }

    fn allocate_request_id(&mut self) -> String {
        let request_id = format!("tauri-{}", self.next_request_id);
        self.next_request_id += 1;
        request_id
    }

    fn ensure_process(&mut self) -> Result<&mut DaemonProcess, SupervisorError> {
        let should_restart = match self.process.as_mut() {
            Some(process) => {
                if process.is_broken() {
                    process.kill();
                    self.process = None;
                    true
                } else {
                    match process.child.try_wait() {
                        Ok(Some(status)) => {
                            let stderr_tail = process.stderr_tail();
                            self.process = None;
                            return Err(SupervisorError::new(
                                "daemon_exited",
                                format!(
                                    "Python daemon exited before handling the request: {status}"
                                ),
                            )
                            .hint("Restart the Tauri shell and check the daemon smoke test output.")
                            .with_stderr_tail(stderr_tail)
                            .retryable());
                        }
                        Ok(None) => false,
                        Err(error) => {
                            self.process = None;
                            return Err(SupervisorError::new(
                                "daemon_status_failed",
                                format!("Could not inspect Python daemon status: {error}"),
                            )
                            .retryable());
                        }
                    }
                }
            }
            None => true,
        };

        if should_restart {
            self.process = Some(DaemonProcess::spawn(self.resource_dir.as_deref())?);
        }

        self.process.as_mut().ok_or_else(|| {
            SupervisorError::new("daemon_unavailable", "Python daemon is unavailable")
        })
    }
}

impl DaemonProcess {
    fn spawn(resource_dir: Option<&Path>) -> Result<Self, SupervisorError> {
        let command = kassiber_command(resource_dir, vec!["daemon".into()]);
        let mut child = Command::new(&command.program)
            .args(&command.args)
            .current_dir(&command.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                SupervisorError::new(
                    "daemon_spawn_failed",
                    format!(
                        "Could not start Kassiber daemon with {:?}: {error}",
                        command.program
                    ),
                )
                .hint(command.failure_hint())
                .details(json!({
                    "cwd": command.cwd,
                    "source": command.source,
                }))
                .retryable()
            })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            SupervisorError::new(
                "daemon_spawn_failed",
                "Python daemon stdin was not captured",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            SupervisorError::new(
                "daemon_spawn_failed",
                "Python daemon stdout was not captured",
            )
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            SupervisorError::new(
                "daemon_spawn_failed",
                "Python daemon stderr was not captured",
            )
        })?;
        let stderr_tail = StderrTail::spawn(stderr);

        let broken = Arc::new(AtomicBool::new(false));
        let mut process = Self {
            child,
            stdin,
            stdout_rx: spawn_stdout_reader(stdout, Arc::clone(&broken)),
            stderr_tail,
            broken,
        };
        let ready = process.read_json_value(DAEMON_READY_TIMEOUT)?;
        if ready.get("kind").and_then(Value::as_str) != Some("daemon.ready") {
            return Err(SupervisorError::new(
                "daemon_protocol_error",
                "Python daemon did not emit daemon.ready on startup",
            )
            .details(ready)
            .with_stderr_tail(process.stderr_tail()));
        }

        Ok(process)
    }

    fn write_json_line(&mut self, payload: &Value) -> Result<(), SupervisorError> {
        let line = serde_json::to_string(payload).map_err(|error| {
            SupervisorError::new(
                "daemon_protocol_error",
                format!("Could not serialize daemon request: {error}"),
            )
        })?;
        writeln!(self.stdin, "{line}").map_err(|error| {
            SupervisorError::new(
                "daemon_write_failed",
                format!("Could not write to Python daemon stdin: {error}"),
            )
            .retryable()
        })?;
        self.stdin.flush().map_err(|error| {
            SupervisorError::new(
                "daemon_write_failed",
                format!("Could not flush Python daemon stdin: {error}"),
            )
            .retryable()
        })
    }

    fn read_json_value(&mut self, timeout: Duration) -> Result<Value, SupervisorError> {
        match self.stdout_rx.recv_timeout(timeout) {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => {
                self.mark_broken();
                self.kill();
                Err(error.with_stderr_tail(self.stderr_tail()))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.mark_broken();
                self.kill();
                Err(SupervisorError::new(
                    "daemon_timeout",
                    format!(
                        "Python daemon did not answer within {} seconds",
                        timeout.as_secs()
                    ),
                )
                .with_stderr_tail(self.stderr_tail())
                .retryable())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.mark_broken();
                self.kill();
                Err(
                    SupervisorError::new("daemon_exited", "Python daemon stdout reader stopped")
                        .with_stderr_tail(self.stderr_tail())
                        .retryable(),
                )
            }
        }
    }

    fn stderr_tail(&self) -> String {
        self.stderr_tail.text()
    }

    fn is_broken(&self) -> bool {
        self.broken.load(Ordering::SeqCst)
    }

    fn mark_broken(&self) {
        self.broken.store(true, Ordering::SeqCst);
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct DaemonCommand {
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    source: &'static str,
}

pub fn run_cli(resource_dir: Option<&Path>, args: Vec<String>) -> i32 {
    let command = kassiber_command(resource_dir, args);
    match Command::new(&command.program)
        .args(&command.args)
        .current_dir(&command.cwd)
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!(
                "Could not start Kassiber CLI with {:?}: {error}",
                command.program
            );
            eprintln!("{}", command.failure_hint());
            1
        }
    }
}

impl DaemonCommand {
    fn failure_hint(&self) -> &'static str {
        match self.source {
            "bundled_sidecar" => {
                "The bundled Kassiber CLI sidecar failed to start; reinstall the desktop package or set KASSIBER_PYTHON to override it."
            }
            _ => "Set KASSIBER_PYTHON to a Python with Kassiber importable.",
        }
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

impl StderrTail {
    fn spawn(stderr: ChildStderr) -> Self {
        let tail = Self {
            bytes: Arc::new(Mutex::new(Vec::new())),
        };
        let thread_tail = tail.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut buffer = [0; 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => return,
                    Ok(count) => thread_tail.append(&buffer[..count]),
                    Err(_) => return,
                }
            }
        });
        tail
    }

    fn append(&self, chunk: &[u8]) {
        if let Ok(mut bytes) = self.bytes.lock() {
            bytes.extend_from_slice(chunk);
            if bytes.len() > STDERR_TAIL_LIMIT {
                let overflow = bytes.len() - STDERR_TAIL_LIMIT;
                bytes.drain(..overflow);
            }
        }
    }

    fn text(&self) -> String {
        self.bytes
            .lock()
            .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
            .unwrap_or_default()
    }
}

fn spawn_stdout_reader(
    stdout: ChildStdout,
    broken: Arc<AtomicBool>,
) -> mpsc::Receiver<Result<Value, SupervisorError>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    broken.store(true, Ordering::SeqCst);
                    let _ = tx.send(Err(SupervisorError::new(
                        "daemon_exited",
                        "Python daemon closed stdout",
                    )
                    .retryable()));
                    return;
                }
                Ok(_) => match serde_json::from_str(line.trim()) {
                    Ok(response) => {
                        if tx.send(Ok(response)).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        broken.store(true, Ordering::SeqCst);
                        let _ = tx.send(Err(SupervisorError::new(
                            "daemon_protocol_error",
                            format!("Python daemon emitted invalid JSON: {error}"),
                        )
                        .details(json!({ "line": line.trim() }))));
                        return;
                    }
                },
                Err(error) => {
                    broken.store(true, Ordering::SeqCst);
                    let _ = tx.send(Err(SupervisorError::new(
                        "daemon_read_failed",
                        format!("Could not read from Python daemon stdout: {error}"),
                    )
                    .retryable()));
                    return;
                }
            }
        }
    });
    rx
}

fn attach_stderr_tail_to_internal_error(response: &mut Value, stderr_tail: String) {
    if stderr_tail.is_empty()
        || response.get("kind").and_then(Value::as_str) != Some("error")
        || response
            .get("error")
            .and_then(Value::as_object)
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
            != Some("internal_error")
    {
        return;
    }

    let Some(error) = response.get_mut("error").and_then(Value::as_object_mut) else {
        return;
    };
    match error.get_mut("details") {
        Some(Value::Object(details)) => {
            details.insert("stderr_tail".to_string(), Value::String(stderr_tail));
        }
        Some(Value::Null) | None => {
            error.insert("details".to_string(), json!({ "stderr_tail": stderr_tail }));
        }
        Some(details) => {
            let previous = details.take();
            *details = json!({
                "details": previous,
                "stderr_tail": stderr_tail,
            });
        }
    }
}

fn request_id_mismatch(expected: &str, response: &Value) -> SupervisorError {
    SupervisorError::new(
        "daemon_request_id_mismatch",
        "Python daemon response request_id did not match the active request",
    )
    .details(json!({
        "expected": expected,
        "actual": response.get("request_id").cloned().unwrap_or(Value::Null),
        "kind": response.get("kind").cloned().unwrap_or(Value::Null),
    }))
}

fn repo_root() -> PathBuf {
    if let Ok(path) = env::var("KASSIBER_REPO_ROOT") {
        // Trust explicit user overrides even when stale; the daemon error then points at the chosen path.
        return PathBuf::from(path);
    }

    let build_repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(PathBuf::from);
    if let Some(path) = build_repo_root {
        if path.exists() {
            return path;
        }
    }

    // Packaged previews use this as a venv search root before falling back to python3.
    env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_python(repo_root: &PathBuf) -> PathBuf {
    let unix_venv = repo_root.join(".venv").join("bin").join("python");
    if unix_venv.exists() {
        return unix_venv;
    }

    let windows_venv = repo_root.join(".venv").join("Scripts").join("python.exe");
    if windows_venv.exists() {
        return windows_venv;
    }

    PathBuf::from("python3")
}

fn kassiber_command(resource_dir: Option<&Path>, args: Vec<String>) -> DaemonCommand {
    if let Ok(python) = env::var("KASSIBER_PYTHON") {
        let repo_root = repo_root();
        let mut python_args = vec!["-m".into(), "kassiber".into()];
        python_args.extend(args);
        return DaemonCommand {
            program: PathBuf::from(python),
            args: python_args,
            cwd: repo_root,
            source: "env_python",
        };
    }

    if let Some(sidecar) = bundled_sidecar(resource_dir) {
        // PyInstaller should not need the app resource directory as cwd; keeping
        // it stable prevents accidental writes relative to a developer checkout.
        let cwd = resource_dir
            .map(PathBuf::from)
            .or_else(|| sidecar.parent().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        return DaemonCommand {
            program: sidecar,
            args,
            cwd,
            source: "bundled_sidecar",
        };
    }

    let repo_root = repo_root();
    let mut python_args = vec!["-m".into(), "kassiber".into()];
    python_args.extend(args);
    DaemonCommand {
        program: default_python(&repo_root),
        args: python_args,
        cwd: repo_root,
        source: "python_fallback",
    }
}

fn bundled_sidecar(resource_dir: Option<&Path>) -> Option<PathBuf> {
    let resource_dir = resource_dir?;
    let sidecar = sidecar_filename()?;
    [
        // Packaged Tauri builds place resources under the configured
        // `binaries/` directory. The flat fallback keeps manually assembled
        // dev bundles easy to smoke-test.
        resource_dir.join("binaries").join(&sidecar),
        resource_dir.join(&sidecar),
    ]
    .into_iter()
    .find(|path| path.exists())
}

fn sidecar_filename() -> Option<String> {
    let triple = match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        // Not built by the prerelease workflow yet; Linux arm64 falls back to
        // developer Python unless a matching sidecar is manually bundled.
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return None,
    };
    let extension = if env::consts::OS == "windows" {
        ".exe"
    } else {
        ""
    };
    Some(format!("kassiber-cli-{triple}{extension}"))
}

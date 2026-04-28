use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
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
    process: Mutex<Option<Arc<DaemonProcess>>>,
    resource_dir: Option<PathBuf>,
    next_request_id: AtomicU64,
}

struct DaemonProcess {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<String, mpsc::Sender<Result<Value, SupervisorError>>>>>,
    stderr_tail: StderrTail,
    broken: Arc<AtomicBool>,
}

#[derive(Clone)]
struct StderrTail {
    bytes: Arc<Mutex<Vec<u8>>>,
}

#[derive(Debug, Clone)]
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
            process: Mutex::new(None),
            resource_dir,
            next_request_id: AtomicU64::new(1),
        }
    }

    #[cfg(test)]
    fn new_with_process(process: DaemonProcess) -> Self {
        Self {
            process: Mutex::new(Some(Arc::new(process))),
            resource_dir: None,
            next_request_id: AtomicU64::new(1),
        }
    }

    pub fn invoke(
        &self,
        kind: &str,
        args: Option<Value>,
        app: &AppHandle,
        streaming: bool,
        client_request_id: Option<Value>,
    ) -> Result<Value, SupervisorError> {
        self.invoke_inner(kind, args, streaming, client_request_id, |response| {
            // Single channel per app; the webview filters by request_id
            // from the payload so we don't need a per-stream listener.
            if let Err(error) = app.emit("daemon://stream", response) {
                eprintln!("kassiber: failed to emit stream event: {error}");
            }
        })
    }

    fn invoke_inner<F>(
        &self,
        kind: &str,
        args: Option<Value>,
        streaming: bool,
        client_request_id: Option<Value>,
        mut emit_stream: F,
    ) -> Result<Value, SupervisorError>
    where
        F: FnMut(&Value),
    {
        // Honor a JS-supplied String request_id so streaming transports can
        // filter `daemon://stream` records as they arrive without buffering;
        // fall back to the supervisor-allocated id otherwise.
        let request_id = client_request_id
            .as_ref()
            .and_then(|value| value.as_str().map(String::from))
            .unwrap_or_else(|| self.allocate_request_id());
        let process = self.ensure_process()?;
        let (tx, rx) = mpsc::channel();
        process.register_request(request_id.clone(), tx)?;

        let mut request = json!({
            "request_id": request_id,
            "kind": kind,
        });
        if let Some(args) = args {
            request["args"] = args;
        }

        if let Err(error) = process.write_json_line(&request) {
            process.unregister_request(&request_id);
            process.mark_broken();
            process.kill();
            return Err(error.with_stderr_tail(process.stderr_tail()));
        }

        // For streaming kinds we use a per-record inactivity timeout so a slow
        // model that keeps emitting tokens stays alive past the 15s
        // total-budget. Non-streaming kinds keep the original total deadline.
        let deadline = if streaming {
            None
        } else {
            Some(Instant::now() + DAEMON_INVOKE_TIMEOUT)
        };

        let result = loop {
            let remaining = if let Some(deadline) = deadline {
                deadline.saturating_duration_since(Instant::now())
            } else {
                DAEMON_STREAM_INACTIVITY_TIMEOUT
            };
            let mut response = match rx.recv_timeout(remaining) {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => break Err(error.with_stderr_tail(process.stderr_tail())),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    process.mark_broken();
                    process.kill();
                    break Err(SupervisorError::new(
                        "daemon_timeout",
                        format!(
                            "Python daemon did not answer within {} seconds",
                            remaining.as_secs()
                        ),
                    )
                    .with_stderr_tail(process.stderr_tail())
                    .retryable());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    process.mark_broken();
                    process.kill();
                    break Err(SupervisorError::new(
                        "daemon_exited",
                        "Python daemon stdout reader stopped",
                    )
                    .with_stderr_tail(process.stderr_tail())
                    .retryable());
                }
            };

            let response_kind = response
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            if response.get("request_id").and_then(request_id_value_key)
                != Some(request_id.as_str())
            {
                process.mark_broken();
                process.kill();
                break Err(request_id_mismatch(&request_id, &response)
                    .with_stderr_tail(process.stderr_tail()));
            }

            if response_kind == kind || response_kind == "error" {
                attach_stderr_tail_to_internal_error(&mut response, process.stderr_tail());
                break Ok(response);
            }

            if streaming {
                emit_stream(&response);
                continue;
            }

            // Pre-streaming "progress" passthrough kept for the existing
            // protocol surface; non-streaming kinds still ignore them.
            if response_kind == "progress" {
                continue;
            }

            process.mark_broken();
            process.kill();
            break Err(SupervisorError::new(
                "daemon_protocol_error",
                "Python daemon emitted a non-terminal record for a non-streaming request",
            )
            .details(json!({
                "request_id": request_id,
                "kind": response_kind,
            }))
            .with_stderr_tail(process.stderr_tail()));
        };
        process.unregister_request(&request_id);
        result
    }

    fn allocate_request_id(&self) -> String {
        let request_id = self.next_request_id.fetch_add(1, Ordering::SeqCst);
        format!("tauri-{request_id}")
    }

    fn ensure_process(&self) -> Result<Arc<DaemonProcess>, SupervisorError> {
        let mut slot = self.process.lock().map_err(|_| {
            SupervisorError::new("daemon_lock_poisoned", "daemon process lock is poisoned")
                .retryable()
        })?;
        let should_restart = match slot.as_ref() {
            Some(process) => {
                if process.is_broken() {
                    process.kill();
                    *slot = None;
                    true
                } else {
                    match process.try_wait() {
                        Ok(Some(status)) => {
                            let stderr_tail = process.stderr_tail();
                            *slot = None;
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
                            *slot = None;
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
            *slot = Some(Arc::new(DaemonProcess::spawn(
                self.resource_dir.as_deref(),
            )?));
        }

        slot.as_ref().cloned().ok_or_else(|| {
            SupervisorError::new("daemon_unavailable", "Python daemon is unavailable")
        })
    }
}

impl DaemonProcess {
    fn spawn(resource_dir: Option<&Path>) -> Result<Self, SupervisorError> {
        Self::spawn_command(kassiber_command(resource_dir, vec!["daemon".into()]))
    }

    fn spawn_command(command: DaemonCommand) -> Result<Self, SupervisorError> {
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
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (ready_tx, ready_rx) = mpsc::channel();
        spawn_stdout_reader(stdout, Arc::clone(&pending), Arc::clone(&broken), ready_tx);
        let process = Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            pending,
            stderr_tail,
            broken,
        };
        let ready = match ready_rx.recv_timeout(DAEMON_READY_TIMEOUT) {
            Ok(Ok(ready)) => ready,
            Ok(Err(error)) => {
                process.mark_broken();
                process.kill();
                return Err(error.with_stderr_tail(process.stderr_tail()));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                process.mark_broken();
                process.kill();
                return Err(SupervisorError::new(
                    "daemon_timeout",
                    format!(
                        "Python daemon did not answer within {} seconds",
                        DAEMON_READY_TIMEOUT.as_secs()
                    ),
                )
                .with_stderr_tail(process.stderr_tail())
                .retryable());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                process.mark_broken();
                process.kill();
                return Err(SupervisorError::new(
                    "daemon_exited",
                    "Python daemon stdout reader stopped",
                )
                .with_stderr_tail(process.stderr_tail())
                .retryable());
            }
        };
        if ready.get("kind").and_then(Value::as_str) != Some("daemon.ready") {
            process.mark_broken();
            process.kill();
            return Err(SupervisorError::new(
                "daemon_protocol_error",
                "Python daemon did not emit daemon.ready on startup",
            )
            .details(ready)
            .with_stderr_tail(process.stderr_tail()));
        }

        Ok(process)
    }

    fn register_request(
        &self,
        request_id: String,
        sender: mpsc::Sender<Result<Value, SupervisorError>>,
    ) -> Result<(), SupervisorError> {
        if self.is_broken() {
            return Err(SupervisorError::new(
                "daemon_exited",
                "Python daemon stdout reader stopped",
            )
            .retryable());
        }
        let mut pending = self.pending.lock().map_err(|_| {
            SupervisorError::new(
                "daemon_lock_poisoned",
                "daemon request registry is poisoned",
            )
            .retryable()
        })?;
        if pending.contains_key(&request_id) {
            return Err(SupervisorError::new(
                "daemon_request_id_conflict",
                "daemon request_id is already in flight",
            )
            .details(json!({ "request_id": request_id })));
        }
        pending.insert(request_id, sender);
        Ok(())
    }

    fn unregister_request(&self, request_id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(request_id);
        }
    }

    fn write_json_line(&self, payload: &Value) -> Result<(), SupervisorError> {
        let line = serde_json::to_string(payload).map_err(|error| {
            SupervisorError::new(
                "daemon_protocol_error",
                format!("Could not serialize daemon request: {error}"),
            )
        })?;
        let mut stdin = self.stdin.lock().map_err(|_| {
            SupervisorError::new("daemon_lock_poisoned", "daemon stdin lock is poisoned")
                .retryable()
        })?;
        writeln!(stdin, "{line}").map_err(|error| {
            SupervisorError::new(
                "daemon_write_failed",
                format!("Could not write to Python daemon stdin: {error}"),
            )
            .retryable()
        })?;
        stdin.flush().map_err(|error| {
            SupervisorError::new(
                "daemon_write_failed",
                format!("Could not flush Python daemon stdin: {error}"),
            )
            .retryable()
        })
    }

    fn stderr_tail(&self) -> String {
        self.stderr_tail.text()
    }

    fn try_wait(&self) -> Result<Option<std::process::ExitStatus>, String> {
        let mut child = self
            .child
            .lock()
            .map_err(|_| "daemon child lock is poisoned".to_string())?;
        child.try_wait().map_err(|error| error.to_string())
    }

    fn is_broken(&self) -> bool {
        self.broken.load(Ordering::SeqCst)
    }

    fn mark_broken(&self) {
        self.broken.store(true, Ordering::SeqCst);
    }

    fn kill(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
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
    pending: Arc<Mutex<HashMap<String, mpsc::Sender<Result<Value, SupervisorError>>>>>,
    broken: Arc<AtomicBool>,
    ready_tx: mpsc::Sender<Result<Value, SupervisorError>>,
) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut startup = Some(ready_tx);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    fail_stdout_reader(
                        &broken,
                        &pending,
                        &mut startup,
                        SupervisorError::new("daemon_exited", "Python daemon closed stdout")
                            .retryable(),
                    );
                    return;
                }
                Ok(_) => match serde_json::from_str::<Value>(line.trim()) {
                    Ok(response) => {
                        if response.get("kind").and_then(Value::as_str) == Some("daemon.ready")
                            && response.get("request_id").is_none()
                        {
                            if let Some(tx) = startup.take() {
                                let _ = tx.send(Ok(response));
                                continue;
                            }
                        }

                        if startup.is_some() {
                            fail_stdout_reader(
                                &broken,
                                &pending,
                                &mut startup,
                                SupervisorError::new(
                                    "daemon_protocol_error",
                                    "Python daemon emitted a request response before daemon.ready",
                                )
                                .details(response),
                            );
                            return;
                        }

                        let Some(request_id) = response
                            .get("request_id")
                            .and_then(request_id_value_key)
                            .map(str::to_string)
                        else {
                            fail_stdout_reader(
                                &broken,
                                &pending,
                                &mut startup,
                                SupervisorError::new(
                                    "daemon_protocol_error",
                                    "Python daemon emitted a response without request_id",
                                )
                                .details(response),
                            );
                            return;
                        };

                        let sender = match pending.lock() {
                            Ok(pending) => pending.get(&request_id).cloned(),
                            Err(_) => {
                                fail_stdout_reader(
                                    &broken,
                                    &pending,
                                    &mut startup,
                                    SupervisorError::new(
                                        "daemon_lock_poisoned",
                                        "daemon request registry is poisoned",
                                    )
                                    .retryable(),
                                );
                                return;
                            }
                        };

                        if let Some(sender) = sender {
                            if sender.send(Ok(response)).is_err() {
                                if let Ok(mut pending) = pending.lock() {
                                    pending.remove(&request_id);
                                }
                            }
                            continue;
                        }

                        fail_stdout_reader(
                            &broken,
                            &pending,
                            &mut startup,
                            SupervisorError::new(
                                "daemon_request_id_mismatch",
                                "Python daemon emitted a response for an unknown request_id",
                            )
                            .details(json!({ "request_id": request_id })),
                        );
                        return;
                    }
                    Err(error) => {
                        fail_stdout_reader(
                            &broken,
                            &pending,
                            &mut startup,
                            SupervisorError::new(
                                "daemon_protocol_error",
                                format!("Python daemon emitted invalid JSON: {error}"),
                            )
                            .details(json!({ "line": line.trim() })),
                        );
                        return;
                    }
                },
                Err(error) => {
                    fail_stdout_reader(
                        &broken,
                        &pending,
                        &mut startup,
                        SupervisorError::new(
                            "daemon_read_failed",
                            format!("Could not read from Python daemon stdout: {error}"),
                        )
                        .retryable(),
                    );
                    return;
                }
            }
        }
    });
}

fn fail_stdout_reader(
    broken: &Arc<AtomicBool>,
    pending: &Arc<Mutex<HashMap<String, mpsc::Sender<Result<Value, SupervisorError>>>>>,
    startup: &mut Option<mpsc::Sender<Result<Value, SupervisorError>>>,
    error: SupervisorError,
) {
    broken.store(true, Ordering::SeqCst);
    if let Some(tx) = startup.take() {
        let _ = tx.send(Err(error.clone()));
    }
    broadcast_pending_error(pending, error);
}

fn broadcast_pending_error(
    pending: &Arc<Mutex<HashMap<String, mpsc::Sender<Result<Value, SupervisorError>>>>>,
    error: SupervisorError,
) {
    let senders = match pending.lock() {
        Ok(mut pending) => pending
            .drain()
            .map(|(_, sender)| sender)
            .collect::<Vec<_>>(),
        Err(_) => return,
    };
    for sender in senders {
        let _ = sender.send(Err(error.clone()));
    }
}

fn request_id_value_key(value: &Value) -> Option<&str> {
    value.as_str()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    fn write_stub_daemon() -> (PathBuf, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "kassiber-supervisor-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        let script = dir.join("stub-daemon.py");
        fs::write(
            &script,
            r#"#!/usr/bin/env python3
import json
import sys
import threading
import time

write_lock = threading.Lock()

def emit(payload):
    with write_lock:
        sys.stdout.write(json.dumps(payload, separators=(",", ":")) + "\n")
        sys.stdout.flush()

emit({"kind":"daemon.ready","schema_version":1,"data":{"version":"test","supported_kinds":["slow","fast","daemon.shutdown"]}})

def slow(request_id):
    emit({"kind":"slow.delta","schema_version":1,"request_id":request_id,"data":{"delta":{"content":"a"}}})
    time.sleep(0.35)
    emit({"kind":"slow","schema_version":1,"request_id":request_id,"data":{"finish_reason":"stop"}})

for line in sys.stdin:
    if not line.strip():
        continue
    request = json.loads(line)
    request_id = request.get("request_id")
    kind = request.get("kind")
    if kind == "slow":
        threading.Thread(target=slow, args=(request_id,), daemon=True).start()
    elif kind == "fast":
        emit({"kind":"fast","schema_version":1,"request_id":request_id,"data":{"ok":True}})
    elif kind == "daemon.shutdown":
        emit({"kind":"daemon.shutdown","schema_version":1,"request_id":request_id,"data":{}})
        break
"#,
        )
        .expect("write stub daemon");
        let mut permissions = fs::metadata(&script).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).expect("chmod stub daemon");
        (dir, script)
    }

    #[test]
    #[cfg(unix)]
    fn demuxes_fast_request_while_streaming_request_is_active() {
        let (dir, script) = write_stub_daemon();
        let process = DaemonProcess::spawn_command(DaemonCommand {
            program: script,
            args: Vec::new(),
            cwd: dir.clone(),
            source: "env_python",
        })
        .expect("spawn stub daemon");
        let supervisor = Arc::new(DaemonSupervisor::new_with_process(process));
        let (delta_tx, delta_rx) = mpsc::channel();

        let slow_supervisor = Arc::clone(&supervisor);
        let slow = thread::spawn(move || {
            slow_supervisor.invoke_inner("slow", None, true, Some(json!("slow-1")), |record| {
                let _ = delta_tx.send(record.clone());
            })
        });

        let delta = delta_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("stream delta");
        assert_eq!(
            delta.get("kind").and_then(Value::as_str),
            Some("slow.delta")
        );

        let started = Instant::now();
        let fast = supervisor
            .invoke_inner("fast", None, false, Some(json!("fast-1")), |_| {})
            .expect("fast response");
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "fast request waited for the slow stream to finish"
        );
        assert_eq!(fast.get("kind").and_then(Value::as_str), Some("fast"));
        assert_eq!(
            fast.get("request_id").and_then(Value::as_str),
            Some("fast-1")
        );

        let slow_terminal = slow.join().expect("slow join").expect("slow response");
        assert_eq!(
            slow_terminal.get("kind").and_then(Value::as_str),
            Some("slow")
        );

        let _ = supervisor.invoke_inner(
            "daemon.shutdown",
            None,
            false,
            Some(json!("shutdown-1")),
            |_| {},
        );
        let _ = fs::remove_dir_all(dir);
    }
}

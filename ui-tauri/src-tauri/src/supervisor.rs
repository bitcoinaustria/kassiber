use serde_json::{json, Value};
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub struct DaemonSupervisor {
    process: Option<DaemonProcess>,
    next_request_id: u64,
}

struct DaemonProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
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
}

impl DaemonSupervisor {
    pub fn new() -> Self {
        Self {
            process: None,
            next_request_id: 1,
        }
    }

    pub fn invoke(&mut self, kind: &str, args: Option<Value>) -> Result<Value, SupervisorError> {
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

        loop {
            let response = process.read_json_line()?;
            if response.get("request_id").and_then(Value::as_str) != Some(request_id.as_str()) {
                continue;
            }
            if response.get("kind").and_then(Value::as_str) == Some("progress") {
                continue;
            }
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
            Some(process) => match process.child.try_wait() {
                Ok(Some(status)) => {
                    self.process = None;
                    return Err(SupervisorError::new(
                        "daemon_exited",
                        format!("Python daemon exited before handling the request: {status}"),
                    )
                    .hint("Restart the Tauri shell and check the daemon smoke test output.")
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
            },
            None => true,
        };

        if should_restart {
            self.process = Some(DaemonProcess::spawn()?);
        }

        self.process.as_mut().ok_or_else(|| {
            SupervisorError::new("daemon_unavailable", "Python daemon is unavailable")
        })
    }
}

impl DaemonProcess {
    fn spawn() -> Result<Self, SupervisorError> {
        let repo_root = repo_root();
        let python = env::var("KASSIBER_DAEMON_PYTHON")
            .unwrap_or_else(|_| default_python(&repo_root).to_string_lossy().to_string());
        let mut child = Command::new(&python)
            .arg("-m")
            .arg("kassiber")
            .arg("daemon")
            .current_dir(&repo_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                SupervisorError::new(
                    "daemon_spawn_failed",
                    format!("Could not start Python daemon with {python:?}: {error}"),
                )
                .hint("Set KASSIBER_DAEMON_PYTHON to a Python with Kassiber importable.")
                .details(json!({ "cwd": repo_root }))
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

        let mut process = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        };
        let ready = process.read_json_line()?;
        if ready.get("kind").and_then(Value::as_str) != Some("daemon.ready") {
            return Err(SupervisorError::new(
                "daemon_protocol_error",
                "Python daemon did not emit daemon.ready on startup",
            )
            .details(ready));
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

    fn read_json_line(&mut self) -> Result<Value, SupervisorError> {
        let mut line = String::new();
        let bytes = self.stdout.read_line(&mut line).map_err(|error| {
            SupervisorError::new(
                "daemon_read_failed",
                format!("Could not read from Python daemon stdout: {error}"),
            )
            .retryable()
        })?;
        if bytes == 0 {
            return Err(
                SupervisorError::new("daemon_exited", "Python daemon closed stdout").retryable(),
            );
        }
        serde_json::from_str(line.trim()).map_err(|error| {
            SupervisorError::new(
                "daemon_protocol_error",
                format!("Python daemon emitted invalid JSON: {error}"),
            )
            .details(json!({ "line": line.trim() }))
        })
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        let _ = self.write_json_line(&json!({
            "request_id": "tauri-drop",
            "kind": "daemon.shutdown"
        }));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn repo_root() -> PathBuf {
    if let Ok(path) = env::var("KASSIBER_REPO_ROOT") {
        return PathBuf::from(path);
    }

    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .map(PathBuf::from)
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

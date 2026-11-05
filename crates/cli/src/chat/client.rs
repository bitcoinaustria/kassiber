//! `_DaemonChatClient` (`kassiber/cli/chat.py:82`): `kassiber chat` runs
//! the same daemon tool loop, consent and cancel protocol as the desktop by
//! re-executing this binary as `kassiber --data-root <root> --env-file
//! <file> daemon`. The database passphrase never reaches the child's argv
//! or environment: it goes over stdin as one `daemon.unlock` request that
//! the transcript does not record.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use kassiber_envelope::{dict, get, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_runtime::store::Secret;

/// A JSON string from any text.
pub(crate) fn s(text: impl AsRef<str>) -> Value {
    kassiber_envelope::s(text.as_ref())
}

const STDERR_TAIL_CHARS: usize = 2000;

/// A fresh `uuid.uuid4().hex`-shaped request id suffix.
pub fn random_hex() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut text = String::new();
    for round in 0..2u8 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u8(round);
        hasher.write_u32(std::process::id());
        text.push_str(&format!("{:016x}", hasher.finish()));
    }
    text
}

/// `json.dumps(payload, separators=(",", ":"))`.
pub fn compact(value: &Value) -> String {
    dumps(value, &Options::new().compact()).unwrap_or_default()
}

pub struct DaemonClient {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Option<String>>,
    stderr_tail: Arc<Mutex<String>>,
    threads: Vec<JoinHandle<()>>,
    transcript: Option<std::fs::File>,
    timeout: Duration,
}

/// What the child is started with.
pub struct Launch<'a> {
    pub data_root: &'a str,
    pub env_file: Option<&'a str>,
    /// The unlocked book's passphrase, sent once as `daemon.unlock`.
    pub passphrase: Option<Secret>,
    pub timeout: Duration,
    pub transcript: Option<std::fs::File>,
}

fn drain_stdout(stream: impl Read, lines: std::sync::mpsc::Sender<Option<String>>) {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if lines
                    .send(Some(String::from_utf8_lossy(&line).into_owned()))
                    .is_err()
                {
                    return;
                }
            }
        }
    }
    let _ = lines.send(None);
}

fn drain_stderr(mut stream: impl Read, tail: Arc<Mutex<String>>) {
    let mut buffer = [0u8; 4096];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                let chunk = String::from_utf8_lossy(&buffer[..read]);
                let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                tail.push_str(&chunk);
                let length = tail.chars().count();
                if length > STDERR_TAIL_CHARS {
                    *tail = tail.chars().skip(length - STDERR_TAIL_CHARS).collect();
                }
            }
        }
    }
}

impl DaemonClient {
    /// Starts the child, waits for `daemon.ready`, and unlocks it when a
    /// passphrase was resolved.
    pub fn start(launch: Launch<'_>) -> Result<Self, AppError> {
        let program = std::env::current_exe()
            .map_err(|error| AppError::new("daemon_start_failed", error.to_string()))?;
        let mut command = Command::new(program);
        command.arg("--data-root").arg(launch.data_root);
        if let Some(env_file) = launch.env_file {
            command.arg("--env-file").arg(env_file);
        }
        command
            .arg("daemon")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| AppError::new("daemon_start_failed", error.to_string()))?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (sender, lines) = channel();
        let tail = Arc::new(Mutex::new(String::new()));
        let stderr_tail = Arc::clone(&tail);
        let threads = vec![
            std::thread::Builder::new()
                .name("kassiber-daemon-stderr-drain".into())
                .spawn(move || drain_stderr(stderr, stderr_tail))
                .map_err(|error| AppError::new("daemon_start_failed", error.to_string()))?,
            std::thread::Builder::new()
                .name("kassiber-daemon-stdout-reader".into())
                .spawn(move || drain_stdout(stdout, sender))
                .map_err(|error| AppError::new("daemon_start_failed", error.to_string()))?,
        ];
        let mut client = DaemonClient {
            stdin: child.stdin.take(),
            child,
            lines,
            stderr_tail: tail,
            threads,
            transcript: launch.transcript,
            timeout: launch.timeout,
        };
        let ready = match client.read(true, None) {
            Ok(ready) => ready,
            Err(error) => {
                client.close();
                return Err(error);
            }
        };
        if get(&ready, "kind") != Some(&s("daemon.ready")) {
            client.close();
            return Err(
                AppError::new("daemon_start_failed", "daemon did not start cleanly")
                    .with_details(dict([("envelope", ready)])),
            );
        }
        if let Some(passphrase) = launch.passphrase {
            let request_id = format!("cli-bootstrap-{}", uuid_dashed());
            let unlock = dict([
                ("kind", s("daemon.unlock")),
                ("request_id", s(&request_id)),
                (
                    "args",
                    dict([
                        ("require_existing_project", Value::Bool(true)),
                        (
                            "auth_response",
                            dict([("passphrase_secret", s(passphrase.expose()))]),
                        ),
                    ]),
                ),
            ]);
            if let Err(error) = client.send(&unlock, false) {
                client.close();
                return Err(error);
            }
            drop(passphrase);
            let unlocked = match client.read(false, None) {
                Ok(unlocked) => unlocked,
                Err(error) => {
                    client.close();
                    return Err(error);
                }
            };
            let ok = get(&unlocked, "kind") == Some(&s("daemon.unlock"))
                && get(&unlocked, "request_id") == Some(&s(&request_id))
                && get(&unlocked, "data").and_then(|data| get(data, "unlocked"))
                    == Some(&Value::Bool(true));
            if !ok {
                client.close();
                let error = match get(&unlocked, "error") {
                    Some(error @ Value::Dict(_)) => error.clone(),
                    _ => Value::Dict(Vec::new()),
                };
                let text = |key: &str, default: &str| match get(&error, key) {
                    Some(value) => kassiber_envelope::py_str(value),
                    None => default.to_owned(),
                };
                let mut failure = AppError::new(
                    text("code", "daemon_unlock_failed"),
                    text("message", "daemon database unlock failed"),
                )
                .retryable(get(&error, "retryable").is_some_and(kassiber_envelope::truthy));
                if let Some(hint) = get(&error, "hint").filter(|h| **h != Value::Null) {
                    failure = failure.with_hint(kassiber_envelope::py_str(hint));
                }
                return Err(failure);
            }
        }
        Ok(client)
    }

    fn record(&mut self, value: &Value) {
        if let Some(transcript) = self.transcript.as_mut() {
            let _ = writeln!(transcript, "{}", compact(value));
            let _ = transcript.flush();
        }
    }

    /// `send(payload, record=...)`; the transcript copy leaves out
    /// `timeout_seconds`, which is local transport control.
    pub fn send(&mut self, payload: &Value, record: bool) -> Result<(), AppError> {
        let closed = || AppError::new("daemon_closed", "daemon input stream is closed");
        if self.stdin.is_none() {
            return Err(closed());
        }
        let line = compact(payload) + "\n";
        if record && self.transcript.is_some() {
            let mut copy = payload.clone();
            if let Some(args @ Value::Dict(_)) = get(&copy, "args").cloned() {
                if get(&args, "timeout_seconds").is_some() {
                    let trimmed = crate::chat::without(&args, "timeout_seconds");
                    kassiber_envelope::set(&mut copy, "args", trimmed);
                }
            }
            self.record(&copy);
        }
        let stdin = self.stdin.as_mut().ok_or_else(closed)?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.flush())
            .map_err(|_| closed())
    }

    fn stderr_snapshot(&self) -> String {
        self.stderr_tail
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `read(record=..., timeout=...)`: one record, waiting at most the
    /// chat timeout.
    pub fn read(&mut self, record: bool, timeout: Option<Duration>) -> Result<Value, AppError> {
        let wait = timeout.unwrap_or(self.timeout);
        let line = match self.lines.recv_timeout(wait) {
            Ok(Some(line)) => line,
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                let stderr = self.stderr_snapshot();
                let details = if stderr.is_empty() {
                    dict::<&str>([])
                } else {
                    dict([("stderr", s(stderr))])
                };
                return Err(
                    AppError::new("daemon_closed", "daemon exited before chat completed")
                        .with_details(details),
                );
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(AppError::new("daemon_timeout", "timed out waiting for daemon response")
                    .with_hint("Increase `kassiber chat --timeout <seconds>` if the selected local model or harness needs longer.")
                    .with_details(dict([("timeout_seconds", Value::Float(wait.as_secs_f64()))]))
                    .retryable(true));
            }
        };
        let payload = kassiber_pytext::loads(&line).map_err(|error| {
            AppError::new("daemon_protocol_error", "daemon emitted invalid JSON").with_details(
                dict([
                    ("line", s(line.chars().take(1000).collect::<String>())),
                    ("error", s(error.to_string())),
                ]),
            )
        })?;
        if !matches!(payload, Value::Dict(_)) {
            return Err(AppError::new(
                "daemon_protocol_error",
                "daemon emitted a non-object JSON record",
            ));
        }
        if record {
            self.record(&payload);
        }
        Ok(payload)
    }

    /// `close()`: end of input, then TERM after 5 s and KILL after 2 more.
    pub fn close(&mut self) {
        self.stdin = None;
        let waited = wait_for(&mut self.child, Duration::from_secs(5));
        if !waited {
            terminate(&mut self.child);
            if !wait_for(&mut self.child, Duration::from_secs(2)) {
                let _ = self.child.kill();
                let _ = wait_for(&mut self.child, Duration::from_secs(2));
            }
        }
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn wait_for(child: &mut Child, limit: Duration) -> bool {
    let deadline = std::time::Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => return false,
        }
    }
}

/// `Popen.terminate()`. Without a signal API in this crate, a child that
/// outlived its stdin is killed outright.
fn terminate(child: &mut Child) {
    let _ = child.kill();
}

/// `str(uuid.uuid4())`-shaped, for the bootstrap request id.
fn uuid_dashed() -> String {
    let hex = random_hex();
    format!(
        "{}-{}-4{}-a{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

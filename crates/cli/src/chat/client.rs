//! `_DaemonChatClient` (`kassiber/cli/chat.py:82`): `kassiber chat` runs
//! the same daemon tool loop, consent and cancel protocol as the desktop by
//! re-executing this binary as `kassiber --data-root <root> --env-file
//! <file> daemon`. The database passphrase never reaches the child's argv
//! or environment: it goes over stdin as one `daemon.unlock` request that
//! the transcript does not record.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
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
/// How often a wait for the daemon looks for Ctrl-C.
const INTERRUPT_SLICE: Duration = Duration::from_millis(50);

enum ReceiveError {
    Channel(RecvTimeoutError),
    Interrupted,
}

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

/// Whether Ctrl-C arrived since it was last asked; asking handles it.
pub type Interrupted = Arc<dyn Fn() -> bool + Send + Sync>;

pub struct DaemonClient {
    child: Option<Child>,
    stdin: Option<Box<dyn Write + Send>>,
    lines: Receiver<Option<String>>,
    stderr_tail: Arc<Mutex<String>>,
    threads: Vec<JoinHandle<()>>,
    transcript: Option<std::fs::File>,
    timeout: Duration,
    interrupt: Option<Interrupted>,
}

/// What the child is started with.
pub struct Launch<'a> {
    pub data_root: &'a str,
    pub env_file: Option<&'a str>,
    /// The unlocked book's passphrase, sent once as `daemon.unlock`.
    pub passphrase: Option<Secret>,
    pub timeout: Duration,
    pub transcript: Option<std::fs::File>,
    /// Ctrl-C ends a wait for the daemon with [`crate::chat::INTERRUPTED`].
    pub interrupt: Option<Interrupted>,
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
        away_from_terminal_interrupts(&mut command);
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
            stdin: child
                .stdin
                .take()
                .map(|stdin| Box::new(stdin) as Box<dyn Write + Send>),
            child: Some(child),
            lines,
            stderr_tail: tail,
            threads,
            transcript: launch.transcript,
            timeout: launch.timeout,
            interrupt: launch.interrupt,
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
        let line = match self.receive(wait) {
            Ok(Some(line)) => line,
            Err(ReceiveError::Interrupted) => return Err(crate::chat::interrupted()),
            Ok(None) | Err(ReceiveError::Channel(RecvTimeoutError::Disconnected)) => {
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
            Err(ReceiveError::Channel(RecvTimeoutError::Timeout)) => {
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

    /// The next line within `wait`; with Ctrl-C installed, the wait is
    /// sliced so a Ctrl-C ends it as `KeyboardInterrupt` would.
    fn receive(&mut self, wait: Duration) -> Result<Option<String>, ReceiveError> {
        let Some(interrupt) = self.interrupt.clone() else {
            return self.lines.recv_timeout(wait).map_err(ReceiveError::Channel);
        };
        let deadline = std::time::Instant::now() + wait;
        loop {
            if interrupt() {
                return Err(ReceiveError::Interrupted);
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.lines.recv_timeout(left.min(INTERRUPT_SLICE)) {
                Err(RecvTimeoutError::Timeout) if left > INTERRUPT_SLICE => {}
                other => return other.map_err(ReceiveError::Channel),
            }
        }
    }

    /// `close()`: end of input, then TERM after 5 s and KILL after 2 more.
    pub fn close(&mut self) {
        self.stdin = None;
        if let Some(child) = self.child.as_mut() {
            let waited = wait_for(child, Duration::from_secs(5));
            if !waited {
                terminate(child);
                if !wait_for(child, Duration::from_secs(2)) {
                    let _ = child.kill();
                    let _ = wait_for(child, Duration::from_secs(2));
                }
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

/// The terminal's Ctrl-C goes to its foreground process group; the daemon
/// child sits in its own so that only the chat sees it and cancels the turn
/// through the protocol (Python's child shares the group and dies:
/// tests/rust_port/repro_chat_ctrl_c_kills_daemon.py).
fn away_from_terminal_interrupts(command: &mut Command) {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(command, 0);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        command,
        0x0000_0200, // CREATE_NEW_PROCESS_GROUP: no console Ctrl-C.
    );
}

#[cfg(test)]
impl DaemonClient {
    /// A client over a scripted daemon: records arrive on `lines`, requests
    /// go to `requests`.
    pub(crate) fn scripted(
        lines: Receiver<Option<String>>,
        requests: Box<dyn Write + Send>,
        timeout: Duration,
        interrupt: Option<Interrupted>,
    ) -> Self {
        DaemonClient {
            child: None,
            stdin: Some(requests),
            lines,
            stderr_tail: Arc::default(),
            threads: Vec::new(),
            transcript: None,
            timeout,
            interrupt,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn the_daemon_child_leads_its_own_process_group() {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("ps -o pgid= -p $$")
            .stdout(Stdio::piped());
        away_from_terminal_interrupts(&mut command);
        let child = command.spawn().unwrap();
        let pid = child.id();
        let output = child.wait_with_output().unwrap();
        let group: u32 = String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(group, pid, "a terminal's Ctrl-C must not reach the child");
    }

    #[test]
    fn ctrl_c_ends_a_wait_for_the_daemon() {
        let (_records, lines) = channel();
        let pressed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&pressed);
        let mut client = DaemonClient::scripted(
            lines,
            Box::new(std::io::sink()),
            Duration::from_secs(20),
            Some(Arc::new(move || {
                flag.swap(false, std::sync::atomic::Ordering::SeqCst)
            })),
        );
        let presser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            pressed.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let started = std::time::Instant::now();
        let error = client.read(true, None).unwrap_err();
        presser.join().unwrap();
        assert_eq!(error.code, crate::chat::INTERRUPTED);
        assert!(started.elapsed() < Duration::from_secs(5));
        // Without Ctrl-C the wait still ends at the chat timeout.
        let (_records, lines) = channel();
        let mut client = DaemonClient::scripted(
            lines,
            Box::new(std::io::sink()),
            Duration::from_millis(200),
            Some(Arc::new(|| false)),
        );
        assert_eq!(client.read(true, None).unwrap_err().code, "daemon_timeout");
    }
}

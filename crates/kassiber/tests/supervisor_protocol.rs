//! The Rust daemon under the desktop supervisor's protocol rules.
//!
//! `Supervisor` below applies the rules of
//! `ui-tauri/src-tauri/src/supervisor.rs` to the real binary over pipes:
//! the first record must be `daemon.ready` without a `request_id`; a
//! record is routed by its string `request_id`; a request resolves on a
//! record of its own kind, `error`, or `auth_required`; other records go to
//! the stream of a streaming request and are a protocol error for any
//! other; `event: true` records carry a kind and no `request_id`; records
//! for an unknown `request_id` are dropped; secret-store control requests
//! are answered on stdin; anything else is a protocol error. Built with
//! `--features test-harness`, which adds the fake store and the daemon's
//! protocol test kinds.

#![cfg(feature = "test-harness")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::{dumps, Options, Value};

type Pending = Arc<Mutex<HashMap<String, Sender<Value>>>>;

/// What the fake desktop secret store answers, and after how long.
#[derive(Clone)]
struct SecretStore {
    secrets: HashMap<(String, String), String>,
    delay: Duration,
    silent: bool,
}

struct Supervisor {
    child: Mutex<Child>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pending: Pending,
    events: Arc<Mutex<Vec<Value>>>,
    dropped: Arc<Mutex<Vec<String>>>,
    protocol_errors: Arc<Mutex<Vec<String>>>,
    /// `request_id` and kind of every routed record, in stdout order.
    arrivals: Arc<Mutex<Vec<(String, String)>>>,
    ready: Value,
}

fn text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Str(text)) => text.as_str().map(str::to_owned),
        _ => None,
    }
}

fn line(value: &Value) -> String {
    let mut text = dumps(value, &Options::new().compact()).unwrap();
    text.push('\n');
    text
}

fn write_line(stdin: &Arc<Mutex<Option<ChildStdin>>>, text: &str) {
    let mut stdin = stdin.lock().unwrap();
    let stdin = stdin.as_mut().expect("stdin is open");
    stdin.write_all(text.as_bytes()).unwrap();
    stdin.flush().unwrap();
}

impl Supervisor {
    fn spawn(fake_store: &str, secret_store: SecretStore) -> Arc<Supervisor> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kassiber"))
            .args(["--data-root", "/fake/kassiber/data", "daemon"])
            .env("KASSIBER_RUST_TEST_FAKE_STORE", fake_store)
            .env_remove("KASSIBER_TEST_NO_EGRESS")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the daemon starts");
        let stdin = Arc::new(Mutex::new(child.stdin.take()));
        let stdout = child.stdout.take().unwrap();
        let pending: Pending = Arc::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(Mutex::new(Vec::new()));
        let protocol_errors = Arc::new(Mutex::new(Vec::new()));
        let arrivals = Arc::new(Mutex::new(Vec::new()));
        let (ready_tx, ready_rx): (Sender<Value>, Receiver<Value>) = mpsc::channel();
        {
            let (stdin, pending, events, dropped, protocol_errors, arrivals) = (
                Arc::clone(&stdin),
                Arc::clone(&pending),
                Arc::clone(&events),
                Arc::clone(&dropped),
                Arc::clone(&protocol_errors),
                Arc::clone(&arrivals),
            );
            std::thread::spawn(move || {
                let mut startup = Some(ready_tx);
                for raw in BufReader::new(stdout).lines() {
                    let Ok(raw) = raw else { return };
                    let Ok(record) = kassiber_pytext::loads(raw.trim()) else {
                        protocol_errors
                            .lock()
                            .unwrap()
                            .push(format!("invalid JSON: {raw}"));
                        return;
                    };
                    let kind = text(get(&record, "kind")).unwrap_or_default();
                    if kind == "daemon.ready" && get(&record, "request_id").is_none() {
                        if let Some(tx) = startup.take() {
                            let _ = tx.send(record);
                            continue;
                        }
                    }
                    if startup.is_some() {
                        protocol_errors
                            .lock()
                            .unwrap()
                            .push(format!("before ready: {raw}"));
                        return;
                    }
                    if kind == "supervisor.ai_secret_store.request" {
                        let store = secret_store.clone();
                        let stdin = Arc::clone(&stdin);
                        std::thread::spawn(move || {
                            std::thread::sleep(store.delay);
                            if store.silent {
                                return;
                            }
                            let data = get(&record, "data").cloned().unwrap_or(Value::Null);
                            let provider = text(get(&data, "provider_name")).unwrap_or_default();
                            let key = (
                                text(get(&data, "service")).unwrap_or_default(),
                                text(get(&data, "account")).unwrap_or_default(),
                            );
                            let answer = match store.secrets.get(&key) {
                                Some(secret) => dict([
                                    ("provider_name", s(provider.as_str())),
                                    ("state", s("ok")),
                                    ("secret", s(secret.as_str())),
                                ]),
                                None => dict([
                                    ("provider_name", s(provider.as_str())),
                                    ("state", s("missing")),
                                    ("secret", Value::Null),
                                ]),
                            };
                            let response = dict([
                                ("kind", s("supervisor.ai_secret_store.response")),
                                ("schema_version", Value::from(1)),
                                ("request_id", get(&record, "request_id").cloned().unwrap()),
                                ("data", answer),
                            ]);
                            write_line(&stdin, &line(&response));
                        });
                        continue;
                    }
                    if get(&record, "event") == Some(&Value::Bool(true)) {
                        if get(&record, "request_id").is_some() || kind.trim().is_empty() {
                            protocol_errors
                                .lock()
                                .unwrap()
                                .push(format!("bad event: {raw}"));
                            return;
                        }
                        events.lock().unwrap().push(record);
                        continue;
                    }
                    let Some(request_id) = text(get(&record, "request_id")) else {
                        protocol_errors
                            .lock()
                            .unwrap()
                            .push(format!("response without request_id: {raw}"));
                        return;
                    };
                    arrivals
                        .lock()
                        .unwrap()
                        .push((request_id.clone(), kind.clone()));
                    let sender = pending.lock().unwrap().get(&request_id).cloned();
                    match sender {
                        Some(sender) => {
                            let _ = sender.send(record);
                        }
                        None => dropped.lock().unwrap().push(request_id),
                    }
                }
            });
        }
        let ready = ready_rx
            .recv_timeout(Duration::from_secs(30))
            .expect("daemon.ready arrives");
        Arc::new(Supervisor {
            child: Mutex::new(child),
            stdin,
            pending,
            events,
            dropped,
            protocol_errors,
            arrivals,
            ready,
        })
    }

    /// Sends a request and waits for its terminal record; streaming
    /// requests also return their intermediate records.
    fn invoke(
        &self,
        request_id: &str,
        kind: &str,
        args: Option<Value>,
        streaming: bool,
    ) -> (Value, Vec<Value>) {
        let (tx, rx) = mpsc::channel();
        self.pending
            .lock()
            .unwrap()
            .insert(request_id.to_owned(), tx);
        let mut request = vec![("request_id", s(request_id)), ("kind", s(kind))];
        if let Some(args) = args {
            request.push(("args", args));
        }
        write_line(&self.stdin, &line(&dict(request)));
        let mut stream = Vec::new();
        loop {
            let record = rx
                .recv_timeout(Duration::from_secs(30))
                .unwrap_or_else(|_| panic!("{kind} answered"));
            let record_kind = text(get(&record, "kind")).unwrap_or_default();
            if record_kind == kind || record_kind == "error" || record_kind == "auth_required" {
                self.pending.lock().unwrap().remove(request_id);
                return (record, stream);
            }
            if streaming {
                stream.push(record);
                continue;
            }
            panic!("non-terminal record for a non-streaming request: {record:?}");
        }
    }

    /// Closes stdin and returns the exit code.
    /// Where the terminal record of `request_id` arrived.
    fn arrived(&self, request_id: &str, kind: &str) -> usize {
        self.arrivals
            .lock()
            .unwrap()
            .iter()
            .position(|(id, k)| id == request_id && k == kind)
            .unwrap_or_else(|| panic!("{request_id} {kind} never arrived"))
    }

    fn finish(&self) -> Option<i32> {
        self.stdin.lock().unwrap().take();
        let status = self.child.lock().unwrap().wait().unwrap();
        assert!(
            self.protocol_errors.lock().unwrap().is_empty(),
            "{:?}",
            self.protocol_errors.lock().unwrap()
        );
        status.code()
    }
}

fn plain() -> String {
    r#"{"project_id":"proj"}"#.to_owned()
}

fn no_secrets() -> SecretStore {
    SecretStore {
        secrets: HashMap::new(),
        delay: Duration::ZERO,
        silent: false,
    }
}

fn kind_of(record: &Value) -> String {
    text(get(record, "kind")).unwrap_or_default()
}

#[test]
fn ready_handshake_lists_the_rust_kinds() {
    let daemon = Supervisor::spawn(&plain(), no_secrets());
    assert_eq!(kind_of(&daemon.ready), "daemon.ready");
    let data = get(&daemon.ready, "data").unwrap();
    assert_eq!(
        text(get(data, "version")).as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    let Some(Value::List(kinds)) = get(data, "supported_kinds") else {
        panic!("supported_kinds");
    };
    for kind in [
        "status",
        "daemon.lock",
        "daemon.unlock",
        "daemon.shutdown",
        "test.stream",
    ] {
        assert!(kinds.contains(&s(kind)), "{kind}");
    }
    assert_eq!(daemon.finish(), Some(0));
}

#[test]
fn fast_requests_are_answered_while_a_stream_runs() {
    let daemon = Supervisor::spawn(&plain(), no_secrets());
    let streaming = {
        let daemon = Arc::clone(&daemon);
        std::thread::spawn(move || {
            daemon.invoke(
                "stream-1",
                "test.stream",
                Some(dict([
                    ("count", Value::from(3)),
                    ("delay_ms", Value::from(150)),
                ])),
                true,
            )
        })
    };
    std::thread::sleep(Duration::from_millis(50));
    let (status, _) = daemon.invoke("fast-1", "status", None, false);
    assert_eq!(kind_of(&status), "status");
    let (done, deltas) = streaming.join().unwrap();
    assert!(daemon.arrived("fast-1", "status") < daemon.arrived("stream-1", "test.stream"));
    assert_eq!(kind_of(&done), "test.stream");
    assert_eq!(deltas.len(), 3);
    assert!(deltas.iter().all(|d| kind_of(d) == "test.stream.delta"
        && text(get(d, "request_id")).as_deref() == Some("stream-1")));
    assert_eq!(daemon.finish(), Some(0));
}

#[test]
fn late_records_for_unknown_requests_are_dropped() {
    let daemon = Supervisor::spawn(&plain(), no_secrets());
    let (late, _) = daemon.invoke("late-1", "test.late", None, false);
    assert_eq!(kind_of(&late), "test.late");
    let (status, _) = daemon.invoke("after-1", "status", None, false);
    assert_eq!(kind_of(&status), "status");
    assert_eq!(*daemon.dropped.lock().unwrap(), vec!["ghost-1".to_owned()]);
    assert_eq!(daemon.finish(), Some(0));
}

#[test]
fn events_go_to_the_event_sink() {
    let daemon = Supervisor::spawn(&plain(), no_secrets());
    let (done, _) = daemon.invoke("event-1", "test.emit_event", None, false);
    assert_eq!(kind_of(&done), "test.emit_event");
    let events = daemon.events.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(kind_of(&events[0]), "test.event");
    assert!(get(&events[0], "request_id").is_none());
    assert_eq!(daemon.finish(), Some(0));
}

fn secret_args(timeout_ms: u64) -> Value {
    dict([
        ("op", s("get")),
        ("provider_name", s("remote")),
        ("store_id", s("linux_secret_service")),
        (
            "service",
            s("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        ),
        ("account", s("remote")),
        ("timeout_ms", Value::from(timeout_ms)),
    ])
}

#[test]
fn the_secret_store_round_trip_defers_other_requests() {
    let mut secrets = HashMap::new();
    secrets.insert(
        (
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
            "remote".to_owned(),
        ),
        "sk-test-secret".to_owned(),
    );
    let daemon = Supervisor::spawn(
        &plain(),
        SecretStore {
            secrets,
            delay: Duration::from_millis(300),
            silent: false,
        },
    );
    let bridged = {
        let daemon = Arc::clone(&daemon);
        std::thread::spawn(move || {
            daemon.invoke(
                "secret-1",
                "test.secret_store",
                Some(secret_args(15_000)),
                false,
            )
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    // Sent while the daemon waits for the desktop; answered after it.
    let (status, _) = daemon.invoke("during-1", "status", None, false);
    assert_eq!(kind_of(&status), "status");
    let (secret, _) = bridged.join().unwrap();
    assert!(daemon.arrived("secret-1", "test.secret_store") < daemon.arrived("during-1", "status"));
    assert_eq!(kind_of(&secret), "test.secret_store");
    let data = get(&secret, "data").unwrap();
    assert_eq!(text(get(data, "state")).as_deref(), Some("ok"));
    assert_eq!(text(get(data, "secret")).as_deref(), Some("sk-test-secret"));
    assert_eq!(daemon.finish(), Some(0));
}

#[test]
fn an_unanswered_secret_store_request_times_out() {
    let daemon = Supervisor::spawn(
        &plain(),
        SecretStore {
            secrets: HashMap::new(),
            delay: Duration::ZERO,
            silent: true,
        },
    );
    let (error, _) = daemon.invoke(
        "secret-2",
        "test.secret_store",
        Some(secret_args(300)),
        false,
    );
    assert_eq!(kind_of(&error), "error");
    let body = get(&error, "error").unwrap();
    assert_eq!(
        text(get(body, "code")).as_deref(),
        Some("secret_ref_unavailable")
    );
    assert_eq!(
        text(get(body, "message")).as_deref(),
        Some("desktop secret store did not answer")
    );
    assert_eq!(get(body, "retryable"), Some(&Value::Bool(true)));
    assert_eq!(daemon.finish(), Some(0));
}

#[test]
fn auth_required_is_terminal_for_a_locked_book() {
    let events_file =
        std::env::temp_dir().join(format!("kassiber-supervisor-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&events_file);
    let config = format!(
        r#"{{"encrypted":true,"passphrase":"correct horse","project_id":"proj","events_file":{}}}"#,
        line(&s(events_file.to_str().unwrap())).trim()
    );
    let daemon = Supervisor::spawn(&config, no_secrets());
    let (locked, _) = daemon.invoke("s-1", "status", None, false);
    assert_eq!(kind_of(&locked), "auth_required");
    assert_eq!(
        text(get(get(&locked, "data").unwrap(), "scope")).as_deref(),
        Some("unlock_database")
    );
    let auth = dict([(
        "auth_response",
        dict([("passphrase_secret", s("correct horse"))]),
    )]);
    let (unlocked, _) = daemon.invoke("u-1", "daemon.unlock", Some(auth), false);
    assert_eq!(kind_of(&unlocked), "daemon.unlock");
    let (status, _) = daemon.invoke("s-2", "status", None, false);
    assert_eq!(kind_of(&status), "status");
    let (lock, _) = daemon.invoke("l-1", "daemon.lock", None, false);
    assert_eq!(kind_of(&lock), "daemon.lock");
    let (relocked, _) = daemon.invoke("s-3", "status", None, false);
    assert_eq!(kind_of(&relocked), "auth_required");
    let (bye, _) = daemon.invoke("q-1", "daemon.shutdown", None, false);
    assert_eq!(kind_of(&bye), "daemon.shutdown");
    assert_eq!(daemon.finish(), Some(0));
    let log = std::fs::read_to_string(&events_file).unwrap();
    let _ = std::fs::remove_file(&events_file);
    let lines: Vec<&str> = log.lines().collect();
    // Ownership is taken before the book opens and released after it
    // closes; a locked book's lazy open takes it and gives it back.
    assert_eq!(
        lines,
        [
            "bootstrap /fake/kassiber/data",
            "acquire proj Desktop",
            "release proj",
            "verify unlock_database true",
            "acquire proj Desktop",
            "open /fake/kassiber/data",
            "opened proj select=false",
            "close",
            "release proj",
            "acquire proj Desktop",
            "release proj",
        ]
    );
}

#[test]
fn end_of_input_shuts_down_cleanly() {
    let daemon = Supervisor::spawn(&plain(), no_secrets());
    let (status, _) = daemon.invoke("s-1", "status", None, false);
    assert_eq!(kind_of(&status), "status");
    assert_eq!(daemon.finish(), Some(0));
}

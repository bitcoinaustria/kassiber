//! A daemon under test, and recorded daemon sessions replayed against it.
//!
//! The pytest freshness tests drove `kassiber --data-root R daemon` over
//! stdin and stdout while a loopback chain served it. Their sessions were
//! recorded once, where Rust and the Python reference agreed: the book
//! under R, the test's environment, the network tape (`kassiber-net`'s
//! test tape), each line sent with how many rows had been read before it,
//! every row read and the book afterwards. `replay` restores the book,
//! starts the daemon on the tape, sends each line at the same point and
//! requires the same rows, per request in order (background events in
//! their own order), and the same book.
//!
//! `KASSIBER_GOLDEN_WRITE=1` (refused under `CI`) rewrites the recorded
//! rows a replay compares with the rows the daemon produced, after a
//! deliberate change; everything else in the recording stays as it was.
//! Review the diff and publish it (plan 22, "Generators become frozen
//! goldens").

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::golden::{
    self, assert_same_book, entries, fixed_elapsed_ms, restore, snapshot, string, value, Golden,
    Scratch,
};
use kassiber_envelope::{get, py_str};
use kassiber_ipc::unix::UnixStream;
use kassiber_pyjson::{Key, Options, Value};

pub const BIN: &str = env!("CARGO_BIN_EXE_kassiber");

/// A response the recording's loopback chain held while the test acted:
/// the first request containing `pattern` waits for the release.
pub const ESPLORA_FETCH: &str = "/scripthash/";
pub const ELECTRUM_FETCH: &str = "blockchain.scripthash.get_history";

pub fn text(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &Options::new().sort_keys(true)).unwrap()
}

fn line(value: &Value) -> String {
    without_rust_only_kinds(fixed_elapsed_ms(&text(value)))
}

/// Python never advertised the daemon's Rust-only kinds, so they leave the
/// `daemon.ready` row before it is compared; every other kind still is.
fn without_rust_only_kinds(line: String) -> String {
    if !line.contains("\"daemon.ready\"") {
        return line;
    }
    kassiber_daemon::RUST_ONLY_KINDS
        .iter()
        .fold(line, |line, kind| {
            line.replace(&format!(", \"{kind}\""), "")
                .replace(&format!("\"{kind}\", "), "")
        })
}

/// The daemon's environment: the recorded one, with every path placed in
/// `scratch` and never the account's app-data root or its offline switch.
pub fn environment(scratch: &Scratch, recorded: &Value) -> Vec<(String, String)> {
    let dir = &scratch.0;
    let mut env: Vec<(String, String)> = vec![
        ("TMPDIR".into(), scratch.path()),
        ("USERPROFILE".into(), scratch.path()),
        ("LOCALAPPDATA".into(), scratch.path()),
        (
            "XDG_DATA_HOME".into(),
            dir.join("xdg-data").display().to_string(),
        ),
        (
            "XDG_CONFIG_HOME".into(),
            dir.join("xdg-config").display().to_string(),
        ),
        (
            "XDG_STATE_HOME".into(),
            dir.join("xdg-state").display().to_string(),
        ),
        ("COLUMNS".into(), "80".into()),
        ("LINES".into(), "24".into()),
        ("KASSIBER_DISABLE_UPDATE_CHECK".into(), "1".into()),
        (
            "KASSIBER_OFFLINE_PREFERENCE_FILE".into(),
            dir.join("offline.json").display().to_string(),
        ),
        (
            "KASSIBER_AGENT_ACCESS_FILE".into(),
            dir.join("agents.json").display().to_string(),
        ),
        (
            "KASSIBER_UPDATE_PREFERENCE_FILE".into(),
            dir.join("updates.json").display().to_string(),
        ),
        (
            "KASSIBER_UPDATE_CACHE_FILE".into(),
            dir.join("update-cache.json").display().to_string(),
        ),
    ];
    for (key, item) in entries(recorded) {
        let item = scratch.place(&py_str(item)).replace("@BIN@", BIN);
        env.push((key, item));
    }
    env
}

pub struct Daemon {
    pub child: Child,
    stdin: Option<ChildStdin>,
    /// Our end of the daemon's stdout: a socket, so a test can close it
    /// under the reader as a test closed the pipe it read.
    output: UnixStream,
    lines: Receiver<String>,
    pub rows: Vec<Value>,
}

impl Daemon {
    /// Start the daemon on `root` with `env`, reading `tape`.
    pub fn start(root: &Path, env: &[(String, String)], tape: &Path, hold: Option<&Path>) -> Self {
        let (output, theirs) = UnixStream::pair().unwrap();
        let mut command = Command::new(BIN);
        command
            .arg("--data-root")
            .arg(root)
            .arg("daemon")
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", std::env::var_os("HOME").unwrap_or_default())
            .env("LANG", "C.UTF-8")
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .env(
                "KASSIBER_TEST_NET_TAPE",
                format!("replay:{}", tape.display()),
            )
            .env(
                "KASSIBER_TEST_NET_TAPE_WRITES",
                tape.with_extension("writes"),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::from(std::os::fd::OwnedFd::from(theirs)))
            .stderr(Stdio::piped());
        if let Some(hold) = hold {
            command.env("KASSIBER_TEST_NET_TAPE_HOLD", hold);
        }
        let mut child = command.spawn().expect("start the daemon");
        // The child holds its own end now.
        drop(command);
        let stdin = child.stdin.take();
        let stdout = output.try_clone().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut daemon = Self {
            child,
            stdin,
            output,
            lines,
            rows: Vec::new(),
        };
        let ready = daemon.read();
        assert_eq!(field(&ready, "kind"), "daemon.ready", "{ready:?}");
        daemon
    }

    /// Stop reading: the daemon's next write fails, as when a test closed
    /// the read end of its stdout.
    pub fn close_output(&self) {
        let _ = kassiber_net::test_support::shutdown_read(&self.output);
    }

    pub fn send_line(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    pub fn send(&mut self, kind: &str, request_id: &str, args: Option<&str>) {
        let line = match args {
            Some(args) => format!(
                "{{\"kind\": {}, \"request_id\": {}, \"args\": {args}}}",
                text(&Value::from(kind)),
                text(&Value::from(request_id))
            ),
            None => format!(
                "{{\"kind\": {}, \"request_id\": {}}}",
                text(&Value::from(kind)),
                text(&Value::from(request_id))
            ),
        };
        self.send_line(&line);
    }

    pub fn close_stdin(&mut self) {
        self.stdin.take();
    }

    /// The next row, or `None` at end of output; panics after `timeout`.
    pub fn next(&mut self, timeout: Duration) -> Option<&Value> {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => {
                let row = kassiber_pytext::loads(&line)
                    .unwrap_or_else(|e| panic!("not a JSON row {line:?}: {e:?}"));
                self.rows.push(row);
                self.rows.last()
            }
            Err(RecvTimeoutError::Disconnected) => None,
            Err(RecvTimeoutError::Timeout) => {
                panic!("no row within {timeout:?}: {:?}", self.kinds())
            }
        }
    }

    pub fn read(&mut self) -> Value {
        self.next(Duration::from_secs(60))
            .cloned()
            .unwrap_or_else(|| panic!("the daemon closed its output: {:?}", self.kinds()))
    }

    /// Whether a row is waiting now.
    pub fn pending(&mut self, wait: Duration) -> Option<Value> {
        match self.lines.recv_timeout(wait) {
            Ok(line) => {
                let row = kassiber_pytext::loads(&line).unwrap();
                self.rows.push(row.clone());
                Some(row)
            }
            Err(_) => None,
        }
    }

    /// The next row for `request_id` of `kind` (or an error for it).
    pub fn until(&mut self, request_id: &str, kind: Option<&str>) -> Value {
        loop {
            let row = self.read();
            if field(&row, "request_id") == request_id {
                let row_kind = field(&row, "kind");
                if row_kind == "error" || kind.is_none_or(|k| row_kind == k) {
                    return row;
                }
            }
        }
    }

    pub fn kinds(&self) -> Vec<String> {
        self.rows.iter().map(|row| field(row, "kind")).collect()
    }

    /// Shut down and require a clean exit with nothing on stderr.
    pub fn finish(mut self) -> Vec<Value> {
        self.send("daemon.shutdown", "stop", None);
        let result = self.until("stop", None);
        assert_eq!(field(&result, "kind"), "daemon.shutdown", "{result:?}");
        self.close_stdin();
        self.wait_success()
    }

    pub fn wait(mut self) -> (std::process::ExitStatus, String, Vec<Value>) {
        while self.next(Duration::from_secs(60)).is_some() {}
        let status = wait_child(&mut self.child, Duration::from_secs(60));
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            let _ = std::io::Read::read_to_string(&mut pipe, &mut stderr);
        }
        (status, stderr, std::mem::take(&mut self.rows))
    }

    pub fn wait_success(self) -> Vec<Value> {
        let (status, stderr, rows) = self.wait();
        assert!(status.success(), "daemon exit {status}: {stderr}");
        assert_eq!(stderr, "", "the daemon wrote to stderr");
        rows
    }
}

/// A daemon a failing test leaves behind is killed, never orphaned.
impl Drop for Daemon {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// The child's exit within `timeout`; past it the child is killed and the
/// test fails, so a mutant that keeps a daemon alive cannot hang a run.
pub fn wait_child(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the daemon did not exit within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn field(row: &Value, key: &str) -> String {
    get(row, key).map(py_str).unwrap_or_default()
}

/// Rows per request id in order, and the rows without one in order.
fn grouped(rows: &[Value]) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for row in rows {
        let key = group(row);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(line(row)),
            None => groups.push((key, vec![line(row)])),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    groups
}

/// Whether this run rewrites the recordings instead of comparing them.
pub fn write_mode() -> bool {
    let on = std::env::var_os("KASSIBER_GOLDEN_WRITE").is_some_and(|v| v == "1");
    assert!(
        !on || std::env::var_os("CI").is_none(),
        "KASSIBER_GOLDEN_WRITE is refused in CI"
    );
    on
}

/// The request a row answers, or `events` for rows without one.
fn group(row: &Value) -> String {
    match get(row, "request_id") {
        Some(id) => format!("request {}", text(id)),
        None => "events".to_owned(),
    }
}

/// `expected` with every row `assert_same_rows` compares replaced by the
/// row `actual` holds at its place (its request, its position there) when
/// the two differ. The recording's order, its row count and the rows that
/// compare equal stay as recorded.
fn rewritten(name: &str, actual: &[Value], expected: &[Value]) -> Vec<Value> {
    for row in actual {
        let key = group(row);
        assert!(
            expected.iter().any(|e| group(e) == key),
            "{name}: unrecorded rows for {key}"
        );
    }
    let mut seen: Vec<(String, usize)> = Vec::new();
    expected
        .iter()
        .map(|recorded| {
            let key = group(recorded);
            let at = match seen.iter_mut().find(|(k, _)| *k == key) {
                Some((_, n)) => {
                    *n += 1;
                    *n - 1
                }
                None => {
                    seen.push((key.clone(), 1));
                    0
                }
            };
            let row = actual
                .iter()
                .filter(|row| group(row) == key)
                .nth(at)
                .unwrap_or_else(|| panic!("{name}: {key}: no row {at}, recorded one"));
            if line(row) == line(recorded) {
                recorded.clone()
            } else {
                in_recorded_order(row, recorded)
            }
        })
        .collect()
}

/// `actual` with its keys in the order `recorded` has them (new keys after
/// them): replayed rows come back key-sorted, and the comparison ignores
/// key order, so the recording's order stays.
fn in_recorded_order(actual: &Value, recorded: &Value) -> Value {
    match (actual, recorded) {
        (Value::Dict(items), Value::Dict(order)) => {
            let mut out: Vec<(Key, Value)> = order
                .iter()
                .filter_map(|(key, old)| {
                    let (_, new) = items.iter().find(|(k, _)| k == key)?;
                    Some((key.clone(), in_recorded_order(new, old)))
                })
                .collect();
            for (key, new) in items {
                if !order.iter().any(|(k, _)| k == key) {
                    out.push((key.clone(), new.clone()));
                }
            }
            Value::Dict(out)
        }
        (Value::List(items), Value::List(old)) => Value::List(
            items
                .iter()
                .enumerate()
                .map(|(at, new)| match old.get(at) {
                    Some(old) => in_recorded_order(new, old),
                    None => new.clone(),
                })
                .collect(),
        ),
        _ => actual.clone(),
    }
}

static WRITING: Mutex<()> = Mutex::new(());

fn slot<'a>(value: &'a mut Value, key: &str) -> &'a mut Value {
    match value {
        Value::Dict(items) => items
            .iter_mut()
            .find(|(k, _)| matches!(k, Key::Str(s) if s.is(key)))
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("no {key:?} in the recording")),
        other => panic!("expected an object, not {other:?}"),
    }
}

/// A recording as its files are laid out: compact ASCII JSON with each
/// case on a line of its own.
fn recording_text(root: &Value) -> String {
    let compact = Options::new().compact();
    let Value::Dict(items) = root else {
        panic!("a recording is an object")
    };
    let Some(((Key::Str(last), Value::List(cases)), head)) = items.split_last() else {
        panic!("a recording ends with its cases")
    };
    assert!(last.is("cases"), "a recording ends with its cases");
    let head = kassiber_pyjson::dumps(&Value::Dict(head.to_vec()), &compact).unwrap();
    let cases: Vec<String> = cases
        .iter()
        .map(|case| kassiber_pyjson::dumps(case, &compact).unwrap())
        .collect();
    format!(
        "{},\"cases\":[\n{}\n]}}\n",
        head.strip_suffix('}').unwrap(),
        cases.join(",\n")
    )
}

/// Write `rows` as the recorded rows of call `index` of `test[param]` in
/// the recording `module` (write mode).
pub fn write_rows(module: &str, test: &str, param: &str, index: usize, rows: Vec<Value>) {
    let _one_writer = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../wallets/tests/data/freshness")
        .join(format!("{module}.json"));
    let before = std::fs::read_to_string(&path).unwrap();
    let mut root = kassiber_pytext::loads(&before).expect("recording JSON");
    assert_eq!(
        recording_text(&root),
        before,
        "{}: not laid out as write mode writes it",
        path.display()
    );
    let Value::List(cases) = slot(&mut root, "cases") else {
        panic!("the cases are a list")
    };
    let mut matching = cases.iter_mut().filter(|case| {
        golden::string(case, "test") == test && golden::string(case, "param") == param
    });
    let case = matching
        .next()
        .unwrap_or_else(|| panic!("no recording of {test}[{param}]"));
    let Value::List(calls) = slot(case, "calls") else {
        panic!("the calls are a list")
    };
    *slot(&mut calls[index], "rows") = Value::List(rows);
    assert!(
        matching.next().is_none(),
        "two recordings of {test}[{param}]"
    );
    // Tests running alongside read the recording; they see it whole.
    let fresh = path.with_extension("json.write");
    std::fs::write(&fresh, recording_text(&root)).unwrap();
    std::fs::rename(&fresh, &path).unwrap();
}

pub fn assert_same_rows(name: &str, actual: &[Value], expected: &[Value]) {
    let actual = grouped(actual);
    let expected = grouped(expected);
    for (key, rows) in &expected {
        let got = actual
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, r)| r.as_slice())
            .unwrap_or(&[]);
        assert!(
            got.len() >= rows.len(),
            "{name}: {key}: {} rows, recorded {}:\n{got:#?}\nrecorded:\n{rows:#?}",
            got.len(),
            rows.len()
        );
        for (index, (a, e)) in got.iter().zip(rows).enumerate() {
            assert_eq!(a, e, "{name}: {key} row {index} differs from the recording");
        }
    }
    for (key, rows) in &actual {
        assert!(
            expected.iter().any(|(k, _)| k == key),
            "{name}: unrecorded rows for {key}: {rows:#?}"
        );
    }
}

/// A recorded session's scratch: its root with the book restored, the
/// owner root, the tape and the environment.
pub struct Prepared {
    pub root: PathBuf,
    pub book: PathBuf,
    pub tape: PathBuf,
    pub hold: PathBuf,
    pub env: Vec<(String, String)>,
}

pub fn prepare(golden: &Golden, scratch: &Scratch, index: usize, session: &Value) -> Prepared {
    let root = PathBuf::from(scratch.place(&string(session, "root")));
    std::fs::create_dir_all(&root).unwrap();
    let book = root.join("kassiber.sqlite3");
    let _ = std::fs::remove_file(&book);
    for suffix in ["-wal", "-shm", "-journal"] {
        let _ = std::fs::remove_file(root.join(format!("kassiber.sqlite3{suffix}")));
    }
    let before = golden.resolve(value(session, "before"));
    if !matches!(before, Value::Null) {
        restore(golden, before, &book);
        // A book the test had encrypted was recorded in plaintext with its
        // synthetic passphrase; encrypt it the product's way.
        if let Some(key) = get(before, "encrypted_with") {
            let owners =
                kassiber_ownership::OwnerNamespace::isolated(scratch.0.join("encrypt-owners"));
            kassiber_store::encryption::encrypt_plaintext(&owners, &root, &py_str(key), None)
                .expect("encrypt the restored book");
        }
    }
    let env = environment(scratch, value(session, "env"));
    for (key, path) in &env {
        if key == "KASSIBER_RUST_TEST_OWNER_ROOT" {
            std::fs::create_dir_all(path).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
    }
    let tape = scratch.0.join(format!("session-{index}.tape"));
    std::fs::write(&tape, py_str(golden.resolve(value(session, "tape")))).unwrap();
    let hold = scratch.0.join(format!("session-{index}.hold"));
    std::fs::create_dir_all(&hold).unwrap();
    Prepared {
        root,
        book,
        tape,
        hold,
        env,
    }
}

/// Replay a recorded session as it was driven: each line goes out once as
/// many rows have arrived as had then; the rows and the book must match.
/// In write mode the rows to record are returned instead of compared.
pub fn replay_session(
    golden: &Golden,
    scratch: &Scratch,
    name: &str,
    index: usize,
    session: &Value,
) -> Option<Vec<Value>> {
    let prepared = prepare(golden, scratch, index, session);
    let mut daemon = Daemon::start(&prepared.root, &prepared.env, &prepared.tape, None);
    let deadline = Instant::now() + Duration::from_secs(120);
    let sent = golden::list(value(session, "sent"));
    for item in sent {
        let after = py_str(value(item, "after_rows")).parse::<usize>().unwrap();
        while daemon.rows.len() < after {
            assert!(
                Instant::now() < deadline,
                "{name}: rows never came: {:?}",
                daemon.kinds()
            );
            if daemon.next(Duration::from_secs(60)).is_none() {
                panic!("{name}: the daemon closed early: {:?}", daemon.kinds());
            }
        }
        daemon.send_line(&scratch.place(&string(item, "line")));
    }
    let expected = golden::list(value(session, "rows"));
    // The test read up to its last expected row; read at least as many.
    while daemon.rows.len() < expected.len() {
        if daemon.next(Duration::from_secs(60)).is_none() {
            break;
        }
    }
    daemon.close_stdin();
    let (status, stderr, rows) = daemon.wait();
    let recorded = py_str(value(session, "returncode"));
    assert_eq!(
        status.code().map(|c| c.to_string()).unwrap_or_default(),
        recorded,
        "{name}: exit status; stderr: {stderr}"
    );
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|row| Value::from(scratch_unplace(scratch, &text(&row)).as_str()))
        .map(|text| kassiber_pytext::loads(&py_str(&text)).unwrap())
        .collect();
    let write = write_mode().then(|| rewritten(name, &rows, expected));
    if write.is_none() {
        assert_same_rows(name, &rows, expected);
    }
    assert_requests_recorded(name, &prepared.tape);
    let after = value(session, "after");
    if !matches!(after, Value::Null) {
        assert_same_book(
            &format!("{name} session {index}"),
            &snapshot(&prepared.book),
            after,
        );
    }
    write
}

/// `text` with the scratch directory put back as the recording's
/// placeholder, so rows naming paths compare.
fn scratch_unplace(scratch: &Scratch, text: &str) -> String {
    text.replace(&scratch.path(), "@TMP@").replace(BIN, "@BIN@")
}

/// Standard base64, as the tape stores payloads.
pub fn base64(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u32, 0);
    for byte in text.bytes().filter(|b| *b != b'=') {
        let Some(value) = ALPHABET.iter().position(|a| *a == byte) else {
            continue;
        };
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    out
}

/// How often `needle` occurs in one request; an empty needle counts the
/// request once.
fn occurrences(bytes: &[u8], needle: &str) -> usize {
    if needle.is_empty() {
        return 1;
    }
    String::from_utf8_lossy(bytes).matches(needle).count()
}

/// How often `needle` occurs in the requests the recording made.
pub fn recorded_requests(tape: &Path, needle: &str) -> usize {
    std::fs::read_to_string(tape)
        .unwrap()
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            (parts.first() == Some(&"w") && parts.len() >= 4)
                .then(|| occurrences(&base64(parts[3]), needle))
        })
        .sum()
}

/// How often `needle` occurs in the requests the replays from `tape` made.
pub fn replayed_requests(tape: &Path, needle: &str) -> usize {
    std::fs::read_to_string(tape.with_extension("writes"))
        .unwrap_or_default()
        .lines()
        .map(|line| occurrences(&base64(line), needle))
        .sum()
}

/// Every request the replays from `tape` made is one the recording made,
/// byte for byte and no more often: the pytest tests counted and inspected
/// what their chain received. A different request fails like an extra one.
/// The replay may stop earlier than the recording did.
pub fn assert_requests_recorded(name: &str, tape: &Path) {
    let mut recorded: Vec<String> = std::fs::read_to_string(tape)
        .unwrap()
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            (parts.first() == Some(&"w") && parts.len() >= 4).then(|| parts[3].to_owned())
        })
        .collect();
    let replayed = std::fs::read_to_string(tape.with_extension("writes")).unwrap_or_default();
    for (index, request) in replayed.lines().enumerate() {
        match recorded.iter().position(|r| r == request) {
            Some(at) => {
                recorded.swap_remove(at);
            }
            None => panic!(
                "{name}: request {index} was not recorded (or not that often): {:?}",
                String::from_utf8_lossy(&base64(request))
            ),
        }
    }
}

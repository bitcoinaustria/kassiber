//! A disposable book, the harness binary run against it, and golden
//! transcripts of what it answered.
//!
//! Every child runs on the sandbox's data root with its XDG roots, offline
//! preference, agent-access and update files, owner namespace and state root
//! redirected into the sandbox, under the test egress guard. A transcript
//! records each command's argv, exit status, stdout and stderr (and book
//! dumps where a test asks for one) and must equal its checked-in golden
//! under `tests/black_box/golden/`. `KASSIBER_BLESS=1` rewrites the goldens
//! instead; review the diff before committing it.

#![allow(dead_code)]

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kassiber_store::rusqlite::types::ValueRef;
use kassiber_store::rusqlite::{self, Connection};

/// The repository root: children run from it, as the CLI tests ran from it.
pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

pub struct Output {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", self.text()))
    }
}

pub struct Sandbox {
    // Held for its Drop.
    dir: tempfile::TempDir,
    pub base: PathBuf,
    pub root: PathBuf,
    pub passphrase: Option<String>,
    env: Vec<(String, String)>,
}

impl Sandbox {
    /// A sandbox whose data root does not exist yet.
    pub fn empty() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("kassiber-black-box-")
            .tempdir()
            .unwrap();
        // Disposable roots must not hold a symlink on their way.
        let base = dir.path().canonicalize().unwrap();
        for sub in ["owners", "xdg-data", "xdg-config", "xdg-state", "state"] {
            std::fs::create_dir_all(base.join(sub)).unwrap();
        }
        let path = |sub: &str| base.join(sub).to_string_lossy().into_owned();
        let env = vec![
            ("XDG_DATA_HOME".into(), path("xdg-data")),
            ("XDG_CONFIG_HOME".into(), path("xdg-config")),
            ("XDG_STATE_HOME".into(), path("xdg-state")),
            ("KASSIBER_STATE_ROOT".into(), path("state")),
            ("KASSIBER_RUST_TEST_OWNER_ROOT".into(), path("owners")),
            (
                "KASSIBER_OFFLINE_PREFERENCE_FILE".into(),
                path("offline.json"),
            ),
            ("KASSIBER_AGENT_ACCESS_FILE".into(), path("agents.json")),
            (
                "KASSIBER_UPDATE_PREFERENCE_FILE".into(),
                path("updates.json"),
            ),
            (
                "KASSIBER_UPDATE_CACHE_FILE".into(),
                path("update-cache.json"),
            ),
            ("KASSIBER_DISABLE_UPDATE_CHECK".into(), "1".into()),
            ("KASSIBER_TEST_NO_EGRESS".into(), "1".into()),
            ("COLUMNS".into(), "80".into()),
            ("LINES".into(), "24".into()),
            ("TZ".into(), "UTC".into()),
        ];
        let root = base.join("data");
        Self {
            dir,
            base,
            root,
            passphrase: None,
            env,
        }
    }

    /// A plaintext book holding the store's current schema and `sql`.
    pub fn book(sql: &str) -> Self {
        let sandbox = Self::empty();
        sandbox.create(sql);
        sandbox
    }

    /// An encrypted book holding the store's current schema and `sql`.
    pub fn encrypted_book(passphrase: &str, sql: &str) -> Self {
        let mut sandbox = Self::empty();
        sandbox.passphrase = Some(passphrase.to_owned());
        sandbox.create(sql);
        sandbox
    }

    fn create(&self, sql: &str) {
        let book = kassiber_store::Book::create(&self.root, self.passphrase.as_deref())
            .unwrap_or_else(|e| panic!("create the book: {e:?}"));
        book.connection().execute_batch(sql).unwrap();
        book.close().map_err(|e| e.error).unwrap();
    }

    /// Set (or override) a variable for every later child.
    pub fn set(&mut self, key: &str, value: impl Into<String>) -> &mut Self {
        self.env.retain(|(k, _)| k != key);
        self.env.push((key.to_owned(), value.into()));
        self
    }

    pub fn unset(&mut self, key: &str) -> &mut Self {
        self.env.retain(|(k, _)| k != key);
        self
    }

    /// The journal clock and ids the CLI tests fixed.
    pub fn journal_clock(&mut self, clock: &str) -> &mut Self {
        self.set("KASSIBER_RUST_TEST_JOURNAL_TIME", clock);
        self.set("KASSIBER_RUST_TEST_JOURNAL_IDS", "sequential")
    }

    pub fn path(&self, sub: &str) -> PathBuf {
        self.base.join(sub)
    }

    /// The database, opened directly (keyed when encrypted).
    pub fn connection(&self) -> Connection {
        let conn = Connection::open(self.root.join("kassiber.sqlite3")).unwrap();
        if let Some(passphrase) = &self.passphrase {
            conn.pragma_update(None, "key", passphrase).unwrap();
        }
        conn
    }

    /// Apply `sql` and close the connection.
    pub fn sql(&self, sql: &str) {
        let conn = self.connection();
        conn.execute_batch(sql).unwrap();
        conn.close().map_err(|(_, e)| e).unwrap();
    }

    pub fn query_strings(&self, sql: &str) -> Vec<String> {
        let conn = self.connection();
        let mut statement = conn.prepare(sql).unwrap();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kassiber"));
        for (key, _) in std::env::vars() {
            if key.starts_with("KASSIBER_")
                || key.starts_with("SATBOOKS_")
                || key.starts_with("XDG_")
            {
                command.env_remove(key);
            }
        }
        command
            .env_remove("COLUMNS")
            .env_remove("LINES")
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .current_dir(repo());
        command
    }

    /// `kassiber --machine --data-root <root> argv...`.
    pub fn machine(&self, argv: &[&str]) -> Output {
        let mut args = vec!["--machine"];
        args.extend_from_slice(argv);
        self.run(&args, None)
    }

    /// `kassiber --data-root <root> argv...`, with the passphrase on stdin
    /// for an encrypted book.
    pub fn run(&self, argv: &[&str], stdin: Option<&[u8]>) -> Output {
        let root = self.root.to_string_lossy().into_owned();
        let (flags, prefix): (Vec<&str>, &[&str]) = match argv.first() {
            Some(&"--machine") => (vec!["--machine"], &argv[1..]),
            _ => (vec![], argv),
        };
        let mut args: Vec<String> = flags.iter().map(|s| (*s).to_owned()).collect();
        args.extend(["--data-root".to_owned(), root]);
        let mut input = stdin.map(<[u8]>::to_vec);
        if let Some(passphrase) = &self.passphrase {
            assert!(input.is_none(), "stdin carries the passphrase");
            args.extend(["--db-passphrase-fd".to_owned(), "0".to_owned()]);
            input = Some(format!("{passphrase}\n").into_bytes());
        }
        args.extend(prefix.iter().map(|s| (*s).to_owned()));
        self.raw(&args, input.as_deref())
    }

    /// The binary with exactly `args`.
    pub fn raw(&self, args: &[String], stdin: Option<&[u8]>) -> Output {
        let mut command = self.command();
        command.args(args);
        spawn(command, stdin)
    }

    /// One daemon session: each request a JSON line, then `daemon.shutdown`.
    /// Returns the response lines after `daemon.ready`.
    pub fn daemon(&self, requests: &[serde_json::Value]) -> Vec<String> {
        let mut wire = Vec::new();
        let mut all = requests.to_vec();
        if let Some(passphrase) = &self.passphrase {
            all.insert(
                0,
                serde_json::json!({"request_id": "unlock", "kind": "daemon.unlock",
                    "args": {"auth_response": {"passphrase_secret": passphrase}}}),
            );
        }
        all.push(serde_json::json!({"request_id": "stop", "kind": "daemon.shutdown"}));
        for request in &all {
            wire.extend_from_slice(request.to_string().as_bytes());
            wire.push(b'\n');
        }
        let root = self.root.to_string_lossy().into_owned();
        let args: Vec<String> = ["--machine", "--data-root", &root, "daemon"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let output = self.raw(&args, Some(&wire));
        assert_eq!(
            (
                output.code,
                String::from_utf8_lossy(&output.stderr).as_ref()
            ),
            (0, ""),
            "{}",
            output.text()
        );
        let text = output.text();
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        assert!(lines[0].contains("\"daemon.ready\""), "{}", lines[0]);
        lines.remove(0);
        assert_eq!(lines.len(), all.len(), "{text}");
        lines
    }

    /// Every table, rows sorted, one line per row; empty tables are counted.
    pub fn dump(&self) -> String {
        let conn = self.connection();
        dump(&conn)
    }

    /// The text with this sandbox's paths replaced by placeholders.
    pub fn scrub(&self, text: &str) -> String {
        text.replace(&self.base.to_string_lossy().into_owned(), "<SANDBOX>")
            .replace(&repo().to_string_lossy().into_owned(), "<REPO>")
    }
}

fn spawn(mut command: Command, stdin: Option<&[u8]>) -> Output {
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("the binary runs");
    if let Some(bytes) = stdin {
        let mut pipe = child.stdin.take().unwrap();
        let bytes = bytes.to_vec();
        std::thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        });
    }
    let output = child.wait_with_output().unwrap();
    Output {
        code: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

pub fn dump(conn: &Connection) -> String {
    let mut tables: Vec<String> = {
        let mut statement = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' \
                 AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap();
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        names
    };
    tables.sort();
    let mut out = String::new();
    let mut empty = 0;
    for table in &tables {
        let mut statement = conn
            .prepare(&format!("SELECT * FROM \"{}\"", table.replace('"', "\"\"")))
            .unwrap();
        let columns: Vec<String> = statement
            .column_names()
            .iter()
            .map(|c| (*c).to_owned())
            .collect();
        let mut rows: Vec<String> = statement
            .query_map([], |row| {
                let mut line = String::new();
                for (index, column) in columns.iter().enumerate() {
                    if index > 0 {
                        line.push_str(" | ");
                    }
                    let _ = write!(line, "{column}=");
                    line.push_str(&value(row.get_ref(index)?));
                }
                Ok(line)
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        if rows.is_empty() {
            empty += 1;
            continue;
        }
        // Generated ids are random: order rows as if they were blank.
        rows.sort_by_cached_key(|row| blank_uuids(row));
        for row in rows {
            let _ = writeln!(out, "{table}: {row}");
        }
    }
    let _ = writeln!(out, "({} tables, {empty} empty)", tables.len());
    out
}

fn value(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".into(),
        ValueRef::Integer(i) => format!("i:{i}"),
        ValueRef::Real(f) => format!("r:{f:?}"),
        ValueRef::Text(t) => format!("t:{:?}", String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => {
            let mut hex = String::with_capacity(b.len() * 2);
            for byte in b {
                let _ = write!(hex, "{byte:02x}");
            }
            format!("b:{hex}")
        }
    }
}

/// What a test observed, compared with (or blessed into) its golden file.
pub struct Transcript {
    name: String,
    text: String,
    start: i64,
}

impl Transcript {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            text: String::new(),
            start: now_us(),
        }
    }

    pub fn note(&mut self, line: &str) {
        let _ = writeln!(self.text, "# {line}");
    }

    pub fn section(&mut self, title: &str, body: &str) {
        let _ = writeln!(self.text, "=== {title}");
        self.text.push_str(body);
        if !body.ends_with('\n') {
            self.text.push_str("\n<no newline>\n");
        }
    }

    /// Record one command and its outcome; paths are scrubbed.
    pub fn output(&mut self, sandbox: &Sandbox, argv: &[&str], output: &Output) {
        let _ = writeln!(self.text, "$ {}", sandbox.scrub(&argv.join(" ")));
        let _ = writeln!(self.text, "exit {}", output.code);
        for (name, bytes) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
            if bytes.is_empty() {
                continue;
            }
            let text = sandbox.scrub(&String::from_utf8_lossy(bytes));
            let _ = writeln!(self.text, "--- {name}");
            self.text.push_str(&text);
            if !text.ends_with('\n') {
                self.text.push_str("\n<no newline>\n");
            }
        }
    }

    /// Run `kassiber --data-root <root> argv` and record it.
    pub fn run(&mut self, sandbox: &Sandbox, argv: &[&str]) -> Output {
        let output = sandbox.run(argv, None);
        self.output(sandbox, argv, &output);
        output
    }

    /// Run `kassiber --machine --data-root <root> argv` and record it.
    pub fn machine(&mut self, sandbox: &Sandbox, argv: &[&str]) -> Output {
        let mut args = vec!["--machine"];
        args.extend_from_slice(argv);
        self.run(sandbox, &args)
    }

    /// One daemon session, recorded request by response.
    pub fn daemon(
        &mut self,
        sandbox: &Sandbox,
        requests: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        let lines = sandbox.daemon(requests);
        let offset = usize::from(sandbox.passphrase.is_some());
        let _ = writeln!(self.text, "$ daemon");
        for (index, line) in lines.iter().enumerate() {
            let request = if index < offset {
                "<unlock>".to_owned()
            } else {
                requests
                    .get(index - offset)
                    .map_or_else(|| "<shutdown>".to_owned(), ToString::to_string)
            };
            let _ = writeln!(self.text, "> {}", sandbox.scrub(&request));
            let _ = writeln!(self.text, "< {}", sandbox.scrub(line));
        }
        lines[offset..lines.len() - 1]
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    pub fn dump(&mut self, sandbox: &Sandbox, title: &str) {
        let dump = sandbox.dump();
        self.section(&format!("book {title}"), &sandbox.scrub(&dump));
    }

    /// The rows that changed since `before` (a `Sandbox::dump`): removed
    /// rows first, then added ones, each sorted.
    pub fn changes(&mut self, sandbox: &Sandbox, before: &str, title: &str) {
        let after = sandbox.dump();
        let mut removed: Vec<&str> = before.lines().collect();
        let mut added = Vec::new();
        for line in after.lines() {
            match removed.iter().position(|l| *l == line) {
                Some(index) => {
                    removed.remove(index);
                }
                None => added.push(line),
            }
        }
        let mut body = String::new();
        for line in removed {
            let _ = writeln!(body, "- {line}");
        }
        for line in added {
            let _ = writeln!(body, "+ {line}");
        }
        if body.is_empty() {
            body.push_str("(unchanged)\n");
        }
        self.section(&format!("book changes {title}"), &sandbox.scrub(&body));
    }

    /// Compare with the golden, or rewrite it under `KASSIBER_BLESS=1`.
    pub fn finish(mut self) {
        self.text = mask_generated(&self.text, self.start, now_us(), &mut Vec::new());
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/black_box/golden")
            .join(format!("{}.txt", self.name));
        if std::env::var_os("KASSIBER_BLESS").is_some_and(|v| v == "1") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &self.text).unwrap();
            return;
        }
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e}; bless it with KASSIBER_BLESS=1", path.display()));
        if expected != self.text {
            let first = expected
                .lines()
                .zip(self.text.lines())
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| expected.lines().count().min(self.text.lines().count()));
            let context = |text: &str| {
                text.lines()
                    .skip(first.saturating_sub(3))
                    .take(8)
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            panic!(
                "{} differs from its golden at line {}\n--- expected\n{}\n--- actual\n{}",
                self.name,
                first + 1,
                context(&expected),
                context(&self.text)
            );
        }
    }
}

/// Microseconds since the epoch.
pub fn now_us() -> i64 {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    i64::try_from(elapsed.as_micros()).unwrap()
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn digits(bytes: &[u8], at: usize, len: usize) -> Option<i64> {
    let slice = bytes.get(at..at + len)?;
    slice
        .iter()
        .all(u8::is_ascii_digit)
        .then(|| std::str::from_utf8(slice).unwrap().parse().unwrap())
}

/// A `YYYY-MM-DD[T ]HH:MM:SS[.f][Z|±HH:MM]` reading at `at`: its epoch
/// microseconds (UTC unless offset) and its length.
fn reading(bytes: &[u8], at: usize) -> Option<(i64, usize)> {
    let year = digits(bytes, at, 4)?;
    if bytes.get(at + 4) != Some(&b'-') || bytes.get(at + 7) != Some(&b'-') {
        return None;
    }
    let month = digits(bytes, at + 5, 2)?;
    let day = digits(bytes, at + 8, 2)?;
    if !matches!(bytes.get(at + 10), Some(b'T' | b' ')) {
        return None;
    }
    let hour = digits(bytes, at + 11, 2)?;
    let minute = digits(bytes, at + 14, 2)?;
    let second = digits(bytes, at + 17, 2)?;
    if bytes.get(at + 13) != Some(&b':') || bytes.get(at + 16) != Some(&b':') {
        return None;
    }
    let mut end = at + 19;
    let mut micros = 0;
    if bytes.get(end) == Some(&b'.') {
        let start = end + 1;
        end = start;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        let fraction = std::str::from_utf8(&bytes[start..end]).unwrap();
        let padded = format!("{fraction:0<6}");
        micros = padded[..6].parse().unwrap_or(0);
    }
    let mut offset = 0;
    match bytes.get(end) {
        Some(b'Z') => end += 1,
        Some(sign @ (b'+' | b'-')) => {
            if let (Some(h), Some(m)) = (digits(bytes, end + 1, 2), digits(bytes, end + 4, 2)) {
                offset = (h * 60 + m) * 60_000_000 * if *sign == b'+' { 1 } else { -1 };
                end += 6;
            }
        }
        _ => {}
    }
    let days = days_from_civil(year, month, day);
    let us = (((days * 24 + hour) * 60 + minute) * 60 + second) * 1_000_000 + micros - offset;
    Some((us, end - at))
}

fn is_uuid4(bytes: &[u8]) -> bool {
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            14 => *b == b'4',
            19 => matches!(b, b'8' | b'9' | b'a' | b'b'),
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(b),
        })
}

/// `text` with every uuid4 replaced by `<uuid>`.
pub fn blank_uuids(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < bytes.len() {
        if at + 36 <= bytes.len() && is_uuid4(&bytes[at..at + 36]) {
            out.push_str("<uuid>");
            at += 36;
            continue;
        }
        let ch = text[at..].chars().next().unwrap();
        out.push(ch);
        at += ch.len_utf8();
    }
    out
}

/// Values the run generated: uuid4s, numbered by first appearance, and
/// wall-clock readings inside `[start, end]` (with five seconds' slack).
/// Fixture dates and ids are never touched.
pub fn mask_generated(text: &str, start: i64, end: i64, seen: &mut Vec<String>) -> String {
    const SLACK: i64 = 5_000_000;
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < bytes.len() {
        if at + 36 <= bytes.len() && is_uuid4(&bytes[at..at + 36]) {
            let id = &text[at..at + 36];
            let number = match seen.iter().position(|s| s == id) {
                Some(n) => n + 1,
                None => {
                    seen.push(id.to_owned());
                    seen.len()
                }
            };
            let _ = write!(out, "<uuid#{number}>");
            at += 36;
            continue;
        }
        if let Some((us, len)) = reading(bytes, at) {
            if us >= start - SLACK && us <= end + SLACK {
                out.push_str("<now>");
                at += len;
                continue;
            }
        }
        let ch = text[at..].chars().next().unwrap();
        out.push(ch);
        at += ch.len_utf8();
    }
    out
}

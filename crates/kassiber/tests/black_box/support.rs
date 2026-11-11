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

/// The fixed identity and clock the seeding opens stamp, so migration
/// audits and instance ids read the same every run.
pub const IDENTITY: &str = "0123456789abcdef0123456789abcdef";
pub const TIMESTAMP: &str = "2026-01-02T03:04:05.678Z";

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
        process_start();
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
        let book = kassiber_store::test_hooks::create(
            &self.root,
            self.passphrase.as_deref(),
            IDENTITY,
            TIMESTAMP,
            &mut |_| Ok(()),
        )
        .unwrap_or_else(|e| panic!("create the book: {e:?}"));
        book.close().map_err(|e| e.error).unwrap();
        self.sql(sql);
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

    /// The book as the store opens it (keyed when encrypted); `migrate`
    /// runs the open-time compatibility pass, as Python's `open_db` did.
    pub fn with_book<T>(&self, migrate: bool, f: impl FnOnce(&Connection) -> T) -> T {
        let options = kassiber_store::OpenOptions {
            passphrase: self.passphrase.as_deref(),
            expected_database_identity: None,
            expected_project: None,
        };
        let book = if migrate {
            kassiber_store::Book::open_migrating_with_sources(
                &self.root, options, true, IDENTITY, TIMESTAMP,
            )
        } else {
            kassiber_store::Book::open(&self.root, options)
        }
        .unwrap_or_else(|e| panic!("open the book: {e:?}"));
        let value = f(book.connection());
        book.close().map_err(|e| e.error).unwrap();
        value
    }

    /// Apply `sql` on a migrating open, then open once more so open-time
    /// backfills see the new rows (the Python harness's `change`).
    pub fn sql(&self, sql: &str) {
        self.with_book(true, |conn| conn.execute_batch(sql).unwrap());
        self.with_book(true, |_| ());
    }

    pub fn query_strings(&self, sql: &str) -> Vec<String> {
        self.with_book(false, |conn| {
            let mut statement = conn.prepare(sql).unwrap();
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows
        })
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
        let root = self.root.to_string_lossy().into_owned();
        let mut all = requests.to_vec();
        if let Some(passphrase) = &self.passphrase {
            all.insert(
                0,
                serde_json::json!({"request_id": "unlock", "kind": "daemon.unlock",
                    "args": {"auth_response": {"passphrase_secret": passphrase}}}),
            );
        }
        self.daemon_argv(&["--machine", "--data-root", &root, "daemon"], &all)
    }

    /// A daemon started with exactly `argv`, fed `requests` and then
    /// `daemon.shutdown`; the response lines after `daemon.ready`.
    pub fn daemon_argv(&self, argv: &[&str], requests: &[serde_json::Value]) -> Vec<String> {
        let mut wire = Vec::new();
        let mut all = requests.to_vec();
        all.push(serde_json::json!({"request_id": "stop", "kind": "daemon.shutdown"}));
        for request in &all {
            wire.extend_from_slice(request.to_string().as_bytes());
            wire.push(b'\n');
        }
        let args: Vec<String> = argv.iter().map(|s| (*s).to_owned()).collect();
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
        self.with_book(false, dump)
    }

    /// The plaintext database dumped without the store's open (which
    /// refuses schema drift).
    pub fn dump_raw(&self) -> String {
        assert!(self.passphrase.is_none());
        let conn = Connection::open(self.root.join("kassiber.sqlite3")).unwrap();
        let text = dump(&conn);
        conn.close().map_err(|(_, e)| e).unwrap();
        text
    }

    /// Apply `sql` on a raw plaintext connection, outside the store.
    pub fn sql_raw(&self, sql: &str) {
        assert!(self.passphrase.is_none());
        let conn = Connection::open(self.root.join("kassiber.sqlite3")).unwrap();
        conn.execute_batch(sql).unwrap();
        conn.close().map_err(|(_, e)| e).unwrap();
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

/// The book at `root` dumped through the store's open.
pub fn dump_at(root: &Path, passphrase: Option<&str>) -> String {
    let options = kassiber_store::OpenOptions {
        passphrase,
        expected_database_identity: None,
        expected_project: None,
    };
    let book = kassiber_store::Book::open(root, options)
        .unwrap_or_else(|e| panic!("open {}: {e:?}", root.display()));
    let text = dump(book.connection());
    book.close().map_err(|e| e.error).unwrap();
    text
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
        // Generated ids, and digests over them, are random: order rows as
        // if they were blank (ties keep rowid order).
        rows.sort_by_cached_key(|row| blank_hex64(&blank_uuids(row)));
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

type Mask = Box<dyn Fn(&str) -> String + Send + Sync>;

/// What a test observed, compared with (or blessed into) its golden file.
pub struct Transcript {
    name: String,
    text: String,
    start: i64,
    masks: Vec<Mask>,
    /// Compact transcripts name requests by index and keep shorter heads.
    compact: bool,
    /// Readings after the run (expiries the product counts from the wall
    /// clock) are generated too.
    future: bool,
}

impl Transcript {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            text: String::new(),
            start: process_start(),
            masks: Vec::new(),
            compact: false,
            future: false,
        }
    }

    /// A transcript for one part of a test run elsewhere (a thread), to be
    /// appended to this one in order.
    pub fn part(&self) -> Transcript {
        let mut part = Transcript::new(&self.name);
        part.start = self.start;
        part.compact = self.compact;
        part
    }

    /// Append a part's text as recorded (masks apply at `finish`).
    pub fn append(&mut self, part: Transcript) {
        self.text.push_str(&part.text);
    }

    /// The raw transcript text, for recorders outside this module.
    pub fn text_mut(&mut self) -> &mut String {
        &mut self.text
    }

    /// Mask readings later than the run as `<later>`: for books without
    /// future fixture dates whose expiries count from the wall clock.
    pub fn future_is_generated(&mut self) -> &mut Self {
        self.future = true;
        self
    }

    /// Requests by index (their bodies live in the test's input file) and
    /// long lines digested past a short head: for sessions of hundreds of
    /// requests.
    pub fn compact(&mut self) -> &mut Self {
        self.compact = true;
        self
    }

    fn shown(&self, index: usize, request: &serde_json::Value, sandbox: &Sandbox) -> String {
        if self.compact {
            format!("#{index}")
        } else {
            sandbox.scrub(&request.to_string())
        }
    }

    /// A further rewrite of the whole transcript before it is compared,
    /// for values a run legitimately generates differently each time.
    pub fn mask(&mut self, mask: impl Fn(&str) -> String + Send + Sync + 'static) -> &mut Self {
        self.masks.push(Box::new(mask));
        self
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
                requests.get(index - offset).map_or_else(
                    || "<shutdown>".to_owned(),
                    |r| self.shown(index - offset, r, sandbox),
                )
            };
            let _ = writeln!(self.text, "> {request}");
            let _ = writeln!(self.text, "< {}", sandbox.scrub(line));
        }
        lines[offset..lines.len() - 1]
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// A daemon started with `argv`: each request and its response line.
    pub fn daemon_argv(
        &mut self,
        sandbox: &Sandbox,
        argv: &[&str],
        requests: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        let lines = sandbox.daemon_argv(argv, requests);
        let _ = writeln!(self.text, "$ {}", sandbox.scrub(&argv.join(" ")));
        for (index, line) in lines.iter().enumerate() {
            let request = requests
                .get(index)
                .map_or_else(|| "<shutdown>".to_owned(), ToString::to_string);
            let _ = writeln!(self.text, "> {}", sandbox.scrub(&request));
            let _ = writeln!(self.text, "< {}", sandbox.scrub(line));
        }
        lines[..lines.len() - 1]
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// One daemon session whose responses `normalize` rewrites before they
    /// are recorded (for per-process clocks and byte counts).
    pub fn daemon_normalized(
        &mut self,
        sandbox: &Sandbox,
        requests: &[serde_json::Value],
        normalize: impl Fn(&mut serde_json::Value),
    ) -> Vec<serde_json::Value> {
        let lines = sandbox.daemon(requests);
        let offset = usize::from(sandbox.passphrase.is_some());
        let _ = writeln!(self.text, "$ daemon");
        let mut responses = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let request = if index < offset {
                "<unlock>".to_owned()
            } else {
                requests
                    .get(index - offset)
                    .map_or_else(|| "<shutdown>".to_owned(), ToString::to_string)
            };
            let mut value: serde_json::Value = serde_json::from_str(line).unwrap();
            normalize(&mut value);
            let _ = writeln!(self.text, "> {}", sandbox.scrub(&request));
            let _ = writeln!(self.text, "< {}", sandbox.scrub(&value.to_string()));
            responses.push(value);
        }
        responses.truncate(responses.len() - 1);
        responses.drain(..offset);
        responses
    }

    pub fn dump(&mut self, sandbox: &Sandbox, title: &str) {
        let dump = sandbox.dump();
        self.section(&format!("book {title}"), &sandbox.scrub(&dump));
    }

    /// The rows that changed since `before` (a `Sandbox::dump`): removed
    /// rows first, then added ones, each sorted.
    pub fn changes(&mut self, sandbox: &Sandbox, before: &str, title: &str) {
        let after = sandbox.dump();
        // Rows compare without their generated ids and clocks, which a
        // rewrite may restamp within the same run.
        let (start, now) = (self.start, now_us());
        let key = |line: &str| mask_generated(&blank_uuids(line), start, now, &mut Vec::new());
        let mut removed: Vec<&str> = before.lines().collect();
        let mut keys: Vec<String> = removed.iter().map(|l| key(l)).collect();
        let mut added = Vec::new();
        for line in after.lines() {
            let wanted = key(line);
            match keys.iter().position(|k| *k == wanted) {
                Some(index) => {
                    removed.remove(index);
                    keys.remove(index);
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

    /// `changes` with generated ids blank: rows a run lists in random id
    /// order would otherwise number those ids differently each run.
    pub fn changes_blanked(&mut self, sandbox: &Sandbox, before: &str, title: &str) {
        let mut scratch = Transcript::new("scratch");
        scratch.start = self.start;
        scratch.changes(sandbox, before, title);
        self.text.push_str(&blank_uuids(&scratch.text));
    }

    /// Compare with the golden, or rewrite it under `KASSIBER_BLESS=1`.
    pub fn finish(mut self) {
        let end = if self.future { i64::MAX } else { now_us() };
        self.text = mask_generated(&self.text, self.start, end, &mut Vec::new());
        for mask in &self.masks {
            self.text = mask(&self.text);
        }
        let full = std::mem::take(&mut self.text);
        let (limit, head) = if self.compact {
            (320, 100)
        } else {
            (LONG_LINE, 240)
        };
        self.text = digest_long_lines(&digest_blocks(&full), limit, head);
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
            let actual = Path::new(env!("CARGO_TARGET_TMPDIR"))
                .join("black_box")
                .join(format!("{}.actual.txt", self.name));
            std::fs::create_dir_all(actual.parent().unwrap()).unwrap();
            std::fs::write(&actual, &full).unwrap();
            eprintln!(
                "the whole transcript, long lines kept: {}",
                actual.display()
            );
            let first = expected
                .lines()
                .zip(self.text.lines())
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| expected.lines().count().min(self.text.lines().count()));
            let want = expected.lines().nth(first).unwrap_or("<end>");
            let have = self.text.lines().nth(first).unwrap_or("<end>");
            let at = want
                .char_indices()
                .zip(have.chars())
                .find(|((_, a), b)| a != b)
                .map_or(want.len().min(have.len()), |((i, _), _)| i);
            let window = |line: &str| {
                let start = line.floor_char_boundary(at.saturating_sub(150));
                let end = line.ceil_char_boundary((at + 250).min(line.len()));
                line[start..end].to_owned()
            };
            panic!(
                "{} differs from its golden at line {}, byte {at}\n--- expected\n{}\n--- actual\n{}",
                self.name,
                first + 1,
                window(want),
                window(have)
            );
        }
    }
}

/// When this test process first asked: every run-generated reading falls
/// after it.
pub fn process_start() -> i64 {
    static START: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *START.get_or_init(now_us)
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

/// `text` with every run of exactly 64 lowercase hex digits blanked.
pub fn blank_hex64(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: &u8| b.is_ascii_digit() || (b'a'..=b'f').contains(b);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < bytes.len() {
        if hex(&bytes[at]) && (at == 0 || !hex(&bytes[at - 1])) {
            let run = bytes[at..].iter().take_while(|b| hex(b)).count();
            if run == 64 {
                out.push_str("<hex64>");
                at += 64;
                continue;
            }
        }
        let ch = text[at..].chars().next().unwrap();
        out.push(ch);
        at += ch.len_utf8();
    }
    out
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
            if us >= start - SLACK && us <= end.saturating_add(SLACK) {
                out.push_str(if us > now_us() + SLACK {
                    "<later>"
                } else {
                    "<now>"
                });
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

/// Replace every `prefix` followed by `len` lowercase hex digits with
/// `prefix#n`, numbered by first appearance.
pub fn number_hex(text: &str, prefix: &str, len: usize) -> String {
    let mut seen: Vec<String> = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(prefix) {
        out.push_str(&rest[..at]);
        let after = &rest[at + prefix.len()..];
        let hex = after.get(..len).filter(|h| {
            h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        });
        match hex {
            Some(hex) => {
                let number = match seen.iter().position(|s| s == hex) {
                    Some(n) => n,
                    None => {
                        seen.push(hex.to_owned());
                        seen.len() - 1
                    }
                };
                let _ = write!(out, "{prefix}#{number}");
                rest = &after[len..];
            }
            None => {
                out.push_str(prefix);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Replace the digits after each `"key":` (or `"key": `) with `<n>`.
pub fn mask_number(text: &str, key: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let needle = format!("\"{key}\":");
    while let Some(at) = rest.find(&needle) {
        out.push_str(&rest[..at + needle.len()]);
        let mut after = &rest[at + needle.len()..];
        if let Some(stripped) = after.strip_prefix(' ') {
            out.push(' ');
            after = stripped;
        }
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            out.push_str("<n>");
        }
        rest = &after[digits..];
    }
    out.push_str(rest);
    out
}

/// Lines longer than this are kept as their length, digest and head.
pub const LONG_LINE: usize = 1200;

/// FNV-1a, 128 bits: a stable fingerprint of a long line.
pub fn fnv128(bytes: &[u8]) -> u128 {
    let mut hash: u128 = 0x6c62272e07bb014262b821756295c58d;
    for byte in bytes {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(0x0000000001000000000000000000013B);
    }
    hash
}

/// Each line over `LONG_LINE` bytes becomes `<head>… [<len> bytes fnv128
/// <digest>]`, so large payloads pin every byte without filling the golden.
pub fn digest_long_lines(text: &str, limit: usize, head: usize) -> String {
    let mut out = String::with_capacity(text.len().min(1 << 20));
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        if body.len() > limit {
            let head = &body[..body.floor_char_boundary(head)];
            let _ = write!(
                out,
                "{head}… [{} bytes fnv128 {:032x}]",
                body.len(),
                fnv128(body.as_bytes())
            );
            if line.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Blocks (a command's stdout or stderr, a section) larger than this keep
/// their first lines and a digest of the rest.
pub const LARGE_BLOCK: usize = 2500;
const BLOCK_HEAD: usize = 12;

fn starts_entry(line: &str) -> bool {
    ["$ ", "exit ", "--- ", "=== ", "# "]
        .iter()
        .any(|p| line.starts_with(p))
}

/// Every block over `LARGE_BLOCK` bytes becomes its first lines and
/// `… [<len> bytes, <n> lines fnv128 <digest>]` over the whole block.
pub fn digest_blocks(text: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len().min(1 << 20));
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at];
        out.push_str(line);
        at += 1;
        if !(line.starts_with("--- ") || line.starts_with("=== ")) {
            continue;
        }
        let end = (at..lines.len())
            .find(|&i| starts_entry(lines[i]))
            .unwrap_or(lines.len());
        let block: String = lines[at..end].concat();
        if block.len() > LARGE_BLOCK {
            for kept in &lines[at..at + BLOCK_HEAD.min(end - at)] {
                out.push_str(kept);
            }
            let _ = writeln!(
                out,
                "… [{} bytes, {} lines fnv128 {:032x}]",
                block.len(),
                end - at,
                fnv128(block.as_bytes())
            );
        } else {
            out.push_str(&block);
        }
        at = end;
    }
    out
}

/// Replace the `len` characters after each `prefix` with `placeholder`.
pub fn mask_after(text: &str, prefix: &str, len: usize, placeholder: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(prefix) {
        let end = at + prefix.len();
        out.push_str(&rest[..end]);
        let after = &rest[end..];
        match after.char_indices().nth(len) {
            Some((cut, _)) => {
                out.push_str(placeholder);
                rest = &after[cut..];
            }
            None => {
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Replace what follows each `prefix` up to (not including) the next
/// `terminator` with `placeholder`, when the terminator comes within 40
/// characters.
pub fn mask_until(text: &str, prefix: &str, terminator: &str, placeholder: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(prefix) {
        let end = at + prefix.len();
        out.push_str(&rest[..end]);
        rest = &rest[end..];
        if let Some(stop) = rest.find(terminator).filter(|stop| *stop <= 40) {
            out.push_str(placeholder);
            rest = &rest[stop..];
        }
    }
    out.push_str(rest);
    out
}

/// A daemon driven one request at a time, each built from earlier replies.
pub struct Live {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: std::io::BufReader<std::process::ChildStdout>,
    pub records: Vec<serde_json::Value>,
}

impl Live {
    pub fn start(sandbox: &Sandbox) -> Self {
        use std::io::BufRead as _;
        let root = sandbox.root.to_string_lossy().into_owned();
        let mut command = sandbox.command();
        command
            .args(["--machine", "--data-root", &root, "daemon"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("the daemon starts");
        let stdin = child.stdin.take();
        let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut ready = String::new();
        stdout.read_line(&mut ready).unwrap();
        assert!(ready.contains("\"daemon.ready\""), "{ready}");
        Self {
            child,
            stdin,
            stdout,
            records: Vec::new(),
        }
    }

    /// Send one request (its `request_id` numbered from 1) and record the
    /// reply in `t`.
    pub fn send(
        &mut self,
        t: &mut Transcript,
        sandbox: &Sandbox,
        mut request: serde_json::Value,
    ) -> serde_json::Value {
        use std::io::BufRead as _;
        request["request_id"] = serde_json::json!((self.records.len() + 1).to_string());
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(request.to_string().as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let line = line.trim_end_matches('\n');
        let _ = writeln!(t.text, "> {}", sandbox.scrub(&request.to_string()));
        let _ = writeln!(t.text, "< {}", sandbox.scrub(line));
        let record: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("reply is not JSON ({e}): {line}"));
        self.records.push(record.clone());
        record
    }

    /// Write one request line as is.
    pub fn write(&mut self, request: &serde_json::Value) {
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(request.to_string().as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    /// Read the next line the daemon prints.
    pub fn read(&mut self) -> String {
        use std::io::BufRead as _;
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).unwrap();
        assert!(read > 0, "the daemon closed its output");
        line.trim_end_matches('\n').to_owned()
    }

    /// Close stdin; the daemon must exit 0 with nothing on stderr.
    pub fn finish(mut self) {
        use std::io::Read as _;
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            pipe.read_to_string(&mut stderr).unwrap();
        }
        assert_eq!((status.code(), stderr.as_str()), (Some(0), ""));
    }
}

/// Reply lines of a balance history read: its rows run to today, so their
/// periods and counts move with the calendar. The line keeps only that it
/// was such a reply.
pub fn calendar_relative(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let reply = line.starts_with("< ");
        if reply
            && (line.contains("\"kind\":\"ui.reports.balance_history\"")
                || line.contains("\\\"kind\\\":\\\"ui.reports.balance_history\\\""))
        {
            out.push_str("< <a balance history reply; its rows run to today>\n");
        } else {
            out.push_str(line);
        }
    }
    out
}

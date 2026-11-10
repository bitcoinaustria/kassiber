//! Desktop daemon sessions against frozen goldens.
//!
//! These tests replaced pytest modules that compared the Rust daemon with the
//! Python reference byte for byte. Each `Desk` is a disposable book seeded
//! with SQL through the store; `session` sends one JSONL session to
//! `kassiber daemon`, requires a clean exit with empty stderr, and records the
//! response lines in the test's golden. A read must leave every table
//! unchanged; a mutating session records which rows each table gained or
//! lost. The goldens were written from the Rust daemon where it agreed with
//! Python, so a golden mismatch is a behaviour change.
//!
//! `KASSIBER_GOLDEN_WRITE=1` rewrites a golden from the current output
//! instead of comparing; it is refused when `CI` is set.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use kassiber_pyjson::Value;
use kassiber_store::rusqlite::types::ValueRef;
use kassiber_store::rusqlite::Connection;
use kassiber_store::{Book, OpenOptions};

/// The desktop fixture the Python harness seeded every book with.
pub const DESKTOP_SQL: &str = include_str!("../../../transactions/tests/data/desktop.sql");

/// Rewrites one row's formatted values: (table, column names, values).
pub type TableMask = fn(&str, &[String], &mut [String]);

/// The journal tests' fixed clock.
pub const JOURNAL_CLOCK: &str = "2026-10-31T12:00:00Z";

pub fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Responses longer than this are kept as a digest, their length and head.
const LONG_LINE: usize = 16 * 1024;

/// FNV-1a, 128 bits: a stable digest for long golden lines.
pub fn fnv1a128(bytes: &[u8]) -> u128 {
    let mut hash: u128 = 0x6c62272e07bb014262b821756295c58d;
    for byte in bytes {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(0x0000000001000000000000000000013b);
    }
    hash
}

/// The test's golden: everything a test observed, compared as text.
pub struct Golden {
    name: String,
    text: String,
    lines: usize,
    /// A response seen before (with its request id blanked) to its line.
    seen: BTreeMap<String, usize>,
}

impl Golden {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            text: String::new(),
            lines: 0,
            seen: BTreeMap::new(),
        }
    }

    pub fn path(&self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/sessions")
            .join(format!("{}.txt", self.name))
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line(&mut self, line: &str) {
        self.text.push_str(line);
        self.text.push('\n');
        self.lines += line.matches('\n').count() + 1;
    }

    /// A response line; one equal to an earlier response but for its
    /// `request_id` is written as a reference to that line.
    pub fn response(&mut self, response: &str, request_id: Option<&str>) {
        let key = match request_id {
            Some(id) => response.replace(&format!("\"request_id\":\"{id}\""), "\"request_id\":?"),
            None => response.to_owned(),
        };
        match self.seen.get(&key) {
            Some(line) => {
                let line = *line;
                self.line(&format!("< same as line {line}"));
            }
            None => {
                self.seen.insert(key, self.lines + 1);
                if response.len() > LONG_LINE {
                    let head: String = response.chars().take(240).collect();
                    self.line(&format!(
                        "< fnv1a128:{:032x} bytes={} head={head}",
                        fnv1a128(response.as_bytes()),
                        response.len()
                    ));
                } else {
                    self.line(&format!("< {response}"));
                }
            }
        }
    }

    /// Compare with the checked-in golden (or rewrite it, see the module).
    pub fn check(&self) {
        let path = self.path();
        if std::env::var_os("KASSIBER_GOLDEN_WRITE").is_some_and(|v| v == "1") {
            assert!(
                std::env::var_os("CI").is_none(),
                "KASSIBER_GOLDEN_WRITE is refused in CI"
            );
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &self.text).unwrap();
            return;
        }
        let expected = fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e} (write it with KASSIBER_GOLDEN_WRITE=1)",
                path.display()
            )
        });
        if expected == self.text {
            return;
        }
        let (want, got): (Vec<&str>, Vec<&str>) =
            (expected.lines().collect(), self.text.lines().collect());
        let at = want
            .iter()
            .zip(&got)
            .position(|(a, b)| a != b)
            .unwrap_or(want.len().min(got.len()));
        let context = |lines: &[&str]| {
            lines[at.saturating_sub(3)..lines.len().min(at + 2)]
                .iter()
                .map(|l| format!("    {l}\n"))
                .collect::<String>()
        };
        panic!(
            "{} differs at line {} ({} expected lines, {} actual)\nexpected:\n{}actual:\n{}",
            path.display(),
            at + 1,
            want.len(),
            got.len(),
            context(&want),
            context(&got),
        );
    }
}

/// One disposable book under `TMPDIR`.
pub struct Desk {
    pub base: tempfile::TempDir,
    pub root: PathBuf,
    pub passphrase: Option<String>,
    pub env: BTreeMap<String, String>,
    pub golden: Golden,
    /// Run-specific text (ports, paths) and the placeholder the golden holds.
    pub replacements: Vec<(String, String)>,
    /// Applied to every recorded response line after `replacements`.
    pub line_mask: Option<fn(&str) -> String>,
    /// Mask generated identities and clocks in sessions and CLI runs.
    pub mask: Option<MaskSpec>,
    /// The last request ends the daemon session (an import): the shutdown
    /// request is still sent, and whatever answers it is recorded.
    pub ends_session: bool,
    /// Applied to every row (table, column names, formatted values) before
    /// tables are compared.
    pub table_mask: Option<TableMask>,
    seed: Option<String>,
    book: Option<String>,
    sessions: usize,
}

impl Desk {
    /// A book seeded with the desktop fixture.
    pub fn new(golden: &str) -> Self {
        Self::with_seed(golden, Some(DESKTOP_SQL))
    }

    pub fn with_seed(golden: &str, seed: Option<&str>) -> Self {
        let base = tempfile::Builder::new()
            .prefix("kassiber-sessions-")
            .tempdir()
            .unwrap();
        let path = base.path().canonicalize().unwrap();
        let mut env = BTreeMap::new();
        for (key, value) in [
            ("XDG_DATA_HOME", "xdg-data"),
            ("XDG_CONFIG_HOME", "xdg-config"),
            ("XDG_STATE_HOME", "xdg-state"),
            ("KASSIBER_STATE_ROOT", "state"),
            ("KASSIBER_RUST_TEST_OWNER_ROOT", "owners"),
            (
                "KASSIBER_OFFLINE_PREFERENCE_FILE",
                "preferences/offline.json",
            ),
            ("KASSIBER_AGENT_ACCESS_FILE", "preferences/agents.json"),
            (
                "KASSIBER_UPDATE_PREFERENCE_FILE",
                "preferences/updates.json",
            ),
            (
                "KASSIBER_UPDATE_CACHE_FILE",
                "preferences/update-cache.json",
            ),
        ] {
            env.insert(
                key.to_owned(),
                path.join(value).to_string_lossy().into_owned(),
            );
        }
        fs::create_dir_all(path.join("owners")).unwrap();
        fs::create_dir_all(path.join("preferences")).unwrap();
        for (key, value) in [
            ("COLUMNS", "80"),
            ("LINES", "24"),
            ("KASSIBER_DISABLE_UPDATE_CHECK", "1"),
            ("KASSIBER_TEST_NO_EGRESS", "1"),
            ("KASSIBER_NO_EGRESS", "1"),
        ] {
            env.insert(key.to_owned(), value.to_owned());
        }
        let mut desk = Self {
            root: path.join("data"),
            base,
            passphrase: None,
            env,
            golden: Golden::new(golden),
            // A version bump is no behaviour change.
            replacements: vec![(
                format!("\"version\":\"{}\"", env!("CARGO_PKG_VERSION")),
                "\"version\":\"$VERSION\"".to_owned(),
            )],
            line_mask: None,
            mask: None,
            ends_session: false,
            table_mask: None,
            seed: seed.map(str::to_owned),
            book: None,
            sessions: 0,
        };
        desk.create();
        desk
    }

    pub fn base_path(&self) -> PathBuf {
        self.base.path().canonicalize().unwrap()
    }

    fn create(&mut self) {
        Book::create(&self.root, self.passphrase.as_deref()).unwrap();
        if let Some(book) = self.book.clone() {
            self.load_book(&book);
        }
        if let Some(seed) = self.seed.clone() {
            self.change(&seed);
        }
    }

    /// Seed the book with `sql` now, and any book `encrypt` makes later.
    pub fn set_seed(&mut self, sql: &str) {
        self.seed = Some(sql.to_owned());
        self.change(sql);
    }

    /// A book holding the rows of a frozen Python-seeded book
    /// (`tests/data/books/<book>.sql`, see `tests/rust_port/freeze_session_books.py`).
    pub fn from_book(golden: &str, book: &str) -> Self {
        let mut desk = Self::with_seed(golden, None);
        desk.book = Some(book.to_owned());
        desk.load_book(book);
        desk
    }

    /// Replace every row with the frozen book's, triggers off, so the rows are
    /// exactly the ones Python left; then settle as `change` does.
    pub fn load_book(&self, book: &str) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/books")
            .join(format!("{book}.sql"));
        let rows = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let conn = self.connection();
        conn.set_db_config(
            kassiber_store::rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
            false,
        )
        .unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF; BEGIN")
            .unwrap();
        let names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_stat%'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for name in names {
            conn.execute_batch(&format!("DELETE FROM \"{name}\""))
                .unwrap();
        }
        conn.execute_batch(&rows).unwrap();
        conn.execute_batch("COMMIT").unwrap();
        drop(conn);
        self.change("");
    }

    /// The journal tests' clock and identifiers (`tests/test_rust_journal_commands.py`).
    pub fn journal_env(&mut self) {
        self.env.remove("KASSIBER_NO_EGRESS");
        for (key, value) in [
            ("KASSIBER_RUST_TEST_JOURNAL_TIME", JOURNAL_CLOCK),
            ("KASSIBER_RUST_TEST_JOURNAL_IDS", "sequential"),
        ] {
            self.env.insert(key.to_owned(), value.to_owned());
        }
    }

    /// Move to a fresh SQLCipher book with the same seed.
    pub fn encrypt(&mut self, passphrase: &str) {
        self.root = self.base_path().join("encrypted/data");
        self.passphrase = Some(passphrase.to_owned());
        self.create();
    }

    /// Apply `sql`, then reopen once so open-time backfills settle before a
    /// baseline is taken (as the Python harness did).
    pub fn change(&self, sql: &str) {
        for script in [sql, ""] {
            let book = Book::open_migrating(
                &self.root,
                OpenOptions {
                    passphrase: self.passphrase.as_deref(),
                    ..OpenOptions::default()
                },
                true,
            )
            .unwrap();
            book.connection().execute_batch(script).unwrap();
            drop(book);
        }
    }

    pub fn connection(&self) -> Connection {
        let database = kassiber_paths::resolve_database_path(&self.root);
        kassiber_store::encryption::open_container(&database, self.passphrase.as_deref()).unwrap()
    }

    /// Every table's rows, each table's rows sorted.
    pub fn tables(&self) -> BTreeMap<String, Vec<String>> {
        self.rows(None)
    }

    fn rows(&self, mask: Option<TableMask>) -> BTreeMap<String, Vec<String>> {
        let conn = self.connection();
        let names: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut tables = BTreeMap::new();
        for name in names {
            let mut statement = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
            let columns: Vec<String> = statement
                .column_names()
                .iter()
                .map(|c| c.to_string())
                .collect();
            let mut rows = Vec::new();
            let mut query = statement.query([]).unwrap();
            while let Some(row) = query.next().unwrap() {
                let mut values: Vec<String> = (0..columns.len())
                    .map(|i| sql_value(row.get_ref(i).unwrap()))
                    .collect();
                if let Some(mask) = mask {
                    mask(&name, &columns, &mut values);
                }
                rows.push(format!("({})", values.join(", ")));
            }
            rows.sort();
            tables.insert(name, rows);
        }
        tables
    }

    /// The process environment for every run of the binary.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kassiber"));
        if let Some(passphrase) = &self.passphrase {
            // `--db-passphrase-fd 3`, opened by a shell around the binary.
            let file = self.base_path().join("passphrase");
            fs::write(&file, passphrase).unwrap();
            command = Command::new("sh");
            command
                .arg("-c")
                .arg("exec \"$0\" \"$@\" 3<\"$KASSIBER_SESSIONS_PASSPHRASE_FILE\"")
                .arg(env!("CARGO_BIN_EXE_kassiber"))
                .env("KASSIBER_SESSIONS_PASSPHRASE_FILE", file);
        }
        command.args(["--machine", "--data-root"]).arg(&self.root);
        if self.passphrase.is_some() {
            command.args(["--db-passphrase-fd", "3"]);
        }
        command.args(args);
        self.environment(&mut command);
        command
    }

    /// The binary with exactly `args`, in the desk's environment.
    pub fn raw_command<S: AsRef<std::ffi::OsStr>>(&self, args: &[S]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kassiber"));
        command.args(args);
        self.environment(&mut command);
        command
    }

    fn environment(&self, command: &mut Command) {
        for (key, _) in std::env::vars() {
            if key.starts_with("KASSIBER_") || key.starts_with("SATBOOKS_") {
                command.env_remove(key);
            }
        }
        command
            .current_dir(repository())
            .envs(&self.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    }

    pub fn run(&self, args: &[&str], input: &[u8]) -> Output {
        let mut child = self.command(args).spawn().expect("the binary starts");
        let mut stdin = child.stdin.take().unwrap();
        let input = input.to_vec();
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
        let output = child.wait_with_output().unwrap();
        writer.join().unwrap();
        output
    }

    /// The JSONL requests for `args` (raw JSON text; `None` omits `args`).
    pub fn requests(kind: &str, args: &[Option<&str>]) -> Vec<String> {
        args.iter()
            .enumerate()
            .map(|(index, arg)| match arg {
                Some(arg) => format!(
                    "{{\"request_id\": \"{index}\", \"kind\": \"{kind}\", \"args\": {arg}}}"
                ),
                None => format!("{{\"request_id\": \"{index}\", \"kind\": \"{kind}\"}}"),
            })
            .collect()
    }

    /// One daemon session of `kind` requests; see the module.
    pub fn session(&mut self, kind: &str, args: &[Option<&str>], mutates: bool) -> Vec<Value> {
        self.raw_session(kind, &Self::requests(kind, args), mutates)
    }

    pub fn raw_session(&mut self, kind: &str, requests: &[String], mutates: bool) -> Vec<Value> {
        let mut requests = requests.to_vec();
        let unlock = self.passphrase.is_some()
            && !requests
                .first()
                .is_some_and(|r| r.contains("\"daemon.unlock\""));
        if let Some(passphrase) = self.passphrase.as_ref().filter(|_| unlock) {
            requests.insert(
                0,
                format!(
                    "{{\"request_id\": \"unlock\", \"kind\": \"daemon.unlock\", \"args\": {{\"auth_response\": {{\"passphrase_secret\": \"{passphrase}\"}}}}}}"
                ),
            );
        }
        if !self.ends_session {
            requests.push("{\"request_id\": \"stop\", \"kind\": \"daemon.shutdown\"}".to_owned());
        }
        let before = self.masked_tables();
        let mut masker = self.mask.clone().map(Masker::new);
        let mut daemon = Daemon::spawn(self, &[]);
        daemon.require_kind(kind);
        self.sessions += 1;
        let number = self.sessions;
        self.golden.line(&format!("## session {number}"));
        let mut terminals = Vec::new();
        for request in &requests {
            let shown = self.replace_run_text(request);
            self.golden.line(&format!("> {shown}"));
            let id = kassiber_pytext::loads(request)
                .ok()
                .and_then(|r| path(&r, &["request_id"]).and_then(text).map(str::to_owned))
                .expect("every request names its request_id");
            daemon.send(request);
            let start = daemon.rows.len();
            let terminal = daemon.terminal(&id);
            for line in &daemon.rows[start..] {
                let mut line = self.normalize(line);
                if let Some(masker) = masker.as_mut() {
                    line = masker.text(&line);
                }
                self.golden.response(&line, Some(&id));
            }
            terminals.push(terminal);
        }
        if self.ends_session {
            daemon.try_send("{\"request_id\": \"stop\", \"kind\": \"daemon.shutdown\"}");
            let start = daemon.rows.len();
            daemon.drain();
            self.golden
                .line("> (after the session ended) daemon.shutdown");
            for line in &daemon.rows[start..] {
                let mut line = self.normalize(line);
                if let Some(masker) = masker.as_mut() {
                    line = masker.text(&line);
                }
                self.golden.response(&line, Some("stop"));
            }
            terminals.push(String::new());
        }
        daemon.exit_cleanly();
        self.record_tables(number, &before, mutates, masker.as_mut());
        let mut responses: Vec<Value> = terminals[..terminals.len() - 1]
            .iter()
            .map(|line| kassiber_pytext::loads(line).unwrap())
            .collect();
        if unlock {
            assert_eq!(kind_of(&responses[0]), "daemon.unlock");
            responses.remove(0);
        }
        responses
    }

    /// Every table's rows with `table_mask` applied.
    pub fn masked_tables(&self) -> BTreeMap<String, Vec<String>> {
        self.rows(self.table_mask)
    }

    /// A read must leave every table unchanged; a mutating session's
    /// changes go into the golden.
    pub fn record_tables(
        &mut self,
        number: usize,
        before: &BTreeMap<String, Vec<String>>,
        mutates: bool,
        masker: Option<&mut Masker>,
    ) {
        let after = self.masked_tables();
        let changes = match masker {
            Some(masker) => {
                let after = masker.tables(&after);
                table_changes(&masker.tables(before), &after)
            }
            None => table_changes(before, &after),
        };
        if mutates {
            self.golden
                .line(&format!("## tables after session {number}"));
            for line in changes {
                let line = self.replace_run_text(&line);
                self.golden.line(&line);
            }
        } else {
            assert!(changes.is_empty(), "a read wrote: {changes:#?}");
        }
    }

    /// Run each argv in order (`--machine --data-root ROOT` first); the exit
    /// status, stdout and stderr go to the golden, then the tables the batch
    /// changed. Returns each stdout's envelope.
    pub fn cli(&mut self, commands: &[Vec<String>], mutates: bool) -> Vec<Option<Value>> {
        let before = self.masked_tables();
        let mut masker = self.mask.clone().map(Masker::new);
        self.sessions += 1;
        let number = self.sessions;
        self.golden.line(&format!("## cli {number}"));
        let mut envelopes = Vec::new();
        for argv in commands {
            let args: Vec<&str> = argv.iter().map(String::as_str).collect();
            let output = self.run(&args, b"");
            let stdout = String::from_utf8(output.stdout).unwrap();
            let stderr = String::from_utf8(output.stderr).unwrap();
            let mut text = |raw: &str| {
                let line = self.replace_run_text(raw);
                match masker.as_mut() {
                    Some(masker) => masker.text(&line),
                    None => line,
                }
            };
            let shown: Vec<String> = argv.iter().map(|a| text(a)).collect();
            let out: Vec<String> = stdout.lines().map(&mut text).collect();
            let err: Vec<String> = stderr.lines().map(&mut text).collect();
            self.golden.line(&format!("$ {}", shown.join(" ")));
            self.golden
                .line(&format!("exit {:?}", output.status.code()));
            for line in out {
                self.golden.line(&format!("stdout {line}"));
            }
            for line in err {
                self.golden.line(&format!("stderr {line}"));
            }
            envelopes.push((!stdout.is_empty()).then(|| kassiber_pytext::loads(&stdout).unwrap()));
        }
        self.record_tables(number, &before, mutates, masker.as_mut());
        envelopes
    }

    /// Run one argv for setup, requiring success; returns its `data`. The
    /// journal clock and sequential identities make what it generates the
    /// same on every run.
    pub fn setup_cli(&self, argv: &[&str]) -> Value {
        let mut command = self.command(argv);
        command
            .env("KASSIBER_RUST_TEST_JOURNAL_IDS", "sequential")
            .env(
                "KASSIBER_RUST_TEST_JOURNAL_TIME",
                self.env
                    .get("KASSIBER_RUST_TEST_JOURNAL_TIME")
                    .map_or(JOURNAL_CLOCK, String::as_str),
            )
            .stdin(Stdio::null());
        let output = command.output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{argv:?}: {stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let envelope = kassiber_pytext::loads(&stdout).unwrap();
        path(&envelope, &["data"]).cloned().unwrap_or(Value::Null)
    }

    /// A scripted daemon session: `script` drives the daemon and returns
    /// whether its last request ended the session. Every line read goes to
    /// the golden through `transform`, then `normalize` (and the mask), then
    /// the tables as `raw_session` records them.
    pub fn scripted(
        &mut self,
        mutates: bool,
        transform: &dyn Fn(&str) -> String,
        script: impl FnOnce(&mut Daemon) -> bool,
    ) -> Vec<Value> {
        let before = self.masked_tables();
        let mut masker = self.mask.clone().map(Masker::new);
        let mut daemon = Daemon::spawn(self, &[]);
        if let Some(passphrase) = self.passphrase.clone() {
            daemon.send(&format!(
                "{{\"request_id\": \"unlock\", \"kind\": \"daemon.unlock\", \"args\": {{\"auth_response\": {{\"passphrase_secret\": \"{passphrase}\"}}}}}}"
            ));
            daemon.terminal("unlock");
        }
        let ended = script(&mut daemon);
        if ended {
            daemon.drain();
        } else {
            daemon.request("daemon.shutdown", "stop", None);
            daemon.terminal("stop");
        }
        daemon.exit_cleanly();
        self.sessions += 1;
        let number = self.sessions;
        self.golden.line(&format!("## session {number}"));
        let mut records = Vec::new();
        for line in &daemon.rows {
            let mut text = self.normalize(&transform(line));
            if let Some(masker) = masker.as_mut() {
                text = masker.text(&text);
            }
            let id = kassiber_pytext::loads(line).ok().and_then(|r| {
                path(&r, &["request_id"])
                    .and_then(text_of)
                    .map(str::to_owned)
            });
            self.golden.response(&text, id.as_deref());
            records.push(kassiber_pytext::loads(line).unwrap());
        }
        self.record_tables(number, &before, mutates, masker.as_mut());
        records
    }

    /// One unrecorded request; returns its terminal record.
    pub fn ask(&self, kind: &str, args: &str) -> Value {
        let mut daemon = Daemon::spawn(self, &[]);
        if let Some(passphrase) = &self.passphrase {
            daemon.send(&format!(
                "{{\"request_id\": \"unlock\", \"kind\": \"daemon.unlock\", \"args\": {{\"auth_response\": {{\"passphrase_secret\": \"{passphrase}\"}}}}}}"
            ));
            daemon.terminal("unlock");
        }
        daemon.request(kind, "ask", Some(args));
        let line = daemon.terminal("ask");
        daemon.request("daemon.shutdown", "stop", None);
        daemon.terminal("stop");
        daemon.exit_cleanly();
        kassiber_pytext::loads(&line).unwrap()
    }

    /// A session whose responses are not recorded: setup through the product.
    pub fn setup(&self, requests: &[(&str, &str)]) {
        let mut wire = String::new();
        if let Some(passphrase) = &self.passphrase {
            let _ = writeln!(
                wire,
                "{{\"request_id\": \"unlock\", \"kind\": \"daemon.unlock\", \"args\": {{\"auth_response\": {{\"passphrase_secret\": \"{passphrase}\"}}}}}}"
            );
        }
        for (index, (kind, args)) in requests.iter().enumerate() {
            let _ = writeln!(
                wire,
                "{{\"request_id\": \"{index}\", \"kind\": \"{kind}\", \"args\": {args}}}"
            );
        }
        wire.push_str("{\"kind\": \"daemon.shutdown\"}\n");
        let output = self.run(&["daemon"], wire.as_bytes());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(output.status.code(), Some(0), "{stdout}");
        for line in stdout.lines().skip(1) {
            let value = kassiber_pytext::loads(line).unwrap();
            assert_ne!(kind_of(&value), "error", "{line}");
        }
    }

    /// `text` with the run's base directory and `replacements` replaced.
    pub fn replace_run_text(&self, text: &str) -> String {
        let mut text = text.replace(&*self.base_path().to_string_lossy(), "$BASE");
        for (from, to) in &self.replacements {
            text = text.replace(from.as_str(), to);
        }
        text
    }

    /// A response line as the golden holds it: `replace_run_text`, then
    /// `line_mask`.
    pub fn normalize(&self, text: &str) -> String {
        let text = self.replace_run_text(text);
        match self.line_mask {
            Some(mask) => mask(&text),
            None => text,
        }
    }

    pub fn check(&self) {
        self.golden.check();
    }
}

fn sql_value(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".to_owned(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => format!("{f:?}"),
        ValueRef::Text(t) => format!("{:?}", String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => {
            let mut hex = String::from("x'");
            for byte in b {
                let _ = write!(hex, "{byte:02x}");
            }
            hex.push('\'');
            hex
        }
    }
}

/// `-`/`+` lines for rows each table lost or gained.
pub fn table_changes(
    before: &BTreeMap<String, Vec<String>>,
    after: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let mut lines = Vec::new();
    let names: std::collections::BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for name in names {
        let empty = Vec::new();
        let old = before.get(name).unwrap_or(&empty);
        let new = after.get(name).unwrap_or(&empty);
        let mut remaining: BTreeMap<&String, isize> = BTreeMap::new();
        for row in old {
            *remaining.entry(row).or_default() += 1;
        }
        for row in new {
            *remaining.entry(row).or_default() -= 1;
        }
        for (row, count) in remaining {
            for _ in 0..count.unsigned_abs() {
                lines.push(format!(
                    "{} {name} {row}",
                    if count > 0 { "-" } else { "+" }
                ));
            }
        }
    }
    lines
}

pub fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .try_fold(value, |value, key| kassiber_envelope::get(value, key))
}

fn text_of(value: &Value) -> Option<&str> {
    text(value)
}

pub fn text(value: &Value) -> Option<&str> {
    match value {
        Value::Str(text) => text.as_str(),
        _ => None,
    }
}

pub fn kind_of(response: &Value) -> &str {
    path(response, &["kind"]).and_then(text).unwrap_or_default()
}

pub fn list<'a>(value: &'a Value, keys: &[&str]) -> &'a [Value] {
    match path(value, keys) {
        Some(Value::List(items)) => items,
        other => panic!("{keys:?} is not a list: {other:?}"),
    }
}

/// One interactive `kassiber daemon` over pipes.
pub struct Daemon {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    lines: std::sync::mpsc::Receiver<String>,
    stderr: Option<std::thread::JoinHandle<String>>,
    /// The ready record.
    pub ready: String,
    /// Every line read after the ready record.
    pub rows: Vec<String>,
}

impl Daemon {
    /// Start `kassiber daemon` with `env` added to the desk's environment.
    pub fn spawn(desk: &Desk, env: &[(&str, &str)]) -> Self {
        let mut command = desk.command(&["daemon"]);
        command.envs(env.iter().copied());
        let mut child = command.spawn().expect("the daemon starts");
        let stdout = child.stdout.take().unwrap();
        let mut stderr_pipe = child.stderr.take().unwrap();
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut stderr_pipe, &mut text);
            text
        });
        let mut daemon = Self {
            stdin: child.stdin.take(),
            child,
            lines,
            stderr: Some(stderr),
            ready: String::new(),
            rows: Vec::new(),
        };
        daemon.ready = daemon.next_line().expect("daemon.ready");
        assert!(
            daemon.ready.contains("\"daemon.ready\""),
            "{}",
            daemon.ready
        );
        daemon
    }

    pub fn require_kind(&self, kind: &str) {
        let ready = kassiber_pytext::loads(&self.ready).unwrap();
        let Some(Value::List(kinds)) = path(&ready, &["data", "supported_kinds"]) else {
            panic!("no supported_kinds in {}", self.ready)
        };
        assert!(
            kinds.iter().any(|k| text(k) == Some(kind)),
            "{kind} is not advertised"
        );
    }

    fn next_line(&mut self) -> Option<String> {
        self.lines
            .recv_timeout(std::time::Duration::from_secs(300))
            .ok()
    }

    pub fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    /// Send unless the daemon already closed its stdin.
    pub fn try_send(&mut self, line: &str) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = stdin
                .write_all(format!("{line}\n").as_bytes())
                .and_then(|()| stdin.flush());
        }
    }

    /// Read every line until the daemon closes stdout.
    pub fn drain(&mut self) {
        self.close_stdin();
        while let Some(line) = self.next_line() {
            self.rows.push(line);
        }
    }

    /// Send `{"kind", "request_id", "args"}` (raw JSON `args`, omitted when `None`).
    pub fn request(&mut self, kind: &str, id: &str, args: Option<&str>) {
        let line = match args {
            Some(args) => {
                format!("{{\"kind\": \"{kind}\", \"request_id\": \"{id}\", \"args\": {args}}}")
            }
            None => format!("{{\"kind\": \"{kind}\", \"request_id\": \"{id}\"}}"),
        };
        self.send(&line);
    }

    /// The next line, recorded in `rows`.
    pub fn read(&mut self) -> String {
        let line = self
            .next_line()
            .unwrap_or_else(|| panic!("the daemon stopped answering after {:#?}", self.rows));
        self.rows.push(line.clone());
        line
    }

    /// Read until the response that ends request `id`: its own record that
    /// is neither progress nor an event.
    pub fn terminal(&mut self, id: &str) -> String {
        loop {
            let line = self.read();
            let row = kassiber_pytext::loads(&line).unwrap();
            if path(&row, &["request_id"]).and_then(text) == Some(id)
                && !kind_of(&row).ends_with(".progress")
                && path(&row, &["event"]) != Some(&Value::Bool(true))
            {
                return line;
            }
        }
    }

    /// Send one request and return its terminal record.
    pub fn step(&mut self, id: &str, kind: &str, args: Option<&str>) -> Value {
        self.request(kind, id, args);
        kassiber_pytext::loads(&self.terminal(id)).unwrap()
    }

    /// Read until a record of `id` whose kind is `kind` or `error`.
    pub fn until(&mut self, id: &str, kind: &str) -> Value {
        loop {
            let line = self.read();
            let row = kassiber_pytext::loads(&line).unwrap();
            if path(&row, &["request_id"]).and_then(text) == Some(id)
                && (kind_of(&row) == kind || kind_of(&row) == "error")
            {
                return row;
            }
        }
    }

    pub fn close_stdin(&mut self) {
        self.stdin.take();
    }

    /// Wait for the exit: (status code, stderr).
    pub fn wait(&mut self) -> (Option<i32>, String) {
        self.close_stdin();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() > deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("the daemon did not exit after its stdin closed");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let stderr = self
            .stderr
            .take()
            .map(|h| h.join().unwrap())
            .unwrap_or_default();
        (status.code(), stderr)
    }

    /// Drain the remaining lines, then require exit 0 and empty stderr.
    pub fn exit_cleanly(&mut self) {
        let (code, stderr) = self.wait();
        while let Ok(line) = self.lines.try_recv() {
            self.rows.push(line);
        }
        assert_eq!((code, stderr.as_str()), (Some(0), ""), "{:#?}", self.rows);
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Python's masks for clock-drawn values: every lowercase UUID becomes
/// `<uuid>`, then every `YYYY-MM-DDTHH:MM:SS[.f]Z` becomes `<now>`.
pub fn mask_uuid_and_now(text: &str) -> String {
    let uuid = |b: &[u8]| -> Option<usize> {
        let shape = [8, 4, 4, 4, 12];
        let mut at = 0;
        for (index, run) in shape.iter().enumerate() {
            if index > 0 {
                if b.get(at) != Some(&b'-') {
                    return None;
                }
                at += 1;
            }
            for _ in 0..*run {
                if !b
                    .get(at)
                    .is_some_and(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
                {
                    return None;
                }
                at += 1;
            }
        }
        Some(at)
    };
    let now = |b: &[u8]| -> Option<usize> {
        let shape = b"dddd-dd-ddTdd:dd:dd";
        for (index, want) in shape.iter().enumerate() {
            let got = *b.get(index)?;
            let ok = if *want == b'd' {
                got.is_ascii_digit()
            } else {
                got == *want
            };
            if !ok {
                return None;
            }
        }
        let mut at = shape.len();
        if b.get(at) == Some(&b'.') && b.get(at + 1).is_some_and(u8::is_ascii_digit) {
            at += 1;
            while b.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
        }
        (b.get(at) == Some(&b'Z')).then_some(at + 1)
    };
    let replace = |text: &str, matcher: &dyn Fn(&[u8]) -> Option<usize>, with: &str| {
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        let mut copied = 0;
        while at < bytes.len() {
            if let Some(length) = matcher(&bytes[at..]) {
                out.push_str(&text[copied..at]);
                out.push_str(with);
                at += length;
                copied = at;
            } else {
                at += 1;
            }
        }
        out.push_str(&text[copied..]);
        out
    };
    let text = replace(text, &uuid, "<uuid>");
    replace(&text, &now, "<now>")
}

/// What a `Masker` hides, after the Python harnesses' maskers
/// (`tests/test_rust_review_workflow.py::Masker`,
/// `tests/rust_port/sync_parity.py::SyncMasker`).
#[derive(Clone, Default)]
pub struct MaskSpec {
    /// Every UUID version (`SyncMasker`), not only uuid4 (`Masker`).
    pub any_version: bool,
    /// 64-hex values the run created become `<hash>`.
    pub hashes: bool,
    /// Identities and hashes the book held when masking began stay as stored.
    pub known: std::collections::BTreeSet<String>,
    /// Clocks at or after this instant (`YYYY-MM-DDTHH:MM:SS`) become `<now>`.
    pub since: String,
}

/// Generated identities to their order of first use, recent clocks to one token.
pub struct Masker {
    spec: MaskSpec,
    ids: BTreeMap<String, String>,
}

fn is_hex(c: u8) -> bool {
    c.is_ascii_digit() || (b'a'..=b'f').contains(&c)
}

fn hex_run(b: &[u8], at: usize, n: usize) -> bool {
    b.len() >= at + n && b[at..at + n].iter().all(|c| is_hex(*c))
}

/// A UUID at the start of `b`: `8-4-4-4-12` lowercase hex, the fourth group
/// starting `[89ab]`, and the third `4` unless `any_version`.
fn uuid_at(b: &[u8], any_version: bool) -> Option<usize> {
    let dash = |at: usize| b.get(at) == Some(&b'-');
    (hex_run(b, 0, 8)
        && dash(8)
        && hex_run(b, 9, 4)
        && dash(13)
        && hex_run(b, 14, 4)
        && (any_version || b[14] == b'4')
        && dash(18)
        && hex_run(b, 19, 4)
        && matches!(b[19], b'8' | b'9' | b'a' | b'b')
        && dash(23)
        && hex_run(b, 24, 12))
    .then_some(36)
}

/// `\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,6})?(?:Z|\+00:00)` at the start.
fn stamp_at(b: &[u8]) -> Option<usize> {
    let shape = b"dddd-dd-ddTdd:dd:dd";
    for (index, want) in shape.iter().enumerate() {
        let got = *b.get(index)?;
        if (*want == b'd' && !got.is_ascii_digit()) || (*want != b'd' && got != *want) {
            return None;
        }
    }
    let mut at = shape.len();
    if b.get(at) == Some(&b'.') {
        let digits = b[at + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if (1..=6).contains(&digits) {
            at += 1 + digits;
        }
    }
    if b.get(at) == Some(&b'Z') {
        Some(at + 1)
    } else if b[at..].starts_with(b"+00:00") {
        Some(at + 6)
    } else {
        None
    }
}

/// `YYYY-MM-DDTHH:MM:SS.ffffff` for a valid stamp, for ordering.
fn stamp_key(stamp: &str) -> Option<String> {
    let number = |range: std::ops::Range<usize>| stamp[range].parse::<u32>().ok();
    let (month, day, hour, minute, second) = (
        number(5..7)?,
        number(8..10)?,
        number(11..13)?,
        number(14..16)?,
        number(17..19)?,
    );
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let fraction: String = stamp[19..]
        .strip_prefix('.')
        .map(|rest| rest.chars().take_while(char::is_ascii_digit).collect())
        .unwrap_or_default();
    Some(format!("{}.{:0<6}", &stamp[..19], fraction))
}

/// Replace every leftmost, non-overlapping match of `matcher`.
fn substitute(
    text: &str,
    matcher: &dyn Fn(&[u8], usize) -> Option<usize>,
    with: &mut dyn FnMut(&str) -> String,
) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut at, mut copied) = (0, 0);
    while at < bytes.len() {
        match matcher(bytes, at) {
            Some(length) => {
                out.push_str(&text[copied..at]);
                out.push_str(&with(&text[at..at + length]));
                at += length;
                copied = at;
            }
            None => at += 1,
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// A 64-hex value with no hex digit on either side.
fn hash_at(b: &[u8], at: usize) -> Option<usize> {
    (hex_run(b, at, 64)
        && (at == 0 || !is_hex(b[at - 1]))
        && !b.get(at + 64).is_some_and(|c| is_hex(*c)))
    .then_some(64)
}

impl Masker {
    pub fn new(spec: MaskSpec) -> Self {
        Self {
            spec,
            ids: BTreeMap::new(),
        }
    }

    fn hashes(&self, text: &str) -> String {
        if !self.spec.hashes {
            return text.to_owned();
        }
        substitute(text, &hash_at, &mut |m| {
            if self.spec.known.contains(m) {
                m.to_owned()
            } else {
                "<hash>".to_owned()
            }
        })
    }

    fn clocks(&self, text: &str) -> String {
        substitute(
            text,
            &|b, at| stamp_at(&b[at..]),
            &mut |m| match stamp_key(m) {
                Some(key) if key >= format!("{}.000000", self.spec.since) => "<now>".to_owned(),
                _ => m.to_owned(),
            },
        )
    }

    /// A line or value with generated identities numbered by first use.
    pub fn text(&mut self, text: &str) -> String {
        let text = self.hashes(text);
        let any = self.spec.any_version;
        let known = &self.spec.known;
        let ids = &mut self.ids;
        let text = substitute(&text, &|b, at| uuid_at(&b[at..], any), &mut |m| {
            if known.contains(m) {
                return m.to_owned();
            }
            let next = format!("<uuid:{}>", ids.len());
            ids.entry(m.to_owned()).or_insert(next).clone()
        });
        self.clocks(&text)
    }

    /// Without numbering new identities: the order rows are mapped in.
    fn blank(&self, text: &str) -> String {
        let text = self.hashes(text);
        let text = substitute(
            &text,
            &|b, at| uuid_at(&b[at..], self.spec.any_version),
            &mut |m| {
                if self.spec.known.contains(m) {
                    m.to_owned()
                } else {
                    self.ids
                        .get(m)
                        .cloned()
                        .unwrap_or_else(|| "<uuid>".to_owned())
                }
            },
        );
        self.clocks(&text)
    }

    /// Tables in name order, rows in the order of their blanked text.
    pub fn tables(
        &mut self,
        tables: &BTreeMap<String, Vec<String>>,
    ) -> BTreeMap<String, Vec<String>> {
        let mut result = BTreeMap::new();
        for (name, rows) in tables {
            let mut keyed: Vec<(String, &String)> =
                rows.iter().map(|r| (self.blank(r), r)).collect();
            keyed.sort();
            let mut mapped: Vec<String> =
                keyed.into_iter().map(|(_, row)| self.text(row)).collect();
            mapped.sort();
            result.insert(name.clone(), mapped);
        }
        result
    }
}

/// Every UUID (any version) and 64-hex value in `tables`.
pub fn known_identities(
    tables: &BTreeMap<String, Vec<String>>,
) -> std::collections::BTreeSet<String> {
    let mut known = std::collections::BTreeSet::new();
    for row in tables.values().flatten() {
        let mut keep = |m: &str| {
            known.insert(m.to_owned());
            m.to_owned()
        };
        substitute(row, &|b, at| uuid_at(&b[at..], true), &mut keep);
        substitute(row, &hash_at, &mut keep);
    }
    known
}

/// Now minus one second, UTC, as `YYYY-MM-DDTHH:MM:SS` (the harnesses'
/// `started`).
pub fn started() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - 1;
    let (days, rest) = ((secs / 86400) as i64, secs % 86400);
    format!(
        "{}T{:02}:{:02}:{:02}",
        civil(days),
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The UTC date `offset` days from today, `YYYY-MM-DD`.
pub fn utc_date(offset: i64) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    civil((secs / 86400) as i64 + offset)
}

fn civil(days: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// `tests/test_rust_wallet_import_desktop.py::mask_line`: every
/// `8-4-4-4-12` hex identity becomes `<uuid>`, and every time stamped
/// yesterday, today or tomorrow (UTC) becomes `<now>`.
pub fn mask_import_line(text: &str) -> String {
    let uuid = |b: &[u8], at: usize| -> Option<usize> {
        let b = &b[at..];
        let dash = |i: usize| b.get(i) == Some(&b'-');
        (hex_run(b, 0, 8)
            && dash(8)
            && hex_run(b, 9, 4)
            && dash(13)
            && hex_run(b, 14, 4)
            && dash(18)
            && hex_run(b, 19, 4)
            && dash(23)
            && hex_run(b, 24, 12))
        .then_some(36)
    };
    let text = substitute(text, &uuid, &mut |_| "<uuid>".to_owned());
    let dates: Vec<String> = (-1..=1).map(utc_date).collect();
    let time = |b: &[u8], at: usize| -> Option<usize> {
        let b = &b[at..];
        if !dates.iter().any(|d| b.starts_with(d.as_bytes()))
            || !matches!(b.get(10), Some(b'T' | b' '))
        {
            return None;
        }
        let mut end = 11
            + b[11..]
                .iter()
                .take_while(|c| c.is_ascii_digit() || matches!(c, b':' | b'.'))
                .count();
        if end == 11 {
            return None;
        }
        if b.get(end) == Some(&b'Z') {
            end += 1;
        } else if matches!(b.get(end), Some(b'+' | b'-'))
            && b.len() >= end + 6
            && b[end + 1..end + 3].iter().all(u8::is_ascii_digit)
            && b[end + 3] == b':'
            && b[end + 4..end + 6].iter().all(u8::is_ascii_digit)
        {
            end += 6;
        }
        Some(end)
    };
    substitute(&text, &time, &mut |_| "<now>".to_owned())
}

/// `mask_tables` of the same module, as a `TableMask`.
pub fn mask_import_row(_table: &str, _columns: &[String], values: &mut [String]) {
    for value in values.iter_mut() {
        *value = mask_import_line(value);
    }
}

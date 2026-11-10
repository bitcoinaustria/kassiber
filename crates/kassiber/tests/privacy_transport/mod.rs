//! Replays the frozen Python reference in `tests/data/privacy_transport.json`
//! (written by `scripts/freeze_privacy_transport.py` before Python's removal)
//! against the `kassiber` binary.
//!
//! Each session builds its book the way the pytest helper `_book` did: a
//! fresh book, `book.sql` plus the `generic-FIFO-ready` export sequence, the
//! processed-projection stamps, one `--machine status`, then the session's
//! case SQL and one more compatibility open. Every step then runs the binary
//! with the helper's environment and must print exactly what Python printed,
//! leave every table untouched, and write the same `--output` bytes.
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use kassiber_store::rusqlite::{types::Value as Sql, Connection, OpenFlags};
use serde_json::Value;

const STAMP: &str = "2026-01-02T03:04:05Z";
const GOLDEN: &str = include_str!("../data/privacy_transport.json");

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn golden() -> Value {
    serde_json::from_str(GOLDEN).unwrap()
}

fn text<'a>(golden: &'a Value, key: &Value) -> &'a str {
    golden["blobs"][key.as_str().unwrap()].as_str().unwrap()
}

/// Every table, rows in a stable order: what `_dump` compared.
fn tables(database: &Path) -> BTreeMap<String, Vec<String>> {
    let conn = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut statement = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
            let count = statement.column_count();
            let mut rows: Vec<String> = statement
                .query_map([], |row| {
                    (0..count)
                        .map(|i| row.get::<_, Sql>(i))
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap()
                .map(|row| format!("{:?}", row.unwrap()))
                .collect();
            rows.sort();
            (name, rows)
        })
        .collect()
}

struct Session<'a> {
    record: &'a Value,
    root: PathBuf,
    data: PathBuf,
    database: PathBuf,
    home: String,
    _scratch: tempfile::TempDir,
}

impl<'a> Session<'a> {
    fn new(record: &'a Value) -> Self {
        let scratch = tempfile::tempdir().unwrap();
        // The daemon refuses symlinked data roots.
        let root = scratch.path().canonicalize().unwrap();
        let data = root.join("data");
        let database = data.join(kassiber_paths::DB_FILENAME);
        let home = std::env::var("HOME").unwrap_or_default();
        let session = Self {
            record,
            root,
            data,
            database,
            home,
            _scratch: scratch,
        };
        session.seed();
        session
    }

    fn reopen(&self) {
        kassiber_store::Book::open_migrating(&self.data, Default::default(), true)
            .unwrap()
            .close()
            .map_err(|failure| failure.error)
            .unwrap();
    }

    fn seed(&self) {
        let reports = repo().join("crates/reports/tests/data");
        kassiber_store::Book::open_migrating(&self.data, Default::default(), false)
            .unwrap()
            .close()
            .map_err(|failure| failure.error)
            .unwrap();
        let sequences: Value = serde_json::from_str(
            &std::fs::read_to_string(reports.join("export_sequences.json")).unwrap(),
        )
        .unwrap();
        let sequence = sequences
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == "generic-FIFO-ready")
            .unwrap();
        let book = std::fs::read_to_string(reports.join("book.sql")).unwrap();
        Connection::open(&self.database)
            .unwrap()
            .execute_batch(&(book + sequence["sql"].as_str().unwrap()))
            .unwrap();
        self.reopen();
        Connection::open(&self.database)
            .unwrap()
            .execute_batch(&format!(
                "UPDATE profiles SET last_processed_at='{STAMP}',last_processed_tx_count=6, \
                 last_processed_input_version=journal_input_version WHERE id='p'; \
                 UPDATE profiles SET journal_semantics_version={} WHERE id='p';",
                kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION
            ))
            .unwrap();
        std::fs::create_dir(self.root.join("owners")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                self.root.join("owners"),
                std::fs::Permissions::from_mode(0o700),
            )
            .unwrap();
        }
        let status = self.run(&["--machine".into(), "status".into()], None);
        assert_eq!(status.status.code(), Some(0), "{}", self.name());
        if let Some(case) = self.record["case"].as_str() {
            let file = self.record["case_file"].as_str().unwrap();
            let cases: Value =
                serde_json::from_str(&std::fs::read_to_string(reports.join(file)).unwrap())
                    .unwrap();
            let sql = cases
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == case)
                .unwrap()["sql"]
                .as_str()
                .unwrap()
                .to_owned();
            Connection::open(&self.database)
                .unwrap()
                .execute_batch(&sql)
                .unwrap();
            self.reopen();
        }
        if let Some(field) = self.record["corrupt_profile_field"].as_str() {
            Connection::open(&self.database)
                .unwrap()
                .execute(
                    &format!("UPDATE profiles SET {field}='bad' WHERE id='p'"),
                    [],
                )
                .unwrap();
        }
        for file in self.record["files"].as_array().unwrap() {
            let bytes: Vec<u8> = file["bytes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|byte| byte.as_u64().unwrap() as u8)
                .collect();
            std::fs::write(self.root.join(file["path"].as_str().unwrap()), bytes).unwrap();
        }
    }

    fn name(&self) -> &str {
        self.record["name"].as_str().unwrap()
    }

    fn local(&self, frozen: &str) -> String {
        frozen
            .replace("{root}", self.root.to_str().unwrap())
            .replace("{home}", &self.home)
    }

    fn frozen(&self, local: &[u8]) -> String {
        String::from_utf8(local.to_vec())
            .unwrap()
            .replace(self.root.to_str().unwrap(), "{root}")
            .replace(&self.home, "{home}")
    }

    /// `cli_runner.run(["--data-root", data, *args])` with `_book`'s environment.
    fn run(&self, args: &[String], stdin: Option<&[u8]>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kassiber"));
        command
            .arg("--data-root")
            .arg(&self.data)
            .args(args)
            .current_dir(repo())
            .env("KASSIBER_RUST_TEST_OWNER_ROOT", self.root.join("owners"))
            .env("KASSIBER_RUST_TEST_REPORT_TIME", STAMP)
            .env("KASSIBER_TEST_NO_EGRESS", "strict")
            .env("KASSIBER_DISABLE_UPDATE_CHECK", "1")
            .env(
                "KASSIBER_OFFLINE_PREFERENCE_FILE",
                self.root.join("offline.json"),
            )
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("xdg-data"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env_remove("COLUMNS")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if self.record["offline"] == Value::Bool(true) {
            command.env("KASSIBER_NO_EGRESS", "1");
        } else {
            command.env_remove("KASSIBER_NO_EGRESS");
        }
        command.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let mut child = command.spawn().unwrap();
        if let Some(bytes) = stdin {
            child.stdin.take().unwrap().write_all(bytes).unwrap();
        }
        child.wait_with_output().unwrap()
    }

    fn step(&self, golden: &Value, step: &Value, before: &BTreeMap<String, Vec<String>>) {
        let name = self.name();
        match step["mode"].as_str().unwrap() {
            "cli" => {
                let argv: Vec<String> = step["argv"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| self.local(a.as_str().unwrap()))
                    .collect();
                let output = step.get("output");
                if let Some(output) = output {
                    // The reference wrote the file first; the native run then
                    // met it (or the stale artifact the test planted) and had
                    // to leave exactly the reference's bytes.
                    let existing = match output["before"].as_str() {
                        Some(text) => text.to_owned(),
                        None => text(golden, &output["after"]).to_owned(),
                    };
                    std::fs::write(self.local(output["path"].as_str().unwrap()), existing).unwrap();
                }
                let actual = self.run(&argv, None);
                assert_eq!(
                    (
                        actual.status.code(),
                        self.frozen(&actual.stdout),
                        self.frozen(&actual.stderr)
                    ),
                    (
                        step["returncode"].as_i64().map(|code| code as i32),
                        text(golden, &step["stdout"]).to_owned(),
                        text(golden, &step["stderr"]).to_owned(),
                    ),
                    "{name}: {argv:?}"
                );
                if let Some(output) = output {
                    assert_eq!(
                        std::fs::read_to_string(self.local(output["path"].as_str().unwrap()))
                            .unwrap(),
                        text(golden, &output["after"]),
                        "{name}: {argv:?}"
                    );
                }
            }
            "daemon" => {
                let mut argv: Vec<String> = step["flags"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|flag| flag.as_str().unwrap().to_owned())
                    .collect();
                argv.push("daemon".into());
                let wire = self.local(text(golden, &step["stdin"]));
                let actual = self.run(&argv, Some(wire.as_bytes()));
                assert_eq!(actual.status.code(), Some(0), "{name}");
                assert_eq!(
                    self.frozen(&actual.stderr),
                    text(golden, &step["stderr"]),
                    "{name}"
                );
                let stdout = self.frozen(&actual.stdout);
                let (ready, rest) = stdout.split_once('\n').unwrap();
                let ready: Value = serde_json::from_str(ready).unwrap();
                assert_eq!(ready["kind"], "daemon.ready", "{name}");
                assert!(
                    ready["data"]["supported_kinds"]
                        .as_array()
                        .unwrap()
                        .contains(&step["kind"]),
                    "{name}"
                );
                assert_eq!(rest, text(golden, &step["stdout_after_ready"]), "{name}");
            }
            other => panic!("{name}: unknown step mode {other}"),
        }
        assert!(
            tables(&self.database) == *before,
            "{name}: the step changed the book"
        );
    }
}

/// Replay every frozen session of `group` whose name starts with `prefix`;
/// returns how many steps ran.
pub fn replay(group: &str, prefix: &str) -> usize {
    let golden = golden();
    let mut steps = 0;
    let mut sessions = 0;
    for record in golden[group].as_array().unwrap() {
        if !record["name"].as_str().unwrap().starts_with(prefix) {
            continue;
        }
        let session = Session::new(record);
        let before = tables(&session.database);
        let rows: BTreeMap<String, u64> = before
            .iter()
            .map(|(table, rows)| (table.clone(), rows.len() as u64))
            .collect();
        let frozen_rows: BTreeMap<String, u64> = record["table_rows"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(table, count)| (table.clone(), count.as_u64().unwrap()))
            .collect();
        assert_eq!(
            rows,
            frozen_rows,
            "{}: the seeded book differs",
            session.name()
        );
        for step in record["steps"].as_array().unwrap() {
            session.step(&golden, step, &before);
            steps += 1;
        }
        sessions += 1;
    }
    assert!(sessions > 0, "no frozen session starts with {prefix}");
    steps
}

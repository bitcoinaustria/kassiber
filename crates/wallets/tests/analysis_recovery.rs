//! A killed durable-mirror synchronize recovers: the index publication is
//! killed after every one of its writes (the `analysis_durable_fixture`
//! example's pause barrier), and the book must hold either the state before
//! the run or, once committed, the uninterrupted run's; a restart must then
//! reach the uninterrupted run's state. Replaces the plaintext `DELETE`
//! journal case of `tests/test_rust_analysis_recovery.py`, whose Python
//! reference agreed with the uninterrupted native run used here.

#![cfg(feature = "test-harness")]

#[path = "../examples/analysis_durable/crash.rs"]
mod crash;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;

/// The fixture runs in a child of this test binary: `fixture_child` below,
/// selected exactly, reads one request and answers as the
/// `analysis_durable_fixture` example does.
const CHILD: &str = "KASSIBER_ANALYSIS_RECOVERY_CHILD";

#[test]
fn fixture_child() {
    if std::env::var_os(CHILD).is_none() {
        return;
    }
    let mut raw = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut raw).unwrap();
    let result = crash::run(&kassiber_pytext::loads(&raw).unwrap());
    println!(
        "{}",
        kassiber_pyjson::dumps(
            &result,
            &kassiber_pyjson::Options::new().sort_keys(true).compact()
        )
        .unwrap()
    );
    std::io::stdout().flush().unwrap();
    std::process::exit(0);
}

fn fixture() -> PathBuf {
    std::env::current_exe().unwrap()
}

/// The first line of `reader` that is a JSON object (the test harness
/// prints its own header first).
fn json_line(reader: &mut impl BufRead) -> String {
    loop {
        let mut line = String::new();
        assert!(
            reader.read_line(&mut line).unwrap() > 0,
            "the fixture child printed nothing"
        );
        // libtest prints "test fixture_child ... " before the child's line.
        if let Some(at) = line.find("{\"") {
            return line[at..].trim_end().to_owned();
        }
    }
}

fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    kassiber_envelope::get(value, key).unwrap_or(&Value::Null)
}

fn text(value: &Value) -> &str {
    match value {
        Value::Str(text) => text.as_str().unwrap(),
        other => panic!("not text: {other:?}"),
    }
}

fn command(tmp: &Path, offline: bool) -> Command {
    let mut command = Command::new(fixture());
    command.args([
        "--exact",
        "fixture_child",
        "--nocapture",
        "--test-threads=1",
    ]);
    for (key, _) in std::env::vars() {
        if key.starts_with("KASSIBER_") {
            command.env_remove(key);
        }
    }
    command
        .env(CHILD, "1")
        .env("KASSIBER_TEST_NO_EGRESS", "strict")
        .env("XDG_DATA_HOME", tmp.join("xdg-data"))
        .env("KASSIBER_OFFLINE_PREFERENCE_FILE", tmp.join("offline.json"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if offline {
        command.env("KASSIBER_NO_EGRESS", "1");
    }
    command
}

/// One uninterrupted fixture run on `request`.
fn native(tmp: &Path, request: &str) -> Value {
    let mut child = command(tmp, false).spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let line = json_line(&mut output.stdout.as_slice());
    kassiber_pytext::loads(&line).unwrap()
}

/// Run until the fixture pauses after write `pause`, then SIGKILL it.
fn pause_and_kill(tmp: &Path, request: &str, pause: usize) {
    let mut child = command(tmp, pause % 2 == 1).spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.as_bytes())
        .unwrap();
    let line = json_line(&mut BufReader::new(child.stdout.take().unwrap()));
    assert_eq!(line, format!("{{\"paused\":{pause}}}"));
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert!(!status.success());
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
    assert_eq!(stderr, "");
}

fn request(database: &Path, action: &str, extra: &str) -> String {
    format!(
        "{{\"database\": {}, \"encrypted\": false, \"journal\": \"delete\"{action}{extra}}}",
        serde_json::to_string(&database.to_string_lossy()).unwrap()
    )
}

fn capture(tmp: &Path, database: &Path) -> Value {
    native(tmp, &request(database, "", ", \"capture\": true"))
}

/// A private directory under `TMPDIR`, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "kassiber-analysis-recovery-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.0.file_name().is_some_and(|n| {
            n.to_string_lossy()
                .starts_with("kassiber-analysis-recovery-")
        }) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[test]
fn native_mirror_every_write_kill_recovers_python_state() {
    let scratch = Scratch::new();
    let tmp = scratch.0.as_path();
    let corpus = kassiber_pytext::loads(include_str!("data/analysis_synchronize.json")).unwrap();
    let Value::List(cases) = corpus else {
        panic!("corpus")
    };
    let cold = cases
        .iter()
        .find(|c| text(get(c, "name")) == "cold-warm-rebuild")
        .unwrap()
        .clone();
    // name, warm, extra request fields
    let selected = [
        ("cold-warm-rebuild", "cold", false, ""),
        (
            "creator-output-invalidates-spender",
            "incremental",
            true,
            "",
        ),
        ("verified-tip-only", "tip", true, ""),
        (
            "dataset-half-open-expiry",
            "expiry",
            true,
            ", \"as_of\": \"2026-01-02T00:00:00Z\"",
        ),
        ("repair-dropped-node-table", "repair", true, ""),
        ("clock-without-dirty-sources", "watermark", true, ""),
        ("cold-warm-rebuild", "warm", true, ""),
        ("cold-warm-rebuild", "rebuild", true, ", \"rebuild\": true"),
    ];
    let mut checked = 0;
    let mut golden = String::new();
    for (case_name, name, warm, action) in selected {
        let case = if case_name == "cold-warm-rebuild" {
            &cold
        } else {
            cases
                .iter()
                .find(|c| text(get(c, "name")) == case_name)
                .unwrap()
        };
        let original = tmp.join(format!("{name}-original.sqlite"));
        let interrupted = tmp.join(format!("{name}-interrupted.sqlite"));
        {
            let conn = Connection::open(&original).unwrap();
            conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
            conn.execute_batch(include_str!(
                "../../reports/tests/data/privacy_hygiene_seed.sql"
            ))
            .unwrap();
            conn.execute_batch(&format!(
                "BEGIN;{}{}COMMIT;",
                include_str!("../../reports/tests/data/book.sql"),
                text(get(case, "sql"))
            ))
            .unwrap();
            conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL")
                .unwrap();
        }
        if warm {
            native(tmp, &request(&original, "", ""));
            let conn = Connection::open(&original).unwrap();
            let Value::List(actions) = get(case, "actions") else {
                panic!("actions")
            };
            for edit in &actions[1..actions.len().saturating_sub(2).max(1)] {
                if text(get(edit, "operation")) == "sql" {
                    conn.execute_batch(text(get(edit, "sql"))).unwrap();
                }
            }
            if name == "repair" {
                conn.execute_batch("DROP TRIGGER chain_index_track_transactions_update;")
                    .unwrap();
            }
        }
        let before = capture(tmp, &original);
        std::fs::copy(&original, &interrupted).unwrap();
        let request = request(&interrupted, action, "");
        let baseline = native(tmp, &request);
        let writes: usize = match get(&baseline, "writes") {
            Value::Int(n) => n.to_string().parse().unwrap(),
            other => panic!("writes: {other:?}"),
        };
        let canonical = |value: &Value| {
            kassiber_pyjson::dumps(
                value,
                &kassiber_pyjson::Options::new().sort_keys(true).compact(),
            )
            .unwrap()
        };
        golden.push_str(&format!(
            "{name} state {}\n{name} capture {}\n",
            canonical(get(&baseline, "state")),
            canonical(get(&baseline, "capture"))
        ));
        let Value::List(phases) = get(&baseline, "phases") else {
            panic!("phases")
        };
        assert_eq!(phases.len(), writes);
        assert_eq!(
            phases[writes - 2..].iter().map(text).collect::<Vec<_>>(),
            ["before_commit", "after_commit"]
        );
        for pause in 1..=writes {
            std::fs::copy(&original, &interrupted).unwrap();
            for suffix in ["-journal", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", interrupted.display()));
            }
            let mut paused = request.trim_end_matches('}').to_owned();
            paused.push_str(&format!(", \"pause\": {pause}}}"));
            pause_and_kill(tmp, &paused, pause);
            let killed = capture(tmp, &interrupted);
            let expected = if pause == writes {
                get(&baseline, "capture")
            } else {
                &before
            };
            assert_eq!(&killed, expected, "{name}: killed after write {pause}");
            let resumed = native(tmp, &request);
            assert_eq!(
                get(&resumed, "state"),
                get(&baseline, "state"),
                "{name} {pause}"
            );
            assert_eq!(
                get(&resumed, "capture"),
                get(&baseline, "capture"),
                "{name} {pause}"
            );
            checked += 1;
        }
        for suffix in ["", "-journal", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", interrupted.display()));
        }
    }
    assert!(checked > 0);
    // The uninterrupted runs as the Python reference computed them.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/analysis_recovery.txt");
    if std::env::var_os("KASSIBER_GOLDEN_WRITE").is_some_and(|v| v == "1") {
        assert!(
            std::env::var_os("CI").is_none(),
            "KASSIBER_GOLDEN_WRITE is refused in CI"
        );
        std::fs::write(&path, &golden).unwrap();
    } else {
        let expected = std::fs::read_to_string(&path).unwrap();
        for (want, got) in expected.lines().zip(golden.lines()) {
            assert_eq!(got, want);
        }
        assert_eq!(expected.lines().count(), golden.lines().count());
    }
}

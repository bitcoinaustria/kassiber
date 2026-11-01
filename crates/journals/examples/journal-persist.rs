//! Local R4-12 oracle/benchmark. Never registered, packaged or installed.
#[path = "../tests/support/mod.rs"]
mod support;
use kassiber_journals::storage::WriteObserver;
use kassiber_journals::{boundary, persistence, storage};
use kassiber_store::rusqlite::Connection;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

#[cfg(unix)]
fn cpu() -> f64 {
    use nix::sys::resource::{getrusage, UsageWho};
    let r = getrusage(UsageWho::RUSAGE_SELF).expect("process CPU clock");
    let u = r.user_time();
    let s = r.system_time();
    (u.tv_sec() + s.tv_sec()) as f64 + (u.tv_usec() + s.tv_usec()) as f64 / 1e6
}
#[cfg(not(unix))]
fn cpu() -> f64 {
    panic!("journal-persist CPU measurements require Unix")
}
struct BenchWrites {
    writes: support::Writes,
    phase: &'static str,
    wall: std::time::Instant,
    cpu: f64,
    phases: serde_json::Map<String, Value>,
}
impl BenchWrites {
    fn new() -> Self {
        Self {
            writes: support::Writes::default(),
            phase: "begin",
            wall: std::time::Instant::now(),
            cpu: cpu(),
            phases: serde_json::Map::new(),
        }
    }
}
impl storage::WriteObserver for BenchWrites {
    fn wrote(&mut self, table: &'static str) -> kassiber_store::rusqlite::Result<()> {
        self.writes.wrote(table)
    }
    fn phase(&mut self, phase: &'static str) {
        let current_cpu = cpu();
        self.phases.insert(
            self.phase.into(),
            json!({"wall":self.wall.elapsed().as_secs_f64(),"cpu":current_cpu-self.cpu}),
        );
        self.phase = phase;
        self.cpu = current_cpu;
        self.wall = std::time::Instant::now();
    }
}
fn replay_floor(c: &Connection, v: &Value) -> Result<(f64, f64), Box<dyn std::error::Error>> {
    use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql};
    let mut batches = Vec::new();
    for batch in v["floor"].as_array().ok_or("floor batches")? {
        let statement = c.prepare(batch[0].as_str().ok_or("floor SQL")?)?;
        let rows = batch[1]
            .as_array()
            .ok_or("floor rows")?
            .iter()
            .map(|r| {
                r.as_array()
                    .expect("bound row")
                    .iter()
                    .map(|v| match v {
                        Value::Null => Sql::Null,
                        Value::Bool(b) => Sql::Integer(i64::from(*b)),
                        Value::String(s) => Sql::Text(s.clone()),
                        Value::Number(n) if n.is_i64() => Sql::Integer(n.as_i64().unwrap()),
                        Value::Number(n) => Sql::Real(n.as_f64().unwrap()),
                        _ => panic!("floor scalar"),
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        batches.push((statement, rows));
    }
    let start_cpu = cpu();
    let start = std::time::Instant::now();
    for (statement, rows) in &mut batches {
        for row in rows {
            statement.execute(params_from_iter(row.iter()))?;
        }
    }
    c.execute_batch("RELEASE journals_process;COMMIT")?;
    Ok((start.elapsed().as_secs_f64(), cpu() - start_cpu))
}

fn publish_committed(
    output: &mut impl Write,
    reply: &Value,
    restore: impl FnOnce() -> kassiber_store::rusqlite::Result<()>,
    checkpoint: impl FnOnce() -> kassiber_store::rusqlite::Result<()>,
) -> io::Result<()> {
    // Once COMMIT succeeds, maintenance errors cannot turn it into a refusal.
    // Consuming the scopes still restores settings through their Drop paths.
    let _ = restore();
    writeln!(output, "{reply}")?;
    output.flush()?;
    let _ = checkpoint();
    Ok(())
}

fn report_failure(
    output: &mut impl Write,
    committed: bool,
    error: Box<dyn std::error::Error>,
) -> Result<(), Box<dyn std::error::Error>> {
    if committed {
        return Err(error);
    }
    writeln!(output, "{}", json!({"ok":false,"error":error.to_string()}))?;
    output.flush()?;
    Ok(())
}

fn run(
    v: &Value,
    output: &mut impl Write,
    committed: &mut bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let c = if let Some(path) = v["database"].as_str() {
        Connection::open(path)?
    } else {
        Connection::open_in_memory()?
    };
    if let Some(key) = v["passphrase"].as_str() {
        c.pragma_update(None, "key", key)?;
        c.execute_batch("PRAGMA journal_mode=DELETE;PRAGMA synchronous=FULL")?;
    } else {
        c.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL")?;
    }
    c.execute_batch("PRAGMA foreign_keys=ON;PRAGMA temp_store=MEMORY;PRAGMA cache_size=-16000")?;
    if v["passphrase"].is_null() {
        c.pragma_update(None, "mmap_size", 268435456)?;
    }
    if v["database"].is_null() {
        support::load_schema(&c, &v["before"], &v["schema"]);
    }
    let checkpoint = storage::DeferredCheckpoint::enter(&c)?;
    let cache = storage::RebuildCache::enter(&c)?;
    c.execute_batch("BEGIN IMMEDIATE;SAVEPOINT journals_process")?;
    if !v["floor"].is_null() {
        let (seconds, cpu_seconds) = replay_floor(&c, v)?;
        *committed = true;
        let reply = json!({"ok":v["expected"].is_null() || support::logical_equal(&support::snapshot(&c), &v["expected"]),"name":v["name"],"seconds":seconds,"cpu_seconds":cpu_seconds,"floor":true});
        publish_committed(
            output,
            &reply,
            || cache.restore(),
            || checkpoint.after_reply(),
        )?;
        return Ok(());
    }
    let r = &v["record"];
    let response = if r["response"].is_null() {
        boundary::empty_response()
    } else {
        boundary::ledger_response(&r["response"])?
    };
    let mut entry_ids = r["entry_ids"].as_array().into_iter().flatten();
    let mut state = boundary::build(
        &r["profile"],
        &r["custody"],
        r["wallets"].as_array().ok_or("wallets")?,
        &response,
        || {
            entry_ids
                .next()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into()
        },
    )?;
    // Reproduce explicit test edits at the persistence boundary. The separate
    // rendering campaign still compares the complete, unedited build state.
    if let Some(inputs) = v["entry_inputs"].as_array() {
        let mut rendered = std::mem::take(&mut state.entries)
            .into_iter()
            .map(|entry| (entry.id.clone(), entry))
            .collect::<std::collections::HashMap<_, _>>();
        for input in inputs {
            let mut entry = rendered
                .remove(input["id"].as_str().ok_or("entry id")?)
                .ok_or("entry missing from rendered state")?;
            if let Some(occurred_at) = input.get("occurred_at") {
                entry.occurred_at = occurred_at.clone();
            }
            if let Some(description) = input.get("description") {
                entry.description = description.as_str().ok_or("description")?.into();
            }
            state.entries.push(entry);
        }
    }
    let mut ids = v["ids"].as_array().ok_or("ids")?.iter();
    let mut identity = || {
        ids.next()
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| kassiber_journals::Refusal::from("UUID tape exhausted".to_owned()))
    };
    let start_cpu = cpu();
    let start = std::time::Instant::now();
    let mut writes = BenchWrites::new();
    let stored = persistence::store_ledger_state(
        &c,
        &r["profile"],
        &state,
        v["created_at"].as_str().ok_or("clock")?,
        &mut identity,
        &mut writes,
    )?;
    writes.phase("commit");
    c.execute_batch("RELEASE journals_process;COMMIT")?;
    *committed = true;
    writes.phase("committed");
    let seconds = start.elapsed().as_secs_f64();
    let cpu_seconds = cpu() - start_cpu;
    let verified = if v["expected"].is_null() {
        true
    } else {
        support::logical_equal(&support::snapshot(&c), &v["expected"])
    };
    let reply = json!({"ok":verified,"name":v["name"],"seconds":seconds,"cpu_seconds":cpu_seconds,"phases":writes.phases,"writes":writes.writes.count,"tables":writes.writes.tables,"stored":stored});
    let reply = if verified {
        reply
    } else {
        json!({"ok":false,"actual":support::snapshot(&c),"name":v["name"]})
    };
    publish_committed(
        output,
        &reply,
        || cache.restore(),
        || checkpoint.after_reply(),
    )?;
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A path per line amortizes interpreter/process overhead across a corpus.
    for line in io::stdin().lock().lines() {
        let mut committed = false;
        let result = (|| {
            let line = line?;
            let v: Value = serde_json::from_str(&std::fs::read_to_string(line)?)?;
            run(&v, &mut io::stdout().lock(), &mut committed)
        })();
        if let Err(e) = result {
            report_failure(&mut io::stdout().lock(), committed, e)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/r4-persist")
                .join(format!("{name}-{}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn connection(path: &std::path::Path, cipher: bool) -> Connection {
        let c = Connection::open(path).unwrap();
        if cipher {
            c.pragma_update(None, "key", "synthetic-test-only").unwrap();
            c.execute_batch("PRAGMA journal_mode=DELETE;PRAGMA synchronous=FULL")
                .unwrap();
        } else {
            c.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL")
                .unwrap();
        }
        c
    }
    struct Reply<'a> {
        bytes: Vec<u8>,
        reader: &'a Connection,
        flushed: &'a Cell<bool>,
        fail_flush: bool,
    }
    impl Write for Reply<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            assert_eq!(
                self.reader
                    .query_row("SELECT count(*) FROM witness", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                2
            );
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                return Err(io::Error::other("injected flush failure"));
            }
            self.flushed.set(true);
            Ok(())
        }
    }

    #[test]
    fn physical_success_is_visible_and_flushed_before_failing_post_commit_maintenance() {
        let scratch = Scratch::new("reply-maintenance");
        for cipher in [false, true] {
            let path = scratch.0.join(format!("{cipher}.sqlite3"));
            let c = connection(&path, cipher);
            c.execute_batch("CREATE TABLE witness(n);INSERT INTO witness VALUES(1);PRAGMA wal_autocheckpoint=73").unwrap();
            let reader = connection(&path, cipher);
            let checkpoint = storage::DeferredCheckpoint::enter(&c).unwrap();
            c.execute_batch("BEGIN IMMEDIATE;INSERT INTO witness VALUES(2);COMMIT")
                .unwrap();
            // An active statement on the writer makes its WAL checkpoint fail.
            let mut statement = c.prepare("SELECT * FROM witness").unwrap();
            let mut rows = statement.query([]).unwrap();
            assert!(rows.next().unwrap().is_some());
            let flushed = Cell::new(false);
            let checkpointed = Cell::new(false);
            let mut output = Reply {
                bytes: Vec::new(),
                reader: &reader,
                flushed: &flushed,
                fail_flush: false,
            };
            publish_committed(
                &mut output,
                &json!({"ok":true}),
                || Err(kassiber_store::rusqlite::Error::InvalidQuery),
                || {
                    assert!(flushed.get(), "checkpoint preceded the success flush");
                    checkpointed.set(true);
                    let result = checkpoint.after_reply();
                    assert_eq!(result.is_err(), !cipher);
                    Err(kassiber_store::rusqlite::Error::InvalidQuery)
                },
            )
            .unwrap();
            assert!(checkpointed.get());
            assert_eq!(output.bytes, b"{\"ok\":true}\n");
            assert_eq!(
                c.pragma_query_value(None, "wal_autocheckpoint", |r| r.get::<_, i64>(0))
                    .unwrap(),
                73
            );
            assert_eq!(
                reader
                    .query_row("SELECT count(*) FROM witness", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                2
            );
        }
    }

    #[test]
    fn physical_flush_failure_drops_checkpoint_without_publishing_a_second_response() {
        let scratch = Scratch::new("reply-flush");
        for cipher in [false, true] {
            let path = scratch.0.join(format!("{cipher}.sqlite3"));
            let c = connection(&path, cipher);
            c.execute_batch("CREATE TABLE witness(n);INSERT INTO witness VALUES(1);PRAGMA wal_autocheckpoint=73").unwrap();
            let reader = connection(&path, cipher);
            let checkpoint = storage::DeferredCheckpoint::enter(&c).unwrap();
            c.execute_batch("BEGIN IMMEDIATE;INSERT INTO witness VALUES(2);COMMIT")
                .unwrap();
            let flushed = Cell::new(false);
            let checkpointed = Cell::new(false);
            let mut output = Reply {
                bytes: Vec::new(),
                reader: &reader,
                flushed: &flushed,
                fail_flush: true,
            };
            let failure = publish_committed(
                &mut output,
                &json!({"ok":true}),
                || Ok(()),
                || {
                    checkpointed.set(true);
                    checkpoint.after_reply()
                },
            )
            .unwrap_err();
            assert!(!flushed.get());
            assert!(!checkpointed.get());
            let bytes = output.bytes.clone();
            assert!(report_failure(&mut output, true, Box::new(failure)).is_err());
            assert_eq!(output.bytes, bytes);
            assert_eq!(
                c.pragma_query_value(None, "wal_autocheckpoint", |r| r.get::<_, i64>(0))
                    .unwrap(),
                73
            );
            assert_eq!(
                reader
                    .query_row("SELECT count(*) FROM witness", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                2
            );
        }
        let mut output = Vec::new();
        report_failure(
            &mut output,
            false,
            io::Error::other("pre-COMMIT refusal").into(),
        )
        .unwrap();
        assert_eq!(output, b"{\"error\":\"pre-COMMIT refusal\",\"ok\":false}\n");
    }

    #[test]
    fn physical_probe_marks_both_commit_paths_before_a_failed_success_flush() {
        struct FailFlush(Vec<u8>);
        impl Write for FailFlush {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::other("injected success flush failure"))
            }
        }
        let fixtures: Vec<Value> =
            serde_json::from_str(include_str!("../tests/data/persistence.json")).unwrap();
        let scratch = Scratch::new("probe-commit-paths");
        for cipher in [false, true] {
            for floor in [false, true] {
                let path = scratch.0.join(format!("{cipher}-{floor}.sqlite3"));
                let mut input = fixtures[0].clone();
                let c = connection(&path, cipher);
                support::load(&c, &input["before"]);
                drop(c);
                input["database"] = json!(path);
                if cipher {
                    input["passphrase"] = json!("synthetic-test-only");
                }
                if floor {
                    input["floor"] = json!([[
                        "UPDATE profiles SET label='floor' WHERE id=?",
                        [[input["record"]["profile"]["id"]]]
                    ]]);
                    input["expected"] = Value::Null;
                }
                let mut committed = false;
                let mut output = FailFlush(Vec::new());
                let error = run(&input, &mut output, &mut committed).unwrap_err();
                assert!(committed, "cipher={cipher} floor={floor}");
                assert!(error.to_string().contains("injected success flush failure"));
                assert_eq!(output.0.iter().filter(|&&b| b == b'\n').count(), 1);
                let first = output.0.clone();
                assert!(report_failure(&mut output, committed, error).is_err());
                assert_eq!(output.0, first);
                let reopened = connection(&path, cipher);
                if floor {
                    assert_eq!(
                        reopened
                            .query_row(
                                "SELECT label FROM profiles WHERE id=?",
                                [input["record"]["profile"]["id"].as_str().unwrap()],
                                |r| r.get::<_, String>(0)
                            )
                            .unwrap(),
                        "floor"
                    );
                } else {
                    assert!(support::logical_equal(
                        &support::snapshot(&reopened),
                        &input["expected"]
                    ));
                }
            }
        }
    }
}

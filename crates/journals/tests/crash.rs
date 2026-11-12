mod support;
use kassiber_journals::storage::WriteObserver;
use kassiber_journals::{boundary, persistence, storage};
use kassiber_store::rusqlite::{self, Connection};
use serde_json::Value;
use std::io::{BufRead, Read, Write};
use std::process::{Command, Stdio};

struct Child(std::process::Child);
impl std::ops::Deref for Child {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for Child {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn cases() -> Vec<Value> {
    let mut cases: Vec<Value> =
        serde_json::from_str(include_str!("data/persistence.json")).unwrap();
    cases.extend(
        serde_json::from_str::<Vec<Value>>(include_str!("data/persistence-stages.json")).unwrap(),
    );
    cases
        .into_iter()
        .filter(|v| {
            v["name"] == "cipher"
                || v["name"] == "filed-change"
                || v["name"].as_str().unwrap().contains("retraction_inbox")
        })
        .collect()
}
fn connect(path: &std::path::Path, cipher: bool) -> Connection {
    let c = Connection::open(path).unwrap();
    if cipher {
        c.pragma_update(None, "key", "synthetic-test-only").unwrap();
        c.execute_batch("PRAGMA journal_mode=DELETE;PRAGMA synchronous=FULL")
            .unwrap();
    } else {
        c.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL")
            .unwrap();
    }
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    c
}
struct Handshake<'a>(&'a Connection);
impl storage::WriteObserver for Handshake<'_> {
    fn wrote(&mut self, _: &'static str) -> rusqlite::Result<()> {
        self.0.cache_flush()?;
        println!("r4-write");
        std::io::stdout().flush().unwrap();
        std::io::stdin().read_exact(&mut [0]).unwrap();
        Ok(())
    }
}
#[test]
fn persistence_crash_child() {
    let Ok(root) = std::env::var("KASSIBER_TEST_PERSIST_CRASH_ROOT") else {
        return;
    };
    let root = std::path::Path::new(&root);
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("case.json")).unwrap()).unwrap();
    let c = connect(
        &root.join("book.sqlite3"),
        std::env::var_os("KASSIBER_TEST_PERSIST_CRASH_CIPHER").is_some(),
    );
    let checkpoint = storage::DeferredCheckpoint::enter(&c).unwrap();
    let cache = storage::RebuildCache::enter(&c).unwrap();
    c.execute_batch("BEGIN IMMEDIATE;SAVEPOINT journals_process")
        .unwrap();
    let r = &v["record"];
    let response = if r["response"].is_null() {
        boundary::empty_response()
    } else {
        boundary::ledger_response(&r["response"]).unwrap()
    };
    let state = boundary::build(
        &r["profile"],
        &r["custody"],
        r["wallets"].as_array().unwrap(),
        &response,
        String::new,
    )
    .unwrap();
    let mut ids = v["ids"].as_array().unwrap().iter();
    let mut observer = Handshake(&c);
    persistence::store_ledger_state(
        &c,
        &r["profile"],
        &state,
        v["created_at"].as_str().unwrap(),
        &mut || Ok(ids.next().unwrap().as_str().unwrap().into()),
        &mut observer,
    )
    .unwrap();
    observer.wrote("before_commit").unwrap();
    c.execute_batch("RELEASE journals_process;COMMIT").unwrap();
    cache.restore().unwrap();
    println!("r4-committed");
    std::io::stdout().flush().unwrap();
    std::io::stdin().read_exact(&mut [0]).unwrap();
    checkpoint.after_reply().unwrap();
}
#[test]
fn killed_writes_recover_complete_stale_books_on_sqlite_and_sqlcipher() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/r4-persist")
        .join(format!("crash-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let run = || {
        let fixtures = cases();
        assert_eq!(fixtures.len(), 3);
        for cipher in [false, true] {
            for v in &fixtures {
                let base = root.join("base.sqlite3");
                let _ = std::fs::remove_file(&base);
                let c = connect(&base, cipher);
                support::load(&c, &v["before"]);
                let mut writes = support::Writes::default();
                c.execute_batch("BEGIN IMMEDIATE").unwrap();
                support::store(&c, v, &mut writes).unwrap();
                c.execute_batch("ROLLBACK").unwrap();
                drop(c);
                std::fs::write(root.join("case.json"), v.to_string()).unwrap();
                for kill_after in 1..=writes.count + 2 {
                    let book = root.join("book.sqlite3");
                    for suffix in ["", "-wal", "-shm", "-journal"] {
                        let _ = std::fs::remove_file(root.join(format!("book.sqlite3{suffix}")));
                    }
                    std::fs::copy(&base, &book).unwrap();
                    let mut command = Command::new(std::env::current_exe().unwrap());
                    command
                        .args([
                            "--exact",
                            "persistence_crash_child",
                            "--nocapture",
                            "--test-threads=1",
                        ])
                        .env("KASSIBER_TEST_PERSIST_CRASH_ROOT", &root)
                        .env_remove("KASSIBER_TEST_PERSIST_CRASH_CIPHER")
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped());
                    if cipher {
                        command.env("KASSIBER_TEST_PERSIST_CRASH_CIPHER", "1");
                    }
                    let mut child = Child(command.spawn().unwrap());
                    let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
                    let mut remaining = kill_after;
                    let committed = loop {
                        let mut line = String::new();
                        if output.read_line(&mut line).unwrap() == 0 {
                            let _ = child.wait();
                            let mut stderr = String::new();
                            child
                                .stderr
                                .as_mut()
                                .unwrap()
                                .read_to_string(&mut stderr)
                                .unwrap();
                            panic!("{} cipher={cipher} kill={kill_after}: {stderr}", v["name"]);
                        }
                        if line.contains("r4-committed") {
                            break true;
                        }
                        if line.contains("r4-write") {
                            remaining -= 1;
                            if remaining == 0 {
                                break false;
                            }
                            child.stdin.as_mut().unwrap().write_all(b"+").unwrap();
                        }
                    };
                    child.kill().unwrap();
                    let result = child.wait().unwrap();
                    assert!(!result.success());
                    let reopened = connect(&book, cipher);
                    let expected = if committed {
                        &v["expected"]
                    } else {
                        &v["before"]
                    };
                    assert!(
                        support::logical_equal(&support::snapshot(&reopened), expected),
                        "{} cipher={cipher} kill={kill_after}",
                        v["name"]
                    );
                    assert_eq!(
                        reopened
                            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                            .unwrap(),
                        "ok"
                    );
                    assert!(reopened
                        .prepare("PRAGMA foreign_key_check")
                        .unwrap()
                        .query([])
                        .unwrap()
                        .next()
                        .unwrap()
                        .is_none());
                }
            }
        }
    };
    let result = std::panic::catch_unwind(run);
    std::fs::remove_dir_all(root).unwrap();
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

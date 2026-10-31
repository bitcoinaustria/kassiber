//! Local R4-12 oracle/benchmark. Never registered, packaged or installed.
#[path = "../tests/support/mod.rs"]
mod support;
use kassiber_journals::{boundary, persistence, storage};
use kassiber_store::rusqlite::Connection;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

fn replay_floor(c: &Connection, v: &Value) -> Result<f64, Box<dyn std::error::Error>> {
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
    let start = std::time::Instant::now();
    for (statement, rows) in &mut batches {
        for row in rows {
            statement.execute(params_from_iter(row.iter()))?;
        }
    }
    c.execute_batch("RELEASE journals_process;COMMIT")?;
    Ok(start.elapsed().as_secs_f64())
}

fn run(v: &Value) -> Result<(), Box<dyn std::error::Error>> {
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
        support::load(&c, &v["before"]);
    }
    let checkpoint = storage::DeferredCheckpoint::enter(&c)?;
    let cache = storage::RebuildCache::enter(&c)?;
    c.execute_batch("BEGIN IMMEDIATE;SAVEPOINT journals_process")?;
    if !v["floor"].is_null() {
        let seconds = replay_floor(&c, v)?;
        cache.restore()?;
        println!(
            "{}",
            json!({"ok":true,"name":v["name"],"seconds":seconds,"floor":true})
        );
        io::stdout().flush()?;
        checkpoint.after_reply()?;
        return Ok(());
    }
    let r = &v["record"];
    let response = if r["response"].is_null() {
        boundary::empty_response()
    } else {
        boundary::ledger_response(&r["response"])?
    };
    let state = boundary::build(
        &r["profile"],
        &r["custody"],
        r["wallets"].as_array().ok_or("wallets")?,
        &response,
        String::new,
    )?;
    let mut ids = v["ids"].as_array().ok_or("ids")?.iter();
    let mut identity = || {
        ids.next()
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| kassiber_journals::Refusal::from("UUID tape exhausted".to_owned()))
    };
    let start = std::time::Instant::now();
    let mut writes = support::Writes::default();
    let stored = persistence::store_ledger_state(
        &c,
        &r["profile"],
        &state,
        v["created_at"].as_str().ok_or("clock")?,
        &mut identity,
        &mut writes,
    )?;
    c.execute_batch("RELEASE journals_process;COMMIT")?;
    let seconds = start.elapsed().as_secs_f64();
    cache.restore()?;
    let verified = if v["expected"].is_null() {
        true
    } else {
        support::logical_equal(&support::snapshot(&c), &v["expected"])
    };
    let reply = json!({"ok":verified,"name":v["name"],"seconds":seconds,"writes":writes.count,"tables":writes.tables,"stored":stored});
    let reply = if verified {
        reply
    } else {
        json!({"ok":false,"actual":support::snapshot(&c),"name":v["name"]})
    };
    println!("{reply}");
    io::stdout().flush()?;
    checkpoint.after_reply()?;
    Ok(())
}
fn main() {
    // A path per line amortizes interpreter/process overhead across a corpus.
    for line in io::stdin().lock().lines() {
        let result = (|| {
            let line = line?;
            let v: Value = serde_json::from_str(&std::fs::read_to_string(line)?)?;
            run(&v)
        })();
        if let Err(e) = result {
            println!("{}", json!({"ok":false,"error":e.to_string()}));
            io::stdout().flush().expect("oracle stdout");
        }
    }
}

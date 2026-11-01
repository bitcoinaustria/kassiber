use kassiber_journals::storage::{
    DeferredCheckpoint, RebuildCache, ENTRY_COLUMNS, POSTING_COLUMNS,
};
use kassiber_store::rusqlite::{types::ValueRef, Connection, Statement};
use serde::Serialize;
use serde_json::json;
use std::hint::black_box;

#[derive(Debug, Default, PartialEq, Serialize)]
struct Touched {
    rows: usize,
    cells: usize,
    types: [usize; 5],
    bytes: usize,
}

fn touch(
    statement: &mut Statement<'_>,
    profile: &str,
) -> kassiber_store::rusqlite::Result<Touched> {
    let columns = statement.column_count();
    let mut touched = Touched::default();
    let mut rows = statement.query([profile])?;
    while let Some(row) = rows.next()? {
        touched.rows += 1;
        for index in 0..columns {
            touched.cells += 1;
            match black_box(row.get_ref(index)?) {
                ValueRef::Null => touched.types[0] += 1,
                ValueRef::Integer(n) => {
                    black_box(n);
                    touched.types[1] += 1;
                }
                ValueRef::Real(n) => {
                    black_box(n);
                    touched.types[2] += 1;
                }
                ValueRef::Text(s) => {
                    black_box(s);
                    touched.types[3] += 1;
                    touched.bytes += s.len();
                }
                ValueRef::Blob(s) => {
                    black_box(s);
                    touched.types[4] += 1;
                    touched.bytes += s.len();
                }
            }
        }
    }
    Ok(black_box(touched))
}

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
    panic!("journal-read-floor CPU measurements require Unix")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
        std::env::args().nth(1).ok_or("request path")?,
    )?)?;
    let path = std::path::Path::new(request["database"].as_str().ok_or("database path")?);
    let target = std::path::Path::new("target").canonicalize()?;
    if !path.canonicalize()?.starts_with(target) {
        return Err("disposable database must be under worktree target/".into());
    }
    let mut trials = Vec::new();
    for run in 0..5 {
        let c = Connection::open(path)?;
        if let Some(key) = request["passphrase"].as_str() {
            c.pragma_update(None, "key", key)?;
            c.execute_batch("PRAGMA journal_mode=DELETE;PRAGMA synchronous=FULL")?;
        } else {
            c.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL")?;
            c.pragma_update(None, "mmap_size", 268435456)?;
        }
        c.execute_batch(
            "PRAGMA foreign_keys=ON;PRAGMA temp_store=MEMORY;PRAGMA cache_size=-16000",
        )?;
        let profile: String = c.query_row("SELECT id FROM profiles LIMIT 1", [], |r| r.get(0))?;
        let checkpoint = DeferredCheckpoint::enter(&c)?;
        let cache = RebuildCache::enter(&c)?;
        let mut entries = c.prepare(&format!(
            "SELECT {} FROM journal_entries WHERE profile_id=?",
            ENTRY_COLUMNS.join(",")
        ))?;
        let mut postings = c.prepare(&format!(
            "SELECT {} FROM journal_quantity_postings WHERE profile_id=?",
            POSTING_COLUMNS.join(",")
        ))?;
        c.execute_batch("BEGIN IMMEDIATE;SAVEPOINT journals_process")?;
        let load = std::fs::read_to_string("/proc/loadavg")?;
        let start_cpu = cpu();
        let start = std::time::Instant::now();
        let entry_touch = touch(&mut entries, &profile)?;
        let posting_touch = touch(&mut postings, &profile)?;
        c.execute_batch("RELEASE journals_process;COMMIT")?;
        let seconds = start.elapsed().as_secs_f64();
        let cpu_seconds = cpu() - start_cpu;
        cache.restore()?;
        checkpoint.after_reply()?;
        trials.push(json!({"run":run,"loadavg":load.trim(),"seconds":seconds,
            "cpu_seconds":cpu_seconds,"entries":entry_touch,"postings":posting_touch}));
    }
    println!(
        "{}",
        json!({"database":path,"fresh_connection_per_trial":true,"trials":trials})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touches_every_column_and_scalar_type_in_only_the_selected_profile() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE retained(profile_id, a, b, c, d, e);INSERT INTO retained VALUES('p',NULL,7,1.25,'abc',x'0001'),('other',NULL,0,0,'',x'')").unwrap();
        let mut statement = c
            .prepare("SELECT a,b,c,d,e FROM retained WHERE profile_id=?")
            .unwrap();
        assert_eq!(
            touch(&mut statement, "p").unwrap(),
            Touched {
                rows: 1,
                cells: 5,
                types: [1; 5],
                bytes: 5,
            }
        );
    }
}

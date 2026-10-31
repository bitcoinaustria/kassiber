//! R4-13 prepared-book oracle. Never a product command or packaged binary.
#[path = "../tests/support/mod.rs"]
#[allow(dead_code)]
mod support;
use kassiber_journals::{
    prebuild::{auto_price_transactions_from_rates_cache, PricingScope},
    process::{self, Prebuild, Repairs},
    storage::WriteObserver,
    Refusal,
};
use kassiber_store::rusqlite::Connection;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

struct Prepared {
    profile: Value,
    cached_pricing: bool,
}
impl Prebuild for Prepared {
    fn resolve(&mut self, _: &Connection) -> Result<Value, Refusal> {
        Ok(self.profile.clone())
    }
    fn repair(&mut self, _: &Connection, _: &mut Value) -> Result<Repairs, Refusal> {
        Ok(Repairs::default())
    }
    fn price(&mut self, c: &Connection, profile: &Value) -> Result<usize, Refusal> {
        if !self.cached_pricing {
            return Ok(0);
        }
        auto_price_transactions_from_rates_cache(
            c,
            &PricingScope {
                profile_id: profile["id"]
                    .as_str()
                    .ok_or("profile id required".to_owned())?,
                fiat_currency: profile["fiat_currency"]
                    .as_str()
                    .ok_or("fiat currency required".to_owned())?,
                decimal_context: kassiber_pydecimal::Context::RP2,
            },
        )
    }
}

/// Pause only in this unpackaged probe, after a completed SQL write. The
/// supervising test kills and reaps the child to exercise driver recovery
/// without Rust destructors or an orderly connection close.
struct Writes {
    observed: usize,
    pause_at: Option<u64>,
    faults: support::Writes,
}
impl WriteObserver for Writes {
    fn wrote(&mut self, table: &'static str) -> kassiber_store::rusqlite::Result<()> {
        self.observed += 1;
        self.faults.wrote(table)?;
        if self.pause_at == Some(self.observed as u64) {
            println!("{}", json!({"paused_at":self.observed,"table":table}));
            io::stdout().flush().expect("kill-point stdout");
            loop {
                std::thread::park();
            }
        }
        Ok(())
    }
}
#[cfg(unix)]
fn cpu() -> f64 {
    use nix::sys::{
        resource::{getrusage, UsageWho},
        time::TimeValLike,
    };
    let usage = getrusage(UsageWho::RUSAGE_SELF).expect("process CPU clock");
    (usage.user_time().num_microseconds() + usage.system_time().num_microseconds()) as f64 / 1e6
}
#[cfg(not(unix))]
fn cpu() -> f64 {
    0.0
}

fn run(v: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = if v["cached_pricing"] == true {
        &v["prepared_repairs_noop"]
    } else {
        &v["prepared_noop"]
    };
    if prepared != true {
        return Err("explicit prepared-book seam required".into());
    }
    let start = std::time::Instant::now();
    let started_cpu = cpu();
    let c = if let Some(path) = v["database"].as_str() {
        let path = std::path::Path::new(path).canonicalize()?;
        if !path.starts_with(std::env::current_dir()?.join("target").canonicalize()?) {
            return Err("oracle book must be disposable under worktree target/".into());
        }
        Connection::open(path)?
    } else {
        Connection::open_in_memory()?
    };
    if let Some(key) = v["passphrase"].as_str() {
        c.pragma_update(None, "key", key)?;
        c.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL")?;
    } else {
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL")?;
    }
    c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-16000")?;
    if v["database"].is_null() {
        support::load(&c, &v["before"]);
    }
    let mut ids = v["ids"].as_array().ok_or("UUID tape required")?.iter();
    let mut identity = || {
        ids.next()
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Refusal::from("UUID tape exhausted".to_owned()))
    };
    let mut phases = Vec::new();
    let mut writes = Writes {
        observed: 0,
        pause_at: v["pause_after_write"].as_u64(),
        faults: support::Writes {
            fail_at: v["fail_after_write"].as_u64().map(|n| n as usize),
            ..Default::default()
        },
    };
    let result = process::process(
        &c,
        &mut Prepared {
            profile: v["profile"].clone(),
            cached_pricing: v["cached_pricing"] == true,
        },
        v["clock"].as_str().ok_or("clock required")?,
        &mut identity,
        &mut |phase| {
            phases.push(
                json!({"phase":phase,"wall":start.elapsed().as_secs_f64(),"cpu":cpu()-started_cpu}),
            )
        },
        &mut writes,
    );
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let mut reply = json!({"ok":false,"error":error.to_string(),
                "refusal":error,"phases":phases});
            if v["dump"] == true {
                reply["dump"] = support::snapshot(&c);
            }
            println!("{reply}");
            io::stdout().flush()?;
            return Ok(());
        }
    };
    let mut reply = json!({"ok":true,"payload":result.payload,"phases":phases,
        "wall_seconds":start.elapsed().as_secs_f64(),"cpu_seconds":cpu()-started_cpu});
    if v["dump"] == true {
        reply["dump"] = support::snapshot(&c);
    }
    #[cfg(feature = "timing")]
    {
        reply["spans"] = serde_json::to_value(kassiber_custody::timing::take())?;
    }
    println!("{reply}");
    io::stdout().flush()?;
    result.after_reply()?;
    Ok(())
}
fn main() {
    for line in io::stdin().lock().lines() {
        let result = (|| {
            let v: Value = serde_json::from_str(&line?)?;
            run(&v)
        })();
        if let Err(error) = result {
            println!("{}", json!({"ok":false,"error":error.to_string()}));
            io::stdout().flush().expect("oracle stdout");
        }
    }
}

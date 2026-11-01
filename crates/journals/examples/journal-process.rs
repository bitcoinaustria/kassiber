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
use kassiber_transfers::Sources;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::io::{self, BufRead, Write};

struct Tapes<'a> {
    ids: RefCell<std::slice::Iter<'a, Value>>,
    ticks: Option<RefCell<std::slice::Iter<'a, Value>>>,
    clock: &'a str,
    clock_reads: Cell<usize>,
    build_clock_reads: Cell<usize>,
    uuid_reads: Cell<usize>,
    pause_precise: Option<u64>,
    fail_precise: Option<u64>,
}
impl Sources for Tapes<'_> {
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        let value = self
            .ids
            .borrow_mut()
            .next()
            .and_then(Value::as_str)
            .ok_or_else(|| {
                kassiber_envelope::AppError::new("internal_error", "UUID tape exhausted")
            })?;
        self.uuid_reads.set(self.uuid_reads.get() + 1);
        Ok(value.to_owned())
    }
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        let value = if let Some(ticks) = &self.ticks {
            ticks
                .borrow_mut()
                .next()
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    kassiber_envelope::AppError::new("internal_error", "clock tape exhausted")
                })?
        } else {
            self.clock
        };
        self.clock_reads.set(self.clock_reads.get() + 1);
        Ok(value.to_owned())
    }
    fn now_precise(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        self.build_clock_reads.set(self.build_clock_reads.get() + 1);
        if self.pause_precise == Some(self.build_clock_reads.get() as u64) {
            pause(json!({"paused_precise": self.build_clock_reads.get()}));
        }
        if self.fail_precise == Some(self.build_clock_reads.get() as u64) {
            let mut error = kassiber_envelope::AppError::new(
                "auto_pair_failed",
                "injected pairing clock failure",
            );
            error.retryable = true;
            return Err(error);
        }
        Ok(self.clock.to_owned())
    }
}

fn pause(record: Value) -> ! {
    println!("{record}");
    io::stdout().flush().expect("kill-point stdout");
    loop {
        std::thread::park();
    }
}

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
    if prepared != true && v["native_prebuild"] != true {
        return Err("explicit native or prepared pipeline mode required".into());
    }
    #[cfg(feature = "timing")]
    kassiber_journals::timing::set_cpu_clock(cpu);
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
        if !v["schema"].is_null() {
            support::load_schema(&c, &v["before"], &v["schema"]);
        } else {
            support::load(&c, &v["before"]);
        }
    }
    let sources = Tapes {
        ids: RefCell::new(v["ids"].as_array().ok_or("UUID tape required")?.iter()),
        ticks: v["ticks"].as_array().map(|v| RefCell::new(v.iter())),
        clock: v["clock"].as_str().ok_or("clock required")?,
        clock_reads: Cell::new(0),
        build_clock_reads: Cell::new(0),
        uuid_reads: Cell::new(0),
        pause_precise: v["pause_precise"].as_u64(),
        fail_precise: v["fail_precise"].as_u64(),
    };
    let mut identity = || sources.uuid(&c).map_err(|e| Refusal::from(e.message));
    let mut phases = Vec::new();
    let mut writes = Writes {
        observed: 0,
        pause_at: v["pause_after_write"].as_u64(),
        faults: support::Writes {
            fail_at: v["fail_after_write"].as_u64().map(|n| n as usize),
            ..Default::default()
        },
    };
    let mut progress = |phase| {
        phases.push(
            json!({"phase":phase,"wall":start.elapsed().as_secs_f64(),"cpu":cpu()-started_cpu}),
        );
    };
    let result = if v["refresh"] == true {
        let scope = kassiber_transfers::Scope {
            workspace_id: v["profile"]["workspace_id"]
                .as_str()
                .ok_or("workspace required")?,
            profile_id: v["profile"]["id"].as_str().ok_or("profile required")?,
        };
        // Both observers write to one ordered event sequence; their contracts
        // differ because only the outer freshness observer may commit.
        let phases = RefCell::new(&mut phases);
        let phase_counts = RefCell::new(std::collections::BTreeMap::new());
        let observe = |phase: &'static str| {
            let mut counts = phase_counts.borrow_mut();
            let occurrence = counts.entry(phase).or_insert(0_u64);
            *occurrence += 1;
            if v["pause_phase"] == phase && v["pause_phase_occurrence"] == *occurrence {
                pause(json!({"paused_phase":phase,"occurrence":occurrence}));
            }
            phases.borrow_mut().push(
                json!({"phase":phase,"wall":start.elapsed().as_secs_f64(),"cpu":cpu()-started_cpu}),
            );
        };
        let mut observer = |phase| {
            observe(phase);
        };
        process::refresh_native(
            &c,
            &scope,
            |c| support::profile(c, scope.profile_id),
            &sources,
            process::RefreshOptions {
                auto_pair: v["auto_pair"] == true,
                skip_rebuild_when_current: v["skip_rebuild_when_current"] == true,
                ..Default::default()
            },
            &mut observer,
            &mut |phase| observe(phase),
            &mut writes,
        )
    } else if v["native_prebuild"] == true {
        process::process_native(
            &c,
            |_| Ok(v["profile"].clone()),
            kassiber_pydecimal::Context::RP2,
            &sources,
            &mut progress,
            &mut writes,
        )
    } else {
        process::process(
            &c,
            &mut Prepared {
                profile: v["profile"].clone(),
                cached_pricing: v["cached_pricing"] == true,
            },
            v["clock"].as_str().ok_or("clock required")?,
            &mut identity,
            &mut progress,
            &mut writes,
        )
    };
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let mut reply = json!({"ok":false,"error":error.to_string(),
                "refusal":error,"phases":phases,"clock_reads":sources.clock_reads.get(),
                "build_clock_reads":sources.build_clock_reads.get(),"uuid_reads":sources.uuid_reads.get(),
                "write_count":writes.observed});
            if v["dump"] == true {
                reply["dump"] = support::snapshot(&c);
            }
            println!("{reply}");
            io::stdout().flush()?;
            return Ok(());
        }
    };
    let payload_json = kassiber_pyjson::dumps(&result.ordered_payload, &Default::default())?;
    let mut reply = json!({"ok":true,"payload":result.payload,"payload_json":payload_json,"phases":phases,
        "clock_reads":sources.clock_reads.get(),"build_clock_reads":sources.build_clock_reads.get(),"uuid_reads":sources.uuid_reads.get(),
        "write_count":writes.observed,
        "wall_seconds":start.elapsed().as_secs_f64(),"cpu_seconds":cpu()-started_cpu});
    if v["dump"] == true {
        reply["dump"] = support::snapshot(&c);
    }
    #[cfg(feature = "timing")]
    {
        reply["cpu_spans"] = serde_json::to_value(kassiber_journals::timing::take())?;
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

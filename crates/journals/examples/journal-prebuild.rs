#[allow(dead_code)]
#[path = "../tests/support/mod.rs"]
mod support;
use kassiber_journals::prebuild::{
    auto_price_transactions_from_rates_cache, prepare_journals,
    refresh_drifted_legacy_pair_components, repair_journal_inputs, repair_journal_source_overlaps,
    source_overlap_result_fields, source_overlap_warning, Prepared, PricingScope,
};
use kassiber_pydecimal::{Context, Rounding};
use kassiber_store::rusqlite::config::DbConfig;
use kassiber_store::rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};

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

fn open(v: &Value) -> Result<Connection, Box<dyn std::error::Error>> {
    let conn = match v["database"].as_str() {
        Some(path) => Connection::open(path)?,
        None => Connection::open_in_memory()?,
    };
    if let Some(passphrase) = v["passphrase"].as_str() {
        conn.pragma_update(None, "key", passphrase)?;
    }
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(conn)
}

fn clear(conn: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    conn.pragma_update(None, "foreign_keys", "OFF")?;
    conn.pragma_update(None, "synchronous", "OFF")?;
    conn.execute_batch("BEGIN")?;
    for kind in ["trigger", "view", "table"] {
        let names = conn
            .prepare("SELECT name FROM sqlite_master WHERE type=? AND sql IS NOT NULL")?
            .query_map([kind], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for name in names {
            conn.execute_batch(&format!("DROP {kind} \"{}\"", name.replace('"', "\"\"")))?;
        }
    }
    conn.execute_batch("COMMIT")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    Ok(())
}

fn phase<T>(
    phases: &mut serde_json::Map<String, Value>,
    name: &str,
    action: impl FnOnce() -> Result<T, kassiber_journals::Refusal>,
) -> Result<T, kassiber_journals::Refusal> {
    let start = cpu();
    let result = action();
    phases.insert(name.into(), json!(cpu() - start));
    result
}

fn profiled_prebuild(
    conn: &Connection,
    profile: &Value,
    clock: &mut impl FnMut() -> Result<String, kassiber_journals::Refusal>,
    identity: &mut impl FnMut() -> Result<String, kassiber_journals::Refusal>,
    phases: &mut serde_json::Map<String, Value>,
) -> Result<Prepared, kassiber_journals::Refusal> {
    conn.execute_batch("SAVEPOINT profile_prebuild")?;
    let result = (|| {
        let repaired = phase(phases, "repairs", || {
            repair_journal_inputs(conn, profile, clock, identity)
        })?;
        let auto_priced = phase(phases, "pricing", || {
            auto_price_transactions_from_rates_cache(
                conn,
                &PricingScope {
                    profile_id: repaired.profile["id"].as_str().expect("profile ID"),
                    fiat_currency: repaired.profile["fiat_currency"]
                        .as_str()
                        .expect("profile currency"),
                    decimal_context: Context::new(32, Rounding::HalfEven),
                },
            )
        })?;
        Ok(Prepared {
            repaired,
            auto_priced,
        })
    })();
    if result.is_err() {
        conn.execute_batch("ROLLBACK TO profile_prebuild")?;
    }
    conn.execute_batch("RELEASE profile_prebuild")?;
    result
}

fn run(
    v: &Value,
    connections: &mut BTreeMap<(String, String), (Connection, Option<Value>)>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let reuse = v["reuse"].as_bool().unwrap_or(false);
    let private = if reuse { None } else { Some(open(v)?) };
    let (conn, previous_schema) = if let Some(conn) = private.as_ref() {
        (conn, None)
    } else {
        let key = (v["database"].to_string(), v["passphrase"].to_string());
        let entry = match connections.entry(key) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => entry.insert((open(v)?, None)),
        };
        (&entry.0, Some(&mut entry.1))
    };
    if !v["before"].is_null() {
        conn.pragma_update(None, "synchronous", "OFF")?;
        let same_schema = previous_schema
            .as_ref()
            .is_some_and(|s| s.as_ref() == Some(&v["schema"]));
        if same_schema {
            conn.pragma_update(None, "foreign_keys", "OFF")?;
            conn.pragma_update(None, "ignore_check_constraints", "ON")?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)?;
            conn.execute_batch("BEGIN")?;
            let tables = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table'")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            for table in tables {
                conn.execute(
                    &format!("DELETE FROM \"{}\"", table.replace('"', "\"\"")),
                    [],
                )?;
            }
            support::insert_dump(conn, &v["before"]);
            conn.execute_batch(
                "COMMIT;PRAGMA foreign_keys=ON;PRAGMA ignore_check_constraints=OFF",
            )?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, true)?;
        } else {
            if previous_schema.as_ref().is_some_and(|s| s.is_some()) {
                clear(conn)?;
                conn.pragma_update(None, "synchronous", "OFF")?;
            }
            if !v["schema"].is_null() {
                support::load_schema(conn, &v["before"], &v["schema"]);
            } else {
                support::load(conn, &v["before"]);
            }
            if let Some(previous) = previous_schema {
                *previous = Some(v["schema"].clone());
            }
        }
        conn.pragma_update(None, "synchronous", "FULL")?;
    }
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let before = cpu();
    let start = std::time::Instant::now();
    let mut phases = serde_json::Map::new();
    let cache_start = v["phase_breakdown"].as_bool().unwrap_or(false).then(cpu);
    let cache = if v["rebuild_cache"].as_bool().unwrap_or(false) {
        Some(kassiber_journals::storage::RebuildCache::enter(conn)?)
    } else {
        None
    };
    if let Some(start) = cache_start {
        phases.insert("cache_enter".into(), json!(cpu() - start));
    }
    let mut ids = v["ids"].as_array().into_iter().flatten();
    let mut ticks = v["ticks"].as_array().into_iter().flatten();
    let mut clock = || {
        ticks
            .next()
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                kassiber_journals::Refusal::from("source audit clock tape exhausted".to_string())
            })
    };
    let mut identity = || {
        ids.next()
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                kassiber_journals::Refusal::from("source audit ID tape exhausted".to_string())
            })
    };
    let mut projection_wire = Value::Null;
    let result = if v["stage"] == "sources" {
        repair_journal_source_overlaps(
            conn,
            v["profile_id"].as_str().unwrap_or("pf"),
            &mut clock,
            &mut identity,
        )
        .and_then(|repair| {
            let warning = source_overlap_warning(
                conn,
                v["profile_id"].as_str().unwrap_or("pf"),
                repair.as_ref(),
            )?;
            let fields = source_overlap_result_fields(repair.as_ref(), warning.as_ref())?;
            projection_wire = json!(kassiber_pyjson::dumps(
                &fields,
                &kassiber_pyjson::Options::new()
            )
            .map_err(|e| kassiber_journals::Refusal::from(e.to_string()))?);
            Ok(json!({"repair":repair,"warning":warning}))
        })
    } else if v["stage"] == "pipeline" {
        let prepared = if v["phase_breakdown"].as_bool().unwrap_or(false) {
            profiled_prebuild(conn, &v["profile"], &mut clock, &mut identity, &mut phases)
        } else {
            prepare_journals(
                conn,
                &v["profile"],
                Context::new(32, Rounding::HalfEven),
                &mut clock,
                &mut identity,
            )
        };
        prepared.and_then(|p| {
            projection_wire = json!(kassiber_pyjson::dumps(&p.repaired.result_fields()?, &kassiber_pyjson::Options::new())
                .map_err(|e| kassiber_journals::Refusal::from(e.to_string()))?);
            let r = p.repaired;
            Ok(json!({"repair":r.source_overlap_repair,"warning":r.source_overlap_warning,
                "consolidation":{"activated":r.consolidation.activated,"unchanged":r.consolidation.unchanged,"skipped":r.consolidation.skipped},
                "profile":r.profile,"auto_priced":p.auto_priced}))
        })
    } else if v["stage"] == "legacy" {
        refresh_drifted_legacy_pair_components(conn, &v["profile"], &mut clock).map(|result| {
            json!({"consolidation":{"activated":result.consolidation.activated,"unchanged":result.consolidation.unchanged,"skipped":result.consolidation.skipped},"profile":result.profile})
        })
    } else {
        auto_price_transactions_from_rates_cache(
            conn,
            &PricingScope {
                profile_id: v["profile_id"].as_str().unwrap_or("p"),
                fiat_currency: v["fiat_currency"].as_str().unwrap_or("EUR"),
                decimal_context: Context::new(32, Rounding::HalfEven),
            },
        )
        .map(|count| json!(count))
    };
    if let Some(cache) = cache {
        let cache_start = v["phase_breakdown"].as_bool().unwrap_or(false).then(cpu);
        cache.restore()?;
        if let Some(start) = cache_start {
            phases.insert("cache_restore".into(), json!(cpu() - start));
        }
    }
    let wall_seconds = start.elapsed().as_secs_f64();
    let cpu_seconds = cpu() - before;
    if v["stage"] == "pipeline" {
        assert!(ids.next().is_none(), "unused source audit identity");
        assert!(ticks.next().is_none(), "unused pre-build clock tick");
    }
    let (count, error) = match result {
        Ok(count) => (Some(count), Value::Null),
        Err(error) => (None, serde_json::to_value(error)?),
    };
    let verified = if v["expected"].is_null() {
        Value::Null
    } else {
        Value::Bool(support::logical_equal(
            &support::snapshot(conn),
            &v["expected"],
        ))
    };
    let dump = v["dump"]
        .as_bool()
        .unwrap_or(false)
        .then(|| support::snapshot(conn));
    conn.execute_batch("ROLLBACK")?;
    let mut reply = json!({"name":v["name"],"count":count,"result":count,"projection_wire":projection_wire,"error":error,"verified":verified,"dump":dump,"cpu_seconds":cpu_seconds,"wall_seconds":wall_seconds});
    if !phases.is_empty() {
        reply["cpu_by_phase"] = Value::Object(phases);
    }
    Ok(reply)
}

fn main() {
    let mut connections = BTreeMap::new();
    for line in io::stdin().lock().lines() {
        let result = (|| {
            let v: Value = serde_json::from_str(&line?)?;
            run(&v, &mut connections)
        })();
        match result {
            Ok(reply) => println!("{reply}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        io::stdout().flush().expect("oracle stdout");
    }
}

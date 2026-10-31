#[allow(dead_code)]
#[path = "../tests/support/mod.rs"]
mod support;
use kassiber_journals::prebuild::{
    auto_price_transactions_from_rates_cache, repair_journal_source_overlaps,
    source_overlap_result_fields, source_overlap_warning, PricingScope,
};
use kassiber_pydecimal::{Context, Rounding};
use kassiber_store::rusqlite::Connection;
use serde_json::{json, Value};
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

fn run(v: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let conn = match v["database"].as_str() {
        Some(path) => Connection::open(path)?,
        None => Connection::open_in_memory()?,
    };
    if let Some(passphrase) = v["passphrase"].as_str() {
        conn.pragma_update(None, "key", passphrase)?;
    }
    if !v["before"].is_null() {
        support::load(&conn, &v["before"]);
    }
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let before = cpu();
    let start = std::time::Instant::now();
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
            &conn,
            v["profile_id"].as_str().unwrap_or("pf"),
            &mut clock,
            &mut identity,
        )
        .and_then(|repair| {
            let warning = source_overlap_warning(
                &conn,
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
    } else {
        auto_price_transactions_from_rates_cache(
            &conn,
            &PricingScope {
                profile_id: v["profile_id"].as_str().unwrap_or("p"),
                fiat_currency: v["fiat_currency"].as_str().unwrap_or("EUR"),
                decimal_context: Context::new(32, Rounding::HalfEven),
            },
        )
        .map(|count| json!(count))
    };
    let wall_seconds = start.elapsed().as_secs_f64();
    let cpu_seconds = cpu() - before;
    let (count, error) = match result {
        Ok(count) => (Some(count), Value::Null),
        Err(error) => (None, serde_json::to_value(error)?),
    };
    let verified = if v["expected"].is_null() {
        Value::Null
    } else {
        Value::Bool(support::logical_equal(
            &support::snapshot(&conn),
            &v["expected"],
        ))
    };
    let dump = v["dump"]
        .as_bool()
        .unwrap_or(false)
        .then(|| support::snapshot(&conn));
    conn.execute_batch("ROLLBACK")?;
    Ok(
        json!({"name":v["name"],"count":count,"result":count,"projection_wire":projection_wire,"error":error,"verified":verified,"dump":dump,"cpu_seconds":cpu_seconds,"wall_seconds":wall_seconds}),
    )
}

fn main() {
    for line in io::stdin().lock().lines() {
        let result = (|| {
            let v: Value = serde_json::from_str(&line?)?;
            run(&v)
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

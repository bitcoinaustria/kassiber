#[allow(dead_code)]
#[path = "../tests/support/mod.rs"]
mod support;
use kassiber_journals::prebuild::{auto_price_transactions_from_rates_cache, PricingScope};
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
    let result = auto_price_transactions_from_rates_cache(
        &conn,
        &PricingScope {
            profile_id: v["profile_id"].as_str().unwrap_or("p"),
            fiat_currency: v["fiat_currency"].as_str().unwrap_or("EUR"),
            decimal_context: Context::new(32, Rounding::HalfEven),
        },
    );
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
        json!({"name":v["name"],"count":count,"error":error,"verified":verified,"dump":dump,"cpu_seconds":cpu_seconds,"wall_seconds":wall_seconds}),
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

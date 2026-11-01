//! Local parity/benchmark probe. Never registered as a product command.
use kassiber_journals::boundary;
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Write},
    time::Instant,
};
fn run(v: &Value, timing_only: bool) -> Value {
    let mut ids = v["entry_ids"].as_array().expect("entry_ids").iter();
    let response = if v["response"].is_null() {
        Ok(boundary::empty_response())
    } else {
        boundary::ledger_response(&v["response"])
    };
    let response = match response {
        Ok(r) => r,
        Err(_) => {
            let error = boundary::response_refusal(
                &v["profile"],
                &v["custody"],
                v["wallets"].as_array().expect("wallets"),
                &v["response"],
                || {
                    ids.next()
                        .expect("declared UUID tape exhausted")
                        .as_str()
                        .expect("UUID")
                        .into()
                },
            );
            return json!({"error":error,"entry_ids_consumed":v["entry_ids"].as_array().expect("entry_ids").len() - ids.len()});
        }
    };
    let cpu_start = cpu_seconds();
    let start = Instant::now();
    let result = boundary::build(
        &v["profile"],
        &v["custody"],
        v["wallets"].as_array().expect("wallets"),
        &response,
        || {
            ids.next()
                .expect("declared UUID tape exhausted")
                .as_str()
                .expect("UUID")
                .into()
        },
    );
    let elapsed = start.elapsed().as_secs_f64();
    let cpu_elapsed = cpu_seconds() - cpu_start;
    let consumed = v["entry_ids"].as_array().expect("entry_ids").len() - ids.len();
    #[cfg(feature = "timing")]
    eprintln!("{:?}", kassiber_custody::timing::take());
    match result {
        Ok(_) if timing_only => {
            json!({"seconds":elapsed,"cpu_seconds":cpu_elapsed,"entry_ids_consumed":consumed})
        }
        Ok(s) => {
            json!({"state":boundary::canonical(&s).expect("canonical state"),"seconds":elapsed,"cpu_seconds":cpu_elapsed,"entry_ids_consumed":consumed})
        }
        Err(e) => {
            json!({"error":e,"seconds":elapsed,"cpu_seconds":cpu_elapsed,"entry_ids_consumed":consumed})
        }
    }
}
#[cfg(unix)]
fn cpu_seconds() -> f64 {
    use nix::sys::resource::{getrusage, UsageWho};
    let usage = getrusage(UsageWho::RUSAGE_SELF).expect("process CPU usage");
    let time = |t: nix::sys::time::TimeVal| t.tv_sec() as f64 + t.tv_usec() as f64 / 1_000_000.0;
    time(usage.user_time()) + time(usage.system_time())
}
#[cfg(not(unix))]
fn cpu_seconds() -> f64 {
    f64::NAN
}
fn main() {
    let timing_only = std::env::args().any(|a| a == "--timing-only");
    let mut cached: Option<(String, Value)> = None;
    // One path per line permits corpus replay without paying process start per book.
    for path in io::stdin().lock().lines() {
        let path = path.expect("path");
        if cached.as_ref().is_none_or(|(p, _)| p != &path) {
            let text = std::fs::read_to_string(&path).expect("input file");
            let v: Value = serde_json::from_str(&text).expect("input JSON");
            cached = Some((path, v));
        }
        let reply = run(&cached.as_ref().expect("loaded input").1, timing_only);
        println!("{reply}");
        io::stdout().flush().expect("stdout");
    }
}

//! Times one schema-2 request file: `ledger-bench <request.json> [runs] [--expect <response.json>]`.
//! Prints the best run's spans, nested spans summed by name, and whether the
//! response equals the expected text byte for byte.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

type Spans = Vec<(&'static str, Duration)>;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("request path");
    let runs: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(3);
    let expect = args
        .iter()
        .position(|a| a == "--expect")
        .map(|i| std::fs::read_to_string(&args[i + 1]).expect("expected response"));
    let request = std::fs::read_to_string(path).expect("request");
    let mut best: Option<(Duration, Spans, String)> = None;
    for _ in 0..runs {
        kassiber_tax_core::timing::take();
        let start = Instant::now();
        let response = kassiber_tax_core::ledger(&request);
        let elapsed = start.elapsed();
        let spans = kassiber_tax_core::timing::take();
        if best.as_ref().is_none_or(|(b, _, _)| elapsed < *b) {
            best = Some((elapsed, spans, response));
        }
    }
    let (elapsed, spans, response) = best.expect("one run");
    let mut totals: BTreeMap<&str, (Duration, usize)> = BTreeMap::new();
    let mut order = Vec::new();
    for (name, duration) in spans {
        let entry = totals.entry(name).or_insert_with(|| {
            order.push(name);
            (Duration::ZERO, 0)
        });
        entry.0 += duration;
        entry.1 += 1;
    }
    let verdict = match &expect {
        Some(text) if *text == response => "identical",
        Some(_) => "DIFFERS",
        None => "unchecked",
    };
    println!(
        "total {:.3} s, response {} bytes, {verdict}",
        elapsed.as_secs_f64(),
        response.len(),
    );
    for name in order {
        let (duration, count) = totals[name];
        println!("  {name:40} {:9.3} s  x{count}", duration.as_secs_f64());
    }
    if verdict == "DIFFERS" {
        std::process::exit(1);
    }
}

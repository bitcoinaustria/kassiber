//! Times one raw v1 snapshot file per stage: `custody-bench <snapshot.json> [runs] [--decisions]`.
//! Prints the best run's spans; nested spans are summed by name.
//! `custody-bench --corpus <requests.jsonl>` prints one line per request with
//! the SHA-256 of the full and the decision-only response, and the total time.
use std::collections::BTreeMap;
use std::io::BufRead;
use std::time::{Duration, Instant};

type Spans = Vec<(&'static str, Duration)>;

fn corpus(path: &str) {
    let file = std::fs::File::open(path).expect("corpus");
    let mut elapsed = Duration::ZERO;
    for (index, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line.expect("corpus line");
        let start = Instant::now();
        let full = kassiber_custody::custody_with_finalized(&line, true);
        let decisions = kassiber_custody::custody_with_finalized(&line, false);
        elapsed += start.elapsed();
        kassiber_custody::timing::take();
        println!(
            "{index} {} {}",
            kassiber_custody::codec::sha256(&full),
            kassiber_custody::codec::sha256(&decisions)
        );
    }
    eprintln!("{path}: {:.3} s", elapsed.as_secs_f64());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--corpus") {
        for path in &args[2..] {
            corpus(path);
        }
        return;
    }
    let path = args.get(1).expect("snapshot path");
    let runs: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(3);
    let finalized = !args.iter().any(|a| a == "--decisions");
    let request = std::fs::read_to_string(path).expect("snapshot");
    let mut best: Option<(Duration, Spans, String)> = None;
    for _ in 0..runs {
        kassiber_custody::timing::take();
        let start = Instant::now();
        let response = kassiber_custody::custody_with_finalized(&request, finalized);
        let elapsed = start.elapsed();
        let spans = kassiber_custody::timing::take();
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
    println!(
        "total {:.3} s, response {} bytes, sha256 {}",
        elapsed.as_secs_f64(),
        response.len(),
        kassiber_custody::codec::sha256(&response)
    );
    for name in order {
        let (duration, count) = totals[name];
        println!("  {name:40} {:9.3} s  x{count}", duration.as_secs_f64());
    }
}

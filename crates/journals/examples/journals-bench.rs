//! Times the Rust journal pipeline on one raw v1 snapshot, best of `--runs`:
//!
//!     journals-bench <snapshot.json> [--runs N] [--decisions] [--oracle] [--json]
//!
//! Spans are summed by name in first-seen order. `compute` covers parsing
//! through the tax engine; reading the file and, with `--oracle`, writing and
//! hashing the custody response text (the parity oracle, not production work)
//! are timed separately. Hashes of the ledger request and response let a
//! bench run double as a parity check against an earlier run. `--json` prints
//! one record line instead of the table (docs/reference/r4-journal-pipeline.md).
//!
//!     journals-bench --check <requests.jsonl>...
//!
//! compares the pipeline's fusion with `custody_with_fusion` for every request.
//! `--roundtrip <requests.jsonl>...` reconstructs the typed snapshot's v1
//! CPython text and requires byte equality for every line, including refusals.
use kassiber_journals::{compute, Fusion};
use std::io::BufRead;
use std::time::{Duration, Instant};

type Spans = Vec<(&'static str, Duration)>;

struct Run {
    compute: Duration,
    spans: Spans,
    fusion: Fusion,
    oracle: Option<String>,
}

fn sha256(text: &str) -> String {
    kassiber_custody::codec::sha256(text)
}

fn once(request: &str, finalized: bool, oracle: bool) -> Run {
    kassiber_custody::timing::take();
    let start = Instant::now();
    let computed = compute(request, finalized);
    let elapsed = start.elapsed();
    let (fusion, oracle) = match computed {
        Ok(computed) => {
            let oracle = oracle.then(|| {
                let _span = kassiber_custody::timing::span("oracle.response");
                kassiber_custody::response(
                    &computed.snapshot.transactions,
                    &computed.stages,
                    computed.snapshot.transactions.len() >= 256,
                )
            });
            (computed.fusion, oracle)
        }
        Err(refusal) => (Fusion::Refused(serde_json::json!(refusal.document())), None),
    };
    Run {
        compute: elapsed,
        spans: kassiber_custody::timing::take(),
        fusion,
        oracle,
    }
}

fn check(paths: &[String]) {
    let (mut compared, mut differences) = (0usize, 0usize);
    for path in paths {
        let file = std::fs::File::open(path).expect("requests");
        for (index, line) in std::io::BufReader::new(file).lines().enumerate() {
            let request = line.expect("request line");
            let Ok(computed) = compute(&request, true) else {
                continue;
            };
            let harness =
                kassiber_custody::custody_with_fusion(&request, true, kassiber_tax_core::ledger);
            let harness = kassiber_custody::codec::to_json(
                &kassiber_custody::codec::loads(&harness).expect("custody response"),
            );
            let fusion = &harness["stages"]["fusion"];
            let same = match &computed.fusion {
                Fusion::Empty => fusion["outcome"] == "empty",
                Fusion::Refused(error) => {
                    fusion["outcome"] == "refusal" && fusion["error"] == *error
                }
                Fusion::Ledger { request, response } => {
                    fusion["outcome"] == "request"
                        && fusion["request_text"] == request.as_str()
                        && fusion["response"]
                            == serde_json::from_str::<serde_json::Value>(response)
                                .expect("ledger response")
                }
                Fusion::Skipped => false,
            };
            compared += 1;
            if !same {
                differences += 1;
                println!("DIFF {path}:{index}");
            }
        }
    }
    println!("compared {compared} requests; {differences} differences");
    if differences > 0 {
        std::process::exit(1);
    }
}

fn roundtrip(paths: &[String]) {
    let mut compared = 0;
    for path in paths {
        let file = std::fs::File::open(path).expect("requests");
        for (index, line) in std::io::BufReader::new(file).lines().enumerate() {
            let request = line.expect("request line");
            let snapshot: kassiber_custody::snapshot::BookSnapshot =
                serde_json::from_str(&request).unwrap_or_else(|e| panic!("{path}:{index}: {e}"));
            let text = snapshot.text().expect("snapshot text");
            assert!(text == request, "snapshot bytes differ at {path}:{index}");
            compared += 1;
        }
    }
    println!("round-tripped {compared} requests; 0 byte differences");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--check") {
        return check(&args[1..]);
    }
    if args.first().is_some_and(|a| a == "--roundtrip") {
        return roundtrip(&args[1..]);
    }
    let path = args.first().expect("snapshot path");
    let runs: usize = args
        .iter()
        .position(|a| a == "--runs")
        .and_then(|at| args.get(at + 1))
        .map(|v| v.parse().expect("--runs N"))
        .unwrap_or(3);
    let finalized = !args.iter().any(|a| a == "--decisions");
    let oracle = args.iter().any(|a| a == "--oracle");
    let json = args.iter().any(|a| a == "--json");

    let start = Instant::now();
    let request = std::fs::read_to_string(path).expect("snapshot");
    let read = start.elapsed();
    let mut best: Option<Run> = None;
    for _ in 0..runs.max(1) {
        let run = once(&request, finalized, oracle);
        if best.as_ref().is_none_or(|b| run.compute < b.compute) {
            best = Some(run);
        }
    }
    let run = best.expect("one run");
    let mut totals: Vec<(&str, Duration, usize)> = Vec::new();
    for (name, duration) in &run.spans {
        match totals.iter_mut().find(|(n, _, _)| n == name) {
            Some(entry) => {
                entry.1 += *duration;
                entry.2 += 1;
            }
            None => totals.push((name, *duration, 1)),
        }
    }
    let (outcome, request_hash, response_hash) = match &run.fusion {
        Fusion::Skipped => ("skipped", None, None),
        Fusion::Empty => ("empty", None, None),
        Fusion::Refused(error) => ("refused", Some(sha256(&error.to_string())), None),
        Fusion::Ledger { request, response } => {
            ("ledger", Some(sha256(request)), Some(sha256(response)))
        }
    };
    let oracle_hash = run.oracle.as_deref().map(sha256);
    if json {
        let record = serde_json::json!({
            "schema": "kassiber.journals-bench.rust.v1",
            "snapshot": path,
            "snapshot_bytes": request.len(),
            "finalized": finalized,
            "runs": runs.max(1),
            "read_seconds": read.as_secs_f64(),
            "compute_seconds": run.compute.as_secs_f64(),
            "spans": totals.iter().map(|(name, duration, count)| serde_json::json!([name, duration.as_secs_f64(), count])).collect::<Vec<_>>(),
            "fusion": outcome,
            "ledger_request_sha256": request_hash,
            "ledger_response_sha256": response_hash,
            "custody_response_sha256": oracle_hash,
        });
        println!("{record}");
        return;
    }
    println!(
        "compute {:.3} s (best of {}), read {:.3} s, snapshot {} bytes, fusion {outcome}",
        run.compute.as_secs_f64(),
        runs.max(1),
        read.as_secs_f64(),
        request.len()
    );
    for (name, duration, count) in totals {
        let calls = if count > 1 {
            format!("  x{count}")
        } else {
            String::new()
        };
        println!("  {name:40} {:9.3} s{calls}", duration.as_secs_f64());
    }
    for (label, hash) in [
        ("ledger request", request_hash),
        ("ledger response", response_hash),
        ("custody response", oracle_hash),
    ] {
        if let Some(hash) = hash {
            println!("{label} sha256 {hash}");
        }
    }
}

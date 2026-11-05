//! The secret floor stays linear on every shared scaling and stress shape.
//!
//! Each shape is built at 40,000 and 400,000 characters as
//! `tests/test_secret_redaction_5h.py` builds it (staircases indent each
//! repeat one column deeper or shallower). Ten times the input must cost
//! about ten times the CPU, not a hundred. Calls are timed in the calling
//! thread's CPU time where Linux reports it, else in wall time.

use std::time::Instant;

use kassiber_envelope::redaction::redact_secret_text;
use serde_json::Value;

const SCALING: &str = include_str!("../../../tests/fixtures/secret_redaction_scaling_cases.json");
const STRESS: &str = include_str!("../../../tests/fixtures/secret_redaction_stress_cases.json");
/// The 400k/40k ratio a linear scan stays under, with room for timer noise.
const RATIO: f64 = 12.0;
/// Absolute slack for timer noise on the cheapest shapes.
const SLACK_SECONDS: f64 = 0.0005;

fn field<'a>(case: &'a Value, name: &str) -> &'a str {
    case.get(name).and_then(Value::as_str).unwrap_or("")
}

fn build(case: &Value, size: usize) -> String {
    let (prefix, repeat, suffix) = (
        field(case, "prefix"),
        field(case, "repeat"),
        field(case, "suffix"),
    );
    let unit = repeat.chars().count();
    match case.get("staircase").and_then(Value::as_str) {
        None => format!("{prefix}{}{suffix}", repeat.repeat(size / unit + 1)),
        Some(direction) => {
            let steps = ((2 * size) as f64).sqrt() as usize;
            let mut text = prefix.to_owned();
            for step in 0..steps {
                let indent = if direction == "down" {
                    steps - 1 - step
                } else {
                    step
                };
                text.push_str(&" ".repeat(indent));
                text.push_str(repeat);
            }
            text.push_str(suffix);
            text
        }
    }
}

/// This thread's CPU seconds (Linux), else wall seconds.
fn cpu_seconds() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
    let nanos: f64 = stat.split_whitespace().next()?.parse().ok()?;
    Some(nanos / 1e9)
}

/// CPU seconds per call, averaged over enough calls to fill 50 ms: the
/// kernel's per-thread accounting is too coarse for a single short call.
fn cost(text: &str) -> f64 {
    let (cpu, wall) = (cpu_seconds(), Instant::now());
    let elapsed = || match (cpu, cpu_seconds()) {
        (Some(before), Some(after)) => after - before,
        _ => wall.elapsed().as_secs_f64(),
    };
    let mut calls = 0u32;
    while calls < 2 || elapsed() < 0.05 {
        std::hint::black_box(redact_secret_text(std::hint::black_box(text)));
        calls += 1;
    }
    elapsed() / f64::from(calls)
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "times optimized code: cargo test --release -p kassiber-envelope --test scanner_scaling"
)]
fn every_shape_scales_linearly_to_400k() {
    let mut cases: Vec<Value> = serde_json::from_str::<Vec<Value>>(SCALING).unwrap();
    let stress: Vec<Value> = serde_json::from_str(STRESS).unwrap();
    assert!(cases.iter().any(|case| case.get("staircase").is_some()));
    cases.extend(stress);
    let mut slow = Vec::new();
    let mut report = Vec::new();
    for case in &cases {
        let small = cost(&build(case, 40_000));
        let large = cost(&build(case, 400_000));
        report.push(format!(
            "{:<28} {:>9.4}s {:>9.4}s {:>6.1}x",
            field(case, "name"),
            small,
            large,
            large / small.max(1e-9)
        ));
        if large > RATIO * small + SLACK_SECONDS {
            slow.push(field(case, "name").to_owned());
        }
    }
    println!("{}", report.join("\n"));
    assert!(
        slow.is_empty(),
        "not linear: {slow:?}\n{}",
        report.join("\n")
    );
}

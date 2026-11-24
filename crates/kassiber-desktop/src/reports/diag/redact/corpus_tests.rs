//! The shared redaction corpora through the Logs screen's own path: a record enters the ring
//! through `Store::emit` (the secret floor at insert: the backend's rules, then this module's
//! extra patterns) and renders through `Record::redacted` in a support mode. Ported from the
//! removed web app's `appLogsRedactionCorpus.test.ts` and `secretRedactionGenerated.test.ts`,
//! over the same `tests/fixtures` corpora. Unit tests salt amount pseudonyms with the corpus
//! salt (`salt()`).

use std::time::Instant;

use serde_json::Value;

use super::*;
use crate::reports::diag::logstore::{Field, Record, Store};

#[path = "../../../../../envelope/tests/support/generated_cases.rs"]
mod generated_cases;
#[path = "../../../../../envelope/tests/support/sha256.rs"]
mod sha256;

macro_rules! fixture {
    ($name:literal) => {
        serde_json::from_str::<Value>(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/",
            $name
        )))
        .unwrap()
    };
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// `emitAppLog({level: "info", module: "test", file: "test", line: 1, msg, fields})`.
fn emit(store: &mut Store, msg: &str, fields: Vec<(String, Field)>) -> Record {
    store.emit(Record {
        id: String::new(),
        ts: String::new(),
        level: "info".into(),
        module: "test".into(),
        file: "test".into(),
        line: 1,
        msg: msg.into(),
        fields,
    });
    let record = store.records.back().unwrap().clone();
    store.clear();
    record
}

fn emitted(msg: &str) -> String {
    emit(&mut Store::new(), msg, vec![]).msg
}

#[test]
fn operational_cases_match_python_in_both_support_modes() {
    let corpus = fixture!("operational_redaction_cases.json");
    assert_eq!(s(&corpus["salt"]), salt(), "the corpus salt");
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 37);
    let mut store = Store::new();
    let mut mismatches = Vec::new();
    for case in cases {
        for mode in [Mode::HighSignal, Mode::PublicSafe] {
            let record = emit(&mut store, s(&case["input"]), vec![]);
            let opts = Opts {
                redacted: true,
                mask_amounts: true,
                mode,
            };
            let out = record.redacted(&opts).msg;
            if out != s(&case["expected"]) {
                mismatches.push(format!(
                    "{}: {:?}\n  expected {:?}\n  desktop  {out:?}",
                    mode.id(),
                    s(&case["input"]),
                    s(&case["expected"])
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn secret_cases_floor_message_and_text_field_as_python() {
    let cases = fixture!("secret_redaction_cases.json");
    let cases = cases.as_array().unwrap();
    assert_eq!(cases.len(), 444);
    let mut store = Store::new();
    let mut mismatches = Vec::new();
    for case in cases {
        let (input, expected) = (s(&case["input"]), s(&case["expected"]));
        let detail = Field {
            ty: "text".into(),
            value: Value::String(input.into()),
        };
        let record = emit(&mut store, input, vec![("detail".into(), detail)]);
        let field = record.fields[0].1.value.as_str().unwrap_or("").to_owned();
        if record.msg != expected || field != expected {
            mismatches.push(format!(
                "{input:?}\n  expected {expected:?}\n  message  {:?}\n  field    {field:?}",
                record.msg
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches[..mismatches.len().min(10)].join("\n")
    );
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

#[test]
fn review_regressions_fail_closed() {
    let mut cases = Vec::new();
    for corpus in [
        fixture!("secret_redaction_5e_cases.json"),
        fixture!("secret_redaction_5f_cases.json"),
        fixture!("secret_redaction_5g_cases.json"),
        fixture!("secret_redaction_5h_cases.json"),
        fixture!("secret_redaction_5i_cases.json"),
    ] {
        cases.extend(corpus.as_array().unwrap().clone());
    }
    assert_eq!(cases.len(), 215 + 135 + 76 + 84 + 133);
    let mut store = Store::new();
    let mut failures = Vec::new();
    for case in &cases {
        let (id, input) = (s(&case["id"]), s(&case["input"]));
        let output = emit(&mut store, input, vec![]).msg;
        let mut problems = Vec::new();
        if let Some(expected) = case["expected"].as_str()
            && output != expected
        {
            problems.push(format!("expected {expected:?}"));
        }
        let sentinel = units(s(&case["sentinel"]));
        let out = units(&output);
        // The sentinel, or any four code units of it.
        let holds = |needle: &[u16]| out.windows(needle.len()).any(|w| w == needle);
        if !sentinel.is_empty() && (holds(&sentinel) || sentinel.windows(4).any(holds)) {
            problems.push("sentinel survives".into());
        }
        for keep in case["preserved"].as_array().unwrap() {
            if !output.contains(s(keep)) {
                problems.push(format!("lost {:?}", s(keep)));
            }
        }
        if !problems.is_empty() {
            failures.push(format!(
                "{id}: {}: {input:?} => {output:?}",
                problems.join(", ")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failing:\n{}",
        failures.len(),
        failures[..failures.len().min(10)].join("\n")
    );
}

#[test]
fn oversized_records_are_masked_whole_counting_code_points() {
    let emoji = "😀".repeat(600_000);
    assert_eq!(emitted(&emoji), emoji);
    let long = format!("password=Pw9Leak {}", "x".repeat(1_000_000));
    assert_eq!(emitted(&long), "[redacted-oversized]");
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "100,000 cases; the release run covers it: cargo test --release"
)]
fn generated_corpus_floors_as_python_and_keeps_non_secrets() {
    let spec = fixture!("secret_redaction_generator.json");
    let mut digest = sha256::Sha256::new();
    let (mut cases, mut failed, mut failures) = (0usize, 0usize, Vec::new());
    generated_cases::for_each_case(&spec, |case| {
        // `Store::emit`'s message floor, without the ring.
        let output = secret_floor_text(&case.input);
        let chars: Vec<char> = case.sentinel.chars().collect();
        let leaked = chars
            .windows(4)
            .any(|w| output.contains(&w.iter().collect::<String>()));
        let lost: Vec<&String> = case
            .preserved
            .iter()
            .filter(|keep| !output.contains(keep.as_str()))
            .collect();
        if leaked || !lost.is_empty() {
            failed += 1;
            if failures.len() < 5 {
                failures.push(format!(
                    "case {}: leaked {leaked}, lost {lost:?}: {:?} => {output:?}",
                    case.index, case.input
                ));
            }
        }
        digest.update(output.as_bytes());
        digest.update(b"\n");
        cases += 1;
    });
    assert!(
        failures.is_empty(),
        "{failed} failing cases, the first:\n{}",
        failures.join("\n")
    );
    assert_eq!(cases, 100_000);
    assert_eq!(digest.hex(), s(&spec["output_sha256"]));
}

// ---- Deadlines and scaling (wall time, as the web suite measured; optimized builds only) ----

/// `prefix + repeat.repeat(floor(size / repeat.length) + 1) + suffix`, or a staircase that
/// indents each repeat one column deeper (or shallower).
fn shaped(case: &Value, size: usize) -> String {
    let (prefix, repeat, suffix) = (s(&case["prefix"]), s(&case["repeat"]), s(&case["suffix"]));
    match case["staircase"].as_str() {
        None => format!(
            "{prefix}{}{suffix}",
            repeat.repeat(size / repeat.encode_utf16().count() + 1)
        ),
        Some(direction) => {
            let steps = ((2 * size) as f64).sqrt() as usize;
            let mut body = String::new();
            for step in 0..steps {
                let indent = if direction == "down" {
                    steps - 1 - step
                } else {
                    step
                };
                body.push_str(&" ".repeat(indent));
                body.push_str(repeat);
            }
            format!("{prefix}{body}{suffix}")
        }
    }
}

/// Milliseconds to emit `text` into a fresh ring.
fn emit_ms(text: &str) -> f64 {
    let mut store = Store::new();
    let start = Instant::now();
    std::hint::black_box(emit(&mut store, text, vec![]));
    start.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "times optimized code: cargo test --release"
)]
fn long_plain_records_floor_well_under_a_second() {
    for separator in ["-", "."] {
        let text = format!("word{separator}").repeat(20_000);
        let start = Instant::now();
        assert_eq!(emitted(&text), text);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        assert!(ms < 500.0, "{separator}: {ms} ms");
    }
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "times optimized code: cargo test --release"
)]
fn adversarial_secret_shapes_floor_within_the_deadline_and_scale_linearly() {
    let stress = fixture!("secret_redaction_stress_cases.json");
    let stress = stress.as_array().unwrap();
    assert_eq!(stress.len(), 53);
    for case in stress {
        let ms = emit_ms(&shaped(case, 40_000));
        assert!(ms < 500.0, "{}: {ms} ms", s(&case["name"]));
    }
    // Aggregate time: quadratic growth would cost ~100x for ten times the input.
    let total = |size| {
        stress
            .iter()
            .map(|case| emit_ms(&shaped(case, size)))
            .sum::<f64>()
    };
    let small = total(40_000);
    let large = total(400_000);
    assert!(large < 25.0 * small + 1000.0, "{large} ms vs {small} ms");
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "times optimized code: cargo test --release"
)]
fn every_scaling_shape_floors_linearly_to_400k() {
    let scaling = fixture!("secret_redaction_scaling_cases.json");
    let scaling = scaling.as_array().unwrap();
    assert_eq!(scaling.len(), 50);
    let mut slow = Vec::new();
    for case in scaling {
        // Each shape on its own: quadratic growth costs ~100x for ten times the input.
        emit_ms(&shaped(case, 4_000));
        let small = emit_ms(&shaped(case, 40_000));
        let large = emit_ms(&shaped(case, 400_000));
        if small >= 500.0 || large >= 25.0 * small + 50.0 {
            slow.push(format!("{}: {small} ms -> {large} ms", s(&case["name"])));
        }
    }
    assert!(slow.is_empty(), "{slow:#?}");
}

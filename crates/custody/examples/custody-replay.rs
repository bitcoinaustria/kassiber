//! Replay frozen custody calls and require byte-identical responses.
//!
//!     cargo run --release -p kassiber-custody --example custody-replay -- DIR
//!
//! DIR holds `*.replay.json` goldens (schema `kassiber.custody-replay/1`). Each
//! names the raw-fact requests one custody recording sends (the recorded
//! request and its permutation) and the response the Rust engine gave when
//! the Python reference compared every stage with it and found no difference
//! (docs/reference/testing.md#custody-recordings). The replay sends each
//! request through the same entry point and fails on any byte of difference.
//! `--self-test` first proves the comparison notices a changed response.
//!
//! After a reviewed custody change, `KASSIBER_GOLDEN_WRITE=1` rewrites each
//! golden's responses with the current engine's instead of comparing; review
//! the diff before keeping it. The switch is refused under CI.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn goldens(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            goldens(&path, found);
        } else if path.to_string_lossy().ends_with(".replay.json") {
            found.push(path);
        }
    }
}

fn respond(call: &serde_json::Value) -> String {
    let request = call["request"].as_str().expect("request text");
    let finalized = call["finalized"].as_bool().expect("finalized flag");
    if call["loaded"].as_bool().expect("loaded flag") {
        kassiber_custody::custody_loaded_with_fusion(request, finalized, kassiber_tax_core::ledger)
    } else {
        kassiber_custody::custody_with_fusion(request, finalized, kassiber_tax_core::ledger)
    }
}

/// The first differing byte offset, or `None` when equal.
fn difference(expected: &str, actual: &str) -> Option<usize> {
    if expected == actual {
        return None;
    }
    Some(
        expected
            .bytes()
            .zip(actual.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(expected.len().min(actual.len())),
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let self_test = args.iter().any(|a| a == "--self-test");
    let Some(dir) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: custody-replay [--self-test] DIR");
        return ExitCode::from(2);
    };
    let write = std::env::var("KASSIBER_GOLDEN_WRITE").is_ok_and(|v| v == "1");
    if write && std::env::var_os("CI").is_some() {
        eprintln!("KASSIBER_GOLDEN_WRITE is refused under CI");
        return ExitCode::from(2);
    }
    let mut files = Vec::new();
    goldens(Path::new(dir), &mut files);
    if files.is_empty() {
        eprintln!("{dir}: no *.replay.json goldens");
        return ExitCode::from(2);
    }
    let (mut calls, mut problems) = (0usize, Vec::new());
    for (index, file) in files.iter().enumerate() {
        let text = fs::read_to_string(file).expect("golden");
        let mut golden: serde_json::Value = serde_json::from_str(&text).expect("golden JSON");
        assert_eq!(
            golden["schema"],
            "kassiber.custody-replay/1",
            "{}",
            file.display()
        );
        if write {
            let mut changed = 0;
            for call in golden["calls"].as_array_mut().expect("calls") {
                let actual = respond(call);
                if call["response"].as_str() != Some(actual.as_str()) {
                    call["response"] = serde_json::Value::String(actual);
                    changed += 1;
                }
                calls += 1;
            }
            if changed > 0 {
                let mut out = serde_json::to_string(&golden).expect("golden JSON");
                out.push('\n');
                fs::write(file, out).expect("rewrite golden");
                problems.push(format!("{}: {changed} responses rewritten", file.display()));
            }
            continue;
        }
        for call in golden["calls"].as_array().expect("calls") {
            let mut expected = call["response"].as_str().expect("response text").to_owned();
            if self_test && index == 0 && calls == 0 {
                // A changed byte must be reported, or the replay proves nothing.
                let flipped = if expected.ends_with('}') { "]" } else { "}" };
                expected.pop();
                expected.push_str(flipped);
                if difference(&expected, &respond(call)).is_none() {
                    eprintln!("self-test: a changed golden was not noticed");
                    return ExitCode::from(1);
                }
                println!("self-test: a changed golden is reported");
                expected = call["response"].as_str().unwrap().to_owned();
            }
            calls += 1;
            if let Some(at) = difference(&expected, &respond(call)) {
                problems.push(format!(
                    "{}: call {} differs at byte {at}",
                    file.display(),
                    calls
                ));
            }
        }
    }
    if write {
        println!(
            "{} recordings, {calls} calls, {} goldens rewritten",
            files.len(),
            problems.len()
        );
        for problem in &problems {
            println!("{problem}");
        }
        return ExitCode::SUCCESS;
    }
    println!(
        "{} recordings, {calls} calls replayed, {} differences",
        files.len(),
        problems.len()
    );
    for problem in problems.iter().take(50) {
        println!("{problem}");
    }
    if problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

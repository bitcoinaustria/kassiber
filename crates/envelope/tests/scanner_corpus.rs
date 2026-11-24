//! The secret floor against Python's output (`scripts/gen_scanner_vectors.py`).
//!
//! `fixture_corpora_match_python` checks every case of the shared fixture
//! corpora. `generator_corpora_match_python` checks the six reviewer
//! generators' cases, written outside the tree because they are large, and
//! hashes the output as the Python tests do so pinned seeds reproduce the
//! pinned digests.

use kassiber_envelope::get;
use kassiber_envelope::redaction::{redact_secret_py, redact_secret_text, redact_secret_value};
use kassiber_pyjson::{Key, PyStr, Value};
use kassiber_pytext::loads;

#[path = "support/sha256.rs"]
mod sha256;
use sha256::Sha256;

fn py_text(value: &Value) -> PyStr {
    match value {
        Value::Str(s) => s.clone(),
        other => panic!("expected a string, got {other:?}"),
    }
}

#[test]
fn fixture_corpora_match_python() {
    let mut counts = std::collections::BTreeMap::new();
    let mut mismatches = Vec::new();
    for (number, line) in include_str!("data/secret_floor.txt").lines().enumerate() {
        let case = loads(line).unwrap();
        let set = py_text(get(&case, "set").unwrap())
            .as_str()
            .unwrap()
            .to_owned();
        let input = py_text(get(&case, "in").unwrap());
        let expected = py_text(get(&case, "out").unwrap());
        *counts.entry(set.clone()).or_insert(0usize) += 1;
        let output = redact_secret_py(&input);
        if output != expected {
            mismatches.push(format!(
                "line {} ({set}): {input:?}\n  python {expected:?}\n  rust   {output:?}",
                number + 1
            ));
        }
        let wrapped = Value::Dict(vec![(Key::from("message"), Value::Str(input.clone()))]);
        let expected_value = Value::Dict(vec![(Key::from("message"), Value::Str(expected))]);
        assert_eq!(
            redact_secret_value(&wrapped),
            expected_value,
            "line {}",
            number + 1
        );
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches[..mismatches.len().min(20)].join("\n")
    );
    let expected: std::collections::BTreeMap<String, usize> = [
        ("cases", 444),
        ("5e_cases", 215),
        ("5f_cases", 135),
        ("5g_cases", 76),
        ("5h_cases", 84),
        ("5i_cases", 133),
    ]
    .into_iter()
    .map(|(set, count)| (set.to_owned(), count))
    .collect();
    assert_eq!(counts, expected);
}

#[test]
fn oversized_records_are_masked_whole() {
    let limit = kassiber_envelope::redaction::scanner::REDACTION_LIMIT;
    assert_eq!(redact_secret_text(&"x".repeat(limit)), "x".repeat(limit));
    let long = format!("password=Pw9Leak {}", "x".repeat(limit));
    assert_eq!(redact_secret_text(&long), "[redacted-oversized]");
}

#[test]
#[ignore = "needs KASSIBER_REDACTION_CORPUS from scripts/gen_scanner_vectors.py generators"]
fn generator_corpora_match_python() {
    let directory = std::path::PathBuf::from(
        std::env::var("KASSIBER_REDACTION_CORPUS")
            .expect("KASSIBER_REDACTION_CORPUS names the corpus directory"),
    );
    let manifest =
        loads(&std::fs::read_to_string(directory.join("manifest.json")).unwrap()).unwrap();
    let Value::Dict(files) = manifest else {
        panic!("manifest is not an object");
    };
    assert_eq!(
        files.len(),
        6 * 3 + 9,
        "every generator's pinned, extra and added seeds"
    );
    let mut report = Vec::new();
    for (name, meta) in &files {
        let Key::Str(name) = name else {
            panic!("file names are strings");
        };
        let name = name.as_str().unwrap().to_owned();
        let pinned = match get(meta, "pinned_sha256") {
            Some(Value::Str(digest)) => Some(digest.as_str().unwrap().to_owned()),
            _ => None,
        };
        let python = py_text(get(meta, "python_sha256").unwrap())
            .as_str()
            .unwrap()
            .to_owned();
        let text = std::fs::read_to_string(directory.join(&name)).unwrap();
        let mut digest = Sha256::new();
        let mut cases = 0usize;
        let mut mismatches = Vec::new();
        for line in text.lines() {
            let case = loads(line).unwrap();
            let input = py_text(get(&case, "in").unwrap());
            let expected = py_text(get(&case, "out").unwrap());
            let output = redact_secret_py(&input);
            if output != expected && mismatches.len() < 10 {
                mismatches.push(format!(
                    "{input:?}\n  python {expected:?}\n  rust   {output:?}"
                ));
            }
            digest.update(
                output
                    .as_str()
                    .expect("outputs hold no surrogates")
                    .as_bytes(),
            );
            digest.update(b"\n");
            cases += 1;
        }
        let rust = digest.hex();
        assert!(
            mismatches.is_empty(),
            "{name}: mismatches\n{}",
            mismatches.join("\n")
        );
        assert_eq!(rust, python, "{name}: digest differs from Python's");
        if let Some(pinned) = &pinned {
            assert_eq!(&rust, pinned, "{name}: digest differs from the pinned one");
        }
        report.push(format!(
            "{name}: {cases} cases identical{}",
            if pinned.is_some() {
                ", pinned digest"
            } else {
                ""
            }
        ));
    }
    println!("{}", report.join("\n"));
}

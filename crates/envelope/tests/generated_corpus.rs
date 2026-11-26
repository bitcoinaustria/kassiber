//! The generated redaction corpus through the secret floor: 100,000 cases built from
//! `tests/fixtures/secret_redaction_generator.json` (`support/generated_cases.rs`). No case's
//! sentinel may survive, even as a four-character fragment; the non-secret text and the help
//! line must; and the outputs hash (one line each) to the digest Python produced. Ported from
//! the removed web app's `secretRedactionGenerated.test.ts`; the desktop's log floor runs the
//! same corpus (`crates/kassiber-desktop/src/reports/diag/redact/corpus_tests.rs`).

use kassiber_envelope::redaction::redact_secret_text;

#[path = "support/generated_cases.rs"]
mod generated_cases;
#[path = "support/sha256.rs"]
mod sha256;

const SPEC: &str = include_str!("../../../tests/fixtures/secret_redaction_generator.json");

#[test]
fn generated_corpus_floors_as_python_and_keeps_non_secrets() {
    let spec: serde_json::Value = serde_json::from_str(SPEC).unwrap();
    let mut digest = sha256::Sha256::new();
    let (mut cases, mut failures, mut failed) = (0usize, Vec::new(), 0usize);
    generated_cases::for_each_case(&spec, |case| {
        let output = redact_secret_text(&case.input);
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
    assert_eq!(digest.hex(), spec["output_sha256"].as_str().unwrap());
}

#[test]
fn quoting_follows_json_stringify() {
    // Control characters escape (short forms where JSON has them); DEL and non-ASCII stay raw.
    assert_eq!(
        generated_cases::quoted("a\"b\\c\n\t\u{1}\u{7f}é"),
        "\"a\\\"b\\\\c\\n\\t\\u0001\u{7f}é\""
    );
}

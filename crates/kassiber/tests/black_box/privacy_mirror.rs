//! The registered Privacy Mirror report, `reports privacy-mirror` and
//! `ui.reports.privacy_mirror`, through cold, warm and incremental
//! publications with the product offline switch off and on and the test
//! egress guard armed. Replaces the kill-holding tests of
//! `tests/test_rust_privacy_mirror_process.py`.

use serde_json::json;

use crate::support::{number_hex, Sandbox, Transcript};

const BOOK: &str = include_str!("../../../reports/tests/data/book.sql");
const STORED: &str = include_str!("sql/mirror_stored.sql");
const INVENTORY: &str = include_str!("sql/mirror_inventory.sql");
const DATASET: &str = include_str!("sql/mirror_dataset.sql");

enum Step {
    Cli(&'static [&'static str]),
    Daemon,
}

const STEPS: &[(Option<&str>, Step)] = &[
    (None, Step::Cli(&["--machine", "reports", "privacy-mirror"])),
    (None, Step::Cli(&["--machine", "reports", "privacy-mirror"])),
    (
        None,
        Step::Cli(&["--format", "table", "reports", "privacy-mirror"]),
    ),
    (
        None,
        Step::Cli(&["--format", "plain", "reports", "privacy-mirror"]),
    ),
    (
        None,
        Step::Cli(&["--format", "csv", "reports", "privacy-mirror"]),
    ),
    (
        None,
        Step::Cli(&[
            "--machine",
            "reports",
            "privacy-mirror",
            "--workspace",
            "Main",
            "--profile",
            "Book",
        ]),
    ),
    (
        None,
        Step::Cli(&[
            "--machine",
            "reports",
            "privacy-mirror",
            "--profile",
            "absent",
        ]),
    ),
    (None, Step::Daemon),
    (
        Some("UPDATE wallet_utxos SET amount=9000 WHERE id='utxo';"),
        Step::Cli(&["--machine", "reports", "privacy-mirror"]),
    ),
    (
        Some("UPDATE chain_analysis_datasets SET status='revoked';"),
        Step::Daemon,
    ),
];

fn book(sql: &str, offline: bool) -> Sandbox {
    let mut sandbox = Sandbox::book(&format!("{BOOK}{sql}"));
    sandbox
        .set("KASSIBER_RUST_TEST_ANALYSIS_IDS", "sequential")
        .set("KASSIBER_RUST_TEST_ANALYSIS_TIME", "2026-01-01T00:00:00Z");
    if offline {
        sandbox.set("KASSIBER_NO_EGRESS", "1");
    }
    sandbox
}

/// Run `steps`, recording each output and the rows it changed.
fn run(t: &mut Transcript, sandbox: &Sandbox, steps: &[&(Option<&str>, Step)]) -> Vec<String> {
    let mut outputs = Vec::new();
    for (change, step) in steps {
        if let Some(sql) = change {
            sandbox.sql(sql);
            t.note(sql);
        }
        let before = sandbox.dump();
        match step {
            Step::Cli(argv) => {
                let out = t.run(sandbox, argv);
                outputs.push(out.text());
            }
            Step::Daemon => {
                let args = [
                    None,
                    Some(json!(null)),
                    Some(json!({})),
                    Some(json!("x")),
                    Some(json!({"wallet": "cold", "redacted": false})),
                ];
                let requests: Vec<_> = args
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| {
                        let mut request = json!({"request_id": (index + 1).to_string(),
                            "kind": "ui.reports.privacy_mirror"});
                        if let Some(arg) = arg {
                            request["args"] = arg.clone();
                        }
                        request
                    })
                    .collect();
                let responses = t.daemon(sandbox, &requests);
                outputs.push(serde_json::to_string(&responses).unwrap());
            }
        }
        t.changes(sandbox, &before, "after the step");
    }
    outputs
}

fn transcript(name: &str) -> Transcript {
    let mut t = Transcript::new(name);
    // Redacted handles are keyed by a per-process secret.
    t.mask(|text| number_hex(text, "ca-ref:", 64));
    t
}

#[test]
fn test_owned_book_with_claims_matches_python_offline_off_and_on() {
    let mut t = transcript("privacy_mirror/owned_book_with_claims");
    for offline in [false, true] {
        t.note(&format!("offline={offline}"));
        let sandbox = book(&format!("{STORED}{INVENTORY}{DATASET}"), offline);
        let outputs = run(&mut t, &sandbox, &STEPS.iter().collect::<Vec<_>>());
        let report: serde_json::Value = serde_json::from_str(&outputs[0]).unwrap();
        assert_eq!(report["kind"], "reports.privacy-mirror");
        assert!(report["data"]["summary"]["finding_count"].as_i64().unwrap() >= 1);
        assert!(report["data"]["entropy"]["evaluated"].as_i64().unwrap() >= 1);
        assert_eq!(report["data"]["redaction"], "ai_export_safe");
        assert!(outputs[0].contains("ca-ref:"));
    }
    t.finish();
}

#[test]
fn test_empty_and_stored_books_match_python() {
    let mut t = transcript("privacy_mirror/empty_and_stored_books");
    for sql in ["", STORED] {
        t.note(&format!("stored={}", !sql.is_empty()));
        let sandbox = book(sql, true);
        let steps: Vec<_> = STEPS[..2].iter().chain(&STEPS[7..8]).collect();
        run(&mut t, &sandbox, &steps);
    }
    t.finish();
}

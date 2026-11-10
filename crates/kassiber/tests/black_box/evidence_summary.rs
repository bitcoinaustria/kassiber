//! `ui.audit.evidence.summary` across the evidence book. Replaces the
//! kill-holding test of `tests/test_rust_evidence_summary.py`.

use serde_json::json;

use crate::desktop::{self, compare, evidence};
use crate::support::{Sandbox, Transcript};

/// The attachments root sits beside `data/` in the book's state root.
pub fn place_files(sandbox: &Sandbox) {
    let attachments = sandbox.path("attachments/p");
    std::fs::create_dir_all(&attachments).unwrap();
    std::fs::write(attachments.join("present.pdf"), b"%PDF").unwrap();
}

#[test]
fn test_whole_book_and_option_variants_match_wire_bytes() {
    let sandbox = desktop::book();
    place_files(&sandbox);
    sandbox.sql(&evidence());
    let mut t = Transcript::new("evidence_summary/whole_book_and_options");
    let responses = compare(
        &mut t,
        &sandbox,
        "ui.audit.evidence.summary",
        &[
            None,
            Some(json!(null)),
            Some(json!({})),
            Some(json!({"unknown": true})),
            Some(json!({"include_journal_state": false})),
            Some(json!({"include_review_state": false})),
            Some(json!({"include_edit_history": true})),
            Some(
                json!({"include_journal_state": false, "include_review_state": false,
                "include_edit_history": true}),
            ),
        ],
    );
    let data = &responses[1]["data"];
    assert!(data["summary"].get("filed_report_snapshot_count").is_some());
    let mut exists = std::collections::BTreeMap::new();
    for item in data["transactions"].as_array().unwrap() {
        for attachment in item["direct_attachments"].as_array().unwrap() {
            exists.insert(
                attachment["id"].as_str().unwrap().to_owned(),
                attachment["exists"].clone(),
            );
        }
    }
    assert_eq!(exists["at-file"], true, "the managed file is found");
    assert_eq!(exists["at-missing"], false);
    t.finish();
}

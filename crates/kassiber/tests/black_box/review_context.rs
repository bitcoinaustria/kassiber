//! `ui.transactions.review_context` across the evidence book with loans.
//! Replaces the kill-holding test of
//! `tests/test_rust_transaction_review_context.py`.

use serde_json::json;

use crate::desktop::{self, compare, evidence, some, LOANS};
use crate::support::Transcript;

#[test]
fn test_review_bundles_match_wire_bytes() {
    let sandbox = desktop::book();
    let attachments = sandbox.path("state/attachments/p");
    std::fs::create_dir_all(&attachments).unwrap();
    std::fs::write(attachments.join("present.pdf"), b"%PDF").unwrap();
    sandbox.sql(&format!("{}{LOANS}", evidence()));
    let mut t = Transcript::new("review_context/review_bundles");
    let mut args: Vec<_> = ["h", " i ", "j", "k", "l", "m", "a", "c"]
        .iter()
        .map(|reference| json!({"transaction": reference}))
        .collect();
    args.extend([
        json!({"transaction": "h", "history_limit": 1, "include_graph": false,
            "include_privacy": false, "include_evidence": false}),
        json!({"transaction": "i", "history_limit": "99"}),
        json!({"transaction": "j", "history_limit": 2.7}),
    ]);
    let responses = compare(
        &mut t,
        &sandbox,
        "ui.transactions.review_context",
        &some(args),
    );
    assert_eq!(responses[0]["data"]["safety"]["network_contacted"], false);
    t.finish();
}

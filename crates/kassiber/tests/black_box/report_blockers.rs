//! `ui.report.blockers` on the barrier book. Replaces the kill-holding
//! tests of `tests/test_rust_report_blockers.py`.

use serde_json::json;

use crate::desktop::{self, barriers, compare, some};
use crate::support::Transcript;

const KIND: &str = "ui.report.blockers";

#[test]
fn test_every_barrier_matches_wire_bytes() {
    let sandbox = desktop::book();
    sandbox.sql(&barriers());
    let mut t = Transcript::new("report_blockers/every_barrier");
    let responses = compare(&mut t, &sandbox, KIND, &some(vec![json!({})]));
    let ids: Vec<&str> = responses[0]["data"]["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids[..2], ["sync_failed", "custody_component_integrity"]);
    for expected in [
        "source_overlap",
        "journal_quarantine",
        "custody_quantity_unresolved",
        "ownership_transfer_review",
        "unreviewed_swap_candidates",
        "missing_prices",
        "silent_payment_scan_pending:sp-pending",
        "silent_payment_scan_degraded:sp-degraded",
        "silent_payment_scan_degraded:sp-blocking",
    ] {
        assert!(ids.contains(&expected), "{expected} in {ids:?}");
    }
    assert!(!ids.contains(&"silent_payment_scan_degraded:sp-done"));
    t.finish();
}

#[test]
fn test_cached_counts_and_partial_barriers_match_wire_bytes() {
    let sandbox = desktop::book();
    sandbox.sql(&barriers());
    let mut t = Transcript::new("report_blockers/cached_counts_and_partial_barriers");
    for update in [
        "UPDATE profiles SET ownership_review_counts_json='{\"total\": \"2\", \"by_reason\": []}' WHERE id='p';",
        "UPDATE profiles SET ownership_review_counts_json='{\"total\": 1, \"by_reason\": {\"a\": null}}' WHERE id='p';",
        "UPDATE profiles SET ownership_review_counts_json='[1]' WHERE id='p';",
        "UPDATE profiles SET ownership_review_counts_json='{\"total\": 0}' WHERE id='p';",
        "UPDATE profiles SET ownership_review_counts_json='bad' WHERE id='p';",
        "DELETE FROM journal_quantity_issues WHERE amount_msat IS NOT NULL;",
        "DELETE FROM journal_quantity_issues WHERE issue_id <> 'q1';",
        "DELETE FROM freshness_source_states; DELETE FROM transaction_edit_events;",
        "UPDATE custody_components SET state='superseded',superseded_at='2025';",
        "DELETE FROM journal_quantity_postings; DELETE FROM journal_quarantines;",
    ] {
        sandbox.sql(update);
        t.note(update);
        compare(&mut t, &sandbox, KIND, &some(vec![json!({})]));
    }
    t.finish();
}

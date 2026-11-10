//! `ui.maintenance.run`. Replaces the kill-holding tests of
//! `tests/test_rust_maintenance_run.py`.

use serde_json::json;

use crate::desktop::{self, compare, mutating, some, ACTIVITY, CURRENT};
use crate::support::Transcript;

const KIND: &str = "ui.maintenance.run";
const STALE: &str = "UPDATE profiles SET journal_input_version=3 WHERE id='p';";

#[test]
fn test_refusals_leave_every_table_unchanged() {
    let sandbox = desktop::book();
    sandbox.sql(&format!("{ACTIVITY}{CURRENT}{STALE}"));
    let mut t = Transcript::new("maintenance_run/refusals");
    let responses = compare(
        &mut t,
        &sandbox,
        KIND,
        &some(vec![
            json!([]),
            json!("x"),
            json!(3),
            json!({"z": 1, "a": 2}),
            json!({"sync": "never", "x": 1}),
            json!({"sync": "bad"}),
            json!({"sync": null}),
            json!({"sync": 1}),
            json!({"sync": true}),
            json!({"sync": " never"}),
            json!({"sync": "NEVER"}),
        ]),
    );
    assert!(
        responses.iter().all(|r| r["kind"] == "error"),
        "{responses:?}"
    );
    t.finish();
}

#[test]
fn test_unhashable_sync_is_the_typed_validation_refusal() {
    let sandbox = desktop::book();
    sandbox.sql(&format!("{ACTIVITY}{CURRENT}{STALE}"));
    let mut t = Transcript::new("maintenance_run/unhashable_sync");
    let responses = compare(
        &mut t,
        &sandbox,
        KIND,
        &some(vec![
            json!({"sync": []}),
            json!({"sync": {"a": 1}}),
            json!({"sync": ["never"]}),
        ]),
    );
    let errors: Vec<_> = responses
        .iter()
        .map(|r| (r["error"]["code"].clone(), r["error"]["details"].clone()))
        .collect();
    assert_eq!(
        errors,
        [
            (json!("validation"), json!({"sync": []})),
            (json!("validation"), json!({"sync": {"a": 1}})),
            (json!("validation"), json!({"sync": ["never"]})),
        ]
    );
    t.finish();
}

#[test]
fn test_journals_of_other_semantics_are_rebuilt() {
    // Counts and input versions match; only the calculation semantics the
    // journals were built with is not the required one, which freshness
    // calls stale. The rebuild meets the mixed-network book.
    let sandbox = desktop::book();
    sandbox.sql(&format!(
        "{ACTIVITY}{CURRENT}UPDATE profiles SET journal_semantics_version=1,\
         required_journal_semantics_version=1 WHERE id='p';"
    ));
    let mut t = Transcript::new("maintenance_run/journals_of_other_semantics");
    let responses = mutating(
        &mut t,
        &sandbox,
        KIND,
        &some(vec![json!({"sync": "never"})]),
    );
    assert_eq!(
        responses[0]["error"]["code"],
        "book_network_review_required"
    );
    t.finish();
}

#[test]
fn test_stale_journals_are_processed_and_reported() {
    let sandbox = desktop::book_with(&desktop::capital_book());
    sandbox.sql("UPDATE profiles SET journal_input_version=8 WHERE id='p';");
    let mut t = Transcript::new("maintenance_run/stale_journals_processed");
    let responses = mutating(
        &mut t,
        &sandbox,
        KIND,
        &some(vec![json!({"sync": "never"}), json!({})]),
    );
    assert!(responses[0]["data"]["maintenance"]
        .get("journals")
        .is_some());
    assert!(responses[1]["data"]["maintenance"]
        .get("journals")
        .is_none());
    t.finish();
}

//! Report-read refresh: the wallet refresh a book's freshness policy
//! enables before report reads and maintenance runs. The only source is a
//! regtest descriptor wallet whose default backend serves mainnet, so each
//! refresh job fails before any request. Replaces the kill-holding tests of
//! `tests/test_rust_report_read_sync.py`.

use serde_json::json;

use crate::desktop::{self, mutating, some, SYNC_BOOK};
use crate::support::{mask_number, Sandbox, Transcript};

const ON: &str = r#"{"report_read_sync": true, "source_classes": {"onchain_wallet": true}}"#;

fn book(policy: &str) -> Sandbox {
    let sandbox = desktop::book_with(SYNC_BOOK);
    if !policy.is_empty() {
        sandbox.sql(&format!(
            "INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p','{policy}');"
        ));
    }
    sandbox
}

fn transcript(name: &str) -> Transcript {
    let mut t = Transcript::new(name);
    t.mask(|text| mask_number(text, "retry_after_seconds"));
    t
}

#[test]
fn test_policy_without_source_classes_runs_nothing() {
    let sandbox = book(r#"{"report_read_sync": true}"#);
    let mut t = transcript("report_read_sync/policy_without_source_classes");
    let responses = mutating(
        &mut t,
        &sandbox,
        "ui.report.blockers",
        &some(vec![json!({}), json!({})]),
    );
    assert!(responses.iter().all(|r| r["kind"] == "ui.report.blockers"));
    t.finish();
}

#[test]
fn test_failed_refresh_blocks_and_is_rate_limited() {
    let sandbox = book(ON);
    let mut t = transcript("report_read_sync/failed_refresh_rate_limited");
    let responses = mutating(
        &mut t,
        &sandbox,
        "ui.report.blockers",
        &some(vec![json!({}), json!({}), json!(null)]),
    );
    assert_eq!(responses[0]["data"]["blockers"][0]["id"], "sync_failed");
    t.finish();
}

#[test]
fn test_maintenance_runs_sync_modes() {
    let sandbox = book(ON);
    let mut t = transcript("report_read_sync/maintenance_sync_modes");
    let responses = mutating(
        &mut t,
        &sandbox,
        "ui.maintenance.run",
        &some(vec![
            json!({"sync": "if_enabled"}),
            json!({"sync": "if_enabled"}),
            json!({"sync": "always"}),
            json!({"sync": "never"}),
        ]),
    );
    assert!(responses[0]["data"]["maintenance"].get("sync").is_some());
    sandbox.sql("DELETE FROM settings WHERE key LIKE 'freshness.policy.%';");
    mutating(
        &mut t,
        &sandbox,
        "ui.maintenance.run",
        &some(vec![
            json!({"sync": "if_enabled"}),
            json!({"sync": "always"}),
        ]),
    );
    t.finish();
}

//! Books from before authored custody components open in the shipped
//! binary: its migrating open runs the custody steps (legacy pair/payout
//! consolidation, the legacy pair fee review) that the store hands to the
//! custody layer. The exact rows are pinned by
//! `crates/transfers/tests/legacy_migrations.rs`; this proves the binary
//! installs that layer for its command paths.
use super::support::*;
use std::io::Read;

fn legacy_book(name: &str) -> Sandbox {
    let sandbox = Sandbox::empty();
    let path = repo().join(format!(
        "crates/transfers/tests/data/legacy_migrations/{name}.input.sqlite3.gz"
    ));
    let mut book = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&path).unwrap())
        .read_to_end(&mut book)
        .unwrap();
    std::fs::create_dir_all(&sandbox.root).unwrap();
    std::fs::write(sandbox.root.join("kassiber.sqlite3"), book).unwrap();
    sandbox
}

#[test]
fn legacy_reviews_migrate_on_the_first_command() {
    let sandbox = legacy_book("scenario-mixed");
    let status = sandbox.machine(&["status"]).json();
    assert_eq!(status["kind"], "status", "{status}");
    let rows = |sql: &str| sandbox.query_strings(sql);
    // Every reachable legacy review is linked to a component or held by a
    // migration issue (a journal quarantine); the fee review ran once.
    assert_eq!(
        rows("SELECT id FROM transaction_pairs WHERE component_id IS NULL ORDER BY id"),
        ["amb-1", "amb-2", "big"]
    );
    assert_eq!(
        rows("SELECT legacy_source_id FROM custody_authored_migration_issues WHERE resolved_at IS NULL ORDER BY legacy_source_id"),
        ["amb-1", "amb-2", "big", "po-deleted", "po-negative"]
    );
    assert_eq!(
        rows("SELECT migration_name FROM schema_migration_audits WHERE migration_name='custody-legacy-pair-fee-review-v1'"),
        ["custody-legacy-pair-fee-review-v1"]
    );
    // A second command retries only the unresolved issues, as Python does:
    // the same issues, stamped again (unchanged within the same second).
    let before = sandbox.dump();
    assert_eq!(sandbox.machine(&["status"]).json()["kind"], "status");
    let after = sandbox.dump();
    let changed: Vec<_> = before
        .lines()
        .zip(after.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(before.lines().count(), after.lines().count());
    assert!(changed.len() <= 5, "{changed:#?}");
    for (a, b) in changed {
        assert!(a.starts_with("custody_authored_migration_issues:"), "{a}");
        let strip = |line: &str| line.split(" | updated_at=").next().unwrap().to_owned();
        assert_eq!(strip(a), strip(b));
    }
}

#[test]
fn legacy_pair_fee_books_are_reviewed_and_invalidated_on_open() {
    let sandbox = legacy_book("historical-pre_pair_fee_c0a015de");
    let status = sandbox.machine(&["status"]).json();
    assert_eq!(status["kind"], "status", "{status}");
    let impact = sandbox.query_strings(
        "SELECT impact_json FROM schema_migration_audits WHERE migration_name='custody-legacy-pair-fee-review-v1'",
    );
    let impact: serde_json::Value = serde_json::from_str(&impact[0]).unwrap();
    assert_eq!(impact["affected_component_count"], 1);
    let required = sandbox
        .query_strings("SELECT CAST(required_journal_semantics_version AS TEXT) FROM profiles");
    assert!(
        required.iter().all(|v| v.parse::<i64>().unwrap() >= 7),
        "{required:?}"
    );
}

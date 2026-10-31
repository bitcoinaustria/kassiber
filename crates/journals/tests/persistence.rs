mod support;
use kassiber_store::rusqlite::Connection;
use serde_json::Value;
use support::*;

fn cases() -> Vec<Value> {
    let mut cases: Vec<Value> =
        serde_json::from_str(include_str!("data/persistence.json")).unwrap();
    cases.extend(
        serde_json::from_str::<Vec<Value>>(include_str!("data/persistence-stages.json")).unwrap(),
    );
    cases
}
fn equal(c: &Connection, expected: &Value, label: &Value) {
    let actual = snapshot(c);
    for (table, value) in expected.as_object().unwrap() {
        assert!(
            logical_equal(&actual[table], value),
            "{label}: table {table}\nactual: {}\nexpected: {value}",
            actual[table]
        );
    }
    assert_eq!(
        actual.as_object().unwrap().len(),
        expected.as_object().unwrap().len()
    );
}
#[test]
fn journal_persistence_matches_complete_python_dumps() {
    for v in cases() {
        let c = Connection::open_in_memory().unwrap();
        load(&c, &v["before"]);
        equal(&c, &v["before"], &v["name"]);
        c.execute_batch("BEGIN IMMEDIATE; SAVEPOINT journals_process")
            .unwrap();
        let mut writes = Writes::default();
        let result = store(&c, &v, &mut writes).unwrap();
        assert!(writes.count > 10);
        assert!(!result.processed_at.is_empty());
        c.execute_batch("RELEASE journals_process; COMMIT").unwrap();
        equal(&c, &v["expected"], &v["name"]);
    }
}
#[test]
fn every_write_failure_and_panic_rolls_back_the_journal() {
    for v in cases() {
        let c = Connection::open_in_memory().unwrap();
        load(&c, &v["before"]);
        verify_every_failure(&c, &v);
    }
}
fn verify_every_failure(c: &Connection, v: &Value) {
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut success = Writes::default();
    store(c, v, &mut success).unwrap();
    c.execute_batch("ROLLBACK").unwrap();
    for fail in 1..=success.count {
        for panic in [false, true] {
            c.execute_batch("BEGIN IMMEDIATE;SAVEPOINT journals_process")
                .unwrap();
            let mut writes = Writes {
                fail_at: (!panic).then_some(fail),
                panic_at: panic.then_some(fail),
                ..Default::default()
            };
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| store(c, v, &mut writes)));
            assert!(
                matches!(result, Ok(Err(_)) | Err(_)),
                "{} write {fail}",
                v["name"]
            );
            equal(c, &v["before"], &v["name"]);
            c.execute_batch("ROLLBACK TO journals_process;RELEASE journals_process;ROLLBACK")
                .unwrap();
            equal(c, &v["before"], &v["name"]);
        }
    }
}
#[test]
fn persistence_refuses_without_a_caller_transaction() {
    let v = &cases()[0];
    let c = Connection::open_in_memory().unwrap();
    load(&c, &v["before"]);
    assert!(store(&c, v, &mut Writes::default()).is_err());
    equal(&c, &v["before"], &v["name"]);
}

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/r4-persist")
            .join(format!("{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn sqlcipher_delete_full_preserves_complete_dumps_and_cache_settings() {
    use kassiber_journals::storage::{DeferredCheckpoint, RebuildCache};
    let scratch = Scratch::new("cipher");
    for (index, v) in cases().into_iter().enumerate() {
        let path = scratch.0.join(format!("{index}.sqlite3"));
        let c = Connection::open(&path).unwrap();
        c.pragma_update(None, "key", "synthetic-test-only").unwrap();
        c.execute_batch("PRAGMA journal_mode=DELETE;PRAGMA synchronous=FULL;PRAGMA cache_size=73")
            .unwrap();
        load(&c, &v["before"]);
        if v["name"] == "cipher" {
            verify_every_failure(&c, &v);
        }
        let checkpoint = DeferredCheckpoint::enter(&c).unwrap();
        let cache = RebuildCache::enter(&c).unwrap();
        c.execute_batch("BEGIN IMMEDIATE;SAVEPOINT journals_process")
            .unwrap();
        store(&c, &v, &mut Writes::default()).unwrap();
        c.execute_batch("RELEASE journals_process;COMMIT").unwrap();
        cache.restore().unwrap();
        checkpoint.after_reply().unwrap();
        assert_eq!(
            c.pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))
                .unwrap(),
            "delete"
        );
        assert_eq!(
            c.pragma_query_value(None, "synchronous", |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            c.pragma_query_value(None, "cache_size", |r| r.get::<_, i64>(0))
                .unwrap(),
            73
        );
        equal(&c, &v["expected"], &v["name"]);
        drop(c);
        let reopened = Connection::open(&path).unwrap();
        reopened
            .pragma_update(None, "key", "synthetic-test-only")
            .unwrap();
        equal(&reopened, &v["expected"], &v["name"]);
    }
}

#[test]
fn wal_checkpoint_waits_for_commit_and_retains_active_readers() {
    use kassiber_journals::storage::DeferredCheckpoint;
    let scratch = Scratch::new("wal");
    let path = scratch.0.join("book.sqlite3");
    let c = Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA journal_mode=WAL;PRAGMA wal_autocheckpoint=37;CREATE TABLE witness(n);INSERT INTO witness VALUES(1);PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let reader = Connection::open(&path).unwrap();
    reader.execute_batch("BEGIN;SELECT * FROM witness").unwrap();
    let checkpoint = DeferredCheckpoint::enter(&c).unwrap();
    assert_eq!(
        c.pragma_query_value(None, "wal_autocheckpoint", |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute_batch("BEGIN IMMEDIATE;INSERT INTO witness VALUES(2);COMMIT")
        .unwrap();
    let frames = |c: &Connection| {
        c.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| {
            Ok((r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })
        .unwrap()
    };
    checkpoint.after_reply().unwrap();
    let (written, done) = frames(&c);
    assert!(
        written > done,
        "the pinned reader prevents a complete checkpoint"
    );
    assert_eq!(
        c.pragma_query_value(None, "wal_autocheckpoint", |r| r.get::<_, i64>(0))
            .unwrap(),
        37
    );
    assert_eq!(
        reader
            .query_row("SELECT COUNT(*) FROM witness", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    reader.execute_batch("ROLLBACK").unwrap();
    let checkpoint = DeferredCheckpoint::enter(&c).unwrap();
    checkpoint.after_reply().unwrap();
    let (written, done) = frames(&c);
    assert_eq!(written, done);
    assert!(
        std::fs::metadata(path.with_extension("sqlite3-wal"))
            .unwrap()
            .len()
            > 0,
        "PASSIVE checkpoints preserve the WAL file for reuse"
    );
}

#[test]
fn dump_comparison_preserves_scalar_types_and_every_real_bit() {
    use serde_json::json;
    assert!(logical_equal(
        &serde_json::from_str("1e+22").unwrap(),
        &json!(1e22)
    ));
    assert!(!logical_equal(&json!(1), &json!(1.0)));
    assert!(!logical_equal(&json!(0.0), &json!(-0.0)));
    assert!(!logical_equal(
        &json!(f64::from_bits(1)),
        &json!(f64::from_bits(2))
    ));
}

#[test]
fn repeated_rebuilds_replace_small_tables_and_keep_sealed_resolutions() {
    for v in cases() {
        let c = Connection::open_in_memory().unwrap();
        load(&c, &v["before"]);
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        let first = store(&c, &v, &mut Writes::default()).unwrap();
        let before = snapshot(&c);
        let second = store(&c, &v, &mut Writes::default()).unwrap();
        assert_eq!(second.filed_report_impacts_resolved, 0);
        assert_ne!(first.processed_at, second.processed_at);
        let after = snapshot(&c);
        for (table, expected) in before.as_object().unwrap() {
            if table.starts_with("journal_")
                || matches!(
                    table.as_str(),
                    "custody_filed_report_impact_resolutions" | "filed_report_chain_resolutions"
                )
            {
                assert!(
                    logical_equal(&after[table], expected),
                    "{} {table}",
                    v["name"]
                );
            }
        }
        c.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn absent_quantity_preserves_quantity_rows_and_clears_reconciliations() {
    let mut v = cases().remove(0);
    let c = Connection::open_in_memory().unwrap();
    load(&c, &v["before"]);
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    store(&c, &v, &mut Writes::default()).unwrap();
    let before = snapshot(&c);
    v["record"]["custody"]["quantity_state"] = Value::Null;
    let result = store(&c, &v, &mut Writes::default()).unwrap();
    assert_eq!(result.quantity_counts.values().sum::<usize>(), 0);
    let after = snapshot(&c);
    for table in [
        "journal_quantity_postings",
        "journal_quantity_issues",
        "journal_quantity_balances",
        "journal_custody_decisions",
        "journal_custody_economic_relations",
    ] {
        assert!(logical_equal(&before[table], &after[table]), "{table}");
    }
    assert!(after["journal_custody_reconciliations"]["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    c.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn synthetic_quarantines_are_filtered_before_readiness() {
    use kassiber_journals::{boundary, persistence, QuarantineRow};
    use serde_json::json;
    let v = cases().remove(0);
    let r = &v["record"];
    let c = Connection::open_in_memory().unwrap();
    load(&c, &v["before"]);
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let response = boundary::ledger_response(&r["response"]).unwrap();
    let mut state = boundary::build(
        &r["profile"],
        &r["custody"],
        r["wallets"].as_array().unwrap(),
        &response,
        String::new,
    )
    .unwrap();
    state.quarantines.push(QuarantineRow {
        transaction_id: json!("synthetic:unknown"),
        workspace_id: json!("w"),
        profile_id: json!("p"),
        reason: "root".into(),
        detail_json: "{}".into(),
    });
    let mut ids = v["ids"].as_array().unwrap().iter();
    let result = persistence::store_ledger_state(
        &c,
        &r["profile"],
        &state,
        v["created_at"].as_str().unwrap(),
        &mut || Ok(ids.next().unwrap().as_str().unwrap().into()),
        &mut Writes::default(),
    )
    .unwrap();
    assert!(result.quarantines.is_empty());
    equal(&c, &v["expected"], &v["name"]);
    c.execute_batch("ROLLBACK").unwrap();
}

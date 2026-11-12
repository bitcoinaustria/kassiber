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

#[test]
fn binding_refusals_precede_decimal_formatting_and_roll_back_every_write() {
    use kassiber_journals::JournalDecimal;
    let v = &cases()[0];
    let c = Connection::open_in_memory().unwrap();
    load(&c, &v["before"]);
    for (bad_metadata, expected) in [(true, "ProgrammingError"), (false, "OverflowError")] {
        c.execute_batch("BEGIN IMMEDIATE;SAVEPOINT journals_process")
            .unwrap();
        let result = store_with(&c, v, &mut Writes::default(), |state| {
            let entry = &mut state.entries[0];
            if bad_metadata {
                entry.workspace_id = state.custody_quantity;
            }
            entry.quantity = JournalDecimal("1E+20".parse().unwrap()).into();
            entry.fiat_value = JournalDecimal("1E+100000001".parse().unwrap());
        });
        assert_eq!(result.unwrap_err().family, expected);
        equal(&c, &v["before"], &v["name"]);
        c.execute_batch("ROLLBACK TO journals_process;RELEASE journals_process;ROLLBACK")
            .unwrap();
    }
}

#[test]
fn nonfinite_entry_quantities_refuse_and_restore_every_table() {
    use kassiber_journals::{boundary, persistence, JournalQuantity, NonFiniteDecimal};
    let v = &cases()[0];
    let record = &v["record"];
    let response = boundary::ledger_response(&record["response"]).unwrap();
    for (text, family, message) in [
        (
            NonFiniteDecimal::PositiveInfinity,
            "OverflowError",
            "cannot convert Infinity to integer",
        ),
        (
            NonFiniteDecimal::NegativeInfinity,
            "OverflowError",
            "cannot convert Infinity to integer",
        ),
        (
            NonFiniteDecimal::NaN,
            "ValueError",
            "cannot convert NaN to integer",
        ),
    ] {
        let mut state = boundary::build(
            &record["profile"],
            &record["custody"],
            record["wallets"].as_array().unwrap(),
            &response,
            String::new,
        )
        .unwrap();
        state.entries[0].quantity = JournalQuantity::NonFinite { text };
        for encrypted in [false, true] {
            let c = Connection::open_in_memory().unwrap();
            if encrypted {
                c.pragma_update(None, "key", "synthetic-test-only").unwrap();
            }
            load(&c, &v["before"]);
            c.execute_batch("BEGIN IMMEDIATE").unwrap();
            let error = persistence::store_ledger_state(
                &c,
                &record["profile"],
                &state,
                v["created_at"].as_str().unwrap(),
                &mut || Ok(String::new()),
                &mut Writes::default(),
            )
            .unwrap_err();
            assert_eq!(error.family, family);
            assert_eq!(error.message, message);
            equal(&c, &v["before"], &v["name"]);
            c.execute_batch("ROLLBACK").unwrap();
        }
    }
}

#[test]
fn typed_entry_negative_zero_forces_rewrites_in_all_five_real_columns() {
    use kassiber_journals::{storage::journal_entry_id, JournalDecimal};
    for column in 0..5 {
        let mut v = cases()[0].clone();
        let c = Connection::open_in_memory().unwrap();
        load(&c, &v["before"]);
        for clock in ["2026-10-31T12:00:00Z", "2026-11-01T12:00:00Z"] {
            v["created_at"] = serde_json::json!(clock);
            c.execute_batch("BEGIN IMMEDIATE;SAVEPOINT journals_process")
                .unwrap();
            let mut writes = Writes::default();
            let mut target = None;
            store_with(&c, &v, &mut writes, |state| {
                let e = &mut state.entries[0];
                target = Some(
                    journal_entry_id(
                        e.profile_id.as_str().unwrap(),
                        e.transaction_id.as_str().unwrap(),
                        e.wallet_id.as_str().unwrap(),
                        e.entry_type,
                        &e.asset,
                        0,
                    )
                    .unwrap(),
                );
                let zero = JournalDecimal("-0.000".parse().unwrap());
                match column {
                    0 => e.fiat_value = zero,
                    1 => e.unit_cost = zero,
                    2 => e.cost_basis = Some(zero),
                    3 => e.proceeds = Some(zero),
                    4 => e.gain_loss = Some(zero),
                    _ => unreachable!(),
                }
            })
            .unwrap();
            let target = target.unwrap();
            assert!(
                writes.tables.contains(&"journal_entries"),
                "column {column}"
            );
            let (created, siblings): (String, i64) = c.query_row(
                "SELECT created_at,(SELECT COUNT(*) FROM journal_entries AS sibling WHERE sibling.occurred_at=entry.occurred_at) FROM journal_entries AS entry WHERE id=?",
                [&target], |r| Ok((r.get(0)?, r.get(1)?)),
            ).unwrap();
            assert_eq!(
                siblings, 1,
                "an unrelated sibling rewrite must not mask the sign check"
            );
            assert_eq!(created, clock, "column {column}");
            c.execute_batch("RELEASE journals_process;COMMIT").unwrap();
        }
    }
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
fn complete_dumps_round_trip_infinite_reals_without_null_substitution() {
    let source = Connection::open_in_memory().unwrap();
    let sql = "CREATE TABLE samples(amount REAL)";
    source.execute_batch(sql).unwrap();
    for value in [Some(f64::INFINITY), Some(f64::NEG_INFINITY), None] {
        source
            .execute("INSERT INTO samples(amount) VALUES(?)", [value])
            .unwrap();
    }
    let restored = Connection::open_in_memory().unwrap();
    load_schema(
        &restored,
        &snapshot(&source),
        &serde_json::json!([["table", "samples", "samples", sql]]),
    );
    let actual = restored
        .prepare("SELECT typeof(amount),amount FROM samples ORDER BY rowid")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<f64>>(1)?.map(f64::to_bits),
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        actual,
        vec![
            ("real".into(), Some(f64::INFINITY.to_bits())),
            ("real".into(), Some(f64::NEG_INFINITY.to_bits())),
            ("null".into(), None),
        ]
    );
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

#[test]
fn malformed_entry_and_holding_bindings_roll_back_before_freshness() {
    let v = cases().remove(0);
    for mode in 0..6 {
        let c = Connection::open_in_memory().unwrap();
        load(&c, &v["before"]);
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        let result = store_with(&c, &v, &mut Writes::default(), |s| match mode {
            0 => s.entries[0].workspace_id = s.custody_quantity,
            1 => s.entries[0].profile_id = s.custody_component_blockers,
            2 => {
                s.entries[0].quantity =
                    kassiber_journals::JournalDecimal("9223372036854775808".parse().unwrap()).into()
            }
            3 => s.account_holdings.pairs[0].0.pop().map(|_| ()).unwrap(),
            4 => s.wallet_holdings.pairs[0].0.push(Value::Null),
            5 => s.entries[0].transaction_id = serde_json::from_str("9223372036854775808").unwrap(),
            _ => unreachable!(),
        });
        assert!(result.is_err(), "mode {mode}");
        equal(&c, &v["before"], &v["name"]);
        c.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn missing_inactive_or_changed_components_never_persist_reconciliations() {
    use serde_json::json;
    let original = cases()
        .into_iter()
        .find(|v| v["name"] == "reconciliation-snapshot:recovered")
        .unwrap();
    for mode in 0..10 {
        let mut v = original.clone();
        let proof =
            &v["record"]["custody"]["interpretation"]["native_component_reconciliations"][0];
        assert!(proof["component_id"].is_string());
        if mode == 1 {
            v["before"]["custody_components"]["rows"][0][8] = json!("draft");
        }
        if mode == 3 {
            v["record"]["custody"]["interpretation"]["native_component_reconciliations"][0]
                ["native_claim_amounts"] = json!([]);
        }
        if mode == 4 {
            v["record"]["custody"]["interpretation"]["native_component_reconciliations"][0]
                ["component_revision"] = Value::Null;
        }
        if mode == 5 {
            for decision in v["record"]["custody"]["quantity_state"]["projection"]["decisions"]
                .as_array_mut()
                .unwrap()
            {
                decision["target"] = Value::Null;
            }
        }
        if mode == 6 || mode == 7 {
            let claim = if mode == 6 { Value::Null } else { json!("") };
            for decision in v["record"]["custody"]["quantity_state"]["projection"]["decisions"]
                .as_array_mut()
                .unwrap()
            {
                decision["selected_claim_id"] = claim.clone();
            }
            v["record"]["custody"]["interpretation"]["native_component_reconciliations"][0]
                ["native_claim_amounts"] = json!([[claim, 2_000_000]]);
        }
        if mode == 8 {
            v["record"]["custody"]["quantity_state"] = Value::Null;
            v["record"]["custody"]["interpretation"]["native_component_reconciliations"][0]
                ["native_claim_amounts"] = json!([["missing", 0]]);
        }
        if mode == 9 {
            v["record"]["custody"]["interpretation"]["native_component_reconciliations"][0]
                ["native_claim_amounts"] = json!([["missing", null]]);
        }
        let c = Connection::open_in_memory().unwrap();
        load(&c, &v["before"]);
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        if mode == 0 {
            v["record"]["custody"]["interpretation"]["native_component_reconciliations"][0]
                ["component_id"] = json!("missing");
        }
        if mode == 2 {
            v["record"]["custody"]["interpretation"]["native_component_reconciliations"][0]
                ["component_revision"] = json!(999);
        }
        store(&c, &v, &mut Writes::default()).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM journal_custody_reconciliations",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0,
            "mode {mode}"
        );
        c.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn economic_relation_fallbacks_and_nonpositive_amounts_preserve_exact_principal() {
    use serde_json::json;
    let original = cases()
        .into_iter()
        .find(|v| v["name"] == "payout:reviewed_conversion")
        .unwrap();
    for mode in 0..12 {
        let mut v = original.clone();
        let q = &mut v["record"]["custody"]["quantity_state"];
        let relation = &mut q["reviewed_conversion_pairs"][0];
        match mode {
            0 => {}
            1 => {
                relation["out_amount"] = Value::Null;
                relation["out_amount_msat"] = json!(7000);
            }
            2 => {
                relation["out_amount"] = Value::Null;
                relation["in_amount"] = Value::Null;
            }
            3 => {
                relation["out_amount"] = json!(0);
            }
            4 => {
                relation["out_amount"] = json!(-1);
            }
            5 => {
                relation["in_amount"] = json!(0);
            }
            6 => {
                relation["in_amount"] = json!(-1);
            }
            7 => {
                relation["out_id"] = json!("missing");
            }
            8 => {
                relation["in_id"] = json!("");
                relation["in_amount"] = Value::Null;
            }
            9 => {
                relation["in_id"] = Value::Null;
                relation["payout_asset"] = json!("EUR");
                relation["payout_occurred_at"] = json!("2026-01-01T00:00:00Z");
            }
            10 => {
                relation["out_amount"] = json!("");
                relation["out_amount_msat"] = json!("");
            }
            11 => {
                relation["in_amount"] = json!("");
                relation["payout_amount"] = json!(17000);
            }
            _ => unreachable!(),
        }
        let c = Connection::open_in_memory().unwrap();
        load(&c, &v["before"]);
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        store(&c, &v, &mut Writes::default()).unwrap();
        let rows: Vec<(i64, i64, String)> = c.prepare("SELECT source_amount_msat,target_amount_msat,target_asset FROM journal_custody_economic_relations WHERE relation_kind='conversion'").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().collect::<Result<_, _>>().unwrap();
        if (3..=8).contains(&mode) {
            assert!(rows.is_empty(), "mode {mode}");
        } else {
            assert_eq!(
                rows,
                [(
                    if mode == 1 { 7000 } else { 10000 },
                    if mode == 11 { 17000 } else { 20000 },
                    if mode == 9 { "EUR" } else { "LBTC" }.to_owned()
                )],
                "mode {mode}"
            );
        }
        c.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn pending_chain_impacts_stay_open_without_dependencies_or_current_authority() {
    use serde_json::json;
    let original = cases()
        .into_iter()
        .find(|v| v["name"].as_str().unwrap().contains("retraction_inbox"))
        .unwrap();
    for mode in 0..4 {
        let mut v = original.clone();
        if mode == 0 {
            v["before"]["filed_report_chain_dependencies"]["rows"] = json!([]);
        }
        if mode == 1 || mode == 2 {
            let status = if mode == 1 { "unavailable" } else { "observed" };
            v["before"]["chain_analysis_watch_inbox"]["rows"][0][6] =
                json!(json!({"after":{"status":status},"before":{}}).to_string());
        }
        let c = Connection::open_in_memory().unwrap();
        load(&c, &v["before"]);
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        let stored = store(&c, &v, &mut Writes::default()).unwrap();
        assert_eq!(stored.filed_report_impacts_resolved, usize::from(mode == 3));
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM filed_report_chain_resolutions",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            i64::from(mode == 3)
        );
        c.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn corrupt_filed_snapshot_refuses_and_rolls_back_freshness() {
    use serde_json::json;
    let mut v = cases()
        .into_iter()
        .find(|v| v["name"].as_str().unwrap().contains("retraction_inbox"))
        .unwrap();
    v["before"]["filed_report_snapshots"]["rows"] = json!([]);
    let c = Connection::open_in_memory().unwrap();
    load(&c, &v["before"]);
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let error = store(&c, &v, &mut Writes::default()).unwrap_err();
    assert_eq!(error.message, "filed report snapshot not found");
    equal(&c, &v["before"], &v["name"]);
    c.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn nested_cache_and_checkpoint_refusals_restore_signed_settings() {
    use kassiber_journals::storage::{DeferredCheckpoint, RebuildCache};
    let scratch = Scratch::new("guard-refusals");
    let c = Connection::open(scratch.0.join("book.sqlite3")).unwrap();
    c.execute_batch("PRAGMA journal_mode=WAL;PRAGMA wal_autocheckpoint=73;PRAGMA cache_size=-321;PRAGMA cache_spill=OFF").unwrap();
    let get = |setting| {
        c.pragma_query_value(None, setting, |r| r.get::<_, i64>(0))
            .unwrap()
    };
    let outer = RebuildCache::enter(&c).unwrap();
    {
        let _inner = RebuildCache::enter(&c).unwrap();
        assert_eq!(get("cache_size"), -131072);
    }
    assert_eq!(get("cache_size"), -131072);
    outer.restore().unwrap();
    assert_eq!(get("cache_size"), -321);
    assert_eq!(get("cache_spill"), 0);
    let checkpoint = DeferredCheckpoint::enter(&c).unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(checkpoint.after_reply().is_err());
    assert_eq!(get("wal_autocheckpoint"), 73);
    c.execute_batch("ROLLBACK").unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _checkpoint = DeferredCheckpoint::enter(&c).unwrap();
        assert_eq!(get("wal_autocheckpoint"), 0);
        panic!("injected");
    }));
    assert!(panic.is_err());
    assert_eq!(get("wal_autocheckpoint"), 73);
}

#[test]
fn component_blockers_keep_filed_impacts_pending_without_quarantines() {
    use serde_json::json;
    let mut v = cases()
        .into_iter()
        .find(|v| v["name"] == "filed-change")
        .unwrap();
    v["record"]["custody"]["component_blockers"] = json!([{"component_id":"unresolved"}]);
    let c = Connection::open_in_memory().unwrap();
    load(&c, &v["before"]);
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let stored = store(&c, &v, &mut Writes::default()).unwrap();
    assert!(stored.quarantines.is_empty());
    assert_eq!(stored.filed_report_impacts_resolved, 0);
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM custody_filed_report_impact_resolutions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    c.execute_batch("ROLLBACK").unwrap();
}

fn physical_connection(path: &std::path::Path, cipher: bool) -> Connection {
    let c = Connection::open(path).unwrap();
    if cipher {
        c.pragma_update(None, "key", "synthetic-test-only").unwrap();
        c.execute_batch("PRAGMA journal_mode=DELETE;PRAGMA synchronous=FULL")
            .unwrap();
    } else {
        c.execute_batch("PRAGMA journal_mode=WAL;PRAGMA synchronous=NORMAL")
            .unwrap();
    }
    c.execute_batch("PRAGMA foreign_keys=ON;PRAGMA cache_size=-321;PRAGMA cache_spill=OFF;PRAGMA wal_autocheckpoint=73").unwrap();
    c.busy_timeout(std::time::Duration::ZERO).unwrap();
    c
}

fn connection_settings(c: &Connection) -> Vec<Value> {
    [
        "journal_mode",
        "synchronous",
        "cache_size",
        "cache_spill",
        "wal_autocheckpoint",
    ]
    .into_iter()
    .map(|name| {
        c.query_row(&format!("PRAGMA {name}"), [], |r| {
            use kassiber_store::rusqlite::types::ValueRef;
            Ok(match r.get_ref(0)? {
                ValueRef::Text(s) => serde_json::json!(std::str::from_utf8(s).unwrap()),
                ValueRef::Integer(n) => serde_json::json!(n),
                _ => panic!("setting scalar"),
            })
        })
        .unwrap()
    })
    .collect()
}

#[test]
fn physical_callers_begin_before_repairs_and_publish_only_committed_dumps() {
    use kassiber_journals::storage::{DeferredCheckpoint, RebuildCache};
    use kassiber_store::rusqlite::{Error, ErrorCode, Transaction, TransactionBehavior};
    let scratch = Scratch::new("caller-snapshot");
    let v = &cases()[0];
    let mut expected = v["expected"].clone();
    // The extra repair UPDATE advances the profile's chain-index clocks once.
    let revision = expected["chain_index_clock"]["rows"][0][2]
        .as_i64()
        .unwrap()
        + 1;
    expected["chain_index_clock"]["rows"][0][2] = serde_json::json!(revision);
    for row in expected["chain_index_dirty"]["rows"]
        .as_array_mut()
        .unwrap()
    {
        if row[1] == "profiles" {
            row[3] = serde_json::json!(revision);
        }
    }
    for cipher in [false, true] {
        let path = scratch.0.join(format!("{cipher}.sqlite3"));
        let c = physical_connection(&path, cipher);
        load(&c, &v["before"]);
        let other = physical_connection(&path, cipher);
        let settings = connection_settings(&c);
        let checkpoint = DeferredCheckpoint::enter(&c).unwrap();
        let cache = RebuildCache::enter(&c).unwrap();
        let transaction = Transaction::new_unchecked(&c, TransactionBehavior::Immediate).unwrap();
        assert!(matches!(other.execute_batch("BEGIN IMMEDIATE"),
            Err(Error::SqliteFailure(error, _)) if error.code == ErrorCode::DatabaseBusy));
        // The first accounting snapshot and a representative pre-build repair
        // occur after the caller owns the writer lock.
        equal(&c, &v["before"], &v["name"]);
        c.execute(
            "UPDATE profiles SET last_processed_tx_count=-1 WHERE id=?",
            [v["record"]["profile"]["id"].as_str().unwrap()],
        )
        .unwrap();
        store(&c, v, &mut Writes::default()).unwrap();
        equal(&other, &v["before"], &v["name"]);
        transaction.commit().unwrap();
        cache.restore().unwrap();
        equal(&other, &expected, &v["name"]);
        checkpoint.after_reply().unwrap();
        assert_eq!(connection_settings(&c), settings);
        drop(other);
        drop(c);
        equal(&physical_connection(&path, cipher), &expected, &v["name"]);
    }
}

#[test]
fn physical_driver_refusals_roll_back_partial_writes_and_preserve_caller_repairs() {
    let scratch = Scratch::new("driver-refusals");
    let v = &cases()[0];
    for cipher in [false, true] {
        for mode in ["prepare", "read", "readonly", "insert", "update", "uuid"] {
            let c =
                physical_connection(&scratch.0.join(format!("{cipher}-{mode}.sqlite3")), cipher);
            load(&c, &v["before"]);
            if mode == "prepare" {
                c.execute_batch("DROP TABLE journal_tax_summary").unwrap();
            }
            if mode == "read" {
                c.execute_batch("UPDATE profiles SET last_processed_at=x'00'")
                    .unwrap();
            }
            if mode == "insert" {
                c.execute_batch("CREATE TEMP TRIGGER refuse_entry BEFORE INSERT ON journal_entries BEGIN SELECT RAISE(ABORT, 'driver insert refusal'); END").unwrap();
            }
            if mode == "update" {
                c.execute_batch("CREATE TEMP TRIGGER refuse_freshness BEFORE UPDATE OF last_processed_at ON profiles BEGIN SELECT RAISE(ABORT, 'driver update refusal'); END").unwrap();
            }
            let before = snapshot(&c);
            c.execute_batch("BEGIN IMMEDIATE;SAVEPOINT journals_process")
                .unwrap();
            c.execute_batch("UPDATE profiles SET last_processed_tx_count=-1")
                .unwrap();
            let repaired = snapshot(&c);
            if mode == "readonly" {
                c.execute_batch("PRAGMA query_only=ON").unwrap();
            }
            let mut input = v.clone();
            if mode == "uuid" {
                input["ids"] = serde_json::json!([]);
            }
            let mut writes = Writes::default();
            let error = store(&c, &input, &mut writes).unwrap_err();
            assert_eq!(
                error.family,
                // Python's sqlite3 subclass for each driver refusal.
                match mode {
                    "uuid" => "RuntimeError",
                    "read" => "DatabaseError",
                    "insert" | "update" => "IntegrityError",
                    _ => "OperationalError",
                },
                "cipher={cipher} {mode}: {error}"
            );
            match mode {
                "insert" => assert!(error.message.contains("driver insert refusal")),
                "update" => assert!(error.message.contains("driver update refusal")),
                "prepare" => assert!(error.message.contains("no such table")),
                "uuid" => assert_eq!(error.message, "persistence UUID tape exhausted"),
                _ => (),
            }
            if matches!(mode, "insert" | "update" | "uuid") {
                assert!(writes.count > 0);
            }
            c.execute_batch("PRAGMA query_only=OFF").unwrap();
            assert!(
                logical_equal(&snapshot(&c), &repaired),
                "cipher={cipher} {mode}"
            );
            c.execute_batch("ROLLBACK TO journals_process;RELEASE journals_process;ROLLBACK")
                .unwrap();
            assert!(
                logical_equal(&snapshot(&c), &before),
                "cipher={cipher} {mode}"
            );
        }
    }
}

#[test]
fn physical_caller_drop_and_unwind_restore_repairs_journals_and_settings() {
    use kassiber_journals::storage::{DeferredCheckpoint, RebuildCache};
    use kassiber_store::rusqlite::{Transaction, TransactionBehavior};
    let scratch = Scratch::new("caller-drop");
    let v = &cases()[0];
    for cipher in [false, true] {
        for panic in [false, true] {
            let path = scratch.0.join(format!("{cipher}-{panic}.sqlite3"));
            let c = physical_connection(&path, cipher);
            load(&c, &v["before"]);
            let settings = connection_settings(&c);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _checkpoint = DeferredCheckpoint::enter(&c).unwrap();
                let _cache = RebuildCache::enter(&c).unwrap();
                let _transaction =
                    Transaction::new_unchecked(&c, TransactionBehavior::Immediate).unwrap();
                c.execute_batch(
                    "SAVEPOINT journals_process;UPDATE profiles SET last_processed_tx_count=-1",
                )
                .unwrap();
                store(&c, v, &mut Writes::default()).unwrap();
                if panic {
                    panic!("caller failed before COMMIT");
                }
                // Dropping the uncommitted caller handle must also undo repairs
                // outside the nested persistence savepoint.
            }));
            assert_eq!(result.is_err(), panic);
            assert!(c.is_autocommit());
            equal(&c, &v["before"], &v["name"]);
            assert_eq!(connection_settings(&c), settings);
            drop(c);
            equal(
                &physical_connection(&path, cipher),
                &v["before"],
                &v["name"],
            );
        }
    }
}

#[test]
fn physical_commit_failure_remains_uncommitted_and_rolls_back_the_whole_caller() {
    use kassiber_journals::storage::{DeferredCheckpoint, RebuildCache};
    use kassiber_store::rusqlite::{Transaction, TransactionBehavior};
    let scratch = Scratch::new("commit-refusal");
    let v = &cases()[0];
    for cipher in [false, true] {
        let path = scratch.0.join(format!("{cipher}.sqlite3"));
        let c = physical_connection(&path, cipher);
        load(&c, &v["before"]);
        c.execute_batch("CREATE TABLE commit_parent(id PRIMARY KEY);CREATE TABLE commit_child(parent REFERENCES commit_parent(id) DEFERRABLE INITIALLY DEFERRED)").unwrap();
        let before = snapshot(&c);
        let settings = connection_settings(&c);
        {
            let _checkpoint = DeferredCheckpoint::enter(&c).unwrap();
            let _cache = RebuildCache::enter(&c).unwrap();
            let transaction =
                Transaction::new_unchecked(&c, TransactionBehavior::Immediate).unwrap();
            c.execute_batch("SAVEPOINT journals_process;UPDATE profiles SET last_processed_tx_count=-1;INSERT INTO commit_child VALUES('missing')").unwrap();
            store(&c, v, &mut Writes::default()).unwrap();
            c.execute_batch("RELEASE journals_process").unwrap();
            assert!(transaction.commit().is_err());
        }
        assert!(c.is_autocommit());
        assert!(logical_equal(&snapshot(&c), &before));
        assert_eq!(connection_settings(&c), settings);
        drop(c);
        assert!(logical_equal(
            &snapshot(&physical_connection(&path, cipher)),
            &before
        ));
    }
}

#[test]
fn physical_savepoint_driver_failures_require_whole_caller_rollback() {
    use kassiber_journals::storage::WriteObserver;
    struct Release<'a> {
        conn: &'a Connection,
        writes: usize,
    }
    impl WriteObserver for Release<'_> {
        fn wrote(&mut self, _: &'static str) -> kassiber_store::rusqlite::Result<()> {
            self.writes += 1;
            Ok(())
        }
        fn phase(&mut self, phase: &'static str) {
            if phase == "complete" {
                self.conn.execute_batch("RELEASE r4_journal_store").unwrap();
            }
        }
    }
    let scratch = Scratch::new("savepoint-driver-refusal");
    let v = &cases()[0];
    for cipher in [false, true] {
        let path = scratch.0.join(format!("{cipher}.sqlite3"));
        let c = physical_connection(&path, cipher);
        load(&c, &v["before"]);
        c.execute_batch("CREATE TABLE savepoint_busy(value INTEGER)")
            .unwrap();
        let before = snapshot(&c);
        c.execute_batch("BEGIN IMMEDIATE;UPDATE profiles SET last_processed_tx_count=-1")
            .unwrap();
        {
            let mut statement = c
                .prepare("INSERT INTO savepoint_busy VALUES(1) RETURNING value")
                .unwrap();
            let mut pending = statement.query([]).unwrap();
            assert!(pending.next().unwrap().is_some());
            let mut writes = Writes::default();
            let error = store(&c, v, &mut writes).unwrap_err();
            assert_eq!(error.family, "OperationalError");
            assert!(error.message.contains("SQL statements in progress"));
            assert_eq!(writes.count, 0);
        }
        c.execute_batch("ROLLBACK").unwrap();
        assert!(logical_equal(&snapshot(&c), &before));
        c.execute_batch("BEGIN IMMEDIATE;UPDATE profiles SET last_processed_tx_count=-1")
            .unwrap();
        let mut release = Release {
            conn: &c,
            writes: 0,
        };
        let error = store(&c, v, &mut release).unwrap_err();
        assert_eq!(error.family, "OperationalError");
        assert!(error.message.contains("no such savepoint"));
        assert!(release.writes > 10);
        assert!(!c.is_autocommit());
        c.execute_batch("ROLLBACK").unwrap();
        assert!(logical_equal(&snapshot(&c), &before));
        drop(c);
        assert!(logical_equal(
            &snapshot(&physical_connection(&path, cipher)),
            &before
        ));
    }
}

struct PragmaFault<'a> {
    conn: &'a Connection,
}
impl<'a> PragmaFault<'a> {
    fn once(conn: &'a Connection, pragma: &'static str, deny: usize) -> Self {
        use kassiber_store::rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        let mut call = 0;
        conn.authorizer(Some(move |context: AuthContext<'_>| {
            if let AuthAction::Pragma { pragma_name, .. } = context.action {
                if pragma_name == pragma {
                    call += 1;
                    if call == deny {
                        return Authorization::Deny;
                    }
                }
            }
            Authorization::Allow
        }))
        .unwrap();
        Self { conn }
    }
}
impl Drop for PragmaFault<'_> {
    fn drop(&mut self) {
        use kassiber_store::rusqlite::hooks::{AuthContext, Authorization};
        self.conn
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
    }
}

#[test]
fn physical_setting_driver_errors_propagate_and_retry_cache_restoration() {
    use kassiber_journals::storage::{DeferredCheckpoint, RebuildCache};
    for cipher in [false, true] {
        let root = Scratch::new(if cipher {
            "cipher-pragma"
        } else {
            "sqlite-pragma"
        });
        let conn = physical_connection(&root.0.join("book.sqlite3"), cipher);
        conn.pragma_update(None, "cache_size", 321).unwrap();
        let before = connection_settings(&conn);
        for deny in [1, 2] {
            let fault = PragmaFault::once(&conn, "cache_size", deny);
            assert!(
                RebuildCache::enter(&conn).is_err(),
                "cache enter call {deny}"
            );
            drop(fault);
            assert_eq!(connection_settings(&conn), before);
        }
        let cache = RebuildCache::enter(&conn).unwrap();
        let fault = PragmaFault::once(&conn, "cache_size", 1);
        assert!(cache.restore().is_err());
        drop(fault);
        assert_eq!(
            connection_settings(&conn),
            before,
            "Drop retries a refused explicit restore"
        );
        for pragma in ["journal_mode", "wal_autocheckpoint"] {
            let fault = PragmaFault::once(&conn, pragma, 1);
            assert!(
                DeferredCheckpoint::enter(&conn).is_err(),
                "checkpoint enter {pragma}"
            );
            drop(fault);
            assert_eq!(connection_settings(&conn), before);
        }
        if !cipher {
            let fault = PragmaFault::once(&conn, "wal_autocheckpoint", 2);
            assert!(DeferredCheckpoint::enter(&conn).is_err());
            drop(fault);
            assert_eq!(connection_settings(&conn), before);
            let checkpoint = DeferredCheckpoint::enter(&conn).unwrap();
            let fault = PragmaFault::once(&conn, "wal_checkpoint", 1);
            assert!(checkpoint.after_reply().is_err());
            drop(fault);
            assert_eq!(connection_settings(&conn), before);
        }
    }
}

#[test]
fn malformed_quantity_refusals_roll_back_before_freshness_on_both_drivers() {
    use kassiber_journals::{boundary, persistence};
    use serde_json::json;
    let v = &cases()[0];
    let root = Scratch::new("quantity-refusal");
    for cipher in [false, true] {
        for mode in ["posting", "balance", "eligible", "source", "target"] {
            let mut quantity = json!({"projection":{"observations":[],"postings":[],"decisions":[]},"tax_eligibility":{"eligible_decisions":[]}});
            if matches!(mode, "posting" | "balance") {
                let posting = json!({"posting_id":"a","asset":"BTC","location_kind":"wallet","location_id":"synthetic","amount_msat":i64::MAX,"state":"observed"});
                if mode == "posting" {
                    let mut invalid = posting.clone();
                    invalid["amount_msat"] = json!(true);
                    quantity["projection"]["postings"] = json!([invalid]);
                } else {
                    let mut second = posting.clone();
                    second["posting_id"] = json!("b");
                    quantity["projection"]["postings"] = json!([posting, second]);
                }
            } else {
                let decision = json!({"state":"internal_verified","source":{"observation_hash":"source","start_msat":0,"end_msat":1},"target":{"observation_hash":"target","start_msat":0,"end_msat":1}});
                if mode == "eligible" {
                    quantity["tax_eligibility"]["eligible_decisions"] = json!([decision]);
                } else {
                    quantity["projection"]["decisions"] = json!([decision]);
                    if mode == "target" {
                        quantity["projection"]["observations"] =
                            json!([{"quantity_hash":"source"}]);
                    }
                }
            }
            let conn =
                physical_connection(&root.0.join(format!("{cipher}-{mode}.sqlite3")), cipher);
            load(&conn, &v["before"]);
            let r = &v["record"];
            let response = if r["response"].is_null() {
                boundary::empty_response()
            } else {
                boundary::ledger_response(&r["response"]).unwrap()
            };
            let mut state = boundary::build(
                &r["profile"],
                &r["custody"],
                r["wallets"].as_array().unwrap(),
                &response,
                String::new,
            )
            .unwrap();
            state.custody_quantity = &quantity;
            state.custody_native_reconciliations = json!([]);
            conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            let mut writes = Writes::default();
            let error = persistence::store_ledger_state(
                &conn,
                &r["profile"],
                &state,
                v["created_at"].as_str().unwrap(),
                &mut || Ok("unused".into()),
                &mut writes,
            )
            .unwrap_err();
            assert!(writes.count > 0);
            let (family, message) = match mode {
                "posting" => (
                    "OverflowError",
                    "quantity posting does not fit SQLite INTEGER",
                ),
                "balance" => (
                    "OverflowError",
                    "quantity balance does not fit SQLite INTEGER",
                ),
                "eligible" => ("KeyError", "eligible source observation"),
                "source" => ("KeyError", "decision source observation"),
                "target" => ("KeyError", "decision target observation"),
                _ => unreachable!(),
            };
            assert_eq!(error.family, family, "{cipher} {mode}: {error}");
            assert_eq!(error.message, message, "{cipher} {mode}");
            equal(&conn, &v["before"], &v["name"]);
            conn.execute_batch("ROLLBACK").unwrap();
        }
    }
}

#[test]
fn quantity_projection_filters_and_zero_balances_preserve_basis_barriers() {
    use serde_json::json;
    for (status, target, eligible) in [
        ("internal_verified", true, true),
        ("internal_reviewed", true, true),
        ("internal_verified", true, false),
        ("external_presumed", true, false),
        ("unknown", true, false),
        ("internal_verified", false, false),
    ] {
        let mut v = cases()[0].clone();
        let q = &mut v["record"]["custody"]["quantity_state"];
        let observations = q["projection"]["observations"].as_array().unwrap();
        let source = observations[0].clone();
        let destination = observations[1].clone();
        let mut decision = json!({"state":status,"reason":"accepted quantity fixture","source":{"observation_hash":source["quantity_hash"],"start_msat":0,"end_msat":1},"target":{"observation_hash":destination["quantity_hash"],"start_msat":0,"end_msat":1}});
        if !target {
            decision["target"] = Value::Null;
        }
        q["projection"]["decisions"] = json!([decision]);
        q["tax_eligibility"]["eligible_decisions"] = if eligible {
            json!([decision])
        } else {
            json!([])
        };
        let mut posting = json!({"posting_id":"first","observation_hash":source["quantity_hash"],"asset":"BTC","location_kind":"wallet","location_id":source["wallet_id"],"amount_msat":1,"state":"observed"});
        q["projection"]["postings"] = json!([posting]);
        posting["posting_id"] = json!("second");
        posting["amount_msat"] = json!(-1);
        q["projection"]["postings"]
            .as_array_mut()
            .unwrap()
            .push(posting);
        q["issues"] = json!([]);
        q["reviewed_conversion_pairs"] = json!([]);
        q["reviewed_direct_payouts"] = json!([]);
        v["record"]["custody"]["interpretation"]["native_component_reconciliations"] = json!([]);
        let c = Connection::open_in_memory().unwrap();
        load(&c, &v["before"]);
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        let stored = store(&c, &v, &mut Writes::default()).unwrap();
        assert_eq!(stored.quantity_counts["postings"], 2);
        assert_eq!(stored.quantity_counts["balances"], 0);
        let expected =
            usize::from(target && matches!(status, "internal_verified" | "internal_reviewed"));
        assert_eq!(
            stored.quantity_counts["decisions"], expected,
            "{status} target={target}"
        );
        let rows = c
            .prepare("SELECT basis_state FROM journal_custody_decisions")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<kassiber_store::rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            rows,
            if expected == 0 {
                vec![]
            } else {
                vec![if eligible {
                    "eligible"
                } else {
                    "blocked_by_prior_custody_basis"
                }
                .to_owned()]
            }
        );
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM journal_quantity_balances",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        c.execute_batch("ROLLBACK").unwrap();
        equal(&c, &v["before"], &v["name"]);
    }
}

#[test]
fn physical_automatic_outer_rollback_keeps_original_refusal_and_stale_dump() {
    use kassiber_journals::storage::{DeferredCheckpoint, RebuildCache};
    let root = Scratch::new("automatic-rollback");
    let v = &cases()[0];
    for cipher in [false, true] {
        let path = root.0.join(format!("{cipher}.sqlite3"));
        let c = physical_connection(&path, cipher);
        load(&c, &v["before"]);
        c.execute_batch("CREATE TEMP TRIGGER refuse_entry BEFORE INSERT ON journal_entries BEGIN SELECT RAISE(ROLLBACK, 'driver rolled back caller'); END").unwrap();
        let settings = connection_settings(&c);
        {
            let _checkpoint = DeferredCheckpoint::enter(&c).unwrap();
            let _cache = RebuildCache::enter(&c).unwrap();
            c.execute_batch("BEGIN IMMEDIATE;SAVEPOINT journals_process;UPDATE profiles SET last_processed_tx_count=-1").unwrap();
            let mut writes = Writes::default();
            let refusal = store(&c, v, &mut writes).unwrap_err();
            assert_eq!(refusal.family, "IntegrityError");
            assert!(refusal.message.contains("driver rolled back caller"));
            assert!(writes.count > 0);
            assert!(c.is_autocommit());
        }
        equal(&c, &v["before"], &v["name"]);
        assert_eq!(connection_settings(&c), settings);
        drop(c);
        equal(
            &physical_connection(&path, cipher),
            &v["before"],
            &v["name"],
        );
    }
}

#[test]
fn physical_drop_restoration_errors_remain_best_effort_after_commit() {
    use kassiber_journals::storage::{DeferredCheckpoint, RebuildCache};
    let root = Scratch::new("drop-restore");
    for cipher in [false, true] {
        let path = root.0.join(format!("{cipher}.sqlite3"));
        let c = physical_connection(&path, cipher);
        c.execute_batch("CREATE TABLE witness(value); INSERT INTO witness VALUES('durable')")
            .unwrap();
        let before = connection_settings(&c);
        let cache = RebuildCache::enter(&c).unwrap();
        let fault = PragmaFault::once(&c, "cache_size", 1);
        drop(cache);
        drop(fault);
        assert_eq!(
            c.pragma_query_value(None, "cache_size", |row| row.get::<_, i64>(0))
                .unwrap(),
            -131072
        );
        c.pragma_update(None, "cache_size", -321).unwrap();
        let checkpoint = DeferredCheckpoint::enter(&c).unwrap();
        let fault = PragmaFault::once(&c, "wal_autocheckpoint", 1);
        drop(checkpoint);
        drop(fault);
        assert_eq!(
            c.pragma_query_value(None, "wal_autocheckpoint", |row| row.get::<_, i64>(0))
                .unwrap(),
            if cipher { 73 } else { 0 }
        );
        c.pragma_update(None, "wal_autocheckpoint", 73).unwrap();
        assert_eq!(connection_settings(&c), before);
        drop(c);
        assert_eq!(
            physical_connection(&path, cipher)
                .query_row("SELECT value FROM witness", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "durable"
        );
    }
}

#[test]
fn quantity_issues_keep_filed_impacts_pending_without_other_barriers() {
    use serde_json::json;
    let mut v = cases()
        .into_iter()
        .find(|v| v["name"] == "filed-change")
        .unwrap();
    v["record"]["custody"]["component_blockers"] = json!([]);
    v["record"]["custody"]["quantity_state"]["issues"] = json!([{
        "issue_id":"unresolved", "issue_type":"quantity_gap", "state":"custody_suspense",
        "asset":"BTC", "amount_msat":1, "occurred_at":null, "transaction_ids":[],
        "reason":"unresolved quantity", "details":{}
    }]);
    let c = Connection::open_in_memory().unwrap();
    load(&c, &v["before"]);
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let stored = store(&c, &v, &mut Writes::default()).unwrap();
    assert!(stored.quarantines.is_empty());
    assert_eq!(stored.quantity_counts["issues"], 1);
    assert_eq!(stored.filed_report_impacts_resolved, 0);
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM custody_filed_report_impact_resolutions",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    c.execute_batch("ROLLBACK").unwrap();
    equal(&c, &v["before"], &v["name"]);
}

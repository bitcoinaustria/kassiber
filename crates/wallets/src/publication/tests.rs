use super::*;
use kassiber_store::rusqlite::types::ValueRef;
use std::sync::atomic::{AtomicU64, Ordering};

fn oracle() -> Value {
    serde_json::from_str(include_str!("../../tests/vectors/python_publication.json")).unwrap()
}

fn schema(conn: &Connection) {
    let manifest: Value =
        serde_json::from_str(include_str!("../../../store/fixtures/current-schema.json")).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    for kind in ["table", "index", "view", "trigger"] {
        for object in manifest["objects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o[0] == kind)
        {
            if let Some(sql) = object[3].as_str() {
                conn.execute_batch(sql).unwrap();
            }
        }
    }
    conn.execute_batch("INSERT INTO chain_index_clock VALUES(1,0)")
        .unwrap();
    conn.execute_batch(oracle()["seed"].as_str().unwrap())
        .unwrap();
}

fn snapshot(conn: &Connection) -> Value {
    let tables = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let mut result = serde_json::Map::new();
    for table in tables {
        let mut stmt = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
        let count = stmt.column_count();
        let mut rows = stmt
            .query_map([], |row| {
                Ok(Value::Array(
                    (0..count)
                        .map(|i| match row.get_ref(i).unwrap() {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(n) => json!(n),
                            ValueRef::Real(n) => json!(n),
                            ValueRef::Text(text) => json!(std::str::from_utf8(text).unwrap()),
                            ValueRef::Blob(_) => panic!("synthetic fixture contains no blobs"),
                        })
                        .collect(),
                ))
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        rows.sort_by_key(Value::to_string);
        result.insert(table, json!(rows));
    }
    Value::Object(result)
}

fn mark(conn: &Connection, case: &Value) -> Result<()> {
    mark_synced(
        conn,
        &Synced {
            profile_id: "p",
            wallet_id: "a",
            wallet_label: "Wällét",
            checkpoint: Some(&case["checkpoint"]),
            partial_success: case["partial_success"].as_bool().unwrap_or(false),
            degraded_reason: case["degraded_reason"].as_str(),
            blocking_reports: case["blocking_reports"].as_bool().unwrap_or(false),
            timestamp: "now",
            updated_at: "updated-now",
        },
    )
}

#[test]
fn publication_retractions_and_metadata_match_unchanged_python_full_table_vectors() {
    for case in oracle()["cases"].as_array().unwrap() {
        let mut conn = Connection::open_in_memory().unwrap();
        schema(&conn);
        conn.execute_batch(case["prior_sql"].as_str().unwrap())
            .unwrap();
        let input = &case["input"];
        let ids: Vec<_> = input["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_owned())
            .collect();
        let outcome = with_wallet_savepoint(
            &mut conn,
            || Ok(()),
            |conn| {
                let outcome = retract_records(conn, "p", "a", "Wällét", &ids, "backend:synthetic")?;
                mark(conn, input)?;
                Ok(outcome)
            },
        )
        .unwrap();
        assert_eq!(outcome, case["outcome"], "{}", input["name"]);
        assert_eq!(snapshot(&conn), case["snapshot"], "{}", input["name"]);
        assert!(conn.is_autocommit());
    }
}

#[test]
fn publication_retractions_chunk_large_sets_and_refuse_unowned_transactions() {
    let mut conn = Connection::open_in_memory().unwrap();
    schema(&conn);
    let before = snapshot(&conn);
    assert_eq!(
        retract_records(&conn, "p", "a", "Wällét", &["abc".into()], "source")
            .unwrap_err()
            .code,
        "observer_apply_outside_transaction"
    );
    assert_eq!(
        mark(&conn, &json!({})).unwrap_err().code,
        "observer_apply_outside_transaction"
    );
    assert_eq!(snapshot(&conn), before);
    let ids: Vec<_> = (0..1100).map(|i| format!("external-{i:04}")).collect();
    let outcome = with_wallet_savepoint(&mut conn, || Ok(()), |conn| {
        for (i, id) in ids.iter().enumerate() {
            conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES(?,'w','p','a',?,?,'date','in','BTC',1001,1,'fixed')",
                params![format!("large-{i}"), id, format!("large-{i}")]).map_err(database_error)?;
        }
        retract_records(conn, "p", "a", "Wällét", &ids, "source")
    }).unwrap();
    assert_eq!(outcome["retracted"], 1100);
    let after = snapshot(&conn);
    assert_eq!(after["transactions"], before["transactions"]);
    assert_eq!(
        conn.query_row(
            "SELECT journal_input_version FROM profiles WHERE id='p'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        8
    );
}

#[test]
fn publication_savepoint_preserves_outer_transaction_and_cancellation_and_unwinds() {
    let mut conn = Connection::open_in_memory().unwrap();
    schema(&conn);
    conn.execute_batch("BEGIN; UPDATE profiles SET label='pending' WHERE id='p'")
        .unwrap();
    let before = snapshot(&conn);
    for fail_at in [0, 1] {
        let mut calls = 0;
        let result = with_wallet_savepoint(
            &mut conn,
            || {
                let call = calls;
                calls += 1;
                if call == fail_at {
                    Err(AppError::new("cancelled", "cancelled"))
                } else {
                    Ok(())
                }
            },
            |conn| {
                retract_records(conn, "p", "a", "wallet", &["abc".into()], "source")?;
                mark(conn, &json!({"checkpoint":{}}))
            },
        );
        assert_eq!(result.unwrap_err().code, "cancelled");
        assert_eq!(snapshot(&conn), before);
        assert!(!conn.is_autocommit());
    }
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<()> = with_wallet_savepoint(
            &mut conn,
            || Ok(()),
            |conn| {
                retract_records(conn, "p", "a", "wallet", &["abc".into()], "source")?;
                panic!("injected persistence panic");
            },
        );
    }));
    assert!(panic.is_err());
    assert_eq!(snapshot(&conn), before);
    with_wallet_savepoint(&mut conn, || Ok(()), |conn| mark(conn, &json!({}))).unwrap();
    assert!(!conn.is_autocommit());
    conn.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        conn.query_row("SELECT label FROM profiles WHERE id='p'", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "primary"
    );
}

// Logical rollback is checked on disposable DELETE/FULL SQLCipher books,
// after each possible write of the implemented retraction/final-metadata path.
#[test]
fn publication_every_write_failure_restores_encrypted_book_state_across_restart() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "kassiber-publication-{}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    for (index, trigger) in [
        "CREATE TEMP TRIGGER fail BEFORE DELETE ON transactions BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TEMP TRIGGER fail BEFORE DELETE ON transaction_tags BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TEMP TRIGGER fail BEFORE UPDATE ON profiles BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TEMP TRIGGER fail BEFORE UPDATE ON wallets BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TEMP TRIGGER fail BEFORE INSERT ON freshness_source_states BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TEMP TRIGGER fail BEFORE UPDATE ON freshness_source_states BEGIN SELECT RAISE(ABORT,'injected'); END",
    ].iter().enumerate() {
        let path = root.join(format!("book-{index}.db"));
        let open = || {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA key='synthetic-test-only'; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;").unwrap();
            conn
        };
        let mut conn = open();
        schema(&conn);
        if index == 5 {
            with_wallet_savepoint(&mut conn, || Ok(()), |conn| mark(conn, &json!({"checkpoint":{}}))).unwrap();
        }
        let before = snapshot(&conn);
        drop(conn);
        let bytes = std::fs::read(&path).unwrap();
        let mut conn = open();
        conn.execute_batch(trigger).unwrap();
        let result = with_wallet_savepoint(&mut conn, || Ok(()), |conn| {
            retract_records(conn, "p", "a", "wallet", &["abc".into()], "source")?;
            mark(conn, &json!({"checkpoint":{}}))
        });
        assert!(result.is_err(), "write {index} was not reached");
        assert_eq!(snapshot(&conn), before, "write {index}");
        assert!(conn.is_autocommit());
        drop(conn);
        println!("write {index}: ciphertext byte identity = {}", std::fs::read(&path).unwrap() == bytes);
        let conn = open();
        assert_eq!(snapshot(&conn), before, "restarted write {index}");
        drop(conn);
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn publication_late_failure_rolls_back_observer_epoch_and_coverage_with_retractions() {
    use crate::observer_apply::{self, Coverage, Prepared};
    use crate::observer_state::Identity;
    let mut conn = Connection::open_in_memory().unwrap();
    schema(&conn);
    let mut prepared = Prepared {
        identity: Identity {
            id: "observer",
            observer_kind: "bdk",
            chain: "bitcoin",
            network: "bitcoin",
            logical_wallet_id: "a",
            source_wallet_id: "a",
            source_key: "descriptor:default",
        },
        workspace_id: "w",
        profile_id: "p",
        branch_keys: &["receive", "change"],
        state: json!({"height":1}),
        coverage: vec![Coverage {
            branch_key: "receive".into(),
            highest_used: None,
            scanned_to: 20,
            details: json!({}),
        }],
        expected_epoch: None,
    };
    with_wallet_savepoint(
        &mut conn,
        || Ok(()),
        |conn| observer_apply::persist(conn, &prepared, "old"),
    )
    .unwrap();
    prepared.expected_epoch = Some(1);
    prepared.state = json!({"height":2});
    prepared.coverage.push(Coverage {
        branch_key: "change".into(),
        highest_used: Some(0),
        scanned_to: 21,
        details: json!({}),
    });
    let before = snapshot(&conn);
    for trigger in [
        "CREATE TEMP TRIGGER fail BEFORE UPDATE ON chain_observer_instances BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TEMP TRIGGER fail BEFORE DELETE ON chain_observer_coverage BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TEMP TRIGGER fail BEFORE INSERT ON chain_observer_coverage WHEN NEW.branch_key='change' BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TEMP TRIGGER fail BEFORE INSERT ON chain_observer_coverage WHEN NEW.branch_key='receive' BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TEMP TRIGGER fail BEFORE DELETE ON transactions BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TEMP TRIGGER fail BEFORE UPDATE ON wallets BEGIN SELECT RAISE(ABORT,'injected'); END",
        "CREATE TEMP TRIGGER fail BEFORE INSERT ON freshness_source_states BEGIN SELECT RAISE(ABORT,'injected'); END",
    ] {
        conn.execute_batch(trigger).unwrap();
        let result = with_wallet_savepoint(&mut conn, || Ok(()), |conn| {
            observer_apply::persist(conn, &prepared, "now")?;
            retract_records(conn, "p", "a", "wallet", &["abc".into()], "source")?;
            mark(conn, &json!({"checkpoint":{}}))
        });
        assert!(result.is_err());
        assert_eq!(snapshot(&conn), before);
        conn.execute_batch("DROP TRIGGER fail").unwrap();
    }
    prepared.expected_epoch = Some(0);
    let result = with_wallet_savepoint(
        &mut conn,
        || Ok(()),
        |conn| {
            // A stale observer must also roll back earlier writes, regardless of order.
            retract_records(conn, "p", "a", "wallet", &["abc".into()], "source")?;
            observer_apply::persist(conn, &prepared, "now")
        },
    );
    assert_eq!(result.unwrap_err().code, "observer_state_stale");
    assert_eq!(snapshot(&conn), before);
}

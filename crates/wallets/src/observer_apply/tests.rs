use super::*;
use serde_json::json;

fn connection() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE chain_observer_instances(
         id TEXT PRIMARY KEY,workspace_id TEXT,profile_id TEXT,logical_wallet_id TEXT,
         source_wallet_id TEXT,source_key TEXT,observer_kind TEXT,chain TEXT,network TEXT,
         state_version INTEGER,state_epoch INTEGER,state_json TEXT,created_at TEXT,updated_at TEXT);
         CREATE TABLE chain_observer_coverage(observer_id TEXT,branch_key TEXT,
         coverage_version INTEGER,highest_used INTEGER,scanned_to INTEGER,details_json TEXT,
         updated_at TEXT,PRIMARY KEY(observer_id,branch_key));",
    )
    .unwrap();
    conn
}

fn prepared() -> Prepared<'static> {
    Prepared {
        identity: Identity {
            id: "observer",
            observer_kind: "bdk",
            chain: "bitcoin",
            network: "bitcoin",
            logical_wallet_id: "wallet",
            source_wallet_id: "wallet",
            source_key: "descriptor:default",
        },
        workspace_id: "workspace",
        profile_id: "profile",
        branch_keys: &["receive", "change"],
        state: json!({"z": "é", "a": [1, true, null]}),
        coverage: vec![
            Coverage {
                branch_key: "receive".into(),
                highest_used: Some(1),
                scanned_to: 22,
                details: json!({"z": 2, "a": 1}),
            },
            Coverage {
                branch_key: "change".into(),
                highest_used: None,
                scanned_to: 20,
                details: json!({}),
            },
        ],
        expected_epoch: None,
    }
}

fn dump(conn: &Connection) -> String {
    let mut result = String::new();
    for table in ["chain_observer_instances", "chain_observer_coverage"] {
        let mut statement = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = statement.column_count();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for column in 0..columns {
                result.push_str(&format!("{:?};", row.get_ref(column).unwrap()));
            }
        }
    }
    result
}

#[test]
fn observer_persistence_matches_unchanged_python_vectors() {
    use kassiber_store::rusqlite::types::ValueRef;
    let vectors: Value = serde_json::from_str(include_str!(
        "../../tests/vectors/python_observer_persistence.json"
    ))
    .unwrap();
    let conn = connection();
    for case in vectors.as_array().unwrap() {
        let input = &case["input"];
        let mut update = prepared();
        update.state = if let Some(raw) = input["raw_state"].as_str() {
            serde_json::from_str(raw).unwrap()
        } else {
            input["state"].clone()
        };
        update.expected_epoch = input["expected_epoch"].as_i64();
        update.coverage = input["coverage"]
            .as_array()
            .unwrap()
            .iter()
            .map(|point| Coverage {
                branch_key: point["branch_key"].as_str().unwrap().into(),
                scanned_to: point["scanned_to"].as_i64().unwrap(),
                highest_used: point["highest_used"].as_i64(),
                details: point["details"].clone(),
            })
            .collect();
        conn.execute_batch("SAVEPOINT wallet_refresh").unwrap();
        let error = match persist(&conn, &update, case["timestamp"].as_str().unwrap()) {
            Ok(()) => Value::Null,
            Err(error) => {
                conn.execute_batch("ROLLBACK TO wallet_refresh").unwrap();
                json!({"code": error.code, "message": error.message,
                    "details": error.details.map(|value| serde_json::from_str::<Value>(&kassiber_pyjson::dumps(&value, &kassiber_pyjson::Options::default()).unwrap()).unwrap()),
                    "hint": error.hint, "retryable": error.retryable})
            }
        };
        conn.execute_batch("RELEASE wallet_refresh").unwrap();
        assert_eq!(error, case["error"], "{}", case["name"]);
        for table in ["chain_observer_instances", "chain_observer_coverage"] {
            let mut stmt = conn
                .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                .unwrap();
            let columns = stmt.column_count();
            let rows: Vec<Vec<Value>> = stmt
                .query_map([], |row| {
                    (0..columns)
                        .map(|column| {
                            Ok(match row.get_ref(column)? {
                                ValueRef::Null => Value::Null,
                                ValueRef::Integer(value) => json!(value),
                                ValueRef::Text(value) => json!(std::str::from_utf8(value).unwrap()),
                                other => panic!("unexpected observer SQL type: {other:?}"),
                            })
                        })
                        .collect()
                })
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap();
            assert_eq!(
                json!(rows),
                case["snapshot"][table],
                "{}: {table}",
                case["name"]
            );
        }
    }
}

#[test]
fn observer_persistence_matches_reference_rows_and_epoch_fences() {
    let conn = connection();
    let mut update = prepared();
    assert_eq!(
        persist(&conn, &update, "first").unwrap_err().code,
        "observer_apply_outside_transaction"
    );
    assert!(dump(&conn).is_empty());
    conn.execute_batch("BEGIN").unwrap();
    persist(&conn, &update, "first").unwrap();
    let row = conn
        .query_row("SELECT * FROM chain_observer_instances", [], |row| {
            Ok((
                (
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ),
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, String>(13)?,
            ))
        })
        .unwrap();
    assert_eq!(
        row,
        (
            (
                "observer".into(),
                "workspace".into(),
                "profile".into(),
                "wallet".into(),
                "wallet".into(),
                "descriptor:default".into(),
                "bdk".into(),
                "bitcoin".into(),
                "bitcoin".into()
            ),
            1,
            1,
            "{\"a\":[1,true,null],\"z\":\"é\"}".into(),
            "first".into(),
            "first".into()
        )
    );
    let coverage: Vec<_> = conn.prepare("SELECT branch_key,coverage_version,highest_used,scanned_to,details_json,updated_at FROM chain_observer_coverage ORDER BY rowid").unwrap()
        .query_map([], |r| Ok((r.get::<_, String>(0)?,r.get::<_, i64>(1)?,r.get::<_, Option<i64>>(2)?,r.get::<_, i64>(3)?,r.get::<_, String>(4)?,r.get::<_, String>(5)?))).unwrap().collect::<std::result::Result<_,_>>().unwrap();
    assert_eq!(
        coverage,
        vec![
            ("change".into(), 2, None, 20, "{}".into(), "first".into()),
            (
                "receive".into(),
                2,
                Some(1),
                22,
                "{\"a\":1,\"z\":2}".into(),
                "first".into()
            )
        ]
    );
    let before = dump(&conn);
    for epoch in [None, Some(0), Some(2)] {
        update.expected_epoch = epoch;
        let error = persist(&conn, &update, "stale").unwrap_err();
        assert_eq!(error.code, "observer_state_stale");
        assert!(error.retryable);
        assert_eq!(dump(&conn), before);
    }
    update.expected_epoch = Some(1);
    update.coverage.clear();
    update.state = json!({"replaced": true});
    persist(&conn, &update, "second").unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state_epoch,created_at,updated_at,state_json FROM chain_observer_instances",
            [],
            |r| Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?
            ))
        )
        .unwrap(),
        (
            2,
            "first".into(),
            "second".into(),
            "{\"replaced\":true}".into()
        )
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM chain_observer_coverage", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    conn.execute_batch("ROLLBACK").unwrap();
    assert!(dump(&conn).is_empty());
}

#[test]
fn observer_persistence_rejects_invalid_coverage_before_writing() {
    let conn = connection();
    conn.execute_batch("BEGIN").unwrap();
    for case in 0..8 {
        let mut update = prepared();
        match case {
            0 => update.state = json!([]),
            1 => update.coverage[0].branch_key = "unknown".into(),
            2 => update.coverage[0].branch_key = "change".into(),
            3 => update.coverage[0].scanned_to = -1,
            4 => update.coverage[0].highest_used = Some(-1),
            5 => update.coverage[0].highest_used = Some(22),
            6 => update.coverage[0].highest_used = Some(23),
            7 => update.coverage[0].details = json!([]),
            _ => unreachable!(),
        }
        assert_eq!(
            persist(&conn, &update, "fixed").unwrap_err().code,
            "observer_state_invalid",
            "case {case}"
        );
        assert!(dump(&conn).is_empty(), "case {case}");
    }
    let mut update = prepared();
    update.coverage[0].highest_used = Some(0);
    update.coverage[0].scanned_to = 1;
    update.coverage[1].scanned_to = 0;
    persist(&conn, &update, "fixed").unwrap();
    conn.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn observer_persistence_every_sql_write_rolls_back() {
    for failure in 0..5 {
        let conn = connection();
        if failure != 4 {
            conn.execute_batch("BEGIN").unwrap();
            persist(&conn, &prepared(), "first").unwrap();
            conn.execute_batch("COMMIT").unwrap();
        }
        let before = dump(&conn);
        // A real SQLite failure, including between the two coverage inserts.
        let (table, action, condition) = match failure {
            0 => ("chain_observer_instances", "UPDATE", "1"),
            1 => ("chain_observer_coverage", "DELETE", "1"),
            2 => (
                "chain_observer_coverage",
                "INSERT",
                "NEW.branch_key='change'",
            ),
            3 => (
                "chain_observer_coverage",
                "INSERT",
                "NEW.branch_key='receive'",
            ),
            4 => ("chain_observer_instances", "INSERT", "1"),
            _ => unreachable!(),
        };
        conn.execute_batch(&format!("CREATE TEMP TRIGGER injected BEFORE {action} ON {table} WHEN {condition} BEGIN SELECT RAISE(FAIL,'injected'); END; SAVEPOINT wallet_refresh;")).unwrap();
        let mut update = prepared();
        update.expected_epoch = if failure == 4 { None } else { Some(1) };
        assert!(persist(&conn, &update, "second").is_err());
        conn.execute_batch("ROLLBACK TO wallet_refresh; RELEASE wallet_refresh")
            .unwrap();
        assert_eq!(dump(&conn), before, "write {failure}");
        assert!(conn.is_autocommit());
    }
}

#[test]
fn opaque_values_replace_sorted_inside_the_savepoint_only() {
    let conn = connection();
    conn.execute_batch(
        "CREATE TABLE chain_observer_values(observer_id TEXT,namespace_version INTEGER,key TEXT,
         value BLOB,updated_at TEXT,PRIMARY KEY(observer_id,key));",
    )
    .unwrap();
    let values = json!({"000000000001":"0aff","000000000000":"00"});
    assert_eq!(
        persist_values(&conn, "o", &values, "t").err().unwrap().code,
        "observer_apply_outside_transaction"
    );
    conn.execute_batch("BEGIN").unwrap();
    conn.execute(
        "INSERT INTO chain_observer_values VALUES('o',1,'stale',x'01','old')",
        [],
    )
    .unwrap();
    persist_values(&conn, "o", &values, "t").unwrap();
    let rows: Vec<(i64, String, Vec<u8>, String)> = conn
        .prepare("SELECT namespace_version,key,value,updated_at FROM chain_observer_values ORDER BY rowid")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (1, "000000000000".into(), vec![0], "t".into()),
            (1, "000000000001".into(), vec![10, 255], "t".into()),
        ]
    );
    for bad in [
        json!({"k":"0"}),
        json!({"k":"zz"}),
        json!({"k":1}),
        json!({"":"00"}),
        json!([]),
    ] {
        assert_eq!(
            persist_values(&conn, "o", &bad, "t").err().unwrap().code,
            "observer_state_invalid",
            "{bad}"
        );
    }
}

//! Python's ID-less import identity state, replayed on an empty schema: the
//! occurrence slots, held duplicate groups and custody quarantine records of
//! every point the 8f repro scenarios inspect (generate_import_identity_vectors.py).
use kassiber_store::rusqlite::{types::Value as Sql, Connection};
use kassiber_wallets::import_identity as identity;
use serde_json::{json, Value};

fn book(case: &Value) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    let creation: Value =
        serde_json::from_str(include_str!("../../store/fixtures/creation.json")).unwrap();
    for sql in creation["statements"].as_array().unwrap() {
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    for table in [
        "workspaces",
        "profiles",
        "wallets",
        "transactions",
        "import_batches",
        "import_batch_transactions",
        "synthetic_import_duplicate_reviews",
    ] {
        for row in case["tables"][table].as_array().unwrap() {
            let row = row.as_object().unwrap();
            let columns: Vec<_> = row.keys().cloned().collect();
            let values: Vec<Sql> = row
                .values()
                .map(|value| match value {
                    Value::Null => Sql::Null,
                    Value::Bool(b) => Sql::Integer(i64::from(*b)),
                    Value::Number(n) if n.is_i64() => Sql::Integer(n.as_i64().unwrap()),
                    Value::Number(n) => Sql::Real(n.as_f64().unwrap()),
                    Value::String(s) => Sql::Text(s.clone()),
                    other => panic!("unexpected stored value {other}"),
                })
                .collect();
            let placeholders = vec!["?"; columns.len()].join(",");
            conn.execute(
                &format!(
                    "INSERT INTO {table}({}) VALUES({placeholders})",
                    columns.join(",")
                ),
                kassiber_store::rusqlite::params_from_iter(values),
            )
            .unwrap();
        }
    }
    conn
}

#[test]
fn python_import_identity_state_matches_every_8f_scenario() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("vectors/python_import_identity.json")).unwrap();
    assert!(cases.len() >= 100);
    let mut held_groups = 0;
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let profile = case["profile_id"].as_str().unwrap();
        let conn = book(case);
        let groups: Vec<Value> = identity::stored_synthetic_occurrences(&conn, profile, None)
            .unwrap()
            .into_iter()
            .map(|((wallet, economic), members)| {
                let slots: serde_json::Map<String, Value> = identity::occurrence_slots(&members)
                    .into_iter()
                    .map(|(number, row)| (number.to_string(), json!(row.id)))
                    .collect();
                json!({"wallet_id": wallet, "economic_hash": economic,
                    "members": members.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
                    "slots": slots})
            })
            .collect();
        assert_eq!(json!(groups), case["groups"], "{name}: groups");
        let held: Vec<Value> = identity::held_duplicate_groups(&conn, profile)
            .unwrap()
            .into_iter()
            .map(|g| {
                json!({"wallet_id": g.wallet_id, "economic_hash": g.economic_hash,
                    "group_fingerprint": g.group_fingerprint,
                    "active": g.active.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
                    "largest": g.largest, "import_count": g.import_count,
                    "hold_reasons": g.hold_reasons, "refilled_excluded_ids": g.refilled_excluded_ids})
            })
            .collect();
        held_groups += held.len();
        assert_eq!(json!(held), case["held"], "{name}: held");
        let quarantines = identity::duplicate_review_quarantines(&conn, profile).unwrap();
        let encoded = kassiber_pyjson::dumps(
            &kassiber_pyjson::Value::List(quarantines),
            &kassiber_pyjson::Options::default().sort_keys(true),
        )
        .unwrap();
        let expected = kassiber_pyjson::dumps(
            &kassiber_pytext::loads(&case["quarantines"].to_string()).unwrap(),
            &kassiber_pyjson::Options::default().sort_keys(true),
        )
        .unwrap();
        assert_eq!(encoded, expected, "{name}: quarantines");
    }
    // Every hold reason appears, so no rule passes vacuously.
    assert!(held_groups >= 50);
    for reason in [
        identity::CROSS_IMPORT_EXCESS,
        identity::PRE_BATCH_EXCESS,
        identity::REFILLED_EXCLUDED_COPY,
    ] {
        assert!(
            cases.iter().any(|c| c["held"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["hold_reasons"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r == reason))),
            "{reason}"
        );
    }
}

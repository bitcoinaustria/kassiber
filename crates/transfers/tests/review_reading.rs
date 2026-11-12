use kassiber_envelope::{dict, get, py_str};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use kassiber_transfers::{review_reading, Scope};

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(values) => values,
        _ => panic!("list"),
    }
}

fn seed_rows(fixture: &Value, scenario: &Value) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=OFF; PRAGMA temp_store=MEMORY")
        .unwrap();
    for statements in [
        get(fixture, "schema").unwrap(),
        get(scenario, "sql").unwrap(),
    ] {
        for sql in list(statements) {
            conn.execute_batch(&py_str(sql)).unwrap();
        }
    }
    conn
}

fn install_objects(conn: &Connection, fixture: &Value) {
    for sql in list(get(fixture, "objects").unwrap()) {
        conn.execute_batch(&py_str(sql)).unwrap();
    }
}

fn seed(fixture: &Value, scenario: &Value) -> Connection {
    let conn = seed_rows(fixture, scenario);
    install_objects(&conn, fixture);
    conn
}

#[test]
fn desktop_pair_readings_match_frozen_python_fingerprints_without_writes() {
    let fixture = kassiber_pytext::loads(include_str!("review_cases.json")).unwrap();
    let mut count = 0;
    for scenario in list(get(&fixture, "scenarios").unwrap()) {
        let conn = seed(&fixture, scenario);
        let scope = Scope {
            workspace_id: "w",
            profile_id: "p",
        };
        let changes = conn.total_changes();
        for reading in list(get(scenario, "pair_readings").unwrap()) {
            let pair = get(reading, "pair").unwrap();
            let actual = review_reading::reading(
                &conn,
                &scope,
                &py_str(get(pair, "out_transaction_id").unwrap()),
                &py_str(get(pair, "in_transaction_id").unwrap()),
            )
            .unwrap();
            let expected = dict([
                ("reviewPairId", get(pair, "id").unwrap().clone()),
                (
                    "pairFingerprint",
                    get(get(reading, "expected").unwrap(), "pair_fingerprint")
                        .unwrap()
                        .clone(),
                ),
            ]);
            assert_eq!(actual, expected, "{:?}", get(scenario, "name"));
            count += 1;
        }
        assert_eq!(
            review_reading::reading(&conn, &scope, "absent", "absent").unwrap(),
            dict([
                ("reviewPairId", Value::Null),
                ("pairFingerprint", Value::Null)
            ])
        );
        assert_eq!(conn.total_changes(), changes);
    }
    assert_eq!(count, 18);
}

#[test]
fn desktop_pair_readings_refuse_ambiguous_reviews_and_foreign_profiles() {
    use kassiber_store::rusqlite::{params_from_iter, types::Value as SqlValue};
    let fixture = kassiber_pytext::loads(include_str!("review_cases.json")).unwrap();
    let scenario = list(get(&fixture, "scenarios").unwrap())
        .iter()
        .find(|scenario| get(scenario, "name") == Some(&kassiber_envelope::s("pair")))
        .unwrap();
    let conn = seed_rows(&fixture, scenario);
    let pair = get(&list(get(scenario, "pair_readings").unwrap())[0], "pair").unwrap();
    let out_id = py_str(get(pair, "out_transaction_id").unwrap());
    let in_id = py_str(get(pair, "in_transaction_id").unwrap());
    let missing = dict([
        ("reviewPairId", Value::Null),
        ("pairFingerprint", Value::Null),
    ]);
    assert_eq!(
        review_reading::reading(
            &conn,
            &Scope {
                workspace_id: "w",
                profile_id: "foreign"
            },
            &out_id,
            &in_id
        )
        .unwrap(),
        missing
    );
    let mut statement = conn
        .prepare("SELECT * FROM custody_component_economic_terms WHERE legacy_source_id=?")
        .unwrap();
    let columns: Vec<String> = statement
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    let mut duplicate: Vec<SqlValue> = statement
        .query_row([py_str(get(pair, "id").unwrap())], |row| {
            (0..columns.len()).map(|index| row.get(index)).collect()
        })
        .unwrap();
    for (name, value) in columns.iter().zip(&mut duplicate) {
        match name.as_str() {
            "id" | "legacy_source_id" => *value = SqlValue::Text(format!("duplicate-{name}")),
            "ordinal" => *value = SqlValue::Integer(1000),
            _ => {}
        }
    }
    conn.execute(
        &format!(
            "INSERT INTO custody_component_economic_terms VALUES ({})",
            vec!["?"; columns.len()].join(",")
        ),
        params_from_iter(duplicate),
    )
    .unwrap();
    conn.execute_batch("UPDATE custody_components SET expected_economic_term_count=2")
        .unwrap();
    install_objects(&conn, &fixture);
    assert_eq!(
        review_reading::reading(
            &conn,
            &Scope {
                workspace_id: "w",
                profile_id: "p"
            },
            &out_id,
            &in_id
        )
        .unwrap(),
        missing
    );
}

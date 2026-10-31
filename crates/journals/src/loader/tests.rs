use super::*;

#[test]
fn native_loader_matches_python_database_bytes() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    for case in cases {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        conn.execute_batch(&text(get(&case, "sql"))).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let changes = conn.total_changes();
        let snapshot = load(
            &conn,
            get(&case, "profile").clone(),
            &text(get(&case, "as_of")),
        )
        .unwrap();
        assert_eq!(
            snapshot.v1().unwrap(),
            text(get(&case, "expected")),
            "{}",
            text(get(&case, "name"))
        );
        // The same in-memory structure as the text adapter, with no wire text.
        let adapter = snapshot.into_custody();
        let reference = kassiber_custody::parse(&text(get(&case, "expected")));
        match (adapter, reference) {
            (Ok(adapter), Ok(reference)) => assert_eq!(
                serde_json::to_value(adapter).unwrap(),
                serde_json::to_value(reference).unwrap()
            ),
            (Err(_), Err(_)) => (),
            _ => panic!("native/text admission differ"),
        }
        assert_eq!(conn.total_changes(), changes);
        assert!(!conn.is_autocommit());
        conn.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn native_loader_requires_caller_write_transaction() {
    let conn = Connection::open_in_memory().unwrap();
    for begin in [None, Some("BEGIN DEFERRED")] {
        if let Some(sql) = begin {
            conn.execute_batch(sql).unwrap();
        }
        assert!(matches!(
            load(&conn, dict([("id", Value::from("p"))]), "now"),
            Err(Error::TransactionRequired)
        ));
        if begin.is_some() {
            assert!(!conn.is_autocommit());
            conn.execute_batch("ROLLBACK").unwrap();
        }
    }
    conn.execute_batch(
        "CREATE TABLE canary(value INTEGER); BEGIN IMMEDIATE; INSERT INTO canary VALUES(1)",
    )
    .unwrap();
    assert!(matches!(
        load(&conn, dict([]), "now"),
        Err(Error::Database(_))
    ));
    assert!(!conn.is_autocommit());
    assert_eq!(
        conn.query_row("SELECT count(*) FROM canary", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    conn.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM canary", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn native_loader_keeps_sql_values_and_refuses_blob_coercion() {
    let conn = Connection::open_in_memory().unwrap();
    let rows=query(&conn,"SELECT NULL AS n,9223372036854775807 AS i,1.0 AS f,'钱' AS s,1 AS duplicate,2 AS duplicate",[]).unwrap();
    assert_eq!(kassiber_pyjson::dumps(&list(rows),&Options::new()).unwrap(),"[{\"n\": null, \"i\": 9223372036854775807, \"f\": 1.0, \"s\": \"\\u94b1\", \"duplicate\": 1}]");
    assert!(matches!(
        query(&conn, "SELECT x'ff' AS blob", []),
        Err(Error::Unsupported(_))
    ));
    let snapshot = LoadedSnapshot {
        facts: dict([("float", Value::Float(f64::INFINITY))]),
    };
    assert_eq!(snapshot.v1().unwrap(), "{\"float\": Infinity}");
    assert!(matches!(
        snapshot.into_custody(),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn native_loader_bounds_gap_worklist() {
    for structured in [false, true] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE wallets(id TEXT,profile_id TEXT,label TEXT,kind TEXT,config_json TEXT);
          CREATE TABLE transactions(id TEXT,profile_id TEXT,wallet_id TEXT,occurred_at TEXT,created_at TEXT,direction TEXT,asset TEXT,amount INTEGER,fee INTEGER,amount_includes_fee INTEGER,excluded INTEGER,kind TEXT,kind_override TEXT,privacy_boundary TEXT,external_id TEXT,raw_json TEXT);
          INSERT INTO wallets VALUES('a','p','A','custom','{}'),('b','p','B','custom','{}');
          WITH RECURSIVE n(i) AS (VALUES(0) UNION ALL SELECT i+1 FROM n WHERE i<50000)
          INSERT INTO transactions(id,profile_id,wallet_id,excluded) SELECT 'orphan-'||i,'p','absent',0 FROM n;").unwrap();
        for i in 0..270 {
            conn.execute("INSERT INTO transactions VALUES(?,'p','a','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','outbound','BTC',?,0,0,0,'withdrawal',NULL,?,'acq','{}')",rusqlite::params![format!("out-{i:03}"),1_000_000+i,if structured { "whirlpool" } else { "" }]).unwrap();
        }
        for i in 0..513 {
            conn.execute("INSERT INTO transactions VALUES(?,'p','b','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','inbound','BTC',900000,0,0,0,'deposit',NULL,NULL,'acq','{}')",[format!("in-{i:03}")]).unwrap();
        }
        let result = worklist::load(&conn, "p", &mut HashMap::new()).unwrap();
        assert_eq!(get(&result, "large_book"), &Value::Bool(true));
        let Value::List(rows) = get(&result, "rows") else {
            panic!("rows")
        };
        let outbound = rows
            .iter()
            .filter(|row| get(row, "direction") == &Value::from("outbound"))
            .collect::<Vec<_>>();
        assert_eq!(outbound.len(), if structured { 256 } else { 64 });
        assert_eq!(
            text(get(outbound[0], "id")),
            if structured { "out-000" } else { "out-206" }
        );
        assert_eq!(
            text(get(outbound.last().unwrap(), "id")),
            if structured { "out-255" } else { "out-269" }
        );
        assert_eq!(rows.len() - outbound.len(), 512);
        assert_eq!(text(get(&rows[0], "id")), "in-000");
    }
}

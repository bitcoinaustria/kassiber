use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::Connection;

fn canonical(v: &Value) -> String {
    dumps(v, &Options::new().sort_keys(true).compact()).unwrap()
}
#[test]
fn mirror_report_scope_and_table_match_python_without_accounting_readiness() {
    let cases = kassiber_pytext::loads(include_str!("data/privacy_mirror.json")).unwrap();
    let Value::List(cases) = cases else { panic!() };
    for case in cases {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("data/privacy_hygiene_seed.sql"))
            .unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        let Value::Str(sql) = get(&case, "sql").unwrap() else {
            panic!()
        };
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
        conn.execute(
            "UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id='p'",
            [],
        )
        .unwrap();
        let before = conn.total_changes();
        let before_tables = tables(&conn);
        let report = kassiber_reports::privacy_mirror::report(
            &conn,
            &dict([("id", s("ws"))]),
            &dict([("id", s("p"))]),
            false,
        )
        .unwrap();
        assert_eq!(
            canonical(&report),
            canonical(get(&case, "expected").unwrap()),
            "{}",
            canonical(get(&case, "name").unwrap())
        );
        assert_eq!(
            canonical(&kassiber_reports::privacy_mirror::table(&report)),
            canonical(get(&case, "table").unwrap())
        );
        assert_eq!(conn.total_changes(), before);
        assert_eq!(tables(&conn), before_tables);
        assert!(conn.is_autocommit());
    }
}

fn tables(conn: &Connection) -> Vec<(String, Vec<Vec<kassiber_store::rusqlite::types::Value>>)> {
    let names=conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    // Books built from the frozen seed keep the layout it was cut from.
    assert_eq!(
        names.len(),
        include_str!("data/privacy_hygiene_seed.sql")
            .lines()
            .filter(|line| line.starts_with("CREATE TABLE "))
            .count()
    );
    names
        .into_iter()
        .map(|name| {
            let mut stmt = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
            let columns = stmt.column_count();
            let mut rows = stmt
                .query_map([], |r| {
                    (0..columns)
                        .map(|i| r.get(i))
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.sort_by_cached_key(|r| format!("{r:?}"));
            (name, rows)
        })
        .collect()
}

#[test]
fn synchronized_mirror_matches_python_cold_warm_incremental_and_caller_rollback() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("data/mirror_integration.json")).unwrap()
    else {
        panic!()
    };
    for case in cases {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("data/privacy_hygiene_seed.sql"))
            .unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        let Value::Str(sql) = get(&case, "sql").unwrap() else {
            panic!()
        };
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
        let before = tables(&conn);
        let caller = get(&case, "caller") == Some(&Value::Bool(true));
        if caller {
            conn.execute_batch("SAVEPOINT caller; INSERT INTO settings(key,value) VALUES('caller-pending','keep');").unwrap();
        }
        let Value::List(actions) = get(&case, "actions").unwrap() else {
            panic!()
        };
        for action in actions {
            if let Some(Value::Str(sql)) = get(action, "sql") {
                conn.execute_batch(sql.as_str().unwrap()).unwrap();
            }
            let actual = kassiber_reports::execute_with_sources(
                &conn,
                "ui.reports.privacy_mirror",
                &dict::<&str>([]),
                &FixedSources,
            )
            .unwrap();
            assert_eq!(
                canonical(&actual),
                canonical(get(action, "report").unwrap()),
                "{}",
                canonical(get(&case, "name").unwrap())
            );
            let table = kassiber_reports::execute_with_sources(
                &conn,
                "reports.privacy-mirror",
                &dict([("csv", Value::Bool(true))]),
                &FixedSources,
            )
            .unwrap();
            assert_eq!(canonical(&table), canonical(get(action, "table").unwrap()));
            assert_eq!(
                canonical(&table_hashes(&conn)),
                canonical(get(action, "tables").unwrap())
            );
            assert_eq!(conn.is_autocommit(), !caller);
        }
        let current = tables(&conn);
        assert!(kassiber_reports::execute(
            &conn,
            "ui.reports.privacy_mirror",
            &dict([("profile", s("absent"))])
        )
        .is_err());
        assert_eq!(tables(&conn), current);
        assert_eq!(conn.is_autocommit(), !caller);
        if caller {
            conn.execute_batch("ROLLBACK TO caller; RELEASE caller")
                .unwrap();
            assert_eq!(tables(&conn), before);
        }
    }
}

fn table_hashes(conn: &Connection) -> Value {
    use sha2::{Digest, Sha256};
    let names=conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    // Books built from the frozen seed keep the layout it was cut from.
    assert_eq!(
        names.len(),
        include_str!("data/privacy_hygiene_seed.sql")
            .lines()
            .filter(|line| line.starts_with("CREATE TABLE "))
            .count()
    );
    dict(names.iter().map(|name| {
        let mut stmt = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
        let columns = stmt
            .column_names()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let mut rows = stmt
            .query_map([], |r| {
                Ok(dict(columns.iter().enumerate().map(|(i, k)| {
                    use kassiber_store::rusqlite::types::ValueRef;
                    let v = match r.get_ref(i).unwrap() {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(n) => Value::Int(n.into()),
                        ValueRef::Real(n) => Value::Float(n),
                        ValueRef::Text(t) => s(std::str::from_utf8(t).unwrap()),
                        ValueRef::Blob(_) => panic!(),
                    };
                    (k.as_str(), v)
                })))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows.sort_by_cached_key(canonical);
        (
            name.as_str(),
            s(format!(
                "{:x}",
                Sha256::digest(canonical(&Value::List(rows)).as_bytes())
            )),
        )
    }))
}

struct FixedSources;
impl kassiber_setup::Sources for FixedSources {
    fn uuid(&self, _: &Connection) -> kassiber_setup::Result<String> {
        Ok("0123456789abcdef0123456789abcdef".to_owned())
    }
    fn now(&self, _: &Connection) -> kassiber_setup::Result<String> {
        Ok("2026-01-01T00:00:00Z".to_owned())
    }
}

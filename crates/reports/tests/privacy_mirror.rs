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
    assert_eq!(names.len(), 112);
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

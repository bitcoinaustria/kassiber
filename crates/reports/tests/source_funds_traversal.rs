use kassiber_envelope::{dict, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_reports::source_funds_traversal;
use kassiber_store::{rusqlite::Connection, Book};

fn dump(conn: &Connection) -> Vec<(String, Vec<String>)> {
    let mut statement = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap();
    let names: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(names.len(), 113);
    names
        .into_iter()
        .map(|name| {
            let mut statement = conn
                .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
                .unwrap();
            let columns = statement.column_count();
            let mut records: Vec<_> = statement
                .query_map([], |row| {
                    Ok((0..columns)
                        .map(|column| format!("{:?}", row.get_ref(column).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            records.sort();
            (name, records)
        })
        .collect()
}

#[test]
fn source_funds_traversal_pins_dependency_order_cycles_caps_scope_and_all_tables() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/source_funds_traversal.json")).unwrap();
    assert!(cases.len() >= 203);
    for passphrase in [None, Some("synthetic-source-funds-traversal-passphrase")] {
        let root = tempfile::tempdir().unwrap();
        let book = Book::create(&root.path().join("data"), passphrase).unwrap();
        let conn = book.connection();
        for case in &cases {
            conn.execute_batch("SAVEPOINT traversal_case").unwrap();
            conn.execute_batch(include_str!("data/book.sql")).unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let before = dump(conn);
            let result = source_funds_traversal::load(
                conn,
                case["profile"].as_str().unwrap(),
                case["target"].as_str().unwrap(),
                case["max_depth"].as_u64().unwrap() as usize,
                case["node_limit"].as_u64().unwrap() as usize,
                case["edge_limit"].as_u64().unwrap() as usize,
            )
            .unwrap();
            let actual = if case["summary_only"].as_bool() == Some(true) {
                let retained = &result.links[case["target"].as_str().unwrap()];
                dict([
                    (
                        "rank",
                        dict(
                            result
                                .rank
                                .into_iter()
                                .map(|(id, rank)| (id, Value::Int(rank.into()))),
                        ),
                    ),
                    ("truncated", Value::Bool(result.truncated)),
                    (
                        "cycle_link_id",
                        result.cycle_link_id.map(s).unwrap_or(Value::Null),
                    ),
                    ("link_count", Value::Int(retained.len().into())),
                    (
                        "first",
                        kassiber_envelope::get(&retained[0], "id").unwrap().clone(),
                    ),
                    (
                        "last",
                        kassiber_envelope::get(retained.last().unwrap(), "id")
                            .unwrap()
                            .clone(),
                    ),
                ])
            } else {
                dict([
                    (
                        "rank",
                        dict(
                            result
                                .rank
                                .into_iter()
                                .map(|(id, rank)| (id, Value::Int(rank.into()))),
                        ),
                    ),
                    (
                        "links",
                        dict(
                            result
                                .links
                                .into_iter()
                                .map(|(id, links)| (id, Value::List(links))),
                        ),
                    ),
                    (
                        "cycle_link_id",
                        result.cycle_link_id.map(s).unwrap_or(Value::Null),
                    ),
                    ("truncated", Value::Bool(result.truncated)),
                ])
            };
            let expected = kassiber_pytext::loads(&case["expected"].to_string()).unwrap();
            let options = Options::new().sort_keys(true).compact();
            assert_eq!(
                dumps(&actual, &options).unwrap(),
                dumps(&expected, &options).unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(dump(conn), before, "{}", case["name"]);
            conn.execute_batch("ROLLBACK TO traversal_case;RELEASE traversal_case")
                .unwrap();
        }
    }
}

#[test]
#[cfg(target_pointer_width = "64")]
fn source_funds_traversal_refuses_sqlite_limit_overflow_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    let before = dump(conn);
    let error =
        source_funds_traversal::load(conn, "p", "in", 8, 5000, i64::MAX as usize).unwrap_err();
    assert_eq!(error.code, "internal_error");
    assert_eq!(
        error.message,
        "Python int too large to convert to SQLite INTEGER"
    );
    assert_eq!(dump(conn), before);
}

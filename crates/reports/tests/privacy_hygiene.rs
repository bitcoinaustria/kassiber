use kassiber_envelope::{dict, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn canonical(v: &Value) -> String {
    dumps(
        v,
        &Options {
            sort_keys: true,
            separators: Some((",".into(), ":".into())),
            ..Default::default()
        },
    )
    .unwrap()
}

fn tables(conn: &Connection) -> BTreeMap<String, String> {
    let names = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([], |r| r.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(names.len(), 113);
    names
        .into_iter()
        .map(|name| {
            let mut stmt = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
            let columns: Vec<_> = stmt.column_names().iter().map(|v| v.to_string()).collect();
            let mut values = stmt
                .query_map([], |row| {
                    Ok(dict(columns.iter().enumerate().map(|(index, name)| {
                        let cell = match row.get_ref(index).unwrap() {
                            kassiber_store::rusqlite::types::ValueRef::Null => Value::Null,
                            kassiber_store::rusqlite::types::ValueRef::Integer(n) => {
                                Value::Int(n.into())
                            }
                            kassiber_store::rusqlite::types::ValueRef::Real(n) => Value::Float(n),
                            kassiber_store::rusqlite::types::ValueRef::Text(v) => {
                                s(std::str::from_utf8(v).unwrap())
                            }
                            kassiber_store::rusqlite::types::ValueRef::Blob(_) => {
                                panic!("fixture must encode blobs explicitly")
                            }
                        };
                        (name.as_str(), cell)
                    })))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            values.sort_by_cached_key(canonical);
            let hash = format!(
                "{:x}",
                Sha256::digest(canonical(&Value::List(values)).as_bytes())
            );
            (name, hash)
        })
        .collect()
}

fn run_cases(endpoints: bool) {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/privacy_hygiene.json")).unwrap();
    assert!(cases.len() >= 200);
    let root = tempfile::tempdir().unwrap();
    let connection = Connection::open(root.path().join("book.sqlite")).unwrap();
    connection
        .execute_batch("PRAGMA synchronous=OFF; PRAGMA journal_mode=MEMORY;")
        .unwrap();
    let conn = &connection;
    conn.execute_batch(include_str!("data/privacy_hygiene_seed.sql"))
        .unwrap();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        if endpoints
            != [
                "host-",
                "url-",
                "transport-",
                "owner-",
                "local-precedes",
                "liquid-surface",
                "unknown-surface",
            ]
            .iter()
            .any(|p| name.starts_with(p))
        {
            continue;
        }
        conn.execute_batch("SAVEPOINT fixture").unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        conn.execute_batch(case["sql"].as_str().unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected_tables: BTreeMap<String, String> =
            serde_json::from_value(case["table_sha256"].clone()).unwrap();
        for (table, hash) in tables(conn) {
            assert_eq!(
                hash, expected_tables[&table],
                "{name}: Python fixture table {table} before read"
            );
        }
        let changes = conn.total_changes();
        let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
        let actual = match kassiber_reports::execute(conn, "reports.privacy-hygiene", &args) {
            Ok(payload) => dict([("ok", payload)]),
            Err(e) => dict([(
                "error",
                dict([
                    ("code", s(e.code)),
                    ("message", s(e.message)),
                    ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                    ("details", e.details.unwrap_or(Value::Null)),
                    ("retryable", Value::Bool(e.retryable)),
                ]),
            )]),
        };
        assert_eq!(
            canonical(&actual),
            case["expected_json"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(changes, conn.total_changes(), "{name}: read must not write");
        assert_eq!(
            tables(conn),
            expected_tables,
            "{name}: Python fixture after read"
        );
        conn.execute_batch("ROLLBACK TO fixture; RELEASE fixture")
            .unwrap();
    }
}

#[test]
fn privacy_hygiene_endpoint_postures_match_python_and_all_112_tables() {
    run_cases(true);
}

#[test]
fn privacy_hygiene_facts_match_python_and_all_112_tables() {
    run_cases(false);
}

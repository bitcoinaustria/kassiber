use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::{rusqlite::Connection, Book};

struct Fixed(std::cell::Cell<usize>);
impl kassiber_setup::Sources for Fixed {
    fn now(&self, _: &Connection) -> Result<String, AppError> {
        Ok("2026-01-02T03:04:05Z".into())
    }
    fn uuid(&self, _: &Connection) -> Result<String, AppError> {
        let next = self.0.get() + 1;
        self.0.set(next);
        Ok(format!("00000000-0000-4000-8000-{next:012}"))
    }
}

fn dump(conn: &Connection) -> Vec<(String, Vec<String>)> {
    let names: Vec<String> = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
        .query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
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
                        .collect::<Vec<_>>())
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            records.sort();
            (
                name,
                records
                    .into_iter()
                    .map(|record| format!("{record:?}"))
                    .collect(),
            )
        })
        .collect()
}

fn outcome(result: Result<Value, AppError>) -> Value {
    match result {
        Ok(data) => dict([("ok", data)]),
        Err(error) => dict([(
            "error",
            dict([
                ("code", s(error.code)),
                ("message", s(error.message)),
                ("hint", error.hint.map(s).unwrap_or(Value::Null)),
                ("details", error.details.unwrap_or(Value::Null)),
                ("retryable", Value::Bool(error.retryable)),
            ]),
        )]),
    }
}

fn replay(encrypted: bool) {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/source_funds_suggest.json")).unwrap();
    assert!(cases.len() >= 100);
    for passphrase in [None, Some("synthetic-suggest-passphrase")]
        .into_iter()
        .filter(|p| encrypted || p.is_none())
    {
        let root = tempfile::tempdir().unwrap();
        let book = Book::create(&root.path().join("data"), passphrase).unwrap();
        let conn = book.connection();
        for case in &cases {
            conn.execute_batch("SAVEPOINT suggest_case").unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let before = dump(conn);
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let actual = outcome(kassiber_reports::execute_with_sources(
                conn,
                case["command"].as_str().unwrap(),
                &args,
                &Fixed(std::cell::Cell::new(0)),
            ));
            let actual = dumps(&actual, &Options::new().compact()).unwrap();
            let expected = case["expected_json"].as_str().unwrap();
            if actual != expected {
                let index = actual
                    .bytes()
                    .zip(expected.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(actual.len().min(expected.len()));
                panic!(
                    "{} at byte {}: actual {:?}, expected {:?}",
                    case["name"],
                    index,
                    &actual[index.saturating_sub(80)..actual.len().min(index + 240)],
                    &expected[index.saturating_sub(80)..expected.len().min(index + 240)]
                );
            }
            assert_eq!(dump(conn), before, "{}", case["name"]);
            conn.execute_batch("ROLLBACK TO suggest_case;RELEASE suggest_case")
                .unwrap();
        }
    }
}

#[test]
fn source_funds_suggest_preserves_preview_bytes_and_all_tables() {
    replay(true);
}

#[test]
fn source_funds_suggest_mutation_witnesses() {
    replay(false);
}

#[test]
fn source_funds_suggest_persists_without_a_preview_marker() {
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/source_funds_suggest.json")).unwrap();
    conn.execute_batch(cases[0]["sql"].as_str().unwrap())
        .unwrap();
    let result = kassiber_reports::execute_with_sources(
        conn,
        "source-funds.suggest",
        &dict::<&str>([]),
        &Fixed(std::cell::Cell::new(0)),
    )
    .unwrap();
    assert_eq!(
        kassiber_envelope::get(&result, "committed"),
        Some(&Value::Bool(true))
    );
    assert!(kassiber_envelope::get(&result, "dry_run").is_none());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM source_funds_links", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(conn.is_autocommit());
}

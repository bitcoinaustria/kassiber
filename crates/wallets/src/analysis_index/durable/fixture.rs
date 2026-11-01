use super::*;
use kassiber_pyjson::Value as Py;

type WriteObserver = Box<dyn FnMut(&str)>;
thread_local! { static WRITE_OBSERVER: std::cell::RefCell<Option<WriteObserver>> = std::cell::RefCell::new(None); }

pub fn set_write_observer(observer: Option<WriteObserver>) {
    WRITE_OBSERVER.with(|slot| *slot.borrow_mut() = observer);
}
pub fn checkpoint(phase: &str) {
    after_write(phase);
}

pub(super) fn after_write(sql: &str) {
    WRITE_OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow_mut().as_mut() {
            observer(sql);
        }
    });
}

pub struct FixedSources(pub String);
impl crate::publication::wallet::Sources for FixedSources {
    fn id(&mut self) -> String {
        "0123456789abcdef0123456789abcdef".to_owned()
    }
    fn now(&mut self) -> String {
        self.0.clone()
    }
}

pub fn capture(conn: &Connection) -> Result<Py, AppError> {
    let names = names(
        conn,
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let mut tables = kassiber_envelope::dict::<String>([]);
    for name in &names {
        let mut data = rows(conn, &format!("SELECT * FROM \"{name}\""), [])?;
        data.sort_by_cached_key(|row| {
            kassiber_pyjson::dumps(row, &Options::new().sort_keys(true).compact()).unwrap()
        });
        kassiber_envelope::set(
            &mut tables,
            name,
            crate::analysis_index::digest(&Py::List(data))?.into(),
        );
    }
    let schema = rows(conn, "SELECT type,name,tbl_name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name", [])?;
    Ok(kassiber_envelope::dict([
        ("tables", tables),
        ("table_count", Py::Int(names.len().into())),
        (
            "schema",
            crate::analysis_index::digest(&Py::List(schema))?.into(),
        ),
    ]))
}

pub fn evaluate(case: &Py, conn: &Connection) -> Result<Py, AppError> {
    use crate::analysis_index::{field, list, Text};
    let profile = field(case, "profile").as_str().unwrap_or("p");
    let mut results = Vec::new();
    for action in list(field(case, "actions")) {
        let value = match field(action, "operation").as_str().unwrap() {
            "sql" => {
                conn.execute_batch(field(action, "sql").as_str().unwrap())
                    .map_err(database_error)?;
                Py::Null
            }
            "install" => Py::List(
                install(conn)?
                    .iter()
                    .map(|name| name.as_str().into())
                    .collect(),
            ),
            "contribute" => {
                let table = field(action, "table").as_str().unwrap();
                assert!(PHYSICAL.contains(&table));
                let key = match field(action, "key") {
                    Py::Int(v) => v.to_string().parse::<i64>().unwrap(),
                    _ => panic!("key"),
                };
                Py::List(
                    replace_source(conn, profile, table, key)?
                        .iter()
                        .map(|v| v.as_str().into())
                        .collect(),
                )
            }
            "contributors" => {
                let nodes = list(field(action, "nodes"))
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect();
                contributing_sources(
                    conn,
                    profile,
                    if field(action, "nodes") == &Py::Null {
                        None
                    } else {
                        Some(&nodes)
                    },
                )?
            }
            "synchronize" => {
                conn.execute_batch("SAVEPOINT durable_fixture")
                    .map_err(database_error)?;
                let mut sources = FixedSources(
                    field(action, "as_of")
                        .as_str()
                        .unwrap_or("2026-01-01T00:00:00Z")
                        .to_owned(),
                );
                let result = synchronize_with_sources(
                    conn,
                    profile,
                    field(action, "rebuild") == &Py::Bool(true),
                    &mut sources,
                );
                if result.is_err() {
                    conn.execute_batch("ROLLBACK TO durable_fixture")
                        .map_err(database_error)?;
                }
                conn.execute_batch("RELEASE durable_fixture")
                    .map_err(database_error)?;
                result?
            }
            _ => return Err(crate::analysis_index::invalid()),
        };
        results.push(kassiber_envelope::dict([
            ("result", value),
            ("state", capture(conn)?),
        ]));
    }
    Ok(Py::List(results))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_index::{field, list, Text};

    #[test]
    fn incremental_synchronization_matches_python_all_112_tables() {
        let cases = kassiber_pytext::loads(include_str!(
            "../../../tests/data/analysis_synchronize.json"
        ))
        .unwrap();
        for case in list(&cases) {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch(include_str!(
                "../../../../reports/tests/data/privacy_hygiene_seed.sql"
            ))
            .unwrap();
            conn.execute_batch(include_str!("../../../../reports/tests/data/book.sql"))
                .unwrap();
            conn.execute_batch(field(case, "sql").as_str().unwrap_or(""))
                .unwrap();
            let actual = evaluate(case, &conn).unwrap();
            for (step, (actual, expected)) in list(&actual)
                .iter()
                .zip(list(field(case, "expected")))
                .enumerate()
            {
                assert_eq!(
                    encoded(&Value::from_py(field(actual, "result"))).unwrap(),
                    encoded(&Value::from_py(field(expected, "result"))).unwrap(),
                    "{} step {step}",
                    field(case, "name").as_str().unwrap()
                );
                for (table, commitment) in
                    crate::analysis_index::fields(field(field(expected, "state"), "tables"))
                {
                    assert_eq!(
                        field(field(field(actual, "state"), "tables"), table),
                        commitment,
                        "{} step {step} table {table}",
                        field(case, "name").as_str().unwrap()
                    );
                }
                assert_eq!(
                    encoded(&Value::from_py(field(actual, "state"))).unwrap(),
                    encoded(&Value::from_py(field(expected, "state"))).unwrap(),
                    "{} step {step}",
                    field(case, "name").as_str().unwrap()
                );
            }
            assert!(conn.is_autocommit());
        }
        assert!(list(&cases).len() >= 25);
    }

    #[test]
    fn durable_triggers_and_contributions_match_python_all_112_tables() {
        let cases =
            kassiber_pytext::loads(include_str!("../../../tests/data/analysis_durable.json"))
                .unwrap();
        for case in list(&cases) {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch(include_str!(
                "../../../../reports/tests/data/privacy_hygiene_seed.sql"
            ))
            .unwrap();
            conn.execute_batch(include_str!("../../../../reports/tests/data/book.sql"))
                .unwrap();
            conn.execute_batch(field(case, "sql").as_str().unwrap_or(""))
                .unwrap();
            let actual = evaluate(case, &conn).unwrap();
            assert_eq!(
                encoded(&Value::from_py(&actual)).unwrap(),
                encoded(&Value::from_py(field(case, "expected"))).unwrap(),
                "{}",
                field(case, "name").as_str().unwrap()
            );
            assert!(conn.is_autocommit());
        }
        assert!(list(&cases).len() >= 15);
    }

    #[test]
    fn durable_sources_preserve_caller_transaction_and_closed_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../../../reports/tests/data/privacy_hygiene_seed.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../../../../reports/tests/data/book.sql"))
            .unwrap();
        let before = capture(&conn).unwrap();
        conn.execute_batch(
            "BEGIN; UPDATE wallets SET label='pending' WHERE id='cold'; SAVEPOINT caller",
        )
        .unwrap();
        let pending = capture(&conn).unwrap();
        install(&conn).unwrap();
        replace_source(&conn, "p", "transactions", 1).unwrap();
        assert!(!conn.is_autocommit());
        assert!(replace_source(&conn, "p", "settings; DELETE FROM wallets", 1).is_err());
        conn.execute_batch("ROLLBACK TO caller; RELEASE caller")
            .unwrap();
        assert_eq!(capture(&conn).unwrap(), pending);
        conn.execute_batch("ROLLBACK").unwrap();
        assert_eq!(capture(&conn).unwrap(), before);
    }

    #[test]
    fn incremental_synchronization_preserves_atomicity_and_warm_contributions() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../../../reports/tests/data/privacy_hygiene_seed.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../../../../reports/tests/data/book.sql"))
            .unwrap();
        let mut sources = FixedSources("2026-01-01T00:00:00Z".into());
        assert!(synchronize_with_sources(&conn, "p", false, &mut sources).is_err());
        conn.execute_batch("BEGIN").unwrap();
        synchronize_with_sources(&conn, "p", false, &mut sources).unwrap();
        conn.execute_batch("COMMIT").unwrap();
        let before = capture(&conn).unwrap();
        conn.execute_batch(
            "BEGIN; UPDATE wallets SET label='pending' WHERE id='cold'; SAVEPOINT caller",
        )
        .unwrap();
        let pending = capture(&conn).unwrap();
        synchronize_with_sources(&conn, "p", false, &mut sources).unwrap();
        assert!(!conn.is_autocommit());
        conn.execute_batch("ROLLBACK TO caller; RELEASE caller")
            .unwrap();
        assert_eq!(capture(&conn).unwrap(), pending);
        conn.execute_batch("ROLLBACK").unwrap();
        assert_eq!(capture(&conn).unwrap(), before);
        conn.execute_batch("CREATE TEMP TRIGGER contribution_guard BEFORE DELETE ON chain_index_sources BEGIN SELECT RAISE(ABORT,'unexpected source replacement'); END; BEGIN").unwrap();
        synchronize_with_sources(&conn, "p", false, &mut sources).unwrap();
        conn.execute_batch(
            "UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id='p'",
        )
        .unwrap();
        synchronize_with_sources(&conn, "p", false, &mut sources).unwrap();
        conn.execute_batch("ROLLBACK; DROP TRIGGER contribution_guard; BEGIN; UPDATE chain_analysis_datasets SET manifest_json='broken';").unwrap();
        conn.execute("INSERT INTO chain_analysis_datasets(id,profile_id,dataset_key,version,chain,network,visibility,status,manifest_json,content_sha256,created_at,updated_at) VALUES('bad','p','bad','1','bitcoin','main','public','active','broken','fixed','fixed','fixed')",[]).unwrap();
        let invalid = capture(&conn).unwrap();
        conn.execute_batch("SAVEPOINT failure").unwrap();
        assert!(synchronize_with_sources(&conn, "p", false, &mut sources).is_err());
        conn.execute_batch("ROLLBACK TO failure; RELEASE failure")
            .unwrap();
        assert_eq!(capture(&conn).unwrap(), invalid);
        conn.execute_batch("ROLLBACK").unwrap();
        assert_eq!(capture(&conn).unwrap(), before);
    }
}

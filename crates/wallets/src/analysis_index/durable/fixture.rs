use super::*;
use kassiber_pyjson::Value as Py;

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
}

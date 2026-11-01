use super::*;

pub fn evaluate(case: &Value, conn: &Connection) -> Result<Value> {
    let mut results = Vec::new();
    for action in list(field(case, "actions")) {
        let operation = text(action, "operation");
        let value = if operation == "sql" {
            if !conn.is_autocommit() {
                conn.execute_batch("COMMIT").map_err(database_error)?;
            }
            conn.execute_batch(text(action, "sql"))
                .map_err(database_error)?;
            Value::Null
        } else if ["begin", "rollback", "commit"].contains(&operation) {
            conn.execute_batch(operation).map_err(database_error)?;
            Value::Null
        } else {
            let empty = dict::<&str>([]);
            match execute(
                conn,
                "ws",
                "p",
                operation,
                get(action, "args").unwrap_or(&empty),
                &mut engine::durable::fixture::FixedSources("2026-01-01T00:00:00Z".to_owned()),
            ) {
                Ok(v) => kassiber_envelope::success(&format!("ui.chain_analysis.{operation}"), v),
                Err(e) => e.envelope(None),
            }
        };
        results.push(dict([
            (
                "wire",
                s(dumps(&value, &Options::new().compact())
                    .map_err(|_| invalid("Invalid fixture envelope"))?),
            ),
            ("result", value),
            ("state", engine::durable::fixture::capture(conn)?),
            ("in_transaction", Value::Bool(!conn.is_autocommit())),
        ]));
    }
    Ok(Value::List(results))
}

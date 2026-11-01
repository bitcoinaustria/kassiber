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
            let mut local = Local::default();
            if let Value::Dict(files) = field(action, "files") {
                for (key, raw) in files {
                    use bitcoin::hex::FromHex;
                    let bytes = Vec::<u8>::from_hex(raw_text(raw)).expect("fixture hex");
                    local.files.push((key.as_str().unwrap().to_owned(), bytes));
                }
            }
            match execute_local(
                conn,
                "ws",
                "p",
                operation,
                get(action, "args").unwrap_or(&empty),
                &mut engine::durable::fixture::FixedSources("2026-01-01T00:00:00Z".to_owned()),
                &mut local,
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

fn raw_text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

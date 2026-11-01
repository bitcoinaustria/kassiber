use super::*;
use kassiber_envelope::{build_error_envelope, success};

/// Keys an in-memory SQLCipher book so watch operations see an encrypted database.
pub const WATCH_KEY: &str = "PRAGMA key='analysis-watch-fixture'";

#[derive(Default)]
pub struct FixedSources {
    counter: u64,
    pub timestamp: String,
}
impl Sources for FixedSources {
    fn id(&mut self) -> String {
        self.counter += 1;
        format!("00000000-0000-4000-8000-{:012x}", self.counter)
    }
    fn now(&mut self) -> String {
        self.timestamp.clone()
    }
}

pub fn evaluate(case: &Value, conn: &Connection) -> Result<Value> {
    let mut sources = FixedSources::default();
    let mut results = Vec::new();
    let Value::List(actions) = field(case, "actions") else {
        panic!("fixture actions")
    };
    let profile = field(case, "profile").as_str().unwrap_or("p");
    for action in actions {
        sources.timestamp = field(action, "as_of")
            .as_str()
            .unwrap_or("2026-01-01T00:00:00Z")
            .into();
        let operation = field(action, "operation").as_str().unwrap();
        let value = match operation {
            "sql" => {
                conn.execute_batch(field(action, "sql").as_str().unwrap())
                    .map_err(database_error)?;
                Value::Null
            }
            "begin" | "rollback" => {
                conn.execute_batch(if operation == "begin" {
                    "BEGIN"
                } else {
                    "ROLLBACK"
                })
                .map_err(database_error)?;
                Value::Null
            }
            _ => match execute_with_sources(
                conn,
                &Scope {
                    workspace_id: "ws",
                    profile_id: profile,
                },
                operation,
                field(action, "args"),
                &mut sources,
            ) {
                Ok(result) => {
                    if !truthy(field(case, "caller_owned"))
                        && is_write(operation)
                        && !conn.is_autocommit()
                    {
                        conn.execute_batch("COMMIT").map_err(database_error)?;
                    }
                    success(&format!("ui.chain_analysis.{operation}"), result)
                }
                Err(error) => build_error_envelope(
                    &error.code,
                    &error.message,
                    error.details,
                    error.hint.as_deref(),
                    error.retryable,
                    None,
                ),
            },
        };
        results.push(dict([
            ("result", value),
            (
                "state",
                crate::analysis_index::durable::fixture::capture(conn)?,
            ),
            ("in_transaction", Value::Bool(!conn.is_autocommit())),
        ]));
    }
    Ok(Value::List(results))
}

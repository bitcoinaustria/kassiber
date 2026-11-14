//! Differential driver only; the product exposes no SQL or build dispatch.
use kassiber_envelope::{dict, get, py_str, s};
use kassiber_journals::review_workflow;
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_transfers::Scope;
use std::io::{self, BufRead, Write};

fn run(input: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let field = |key| get(input, key).unwrap_or(&Value::Null);
    let conn = Connection::open_in_memory()?;
    if field("encrypted") == &Value::Bool(true) {
        conn.pragma_update(None, "key", "synthetic-review-case-key")?;
    }
    conn.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA foreign_keys=OFF")?;
    conn.execute_batch(&py_str(field("sql")))?;
    conn.execute_batch("PRAGMA foreign_keys=ON")?;
    match py_str(field("transaction")).as_str() {
        "read" => conn.execute_batch("BEGIN")?,
        "write" => conn.execute_batch("BEGIN IMMEDIATE")?,
        _ => (),
    }
    let owns = conn.is_autocommit();
    let changes = conn.total_changes();
    let profile = field("profile");
    let workspace = py_str(get(profile, "workspace_id").unwrap());
    let profile_id = py_str(get(profile, "id").unwrap());
    let Value::List(cases) = field("cases") else {
        return Err("case requests must be a list".into());
    };
    let mut outputs = Vec::new();
    for args in cases {
        let result = review_workflow::cases(
            &conn,
            &Scope {
                workspace_id: &workspace,
                profile_id: &profile_id,
            },
            args,
            &py_str(field("as_of")),
        );
        assert_eq!(conn.total_changes(), changes);
        assert_eq!(conn.is_autocommit(), owns);
        let result = match result {
            Ok(result) => dict([("result", result)]),
            Err(review_workflow::Error::Service(error)) => dict([(
                "error",
                dict([
                    ("code", s(error.code)),
                    ("message", s(error.message)),
                    ("hint", error.hint.map(s).unwrap_or(Value::Null)),
                    ("details", error.details.unwrap_or(Value::Null)),
                ]),
            )]),
            Err(review_workflow::Error::Build(error)) => dict([(
                "build_error",
                kassiber_pytext::loads(&serde_json::to_string(&error)?)?,
            )]),
        };
        outputs.push(result);
    }
    Ok(Value::List(outputs))
}
fn main() {
    for line in io::stdin().lock().lines() {
        let result = (|| {
            let line = line?;
            run(&kassiber_pytext::loads(&line)?)
        })();
        let result = result.unwrap_or_else(|error| dict([("probe_error", s(error.to_string()))]));
        println!(
            "{}",
            dumps(&result, &Options::default()).expect("finite probe result")
        );
        io::stdout().flush().expect("probe stdout");
    }
}

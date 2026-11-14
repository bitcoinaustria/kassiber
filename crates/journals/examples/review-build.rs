use kassiber_envelope::{get, py_str};
use kassiber_journals::boundary;
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use serde_json::json;
use std::io::{self, BufRead, Write};

fn run(input: &Value) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let field = |key| get(input, key).unwrap_or(&Value::Null);
    let conn = Connection::open_in_memory()?;
    if field("encrypted") == &Value::Bool(true) {
        conn.pragma_update(None, "key", "synthetic-review-build-key")?;
    }
    conn.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA foreign_keys=OFF")?;
    conn.execute_batch(&py_str(field("sql")))?;
    conn.execute_batch("PRAGMA foreign_keys=ON")?;
    conn.execute_batch(if field("writer") == &Value::Bool(true) {
        "BEGIN IMMEDIATE"
    } else {
        "BEGIN"
    })?;
    let changes = conn.total_changes();
    // Review's build: the `journals process` path, with review's quarantine
    // filter. Render entry identities stay empty, as processing leaves them.
    let result = kassiber_journals::pipeline::load_review_state(
        &conn,
        field("profile").clone(),
        &py_str(field("as_of")),
        |state| boundary::canonical(&state),
    )
    .and_then(|state| state)
    .map(|state| json!({"state": state}));
    assert_eq!(conn.total_changes(), changes, "journal build wrote rows");
    assert!(
        !conn.is_autocommit(),
        "journal build ended caller transaction"
    );
    conn.execute_batch("ROLLBACK")?;
    Ok(match result {
        Ok(result) => result,
        Err(error) => json!({"error": error}),
    })
}

fn main() {
    for line in io::stdin().lock().lines() {
        let result = (|| {
            let line = line?;
            let input = kassiber_pytext::loads(&line)?;
            run(&input)
        })();
        let output = result.unwrap_or_else(|error| json!({"probe_error": error.to_string()}));
        println!("{output}");
        io::stdout().flush().expect("probe stdout");
    }
}

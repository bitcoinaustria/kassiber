use kassiber_envelope::{get, py_str};
use kassiber_journals::{boundary, pipeline::Prepared};
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
    let result = (|| {
        let prepared = Prepared::load(&conn, field("profile").clone(), &py_str(field("as_of")))?;
        let Value::List(ids) = field("entry_ids") else {
            panic!("entry ids")
        };
        let mut ids = ids.iter();
        let mut identity = || py_str(ids.next().expect("entry identity tape exhausted"));
        let state = if field("review") == &Value::Bool(true) {
            prepared.review_state(&conn, &mut identity)?
        } else {
            prepared.render(&mut identity)?
        };
        assert!(ids.next().is_none(), "entry identity tape not consumed");
        Ok::<_, kassiber_journals::Refusal>(json!({"state": boundary::canonical(&state)?}))
    })();
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

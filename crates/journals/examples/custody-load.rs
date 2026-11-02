//! Test-only native loader probe. Private book data is returned only on stdout
//! to the local parity harness; no production command registers this interface.
use kassiber_envelope::{get, py_str};
use kassiber_journals::loader;
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::Connection, Book, OpenOptions};
use serde_json::json;
use std::{
    io::{self, BufRead, Write},
    path::Path,
    time::Instant,
};

#[cfg(unix)]
fn cpu() -> (f64, f64) {
    use nix::sys::{
        resource::{getrusage, UsageWho},
        time::TimeValLike,
    };
    let usage = getrusage(UsageWho::RUSAGE_SELF).expect("process CPU clock");
    (
        usage.user_time().num_microseconds() as f64 / 1e6,
        usage.system_time().num_microseconds() as f64 / 1e6,
    )
}
#[cfg(not(unix))]
fn cpu() -> (f64, f64) {
    (0.0, 0.0)
}

fn run(input: &Value) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let field = |key| get(input, key).unwrap_or(&Value::Null);
    let book;
    let memory;
    let conn = if let Value::Str(sql) = field("sql") {
        memory = Connection::open_in_memory()?;
        // Python iterdump restores tables alphabetically, before all FK
        // targets exist. Re-enable checking after this isolated fixture copy.
        memory.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA foreign_keys=OFF")?;
        memory
            .execute_batch(sql.as_str().ok_or("surrogate SQL")?)
            .map_err(|_| "SQL fixture installation failed")?;
        memory.execute_batch("PRAGMA foreign_keys=ON")?;
        &memory
    } else {
        let passphrase = match field("passphrase") {
            Value::Str(value) => value.as_str(),
            _ => None,
        };
        book = Book::open(
            Path::new(&py_str(field("root"))),
            OpenOptions {
                passphrase,
                ..Default::default()
            },
        )?;
        book.connection()
    };
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let before = conn.total_changes();
    if field("sql_floor") == &Value::Bool(true) {
        let profile = py_str(get(field("profile"), "id").unwrap_or(&Value::Null));
        let mut timings = vec![];
        for sql in loader::floor_queries() {
            let mut stmt = conn.prepare_cached(sql)?;
            let clock = cpu();
            let start = Instant::now();
            let mut rows = stmt.query([&profile])?;
            let mut count = 0;
            while rows.next()?.is_some() {
                count += 1;
            }
            let wall = start.elapsed().as_secs_f64();
            let end = cpu();
            timings
                .push(json!({"wall":wall,"user":end.0-clock.0,"sys":end.1-clock.1,"rows":count}));
        }
        conn.execute_batch("ROLLBACK")?;
        return Ok(json!({"sql_floor":timings}));
    }
    let clock = cpu();
    let start = Instant::now();
    let snapshot = match loader::load(conn, field("profile").clone(), &py_str(field("as_of"))) {
        Ok(snapshot) => snapshot,
        Err(loader::Error::Reference(refusal)) => {
            assert_eq!(before, conn.total_changes(), "loader refusal wrote rows");
            assert!(
                !conn.is_autocommit(),
                "loader refusal ended caller transaction"
            );
            conn.execute_batch("ROLLBACK")?;
            let mut output = reference_refusal(refusal);
            output["writes"] = json!(0);
            return Ok(output);
        }
        Err(error) => {
            assert_eq!(before, conn.total_changes(), "loader refusal wrote rows");
            assert!(
                !conn.is_autocommit(),
                "loader refusal ended caller transaction"
            );
            conn.execute_batch("ROLLBACK")?;
            return Ok(json!({"error":error.to_string(),"writes":0}));
        }
    };
    let seconds = start.elapsed().as_secs_f64();
    let load_cpu = cpu();
    if field("project") == &Value::Bool(true) {
        let projection = snapshot.project();
        assert_eq!(before, conn.total_changes(), "projection wrote rows");
        assert!(!conn.is_autocommit(), "projection ended caller transaction");
        conn.execute_batch("ROLLBACK")?;
        let projection = match projection {
            Ok(stages) => {
                let projection = stages
                    .finalized_projection
                    .expect("requested projection")
                    .py(&snapshot.custody().transactions)?;
                let text = kassiber_pyjson::dumps(
                    &projection_diagnostics(&projection),
                    &kassiber_pyjson::Options::new().ensure_ascii(true),
                )?;
                json!({"text":text})
            }
            Err(refusal) => {
                // Preserve the downstream stage's own message, including its
                // existing diagnostic gaps. The loader must not invent one.
                let mut error: kassiber_journals::Refusal = refusal.message.clone().into();
                error.family = refusal.code.clone();
                if ![
                    "ValueError",
                    "TypeError",
                    "KeyError",
                    "OverflowError",
                    "AppError",
                ]
                .contains(&refusal.code.as_str())
                {
                    error.code = Some(refusal.code.clone());
                } else if refusal.code == "AppError" {
                    error.code = Some("app_error".into());
                }
                let mut output = reference_refusal(error);
                output["stage"] = json!(refusal.stage);
                output
            }
        };
        let mut output = json!({"projection":projection,"writes":conn.total_changes()-before});
        // Snapshot bytes remain a distinct transport gate. A serialization
        // refusal cannot replace the projection's actual outcome.
        match snapshot.v1() {
            Ok(v1) => output["v1"] = json!(v1),
            Err(error) => {
                output["v1_error"] = json!("custody snapshot v1 serialization failed");
                if let loader::Error::Json(kassiber_pyjson::Error::BytesNotSerializable) = error {
                    let refusal = reference_refusal(bytes_refusal());
                    output["boundaries"] = refusal["boundaries"].clone();
                }
            }
        }
        return Ok(output);
    }
    let v1 = if field("timing_only") == &Value::Bool(true) {
        None
    } else {
        match snapshot.v1() {
            Ok(text) => Some(text),
            Err(loader::Error::Json(kassiber_pyjson::Error::BytesNotSerializable)) => {
                assert_eq!(
                    before,
                    conn.total_changes(),
                    "serialization refusal wrote rows"
                );
                assert!(
                    !conn.is_autocommit(),
                    "serialization ended caller transaction"
                );
                conn.execute_batch("ROLLBACK")?;
                let mut output = reference_refusal(bytes_refusal());
                output["writes"] = json!(0);
                return Ok(output);
            }
            Err(error) => return Err(error.into()),
        }
    };
    let start = Instant::now();
    let adapter_cpu = cpu();
    let snapshot = snapshot.into_custody();
    let adapter_seconds = start.elapsed().as_secs_f64();
    let end_cpu = cpu();
    assert_eq!(before, conn.total_changes(), "loader wrote rows");
    assert!(!conn.is_autocommit(), "loader ended caller transaction");
    conn.execute_batch("ROLLBACK")?;
    Ok(match snapshot {
        Ok(snapshot) => {
            json!({"v1":v1,"seconds":seconds,"adapter_seconds":adapter_seconds,"user":load_cpu.0-clock.0,"sys":load_cpu.1-clock.1,"adapter_user":end_cpu.0-adapter_cpu.0,"adapter_sys":end_cpu.1-adapter_cpu.1,"transactions":snapshot.transactions.len()})
        }
        Err(_) => {
            json!({"v1":v1,"seconds":seconds,"adapter_seconds":adapter_seconds,"adapter_error":"custody snapshot admission refused"})
        }
    })
}

// Match the existing Python test serializer without coercing nonfinite values
// through serde. ASCII text also retains surrogate code points losslessly.
fn projection_diagnostics(value: &Value) -> Value {
    use kassiber_pyjson::Key;
    match value {
        Value::Bytes(_) => kassiber_pyjson::Value::Dict(vec![(
            kassiber_pyjson::Key::from("$bytes"),
            kassiber_custody::codec::from_json(
                &kassiber_custody::codec::to_json(value)["$kassiber_bytes"],
            )
            .unwrap(),
        )]),
        Value::Float(value) if !value.is_finite() => Value::Dict(vec![(
            Key::from("$float"),
            Value::from(kassiber_pyjson::float_repr(*value)),
        )]),
        Value::List(values) => Value::List(values.iter().map(projection_diagnostics).collect()),
        Value::Dict(values) => Value::Dict(
            values
                .iter()
                .map(|(key, value)| (key.clone(), projection_diagnostics(value)))
                .collect(),
        ),
        value => value.clone(),
    }
}

fn reference_refusal(refusal: kassiber_journals::Refusal) -> serde_json::Value {
    let code = refusal.code.as_deref().unwrap_or("internal_error");
    let cli =
        kassiber_envelope::build_error_envelope(code, &refusal.message, None, None, false, None);
    let daemon = kassiber_envelope::build_error_envelope(
        code,
        &kassiber_envelope::redaction::redact_secret_text(&refusal.message),
        None,
        None,
        false,
        None,
    );
    let project = |value: Value| {
        let error = get(&value, "error").unwrap();
        json!({"code":py_str(get(error,"code").unwrap()), "message":py_str(get(error,"message").unwrap())})
    };
    json!({"error":"custody snapshot reference refused","refusal":refusal,
           "boundaries":{"cli":project(cli),"daemon":project(daemon)}})
}
fn bytes_refusal() -> kassiber_journals::Refusal {
    let mut error: kassiber_journals::Refusal = "Object of type bytes is not JSON serializable"
        .to_owned()
        .into();
    error.family = "TypeError".into();
    error
}
fn main() {
    for line in io::stdin().lock().lines() {
        let output = match line {
            Ok(line) => match kassiber_pytext::loads(&line) {
                Ok(input) => match run(&input) {
                    Ok(value) => value,
                    Err(error) => json!({"error":error.to_string()}),
                },
                Err(_) => json!({"error":"invalid loader probe request"}),
            },
            Err(_) => break,
        };
        println!("{output}");
        io::stdout().flush().expect("probe stdout");
    }
}

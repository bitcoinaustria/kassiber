use kassiber_envelope::{build_error_envelope, dict, get, py_str, success, AppError};
use kassiber_pyjson::{Options, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::{
    freshness::{redact, Queue},
    publication::wallet::Sources,
};
use std::io::Read;

struct Fixed {
    id: u64,
    now: i64,
}
impl Sources for Fixed {
    fn id(&mut self) -> String {
        self.id += 1;
        format!("job-{:04}", self.id)
    }
    fn now(&mut self) -> String {
        self.now += 1;
        kassiber_chain::format_unix_time(1_893_456_000 + self.now).unwrap()
    }
}
fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    get(v, key).unwrap_or(&Value::Null)
}
fn text(v: &Value, key: &str) -> String {
    py_str(field(v, key))
}
fn main() {
    let mut raw = String::new();
    std::io::stdin()
        .take(64 * 1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let input = kassiber_pytext::loads(&raw).unwrap();
    let conn = Connection::open(text(&input, "book")).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let mut sources = Fixed { id: 0, now: 0 };
    let mut results = Vec::new();
    let Value::List(operations) = field(&input, "operations") else {
        panic!("operations required")
    };
    for operation in operations {
        let command = text(operation, "op");
        let args = field(operation, "args");
        let result: Result<Value, AppError> = (|| {
            let mut queue = Queue::new(&conn, &mut sources);
            Ok(match command.as_str() {
                "upsert" => queue.upsert(args)?,
                "enqueue" => queue.enqueue(args)?,
                "cancel" => queue.cancel(&text(args, "id"))?,
                "pause" | "resume" => queue.pause(
                    &text(args, "profile_id"),
                    &text(args, "source_key"),
                    command == "pause",
                )?,
                "recover" => Value::List(
                    queue.recover(
                        get(args, "profile_id")
                            .filter(|v| !matches!(v, Value::Null))
                            .map(py_str)
                            .as_deref(),
                    )?,
                ),
                "due" => Value::List(
                    queue.due(
                        get(args, "profile_id")
                            .filter(|v| !matches!(v, Value::Null))
                            .map(py_str)
                            .as_deref(),
                        get(args, "limit")
                            .map(py_str)
                            .map(|v| v.parse().unwrap())
                            .unwrap_or(10),
                    )?,
                ),
                "dispatchable" => queue
                    .dispatchable(&text(args, "profile_id"), &text(args, "id"))?
                    .unwrap_or(Value::Null),
                "selected" => {
                    let Value::List(ids) = field(args, "ids") else {
                        panic!("ids required")
                    };
                    Value::List(queue.selected(
                        &text(args, "profile_id"),
                        &ids.iter().map(py_str).collect::<Vec<_>>(),
                    )?)
                }
                "snapshot" => queue.snapshot(&text(args, "profile_id"))?,
                "policy" => queue.policy(&text(args, "profile_id"))?,
                "set_policy" => queue.set_policy(&text(args, "profile_id"), args)?,
                "reset_checkpoint" => Value::Bool(queue.reset_checkpoint(
                    &text(args, "profile_id"),
                    &text(args, "source_key"),
                    &text(args, "stale_reason"),
                )?),
                "delete_sources" => {
                    let Value::List(keys) = field(args, "keys") else {
                        panic!("keys required")
                    };
                    queue.delete_sources(
                        &text(args, "profile_id"),
                        &keys.iter().map(py_str).collect::<Vec<_>>(),
                    )?;
                    Value::Null
                }
                "requested_due" => {
                    let Value::List(ids) = field(args, "ids") else {
                        panic!("ids required")
                    };
                    Value::List(queue.requested_due(
                        &text(args, "profile_id"),
                        &ids.iter().map(py_str).collect::<Vec<_>>(),
                        text(args, "limit").parse().unwrap(),
                    )?)
                }
                "dispatch_order" => {
                    let Value::List(ids) = field(args, "ids") else {
                        panic!("ids required")
                    };
                    Value::List(
                        queue
                            .dispatch_order(
                                &text(args, "profile_id"),
                                &ids.iter().map(py_str).collect::<Vec<_>>(),
                            )?
                            .iter()
                            .map(|job| field(job, "id").clone())
                            .collect(),
                    )
                }
                "workspace_profiles" => {
                    let (workspace, profiles) = kassiber_wallets::freshness::workspace_profiles(
                        &conn,
                        &text(args, "workspace_id"),
                    )?;
                    let label = |v: &Value| {
                        dict([
                            ("id", field(v, "id").clone()),
                            ("label", field(v, "label").clone()),
                        ])
                    };
                    dict([
                        ("workspace", label(&workspace)),
                        (
                            "profiles",
                            Value::List(profiles.iter().map(label).collect()),
                        ),
                    ])
                }
                "progress" => {
                    let job = queue.job(&text(args, "id"))?;
                    queue.progress(&job, field(args, "progress"))?;
                    Value::Null
                }
                "progress_removed" => {
                    let job = queue.job(&text(args, "id"))?;
                    conn.execute_batch(
                        "DELETE FROM freshness_jobs; DELETE FROM freshness_source_states;",
                    )
                    .unwrap();
                    queue.progress(&job, field(args, "progress"))?;
                    Value::Null
                }
                "redact" => redact(args),
                "sql" => {
                    conn.execute_batch(&text(args, "sql")).unwrap();
                    Value::Null
                }
                _ => panic!("unknown fixture operation"),
            })
        })();
        results.push(match result {
            Ok(value) => success(&command, value),
            Err(e) => build_error_envelope(
                &e.code,
                &e.message,
                e.details,
                e.hint.as_deref(),
                e.retryable,
                None,
            ),
        });
    }
    println!(
        "{}",
        kassiber_pyjson::dumps(
            &dict([("results", Value::List(results))]),
            &Options::new().sort_keys(true)
        )
        .unwrap()
    );
}

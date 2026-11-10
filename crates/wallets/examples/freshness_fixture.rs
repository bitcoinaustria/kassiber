use kassiber_envelope::{build_error_envelope, dict, get, py_str, success, AppError};
use kassiber_pyjson::{Options, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::{
    freshness::{
        execution::{self, Failure, Handler, Progress},
        planning::{Plan, Request, Scope},
        redact, sources, ui, Queue,
    },
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
struct Script<'a>(&'a Value);
impl Handler for Script<'_> {
    fn supports(&self, kind: &str) -> bool {
        get(field(self.0, "handlers"), kind).is_some()
    }
    fn run(
        &mut self,
        conn: &mut Connection,
        job: &Value,
        progress: &mut Progress<'_>,
    ) -> Result<Value, Failure> {
        let handler = field(field(self.0, "handlers"), &text(job, "job_type"));
        let script = get(field(handler, "jobs"), &text(job, "id")).unwrap_or(handler);
        if let Value::List(steps) = field(script, "steps") {
            for step in steps {
                match text(step, "op").as_str() {
                    "emit" => progress.emit(conn, field(step, "value"))?,
                    "check" => progress.check(conn)?,
                    "commit" => {
                        if !conn.is_autocommit() {
                            conn.execute_batch("COMMIT")?;
                        }
                    }
                    "sql" => {
                        if conn.is_autocommit() {
                            conn.execute_batch("BEGIN DEFERRED")?;
                        }
                        conn.execute(&text(step, "sql"), [])?;
                    }
                    "fail" => {
                        return Err(AppError {
                            code: text(step, "code"),
                            message: text(step, "message"),
                            hint: get(step, "hint")
                                .filter(|v| !matches!(v, Value::Null))
                                .map(|value| py_str(value).into_boxed_str()),
                            details: get(step, "details")
                                .filter(|v| !matches!(v, Value::Null))
                                .cloned(),
                            retryable: get(step, "retryable")
                                .is_some_and(kassiber_envelope::truthy),
                            message_codepoints: None,
                            sqlite_origin: false,
                        }
                        .into())
                    }
                    "database" => {
                        return Err(Failure::Database(
                            kassiber_store::rusqlite::Error::SqliteFailure(
                                kassiber_store::rusqlite::ffi::Error::new(
                                    text(step, "code").parse().unwrap(),
                                ),
                                Some(text(step, "message")),
                            ),
                        ))
                    }
                    "unexpected" => {
                        return Err(Failure::Unexpected {
                            message: text(step, "message"),
                            class: text(step, "class"),
                        })
                    }
                    _ => panic!("unknown script step"),
                }
            }
        }
        Ok(get(script, "result")
            .cloned()
            .unwrap_or_else(|| dict::<&str>([])))
    }
}
fn main() {
    let mut raw = String::new();
    std::io::stdin()
        .take(64 * 1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    println!("{}", run(&raw));
}

/// One fixture invocation: the JSON input a test hands the binary, and the
/// line it prints. The freshness tests also call this in process.
pub fn run(raw: &str) -> String {
    let gate = kassiber_net::Gate::new(
        || kassiber_net::Offline::Unavailable,
        kassiber_net::Ledger::default(),
    );
    let input = kassiber_pytext::loads(raw).unwrap();
    let mut conn = Connection::open(text(&input, "book")).unwrap();
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
                "run_job" | "run_requested" => {
                    let mut events = Vec::new();
                    let mut observer = |value: &Value| {
                        events.push(value.clone());
                        Ok(())
                    };
                    let clock = |seconds: i64| {
                        Ok(
                            kassiber_chain::format_unix_time(1_893_456_000 + seconds.max(0))
                                .unwrap(),
                        )
                    };
                    let mut handler = Script(args);
                    let output = if command == "run_job" {
                        execution::run_job(
                            &mut conn,
                            &text(args, "id"),
                            &mut sources,
                            &mut handler,
                            &mut observer,
                            &clock,
                            get(args, "missing_ok").is_some_and(kassiber_envelope::truthy),
                        )?
                        .unwrap_or(Value::Null)
                    } else {
                        let Value::List(ids) = field(args, "ids") else {
                            panic!("ids required")
                        };
                        Value::List(execution::run_requested(
                            &mut conn,
                            &text(args, "profile_id"),
                            &ids.iter().map(py_str).collect::<Vec<_>>(),
                            text(args, "limit").parse().unwrap(),
                            &mut sources,
                            &mut handler,
                            &mut observer,
                            &clock,
                        )?)
                    };
                    dict([("output", output), ("events", Value::List(events))])
                }
                "ui_payload" => ui::payload(args),
                "ui_snapshot" => ui::payload(&queue.snapshot(&text(args, "profile_id"))?),
                "sync_results" => {
                    let Value::List(jobs) = field(args, "jobs") else {
                        panic!("jobs required")
                    };
                    Value::List(ui::results(jobs))
                }
                "specs" => Value::List(sources::build(
                    &conn,
                    &text(args, "profile_id"),
                    &sources::Options {
                        wallet: get(args, "wallet_ref")
                            .filter(|v| !matches!(v, Value::Null))
                            .map(py_str),
                        rates: get(args, "include_rates").is_none_or(kassiber_envelope::truthy),
                        journals: get(args, "include_journals")
                            .is_none_or(kassiber_envelope::truthy),
                        auto_pair: get(args, "auto_pair_before_journals")
                            .is_some_and(kassiber_envelope::truthy),
                        force_full: get(args, "force_full").is_some_and(kassiber_envelope::truthy),
                    },
                )?),
                "filter_specs" => {
                    let Value::List(specs) = field(args, "specs") else {
                        panic!("specs required")
                    };
                    Value::List(sources::filter_by_policy(
                        specs.clone(),
                        field(args, "policy"),
                        get(args, "force").is_some_and(kassiber_envelope::truthy),
                    ))
                }
                "plan"
                | "plan_busy"
                | "plan_changed"
                | "plan_other_connection"
                | "plan_execute" => {
                    let flag = |key| {
                        get(args, key).and_then(|v| match v {
                            Value::Bool(value) => Some(*value),
                            _ => None,
                        })
                    };
                    let request = Request {
                        scope: get(args, "workspace_id")
                            .map(|v| Scope::Workspace(py_str(v)))
                            .unwrap_or(Scope::ActiveProfile),
                        wallet: get(args, "wallet")
                            .filter(|v| !matches!(v, Value::Null))
                            .map(py_str),
                        rates: flag("rates"),
                        journals: flag("journals"),
                        force_full: flag("force_full").unwrap_or(false),
                        auto_pair: flag("auto_pair").unwrap_or(false),
                        wallets_sync: false,
                    };
                    let backend = || {
                        serde_json::from_str(
                            &kassiber_pyjson::dumps(field(args, "backend"), &Options::new())
                                .unwrap(),
                        )
                        .unwrap()
                    };
                    let plan = Plan::new(&conn, &request, "test", |_, _| Ok(backend()))?;
                    if command == "plan_busy" {
                        let _ = Plan::new(&conn, &request, "test", |_, _| Ok(backend()))?;
                    }
                    if command == "plan_changed" {
                        conn.execute_batch(&text(args, "sql")).unwrap();
                    }
                    if command == "plan_other_connection" {
                        let other = Connection::open(text(&input, "book")).unwrap();
                        plan.validate(&other, |_, _| Ok(backend()))?;
                    }
                    plan.validate(&conn, |_, _| {
                        Ok(get(args, "changed_backend")
                            .map(|v| {
                                serde_json::from_str(
                                    &kassiber_pyjson::dumps(v, &Options::new()).unwrap(),
                                )
                                .unwrap()
                            })
                            .unwrap_or_else(backend))
                    })?;
                    if command == "plan_execute" {
                        let mut events = Vec::new();
                        let mut observer = |value: &Value| {
                            events.push(value.clone());
                            Ok(())
                        };
                        let clock = |seconds: i64| {
                            Ok(
                                kassiber_chain::format_unix_time(1_893_456_000 + seconds.max(0))
                                    .unwrap(),
                            )
                        };
                        let output = plan.execute(
                            &mut conn,
                            &execution::Options {
                                run: get(args, "run").is_none_or(kassiber_envelope::truthy),
                                limit: get(args, "limit").map(|v| py_str(v).parse().unwrap()),
                            },
                            &mut sources,
                            &mut Script(args),
                            &mut observer,
                            &clock,
                            |_, _| Ok(backend()),
                        )?;
                        dict([("output", output), ("events", Value::List(events))])
                    } else {
                        plan.specifications()
                    }
                }
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
        assert!(gate.attempts().records.is_empty());
        assert!(gate.ledger().snapshot(0, 100).records.is_empty());
    }
    kassiber_pyjson::dumps(
        &dict([("results", Value::List(results))]),
        &Options::new().sort_keys(true),
    )
    .unwrap()
}

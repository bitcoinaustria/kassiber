use super::*;
use kassiber_envelope::{build_error_envelope, success};

/// Keys an in-memory SQLCipher book so watch operations see an encrypted database.
pub const WATCH_KEY: &str = "PRAGMA key='analysis-watch-fixture'";

/// `2026-01-01T00:00:00Z`, the fixture's default wall clock.
pub const EPOCH: i64 = 1_767_225_600;

pub struct FixedRuntime {
    pub epoch: i64,
    pub offline: bool,
}
impl Runtime for FixedRuntime {
    fn epoch(&self) -> i64 {
        self.epoch
    }
    fn offline(&self) -> bool {
        self.offline
    }
}

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
    let mut jobs: Vec<Value> = Vec::new();
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
            "datasets.import" | "datasets.import.job" | "datasets.discard.job" => {
                let source = base64(field(action, "source").as_str().unwrap_or(""));
                let limit = match field(action, "cancel_after") {
                    Value::Int(n) => Some(usize::try_from(n.clone()).unwrap()),
                    _ => None,
                };
                let log = std::cell::RefCell::new(Vec::new());
                let progress = |value: Value| log.borrow_mut().push(value);
                let cancelled = || limit.is_some_and(|limit| log.borrow().len() >= limit);
                let args = field(action, "args");
                let reader: Box<dyn std::io::BufRead + Send> =
                    Box::new(std::io::Cursor::new(source));
                let outcome = match operation {
                    "datasets.import" => import_dataset(
                        conn,
                        &Scope {
                            workspace_id: "ws",
                            profile_id: profile,
                        },
                        args,
                        reader,
                        &mut sources,
                    ),
                    "datasets.import.job" => datasets::import(
                        conn,
                        profile,
                        &datasets::Import {
                            manifest: field(args, "manifest"),
                            format: get(args, "format").unwrap_or(&s("csv")),
                            adapter: get(args, "adapter").unwrap_or(&s("generic")),
                            expected_sha256: get(args, "expected_sha256"),
                            progress: &progress,
                            cancelled: &cancelled,
                        },
                        &mut crate::analysis_index::reads::preview::Stream(reader),
                        &mut sources,
                    ),
                    _ => datasets::discard_with(
                        conn,
                        profile,
                        field(args, "id"),
                        &progress,
                        &cancelled,
                    ),
                };
                let kind = format!("ui.chain_analysis.{}", operation.trim_end_matches(".job"));
                let envelope = match outcome {
                    Ok(result) => success(&kind, result),
                    Err(error) => build_error_envelope(
                        &error.code,
                        &error.message,
                        error.details,
                        error.hint.as_deref(),
                        error.retryable,
                        None,
                    ),
                };
                if operation.ends_with(".job") {
                    jobs.push(Value::List(log.into_inner()));
                } else {
                    jobs.push(Value::Null);
                }
                envelope
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
                &mut FixedRuntime {
                    epoch: match field(action, "epoch") {
                        Value::Int(n) => i64::try_from(n.clone()).unwrap(),
                        _ => EPOCH,
                    },
                    offline: truthy(field(case, "offline")),
                },
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
        let mut record = dict([
            ("result", value),
            (
                "state",
                crate::analysis_index::durable::fixture::capture(conn)?,
            ),
            ("in_transaction", Value::Bool(!conn.is_autocommit())),
        ]);
        if let Some(Value::List(progress)) = jobs.pop() {
            set(&mut record, "progress", Value::List(progress));
        }
        results.push(record);
    }
    Ok(Value::List(results))
}

/// Standard base64 for synthetic fixture file bytes.
fn base64(text: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let mut out = Vec::new();
    for chunk in text.as_bytes().chunks(4) {
        let digits: Vec<u8> = chunk
            .iter()
            .copied()
            .filter(|c| *c != b'=')
            .map(value)
            .collect();
        let n = digits
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, d)| n | (u32::from(*d) << (18 - 6 * i)));
        for i in 0..digits.len().saturating_sub(1) {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    out
}

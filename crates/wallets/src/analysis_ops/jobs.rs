//! `daemon_chain_analysis.job_starter` for dataset import and cleanup, on the
//! read port's `analysis_index::runtime`. Workers never receive the caller's
//! connection: each opens its own, fenced to the original database identity,
//! and reads only a picker-staged local file. Nothing egresses.
use super::datasets::{discard_with, get_dataset, import_dataset, Import};
use super::*;
use crate::analysis_index::runtime::{scope_key, Runtime, Worker};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A worker's own connection; dropping it closes the connection.
pub trait WorkerBook: Send {
    fn connection(&self) -> &Connection;
}
impl WorkerBook for kassiber_store::Book {
    fn connection(&self) -> &Connection {
        kassiber_store::Book::connection(self)
    }
}
impl WorkerBook for Connection {
    fn connection(&self) -> &Connection {
        self
    }
}

/// Opens the worker's own connection to the same book, or refuses.
pub type OpenWorker = Box<dyn FnOnce() -> Result<Box<dyn WorkerBook>> + Send>;

const DURATION_LIMIT: Duration = Duration::from_secs(1800);

fn normalize(args: &Value) -> Result<Value> {
    match args {
        Value::Null => Ok(dict::<&str>([])),
        Value::Dict(_) => Ok(args.clone()),
        _ => Err(invalid("Chain analysis arguments must be an object")),
    }
}

/// `ui.chain_analysis.datasets.import` with a staged source token.
pub fn import_token(
    conn: &Connection,
    scope: &Scope<'_>,
    runtime: &Runtime,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    let mut args = normalize(args)?;
    check_scope(&args, scope)?;
    remove(&mut args, "expected_scope");
    arguments(
        &args,
        &[
            "source_token",
            "manifest",
            "format",
            "adapter",
            "expected_sha256",
        ],
        &["manifest", "expected_sha256"],
    )?;
    let key = scope_key(conn, scope.profile_id);
    let token = get(&args, "source_token").cloned().unwrap_or(Value::Null);
    let mut source = runtime.open(&key, &token, "dataset")?;
    let format = get(&args, "format").cloned().unwrap_or(s("csv"));
    let adapter = get(&args, "adapter").cloned().unwrap_or(s("generic"));
    import_dataset(
        conn,
        scope.profile_id,
        &Import {
            manifest: field(&args, "manifest"),
            format: &format,
            adapter: &adapter,
            expected_sha256: get(&args, "expected_sha256"),
            progress: &|_| {},
            cancelled: &|| false,
        },
        &mut source,
        sources,
    )
}

/// The completed local validation an AI dataset import is bound to:
/// `SOURCES.preview(scope_key(conn, profile), args["source_token"], recipe,
/// args["expected_sha256"])` with Python's recipe
/// `{"manifest": normalize_manifest(args["manifest"]), "format", "adapter"}`.
/// Never reads the file again on this thread.
pub fn remembered_preview(
    conn: &Connection,
    profile: &str,
    runtime: &Runtime,
    args: &Value,
) -> Result<Value> {
    let manifest =
        crate::analysis_index::reads::preview::normalize_manifest(field(args, "manifest"))?;
    let recipe = dict([
        ("manifest", manifest),
        ("format", get(args, "format").cloned().unwrap_or(s("csv"))),
        (
            "adapter",
            get(args, "adapter").cloned().unwrap_or(s("generic")),
        ),
    ]);
    runtime.preview(
        &scope_key(conn, profile),
        get(args, "source_token").unwrap_or(&Value::Null),
        &recipe,
        get(args, "expected_sha256").unwrap_or(&Value::Null),
    )
}

/// `datasets.import.start` / `datasets.discard.start`: validate on the
/// daemon thread, then admit one bounded worker and return its receipt.
#[allow(clippy::too_many_arguments)]
pub fn start(
    conn: &Connection,
    scope: &Scope<'_>,
    runtime: &Arc<Runtime>,
    operation: &str,
    args: &Value,
    open_worker: OpenWorker,
    mut ids: Box<dyn Sources + Send>,
) -> Result<Value> {
    let mut args = normalize(args)?;
    check_scope(&args, scope)?;
    remove(&mut args, "expected_scope");
    let discarding = operation == "datasets.discard.start";
    if discarding {
        arguments(&args, &["id"], &["id"])?;
    } else {
        arguments(
            &args,
            &[
                "source_token",
                "manifest",
                "format",
                "adapter",
                "expected_sha256",
            ],
            &["source_token", "manifest", "expected_sha256"],
        )?;
    }
    let manifest = if discarding {
        Value::Null
    } else {
        crate::analysis_index::reads::preview::normalize_manifest(field(&args, "manifest"))?
    };
    let profile = scope.profile_id.to_owned();
    let key = scope_key(conn, &profile);
    let token = get(&args, "source_token").cloned().unwrap_or(Value::Null);
    // Refuse stale or foreign selections before creating a running receipt.
    if discarding {
        let status = field(&get_dataset(conn, &profile, field(&args, "id"))?, "status").clone();
        if status != s("staging") && status != s("failed") {
            return Err(invalid("Only incomplete imports can be discarded"));
        }
    } else {
        drop(runtime.open(&key, &token, "dataset")?);
    }
    // The worker opens its own connection, fenced to this identity.
    super::watches::database_instance_id(conn)?;
    let format = get(&args, "format").cloned().unwrap_or(s("csv"));
    let adapter = get(&args, "adapter").cloned().unwrap_or(s("generic"));
    let expected = get(&args, "expected_sha256")
        .cloned()
        .unwrap_or(Value::Null);
    let mut submitted = dict([
        ("operation", s(operation)),
        ("manifest", manifest.clone()),
        ("format", format.clone()),
        ("adapter", adapter.clone()),
        ("expected_sha256", expected.clone()),
    ]);
    let id = field(&args, "id").clone();
    if discarding {
        set(&mut submitted, "id", id.clone());
    }
    let sources = Arc::clone(runtime);
    let worker_key = key.clone();
    let compute = move |worker: &Worker<'_>| -> Result<Value> {
        let began = Instant::now();
        let stopped = || worker.cancelled() || began.elapsed() >= DURATION_LIMIT;
        let phase = if discarding { "discarding" } else { "staging" };
        let update = |value: Value| {
            let mut receipt = dict([("phase", s(phase))]);
            if let Value::Dict(pairs) = value {
                for (key, item) in pairs {
                    if let kassiber_pyjson::Key::Str(key) = key {
                        set(&mut receipt, &routing::py_text(&Value::Str(key)), item);
                    }
                }
            }
            worker.progress(receipt);
        };
        if stopped() {
            return Ok(dict([("status", s("cancelled"))]));
        }
        let outcome = (|| -> Result<Value> {
            let book = open_worker()?;
            let conn = book.connection();
            if discarding {
                return discard_with(conn, &profile, &id, &update, &stopped);
            }
            let mut source = sources.open(&worker_key, &token, "dataset")?;
            import_dataset(
                conn,
                &profile,
                &Import {
                    manifest: &manifest,
                    format: &format,
                    adapter: &adapter,
                    expected_sha256: Some(&expected),
                    progress: &update,
                    cancelled: &stopped,
                },
                &mut source,
                &mut *ids,
            )
        })();
        match outcome {
            Err(_) if stopped() => Ok(dict([
                ("status", s("cancelled")),
                (
                    "reason",
                    s(if worker.cancelled() {
                        "cancelled"
                    } else {
                        "duration_limit"
                    }),
                ),
            ])),
            other => other,
        }
    };
    runtime.start(conn, key, submitted, Box::new(compute))
}

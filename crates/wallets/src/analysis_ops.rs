//! Authored investigation operations. These claims never grant ownership or
//! accounting authority. The transport resolves scope; this service owns writes.
use crate::analysis_index::Text;
use crate::database_error;
use crate::publication::row::{field, first, py_eq, Result};
use crate::publication::wallet::{Sources, SystemSources};
use kassiber_envelope::{dict, get, s, set, truthy, AppError};
use kassiber_pyjson::{dumps, Key, Options, Value};
use kassiber_pytext::{lower, strip_py};
use kassiber_store::rusqlite::{params, types::Value as Sql, Connection};

mod acquisition;
mod cases;
pub mod datasets;
#[cfg(any(test, feature = "test-harness"))]
pub mod fixture;
pub mod jobs;
mod labels;
mod routing;
mod sources;
mod watches;

pub struct Scope<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
}

/// Facts Python reads ambiently: wall-clock seconds, the offline preference
/// and a caller's cancellation. None of them grants network authority.
pub trait Runtime {
    fn epoch(&self) -> i64;
    fn offline(&self) -> bool;
    fn cancelled(&self) -> bool {
        false
    }
    /// `KASSIBER_NO_EGRESS`, read like `egress_policy._environment_blocks_egress`.
    fn no_egress(&self) -> bool {
        matches!(
            std::env::var("KASSIBER_NO_EGRESS")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        )
    }
    /// The transport gate for an already reviewed explicit action. The
    /// default reads no preference and therefore refuses every route.
    fn gate(&self) -> kassiber_net::Gate {
        kassiber_net::Gate::new(
            || kassiber_net::Offline::Unavailable,
            kassiber_net::Ledger::default(),
        )
    }
}

/// Wall clock with a fail-closed offline reading.
pub struct SystemRuntime {
    pub offline: bool,
}
impl Runtime for SystemRuntime {
    fn epoch(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }
    fn offline(&self) -> bool {
        self.offline
    }
}

pub fn execute(
    conn: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
) -> Result<Value> {
    execute_with_sources(
        conn,
        scope,
        operation,
        args,
        &mut SystemSources::default(),
        &mut SystemRuntime { offline: true },
    )
}

pub fn execute_with_sources(
    conn: &Connection,
    scope: &Scope<'_>,
    operation: &str,
    args: &Value,
    sources: &mut dyn Sources,
    runtime: &mut dyn Runtime,
) -> Result<Value> {
    let mut args = match args {
        Value::Null => dict::<&str>([]),
        Value::Dict(_) => args.clone(),
        _ => return Err(invalid("Chain analysis arguments must be an object")),
    };
    check_scope(&args, scope)?;
    remove(&mut args, "expected_scope");
    match operation {
        "labels.upsert" => labels::upsert(conn, scope.profile_id, &args, sources),
        "labels.delete" => labels::delete(conn, scope.profile_id, &args, sources),
        "labels.import" => labels::import_labels(conn, scope.profile_id, &args, sources),
        "cases.save" => cases::save(conn, scope.profile_id, &args, sources),
        "cases.delete" => cases::delete(conn, scope.profile_id, &args),
        "datasets.revoke" => datasets::revoke(conn, scope.profile_id, &args, sources),
        "datasets.discard" => datasets::discard(conn, scope.profile_id, &args),
        "watches.preview"
        | "watches.create"
        | "watches.list"
        | "watches.configure"
        | "watches.delete"
        | "watches.inbox"
        | "watches.acknowledge"
        | "watches.evaluate" => {
            watches::dispatch(conn, scope.profile_id, &operation[8..], &args, sources)
        }
        "acquire.plan" => acquisition::plan(conn, scope.profile_id, &args, sources),
        "acquire.apply" => acquisition::apply(conn, scope.profile_id, &args, sources, runtime),
        "sources.plan" | "sources.authorize" | "sources.list" | "sources.revoke"
        | "sources.run" => {
            sources::dispatch(conn, scope.profile_id, operation, &args, sources, runtime)
        }
        _ => Err(AppError::new(
            "not_implemented",
            "Unsupported chain analysis operation",
        )),
    }
}

/// `datasets.import` over a caller-opened local file (the CLI's `--file`).
/// Read failures surface as the CLI's open failure; nothing egresses.
pub fn import_dataset(
    conn: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    file: Box<dyn std::io::BufRead + Send>,
    sources: &mut dyn Sources,
) -> Result<Value> {
    let mut args = match args {
        Value::Null => dict::<&str>([]),
        Value::Dict(_) => args.clone(),
        _ => return Err(invalid("Chain analysis arguments must be an object")),
    };
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
    let format = get(&args, "format").cloned().unwrap_or(s("csv"));
    let adapter = get(&args, "adapter").cloned().unwrap_or(s("generic"));
    let mut stream = crate::analysis_index::reads::preview::Stream(file);
    datasets::import_dataset(
        conn,
        scope.profile_id,
        &datasets::Import {
            manifest: field(&args, "manifest"),
            format: &format,
            adapter: &adapter,
            expected_sha256: get(&args, "expected_sha256"),
            progress: &|_| {},
            cancelled: &|| false,
        },
        &mut stream,
        sources,
    )
}

/// `daemon_chain_analysis_watches.has_pending_work`: enabled watches or
/// active source grants. A plain read; it never contacts anything.
pub fn has_pending_work(conn: &Connection) -> Result<bool> {
    Ok(first(
        conn,
        "SELECT 1 FROM chain_analysis_watches WHERE enabled=1 UNION ALL SELECT 1 FROM chain_analysis_acquisition_grants WHERE status='active' LIMIT 1",
        [],
    )?
    .is_some())
}

/// `worker_tick`: publish authorized source batches before evaluating the
/// dependent watches. Offline mode pauses acquisition but not local watches.
pub fn worker_tick(
    conn: &Connection,
    sources: &mut dyn Sources,
    runtime: &dyn Runtime,
) -> Result<()> {
    if !has_pending_work(conn)? {
        return Ok(());
    }
    let profiles = crate::publication::row::rows(
        conn,
        "SELECT profile_id FROM chain_analysis_watches WHERE enabled=1 UNION SELECT profile_id FROM chain_analysis_acquisition_grants WHERE status='active' ORDER BY profile_id",
        [],
    )?;
    let acquire = !runtime.offline();
    for row in profiles {
        if runtime.cancelled() {
            return Ok(());
        }
        let profile = routing::py_text(field(&row, "profile_id"));
        if acquire {
            acquisition::run_due(conn, &profile, sources, runtime)?;
        }
        if !runtime.cancelled()
            && first(
                conn,
                "SELECT 1 FROM chain_analysis_watches WHERE profile_id=? AND enabled=1 LIMIT 1",
                [profile.as_str()],
            )?
            .is_some()
        {
            watches::evaluate_due(conn, &profile, &|| runtime.cancelled(), sources)?;
        }
        if !conn.is_autocommit() {
            conn.execute_batch("COMMIT").map_err(database_error)?;
        }
    }
    Ok(())
}

/// Undelivered, unacknowledged inbox events for the generic wake-up event.
pub fn pending_deliveries(conn: &Connection) -> Result<Vec<String>> {
    Ok(crate::publication::row::rows(
        conn,
        "SELECT id FROM chain_analysis_watch_inbox WHERE delivered_at IS NULL AND acknowledged_at IS NULL LIMIT 100",
        [],
    )?
    .iter()
    .map(|row| routing::py_text(field(row, "id")))
    .collect())
}

/// Receipt after the at-least-once `ui.chain_analysis.watches.changed` event.
pub fn mark_delivered(conn: &Connection, ids: &[String], sources: &mut dyn Sources) -> Result<()> {
    for id in ids {
        conn.execute(
            "UPDATE chain_analysis_watch_inbox SET delivered_at=? WHERE id=?",
            params![sources.now(), id],
        )
        .map_err(database_error)?;
    }
    if !conn.is_autocommit() {
        conn.execute_batch("COMMIT").map_err(database_error)?;
    }
    Ok(())
}

/// A runtime whose cancellation is the worker's stop flag.
struct Stoppable<'a> {
    inner: &'a dyn Runtime,
    stop: &'a dyn Fn() -> bool,
}
impl Runtime for Stoppable<'_> {
    fn epoch(&self) -> i64 {
        self.inner.epoch()
    }
    fn offline(&self) -> bool {
        self.inner.offline()
    }
    fn cancelled(&self) -> bool {
        (self.stop)()
    }
    fn no_egress(&self) -> bool {
        self.inner.no_egress()
    }
    fn gate(&self) -> kassiber_net::Gate {
        self.inner.gate()
    }
}

/// `daemon_chain_analysis_watches.start_worker`: requires a keyed book.
pub fn watch_worker_ready(conn: &Connection) -> bool {
    watches::require_encrypted(conn).is_ok()
}

/// The worker thread's state: its own fenced connection, opened on first use.
pub struct LocalWorker {
    open: Option<jobs::OpenWorker>,
    book: Option<Box<dyn jobs::WorkerBook>>,
    sources: Box<dyn Sources + Send>,
    runtime: Box<dyn Runtime + Send>,
}

impl LocalWorker {
    pub fn new(
        open: jobs::OpenWorker,
        sources: Box<dyn Sources + Send>,
        runtime: Box<dyn Runtime + Send>,
    ) -> Self {
        Self {
            open: Some(open),
            book: None,
            sources,
            runtime,
        }
    }

    fn ensure_open(&mut self) -> Result<()> {
        if self.book.is_none() {
            let open = self.open.take().ok_or_else(|| {
                AppError::new("internal_error", "Local evidence worker unavailable")
            })?;
            self.book = Some(open()?);
        }
        Ok(())
    }

    /// `worker_tick` then `deliver_pending`'s query; failures roll back.
    pub fn pass(&mut self, stop: &dyn Fn() -> bool) -> Result<Vec<String>> {
        self.ensure_open()?;
        let conn = self.book.as_ref().unwrap().connection();
        let runtime = Stoppable {
            inner: &*self.runtime,
            stop,
        };
        let result =
            worker_tick(conn, &mut *self.sources, &runtime).and_then(|()| pending_deliveries(conn));
        if result.is_err() && !conn.is_autocommit() {
            let _ = conn.execute_batch("ROLLBACK");
        }
        result
    }

    pub fn delivered(&mut self, ids: &[String]) -> Result<()> {
        self.ensure_open()?;
        let conn = self.book.as_ref().unwrap().connection();
        mark_delivered(conn, ids, &mut *self.sources)
    }
}

/// `expected_scope`: an operation stays bound to its original book.
fn check_scope(args: &Value, scope: &Scope<'_>) -> Result<()> {
    if let Some(expected) = get(args, "expected_scope") {
        if expected != &Value::Null
            && !py_eq(
                expected,
                &dict([
                    ("workspace_id", s(scope.workspace_id)),
                    ("profile_id", s(scope.profile_id)),
                ]),
            )
        {
            return Err(AppError::new(
                "scope_changed",
                "The active book changed; reopen the investigation in its original book",
            ));
        }
    }
    Ok(())
}

/// Python's `WRITE_OPERATIONS`: only these commit the caller's connection.
pub fn is_write(operation: &str) -> bool {
    !matches!(
        operation,
        "watches.preview"
            | "watches.list"
            | "watches.inbox"
            | "sources.plan"
            | "sources.list"
            | "acquire.plan"
    )
}

fn invalid(message: &str) -> AppError {
    AppError::new("validation", message)
}

fn stale(message: &str) -> AppError {
    AppError::new("chain_analysis_stale", message).retryable(true)
}

fn arguments(args: &Value, allowed: &[&str], required: &[&str]) -> Result<()> {
    let Value::Dict(pairs) = args else {
        return Err(invalid("Unsupported or missing chain analysis arguments"));
    };
    if pairs.iter().any(|(key, _)| {
        !matches!(key, Key::Str(text) if text.as_str().is_some_and(|k| allowed.contains(&k)))
    }) || required.iter().any(|key| get(args, key).is_none()) {
        return Err(invalid("Unsupported or missing chain analysis arguments"));
    }
    Ok(())
}

fn text_value(value: &Value, field: &str, limit: usize) -> Result<kassiber_pyjson::PyStr> {
    if let Value::Str(text) = value {
        if text.code_points().count() <= limit && !text.code_points().any(|c| c < 32) {
            let text = strip_py(text);
            if text.code_points().next().is_some() {
                return Ok(text);
            }
        }
    }
    Err(invalid(&format!(
        "{field} must be nonempty text up to {limit} characters"
    )))
}

fn remove(value: &mut Value, name: &str) {
    if let Value::Dict(pairs) = value {
        pairs.retain(|(k, _)| k != &Key::from(name));
    }
}

fn canonical(value: &Value) -> Result<String> {
    dumps(
        value,
        &Options::new().sort_keys(true).compact().allow_nan(false),
    )
    .map_err(|error| AppError::new("internal_error", error.to_string()))
}

fn digest(value: &Value) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(Sha256::digest(canonical(value)?.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn sql(value: &Value, index: usize) -> Result<Sql> {
    Ok(match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Int(n) => Sql::Integer(i64::try_from(n.clone()).map_err(|_| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        })?),
        Value::Float(n) => Sql::Real(*n),
        Value::Str(t) => Sql::Text(t.as_str().ok_or_else(|| encoding_error(t))?.to_owned()),
        other => {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "Error binding parameter {index}: type '{}' is not supported",
                    kassiber_envelope::type_name(other)
                ),
            ))
        }
    })
}

fn encoding_error(text: &kassiber_pyjson::PyStr) -> AppError {
    let points: Vec<_> = text.code_points().collect();
    let start = points
        .iter()
        .position(|p| (0xd800..=0xdfff).contains(p))
        .unwrap();
    let end = start
        + points[start..]
            .iter()
            .take_while(|p| (0xd800..=0xdfff).contains(*p))
            .count();
    let position = if end == start + 1 {
        format!("character '\\u{:04x}' in position {start}", points[start])
    } else {
        format!("characters in position {start}-{}", end - 1)
    };
    AppError::new(
        "internal_error",
        format!("'utf-8' codec can't encode {position}: surrogates not allowed"),
    )
}

fn atomic<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT chain_analysis_ops")
        .map_err(database_error)?;
    match f() {
        Ok(value) => match conn.execute_batch("RELEASE SAVEPOINT chain_analysis_ops") {
            Ok(()) => Ok(value),
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK TO SAVEPOINT chain_analysis_ops; RELEASE SAVEPOINT chain_analysis_ops");
                Err(database_error(e))
            }
        },
        Err(e) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT chain_analysis_ops; RELEASE SAVEPOINT chain_analysis_ops",
            );
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests;

use super::{
    dict, empty, field, integer, json, or_text, put, redact, s, truthy, Queue, Result, Value,
    TERMINAL,
};
use crate::publication::wallet::Sources;
use kassiber_envelope::{
    redaction::{redact_operational_text, redact_operational_value},
    AppError,
};
use kassiber_store::rusqlite::{self, params, Connection};
use sha2::{Digest, Sha256};

pub enum Failure {
    App(AppError),
    Database(rusqlite::Error),
    Unexpected { message: String, class: String },
}

impl From<AppError> for Failure {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}
impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

pub trait Handler {
    fn supports(&self, job_type: &str) -> bool;
    fn prefetch(
        &mut self,
        _conn: &Connection,
        _jobs: &[Value],
        _observer: &mut dyn FnMut(&Value) -> Result<()>,
    ) -> Result<()> {
        Ok(())
    }
    fn run(
        &mut self,
        conn: &mut Connection,
        job: &Value,
        progress: &mut Progress<'_>,
    ) -> std::result::Result<Value, Failure>;
    /// `_wallets_sync_payload`'s direct `sync_wallet` when the freshness run
    /// produced nothing: the rows, or `None` when this handler has no route.
    fn direct_sync(
        &mut self,
        _conn: &mut Connection,
        _action: &crate::sync::Sync,
        _sources: &mut dyn Sources,
    ) -> Result<Option<Vec<Value>>> {
        Ok(None)
    }
}

pub struct Progress<'a> {
    job: Value,
    pub(super) sources: &'a mut dyn Sources,
    observer: &'a mut dyn FnMut(&Value) -> Result<()>,
}

pub(super) fn begin(conn: &Connection) -> Result<()> {
    if conn.is_autocommit() {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(crate::database_error)?;
    }
    Ok(())
}
pub(super) fn commit(conn: &Connection) -> Result<()> {
    if !conn.is_autocommit() {
        conn.execute_batch("COMMIT")
            .map_err(crate::database_error)?;
    }
    Ok(())
}
fn rollback(conn: &Connection) -> Result<()> {
    if !conn.is_autocommit() {
        conn.execute_batch("ROLLBACK")
            .map_err(crate::database_error)?;
    }
    Ok(())
}
fn removed() -> AppError {
    AppError::new("state_not_ready", "Freshness job disappeared")
}
fn is_removed(error: &AppError) -> bool {
    error == &removed()
}

impl Progress<'_> {
    pub fn check(&self, conn: &Connection) -> Result<()> {
        check_job(conn, &or_text(field(&self.job, "id")))
    }

    pub fn emit(&mut self, conn: &Connection, value: &Value) -> Result<()> {
        self.check(conn)?;
        // An event inside a caller's transaction persists with it.
        let owned = conn.is_autocommit();
        begin(conn)?;
        Queue::new(conn, self.sources).progress(&self.job, value)?;
        let mut event = dict(
            [
                "job_id",
                "job_type",
                "source_key",
                "source_type",
                "source_label",
            ]
            .map(|key| {
                (
                    key,
                    field(&self.job, if key == "job_id" { "id" } else { key }).clone(),
                )
            }),
        );
        if let Value::Dict(pairs) = redact(value) {
            for (key, value) in pairs {
                put(&mut event, &super::key_text(&key), value);
            }
        }
        (self.observer)(&event)?;
        if owned {
            commit(conn)?;
        }
        Ok(())
    }
}

pub(super) fn check_job(conn: &Connection, id: &str) -> Result<()> {
    let row = super::first(
        conn,
        "SELECT cancel_requested,status FROM freshness_jobs WHERE id=?",
        [id],
    )?
    .ok_or_else(removed)?;
    if truthy(field(&row, "cancel_requested")) || field(&row, "status") == &s("cancelled") {
        return Err(AppError::new("cancelled", "Freshness job was cancelled"));
    }
    Ok(())
}

pub fn retry_time(seconds: i64) -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| AppError::new("internal_error", "Invalid system time"))?
        .as_secs();
    kassiber_chain::format_unix_time((now as i64).saturating_add(seconds.max(0)))
        .ok_or_else(|| AppError::new("internal_error", "Invalid freshness retry time"))
}

fn backoff(job: &Value, busy: bool) -> Result<i64> {
    let attempts = integer(field(job, "attempts"))?
        .max(if busy { 1 } else { 0 })
        .saturating_add(i64::from(!busy));
    let base = 30_i64
        .saturating_mul(
            1_i64
                .checked_shl((attempts - 1).clamp(0, 10) as u32)
                .unwrap_or(i64::MAX),
        )
        .min(21600);
    let seed = format!(
        "{}{}:{attempts}",
        if busy { "database-busy:" } else { "" },
        or_text(field(job, "id"))
    );
    let digest = Sha256::digest(seed.as_bytes());
    let jitter = u16::from_be_bytes([digest[0], digest[1]]) as i64 % ((base / 4).min(300) + 1);
    Ok(base + jitter)
}

fn retry(
    error: &AppError,
    job: &Value,
    clock: &dyn Fn(i64) -> Result<String>,
) -> Result<(Option<String>, Option<String>)> {
    let details = error.details.as_ref().unwrap_or(&Value::Null);
    let after = if truthy(field(details, "retry_after_seconds")) {
        field(details, "retry_after_seconds")
    } else {
        field(details, "retry_after")
    };
    if !matches!(after, Value::Null) {
        if let Ok(seconds) = integer(after) {
            let reason = if truthy(field(details, "retry_reason")) {
                or_text(field(details, "retry_reason"))
            } else {
                "retry-after".into()
            };
            return Ok((Some(clock(seconds)?), Some(reason)));
        }
    }
    if error.message.to_lowercase().contains("429") || error.code.to_lowercase().contains("rate") {
        return Ok((
            Some(clock(backoff(job, false)?)?),
            Some("exponential-backoff".into()),
        ));
    }
    Ok((None, None))
}

fn wrap(conn: &Connection, job: &Value, failure: Failure) -> Result<AppError> {
    match failure {
        Failure::App(error) => Ok(error),
        Failure::Unexpected { message, class } => {
            let message = if message.is_empty() {
                class.rsplit('.').next().unwrap_or(&class).to_owned()
            } else {
                message
            };
            Ok(AppError::new("freshness_job_failed", message)
                .retryable(true)
                .with_details(dict([("error_class", s(class))])))
        }
        Failure::Database(error) => {
            let mut details = empty();
            if let rusqlite::Error::SqliteFailure(code, _) = &error {
                let name = match code.extended_code {
                    5 => "SQLITE_BUSY",
                    6 => "SQLITE_LOCKED",
                    261 => "SQLITE_BUSY_RECOVERY",
                    517 => "SQLITE_BUSY_SNAPSHOT",
                    773 => "SQLITE_BUSY_TIMEOUT",
                    262 => "SQLITE_LOCKED_SHAREDCACHE",
                    518 => "SQLITE_LOCKED_VTAB",
                    1 => "SQLITE_ERROR",
                    19 => "SQLITE_CONSTRAINT",
                    787 => "SQLITE_CONSTRAINT_FOREIGNKEY",
                    1555 => "SQLITE_CONSTRAINT_PRIMARYKEY",
                    1811 => "SQLITE_CONSTRAINT_TRIGGER",
                    2067 => "SQLITE_CONSTRAINT_UNIQUE",
                    _ => "",
                };
                let class =
                    match code.code {
                        rusqlite::ErrorCode::ConstraintViolation
                        | rusqlite::ErrorCode::TypeMismatch => "sqlite3.IntegrityError",
                        rusqlite::ErrorCode::TooBig => "sqlite3.DataError",
                        rusqlite::ErrorCode::InternalMalfunction
                        | rusqlite::ErrorCode::NotFound => "sqlite3.InternalError",
                        rusqlite::ErrorCode::ApiMisuse
                        | rusqlite::ErrorCode::ParameterOutOfRange => "sqlite3.ProgrammingError",
                        rusqlite::ErrorCode::OutOfMemory => "builtins.MemoryError",
                        rusqlite::ErrorCode::DatabaseCorrupt
                        | rusqlite::ErrorCode::NotADatabase
                        | rusqlite::ErrorCode::AuthorizationForStatementDenied => {
                            "sqlite3.DatabaseError"
                        }
                        _ => "sqlite3.OperationalError",
                    };
                put(&mut details, "error_class", s(class));
                put(
                    &mut details,
                    "sqlite_error_code",
                    Value::from(code.extended_code),
                );
                if !name.is_empty() {
                    put(&mut details, "sqlite_error_name", s(name));
                }
                if matches!(
                    code.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) {
                    rollback(conn)?;
                    put(
                        &mut details,
                        "retry_after_seconds",
                        Value::from(backoff(job, true)?),
                    );
                    put(&mut details, "retry_reason", s("database-busy"));
                    return Ok(AppError::new("database_busy","The local database is busy; this freshness job will retry automatically.").with_hint("No action is required unless retries continue after other Kassiber work finishes.").retryable(true).with_details(details));
                }
            }
            Ok(AppError::new("freshness_job_failed", error.to_string())
                .retryable(true)
                .with_details(details))
        }
    }
}

impl Queue<'_> {
    fn running(&mut self, job: &Value) -> Result<Value> {
        let now = self.sources.now();
        self.conn.execute("UPDATE freshness_jobs SET status='running',phase='discovery',attempts=attempts+1,started_at=COALESCE(started_at,?),updated_at=? WHERE id=? AND status IN ('queued','rate_limited')",params![now,now,or_text(field(job,"id"))]).map_err(crate::database_error)?;
        self.progress(job, &dict([("phase", s("discovery"))]))?;
        self.job(&or_text(field(job, "id")))
    }

    fn success(&mut self, job: &Value, result: &Value) -> Result<Value> {
        let redacted = redact(result);
        let state = self
            .state(
                &or_text(field(job, "profile_id")),
                &or_text(field(job, "source_key")),
            )?
            .unwrap_or_else(empty);
        let checkpoint = match field(result, "freshness_checkpoint") {
            value @ Value::Dict(_) => value,
            _ => field(&state, "checkpoint"),
        };
        let now = self.sources.now();
        let count = self.conn.execute("UPDATE freshness_jobs SET status='done',phase='done',result_json=?,finished_at=?,updated_at=? WHERE id=?",params![json(&redacted)?,now,now,or_text(field(job,"id"))]).map_err(crate::database_error)?;
        if count == 0 {
            return Err(removed());
        }
        let partial = truthy(field(&redacted, "partial_success"));
        let mut state = Self::state_input(job, if partial { "partially_stale" } else { "fresh" });
        for (key, value) in [
            (
                "stale_reason",
                if partial {
                    s("partial_success")
                } else {
                    Value::Null
                },
            ),
            (
                "blocking_reports",
                Value::Bool(truthy(field(&redacted, "blocking_reports"))),
            ),
            ("retry_count", Value::from(0)),
            ("last_success_at", s(now)),
            ("last_phase", s("done")),
            ("progress", dict([("phase", s("done"))])),
            ("checkpoint", checkpoint.clone()),
        ] {
            put(&mut state, key, value);
        }
        self.upsert(&state)?;
        self.job(&or_text(field(job, "id")))
    }

    fn error(
        &mut self,
        job: &Value,
        error: &AppError,
        clock: &dyn Fn(i64) -> Result<String>,
    ) -> Result<Value> {
        let (cooldown, reason) = retry(error, job, clock)?;
        let now = self.sources.now();
        let code = if error.code.is_empty() {
            "freshness_job_failed"
        } else {
            &error.code
        };
        let payload = redact(&dict([
            ("code", s(code)),
            ("message", s(redact_operational_text(&error.message))),
            (
                "hint",
                error
                    .hint
                    .as_ref()
                    .map_or(Value::Null, |v| s(redact_operational_text(v))),
            ),
            ("retryable", Value::Bool(error.retryable)),
            (
                "details",
                error
                    .details
                    .as_ref()
                    .map_or(Value::Null, redact_operational_value),
            ),
            (
                "rate_limited_until",
                cooldown.as_ref().map_or(Value::Null, |v| s(v.as_str())),
            ),
        ]));
        let status = if cooldown.is_some() {
            "rate_limited"
        } else {
            "error"
        };
        let count = self.conn.execute("UPDATE freshness_jobs SET status=?,phase='error',error_json=?,cooldown_until=?,run_after=?,finished_at=CASE WHEN ? IS NULL THEN ? ELSE finished_at END,updated_at=? WHERE id=?",params![status,json(&payload)?,cooldown,cooldown,cooldown,now,now,or_text(field(job,"id"))]).map_err(crate::database_error)?;
        if count == 0 {
            return Err(removed());
        }
        let mut state = Self::state_input(
            job,
            if cooldown.is_some() {
                "rate_limited"
            } else {
                "failed"
            },
        );
        for (key, value) in [
            ("stale_reason", s(code)),
            (
                "blocking_reports",
                Value::Bool(field(job, "job_type") != &s("wallet_graph_references")),
            ),
            (
                "rate_limited_until",
                cooldown.as_ref().map_or(Value::Null, |v| s(v.as_str())),
            ),
            (
                "cooldown_reason",
                reason.as_ref().map_or(Value::Null, |v| s(v.as_str())),
            ),
            ("retry_count", field(job, "attempts").clone()),
            ("last_error_at", s(now)),
            ("last_error_code", s(code)),
            (
                "last_error_message",
                s(redact_operational_text(&error.message)),
            ),
            ("last_phase", s("error")),
            ("progress", dict([("phase", s("error"))])),
        ] {
            put(&mut state, key, value);
        }
        self.upsert(&state)?;
        self.job(&or_text(field(job, "id")))
    }
}

pub fn run_job(
    conn: &mut Connection,
    id: &str,
    sources: &mut dyn Sources,
    handler: &mut dyn Handler,
    observer: &mut dyn FnMut(&Value) -> Result<()>,
    clock: &dyn Fn(i64) -> Result<String>,
    missing_ok: bool,
) -> Result<Option<Value>> {
    let job = match Queue::new(conn, sources).job(id) {
        Err(error) if missing_ok && error.code == "not_found" => return Ok(None),
        result => result?,
    };
    let result = (|| {
        if TERMINAL.contains(&or_text(field(&job, "status")).as_str()) {
            return Ok(job);
        }
        let kind = or_text(field(&job, "job_type"));
        if !handler.supports(&kind) {
            return Err(AppError::new(
                "config_error",
                format!("No freshness handler is configured for '{kind}'"),
            ));
        }
        begin(conn)?;
        let job = Queue::new(conn, sources).running(&job)?;
        commit(conn)?;
        let mut progress = Progress {
            job: job.clone(),
            sources,
            observer,
        };
        let outcome = progress
            .check(conn)
            .map_err(Failure::App)
            .and_then(|()| handler.run(conn, &job, &mut progress))
            .and_then(|value| {
                progress.check(conn)?;
                Ok(value)
            });
        let result = match outcome {
            Ok(value) => {
                begin(conn)?;
                Queue::new(conn, progress.sources)
                    .success(&job, &if truthy(&value) { value } else { empty() })?
            }
            Err(Failure::App(error))
                if is_removed(&error)
                    && super::first(conn, "SELECT id FROM freshness_jobs WHERE id=?", [id])?
                        .is_none() =>
            {
                return Err(error)
            }
            Err(Failure::App(error)) if error.code == "cancelled" => {
                begin(conn)?;
                Queue::new(conn, progress.sources).cancelled(&job)?
            }
            Err(failure) => {
                let error = wrap(conn, &job, failure)?;
                begin(conn)?;
                Queue::new(conn, progress.sources).error(&job, &error, clock)?
            }
        };
        commit(conn)?;
        Ok(result)
    })();
    match result {
        Err(error) if is_removed(&error) => {
            rollback(conn)?;
            if missing_ok {
                Ok(None)
            } else {
                Err(removed())
            }
        }
        result => result.map(Some),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_requested(
    conn: &mut Connection,
    profile: &str,
    ids: &[String],
    limit: i64,
    sources: &mut dyn Sources,
    handler: &mut dyn Handler,
    observer: &mut dyn FnMut(&Value) -> Result<()>,
    clock: &dyn Fn(i64) -> Result<String>,
) -> Result<Vec<Value>> {
    let ordered = Queue::new(conn, sources).dispatch_order(profile, ids)?;
    let mut completed = Vec::new();
    for requested in ordered {
        if completed.len() >= limit.max(0) as usize {
            break;
        }
        let id = or_text(field(&requested, "id"));
        let Some(job) = Queue::new(conn, sources).dispatchable(profile, &id)? else {
            continue;
        };
        if TERMINAL.contains(&or_text(field(&job, "status")).as_str()) {
            completed.push(job);
        } else if let Some(outcome) = run_job(conn, &id, sources, handler, observer, clock, true)? {
            completed.push(outcome);
        }
    }
    Ok(completed)
}

pub struct Options {
    pub run: bool,
    pub limit: Option<i64>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            run: true,
            limit: None,
        }
    }
}

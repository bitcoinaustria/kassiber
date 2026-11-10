use crate::publication::{
    row::{field, first, or_text, py_eq, rows, Result},
    wallet::Sources,
};
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::{Key, Options, Value};
use kassiber_store::rusqlite::{params, Connection};

pub mod execution;
pub mod graph;
pub mod native;
pub mod planning;
pub mod settings;
pub mod sources;
pub mod ui;

const JOB_TYPES: &[&str] = &[
    "onchain_wallet_history",
    "wallet_graph_references",
    "btcpay_wallet_source",
    "btcpay_provenance",
    "market_rate_coverage",
    "journal_refresh",
];
const TERMINAL: &[&str] = &["done", "error", "cancelled"];
const DUE: &str = "status IN ('queued','rate_limited')
    AND (run_after IS NULL OR run_after <= ?)
    AND (cooldown_until IS NULL OR cooldown_until <= ?)
    AND NOT EXISTS (SELECT 1 FROM freshness_source_states
      WHERE freshness_source_states.profile_id=freshness_jobs.profile_id
        AND freshness_source_states.source_key=freshness_jobs.source_key AND paused=1)";

fn empty() -> Value {
    dict::<&str>([])
}

pub fn control(
    conn: &Connection,
    sources: &mut dyn Sources,
    action: &str,
    args: &Value,
) -> Result<Value> {
    let context = kassiber_setup::context::snapshot(conn)?;
    let profile_id = or_text(field(&context, "profile_id"));
    let profile =
        first(conn, "SELECT * FROM profiles WHERE id=?", [&profile_id])?.ok_or_else(|| {
            AppError::new(
                "validation",
                format!("ui.freshness.{action} requires an active profile"),
            )
        })?;
    let key = if action == "cancel" {
        "job_id"
    } else {
        "source_key"
    };
    let value = get(args, key)
        .and_then(|v| match v {
            Value::Str(v) => v.as_str(),
            _ => None,
        })
        .map(kassiber_pytext::strip)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "validation",
                format!("ui.freshness.{action} requires args.{key}"),
            )
        })?;
    execution::begin(conn)?;
    let mut queue = Queue::new(conn, sources);
    let result = match action {
        "cancel" => queue.cancel(value)?,
        "pause" | "resume" => queue.pause(&profile_id, value, action == "pause")?,
        _ => {
            return Err(AppError::new(
                "validation",
                format!("Unsupported freshness control action '{action}'"),
            ))
        }
    };
    execution::commit(conn)?;
    let mut status = queue.snapshot(&profile_id)?;
    put(&mut status, "result", result);
    put(
        &mut status,
        "profile",
        dict([
            ("id", field(&profile, "id").clone()),
            ("label", field(&profile, "label").clone()),
        ]),
    );
    Ok(ui::payload(&status))
}
fn put(value: &mut Value, key: &str, item: Value) {
    if let Value::Dict(pairs) = value {
        if let Some((_, old)) = pairs.iter_mut().find(|(k, _)| k == &Key::from(key)) {
            *old = item;
        } else {
            pairs.push((Key::from(key), item));
        }
    }
}
fn remove(value: &mut Value, key: &str) {
    if let Value::Dict(pairs) = value {
        pairs.retain(|(k, _)| k != &Key::from(key));
    }
}
fn json(value: &Value) -> Result<String> {
    kassiber_pyjson::dumps(value, &Options::new().sort_keys(true).separators(",", ":"))
        .map_err(|_| AppError::new("internal_error", "Invalid freshness JSON"))
}
fn load(value: &Value) -> Value {
    if let Value::Str(raw) = value {
        if let Some(raw) = raw.as_str() {
            if let Ok(value) = kassiber_pytext::loads(raw) {
                if !matches!(value, Value::Null) {
                    return value;
                }
            }
        }
    }
    empty()
}
fn payload(mut row: Value) -> Value {
    for key in ["payload", "progress", "result", "error", "checkpoint"] {
        let raw = format!("{key}_json");
        if get(&row, &raw).is_some() {
            let parsed = load(field(&row, &raw));
            remove(&mut row, &raw);
            put(&mut row, key, parsed);
        }
    }
    for key in ["blocking_reports", "paused", "cancel_requested"] {
        if get(&row, key).is_some() {
            let boolean = truthy(field(&row, key));
            put(&mut row, key, Value::Bool(boolean));
        }
    }
    row
}
fn optional(value: &Value) -> Option<String> {
    (!matches!(value, Value::Null)).then(|| py_str(value))
}
fn integer(value: &Value) -> Result<i64> {
    match value {
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Int(n) => n
            .to_string()
            .parse()
            .map_err(|_| AppError::new("internal_error", "Freshness integer out of range")),
        Value::Str(text) => text
            .as_str()
            .and_then(|text| kassiber_pytext::parse_int(text).ok())
            .and_then(|n| n.to_string().parse().ok())
            .ok_or_else(|| AppError::new("validation", "Freshness integer required")),
        Value::Float(n)
            if n.is_finite() && *n >= i64::MIN as f64 && *n < 9_223_372_036_854_775_808.0 =>
        {
            Ok(n.trunc() as i64)
        }
        _ => Err(AppError::new("validation", "Freshness integer required")),
    }
}
fn key_text(key: &Key) -> String {
    if let Key::Str(text) = key {
        return text
            .code_points()
            .map(|point| char::from_u32(point).unwrap_or('\u{fffd}'))
            .collect();
    }
    py_str(&match key {
        Key::Null => Value::Null,
        Key::Bool(v) => Value::Bool(*v),
        Key::Int(v) => Value::Int(v.clone()),
        Key::Float(v) => Value::Float(*v),
        Key::Str(v) => Value::Str(v.clone()),
    })
}

pub fn workspace_profiles(conn: &Connection, workspace_id: &str) -> Result<(Value, Vec<Value>)> {
    let workspace = first(
        conn,
        "SELECT * FROM workspaces WHERE id=?",
        [kassiber_pytext::strip(workspace_id)],
    )?
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            format!("Book set '{workspace_id}' was not found"),
        )
    })?;
    let profiles = rows(
        conn,
        "SELECT * FROM profiles WHERE workspace_id=? ORDER BY created_at ASC,label ASC",
        [or_text(field(&workspace, "id"))],
    )?;
    Ok((workspace, profiles))
}

pub fn redact(value: &Value) -> Value {
    match value {
        Value::Dict(pairs) => {
            let mut result = Vec::new();
            let mut insert = |key: Key, value| {
                if let Some((_, old)) = result.iter_mut().find(|(other, _)| *other == key) {
                    *old = value;
                } else {
                    result.push((key, value));
                }
            };
            for (key, value) in pairs {
                let name = key_text(key);
                let lower = name.to_lowercase();
                if [
                    "checkpoint",
                    "checkpoint_json",
                    "freshness_checkpoint",
                    "_freshness_checkpoint",
                    "esplora_history_memberships",
                ]
                .contains(&lower.as_str())
                {
                    continue;
                }
                if ["backend_url", "url"].contains(&lower.as_str()) {
                    insert(
                        Key::from(format!("has_{lower}")),
                        Value::Bool(truthy(value)),
                    );
                } else if [
                    "token",
                    "auth_header",
                    "authorization",
                    "password",
                    "secret",
                    "api_key",
                    "cookie",
                    "descriptor",
                    "xpub",
                    "zpub",
                    "ypub",
                ]
                .contains(&lower.as_str())
                    || ["token", "password", "secret"]
                        .iter()
                        .any(|part| lower.contains(part))
                {
                    insert(
                        key.clone(),
                        if truthy(value) {
                            s("<redacted>")
                        } else {
                            Value::Null
                        },
                    );
                } else {
                    insert(key.clone(), redact(value));
                }
            }
            Value::Dict(result)
        }
        Value::List(items) => Value::List(items.iter().map(redact).collect()),
        _ => value.clone(),
    }
}

pub struct Queue<'a> {
    conn: &'a Connection,
    sources: &'a mut dyn Sources,
}

impl<'a> Queue<'a> {
    pub fn new(conn: &'a Connection, sources: &'a mut dyn Sources) -> Self {
        Self { conn, sources }
    }

    pub fn policy(&self, profile: &str) -> Result<Value> {
        let mut classes = dict([
            ("btcpay_provenance", Value::Bool(false)),
            ("btcpay_wallet", Value::Bool(false)),
            ("journals", Value::Bool(true)),
            ("market_rates", Value::Bool(false)),
            ("onchain_wallet", Value::Bool(false)),
        ]);
        let mut background = false;
        let mut report_read = false;
        let setting = first(
            self.conn,
            "SELECT value FROM settings WHERE key=?",
            [format!("freshness.policy.profile.{profile}")],
        )?;
        if let Some(setting) = setting {
            let configured = load(field(&setting, "value"));
            if matches!(configured, Value::Dict(_)) {
                background = truthy(field(&configured, "background_enabled"));
                report_read = truthy(field(&configured, "report_read_sync"));
                if let Value::Dict(pairs) = field(&configured, "source_classes") {
                    for (key, value) in pairs {
                        let name = key_text(key);
                        if get(&classes, &name).is_some() {
                            put(&mut classes, &name, Value::Bool(truthy(value)));
                        }
                    }
                }
            }
        } else if let Some(legacy) = first(
            self.conn,
            "SELECT value FROM settings WHERE key=?",
            [format!(
                "ai.auto_sync_before_report_reads.profile.{profile}"
            )],
        )? {
            if ["1", "true", "yes", "on"].contains(
                &kassiber_pytext::strip(&py_str(field(&legacy, "value")))
                    .to_lowercase()
                    .as_str(),
            ) {
                report_read = true;
                for key in ["onchain_wallet", "btcpay_wallet", "btcpay_provenance"] {
                    put(&mut classes, key, Value::Bool(true));
                }
            }
        }
        Ok(dict([
            ("background_enabled", Value::Bool(background)),
            ("source_classes", classes),
            ("report_read_sync", Value::Bool(report_read)),
        ]))
    }

    pub fn state(&self, profile: &str, key: &str) -> Result<Option<Value>> {
        Ok(first(
            self.conn,
            "SELECT * FROM freshness_source_states WHERE profile_id=? AND source_key=?",
            [profile, key],
        )?
        .map(payload))
    }

    pub fn set_policy(&self, profile: &str, input: &Value) -> Result<Value> {
        let mut current = self.policy(profile)?;
        let mut classes = field(&current, "source_classes").clone();
        if let Value::Dict(pairs) = field(input, "source_classes") {
            for (key, value) in pairs {
                let key = key_text(key);
                if get(&classes, &key).is_none() {
                    return Err(AppError::new(
                        "validation",
                        format!("Unsupported freshness source class '{key}'"),
                    )
                    .with_details(dict([("source_class", s(key))])));
                }
                put(&mut classes, &key, Value::Bool(truthy(value)));
            }
        }
        put(&mut current, "source_classes", classes);
        for key in ["background_enabled", "report_read_sync"] {
            if let Some(value) = get(input, key).filter(|v| !matches!(v, Value::Null)) {
                put(&mut current, key, Value::Bool(truthy(value)));
            }
        }
        self.conn.execute("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("freshness.policy.profile.{profile}"),json(&current)?]).map_err(crate::database_error)?;
        Ok(current)
    }

    pub fn reset_checkpoint(&mut self, profile: &str, key: &str, reason: &str) -> Result<bool> {
        let Some(previous) = self.state(profile, key)? else {
            return Ok(false);
        };
        let mut update = Self::state_input(&previous, "partially_stale");
        for key in ["blocking_reports", "paused"] {
            put(&mut update, key, field(&previous, key).clone());
        }
        put(&mut update, "stale_reason", s(reason));
        put(&mut update, "progress", empty());
        put(&mut update, "checkpoint", empty());
        self.upsert(&update)?;
        Ok(true)
    }

    pub fn delete_sources(&self, profile: &str, keys: &[String]) -> Result<()> {
        for table in ["freshness_jobs", "freshness_source_states"] {
            if keys.is_empty() {
                continue;
            }
            let placeholders = vec!["?"; keys.len()].join(",");
            self.conn
                .execute(
                    &format!(
                        "DELETE FROM {table} WHERE profile_id=? AND source_key IN ({placeholders})"
                    ),
                    kassiber_store::rusqlite::params_from_iter(
                        std::iter::once(profile).chain(keys.iter().map(String::as_str)),
                    ),
                )
                .map_err(crate::database_error)?;
        }
        Ok(())
    }

    pub fn upsert(&mut self, input: &Value) -> Result<Value> {
        let profile = or_text(field(input, "profile_id"));
        let key = or_text(field(input, "source_key"));
        let previous = self.state(&profile, &key)?.unwrap_or_else(empty);
        let inherited = |key| {
            get(input, key)
                .filter(|v| !matches!(v, Value::Null))
                .unwrap_or_else(|| field(&previous, key))
        };
        let paused = truthy(inherited("paused"));
        let retry = if truthy(inherited("retry_count")) {
            integer(inherited("retry_count"))?
        } else {
            0
        };
        let checkpoint = inherited("checkpoint");
        let checkpoint = if truthy(checkpoint) {
            checkpoint.clone()
        } else {
            empty()
        };
        let progress = inherited("progress").clone();
        let now = self.sources.now();
        let blocked = truthy(field(input, "blocking_reports"));
        let rate_until = optional(field(input, "rate_limited_until"));
        let cooling = rate_until
            .as_deref()
            .map(crate::time::instant)
            .transpose()?
            .flatten()
            .is_some_and(|time| {
                let unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();
                time > 62_135_596_800_000_000 + unix.as_micros() as i64
            });
        let status = if paused {
            "paused".to_owned()
        } else if cooling {
            "rate_limited".to_owned()
        } else if blocked {
            "blocking_reports".to_owned()
        } else {
            or_text(field(input, "status"))
        };
        let state = if truthy(field(input, "state")) {
            py_str(field(input, "state"))
        } else {
            status.clone()
        };
        self.conn.execute("INSERT INTO freshness_source_states(
            profile_id,source_key,source_type,source_label,status,state,stale_reason,blocking_reports,paused,rate_limited_until,
            cooldown_reason,retry_count,last_success_at,last_error_at,last_error_code,last_error_message,last_phase,progress_json,checkpoint_json,updated_at)
            VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(profile_id,source_key) DO UPDATE SET
            source_type=excluded.source_type,source_label=excluded.source_label,status=excluded.status,state=excluded.state,
            stale_reason=excluded.stale_reason,blocking_reports=excluded.blocking_reports,paused=excluded.paused,rate_limited_until=excluded.rate_limited_until,
            cooldown_reason=excluded.cooldown_reason,retry_count=excluded.retry_count,
            last_success_at=COALESCE(excluded.last_success_at,freshness_source_states.last_success_at),last_error_at=excluded.last_error_at,
            last_error_code=excluded.last_error_code,last_error_message=excluded.last_error_message,last_phase=excluded.last_phase,
            progress_json=excluded.progress_json,checkpoint_json=excluded.checkpoint_json,updated_at=excluded.updated_at",
            params![profile,key,or_text(field(input,"source_type")),or_text(field(input,"source_label")),status,state,
                optional(field(input,"stale_reason")),blocked,paused,rate_until,optional(field(input,"cooldown_reason")),retry,
                optional(field(input,"last_success_at")),optional(field(input,"last_error_at")),optional(field(input,"last_error_code")),
                optional(field(input,"last_error_message")),optional(field(input,"last_phase")),json(&redact(&if matches!(progress,Value::Null) { empty() } else { progress }))?,json(&checkpoint)?,now]
        ).map_err(crate::database_error)?;
        Ok(self.state(&profile, &key)?.unwrap_or_else(empty))
    }

    fn state_input(job: &Value, status: &str) -> Value {
        dict([
            ("profile_id", field(job, "profile_id").clone()),
            ("source_key", field(job, "source_key").clone()),
            ("source_type", field(job, "source_type").clone()),
            ("source_label", field(job, "source_label").clone()),
            ("status", s(status)),
        ])
    }

    pub fn job(&self, id: &str) -> Result<Value> {
        first(self.conn, "SELECT * FROM freshness_jobs WHERE id=?", [id])?
            .map(payload)
            .ok_or_else(|| AppError::new("not_found", "Freshness job was not found"))
    }

    fn cancelled(&mut self, job: &Value) -> Result<Value> {
        let now = self.sources.now();
        let phase = if truthy(field(job, "phase")) {
            py_str(field(job, "phase"))
        } else {
            "error".into()
        };
        let count = self.conn.execute("UPDATE freshness_jobs SET cancel_requested=1,status='cancelled',phase=?,finished_at=?,updated_at=? WHERE id=?",
            params![phase, now, now, or_text(field(job,"id"))]).map_err(crate::database_error)?;
        if count == 0 {
            return Err(AppError::new(
                "state_not_ready",
                "Freshness job disappeared",
            ));
        }
        let mut state = Self::state_input(job, "partially_stale");
        put(&mut state, "stale_reason", s("cancelled"));
        put(
            &mut state,
            "blocking_reports",
            Value::Bool(or_text(field(job, "job_type")) != "wallet_graph_references"),
        );
        put(&mut state, "last_phase", field(job, "phase").clone());
        self.upsert(&state)?;
        self.job(&or_text(field(job, "id")))
    }

    pub fn enqueue(&mut self, input: &Value) -> Result<Value> {
        let kind = or_text(field(input, "job_type"));
        if !JOB_TYPES.contains(&kind.as_str()) {
            return Err(AppError::new(
                "validation",
                format!("Unsupported freshness job type '{kind}'"),
            ));
        }
        let profile = or_text(field(input, "profile_id"));
        let key = or_text(field(input, "source_key"));
        let state = self.state(&profile, &key)?.unwrap_or_else(empty);
        if truthy(field(&state, "paused")) {
            let mut update = Self::state_input(input, "paused");
            put(&mut update, "paused", Value::Bool(true));
            put(&mut update, "stale_reason", s("paused"));
            return self.upsert(&update);
        }
        if let Some(existing) = first(self.conn,"SELECT * FROM freshness_jobs WHERE profile_id=? AND source_key=? AND job_type=? AND status IN ('queued','running','rate_limited') ORDER BY created_at ASC LIMIT 1",params![profile,key,kind])? {
            let mut existing = payload(existing);
            if get(input,"single_flight").is_none_or(truthy) || field(&existing,"status") == &s("running") {
                if field(&existing,"status") != &s("running") && kind == "journal_refresh" {
                    let current = field(&existing,"payload");
                    let incoming = field(input,"payload");
                    let mut merged = current.clone();
                    if truthy(field(incoming,"auto_pair")) { put(&mut merged,"auto_pair",Value::Bool(true)); }
                    if !(truthy(field(current,"skip_rebuild_when_current")) && truthy(field(incoming,"skip_rebuild_when_current"))) { remove(&mut merged,"skip_rebuild_when_current"); }
                    if !py_eq(&merged, current) {
                        self.conn.execute("UPDATE freshness_jobs SET payload_json=?,updated_at=? WHERE id=?",params![json(&redact(&merged))?,self.sources.now(),or_text(field(&existing,"id"))]).map_err(crate::database_error)?;
                        existing = self.job(&or_text(field(&existing,"id")))?;
                    }
                }
                return Ok(existing);
            }
            self.cancelled(&existing)?;
        }
        let id = self.sources.id();
        let now = self.sources.now();
        let priority = get(input, "priority")
            .map(integer)
            .transpose()?
            .unwrap_or(100);
        let data = get(input, "payload")
            .filter(|v| truthy(v))
            .cloned()
            .unwrap_or_else(empty);
        self.conn.execute("INSERT INTO freshness_jobs(id,profile_id,job_type,source_key,source_type,source_label,status,phase,priority,payload_json,progress_json,result_json,error_json,attempts,cancel_requested,run_after,cooldown_until,created_at,started_at,finished_at,updated_at)
            VALUES(?,?,?,?,?,?,'queued','discovery',?,?,'{}','{}','{}',0,0,?,NULL,?,NULL,NULL,?)",
            params![id,profile,kind,key,or_text(field(input,"source_type")),or_text(field(input,"source_label")),priority,json(&redact(&data))?,optional(field(input,"run_after")),now,now]).map_err(crate::database_error)?;
        let mut update = Self::state_input(input, "queued");
        put(&mut update, "stale_reason", s("queued"));
        put(&mut update, "progress", dict([("phase", s("discovery"))]));
        self.upsert(&update)?;
        self.job(&id)
    }

    pub fn progress(&mut self, job: &Value, progress: &Value) -> Result<()> {
        let redacted = redact(progress);
        let phase = [field(&redacted, "phase"), field(job, "phase")]
            .into_iter()
            .find(|v| truthy(v))
            .map_or_else(|| "backend_fetch".into(), py_str);
        let now = self.sources.now();
        let count = self
            .conn
            .execute(
                "UPDATE freshness_jobs SET phase=?,progress_json=?,updated_at=? WHERE id=?",
                params![phase, json(&redacted)?, now, or_text(field(job, "id"))],
            )
            .map_err(crate::database_error)?;
        if count == 0 {
            return Err(AppError::new(
                "state_not_ready",
                "Freshness job disappeared",
            ));
        }
        let mut state = Self::state_input(job, "syncing");
        put(&mut state, "stale_reason", s("syncing"));
        put(&mut state, "last_phase", s(phase));
        put(&mut state, "progress", redacted);
        self.upsert(&state)?;
        Ok(())
    }

    pub fn cancel(&mut self, id: &str) -> Result<Value> {
        let job = self.job(id)?;
        if TERMINAL.contains(&or_text(field(&job, "status")).as_str()) {
            return Ok(job);
        }
        if field(&job, "status") == &s("running") {
            let now = self.sources.now();
            self.conn
                .execute(
                    "UPDATE freshness_jobs SET cancel_requested=1,updated_at=? WHERE id=?",
                    params![now, id],
                )
                .map_err(crate::database_error)?;
            let phase = if truthy(field(&job, "phase")) {
                field(&job, "phase").clone()
            } else {
                s("backend_fetch")
            };
            self.progress(
                &job,
                &dict([
                    ("phase", phase),
                    ("cancellation_requested", Value::Bool(true)),
                ]),
            )?;
            return self.job(id);
        }
        self.cancelled(&job)
    }

    pub fn pause(&mut self, profile: &str, key: &str, paused: bool) -> Result<Value> {
        let previous = self
            .state(profile, key)?
            .ok_or_else(|| AppError::new("not_found", "Freshness source was not found"))?;
        let status = if paused {
            "paused"
        } else if truthy(field(&previous, "stale_reason")) {
            "partially_stale"
        } else {
            "fresh"
        };
        let mut update = Self::state_input(&previous, status);
        put(&mut update, "paused", Value::Bool(paused));
        put(
            &mut update,
            "stale_reason",
            field(&previous, "stale_reason").clone(),
        );
        put(
            &mut update,
            "blocking_reports",
            field(&previous, "blocking_reports").clone(),
        );
        self.upsert(&update)
    }

    pub fn recover(&mut self, profile: Option<&str>) -> Result<Vec<Value>> {
        let jobs = rows(self.conn,"SELECT * FROM freshness_jobs WHERE status='running' AND (? IS NULL OR profile_id=?) ORDER BY priority ASC,created_at ASC,id ASC",params![profile.filter(|p| !p.is_empty()),profile.filter(|p| !p.is_empty())])?;
        if jobs.is_empty() {
            return Ok(vec![]);
        }
        let now = self.sources.now();
        let mut recovered = Vec::new();
        for job in jobs.into_iter().map(payload) {
            if field(&job, "job_type") == &s("wallet_graph_references") {
                continue;
            }
            let error = dict([("code",s("worker_interrupted")),("message",s("Previous daemon worker stopped before this job finished; the job was requeued.")),("retryable",Value::Bool(true))]);
            self.conn.execute("UPDATE freshness_jobs SET status='queued',phase='discovery',cancel_requested=0,run_after=COALESCE(run_after,?),error_json=?,updated_at=? WHERE id=?",params![now,json(&error)?,now,or_text(field(&job,"id"))]).map_err(crate::database_error)?;
            let mut state = Self::state_input(&job, "blocking_reports");
            put(&mut state, "stale_reason", s("worker_interrupted"));
            put(&mut state, "blocking_reports", Value::Bool(true));
            put(&mut state, "last_phase", field(&job, "phase").clone());
            put(
                &mut state,
                "progress",
                dict([("phase", s("discovery")), ("recovered", Value::Bool(true))]),
            );
            self.upsert(&state)?;
            recovered.push(self.job(&or_text(field(&job, "id")))?);
        }
        Ok(recovered)
    }

    pub fn due(&mut self, profile: Option<&str>, limit: i64) -> Result<Vec<Value>> {
        if limit <= 0 {
            return Ok(vec![]);
        }
        let now = self.sources.now();
        let profile = profile.filter(|p| !p.is_empty());
        Ok(rows(self.conn,&format!("SELECT * FROM freshness_jobs WHERE {DUE} AND (? IS NULL OR profile_id=?) ORDER BY priority ASC,created_at ASC,id ASC LIMIT ?"),params![now,now,profile,profile,limit])?.into_iter().map(payload).collect())
    }

    pub fn dispatchable(&mut self, profile: &str, id: &str) -> Result<Option<Value>> {
        let now = self.sources.now();
        Ok(first(self.conn,&format!("SELECT * FROM freshness_jobs WHERE id=? AND profile_id=? AND (status IN ('done','error','cancelled') OR ({DUE}))"),params![id,profile,now,now])?.map(payload))
    }

    pub fn selected(&self, profile: &str, ids: &[String]) -> Result<Vec<Value>> {
        let mut result = Vec::new();
        let mut unique = std::collections::HashSet::new();
        let ids: Vec<_> = ids.iter().filter(|id| unique.insert(id.as_str())).collect();
        for batch in ids.chunks(500) {
            let placeholders = vec!["?"; batch.len()].join(",");
            result.extend(rows(self.conn,&format!("SELECT * FROM freshness_jobs WHERE profile_id=? AND id IN ({placeholders})"),
                kassiber_store::rusqlite::params_from_iter(std::iter::once(profile).chain(batch.iter().map(|v| v.as_str()))))?.into_iter().map(payload));
        }
        Ok(result)
    }

    pub fn requested_due(
        &mut self,
        profile: &str,
        requested: &[String],
        limit: i64,
    ) -> Result<Vec<Value>> {
        if requested.is_empty() {
            return Ok(vec![]);
        }
        let count: i64 = self.conn.query_row("SELECT count(*) FROM freshness_jobs WHERE profile_id=? AND status IN ('queued','running','rate_limited')",[profile],|r| r.get(0)).map_err(crate::database_error)?;
        let requested: std::collections::HashSet<_> =
            requested.iter().map(String::as_str).collect();
        Ok(self
            .due(Some(profile), count)?
            .into_iter()
            .filter(|v| requested.contains(or_text(field(v, "id")).as_str()))
            .take(limit.max(0) as usize)
            .collect())
    }

    pub fn dispatch_order(&self, profile: &str, requested: &[String]) -> Result<Vec<Value>> {
        let mut positions = std::collections::HashMap::new();
        for (index, id) in requested.iter().enumerate() {
            positions.insert(id, index);
        }
        let mut jobs = Vec::new();
        for job in self.selected(profile, requested)? {
            let id = or_text(field(&job, "id"));
            let terminal = TERMINAL.contains(&or_text(field(&job, "status")).as_str());
            let position = if terminal {
                *positions.get(&id).expect("selected job")
            } else {
                0
            };
            let key = (
                u8::from(!terminal),
                position,
                if terminal {
                    0
                } else {
                    integer(field(&job, "priority"))?
                },
                if terminal {
                    String::new()
                } else {
                    or_text(field(&job, "created_at"))
                },
                if terminal { String::new() } else { id },
            );
            jobs.push((key, job));
        }
        jobs.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(jobs.into_iter().map(|(_, job)| job).collect())
    }

    pub fn snapshot(&self, profile: &str) -> Result<Value> {
        let sources: Vec<_> = rows(self.conn,"SELECT * FROM freshness_source_states WHERE profile_id=? ORDER BY source_type ASC,source_label ASC,source_key ASC",[profile])?.into_iter().map(payload).collect();
        let jobs: Vec<_> = rows(self.conn,"SELECT * FROM freshness_jobs WHERE profile_id=? AND status IN ('queued','running','rate_limited') ORDER BY CASE status WHEN 'running' THEN 0 WHEN 'queued' THEN 1 WHEN 'rate_limited' THEN 2 ELSE 3 END,priority ASC,created_at DESC,id ASC",[profile])?.into_iter().map(payload).collect();
        let blocked = sources
            .iter()
            .filter(|v| truthy(field(v, "blocking_reports")))
            .count();
        let cooling = sources
            .iter()
            .filter(|v| {
                field(v, "status") == &s("rate_limited") || truthy(field(v, "rate_limited_until"))
            })
            .count();
        let summary = dict([
            ("sources", Value::from(sources.len())),
            ("active_jobs", Value::from(jobs.len())),
            ("blocking_reports", Value::from(blocked)),
            ("rate_limited", Value::from(cooling)),
        ]);
        Ok(dict([
            ("policy", self.policy(profile)?),
            ("sources", Value::List(sources)),
            ("jobs", Value::List(jobs)),
            ("summary", summary),
        ]))
    }
}

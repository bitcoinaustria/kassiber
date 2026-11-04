use super::{execution, put, sources, ui, workspace_profiles, Queue};
use crate::{
    publication::{
        row::{field, first, or_text, Result},
        wallet::{fallback, Sources, SystemSources},
    },
    sync::{Request as SyncRequest, Sync},
};
use kassiber_envelope::{dict, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use serde_json::Value as Json;
use std::sync::atomic::{AtomicBool, Ordering};

static EXECUTING: AtomicBool = AtomicBool::new(false);

struct Combined<'a, N> {
    native: N,
    journals: Option<&'a mut dyn execution::Handler>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_slot_excludes_competing_threads_and_releases_on_unwind() {
        let slot = Execution::acquire().unwrap();
        let error = std::thread::spawn(|| Execution::acquire().err().unwrap())
            .join()
            .unwrap();
        assert_eq!(error.code, "project_operation_in_progress");
        assert!(error.retryable);
        drop(slot);
        let outcome = std::panic::catch_unwind(|| {
            let _slot = Execution::acquire().unwrap();
            panic!("synthetic slot unwind");
        });
        assert!(outcome.is_err());
        let _slot = Execution::acquire().unwrap();
    }
}

impl<N: execution::Handler> execution::Handler for Combined<'_, N> {
    fn supports(&self, kind: &str) -> bool {
        self.native.supports(kind)
            || self
                .journals
                .as_ref()
                .is_some_and(|handler| handler.supports(kind))
    }
    fn prefetch(
        &mut self,
        conn: &Connection,
        jobs: &[Value],
        observer: &mut dyn FnMut(&Value) -> Result<()>,
    ) -> Result<()> {
        self.native.prefetch(conn, jobs, observer)
    }
    fn run(
        &mut self,
        conn: &mut Connection,
        job: &Value,
        progress: &mut execution::Progress<'_>,
    ) -> std::result::Result<Value, execution::Failure> {
        if self.native.supports(&or_text(field(job, "job_type"))) {
            self.native.run(conn, job, progress)
        } else if let Some(handler) = self.journals.as_mut() {
            handler.run(conn, job, progress)
        } else {
            Err(AppError::new("config_error", "Journal refresh is unavailable").into())
        }
    }
}

pub struct Execution {
    _private: (),
}
impl Execution {
    pub fn acquire() -> Result<Self> {
        EXECUTING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                AppError::new(
                    "project_operation_in_progress",
                    "Another refresh is still running",
                )
                .with_hint("Wait for the current refresh to finish, then retry.")
                .retryable(true)
            })?;
        Ok(Self { _private: () })
    }
}
impl Drop for Execution {
    fn drop(&mut self) {
        EXECUTING.store(false, Ordering::Release);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    ActiveProfile,
    Workspace(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub scope: Scope,
    pub wallet: Option<String>,
    pub rates: Option<bool>,
    pub journals: Option<bool>,
    pub force_full: bool,
    pub auto_pair: bool,
    pub wallets_sync: bool,
}

#[derive(PartialEq)]
pub(super) struct Captured {
    pub(super) context: Value,
    workspace: Option<Value>,
    pub(super) profiles: Vec<Value>,
    pub(super) wallets: Vec<Vec<Value>>,
    policies: Vec<Value>,
    pub(super) specs: Vec<Vec<Value>>,
}

fn capture(conn: &Connection, request: &Request) -> Result<Captured> {
    let context = kassiber_setup::context::snapshot(conn)?;
    let (workspace, profiles) = match &request.scope {
        Scope::ActiveProfile => {
            let profile = first(
                conn,
                "SELECT * FROM profiles WHERE id=?",
                [or_text(field(&context, "profile_id"))],
            )?;
            (None, profile.into_iter().collect::<Vec<_>>())
        }
        Scope::Workspace(id) => {
            if request.wallet.is_some() || request.force_full {
                return Err(AppError::new(
                    "validation",
                    "Workspace freshness does not accept wallet or force_full",
                ));
            }
            if kassiber_pytext::strip(id).is_empty() {
                return Err(AppError::new(
                    "validation",
                    "ui.workspace.freshness.run requires args.workspace_id",
                ));
            }
            let (workspace, profiles) = workspace_profiles(conn, id)?;
            (Some(workspace), profiles)
        }
    };
    let mut captured = Captured {
        context,
        workspace,
        profiles,
        wallets: vec![],
        policies: vec![],
        specs: vec![],
    };
    for profile in &captured.profiles {
        let id = or_text(field(profile, "id"));
        let policy = Queue::new(conn, &mut SystemSources::default()).policy(&id)?;
        let included = |requested: Option<bool>, class: &str| {
            requested != Some(false) && truthy(field(field(&policy, "source_classes"), class))
        };
        let specs = sources::build(
            conn,
            &id,
            &sources::Options {
                wallet: request.wallet.clone(),
                rates: included(request.rates, "market_rates"),
                journals: !request.wallets_sync && included(request.journals, "journals"),
                auto_pair: request.auto_pair,
                force_full: request.force_full,
            },
        )?;
        captured
            .wallets
            .push(sources::wallets(conn, &id, request.wallet.as_deref())?);
        captured.policies.push(policy);
        captured.specs.push(specs);
    }
    Ok(captured)
}

pub struct Plan {
    _execution: Execution,
    request: Request,
    pub(super) captured: Captured,
    pub(super) identity: std::sync::Arc<()>,
    actions: Vec<Sync>,
    connection: usize,
    database: String,
    database_identity: Option<Value>,
    wallet_sync_compatible: bool,
}

fn identity(conn: &Connection) -> Result<Option<Value>> {
    first(
        conn,
        "SELECT value FROM settings WHERE key='database_instance_id'",
        [],
    )
}

/// A report-read refresh with no source to enqueue and no due job answers
/// Python's empty sync payload without admitting a plan, so books whose
/// wallets the native refresh cannot run still read normally while their
/// source classes stay off.
pub fn report_read_idle(
    conn: &Connection,
    profile: &str,
    policy: &Value,
    force: bool,
    sources: &mut dyn Sources,
) -> Result<Option<Value>> {
    let rates = truthy(field(field(policy, "source_classes"), "market_rates"));
    let specs = super::sources::build(
        conn,
        profile,
        &super::sources::Options {
            wallet: None,
            rates,
            journals: false,
            auto_pair: false,
            force_full: false,
        },
    )?;
    if !super::sources::filter_by_policy(specs, policy, force).is_empty()
        || !Queue::new(conn, sources).due(Some(profile), 1)?.is_empty()
    {
        return Ok(None);
    }
    let freshness = ui::payload(&Queue::new(conn, sources).snapshot(profile)?);
    let mut payload = ui::payload(&dict([
        ("results", Value::List(vec![])),
        ("enqueued", Value::List(vec![])),
        ("completed", Value::List(vec![])),
        ("freshness", freshness),
    ]));
    put(&mut payload, "ok", Value::Bool(true));
    Ok(Some(payload))
}

/// One book's local journal step: the jobs it queued and ran, and its
/// summary for the reply.
struct JournalStep {
    enqueued: Vec<Value>,
    completed: Vec<Value>,
    summary: Value,
}

/// `run_local_journal_refresh` without its execution slot: queue and run
/// only the journal source of one book, honoring the policy's journals
/// source class.
#[allow(clippy::too_many_arguments)]
fn journal_step(
    conn: &mut Connection,
    profile_id: &str,
    auto_pair: bool,
    sources: &mut dyn Sources,
    handler: &mut dyn execution::Handler,
    observer: &mut dyn FnMut(&Value) -> Result<()>,
    clock: &dyn Fn(i64) -> Result<String>,
) -> Result<JournalStep> {
    let policy = Queue::new(conn, sources).policy(profile_id)?;
    if !truthy(field(field(&policy, "source_classes"), "journals")) {
        let mut summary = ui::journal_step_summary(&[], &[]);
        put(&mut summary, "status", s("disabled"));
        return Ok(JournalStep {
            enqueued: vec![],
            completed: vec![],
            summary,
        });
    }
    let mut payload = dict([("skip_rebuild_when_current", Value::Bool(true))]);
    if auto_pair {
        put(&mut payload, "auto_pair", Value::Bool(true));
    }
    let mut input = sources::journal(profile_id, payload);
    put(&mut input, "profile_id", s(profile_id));
    execution::begin(conn)?;
    let job = Queue::new(conn, sources).enqueue(&input)?;
    execution::commit(conn)?;
    let local = if truthy(field(&job, "job_type")) {
        vec![job]
    } else {
        vec![]
    };
    let ids = local
        .iter()
        .map(|job| or_text(field(job, "id")))
        .collect::<Vec<_>>();
    let selected = Queue::new(conn, sources).requested_due(profile_id, &ids, 1)?;
    let done = execution::run_requested(
        conn,
        profile_id,
        &selected
            .iter()
            .map(|job| or_text(field(job, "id")))
            .collect::<Vec<_>>(),
        1,
        sources,
        handler,
        &mut |event| observer(&ui::payload(event)),
        clock,
    )?;
    Ok(JournalStep {
        summary: ui::journal_step_summary(&local, &done),
        enqueued: local,
        completed: done,
    })
}

/// `run_local_journal_refresh(auto_pair=True, skip_rebuild_when_current=True)`
/// after a user-triggered import: the journal step summary for the reply. A
/// refresh already running elsewhere refuses with
/// `project_operation_in_progress`, which callers report as deferred.
pub fn local_journal_refresh(
    conn: &mut Connection,
    profile_id: &str,
    sources: &mut dyn Sources,
    handler: &mut dyn execution::Handler,
) -> Result<Value> {
    let _execution = Execution::acquire()?;
    let step = journal_step(
        conn,
        profile_id,
        true,
        sources,
        handler,
        &mut |_| Ok(()),
        &execution::retry_time,
    )?;
    Ok(ui::payload(&step.summary))
}

impl Plan {
    pub fn new(
        conn: &Connection,
        request: &Request,
        version: &str,
        mut backend: impl FnMut(&Connection, &str) -> Result<Json>,
    ) -> Result<Self> {
        let execution = Execution::acquire()?;
        if !conn.is_autocommit() {
            return Err(AppError::new(
                "observer_prepare_in_transaction",
                "Freshness admission must start outside a transaction",
            ));
        }
        if request
            .wallet
            .as_deref()
            .is_some_and(|v| kassiber_pytext::strip(v).is_empty())
        {
            return Err(AppError::new(
                "validation",
                "Freshness wallet must be a non-empty string",
            ));
        }
        let captured = capture(conn, request)?;
        let mut actions = Vec::new();
        for ((profile, specs), wallets) in captured
            .profiles
            .iter()
            .zip(&captured.specs)
            .zip(&captured.wallets)
        {
            for spec in specs {
                if ![s("onchain_wallet_history"), s("journal_refresh")]
                    .contains(field(spec, "job_type"))
                {
                    return Err(fallback("freshness_source"));
                }
            }
            let id = or_text(field(profile, "id"));
            if request.wallet.is_some() && wallets.len() > 1 {
                return Err(fallback("freshness_ambiguous_wallet"));
            }
            for wallet in wallets {
                let config = sources::wallet_config(wallet)?;
                if request.wallet.is_none() && crate::config::boolean(config.get("deprecated"))? {
                    continue;
                }
                if matches!(
                    or_text(field(wallet, "kind")).as_str(),
                    "coreln" | "lnd" | "nwc" | "phoenix" | "silent-payment"
                ) {
                    return Err(fallback("wallet_route"));
                }
            }
            actions.push(Sync::new(
                conn,
                &SyncRequest {
                    profile_id: &id,
                    wallet: request.wallet.as_deref(),
                    all: request.wallet.is_none(),
                    force_full: request.force_full,
                    version,
                },
                &mut backend,
            )?);
        }
        let mut wallet_sync_compatible = !captured.profiles.is_empty();
        for (profile, specs) in captured.profiles.iter().zip(&captured.specs) {
            wallet_sync_compatible &= !specs.is_empty();
            for spec in specs {
                if first(conn,
                    "SELECT paused FROM freshness_source_states WHERE profile_id=? AND source_key=?",
                    [or_text(field(profile, "id")), or_text(field(spec, "source_key"))])?
                    .is_some_and(|state| truthy(field(&state, "paused"))) {
                    wallet_sync_compatible = false;
                }
            }
        }
        Ok(Self {
            wallet_sync_compatible,
            _execution: execution,
            request: request.clone(),
            captured,
            identity: std::sync::Arc::new(()),
            actions,
            connection: std::ptr::from_ref(conn).addr(),
            database: conn.path().unwrap_or("").into(),
            database_identity: identity(conn)?,
        })
    }

    pub fn supports_wallets_sync(&self) -> bool {
        self.wallet_sync_compatible
    }

    pub fn validate(
        &self,
        conn: &Connection,
        mut backend: impl FnMut(&Connection, &str) -> Result<Json>,
    ) -> Result<()> {
        if !conn.is_autocommit()
            || std::ptr::from_ref(conn).addr() != self.connection
            || conn.path().unwrap_or("") != self.database
            || identity(conn)? != self.database_identity
            || capture(conn, &self.request)? != self.captured
        {
            return Err(AppError::new(
                "stale_context",
                "Freshness selection changed",
            ));
        }
        for action in &self.actions {
            action.validate(conn, &mut backend)?;
        }
        Ok(())
    }

    pub fn specifications(&self) -> Value {
        let label = |v: &Value| {
            dict([
                ("id", field(v, "id").clone()),
                ("label", field(v, "label").clone()),
            ])
        };
        dict([
            (
                "workspace",
                self.captured
                    .workspace
                    .as_ref()
                    .map(label)
                    .unwrap_or(Value::Null),
            ),
            (
                "books",
                Value::List(
                    self.captured
                        .profiles
                        .iter()
                        .zip(&self.captured.specs)
                        .map(|(profile, specs)| {
                            dict([
                                ("profile", label(profile)),
                                ("specs", Value::List(specs.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    #[allow(clippy::too_many_arguments)]
    pub fn execute(
        self,
        conn: &mut Connection,
        options: &execution::Options,
        sources: &mut dyn Sources,
        handler: &mut dyn execution::Handler,
        observer: &mut dyn FnMut(&Value) -> Result<()>,
        clock: &dyn Fn(i64) -> Result<String>,
        backend: impl FnMut(&Connection, &str) -> Result<Json>,
    ) -> Result<Value> {
        self.validate(conn, backend)?;
        if self.captured.profiles.is_empty() && self.captured.workspace.is_none() {
            // Python spreads the snapshot after these keys, so they lead.
            let mut result = dict([
                ("profile", Value::Null),
                ("enqueued", Value::List(vec![])),
                ("completed", Value::List(vec![])),
            ]);
            if let (Value::Dict(out), Value::Dict(snapshot)) =
                (&mut result, Queue::new(conn, sources).snapshot("")?)
            {
                for (key, value) in snapshot {
                    match out.iter_mut().find(|(k, _)| *k == key) {
                        Some((_, old)) => *old = value,
                        None => out.push((key, value)),
                    }
                }
            }
            return Ok(ui::payload(&result));
        }
        let label = |v: &Value| {
            dict([
                ("id", field(v, "id").clone()),
                ("label", field(v, "label").clone()),
            ])
        };
        let workspace = self.captured.workspace.as_ref().map(label);
        let mut books = Vec::new();
        let mut totals = [0usize; 6];
        for (profile, specs) in self.captured.profiles.iter().zip(&self.captured.specs) {
            let id = or_text(field(profile, "id"));
            execution::begin(conn)?;
            let recovered = Queue::new(conn, sources).recover(Some(&id))?;
            execution::commit(conn)?;
            execution::begin(conn)?;
            let mut enqueued = Vec::new();
            for spec in specs {
                let mut input = spec.clone();
                put(&mut input, "profile_id", s(id.as_str()));
                let job = Queue::new(conn, sources).enqueue(&input)?;
                if truthy(field(&job, "job_type")) {
                    enqueued.push(job);
                }
            }
            execution::commit(conn)?;
            let limit = options
                .limit
                .filter(|limit| *limit != 0)
                .unwrap_or(enqueued.len().max(1) as i64);
            let run_total = limit.min(enqueued.len().max(1) as i64).max(1) as usize;
            let mut seen = Vec::new();
            let mut book_progress = |event: &Value| {
                let mut event = event.clone();
                if let Some(workspace) = &workspace {
                    put(&mut event, "workspace", workspace.clone());
                    put(&mut event, "profile", label(profile));
                } else {
                    let job_id = or_text(field(&event, "job_id"));
                    if !job_id.is_empty() && !seen.contains(&job_id) {
                        seen.push(job_id.clone());
                    }
                    if let Some(index) = seen.iter().position(|id| id == &job_id) {
                        put(&mut event, "job_index", Value::from(index + 1));
                    }
                    put(
                        &mut event,
                        "job_total",
                        Value::from(run_total.max(seen.len())),
                    );
                }
                observer(&ui::payload(&event))
            };
            let ids = enqueued
                .iter()
                .map(|job| or_text(field(job, "id")))
                .collect::<Vec<_>>();
            let mut completed = if options.run {
                let selected = if workspace.is_none() {
                    Queue::new(conn, sources).requested_due(&id, &ids, limit)?
                } else {
                    enqueued.clone()
                };
                if workspace.is_none() {
                    handler.prefetch(conn, &selected, &mut book_progress)?;
                }
                execution::run_requested(
                    conn,
                    &id,
                    &selected
                        .iter()
                        .map(|job| or_text(field(job, "id")))
                        .collect::<Vec<_>>(),
                    limit,
                    sources,
                    handler,
                    &mut book_progress,
                    clock,
                )?
            } else {
                vec![]
            };
            let mut journals = None;
            if self.request.wallets_sync && self.request.journals != Some(false) {
                let step = journal_step(
                    conn,
                    &id,
                    self.request.auto_pair,
                    sources,
                    handler,
                    observer,
                    clock,
                )?;
                journals = Some(step.summary);
                enqueued.extend(step.enqueued);
                completed.extend(step.completed);
            } else if workspace.is_none() {
                if specs
                    .iter()
                    .any(|spec| field(spec, "job_type") == &s("journal_refresh"))
                {
                    journals = Some(ui::journal_step_summary(&enqueued, &completed));
                } else if self.request.journals == Some(true) {
                    let mut summary = ui::journal_step_summary(&[], &[]);
                    put(&mut summary, "status", s("disabled"));
                    journals = Some(summary);
                }
            }
            let mut result = Queue::new(conn, sources).snapshot(&id)?;
            if let Some(journals) = journals {
                put(&mut result, "journals", journals);
            }
            let blocked =
                super::integer(field(field(&result, "summary"), "blocking_reports"))? as usize;
            let cooling =
                super::integer(field(field(&result, "summary"), "rate_limited"))? as usize;
            let errors = completed
                .iter()
                .filter(|job| {
                    ![s("done"), s("cancelled"), s("rate_limited")].contains(field(job, "status"))
                })
                .count();
            for (total, value) in totals.iter_mut().zip([
                enqueued.len(),
                completed.len(),
                errors,
                cooling,
                usize::from(blocked > 0),
                usize::from(blocked == 0),
            ]) {
                *total += value;
            }
            for (key, value) in [
                ("profile", label(profile)),
                ("results", Value::List(ui::results(&completed))),
                ("recovered", Value::List(recovered)),
                ("enqueued", Value::List(enqueued)),
                ("completed", Value::List(completed)),
            ] {
                put(&mut result, key, value);
            }
            if workspace.is_some() {
                put(
                    &mut result,
                    "attention",
                    dict([
                        ("blockedReports", Value::Bool(blocked > 0)),
                        ("rateLimited", Value::Bool(cooling > 0)),
                        ("errors", Value::from(errors)),
                    ]),
                );
            } else {
                put(
                    &mut result,
                    "force_full",
                    Value::Bool(self.request.force_full),
                );
            }
            books.push(result);
        }
        let result = if let Some(workspace) = workspace {
            let mut summary = dict([
                ("books", Value::from(books.len())),
                ("ok", Value::Bool(totals[2] == 0 && totals[4] == 0)),
                ("reports_blocked", Value::from(totals[4])),
            ]);
            for (key, total) in [
                "enqueued",
                "completed",
                "errors",
                "rate_limited",
                "blocked_books",
                "synced_books",
            ]
            .into_iter()
            .zip(totals)
            {
                put(&mut summary, key, Value::from(total));
            }
            dict([
                ("workspace", workspace),
                ("books", Value::List(books)),
                ("summary", summary),
            ])
        } else {
            books.remove(0)
        };
        Ok(ui::payload(&result))
    }

    /// `_run_auto_sync_wallets` after its rate limit: enqueue the book's
    /// wallet sources (filtered by source class unless `force`d), run the due
    /// jobs (or exactly the requested ones when forced) through the native
    /// handler, and answer Python's sync payload. Sources the native handler
    /// cannot run refuse with `not_implemented` before any of them starts.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_report_read(
        self,
        conn: &mut Connection,
        force: bool,
        sources: &mut dyn Sources,
        gate: &kassiber_net::Gate,
        consent: &super::native::Consent,
        cancel: &crate::scan::Cancellation,
        deadline: std::time::Instant,
        start_time: u64,
        version: &str,
        mut backend: impl FnMut(&Connection, &str) -> Result<Json>,
    ) -> Result<Value> {
        let unsupported = |kind: &str| {
            AppError::new(
                "not_implemented",
                "Report-read refresh of this source is not implemented in the Rust daemon yet",
            )
            .with_details(dict([("job_type", s(kind))]))
        };
        self.validate(conn, &mut backend)?;
        let (Some(profile), Some(policy), Some(specs)) = (
            self.captured.profiles.first(),
            self.captured.policies.first(),
            self.captured.specs.first(),
        ) else {
            return Err(AppError::new(
                "not_found",
                "Freshness profile was not found",
            ));
        };
        let profile = or_text(field(profile, "id"));
        let specs = sources::filter_by_policy(specs.clone(), policy, force);
        let proof = super::native::Consent::consume(Some(consent), &self)?;
        let resolver = std::cell::RefCell::new(&mut backend);
        let mut native = super::native::Native::new(
            &self,
            conn,
            proof,
            gate,
            cancel,
            deadline,
            start_time,
            version,
            |conn, name| resolver.borrow_mut()(conn, name),
        )?;
        execution::begin(conn)?;
        let mut enqueued = Vec::new();
        for spec in &specs {
            let mut input = spec.clone();
            put(&mut input, "profile_id", s(profile.as_str()));
            let job = Queue::new(conn, sources).enqueue(&input)?;
            if truthy(field(&job, "job_type")) {
                enqueued.push(job);
            }
        }
        execution::commit(conn)?;
        let limit = enqueued.len().max(1);
        let handler: &mut dyn execution::Handler = &mut native;
        let completed = if force {
            let ids: Vec<String> = enqueued.iter().map(|j| or_text(field(j, "id"))).collect();
            for job in Queue::new(conn, sources).selected(&profile, &ids)? {
                let kind = or_text(field(&job, "job_type"));
                if !handler.supports(&kind) {
                    return Err(unsupported(&kind));
                }
            }
            execution::run_requested(
                conn,
                &profile,
                &ids,
                limit as i64,
                sources,
                handler,
                &mut |_| Ok(()),
                &execution::retry_time,
            )?
        } else {
            let mut completed = Vec::new();
            for _ in 0..limit {
                let Some(job) = Queue::new(conn, sources).due(Some(&profile), 1)?.pop() else {
                    break;
                };
                let kind = or_text(field(&job, "job_type"));
                if !handler.supports(&kind) {
                    return Err(unsupported(&kind));
                }
                if let Some(done) = execution::run_job(
                    conn,
                    &or_text(field(&job, "id")),
                    sources,
                    handler,
                    &mut |_| Ok(()),
                    &execution::retry_time,
                    false,
                )? {
                    completed.push(done);
                }
            }
            completed
        };
        let freshness = ui::payload(&Queue::new(conn, sources).snapshot(&profile)?);
        let mut payload = ui::payload(&dict([
            ("results", Value::List(ui::results(&completed))),
            ("enqueued", Value::List(enqueued)),
            ("completed", Value::List(completed)),
            ("freshness", freshness),
        ]));
        // The UI projection already defaults `ok` from the error rows.
        let ok = field(&payload, "ok") != &Value::Bool(false);
        put(&mut payload, "ok", Value::Bool(ok));
        Ok(payload)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn execute_native(
        self,
        conn: &mut Connection,
        options: &execution::Options,
        sources: &mut dyn Sources,
        observer: &mut dyn FnMut(&Value) -> Result<()>,
        gate: &kassiber_net::Gate,
        consent: Option<&super::native::Consent>,
        cancel: &crate::scan::Cancellation,
        deadline: std::time::Instant,
        start_time: u64,
        version: &str,
        mut backend: impl FnMut(&Connection, &str) -> Result<Json>,
        journals: Option<&mut dyn execution::Handler>,
    ) -> Result<Value> {
        cancel.check(deadline)?;
        self.validate(conn, &mut backend)?;
        let needs_journals = self
            .captured
            .specs
            .iter()
            .flatten()
            .any(|spec| field(spec, "job_type") == &s("journal_refresh"))
            || self.request.wallets_sync
                && self.request.journals != Some(false)
                && self
                    .captured
                    .policies
                    .iter()
                    .any(|policy| truthy(field(field(policy, "source_classes"), "journals")));
        if needs_journals
            && !journals
                .as_ref()
                .is_some_and(|handler| handler.supports("journal_refresh"))
        {
            return Err(fallback("freshness_journal_refresh"));
        }
        let proof = super::native::Consent::consume(consent, &self)?;
        let resolver = std::cell::RefCell::new(&mut backend);
        let native = super::native::Native::new(
            &self,
            conn,
            proof,
            gate,
            cancel,
            deadline,
            start_time,
            version,
            |conn, name| resolver.borrow_mut()(conn, name),
        )?;
        let mut handler = Combined { native, journals };
        let result = self.execute(
            conn,
            options,
            sources,
            &mut handler,
            observer,
            &execution::retry_time,
            |conn, name| resolver.borrow_mut()(conn, name),
        );
        proof.finish_graph_grants(conn, result.is_ok(), |conn, name| {
            resolver.borrow_mut()(conn, name)
        });
        result
    }
}

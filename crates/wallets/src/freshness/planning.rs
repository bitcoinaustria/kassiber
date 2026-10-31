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

struct Execution;
impl Execution {
    fn acquire() -> Result<Self> {
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
        Ok(Self)
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
                journals: included(request.journals, "journals"),
                auto_pair: false,
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
}

fn identity(conn: &Connection) -> Result<Option<Value>> {
    first(
        conn,
        "SELECT value FROM settings WHERE key='database_instance_id'",
        [],
    )
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
        if request.journals != Some(false) {
            return Err(fallback("freshness_journal_refresh"));
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
                if field(spec, "job_type") != &s("onchain_wallet_history") {
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
        Ok(Self {
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
            let mut result = Queue::new(conn, sources).snapshot("")?;
            for (key, value) in [
                ("profile", Value::Null),
                ("enqueued", Value::List(vec![])),
                ("completed", Value::List(vec![])),
            ] {
                put(&mut result, key, value);
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
            let completed = if options.run {
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
            let mut result = Queue::new(conn, sources).snapshot(&id)?;
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
    ) -> Result<Value> {
        cancel.check(deadline)?;
        self.validate(conn, &mut backend)?;
        let proof = super::native::Consent::consume(consent, &self)?;
        let resolver = std::cell::RefCell::new(&mut backend);
        let mut handler = super::native::Native::new(
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

use super::{
    execution::{self, Failure, Handler, Progress},
    planning::Plan,
    put, sources,
};
use crate::{
    fetch::{Fetch, FetchConsent},
    publication::{
        row::{field, first, or_text, Result},
        wallet,
    },
    scan::{Cancellation, Checked},
};
use kassiber_envelope::{dict, s, truthy, AppError};
use kassiber_net::Gate;
use kassiber_pyjson::{Options, Value};
use kassiber_store::rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as Json;
mod jobs;

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

pub struct Consent {
    identity: Arc<()>,
    started: AtomicBool,
    revoked: Arc<AtomicBool>,
    grants: Mutex<Vec<Arc<FetchConsent>>>,
    graph_grants: Mutex<Vec<super::graph::Grant>>,
    pub(super) report_read: bool,
}

impl Consent {
    pub fn explicit_trigger(plan: &Plan) -> Self {
        Self {
            identity: plan.identity.clone(),
            started: AtomicBool::new(false),
            revoked: Arc::new(AtomicBool::new(false)),
            grants: Mutex::new(vec![]),
            graph_grants: Mutex::new(vec![]),
            report_read: false,
        }
    }
    /// A report read under a book whose freshness policy enables report-read
    /// refresh. Each fetch re-checks that policy and its source class, and no
    /// graph follow-up is collected.
    pub fn report_read(plan: &Plan) -> Self {
        Self {
            report_read: true,
            ..Self::explicit_trigger(plan)
        }
    }
    /// `require_network_permission`: an automatic trigger needs its feature
    /// and the source class enabled when the request is made.
    fn permitted(&self, conn: &Connection, profile: &str) -> Result<()> {
        if !self.report_read {
            return Ok(());
        }
        let policy =
            super::Queue::new(conn, &mut wallet::SystemSources::default()).policy(profile)?;
        if !truthy(field(&policy, "report_read_sync"))
            || !truthy(field(field(&policy, "source_classes"), "onchain_wallet"))
        {
            return Err(AppError::new(
                "cancelled",
                "Freshness network permission was disabled",
            ));
        }
        Ok(())
    }
    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::SeqCst);
        if let Ok(grants) = self.grants.lock() {
            for grant in grants.iter() {
                grant.revoke();
            }
        }
    }
    pub fn check(&self) -> Result<()> {
        if self.revoked.load(Ordering::SeqCst) {
            return Err(AppError::new(
                "interaction_required",
                "Freshness sync consent was revoked",
            ));
        }
        Ok(())
    }
    pub fn take_graph_grants(&self) -> Vec<super::graph::Grant> {
        self.graph_grants
            .lock()
            .map(|mut grants| std::mem::take(&mut *grants))
            .unwrap_or_default()
    }

    pub(super) fn finish_graph_grants(
        &self,
        conn: &Connection,
        succeeded: bool,
        mut backend: impl FnMut(&Connection, &str) -> Result<Json>,
    ) {
        if let Ok(mut grants) = self.graph_grants.lock() {
            grants.retain(|grant| succeeded && grant.validate(conn, &mut backend).is_ok());
        }
    }

    fn collect_graph(&self, grant: super::graph::Grant) {
        if self.report_read || self.check().is_err() {
            return;
        }
        if let Ok(mut grants) = self.graph_grants.lock() {
            grants.retain(|old| !old.same_wallet(&grant));
            if grants.len() < super::graph::MAX_PENDING {
                grants.push(grant);
            }
        }
    }
    pub(super) fn consume<'a>(consent: Option<&'a Self>, plan: &Plan) -> Result<&'a Self> {
        let proof = consent
            .filter(|proof| Arc::ptr_eq(&proof.identity, &plan.identity))
            .ok_or_else(|| {
                AppError::new(
                    "interaction_required",
                    "Freshness sync requires consent for this exact action",
                )
            })?;
        proof.check()?;
        if proof.started.swap(true, Ordering::SeqCst) {
            return Err(AppError::new(
                "interaction_required",
                "Freshness sync consent was already used",
            ));
        }
        Ok(proof)
    }
    fn grant(&self, action: &Fetch) -> Result<Arc<FetchConsent>> {
        self.check()?;
        let grant = Arc::new(if self.report_read {
            FetchConsent::narrowly_scoped(action)
        } else {
            FetchConsent::explicit_trigger(action)
        });
        self.grants
            .lock()
            .map_err(|_| {
                AppError::new(
                    "interaction_required",
                    "Freshness sync consent is unavailable",
                )
            })?
            .push(grant.clone());
        self.check()?;
        Ok(grant)
    }
}

#[derive(Clone)]
struct Source {
    profile: String,
    key: String,
    wallet: Value,
    backend_name: String,
    backend: Result<Json>,
}

struct Prepared {
    id: String,
    selected: Source,
    action: Fetch,
    grant: Arc<FetchConsent>,
    fetch: Json,
    graph: Option<super::graph::Grant>,
}

/// A job's discovery: its source and the inert fetch, before any request.
struct Discovered {
    id: String,
    selected: Source,
    action: Fetch,
    grant: Arc<FetchConsent>,
    graph: Option<super::graph::Grant>,
}

pub(super) struct Native<'a, B> {
    sources: Vec<Source>,
    backends: &'a kassiber_setup::backends::Config,
    book_id: String,
    context: Value,
    proof: &'a Consent,
    gate: &'a Gate,
    cancel: &'a Cancellation,
    deadline: Instant,
    start_time: u64,
    version: String,
    backend: B,
    prefetched: std::collections::BTreeMap<String, Result<Prepared>>,
}

impl<'a, B: FnMut(&Connection, &str) -> Result<Json>> Native<'a, B> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        plan: &Plan,
        conn: &Connection,
        proof: &'a Consent,
        gate: &'a Gate,
        cancel: &'a Cancellation,
        deadline: Instant,
        start_time: u64,
        version: &str,
        backends: &'a kassiber_setup::backends::Config,
        mut backend: B,
    ) -> Result<Self> {
        let book_id: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key='database_instance_id'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(crate::database_error)?;
        let mut selected = Vec::new();
        for ((profile, specs), wallets) in plan
            .captured
            .profiles
            .iter()
            .zip(&plan.captured.specs)
            .zip(&plan.captured.wallets)
        {
            for spec in specs {
                if field(spec, "job_type") != &s("onchain_wallet_history") {
                    continue;
                }
                let id = or_text(field(field(spec, "payload"), "wallet_id"));
                let row = wallets
                    .iter()
                    .find(|wallet| field(wallet, "id") == &s(id.as_str()))
                    .ok_or_else(|| {
                        AppError::new("stale_context", "Freshness wallet selection changed")
                    })?;
                let config = sources::wallet_config(row)?;
                let name = config["backend"].as_str().unwrap_or("").to_owned();
                selected.push(Source {
                    profile: or_text(field(profile, "id")),
                    key: or_text(field(spec, "source_key")),
                    wallet: row.clone(),
                    backend: backend(conn, &name),
                    backend_name: name,
                });
            }
        }
        Ok(Self {
            sources: selected,
            backends,
            book_id: book_id.unwrap_or_default(),
            context: plan.captured.context.clone(),
            proof,
            gate,
            cancel,
            deadline,
            start_time,
            version: version.into(),
            backend,
            prefetched: Default::default(),
        })
    }

    fn source(&self, job: &Value) -> Result<Source> {
        self.sources
            .iter()
            .find(|source| {
                field(job, "profile_id") == &s(source.profile.as_str())
                    && field(job, "source_key") == &s(source.key.as_str())
            })
            .cloned()
            .ok_or_else(|| AppError::new("stale_context", "Freshness source was not admitted"))
    }

    fn prepare(
        &mut self,
        conn: &Connection,
        job: &Value,
        emit: &mut dyn FnMut(&Value) -> Result<()>,
    ) -> Result<Prepared> {
        let discovered = self.discover(conn, job, emit)?;
        let prepared = self.fetch(conn, discovered, emit)?;
        self.repair(conn, prepared, emit)
    }

    /// `discover_wallet_backend` and the ownership preflight: Python finishes
    /// both for every wallet of a batch before its first request.
    fn discover(
        &mut self,
        conn: &Connection,
        job: &Value,
        emit: &mut dyn FnMut(&Value) -> Result<()>,
    ) -> Result<Discovered> {
        let id = or_text(field(job, "id"));
        check(
            conn,
            &self.context,
            self.proof,
            self.cancel,
            self.deadline,
            &id,
        )?;
        let selected = self.source(job)?;
        if (self.backend)(conn, &selected.backend_name) != selected.backend {
            return Err(AppError::new(
                "stale_context",
                "Wallet sync backend changed",
            ));
        }
        let backend = selected.backend.as_ref().map_err(Clone::clone)?;
        let wallet_id = or_text(field(&selected.wallet, "id"));
        let label = field(&selected.wallet, "label");
        emit(&dict([
            ("phase", s("discovery")),
            ("wallet", label.clone()),
        ]))?;
        let action = Fetch::new(
            conn,
            &selected.profile,
            &wallet_id,
            backend,
            truthy(field(field(job, "payload"), "force_full")),
            &self.version,
        )?;
        if action.captured_config() != or_text(field(&selected.wallet, "config_json")) {
            return Err(AppError::new(
                "stale_context",
                "Wallet sync selection changed",
            ));
        }
        self.proof.permitted(conn, &selected.profile)?;
        let grant = self.proof.grant(&action)?;
        let graph = super::graph::Grant::capture(
            conn,
            job,
            &selected.wallet,
            backend,
            self.proof.revoked.clone(),
            self.cancel.clone(),
        )
        .ok();
        Ok(Discovered {
            id,
            selected,
            action,
            grant,
            graph,
        })
    }

    fn fetch(
        &mut self,
        conn: &Connection,
        discovered: Discovered,
        emit: &mut dyn FnMut(&Value) -> Result<()>,
    ) -> Result<Prepared> {
        let Discovered {
            id,
            selected,
            action,
            grant,
            graph,
        } = discovered;
        let fetch = self.run_fetch(conn, &id, &selected, &action, &grant, emit)?;
        Ok(Prepared {
            id,
            selected,
            action,
            grant,
            fetch,
            graph,
        })
    }

    /// `_prepare_negative_balance_repairs` for one prepared job.
    fn repair(
        &mut self,
        conn: &Connection,
        prepared: Prepared,
        emit: &mut dyn FnMut(&Value) -> Result<()>,
    ) -> Result<Prepared> {
        let Prepared {
            id,
            selected,
            action,
            grant,
            fetch,
            graph,
        } = prepared;
        let fetch = crate::fetch::with_repair(
            conn,
            &action,
            &grant,
            fetch,
            &mut |repair, repair_grant| {
                // The repair is a whole `fetch_wallet_backend`, discovery first.
                emit(&dict([
                    ("phase", s("discovery")),
                    ("wallet", field(&selected.wallet, "label").clone()),
                ]))?;
                self.run_fetch(conn, &id, &selected, repair, repair_grant, emit)
            },
        )?;
        Ok(Prepared {
            id,
            selected,
            action,
            grant,
            fetch,
            graph,
        })
    }

    /// Run `fetch` (the job's own or its widened repair) under the batch's
    /// checks. Between requests they fence cancellation, consent, the job,
    /// the book, the wallet row and its backend; all cheap reads. The full
    /// `Fetch::revalidate` rebuilds the observer plan (key derivation and a
    /// BDK wallet), so it runs once before publication in `run`, not here:
    /// the transport calls this per request and per record.
    #[allow(clippy::too_many_arguments)]
    fn run_fetch(
        &mut self,
        conn: &Connection,
        id: &str,
        selected: &Source,
        fetch: &Fetch,
        grant: &FetchConsent,
        emit: &mut dyn FnMut(&Value) -> Result<()>,
    ) -> Result<Json> {
        let wallet_id = or_text(field(&selected.wallet, "id"));
        let label = field(&selected.wallet, "label");
        let resolver = std::cell::RefCell::new(&mut self.backend);
        let check_batch = || {
            check(
                conn,
                &self.context,
                self.proof,
                self.cancel,
                self.deadline,
                id,
            )?;
            let current = first(
                conn,
                "SELECT kind,config_json FROM wallets WHERE id=? AND profile_id=?",
                [&wallet_id, &selected.profile],
            )?;
            if current.as_ref().is_none_or(|wallet| {
                ["kind", "config_json"]
                    .iter()
                    .any(|key| field(wallet, key) != field(&selected.wallet, key))
            }) {
                return Err(AppError::new(
                    "stale_context",
                    "Wallet sync selection changed",
                ));
            }
            if resolver.borrow_mut()(conn, &selected.backend_name) != selected.backend {
                return Err(AppError::new(
                    "stale_context",
                    "Wallet sync backend changed",
                ));
            }
            self.proof.permitted(conn, &selected.profile)
        };
        let control = Checked {
            cancel: self.cancel,
            check: &check_batch,
        };
        fetch.run_with_progress(
            self.gate,
            fetch.scope(),
            Some(grant),
            self.deadline,
            &control,
            self.start_time,
            &mut |event| {
                let mut event = kassiber_pytext::loads(&event.to_string())
                    .map_err(|_| AppError::new("internal_error", "Invalid wallet progress JSON"))?;
                put(&mut event, "wallet", label.clone());
                emit(&event)
            },
        )
    }
}

fn check(
    conn: &Connection,
    context: &Value,
    proof: &Consent,
    cancel: &Cancellation,
    deadline: Instant,
    id: &str,
) -> Result<()> {
    cancel.check(deadline)?;
    proof.check()?;
    execution::check_job(conn, id)?;
    if kassiber_setup::context::snapshot(conn)? != *context {
        return Err(AppError::new(
            "stale_context",
            "Freshness selection changed",
        ));
    }
    Ok(())
}

fn failure(error: AppError) -> Failure {
    if error.code == "database_busy" {
        if let Some(code) =
            error
                .details
                .as_ref()
                .and_then(|v| match field(v, "sqlite_error_code") {
                    Value::Int(n) => n.to_string().parse::<i32>().ok(),
                    _ => None,
                })
        {
            return Failure::Database(kassiber_store::rusqlite::Error::SqliteFailure(
                kassiber_store::rusqlite::ffi::Error::new(code),
                None,
            ));
        }
    }
    Failure::App(error)
}

impl<B: FnMut(&Connection, &str) -> Result<Json>> Handler for Native<'_, B> {
    fn supports(&self, kind: &str) -> bool {
        matches!(
            kind,
            "onchain_wallet_history"
                | "btcpay_wallet_source"
                | "btcpay_provenance"
                | "market_rate_coverage"
        )
    }
    /// Python reaches `sync_wallet` only for a user-triggered sync, never a
    /// report-read refresh. The sync service checks its own selection, and
    /// cancelling the request stops it between requests.
    fn direct_sync(
        &mut self,
        conn: &mut Connection,
        action: &crate::sync::Sync,
        sources: &mut dyn wallet::Sources,
    ) -> Result<Option<Vec<Value>>> {
        if self.proof.report_read {
            return Ok(None);
        }
        self.proof.check()?;
        let consent = crate::sync::SyncConsent::explicit_trigger(action);
        // Greenfield reads open their own grants, bound to wallet and store.
        let btcpay = crate::sync::btcpay::Context {
            backends: self.backends,
            gate: self.gate,
            version: &self.version,
            book_id: &self.book_id,
            progress: None,
        };
        let backend = &mut self.backend;
        let rows = action.run(
            conn,
            self.gate,
            Some(&consent),
            self.cancel,
            self.deadline,
            self.start_time,
            sources,
            |conn, name| backend(conn, name),
            Some(&btcpay),
        );
        consent.revoke();
        rows.map(Some)
    }
    fn prefetch(
        &mut self,
        conn: &Connection,
        jobs: &[Value],
        observer: &mut dyn FnMut(&Value) -> Result<()>,
    ) -> Result<()> {
        let mut discovered = Vec::new();
        for job in jobs {
            if field(job, "job_type") == &s("onchain_wallet_history")
                && [s("queued"), s("rate_limited")].contains(field(job, "status"))
            {
                discovered.push((
                    or_text(field(job, "id")),
                    self.discover(conn, job, observer),
                ));
            }
        }
        let mut fetched = Vec::new();
        for (id, result) in discovered {
            fetched.push((
                id,
                result.and_then(|found| self.fetch(conn, found, observer)),
            ));
        }
        for (id, result) in fetched {
            let result = result.and_then(|prepared| self.repair(conn, prepared, observer));
            self.prefetched.insert(id, result);
        }
        Ok(())
    }
    fn run(
        &mut self,
        conn: &mut Connection,
        job: &Value,
        progress: &mut Progress<'_>,
    ) -> std::result::Result<Value, Failure> {
        match or_text(field(job, "job_type")).as_str() {
            "btcpay_wallet_source" => {
                return self.btcpay_wallet(conn, job, progress).map_err(failure)
            }
            "btcpay_provenance" => {
                return self.btcpay_provenance(conn, job, progress).map_err(failure)
            }
            "market_rate_coverage" => {
                return self.market_rates(conn, job, progress).map_err(failure)
            }
            _ => {}
        }
        let run = |this: &mut Self,
                   conn: &mut Connection,
                   progress: &mut Progress<'_>|
         -> Result<Value> {
            let id = or_text(field(job, "id"));
            check(
                conn,
                &this.context,
                this.proof,
                this.cancel,
                this.deadline,
                &id,
            )?;
            let selected = this.source(job)?;
            let wallet_id = or_text(field(&selected.wallet, "id"));
            let label = field(&selected.wallet, "label");
            progress.emit(
                conn,
                &dict([("phase", s("discovery")), ("wallet", label.clone())]),
            )?;
            progress.emit(
                conn,
                &dict([("phase", s("backend_fetch")), ("wallet", label.clone())]),
            )?;
            let Prepared {
                action,
                grant,
                fetch,
                graph,
                ..
            } = if let Some(result) = this.prefetched.remove(&id) {
                if result.is_err() {
                    progress.emit(conn, &dict([("phase", s("batch_prefetch"))]))?;
                }
                result?
            } else {
                this.prepare(conn, job, &mut |event| progress.emit(conn, event))?
            };
            check(
                conn,
                &this.context,
                this.proof,
                this.cancel,
                this.deadline,
                &id,
            )?;
            action.revalidate(conn)?;
            let backends = this.backends;
            let btcpay = crate::sync::btcpay::Context {
                backends,
                gate: this.gate,
                version: &this.version,
                book_id: &this.book_id,
                progress: None,
            };
            let mut results = wallet::apply_job(
                conn,
                &wallet::Request {
                    profile_id: &selected.profile,
                    wallet_id: &wallet_id,
                    captured_config: action.captured_config(),
                    fetch: &fetch,
                    btcpay: Some(&btcpay),
                },
                progress.sources,
                |conn, _| {
                    check(
                        conn,
                        &this.context,
                        this.proof,
                        this.cancel,
                        this.deadline,
                        &id,
                    )?;
                    if (this.backend)(conn, &selected.backend_name) != selected.backend {
                        return Err(AppError::new(
                            "stale_context",
                            "Wallet sync backend changed",
                        ));
                    }
                    this.proof.permitted(conn, &selected.profile)?;
                    action.check_permission(this.gate, Some(&grant))
                },
                &mut |conn, checkpoint| {
                    this.proof.check()?;
                    this.cancel.check(this.deadline)?;
                    this.proof.permitted(conn, &selected.profile)?;
                    let rescan = action.rescan(conn, checkpoint)?;
                    let grant = FetchConsent::for_repair(&rescan, &grant);
                    rescan.check_permission(this.gate, Some(&grant))?;
                    rescan.run(
                        this.gate,
                        rescan.scope(),
                        Some(&grant),
                        this.deadline,
                        this.cancel,
                        this.start_time,
                    )
                },
            )?;
            progress.emit(
                conn,
                &dict([("phase", s("import")), ("wallet", label.clone())]),
            )?;
            check(
                conn,
                &this.context,
                this.proof,
                this.cancel,
                this.deadline,
                &id,
            )?;
            let mut config = kassiber_pytext::loads(action.captured_config()).map_err(|_| {
                AppError::new("internal_error", "Invalid wallet configuration JSON")
            })?;
            put(&mut config, "last_synced_at", s(progress.sources.now()));
            let config =
                kassiber_pyjson::dumps(&config, &Options::new().sort_keys(true)).map_err(|_| {
                    AppError::new("internal_error", "Invalid wallet configuration JSON")
                })?;
            execution::begin(conn)?;
            conn.execute(
                "UPDATE wallets SET config_json=? WHERE id=?",
                params![config, wallet_id],
            )
            .map_err(crate::database_error)?;
            execution::commit(conn)?;
            let result = results.pop().ok_or_else(|| {
                AppError::new("state_not_ready", "Wallet sync produced no result")
            })?;
            if let Some(graph) = graph {
                this.proof.collect_graph(graph);
            }
            Ok(result)
        };
        run(self, conn, progress).map_err(failure)
    }
}

use super::{
    execution::{self, Failure, Handler, Progress},
    planning::Plan,
    put, sources,
};
use crate::{
    fetch::{Fetch, FetchConsent},
    publication::{
        row::{field, or_text, Result},
        wallet,
    },
    scan::Cancellation,
};
use kassiber_envelope::{dict, s, truthy, AppError};
use kassiber_net::Gate;
use kassiber_pyjson::{Options, Value};
use kassiber_store::rusqlite::{params, Connection};
use serde_json::Value as Json;
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
    revoked: AtomicBool,
    grants: Mutex<Vec<Arc<FetchConsent>>>,
}

impl Consent {
    pub fn explicit_trigger(plan: &Plan) -> Self {
        Self {
            identity: plan.identity.clone(),
            started: AtomicBool::new(false),
            revoked: AtomicBool::new(false),
            grants: Mutex::new(vec![]),
        }
    }
    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::SeqCst);
        if let Ok(grants) = self.grants.lock() {
            for grant in grants.iter() {
                grant.revoke();
            }
        }
    }
    fn check(&self) -> Result<()> {
        if self.revoked.load(Ordering::SeqCst) {
            return Err(AppError::new(
                "interaction_required",
                "Freshness sync consent was revoked",
            ));
        }
        Ok(())
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
        let grant = Arc::new(FetchConsent::explicit_trigger(action));
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

struct Source {
    profile: String,
    key: String,
    wallet: Value,
    backend_name: String,
    backend: Result<Json>,
}

pub(super) struct Native<'a, B> {
    sources: Vec<Source>,
    context: Value,
    proof: &'a Consent,
    gate: &'a Gate,
    cancel: &'a Cancellation,
    deadline: Instant,
    start_time: u64,
    version: String,
    backend: B,
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
        mut backend: B,
    ) -> Result<Self> {
        let mut selected = Vec::new();
        for ((profile, specs), wallets) in plan
            .captured
            .profiles
            .iter()
            .zip(&plan.captured.specs)
            .zip(&plan.captured.wallets)
        {
            for spec in specs {
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
            context: plan.captured.context.clone(),
            proof,
            gate,
            cancel,
            deadline,
            start_time,
            version: version.into(),
            backend,
        })
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
        kind == "onchain_wallet_history"
    }
    fn run(
        &mut self,
        conn: &mut Connection,
        job: &Value,
        progress: &mut Progress<'_>,
    ) -> std::result::Result<Value, Failure> {
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
            let selected = this
                .sources
                .iter()
                .find(|source| {
                    field(job, "profile_id") == &s(source.profile.as_str())
                        && field(job, "source_key") == &s(source.key.as_str())
                })
                .ok_or_else(|| {
                    AppError::new("stale_context", "Freshness source was not admitted")
                })?;
            if (this.backend)(conn, &selected.backend_name) != selected.backend {
                return Err(AppError::new(
                    "stale_context",
                    "Wallet sync backend changed",
                ));
            }
            let backend = selected.backend.as_ref().map_err(Clone::clone)?;
            let wallet_id = or_text(field(&selected.wallet, "id"));
            let action = Fetch::new(
                conn,
                &selected.profile,
                &wallet_id,
                backend,
                truthy(field(field(job, "payload"), "force_full")),
                &this.version,
            )?;
            if action.captured_config() != or_text(field(&selected.wallet, "config_json")) {
                return Err(AppError::new(
                    "stale_context",
                    "Wallet sync selection changed",
                ));
            }
            let grant = this.proof.grant(&action)?;
            let label = field(&selected.wallet, "label");
            progress.emit(
                conn,
                &dict([("phase", s("discovery")), ("wallet", label.clone())]),
            )?;
            progress.emit(
                conn,
                &dict([("phase", s("backend_fetch")), ("wallet", label.clone())]),
            )?;
            check(
                conn,
                &this.context,
                this.proof,
                this.cancel,
                this.deadline,
                &id,
            )?;
            let fetch = action.run(
                this.gate,
                action.scope(),
                Some(&grant),
                this.deadline,
                this.cancel,
                this.start_time,
            )?;
            check(
                conn,
                &this.context,
                this.proof,
                this.cancel,
                this.deadline,
                &id,
            )?;
            action.revalidate(conn)?;
            let mut results = wallet::apply_job(
                conn,
                &wallet::Request {
                    profile_id: &selected.profile,
                    wallet_id: &wallet_id,
                    captured_config: action.captured_config(),
                    fetch: &fetch,
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
                    action.check_permission(this.gate, Some(&grant))
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
            results
                .pop()
                .ok_or_else(|| AppError::new("state_not_ready", "Wallet sync produced no result"))
        };
        run(self, conn, progress).map_err(failure)
    }
}

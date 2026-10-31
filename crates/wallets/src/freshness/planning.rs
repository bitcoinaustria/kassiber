use super::{sources, workspace_profiles, Queue};
use crate::{
    publication::{
        row::{field, first, or_text, Result},
        wallet::{fallback, SystemSources},
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
struct Captured {
    context: Value,
    workspace: Option<Value>,
    profiles: Vec<Value>,
    wallets: Vec<Vec<Value>>,
    policies: Vec<Value>,
    specs: Vec<Vec<Value>>,
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
    captured: Captured,
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
}

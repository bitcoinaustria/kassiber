use crate::{
    fetch::{Fetch, FetchConsent},
    publication::{
        row::{field, first, or_text, rows},
        wallet::{self, Sources},
    },
    scan::Cancellation,
};
use kassiber_envelope::{dict, s, AppError};
use kassiber_net::Gate;
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use serde_json::Value as Json;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

type Result<T> = std::result::Result<T, AppError>;

pub struct Request<'a> {
    pub profile_id: &'a str,
    pub wallet: Option<&'a str>,
    pub all: bool,
    pub force_full: bool,
    pub version: &'a str,
}

struct Selected {
    row: Value,
    environment: Value,
    backend_name: String,
    backend: Option<Json>,
    fetch: Result<Option<Fetch>>,
}

pub struct Sync {
    identity: Arc<()>,
    connection: usize,
    database: String,
    database_identity: Option<Value>,
    profile: Value,
    request_wallet: Option<String>,
    all: bool,
    selected: Vec<Selected>,
}

pub struct SyncConsent {
    identity: Arc<()>,
    grants: Vec<Option<FetchConsent>>,
    started: AtomicBool,
    revoked: AtomicBool,
}

impl SyncConsent {
    pub fn explicit_trigger(action: &Sync) -> Self {
        Self {
            identity: action.identity.clone(),
            grants: action
                .selected
                .iter()
                .map(|w| {
                    w.fetch
                        .as_ref()
                        .ok()
                        .and_then(|f| f.as_ref())
                        .map(FetchConsent::explicit_trigger)
                })
                .collect(),
            started: AtomicBool::new(false),
            revoked: AtomicBool::new(false),
        }
    }

    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::SeqCst);
        for grant in self.grants.iter().flatten() {
            grant.revoke();
        }
    }
}

fn config(row: &Value) -> Result<Json> {
    let raw = or_text(field(row, "config_json"));
    let parsed: Json = serde_json::from_str(if raw.is_empty() { "{}" } else { &raw })
        .map_err(|_| AppError::new("internal_error", "Invalid wallet configuration JSON"))?;
    if !parsed.is_object() {
        return Err(AppError::new(
            "internal_error",
            "Wallet configuration must be an object",
        ));
    }
    Ok(parsed)
}

fn selected(
    conn: &Connection,
    profile: &str,
    reference: Option<&str>,
    all: bool,
) -> Result<Vec<Value>> {
    if all && reference.is_some_and(|v| !v.is_empty()) {
        return Err(AppError::new(
            "validation",
            "--wallet and --all are mutually exclusive",
        ));
    }
    if all {
        return rows(
            conn,
            "SELECT * FROM wallets WHERE profile_id=? ORDER BY label ASC",
            [profile],
        )?
        .into_iter()
        .filter_map(
            |row| match crate::freshness::sources::stored_wallet_config(&row) {
                Ok(c) => match crate::config::boolean(c.get("deprecated")) {
                    Ok(true) => None,
                    Ok(false) => Some(Ok(row)),
                    Err(e) => Some(Err(e)),
                },
                Err(e) => Some(Err(e)),
            },
        )
        .collect();
    }
    let reference = reference
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::new("app_error", "Provide --wallet or use --all"))?;
    let wallet = crate::resolve(conn, profile, reference)?;
    Ok(vec![first(
        conn,
        "SELECT * FROM wallets WHERE profile_id=? AND id=?",
        [profile, &wallet.id],
    )?
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            "Sync wallet does not belong to the selected book",
        )
    })?])
}

fn no_source(c: &Json) -> bool {
    ![
        "descriptor",
        "change_descriptor",
        "xpub",
        "addresses",
        "silent_payment",
        "silent_payments",
        "source_file",
        "source_format",
        "sync_source",
        "btcpay_provenance",
    ]
    .iter()
    .any(|k| c.get(k).is_some_and(crate::descriptors::truthy))
}

fn database_identity(conn: &Connection) -> Result<Option<Value>> {
    first(
        conn,
        "SELECT value FROM settings WHERE key='database_instance_id'",
        [],
    )
}

pub fn resolve_backend(
    conn: &Connection,
    config: &kassiber_setup::backends::Config,
    requested: &str,
) -> Result<Json> {
    let mut current = config.clone();
    current.merge(conn)?;
    let name = if requested.trim().is_empty() {
        current.default_backend.as_str()
    } else {
        requested
    };
    if name.trim().is_empty() {
        return Err(
            AppError::new("backend_not_configured", "No sync backend is configured")
                .with_hint("Add a backend in Settings before refreshing this wallet."),
        );
    }
    current.resolve(conn, name)
}

impl Sync {
    pub fn new(
        conn: &Connection,
        request: &Request<'_>,
        mut backend: impl FnMut(&Connection, &str) -> Result<Json>,
    ) -> Result<Self> {
        if !conn.is_autocommit() {
            return Err(AppError::new(
                "observer_prepare_in_transaction",
                "Wallet sync must start outside a transaction",
            ));
        }
        let profile = first(
            conn,
            "SELECT * FROM profiles WHERE id=?",
            [request.profile_id],
        )?
        .ok_or_else(|| AppError::new("not_found", "Profile not found"))?;
        let mut wallets = Vec::new();
        for row in selected(conn, request.profile_id, request.wallet, request.all)? {
            let c = config(&row)?;
            let kind = or_text(field(&row, "kind"));
            let (_, environment) = wallet::network_scope(
                conn,
                request.profile_id,
                &kind,
                &or_text(field(&row, "config_json")),
            )?;
            let node_backend = matches!(kind.as_str(), "coreln" | "lnd")
                && c.get("backend").is_some_and(crate::descriptors::truthy);
            let backend_name = c["backend"].as_str().unwrap_or("").to_owned();
            let (resolved, fetch) = if no_source(&c) && !node_backend {
                (None, Ok(None))
            } else {
                wallet::require_supported(conn, request.profile_id, &row)?;
                let resolved = backend(conn, &backend_name);
                match resolved {
                    Ok(resolved) => {
                        let fetch = Fetch::new(
                            conn,
                            request.profile_id,
                            &or_text(field(&row, "id")),
                            &resolved,
                            request.force_full,
                            request.version,
                        )
                        .map(Some);
                        if let Err(e) = &fetch {
                            if e.code == "native_fallback_required" {
                                return Err(e.clone());
                            }
                        }
                        (Some(resolved), fetch)
                    }
                    Err(e) => {
                        if e.code == "native_fallback_required" {
                            return Err(e);
                        }
                        (None, Err(e))
                    }
                }
            };
            wallets.push(Selected {
                row,
                environment,
                backend_name,
                backend: resolved,
                fetch,
            });
        }
        Ok(Self {
            identity: Arc::new(()),
            connection: std::ptr::from_ref(conn).addr(),
            database: conn.path().unwrap_or("").into(),
            database_identity: database_identity(conn)?,
            profile,
            request_wallet: request.wallet.map(str::to_owned),
            all: request.all,
            selected: wallets,
        })
    }

    pub fn wallet_ids(&self) -> Vec<String> {
        self.selected
            .iter()
            .map(|w| or_text(field(&w.row, "id")))
            .collect()
    }

    pub(crate) fn validate(
        &self,
        conn: &Connection,
        backend: &mut impl FnMut(&Connection, &str) -> Result<Json>,
    ) -> Result<()> {
        let profile_id = or_text(field(&self.profile, "id"));
        let profile = first(conn, "SELECT * FROM profiles WHERE id=?", [&profile_id])?;
        let stable_profile = profile.as_ref().is_some_and(|p| {
            ["id", "workspace_id"]
                .iter()
                .all(|k| field(p, k) == field(&self.profile, k))
        });
        if std::ptr::from_ref(conn).addr() != self.connection
            || conn.path().unwrap_or("") != self.database
            || database_identity(conn)? != self.database_identity
            || !stable_profile
        {
            return Err(AppError::new("stale_context", "Wallet sync book changed"));
        }
        let current = selected(conn, &profile_id, self.request_wallet.as_deref(), self.all)?;
        if current.len() != self.selected.len()
            || current.iter().zip(&self.selected).any(|(a, b)| a != &b.row)
        {
            return Err(AppError::new(
                "stale_context",
                "Wallet sync selection changed",
            ));
        }
        for wallet in &self.selected {
            let (_, environment) = wallet::network_scope(
                conn,
                &profile_id,
                &or_text(field(&wallet.row, "kind")),
                &or_text(field(&wallet.row, "config_json")),
            )?;
            if environment != wallet.environment {
                return Err(AppError::new(
                    "stale_context",
                    "Wallet sync network changed",
                ));
            }
            if let Some(captured) = &wallet.backend {
                if backend(conn, &wallet.backend_name)? != *captured {
                    return Err(AppError::new(
                        "stale_context",
                        "Wallet sync backend changed",
                    ));
                }
            }
            if let Ok(Some(action)) = &wallet.fetch {
                action.revalidate(conn)?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &self,
        conn: &mut Connection,
        gate: &Gate,
        consent: Option<&SyncConsent>,
        cancel: &Cancellation,
        deadline: Instant,
        start_time: u64,
        sources: &mut dyn Sources,
        mut backend: impl FnMut(&Connection, &str) -> Result<Json>,
    ) -> Result<Vec<Value>> {
        self.execute(
            conn,
            gate,
            consent,
            cancel,
            deadline,
            sources,
            &mut backend,
            |action, grant| {
                action.run(
                    gate,
                    action.scope(),
                    Some(grant),
                    deadline,
                    cancel,
                    start_time,
                )
            },
            |_| Ok(()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn execute(
        &self,
        conn: &mut Connection,
        gate: &Gate,
        consent: Option<&SyncConsent>,
        cancel: &Cancellation,
        deadline: Instant,
        sources: &mut dyn Sources,
        backend: &mut impl FnMut(&Connection, &str) -> Result<Json>,
        mut fetch: impl FnMut(&Fetch, &FetchConsent) -> Result<Json>,
        mut stage: impl FnMut(&str) -> Result<()>,
    ) -> Result<Vec<Value>> {
        cancel.check(deadline)?;
        if !conn.is_autocommit() {
            return Err(AppError::new(
                "observer_prepare_in_transaction",
                "Wallet sync must run outside a transaction",
            ));
        }
        self.validate(conn, backend)?;
        let proof = consent
            .filter(|p| {
                Arc::ptr_eq(&p.identity, &self.identity) && !p.revoked.load(Ordering::SeqCst)
            })
            .ok_or_else(|| {
                AppError::new(
                    "interaction_required",
                    "Wallet sync requires consent for this exact action",
                )
            })?;
        if proof.started.swap(true, Ordering::SeqCst) {
            return Err(AppError::new(
                "interaction_required",
                "Wallet sync consent was already used",
            ));
        }
        let mut prepared = Vec::new();
        for (wallet, grant) in self.selected.iter().zip(&proof.grants) {
            cancel.check(deadline)?;
            self.validate(conn, backend)?;
            let result = match &wallet.fetch {
                Ok(Some(action)) => {
                    let grant = grant.as_ref().expect("matching service grant");
                    action
                        .check_permission(gate, Some(grant))
                        .and_then(|()| fetch(action, grant))
                        .map(Some)
                }
                Ok(None) => Ok(None),
                Err(e) => Err(e.clone()),
            };
            prepared.push(result);
        }
        let profile_id = or_text(field(&self.profile, "id"));
        let mut results = Vec::new();
        for ((wallet, grant), prepared) in self.selected.iter().zip(&proof.grants).zip(prepared) {
            cancel.check(deadline)?;
            let result = match prepared {
                Ok(Some(fetch)) => {
                    let action = wallet
                        .fetch
                        .as_ref()
                        .ok()
                        .and_then(|a| a.as_ref())
                        .expect("prepared action");
                    self.validate(conn, backend)?;
                    wallet::apply(
                        conn,
                        &wallet::Request {
                            profile_id: &profile_id,
                            wallet_id: &or_text(field(&wallet.row, "id")),
                            captured_config: action.captured_config(),
                            fetch: &fetch,
                        },
                        sources,
                        |name| {
                            cancel.check(deadline)?;
                            action.check_permission(gate, grant.as_ref())?;
                            stage(name)?;
                            cancel.check(deadline)?;
                            action.check_permission(gate, grant.as_ref())
                        },
                    )
                }
                Ok(None) => Ok(vec![dict([
                    ("wallet", field(&wallet.row, "label").clone()),
                    ("status", s("skipped")),
                    (
                        "reason",
                        s("no file source, descriptor, or backend addresses configured"),
                    ),
                ])]),
                Err(e) => Err(e),
            };
            match result {
                Ok(rows) => results.extend(rows),
                Err(e) if self.all => results.push(error_row(field(&wallet.row, "label"), &e)),
                Err(e) => return Err(e),
            }
        }
        Ok(results)
    }
}

fn error_row(label: &Value, e: &AppError) -> Value {
    dict([
        ("wallet", label.clone()),
        ("status", s("error")),
        ("code", s(e.code.as_str())),
        ("message", s(redact_backend_text(&e.message))),
        (
            "hint",
            s(e.hint
                .as_deref()
                .map(redact_backend_text)
                .unwrap_or_default()),
        ),
        (
            "details",
            e.details.as_ref().map_or(Value::Null, redact_backend_value),
        ),
        ("retryable", Value::Bool(e.retryable)),
    ])
}

fn redact_backend_text(value: &str) -> String {
    use std::sync::OnceLock;
    static URLS: OnceLock<regex::Regex> = OnceLock::new();
    let pattern = URLS.get_or_init(|| {
        regex::Regex::new(r#"\b[a-zA-Z][a-zA-Z0-9+.-]*://(?:\[[^\]\s]+\][^\s,;)"']*|[^\s,;)"']+)"#)
            .expect("backend URL pattern")
    });
    pattern
        .replace_all(value, |m: &regex::Captures<'_>| {
            let raw = m.get(0).unwrap().as_str();
            let url = raw.trim_end_matches([':', '.', '!', '?']);
            format!("{}{}", wallet::redact_url(url), &raw[url.len()..])
        })
        .into_owned()
}

fn redact_backend_value(value: &Value) -> Value {
    redact_backend_at_depth(value, 0)
}

fn redact_backend_at_depth(value: &Value, depth: usize) -> Value {
    if depth > 8 {
        return value.clone();
    }
    match value {
        Value::Str(v) => v
            .as_str()
            .map_or(Value::Null, |v| s(redact_backend_text(v))),
        Value::Dict(v) => Value::Dict(
            v.iter()
                .map(|(k, v)| (k.clone(), redact_backend_at_depth(v, depth + 1)))
                .collect(),
        ),
        Value::List(v) => Value::List(
            v.iter()
                .map(|v| redact_backend_at_depth(v, depth + 1))
                .collect(),
        ),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests;

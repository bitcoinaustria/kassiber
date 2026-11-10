use crate::{
    fetch::{Fetch, FetchConsent},
    publication::{
        row::{field, first, or_text, rows},
        wallet::{self, Sources},
    },
    scan::Cancellation,
};
use kassiber_envelope::{dict, py_str, s, AppError};
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
    /// `(source_file, source_format)`: Python's file route.
    file: Option<(Value, Value)>,
    /// A BTCPay wallet source (`sync_source`).
    btcpay: bool,
    /// A Core Lightning or LND wallet's resolved node backend.
    node: Option<Result<Json>>,
}

pub struct Sync {
    identity: Arc<()>,
    connection: usize,
    database: String,
    database_identity: Option<Value>,
    profile: Value,
    request_wallet: Option<String>,
    all: bool,
    force_full: bool,
    selected: Vec<Selected>,
    /// Wallets whose route this service cannot take, kept for a caller that
    /// may never run it (a freshness plan's direct sync).
    refused: Vec<(String, AppError)>,
    /// Every selected row as prepared, refused ones included.
    rows: Vec<Value>,
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

/// `published` names wallets this operation already synced: a retired
/// wallet's first sync records its sync time, which must not drop it from
/// the selection it was prepared in.
fn selected(
    conn: &Connection,
    profile: &str,
    reference: Option<&str>,
    all: bool,
    force_full: bool,
    published: &[Value],
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
                Ok(_) if published.contains(field(&row, "id")) => Some(Ok(row)),
                Ok(c) => match crate::config::in_sync_all(&c, force_full) {
                    Ok(false) => None,
                    Ok(true) => Some(Ok(row)),
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

/// `classify_wallet_sync(...) == "none"` past the BTCPay, node and file
/// routes: no addresses, descriptor material or silent-payment material.
fn no_source(c: &Json) -> Result<bool> {
    let text = |key: &str| crate::config::optional_text(c.get(key)).is_some();
    let descriptor = text("descriptor")
        || (text("xpub")
            && c.get("script_types")
                .is_some_and(crate::descriptors::truthy));
    Ok(!(descriptor
        || crate::silent_payments::has_material(c)
        || !crate::config::addresses(c.get("addresses"))?.is_empty()))
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
        backend: impl FnMut(&Connection, &str) -> Result<Json>,
    ) -> Result<Self> {
        Self::build(conn, request, backend, false)
    }

    /// [`Sync::new`] that records a wallet's `native_fallback_required`
    /// instead of refusing: running the action refuses with the first one,
    /// and [`Sync::refusal`] names it for a caller that must refuse earlier.
    pub fn new_deferred(
        conn: &Connection,
        request: &Request<'_>,
        backend: impl FnMut(&Connection, &str) -> Result<Json>,
    ) -> Result<Self> {
        Self::build(conn, request, backend, true)
    }

    /// The deferred refusal of the first wallet in `wallets`, or of any
    /// wallet when `wallets` is `None`.
    pub fn refusal(&self, wallets: Option<&[String]>) -> Option<&AppError> {
        self.refused
            .iter()
            .find(|(id, _)| wallets.is_none_or(|ids| ids.contains(id)))
            .map(|(_, error)| error)
    }

    fn build(
        conn: &Connection,
        request: &Request<'_>,
        mut backend: impl FnMut(&Connection, &str) -> Result<Json>,
        defer: bool,
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
        let mut refused = Vec::new();
        let rows = selected(
            conn,
            request.profile_id,
            request.wallet,
            request.all,
            request.force_full,
            &[],
        )?;
        for row in rows.clone() {
            let id = or_text(field(&row, "id"));
            let prepared = (|| -> Result<Selected> {
                // Python's explicit `--wallet` path crashes on an unreadable
                // configuration; Rust refuses it typed, as `--all` does.
                let c = crate::freshness::sources::stored_wallet_config(&row)?;
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
                let py_config = sources::config(&row);
                // Python's dispatch order: BTCPay source, node, file, chain.
                let btcpay = crate::config::optional_text(c.get("sync_source")).is_some();
                if btcpay {
                    crate::config::normalize_btcpay(&mut c.clone())?;
                }
                let file = if btcpay || node_backend {
                    None
                } else {
                    sources::file_source(&py_config)
                };
                // `hooks.sync_core_lightning_wallet` resolves the wallet's
                // backend when it runs; a resolution failure is that wallet's.
                let node = (!btcpay && node_backend).then(|| backend(conn, &backend_name));
                if let Some(Err(error)) = &node {
                    if error.code == "native_fallback_required" {
                        return Err(error.clone());
                    }
                }
                let (resolved, fetch) = if btcpay
                    || node.is_some()
                    || file.is_some()
                    || (no_source(&c)? && !node_backend)
                {
                    (None, Ok(None))
                } else {
                    wallet::require_supported(&row)?;
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
                Ok(Selected {
                    row,
                    environment,
                    backend_name,
                    backend: resolved,
                    fetch,
                    file,
                    btcpay,
                    node,
                })
            })();
            match prepared {
                Ok(selected) => wallets.push(selected),
                Err(error) if defer && error.code == "native_fallback_required" => {
                    refused.push((id, error))
                }
                Err(error) => return Err(error),
            }
        }
        Ok(Self {
            identity: Arc::new(()),
            connection: std::ptr::from_ref(conn).addr(),
            database: conn.path().unwrap_or("").into(),
            database_identity: database_identity(conn)?,
            profile,
            request_wallet: request.wallet.map(str::to_owned),
            all: request.all,
            force_full: request.force_full,
            selected: wallets,
            refused,
            rows,
        })
    }

    /// What `sync_wallets` answers when no selected wallet has anything to
    /// refresh (exchange API wallets, empty manual wallets): one `skipped` row
    /// each, with no I/O and no writes. `None` when any wallet has a source,
    /// including ones Python classifies by keys this service does not read.
    pub fn sourceless_results(&self) -> Option<Vec<Value>> {
        if !self.refused.is_empty() {
            return None;
        }
        self.selected
            .iter()
            .map(|wallet| {
                let config = config(&wallet.row).ok()?;
                let kind = or_text(field(&wallet.row, "kind"));
                let sourced = matches!(wallet.fetch, Ok(Some(_)) | Err(_))
                    || wallet.file.is_some()
                    || wallet.btcpay
                    || wallet.backend.is_some()
                    || !no_source(&config).ok()?
                    // Python's file route: both keys present (`import_file`).
                    || ["source_file", "source_format"]
                        .iter()
                        .all(|key| config.get(*key).is_some_and(crate::descriptors::truthy))
                    || matches!(
                        kind.as_str(),
                        "coreln" | "lnd" | "nwc" | "phoenix" | "silent-payment"
                    );
                (!sourced).then(|| {
                    dict([
                        ("wallet", field(&wallet.row, "label").clone()),
                        ("status", s("skipped")),
                        (
                            "reason",
                            s("no file source, descriptor, or backend addresses configured"),
                        ),
                    ])
                })
            })
            .collect()
    }

    pub fn wallet_ids(&self) -> Vec<String> {
        self.selected
            .iter()
            .map(|w| or_text(field(&w.row, "id")))
            .collect()
    }

    /// `publishing` names the wallet about to publish: wallets this operation
    /// already published have moved on, so only its fetch is revalidated.
    pub(crate) fn validate(
        &self,
        conn: &Connection,
        backend: &mut impl FnMut(&Connection, &str) -> Result<Json>,
        publishing: Option<usize>,
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
        // A published wallet's row records its sync time; later ones must
        // be exactly as prepared.
        let published: Vec<Value> = self
            .selected
            .iter()
            .take(publishing.unwrap_or(0))
            .map(|w| field(&w.row, "id").clone())
            .collect();
        let current = selected(
            conn,
            &profile_id,
            self.request_wallet.as_deref(),
            self.all,
            self.force_full,
            &published,
        )?;
        // A refused wallet never publishes, so the whole selection stands.
        if !self.refused.is_empty() && current != self.rows {
            return Err(AppError::new(
                "stale_context",
                "Wallet sync selection changed",
            ));
        }
        if self.refused.is_empty()
            && (current.len() != self.selected.len()
                || current
                    .iter()
                    .zip(&self.selected)
                    .enumerate()
                    .any(|(index, (a, b))| {
                        if publishing.is_some_and(|selected| index < selected) {
                            field(a, "id") != field(&b.row, "id")
                        } else {
                            a != &b.row
                        }
                    }))
        {
            return Err(AppError::new(
                "stale_context",
                "Wallet sync selection changed",
            ));
        }
        for (index, wallet) in self.selected.iter().enumerate() {
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
                if publishing.is_none_or(|selected| selected == index) {
                    action.revalidate(conn)?;
                }
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
        btcpay: Option<&btcpay::Context<'_>>,
    ) -> Result<Vec<Value>> {
        self.execute_with(
            conn,
            gate,
            consent,
            cancel,
            deadline,
            sources,
            &mut backend,
            btcpay,
            |action, grant| {
                action.emit_silent_progress();
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

    #[cfg(test)]
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
        fetch: impl FnMut(&Fetch, &FetchConsent) -> Result<Json>,
        stage: impl FnMut(&str) -> Result<()>,
    ) -> Result<Vec<Value>> {
        self.execute_with(
            conn, gate, consent, cancel, deadline, sources, backend, None, fetch, stage,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_with(
        &self,
        conn: &mut Connection,
        gate: &Gate,
        consent: Option<&SyncConsent>,
        cancel: &Cancellation,
        deadline: Instant,
        sources: &mut dyn Sources,
        backend: &mut impl FnMut(&Connection, &str) -> Result<Json>,
        btcpay: Option<&btcpay::Context<'_>>,
        mut fetch_with: impl FnMut(&Fetch, &FetchConsent) -> Result<Json>,
        mut stage: impl FnMut(&str) -> Result<()>,
    ) -> Result<Vec<Value>> {
        if let Some(error) = self.refusal(None) {
            return Err(error.clone());
        }
        cancel.check(deadline)?;
        if !conn.is_autocommit() {
            return Err(AppError::new(
                "observer_prepare_in_transaction",
                "Wallet sync must run outside a transaction",
            ));
        }
        self.validate(conn, backend, None)?;
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
            self.validate(conn, backend, None)?;
            let result = match &wallet.fetch {
                Ok(Some(action)) => {
                    let grant = grant.as_ref().expect("matching service grant");
                    action
                        .check_permission(gate, Some(grant))
                        .and_then(|()| fetch_with(action, grant))
                        .map(Some)
                }
                Ok(None) => Ok(None),
                Err(e) => Err(e.clone()),
            };
            prepared.push(result);
        }
        // `_prepare_negative_balance_repairs`, after every wallet's fetch.
        for ((wallet, grant), result) in self.selected.iter().zip(&proof.grants).zip(&mut prepared)
        {
            let (Ok(Some(action)), Ok(Some(_))) = (&wallet.fetch, &*result) else {
                continue;
            };
            cancel.check(deadline)?;
            let grant = grant.as_ref().expect("matching service grant");
            let Ok(Some(first)) = std::mem::replace(result, Ok(None)) else {
                unreachable!("checked above");
            };
            *result =
                crate::fetch::with_repair(conn, action, grant, first, &mut |repair, grant| {
                    repair.check_permission(gate, Some(grant))?;
                    fetch_with(repair, grant)
                })
                .map(Some);
        }
        let profile_id = or_text(field(&self.profile, "id"));
        let book_id = self
            .database_identity
            .as_ref()
            .map(py_str)
            .unwrap_or_default();
        let node_ctx = node::Context {
            gate,
            book_id: &book_id,
        };
        let mut results = Vec::new();
        for (index, ((wallet, grant), prepared)) in self
            .selected
            .iter()
            .zip(&proof.grants)
            .zip(prepared)
            .enumerate()
        {
            cancel.check(deadline)?;
            let result = match (&wallet.file, prepared) {
                (Some((file, format)), _) => {
                    self.validate(conn, backend, Some(index))?;
                    sync_local(
                        conn,
                        &self.profile,
                        &wallet.row,
                        Local::File(file, format),
                        btcpay,
                        sources,
                    )
                }
                (None, _) if wallet.node.is_some() => {
                    self.validate(conn, backend, Some(index))?;
                    sync_local(
                        conn,
                        &self.profile,
                        &wallet.row,
                        Local::Node(wallet.node.as_ref().expect("node route"), &node_ctx),
                        btcpay,
                        sources,
                    )
                }
                (None, _) if wallet.btcpay => {
                    self.validate(conn, backend, Some(index))?;
                    sync_local(
                        conn,
                        &self.profile,
                        &wallet.row,
                        Local::Btcpay,
                        btcpay,
                        sources,
                    )
                }
                (None, Ok(Some(fetch))) => {
                    let action = wallet
                        .fetch
                        .as_ref()
                        .ok()
                        .and_then(|a| a.as_ref())
                        .expect("prepared action");
                    self.validate(conn, backend, Some(index))?;
                    let grant = grant.as_ref().expect("matching service grant");
                    crate::progress::silenced(|| {
                        wallet::apply_with_rescan(
                            conn,
                            &wallet::Request {
                                profile_id: &profile_id,
                                wallet_id: &or_text(field(&wallet.row, "id")),
                                captured_config: action.captured_config(),
                                fetch: &fetch,
                                btcpay,
                            },
                            sources,
                            |name| {
                                cancel.check(deadline)?;
                                action.check_permission(gate, Some(grant))?;
                                stage(name)?;
                                cancel.check(deadline)?;
                                action.check_permission(gate, Some(grant))
                            },
                            &mut |conn, checkpoint| {
                                cancel.check(deadline)?;
                                let rescan = action.rescan(conn, checkpoint)?;
                                let grant = FetchConsent::for_repair(&rescan, grant);
                                rescan.check_permission(gate, Some(&grant))?;
                                fetch_with(&rescan, &grant)
                            },
                        )
                    })
                }
                (None, Ok(None)) => Ok(vec![dict([
                    ("wallet", field(&wallet.row, "label").clone()),
                    ("status", s("skipped")),
                    (
                        "reason",
                        s("no file source, descriptor, or backend addresses configured"),
                    ),
                ])]),
                (None, Err(e)) => Err(e),
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

enum Local<'a> {
    File(&'a Value, &'a Value),
    Btcpay,
    /// A Core Lightning wallet and its resolved node backend.
    Node(&'a Result<Json>, &'a node::Context<'a>),
}

/// The wallet's stored configuration as a JSON object.
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

enum RouteKind {
    Local,
    Node,
}

/// One file-, BTCPay- or node-sourced wallet under the coordinator's savepoint:
/// its source, the enrichments Python merges, the synced stamp, or nothing.
fn sync_local(
    conn: &mut Connection,
    profile: &Value,
    row: &Value,
    route: Local<'_>,
    btcpay: Option<&btcpay::Context<'_>>,
    clock: &mut dyn Sources,
) -> Result<Vec<Value>> {
    let profile_id = or_text(field(profile, "id"));
    let workspace_id = or_text(field(profile, "workspace_id"));
    let tx = kassiber_store::write_savepoint(conn).map_err(crate::database_error)?;
    let wallet_id = or_text(field(row, "id"));
    let py_config = sources::config(row);
    let json_config = config(row)?;
    let scope = btcpay::Scope {
        workspace_id: &workspace_id,
        profile_id: &profile_id,
        wallet: row,
    };
    let route_kind = match route {
        Local::Node(..) => RouteKind::Node,
        _ => RouteKind::Local,
    };
    let mut outcome = match route {
        Local::File(file, format) => {
            let mut outcome = sources::import_file(&tx, &profile_id, row, file, format)?;
            match btcpay {
                Some(ctx) => {
                    if let Some(enriched) =
                        btcpay::enrichment(&tx, ctx, &scope, &json_config, &Value::Null)?
                    {
                        kassiber_envelope::set(&mut outcome, "btcpay_provenance", enriched);
                    }
                }
                None if sources::has_btcpay_provenance(&py_config) => {
                    return Err(wallet::fallback("btcpay_provenance"));
                }
                None => {}
            }
            outcome
        }
        Local::Node(resolved, ctx) => {
            // `_wrap_sync_progress_for_wallet`, then the node's sync hook.
            crate::progress::for_wallet(field(row, "label"), || {
                crate::progress::emit(dict([("phase", s("backend_fetch"))]));
                let backend = resolved.as_ref().map_err(Clone::clone)?;
                if or_text(field(row, "kind")) == "lnd" {
                    node::sync_lnd(&tx, ctx, profile, row, backend, clock)
                } else {
                    node::sync_core_lightning(&tx, ctx, profile, row, backend, clock)
                }
            })?
        }
        Local::Btcpay => {
            let ctx = btcpay.ok_or_else(|| wallet::fallback("wallet_route"))?;
            // `wallets sync` reads no checkpoint for local sources.
            btcpay::sync_wallet(
                &tx,
                ctx,
                &scope,
                &json_config,
                btcpay::PAGE_SIZE,
                &Value::Null,
            )?
        }
    };
    // Python merges Bull Bitcoin export routes for every route but LND's.
    let lnd = matches!(route_kind, RouteKind::Node) && or_text(field(row, "kind")) == "lnd";
    if !lnd {
        if let Some(enriched) = sources::bull_enrichment(&tx, &profile_id, &wallet_id, &py_config)?
        {
            kassiber_envelope::set(&mut outcome, "bullbitcoin_wallet_exports", enriched);
        }
    }
    let label = or_text(field(row, "label"));
    // A node route reports its own status; only a synced one is stamped.
    let node_route = matches!(route_kind, RouteKind::Node);
    let result = if node_route {
        let mut result = dict([("wallet", field(row, "label").clone())]);
        if let Value::Dict(pairs) = outcome {
            for (key, value) in pairs {
                if let kassiber_pyjson::Key::Str(name) = &key {
                    kassiber_envelope::set(&mut result, &py_str(&Value::Str(name.clone())), value);
                }
            }
        }
        result
    } else {
        sources::synced(field(row, "label"), outcome)
    };
    if node_route && kassiber_envelope::get(&result, "status") != Some(&s("synced")) {
        tx.commit().map_err(crate::database_error)?;
        return Ok(vec![result]);
    }
    let checkpoint = kassiber_envelope::get(&result, "freshness_checkpoint")
        .filter(|v| matches!(v, Value::Dict(_)))
        .map(|v| {
            serde_json::from_str::<Json>(
                &kassiber_pyjson::dumps(v, &kassiber_pyjson::Options::new()).unwrap_or_default(),
            )
            .map_err(|_| AppError::new("internal_error", "Sync checkpoint cannot be encoded"))
        })
        .transpose()?;
    let synced_at = clock.now();
    let updated_at = if checkpoint.is_some() {
        clock.now()
    } else {
        String::new()
    };
    let flag =
        |key: &str| kassiber_envelope::get(&result, key).is_some_and(kassiber_envelope::truthy);
    crate::publication::mark_synced(
        &tx,
        &crate::publication::Synced {
            profile_id: &profile_id,
            wallet_id: &wallet_id,
            wallet_label: &label,
            checkpoint: checkpoint.as_ref(),
            partial_success: flag("partial_success"),
            degraded_reason: None,
            blocking_reports: flag("blocking_reports"),
            timestamp: &synced_at,
            updated_at: &updated_at,
        },
    )?;
    tx.commit().map_err(crate::database_error)?;
    Ok(vec![result])
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

pub mod btcpay;
mod node;
pub(crate) mod sources;
#[cfg(test)]
mod tests;

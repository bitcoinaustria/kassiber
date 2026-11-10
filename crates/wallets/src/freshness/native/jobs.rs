//! The freshness jobs beside on-chain history (`daemon_freshness._freshness_handlers`):
//! BTCPay wallet sources, BTCPay provenance (wallet and account routes) and
//! market-rate coverage. Each loads its source at run time, as Python does,
//! and reaches the network only under its source class's permission.
use super::*;
use std::cell::RefCell;

/// `require_network_permission`: an automatic trigger needs its feature and
/// the source class; an explicit run checks only the market-rate class.
fn permitted(conn: &Connection, proof: &Consent, profile: &str, class: &str) -> Result<()> {
    if !proof.report_read && class != "market_rates" {
        return Ok(());
    }
    let policy =
        super::super::Queue::new(conn, &mut wallet::SystemSources::default()).policy(profile)?;
    let trigger = !proof.report_read || truthy(field(&policy, "report_read_sync"));
    if !trigger || !truthy(field(field(&policy, "source_classes"), class)) {
        return Err(AppError::new(
            "cancelled",
            "Freshness network permission was disabled",
        ));
    }
    Ok(())
}

/// `_load_freshness_profile`.
fn job_profile(conn: &Connection, job: &Value) -> Result<Value> {
    first(
        conn,
        "SELECT * FROM profiles WHERE id=?",
        [or_text(field(job, "profile_id"))],
    )?
    .ok_or_else(|| AppError::new("not_found", "Freshness job profile was not found"))
}

/// `_load_freshness_profile_wallet`.
fn job_wallet(conn: &Connection, job: &Value) -> Result<(Value, Value)> {
    let profile = job_profile(conn, job)?;
    let wallet_id = match field(field(job, "payload"), "wallet_id") {
        Value::Str(id) => kassiber_envelope::py_str(&Value::Str(id.clone())),
        _ => String::new(),
    };
    let wallet = first(
        conn,
        "SELECT * FROM wallets WHERE profile_id=? AND id=?",
        [or_text(field(job, "profile_id")), wallet_id],
    )?
    .ok_or_else(|| AppError::new("not_found", "Freshness job wallet was not found"))?;
    Ok((profile, wallet))
}

/// `_job_force_full`.
fn force_full(job: &Value) -> bool {
    truthy(field(field(job, "payload"), "force_full"))
}

/// `_source_checkpoint_for_job`: the source's stored checkpoint, reduced by
/// `full_scan_checkpoint` for a forced job.
fn job_checkpoint(
    conn: &Connection,
    sources: &mut dyn wallet::Sources,
    job: &Value,
) -> Result<Value> {
    let state = super::super::Queue::new(conn, sources).state(
        &or_text(field(job, "profile_id")),
        &or_text(field(job, "source_key")),
    )?;
    let checkpoint = match state.as_ref().map(|state| field(state, "checkpoint")) {
        Some(value @ Value::Dict(_)) => value.clone(),
        _ => dict::<&str>([]),
    };
    if !force_full(job) {
        return Ok(checkpoint);
    }
    let encoded = kassiber_pyjson::dumps(&checkpoint, &Options::new())
        .map_err(|_| AppError::new("internal_error", "Invalid freshness checkpoint"))?;
    let reduced = crate::acquisition::full_scan_checkpoint(
        &serde_json::from_str(&encoded)
            .map_err(|_| AppError::new("internal_error", "Invalid freshness checkpoint"))?,
    );
    kassiber_pytext::loads(&reduced.to_string())
        .map_err(|_| AppError::new("internal_error", "Invalid freshness checkpoint"))
}

/// `{"force_full": True, **outcome}` for a forced job.
fn forced(job: &Value, outcome: Value) -> Value {
    if !force_full(job) {
        return outcome;
    }
    let mut result = dict([("force_full", Value::Bool(true))]);
    if let Value::Dict(pairs) = outcome {
        for (key, value) in pairs {
            put(&mut result, &super::super::key_text(&key), value);
        }
    }
    result
}

/// `{**head, **outcome}`: the outcome's keys update the head's in place.
fn spread(mut head: Value, outcome: Value) -> Value {
    if let Value::Dict(pairs) = outcome {
        for (key, value) in pairs {
            put(&mut head, &super::super::key_text(&key), value);
        }
    }
    head
}

/// The wallet's stored configuration (`json.loads(wallet["config_json"] or "{}")`).
fn wallet_json(wallet: &Value) -> Result<Json> {
    let raw = or_text(field(wallet, "config_json"));
    serde_json::from_str(if raw.is_empty() { "{}" } else { &raw })
        .map_err(|_| AppError::new("internal_error", "Invalid wallet configuration JSON"))
}

/// `_mark_daemon_wallet_synced`: the wallet's stored config, stamped (a sync
/// may have changed it: a Bark payment ledger switches to the wallet import).
fn mark_synced(conn: &Connection, sources: &mut dyn wallet::Sources, wallet: &Value) -> Result<()> {
    // Read the stored config under the writer lock, so no other writer's
    // change can land between the read and the stamp.
    let owned = conn.is_autocommit();
    execution::begin(conn)?;
    let stamped = stamp_synced(conn, sources, wallet);
    if stamped.is_err() && owned {
        let _ = conn.execute_batch("ROLLBACK");
        return stamped;
    }
    execution::commit(conn)
}

fn stamp_synced(
    conn: &Connection,
    sources: &mut dyn wallet::Sources,
    wallet: &Value,
) -> Result<()> {
    let stored: Option<Option<String>> = conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id = ?",
            [or_text(field(wallet, "id"))],
            |row| row.get(0),
        )
        .optional()
        .map_err(crate::database_error)?;
    let raw = match stored {
        Some(config) => config.unwrap_or_default(),
        None => or_text(field(wallet, "config_json")),
    };
    let mut config = kassiber_pytext::loads(if raw.is_empty() { "{}" } else { &raw })
        .map_err(|_| AppError::new("internal_error", "Invalid wallet configuration JSON"))?;
    put(&mut config, "last_synced_at", s(sources.now()));
    let config = kassiber_pyjson::dumps(&config, &Options::new().sort_keys(true))
        .map_err(|_| AppError::new("internal_error", "Invalid wallet configuration JSON"))?;
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id=?",
        params![config, or_text(field(wallet, "id"))],
    )
    .map_err(crate::database_error)?;
    Ok(())
}

impl<B: FnMut(&Connection, &str) -> Result<Json>> Native<'_, B> {
    fn checked(&self, conn: &Connection, job: &Value) -> Result<()> {
        check(
            conn,
            &self.context,
            self.proof,
            self.cancel,
            self.deadline,
            &or_text(field(job, "id")),
        )
    }

    /// `btcpay_wallet`: the wallet's BTCPay source under the
    /// `btcpay_wallet` class, then the daemon's synced stamp.
    pub(super) fn btcpay_wallet(
        &mut self,
        conn: &mut Connection,
        job: &Value,
        progress: &mut Progress<'_>,
    ) -> Result<Value> {
        let (profile, wallet) = job_wallet(conn, job)?;
        let checkpoint = job_checkpoint(conn, progress.sources, job)?;
        let label = field(&wallet, "label").clone();
        let profile_id = or_text(field(&profile, "id"));
        progress.emit(
            conn,
            &dict([("phase", s("discovery")), ("wallet", label.clone())]),
        )?;
        self.checked(conn, job)?;
        progress.emit(
            conn,
            &dict([("phase", s("backend_fetch")), ("wallet", label.clone())]),
        )?;
        permitted(conn, self.proof, &profile_id, "btcpay_wallet")?;
        let config = wallet_json(&wallet)?;
        let workspace_id = or_text(field(&profile, "workspace_id"));
        let outcome = {
            let events = RefCell::new(&mut *progress);
            let emit = |event: &Value| events.borrow_mut().emit(conn, event);
            let ctx = crate::sync::btcpay::Context {
                backends: self.backends,
                gate: self.gate,
                version: &self.version,
                book_id: &self.book_id,
                progress: Some(&emit),
            };
            let (synced, events) = crate::progress::collect(|| {
                crate::sync::btcpay::sync_wallet(
                    conn,
                    &ctx,
                    &crate::sync::btcpay::Scope {
                        workspace_id: &workspace_id,
                        profile_id: &profile_id,
                        wallet: &wallet,
                    },
                    &config,
                    crate::sync::btcpay::PAGE_SIZE,
                    &checkpoint,
                )
            });
            // Import progress the wallet's savepoint held back, in order.
            for event in &events {
                emit(event)?;
            }
            synced?
        };
        self.checked(conn, job)?;
        progress.emit(
            conn,
            &dict([("phase", s("import")), ("wallet", label.clone())]),
        )?;
        mark_synced(conn, progress.sources, &wallet)?;
        Ok(spread(
            dict([("wallet", label), ("status", s("synced"))]),
            forced(job, outcome),
        ))
    }

    /// `btcpay_provenance`: an account route refreshes its store's
    /// provenance; a wallet route enriches the wallet.
    pub(super) fn btcpay_provenance(
        &mut self,
        conn: &mut Connection,
        job: &Value,
        progress: &mut Progress<'_>,
    ) -> Result<Value> {
        // `str_or_none(payload.get("account_route_id"))`.
        let route_id = match field(field(job, "payload"), "account_route_id") {
            Value::Null => String::new(),
            value => kassiber_pytext::strip(&kassiber_envelope::py_str(value)).to_owned(),
        };
        if !route_id.is_empty() {
            return self.btcpay_account_route(conn, job, progress, &route_id);
        }
        let (profile, wallet) = job_wallet(conn, job)?;
        let checkpoint = job_checkpoint(conn, progress.sources, job)?;
        let label = field(&wallet, "label").clone();
        let profile_id = or_text(field(&profile, "id"));
        progress.emit(
            conn,
            &dict([("phase", s("backend_fetch")), ("wallet", label.clone())]),
        )?;
        self.checked(conn, job)?;
        permitted(conn, self.proof, &profile_id, "btcpay_provenance")?;
        let config = wallet_json(&wallet)?;
        let workspace_id = or_text(field(&profile, "workspace_id"));
        let outcome = {
            let events = RefCell::new(&mut *progress);
            let emit = |event: &Value| events.borrow_mut().emit(conn, event);
            let ctx = crate::sync::btcpay::Context {
                backends: self.backends,
                gate: self.gate,
                version: &self.version,
                book_id: &self.book_id,
                progress: Some(&emit),
            };
            crate::sync::btcpay::enrichment_or_empty(
                conn,
                &ctx,
                &crate::sync::btcpay::Scope {
                    workspace_id: &workspace_id,
                    profile_id: &profile_id,
                    wallet: &wallet,
                },
                &config,
                &checkpoint,
            )?
        };
        self.checked(conn, job)?;
        progress.emit(
            conn,
            &dict([("phase", s("decode_enrich")), ("wallet", label.clone())]),
        )?;
        Ok(spread(
            dict([("wallet", label), ("status", s("synced"))]),
            forced(job, outcome),
        ))
    }

    fn btcpay_account_route(
        &mut self,
        conn: &mut Connection,
        job: &Value,
        progress: &mut Progress<'_>,
        route_id: &str,
    ) -> Result<Value> {
        let profile = job_profile(conn, job)?;
        let profile_id = or_text(field(&profile, "id"));
        let route = first(
            conn,
            "SELECT * FROM btcpay_account_routes WHERE profile_id = ? AND id = ?",
            [profile_id.as_str(), route_id],
        )?
        .ok_or_else(|| {
            AppError::new(
                "not_found",
                "Freshness job BTCPay account route was not found",
            )
        })?;
        let checkpoint = job_checkpoint(conn, progress.sources, job)?;
        let store = field(&route, "store_id").clone();
        let backend = field(&route, "backend_name").clone();
        progress.emit(
            conn,
            &dict([("phase", s("backend_fetch")), ("store_id", store.clone())]),
        )?;
        self.checked(conn, job)?;
        permitted(conn, self.proof, &profile_id, "btcpay_provenance")?;
        let workspace_id = or_text(field(&profile, "workspace_id"));
        let store_id = match &store {
            Value::Str(text) => text.clone(),
            other => kassiber_envelope::py_str(other).as_str().into(),
        };
        let backend_name = or_text(&backend);
        let outcome = {
            let events = RefCell::new(&mut *progress);
            let emit = |event: &Value| events.borrow_mut().emit(conn, event);
            let env = crate::btcpay::connect::Env {
                conn,
                backends: self.backends,
                gate: self.gate.clone(),
                version: &self.version,
                book_id: self.book_id.clone(),
                progress: Some(&emit),
            };
            let args = dict([
                ("account_route_id", s(route_id)),
                ("backend", backend.clone()),
                ("store_id", store.clone()),
            ]);
            crate::btcpay::provenance::sync(
                &env,
                crate::btcpay::provenance::Sync {
                    workspace_id: &workspace_id,
                    profile_id: &profile_id,
                    backend: &backend_name,
                    store_id: &store_id,
                    page_size: crate::sync::btcpay::PAGE_SIZE,
                    include_payouts: true,
                    suggest: true,
                    kind: "freshness.btcpay_provenance",
                    args: &args,
                    checkpoint: &checkpoint,
                },
            )?
        };
        self.checked(conn, job)?;
        progress.emit(
            conn,
            &dict([("phase", s("decode_enrich")), ("store_id", store.clone())]),
        )?;
        Ok(spread(
            dict([
                ("status", s("synced")),
                ("account_route_id", s(route_id)),
                ("backend", backend),
                ("store_id", store),
            ]),
            forced(job, outcome),
        ))
    }

    /// `market_rates`: the bundled hourly seed always (an offline,
    /// idempotent cache fill), live provider refreshes only with the
    /// `market_rates` class enabled.
    pub(super) fn market_rates(
        &mut self,
        conn: &mut Connection,
        job: &Value,
        progress: &mut Progress<'_>,
    ) -> Result<Value> {
        progress.emit(conn, &dict([("phase", s("rate_coverage"))]))?;
        self.checked(conn, job)?;
        execution::begin(conn)?;
        let seeded = kassiber_rates::ensure_bundled_seed(conn, None, true);
        match seeded {
            Ok(_) => execution::commit(conn)?,
            Err(_) => {
                let _ = conn.execute_batch("ROLLBACK");
            }
        }
        let summary = seeded?;
        self.checked(conn, job)?;
        let provider = kassiber_rates::market_provider(conn)?;
        let bundled = dict([
            ("source", s("kraken-csv")),
            ("path", s(kassiber_rates::bundled_seed_path(true))),
            ("summary", summary),
        ]);
        let profile = or_text(field(job, "profile_id"));
        let policy = super::super::Queue::new(conn, progress.sources).policy(&profile)?;
        if !truthy(field(field(&policy, "source_classes"), "market_rates")) {
            return Ok(dict([
                ("status", s("synced")),
                ("provider", s(provider.as_str())),
                ("live_refresh", Value::Bool(false)),
                ("skipped_reason", s("market_rates_disabled")),
                ("bundled_seed", bundled),
                ("latest", Value::List(vec![])),
                ("sync", Value::List(vec![])),
            ]));
        }
        permitted(conn, self.proof, &profile, "market_rates")?;
        let providers = kassiber_rates::Providers::new(self.gate.clone(), &self.version);
        let consent = kassiber_rates::FetchConsent::explicit(
            conn,
            kassiber_rates::FetchAction::Latest,
            &provider,
            None,
        )?;
        let latest = providers.latest(
            conn,
            None,
            Some(&provider),
            Some(&consent),
            kassiber_rates::FetchAction::Latest,
        )?;
        self.checked(conn, job)?;
        permitted(conn, self.proof, &profile, "market_rates")?;
        let sync = if ["coinbase-exchange", "mempool"].contains(&provider.as_str()) {
            let consent = kassiber_rates::FetchConsent::explicit(
                conn,
                kassiber_rates::FetchAction::Sync,
                &provider,
                None,
            )?;
            providers.sync(
                conn,
                &dict([
                    ("source", s(provider.as_str())),
                    ("warm_cache_when_idle", Value::Bool(false)),
                ]),
                Some(&consent),
                kassiber_rates::FetchAction::Sync,
            )?
        } else {
            Value::List(vec![])
        };
        Ok(dict([
            ("status", s("synced")),
            ("provider", s(provider.as_str())),
            ("live_refresh", Value::Bool(true)),
            ("bundled_seed", bundled),
            ("latest", latest),
            ("sync", sync),
        ]))
    }
}

//! BTCPay wallet sync routes: wallet history (`_sync_btcpay_wallet`), the
//! payment ledger (`_sync_btcpay_payment_ledger`) and provenance enrichment
//! of a mapped wallet (`enrich_wallet_from_btcpay_provenance`). Every
//! Greenfield read runs under a grant bound to this explicit sync.
use crate::btcpay::connect::{deadline, Env};
use crate::btcpay::{bark, ledger, provenance};
use crate::file_import::{self, RecordImport};
use crate::publication::row::{field, or_text};
use kassiber_chain::btcpay::permissions::{MODIFY_STORE_SETTINGS, VIEW_WALLET};
use kassiber_chain::btcpay::{fetch, payment_methods as methods, url};
use kassiber_envelope::{dict, get, s, set, truthy, AppError};
use kassiber_net::Gate;
use kassiber_pyjson::{Key, PyStr, Value};
use kassiber_setup::backends::Config;
use kassiber_store::rusqlite::Connection;
use serde_json::Value as Json;

type Result<T> = std::result::Result<T, AppError>;

pub const PAGE_SIZE: i64 = 100;

/// What the BTCPay routes need from the open book beyond the connection.
pub struct Context<'a> {
    pub backends: &'a Config,
    pub gate: &'a Gate,
    pub version: &'a str,
    pub book_id: &'a str,
    /// `emit_sync_progress` for page reads; `None` outside freshness jobs.
    pub progress: Option<kassiber_chain::btcpay::fetch::Progress<'a>>,
}

impl Context<'_> {
    pub(crate) fn env<'c>(&'c self, conn: &'c Connection) -> Env<'c> {
        Env {
            conn,
            backends: self.backends,
            gate: self.gate.clone(),
            version: self.version,
            book_id: self.book_id.to_owned(),
            progress: self.progress,
        }
    }
}

/// The wallet and book this route serves.
pub(crate) struct Scope<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
    pub wallet: &'a Value,
}

impl Scope<'_> {
    fn wallet_id(&self) -> String {
        or_text(field(self.wallet, "id"))
    }

    /// The grant binding: this sync, this wallet, this store.
    fn binding(&self, backend: &str, store: &str) -> Value {
        dict([
            ("wallet", s(self.wallet_id())),
            ("backend", s(backend)),
            ("store_id", s(store)),
        ])
    }
}

fn text(value: &Json) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// `resolve_backend` plus the BTCPay kind check.
fn btcpay_backend(env: &Env<'_>, name: &str, hint: &str) -> Result<(Json, String)> {
    let backend = env.backends.resolve_configured(env.conn, Some(name))?;
    let kind = provenance::backend_kind(&text(&backend["kind"]));
    if kind != "btcpay" {
        return Err(AppError::new(
            "validation",
            format!(
                "Backend '{}' has kind '{}', expected 'btcpay'",
                text(&backend["name"]),
                text(&backend["kind"])
            ),
        )
        .with_hint(hint));
    }
    Ok((backend, kind))
}

const CREATE_HINT: &str = "Create a BTCPay backend with `kassiber backends create --kind btcpay --url <server> --token-stdin` or `--token-fd FD`.";

/// `_btcpay_error_summary`.
fn error_summary(error: &AppError) -> Value {
    let missing = match &error.details {
        Some(details @ Value::Dict(_)) => get(details, "missing_permission")
            .cloned()
            .unwrap_or(Value::Null),
        _ => Value::Null,
    };
    dict([
        ("status", s("unavailable")),
        ("code", s(error.code.as_str())),
        (
            "message",
            s(provenance::redact_backend_text(&error.message)),
        ),
        (
            "hint",
            s(error
                .hint
                .as_deref()
                .map(provenance::redact_backend_text)
                .unwrap_or_default()),
        ),
        ("missing_permission", missing),
    ])
}

/// `_sync_store_provenance`: a failure becomes the outcome, never the sync's.
fn store_provenance(
    env: &Env<'_>,
    scope: &Scope<'_>,
    backend: &str,
    store_id: &PyStr,
    suggest: bool,
    checkpoint: &Value,
) -> Result<(Value, Value)> {
    let args = scope.binding(backend, &url::from_py(store_id));
    match provenance::sync(
        env,
        provenance::Sync {
            workspace_id: scope.workspace_id,
            profile_id: scope.profile_id,
            backend,
            store_id,
            page_size: PAGE_SIZE,
            include_payouts: true,
            suggest,
            kind: "wallets.sync",
            args: &args,
            checkpoint,
        },
    ) {
        Ok(mut outcome) => {
            let checkpoint =
                pop(&mut outcome, "freshness_checkpoint").unwrap_or_else(|| checkpoint.clone());
            set(&mut outcome, "status", s("synced"));
            Ok((outcome, checkpoint))
        }
        Err(error) if error.code == "internal_error" => Err(error),
        Err(error) => {
            let mut summary = error_summary(&error);
            if !["auth_error", "not_found"].contains(&error.code.as_str()) {
                set(&mut summary, "status", s("error"));
                set(&mut summary, "retryable", Value::Bool(error.retryable));
            }
            Ok((summary, checkpoint.clone()))
        }
    }
}

fn pop(value: &mut Value, key: &str) -> Option<Value> {
    let Value::Dict(pairs) = value else {
        return None;
    };
    let index = pairs.iter().position(|(k, _)| *k == Key::from(key))?;
    Some(pairs.remove(index).1)
}

/// `fetch_btcpay_records` under this sync's grant: rows and page metadata.
fn wallet_history(
    env: &Env<'_>,
    scope: &Scope<'_>,
    backend: &Json,
    store_id: &PyStr,
    method: &str,
    page_size: i64,
    checkpoint: &Value,
) -> Result<(Vec<Value>, Value)> {
    let name = text(&backend["name"]);
    let (client, grant) = env.client(
        "wallets.sync",
        &scope.binding(&name, &url::from_py(store_id)),
        backend,
        None,
    )?;
    let reads = fetch::Reads {
        client: &client,
        consent: &grant.0,
        deadline: deadline(),
        progress: env.progress,
    };
    fetch::wallet_records(reads, store_id, method, page_size, checkpoint)
}

fn suggestions(conn: &Connection, scope: &Scope<'_>) -> Result<Value> {
    let suggested = kassiber_transactions::commercial::execute(
        conn,
        &kassiber_transactions::Scope {
            workspace_id: scope.workspace_id,
            profile_id: scope.profile_id,
        },
        &kassiber_transactions::commercial::Command::Suggest { limit: 500 },
    )?;
    let total = match get(&suggested, "suggestions") {
        Some(Value::List(items)) => items.len(),
        _ => 0,
    };
    Ok(dict([
        (
            "created",
            get(&suggested, "created").cloned().unwrap_or(Value::Null),
        ),
        ("total", Value::Int(total.into())),
    ]))
}

fn page_field(meta: &Value, key: &str) -> Value {
    get(meta, key).cloned().unwrap_or(Value::Null)
}

/// `btcpay_meta.get(key, default)`.
fn page_field_or(meta: &Value, key: &str, default: Value) -> Value {
    get(meta, key).cloned().unwrap_or(default)
}

/// `_sync_btcpay_wallet` for a wallet whose `sync_source` is BTCPay.
pub(crate) fn sync_wallet(
    conn: &Connection,
    ctx: &Context<'_>,
    scope: &Scope<'_>,
    config: &Json,
    page_size: i64,
    checkpoint: &Value,
) -> Result<Value> {
    let env = ctx.env(conn);
    // The source's previous `freshness_checkpoint` (`wallet["_freshness_checkpoint"]`).
    let checkpoint = match checkpoint {
        value @ Value::Dict(_) => value.clone(),
        _ => dict::<&str>([]),
    };
    let previous_provenance = match get(&checkpoint, "invoice_provenance") {
        Some(value @ Value::Dict(_)) => value.clone(),
        _ => dict::<&str>([]),
    };
    let mut btcpay = config.clone();
    crate::config::normalize_btcpay(&mut btcpay)?;
    if !crate::descriptors::truthy(&btcpay["sync_source"]) {
        return Err(AppError::new(
            "validation",
            format!(
                "Wallet '{}' does not have BTCPay sync configured",
                or_text(field(scope.wallet, "label"))
            ),
        )
        .with_hint("Run `kassiber wallets sync-btcpay --wallet ... --backend ... --store-id ...` first, or store the config with `wallets update`."));
    }
    let (backend, kind) = btcpay_backend(&env, &text(&btcpay["backend"]), CREATE_HINT)?;
    let name = text(&backend["name"]);
    let store = text(&btcpay["store_id"]);
    let store_id = PyStr::from(store.as_str());
    if ledger::is_ledger(config) {
        return payment_ledger(
            &env,
            scope,
            &backend,
            config,
            &store_id,
            page_size,
            checkpoint,
            &previous_provenance,
        );
    }
    let method = text(&btcpay["payment_method_id"]);
    if methods::canonical(&method) == methods::BARK_PAYMENT_METHOD_ID {
        return bark_wallet(
            &env, scope, &backend, &store_id, page_size, checkpoint, None,
        );
    }
    // Payouts first: they tell the wallet import how much of a send was fee.
    let (mut provenance, provenance_checkpoint) =
        store_provenance(&env, scope, &name, &store_id, false, &previous_provenance)?;
    let (rows, meta) = wallet_history(
        &env,
        scope,
        &backend,
        &store_id,
        &method,
        page_size,
        &checkpoint,
    )?;
    // The store's provenance is already written; the import stands alone.
    nested(conn, || {
        let mut records = file_import::btcpay_rows(rows)?;
        let wallet_id = scope.wallet_id();
        let totals = ledger::payout_totals(
            conn,
            scope.profile_id,
            &methods::asset(&method).unwrap_or_else(|| "BTC".into()),
        )?;
        let backfilled = ledger::backfill_payout_fees(conn, scope.profile_id, &wallet_id, &totals)?;
        let stored = ledger::outbound_fees(conn, scope.profile_id, &wallet_id)?;
        let separated = ledger::apply_payout_fees(&mut records, &totals, &stored)?;
        let mut outcome = file_import::import_records(
            conn,
            &RecordImport {
                profile_id: scope.profile_id,
                wallet_id: &wallet_id,
                label: &format!("btcpay:{name}:{store}"),
                match_existing_only: false,
                report_updates: false,
                btcpay: true,
                settlement_ids: &[],
            },
            &records,
        )?;
        if backfilled > 0 && !get(&outcome, "journal_invalidated").is_some_and(truthy) {
            crate::invalidate_journals(conn, scope.profile_id)?;
            set(&mut outcome, "journal_invalidated", Value::Bool(true));
        }
        if get(&provenance, "status") == Some(&s("synced")) {
            set(&mut provenance, "suggestions", suggestions(conn, scope)?);
        }
        let mut fields: Vec<(&str, Value)> = vec![
            ("backend", s(name.as_str())),
            ("backend_kind", s(kind.as_str())),
            (
                "backend_url",
                s(url::redact_backend_url(&text(&backend["url"]))),
            ),
            ("store_id", s(store.as_str())),
            ("payment_method_id", s(method.as_str())),
            ("page_size", Value::Int(page_size.into())),
            ("fetched", Value::Int(records.len().into())),
            (
                "payout_fees",
                dict([
                    ("separated", Value::Int(separated.into())),
                    ("backfilled", Value::Int(backfilled.into())),
                ]),
            ),
        ];
        let mut checkpoint = checkpoint.clone();
        if truthy(&meta) {
            fields.extend([
                (
                    "pages_fetched",
                    page_field_or(&meta, "pages_fetched", Value::Int(0.into())),
                ),
                (
                    "stopped_by_known_page",
                    Value::Bool(truthy(&page_field(&meta, "stopped_by_known_page"))),
                ),
                ("stop_reason", page_field(&meta, "stop_reason")),
                ("deep_audit", page_field(&meta, "deep_audit")),
            ]);
            for (key, value) in [
                (
                    "backend",
                    dict([("name", s(name.as_str())), ("kind", s(kind.as_str()))]),
                ),
                (
                    "btcpay_pages",
                    page_field_or(&meta, "btcpay_pages", dict::<&str>([])),
                ),
                (
                    "btcpay_pagination",
                    page_field_or(&meta, "btcpay_pagination", dict::<&str>([])),
                ),
                ("store_id", s(store.as_str())),
                ("payment_method_id", s(method.as_str())),
            ] {
                set(&mut checkpoint, key, value);
            }
        }
        for (key, value) in fields {
            set(&mut outcome, key, value);
        }
        let keep = truthy(&meta) || truthy(&provenance_checkpoint);
        set(&mut checkpoint, "invoice_provenance", provenance_checkpoint);
        set(&mut outcome, "invoice_provenance", provenance);
        if keep {
            set(&mut outcome, "freshness_checkpoint", checkpoint);
        }
        Ok(outcome)
    })
}

/// `_sync_bark_wallet`: a store's whole Bark wallet (Bark plugin 0.10 and
/// later). Provenance first, so rows paying an invoice book under the
/// payment's provenance key; `migrated` is the `BTC-ARK` payment ledger's
/// config when this sync switches it to the wallet.
fn bark_wallet(
    env: &Env<'_>,
    scope: &Scope<'_>,
    backend: &Json,
    store_id: &PyStr,
    page_size: i64,
    checkpoint: Value,
    migrated: Option<&Json>,
) -> Result<Value> {
    let name = text(&backend["name"]);
    let store = url::from_py(store_id);
    let previous_provenance = match get(&checkpoint, "invoice_provenance") {
        Some(value @ Value::Dict(_)) => value.clone(),
        _ => dict::<&str>([]),
    };
    let (mut provenance, provenance_checkpoint) =
        store_provenance(env, scope, &name, store_id, false, &previous_provenance)?;
    let (client, grant) =
        env.client("wallets.sync", &scope.binding(&name, &store), backend, None)?;
    let reads = fetch::Reads {
        client: &client,
        consent: &grant.0,
        deadline: deadline(),
        progress: env.progress,
    };
    let from = if migrated.is_some() {
        dict::<&str>([])
    } else {
        checkpoint.clone()
    };
    let (rows, meta) = fetch::bark_wallet_rows(reads, store_id, page_size, &from, &mut |row| {
        bark::records(row, &store).map(|_| ())
    })?;
    let mut records = Vec::new();
    for row in &rows {
        records.extend(bark::records(row, &store)?);
    }
    let label = or_text(field(scope.wallet, "label"));
    if migrated.is_some() && get(&provenance, "status") != Some(&s("synced")) {
        // Without current provenance the booked payments cannot be matched.
        let code = get(&provenance, "code")
            .filter(|v| truthy(v))
            .map(kassiber_envelope::py_str);
        let hint = get(&provenance, "hint")
            .filter(|v| truthy(v))
            .map(kassiber_envelope::py_str);
        let mut error = AppError::new(
            code.as_deref().unwrap_or("unavailable"),
            format!("BTCPay invoices for store {store} cannot be read, so '{label}' cannot switch to the Bark wallet yet"),
        )
        .with_hint(hint.as_deref().unwrap_or("Sync again once BTCPay answers; the payment ledger stays as it is."))
        .with_details(dict([("store_id", s(store.as_str()))]));
        error.retryable = get(&provenance, "retryable").is_some_and(truthy);
        return Err(error);
    }
    let conn = env.conn;
    let wallet_id = scope.wallet_id();
    bark::require_book_network(conn, scope.profile_id, &store)?;
    // Freshness progress commits on this connection: the import stays quiet
    // inside its savepoint, as Python's does.
    nested(conn, || {
        crate::progress::silenced(|| {
            let identities =
                bark::assign_identities(conn, scope.profile_id, &wallet_id, &store, &mut records)?;
            let mut kept = 0;
            if let Some(config) = migrated {
                kept = bark::migrate_payment_ledger(
                    conn,
                    scope.profile_id,
                    &wallet_id,
                    &label,
                    &records,
                )?;
                conn.execute(
                    "UPDATE wallets SET config_json = ? WHERE id = ?",
                    [
                        crate::config::stored_json(&bark::bark_config(config)),
                        wallet_id.clone(),
                    ],
                )
                .map_err(crate::database_error)?;
            }
            let moved = bark::adopt_lightning_ledger_rows(
                conn,
                scope.profile_id,
                &wallet_id,
                &name,
                &store,
                &records,
            )?;
            let import_rows =
                file_import::record_rows(records.iter().map(|r| r.record.clone()).collect());
            let mut outcome = file_import::import_records(
                conn,
                &RecordImport {
                    profile_id: scope.profile_id,
                    wallet_id: &wallet_id,
                    label: &format!("btcpay-bark:{name}:{store}"),
                    match_existing_only: false,
                    report_updates: false,
                    btcpay: true,
                    settlement_ids: &[],
                },
                &import_rows,
            )?;
            if (kept > 0 || moved > 0 || identities.rekeyed > 0)
                && !get(&outcome, "journal_invalidated").is_some_and(truthy)
            {
                crate::invalidate_journals(conn, scope.profile_id)?;
                set(&mut outcome, "journal_invalidated", Value::Bool(true));
            }
            if get(&provenance, "status") == Some(&s("synced")) {
                set(&mut provenance, "suggestions", suggestions(conn, scope)?);
            }
            let int = |n: i64| Value::Int(n.into());
            let fields: Vec<(&str, Value)> = vec![
                ("backend", s(name.as_str())),
                ("backend_kind", s("btcpay")),
                (
                    "backend_url",
                    s(url::redact_backend_url(&text(&backend["url"]))),
                ),
                ("store_id", s(store.as_str())),
                ("payment_method_id", s(methods::BARK_PAYMENT_METHOD_ID)),
                ("page_size", int(page_size)),
                ("fetched", int(records.len() as i64)),
                (
                    "bark",
                    dict([
                        ("linked_to_payments", int(identities.linked)),
                        ("ambiguous_payments", int(identities.ambiguous)),
                        ("identities_rekeyed", int(identities.rekeyed)),
                        ("lightning_ledger_rows_moved", int(moved)),
                        (
                            "migrated_from_payment_ledger",
                            Value::Bool(migrated.is_some()),
                        ),
                        ("ledger_rows_kept", int(kept)),
                    ]),
                ),
                (
                    "pages_fetched",
                    page_field_or(&meta, "pages_fetched", int(0)),
                ),
                (
                    "stopped_by_known_page",
                    Value::Bool(truthy(&page_field(&meta, "stopped_by_known_page"))),
                ),
                ("stop_reason", page_field(&meta, "stop_reason")),
                ("deep_audit", page_field(&meta, "deep_audit")),
            ];
            for (key, value) in fields {
                set(&mut outcome, key, value);
            }
            let mut next = checkpoint.clone();
            pop(&mut next, "btcpay_pages");
            pop(&mut next, "btcpay_pagination");
            for (key, value) in [
                (
                    "backend",
                    dict([("name", s(name.as_str())), ("kind", s("btcpay"))]),
                ),
                (
                    "btcpay_pages",
                    page_field_or(&meta, "btcpay_pages", dict::<&str>([])),
                ),
                (
                    "btcpay_pagination",
                    page_field_or(&meta, "btcpay_pagination", dict::<&str>([])),
                ),
                ("store_id", s(store.as_str())),
                ("payment_method_id", s(methods::BARK_PAYMENT_METHOD_ID)),
                ("invoice_provenance", provenance_checkpoint.clone()),
            ] {
                set(&mut next, key, value);
            }
            set(&mut outcome, "invoice_provenance", provenance.clone());
            set(&mut outcome, "freshness_checkpoint", next);
            Ok(outcome)
        })
    })
}

/// A savepoint of its own: the caller's commit points stay Python's.
fn nested<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let fail = |e: kassiber_store::rusqlite::Error| crate::database_error(e);
    kassiber_store::begin_write_savepoint(conn, "btcpay_wallet_sync").map_err(fail)?;
    match f() {
        Ok(value) => {
            conn.execute_batch("RELEASE SAVEPOINT btcpay_wallet_sync")
                .map_err(fail)?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT btcpay_wallet_sync; RELEASE SAVEPOINT btcpay_wallet_sync",
            );
            Err(error)
        }
    }
}

/// `_sync_btcpay_payment_ledger`: the store's provenance is the balance
/// source, so a failed refresh or a double booking fails the sync.
#[allow(clippy::too_many_arguments)]
fn payment_ledger(
    env: &Env<'_>,
    scope: &Scope<'_>,
    backend: &Json,
    config: &Json,
    store_id: &PyStr,
    page_size: i64,
    checkpoint: Value,
    previous_provenance: &Value,
) -> Result<Value> {
    let label = or_text(field(scope.wallet, "label"));
    let store = url::from_py(store_id);
    let name = text(&backend["name"]);
    let ids = ledger::payment_method_ids(config);
    if ids.is_empty() {
        return Err(AppError::new(
            "validation",
            format!("Wallet '{label}' has no BTCPay payment methods to book"),
        )
        .with_hint("Run `kassiber btcpay setup --backend NAME --route STORE:BTC-LN=payment_ledger` to rebuild it."));
    }
    let mut bark_status = None;
    if bark::is_bark_ledger(config) {
        // Bark plugin 0.10 and later serve the whole wallet: switch to it and
        // keep what the ledger booked. Older plugins keep the ledger.
        let (client, grant) =
            env.client("wallets.sync", &scope.binding(&name, &store), backend, None)?;
        let reads = fetch::Reads {
            client: &client,
            consent: &grant.0,
            deadline: deadline(),
            progress: env.progress,
        };
        bark_status = fetch::bark_wallet_history_status(reads, &store)?;
        if bark_status.is_none() {
            return bark_wallet(
                env,
                scope,
                backend,
                store_id,
                page_size,
                checkpoint,
                Some(config),
            );
        }
    }
    let args = scope.binding(&name, &store);
    let mut provenance = provenance::sync(
        env,
        provenance::Sync {
            workspace_id: scope.workspace_id,
            profile_id: scope.profile_id,
            backend: &name,
            store_id,
            page_size,
            include_payouts: true,
            suggest: false,
            kind: "wallets.sync",
            args: &args,
            checkpoint: previous_provenance,
        },
    )?;
    let provenance_checkpoint =
        pop(&mut provenance, "freshness_checkpoint").unwrap_or_else(|| dict::<&str>([]));
    if get(&provenance, "payouts_available") == Some(&Value::Bool(false)) {
        let missing = get(&provenance, "payouts_permission_missing").is_some_and(truthy);
        return Err(AppError::new(
            if missing { "auth_error" } else { "unavailable" },
            format!("BTCPay payouts for store {store} cannot be read, so '{label}' would miss refunds and payouts"),
        )
        .with_hint("Use a key with the view-payouts permission (the read-only preset includes it), or keep this store's Lightning invoices as provenance only.")
        .with_details(dict([
            ("store_id", s(store.as_str())),
            ("payouts_permission_missing", Value::Bool(missing)),
        ])));
    }
    let wallet_id = scope.wallet_id();
    let conn = env.conn;
    nested(conn, || {
        let (count, wallets) =
            ledger::existing_tracked_elsewhere(conn, scope.profile_id, &wallet_id)?;
        if count > 0 {
            return Err(AppError::new(
            "conflict",
            format!(
                "{count} payment(s) in '{label}' are also booked by {}",
                wallets.join(", ")
            ),
        )
        .with_hint("Counting them twice would overstate the balance and income. Exclude those payments in this ledger, or archive the ledger and keep the store's Lightning invoices as provenance.")
        .with_details(dict([
            ("duplicates", Value::Int(count.into())),
            ("wallets", Value::List(wallets.into_iter().map(s).collect())),
        ])));
        }
        let (records, counts) = ledger::records(conn, scope.profile_id, &store, &ids)?;
        let (records, held) =
            ledger::split_tracked_elsewhere(conn, scope.profile_id, &wallet_id, records)?;
        let mut outcome = file_import::import_records(
            conn,
            &RecordImport {
                profile_id: scope.profile_id,
                wallet_id: &wallet_id,
                label: &format!("btcpay-ledger:{name}:{store}"),
                match_existing_only: false,
                report_updates: false,
                btcpay: false,
                settlement_ids: &[],
            },
            &records,
        )?;
        set(&mut provenance, "status", s("synced"));
        set(&mut provenance, "suggestions", suggestions(conn, scope)?);
        let mut ledger_counts = counts;
        set(
            &mut ledger_counts,
            "held_tracked_elsewhere",
            Value::Int(held.len().into()),
        );
        let missing = get(&provenance, "payouts_permission_missing").is_some_and(truthy);
        let fields: Vec<(&str, Value)> = vec![
            ("backend", s(name.as_str())),
            ("backend_kind", s("btcpay")),
            (
                "backend_url",
                s(url::redact_backend_url(&text(&backend["url"]))),
            ),
            ("store_id", Value::Str(store_id.clone())),
            ("source_mode", s(ledger::SOURCE_MODE)),
            (
                "payment_method_ids",
                Value::List(ids.into_iter().map(s).collect()),
            ),
            ("fetched", Value::Int((records.len() + held.len()).into())),
            ("ledger", ledger_counts),
            ("payouts_permission_missing", Value::Bool(missing)),
            ("invoice_provenance", provenance),
            ("freshness_checkpoint", {
                let mut checkpoint = checkpoint.clone();
                set(&mut checkpoint, "invoice_provenance", provenance_checkpoint);
                checkpoint
            }),
        ];
        for (key, value) in fields {
            set(&mut outcome, key, value);
        }
        if let Some(status) = bark_status {
            set(&mut outcome, "bark_wallet_history", s(status));
        }
        Ok(outcome)
    })
}

/// `_is_missing_wallet_history_permission`. BTCPay 2.4 names `canviewwallet`
/// (the read-only key's wallet permission); 2.3 named `canmodifystoresettings`.
/// BTCPay may scope the missing permission to a store (`...:<storeId>`); only
/// this route's store counts.
fn missing_history_permission(error: &AppError, store_id: &str) -> bool {
    if error.code != "auth_error" {
        return false;
    }
    let details = match &error.details {
        Some(details @ Value::Dict(_)) => details.clone(),
        _ => dict::<&str>([]),
    };
    let missing = get(&details, "missing_permission")
        .filter(|v| truthy(v))
        .map(kassiber_envelope::py_str)
        .unwrap_or_default();
    let (name, scope) = match missing.split_once(':') {
        Some((name, scope)) => (name, Some(scope)),
        None => (missing.as_str(), None),
    };
    ([MODIFY_STORE_SETTINGS, VIEW_WALLET].contains(&name)
        && scope.is_none_or(|scope| scope == store_id))
        || (missing.is_empty() && get(&details, "http_status") == Some(&Value::Int(403.into())))
}

/// `enrich_wallet_from_btcpay_provenance` as the freshness job calls it:
/// a wallet whose routes are gone answers zero totals rather than nothing.
pub(crate) fn enrichment_or_empty(
    conn: &Connection,
    ctx: &Context<'_>,
    scope: &Scope<'_>,
    config: &Json,
    checkpoint: &Value,
) -> Result<Value> {
    if let Some(outcome) = enrichment(conn, ctx, scope, config, checkpoint)? {
        return Ok(outcome);
    }
    let mut next = match checkpoint {
        value @ Value::Dict(_) => value.clone(),
        _ => dict::<&str>([]),
    };
    set(&mut next, "routes", dict::<&str>([]));
    set(&mut next, "invoice_routes", dict::<&str>([]));
    let mut pairs: Vec<(&str, Value)> = [
        "routes",
        "fetched",
        "btcpay_notes_set",
        "btcpay_tags_added",
        "btcpay_tags_created",
        "labels_unavailable_routes",
    ]
    .into_iter()
    .map(|key| (key, Value::Int(0.into())))
    .collect();
    pairs.push(("route_results", Value::List(vec![])));
    pairs.push(("freshness_checkpoint", next));
    Ok(dict(pairs))
}

/// `enrich_wallet_from_btcpay_provenance`: wallet labels and comments where
/// the key can read history, and each mapped store's provenance either way.
/// `None` without routes.
pub(crate) fn enrichment(
    conn: &Connection,
    ctx: &Context<'_>,
    scope: &Scope<'_>,
    config: &Json,
    checkpoint: &Value,
) -> Result<Option<Value>> {
    let routes = crate::config::provenance(config.get("btcpay_provenance").unwrap_or(&Json::Null))?;
    let routes = routes.as_array().cloned().unwrap_or_default();
    if routes.is_empty() {
        return Ok(None);
    }
    let env = ctx.env(conn);
    let wallet_id = scope.wallet_id();
    // `wallet["_freshness_checkpoint"]`: its `routes` and `invoice_routes`.
    let checkpoint = match checkpoint {
        value @ Value::Dict(_) => value.clone(),
        _ => dict::<&str>([]),
    };
    let previous = |key: &str| match get(&checkpoint, key) {
        Some(value @ Value::Dict(_)) => value.clone(),
        _ => dict::<&str>([]),
    };
    let (previous_routes, previous_invoices) = (previous("routes"), previous("invoice_routes"));
    let mut totals = [0i64; 6];
    let mut results = Vec::new();
    let mut stores: Vec<(String, Value)> = Vec::new();
    let mut route_checkpoints: Vec<(String, Value)> = Vec::new();
    let mut invoice_checkpoints: Vec<(String, Value)> = Vec::new();
    for route in routes {
        let (backend, kind) = btcpay_backend(
            &env,
            &text(&route["backend"]),
            "Use a BTCPay backend for BTCPay provenance enrichment.",
        )?;
        let name = text(&backend["name"]);
        let store = text(&route["store_id"]);
        let method = text(&route["payment_method_id"]);
        let route_key = format!("{name}:{store}:{method}");
        let mut result = dict([
            ("backend", s(name.as_str())),
            ("backend_kind", s(kind.as_str())),
            (
                "backend_url",
                s(url::redact_backend_url(&text(&backend["url"]))),
            ),
            ("store_id", s(store.as_str())),
            ("payment_method_id", s(method.as_str())),
        ]);
        let store_id = PyStr::from(store.as_str());
        let route_checkpoint = get(&previous_routes, &route_key)
            .cloned()
            .unwrap_or_else(|| dict::<&str>([]));
        match wallet_history(
            &env,
            scope,
            &backend,
            &store_id,
            &method,
            PAGE_SIZE,
            &route_checkpoint,
        ) {
            Err(error) if missing_history_permission(&error, &store) => {
                for (key, value) in [
                    ("fetched", Value::Int(0.into())),
                    ("labels", s("unavailable_read_only_key")),
                    ("btcpay_notes_set", Value::Int(0.into())),
                    ("btcpay_tags_added", Value::Int(0.into())),
                    ("btcpay_tags_created", Value::Int(0.into())),
                ] {
                    set(&mut result, key, value);
                }
                totals[5] += 1;
                if let Some(kept) = get(&previous_routes, &route_key) {
                    route_checkpoints.push((route_key.clone(), kept.clone()));
                }
            }
            Err(error) => return Err(error),
            Ok((rows, meta)) => {
                let records = file_import::btcpay_rows(rows)?;
                let metadata =
                    file_import::btcpay_metadata(conn, scope.profile_id, &wallet_id, &records)?;
                for (key, value) in [
                    ("fetched", Value::Int(records.len().into())),
                    ("labels", s("synced")),
                    ("pages_fetched", page_field(&meta, "pages_fetched")),
                    (
                        "stopped_by_known_page",
                        Value::Bool(truthy(&page_field(&meta, "stopped_by_known_page"))),
                    ),
                    ("stop_reason", page_field(&meta, "stop_reason")),
                    ("deep_audit", page_field(&meta, "deep_audit")),
                ] {
                    set(&mut result, key, value);
                }
                let count = |key: &str| {
                    metadata
                        .iter()
                        .find(|(k, _)| *k == key)
                        .and_then(|(_, v)| match v {
                            Value::Int(n) => i64::try_from(n.clone()).ok(),
                            _ => None,
                        })
                        .unwrap_or_default()
                };
                totals[1] += records.len() as i64;
                totals[2] += count("btcpay_notes_set");
                totals[3] += count("btcpay_tags_added");
                totals[4] += count("btcpay_tags_created");
                for (key, value) in metadata {
                    set(&mut result, key, value);
                }
                route_checkpoints.push((
                    route_key.clone(),
                    dict([
                        (
                            "backend",
                            dict([("name", s(name.as_str())), ("kind", s(kind.as_str()))]),
                        ),
                        ("btcpay_pages", page_field(&meta, "btcpay_pages")),
                        ("btcpay_pagination", page_field(&meta, "btcpay_pagination")),
                        ("store_id", s(store.as_str())),
                        ("payment_method_id", s(method.as_str())),
                    ]),
                ));
            }
        }
        let store_key = format!("{name}:{store}");
        let provenance = match stores.iter().find(|(key, _)| *key == store_key) {
            Some((_, provenance)) => provenance.clone(),
            None => {
                let previous = match get(&previous_invoices, &store_key) {
                    Some(value @ Value::Dict(_)) => value.clone(),
                    _ => dict::<&str>([]),
                };
                let (provenance, checkpoint) =
                    store_provenance(&env, scope, &name, &store_id, true, &previous)?;
                stores.push((store_key.clone(), provenance.clone()));
                invoice_checkpoints.push((store_key, checkpoint));
                provenance
            }
        };
        set(&mut result, "invoice_provenance", provenance);
        results.push(result);
        totals[0] += 1;
    }
    let sorted = |mut entries: Vec<(String, Value)>| {
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Value::Dict(
            entries
                .into_iter()
                .map(|(k, v)| (Key::from(k.as_str()), v))
                .collect(),
        )
    };
    let mut pairs: Vec<(&str, Value)> = [
        "routes",
        "fetched",
        "btcpay_notes_set",
        "btcpay_tags_added",
        "btcpay_tags_created",
        "labels_unavailable_routes",
    ]
    .into_iter()
    .zip(totals)
    .map(|(key, value)| (key, Value::Int(value.into())))
    .collect();
    pairs.push(("route_results", Value::List(results)));
    let mut checkpoint = checkpoint;
    set(&mut checkpoint, "routes", sorted(route_checkpoints));
    set(
        &mut checkpoint,
        "invoice_routes",
        sorted(invoice_checkpoints),
    );
    pairs.push(("freshness_checkpoint", checkpoint));
    Ok(Some(dict(pairs)))
}

/// `wallets sync-btcpay` (`sync_btcpay_into_wallet`): point the wallet at a
/// store's wallet history, then sync it. As in Python, the configuration and
/// the store's provenance are written before the import, which stands alone.
pub fn sync_into_wallet(conn: &Connection, ctx: &Context<'_>, args: &Value) -> Result<Value> {
    let raw = |key: &str| {
        get(args, key)
            .filter(|v| !matches!(v, Value::Null))
            .map(kassiber_envelope::py_str)
    };
    let required = |key: &str, message: &str| {
        raw(key)
            .map(|v| kassiber_pytext::strip(&v).to_owned())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| AppError::new("validation", message))
    };
    let store = required("store_id", "BTCPay store id cannot be empty")?;
    let method = methods::canonical(&required(
        "payment_method_id",
        "BTCPay payment method id cannot be empty",
    )?);
    methods::require_wallet_history(&method, &method)?;
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    let id = |value: &Value| kassiber_envelope::py_str(get(value, "id").unwrap_or(&Value::Null));
    let (workspace_id, profile_id) = (id(&workspace), id(&profile));
    let wallet = crate::resolve(conn, &profile_id, &raw("wallet").unwrap_or_default())?;
    let backend_name = raw("backend").unwrap_or_default();
    btcpay_backend(&ctx.env(conn), &backend_name, CREATE_HINT)?;
    crate::lifecycle::execute(
        conn,
        crate::Operation::Update {
            profile_id: profile_id.clone(),
            wallet: wallet.id.clone(),
            label: None,
            account: None,
            config: serde_json::json!({
                "backend": backend_name,
                "store_id": store,
                "payment_method_id": method,
                "sync_source": "btcpay",
                // Wallet history replaces any payment-ledger mode.
                "source_mode": null,
                "payment_method_ids": null,
            }),
            clear: Vec::new(),
        },
    )?;
    let row =
        crate::publication::row::first(conn, "SELECT * FROM wallets WHERE id=?", [&wallet.id])?
            .ok_or_else(|| AppError::new("not_found", "Wallet not found"))?;
    let raw_config = or_text(field(&row, "config_json"));
    let config: Json = serde_json::from_str(if raw_config.is_empty() {
        "{}"
    } else {
        &raw_config
    })
    .map_err(|_| AppError::new("internal_error", "Invalid wallet configuration JSON"))?;
    let page_size = match get(args, "page_size") {
        Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap_or(i64::MAX),
        _ => PAGE_SIZE,
    };
    let scope = Scope {
        workspace_id: &workspace_id,
        profile_id: &profile_id,
        wallet: &row,
    };
    let outcome = sync_wallet(conn, ctx, &scope, &config, page_size, &Value::Null)?;
    nested(conn, || {
        let now = crate::lifecycle::now(conn)?;
        crate::publication::mark_synced(
            conn,
            &crate::publication::Synced {
                profile_id: &profile_id,
                wallet_id: &wallet.id,
                wallet_label: &wallet.label,
                checkpoint: None,
                partial_success: false,
                degraded_reason: None,
                blocking_reports: false,
                timestamp: &now,
                updated_at: "",
            },
        )
    })?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bark_import_savepoint_waits_for_another_writer() {
        // `bark_wallet` books, and switches a BTC-ARK ledger, inside `nested`:
        // it reads (identities, ledger rows) before its first write, so with
        // another connection holding the writer a deferred savepoint would fail
        // at once as database_busy. It must wait and succeed.
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        let path = std::env::temp_dir().join(format!("bark-writer-{}.sqlite3", std::process::id()));
        let clean = || {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
            }
        };
        clean();
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "PRAGMA journal_mode=WAL; CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT);
                 CREATE TABLE booked(v INTEGER); INSERT INTO booked VALUES(1);",
            )
            .unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.busy_timeout(Duration::from_secs(30)).unwrap();
        let (held, wait) = mpsc::channel();
        let holder = {
            let path = path.clone();
            std::thread::spawn(move || {
                let other = Connection::open(path).unwrap();
                other.execute_batch("BEGIN IMMEDIATE").unwrap();
                held.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(400));
                other.execute_batch("COMMIT").unwrap();
            })
        };
        wait.recv().unwrap();
        let started = Instant::now();
        nested(&conn, || {
            let read: i64 = conn
                .query_row("SELECT v FROM booked", [], |row| row.get(0))
                .map_err(crate::database_error)?;
            conn.execute("UPDATE booked SET v = ?", [read + 1])
                .map_err(crate::database_error)?;
            Ok(())
        })
        .unwrap();
        holder.join().unwrap();
        assert!(
            started.elapsed() >= Duration::from_millis(300),
            "{:?}",
            started.elapsed()
        );
        let booked: i64 = conn
            .query_row("SELECT v FROM booked", [], |row| row.get(0))
            .unwrap();
        assert_eq!(booked, 2);
        drop(conn);
        clean();
    }

    fn error(code: &str, details: &str) -> AppError {
        AppError::new(code, "denied https://user:pw@shop.example/api?x=1")
            .with_details(kassiber_pytext::loads(details).unwrap())
    }

    #[test]
    fn only_the_wallet_permissions_or_a_bare_403_mean_read_only() {
        let read_only =
            |code: &str, details: &str| missing_history_permission(&error(code, details), "S1");
        let modify = "btcpay.store.canmodifystoresettings";
        for wallet in [modify, "btcpay.store.canviewwallet"] {
            assert!(read_only(
                "auth_error",
                &format!(r#"{{"missing_permission": "{wallet}"}}"#)
            ));
            assert!(read_only(
                "auth_error",
                &format!(r#"{{"missing_permission": "{wallet}:S1"}}"#)
            ));
            assert!(!read_only(
                "auth_error",
                &format!(r#"{{"missing_permission": "{wallet}:S2"}}"#)
            ));
        }
        assert!(read_only("auth_error", r#"{"http_status": 403}"#));
        for details in [
            r#"{"missing_permission": "btcpay.store.canmanagewallets:S1"}"#.to_owned(),
            format!(r#"{{"missing_permission": "{modify}:S2"}}"#),
            format!(r#"{{"missing_permission": "{modify}:"}}"#),
            format!(r#"{{"missing_permission": "{modify}:S1:x"}}"#),
            format!(r#"{{"missing_permission": "{modify}:s1"}}"#),
            r#"{"missing_permission": "btcpay.store.canviewstoresettings:S1"}"#.to_owned(),
            format!(r#"{{"missing_permission": "{modify}x:S1"}}"#),
            r#"{"missing_permission": "btcpay.store.canviewstoresettings", "http_status": 403}"#
                .to_owned(),
            r#"{"http_status": 401}"#.to_owned(),
        ] {
            assert!(!read_only("auth_error", &details), "{details}");
        }
        assert!(!read_only("unavailable", r#"{"http_status": 403}"#));
    }

    #[test]
    fn error_summaries_redact_urls_and_name_the_permission() {
        let summary = error_summary(&error("auth_error", r#"{"missing_permission": "p"}"#));
        assert_eq!(get(&summary, "status"), Some(&s("unavailable")));
        assert_eq!(get(&summary, "missing_permission"), Some(&s("p")));
        let message = kassiber_envelope::py_str(get(&summary, "message").unwrap());
        assert!(!message.contains("user:pw"), "{message}");
        assert_eq!(get(&summary, "hint"), Some(&s("")));
    }
}

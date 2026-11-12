//! Lightning node wallet routes of `sync_wallets`: Core Lightning
//! (`lightning/cln.py::sync_core_lightning_wallet`). The node is read through
//! `lightning-cli` under a grant bound to this sync; only curated rows reach
//! `lightning_node_records`, and only invoice income, completed payments and
//! daily routing fees become wallet transactions.
use crate::file_import::{self, RecordImport};
use crate::publication::row::{field, or_text, Result};
use crate::publication::wallet::Sources;
use kassiber_chain::lightning::{cln_wallet_records, ClnReportClient, SyncAction};
use kassiber_envelope::{dict, get, s, set, truthy, AppError};
use kassiber_net::{Consent, Gate};
use kassiber_pyjson::{Key, Options, Value};
use kassiber_store::rusqlite::{params, types::Value as Sql, Connection, OptionalExtension};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

const CLN_IMPORT_SOURCE: &str = "core-lightning";
const UNKNOWN_OCCURRED_AT: &str = "1970-01-01T00:00:00Z";

/// What a node route needs beyond the book: the gate its process calls pass.
pub(crate) struct Context<'a> {
    pub gate: &'a Gate,
    pub book_id: &'a str,
}

/// `_json_dumps`: compact, key-sorted JSON.
fn compact(value: &Value) -> Result<String> {
    kassiber_pyjson::dumps(value, &Options::new().sort_keys(true).separators(",", ":"))
        .map_err(|_| AppError::new("internal_error", "Lightning record cannot be encoded"))
}

/// `_stable_hash`.
fn stable_hash(parts: Vec<Value>) -> Result<String> {
    let encoded = compact(&Value::List(parts))?;
    Ok(Sha256::digest(encoded.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn sql(value: &Value) -> Sql {
    match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Int(n) => i64::try_from(n.clone()).map_or(Sql::Null, Sql::Integer),
        Value::Str(text) => Sql::Text(kassiber_envelope::py_str(&Value::Str(text.clone()))),
        other => Sql::Text(kassiber_envelope::py_str(other)),
    }
}

fn integer(value: &Value) -> i64 {
    match value {
        Value::Int(n) => i64::try_from(n.clone()).unwrap_or_default(),
        _ => 0,
    }
}

/// `_upsert_lightning_record`: (inserted, changed).
#[allow(clippy::too_many_arguments)]
fn upsert(
    conn: &Connection,
    profile: &Value,
    wallet_id: &str,
    backend: &str,
    sync_id: &str,
    node_id: &Value,
    record: &Value,
    timestamp: &str,
) -> Result<(bool, bool)> {
    let profile_id = or_text(field(profile, "id"));
    let kind = or_text(field(record, "record_type"));
    if kind == "channel" && truthy(field(record, "tag")) && truthy(field(record, "txid")) {
        // Rows from before lifecycle identity included the channel account
        // would otherwise double the whole-transaction amount.
        let legacy = stable_hash(vec![
            s("channel"),
            field(record, "tag").clone(),
            field(record, "txid").clone(),
        ])?;
        if Value::Str(legacy.as_str().into()) != *field(record, "external_id") {
            conn.execute(
                "DELETE FROM lightning_node_records WHERE profile_id = ? AND wallet_id = ? AND backend_name = ? AND record_type = 'channel' AND external_id = ?",
                params![profile_id, wallet_id, backend, legacy],
            )
            .map_err(crate::database_error)?;
        }
    }
    let id = stable_hash(vec![
        s(profile_id.as_str()),
        s(wallet_id),
        s(backend),
        field(record, "record_type").clone(),
        field(record, "external_id").clone(),
    ])?;
    let existing: Option<(i64, i64, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT amount_msat, fee_msat, status, raw_json FROM lightning_node_records WHERE id = ?",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(crate::database_error)?;
    let occurred = match field(record, "occurred_at") {
        value if truthy(value) => sql(value),
        _ => Sql::Text(UNKNOWN_OCCURRED_AT.into()),
    };
    let raw = match field(record, "raw_json") {
        value if truthy(value) => kassiber_envelope::py_str(value),
        _ => "{}".into(),
    };
    conn.execute(
        "INSERT INTO lightning_node_records(
            id, workspace_id, profile_id, wallet_id, backend_name, node_id,
            record_type, external_id, occurred_at, account, peer_id, channel_id,
            direction, amount_msat, fee_msat, tag, status, currency, payment_hash,
            txid, outpoint, sync_id, raw_json, first_seen_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, wallet_id, backend_name, record_type, external_id)
        DO UPDATE SET
            node_id = excluded.node_id,
            occurred_at = excluded.occurred_at,
            account = excluded.account,
            peer_id = excluded.peer_id,
            channel_id = excluded.channel_id,
            direction = excluded.direction,
            amount_msat = excluded.amount_msat,
            fee_msat = excluded.fee_msat,
            tag = excluded.tag,
            status = excluded.status,
            currency = excluded.currency,
            payment_hash = excluded.payment_hash,
            txid = excluded.txid,
            outpoint = excluded.outpoint,
            sync_id = excluded.sync_id,
            raw_json = excluded.raw_json,
            updated_at = excluded.updated_at",
        params![
            id,
            sql(field(profile, "workspace_id")),
            profile_id,
            wallet_id,
            backend,
            sql(node_id),
            sql(field(record, "record_type")),
            sql(field(record, "external_id")),
            occurred,
            sql(field(record, "account")),
            sql(field(record, "peer_id")),
            sql(field(record, "channel_id")),
            sql(field(record, "direction")),
            integer(field(record, "amount_msat")),
            integer(field(record, "fee_msat")),
            sql(field(record, "tag")),
            sql(field(record, "status")),
            sql(field(record, "currency")),
            sql(field(record, "payment_hash")),
            sql(field(record, "txid")),
            sql(field(record, "outpoint")),
            sync_id,
            raw,
            timestamp,
            timestamp,
        ],
    )
    .map_err(crate::database_error)?;
    let Some((amount, fee, status, raw_before)) = existing else {
        return Ok((true, false));
    };
    let status_now = match field(record, "status") {
        Value::Null => String::new(),
        value => kassiber_envelope::py_str(value),
    };
    let changed = amount != integer(field(record, "amount_msat"))
        || fee != integer(field(record, "fee_msat"))
        || status.unwrap_or_default() != status_now
        || raw_before
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| "{}".into())
            != raw;
    Ok((false, changed))
}

fn btc(msat: i64) -> Value {
    Value::Str(file_import::msat_to_btc(msat).as_str().into())
}

fn text_or(value: &Value, default: &str) -> Value {
    if truthy(value) {
        value.clone()
    } else {
        s(default)
    }
}

/// `_record_to_import` with `_pay_to_import`: invoice income and completed
/// payments as wallet transactions; every other row stays node metadata.
fn to_import(record: &Value) -> Option<Value> {
    let kind = or_text(field(record, "record_type"));
    let direction = or_text(field(record, "direction"));
    let amount = integer(field(record, "amount_msat"));
    let occurred = text_or(field(record, "occurred_at"), UNKNOWN_OCCURRED_AT);
    let external = kassiber_envelope::py_str(field(record, "external_id"));
    let hash = field(record, "payment_hash").clone();
    let source = if truthy(&hash) {
        s("core_lightning")
    } else {
        Value::Null
    };
    if kind == "pay" && direction == "outbound" {
        if amount <= 0 {
            return None;
        }
        return Some(dict([
            ("id", s(format!("cln:pay:{external}").as_str())),
            ("occurred_at", occurred.clone()),
            ("confirmed_at", occurred),
            ("direction", s("outbound")),
            ("asset", s("BTC")),
            ("amount", btc(amount)),
            ("fee", btc(integer(field(record, "fee_msat")))),
            ("kind", s("cln_pay")),
            (
                "description",
                text_or(field(record, "status"), "Core Lightning payment"),
            ),
            ("counterparty", field(record, "peer_id").clone()),
            ("payment_hash", hash),
            ("payment_hash_source", source),
            ("raw_json", s("{}")),
        ]));
    }
    if kind != "income"
        || kassiber_pytext::lower(&or_text(field(record, "tag"))) != "invoice"
        || truthy(field(record, "txid"))
        || amount <= 0
        || direction != "inbound"
    {
        return None;
    }
    Some(dict([
        ("id", s(format!("cln:income:{external}").as_str())),
        ("occurred_at", occurred.clone()),
        ("confirmed_at", occurred),
        ("direction", s("inbound")),
        ("asset", s("BTC")),
        ("amount", btc(amount)),
        ("fee", Value::Int(0.into())),
        ("kind", s("cln_invoice")),
        (
            "description",
            text_or(field(record, "status"), "Core Lightning invoice"),
        ),
        ("counterparty", field(record, "account").clone()),
        ("payment_hash", hash),
        ("payment_hash_source", source),
        ("raw_json", s("{}")),
    ]))
}

/// `_daily_routing_income_imports`: earned routing fees, one row per UTC day.
fn routing_income(records: &[Value]) -> Vec<Value> {
    let mut days: std::collections::BTreeMap<String, i64> = Default::default();
    for record in records {
        if or_text(field(record, "record_type")) != "forward_day"
            || kassiber_pytext::lower(&or_text(field(record, "status"))) != "settled"
        {
            continue;
        }
        let fee = integer(field(record, "fee_msat")).max(0);
        if fee <= 0 {
            continue;
        }
        let occurred = or_text(field(record, "occurred_at"));
        if occurred.chars().count() < 10 || occurred == UNKNOWN_OCCURRED_AT {
            continue;
        }
        let day: String = occurred.chars().take(10).collect();
        *days.entry(day).or_default() += fee;
    }
    days.into_iter()
        .map(|(day, fee)| {
            let at = format!("{day}T23:59:59Z");
            dict([
                ("id", s(format!("cln:routing:{day}").as_str())),
                ("occurred_at", s(at.as_str())),
                ("confirmed_at", s(at.as_str())),
                ("direction", s("inbound")),
                ("asset", s("BTC")),
                ("amount", btc(fee)),
                ("fee", Value::Int(0.into())),
                ("kind", s("routing_income")),
                (
                    "description",
                    s(format!("Lightning routing fees ({day})").as_str()),
                ),
                ("counterparty", Value::Null),
                ("payment_hash", Value::Null),
                ("payment_hash_source", Value::Null),
                ("raw_json", s("{}")),
            ])
        })
        .collect()
}

/// `_stamp_lightning_import_network`: the node's observed network on each
/// ledger row's raw JSON.
pub(crate) fn stamp_network(records: Vec<Value>, network: &Value) -> Result<Vec<Value>> {
    let text = kassiber_envelope::py_str(network);
    if kassiber_pytext::strip(&text).is_empty() {
        return Ok(records);
    }
    records
        .into_iter()
        .map(|mut record| {
            let raw = match field(&record, "raw_json") {
                Value::Str(raw) => {
                    kassiber_pytext::loads(&kassiber_envelope::py_str(&Value::Str(raw.clone())))
                        .ok()
                        .filter(|v| matches!(v, Value::Dict(_)))
                        .unwrap_or_else(|| dict::<&str>([]))
                }
                Value::Dict(_) => field(&record, "raw_json").clone(),
                _ => dict::<&str>([]),
            };
            let mut payload = raw;
            set(&mut payload, "chain", s("lightning"));
            set(&mut payload, "network", network.clone());
            let encoded = kassiber_pyjson::dumps(&payload, &Options::new().sort_keys(true))
                .map_err(|_| {
                    AppError::new("internal_error", "Lightning record cannot be encoded")
                })?;
            set(&mut record, "raw_json", s(encoded.as_str()));
            Ok(record)
        })
        .collect()
}

/// `sync_core_lightning_wallet(..., commit=False)` inside the wallet's savepoint.
pub(crate) fn sync_core_lightning(
    conn: &Connection,
    ctx: &Context<'_>,
    profile: &Value,
    wallet: &Value,
    backend: &Json,
    clock: &mut dyn Sources,
) -> Result<Value> {
    let kind = match &backend["kind"] {
        Json::Null => String::new(),
        Json::String(kind) => kind.clone(),
        other => other.to_string(),
    };
    let name = backend["name"].as_str().unwrap_or_default().to_owned();
    if kassiber_pytext::lower(&kind) != "coreln" {
        let shown = match &backend["kind"] {
            Json::Null => "None".to_owned(),
            Json::String(kind) => kind.clone(),
            other => other.to_string(),
        };
        return Err(AppError::new(
            "validation",
            format!("Backend '{name}' has kind '{shown}', expected 'coreln'"),
        )
        .with_hint("Create a Core Lightning backend with `kassiber backends create <name> --kind coreln --url cln://local`."));
    }
    let profile_id = or_text(field(profile, "id"));
    let wallet_id = or_text(field(wallet, "id"));
    let started_at = clock.now();
    let sync_id = clock.id();
    conn.execute(
        "INSERT INTO lightning_node_syncs(id, workspace_id, profile_id, wallet_id, backend_name, started_at, status, created_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?)",
        params![sync_id, sql(field(profile, "workspace_id")), profile_id, wallet_id, name, started_at, "running", started_at],
    )
    .map_err(crate::database_error)?;
    let client = ClnReportClient::for_wallet_sync(
        backend,
        ctx.gate.clone(),
        &SyncAction {
            book_id: ctx.book_id.to_owned(),
            profile_id: profile_id.clone(),
            wallet_id: wallet_id.clone(),
        },
    )?;
    let consent = Consent::explicit_trigger(client.request());
    let snapshot = client.wallet_snapshot(
        Some(&consent),
        Instant::now() + Duration::from_secs(900),
        || Ok(()),
    );
    consent.revoke();
    let snapshot = snapshot?;
    let completed_at = clock.now();
    let records = cln_wallet_records(&snapshot, &completed_at)?;
    let node_id = Value::Str(snapshot.node_id.clone());
    let (mut inserted, mut updated) = (0, 0);
    let mut counts: Vec<(String, i64)> = Vec::new();
    let mut imports = Vec::new();
    for record in &records {
        let kind = or_text(field(record, "record_type"));
        match counts.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, count)) => *count += 1,
            None => counts.push((kind, 1)),
        }
        let (was_inserted, was_updated) = upsert(
            conn,
            profile,
            &wallet_id,
            &name,
            &sync_id,
            &node_id,
            record,
            &completed_at,
        )?;
        inserted += i64::from(was_inserted);
        updated += i64::from(was_updated);
        imports.extend(to_import(record));
    }
    imports.extend(routing_income(&records));
    let imports = stamp_network(imports, &Value::Str(snapshot.network.clone()))?;
    let fetched = imports.len();
    let outcome = file_import::import_records(
        conn,
        &RecordImport {
            profile_id: &profile_id,
            wallet_id: &wallet_id,
            label: CLN_IMPORT_SOURCE,
            match_existing_only: false,
            report_updates: false,
            btcpay: false,
            settlement_ids: &[],
        },
        &file_import::record_rows(imports),
    )?;
    let status = if snapshot.errors.is_empty() {
        "synced"
    } else {
        "partial"
    };
    let counts = Value::Dict(
        counts
            .into_iter()
            .map(|(k, n)| (Key::from(k.as_str()), Value::Int(n.into())))
            .collect(),
    );
    let errors = Value::Dict(
        snapshot
            .errors
            .iter()
            .map(|(method, message)| (Key::from(method.as_str()), s(message.as_str())))
            .collect(),
    );
    conn.execute(
        "UPDATE lightning_node_syncs SET node_id = ?, node_alias = ?, completed_at = ?, status = ?, fetched_counts_json = ?, error_json = ? WHERE id = ?",
        params![
            sql(&node_id),
            sql(&Value::Str(snapshot.node_alias.clone())),
            completed_at,
            status,
            compact(&counts)?,
            compact(&errors)?,
            sync_id
        ],
    )
    .map_err(crate::database_error)?;
    crate::invalidate_journals(conn, &profile_id)?;
    let count = |key: &str| get(&outcome, key).cloned().unwrap_or(Value::Int(0.into()));
    Ok(dict([
        ("wallet", field(wallet, "label").clone()),
        ("backend", s(name.as_str())),
        ("backend_kind", s("coreln")),
        (
            "backend_url",
            s(
                kassiber_setup::backends::redact_backend_url(&match &backend["url"] {
                    Json::String(url) => s(url.as_str()),
                    Json::Null => Value::Null,
                    other => s(other.to_string().as_str()),
                })?
                .as_str(),
            ),
        ),
        ("node_id", node_id),
        ("node_alias", Value::Str(snapshot.node_alias.clone())),
        ("status", s(status)),
        ("sync_id", s(sync_id.as_str())),
        ("started_at", s(started_at.as_str())),
        ("completed_at", s(completed_at.as_str())),
        ("records_fetched", Value::Int(records.len().into())),
        ("records_inserted", Value::Int(inserted.into())),
        ("records_updated", Value::Int(updated.into())),
        ("fetched_counts", counts),
        ("method_errors", errors),
        (
            "transactions",
            dict([
                ("fetched", Value::Int(fetched.into())),
                ("imported", count("imported")),
                ("skipped", count("skipped")),
                ("updated", count("updated")),
            ]),
        ),
        ("read_only", Value::Bool(true)),
    ]))
}

const LND_IMPORT_SOURCE: &str = "lnd";

/// `_persist_lnd_channel_records`: channel-aware lifecycle rows, an
/// incomplete amount never replacing a known one.
fn persist_lnd_channels(
    conn: &Connection,
    profile: &Value,
    wallet_id: &str,
    backend: &str,
    records: &[Value],
    timestamp: &str,
) -> Result<()> {
    let profile_id = or_text(field(profile, "id"));
    for record in records {
        let external = or_text(field(record, "external_id"));
        let legacy = format!(
            "channel:{}:{}",
            or_text(field(record, "tag")),
            or_text(field(record, "txid"))
        );
        if legacy != external {
            conn.execute(
                "DELETE FROM lightning_node_records WHERE profile_id = ? AND wallet_id = ? AND backend_name = ? AND record_type = 'channel' AND external_id = ?",
                params![profile_id, wallet_id, backend, legacy],
            )
            .map_err(crate::database_error)?;
        }
        conn.execute(
            "INSERT INTO lightning_node_records(
                id, workspace_id, profile_id, wallet_id, backend_name, node_id,
                record_type, external_id, occurred_at, account, peer_id, channel_id,
                direction, amount_msat, fee_msat, tag, status, currency, payment_hash,
                txid, outpoint, sync_id, raw_json, first_seen_at, updated_at
            ) VALUES(?, ?, ?, ?, ?, NULL, 'channel', ?, ?, NULL, NULL, ?,
                     '', ?, 0, ?, ?, 'bc', NULL, ?, ?, NULL, ?, ?, ?)
            ON CONFLICT(profile_id, wallet_id, backend_name, record_type, external_id)
            DO UPDATE SET txid = excluded.txid, outpoint = excluded.outpoint,
                          channel_id = excluded.channel_id,
                          amount_msat = CASE
                              WHEN excluded.amount_msat < 0
                                   AND lightning_node_records.amount_msat >= 0
                              THEN lightning_node_records.amount_msat
                              ELSE excluded.amount_msat
                          END,
                          tag = excluded.tag, status = excluded.status,
                          raw_json = excluded.raw_json,
                          updated_at = excluded.updated_at",
            params![
                format!("{wallet_id}:{backend}:channel:{external}"),
                sql(field(profile, "workspace_id")),
                profile_id,
                wallet_id,
                backend,
                external,
                UNKNOWN_OCCURRED_AT,
                sql(field(record, "channel_id")),
                integer(field(record, "amount_msat")),
                or_text(field(record, "tag")),
                or_text(field(record, "status")),
                or_text(field(record, "txid")),
                sql(field(record, "outpoint")),
                match field(record, "raw_json") {
                    value if truthy(value) => kassiber_envelope::py_str(value),
                    _ => "{}".into(),
                },
                timestamp,
                timestamp,
            ],
        )
        .map_err(crate::database_error)?;
    }
    Ok(())
}

/// The ledger rows' msat `amount` and `fee` rendered as BTC (`msat_to_btc`).
fn as_btc(records: Vec<Value>) -> Vec<Value> {
    records
        .into_iter()
        .map(|mut record| {
            for key in ["amount", "fee"] {
                if let Some(Value::Int(msat)) = get(&record, key).cloned() {
                    let rendered = if key == "fee" && msat == 0.into() {
                        Value::Int(0.into())
                    } else {
                        btc(i64::try_from(msat).unwrap_or(i64::MAX))
                    };
                    set(&mut record, key, rendered);
                }
            }
            record
        })
        .collect()
}

/// `sync_lnd_wallet(..., commit=False)` inside the wallet's savepoint.
pub(crate) fn sync_lnd(
    conn: &Connection,
    ctx: &Context<'_>,
    profile: &Value,
    wallet: &Value,
    backend: &Json,
    clock: &mut dyn Sources,
) -> Result<Value> {
    let name = backend["name"].as_str().unwrap_or_default().to_owned();
    let kind = match &backend["kind"] {
        Json::Null => String::new(),
        Json::String(kind) => kind.clone(),
        other => other.to_string(),
    };
    if kassiber_pytext::lower(&kind) != "lnd" {
        let shown = match &backend["kind"] {
            Json::Null => "None".to_owned(),
            Json::String(kind) => kind.clone(),
            other => other.to_string(),
        };
        return Err(AppError::new(
            "validation",
            format!("Backend '{name}' has kind '{shown}', expected 'lnd'"),
        )
        .with_hint("Create an LND backend with `kassiber backends create <name> --kind lnd --url https://... --token-stdin`."));
    }
    let profile_id = or_text(field(profile, "id"));
    let wallet_id = or_text(field(wallet, "id"));
    let client = kassiber_chain::lightning::LndReportClient::for_wallet_sync(
        backend,
        ctx.gate.clone(),
        env!("CARGO_PKG_VERSION"),
        &SyncAction {
            book_id: ctx.book_id.to_owned(),
            profile_id: profile_id.clone(),
            wallet_id: wallet_id.clone(),
        },
    )?;
    let consent = Consent::explicit_trigger(client.request());
    let deadline = Instant::now() + Duration::from_secs(900);
    let result = (|| {
        let history = client.wallet_history(Some(&consent), deadline)?;
        let network = Value::Str(history.network.clone());
        let imports = stamp_network(
            as_btc(kassiber_chain::lightning::lnd_import_records(&history)?),
            &network,
        )?;
        let settled = kassiber_chain::lightning::lnd_settlement_ids(&history)?;
        let fetched = imports.len();
        let outcome = file_import::import_records(
            conn,
            &RecordImport {
                profile_id: &profile_id,
                wallet_id: &wallet_id,
                label: LND_IMPORT_SOURCE,
                match_existing_only: false,
                report_updates: false,
                btcpay: false,
                settlement_ids: &settled,
            },
            &file_import::record_rows(imports),
        )?;
        let (open, closed) = client.wallet_channels(Some(&consent), deadline)?;
        let channels =
            kassiber_chain::lightning::lnd_channel_records(&open, &closed, &history.network)?;
        persist_lnd_channels(conn, profile, &wallet_id, &name, &channels, &clock.now())?;
        crate::invalidate_journals(conn, &profile_id)?;
        let count = |key: &str| get(&outcome, key).cloned().unwrap_or(Value::Int(0.into()));
        Ok(dict([
            ("wallet", field(wallet, "label").clone()),
            ("backend", s(name.as_str())),
            ("backend_kind", s("lnd")),
            ("status", s("synced")),
            (
                "transactions",
                dict([
                    ("fetched", Value::Int(fetched.into())),
                    ("imported", count("imported")),
                    ("skipped", count("skipped")),
                ]),
            ),
            (
                "channels",
                dict([("records", Value::Int(channels.len().into()))]),
            ),
            (
                "routing",
                dict([(
                    "forward_events_aggregated",
                    Value::Int(history.forwards.len().into()),
                )]),
            ),
        ]))
    })();
    consent.revoke();
    result
}

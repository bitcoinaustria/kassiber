//! BTCPay provenance storage (`core/commercial.py`): invoice, payment and
//! payout records, per-store sync states and account routes. Records are
//! evidence only; nothing here changes a transaction or a balance.
use crate::{
    documents::{decimal, exact, present, timestamp, value},
    sql::{error, now, rows, uuid, Result, Scope},
};
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

const NULL: Value = Value::Null;

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    get(v, key).unwrap_or(&NULL)
}

fn either<'a>(operands: &[&'a Value]) -> &'a Value {
    operands
        .iter()
        .find(|v| truthy(v))
        .copied()
        .unwrap_or_else(|| operands[operands.len() - 1])
}

fn text(v: &Value) -> String {
    py_str(v)
}

fn opt_text(v: &Value) -> Option<String> {
    (!matches!(v, Value::Null)).then(|| text(v))
}

fn context() -> Context {
    Context::RP2
}

/// `_fiat_currency`.
pub fn fiat_currency(v: &Value) -> Option<String> {
    let code = kassiber_pytext::upper(kassiber_pytext::strip(&if truthy(v) {
        text(v)
    } else {
        String::new()
    }));
    (!code.is_empty() && !["BTC", "SATS", "SAT", "MSAT", "LBTC"].contains(&code.as_str()))
        .then_some(code)
}

/// `_amount_msat`: `btc_to_msat(dec(value))` (ROUND_HALF_UP).
fn amount_msat(v: &Value) -> Result<Option<i64>> {
    if !present(v) {
        return Ok(None);
    }
    let product = decimal(v)?
        .mul(&Decimal::from(100_000_000_000_u64), &context())
        .map_err(|_| AppError::new("app_error", "Invalid decimal value"))?;
    product
        .to_fixed_places(0, Rounding::HalfUp)
        .ok()
        .and_then(|t| t.parse().ok())
        .map(Some)
        .ok_or_else(|| {
            AppError::new(
                "app_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        })
}

/// `_computed_rate(fiat_value, crypto_amount)`.
fn computed_rate(fiat: &Value, amount: &Value) -> Result<Option<String>> {
    if !present(fiat) || !present(amount) {
        return Ok(None);
    }
    let amount = decimal(amount)?;
    if amount.is_zero() {
        return Ok(None);
    }
    let fiat = kassiber_pydecimal::finite_price(&text(fiat))
        .map_err(|e| AppError::new("validation", e.message("fiat_value")))?;
    let rate = fiat
        .div(&amount, &context())
        .map_err(|e| AppError::new("app_error", e.to_string()))?;
    exact(&s(rate.to_canonical()))
}

/// `_json(value)`: `json.dumps(json_ready(value or {}), sort_keys=True)`.
fn json(v: &Value) -> Result<String> {
    let empty = dict::<&str>([]);
    let options = kassiber_pyjson::Options {
        sort_keys: true,
        ..Default::default()
    };
    kassiber_pyjson::dumps(if truthy(v) { v } else { &empty }, &options)
        .map_err(|e| AppError::new("internal_error", e.to_string()))
}

fn sql_opt(v: Option<String>) -> Option<String> {
    v
}

struct Record<'a> {
    backend: &'a str,
    store_id: &'a Value,
    payment_method_id: Option<String>,
    record_type: &'a str,
    stable_key: String,
    invoice_id: Option<String>,
    payment_id: Option<String>,
    order_id: Option<String>,
    status: Option<String>,
    occurred_at: Option<String>,
    asset: Option<String>,
    amount: Option<i64>,
    txid: Option<String>,
    payment_hash: Option<String>,
    destination: Option<String>,
    payment_request_id: Option<String>,
    origin_kind: Option<String>,
    origin_app_id: Option<String>,
    origin_label: Option<String>,
    origin_url: Option<String>,
    origin_source: Option<String>,
    fiat_currency: Option<String>,
    fiat_value_exact: Option<String>,
    fiat_rate_exact: Option<String>,
    pricing_timestamp: Option<String>,
    raw_json: String,
}

/// `_upsert_record`: `true` when inserted.
fn upsert_record(conn: &Connection, scope: &Scope<'_>, r: Record<'_>, now: &str) -> Result<bool> {
    let store = text(r.store_id);
    let existing = rows(
        conn,
        "SELECT id FROM btcpay_provenance_records WHERE profile_id = ? AND stable_key = ?",
        params![scope.profile_id, r.stable_key],
    )?;
    if let Some(row) = existing.first() {
        conn.execute(
            "UPDATE btcpay_provenance_records SET backend_name = ?, store_id = ?, payment_method_id = ?, record_type = ?, invoice_id = ?, payment_id = ?, order_id = ?, status = ?, occurred_at = ?, asset = ?, amount = ?, txid = ?, payment_hash = ?, destination = ?, payment_request_id = ?, origin_kind = ?, origin_app_id = ?, origin_label = ?, origin_url = ?, origin_source = ?, fiat_currency = ?, fiat_value_exact = ?, fiat_rate_exact = ?, pricing_timestamp = ?, raw_json = ?, updated_at = ? WHERE id = ?",
            params![
                r.backend, store, r.payment_method_id, r.record_type, r.invoice_id, r.payment_id,
                r.order_id, r.status, r.occurred_at, r.asset, r.amount, r.txid, r.payment_hash,
                r.destination, r.payment_request_id, r.origin_kind, r.origin_app_id,
                r.origin_label, r.origin_url, r.origin_source, r.fiat_currency,
                r.fiat_value_exact, r.fiat_rate_exact, r.pricing_timestamp, r.raw_json, now,
                text(field(row, "id")),
            ],
        )
        .map_err(error)?;
        return Ok(false);
    }
    let id = uuid(conn)?;
    conn.execute(
        "INSERT INTO btcpay_provenance_records(id, workspace_id, profile_id, backend_name, store_id, payment_method_id, record_type, stable_key, invoice_id, payment_id, order_id, status, occurred_at, asset, amount, txid, payment_hash, destination, payment_request_id, origin_kind, origin_app_id, origin_label, origin_url, origin_source, fiat_currency, fiat_value_exact, fiat_rate_exact, pricing_timestamp, raw_json, created_at, updated_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            id, scope.workspace_id, scope.profile_id, r.backend, store, r.payment_method_id,
            r.record_type, r.stable_key, r.invoice_id, r.payment_id, r.order_id, r.status,
            r.occurred_at, r.asset, r.amount, r.txid, r.payment_hash, r.destination,
            r.payment_request_id, r.origin_kind, r.origin_app_id, r.origin_label, r.origin_url,
            r.origin_source, r.fiat_currency, r.fiat_value_exact, r.fiat_rate_exact,
            r.pricing_timestamp, r.raw_json, now, now,
        ],
    )
    .map_err(error)?;
    Ok(true)
}

fn asset_from_payment_method(v: &Value) -> Option<String> {
    let raw = kassiber_pytext::upper(kassiber_pytext::strip(&if truthy(v) {
        text(v)
    } else {
        String::new()
    }));
    (!raw.is_empty()).then(|| raw.split('-').next().unwrap_or_default().to_owned())
}

/// `_stable_payment_id`.
fn stable_payment_id(payment: &Value) -> Result<String> {
    let raw = json(either(&[field(payment, "payment"), payment]))?;
    let digest: String = Sha256::digest(raw.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(format!("sha256:{digest}"))
}

/// `upsert_btcpay_provenance` for normalized invoices (`commit=False`).
pub fn upsert_provenance(
    conn: &Connection,
    scope: &Scope<'_>,
    backend: &str,
    invoices: &[Value],
) -> Result<Value> {
    let (mut inserted, mut updated) = (0, 0);
    let now = now(conn)?;
    for invoice in invoices {
        let invoice_id = text(field(invoice, "invoice_id"));
        let store = field(invoice, "store_id");
        let currency = fiat_currency(field(invoice, "currency"));
        let origin_url = opt_text(either(&[
            field(invoice, "origin_url"),
            field(invoice, "order_url"),
        ]));
        let created = timestamp(field(invoice, "created_at"))?;
        let was_insert = upsert_record(
            conn,
            scope,
            Record {
                backend,
                store_id: store,
                payment_method_id: None,
                record_type: "invoice",
                stable_key: format!("btcpay:{}:invoice:{invoice_id}", text(store)),
                invoice_id: Some(invoice_id.clone()),
                payment_id: None,
                order_id: opt_text(field(invoice, "order_id")),
                status: opt_text(field(invoice, "status")),
                occurred_at: created.clone(),
                asset: None,
                amount: None,
                txid: None,
                payment_hash: None,
                destination: None,
                payment_request_id: opt_text(field(invoice, "payment_request_id")),
                origin_kind: opt_text(field(invoice, "origin_kind")),
                origin_app_id: opt_text(field(invoice, "origin_app_id")),
                origin_label: opt_text(field(invoice, "origin_label")),
                origin_url: origin_url.clone(),
                origin_source: opt_text(field(invoice, "origin_source")),
                fiat_value_exact: if currency.is_some() {
                    exact(field(invoice, "amount"))?
                } else {
                    None
                },
                fiat_currency: currency.clone(),
                fiat_rate_exact: None,
                pricing_timestamp: created.clone(),
                raw_json: json(field(invoice, "invoice"))?,
            },
            &now,
        )?;
        inserted += usize::from(was_insert);
        updated += usize::from(!was_insert);
        let payments = match either(&[field(invoice, "payments"), &Value::List(Vec::new())]) {
            Value::List(items) => items.clone(),
            other => {
                return Err(AppError::new(
                    "internal_error",
                    format!(
                        "'{}' object is not iterable",
                        kassiber_envelope::type_name(other)
                    ),
                ))
            }
        };
        for payment in &payments {
            let payment_id = match either(&[field(payment, "payment_id"), field(payment, "txid")]) {
                v if truthy(v) => text(v),
                _ => stable_payment_id(payment)?,
            };
            let method = field(payment, "payment_method_id");
            let payment_currency = fiat_currency(either(&[
                field(payment, "invoice_currency"),
                field(invoice, "currency"),
            ]));
            let fiat_value = if payment_currency.is_some() {
                either(&[field(payment, "invoice_amount"), field(invoice, "amount")]).clone()
            } else {
                Value::Null
            };
            let rate = if payment_currency.is_some() {
                match field(payment, "rate") {
                    v if truthy(v) => Some(text(v)),
                    _ => computed_rate(&fiat_value, field(payment, "amount"))?,
                }
            } else {
                None
            };
            let occurred = timestamp(either(&[
                field(payment, "received_at"),
                field(invoice, "created_at"),
            ]))?;
            let was_insert = upsert_record(
                conn,
                scope,
                Record {
                    backend,
                    store_id: store,
                    payment_method_id: opt_text(method),
                    record_type: "payment",
                    stable_key: format!(
                        "btcpay:{}:invoice:{invoice_id}:payment:{payment_id}",
                        text(store)
                    ),
                    invoice_id: Some(invoice_id.clone()),
                    payment_id: Some(payment_id),
                    order_id: opt_text(field(invoice, "order_id")),
                    status: opt_text(either(&[
                        field(payment, "status"),
                        field(invoice, "status"),
                    ])),
                    occurred_at: occurred.clone(),
                    asset: asset_from_payment_method(method),
                    amount: amount_msat(field(payment, "amount"))?,
                    txid: opt_text(field(payment, "txid")),
                    payment_hash: opt_text(field(payment, "payment_hash")),
                    destination: opt_text(field(payment, "destination")),
                    payment_request_id: opt_text(field(invoice, "payment_request_id")),
                    origin_kind: opt_text(field(invoice, "origin_kind")),
                    origin_app_id: opt_text(field(invoice, "origin_app_id")),
                    origin_label: opt_text(field(invoice, "origin_label")),
                    origin_url: origin_url.clone(),
                    origin_source: opt_text(field(invoice, "origin_source")),
                    fiat_currency: payment_currency,
                    fiat_value_exact: exact(&fiat_value)?,
                    fiat_rate_exact: exact(&rate.map_or(Value::Null, s))?,
                    pricing_timestamp: occurred,
                    raw_json: json(field(payment, "payment"))?,
                },
                &now,
            )?;
            inserted += usize::from(was_insert);
            updated += usize::from(!was_insert);
        }
    }
    Ok(dict([
        ("invoices_seen", Value::Int(invoices.len().into())),
        ("records_inserted", Value::Int(inserted.into())),
        ("records_updated", Value::Int(updated.into())),
    ]))
}

/// `upsert_btcpay_payouts` (`commit=False`).
pub fn upsert_payouts(
    conn: &Connection,
    scope: &Scope<'_>,
    backend: &str,
    payouts: &[Value],
) -> Result<Value> {
    let (mut inserted, mut updated) = (0, 0);
    let now = now(conn)?;
    for payout in payouts {
        let payout_id = text(field(payout, "payout_id"));
        let store = field(payout, "store_id");
        let fiat_value = field(payout, "fiat_value");
        let occurred = timestamp(field(payout, "occurred_at"))?;
        let was_insert = upsert_record(
            conn,
            scope,
            Record {
                backend,
                store_id: store,
                payment_method_id: opt_text(field(payout, "payout_method_id")),
                record_type: "payout",
                stable_key: format!("btcpay:{}:payout:{payout_id}", text(store)),
                invoice_id: opt_text(field(payout, "invoice_id")),
                payment_id: Some(payout_id),
                order_id: None,
                status: opt_text(field(payout, "state")),
                occurred_at: occurred.clone(),
                asset: opt_text(field(payout, "asset")),
                amount: amount_msat(field(payout, "amount"))?,
                txid: opt_text(field(payout, "txid")),
                payment_hash: opt_text(field(payout, "payment_hash")),
                destination: opt_text(field(payout, "destination")),
                payment_request_id: None,
                origin_kind: opt_text(field(payout, "origin_kind")),
                origin_app_id: opt_text(field(payout, "pull_payment_id")),
                origin_label: opt_text(field(payout, "origin_label")),
                origin_url: opt_text(field(payout, "origin_url")),
                origin_source: Some("btcpay_payout".into()),
                fiat_currency: sql_opt(opt_text(field(payout, "fiat_currency"))),
                fiat_value_exact: exact(fiat_value)?,
                fiat_rate_exact: if truthy(fiat_value) {
                    computed_rate(fiat_value, field(payout, "amount"))?
                } else {
                    None
                },
                pricing_timestamp: occurred,
                raw_json: json(field(payout, "payout"))?,
            },
            &now,
        )?;
        inserted += usize::from(was_insert);
        updated += usize::from(!was_insert);
    }
    Ok(dict([
        ("payouts_seen", Value::Int(payouts.len().into())),
        ("payout_records_inserted", Value::Int(inserted.into())),
        ("payout_records_updated", Value::Int(updated.into())),
    ]))
}

/// `record_btcpay_store_sync`; `failure` is `(code, already redacted message)`.
pub fn record_store_sync(
    conn: &Connection,
    profile_id: &str,
    backend: &str,
    store_id: &str,
    failure: Option<(&str, &str)>,
    seen: (Option<i64>, Option<i64>),
) -> Result<()> {
    let now = now(conn)?;
    let success = failure.is_none().then(|| now.clone());
    let message = failure
        .filter(|(_, m)| !m.is_empty())
        .map(|(_, m)| m.chars().take(500).collect::<String>());
    conn.execute(
        "INSERT INTO btcpay_store_sync_states(profile_id, backend_name, store_id, last_attempt_at, last_success_at, last_error_code, last_error_message, invoices_seen, payouts_seen, updated_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(profile_id, backend_name, store_id) DO UPDATE SET last_attempt_at = excluded.last_attempt_at, last_success_at = COALESCE(excluded.last_success_at, last_success_at), last_error_code = excluded.last_error_code, last_error_message = excluded.last_error_message, invoices_seen = COALESCE(excluded.invoices_seen, invoices_seen), payouts_seen = COALESCE(excluded.payouts_seen, payouts_seen), updated_at = excluded.updated_at",
        params![
            profile_id,
            kassiber_pytext::lower(kassiber_pytext::strip(backend)),
            store_id,
            now,
            success,
            failure.map(|(code, _)| code),
            message,
            seen.0,
            seen.1,
            now,
        ],
    )
    .map_err(error)?;
    Ok(())
}

fn instant(text: &str) -> Option<i64> {
    crate::metadata::time::instant(text).map(|us| us / 1_000_000)
}

/// `btcpay_store_sync_states`, sorted by `(backend, store)`.
pub fn store_sync_states(
    conn: &Connection,
    profile_id: &str,
    stores: &[(String, String)],
) -> Result<Vec<((String, String), Value)>> {
    let now = now(conn)?;
    let mut wanted: Vec<(String, String)> = stores
        .iter()
        .map(|(b, s)| (kassiber_pytext::lower(kassiber_pytext::strip(b)), s.clone()))
        .collect();
    wanted.sort();
    wanted.dedup();
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let found = rows(
        conn,
        "SELECT * FROM btcpay_store_sync_states WHERE profile_id = ?",
        [profile_id],
    )?;
    let mut out = Vec::new();
    for key in wanted {
        let row = found.iter().rev().find(|r| {
            text(field(r, "backend_name")) == key.0 && text(field(r, "store_id")) == key.1
        });
        let payload = match row {
            None => dict([
                ("last_attempt_at", Value::Null),
                ("last_success_at", Value::Null),
                ("last_error_code", Value::Null),
                ("age_seconds", Value::Null),
                ("stale", Value::Bool(true)),
                ("never_synced", Value::Bool(true)),
            ]),
            Some(row) => {
                let success = value(row, "last_success_at");
                let age = if truthy(&success) {
                    instant(&text(&success))
                        .zip(instant(&now))
                        .map(|(then, current)| (current - then).max(0))
                } else {
                    None
                };
                dict([
                    ("last_attempt_at", value(row, "last_attempt_at")),
                    ("last_success_at", success.clone()),
                    ("last_error_code", value(row, "last_error_code")),
                    (
                        "age_seconds",
                        age.map_or(Value::Null, |n| Value::Int(n.into())),
                    ),
                    (
                        "stale",
                        Value::Bool(
                            age.is_none_or(|n| n > 86_400)
                                || truthy(&value(row, "last_error_code")),
                        ),
                    ),
                    ("never_synced", Value::Bool(!truthy(&success))),
                ])
            }
        };
        out.push((key, payload));
    }
    Ok(out)
}

fn route_payload(row: &Value) -> Value {
    dict([
        ("id", s(text(field(row, "id")))),
        ("backend", s(text(field(row, "backend_name")))),
        ("store_id", s(text(field(row, "store_id")))),
        (
            "payment_method_id",
            s(text(field(row, "payment_method_id"))),
        ),
        ("action", s(text(field(row, "action")))),
        (
            "label",
            s(if truthy(field(row, "label")) {
                text(field(row, "label"))
            } else {
                String::new()
            }),
        ),
        ("created_at", s(text(field(row, "created_at")))),
        ("updated_at", s(text(field(row, "updated_at")))),
    ])
}

/// `list_btcpay_account_routes`.
pub fn account_routes(
    conn: &Connection,
    profile_id: &str,
    backend: Option<&str>,
) -> Result<Vec<Value>> {
    let backend = backend
        .filter(|b| !b.is_empty())
        .map(|b| kassiber_pytext::lower(kassiber_pytext::strip(b)));
    Ok(rows(
        conn,
        "SELECT * FROM btcpay_account_routes WHERE profile_id = ?1 AND (?2 IS NULL OR backend_name = ?2) ORDER BY backend_name ASC, store_id ASC, payment_method_id ASC, action ASC",
        params![profile_id, backend],
    )?
    .iter()
    .map(route_payload)
    .collect())
}

/// `upsert_btcpay_account_route` (only `provenance_only` routes exist).
pub fn upsert_account_route(
    conn: &Connection,
    scope: &Scope<'_>,
    backend: &str,
    store_id: &str,
    payment_method_id: &str,
    label: Option<&str>,
) -> Result<Value> {
    let backend = kassiber_pytext::lower(kassiber_pytext::strip(backend));
    let id = uuid(conn)?;
    let now = now(conn)?;
    conn.execute(
        "INSERT INTO btcpay_account_routes(id, workspace_id, profile_id, backend_name, store_id, payment_method_id, action, label, created_at, updated_at) VALUES(?, ?, ?, ?, ?, ?, 'provenance_only', ?, ?, ?) ON CONFLICT(profile_id, backend_name, store_id, payment_method_id, action) DO UPDATE SET label = excluded.label, updated_at = excluded.updated_at",
        params![id, scope.workspace_id, scope.profile_id, backend, store_id, payment_method_id, label, now, now],
    )
    .map_err(error)?;
    let found = rows(
        conn,
        "SELECT * FROM btcpay_account_routes WHERE profile_id = ? AND backend_name = ? AND store_id = ? AND payment_method_id = ? AND action = 'provenance_only'",
        params![scope.profile_id, backend, store_id, payment_method_id],
    )?;
    found.first().map(route_payload).ok_or_else(|| {
        AppError::new(
            "internal",
            "Stored BTCPay account route could not be loaded",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `kassiber/__init__.py` pins Python's decimal context at 32 digits;
    /// a regtest payout of 10 EUR for 0.00014687 BTC stored 32 digits there
    /// and only 28 natively.
    #[test]
    fn computed_rates_keep_python_precision() {
        assert_eq!(
            computed_rate(&s("10"), &s("0.00014687")).unwrap(),
            Some("68087.424252740518826172805882753".into())
        );
    }
}

//! `review_link`: one commercial link's review state. Reviewing a BTCPay link
//! applies its exact pricing (and kind) to the matched transaction with a
//! snapshot to restore; reopening or rejecting restores that snapshot. Payouts
//! settled by one transaction are reviewed together as a batch.
use super::*;
use crate::documents::{exact, present};
use kassiber_envelope::{py_str, type_name};
use kassiber_store::rusqlite::types::Value as SqlValue;
use sha2::{Digest, Sha256};

const APPLY_COLUMNS: &[&str] = &[
    "fiat_currency",
    "fiat_rate",
    "fiat_value",
    "fiat_price_source",
    "fiat_rate_exact",
    "fiat_value_exact",
    "pricing_source_kind",
    "pricing_provider",
    "pricing_pair",
    "pricing_timestamp",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
    "pricing_external_ref",
    "pricing_quality",
    "kind",
    "commercial_applied_link_id",
];
const PAYOUT_KINDS: &[&str] = &["expense", "refund", "transfer", "none"];

pub struct Review {
    pub link: String,
    pub state: Value,
    pub reconciliation_state: Value,
    pub commercial_kind: Value,
    pub notes: Value,
}

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message).retryable(false)
}

fn first(
    conn: &Connection,
    sql: &str,
    params: impl kassiber_store::rusqlite::Params,
) -> Result<Option<Value>> {
    Ok(rows(conn, sql, params)?.into_iter().next())
}

fn link_row(conn: &Connection, profile: &str, id: &str) -> Result<Value> {
    first(
        conn,
        "SELECT * FROM commercial_links WHERE profile_id = ? AND id = ?",
        [profile, id],
    )?
    .ok_or_else(|| AppError::new("not_found", format!("Commercial link '{id}' not found")))
}

/// `_normalize_choice` with a default for a falsy value.
fn normalized(v: &Value, allowed: &[&str], label: &str, default: Option<&str>) -> Result<String> {
    match default {
        Some(default) if !truthy(v) => choice(&s(default), allowed, label),
        _ => choice(v, allowed, label),
    }
}

/// Python's `a or b`.
fn or(a: &Value, b: &Value) -> Value {
    if truthy(a) {
        a.clone()
    } else {
        b.clone()
    }
}

fn payout_links(
    conn: &Connection,
    profile: &str,
    transaction: &Value,
    exclude: Option<&str>,
    states: &[&str],
) -> Result<Vec<Value>> {
    let placeholders = vec!["?"; states.len()].join(",");
    let mut params = vec![SqlValue::Text(profile.into()), sql(transaction)?];
    params.extend(states.iter().map(|state| SqlValue::Text((*state).into())));
    params.push(exclude.map_or(SqlValue::Null, |id| SqlValue::Text(id.into())));
    rows(
        conn,
        &format!(
            "SELECT cl.* FROM commercial_links cl \
             JOIN btcpay_provenance_records p ON p.id = cl.btcpay_record_id \
             WHERE cl.profile_id = ? AND cl.transaction_id = ? \
               AND cl.link_type = 'btcpay_payment_transaction' \
               AND p.record_type = 'payout' \
               AND cl.state IN ({placeholders}) \
               AND cl.id != COALESCE(?, '') \
             ORDER BY cl.created_at ASC, cl.id ASC"
        ),
        kassiber_store::rusqlite::params_from_iter(params),
    )
}

/// The reviewed payout link that applied its batch to this link's transaction.
fn batch_owner(conn: &Connection, profile: &str, link: &Value) -> Result<Option<String>> {
    let transaction = value(link, "transaction_id");
    if !truthy(&transaction) || text(link, "link_type") != "btcpay_payment_transaction" {
        return Ok(None);
    }
    let tx = first(
        conn,
        "SELECT commercial_applied_link_id FROM transactions WHERE profile_id = ? AND id = ?",
        params![profile, sql(&transaction)?],
    )?;
    let owner = tx
        .map(|tx| value(&tx, "commercial_applied_link_id"))
        .unwrap_or(Value::Null);
    if !truthy(&owner) || owner == value(link, "id") {
        return Ok(None);
    }
    let ids: Vec<Value> = payout_links(
        conn,
        profile,
        &transaction,
        None,
        &["suggested", "reviewed"],
    )?
    .iter()
    .map(|row| value(row, "id"))
    .collect();
    Ok((ids.contains(&owner) && ids.contains(&value(link, "id"))).then(|| py_str(&owner)))
}

fn reviewed_members(conn: &Connection, profile: &str, owner: &Value) -> Result<Vec<String>> {
    Ok(payout_links(
        conn,
        profile,
        &value(owner, "transaction_id"),
        Some(text(owner, "id")),
        &["reviewed"],
    )?
    .iter()
    .map(|row| text(row, "id").to_owned())
    .collect())
}

fn record_snapshot(conn: &Connection, link: &Value) -> Result<(Value, Value)> {
    let record = value(link, "btcpay_record_id");
    if !truthy(&record) {
        return Ok((
            value(link, "reviewed_record_snapshot_json"),
            value(link, "reviewed_record_snapshot_sha256"),
        ));
    }
    let Some(row) = first(
        conn,
        "SELECT raw_json FROM btcpay_provenance_records WHERE id = ?",
        params![sql(&record)?],
    )?
    else {
        return Ok((Value::Null, Value::Null));
    };
    let raw = or(&value(&row, "raw_json"), &s("{}"));
    let digest: String = Sha256::digest(py_str(&raw).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((raw, s(digest)))
}

fn dumps_sorted(value: &Value) -> Result<String> {
    kassiber_pyjson::dumps(
        value,
        &kassiber_pyjson::Options {
            sort_keys: true,
            ..Default::default()
        },
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))
}

fn loads(text: &Value) -> Result<Value> {
    kassiber_pytext::loads(&py_str(text))
        .map_err(|e| AppError::new("internal_error", e.to_string()))
}

/// `json.loads(snapshot).get(key)`.
fn snapshot_get(snapshot: &Value, key: &str) -> Result<Value> {
    if !matches!(snapshot, Value::Dict(_)) {
        return Err(AppError::new(
            "internal_error",
            format!("'{}' object has no attribute 'get'", type_name(snapshot)),
        ));
    }
    Ok(value(snapshot, key))
}

fn update_transaction(
    conn: &Connection,
    profile: &str,
    transaction: &Value,
    updates: &[(&str, Value)],
) -> Result<()> {
    let assignments = updates
        .iter()
        .map(|(column, _)| format!("{column} = ?"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut params = updates
        .iter()
        .map(|(_, v)| sql(v))
        .collect::<Result<Vec<_>>>()?;
    params.push(SqlValue::Text(profile.into()));
    params.push(sql(transaction)?);
    conn.execute(
        &format!("UPDATE transactions SET {assignments} WHERE profile_id = ? AND id = ?"),
        kassiber_store::rusqlite::params_from_iter(params),
    )
    .map_err(error)?;
    Ok(())
}

fn restore(conn: &Connection, profile: &str, link: &Value) -> Result<bool> {
    let snapshot = value(link, "applied_transaction_snapshot_json");
    if !truthy(&snapshot) {
        return Err(AppError::new(
            "validation",
            format!(
                "Commercial link '{}' cannot be reverted because it has no transaction snapshot",
                text(link, "id")
            ),
        ));
    }
    let tx = first(
        conn,
        "SELECT commercial_applied_link_id FROM transactions WHERE profile_id = ? AND id = ?",
        params![profile, sql(&value(link, "transaction_id"))?],
    )?;
    if tx.is_none_or(|tx| value(&tx, "commercial_applied_link_id") != value(link, "id")) {
        return Ok(false);
    }
    let snapshot = loads(&snapshot)?;
    let updates = APPLY_COLUMNS
        .iter()
        .map(|column| Ok((*column, snapshot_get(&snapshot, column)?)))
        .collect::<Result<Vec<_>>>()?;
    update_transaction(conn, profile, &value(link, "transaction_id"), &updates)?;
    Ok(true)
}

fn origin_label(record: &Value) -> &'static str {
    match text(record, "origin_kind") {
        "refund" => "BTCPay refund",
        "pull_payment" | "store_payout" => "BTCPay payout",
        "payment_request" => "BTCPay payment request",
        "crowdfund" => "BTCPay crowdfund",
        "pos" => "BTCPay point of sale",
        _ => "BTCPay invoice",
    }
}

/// `_attach_btcpay_origin_url`: a literal URL reference, never fetched.
fn attach_origin(
    conn: &Connection,
    scope: &Scope<'_>,
    transaction: &Value,
    record: &Value,
) -> Result<()> {
    let origin = value(record, "origin_url");
    let raw = strip(&if truthy(&origin) {
        py_str(&origin)
    } else {
        String::new()
    })
    .to_owned();
    if raw.is_empty() {
        return Ok(());
    }
    if crate::attachments::validate_literal_url(&raw).is_err() {
        return Err(AppError::new("internal_error", "Invalid IPv6 URL"));
    }
    let (scheme, _, _) = crate::attachments::url_parts(&raw);
    if !matches!(scheme.as_str(), "http" | "https") {
        return Ok(());
    }
    let existing = first(
        conn,
        "SELECT id FROM attachments WHERE profile_id = ? AND transaction_id = ? AND attachment_type = 'url' AND source_url = ? LIMIT 1",
        params![scope.profile_id, sql(transaction)?, raw],
    )?;
    if existing.is_some() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO attachments(id, workspace_id, profile_id, transaction_id, attachment_type, label, source_url, media_type, created_at) VALUES(?, ?, ?, ?, 'url', ?, ?, 'text/uri-list', ?)",
        params![
            uuid(conn)?,
            scope.workspace_id,
            scope.profile_id,
            sql(transaction)?,
            origin_label(record),
            raw,
            now(conn)?
        ],
    )
    .map_err(error)?;
    Ok(())
}

fn context() -> Context {
    Context::new(32, Rounding::HalfEven)
}

fn arithmetic(e: kassiber_pydecimal::DecimalError) -> AppError {
    AppError::new("internal_error", e.to_string())
}

/// `pricing_payload`'s `exact_decimal(finite_decimal(value, field))`: a
/// bounded price with every zero canonical.
fn exact_decimal(v: &Value, field: &str) -> Result<Value> {
    if !present(v) {
        return Ok(Value::Null);
    }
    let price = kassiber_pydecimal::finite_price(&py_str(v))
        .map_err(|e| AppError::new("validation", e.message(field)))?;
    Ok(s(price.to_fixed_string().map_err(arithmetic)?))
}

/// `pricing_payload(amount_msat=tx["amount"], ...)` (f823b4729): the prices
/// must also imply bounded valuations for the transaction's own quantity.
fn validate_derived(amount: &Value, rate: &Value, value: &Value) -> Result<()> {
    let amount = integer(amount)?
        .to_string()
        .parse::<Decimal>()
        .map_err(arithmetic)?;
    let text = |v: &Value| present(v).then(|| py_str(v));
    kassiber_pydecimal::validate_transaction_price(
        "pricing input",
        &amount,
        text(rate).as_deref(),
        text(value).as_deref(),
        &context(),
    )
    .map_err(|error| match error {
        kassiber_pydecimal::TransactionPriceError::Validation(message) => {
            AppError::new("validation", message)
        }
        kassiber_pydecimal::TransactionPriceError::Arithmetic(e) => arithmetic(e),
    })
}

fn float(v: &Value) -> Value {
    match v {
        Value::Str(t) => t
            .as_str()
            .and_then(|t| kassiber_pytext::parse_float(t).ok())
            .map_or(Value::Null, Value::Float),
        _ => Value::Null,
    }
}

struct Applied {
    applied: bool,
    snapshot: Value,
    batch: Vec<String>,
}

fn batch_members(
    conn: &Connection,
    profile: &str,
    link: &Value,
    record: &Value,
    tx: &Value,
) -> Result<Vec<(Value, Value)>> {
    let mut members = Vec::new();
    for member_link in payout_links(
        conn,
        profile,
        &value(tx, "id"),
        Some(text(link, "id")),
        &["suggested", "reviewed"],
    )? {
        let Some(member) = first(
            conn,
            "SELECT * FROM btcpay_provenance_records WHERE id = ?",
            params![sql(&value(&member_link, "btcpay_record_id"))?],
        )?
        else {
            continue;
        };
        let matches = matching(conn, profile, &member, true)?;
        if matches.len() != 1 || value(&matches[0], "id") != value(tx, "id") {
            return Err(AppError::new(
                "ambiguous",
                "A payout in this batch matches more than one wallet transaction",
            )
            .with_hint("Resolve the duplicate rows, or reject that payout's suggestion, before reviewing the batch.")
            .with_details(dict([
                ("btcpay_record_id", value(&member, "id")),
                ("matches", Value::Int(matches.len().into())),
            ])));
        }
        if let Some(elsewhere) = first(
            conn,
            "SELECT id FROM commercial_links WHERE profile_id = ? AND btcpay_record_id = ? AND state = 'reviewed' AND link_type = 'btcpay_payment_transaction' AND transaction_id != ? LIMIT 1",
            params![profile, sql(&value(&member, "id"))?, sql(&value(tx, "id"))?],
        )? {
            return Err(AppError::new(
                "conflict",
                "A payout in this batch is already reviewed against another transaction",
            )
            .with_details(dict([
                ("btcpay_record_id", value(&member, "id")),
                ("reviewed_link_id", value(&elsewhere, "id")),
            ])));
        }
        let (asset, record_asset) = (value(&member, "asset"), value(record, "asset"));
        if or(&asset, &record_asset) != or(&record_asset, &asset) {
            return Err(AppError::new(
                "validation",
                "Payouts in one transaction must share an asset",
            )
            .with_details(dict([("btcpay_record_id", value(&member, "id"))])));
        }
        let (currency, record_currency) = (
            value(&member, "fiat_currency"),
            value(record, "fiat_currency"),
        );
        if truthy(&currency) && truthy(&record_currency) && currency != record_currency {
            let mut currencies = vec![py_str(&currency), py_str(&record_currency)];
            currencies.sort();
            currencies.dedup();
            return Err(AppError::new("validation", "Payouts in one transaction use different currencies")
                .with_hint("Kassiber prices a batch in one currency. Reject the payouts that should not set this transaction's price.")
                .with_details(dict([("currencies", Value::List(currencies.into_iter().map(s).collect()))])));
        }
        members.push((member_link, member));
    }
    Ok(members)
}

/// `find_active_review_for_transaction`: the active authored custody
/// component (or its legacy review) owning this boundary.
fn custody_review(conn: &Connection, profile: &str, transaction: &Value) -> Result<Option<Value>> {
    let Some(row) = first(
        conn,
        "SELECT component.id AS component_id, term.legacy_source_id AS review_id, term.term_kind \
         FROM custody_component_legs leg \
         JOIN custody_components component ON component.id = leg.component_id \
         LEFT JOIN custody_component_economic_terms term \
           ON term.component_id = component.id \
          AND (term.source_leg_id = leg.id OR term.target_leg_id = leg.id) \
         WHERE component.profile_id = ? AND component.state = 'active' \
           AND COALESCE(leg.anchor_transaction_id, leg.transaction_id) = ? \
         ORDER BY component.created_at, component.id, term.ordinal LIMIT 1",
        params![profile, sql(transaction)?],
    )?
    else {
        return Ok(None);
    };
    Ok(Some(dict([
        (
            "id",
            or(&value(&row, "review_id"), &value(&row, "component_id")),
        ),
        ("component_id", value(&row, "component_id")),
    ])))
}

fn apply(
    conn: &Connection,
    scope: &Scope<'_>,
    link: &Value,
    commercial: &Value,
) -> Result<Applied> {
    let profile = scope.profile_id;
    let unapplied = || Applied {
        applied: false,
        snapshot: value(link, "applied_transaction_snapshot_json"),
        batch: Vec::new(),
    };
    let record_id = value(link, "btcpay_record_id");
    if !truthy(&record_id) {
        return Ok(unapplied());
    }
    let Some(record) = first(
        conn,
        "SELECT * FROM btcpay_provenance_records WHERE id = ?",
        params![sql(&record_id)?],
    )?
    else {
        return Ok(unapplied());
    };
    let transaction = value(link, "transaction_id");
    let tx = first(
        conn,
        &format!(
            "SELECT id, direction, asset, amount, {} FROM transactions WHERE profile_id = ? AND id = ?",
            APPLY_COLUMNS.join(", ")
        ),
        params![profile, sql(&transaction)?],
    )?
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            format!(
                "Transaction '{}' not found for commercial link '{}'",
                py_str(&transaction),
                text(link, "id")
            ),
        )
    })?;
    let owner = value(&tx, "commercial_applied_link_id");
    if truthy(&owner) && owner != value(link, "id") {
        return Err(AppError::new(
            "conflict",
            "Transaction already has reviewed BTCPay pricing from another commercial link",
        )
        .with_hint("Reject or revert the existing reviewed link before reviewing another one.")
        .with_details(dict([
            ("transaction_id", value(&tx, "id")),
            ("applied_link_id", owner.clone()),
        ])));
    }
    if let Some(existing) = first(
        conn,
        "SELECT id FROM commercial_links WHERE profile_id = ? AND btcpay_record_id = ? AND link_type = 'btcpay_payment_transaction' AND state = 'reviewed' AND id != ? LIMIT 1",
        params![profile, sql(&value(&record, "id"))?, text(link, "id")],
    )? {
        return Err(AppError::new(
            "conflict",
            "BTCPay payment is already reviewed against another transaction",
        )
        .with_hint("Reject the existing reviewed link before reviewing this payment again.")
        .with_details(dict([("btcpay_record_id", value(&record, "id")), ("reviewed_link_id", value(&existing, "id"))])));
    }
    let record_type = text(&record, "record_type");
    let direction = value(&tx, "direction");
    if record_type == "payout" {
        if direction != s("outbound") {
            return Err(AppError::new(
                "validation",
                "BTCPay payouts can only be reviewed against outbound transactions",
            )
            .with_details(dict([
                ("btcpay_record_id", value(&record, "id")),
                ("transaction_direction", direction.clone()),
            ])));
        }
        if !matches!(commercial, Value::Null) && !PAYOUT_KINDS.iter().any(|k| *commercial == s(*k))
        {
            return Err(AppError::new(
                "validation",
                "A BTCPay payout cannot be reviewed as income",
            )
            .with_hint("Review refunds as refund and supplier or payroll payouts as expense."));
        }
    }
    if matches!(record_type, "payment" | "payout") {
        let matches = matching(conn, profile, &record, true)?;
        if matches.len() != 1 || value(&matches[0], "id") != value(&tx, "id") {
            return Err(AppError::new("ambiguous", "BTCPay payment matches multiple wallet transactions")
                .with_hint("Resolve the duplicate txid/payment-hash rows before reviewing this provenance link.")
                .with_details(dict([
                    ("btcpay_record_id", value(&record, "id")),
                    ("matches", Value::Int(matches.len().into())),
                ])));
        }
    }
    let batch = if record_type == "payout" {
        batch_members(conn, profile, link, &record, &tx)?
    } else {
        Vec::new()
    };
    let asset = value(&record, "asset");
    if truthy(&asset) && asset != value(&tx, "asset") {
        return Err(AppError::new(
            "validation",
            "BTCPay payment asset does not match the target transaction asset",
        )
        .with_details(dict([
            ("payment_asset", asset.clone()),
            ("transaction_asset", value(&tx, "asset")),
        ])));
    }
    let currency = fiat_currency(&value(&record, "fiat_currency"));
    let tx_currency = value(&tx, "fiat_currency");
    if !currency.is_empty() && truthy(&tx_currency) && s(currency.as_str()) != tx_currency {
        return Err(AppError::new(
            "validation",
            "BTCPay invoice currency does not match the target transaction currency",
        )
        .with_details(dict([
            ("payment_currency", value(&record, "fiat_currency")),
            ("transaction_currency", tx_currency),
        ])));
    }
    if *commercial == s("income") && direction != s("inbound") {
        return Err(AppError::new(
            "validation",
            "Commercial income can only be applied to inbound transactions",
        ));
    }
    if *commercial == s("expense") && direction != s("outbound") {
        return Err(AppError::new(
            "validation",
            "Commercial expense can only be applied to outbound transactions",
        ));
    }
    if let Some(review) = custody_review(conn, profile, &value(&tx, "id"))? {
        return Err(AppError::new("validation", "Commercial review cannot overwrite a transaction in an active custody review")
            .with_hint("Reopen or supersede the custody review first, or review the merchant-side transaction instead.")
            .with_details(dict([
                ("transaction_id", value(&tx, "id")),
                ("review_id", value(&review, "id")),
                ("component_id", value(&review, "component_id")),
            ])));
    }
    let snapshot = if owner == value(link, "id")
        && truthy(&value(link, "applied_transaction_snapshot_json"))
    {
        value(link, "applied_transaction_snapshot_json")
    } else {
        s(dumps_sorted(&dict(
            APPLY_COLUMNS.iter().map(|c| (*c, value(&tx, c))),
        ))?)
    };
    let source_kind = match record_type {
        "payment" => "btcpay_payment",
        "payout" => "btcpay_payout",
        _ => "btcpay_invoice",
    };
    let mut amount = integer(&value(&record, "amount"))?.abs();
    let (mut fiat_value, mut rate) = if currency.is_empty() {
        (Value::Null, Value::Null)
    } else {
        (
            value(&record, "fiat_value_exact"),
            value(&record, "fiat_rate_exact"),
        )
    };
    if !batch.is_empty() {
        // Batched payouts price the transaction together: the summed payout
        // value over the summed payout amount. A member without a fiat value
        // leaves the transaction on its existing pricing.
        let members: Vec<&Value> = std::iter::once(&record)
            .chain(batch.iter().map(|(_, r)| r))
            .collect();
        amount = 0;
        for member in &members {
            amount += integer(&value(member, "amount"))?.abs();
        }
        let priced = members.iter().all(|m| {
            truthy(&value(m, "fiat_value_exact"))
                && !fiat_currency(&value(m, "fiat_currency")).is_empty()
        });
        fiat_value = if priced {
            let mut total = Decimal::zero();
            for member in &members {
                total = total
                    .add(&decimal(&value(member, "fiat_value_exact"))?, &context())
                    .map_err(arithmetic)?;
            }
            exact(&s(total.to_string()))?.map_or(Value::Null, s)
        } else {
            Value::Null
        };
        rate = Value::Null;
    }
    if !truthy(&rate) && truthy(&fiat_value) && amount > 0 {
        let btc = amount
            .to_string()
            .parse::<Decimal>()
            .map_err(arithmetic)?
            .div(
                &"100000000000".parse::<Decimal>().map_err(arithmetic)?,
                &context(),
            )
            .map_err(arithmetic)?;
        let quotient = decimal(&fiat_value)?
            .div(&btc, &context())
            .map_err(arithmetic)?;
        rate = exact(&s(quotient.to_string()))?.map_or(Value::Null, s);
    }
    let mut updates: Vec<(&str, Value)> = Vec::new();
    if truthy(&fiat_value) || truthy(&rate) {
        let rate_exact = exact_decimal(&rate, "fiat_rate")?;
        let value_exact = exact_decimal(&fiat_value, "fiat_value")?;
        validate_derived(&value(&tx, "amount"), &rate_exact, &value_exact)?;
        let timestamp = or(
            &value(&record, "pricing_timestamp"),
            &value(&record, "occurred_at"),
        );
        updates.extend([
            ("fiat_rate", float(&rate_exact)),
            ("fiat_value", float(&value_exact)),
            ("fiat_rate_exact", rate_exact),
            ("fiat_value_exact", value_exact),
            ("fiat_price_source", s("import")),
            ("pricing_source_kind", s(source_kind)),
            ("pricing_provider", s("btcpay")),
            (
                "pricing_pair",
                if currency.is_empty() {
                    Value::Null
                } else {
                    s(format!("{}-{currency}", py_str(&or(&asset, &s("BTC")))))
                },
            ),
            ("pricing_timestamp", timestamp),
            ("pricing_fetched_at", value(&record, "updated_at")),
            (
                "pricing_granularity",
                s(if !batch.is_empty() {
                    "payout_batch"
                } else if record_type == "payout" {
                    "payout"
                } else {
                    "invoice_payment"
                }),
            ),
            ("pricing_method", s("reviewed_commercial_link")),
            ("pricing_external_ref", value(&record, "stable_key")),
            ("pricing_quality", s("exact")),
        ]);
        if !currency.is_empty() {
            updates.push(("fiat_currency", s(currency.as_str())));
        }
    }
    if truthy(commercial) && *commercial != s("transfer") {
        updates.push(("kind", commercial.clone()));
    } else if matches!(commercial, Value::Null) || *commercial == s("transfer") {
        updates.push(("kind", snapshot_get(&loads(&snapshot)?, "kind")?));
    }
    let batch_ids: Vec<String> = batch
        .iter()
        .map(|(l, _)| text(l, "id").to_owned())
        .collect();
    if updates.is_empty() {
        return Ok(Applied {
            applied: false,
            snapshot,
            batch: batch_ids,
        });
    }
    updates.push(("commercial_applied_link_id", value(link, "id")));
    update_transaction(conn, profile, &transaction, &updates)?;
    attach_origin(conn, scope, &transaction, &record)?;
    for (_, member) in &batch {
        attach_origin(conn, scope, &transaction, member)?;
    }
    Ok(Applied {
        applied: true,
        snapshot,
        batch: batch_ids,
    })
}

pub(super) fn review(conn: &Connection, scope: &Scope<'_>, request: &Review) -> Result<Value> {
    let profile = scope.profile_id;
    let mut link_ref = request.link.clone();
    let mut before = link_row(conn, profile, &link_ref)?;
    let state = normalized(
        &request.state,
        &["suggested", "reviewed", "rejected"],
        "link state",
        None,
    )?;
    let owner = batch_owner(conn, profile, &before)?;
    let was_reviewed = text(&before, "state") == "reviewed";
    if state == "rejected"
        && was_reviewed
        && (owner.is_some() || !reviewed_members(conn, profile, &before)?.is_empty())
    {
        return Err(validation(
            "This payout was reviewed together with the other payouts of its transaction",
        )
        .with_hint(
            "Reopen the review first, then mark the payouts that do not belong as not related.",
        ));
    }
    if let Some(owner) = owner {
        if state == "reviewed" || was_reviewed {
            // One payout transaction carries one reviewed kind and price.
            // Reviewing or reopening any payout of a reviewed batch acts on
            // the whole batch.
            before = link_row(conn, profile, &owner)?;
            link_ref = owner;
        }
    }
    let reconciliation = normalized(
        &request.reconciliation_state,
        &["unreviewed", "matched", "mismatch", "ignored"],
        "reconciliation state",
        Some(if state == "reviewed" {
            "matched"
        } else {
            "ignored"
        }),
    )?;
    let commercial = if matches!(request.commercial_kind, Value::Null) {
        value(&before, "commercial_kind")
    } else {
        match normalized(
            &request.commercial_kind,
            &["income", "expense", "refund", "transfer", "none"],
            "commercial kind",
            None,
        )?
        .as_str()
        {
            "none" => Value::Null,
            kind => s(kind),
        }
    };
    let now = now(conn)?;
    let mut applied = false;
    let mut restored = false;
    let mut apply_snapshot = value(&before, "applied_transaction_snapshot_json");
    let mut snapshot = value(&before, "reviewed_record_snapshot_json");
    let mut sha = value(&before, "reviewed_record_snapshot_sha256");
    let mut batch: Vec<String> = Vec::new();
    let mut reopened: Vec<String> = Vec::new();
    let before_reviewed = text(&before, "state") == "reviewed";
    let has_transaction = truthy(&value(&before, "transaction_id"));
    if before_reviewed && state != "reviewed" && has_transaction {
        restored = restore(conn, profile, &before)?;
        if restored {
            reopened = reviewed_members(conn, profile, &before)?;
        }
    }
    if state == "reviewed" && !truthy(&sha) {
        (snapshot, sha) = record_snapshot(conn, &before)?;
    }
    if state == "reviewed" && has_transaction {
        let result = apply(conn, scope, &before, &commercial)?;
        applied = result.applied;
        apply_snapshot = result.snapshot;
        batch = result.batch;
    }
    let reviewed_at = if state == "reviewed" {
        s(now.as_str())
    } else {
        value(&before, "reviewed_at")
    };
    conn.execute(
        "UPDATE commercial_links SET state = ?, reconciliation_state = ?, commercial_kind = ?, applied_transaction_snapshot_json = ?, reviewed_record_snapshot_json = ?, reviewed_record_snapshot_sha256 = ?, notes = COALESCE(?, notes), reviewed_at = ?, updated_at = ? WHERE profile_id = ? AND id = ?",
        params![
            state,
            reconciliation,
            sql(&commercial)?,
            sql(&apply_snapshot)?,
            sql(&snapshot)?,
            sql(&sha)?,
            sql(&request.notes)?,
            sql(&reviewed_at)?,
            now,
            profile,
            link_ref
        ],
    )
    .map_err(error)?;
    for member_id in &batch {
        let member = link_row(conn, profile, member_id)?;
        let (member_snapshot, member_sha) =
            if truthy(&value(&member, "reviewed_record_snapshot_sha256")) {
                (
                    value(&member, "reviewed_record_snapshot_json"),
                    value(&member, "reviewed_record_snapshot_sha256"),
                )
            } else {
                record_snapshot(conn, &member)?
            };
        conn.execute(
            "UPDATE commercial_links SET state = 'reviewed', reconciliation_state = ?, commercial_kind = ?, reviewed_record_snapshot_json = ?, reviewed_record_snapshot_sha256 = ?, reviewed_at = ?, updated_at = ? WHERE profile_id = ? AND id = ?",
            params![
                reconciliation,
                sql(&commercial)?,
                sql(&member_snapshot)?,
                sql(&member_sha)?,
                now,
                now,
                profile,
                member_id
            ],
        )
        .map_err(error)?;
    }
    for member_id in &reopened {
        conn.execute(
            "UPDATE commercial_links SET state = ?, reconciliation_state = ?, updated_at = ? WHERE profile_id = ? AND id = ?",
            params![state, reconciliation, now, profile, member_id],
        )
        .map_err(error)?;
    }
    if applied || restored {
        crate::sql::invalidate(conn, profile)?;
    }
    let row = rows(
        conn,
        &format!("{LINK_SELECT} WHERE cl.profile_id=? AND cl.id=?"),
        [profile, link_ref.as_str()],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            format!("Commercial link '{link_ref}' not found"),
        )
    })?;
    let mut link = link_payload(&row)?;
    kassiber_envelope::set(&mut link, "applied_to_transaction", Value::Bool(applied));
    kassiber_envelope::set(&mut link, "restored_transaction", Value::Bool(restored));
    let ids = if batch.is_empty() { reopened } else { batch };
    kassiber_envelope::set(
        &mut link,
        "batch_link_ids",
        Value::List(ids.into_iter().map(s).collect()),
    );
    Ok(link)
}

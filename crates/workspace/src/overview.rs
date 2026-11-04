//! `build_overview_snapshot` and `build_workspace_overview_snapshot`: one
//! book's balances, valuations, series and readiness, and the book-set
//! roll-up. Display balances prefer observed chain inventory; book balances
//! remain the accounting fallback. Floats follow CPython's arithmetic order.
use std::collections::{BTreeMap, BTreeSet};

use kassiber_envelope::{dict, get, py_str, s, set, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params_from_iter, types::Value as SqlValue, Connection};
use kassiber_transactions::Scope;

use crate::readiness::{journal_freshness, profile_readiness, stored_int};
use crate::{count, py_int, rows, sql_error, text, val, Result};

const DISPLAY_BALANCE_ASSETS: [&str; 3] = ["BTC", "LBTC", "L-BTC"];
const BASIS_REASONS: [&str; 4] = [
    "journals_stale",
    "quarantines",
    "custody_unresolved",
    "missing_prices",
];
const COMPLETENESS_REASONS: [&str; 5] = [
    "journals_stale",
    "quarantines",
    "custody_unresolved",
    "missing_prices",
    "market_rate_missing",
];

/// The overview's clock: Python's `datetime.now` for relative labels.
pub struct Clock {
    /// Microseconds since 0001-01-01, as `kassiber_rates::time` counts them.
    pub now: i64,
}
impl Default for Clock {
    fn default() -> Self {
        Clock {
            now: kassiber_rates::time::now(),
        }
    }
}

/// `float(msat_to_btc(msat))`, correctly rounded like `float(Decimal)`.
fn btc(msat: i128) -> f64 {
    format!("{msat}e-11").parse().expect("finite msat")
}

/// `int(value)` for stored msat amounts (`int(row[...] or 0)`).
fn msat(value: &Value) -> Result<i128> {
    if !truthy(value) {
        return Ok(0);
    }
    let n = py_int(value)?;
    i128::try_from(n).map_err(|_| AppError::new("internal_error", "msat amount out of range"))
}

/// `float(dec(value))`: Python's `Decimal(str(value))` then `float()`.
fn dec_float(value: &Value) -> Result<f64> {
    Ok(match value {
        Value::Null => 0.0,
        Value::Float(x) => *x,
        Value::Int(n) => n.to_string().parse().unwrap_or(f64::INFINITY),
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Str(raw) => {
            let raw = raw.as_str().unwrap_or_default();
            if raw.is_empty() {
                return Ok(0.0);
            }
            let cleaned = kassiber_pytext::strip(raw).replace('_', "");
            cleaned
                .parse()
                .map_err(|_| AppError::new("app_error", format!("Invalid decimal value: {raw}")))?
        }
        other => {
            return Err(AppError::new(
                "app_error",
                format!("Invalid decimal value: {}", py_str(other)),
            ))
        }
    })
}

/// `float(value)` for a REAL/INTEGER/TEXT column.
fn float(value: &Value) -> Result<f64> {
    match value {
        Value::Float(x) => Ok(*x),
        Value::Int(n) => Ok(n.to_string().parse().unwrap_or(f64::INFINITY)),
        Value::Bool(b) => Ok(f64::from(u8::from(*b))),
        Value::Str(raw) => {
            kassiber_pytext::parse_float(raw.as_str().unwrap_or_default()).map_err(|_| {
                AppError::new(
                    "internal_error",
                    format!(
                        "could not convert string to float: {}",
                        kassiber_envelope::py_repr(value)
                    ),
                )
            })
        }
        _ => Err(AppError::new(
            "internal_error",
            "float() argument must be a string or a real number, not 'NoneType'",
        )),
    }
}

/// `btc_to_msat(x)`: `Decimal(repr(x)) * 10**11`, rounded half up.
fn btc_to_msat(x: f64) -> i128 {
    let repr = kassiber_pyjson::float_repr(x);
    let (mantissa, exponent) = match repr.split_once('e') {
        Some((m, e)) => (m.to_owned(), e.parse::<i32>().unwrap_or(0)),
        None => (repr.clone(), 0),
    };
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches('-');
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits: String = format!("{whole}{fraction}");
    let shift = exponent + 11 - fraction.len() as i32;
    let value: i128 = digits.parse().unwrap_or(0);
    let magnitude = if shift >= 0 {
        value.saturating_mul(10i128.saturating_pow(shift as u32))
    } else {
        let divisor = 10i128.saturating_pow((-shift) as u32);
        let (quotient, remainder) = (value / divisor, value % divisor);
        quotient + i128::from(remainder * 2 >= divisor)
    };
    if negative {
        -magnitude
    } else {
        magnitude
    }
}

/// `json.loads(value or "{}")` when it parses to an object, else `{}`.
fn json_config(value: &Value) -> Value {
    let empty = dict::<&str>([]);
    let raw = match value {
        Value::Str(text) if !text.is("") => text.as_str().unwrap_or("{}"),
        Value::Null | Value::Str(_) => "{}",
        _ => return empty,
    };
    kassiber_pytext::loads(raw)
        .ok()
        .filter(|v| matches!(v, Value::Dict(_)))
        .unwrap_or(empty)
}

/// `_string_or_empty(value)`.
fn string_or_empty(value: Option<&Value>) -> String {
    match value {
        Some(Value::Str(text)) => {
            kassiber_pytext::strip(text.as_str().unwrap_or_default()).to_owned()
        }
        _ => String::new(),
    }
}

/// `str_or_none(value)`.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(value) => {
            let text = kassiber_pytext::strip(&py_str(value)).to_owned();
            (!text.is_empty()).then_some(text)
        }
    }
}

fn chain_alias(chain: &str) -> Option<&'static str> {
    match chain {
        "bitcoin" | "btc" | "onchain" => Some("bitcoin"),
        "liquid" | "lbtc" | "elements" => Some("liquid"),
        _ => None,
    }
}
/// `normalize_chain(value)`.
fn normalize_chain(value: &Value) -> Option<String> {
    let raw = if truthy(value) {
        py_str(value)
    } else {
        "bitcoin".into()
    };
    let chain = kassiber_pytext::lower(kassiber_pytext::strip(&raw));
    if chain.is_empty() {
        return Some("bitcoin".into());
    }
    chain_alias(&chain).map(str::to_owned)
}
const BITCOIN_NETWORKS: [(&str, &str); 7] = [
    ("main", "main"),
    ("mainnet", "main"),
    ("bitcoin", "main"),
    ("test", "test"),
    ("testnet", "test"),
    ("regtest", "regtest"),
    ("signet", "signet"),
];
const LIQUID_NETWORKS: [(&str, &str); 10] = [
    ("liquid", "liquidv1"),
    ("liquidv1", "liquidv1"),
    ("main", "liquidv1"),
    ("mainnet", "liquidv1"),
    ("liquidtestnet", "liquidtestnet"),
    ("test", "liquidtestnet"),
    ("testnet", "liquidtestnet"),
    ("elements", "elementsregtest"),
    ("elementsregtest", "elementsregtest"),
    ("regtest", "elementsregtest"),
];
/// `normalize_network(chain, value)`.
fn normalize_network(chain: &Value, value: &Value) -> Option<String> {
    let chain = normalize_chain(chain)?;
    let (aliases, default): (&[(&str, &str)], &str) = if chain == "bitcoin" {
        (&BITCOIN_NETWORKS, "main")
    } else {
        (&LIQUID_NETWORKS, "liquidv1")
    };
    let raw = if truthy(value) {
        py_str(value)
    } else {
        default.into()
    };
    let mut network = kassiber_pytext::lower(kassiber_pytext::strip(&raw));
    if network.is_empty() {
        network = default.into();
    }
    aliases
        .iter()
        .find(|(alias, _)| *alias == network)
        .map(|(_, target)| (*target).to_owned())
}

/// `_unique_text_values(values)`.
fn unique(values: &[Option<&Value>]) -> Vec<String> {
    let mut output = Vec::new();
    for value in values {
        let text = string_or_empty(*value);
        if !text.is_empty() && !output.contains(&text) {
            output.push(text);
        }
    }
    output
}
fn chain_filter_values(value: &Value) -> Vec<String> {
    match normalize_chain(value) {
        None => unique(&[Some(value)]),
        Some(canonical) => {
            let aliases: Vec<Value> = ["bitcoin", "btc", "onchain", "liquid", "lbtc", "elements"]
                .into_iter()
                .filter(|alias| chain_alias(alias) == Some(canonical.as_str()))
                .map(s)
                .collect();
            let canonical = s(canonical);
            let mut items = vec![Some(&canonical), Some(value)];
            items.extend(aliases.iter().map(Some));
            unique(&items)
        }
    }
}
fn network_filter_values(chain: &str, value: &Value) -> Vec<String> {
    match normalize_network(&s(chain), value) {
        None => unique(&[Some(value)]),
        Some(canonical) => {
            let map: &[(&str, &str)] = if chain == "bitcoin" {
                &BITCOIN_NETWORKS
            } else {
                &LIQUID_NETWORKS
            };
            let aliases: Vec<Value> = map
                .iter()
                .filter(|(_, target)| *target == canonical)
                .map(|(alias, _)| s(*alias))
                .collect();
            let canonical = s(canonical);
            let mut items = vec![Some(&canonical), Some(value)];
            items.extend(aliases.iter().map(Some));
            unique(&items)
        }
    }
}

/// `has_descriptor_sync_material(config)`.
fn has_descriptor(config: &Value) -> bool {
    str_or_none(get(config, "descriptor")).is_some()
        || (str_or_none(get(config, "xpub")).is_some()
            && get(config, "script_types").is_some_and(truthy))
}

struct BackendSummary {
    name: String,
    sync_mode: &'static str,
}
/// `_wallet_backend_summary(kind, config, default_backend)`.
fn backend_summary(kind: &str, config: &Value, default_backend: Option<&str>) -> BackendSummary {
    let explicit = string_or_empty(get(config, "backend"));
    let default = default_backend
        .map(|v| kassiber_pytext::strip(v).to_owned())
        .unwrap_or_default();
    let source_file = string_or_empty(get(config, "source_file"));
    let source_format = string_or_empty(get(config, "source_format"));
    let sync_source = string_or_empty(get(config, "sync_source"));
    let mut name = explicit;
    let mut sync_mode = "not_configured";
    if sync_source == "btcpay" {
        sync_mode = "btcpay";
    } else if !source_file.is_empty() && !source_format.is_empty() {
        sync_mode = "file_import";
    } else if str_or_none(get(config, "sp_descriptor")).is_some() && kind == "silent-payment" {
        sync_mode = "backend_silent_payment";
    } else if has_descriptor(config) && ["descriptor", "xpub", "address"].contains(&kind) {
        sync_mode = "backend_descriptor";
        if name.is_empty() {
            name = default.clone();
        }
    } else if get(config, "addresses").is_some_and(truthy) && kind == "address" {
        sync_mode = "backend_addresses";
        if name.is_empty() {
            name = default;
        }
    }
    BackendSummary { name, sync_mode }
}

/// `_db_backend_for_wallet_balance(conn, name)` merged with its config.
fn backend_row(conn: &Connection, name: &str) -> Result<Option<Value>> {
    if name.is_empty() {
        return Ok(None);
    }
    let Some(row) = rows(
        conn,
        "SELECT name, kind, chain, network, batch_size, timeout, config_json FROM backends WHERE name = ?",
        [name],
    )?
    .into_iter()
    .next() else {
        return Ok(None);
    };
    let mut backend = dict(
        ["name", "kind", "chain", "network", "batch_size", "timeout"]
            .map(|key| (key, val(&row, key))),
    );
    if let Value::Dict(extra) = json_config(&val(&row, "config_json")) {
        for (key, value) in extra {
            if let kassiber_pyjson::Key::Str(key) = &key {
                set(&mut backend, &py_str(&Value::Str(key.clone())), value);
            }
        }
    }
    Ok(Some(backend))
}

/// `normalize_backend_kind(kind)`.
fn backend_kind_alias(kind: &str) -> String {
    let value = kassiber_pytext::lower(kassiber_pytext::strip(kind));
    match value.as_str() {
        "bitcoin-core" | "bitcoincore" | "core" => "bitcoinrpc".into(),
        "core-ln" | "core-lightning" => "coreln".into(),
        "liquid-esplora" | "mempool" => "esplora".into(),
        _ => value,
    }
}

/// `_wallet_utxo_source_filter`: `(column, values)`; `None` skips a column.
type SourceFilter = Vec<(&'static str, Vec<String>)>;
fn source_filter(
    config: &Value,
    summary: &BackendSummary,
    backend: Option<&Value>,
) -> SourceFilter {
    if string_or_empty(get(config, "source_format")) == "wasabi_bundle" {
        let chain_raw = val(config, "chain");
        let chains = chain_filter_values(if truthy(&chain_raw) {
            &chain_raw
        } else {
            &Value::Null
        });
        let chains = if truthy(&chain_raw) {
            chains
        } else {
            chain_filter_values(&s("bitcoin"))
        };
        let network_raw = val(config, "network");
        let network = if truthy(&network_raw) {
            network_raw
        } else {
            s("mainnet")
        };
        let networks =
            network_filter_values(chains.first().map_or("bitcoin", String::as_str), &network);
        return vec![
            ("backend_name", vec!["wasabi".into()]),
            ("backend_kind", vec!["wasabi_bundle".into()]),
            ("chain", chains),
            ("network", networks),
        ];
    }
    let backend_value = |key: &str| backend.and_then(|b| str_or_none(get(b, key)));
    let chain_source = backend_value("chain")
        .map(s)
        .unwrap_or_else(|| val(config, "chain"));
    let chains = chain_filter_values(&chain_source);
    let chain = chains
        .first()
        .cloned()
        .unwrap_or_else(|| string_or_empty(Some(&chain_source)));
    let network_source = backend_value("network")
        .map(s)
        .unwrap_or_else(|| val(config, "network"));
    let networks = network_filter_values(
        if chain.is_empty() { "bitcoin" } else { &chain },
        &network_source,
    );
    let kind = backend_value("kind");
    let kinds = match &kind {
        Some(kind) if !string_or_empty(Some(&s(kind.as_str()))).is_empty() => {
            let normalized = s(backend_kind_alias(kind));
            let raw = s(kind.as_str());
            unique(&[Some(&normalized), Some(&raw)])
        }
        _ => Vec::new(),
    };
    let summary_name = s(summary.name.as_str());
    let backend_name = backend.map(|b| val(b, "name"));
    let names = unique(&[Some(&summary_name), backend_name.as_ref()]);
    vec![
        ("backend_name", names),
        ("backend_kind", kinds),
        ("chain", chains),
        ("network", networks),
    ]
}
/// `_source_filter_sql`: `_filter_values` drops blanks and duplicates.
fn filter_sql(filter: &SourceFilter, params: &mut Vec<SqlValue>) -> String {
    let mut clauses = Vec::new();
    for (column, values) in filter {
        let mut normalized: Vec<String> = Vec::new();
        for value in values {
            let text = kassiber_pytext::strip(value).to_owned();
            if !text.is_empty() && !normalized.contains(&text) {
                normalized.push(text);
            }
        }
        if normalized.is_empty() {
            continue;
        }
        if normalized.len() == 1 {
            clauses.push(format!("{column} = ?"));
        } else {
            clauses.push(format!(
                "{column} IN ({})",
                vec!["?"; normalized.len()].join(", ")
            ));
        }
        params.extend(normalized.into_iter().map(SqlValue::Text));
    }
    if clauses.is_empty() {
        String::new()
    } else {
        format!(" AND {}", clauses.join(" AND "))
    }
}

#[derive(Clone)]
struct Balance {
    balance: Value,
    source: &'static str,
    book: f64,
    chain: Value,
    last_seen: Value,
    active: Value,
}

fn book_delta(entry_type: &str, quantity: &Value) -> Result<i128> {
    if ["income", "transfer_fee"].contains(&entry_type) {
        return Ok(0);
    }
    if *quantity == Value::Null {
        return Err(AppError::new(
            "internal_error",
            "unsupported operand type(s) for +=: 'decimal.Decimal' and 'NoneType'",
        ));
    }
    msat(quantity)
}

fn canonical_processed(conn: &Connection, profile: &str) -> Result<bool> {
    let present: Option<i64> = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM journal_quantity_postings WHERE profile_id = ?) \
             OR EXISTS(SELECT 1 FROM journal_quantity_issues WHERE profile_id = ?) AS present",
            [profile, profile],
            |r| r.get(0),
        )
        .ok();
    Ok(present.is_some_and(|p| p != 0))
}

/// `_wallet_balances`: canonical quantity balances, then journal holdings,
/// then journal entries, then raw transactions. Keys keep Python's order.
fn wallet_balances(
    conn: &Connection,
    profile: &str,
    has_book_state: bool,
) -> Result<Vec<(Value, f64)>> {
    if has_book_state {
        if canonical_processed(conn, profile)? {
            return rows(
                conn,
                "SELECT location_id AS wallet_id, SUM(amount_msat) AS amount_msat FROM journal_quantity_balances \
                 WHERE profile_id = ? AND location_kind = 'wallet' AND asset IN ('BTC', 'LBTC') GROUP BY location_id",
                [profile],
            )?
            .iter()
            .map(|row| Ok((s(text(row, "wallet_id")), btc(msat(&val(row, "amount_msat"))?))))
            .collect();
        }
        let holdings = rows(
            conn,
            "SELECT wallet_id, SUM(quantity) AS quantity FROM journal_wallet_holdings \
             WHERE profile_id = ? AND asset IN ('BTC', 'LBTC') GROUP BY wallet_id",
            [profile],
        )
        .unwrap_or_default();
        if !holdings.is_empty() {
            return holdings
                .iter()
                .map(|row| Ok((val(row, "wallet_id"), btc(msat(&val(row, "quantity"))?))))
                .collect();
        }
        let mut balances: Vec<(Value, i128)> = Vec::new();
        for row in rows(
            conn,
            "SELECT wallet_id, entry_type, quantity FROM journal_entries \
             WHERE profile_id = ? AND asset IN ('BTC', 'LBTC') ORDER BY occurred_at ASC, created_at ASC, id ASC",
            [profile],
        )? {
            let delta = book_delta(&text(&row, "entry_type"), &val(&row, "quantity"))?;
            let key = val(&row, "wallet_id");
            match balances.iter_mut().find(|(k, _)| *k == key) {
                Some((_, total)) => *total += delta,
                None => balances.push((key, delta)),
            }
        }
        return Ok(balances.into_iter().map(|(k, v)| (k, btc(v))).collect());
    }
    rows(
        conn,
        "SELECT wallet_id, SUM(CASE WHEN direction = 'inbound' THEN amount WHEN direction = 'outbound' THEN -amount - fee ELSE 0 END) AS quantity \
         FROM transactions WHERE profile_id = ? AND excluded = 0 AND asset IN ('BTC', 'LBTC') GROUP BY wallet_id",
        [profile],
    )?
    .iter()
    .map(|row| Ok((val(row, "wallet_id"), btc(msat(&val(row, "quantity"))?))))
    .collect()
}

fn default_backend(conn: &Connection) -> Result<Option<String>> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'default_backend'",
            [],
            |r| r.get(0),
        )
        .ok();
    Ok(value
        .map(|v| kassiber_pytext::strip(&v).to_owned())
        .filter(|v| !v.is_empty()))
}

/// `_display_wallet_balances`, in the wallets table's scan order.
fn display_balances(
    conn: &Connection,
    profile: &str,
    book: &[(Value, f64)],
    fallback: &'static str,
) -> Result<Vec<(String, Balance)>> {
    let default = default_backend(conn)?;
    let mut output = Vec::new();
    for row in rows(
        conn,
        "SELECT id, kind, config_json FROM wallets WHERE profile_id = ?",
        [profile],
    )? {
        let id = text(&row, "id");
        let book_balance = book
            .iter()
            .find(|(key, _)| *key == s(id.as_str()))
            .map_or(0.0, |(_, v)| *v);
        let config = json_config(&val(&row, "config_json"));
        let chain = chain_balance(conn, &row, &config, default.as_deref())?;
        output.push((
            id,
            match chain {
                None => Balance {
                    balance: Value::Float(book_balance),
                    source: fallback,
                    book: book_balance,
                    chain: Value::Null,
                    last_seen: Value::Null,
                    active: Value::Null,
                },
                Some((balance, last_seen, active)) => Balance {
                    balance: balance.clone(),
                    source: "chain",
                    book: book_balance,
                    chain: balance,
                    last_seen,
                    active,
                },
            },
        ));
    }
    Ok(output)
}

/// `_chain_balance_for_wallet`: observed unspent inventory, when the source
/// has refreshed it at least once.
fn chain_balance(
    conn: &Connection,
    row: &Value,
    config: &Value,
    default: Option<&str>,
) -> Result<Option<(Value, Value, Value)>> {
    let summary = backend_summary(&text(row, "kind"), config, default);
    if string_or_empty(get(config, "source_format")) != "wasabi_bundle"
        && ![
            "backend_descriptor",
            "backend_addresses",
            "backend_silent_payment",
        ]
        .contains(&summary.sync_mode)
    {
        return Ok(None);
    }
    let backend = backend_row(conn, &summary.name)?;
    let filter = source_filter(config, &summary, backend.as_ref());
    let id = text(row, "id");
    let mut params = vec![SqlValue::Text(id.clone())];
    let clause = filter_sql(&filter, &mut params);
    let active = rows(
        conn,
        &format!("SELECT COUNT(*) AS active_count, MAX(last_seen_at) AS active_last_seen_at FROM wallet_utxos WHERE wallet_id = ? AND spent_at IS NULL {clause}"),
        params_from_iter(params.clone()),
    )?;
    let refresh = rows(
        conn,
        &format!("SELECT observed_count, active_count, last_seen_at FROM wallet_utxo_refreshes WHERE wallet_id = ? {clause}"),
        params_from_iter(params.clone()),
    )?;
    let active = active.into_iter().next();
    let last_seen = match refresh.first() {
        Some(refresh) => val(refresh, "last_seen_at"),
        None => active
            .as_ref()
            .map_or(Value::Null, |row| val(row, "active_last_seen_at")),
    };
    if !truthy(&last_seen) {
        return Ok(None);
    }
    let active_count = active.as_ref().map_or(0, |row| count(row, "active_count"));
    let totals = rows(
        conn,
        &format!("SELECT asset, SUM(amount) AS amount_msat FROM wallet_utxos WHERE wallet_id = ? AND spent_at IS NULL {clause} GROUP BY asset ORDER BY asset ASC"),
        params_from_iter(params),
    )?;
    // `sum()` of no floats is the integer 0.
    let mut balance: Option<f64> = None;
    for total in &totals {
        let asset = kassiber_pytext::upper(&text(total, "asset"));
        if DISPLAY_BALANCE_ASSETS.contains(&asset.as_str()) {
            let amount = btc(msat(&val(total, "amount_msat"))?);
            balance = Some(balance.unwrap_or(0.0) + amount);
        }
    }
    Ok(Some((
        balance.map_or(Value::Int(0.into()), Value::Float),
        last_seen,
        Value::Int(active_count.into()),
    )))
}

/// `_chain_duplicate_outpoint_adjustment_btc`.
fn duplicate_adjustment(
    conn: &Connection,
    profile: &str,
    balances: &[(String, Balance)],
) -> Result<f64> {
    let mut chain_ids: Vec<&str> = balances
        .iter()
        .filter(|(_, b)| b.source == "chain")
        .map(|(id, _)| id.as_str())
        .collect();
    chain_ids.sort();
    if chain_ids.len() < 2 {
        return Ok(0.0);
    }
    let default = default_backend(conn)?;
    let mut params: Vec<SqlValue> = vec![SqlValue::Text(profile.into())];
    params.extend(chain_ids.iter().map(|id| SqlValue::Text((*id).into())));
    let wallets = rows(
        conn,
        &format!(
            "SELECT id, kind, config_json FROM wallets WHERE profile_id = ? AND id IN ({})",
            vec!["?"; chain_ids.len()].join(", ")
        ),
        params_from_iter(params),
    )?;
    let mut amounts: Vec<((String, String), Vec<i128>)> = Vec::new();
    for row in wallets {
        let config = json_config(&val(&row, "config_json"));
        let summary = backend_summary(&text(&row, "kind"), &config, default.as_deref());
        let backend = backend_row(conn, &summary.name)?;
        let filter = source_filter(&config, &summary, backend.as_ref());
        let mut params = vec![SqlValue::Text(text(&row, "id"))];
        params.extend(["BTC", "L-BTC", "LBTC"].map(|a| SqlValue::Text(a.into())));
        let clause = filter_sql(&filter, &mut params);
        for entry in rows(
            conn,
            &format!("SELECT UPPER(asset) AS asset, COALESCE(NULLIF(outpoint, ''), lower(txid) || ':' || vout) AS outpoint_key, SUM(amount) AS amount_msat FROM wallet_utxos WHERE wallet_id = ? AND spent_at IS NULL AND UPPER(asset) IN (?, ?, ?) {clause} GROUP BY UPPER(asset), outpoint_key"),
            params_from_iter(params),
        )? {
            let key = (text(&entry, "asset"), text(&entry, "outpoint_key"));
            let amount = msat(&val(&entry, "amount_msat"))?;
            match amounts.iter_mut().find(|(k, _)| *k == key) {
                Some((_, list)) => list.push(amount),
                None => amounts.push((key, vec![amount])),
            }
        }
    }
    let mut duplicate = 0i128;
    for (_, list) in amounts {
        if list.len() < 2 {
            continue;
        }
        let sum: i128 = list.iter().sum();
        duplicate += (sum - list.iter().max().unwrap()).max(0);
    }
    Ok(btc(duplicate))
}

/// `_latest_rate_row(conn, pair)`.
fn latest_rate(conn: &Connection, pair: &str) -> Result<Option<Value>> {
    Ok(rows(
        conn,
        "SELECT pair, timestamp, rate, source, fetched_at, granularity, method FROM rates_cache WHERE pair = ? \
         ORDER BY timestamp DESC, CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC, fetched_at DESC, source ASC LIMIT 1",
        [pair],
    )?
    .into_iter()
    .next())
}
/// `_latest_transaction_rate(conn, profile_id, fiat)`.
fn latest_transaction_rate(conn: &Connection, profile: &str, fiat: &str) -> Result<f64> {
    match rows(
        conn,
        "SELECT fiat_rate FROM transactions WHERE profile_id = ? AND excluded = 0 AND asset = 'BTC' AND fiat_currency = ? \
         AND fiat_rate IS NOT NULL ORDER BY occurred_at DESC, created_at DESC, id DESC LIMIT 1",
        [profile, fiat],
    )?
    .first()
    {
        Some(row) => float(&val(row, "fiat_rate")),
        None => Ok(0.0),
    }
}
/// `_latest_rate(pair) or _latest_transaction_rate(...)`: a zero cached
/// rate falls back to the transaction rate.
fn price(conn: &Connection, pair: &str, profile: &str, fiat: &str) -> Result<f64> {
    let cached = match latest_rate(conn, pair)? {
        Some(row) => float(&val(&row, "rate"))?,
        None => 0.0,
    };
    if cached != 0.0 && !cached.is_nan() {
        return Ok(cached);
    }
    if cached.is_nan() {
        return Ok(cached);
    }
    latest_transaction_rate(conn, profile, fiat)
}

/// `_market_rate_snapshot(conn, fiat_currency)`.
fn market_rate(conn: &Connection, fiat: &Value) -> Result<(Value, Option<f64>, String)> {
    let raw = if truthy(fiat) {
        py_str(fiat)
    } else {
        "EUR".into()
    };
    let mut code = kassiber_pytext::upper(kassiber_pytext::strip(&raw));
    if code.is_empty() {
        code = "EUR".into();
    }
    let pair = kassiber_rates::transaction_rate_pair("BTC", &code);
    let row = match &pair {
        Some(pair) => latest_rate(conn, pair)?,
        None => None,
    };
    let rate = row
        .as_ref()
        .map(|row| float(&val(row, "rate")))
        .transpose()?;
    let field = |key: &str| row.as_ref().map_or(Value::Null, |row| val(row, key));
    Ok((
        dict([
            ("asset", s("BTC")),
            ("fiatCurrency", s(code.as_str())),
            ("pair", pair.clone().map_or(Value::Null, s)),
            ("rate", rate.map_or(Value::Null, Value::Float)),
            ("timestamp", field("timestamp")),
            ("source", field("source")),
            ("fetchedAt", field("fetched_at")),
            ("granularity", field("granularity")),
            ("method", field("method")),
        ]),
        rate,
        code,
    ))
}

/// `report_builders._holdings_basis_delta` as `float()`.
fn basis_delta(row: &Value) -> Result<f64> {
    let kind = text(row, "entry_type");
    if kind == "income" {
        return Ok(0.0);
    }
    let quantity = val(row, "quantity");
    if quantity == Value::Null {
        return Err(AppError::new(
            "internal_error",
            "'>=' not supported between instances of 'NoneType' and 'int'",
        ));
    }
    if msat(&quantity)? >= 0 {
        dec_float(&val(row, "fiat_value"))
    } else {
        Ok(-dec_float(&val(row, "cost_basis"))?)
    }
}
const BASIS_SQL: &str = "FROM journal_entries WHERE profile_id = ? AND asset IN ('BTC', 'LBTC') ORDER BY occurred_at ASC, created_at ASC, id ASC";

/// `_current_portfolio_cost_basis`.
fn current_cost_basis(conn: &Connection, profile: &str) -> Result<f64> {
    let mut total = 0.0;
    for row in rows(
        conn,
        &format!("SELECT entry_type, quantity, fiat_value, cost_basis {BASIS_SQL}"),
        [profile],
    )? {
        total += basis_delta(&row)?;
    }
    Ok(total)
}

/// `_fiat_snapshot(...)`.
fn fiat_snapshot(
    conn: &Connection,
    profile: &str,
    fiat_currency: &Value,
    rate: f64,
    total: f64,
    adjustment: f64,
) -> Result<Value> {
    let market = total * rate;
    let realized = rows(
        conn,
        "SELECT SUM(COALESCE(gain_loss, 0)) AS gain_loss FROM (SELECT gain_loss FROM journal_entries WHERE profile_id = ? \
         AND gain_loss IS NOT NULL AND occurred_at >= date('now', 'start of year') ORDER BY occurred_at, created_at, id)",
        [profile],
    )?;
    let realized = val(&realized[0], "gain_loss");
    let realized = if truthy(&realized) {
        float(&realized)?
    } else {
        0.0
    };
    let basis = current_cost_basis(conn, profile)?;
    let code = if truthy(fiat_currency) {
        kassiber_pytext::upper(&py_str(fiat_currency))
    } else {
        "EUR".into()
    };
    let mut payload = dict([
        ("fiatCurrency", s(code)),
        ("eurBalance", Value::Float(market)),
        ("eurCostBasis", Value::Float(basis)),
        ("eurUnrealized", Value::Float(market - basis)),
        ("eurRealizedYTD", Value::Float(realized)),
    ]);
    if adjustment > 0.0 {
        set(
            &mut payload,
            "chainDuplicateOutpointAdjustmentBtc",
            Value::Float(adjustment),
        );
    }
    Ok(payload)
}

/// `_unavailable_fiat_completeness()`.
pub fn unavailable_completeness() -> Value {
    dict([
        ("state", s("unavailable")),
        ("costBasisComplete", Value::Bool(false)),
        ("reasons", Value::List(vec![])),
        ("quarantineCount", 0.into()),
        ("quarantinedInboundMsat", 0.into()),
        ("quarantinedOutboundMsat", 0.into()),
        ("basisCoveredMsat", Value::Null),
        ("basisUncoveredMsat", Value::Null),
        ("earliestIncompleteAt", Value::Null),
        ("missingPriceCount", 0.into()),
        ("marketRateMissing", Value::Bool(false)),
    ])
}

/// `custody_quantity_readiness_summary(...)`: `(issue_count, blocked_from)`;
/// an unreadable issue table counts as one unresolved gap.
fn custody_issues(conn: &Connection, profile: &str) -> (i64, Value) {
    match rows(
        conn,
        "SELECT issue_id, issue_type, state, asset, amount_msat, occurred_at, reason, blocks_from \
         FROM journal_quantity_issues WHERE profile_id = ? ORDER BY COALESCE(occurred_at, ''), issue_id",
        [profile],
    ) {
        Err(_) => (1, Value::Null),
        Ok(issues) => {
            let blocked = issues
                .iter()
                .filter(|issue| truthy(&val(issue, "blocks_from")))
                .map(|issue| py_str(&val(issue, "blocks_from")))
                .min();
            (issues.len() as i64, blocked.map_or(Value::Null, s))
        }
    }
}

/// `_journal_basis_covered_msat`.
fn basis_covered(conn: &Connection, profile: &str) -> Result<i128> {
    if let Ok(holdings) = rows(
        conn,
        "SELECT COUNT(*) AS row_count, SUM(quantity) AS quantity FROM journal_wallet_holdings \
         WHERE profile_id = ? AND asset IN ('BTC', 'LBTC')",
        [profile],
    ) {
        if let Some(row) = holdings.first() {
            if count(row, "row_count") > 0 {
                return Ok(msat(&val(row, "quantity"))?.max(0));
            }
        }
    }
    let mut total = 0i128;
    for row in rows(
        conn,
        "SELECT entry_type, quantity FROM journal_entries WHERE profile_id = ? AND asset IN ('BTC', 'LBTC')",
        [profile],
    )? {
        if !["income", "transfer_fee"].contains(&text(&row, "entry_type").as_str()) {
            total += msat(&val(&row, "quantity"))?;
        }
    }
    Ok(total.max(0))
}

/// `_overview_fiat_completeness(...)`.
fn completeness(
    conn: &Connection,
    profile: &str,
    freshness: &Value,
    display_btc: f64,
    rate_missing: bool,
) -> Result<Value> {
    let stale = truthy(&val(freshness, "needs_processing"));
    let quarantines = count(freshness, "quarantine_count");
    let (inbound, outbound, earliest_quarantine) = if quarantines != 0 {
        let row = rows(
            conn,
            "SELECT COALESCE(SUM(CASE WHEN t.excluded = 0 AND t.asset IN ('BTC', 'LBTC') AND t.direction = 'inbound' \
             THEN ABS(COALESCE(t.amount, 0)) ELSE 0 END), 0) AS inbound_msat, \
             COALESCE(SUM(CASE WHEN t.excluded = 0 AND t.asset IN ('BTC', 'LBTC') AND t.direction = 'outbound' \
             THEN ABS(COALESCE(t.amount, 0)) + ABS(COALESCE(t.fee, 0)) ELSE 0 END), 0) AS outbound_msat, \
             MIN(t.occurred_at) AS earliest_at FROM journal_quarantines q JOIN transactions t ON t.id = q.transaction_id \
             WHERE q.profile_id = ?",
            [profile],
        )?;
        let row = &row[0];
        (
            msat(&val(row, "inbound_msat"))?,
            msat(&val(row, "outbound_msat"))?,
            val(row, "earliest_at"),
        )
    } else {
        (0, 0, Value::Null)
    };
    let (custody_count, blocked_from) = custody_issues(conn, profile);
    let missing = rows(
        conn,
        &format!(
            "SELECT COUNT(*) AS count, MIN(t.occurred_at) AS earliest_at FROM transactions t WHERE t.profile_id = ? \
             AND t.excluded = 0 AND (t.amount > 0 OR t.fee > 0) AND {}",
            kassiber_rates::missing_price_sql("t.")
        ),
        [profile],
    )?;
    let missing_count = count(&missing[0], "count");
    let missing_earliest = val(&missing[0], "earliest_at");
    let flags = [
        ("journals_stale", stale),
        ("quarantines", quarantines > 0),
        ("custody_unresolved", custody_count > 0),
        ("missing_prices", missing_count > 0),
        ("market_rate_missing", rate_missing),
    ];
    let flag = |name: &str| flags.iter().any(|(n, f)| *n == name && *f);
    let reasons: Vec<Value> = COMPLETENESS_REASONS
        .iter()
        .filter(|r| flag(r))
        .map(|r| s(*r))
        .collect();
    let complete = !BASIS_REASONS.iter().any(|r| flag(r));
    let state = if stale {
        "stale"
    } else if !complete {
        "incomplete"
    } else if rate_missing {
        "unavailable"
    } else {
        "complete"
    };
    let covered = if stale {
        None
    } else {
        Some(basis_covered(conn, profile)?)
    };
    let display = btc_to_msat(display_btc).max(0);
    let uncovered = covered.map(|c| (display - c).max(0));
    let mut earliest = Value::Null;
    if !stale && !complete {
        let starts: Vec<&Value> = [
            (flag("quarantines"), &earliest_quarantine),
            (flag("custody_unresolved"), &blocked_from),
            (flag("missing_prices"), &missing_earliest),
        ]
        .into_iter()
        .filter(|(active, _)| *active)
        .map(|(_, start)| start)
        .collect();
        if !starts.is_empty() && starts.iter().all(|s| truthy(s)) {
            earliest = s(starts.iter().map(|v| py_str(v)).min().unwrap());
        }
    }
    let int = |v: i128| Value::Int(v.into());
    Ok(dict([
        ("state", s(state)),
        ("costBasisComplete", Value::Bool(complete)),
        ("reasons", Value::List(reasons)),
        ("quarantineCount", quarantines.into()),
        ("quarantinedInboundMsat", int(inbound)),
        ("quarantinedOutboundMsat", int(outbound)),
        ("basisCoveredMsat", covered.map_or(Value::Null, int)),
        ("basisUncoveredMsat", uncovered.map_or(Value::Null, int)),
        ("earliestIncompleteAt", earliest),
        ("missingPriceCount", missing_count.into()),
        ("marketRateMissing", Value::Bool(rate_missing)),
    ]))
}

/// `_relative_last(value)`.
fn relative_last(value: &Value, clock: &Clock) -> Value {
    if !truthy(value) {
        return s("never");
    }
    let raw = py_str(value);
    let Some(at) = kassiber_rates::time::parse(&raw.replace('Z', "+00:00")) else {
        return value.clone();
    };
    let seconds = ((clock.now - at).div_euclid(1_000_000)).max(0);
    if seconds < 60 {
        return s("just now");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return s(format!("{minutes}m ago"));
    }
    let hours = minutes / 60;
    if hours < 24 {
        return s(format!("{hours}h ago"));
    }
    let days = hours / 24;
    if days < 30 {
        return s(format!("{days}d ago"));
    }
    // `dt.date()` is the spelled calendar date, before any offset applies.
    let spelled: String = raw.chars().take(10).collect();
    if spelled.len() == 10 && spelled.as_bytes()[4] == b'-' && spelled.as_bytes()[7] == b'-' {
        s(spelled)
    } else {
        s(kassiber_rates::time::format(at).unwrap_or_default()[..10].to_owned())
    }
}

/// `core_import_batches.observed_assets(conn, profile_id)`.
fn observed_assets(conn: &Connection, profile: &str) -> Result<BTreeMap<String, Vec<Value>>> {
    let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in rows(
        conn,
        "SELECT wallet_id, asset, COUNT(*) AS n FROM transactions WHERE profile_id = ? AND wallet_id IS NOT NULL \
         AND asset IS NOT NULL AND asset != '' GROUP BY wallet_id, asset ORDER BY wallet_id, n DESC, asset ASC",
        [profile],
    )? {
        grouped
            .entry(text(&row, "wallet_id"))
            .or_default()
            .push(s(kassiber_pytext::upper(&text(&row, "asset"))));
    }
    Ok(grouped)
}

/// `_import_summary_by_wallet`: `(batch_count, imported_at, source_format)`.
fn import_summary(
    conn: &Connection,
    profile: &str,
) -> Result<BTreeMap<String, (i64, Value, String)>> {
    let mut summary: BTreeMap<String, (i64, Value, String)> = BTreeMap::new();
    for row in rows(
        conn,
        "SELECT wallet_id, source_format, imported_at FROM import_batches WHERE profile_id = ? AND wallet_id IS NOT NULL \
         ORDER BY wallet_id ASC, imported_at ASC, id ASC",
        [profile],
    )? {
        let entry = summary
            .entry(text(&row, "wallet_id"))
            .or_insert((0, Value::Null, String::new()));
        entry.0 += 1;
        entry.1 = val(&row, "imported_at");
        entry.2 = text(&row, "source_format");
    }
    Ok(summary)
}

/// `parse_bool(value, default=False)`.
fn parse_bool(value: Option<&Value>) -> Result<bool> {
    let Some(value) = value.filter(|v| **v != Value::Null) else {
        return Ok(false);
    };
    let text = kassiber_pytext::lower(kassiber_pytext::strip(&py_str(value)));
    match text.as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(AppError::new(
            "app_error",
            format!("Invalid boolean value: {}", py_str(value)),
        )),
    }
}

/// `_map_wallet_kind(kind)`.
fn map_kind(kind: &str) -> String {
    let normalized = kassiber_pytext::lower(kind).replace('_', "-");
    match normalized.as_str() {
        "coreln" => "core-ln".into(),
        "btcpay-json" | "btcpay-csv" => "btcpay".into(),
        "phoenix-csv" => "csv".into(),
        "" => "csv".into(),
        _ => normalized,
    }
}

/// `_connections(conn, profile_id, balances)`.
fn connections(
    conn: &Connection,
    profile: &str,
    balances: &[(String, Balance)],
    clock: &Clock,
) -> Result<Vec<Value>> {
    let assets = observed_assets(conn, profile)?;
    let imports = import_summary(conn, profile)?;
    let mut output = Vec::new();
    for row in rows(
        conn,
        "SELECT w.id, w.label, w.kind, w.config_json, w.created_at, COUNT(t.id) AS tx_count, MAX(t.occurred_at) AS last_tx_at \
         FROM wallets w LEFT JOIN transactions t ON t.wallet_id = w.id AND t.excluded = 0 WHERE w.profile_id = ? \
         GROUP BY w.id ORDER BY w.label ASC",
        [profile],
    )? {
        let id = text(&row, "id");
        let tx_count = count(&row, "tx_count");
        let config = json_config(&val(&row, "config_json"));
        let last_synced = string_or_empty(get(&config, "last_synced_at"));
        let summary = backend_summary(&text(&row, "kind"), &config, None);
        let source_format = string_or_empty(get(&config, "source_format"));
        let sync_source = {
            let raw = val(&config, "sync_source");
            string_or_empty(Some(&if truthy(&raw) { raw } else { s(source_format.as_str()) }))
        };
        let chain = string_or_empty(get(&config, "chain"));
        let network = string_or_empty(get(&config, "network"));
        let policy = string_or_empty(get(&config, "policy_asset"));
        let method = string_or_empty(get(&config, "payment_method_id"));
        let wallet_chain = {
            let raw = val(&config, "chain");
            normalize_chain(if truthy(&raw) { &raw } else { &Value::Null })
                .unwrap_or_else(|| "bitcoin".into())
        };
        let last_basis = if !last_synced.is_empty() {
            s(last_synced.as_str())
        } else if truthy(&val(&row, "last_tx_at")) {
            val(&row, "last_tx_at")
        } else {
            val(&row, "created_at")
        };
        let balance = balances.iter().find(|(key, _)| *key == id).map(|(_, b)| b);
        let import = imports.get(&id);
        let mut connection = dict([
            ("id", val(&row, "id")),
            ("kind", s(map_kind(&text(&row, "kind")))),
            ("label", val(&row, "label")),
            ("last", relative_last(&last_basis, clock)),
            (
                "lastSyncAt",
                if last_synced.is_empty() {
                    Value::Null
                } else {
                    s(last_synced.as_str())
                },
            ),
            ("lastTransactionAt", val(&row, "last_tx_at")),
            (
                "balance",
                balance.map_or(Value::Float(0.0), |b| b.balance.clone()),
            ),
            ("balanceSource", s(balance.map_or("books", |b| b.source))),
            (
                "bookBalance",
                Value::Float(balance.map_or(0.0, |b| b.book)),
            ),
            ("chainBalance", balance.map_or(Value::Null, |b| b.chain.clone())),
            (
                "chainLastSeenAt",
                balance.map_or(Value::Null, |b| b.last_seen.clone()),
            ),
            (
                "chainActiveCount",
                balance.map_or(Value::Null, |b| b.active.clone()),
            ),
            ("status", s(if tx_count != 0 { "synced" } else { "idle" })),
            ("transactionCount", tx_count.into()),
            ("syncMode", s(summary.sync_mode)),
            ("syncSource", s(sync_source)),
            (
                "sourceFormat",
                s(if source_format.is_empty() {
                    import.map(|i| i.2.clone()).unwrap_or_default()
                } else {
                    source_format.clone()
                }),
            ),
            (
                "observedAssets",
                Value::List(assets.get(&id).cloned().unwrap_or_default()),
            ),
            ("importBatchCount", import.map_or(0, |i| i.0).into()),
            ("lastImportAt", import.map_or(Value::Null, |i| i.1.clone())),
            ("deprecated", Value::Bool(parse_bool(get(&config, "deprecated"))?)),
            ("chain", s(if chain.is_empty() { wallet_chain } else { chain })),
            ("network", if network.is_empty() { Value::Null } else { s(network) }),
            ("policyAsset", if policy.is_empty() { Value::Null } else { s(policy) }),
            (
                "paymentMethodId",
                if method.is_empty() { Value::Null } else { s(method) },
            ),
        ]);
        if has_descriptor(&config) {
            let gap = val(&config, "gap_limit");
            let gap = if gap == Value::Null || gap == s("") {
                Value::Int(100.into())
            } else {
                Value::Int(py_int(&gap)?)
            };
            set(&mut connection, "gap", gap);
        }
        let kind = text(&row, "kind");
        if ["coreln", "lnd", "nwc"].contains(&kind.as_str()) {
            set(
                &mut connection,
                "lightningCapabilities",
                kassiber_setup::backends::lightning_capabilities(&kind),
            );
        }
        output.push(connection);
    }
    Ok(output)
}

/// `_canonical_quantity_processed` deltas per transaction or day, in msat.
fn quantity_deltas(conn: &Connection, profile: &str, by_day: bool) -> Result<Vec<(String, i128)>> {
    let mut deltas: Vec<(String, i128)> = Vec::new();
    let mut add = |key: String, value: i128| match deltas.iter_mut().find(|(k, _)| *k == key) {
        Some((_, total)) => *total += value,
        None => deltas.push((key, value)),
    };
    if canonical_processed(conn, profile)? {
        let sql = if by_day {
            "SELECT occurred_at AS key, SUM(amount_msat) AS amount_msat FROM journal_quantity_postings WHERE profile_id = ? \
             AND location_kind = 'wallet' AND state = 'observed' AND asset IN ('BTC', 'LBTC') GROUP BY occurred_at ORDER BY occurred_at"
        } else {
            "SELECT transaction_id AS key, SUM(amount_msat) AS amount_msat FROM journal_quantity_postings WHERE profile_id = ? \
             AND location_kind = 'wallet' AND state = 'observed' AND transaction_id IS NOT NULL AND asset IN ('BTC', 'LBTC') \
             GROUP BY transaction_id ORDER BY MIN(occurred_at), transaction_id"
        };
        for row in rows(conn, sql, [profile])? {
            let key = if by_day {
                match parse_day(&val(&row, "key")) {
                    Some(day) => day,
                    None => continue,
                }
            } else {
                text(&row, "key")
            };
            add(key, msat(&val(&row, "amount_msat"))?);
        }
        return Ok(deltas);
    }
    let sql = if by_day {
        "SELECT COALESCE(t.occurred_at, je.occurred_at) AS key, je.entry_type, je.quantity FROM journal_entries je \
         LEFT JOIN transactions t ON t.id = je.transaction_id WHERE je.profile_id = ? AND je.asset IN ('BTC', 'LBTC') \
         ORDER BY key ASC, je.created_at ASC, je.id ASC"
    } else {
        "SELECT transaction_id AS key, entry_type, quantity FROM journal_entries WHERE profile_id = ? \
         AND asset IN ('BTC', 'LBTC') ORDER BY occurred_at ASC, created_at ASC, id ASC"
    };
    for row in rows(conn, sql, [profile])? {
        let key = if by_day {
            match parse_day(&val(&row, "key")) {
                Some(day) => day,
                None => continue,
            }
        } else {
            if !truthy(&val(&row, "key")) {
                continue;
            }
            text(&row, "key")
        };
        add(
            key,
            book_delta(&text(&row, "entry_type"), &val(&row, "quantity"))?,
        );
    }
    Ok(deltas)
}

/// `_parse_day(value)`: `date.fromisoformat(str(value)[:10])`, as ISO text.
fn parse_day(value: &Value) -> Option<String> {
    if !truthy(value) {
        return None;
    }
    let head: String = py_str(value).chars().take(10).collect();
    let bytes = head.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<u32> {
        let part = &head[range];
        part.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (year >= 1 && (1..=days).contains(&day)).then_some(head)
}

/// `_portfolio_cost_basis_by_date` / `_by_transaction`: running basis.
fn cost_basis_by(conn: &Connection, profile: &str, by_day: bool) -> Result<BTreeMap<String, f64>> {
    let sql = if by_day {
        "SELECT COALESCE(t.occurred_at, je.occurred_at) AS key, je.entry_type, je.quantity, je.fiat_value, \
         COALESCE(je.cost_basis, 0) AS cost_basis FROM journal_entries je LEFT JOIN transactions t ON t.id = je.transaction_id \
         WHERE je.profile_id = ? AND je.asset IN ('BTC', 'LBTC') ORDER BY key ASC, je.created_at ASC, je.id ASC"
            .to_owned()
    } else {
        format!("SELECT transaction_id AS key, entry_type, quantity, fiat_value, cost_basis {BASIS_SQL}")
    };
    let mut basis = 0.0;
    let mut output = BTreeMap::new();
    for row in rows(conn, &sql, [profile])? {
        let key = if by_day {
            let key: String = text(&row, "key").chars().take(10).collect();
            if key.is_empty() {
                continue;
            }
            key
        } else {
            if !truthy(&val(&row, "key")) {
                continue;
            }
            text(&row, "key")
        };
        basis += basis_delta(&row)?;
        output.insert(key, basis);
    }
    Ok(output)
}

/// `_balance_series_from_month_deltas`.
fn month_series(by_month: &BTreeMap<String, i128>) -> Vec<Value> {
    if by_month.is_empty() {
        return vec![Value::Float(0.0); 12];
    }
    let keep: BTreeSet<&String> = by_month.keys().rev().take(12).collect();
    let mut cumulative = 0i128;
    let mut series = Vec::new();
    for (month, delta) in by_month {
        cumulative += delta;
        if keep.contains(month) {
            series.push(btc(cumulative));
        }
    }
    if series.len() < 12 {
        let first = series[0];
        let mut padded = vec![first; 12 - series.len()];
        padded.extend(series);
        series = padded;
    }
    series[series.len() - 12..]
        .iter()
        .map(|v| Value::Float(*v))
        .collect()
}

/// `_balance_series(conn, profile_id, has_book_state)`.
fn balance_series(conn: &Connection, profile: &str, has_book_state: bool) -> Result<Vec<Value>> {
    let mut by_month: BTreeMap<String, i128> = BTreeMap::new();
    if has_book_state {
        if canonical_processed(conn, profile)? {
            for row in rows(
                conn,
                "SELECT SUBSTR(occurred_at, 1, 7) AS month, SUM(amount_msat) AS amount_msat FROM journal_quantity_postings \
                 WHERE profile_id = ? AND location_kind = 'wallet' AND state = 'observed' AND asset IN ('BTC', 'LBTC') \
                 AND occurred_at IS NOT NULL GROUP BY SUBSTR(occurred_at, 1, 7) ORDER BY month",
                [profile],
            )? {
                if truthy(&val(&row, "month")) {
                    by_month.insert(text(&row, "month"), msat(&val(&row, "amount_msat"))?);
                }
            }
            return Ok(month_series(&by_month));
        }
        for row in rows(
            conn,
            "SELECT occurred_at, entry_type, quantity FROM journal_entries WHERE profile_id = ? AND asset IN ('BTC', 'LBTC') \
             ORDER BY occurred_at ASC, created_at ASC, id ASC",
            [profile],
        )? {
            let key: String = text(&row, "occurred_at").chars().take(7).collect();
            if key.is_empty() {
                continue;
            }
            *by_month.entry(key).or_default() +=
                book_delta(&text(&row, "entry_type"), &val(&row, "quantity"))?;
        }
        return Ok(month_series(&by_month));
    }
    for row in rows(
        conn,
        "SELECT occurred_at, direction, amount, fee FROM transactions WHERE profile_id = ? AND excluded = 0 \
         AND asset IN ('BTC', 'LBTC') ORDER BY occurred_at ASC, created_at ASC, id ASC",
        [profile],
    )? {
        let key: String = text(&row, "occurred_at").chars().take(7).collect();
        if key.is_empty() {
            continue;
        }
        *by_month.entry(key).or_default() += signed(&row)?;
    }
    Ok(month_series(&by_month))
}

/// `amount if inbound else -amount - fee`, in msat.
fn signed(row: &Value) -> Result<i128> {
    let amount = msat(&val(row, "amount"))?;
    let fee = msat(&val(row, "fee"))?;
    Ok(if text(row, "direction") == "inbound" {
        amount
    } else {
        -amount - fee
    })
}

/// `_portfolio_series(...)`.
#[allow(clippy::too_many_arguments)]
fn portfolio_series(
    conn: &Connection,
    profile: &str,
    fiat: &Value,
    fallback_rate: f64,
    final_btc: f64,
    final_value: f64,
    has_book_state: bool,
) -> Result<Vec<Value>> {
    let raw = rows(
        conn,
        "SELECT occurred_at, direction, amount, fee, fiat_rate, fiat_value FROM transactions WHERE profile_id = ? \
         AND excluded = 0 AND asset IN ('BTC', 'LBTC') ORDER BY occurred_at ASC, created_at ASC, id ASC",
        [profile],
    )?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let basis_by_date = cost_basis_by(conn, profile, true)?;
    let mut tx_days = BTreeSet::new();
    let mut rates_by_day: BTreeMap<String, f64> = BTreeMap::new();
    let mut raw_deltas: BTreeMap<String, i128> = BTreeMap::new();
    for row in &raw {
        let Some(day) = parse_day(&val(row, "occurred_at")) else {
            continue;
        };
        tx_days.insert(day.clone());
        *raw_deltas.entry(day.clone()).or_default() += signed(row)?;
        // `_rate_from_transaction(row)`.
        let rate = if val(row, "fiat_rate") != Value::Null {
            Some(float(&val(row, "fiat_rate"))?)
        } else if val(row, "fiat_value") != Value::Null && truthy(&val(row, "amount")) {
            let amount = btc(msat(&val(row, "amount"))?);
            if amount != 0.0 {
                Some(float(&val(row, "fiat_value"))?.abs() / amount)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(rate) = rate {
            rates_by_day.insert(day, rate);
        }
    }
    let deltas: BTreeMap<String, i128> = if has_book_state {
        quantity_deltas(conn, profile, true)?.into_iter().collect()
    } else {
        raw_deltas
    };
    let days: Vec<String> = deltas
        .keys()
        .cloned()
        .chain(tx_days)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if days.is_empty() {
        return Ok(Vec::new());
    }
    let pair = kassiber_rates::transaction_rate_pair(
        "BTC",
        &text(&dict([("fiat", fiat.clone())]), "fiat"),
    );
    let daily = match &pair {
        Some(pair) => rows(
            conn,
            "WITH keyed AS (SELECT CASE WHEN source = 'kraken-csv' AND granularity = 'daily' AND method = 'ohlcvt_csv' \
             THEN date(timestamp, '-1 day') ELSE substr(timestamp, 1, 10) END AS rate_day, timestamp, rate, source, fetched_at, \
             granularity, method FROM rates_cache WHERE pair = ? AND timestamp >= ?), ranked AS (SELECT rate_day, timestamp, rate, \
             source, fetched_at, granularity, method, ROW_NUMBER() OVER (PARTITION BY rate_day ORDER BY CASE WHEN source = 'manual' \
             AND timestamp LIKE substr(timestamp, 1, 10) || 'T00:00:00%' THEN 0 WHEN granularity = 'daily' THEN 1 ELSE 2 END ASC, \
             timestamp DESC, CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC, fetched_at DESC, source ASC) AS rn FROM keyed) \
             SELECT rate_day, timestamp, rate, source, fetched_at, granularity, method FROM ranked WHERE rn = 1 ORDER BY rate_day ASC",
            [pair.as_str(), days[0].as_str()],
        )?,
        None => Vec::new(),
    };
    let point = |date: &str, balance: f64, value: f64, basis: f64, price: f64| {
        dict([
            ("date", s(date)),
            ("label", s(date)),
            ("balanceBtc", Value::Float(balance)),
            ("valueEur", Value::Float(value)),
            ("costBasisEur", Value::Float(basis)),
            ("priceEur", Value::Float(price)),
        ])
    };
    let rate_of = |row: &Value| -> Result<f64> {
        let rate = val(row, "rate");
        if truthy(&rate) {
            float(&rate)
        } else {
            Ok(0.0)
        }
    };
    if !daily.is_empty() {
        let basis_items: Vec<(String, f64)> = basis_by_date
            .iter()
            .filter_map(|(day, value)| parse_day(&s(day.as_str())).map(|d| (d, *value)))
            .collect();
        let (mut quantity, mut tx_index, mut basis_index, mut day_basis) = (0i128, 0, 0, 0.0);
        let mut output = Vec::new();
        for rate_row in &daily {
            let Some(rate_day) = parse_day(&val(rate_row, "rate_day")) else {
                continue;
            };
            while tx_index < days.len() && days[tx_index] <= rate_day {
                quantity += deltas.get(&days[tx_index]).copied().unwrap_or(0);
                tx_index += 1;
            }
            while basis_index < basis_items.len() && basis_items[basis_index].0 <= rate_day {
                day_basis = basis_items[basis_index].1;
                basis_index += 1;
            }
            let balance = btc(quantity);
            let price = rate_of(rate_row)?;
            let mut item = point(&rate_day, balance, balance * price, day_basis, price);
            set(&mut item, "priceTimestamp", val(rate_row, "timestamp"));
            set(&mut item, "priceSource", val(rate_row, "source"));
            output.push(item);
        }
        let last_day = days.last().unwrap();
        let last_output = output.last().map(|p| text(p, "date"));
        if last_output.as_ref().is_some_and(|last| last_day > last) {
            while tx_index < days.len() {
                quantity += deltas.get(&days[tx_index]).copied().unwrap_or(0);
                tx_index += 1;
            }
            while basis_index < basis_items.len() {
                day_basis = basis_items[basis_index].1;
                basis_index += 1;
            }
            let rate = if fallback_rate != 0.0 {
                fallback_rate
            } else {
                rate_of(daily.last().unwrap())?
            };
            let value = if fallback_rate != 0.0 {
                final_value
            } else {
                btc(quantity) * rate
            };
            output.push(point(last_day, final_btc, value, day_basis, rate));
        }
        return Ok(output);
    }
    let (mut quantity, mut latest, mut day_basis) = (0i128, fallback_rate, 0.0);
    let mut output = Vec::new();
    for (index, day) in days.iter().enumerate() {
        quantity += deltas.get(day).copied().unwrap_or(0);
        if let Some(rate) = rates_by_day.get(day) {
            latest = *rate;
        }
        day_basis = basis_by_date.get(day).copied().unwrap_or(day_basis);
        let balance = btc(quantity);
        let last = index + 1 == days.len();
        output.push(point(
            day,
            if last { final_btc } else { balance },
            if last { final_value } else { balance * latest },
            day_basis,
            if fallback_rate != 0.0 {
                fallback_rate
            } else {
                latest
            },
        ));
    }
    Ok(output)
}

/// `parse_marker(description, key)`.
fn marker(description: &Value, key: &str) -> Option<String> {
    let raw = if truthy(description) {
        py_str(description)
    } else {
        String::new()
    };
    let prefix = format!("{key}=");
    raw.split(|c: char| kassiber_pytext::is_space(c as u32))
        .filter(|token| !token.is_empty())
        .find_map(|token| token.strip_prefix(&prefix).map(str::to_owned))
}

/// `_tax_free_balance_snapshot(conn, profile, freshness)`.
fn tax_free_balance(
    conn: &Connection,
    profile: &Value,
    freshness: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    if kassiber_pytext::lower(&text(profile, "tax_country")) != "at" {
        return Ok(Value::Null);
    }
    let entries = rows(
        conn,
        "SELECT wallet_id, entry_type, asset, occurred_at, quantity, fiat_value, cost_basis, description, at_category \
         FROM journal_entries WHERE profile_id = ? AND asset IN ('BTC', 'LBTC') ORDER BY occurred_at ASC, created_at ASC, id ASC",
        [text(profile, "id")],
    )?;
    let report =
        kassiber_reports::exit_tax::overview_inventory(conn, profile, entries.clone(), sources)?;
    let totals = val(&report, "totals");
    let alt = val(&totals, "altQuantitySats");
    let neu = val(&totals, "neuQuantitySats");
    let (alt_sats, neu_sats) = (py_int(&alt)?, py_int(&neu)?);
    let needs = truthy(&val(freshness, "needs_processing"));
    let quarantines = count(freshness, "quarantine_count");
    let status = if needs {
        "needs_journals"
    } else if quarantines != 0 {
        "quarantines"
    } else {
        "current"
    };
    let mut remaining: BTreeMap<String, Value> = BTreeMap::new();
    if let Value::List(lots) = val(&report, "lots") {
        for lot in lots.iter().filter(|lot| text(lot, "regime") == "alt") {
            remaining.insert(text(lot, "asset"), val(lot, "quantitySats"));
        }
    }
    // `_tax_free_wallet_summaries`, in msat per (wallet, asset).
    let mut by_wallet_asset: Vec<((String, String), i128)> = Vec::new();
    for entry in &entries {
        let kind = text(entry, "entry_type");
        if kind == "transfer_fee" {
            continue;
        }
        let wallet = text(entry, "wallet_id");
        if wallet.is_empty() {
            continue;
        }
        let key = (wallet, text(entry, "asset"));
        let quantity = msat(&val(entry, "quantity"))?;
        if quantity == 0 {
            continue;
        }
        let mut delta = None;
        if ["transfer_out", "transfer_in"].contains(&kind.as_str()) {
            let name = if kind == "transfer_out" {
                "at_alt_out"
            } else {
                "at_alt_in"
            };
            if let Some(alt) = marker(&val(entry, "description"), name)
                .and_then(|raw| kassiber_pytext::parse_int(&raw).ok())
            {
                let alt = i128::try_from(alt).unwrap_or_default();
                delta = Some(if kind == "transfer_out" { -alt } else { alt });
            }
        }
        let delta = match delta {
            Some(delta) => delta,
            None if entry_has_alt(entry, quantity) => quantity,
            None => continue,
        };
        match by_wallet_asset.iter_mut().find(|(k, _)| *k == key) {
            Some((_, total)) => *total += delta,
            None => by_wallet_asset.push((key, delta)),
        }
    }
    let mut has_alt: BTreeMap<String, bool> = BTreeMap::new();
    for ((wallet, asset), quantity) in &by_wallet_asset {
        let left = remaining
            .get(asset)
            .map(|v| py_int(v).map(|n| n > 0.into()))
            .transpose()?
            .unwrap_or(false);
        let flag = *quantity > 0 && left;
        let entry = has_alt.entry(wallet.clone()).or_insert(false);
        *entry = *entry || flag;
    }
    let wallets: Vec<Value> = has_alt
        .into_iter()
        .map(|(wallet, flag)| {
            dict([
                ("walletId", s(wallet)),
                ("hasTaxFreeBalance", Value::Bool(flag)),
            ])
        })
        .collect();
    Ok(dict([
        ("rule", s("austrian_altbestand")),
        ("jurisdictionCode", val(&report, "jurisdictionCode")),
        ("fiatCurrency", val(&report, "fiatCurrency")),
        ("status", s(status)),
        ("taxFreeQuantitySats", Value::Int(alt_sats.clone())),
        ("taxableQuantitySats", Value::Int(neu_sats.clone())),
        (
            "totalQuantitySats",
            Value::Int(alt_sats.clone() + neu_sats.clone()),
        ),
        ("taxFreeMarketValue", val(&totals, "altMarketValue")),
        ("taxableMarketValue", val(&totals, "neuMarketValue")),
        ("needsJournals", Value::Bool(needs)),
        ("quarantines", quarantines.into()),
        ("wallets", Value::List(wallets)),
        (
            "buckets",
            Value::List(vec![
                dict([
                    ("id", s("altbestand")),
                    ("regime", s("alt")),
                    ("label", s("Altbestand")),
                    ("quantitySats", Value::Int(alt_sats)),
                    ("marketValue", val(&totals, "altMarketValue")),
                    ("taxFree", Value::Bool(true)),
                ]),
                dict([
                    ("id", s("neubestand")),
                    ("regime", s("neu")),
                    ("label", s("Neubestand")),
                    ("quantitySats", Value::Int(neu_sats)),
                    ("marketValue", val(&totals, "neuMarketValue")),
                    ("taxFree", Value::Bool(false)),
                ]),
            ]),
        ),
    ]))
}

/// `_entry_has_alt_regime(entry)`.
fn entry_has_alt(entry: &Value, quantity: i128) -> bool {
    if let Some(marker) =
        marker(&val(entry, "description"), "at_regime").filter(|m| m == "alt" || m == "neu")
    {
        return marker == "alt";
    }
    let category = val(entry, "at_category");
    if quantity < 0 && truthy(&category) {
        return py_str(&category).starts_with("alt");
    }
    let occurred = val(entry, "occurred_at");
    if !truthy(&occurred) {
        return false;
    }
    kassiber_rates::time::parse(&py_str(&occurred).replace('Z', "+00:00"))
        .is_some_and(|at| at < kassiber_rates::time::parse("2021-02-28T23:00:00Z").unwrap())
}

/// `_empty_overview_snapshot()`.
fn empty_overview() -> Value {
    dict([
        ("priceEur", Value::Float(0.0)),
        ("priceUsd", Value::Float(0.0)),
        (
            "marketRate",
            dict([
                ("asset", s("BTC")),
                ("fiatCurrency", s("EUR")),
                ("pair", s("BTC-EUR")),
                ("rate", Value::Null),
                ("timestamp", Value::Null),
                ("source", Value::Null),
                ("fetchedAt", Value::Null),
                ("granularity", Value::Null),
                ("method", Value::Null),
            ]),
        ),
        ("connections", Value::List(vec![])),
        ("txs", Value::List(vec![])),
        ("balanceSeries", Value::List(vec![Value::Float(0.0); 12])),
        ("portfolioSeries", Value::List(vec![])),
        (
            "fiat",
            dict([
                ("fiatCurrency", s("EUR")),
                ("eurBalance", Value::Float(0.0)),
                ("eurCostBasis", Value::Float(0.0)),
                ("eurUnrealized", Value::Float(0.0)),
                ("eurRealizedYTD", Value::Float(0.0)),
                ("completeness", unavailable_completeness()),
            ]),
        ),
        ("taxFreeBalance", Value::Null),
        (
            "status",
            dict([
                ("workspace", Value::Null),
                ("profile", Value::Null),
                ("transactionCount", 0.into()),
                ("needsJournals", Value::Bool(false)),
                ("quarantines", 0.into()),
            ]),
        ),
    ])
}

/// `_has_book_balance_state(freshness)`.
fn has_book_state(freshness: &Value) -> bool {
    !truthy(&val(freshness, "needs_processing"))
        && (truthy(&val(freshness, "last_processed_at"))
            || count(freshness, "journal_entry_count") > 0)
}

/// `_build_profile_overview_snapshot(conn, workspace, profile)`.
fn profile_overview(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    clock: &Clock,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let freshness = journal_freshness(conn, Some(profile))?;
    let id = text(profile, "id");
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0",
            [&id],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    let price_eur = price(conn, "BTC-EUR", &id, "EUR")?;
    let price_usd = price(conn, "BTC-USD", &id, "USD")?;
    let (market, market_rate, code) = market_rate(conn, &val(profile, "fiat_currency"))?;
    let book_rate = match market_rate {
        Some(rate) => rate,
        None => latest_transaction_rate(conn, &id, &code)?,
    };
    let book_state = has_book_state(&freshness);
    let book = wallet_balances(conn, &id, book_state)?;
    let info = display_balances(
        conn,
        &id,
        &book,
        if book_state { "books" } else { "transactions" },
    )?;
    let mut display_values = Vec::new();
    for (_, balance) in &info {
        display_values.push(match &balance.balance {
            Value::Float(x) if *x != 0.0 => *x,
            Value::Float(_) | Value::Int(_) => 0.0,
            other => float(other)?,
        });
    }
    let adjustment = duplicate_adjustment(conn, &id, &info)?;
    // `max(0.0, sum(...) - adjustment)`; `sum()` starts from integer 0.
    let difference = display_values.iter().fold(0.0, |a, b| a + b) - adjustment;
    let total = if difference > 0.0 { difference } else { 0.0 };
    let counts: Vec<(&str, i64)> = ["chain", "books", "transactions"]
        .into_iter()
        .map(|source| {
            (
                source,
                info.iter().filter(|(_, b)| b.source == source).count() as i64,
            )
        })
        .collect();
    let active_sources: Vec<&str> = counts
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(s, _)| *s)
        .collect();
    let balance_source = match active_sources.len() {
        1 => active_sources[0],
        0 if book_state => "books",
        0 => "transactions",
        _ => "mixed",
    };
    let needs = truthy(&val(&freshness, "needs_processing"));
    let quarantines = val(&freshness, "quarantine_count");
    let balance_status = if needs {
        "needs_journals"
    } else if truthy(&quarantines) {
        "quarantines"
    } else {
        "current"
    };
    let mut fiat = fiat_snapshot(
        conn,
        &id,
        &val(profile, "fiat_currency"),
        book_rate,
        total,
        adjustment,
    )?;
    set(
        &mut fiat,
        "completeness",
        completeness(conn, &id, &freshness, total, market_rate.is_none())?,
    );
    let scope = Scope {
        workspace_id: &text(workspace, "id"),
        profile_id: &id,
    };
    let recent = kassiber_transactions::desktop::overview_rows(
        conn,
        &scope,
        kassiber_transactions::desktop::OverviewRows::Recent,
    )?;
    let running = activity_running(conn, &id, book_state)?;
    let activity = kassiber_transactions::desktop::overview_rows(
        conn,
        &scope,
        kassiber_transactions::desktop::OverviewRows::Activity(&|tx: &str| {
            running.get(tx).copied().unwrap_or((0.0, 0.0))
        }),
    )?;
    let markers = [
        "id",
        "externalId",
        "explorerId",
        "date",
        "occurredAt",
        "type",
        "asset",
        "chain",
        "network",
        "paymentMethod",
        "account",
        "counter",
        "amountSat",
        "feeSat",
        "eur",
        "rate",
        "fiatCurrency",
        "tag",
        "conf",
        "internal",
        "balanceBtc",
        "costBasisEur",
        "pair",
        "excluded",
        "quarantineReason",
        "reviewStatus",
        "queueStatus",
    ];
    let activity: Vec<Value> = activity
        .into_iter()
        .map(|row| {
            let mut fields: Vec<(String, Value)> = match row {
                Value::Dict(items) => items
                    .into_iter()
                    .filter_map(|(key, value)| match &key {
                        kassiber_pyjson::Key::Str(name)
                            if markers.contains(&name.as_str().unwrap_or_default()) =>
                        {
                            Some((name.as_str().unwrap_or_default().to_owned(), value))
                        }
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            fields.push(("summaryOnly".into(), Value::Bool(true)));
            dict(fields)
        })
        .collect();
    let eur_balance = match val(&fiat, "eurBalance") {
        Value::Float(x) => x,
        _ => 0.0,
    };
    Ok(dict([
        ("priceEur", Value::Float(price_eur)),
        ("priceUsd", Value::Float(price_usd)),
        ("marketRate", market),
        (
            "connections",
            Value::List(connections(conn, &id, &info, clock)?),
        ),
        ("txs", Value::List(recent)),
        ("activityTxs", Value::List(activity)),
        (
            "balanceSeries",
            Value::List(balance_series(conn, &id, book_state)?),
        ),
        (
            "portfolioSeries",
            Value::List(portfolio_series(
                conn,
                &id,
                &val(profile, "fiat_currency"),
                book_rate,
                total,
                eur_balance,
                book_state,
            )?),
        ),
        ("fiat", fiat),
        (
            "balanceSummary",
            dict([
                ("totalBtc", Value::Float(total)),
                ("status", s(balance_status)),
                ("source", s(balance_source)),
                ("needsJournals", Value::Bool(needs)),
                ("quarantines", quarantines.clone()),
                ("chainWalletCount", counts[0].1.into()),
                ("bookWalletCount", counts[1].1.into()),
                ("transactionWalletCount", counts[2].1.into()),
                ("duplicateOutpointAdjustmentBtc", Value::Float(adjustment)),
            ]),
        ),
        (
            "taxFreeBalance",
            tax_free_balance(conn, profile, &freshness, sources)?,
        ),
        (
            "status",
            dict([
                ("workspace", val(workspace, "label")),
                ("profile", val(profile, "label")),
                ("transactionCount", active.into()),
                ("needsJournals", Value::Bool(needs)),
                ("quarantines", quarantines),
            ]),
        ),
    ]))
}

/// Running `(balance_btc, cost_basis)` per activity row, as
/// `_activity_transactions` accumulates them in oldest-first order.
fn activity_running(
    conn: &Connection,
    profile: &str,
    book_state: bool,
) -> Result<BTreeMap<String, (f64, f64)>> {
    let deltas: BTreeMap<String, i128> = if book_state {
        quantity_deltas(conn, profile, false)?.into_iter().collect()
    } else {
        BTreeMap::new()
    };
    let basis = cost_basis_by(conn, profile, false)?;
    let mut quantity = 0i128;
    let mut running_basis = 0.0;
    let mut output = BTreeMap::new();
    for row in rows(
        conn,
        "SELECT t.id, t.direction, t.amount, t.fee FROM transactions t JOIN wallets w ON w.id = t.wallet_id \
         LEFT JOIN journal_quarantines jq ON jq.transaction_id = t.id WHERE t.profile_id = ? AND t.asset IN ('BTC', 'LBTC') AND COALESCE(t.excluded, 0) = 0 \
         ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC",
        [profile],
    )? {
        let id = text(&row, "id");
        quantity += if book_state {
            deltas.get(&id).copied().unwrap_or(0)
        } else {
            signed(&row)?
        };
        running_basis = basis.get(&id).copied().unwrap_or(running_basis);
        output.insert(id, (btc(quantity), running_basis));
    }
    Ok(output)
}

/// `build_overview_snapshot(conn)`: the active book.
pub fn overview(
    conn: &Connection,
    clock: &Clock,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let context = kassiber_setup::context::snapshot(conn)?;
    if !truthy(&val(&context, "workspace_id")) || !truthy(&val(&context, "profile_id")) {
        return Ok(empty_overview());
    }
    let workspace = rows(
        conn,
        "SELECT * FROM workspaces WHERE id = ?",
        [text(&context, "workspace_id")],
    )?
    .into_iter()
    .next();
    let profile = rows(
        conn,
        "SELECT * FROM profiles WHERE id = ?",
        [text(&context, "profile_id")],
    )?
    .into_iter()
    .next();
    match (workspace, profile) {
        (Some(workspace), Some(profile)) => {
            profile_overview(conn, &workspace, &profile, clock, sources)
        }
        _ => Ok(empty_overview()),
    }
}

/// `_profile_overview_for_status(conn, workspace, profile)`.
fn book_overview(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    clock: &Clock,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let snapshot = profile_overview(conn, workspace, profile, clock, sources)?;
    let wallets = match val(&snapshot, "connections") {
        Value::List(items) => items.len() as i64,
        _ => 0,
    };
    let status = val(&snapshot, "status");
    let transactions = count(&status, "transactionCount");
    let freshness = journal_freshness(conn, Some(profile))?;
    let (mut ready, mut hints) = profile_readiness(wallets, transactions, &freshness);
    let reasons = val(&val(&val(&snapshot, "fiat"), "completeness"), "reasons");
    let has = |reason: &str| matches!(&reasons, Value::List(items) if items.contains(&s(reason)));
    let extra: Vec<Value> = [
        (
            "custody_unresolved",
            "Resolve custody gaps before trusting cost basis or reports.",
        ),
        (
            "missing_prices",
            "Fill missing transaction prices before trusting cost basis or reports.",
        ),
    ]
    .into_iter()
    .filter(|(reason, _)| has(reason))
    .map(|(_, hint)| s(hint))
    .collect();
    if !extra.is_empty() {
        hints.retain(|hint| !py_str(hint).starts_with("Reports are ready"));
        hints.extend(extra);
        ready = false;
    }
    let fiat_code = {
        let raw = val(profile, "fiat_currency");
        kassiber_pytext::upper(&if truthy(&raw) {
            py_str(&raw)
        } else {
            "EUR".into()
        })
    };
    let mut status = status;
    for (key, value) in [
        ("workspaceId", val(workspace, "id")),
        ("profileId", val(profile, "id")),
        ("workspace", val(workspace, "label")),
        ("profile", val(profile, "label")),
        ("journalEntryCount", val(&freshness, "journal_entry_count")),
        ("freshnessStatus", val(&freshness, "status")),
        ("freshnessReason", val(&freshness, "reason")),
    ] {
        set(&mut status, key, value);
    }
    Ok(dict([
        (
            "profile",
            dict([
                ("id", val(profile, "id")),
                ("label", val(profile, "label")),
                ("fiatCurrency", s(fiat_code)),
                ("taxCountry", val(profile, "tax_country")),
                (
                    "taxLongTermDays",
                    stored_int(profile, "tax_long_term_days")?,
                ),
                ("gainsAlgorithm", val(profile, "gains_algorithm")),
            ]),
        ),
        (
            "workspace",
            dict([
                ("id", val(workspace, "id")),
                ("label", val(workspace, "label")),
            ]),
        ),
        ("connections", val(&snapshot, "connections")),
        ("txs", val(&snapshot, "txs")),
        ("activityTxs", val(&snapshot, "activityTxs")),
        ("balanceSeries", val(&snapshot, "balanceSeries")),
        ("portfolioSeries", val(&snapshot, "portfolioSeries")),
        ("fiat", val(&snapshot, "fiat")),
        ("balanceSummary", val(&snapshot, "balanceSummary")),
        ("taxFreeBalance", val(&snapshot, "taxFreeBalance")),
        ("marketRate", val(&snapshot, "marketRate")),
        ("status", status),
        ("journals", freshness),
        (
            "readiness",
            dict([("ready", Value::Bool(ready)), ("hints", Value::List(hints))]),
        ),
    ]))
}

fn empty_workspace(workspace: Option<&Value>) -> Value {
    dict([
        (
            "workspace",
            workspace.map_or(Value::Null, |w| {
                dict([("id", val(w, "id")), ("label", val(w, "label"))])
            }),
        ),
        (
            "scope",
            dict([("kind", s("workspace")), ("label", s("Book set"))]),
        ),
        ("books", Value::List(vec![])),
        ("connections", Value::List(vec![])),
        ("txs", Value::List(vec![])),
        ("activityTxs", Value::List(vec![])),
        ("balanceSeries", Value::List(vec![Value::Float(0.0); 12])),
        ("portfolioSeries", Value::List(vec![])),
        ("fiat", empty_workspace_fiat()),
        (
            "status",
            dict([
                (
                    "workspace",
                    workspace.map_or(Value::Null, |w| val(w, "label")),
                ),
                (
                    "workspaceId",
                    workspace.map_or(Value::Null, |w| val(w, "id")),
                ),
                ("bookCount", 0.into()),
                ("transactionCount", 0.into()),
                ("needsJournals", Value::Bool(false)),
                ("quarantines", 0.into()),
                ("ready", Value::Bool(false)),
                ("mixedFiat", Value::Bool(false)),
            ]),
        ),
    ])
}
fn empty_workspace_fiat() -> Value {
    dict([
        ("mode", s("empty")),
        ("fiatCurrency", Value::Null),
        ("currencies", Value::List(vec![])),
        ("mixed", Value::Bool(false)),
        ("partial", Value::Bool(false)),
        ("eurBalance", Value::Float(0.0)),
        ("eurCostBasis", Value::Float(0.0)),
        ("eurUnrealized", Value::Float(0.0)),
        ("eurRealizedYTD", Value::Float(0.0)),
        ("btcBalance", Value::Float(0.0)),
        ("books", Value::List(vec![])),
        ("completeness", unavailable_completeness()),
    ])
}

/// `float(value or 0)` for already-built payload numbers.
fn number(value: &Value) -> f64 {
    match value {
        Value::Float(x) => *x,
        Value::Int(n) => n.to_string().parse().unwrap_or(0.0),
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => 0.0,
    }
}

/// `_rollup_fiat_completeness(items)`.
fn rollup_completeness(items: &[Value]) -> Value {
    if items.is_empty() {
        return unavailable_completeness();
    }
    let rank = |state: &str| match state {
        "complete" => 0,
        "unavailable" => 1,
        "incomplete" => 2,
        "stale" => 3,
        _ => 1,
    };
    let state = items
        .iter()
        .map(|item| {
            let state = val(item, "state");
            if truthy(&state) {
                py_str(&state)
            } else {
                "unavailable".into()
            }
        })
        .fold(None::<String>, |best, state| match best {
            Some(best) if rank(&best) >= rank(&state) => Some(best),
            _ => Some(state),
        })
        .unwrap();
    let reasons: BTreeSet<String> = items
        .iter()
        .flat_map(|item| match val(item, "reasons") {
            Value::List(list) => list.iter().map(py_str).collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();
    let complete = items
        .iter()
        .all(|item| val(item, "costBasisComplete") == Value::Bool(true));
    let optional = |key: &str| -> Value {
        let values: Vec<Value> = items.iter().map(|item| val(item, key)).collect();
        if values.contains(&Value::Null) {
            return Value::Null;
        }
        Value::Int(
            values
                .iter()
                .map(|v| py_int(v).unwrap_or_default())
                .fold(0.into(), |a: kassiber_pyjson::BigInt, b| a + b),
        )
    };
    let sum = |key: &str| -> Value {
        Value::Int(
            items
                .iter()
                .map(|item| {
                    let v = val(item, key);
                    if truthy(&v) {
                        py_int(&v).unwrap_or_default()
                    } else {
                        0.into()
                    }
                })
                .fold(0.into(), |a: kassiber_pyjson::BigInt, b| a + b),
        )
    };
    let incomplete: Vec<&Value> = items
        .iter()
        .filter(|item| val(item, "costBasisComplete") != Value::Bool(true))
        .collect();
    let earliest = if !incomplete.is_empty()
        && incomplete
            .iter()
            .all(|item| truthy(&val(item, "earliestIncompleteAt")))
    {
        incomplete
            .iter()
            .map(|item| py_str(&val(item, "earliestIncompleteAt")))
            .min()
            .map_or(Value::Null, s)
    } else {
        Value::Null
    };
    dict([
        ("state", s(state)),
        ("costBasisComplete", Value::Bool(complete)),
        (
            "reasons",
            Value::List(
                COMPLETENESS_REASONS
                    .iter()
                    .filter(|r| reasons.contains(**r))
                    .map(|r| s(*r))
                    .collect(),
            ),
        ),
        ("quarantineCount", sum("quarantineCount")),
        ("quarantinedInboundMsat", sum("quarantinedInboundMsat")),
        ("quarantinedOutboundMsat", sum("quarantinedOutboundMsat")),
        ("basisCoveredMsat", optional("basisCoveredMsat")),
        ("basisUncoveredMsat", optional("basisUncoveredMsat")),
        ("earliestIncompleteAt", earliest),
        ("missingPriceCount", sum("missingPriceCount")),
        (
            "marketRateMissing",
            Value::Bool(
                items
                    .iter()
                    .any(|item| val(item, "marketRateMissing") == Value::Bool(true)),
            ),
        ),
    ])
}

/// `ui.workspace.overview.snapshot` arguments: only a non-blank workspace id.
fn workspace_id(args: Option<&Value>) -> Result<String> {
    let empty = dict::<&str>([]);
    let args = match args {
        None | Some(Value::Null) => &empty,
        Some(args @ Value::Dict(_)) => args,
        Some(other) => {
            return Err(
                AppError::new("validation", "ui snapshot args must be an object")
                    .with_details(dict([("type", s(kassiber_envelope::type_name(other)))])),
            )
        }
    };
    let Value::Dict(pairs) = args else {
        unreachable!()
    };
    let mut unknown: Vec<kassiber_pyjson::PyStr> = pairs
        .iter()
        .filter_map(|(key, _)| match key {
            kassiber_pyjson::Key::Str(key) if !key.is("workspace_id") => Some(key.clone()),
            _ => None,
        })
        .collect();
    if !unknown.is_empty() {
        unknown.sort();
        return Err(AppError::new(
            "validation",
            "ui.workspace.overview.snapshot received unsupported arguments",
        )
        .with_details(dict([(
            "unknown",
            Value::List(unknown.into_iter().map(Value::Str).collect()),
        )])));
    }
    match get(args, "workspace_id") {
        Some(Value::Str(id)) if kassiber_pytext::strip_py(id).code_points().next().is_some() => {
            Ok(py_str(&Value::Str(kassiber_pytext::strip_py(id))))
        }
        _ => Err(AppError::new(
            "validation",
            "ui.workspace.overview.snapshot requires args.workspace_id",
        )),
    }
}

/// `build_workspace_overview_snapshot(conn, args)`.
pub fn workspace_overview(
    conn: &Connection,
    args: Option<&Value>,
    clock: &Clock,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    let id = workspace_id(args)?;
    let Some(workspace) = rows(conn, "SELECT * FROM workspaces WHERE id = ?", [&id])?
        .into_iter()
        .next()
    else {
        return Err(AppError::new(
            "not_found",
            format!("Book set '{id}' was not found"),
        ));
    };
    let profiles = rows(
        conn,
        "SELECT * FROM profiles WHERE workspace_id = ? ORDER BY created_at ASC, label ASC",
        [text(&workspace, "id")],
    )?;
    if profiles.is_empty() {
        return Ok(empty_workspace(Some(&workspace)));
    }
    let books: Vec<Value> = profiles
        .iter()
        .map(|profile| book_overview(conn, &workspace, profile, clock, sources))
        .collect::<Result<_>>()?;
    let currencies: Vec<String> = books
        .iter()
        .filter_map(|book| {
            let fiat = val(&val(book, "profile"), "fiatCurrency");
            truthy(&fiat).then(|| kassiber_pytext::upper(&py_str(&fiat)))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let same_fiat = currencies.len() == 1;
    let boundary = |items: Value, profile: &Value| -> Vec<Value> {
        let Value::List(items) = items else {
            return Vec::new();
        };
        items
            .into_iter()
            .map(|mut item| {
                set(&mut item, "workspaceId", val(&workspace, "id"));
                set(&mut item, "workspaceLabel", val(&workspace, "label"));
                set(&mut item, "profileId", val(profile, "id"));
                set(&mut item, "profileLabel", val(profile, "label"));
                set(
                    &mut item,
                    "book",
                    dict([("id", val(profile, "id")), ("label", val(profile, "label"))]),
                );
                item
            })
            .collect()
    };
    let (mut connections, mut txs, mut activity) = (Vec::new(), Vec::new(), Vec::new());
    for book in &books {
        let profile = profiles
            .iter()
            .find(|p| val(p, "id") == val(&val(book, "profile"), "id"))
            .unwrap();
        connections.extend(boundary(val(book, "connections"), profile));
        txs.extend(boundary(val(book, "txs"), profile));
        activity.extend(boundary(val(book, "activityTxs"), profile));
    }
    let recent_key = |item: &Value| {
        let at = val(item, "occurredAt");
        let at = if truthy(&at) { at } else { val(item, "date") };
        (
            if truthy(&at) {
                py_str(&at)
            } else {
                String::new()
            },
            if truthy(&val(item, "id")) {
                py_str(&val(item, "id"))
            } else {
                String::new()
            },
        )
    };
    let newest = |mut items: Vec<Value>| {
        items.sort_by_key(|a| std::cmp::Reverse(recent_key(a)));
        items.truncate(20);
        items
    };
    let txs = newest(txs);
    let activity = newest(activity);
    let total_transactions: i64 = books
        .iter()
        .map(|b| count(&val(b, "status"), "transactionCount"))
        .sum();
    let quarantines: i64 = books
        .iter()
        .map(|b| count(&val(b, "journals"), "quarantine_count"))
        .sum();
    let needs = books
        .iter()
        .any(|b| truthy(&val(&val(b, "journals"), "needs_processing")));
    let ready_books = books
        .iter()
        .filter(|b| truthy(&val(&val(b, "readiness"), "ready")))
        .count() as i64;
    let blocked = books.len() as i64 - ready_books;
    // `_sum_balance_series(books)`.
    let mut series = [0.0f64; 12];
    for book in &books {
        let mut values: Vec<f64> = match val(book, "balanceSeries") {
            Value::List(items) => items.iter().map(number).collect(),
            _ => Vec::new(),
        };
        if values.len() < 12 {
            let first = values.first().copied().unwrap_or(0.0);
            let mut padded = vec![first; 12 - values.len()];
            padded.extend(values);
            values = padded;
        }
        for (index, value) in values[values.len() - 12..].iter().enumerate() {
            series[index] += value;
        }
    }
    Ok(dict([
        (
            "workspace",
            dict([
                ("id", val(&workspace, "id")),
                ("label", val(&workspace, "label")),
            ]),
        ),
        (
            "scope",
            dict([("kind", s("workspace")), ("label", s("Book set"))]),
        ),
        ("books", Value::List(books.clone())),
        ("connections", Value::List(connections)),
        ("txs", Value::List(txs)),
        ("activityTxs", Value::List(activity)),
        (
            "balanceSeries",
            Value::List(series.iter().map(|v| Value::Float(*v)).collect()),
        ),
        (
            "portfolioSeries",
            Value::List(workspace_series(&books, same_fiat)),
        ),
        ("fiat", workspace_fiat(&books, &currencies)),
        (
            "status",
            dict([
                ("workspace", val(&workspace, "label")),
                ("workspaceId", val(&workspace, "id")),
                ("bookCount", (books.len() as i64).into()),
                ("transactionCount", total_transactions.into()),
                ("needsJournals", Value::Bool(needs)),
                ("quarantines", quarantines.into()),
                ("ready", Value::Bool(blocked == 0)),
                ("readyBooks", ready_books.into()),
                ("blockedBooks", blocked.into()),
                ("mixedFiat", Value::Bool(!same_fiat)),
            ]),
        ),
    ]))
}

/// `_workspace_portfolio_series(books, same_fiat)`.
fn workspace_series(books: &[Value], same_fiat: bool) -> Vec<Value> {
    let mut book_points: Vec<(&Value, Vec<Value>)> = Vec::new();
    for book in books {
        let mut points: Vec<Value> = match val(book, "portfolioSeries") {
            Value::List(items) => items
                .into_iter()
                .filter(|p| truthy(&val(p, "date")))
                .collect(),
            _ => Vec::new(),
        };
        points.sort_by_key(|p| py_str(&val(p, "date")));
        if !points.is_empty() {
            book_points.push((book, points));
        }
    }
    let dates: BTreeSet<String> = book_points
        .iter()
        .flat_map(|(_, points)| points.iter().map(|p| py_str(&val(p, "date"))))
        .collect();
    let mut indexes: BTreeMap<String, usize> = BTreeMap::new();
    let mut last: BTreeMap<String, Value> = BTreeMap::new();
    let mut output = Vec::new();
    for date in dates {
        let mut balance = 0.0;
        let (mut value_total, mut basis_total) = (0.0, 0.0);
        let mut carried = Vec::new();
        for (book, points) in &book_points {
            let profile = val(book, "profile");
            let id = py_str(&val(&profile, "id"));
            let index = indexes.entry(id.clone()).or_insert(0);
            while *index < points.len() && py_str(&val(&points[*index], "date")) <= date {
                last.insert(id.clone(), points[*index].clone());
                *index += 1;
            }
            let Some(point) = last.get(&id) else {
                continue;
            };
            let (b, v, c) = (
                number(&val(point, "balanceBtc")),
                number(&val(point, "valueEur")),
                number(&val(point, "costBasisEur")),
            );
            balance += b;
            if same_fiat {
                value_total += v;
                basis_total += c;
            }
            carried.push(dict([
                ("profileId", val(&profile, "id")),
                ("profileLabel", val(&profile, "label")),
                ("fiatCurrency", val(&profile, "fiatCurrency")),
                ("balanceBtc", Value::Float(b)),
                ("value", Value::Float(v)),
                ("costBasis", Value::Float(c)),
            ]));
        }
        let mut aggregate = dict([
            ("date", s(date.as_str())),
            ("label", s(date.as_str())),
            ("balanceBtc", Value::Float(balance)),
            ("books", Value::List(vec![])),
        ]);
        if same_fiat {
            set(&mut aggregate, "valueEur", Value::Float(value_total));
            set(&mut aggregate, "costBasisEur", Value::Float(basis_total));
        }
        set(&mut aggregate, "books", Value::List(carried));
        if same_fiat && balance != 0.0 {
            set(
                &mut aggregate,
                "priceEur",
                Value::Float(value_total / balance),
            );
        }
        output.push(aggregate);
    }
    output
}

/// `_workspace_fiat_rollup(books, currencies)`.
fn workspace_fiat(books: &[Value], currencies: &[String]) -> Value {
    let btc_balance: f64 = books
        .iter()
        .map(|book| {
            let summary = val(book, "balanceSummary");
            match val(&summary, "totalBtc") {
                Value::Float(x) => x,
                Value::Int(n) => n.to_string().parse().unwrap_or(0.0),
                _ => match val(book, "connections") {
                    Value::List(items) => items
                        .iter()
                        .map(|c| number(&val(c, "balance")))
                        .fold(0.0, |a, b| a + b),
                    _ => 0.0,
                },
            }
        })
        .fold(0.0, |a, b| a + b);
    if books.is_empty() {
        return empty_workspace_fiat();
    }
    let rows: Vec<Value> = books
        .iter()
        .map(|book| {
            let fiat = val(book, "fiat");
            let profile = val(book, "profile");
            let completeness = val(&fiat, "completeness");
            dict([
                ("profileId", val(&profile, "id")),
                ("profileLabel", val(&profile, "label")),
                ("fiatCurrency", val(&profile, "fiatCurrency")),
                ("balance", Value::Float(number(&val(&fiat, "eurBalance")))),
                (
                    "costBasis",
                    Value::Float(number(&val(&fiat, "eurCostBasis"))),
                ),
                (
                    "unrealized",
                    Value::Float(number(&val(&fiat, "eurUnrealized"))),
                ),
                (
                    "realizedYTD",
                    Value::Float(number(&val(&fiat, "eurRealizedYTD"))),
                ),
                (
                    "completeness",
                    if truthy(&completeness) {
                        completeness
                    } else {
                        unavailable_completeness()
                    },
                ),
            ])
        })
        .collect();
    let completeness = rollup_completeness(
        &rows
            .iter()
            .map(|r| val(r, "completeness"))
            .collect::<Vec<_>>(),
    );
    let total = |key: &str| {
        Value::Float(
            rows.iter()
                .map(|r| number(&val(r, key)))
                .fold(0.0, |a, b| a + b),
        )
    };
    let list = Value::List(currencies.iter().map(|c| s(c.as_str())).collect());
    if currencies.len() == 1 {
        return dict([
            ("mode", s("single")),
            ("fiatCurrency", s(currencies[0].as_str())),
            ("currencies", list),
            ("mixed", Value::Bool(false)),
            ("partial", Value::Bool(false)),
            ("eurBalance", total("balance")),
            ("eurCostBasis", total("costBasis")),
            ("eurUnrealized", total("unrealized")),
            ("eurRealizedYTD", total("realizedYTD")),
            ("btcBalance", Value::Float(btc_balance)),
            ("books", Value::List(rows)),
            ("completeness", completeness),
        ]);
    }
    dict([
        ("mode", s("mixed")),
        ("fiatCurrency", Value::Null),
        ("currencies", list),
        ("mixed", Value::Bool(true)),
        ("partial", Value::Bool(true)),
        ("eurBalance", Value::Null),
        ("eurCostBasis", Value::Null),
        ("eurUnrealized", Value::Null),
        ("eurRealizedYTD", Value::Null),
        ("btcBalance", Value::Float(btc_balance)),
        ("books", Value::List(rows)),
        ("completeness", completeness),
        (
            "label",
            s("Mixed fiat currencies; per-book fiat rows are shown without conversion."),
        ),
    ])
}

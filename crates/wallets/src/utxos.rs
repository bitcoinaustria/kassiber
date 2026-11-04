//! `ui.wallets.utxos`: the stored output inventory of one wallet
//! (`build_wallet_utxos_snapshot` in `core/ui_snapshot.py`).
//!
//! A local read: it classifies the wallet against its configured backend and
//! reads `wallet_utxos`; it never refreshes the inventory or contacts the
//! backend. Wallet configuration (descriptors, xpubs) is only classified,
//! never projected.
use crate::publication::row::{field, first, rows, Result};
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_store::rusqlite::{types::Value as Sql, Connection};

const ROW_LIMIT: i64 = 500;
const CHAIN_ALIASES: &[(&str, &str)] = &[
    ("bitcoin", "bitcoin"),
    ("btc", "bitcoin"),
    ("onchain", "bitcoin"),
    ("liquid", "liquid"),
    ("lbtc", "liquid"),
    ("elements", "liquid"),
];
const BITCOIN_NETWORKS: &[(&str, &str)] = &[
    ("main", "main"),
    ("mainnet", "main"),
    ("bitcoin", "main"),
    ("test", "test"),
    ("testnet", "test"),
    ("regtest", "regtest"),
    ("signet", "signet"),
];
const LIQUID_NETWORKS: &[(&str, &str)] = &[
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

/// `str_or_none(value)`.
fn str_or_none(value: &Value) -> Option<String> {
    if matches!(value, Value::Null) {
        return None;
    }
    let text = py_str(value);
    let text = kassiber_pytext::strip(&text);
    (!text.is_empty()).then(|| text.to_owned())
}

/// `_string_or_empty(value)`: only strings count.
fn string_or_empty(value: &Value) -> String {
    match value {
        Value::Str(_) => str_or_none(value).unwrap_or_default(),
        _ => String::new(),
    }
}

/// `str(value or "")`.
fn or_text(value: &Value) -> String {
    if truthy(value) {
        py_str(value)
    } else {
        String::new()
    }
}

fn status(supported: bool, status: &str, reason: &str, message: &str) -> Value {
    dict([
        ("supported", Value::Bool(supported)),
        ("status", s(status)),
        ("reason", s(reason)),
        ("message", s(message)),
    ])
}

/// `_wallet_backend_summary(kind, config, default_backend)`.
pub struct BackendSummary {
    pub name: String,
    pub source: &'static str,
    pub sync_mode: &'static str,
}

pub fn backend_summary(kind: &str, config: &Value, default_backend: &str) -> BackendSummary {
    let mut name = string_or_empty(field(config, "backend"));
    let default_name = string_or_empty(&s(default_backend));
    let mut source = if name.is_empty() { "none" } else { "explicit" };
    let descriptor = has_descriptor(config);
    let silent_payment = str_or_none(field(config, "sp_descriptor")).is_some();
    let sync_mode = if string_or_empty(field(config, "sync_source")) == "btcpay" {
        "btcpay"
    } else if !string_or_empty(field(config, "source_file")).is_empty()
        && !string_or_empty(field(config, "source_format")).is_empty()
    {
        "file_import"
    } else if silent_payment && kind == "silent-payment" {
        "backend_silent_payment"
    } else if descriptor && ["descriptor", "xpub", "address"].contains(&kind) {
        "backend_descriptor"
    } else if truthy(field(config, "addresses")) && kind == "address" {
        "backend_addresses"
    } else {
        "not_configured"
    };
    if name.is_empty()
        && !default_name.is_empty()
        && ["backend_descriptor", "backend_addresses"].contains(&sync_mode)
    {
        name = default_name;
        source = "default";
    }
    if name.is_empty() {
        source = "none";
    }
    BackendSummary {
        name,
        source,
        sync_mode,
    }
}

/// `has_descriptor_sync_material(config)`.
fn has_descriptor(config: &Value) -> bool {
    str_or_none(field(config, "descriptor")).is_some()
        || str_or_none(field(config, "xpub")).is_some() && truthy(field(config, "script_types"))
}

/// `normalize_backend_kind(kind)`: `str(kind).strip().lower()` with aliases.
fn backend_kind(kind: &Value) -> String {
    let value = kassiber_pytext::strip(&py_str(kind)).to_lowercase();
    match value.as_str() {
        "bitcoin-core" | "bitcoincore" | "core" => "bitcoinrpc".into(),
        "core-ln" | "core-lightning" => "coreln".into(),
        "liquid-esplora" | "mempool" => "esplora".into(),
        _ => value,
    }
}

/// `backend_supports_silent_payments(backend)`.
fn supports_silent_payments(backend: &Value) -> Result<bool> {
    let value = |key: &str| {
        let direct = field(backend, key);
        if matches!(direct, Value::Null) || *direct == s("") {
            if let config @ Value::Dict(_) = field(backend, "config") {
                return field(config, key).clone();
            }
        }
        direct.clone()
    };
    let mut declared = false;
    for key in ["silent_payments", "sp_capable", "bip352"] {
        let value = value(key);
        if str_or_none(&value).is_none() {
            continue;
        }
        declared = true;
        match kassiber_pytext::strip(&py_str(&value))
            .to_lowercase()
            .as_str()
        {
            "1" | "true" | "yes" | "on" => return Ok(true),
            "0" | "false" | "no" | "off" => {}
            _ => {
                return Err(AppError::new(
                    "app_error",
                    format!("Invalid boolean value: {}", py_str(&value)),
                ))
            }
        }
    }
    if declared {
        return Ok(false);
    }
    Ok([
        "silent_payment_scan_file",
        "sp_scan_file",
        "silent_payment_scan_path",
        "sp_scan_path",
    ]
    .iter()
    .any(|key| str_or_none(&value(key)).is_some()))
}

/// `_wallet_utxo_support(wallet, config, backend_summary, backend)`.
fn support(config: &Value, summary: &BackendSummary, backend: Option<&Value>) -> Result<Value> {
    if string_or_empty(field(config, "source_format")) == "wasabi_bundle" {
        return Ok(status(true, "supported", "wasabi_import", ""));
    }
    if ![
        "backend_descriptor",
        "backend_addresses",
        "backend_silent_payment",
    ]
    .contains(&summary.sync_mode)
    {
        return Ok(status(
            false,
            "unsupported_source",
            "not_chain_backed",
            "This source is not a chain-backed watch-only wallet.",
        ));
    }
    let Some(backend) = backend.filter(|b| matches!(b, Value::Dict(_))) else {
        return Ok(status(
            false,
            "unsupported_source",
            "backend_missing",
            "This wallet needs a configured chain backend or local scanner before UTXO inventory can refresh.",
        ));
    };
    let kind = backend_kind(field(backend, "kind"));
    let shown = if kind.is_empty() {
        "this backend"
    } else {
        &kind
    };
    if summary.sync_mode == "backend_silent_payment" {
        if !["esplora", "electrum", "bitcoinrpc", "custom"].contains(&kind.as_str()) {
            return Ok(status(
                false,
                "silent_payment_backend_unsupported",
                "backend_kind",
                &format!("Silent Payments scanning is not implemented for {shown} sources."),
            ));
        }
        if !supports_silent_payments(backend)? {
            return Ok(status(
                false,
                "silent_payment_backend_unsupported",
                "missing_sp_capability",
                "This backend is not marked Silent Payments capable; ordinary scripthash sync cannot discover BIP352 outputs.",
            ));
        }
        return Ok(status(true, "supported", "", ""));
    }
    if !["esplora", "electrum", "bitcoinrpc"].contains(&kind.as_str()) {
        return Ok(status(
            false,
            "unsupported_source",
            "backend_kind",
            &format!("UTXO inventory is not implemented for {shown} sources yet."),
        ));
    }
    let descriptor = has_descriptor(config);
    let chain =
        kassiber_pytext::strip(&or_text_default(field(config, "chain"), "bitcoin")).to_lowercase();
    if chain == "liquid" {
        if !descriptor {
            return Ok(status(
                false,
                "liquid_unblind_blocked",
                "liquid_descriptor_required",
                "Liquid UTXO inventory requires descriptor-backed outputs so Kassiber can unblind them locally.",
            ));
        }
        let json = serde_json::from_str::<serde_json::Value>(
            &kassiber_pyjson::dumps(config, &Default::default())
                .map_err(|_| AppError::new("internal_error", "Invalid wallet configuration"))?,
        )
        .map_err(|_| AppError::new("internal_error", "Invalid wallet configuration"))?;
        match crate::descriptors::load(&json) {
            Err(error) => {
                let error = error.into_app();
                return Ok(status(
                    false,
                    "liquid_unblind_blocked",
                    "descriptor_unavailable",
                    &error.message,
                ));
            }
            Ok(None) => {
                // `liquid_plan_can_unblind(None)` reads `plan.chain`.
                return Err(AppError::new(
                    "internal_error",
                    "'NoneType' object has no attribute 'chain'",
                ));
            }
            Ok(Some(plan)) if !plan.can_unblind() => {
                return Ok(status(
                    false,
                    "liquid_unblind_blocked",
                    "missing_blinding_keys",
                    "Liquid UTXO inventory needs private blinding keys before Kassiber can account for outputs.",
                ));
            }
            Ok(Some(_)) => {}
        }
    }
    if !descriptor && !truthy(field(config, "addresses")) {
        return Ok(status(
            false,
            "unsupported_source",
            "no_watch_targets",
            "This wallet has no descriptor, xpub, or addresses to scan.",
        ));
    }
    Ok(status(true, "supported", "", ""))
}

/// `str(value or default)`.
fn or_text_default(value: &Value, default: &str) -> String {
    if truthy(value) {
        py_str(value)
    } else {
        default.to_owned()
    }
}

/// `_unique_text_values(values)`.
fn unique(values: &[&Value]) -> Vec<String> {
    let mut output: Vec<String> = vec![];
    for value in values {
        let text = string_or_empty(value);
        if !text.is_empty() && !output.contains(&text) {
            output.push(text);
        }
    }
    output
}

/// `normalize_chain(value)` from `wallet_descriptors`; `None` for ValueError.
fn normalize_chain(value: &Value) -> Option<&'static str> {
    let chain = kassiber_pytext::strip(&or_text_default(value, "bitcoin")).to_lowercase();
    if chain.is_empty() {
        return Some("bitcoin");
    }
    CHAIN_ALIASES
        .iter()
        .find(|(alias, _)| *alias == chain)
        .map(|(_, target)| *target)
}

/// `_wallet_utxo_chain_filter_values(value)`.
fn chain_values(value: &Value) -> Vec<String> {
    let Some(canonical) = normalize_chain(value) else {
        return unique(&[value]);
    };
    let aliases: Vec<Value> = CHAIN_ALIASES
        .iter()
        .filter(|(_, target)| *target == canonical)
        .map(|(alias, _)| s(*alias))
        .collect();
    let canonical = s(canonical);
    let mut values = vec![&canonical, value];
    values.extend(aliases.iter());
    unique(&values)
}

/// `_wallet_utxo_network_filter_values(chain, value)`.
fn network_values(chain: &str, value: &Value) -> Vec<String> {
    let Some(normalized) = normalize_chain(&s(chain)) else {
        return unique(&[value]);
    };
    let (aliases, default) = if normalized == "bitcoin" {
        (BITCOIN_NETWORKS, "main")
    } else {
        (LIQUID_NETWORKS, "liquidv1")
    };
    let mut network = kassiber_pytext::strip(&or_text_default(value, default)).to_lowercase();
    if network.is_empty() {
        network = default.to_owned();
    }
    let Some(canonical) = aliases
        .iter()
        .find(|(alias, _)| *alias == network)
        .map(|(_, target)| *target)
    else {
        return unique(&[value]);
    };
    // Python picks the alias map by the caller's chain text, not the
    // normalized chain.
    let map = if chain == "bitcoin" {
        BITCOIN_NETWORKS
    } else {
        LIQUID_NETWORKS
    };
    let aliases: Vec<Value> = map
        .iter()
        .filter(|(_, target)| *target == canonical)
        .map(|(alias, _)| s(*alias))
        .collect();
    let canonical = s(canonical);
    let mut values = vec![&canonical, value];
    values.extend(aliases.iter());
    unique(&values)
}

/// `backend_value(backend, key)`: `str_or_none(backend.get(key))`.
fn backend_value(backend: Option<&Value>, key: &str) -> Value {
    match backend {
        Some(backend @ Value::Dict(_)) => str_or_none(field(backend, key)).map_or(Value::Null, s),
        _ => Value::Null,
    }
}

/// `_wallet_utxo_source_filter(config, backend_summary, backend)`, as the
/// four column filters in Python's order.
fn source_filter(
    config: &Value,
    summary: &BackendSummary,
    backend: Option<&Value>,
) -> [(&'static str, Vec<String>); 4] {
    if string_or_empty(field(config, "source_format")) == "wasabi_bundle" {
        let chain = if truthy(field(config, "chain")) {
            field(config, "chain").clone()
        } else {
            s("bitcoin")
        };
        let chains = chain_values(&chain);
        let network = if truthy(field(config, "network")) {
            field(config, "network").clone()
        } else {
            s("mainnet")
        };
        let networks = network_values(chains.first().map_or("bitcoin", String::as_str), &network);
        return [
            ("backend_name", vec!["wasabi".into()]),
            ("backend_kind", vec!["wasabi_bundle".into()]),
            ("chain", chains),
            ("network", networks),
        ];
    }
    let backend_chain = backend_value(backend, "chain");
    let chain_source = if truthy(&backend_chain) {
        backend_chain
    } else {
        field(config, "chain").clone()
    };
    let chains = chain_values(&chain_source);
    let chain = chains
        .first()
        .cloned()
        .unwrap_or_else(|| string_or_empty(&chain_source));
    let backend_network = backend_value(backend, "network");
    let network = if truthy(&backend_network) {
        backend_network
    } else {
        field(config, "network").clone()
    };
    let networks = network_values(if chain.is_empty() { "bitcoin" } else { &chain }, &network);
    let kind = backend_value(backend, "kind");
    let kinds = if string_or_empty(&kind).is_empty() {
        vec![]
    } else {
        unique(&[&s(backend_kind(&kind)), &kind])
    };
    let backend_name = match backend {
        Some(b @ Value::Dict(_)) => field(b, "name").clone(),
        _ => Value::Null,
    };
    let names = unique(&[&s(summary.name.as_str()), &backend_name]);
    [
        ("backend_name", names),
        ("backend_kind", kinds),
        ("chain", chains),
        ("network", networks),
    ]
}

/// `_source_filter_sql(prefix=prefix, **filter)`.
fn filter_sql(filter: &[(&str, Vec<String>); 4], prefix: &str) -> (String, Vec<Sql>) {
    let mut clauses = vec![];
    let mut params = vec![];
    for (column, values) in filter {
        let mut normalized: Vec<String> = vec![];
        for value in values {
            if let Some(text) = str_or_none(&s(value.as_str())) {
                if !normalized.contains(&text) {
                    normalized.push(text);
                }
            }
        }
        match normalized.len() {
            0 => continue,
            1 => clauses.push(format!("{prefix}{column} = ?")),
            n => clauses.push(format!("{prefix}{column} IN ({})", vec!["?"; n].join(", "))),
        }
        params.extend(normalized.into_iter().map(Sql::Text));
    }
    if clauses.is_empty() {
        (String::new(), params)
    } else {
        (format!(" AND {}", clauses.join(" AND ")), params)
    }
}

fn int(value: &Value) -> Result<i64> {
    match value {
        Value::Null => Ok(0),
        Value::Int(n) => i64::try_from(n.clone())
            .map_err(|_| AppError::new("internal_error", "UTXO amount out of range")),
        Value::Bool(b) => Ok(i64::from(*b)),
        _ => kassiber_pytext::parse_int(&py_str(value))
            .ok()
            .and_then(|n| i64::try_from(n).ok())
            .ok_or_else(|| {
                AppError::new(
                    "internal_error",
                    format!("invalid literal for int() with base 10: {}", py_str(value)),
                )
            }),
    }
}

/// `float(msat_to_btc(msat))`: the exact decimal quotient, correctly rounded.
fn btc(msat: i64) -> Value {
    let sign = if msat < 0 { "-" } else { "" };
    let abs = msat.unsigned_abs();
    let text = format!(
        "{sign}{}.{:011}",
        abs / 100_000_000_000,
        abs % 100_000_000_000
    );
    Value::Float(text.parse().expect("decimal BTC amount"))
}

fn amounts(msat: i64) -> [(&'static str, Value); 3] {
    [
        ("amount", btc(msat)),
        ("amount_sat", Value::from(msat.div_euclid(1000))),
        ("amount_msat", Value::from(msat)),
    ]
}

/// `json.loads(value or default)`, or `None` when that raises.
fn loads(value: &Value, default: &str) -> Option<Value> {
    match value {
        Value::Str(text) if text.code_points().next().is_some() => {
            kassiber_pytext::loads(text.as_str()?).ok()
        }
        Value::Null | Value::Str(_) => kassiber_pytext::loads(default).ok(),
        _ => None,
    }
}

fn or_empty(value: &Value) -> Value {
    if truthy(value) {
        value.clone()
    } else {
        s("")
    }
}

fn list(conn: &Connection, wallet: &str, filter: &[(&str, Vec<String>); 4]) -> Result<Vec<Value>> {
    let (source_where, mut params) = filter_sql(filter, "");
    params.insert(0, Sql::Text(wallet.to_owned()));
    params.push(Sql::Integer(ROW_LIMIT));
    let sql = format!(
        "SELECT u.*, (SELECT t.id FROM transactions t WHERE t.profile_id = u.profile_id \
         AND t.wallet_id = u.wallet_id AND lower(t.external_id) = lower(u.txid) \
         ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC LIMIT 1) AS transaction_id \
         FROM wallet_utxos u WHERE u.wallet_id = ? AND spent_at IS NULL{source_where} \
         ORDER BY asset ASC, CASE WHEN block_height IS NULL OR block_height <= 0 THEN 1 ELSE 0 END ASC, \
         block_height ASC, txid ASC, vout ASC LIMIT ?"
    );
    let mut output = vec![];
    for row in rows(
        conn,
        &sql,
        kassiber_store::rusqlite::params_from_iter(params),
    )? {
        let msat = int(field(&row, "amount"))?;
        let history = match loads(field(&row, "anon_history_json"), "[]") {
            Some(list @ Value::List(_)) => list,
            _ => Value::List(vec![]),
        };
        let samourai = match loads(field(&row, "raw_json"), "{}") {
            Some(raw @ Value::Dict(_)) => match get(&raw, "samourai") {
                Some(value @ Value::Dict(_)) => value.clone(),
                _ => Value::Null,
            },
            _ => Value::Null,
        };
        let mut item: Vec<(Key, Value)> = vec![
            ("id".into(), field(&row, "id").clone()),
            (
                "transaction_id".into(),
                or_empty(field(&row, "transaction_id")),
            ),
            ("outpoint".into(), field(&row, "outpoint").clone()),
            ("txid".into(), field(&row, "txid").clone()),
            ("vout".into(), Value::from(int(field(&row, "vout"))?)),
            ("asset".into(), field(&row, "asset").clone()),
        ];
        item.extend(amounts(msat).map(|(k, v)| (k.into(), v)));
        for key in [
            "confirmation_status",
            "confirmations",
            "block_height",
            "block_time",
        ] {
            item.push((key.into(), field(&row, key).clone()));
        }
        for key in ["address", "address_label", "branch_label"] {
            item.push((key.into(), or_empty(field(&row, key))));
        }
        for key in ["branch_index", "address_index", "anonymity_score"] {
            item.push((key.into(), field(&row, key).clone()));
        }
        item.push(("spent_by".into(), or_empty(field(&row, "spent_by"))));
        item.push((
            "excluded_from_coinjoin".into(),
            match field(&row, "excluded_from_coinjoin") {
                Value::Null => Value::Null,
                value => Value::Bool(truthy(value)),
            },
        ));
        item.push(("key_state".into(), or_empty(field(&row, "key_state"))));
        item.push(("anon_history".into(), history));
        item.push((
            "source".into(),
            dict([
                ("backend", or_empty(field(&row, "backend_name"))),
                ("backend_kind", or_empty(field(&row, "backend_kind"))),
                ("chain", field(&row, "chain").clone()),
                ("network", field(&row, "network").clone()),
                ("first_seen_at", field(&row, "first_seen_at").clone()),
                ("last_seen_at", field(&row, "last_seen_at").clone()),
                ("spent_at", field(&row, "spent_at").clone()),
            ]),
        ));
        item.push(("samourai".into(), samourai));
        output.push(Value::Dict(item));
    }
    Ok(output)
}

/// `wallet_output_inventory_summary`: `(active_count, last_seen_at)`.
fn summary(
    conn: &Connection,
    wallet: &str,
    filter: &[(&str, Vec<String>); 4],
) -> Result<(i64, Value)> {
    let (source_where, params) = filter_sql(filter, "");
    let with_wallet = || {
        std::iter::once(Sql::Text(wallet.to_owned()))
            .chain(params.iter().cloned())
            .collect::<Vec<_>>()
    };
    let active = first(
        conn,
        &format!(
            "SELECT COUNT(*) AS active_count, MAX(last_seen_at) AS active_last_seen_at \
             FROM wallet_utxos WHERE wallet_id = ? AND spent_at IS NULL{source_where}"
        ),
        kassiber_store::rusqlite::params_from_iter(with_wallet()),
    )?;
    let refresh = first(
        conn,
        &format!(
            "SELECT observed_count, active_count, last_seen_at FROM wallet_utxo_refreshes \
             WHERE wallet_id = ?{source_where}"
        ),
        kassiber_store::rusqlite::params_from_iter(with_wallet()),
    )?;
    let count = active
        .as_ref()
        .map_or(Ok(0), |row| int(field(row, "active_count")))?;
    let last_seen = match (&refresh, &active) {
        (Some(row), _) => field(row, "last_seen_at").clone(),
        (None, Some(row)) => field(row, "active_last_seen_at").clone(),
        (None, None) => Value::Null,
    };
    Ok((count, last_seen))
}

fn totals(
    conn: &Connection,
    wallet: &str,
    filter: &[(&str, Vec<String>); 4],
) -> Result<Vec<Value>> {
    let (source_where, params) = filter_sql(filter, "");
    let params = std::iter::once(Sql::Text(wallet.to_owned())).chain(params);
    rows(
        conn,
        &format!(
            "SELECT asset, SUM(amount) AS amount_msat FROM wallet_utxos WHERE wallet_id = ? \
             AND spent_at IS NULL{source_where} GROUP BY asset ORDER BY asset ASC"
        ),
        kassiber_store::rusqlite::params_from_iter(params),
    )?
    .into_iter()
    .map(|row| {
        let msat = int(field(&row, "amount_msat"))?;
        let mut item = vec![("asset", field(&row, "asset").clone())];
        item.extend(amounts(msat));
        Ok(dict(item))
    })
    .collect()
}

/// The `UnicodeEncodeError` text CPython's strict UTF-8 encoder raises for
/// the first run of lone surrogates in `text`.
fn surrogate_error(text: &kassiber_pyjson::PyStr) -> AppError {
    let points: Vec<u32> = text.code_points().collect();
    let surrogate = |p: &u32| (0xd800..=0xdfff).contains(p);
    let start = points.iter().position(surrogate).unwrap_or(0);
    let end = start
        + points[start..]
            .iter()
            .take_while(|p| surrogate(p))
            .count()
            .max(1);
    let what = if end - start == 1 {
        format!("character '\\u{:04x}' in position {start}", points[start])
    } else {
        format!("characters in position {start}-{}", end - 1)
    };
    AppError::new(
        "internal_error",
        format!("'utf-8' codec can't encode {what}: surrogates not allowed"),
    )
}

fn label_or_null(value: &Value) -> Value {
    match value {
        Value::Str(text) if text.code_points().next().is_some() => value.clone(),
        _ => Value::Null,
    }
}

/// `build_wallet_utxos_snapshot(conn, runtime_config, args)`. `args` is the
/// raw request value; `backend` looks up one merged runtime backend.
pub fn snapshot(
    conn: &Connection,
    args: Option<&Value>,
    default_backend: Option<&str>,
    backend: &dyn Fn(&str) -> Option<Value>,
) -> Result<Value> {
    let context = kassiber_setup::context::snapshot(conn)?;
    let profile = if or_text(field(&context, "workspace_id")).is_empty()
        || or_text(field(&context, "profile_id")).is_empty()
    {
        None
    } else {
        first(
            conn,
            "SELECT * FROM profiles WHERE id = ?",
            [or_text(field(&context, "profile_id"))],
        )?
    };
    let Some(profile) = profile else {
        return Ok(dict([
            ("wallet", Value::Null),
            ("utxos", Value::List(vec![])),
            ("totals", Value::List(vec![])),
            (
                "support",
                status(
                    false,
                    "unsupported_source",
                    "no_active_profile",
                    "No active profile is selected.",
                ),
            ),
            (
                "freshness",
                dict([
                    ("status", s("no_profile")),
                    ("last_seen_at", Value::Null),
                    ("last_synced_at", Value::Null),
                    ("stale", Value::Bool(false)),
                ]),
            ),
        ]));
    };
    let Some(args @ Value::Dict(_)) = args else {
        return Err(AppError::new(
            "validation",
            "ui.wallets.utxos args must be an object",
        ));
    };
    let reference = if truthy(field(args, "wallet")) {
        field(args, "wallet")
    } else {
        field(args, "connection")
    };
    let reference = match reference {
        Value::Str(text)
            if kassiber_pytext::strip_py(text)
                .code_points()
                .next()
                .is_some() =>
        {
            kassiber_pytext::strip_py(text)
        }
        _ => {
            return Err(AppError::new(
                "validation",
                "ui.wallets.utxos wallet must be a non-empty string",
            ))
        }
    };
    let reference_text = reference.as_str().map(str::to_owned);
    let wallet = match &reference_text {
        Some(text) => first(
            conn,
            "SELECT w.id, w.label, w.kind, w.config_json, w.created_at, a.code AS account_code, \
             a.label AS account_label FROM wallets w LEFT JOIN accounts a ON a.id = w.account_id \
             WHERE w.profile_id = ? AND (w.id = ? OR w.label = ?) \
             ORDER BY CASE WHEN w.id = ? THEN 0 ELSE 1 END, w.label ASC LIMIT 1",
            [&or_text(field(&profile, "id")), text, text, text],
        )?,
        // Python binds the reference as a SQLite parameter, which fails to
        // encode lone surrogates before any lookup.
        None => return Err(surrogate_error(&reference)),
    };
    let Some(wallet) = wallet else {
        let message = kassiber_pyjson::PyStr::from_code_points(
            "Wallet '"
                .chars()
                .map(u32::from)
                .chain(reference.code_points())
                .chain(
                    "' was not found in the active profile"
                        .chars()
                        .map(u32::from),
                ),
        )
        .unwrap_or_else(|_| "Wallet was not found in the active profile".into());
        return Err(AppError::new("not_found", "").with_python_message(message));
    };
    let config = match loads(field(&wallet, "config_json"), "{}") {
        Some(config @ Value::Dict(_)) => config,
        _ => dict::<&str>([]),
    };
    let kind = or_text(field(&wallet, "kind"));
    let summary = backend_summary(&kind, &config, default_backend.unwrap_or(""));
    let configured = if summary.name.is_empty() {
        None
    } else {
        backend(&summary.name)
    };
    let support = support(&config, &summary, configured.as_ref())?;
    let supported = truthy(field(&support, "supported"));
    let wallet_id = or_text(field(&wallet, "id"));
    let (rows, totals, last_seen, count) = if supported {
        let filter = source_filter(&config, &summary, configured.as_ref());
        let rows = list(conn, &wallet_id, &filter)?;
        let (count, last_seen) = self::summary(conn, &wallet_id, &filter)?;
        (rows, totals(conn, &wallet_id, &filter)?, last_seen, count)
    } else {
        (vec![], vec![], Value::Null, 0)
    };
    let last_synced = string_or_empty(field(&config, "last_synced_at"));
    let last_synced = if last_synced.is_empty() {
        Value::Null
    } else {
        s(last_synced)
    };
    let freshness = if !supported {
        field(&support, "status").clone()
    } else if truthy(&last_seen) {
        s("current")
    } else if truthy(&last_synced) {
        s("stale")
    } else {
        s("never_refreshed")
    };
    let returned = rows.len();
    let backend_kind = match &configured {
        Some(b @ Value::Dict(_)) => s(or_text(field(b, "kind"))),
        _ => s(""),
    };
    Ok(dict([
        (
            "wallet",
            dict([
                ("id", field(&wallet, "id").clone()),
                ("label", field(&wallet, "label").clone()),
                ("kind", field(&wallet, "kind").clone()),
                (
                    "account",
                    dict([
                        ("code", or_empty(field(&wallet, "account_code"))),
                        ("label", or_empty(field(&wallet, "account_label"))),
                    ]),
                ),
                (
                    "backend",
                    dict([
                        ("name", s(summary.name.as_str())),
                        ("source", s(summary.source)),
                        ("kind", backend_kind),
                    ]),
                ),
                ("chain", s(or_text(field(&config, "chain")))),
                ("network", s(or_text(field(&config, "network")))),
                ("sync_mode", s(summary.sync_mode)),
            ]),
        ),
        ("utxos", Value::List(rows)),
        ("totals", Value::List(totals)),
        ("support", support),
        (
            "freshness",
            dict([
                ("status", freshness.clone()),
                ("last_seen_at", last_seen),
                ("last_synced_at", last_synced),
                ("stale", Value::Bool(supported && freshness != s("current"))),
                ("active_count", Value::from(count)),
            ]),
        ),
        (
            "summary",
            dict([
                (
                    "workspace",
                    label_or_null(field(&context, "workspace_label")),
                ),
                ("profile", label_or_null(field(&context, "profile_label"))),
                ("count", Value::from(count)),
                ("returned_count", Value::from(returned)),
                (
                    "truncated",
                    Value::Bool(supported && count > returned as i64),
                ),
                (
                    "row_limit",
                    if supported {
                        Value::from(ROW_LIMIT)
                    } else {
                        Value::Null
                    },
                ),
            ]),
        ),
    ]))
}

//! Authoritative Bitcoin record insertion and merging, from
//! `core/imports.py`'s `insert_wallet_records(..., authoritative_chain_observer=True)`.
//! Only the closed chain-observer record shape is accepted; importer pricing,
//! settlement and exchange-evidence fields fail closed instead of guessing.
use super::physical::{canonical_network, canonical_txid};
use super::row::{blank, field, first, or_text, py_eq, rows, Result};
use crate::database_error;
use kassiber_envelope::{dict, get, py_str, s, set, truthy, AppError};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::{Key, Value};
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql, Connection};

const UNKNOWN_OCCURRED_AT: &str = "1970-01-01T00:00:00Z";
const RATES_CACHE: &str = "rates_cache";
const OBSERVER_DERIVED_KINDS: [&str; 3] = ["deposit", "withdrawal", "fee"];
const AUTHORED_COLUMNS: [&str; 6] = [
    "note",
    "review_status",
    "kind_override",
    "taxability_override",
    "at_regime_override",
    "at_category_override",
];
const PRICE_COLUMNS: [&str; 14] = [
    "fiat_rate",
    "fiat_value",
    "fiat_rate_exact",
    "fiat_value_exact",
    "fiat_price_source",
    "pricing_source_kind",
    "pricing_provider",
    "pricing_pair",
    "pricing_timestamp",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
    "pricing_external_ref",
    "pricing_quality",
];
const PRIVACY_HOP_TYPES: [&str; 5] = [
    "coinjoin",
    "collaborative",
    "payjoin",
    "payment_in_coinjoin",
    "sweep",
];
const PRIVACY_BOUNDARY_RAW_KEYS: [&str; 8] = [
    "privacy_boundary",
    "privacy_boundary_kind",
    "privacyBoundary",
    "privacyBoundaryKind",
    "privacy_hop",
    "privacy_hop_kind",
    "privacyHop",
    "privacyHopKind",
];
const LIKELY_COINJOIN_KEYS: [&str; 5] = [
    "islikelycoinjoin",
    "is_likely_coinjoin",
    "isLikelyCoinJoin",
    "likely_coinjoin",
    "likelyCoinJoin",
];
/// The record keys a Bitcoin chain observer emits (`_record_from_bitcoin_graph`).
const CHAIN_RECORD_KEYS: [&str; 18] = [
    "txid",
    "occurred_at",
    "confirmed_at",
    "direction",
    "asset",
    "amount",
    "fee",
    "fiat_rate",
    "fiat_value",
    "kind",
    "description",
    "counterparty",
    "raw_json",
    "privacy_boundary",
    "payment_hash",
    "payment_hash_source",
    "swap_refund_funding_txid",
    "swap_refund_funding_vout",
];
const DUPLICATE_HINT: &str = "Choose one transaction as the keeper and exclude every sibling through the audited transaction metadata action, then retry refresh.";
const AUTHORED_HINT: &str = "Clear the authored note, review status, tags or overrides on that transaction through the audited transaction metadata action, then retry refresh.";
const RECORD_CONFLICT_HINT: &str = "One refresh produced opposite wallet directions for the same transaction. Re-run the refresh; if it repeats, report the wallet backend and txid.";
pub(crate) const EXISTING_COLUMNS: &str = "
       id, workspace_id, profile_id, wallet_id, external_id, external_id_kind, fingerprint,
       occurred_at, direction, asset, amount, fee, amount_includes_fee,
       confirmed_at, fiat_rate, fiat_value, fiat_price_source, fiat_rate_exact,
       fiat_value_exact, pricing_source_kind, pricing_provider, pricing_pair,
       pricing_timestamp, pricing_fetched_at, pricing_granularity, pricing_method,
       pricing_external_ref, pricing_quality, kind, privacy_boundary, description,
       counterparty, excluded, kind_override, note, review_status,
       taxability_override, at_regime_override, at_category_override,
       raw_json, payment_hash, payment_hash_source,
       swap_refund_funding_txid, swap_refund_funding_vout,
       (SELECT observation.authority_version FROM chain_observation_provenance observation
         WHERE observation.transaction_id = id) AS observation_authority_version,
       (SELECT observation.graph_hash FROM chain_observation_provenance observation
         WHERE observation.transaction_id = id) AS observation_graph_hash,
       (SELECT observation.quantity_hash FROM chain_observation_provenance observation
         WHERE observation.transaction_id = id) AS observation_quantity_hash
";

fn context() -> Context {
    Context::new(28, Rounding::HalfEven)
}

fn invalid_decimal(value: &Value) -> AppError {
    AppError::new(
        "app_error",
        format!("Invalid decimal value: {}", py_str(value)),
    )
}

/// `msat.dec(value, default)`.
fn dec(value: &Value, default: &str) -> Result<Decimal> {
    if blank(value) {
        return default.parse().map_err(|_| invalid_decimal(value));
    }
    let text = match value {
        Value::Decimal(d) => return Ok(d.clone()),
        other => py_str(other),
    };
    kassiber_pytext::strip(&text)
        .parse()
        .map_err(|_| invalid_decimal(value))
}

/// `msat.btc_to_msat(value)`.
pub(crate) fn btc_to_msat(value: &Decimal) -> Result<i64> {
    let product = value
        .mul(&Decimal::from(100_000_000_000_u64), &context())
        .map_err(|_| AppError::new("app_error", "Invalid decimal value"))?;
    product
        .to_fixed_places(0, Rounding::HalfUp)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| {
            AppError::new(
                "app_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        })
}

/// `int(value or 0)` for a stored msat/flag column.
fn int_or_zero(value: &Value) -> Result<i64> {
    if !truthy(value) {
        return Ok(0);
    }
    match value {
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Int(n) => i64::try_from(n.clone())
            .map_err(|_| AppError::new("app_error", "Stored integer is out of range")),
        Value::Float(f) if f.is_finite() => Ok(f.trunc() as i64),
        Value::Str(text) => text
            .as_str()
            .and_then(|text| kassiber_pytext::parse_int(text).ok())
            .and_then(|n| i64::try_from(n).ok())
            .ok_or_else(|| AppError::new("app_error", "Stored integer is invalid")),
        _ => Err(AppError::new("app_error", "Stored integer is invalid")),
    }
}

/// `util.str_or_none`.
fn str_or_none(value: &Value) -> Option<String> {
    if matches!(value, Value::Null) {
        return None;
    }
    let text = py_str(value);
    let text = kassiber_pytext::strip(&text);
    (!text.is_empty()).then(|| text.to_owned())
}

fn opt(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::from)
}

/// `time_utils.parse_timestamp`.
pub(crate) fn parse_timestamp(value: &Value) -> Result<String> {
    if !truthy(value) {
        return Err(AppError::new("app_error", "Missing occurred_at/date value"));
    }
    let raw = py_str(value);
    let raw = kassiber_pytext::strip(&raw);
    let expanded;
    let raw = if raw.chars().count() == 10 {
        expanded = format!("{raw}T00:00:00+00:00");
        expanded.as_str()
    } else {
        raw
    };
    let refused = || {
        AppError::new(
            "validation",
            format!("Invalid occurred_at/date value '{}'", py_str(value)),
        )
        .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z")
    };
    let instant = kassiber_chain::parse_iso_time(raw).ok_or_else(refused)?;
    let seconds = instant.div_euclid(1_000_000) - 62_135_596_800;
    kassiber_chain::format_unix_time(seconds).ok_or_else(refused)
}

fn hex_bytes(text: &str) -> bool {
    // `bytes.fromhex`: whitespace may separate, never split, a byte.
    let mut chars = text.chars();
    while let Some(first) = chars.next() {
        if matches!(first, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ') {
            continue;
        }
        if !first.is_ascii_hexdigit() || !chars.next().is_some_and(|c| c.is_ascii_hexdigit()) {
            return false;
        }
    }
    true
}

/// `imports._canonical_txid`, whose `bytes.fromhex` check permits spaces.
fn import_canonical_txid(value: &Value) -> Option<String> {
    let text = kassiber_pytext::lower(kassiber_pytext::strip(&or_text(value)));
    (text.chars().count() == 64 && hex_bytes(&text)).then_some(text)
}

fn json_payload(raw: &Value) -> Option<Value> {
    match raw {
        Value::Dict(_) => Some(raw.clone()),
        Value::Str(text) if text.code_points().next().is_some() => {
            match kassiber_pytext::loads(&py_str(raw)) {
                Ok(parsed @ Value::Dict(_)) => Some(parsed),
                _ => None,
            }
        }
        _ => None,
    }
}

fn boolish(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        other if blank(other) => false,
        other => matches!(
            kassiber_pytext::lower(kassiber_pytext::strip(&py_str(other))).as_str(),
            "1" | "true" | "yes" | "on"
        ),
    }
}

fn explicit_boundary(payload: &Value, key: &str) -> Result<Option<String>> {
    let Some(value) = get(payload, key) else {
        return Ok(None);
    };
    let Some(text) = str_or_none(value) else {
        return Ok(None);
    };
    let normalized = kassiber_pytext::lower(kassiber_pytext::strip(&text)).replace('-', "_");
    if PRIVACY_HOP_TYPES.contains(&normalized.as_str()) {
        return Ok(Some(normalized));
    }
    Err(AppError::new(
        "validation",
        format!("Unsupported privacy boundary '{text}'"),
    )
    .with_hint(format!(
        "Supported privacy boundaries: {}.",
        PRIVACY_HOP_TYPES.join(", ")
    )))
}

/// `privacy_hops.privacy_boundary_from_import_record`.
fn privacy_boundary(record: &Value) -> Result<Option<String>> {
    for source in [
        Some(record.clone()),
        json_payload(field(record, "raw_json")),
    ] {
        let Some(payload) = source else {
            return Ok(None);
        };
        for key in PRIVACY_BOUNDARY_RAW_KEYS {
            if let Some(boundary) = explicit_boundary(&payload, key)? {
                return Ok(Some(boundary));
            }
        }
        if LIKELY_COINJOIN_KEYS
            .iter()
            .any(|key| boolish(get(&payload, key).unwrap_or(&Value::Null)))
        {
            return Ok(Some("coinjoin".into()));
        }
    }
    Ok(None)
}

fn identity_key(key: &Key) -> String {
    match key {
        Key::Str(text) => kassiber_pytext::lower(&py_str(&Value::Str(text.clone())))
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect(),
        _ => String::new(),
    }
}

/// `imports._external_id_kind`.
fn external_id_kind(
    record: &Value,
    raw: Option<&Value>,
    external_id: &str,
) -> Option<&'static str> {
    let canonical = import_canonical_txid(&Value::from(external_id))?;
    let declared = if truthy(field(record, "external_id_kind")) {
        field(record, "external_id_kind").clone()
    } else {
        raw.and_then(|raw| get(raw, "external_id_kind"))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let declared = kassiber_pytext::lower(kassiber_pytext::strip(&or_text(&declared)));
    if matches!(
        declared.as_str(),
        "txid" | "transaction_hash" | "onchain_txid"
    ) {
        return Some("txid");
    }
    let mut payloads = vec![record.clone()];
    if let Some(raw) = raw {
        payloads.push(raw.clone());
        for key in ["tx", "ownership_graph"] {
            if let Some(nested @ Value::Dict(_)) = get(raw, key) {
                payloads.push(nested.clone());
            }
        }
    }
    let keys = [
        "txid",
        "txhash",
        "transactionid",
        "transactionhash",
        "onchaintxid",
    ];
    payloads
        .iter()
        .any(|payload| match payload {
            Value::Dict(pairs) => pairs.iter().enumerate().any(|(index, (key, _))| {
                // A duplicate JSON key is visited once, with its last value.
                let value = &pairs[pairs
                    .iter()
                    .rposition(|(other, _)| other == key)
                    .unwrap_or(index)]
                .1;
                keys.contains(&identity_key(key).as_str())
                    && import_canonical_txid(value).as_deref() == Some(canonical.as_str())
            }),
            _ => false,
        })
        .then_some("txid")
}

fn asset_code(value: &Value) -> String {
    let text = kassiber_pytext::strip(&or_text(value)).to_owned();
    if text.is_empty() {
        return text;
    }
    let lowered = kassiber_pytext::lower(&text);
    if lowered.chars().count() == 64
        && lowered
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return lowered;
    }
    if matches!(lowered.as_str(), "l-btc" | "lbtc") {
        return "LBTC".into();
    }
    text.to_uppercase()
}

fn direction(direction: &Value, amount: &Decimal) -> Result<String> {
    if !truthy(direction) {
        return Ok(if amount.is_negative() {
            "outbound"
        } else {
            "inbound"
        }
        .into());
    }
    let value = kassiber_pytext::lower(kassiber_pytext::strip(&py_str(direction)));
    match value.as_str() {
        "in" | "inbound" | "receive" | "received" | "deposit" | "credit" | "buy" => {
            Ok("inbound".into())
        }
        "out" | "outbound" | "send" | "sent" | "withdrawal" | "withdraw" | "debit" | "sell" => {
            Ok("outbound".into())
        }
        _ => Err(AppError::new(
            "app_error",
            format!("Unsupported direction '{}'", py_str(direction)),
        )),
    }
}

fn hex64(value: Option<String>) -> Option<String> {
    let text = kassiber_pytext::lower(kassiber_pytext::strip(&value?));
    (text.chars().count() == 64 && hex_bytes(&text)).then_some(text)
}

/// `normalize_import_record` for the closed chain-observer record shape.
pub(crate) fn normalize(record: &Value, source_label: &str) -> Result<Value> {
    let Value::Dict(pairs) = record else {
        return Err(unsupported("record"));
    };
    if let Some((key, _)) = pairs.iter().find(|(key, _)| {
        !matches!(key, Key::Str(name) if name.as_str().is_some_and(|n| CHAIN_RECORD_KEYS.contains(&n)))
    }) {
        return Err(unsupported(&format!("{key:?}")));
    }
    for price in ["fiat_rate", "fiat_value"] {
        if !blank(field(record, price)) {
            return Err(unsupported(price));
        }
    }
    let Value::Str(raw_text) = field(record, "raw_json") else {
        return Err(unsupported("raw_json"));
    };
    let raw_amount = dec(field(record, "amount"), "0")?;
    let direction = direction(field(record, "direction"), &raw_amount)?;
    let amount = raw_amount
        .abs(&context())
        .map_err(|_| invalid_decimal(field(record, "amount")))?;
    let fee = dec(field(record, "fee"), "0")?
        .abs(&context())
        .map_err(|_| invalid_decimal(field(record, "fee")))?;
    let mut raw_json = Value::Str(raw_text.clone());
    let raw_payload = match kassiber_pytext::loads(&py_str(&raw_json)) {
        Ok(Value::Dict(mut payload)) => {
            payload.retain(|(key, _)| !matches!(key, Key::Str(name) if name.as_str() == Some("_kassiber_provenance")));
            let label =
                kassiber_pytext::lower(kassiber_pytext::strip(source_label)).replace('_', "-");
            if matches!(label.as_str(), "lnd" | "core-lightning") {
                return Err(unsupported("source_label"));
            }
            let payload = Value::Dict(payload);
            raw_json = Value::from(
                kassiber_pyjson::dumps(
                    &kassiber_envelope::json_ready(payload.clone()),
                    &kassiber_pyjson::Options::new().sort_keys(true),
                )
                .map_err(|_| unsupported("raw_json"))?,
            );
            Some(payload)
        }
        _ => None,
    };
    let occurred = field(record, "occurred_at");
    let occurred_at = parse_timestamp(occurred)?;
    let confirmed = field(record, "confirmed_at");
    let confirmed_at = if blank(confirmed) {
        Value::Null
    } else {
        Value::from(parse_timestamp(confirmed)?)
    };
    let payment_hash = hex64(str_or_none(field(record, "payment_hash")));
    let payment_hash_source = if payment_hash.is_some() {
        str_or_none(field(record, "payment_hash_source"))
    } else {
        None
    };
    let funding_txid = hex64(str_or_none(field(record, "swap_refund_funding_txid")));
    let funding_vout = match &funding_txid {
        None => Value::Null,
        Some(_) => match field(record, "swap_refund_funding_vout") {
            Value::Int(n) if *n >= 0.into() => Value::Int(n.clone()),
            Value::Bool(b) => Value::Int(i64::from(*b).into()),
            Value::Str(text) => text
                .as_str()
                .and_then(|text| kassiber_pytext::parse_int(text).ok())
                .filter(|n| *n >= 0.into())
                .map_or(Value::Null, Value::Int),
            Value::Float(f) if f.is_finite() && f.trunc() >= 0.0 => {
                Value::Int((f.trunc() as i64).into())
            }
            Value::Float(f) if f.is_infinite() => {
                return Err(unsupported("swap_refund_funding_vout"))
            }
            _ => Value::Null,
        },
    };
    let default_asset = Value::from("BTC");
    let asset = asset_code(if truthy(field(record, "asset")) {
        field(record, "asset")
    } else {
        &default_asset
    });
    let txid = field(record, "txid");
    let external_id = or_text(txid);
    Ok(dict([
        ("external_id", s(external_id.clone())),
        (
            "external_id_kind",
            external_id_kind(record, raw_payload.as_ref(), &external_id).map_or(Value::Null, s),
        ),
        ("occurred_at", s(occurred_at)),
        ("confirmed_at", confirmed_at),
        ("direction", s(direction)),
        ("asset", s(asset)),
        ("amount", Value::Decimal(amount)),
        ("fee", Value::Decimal(fee)),
        ("amount_includes_fee", Value::Bool(false)),
        ("fiat_currency", s("")),
        ("fiat_rate", Value::Null),
        ("fiat_value", Value::Null),
        ("fiat_rate_exact", Value::Null),
        ("fiat_value_exact", Value::Null),
        ("fiat_price_source", Value::Null),
        ("pricing_source_kind", Value::Null),
        ("pricing_provider", Value::Null),
        ("pricing_pair", Value::Null),
        ("pricing_timestamp", Value::Null),
        ("pricing_fetched_at", Value::Null),
        ("pricing_granularity", Value::Null),
        ("pricing_method", Value::Null),
        ("pricing_external_ref", Value::Null),
        ("pricing_quality", s("missing")),
        ("kind", field(record, "kind").clone()),
        ("privacy_boundary", opt(privacy_boundary(record)?)),
        ("description", field(record, "description").clone()),
        ("counterparty", field(record, "counterparty").clone()),
        ("payment_hash", opt(payment_hash)),
        ("payment_hash_source", opt(payment_hash_source)),
        ("swap_refund_funding_txid", opt(funding_txid)),
        ("swap_refund_funding_vout", funding_vout),
        ("raw_json", raw_json),
    ]))
}

fn unsupported(what: &str) -> AppError {
    AppError::new(
        "not_implemented",
        "Native publication accepts only closed Bitcoin chain-observer records",
    )
    .with_details(dict([("field", s(what))]))
}

fn decimal(normalized: &Value, key: &str) -> Decimal {
    match field(normalized, key) {
        Value::Decimal(d) => d.clone(),
        _ => unreachable!("normalized decimals"),
    }
}

/// `fingerprints.make_transaction_fingerprint`.
pub(crate) fn fingerprint(wallet_id: &str, normalized: &Value) -> Result<String> {
    use sha2::{Digest, Sha256};
    let payload = dict([
        ("wallet_id", s(wallet_id)),
        ("external_id", field(normalized, "external_id").clone()),
        ("occurred_at", field(normalized, "occurred_at").clone()),
        ("direction", field(normalized, "direction").clone()),
        ("asset", field(normalized, "asset").clone()),
        ("amount", s(decimal(normalized, "amount").to_string())),
        ("fee", s(decimal(normalized, "fee").to_string())),
    ]);
    let text = kassiber_pyjson::dumps(&payload, &kassiber_pyjson::Options::new().sort_keys(true))
        .map_err(|_| AppError::new("app_error", "Fingerprint cannot be encoded"))?;
    Ok(Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn sql(value: &Value) -> Result<Sql> {
    Ok(match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Int(n) => Sql::Integer(i64::try_from(n.clone()).map_err(|_| {
            AppError::new(
                "app_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        })?),
        Value::Float(f) => Sql::Real(*f),
        Value::Str(_) => Sql::Text(py_str(value)),
        _ => return Err(AppError::new("app_error", "Unsupported SQL value")),
    })
}

fn conflict(
    message: &str,
    kind: &str,
    txid: &str,
    normalized: &Value,
    matches: usize,
    active: usize,
    hint: &str,
) -> AppError {
    AppError::new("observer_projection_conflict", message)
        .with_hint(hint)
        .with_details(dict([
            ("conflict_kind", s(kind)),
            ("txid", s(txid)),
            ("asset", field(normalized, "asset").clone()),
            ("direction", field(normalized, "direction").clone()),
            ("match_count", Value::Int(matches.into())),
            ("active_match_count", Value::Int(active.into())),
        ]))
}

fn has_authored_state(conn: &Connection, row: &Value) -> Result<bool> {
    if AUTHORED_COLUMNS.iter().any(|c| !blank(field(row, c))) {
        return Ok(true);
    }
    Ok(first(
        conn,
        "SELECT 1 FROM transaction_tags WHERE transaction_id = ? LIMIT 1",
        [sql(field(row, "id"))?],
    )?
    .is_some())
}

fn superseded_native(conn: &Connection, row: &Value) -> Result<bool> {
    let Some(provenance) = first(
        conn,
        "SELECT authority_version, graph_hash, quantity_hash FROM chain_observation_provenance WHERE transaction_id = ?",
        [sql(field(row, "id"))?],
    )?
    else {
        return Ok(false);
    };
    let mut candidate = row.clone();
    for (target, source) in [
        ("observation_authority_version", "authority_version"),
        ("observation_graph_hash", "graph_hash"),
        ("observation_quantity_hash", "quantity_hash"),
    ] {
        set(&mut candidate, target, field(&provenance, source).clone());
    }
    super::physical::current_authority(&candidate)
}

/// `importers.GENERIC_LEDGER_KIND_DIRECTIONS.get(kind)`.
fn kind_direction(kind: &Value) -> Option<&'static str> {
    Some(match kind.as_str_opt()? {
        "airdrop" | "buy" | "deposit" | "hardfork" | "income" | "interest" | "lending_interest"
        | "mining" | "staking" | "wages" => "inbound",
        "donation" | "gift" | "lost" | "sell" | "spend" | "stolen" | "withdrawal" => "outbound",
        _ => return None,
    })
}

trait AsStrOpt {
    fn as_str_opt(&self) -> Option<&str>;
}
impl AsStrOpt for Value {
    fn as_str_opt(&self) -> Option<&str> {
        match self {
            Value::Str(text) => text.as_str(),
            _ => None,
        }
    }
}

/// `_find_authoritative_chain_transaction`; returns the keeper and the rows it supersedes.
fn find_authoritative(
    conn: &Connection,
    wallet_id: &str,
    normalized: &Value,
    fingerprint: &str,
) -> Result<(Option<Value>, Vec<Value>)> {
    let Some(external) = canonical_txid(field(normalized, "external_id")) else {
        return Ok((None, vec![]));
    };
    let candidates = rows(
        conn,
        &format!(
            "SELECT {EXISTING_COLUMNS} FROM transactions WHERE wallet_id = ? AND external_id IS NOT NULL
             AND LOWER(external_id) = ? AND asset = ? ORDER BY created_at DESC, id DESC"
        ),
        params_from_iter([
            Sql::Text(wallet_id.into()),
            Sql::Text(external.clone()),
            sql(field(normalized, "asset"))?,
        ]),
    )?;
    if candidates.is_empty() {
        return Ok((None, vec![]));
    }
    let direction = field(normalized, "direction");
    let excluded = |row: &Value| truthy(field(row, "excluded"));
    let same: Vec<&Value> = candidates
        .iter()
        .filter(|row| py_eq(field(row, "direction"), direction))
        .collect();
    let other: Vec<&Value> = candidates
        .iter()
        .filter(|row| !py_eq(field(row, "direction"), direction))
        .collect();
    let active_same: Vec<&Value> = same.iter().copied().filter(|r| !excluded(r)).collect();
    let active_other: Vec<&Value> = other.iter().copied().filter(|r| !excluded(r)).collect();
    let mut obsolete = Vec::new();
    for row in &active_other {
        if superseded_native(conn, row)? {
            obsolete.push(*row);
        }
    }
    let excluded_same: Vec<&Value> = same.iter().copied().filter(|r| excluded(r)).collect();
    let exact: Vec<&Value> = candidates
        .iter()
        .filter(|row| py_eq(field(row, "fingerprint"), &Value::from(fingerprint)))
        .collect();
    let active = active_same.len() + active_other.len();
    let fail = |message: &str, kind: &str, hint: &str| {
        conflict(
            message,
            kind,
            &external,
            normalized,
            candidates.len(),
            active,
            hint,
        )
    };
    let ambiguous = "Authoritative chain observation matched ambiguous transaction rows";
    let keeper = if active_same.len() > 1 {
        return Err(fail(
            ambiguous,
            "multiple_active_transaction_rows",
            DUPLICATE_HINT,
        ));
    } else if let Some(row) = active_same.first() {
        *row
    } else if !excluded_same.is_empty() && !obsolete.is_empty() {
        return Err(fail(
            "An excluded transaction owns this observation direction",
            "excluded_direction_owns_observation",
            DUPLICATE_HINT,
        ));
    } else if obsolete.len() > 1 {
        return Err(fail(
            ambiguous,
            "multiple_active_transaction_rows",
            DUPLICATE_HINT,
        ));
    } else if let Some(row) = obsolete.first() {
        *row
    } else if same.len() > 1 {
        return Err(fail(
            ambiguous,
            "multiple_excluded_transaction_rows",
            DUPLICATE_HINT,
        ));
    } else if let [row] = same.as_slice() {
        *row
    } else {
        return Ok((None, vec![]));
    };
    if let Some(row) = exact.first() {
        if !py_eq(field(row, "id"), field(keeper, "id")) && excluded(row) {
            return Err(fail(
                "An excluded transaction owns the authoritative observer fingerprint",
                "excluded_exact_transaction_row",
                DUPLICATE_HINT,
            ));
        }
    }
    if !py_eq(field(keeper, "direction"), direction) {
        let kind_override = field(keeper, "kind_override");
        if truthy(kind_override) && kind_direction(kind_override) != direction.as_str_opt() {
            return Err(fail(
                "An authored transaction kind contradicts the observed direction",
                "authored_kind_contradicts_direction",
                AUTHORED_HINT,
            ));
        }
    }
    let mut superseded = Vec::new();
    for row in obsolete {
        if py_eq(field(row, "id"), field(keeper, "id")) {
            continue;
        }
        if has_authored_state(conn, row)? {
            return Err(fail(
                "A superseded observer projection carries authored metadata",
                "authored_superseded_transaction_row",
                AUTHORED_HINT,
            ));
        }
        superseded.push(row.clone());
    }
    Ok((Some(keeper.clone()), superseded))
}

/// `_find_existing_transaction` for a record whose txid is not canonical.
fn find_by_fingerprint(
    conn: &Connection,
    wallet_id: &str,
    normalized: &Value,
    fingerprint: &str,
) -> Result<Option<Value>> {
    if let Some(row) = first(
        conn,
        &format!("SELECT {EXISTING_COLUMNS} FROM transactions WHERE fingerprint = ?"),
        [fingerprint],
    )? {
        return Ok(Some(row));
    }
    if !truthy(field(normalized, "external_id")) {
        return Ok(None);
    }
    first(
        conn,
        &format!(
            "SELECT {EXISTING_COLUMNS} FROM transactions WHERE wallet_id = ? AND external_id = ?
             AND direction = ? AND asset = ? AND amount = ? AND fee = ? ORDER BY created_at DESC LIMIT 1"
        ),
        params_from_iter([
            Sql::Text(wallet_id.into()),
            sql(field(normalized, "external_id"))?,
            sql(field(normalized, "direction"))?,
            sql(field(normalized, "asset"))?,
            Sql::Integer(btc_to_msat(&decimal(normalized, "amount"))?),
            Sql::Integer(btc_to_msat(&decimal(normalized, "fee"))?),
        ]),
    )
}

fn raw_payload(row: &Value) -> Option<Value> {
    let raw = field(row, "raw_json");
    if !truthy(raw) {
        return None;
    }
    match raw {
        Value::Dict(_) => Some(raw.clone()),
        Value::Str(_) => match kassiber_pytext::loads(&py_str(raw)) {
            Ok(parsed @ Value::Dict(_)) => Some(parsed),
            _ => None,
        },
        _ => None,
    }
}

fn privacy_import_authored(existing: &Value) -> bool {
    let current = field(existing, "privacy_boundary");
    if blank(current) {
        return false;
    }
    let Some(payload) = raw_payload(existing) else {
        return false;
    };
    match privacy_boundary(&payload) {
        Ok(Some(boundary)) => py_eq(&Value::from(boundary), current),
        _ => false,
    }
}

fn same_raw_source(existing: &Value, normalized: &Value) -> bool {
    let (Some(old), Some(new)) = (raw_payload(existing), raw_payload(normalized)) else {
        return false;
    };
    let source = get(&old, "source").unwrap_or(&Value::Null);
    truthy(source) && py_eq(source, get(&new, "source").unwrap_or(&Value::Null))
}

fn same_wasabi_source(existing: &Value, normalized: &Value) -> bool {
    same_raw_source(existing, normalized)
        && raw_payload(existing).is_some_and(|payload| {
            py_eq(
                get(&payload, "source").unwrap_or(&Value::Null),
                &Value::from("wasabi_gethistory"),
            )
        })
}

fn graph_version(row: &Value) -> i64 {
    let Some(payload) = raw_payload(row) else {
        return 0;
    };
    match get(&payload, "ownership_graph_version") {
        Some(Value::Int(n)) if *n >= 0.into() => i64::try_from(n.clone()).unwrap_or(i64::MAX),
        _ => 0,
    }
}

type Evidence = Vec<(Vec<Value>, Value)>;

fn graph_evidence(row: &Value) -> Evidence {
    let Some(payload) = raw_payload(row) else {
        return vec![];
    };
    if graph_version(row) <= 0 {
        return vec![];
    }
    let mut evidence: Evidence = Vec::new();
    let mut put = |key: Vec<Value>, value: Value| {
        if let Some(slot) = evidence.iter_mut().find(|(k, _)| keys_eq(k, &key)) {
            slot.1 = value;
        } else {
            evidence.push((key, value));
        }
    };
    let fields = ["scriptpubkey", "value_sats", "asset_id", "asset", "role"];
    if let Some(vin) = get(&payload, "vin").filter(|v| truthy(v)) {
        for (position, entry) in iterate(vin).into_iter().enumerate() {
            if !matches!(entry, Value::Dict(_)) {
                continue;
            }
            let txid = Value::from(or_text(get(&entry, "txid").unwrap_or(&Value::Null)));
            let vout = get(&entry, "vout")
                .cloned()
                .unwrap_or(Value::Int(position.into()));
            let Some(prevout @ Value::Dict(_)) = get(&entry, "prevout") else {
                continue;
            };
            for name in fields {
                let value = get(prevout, name).unwrap_or(&Value::Null);
                if !blank(value) {
                    put(
                        vec![s("vin"), txid.clone(), vout.clone(), s(name)],
                        value.clone(),
                    );
                }
            }
        }
    }
    if let Some(vout) = get(&payload, "vout").filter(|v| truthy(v)) {
        for (position, entry) in iterate(vout).into_iter().enumerate() {
            if !matches!(entry, Value::Dict(_)) {
                continue;
            }
            let n = get(&entry, "n")
                .cloned()
                .unwrap_or(Value::Int(position.into()));
            for name in fields {
                let value = get(&entry, name).unwrap_or(&Value::Null);
                if !blank(value) {
                    put(vec![s("vout"), n.clone(), s(name)], value.clone());
                }
            }
        }
    }
    evidence
}

/// Python iterates a list's items or a dict's keys; other truthy values raise.
fn iterate(value: &Value) -> Vec<Value> {
    match value {
        Value::List(items) => items.clone(),
        Value::Str(_) => py_str(value)
            .chars()
            .map(|c| Value::from(c.to_string()))
            .collect(),
        Value::Dict(pairs) => pairs
            .iter()
            .map(|(key, _)| match key {
                Key::Str(text) => Value::Str(text.clone()),
                _ => Value::Null,
            })
            .collect(),
        _ => vec![],
    }
}

/// Tuple keys compare with Python equality (1 == 1.0 == True).
fn keys_eq(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| py_eq(a, b))
}

fn strict_enrichment(existing: &Value, incoming: &Value) -> bool {
    let version = graph_version(existing);
    if version <= 0 || graph_version(incoming) != version {
        return false;
    }
    let old = graph_evidence(existing);
    let new = graph_evidence(incoming);
    let mut enriched = false;
    for (key, old_value) in &old {
        let Some((_, new_value)) = new.iter().find(|(k, _)| keys_eq(k, key)) else {
            return false;
        };
        if py_eq(new_value, old_value) {
            continue;
        }
        let pair = [py_str(old_value), py_str(new_value)];
        if key.last().is_some_and(|k| py_eq(k, &Value::from("role")))
            && pair.contains(&"external".to_owned())
            && pair.contains(&"owned".to_owned())
            && pair[0] != pair[1]
        {
            if pair[1] != "owned" {
                return false;
            }
            enriched = true;
            continue;
        }
        return false;
    }
    if new
        .iter()
        .any(|(key, _)| !old.iter().any(|(k, _)| keys_eq(k, key)))
    {
        enriched = true;
    }
    enriched
}

fn attestation_upgrade(existing: &Value, incoming: &Value) -> bool {
    let (Some(old), Some(new)) = (raw_payload(existing), raw_payload(incoming)) else {
        return false;
    };
    if get(&old, "observer_owned_scripts").is_some() {
        return false;
    }
    matches!(get(&new, "observer_owned_scripts"), Some(Value::List(items)) if !items.is_empty())
}

fn script_set(items: &[Value]) -> std::collections::BTreeSet<String> {
    items
        .iter()
        .filter(|value| truthy(value))
        .map(|value| kassiber_pytext::lower(kassiber_pytext::strip(&py_str(value))))
        .collect()
}

/// `_attestation_rescope`: the stored payload with its attestation narrowed.
fn attestation_rescope(existing: &Value, incoming: &Value) -> Result<Option<String>> {
    let (Some(old), Some(new)) = (raw_payload(existing), raw_payload(incoming)) else {
        return Ok(None);
    };
    let (Some(Value::List(old_scripts)), Some(Value::List(new_scripts))) = (
        get(&old, "observer_owned_scripts"),
        get(&new, "observer_owned_scripts"),
    ) else {
        return Ok(None);
    };
    let old_set = script_set(old_scripts);
    let new_set = script_set(new_scripts);
    if new_set.is_empty() || !(new_set.is_subset(&old_set) && new_set != old_set) {
        return Ok(None);
    }
    let scoped = scoped_scripts(&old, &old_set);
    let narrowed: Vec<String> = new_set.into_iter().collect();
    if scoped != narrowed {
        return Ok(None);
    }
    let mut payload = old;
    set(
        &mut payload,
        "observer_owned_scripts",
        Value::List(narrowed.into_iter().map(Value::from).collect()),
    );
    Ok(Some(dumps(&payload)?))
}

/// `onchain.graph_scoped_scripts` over a decoded Python payload.
fn scoped_scripts(graph: &Value, given: &std::collections::BTreeSet<String>) -> Vec<String> {
    let mut in_graph = std::collections::BTreeSet::new();
    if let Some(vin) = get(graph, "vin").filter(|v| truthy(v)) {
        for entry in iterate(vin) {
            let script = if matches!(entry, Value::Dict(_)) {
                match get(&entry, "prevout") {
                    Some(prevout @ Value::Dict(_)) => output_script(prevout),
                    _ => Value::Null,
                }
            } else {
                Value::Null
            };
            if !truthy(&script) {
                return given.iter().cloned().collect();
            }
            in_graph.insert(kassiber_pytext::lower(kassiber_pytext::strip(&py_str(
                &script,
            ))));
        }
    }
    if let Some(vout) = get(graph, "vout").filter(|v| truthy(v)) {
        for entry in iterate(vout) {
            let script = if matches!(entry, Value::Dict(_)) {
                output_script(&entry)
            } else {
                Value::Null
            };
            if truthy(&script) {
                in_graph.insert(kassiber_pytext::lower(kassiber_pytext::strip(&py_str(
                    &script,
                ))));
            }
        }
    }
    given.intersection(&in_graph).cloned().collect()
}

/// `onchain.output_script`.
fn output_script(entry: &Value) -> Value {
    let mut script = get(entry, "scriptpubkey").cloned().unwrap_or(Value::Null);
    if matches!(script, Value::Null) {
        script = get(entry, "script_hex").cloned().unwrap_or(Value::Null);
    }
    let nested = get(entry, "scriptPubKey");
    if matches!(script, Value::Null) {
        match nested {
            Some(nested @ Value::Dict(_)) => {
                let hex = get(nested, "hex").unwrap_or(&Value::Null);
                script = if truthy(hex) {
                    hex.clone()
                } else {
                    get(nested, "scriptpubkey").cloned().unwrap_or(Value::Null)
                };
            }
            Some(text @ Value::Str(_)) => script = text.clone(),
            _ => {}
        }
    }
    script
}

fn dumps(value: &Value) -> Result<String> {
    kassiber_pyjson::dumps(value, &kassiber_pyjson::Options::new().sort_keys(true))
        .map_err(|_| AppError::new("app_error", "Stored payload cannot be encoded"))
}

/// `_refreshed_block_occurrence`.
fn refreshed_block(existing: &Value, normalized: &Value) -> Result<Option<String>> {
    let txid = canonical_txid(field(existing, "external_id"));
    if txid.is_none()
        || txid != canonical_txid(field(normalized, "external_id"))
        || !py_eq(field(existing, "direction"), field(normalized, "direction"))
        || !py_eq(field(existing, "asset"), field(normalized, "asset"))
    {
        return Ok(None);
    }
    let old = raw_payload(existing).unwrap_or(Value::Dict(vec![]));
    let new = raw_payload(normalized).unwrap_or(Value::Dict(vec![]));
    let null = Value::Null;
    let chain = get(&old, "chain").unwrap_or(&null);
    let chain_name = match chain.as_str_opt() {
        Some(name @ ("bitcoin" | "liquid")) => name,
        _ => return Ok(None),
    };
    if !py_eq(chain, get(&new, "chain").unwrap_or(&null))
        || !truthy(get(&old, "network").unwrap_or(&null))
        || !truthy(get(&new, "network").unwrap_or(&null))
        || !py_eq(
            get(&old, "chain_instance_id").unwrap_or(&null),
            get(&new, "chain_instance_id").unwrap_or(&null),
        )
    {
        return Ok(None);
    }
    let old_network = canonical_network(chain_name, get(&old, "network").unwrap_or(&null));
    let new_network = canonical_network(chain_name, get(&new, "network").unwrap_or(&null));
    if old_network.is_none() || new_network.is_none() || old_network != new_network {
        return Ok(None);
    }
    let (Some(old_status @ Value::Dict(_)), Some(new_status @ Value::Dict(_))) =
        (get(&old, "status"), get(&new, "status"))
    else {
        return Ok(None);
    };
    if get(old_status, "confirmed") != Some(&Value::Bool(true))
        || get(new_status, "confirmed") != Some(&Value::Bool(true))
    {
        return Ok(None);
    }
    let Some(block_hash) = canonical_txid(get(new_status, "block_hash").unwrap_or(&null)) else {
        return Ok(None);
    };
    let height = match get(new_status, "block_height") {
        Some(Value::Int(n)) if *n >= 0.into() && i64::try_from(n.clone()).is_ok() => n.clone(),
        _ => return Ok(None),
    };
    let old_hash = canonical_txid(get(old_status, "block_hash").unwrap_or(&null));
    if old_hash.as_deref() == Some(block_hash.as_str())
        && py_eq(
            get(old_status, "block_height").unwrap_or(&null),
            &Value::Int(height.clone()),
        )
    {
        return Ok(None);
    }
    let mut status = old_status.clone();
    set(&mut status, "block_hash", Value::from(block_hash));
    set(&mut status, "block_height", Value::Int(height));
    let mut payload = old;
    set(&mut payload, "status", status);
    Ok(Some(dumps(&payload)?))
}

#[derive(Default)]
struct Updates(Vec<(String, Value)>);
impl Updates {
    fn put(&mut self, key: &str, value: Value) {
        if let Some(slot) = self.0.iter_mut().find(|(k, _)| k == key) {
            slot.1 = value;
        } else {
            self.0.push((key.into(), value));
        }
    }
    fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn recomputed_value(existing: &Value, amount: &Decimal, updates: &mut Updates) -> Result<()> {
    let exact = field(existing, "fiat_rate_exact");
    let rate = if truthy(exact) {
        exact
    } else {
        field(existing, "fiat_rate")
    };
    if !blank(rate) {
        let product = dec(rate, "0")?
            .mul(amount, &context())
            .map_err(|_| invalid_decimal(rate))?;
        let text = product
            .to_fixed_string()
            .map_err(|_| invalid_decimal(rate))?;
        let float: f64 = text.parse().map_err(|_| invalid_decimal(rate))?;
        updates.put("fiat_value", Value::Float(float));
        updates.put("fiat_value_exact", Value::from(text));
    } else if !matches!(field(existing, "fiat_value"), Value::Null)
        || !matches!(field(existing, "fiat_value_exact"), Value::Null)
    {
        updates.put("fiat_value", Value::Null);
        updates.put("fiat_value_exact", Value::Null);
    }
    Ok(())
}

/// `_transaction_merge_updates(..., authoritative_chain_observer=True)`.
fn merge_updates(existing: &Value, normalized: &Value, fingerprint: &str) -> Result<Updates> {
    let mut updates = Updates::default();
    let n = |key: &str| field(normalized, key);
    let e = |key: &str| field(existing, key);
    let fp = Value::from(fingerprint);
    if n("external_id_kind") == &Value::from("txid")
        && !py_eq(e("external_id_kind"), &Value::from("txid"))
    {
        updates.put("external_id_kind", s("txid"));
        if !py_eq(e("external_id"), n("external_id")) {
            updates.put("external_id", n("external_id").clone());
        }
    }
    let amount = decimal(normalized, "amount");
    let incoming_amount = btc_to_msat(&amount)?;
    let incoming_fee = btc_to_msat(&decimal(normalized, "fee"))?;
    let incoming_includes_fee = i64::from(truthy(n("amount_includes_fee")));
    if !py_eq(e("external_id"), n("external_id")) {
        updates.put("external_id", n("external_id").clone());
    }
    let existing_direction = e("direction");
    if !matches!(existing_direction, Value::Null) && !py_eq(existing_direction, n("direction")) {
        updates.put("direction", n("direction").clone());
        let kind = e("kind");
        if truthy(n("kind"))
            && (!truthy(kind)
                || kind
                    .as_str_opt()
                    .is_some_and(|k| OBSERVER_DERIVED_KINDS.contains(&k)))
        {
            updates.put("kind", n("kind").clone());
        }
    }
    let amount_changed = int_or_zero(e("amount"))? != incoming_amount;
    if amount_changed {
        updates.put("amount", Value::Int(incoming_amount.into()));
    }
    if int_or_zero(e("fee"))? != incoming_fee {
        updates.put("fee", Value::Int(incoming_fee.into()));
    }
    if int_or_zero(e("amount_includes_fee"))? != incoming_includes_fee {
        updates.put(
            "amount_includes_fee",
            Value::Int(incoming_includes_fee.into()),
        );
    }
    if !updates.is_empty() && !py_eq(e("fingerprint"), &fp) {
        updates.put("fingerprint", fp.clone());
    }
    if n("kind") == &Value::from("routing_income") {
        return Err(unsupported("kind"));
    }
    let unknown = Value::from(UNKNOWN_OCCURRED_AT);
    if py_eq(e("occurred_at"), &unknown) && !py_eq(n("occurred_at"), &unknown) {
        updates.put("occurred_at", n("occurred_at").clone());
    }
    let stored = updates
        .get("occurred_at")
        .cloned()
        .unwrap_or_else(|| e("occurred_at").clone());
    if !py_eq(e("fingerprint"), &fp) && py_eq(&stored, n("occurred_at")) {
        updates.put("fingerprint", fp.clone());
    }
    let incoming_confirmed = n("confirmed_at");
    let added = blank(e("confirmed_at")) && !matches!(incoming_confirmed, Value::Null);
    let changed =
        !matches!(incoming_confirmed, Value::Null) && !py_eq(e("confirmed_at"), incoming_confirmed);
    let removed = !blank(e("confirmed_at")) && matches!(incoming_confirmed, Value::Null);
    if added || changed || removed {
        updates.put("confirmed_at", incoming_confirmed.clone());
        if !py_eq(e("occurred_at"), n("occurred_at")) {
            updates.put("occurred_at", n("occurred_at").clone());
        }
        if !py_eq(e("fingerprint"), &fp) {
            updates.put("fingerprint", fp.clone());
        }
    }
    let cached_time_price = py_eq(e("fiat_price_source"), &Value::from(RATES_CACHE));
    if (added || changed || removed) && cached_time_price {
        for column in PRICE_COLUMNS {
            updates.put(column, Value::Null);
        }
    } else if amount_changed {
        recomputed_value(existing, &amount, &mut updates)?;
    }
    let refreshable = same_wasabi_source(existing, normalized)
        && e("kind")
            .as_str_opt()
            .is_some_and(|k| matches!(k, "coinjoin" | "deposit" | "withdrawal"));
    if truthy(n("kind")) && !py_eq(e("kind"), n("kind")) && (!truthy(e("kind")) || refreshable) {
        updates.put("kind", n("kind").clone());
    }
    if truthy(n("privacy_boundary"))
        && !py_eq(e("privacy_boundary"), n("privacy_boundary"))
        && (!truthy(e("privacy_boundary")) || privacy_import_authored(existing))
    {
        updates.put("privacy_boundary", n("privacy_boundary").clone());
    } else if truthy(e("privacy_boundary"))
        && matches!(n("privacy_boundary"), Value::Null)
        && privacy_import_authored(existing)
        && same_raw_source(existing, normalized)
    {
        updates.put("privacy_boundary", Value::Null);
    }
    if !truthy(e("description")) && truthy(n("description")) {
        updates.put("description", n("description").clone());
    }
    if !truthy(e("counterparty")) && truthy(n("counterparty")) {
        updates.put("counterparty", n("counterparty").clone());
    }
    if !truthy(e("payment_hash")) && truthy(n("payment_hash")) {
        updates.put("payment_hash", n("payment_hash").clone());
        updates.put("payment_hash_source", n("payment_hash_source").clone());
    } else if truthy(e("payment_hash"))
        && py_eq(e("payment_hash"), n("payment_hash"))
        && py_eq(e("payment_hash_source"), &Value::from("chain_script"))
        && py_eq(
            n("payment_hash_source"),
            &Value::from("chain_script_unique_outpoint"),
        )
    {
        updates.put("payment_hash_source", n("payment_hash_source").clone());
    }
    if !truthy(e("swap_refund_funding_txid")) && truthy(n("swap_refund_funding_txid")) {
        updates.put(
            "swap_refund_funding_txid",
            n("swap_refund_funding_txid").clone(),
        );
        updates.put(
            "swap_refund_funding_vout",
            n("swap_refund_funding_vout").clone(),
        );
    } else if truthy(n("swap_refund_funding_txid"))
        && py_eq(e("swap_refund_funding_txid"), n("swap_refund_funding_txid"))
        && matches!(e("swap_refund_funding_vout"), Value::Null)
        && !matches!(n("swap_refund_funding_vout"), Value::Null)
    {
        updates.put(
            "swap_refund_funding_vout",
            n("swap_refund_funding_vout").clone(),
        );
    }
    let graph_upgrade = graph_version(normalized) > graph_version(existing)
        || strict_enrichment(existing, normalized);
    let attestation = attestation_upgrade(existing, normalized);
    if (!updates.is_empty() || graph_upgrade || attestation)
        && truthy(n("raw_json"))
        && !py_eq(n("raw_json"), e("raw_json"))
    {
        updates.put("raw_json", n("raw_json").clone());
    } else if let Some(refreshed) = refreshed_block(existing, normalized)? {
        updates.put("raw_json", Value::from(refreshed));
    }
    Ok(updates)
}

fn change_record(
    id: &Value,
    wallet_label: &str,
    normalized: &Value,
    fields: Vec<String>,
) -> Result<Value> {
    let mut fields = fields;
    fields.sort();
    Ok(dict([
        ("transaction_id", id.clone()),
        ("external_id", field(normalized, "external_id").clone()),
        ("wallet", s(wallet_label)),
        ("asset", field(normalized, "asset").clone()),
        ("direction", field(normalized, "direction").clone()),
        (
            "amount_msat",
            Value::Int(btc_to_msat(&decimal(normalized, "amount"))?.into()),
        ),
        (
            "changed_fields",
            Value::List(fields.into_iter().map(Value::from).collect()),
        ),
        (
            "pricing_external_ref",
            field(normalized, "pricing_external_ref").clone(),
        ),
    ]))
}

/// The scoped wallet and profile values the writer needs.
pub struct Scope<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
    pub profile_fiat_currency: &'a str,
    pub wallet_id: &'a str,
    pub wallet_label: &'a str,
}

/// `insert_wallet_records(..., commit=False, authoritative_chain_observer=True)`.
/// `new_id` and `now` replace `uuid4()` and `now_iso()` for deterministic tests.
pub fn insert_records(
    conn: &Connection,
    scope: &Scope<'_>,
    records: &[Value],
    source_label: &str,
    new_id: &mut dyn FnMut() -> String,
    now: &mut dyn FnMut() -> String,
) -> Result<Value> {
    if conn.is_autocommit() {
        return Err(AppError::new(
            "observer_apply_outside_transaction",
            "Record insertion requires the caller's wallet transaction",
        ));
    }
    let wallet = first(
        conn,
        "SELECT kind,config_json FROM wallets WHERE id=? AND profile_id=?",
        [scope.wallet_id, scope.profile_id],
    )?
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            "Import wallet does not belong to the selected book",
        )
    })?;
    let mut normalized: Vec<Value> = records
        .iter()
        .map(|record| normalize(record, source_label))
        .collect::<Result<_>>()?;
    for record in &normalized {
        let mut guarded = record.clone();
        set(&mut guarded, "wallet_kind", field(&wallet, "kind").clone());
        set(
            &mut guarded,
            "wallet_config_json",
            field(&wallet, "config_json").clone(),
        );
        kassiber_setup::networks::guard_observation(conn, scope.profile_id, &guarded, "import")?;
    }
    let mut projected: Vec<((String, Value), Value)> = Vec::new();
    for record in &mut normalized {
        let Some(external) = canonical_txid(field(record, "external_id")) else {
            continue;
        };
        set(record, "external_id", Value::from(external.clone()));
        set(record, "external_id_kind", s("txid"));
        let key = (external.clone(), field(record, "asset").clone());
        match projected
            .iter()
            .find(|(k, _)| k.0 == key.0 && py_eq(&k.1, &key.1))
        {
            Some((_, direction)) if !py_eq(direction, field(record, "direction")) => {
                return Err(AppError::new(
                    "observer_projection_conflict",
                    "One chain observation produced opposite wallet directions for the same transaction",
                )
                .with_hint(RECORD_CONFLICT_HINT)
                .with_details(dict([
                    ("conflict_kind", s("conflicting_observer_projection_records")),
                    ("txid", s(external)),
                    ("asset", field(record, "asset").clone()),
                ])));
            }
            Some(_) => {}
            None => projected.push((key, field(record, "direction").clone())),
        }
    }
    let (mut imported, mut skipped, mut updated, mut unchanged, mut superseded, mut rescoped) =
        (0, 0, 0, 0, 0, 0);
    let mut inserted_records = Vec::new();
    let mut updated_records = Vec::new();
    let mut resolved = Vec::new();
    let mut superseded_records = Vec::new();
    for record in &normalized {
        let fingerprint = fingerprint(scope.wallet_id, record)?;
        let (existing, obsolete) = if canonical_txid(field(record, "external_id")).is_some() {
            find_authoritative(conn, scope.wallet_id, record, &fingerprint)?
        } else {
            (
                find_by_fingerprint(conn, scope.wallet_id, record, &fingerprint)?,
                vec![],
            )
        };
        for row in obsolete {
            conn.execute(
                "UPDATE transactions SET excluded = 1 WHERE id = ?",
                [sql(field(&row, "id"))?],
            )
            .map_err(database_error)?;
            superseded += 1;
            superseded_records.push(dict([
                ("transaction_id", field(&row, "id").clone()),
                ("wallet", s(scope.wallet_label)),
                ("external_id", field(&row, "external_id").clone()),
                ("asset", field(&row, "asset").clone()),
                ("direction", field(&row, "direction").clone()),
            ]));
        }
        let resolution = |id: &Value| {
            dict([
                ("transaction_id", s(py_str(id))),
                ("external_id", s(py_str(field(record, "external_id")))),
                ("asset", s(py_str(field(record, "asset")))),
                ("direction", s(py_str(field(record, "direction")))),
            ])
        };
        if let Some(existing) = existing {
            resolved.push(resolution(field(&existing, "id")));
            let updates = merge_updates(&existing, record, &fingerprint)?;
            if !updates.is_empty() {
                let assignments = updates
                    .0
                    .iter()
                    .map(|(column, _)| format!("{column} = ?"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut values = updates
                    .0
                    .iter()
                    .map(|(_, value)| sql(value))
                    .collect::<Result<Vec<_>>>()?;
                values.push(sql(field(&existing, "id"))?);
                conn.execute(
                    &format!("UPDATE transactions SET {assignments} WHERE id = ?"),
                    params_from_iter(values),
                )
                .map_err(database_error)?;
                updated += 1;
                updated_records.push(change_record(
                    field(&existing, "id"),
                    scope.wallet_label,
                    record,
                    updates.0.iter().map(|(k, _)| k.clone()).collect(),
                )?);
            } else {
                if let Some(narrowed) = attestation_rescope(&existing, record)? {
                    conn.execute(
                        "UPDATE transactions SET raw_json = ? WHERE id = ?",
                        params_from_iter([Sql::Text(narrowed), sql(field(&existing, "id"))?]),
                    )
                    .map_err(database_error)?;
                    rescoped += 1;
                }
                unchanged += 1;
            }
            skipped += 1;
            continue;
        }
        let id = new_id();
        let n = |key: &str| sql(field(record, key));
        let currency = field(record, "fiat_currency");
        let values = vec![
            Sql::Text(id.clone()),
            Sql::Text(scope.workspace_id.into()),
            Sql::Text(scope.profile_id.into()),
            Sql::Text(scope.wallet_id.into()),
            if truthy(field(record, "external_id")) {
                n("external_id")?
            } else {
                Sql::Null
            },
            n("external_id_kind")?,
            Sql::Text(fingerprint.clone()),
            n("occurred_at")?,
            n("confirmed_at")?,
            n("direction")?,
            n("asset")?,
            Sql::Integer(btc_to_msat(&decimal(record, "amount"))?),
            Sql::Integer(btc_to_msat(&decimal(record, "fee"))?),
            Sql::Integer(i64::from(truthy(field(record, "amount_includes_fee")))),
            if truthy(currency) {
                sql(currency)?
            } else {
                Sql::Text(scope.profile_fiat_currency.into())
            },
            n("fiat_rate")?,
            n("fiat_value")?,
            n("fiat_price_source")?,
            n("fiat_rate_exact")?,
            n("fiat_value_exact")?,
            n("pricing_source_kind")?,
            n("pricing_provider")?,
            n("pricing_pair")?,
            n("pricing_timestamp")?,
            n("pricing_fetched_at")?,
            n("pricing_granularity")?,
            n("pricing_method")?,
            n("pricing_external_ref")?,
            n("pricing_quality")?,
            n("kind")?,
            n("privacy_boundary")?,
            n("description")?,
            n("counterparty")?,
            n("raw_json")?,
            n("payment_hash")?,
            n("payment_hash_source")?,
            n("swap_refund_funding_txid")?,
            n("swap_refund_funding_vout")?,
            Sql::Text(now()),
        ];
        conn.execute(
            "INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                external_id_kind, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                amount_includes_fee, fiat_currency,
                fiat_rate, fiat_value, fiat_price_source, fiat_rate_exact,
                fiat_value_exact, pricing_source_kind, pricing_provider, pricing_pair,
                pricing_timestamp, pricing_fetched_at, pricing_granularity,
                pricing_method, pricing_external_ref, pricing_quality, kind,
                privacy_boundary, description, counterparty, raw_json,
                payment_hash, payment_hash_source, swap_refund_funding_txid,
                swap_refund_funding_vout,
                created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params_from_iter(values),
        )
        .map_err(database_error)?;
        let id = Value::from(id);
        resolved.push(resolution(&id));
        imported += 1;
        inserted_records.push(change_record(
            &id,
            scope.wallet_label,
            record,
            vec!["transaction".into(), "pricing".into(), "metadata".into()],
        )?);
    }
    let invalidated = imported + updated + superseded > 0;
    if invalidated {
        crate::invalidate_journals(conn, scope.profile_id)?;
    }
    let mut outcome = dict([
        ("wallet", s(scope.wallet_label)),
        ("source", s(source_label)),
        ("imported", Value::Int(imported.into())),
        ("skipped", Value::Int(skipped.into())),
        ("unchanged", Value::Int(unchanged.into())),
        ("journal_invalidated", Value::Bool(invalidated)),
        ("inserted_records", Value::List(inserted_records)),
        ("updated_records", Value::List(updated_records)),
        ("_observer_resolved_records", Value::List(resolved)),
    ]);
    if rescoped > 0 {
        set(
            &mut outcome,
            "attestations_narrowed",
            Value::Int(rescoped.into()),
        );
    }
    if superseded > 0 {
        set(
            &mut outcome,
            "observer_superseded",
            Value::Int(superseded.into()),
        );
        set(
            &mut outcome,
            "observer_superseded_records",
            Value::List(superseded_records),
        );
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests;

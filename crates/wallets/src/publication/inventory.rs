//! Source-backed unspent-output inventory, from
//! `output_inventory.update_wallet_output_inventory(..., commit=False)`.
use super::row::{blank, field, Result};
use crate::database_error;
use kassiber_envelope::{dict, get, s, truthy, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql, Connection};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const SAFE_FIELDS: [&str; 31] = [
    "role",
    "group_id",
    "group_label",
    "parent_wallet_id",
    "source",
    "section",
    "script_type",
    "root_path",
    "gap_limit",
    "privacy_boundary",
    "whirlpool",
    "toxic_change",
    "minimum_mix_count",
    "mix_count",
    "mix_count_confidence",
    "target_mix_count",
    "pool_denomination_sat",
    "coordinator_fee_sat",
    "miner_fee_sat",
    "round_txid",
    "round_txids",
    "tx0_role",
    "whirlpool_event",
    "privacy_event",
    "exit_kind",
    "ricochet_hops",
    "watch_only",
    "bip47",
    "paynym",
    "scanned_without_explicit_descriptor",
    "sections",
];
const NON_NEGATIVE: [&str; 8] = [
    "gap_limit",
    "minimum_mix_count",
    "mix_count",
    "target_mix_count",
    "pool_denomination_sat",
    "coordinator_fee_sat",
    "miner_fee_sat",
    "ricochet_hops",
];

fn validation(message: &str) -> AppError {
    AppError::new("validation", message)
}

fn py_str(value: &Value) -> String {
    match value {
        Value::Str(text) => text
            .code_points()
            .map(|p| char::from_u32(p).unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
        other => kassiber_envelope::py_str(other),
    }
}

fn str_or_none(value: &Value) -> Option<String> {
    if matches!(value, Value::Null) {
        return None;
    }
    let text = py_str(value);
    let text = kassiber_pytext::strip(&text);
    (!text.is_empty()).then(|| text.to_owned())
}

fn hex_bytes(text: &str) -> bool {
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

fn txid(value: &Value) -> Result<String> {
    let text = str_or_none(value).ok_or_else(|| validation("UTXO record is missing txid"))?;
    let normalized = kassiber_pytext::lower(kassiber_pytext::strip(&text));
    if normalized.chars().count() != 64 || !hex_bytes(&normalized) {
        return Err(validation("UTXO record has an invalid txid"));
    }
    Ok(normalized)
}

fn txid_or_none(value: &Value) -> Option<String> {
    let text = if truthy(value) {
        py_str(value)
    } else {
        String::new()
    };
    let normalized = kassiber_pytext::lower(kassiber_pytext::strip(&text));
    (normalized.chars().count() == 64 && hex_bytes(&normalized)).then_some(normalized)
}

/// `int(value)` for UTXO numeric fields.
fn python_int(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Int(n) => i64::try_from(n.clone()).ok(),
        Value::Float(f) if f.is_finite() => Some(f.trunc() as i64),
        Value::Str(text) => text
            .as_str()
            .and_then(|text| kassiber_pytext::parse_int(text).ok())
            .and_then(|n| i64::try_from(n).ok()),
        _ => None,
    }
}

fn integer(value: &Value, name: &str, minimum: Option<i64>) -> Result<Option<i64>> {
    if blank(value) {
        return Ok(None);
    }
    let invalid = || validation(&format!("UTXO record has an invalid {name}"));
    let normalized = python_int(value).ok_or_else(invalid)?;
    if minimum.is_some_and(|minimum| normalized < minimum) {
        return Err(invalid());
    }
    Ok(Some(normalized))
}

fn boolean(value: &Value) -> Option<bool> {
    if blank(value) {
        return None;
    }
    if let Value::Bool(b) = value {
        return Some(*b);
    }
    match kassiber_pytext::lower(kassiber_pytext::strip(&py_str(value))).as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn script(value: &Value) -> Option<String> {
    let text = str_or_none(value)?;
    let normalized = kassiber_pytext::lower(&text);
    (normalized.chars().count().is_multiple_of(2) && hex_bytes(&normalized)).then_some(normalized)
}

fn either<'a>(output: &'a Value, first: &str, second: &str) -> &'a Value {
    get(output, first)
        .or_else(|| get(output, second))
        .unwrap_or(&Value::Null)
}

fn opt_text(value: Option<String>) -> Sql {
    value.map_or(Sql::Null, Sql::Text)
}

fn opt_int(value: Option<i64>) -> Sql {
    value.map_or(Sql::Null, Sql::Integer)
}

fn dumps(value: &Value) -> Result<String> {
    kassiber_pyjson::dumps(
        &kassiber_envelope::json_ready(value.clone()),
        &kassiber_pyjson::Options::new().sort_keys(true),
    )
    .map_err(|_| validation("UTXO record cannot be encoded"))
}

fn asset_code(value: &Value) -> String {
    let text = kassiber_pytext::strip(&if truthy(value) {
        py_str(value)
    } else {
        String::new()
    })
    .to_owned();
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

fn samourai_value(key: &str, value: &Value) -> Option<Value> {
    if NON_NEGATIVE.contains(&key) {
        return python_int(value)
            .filter(|n| *n >= 0)
            .map(|n| Value::Int(n.into()));
    }
    if matches!(
        key,
        "privacy_boundary" | "whirlpool" | "toxic_change" | "watch_only" | "paynym"
    ) {
        return Some(Value::Bool(truthy(value)));
    }
    if key == "round_txids" {
        let Value::List(items) = value else {
            return None;
        };
        let txids: Vec<Value> = items
            .iter()
            .filter_map(txid_or_none)
            .map(Value::from)
            .collect();
        return (!txids.is_empty()).then_some(Value::List(txids));
    }
    if key == "round_txid" {
        return txid_or_none(value).map(Value::from);
    }
    if key == "sections" {
        let Value::List(items) = value else {
            return None;
        };
        let sections: Vec<Value> = items
            .iter()
            .map(|item| kassiber_pytext::lower(kassiber_pytext::strip(&py_str(item))))
            .filter(|item| !item.is_empty())
            .map(Value::from)
            .collect();
        return (!sections.is_empty()).then_some(Value::List(sections));
    }
    let allowed: &[&str] = match key {
        "mix_count_confidence" => &["minimum", "exact", "estimated", "unknown"],
        "tx0_role" => &["deposit", "premix", "badbank", "fee"],
        "whirlpool_event" => &[
            "tx0",
            "premix_pending",
            "first_mix",
            "remix",
            "mix_to_wallet",
            "external_spend",
        ],
        "privacy_event" => &[
            "coinjoin",
            "payjoin",
            "tx0",
            "first_mix",
            "remix",
            "ricochet",
            "exit",
        ],
        "exit_kind" => &[
            "cold_storage",
            "external_spend",
            "ricochet",
            "toxic_change_spend",
        ],
        _ => &[],
    };
    if !allowed.is_empty() {
        let normalized = kassiber_pytext::lower(kassiber_pytext::strip(&if truthy(value) {
            py_str(value)
        } else {
            String::new()
        }));
        return allowed
            .contains(&normalized.as_str())
            .then_some(Value::from(normalized));
    }
    match value {
        Value::Str(_) => {
            let text = py_str(value);
            let text = kassiber_pytext::strip(&text);
            (!text.is_empty()).then(|| Value::from(text.chars().take(128).collect::<String>()))
        }
        Value::Int(_) | Value::Bool(_) => Some(value.clone()),
        _ => None,
    }
}

fn safe_samourai(metadata: &Value) -> Vec<(Key, Value)> {
    let Value::Dict(pairs) = metadata else {
        return vec![];
    };
    let mut safe: Vec<(Key, Value)> = Vec::new();
    for (index, (key, _)) in pairs.iter().enumerate() {
        // A duplicate JSON key is visited once, at its first position, with its last value.
        if pairs[..index].iter().any(|(earlier, _)| earlier == key) {
            continue;
        }
        let value = &pairs[pairs
            .iter()
            .rposition(|(other, _)| other == key)
            .unwrap_or(index)]
        .1;
        let Key::Str(name) = key else { continue };
        let name = py_str(&Value::Str(name.clone()));
        if !SAFE_FIELDS.contains(&name.as_str()) {
            continue;
        }
        if let Some(normalized) = samourai_value(&name, value) {
            safe.push((key.clone(), normalized));
        }
    }
    safe
}

fn samourai_from_config(config: &Value) -> Option<Vec<(Key, Value)>> {
    if !truthy(config) {
        return None;
    }
    let config = match config {
        Value::Str(_) => kassiber_pytext::loads(&py_str(config)).ok()?,
        other => other.clone(),
    };
    let metadata = get(&config, "samourai")?;
    if !matches!(metadata, Value::Dict(_)) {
        return None;
    }
    let safe = safe_samourai(metadata);
    (!safe.is_empty()).then_some(safe)
}

fn with_samourai(raw_json: &str, metadata: &[(Key, Value)]) -> Result<String> {
    let mut raw = match kassiber_pytext::loads(if raw_json.is_empty() { "{}" } else { raw_json }) {
        Ok(value @ Value::Dict(_)) => value,
        _ => Value::Dict(vec![]),
    };
    let mut merged = metadata.to_vec();
    for (key, value) in safe_samourai(get(&raw, "samourai").unwrap_or(&Value::Null)) {
        if let Some(slot) = merged.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            merged.push((key, value));
        }
    }
    kassiber_envelope::set(&mut raw, "samourai", Value::Dict(merged));
    dumps(&raw)
}

pub struct Source<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
    pub wallet_id: &'a str,
    pub wallet_config_json: &'a Value,
    pub backend_name: &'a str,
    pub backend_kind: &'a str,
    pub chain: &'a str,
    pub network: &'a str,
}

struct Row {
    values: Vec<Sql>,
    outpoint: String,
    spent: bool,
    binding_error: Option<Box<AppError>>,
}

fn normalize(output: &Value, source: &Source<'_>, seen_at: &str) -> Result<Row> {
    let txid = txid(field(output, "txid"))?;
    let vout = integer(field(output, "vout"), "vout", Some(0))?
        .ok_or_else(|| validation("UTXO record is missing vout"))?;
    let amount = integer(field(output, "amount_sats"), "amount_sats", Some(0))?
        .ok_or_else(|| validation("UTXO record is missing amount_sats"))?;
    let block_height = integer(field(output, "block_height"), "block_height", None)?;
    let confirmations = integer(field(output, "confirmations"), "confirmations", Some(0))?;
    let branch_index = integer(field(output, "branch_index"), "branch_index", None)?;
    let address_index = integer(field(output, "address_index"), "address_index", None)?;
    let anonymity = integer(
        either(output, "anonymity_score", "anonymityScore"),
        "anonymity_score",
        Some(0),
    )?;
    let excluded = boolean(either(
        output,
        "excluded_from_coinjoin",
        "excludedFromCoinjoin",
    ));
    let spent_by = str_or_none(either(output, "spent_by", "spentBy"));
    let spent_flag = boolean(field(output, "spent"));
    let mut spent_at = str_or_none(field(output, "spent_at"));
    if spent_at.is_none() && (spent_by.is_some() || spent_flag == Some(true)) {
        spent_at = Some(seen_at.into());
    }
    let history = match get(output, "anon_history").or_else(|| get(output, "anonHistory")) {
        Some(list @ Value::List(_)) => list.clone(),
        _ => Value::List(vec![]),
    };
    let status = str_or_none(field(output, "confirmation_status")).unwrap_or_else(|| {
        if block_height.is_some_and(|h| h > 0) {
            "confirmed"
        } else {
            "mempool"
        }
        .into()
    });
    let amount = amount.checked_mul(1000).ok_or_else(|| {
        AppError::new(
            "app_error",
            "Python int too large to convert to SQLite INTEGER",
        )
    })?;
    let id: String = Sha256::digest(
        format!("{}:{}:{txid}:{vout}", source.profile_id, source.wallet_id).as_bytes(),
    )
    .iter()
    .map(|b| format!("{b:02x}"))
    .collect();
    let chain = field(output, "chain");
    let network = field(output, "network");
    let outpoint = format!("{txid}:{vout}");
    let raw = dumps(
        get(output, "raw")
            .filter(|v| truthy(v))
            .unwrap_or(&Value::Dict(vec![])),
    )?;
    let key_state = str_or_none(either(output, "key_state", "keyState"));
    let spent = spent_at.is_some();
    Ok(Row {
        outpoint: outpoint.clone(),
        spent,
        binding_error: [
            field(output, "block_time"),
            field(output, "address"),
            field(output, "address_label"),
            field(output, "branch_label"),
            either(output, "spent_by", "spentBy"),
            either(output, "key_state", "keyState"),
            field(output, "spent_at"),
        ]
        .into_iter()
        .find_map(|value| match value {
            Value::Str(text) if text.has_surrogates() => Some(Box::new(
                crate::unicode_binding_error(&kassiber_pytext::strip_py(text)),
            )),
            _ => None,
        }),
        values: vec![
            Sql::Text(id),
            Sql::Text(source.workspace_id.into()),
            Sql::Text(source.profile_id.into()),
            Sql::Text(source.wallet_id.into()),
            Sql::Text(source.backend_name.into()),
            Sql::Text(source.backend_kind.into()),
            Sql::Text(if truthy(chain) {
                py_str(chain)
            } else {
                source.chain.into()
            }),
            Sql::Text(if truthy(network) {
                py_str(network)
            } else {
                source.network.into()
            }),
            Sql::Text(if truthy(field(output, "asset")) {
                asset_code(field(output, "asset"))
            } else {
                "BTC".into()
            }),
            Sql::Integer(amount),
            Sql::Text(txid),
            Sql::Integer(vout),
            Sql::Text(outpoint),
            Sql::Text(status),
            opt_int(confirmations),
            opt_int(block_height),
            opt_text(str_or_none(field(output, "block_time"))),
            opt_text(str_or_none(field(output, "address"))),
            opt_text(script(field(output, "script_pubkey"))),
            opt_text(str_or_none(field(output, "address_label"))),
            opt_text(str_or_none(field(output, "branch_label"))),
            opt_int(branch_index),
            opt_int(address_index),
            opt_int(anonymity),
            opt_text(spent_by),
            opt_int(excluded.map(i64::from)),
            opt_text(key_state),
            Sql::Text(dumps(&history)?),
            Sql::Text(seen_at.into()),
            Sql::Text(seen_at.into()),
            opt_text(spent_at),
            Sql::Text(raw),
        ],
    })
}

/// Persist one wallet's latest unspent-output view; never commits.
pub fn update(
    conn: &Connection,
    source: &Source<'_>,
    outputs: &[Value],
    seen_at: &str,
) -> Result<Value> {
    super::row::require_transaction(conn)?;
    let samourai = samourai_from_config(source.wallet_config_json);
    let mut rows = outputs
        .iter()
        .map(|output| normalize(output, source, seen_at))
        .collect::<Result<Vec<_>>>()?;
    let active: BTreeSet<String> = rows
        .iter()
        .filter(|r| !r.spent)
        .map(|r| r.outpoint.clone())
        .collect();
    if let Some(metadata) = &samourai {
        for row in &mut rows {
            let Sql::Text(raw) = &row.values[31] else {
                unreachable!()
            };
            row.values[31] = Sql::Text(with_samourai(raw, metadata)?);
        }
    }
    let mut statement = conn
        .prepare(
            "INSERT INTO wallet_utxos(
            id, workspace_id, profile_id, wallet_id, backend_name, backend_kind,
            chain, network, asset, amount, txid, vout, outpoint,
            confirmation_status, confirmations, block_height, block_time,
            address, script_pubkey, address_label, branch_label, branch_index, address_index,
            anonymity_score, spent_by, excluded_from_coinjoin, key_state,
            anon_history_json, first_seen_at, last_seen_at, spent_at, raw_json
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(wallet_id, txid, vout) DO UPDATE SET
            backend_name = excluded.backend_name,
            backend_kind = excluded.backend_kind,
            chain = excluded.chain,
            network = excluded.network,
            asset = excluded.asset,
            amount = excluded.amount,
            outpoint = excluded.outpoint,
            confirmation_status = excluded.confirmation_status,
            confirmations = excluded.confirmations,
            block_height = excluded.block_height,
            block_time = excluded.block_time,
            address = excluded.address,
            script_pubkey = excluded.script_pubkey,
            address_label = excluded.address_label,
            branch_label = excluded.branch_label,
            branch_index = excluded.branch_index,
            address_index = excluded.address_index,
            anonymity_score = excluded.anonymity_score,
            spent_by = excluded.spent_by,
            excluded_from_coinjoin = excluded.excluded_from_coinjoin,
            key_state = excluded.key_state,
            anon_history_json = excluded.anon_history_json,
            last_seen_at = excluded.last_seen_at,
            spent_at = excluded.spent_at,
            raw_json = excluded.raw_json",
        )
        .map_err(database_error)?;
    for row in &rows {
        if let Some(error) = &row.binding_error {
            return Err(error.as_ref().clone());
        }
        statement
            .execute(params_from_iter(row.values.iter()))
            .map_err(database_error)?;
    }
    let scope = [
        Sql::Text(seen_at.into()),
        Sql::Text(source.wallet_id.into()),
        Sql::Text(source.backend_name.into()),
        Sql::Text(source.backend_kind.into()),
        Sql::Text(source.chain.into()),
        Sql::Text(source.network.into()),
    ];
    let filter = if active.is_empty() {
        String::new()
    } else {
        format!(
            " AND outpoint NOT IN ({})",
            vec!["?"; active.len()].join(", ")
        )
    };
    let spent = conn
        .execute(
            &format!(
                "UPDATE wallet_utxos SET spent_at = ? WHERE wallet_id = ?
                 AND COALESCE(backend_name, '') = ? AND COALESCE(backend_kind, '') = ?
                 AND chain = ? AND network = ? AND spent_at IS NULL{filter}"
            ),
            params_from_iter(
                scope
                    .into_iter()
                    .chain(active.iter().cloned().map(Sql::Text)),
            ),
        )
        .map_err(database_error)?;
    conn.execute(
        "INSERT INTO wallet_utxo_refreshes(
            wallet_id, workspace_id, profile_id, backend_name, backend_kind,
            chain, network, observed_count, active_count, last_seen_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(wallet_id) DO UPDATE SET
            workspace_id = excluded.workspace_id,
            profile_id = excluded.profile_id,
            backend_name = excluded.backend_name,
            backend_kind = excluded.backend_kind,
            chain = excluded.chain,
            network = excluded.network,
            observed_count = excluded.observed_count,
            active_count = excluded.active_count,
            last_seen_at = excluded.last_seen_at",
        params_from_iter([
            Sql::Text(source.wallet_id.into()),
            Sql::Text(source.workspace_id.into()),
            Sql::Text(source.profile_id.into()),
            Sql::Text(source.backend_name.into()),
            Sql::Text(source.backend_kind.into()),
            Sql::Text(source.chain.into()),
            Sql::Text(source.network.into()),
            Sql::Integer(rows.len() as i64),
            Sql::Integer(active.len() as i64),
            Sql::Text(seen_at.into()),
        ]),
    )
    .map_err(database_error)?;
    Ok(dict([
        ("observed", Value::Int(rows.len().into())),
        ("active", Value::Int(active.len().into())),
        ("spent", Value::Int(spent.into())),
        ("last_seen_at", s(seen_at)),
    ]))
}

#[cfg(test)]
mod tests;

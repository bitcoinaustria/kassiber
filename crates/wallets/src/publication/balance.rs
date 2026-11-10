//! Negative-balance rescan hints, from `sync.negative_balance_events`. These
//! only decide whether Python would widen discovery; they grant no ordering,
//! dating or accounting authority.
use super::physical::{canonical_txid, stored_tx_mapping};
use super::row::{blank, field, or_text, Result};
use kassiber_envelope::{dict, get, s, truthy, AppError};
use kassiber_pyjson::Value;
use std::collections::BTreeMap;

const UNKNOWN_OCCURRED_AT: &str = "1970-01-01T00:00:00Z";

fn int(value: &Value) -> Result<i128> {
    if !truthy(value) {
        return Ok(0);
    }
    let invalid = || AppError::new("app_error", "Stored integer is invalid");
    match value {
        Value::Bool(b) => Ok(i128::from(*b)),
        Value::Int(n) => i128::try_from(n.clone()).map_err(|_| invalid()),
        Value::Float(f) if f.is_finite() => Ok(f.trunc() as i128),
        Value::Str(text) => text
            .as_str()
            .and_then(|t| kassiber_pytext::parse_int(t).ok())
            .and_then(|n| i128::try_from(n).ok())
            .ok_or_else(invalid),
        _ => Err(invalid()),
    }
}

fn unconfirmed_chain_position(row: &Value) -> bool {
    if truthy(field(row, "confirmed_at")) {
        return false;
    }
    let Some(raw) = stored_tx_mapping(field(row, "raw_json"), true) else {
        return false;
    };
    let txid = get(&raw, "txid")
        .filter(|v| truthy(v))
        .unwrap_or(field(row, "external_id"));
    if canonical_txid(txid).is_none() {
        return false;
    }
    matches!(get(&raw, "status"), Some(status @ Value::Dict(_)) if get(status, "confirmed") == Some(&Value::Bool(false)))
}

fn number(value: i128) -> Value {
    Value::Int(value.into())
}

/// `negative_balance_events(rows)`.
pub(crate) fn events(rows: &[Value]) -> Result<Vec<Value>> {
    let mut dated: BTreeMap<(String, String), Vec<(&Value, i128)>> = BTreeMap::new();
    let mut undated_inflows: BTreeMap<String, i128> = BTreeMap::new();
    let mut undated_deltas: BTreeMap<String, i128> = BTreeMap::new();
    for row in rows {
        let asset = or_text(field(row, "asset"));
        let (amount, fee) = (int(field(row, "amount"))?, int(field(row, "fee"))?);
        let direction = field(row, "direction");
        let delta = if direction == &Value::from("inbound") {
            amount
        } else if direction == &Value::from("outbound") {
            -amount - fee
        } else {
            0
        };
        let occurred = field(row, "occurred_at");
        if blank(occurred)
            || occurred == &Value::from(UNKNOWN_OCCURRED_AT)
            || unconfirmed_chain_position(row)
        {
            *undated_inflows.entry(asset.clone()).or_default() += delta.max(0);
            *undated_deltas.entry(asset).or_default() += delta;
        } else {
            dated
                .entry((kassiber_envelope::py_str(occurred), asset))
                .or_default()
                .push((row, delta));
        }
    }
    let mut balances = undated_inflows.clone();
    let mut first_negative: Vec<(String, Value)> = Vec::new();
    for ((occurred_at, asset), group) in &dated {
        let delta: i128 = group.iter().map(|(_, d)| d).sum();
        let running = balances.get(asset).copied().unwrap_or(0) + delta;
        balances.insert(asset.clone(), running);
        if running < 0 && !first_negative.iter().any(|(a, _)| a == asset) {
            let empty = Value::Dict(vec![]);
            let row = if group.len() == 1 { group[0].0 } else { &empty };
            first_negative.push((
                asset.clone(),
                dict([
                    ("asset", s(asset.clone())),
                    (
                        "transaction_id",
                        get(row, "id").cloned().unwrap_or(Value::Null),
                    ),
                    (
                        "external_id",
                        get(row, "external_id").cloned().unwrap_or(Value::Null),
                    ),
                    ("occurred_at", s(occurred_at.clone())),
                    ("delta_msat", number(delta)),
                    ("running_balance_msat", number(running)),
                ]),
            ));
        }
    }
    for (asset, delta) in &undated_deltas {
        let running = balances.get(asset).copied().unwrap_or(0)
            - undated_inflows.get(asset).copied().unwrap_or(0)
            + delta;
        if running < 0 && !first_negative.iter().any(|(a, _)| a == asset) {
            first_negative.push((
                asset.clone(),
                dict([
                    ("asset", s(asset.clone())),
                    ("transaction_id", Value::Null),
                    ("external_id", Value::Null),
                    ("occurred_at", Value::Null),
                    ("delta_msat", number(*delta)),
                    ("running_balance_msat", number(running)),
                ]),
            ));
        }
    }
    Ok(first_negative.into_iter().map(|(_, v)| v).collect())
}

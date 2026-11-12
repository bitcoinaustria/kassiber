//! Durable policy-epoch coverage, from `ownership_policy_epochs.py`'s
//! `ensure_active_wallet_epoch` and `record_observer_policy_coverage`.
//! Coverage is a technical scan fact, never an ownership-completeness claim.
use super::row::{blank, field, first, or_text, Result};
use crate::database_error;
use kassiber_envelope::{get, py_str, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql, Connection};

const PRIVATE_FIELDS: [&str; 12] = [
    "addresses",
    "blinding_key",
    "chain",
    "change_descriptor",
    "descriptor",
    "gap_limit",
    "network",
    "ownership_scan_to_index",
    "samourai",
    "script_types",
    "synthesize_change",
    "xpub",
];

/// The structural observer identity a coverage witness belongs to.
pub struct Identity<'a> {
    pub source_wallet_id: &'a str,
    pub source_key: &'a str,
    pub observer_kind: &'a str,
    pub chain: &'a str,
    pub network: &'a str,
    pub branch_keys: &'a [String],
}

pub struct Point {
    pub branch_key: String,
    pub scanned_to: i64,
    pub highest_used: Option<i64>,
}

fn wallet_config(raw: &Value) -> Value {
    let text = or_text(raw);
    match kassiber_pytext::loads(if text.is_empty() { "{}" } else { &text }) {
        Ok(value @ Value::Dict(_)) => value,
        _ => Value::Dict(vec![]),
    }
}

/// `private_policy_material(config)`.
fn private_material(config: &Value) -> Value {
    let mut material = Vec::new();
    for key in PRIVATE_FIELDS {
        let Some(value) = get(config, key) else {
            continue;
        };
        if blank(value) || matches!(value, Value::List(items) if items.is_empty()) {
            continue;
        }
        material.push((Key::from(key), value.clone()));
    }
    Value::Dict(material)
}

fn chain_value(value: &Value) -> Result<String> {
    let text = if kassiber_envelope::truthy(value) {
        py_str(value)
    } else {
        "bitcoin".into()
    };
    let lowered = kassiber_pytext::lower(kassiber_pytext::strip(&text));
    Ok(match lowered.as_str() {
        "" | "bitcoin" | "btc" | "onchain" => "bitcoin",
        "liquid" | "lbtc" | "elements" => "liquid",
        _ => {
            return Err(AppError::new(
                "app_error",
                format!("Unsupported wallet chain '{}'", py_str(value)),
            ))
        }
    }
    .into())
}

fn network_value(chain: &str, value: &Value) -> Result<String> {
    let default = if chain == "bitcoin" {
        "main"
    } else {
        "liquidv1"
    };
    let text = if kassiber_envelope::truthy(value) {
        py_str(value)
    } else {
        default.into()
    };
    let lowered = kassiber_pytext::lower(kassiber_pytext::strip(&text));
    if lowered.is_empty() {
        return Ok(default.into());
    }
    super::physical::canonical_network(chain, &Value::from(lowered.as_str()))
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError::new(
                "app_error",
                format!("Unsupported {chain} network '{}'", py_str(value)),
            )
        })
}

fn dumps(value: &Value) -> Result<String> {
    kassiber_pyjson::dumps(value, &kassiber_pyjson::Options::new().sort_keys(true))
        .map_err(|_| AppError::new("app_error", "Policy material cannot be encoded"))
}

/// `ensure_active_wallet_epoch(conn, wallet, chain=..., network=...)`.
pub fn ensure_active_epoch(
    conn: &Connection,
    wallet_id: &str,
    chain: &str,
    network: &str,
    new_id: &mut dyn FnMut() -> String,
    now: &mut dyn FnMut() -> String,
) -> Result<String> {
    super::row::require_transaction(conn)?;
    let wallet =
        first(conn, "SELECT * FROM wallets WHERE id = ?", [wallet_id])?.ok_or_else(|| {
            AppError::new(
                "internal_error",
                "observer policy source wallet does not exist",
            )
        })?;
    if let Some(row) = first(
        conn,
        "SELECT id FROM wallet_policy_epochs WHERE wallet_id = ? AND status = 'active'",
        [wallet_id],
    )? {
        return Ok(py_str(field(&row, "id")));
    }
    let config = wallet_config(field(&wallet, "config_json"));
    let material = private_material(&config);
    let chain_input = if chain.is_empty() {
        get(&config, "chain").cloned().unwrap_or(Value::Null)
    } else {
        Value::from(chain)
    };
    let epoch_chain = chain_value(&chain_input)?;
    let network_input = if network.is_empty() {
        get(&config, "network").cloned().unwrap_or(Value::Null)
    } else {
        Value::from(network)
    };
    let epoch_network = network_value(&epoch_chain, &network_input)?;
    let epoch_id = new_id();
    conn.execute(
        "INSERT INTO wallet_policy_epochs(
            id, workspace_id, profile_id, wallet_id, chain, network, status,
            private_material_json, created_at, retired_at
        ) VALUES(?, ?, ?, ?, ?, ?, 'active', ?, ?, NULL)",
        params_from_iter([
            Sql::Text(epoch_id.clone()),
            Sql::Text(or_text(field(&wallet, "workspace_id"))),
            Sql::Text(or_text(field(&wallet, "profile_id"))),
            Sql::Text(wallet_id.into()),
            Sql::Text(epoch_chain),
            Sql::Text(epoch_network),
            Sql::Text(dumps(&material)?),
            Sql::Text(now()),
        ]),
    )
    .map_err(database_error)?;
    Ok(epoch_id)
}

/// `record_observer_policy_coverage(conn, identity, coverage)`; never commits.
pub fn record_coverage(
    conn: &Connection,
    identity: &Identity<'_>,
    coverage: &[Point],
    new_id: &mut dyn FnMut() -> String,
    now: &mut dyn FnMut() -> String,
) -> Result<String> {
    let epoch_id = ensure_active_epoch(
        conn,
        identity.source_wallet_id,
        identity.chain,
        identity.network,
        new_id,
        now,
    )?;
    let source = first(
        conn,
        "SELECT id FROM wallet_policy_sources WHERE epoch_id = ? AND source_wallet_id = ? AND source_key = ?",
        [epoch_id.as_str(), identity.source_wallet_id, identity.source_key],
    )?;
    let observed_at = now();
    let mut branch_keys: Vec<Value> = Vec::new();
    for key in identity.branch_keys {
        let key = Value::from(key.as_str());
        if !branch_keys.contains(&key) {
            branch_keys.push(key);
        }
    }
    let branch_keys =
        kassiber_pyjson::dumps(&Value::List(branch_keys), &kassiber_pyjson::Options::new())
            .map_err(|_| AppError::new("app_error", "Branch keys cannot be encoded"))?;
    let source_id = match source {
        None => {
            let source_id = new_id();
            conn.execute(
                "INSERT INTO wallet_policy_sources(
                    id, epoch_id, source_wallet_id, source_key, observer_kind,
                    branch_keys_json, created_at, updated_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)",
                params_from_iter([
                    source_id.as_str(),
                    epoch_id.as_str(),
                    identity.source_wallet_id,
                    identity.source_key,
                    identity.observer_kind,
                    branch_keys.as_str(),
                    observed_at.as_str(),
                    observed_at.as_str(),
                ]),
            )
            .map_err(database_error)?;
            source_id
        }
        Some(row) => {
            let source_id = py_str(field(&row, "id"));
            conn.execute(
                "UPDATE wallet_policy_sources SET observer_kind = ?, branch_keys_json = ?, updated_at = ? WHERE id = ?",
                params_from_iter([
                    identity.observer_kind,
                    branch_keys.as_str(),
                    observed_at.as_str(),
                    source_id.as_str(),
                ]),
            )
            .map_err(database_error)?;
            source_id
        }
    };
    for point in coverage {
        conn.execute(
            "INSERT INTO wallet_policy_coverage_witnesses(
                source_id, branch_key, scanned_to_exclusive, highest_used,
                observer_kind, observed_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            ON CONFLICT(source_id, branch_key) DO UPDATE SET
                scanned_to_exclusive = excluded.scanned_to_exclusive,
                highest_used = excluded.highest_used,
                observer_kind = excluded.observer_kind,
                observed_at = excluded.observed_at",
            params_from_iter([
                Sql::Text(source_id.clone()),
                Sql::Text(point.branch_key.clone()),
                Sql::Integer(point.scanned_to),
                point.highest_used.map_or(Sql::Null, Sql::Integer),
                Sql::Text(identity.observer_kind.into()),
                Sql::Text(observed_at.clone()),
            ]),
        )
        .map_err(database_error)?;
    }
    Ok(epoch_id)
}

#[cfg(test)]
pub(crate) mod tests;

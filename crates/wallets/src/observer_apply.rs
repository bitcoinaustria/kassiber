//! Private observer-state persistence for the wallet publication transaction.
//! No network, product routing, custody authority or implicit commit lives here.
use crate::{database_error, observer_state::Identity};
use kassiber_envelope::{dict, s, AppError};
use kassiber_store::{observer, rusqlite::Connection};
use serde_json::Value;
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, AppError>;

pub struct Coverage {
    pub branch_key: String,
    pub highest_used: Option<i64>,
    pub scanned_to: i64,
    pub details: Value,
}

/// A fully prepared replacement. Scope and branch keys come from the stored
/// wallet identity, never from remote observer facts.
pub struct Prepared<'a> {
    pub identity: Identity<'a>,
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
    pub branch_keys: &'a [&'a str],
    pub state: Value,
    pub coverage: Vec<Coverage>,
    pub expected_epoch: Option<i64>,
}

fn invalid(message: &str, branch: Option<&str>) -> AppError {
    let error = AppError::new("observer_state_invalid", message);
    if let Some(branch) = branch {
        error.with_details(dict([("branch_key", s(branch))]))
    } else {
        error
    }
}

fn encode(value: &Value) -> Result<String> {
    validate_json(value, "$")?;
    if !value.is_object() {
        return Err(invalid("Observer state payload must be an object", None));
    }
    let value = kassiber_pytext::loads(&value.to_string())
        .map_err(|_| invalid("Observer state payload must be an object", None))?;
    kassiber_pyjson::dumps(
        &value,
        &kassiber_pyjson::Options::default()
            .sort_keys(true)
            .compact()
            .ensure_ascii(false),
    )
    .map_err(|_| invalid("Observer state payload must be an object", None))
}

fn validate_json(value: &Value, path: &str) -> Result<()> {
    match value {
        Value::Number(number) => {
            let text = number.to_string();
            if text.contains(['.', 'e', 'E'])
                && text.parse::<f64>().is_ok_and(|value| !value.is_finite())
            {
                return Err(AppError::new(
                    "observer_state_invalid",
                    "Observer state contains a non-finite number",
                )
                .with_details(dict([("path", s(path))])));
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                validate_json(value, &format!("{path}[{index}]"))?;
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                validate_json(value, &format!("{path}.{key}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Python's `persist_observer_state`, with mandatory captured-epoch fencing.
/// The coordinator must roll back its wallet savepoint on any error. There is
/// no unconditional overwrite mode on the native sync path.
pub fn persist(conn: &Connection, prepared: &Prepared<'_>, timestamp: &str) -> Result<()> {
    if conn.is_autocommit() {
        return Err(AppError::new(
            "observer_apply_outside_transaction",
            "Observer state application requires the caller's wallet transaction",
        ));
    }
    let state_json = encode(&prepared.state)?;
    let mut points: Vec<_> = prepared.coverage.iter().collect();
    points.sort_by(|a, b| a.branch_key.cmp(&b.branch_key));
    let mut seen = BTreeSet::new();
    let mut coverage = Vec::with_capacity(points.len());
    for point in points {
        let branch = point.branch_key.as_str();
        if !seen.insert(branch) {
            return Err(invalid(
                "Observer coverage contains a duplicate branch",
                Some(branch),
            ));
        }
        if !prepared.branch_keys.contains(&branch) {
            return Err(invalid(
                "Observer coverage names an unknown branch",
                Some(branch),
            ));
        }
        if point.scanned_to < 0 || point.highest_used.is_some_and(|index| index < 0) {
            return Err(invalid(
                "Observer coverage indices must be non-negative",
                Some(branch),
            ));
        }
        if point
            .highest_used
            .is_some_and(|index| index >= point.scanned_to)
        {
            return Err(AppError::new(
                "observer_state_invalid",
                "Observer coverage cannot contain a used index outside its exclusive scan boundary",
            )
            .with_details(dict([
                ("branch_key", s(branch)),
                (
                    "highest_used",
                    kassiber_pyjson::Value::Int(point.highest_used.unwrap().into()),
                ),
                (
                    "scanned_to",
                    kassiber_pyjson::Value::Int(point.scanned_to.into()),
                ),
            ])));
        }
        coverage.push(observer::Coverage {
            branch_key: point.branch_key.clone(),
            coverage_version: 2,
            highest_used: point.highest_used,
            scanned_to: point.scanned_to,
            details_json: encode(&if point.details.is_null() {
                Value::Object(Default::default())
            } else {
                point.details.clone()
            })?,
        });
    }
    let identity = &prepared.identity;
    let instance = observer::Instance {
        observer_kind: identity.observer_kind.into(),
        chain: identity.chain.into(),
        network: identity.network.into(),
        logical_wallet_id: identity.logical_wallet_id.into(),
        source_wallet_id: identity.source_wallet_id.into(),
        source_key: identity.source_key.into(),
        state_version: 1,
        state_epoch: 0, // SQL creates/increments the epoch; never trusted input.
        state_json,
    };
    if !observer::replace(
        conn,
        &observer::Replacement {
            id: identity.id,
            workspace_id: prepared.workspace_id,
            profile_id: prepared.profile_id,
            instance: &instance,
            coverage: &coverage,
            expected_epoch: prepared.expected_epoch,
            timestamp,
        },
    )
    .map_err(database_error)?
    {
        return Err(AppError::new(
            "observer_state_stale",
            "Chain observer state changed after this refresh was prepared",
        )
        .with_hint("Retry the wallet refresh against the latest observer state.")
        .with_details(dict([
            ("observer_id", s(identity.id)),
            (
                "expected_epoch",
                prepared
                    .expected_epoch
                    .map_or(kassiber_pyjson::Value::Null, |value| {
                        kassiber_pyjson::Value::Int(value.into())
                    }),
            ),
        ]))
        .retryable(true));
    }
    Ok(())
}

/// `persist_observer_values` over the fetch's hex-encoded opaque map:
/// replace every value of one observer inside the caller's savepoint.
pub fn persist_values(
    conn: &Connection,
    observer_id: &str,
    values: &Value,
    timestamp: &str,
) -> Result<()> {
    if conn.is_autocommit() {
        return Err(AppError::new(
            "observer_apply_outside_transaction",
            "Observer values require the caller's wallet transaction",
        ));
    }
    let Some(values) = values.as_object() else {
        return Err(invalid("Observer values must remain opaque bytes", None));
    };
    let mut decoded: Vec<(&str, Vec<u8>)> = Vec::with_capacity(values.len());
    for (key, value) in values {
        if key.is_empty() {
            return Err(invalid(
                "Observer value keys must be non-empty strings",
                None,
            ));
        }
        let bytes = value.as_str().and_then(|text| {
            (text.len() % 2 == 0)
                .then(|| {
                    (0..text.len())
                        .step_by(2)
                        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
                        .collect::<Option<Vec<u8>>>()
                })
                .flatten()
        });
        let Some(bytes) = bytes else {
            return Err(AppError::new(
                "observer_state_invalid",
                "Observer values must remain opaque bytes",
            )
            .with_details(dict([("key", s(key.as_str()))])));
        };
        decoded.push((key, bytes));
    }
    decoded.sort();
    let rows: Vec<(&str, &[u8])> = decoded.iter().map(|(k, v)| (*k, v.as_slice())).collect();
    observer::replace_values(
        conn,
        observer_id,
        crate::observer_state::VALUES_NAMESPACE_VERSION,
        &rows,
        timestamp,
    )
    .map_err(database_error)
}

#[cfg(test)]
mod tests;

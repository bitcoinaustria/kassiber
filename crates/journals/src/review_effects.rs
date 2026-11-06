//! Economic commitments over the canonical review build, independent of row IDs.
use crate::review_workflow::Error;
use crate::{JournalEntryRow, JournalState};
use kassiber_custody::codec;
use kassiber_envelope::{dict, get, s, set, AppError};
use kassiber_pydecimal::{Context, Decimal};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use kassiber_transfers::review_support::sql;
use kassiber_transfers::{review_operations, Scope, Sources};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, AppError>;

fn json(value: &serde_json::Value) -> Result<Value> {
    codec::from_json(value).map_err(|message| AppError::new("internal_error", message))
}

fn scalar(value: &serde_json::Value) -> Result<Value> {
    Ok(if value.is_null() {
        Value::Null
    } else {
        s(kassiber_envelope::py_str(&json(value)?))
    })
}

fn entry(row: &JournalEntryRow) -> Result<Value> {
    Ok(dict([
        ("transaction_id", scalar(&row.transaction_id)?),
        ("wallet_id", scalar(row.wallet_id)?),
        ("account_id", scalar(row.account_id)?),
        ("occurred_at", scalar(&row.occurred_at)?),
        ("entry_type", s(row.entry_type)),
        ("asset", s(row.asset.as_ref())),
        ("quantity", s(row.quantity.to_string())),
        ("fiat_value", s(row.fiat_value.0.to_string())),
        ("unit_cost", s(row.unit_cost.0.to_string())),
        (
            "cost_basis",
            row.cost_basis
                .as_ref()
                .map(|v| s(v.0.to_string()))
                .unwrap_or(Value::Null),
        ),
        (
            "proceeds",
            row.proceeds
                .as_ref()
                .map(|v| s(v.0.to_string()))
                .unwrap_or(Value::Null),
        ),
        (
            "gain_loss",
            row.gain_loss
                .as_ref()
                .map(|v| s(v.0.to_string()))
                .unwrap_or(Value::Null),
        ),
        (
            "at_category",
            row.at_category.as_deref().map(s).unwrap_or(Value::Null),
        ),
        (
            "at_kennzahl",
            row.at_kennzahl
                .flatten()
                .map(|v| s(v.to_string()))
                .unwrap_or(Value::Null),
        ),
        (
            "capital_gains_type",
            row.capital_gains_type.map(s).unwrap_or(Value::Null),
        ),
    ]))
}

fn sorted(values: impl IntoIterator<Item = Value>) -> Result<Vec<Value>> {
    let mut keyed = values
        .into_iter()
        .map(|value| Ok((review_operations::canonical(&value)?, value)))
        .collect::<Result<Vec<_>>>()?;
    keyed.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(keyed.into_iter().map(|(_, value)| value).collect())
}

/// Commit all economic rows before bounding the presentation to 100 rows.
pub fn economic(state: &JournalState<'_>) -> Result<Value> {
    let entries = sorted(
        state
            .entries
            .iter()
            .map(entry)
            .collect::<Result<Vec<_>>>()?,
    )?;
    let quarantines = state
        .quarantines
        .iter()
        .map(|row| {
            Ok((
                kassiber_envelope::py_str(&json(&row.transaction_id)?),
                row.reason.clone(),
            ))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let holdings = sorted(
        state
            .wallet_holdings
            .pairs
            .iter()
            .map(|(key, holding)| {
                Ok(dict([
                    ("wallet_id", json(&key[0])?),
                    ("asset", json(&key[3])?),
                    ("quantity", s(holding.quantity.0.to_string())),
                    ("cost_basis", s(holding.cost_basis.0.to_string())),
                ]))
            })
            .collect::<Result<Vec<_>>>()?,
    )?;
    let blocked = codec::truth(state.custody_component_blockers)
        || codec::truth(&state.custody_quantity["issues"]);
    // serde_json serializes nonfinite f64 values as null. Python's economic
    // commitment refuses them instead; check before crossing that boundary.
    for row in &state.tax_summary {
        for number in [row.quantity, row.proceeds, row.cost_basis, row.gain_loss] {
            if !number.is_finite() {
                return Err(AppError::new(
                    "validation",
                    "Review data must contain finite JSON values",
                ));
            }
        }
    }
    let summary = serde_json::to_value(&state.tax_summary)
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let payload = dict([
        ("entries", Value::List(entries.clone())),
        (
            "quarantines",
            Value::List(
                quarantines
                    .iter()
                    .map(|(tx, reason)| Value::List(vec![s(tx.as_str()), s(reason.as_str())]))
                    .collect(),
            ),
        ),
        ("holdings", Value::List(holdings.clone())),
        ("tax_summary", json(&summary)?),
        ("blocked", Value::Bool(blocked)),
    ]);
    Ok(dict([
        ("entries_count", entries.len().into()),
        ("quarantine_count", quarantines.len().into()),
        (
            "quarantines",
            Value::List(
                quarantines
                    .iter()
                    .take(100)
                    .map(|(tx, reason)| {
                        dict([
                            ("transaction_id", s(tx.as_str())),
                            ("reason", s(reason.as_str())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "wallet_holdings",
            Value::List(holdings.into_iter().take(100).collect()),
        ),
        (
            "report_ready",
            Value::Bool(quarantines.is_empty() && !blocked),
        ),
        ("journal_digest", s(review_operations::digest(&payload)?)),
    ]))
}

fn decimal_text(value: &Decimal) -> Result<Value> {
    let text = value
        .to_fixed_string()
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    Ok(s(if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }))
}

fn acquisition(state: &JournalState<'_>, conn: &Connection, scope: &Scope<'_>) -> Result<Value> {
    // Policy and identity come from this connection, including pending caller edits.
    let current = sql::rows(
        conn,
        "SELECT * FROM profiles WHERE id=?",
        [scope.profile_id],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("not_found", "Profile was not found"))?;
    let Value::Dict(fields) = current else {
        unreachable!("profile row")
    };
    let policy = Value::Dict(fields.into_iter().filter(|(key, _)| {
        matches!(key, kassiber_pyjson::Key::Str(key) if key.as_str().is_some_and(|key| {
            key.starts_with("tax_") || ["gains_algorithm", "fiat_currency", "cost_basis_pool_scope",
                "require_coarse_review", "bitcoin_rail_carrying_value"].contains(&key)
        }))
    }).collect());
    let identity = sql::rows(
        conn,
        "SELECT value FROM settings WHERE key='database_instance_id'",
        [],
    )?
    .into_iter()
    .next()
    .map(|row| get(&row, "value").cloned().unwrap_or(Value::Null))
    .unwrap_or(Value::Null);
    if !matches!(&identity, Value::Str(id) if id.as_str().is_some_and(|id| kassiber_store::valid_hex(id, 32)))
    {
        return Err(AppError::new(
            "invalid_project_database",
            "database instance identity is missing or invalid",
        ));
    }
    let mut totals = BTreeMap::<String, [Decimal; 5]>::new();
    for entry in &state.entries {
        let row = totals
            .entry(entry.asset.to_string())
            .or_insert_with(|| std::array::from_fn(|_| Decimal::zero()));
        let mut add = |index: usize, number: &Decimal| -> Result<()> {
            row[index] = row[index]
                .add(number, &Context::RP2)
                .map_err(|error| AppError::new("internal_error", error.to_string()))?;
            Ok(())
        };
        match entry.entry_type {
            "acquisition" => add(0, &entry.fiat_value.0)?,
            "income" => add(1, &entry.fiat_value.0)?,
            "disposal" | "fee" | "transfer_fee" => {
                for (index, number) in [
                    (2, &entry.cost_basis),
                    (3, &entry.proceeds),
                    (4, &entry.gain_loss),
                ] {
                    add(
                        index,
                        number
                            .as_ref()
                            .map(|number| &number.0)
                            .unwrap_or(&Decimal::zero()),
                    )?;
                }
            }
            _ => (),
        }
    }
    let totals = totals
        .into_iter()
        .map(|(asset, totals)| {
            Ok(dict([
                ("asset", s(asset)),
                ("acquisition_basis", decimal_text(&totals[0])?),
                ("income", decimal_text(&totals[1])?),
                ("disposal_basis", decimal_text(&totals[2])?),
                ("proceeds", decimal_text(&totals[3])?),
                ("gain_loss", decimal_text(&totals[4])?),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(dict([
        (
            "acquisition_context",
            dict([
                ("database_instance_id", identity),
                ("profile_policy", policy),
            ]),
        ),
        ("accounting_totals", Value::List(totals)),
    ]))
}

/// Include live policy-bound acquisition totals only when the batch classifies
/// an inbound. Everything a plan commits to except the per-year tax figures,
/// which need the state stored (see [`effects`]).
pub fn effects_without_tax(
    state: &JournalState<'_>,
    conn: &Connection,
    scope: &Scope<'_>,
    operations: &Value,
) -> Result<Value> {
    let mut effects = if matches!(operations, Value::List(ops) if ops.iter().any(|op| get(op, "type") == Some(&s("kind_override"))))
    {
        acquisition(state, conn, scope)?
    } else {
        dict([] as [(&str, Value); 0])
    };
    let Value::Dict(fields) = economic(state)? else {
        unreachable!("economic effects")
    };
    for (key, value) in fields {
        let kassiber_pyjson::Key::Str(key) = key else {
            unreachable!("effect key")
        };
        set(&mut effects, key.as_str().expect("ASCII effect key"), value);
    }
    Ok(effects)
}

/// `_effects`: [`effects_without_tax`] and the state's per-year tax figures.
pub fn effects(
    state: &JournalState<'_>,
    conn: &Connection,
    scope: &Scope<'_>,
    operations: &Value,
    sources: &dyn Sources,
) -> std::result::Result<Value, Error> {
    let mut effects = effects_without_tax(state, conn, scope, operations)?;
    let tax = crate::review_tax::tax_effects(conn, scope, state, sources)?;
    set(&mut effects, "tax", tax);
    Ok(effects)
}

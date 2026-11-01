//! Closed chain-observation provenance and its journal-staleness comparison,
//! from `chain_observer/provenance.py` and `sync._observed_accounting_inputs`.
use super::physical::{canonical_txid, confirmation_states, Physical};
use super::row::{field, first, or_text, py_eq, rows, Result};
use crate::database_error;
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql, Connection};
use std::collections::{BTreeMap, BTreeSet};

pub const AUTHORITY_VERSION: i64 = 1;

/// `normalize_asset_code`.
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

type RecordKey = (String, String, String);

/// `provenance._record_key`.
fn record_key(record: &Value) -> RecordKey {
    let txid = field(record, "txid");
    let txid = if truthy(txid) {
        txid
    } else {
        field(record, "external_id")
    };
    (
        kassiber_pytext::lower(kassiber_pytext::strip(&or_text(txid))),
        asset_code(field(record, "asset")),
        kassiber_pytext::lower(kassiber_pytext::strip(&or_text(field(record, "direction")))),
    )
}

/// One structural observer and the records it contributed.
pub struct ObserverRecords<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub records: &'a [Value],
}

/// `provenance_entries_for_facts(records_by_observer, final_records)`.
pub fn entries_for_facts(
    observers: &[ObserverRecords<'_>],
    final_records: Option<&[Value]>,
) -> Vec<Value> {
    type Sources = (BTreeSet<String>, BTreeSet<String>);
    let mut sources: BTreeMap<RecordKey, Sources> = BTreeMap::new();
    for observer in observers {
        for record in observer.records {
            let key = record_key(record);
            if key.0.is_empty() {
                continue;
            }
            let item = sources.entry(key).or_default();
            item.0.insert(observer.id.into());
            item.1.insert(observer.kind.into());
        }
    }
    if let Some(final_records) = final_records {
        let mut by_txid: BTreeMap<String, Sources> = BTreeMap::new();
        for (key, (ids, kinds)) in &sources {
            let aggregate = by_txid.entry(key.0.clone()).or_default();
            aggregate.0.extend(ids.iter().cloned());
            aggregate.1.extend(kinds.iter().cloned());
        }
        let mut projected = BTreeMap::new();
        for record in final_records {
            let key = record_key(record);
            let source = sources.get(&key).or_else(|| by_txid.get(&key.0));
            if let (false, Some(source)) = (key.0.is_empty(), source) {
                projected.insert(key, source.clone());
            }
        }
        sources = projected;
    }
    sources
        .into_iter()
        .map(|((txid, asset, direction), (ids, kinds))| {
            dict([
                ("external_id", s(txid)),
                ("asset", s(asset)),
                ("direction", s(direction)),
                (
                    "observer_ids",
                    Value::List(ids.into_iter().map(Value::from).collect()),
                ),
                (
                    "observer_kinds",
                    Value::List(kinds.into_iter().map(Value::from).collect()),
                ),
            ])
        })
        .collect()
}

/// `fee_attribution_from_raw`.
fn fee_attribution(raw: &Value) -> &'static str {
    let payload = match raw {
        Value::Str(_) | Value::Null => {
            let text = or_text(raw);
            match kassiber_pytext::loads(if text.is_empty() { "{}" } else { &text }) {
                Ok(value) => value,
                Err(_) => return "unknown",
            }
        }
        other => other.clone(),
    };
    if !matches!(payload, Value::Dict(_)) {
        return "unknown";
    }
    if let Some(component @ Value::Dict(_)) = get(&payload, "component") {
        match get(component, "fee_attribution") {
            Some(Value::Str(text)) if text.as_str() == Some("exact") => return "exact",
            Some(Value::Str(text)) if text.as_str() == Some("implicit_wallet_delta") => {
                return "implicit_wallet_delta"
            }
            _ => {}
        }
    }
    let observer =
        kassiber_pytext::lower(&or_text(get(&payload, "observer").unwrap_or(&Value::Null)));
    if matches!(observer.as_str(), "bdk" | "bitcoinrpc") {
        "exact"
    } else {
        "unknown"
    }
}

fn graph_hash(raw: &Value) -> Result<String> {
    match raw {
        Value::Str(_) => crate::observer_state::graph_hash(&py_str(raw)),
        other => crate::observer_state::graph_hash_value(other),
    }
}

fn mismatch(reason: &str, details: Vec<(&str, Value)>) -> AppError {
    let mut all = vec![
        ("conflict_kind", s("provenance_resolution_mismatch")),
        ("resolution_reason", s(reason)),
    ];
    all.extend(details);
    AppError::new(
        "observer_projection_conflict",
        "Authoritative observation transaction resolution failed",
    )
    .with_hint("Retry refresh. If the conflict persists, review duplicate transactions.")
    .with_details(dict(all))
}

fn strings_json(values: &Value) -> Result<String> {
    let Value::List(items) = values else {
        return Err(AppError::new(
            "observer_state_invalid",
            "Observer identities must be a list",
        ));
    };
    let set: BTreeSet<String> = items.iter().map(py_str).collect();
    kassiber_pyjson::dumps(
        &Value::List(set.into_iter().map(Value::from).collect()),
        &kassiber_pyjson::Options::new(),
    )
    .map_err(|_| AppError::new("app_error", "Observer identities cannot be encoded"))
}

pub struct Target<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
    pub wallet_id: &'a str,
    pub application_revision: &'a str,
    pub chain: &'a str,
    pub network: &'a str,
}

/// `persist_chain_observation_provenance`; never commits.
pub fn persist(
    conn: &Connection,
    target: &Target<'_>,
    entries: &[Value],
    resolved: &[Value],
    now: &mut dyn FnMut() -> String,
) -> Result<usize> {
    super::row::require_transaction(conn)?;
    let mut resolved_by_key: Vec<(RecordKey, String)> = Vec::new();
    for record in resolved {
        let key = record_key(record);
        let id = or_text(field(record, "transaction_id"));
        if key.0.is_empty() || id.is_empty() {
            return Err(mismatch("invalid_resolved_record", vec![]));
        }
        match resolved_by_key.iter().find(|(k, _)| *k == key) {
            Some((_, previous)) if *previous != id => {
                return Err(mismatch("conflicting_resolved_ids", vec![]))
            }
            Some(_) => {}
            None => resolved_by_key.push((key, id)),
        }
    }
    let keys: Vec<RecordKey> = entries.iter().map(record_key).collect();
    let unique: BTreeSet<&RecordKey> = keys.iter().collect();
    if unique.len() != keys.len() {
        return Err(mismatch("duplicate_provenance_key", vec![]));
    }
    let resolved_keys: BTreeSet<&RecordKey> = resolved_by_key.iter().map(|(k, _)| k).collect();
    if unique != resolved_keys {
        return Err(mismatch(
            "provenance_key_mismatch",
            vec![
                ("entry_count", Value::Int(keys.len().into())),
                ("resolved_count", Value::Int(resolved_by_key.len().into())),
            ],
        ));
    }
    let timestamp = now();
    let native = matches!(
        kassiber_pytext::lower(kassiber_pytext::strip(target.chain)).as_str(),
        "bitcoin" | "liquid"
    );
    for (entry, key) in entries.iter().zip(&keys) {
        let external = kassiber_pytext::strip(&or_text(field(entry, "external_id"))).to_owned();
        let canonical = native
            .then(|| canonical_txid(&Value::from(external.as_str())))
            .flatten();
        let id = &resolved_by_key
            .iter()
            .find(|(k, _)| k == key)
            .expect("checked")
            .1;
        let row = first(
            conn,
            "SELECT * FROM transactions WHERE id = ? AND workspace_id = ? AND profile_id = ? AND wallet_id = ?",
            [id.as_str(), target.workspace_id, target.profile_id, target.wallet_id],
        )?
        .ok_or_else(|| mismatch("resolved_row_missing", vec![]))?;
        let row_key = (
            kassiber_pytext::lower(kassiber_pytext::strip(&or_text(field(&row, "external_id")))),
            asset_code(field(&row, "asset")),
            kassiber_pytext::lower(kassiber_pytext::strip(&or_text(field(&row, "direction")))),
        );
        if row_key != *key {
            return Err(mismatch("resolved_row_identity_mismatch", vec![]));
        }
        let row_id = py_str(field(&row, "id"));
        if canonical.is_some() && or_text(field(&row, "external_id_kind")) != "txid" {
            conn.execute(
                "UPDATE transactions SET external_id_kind = 'txid' WHERE id = ?",
                [row_id.as_str()],
            )
            .map_err(database_error)?;
        }
        conn.execute(
            "INSERT INTO chain_observation_provenance(
                transaction_id, workspace_id, profile_id, wallet_id,
                authority_version, observer_ids_json, observer_kinds_json,
                chain, network, application_revision, graph_hash, quantity_hash,
                fee_attribution, observed_at, updated_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(transaction_id) DO UPDATE SET
                authority_version = excluded.authority_version,
                observer_ids_json = excluded.observer_ids_json,
                observer_kinds_json = excluded.observer_kinds_json,
                chain = excluded.chain,
                network = excluded.network,
                application_revision = excluded.application_revision,
                graph_hash = excluded.graph_hash,
                quantity_hash = excluded.quantity_hash,
                fee_attribution = excluded.fee_attribution,
                observed_at = excluded.observed_at,
                updated_at = excluded.updated_at",
            params_from_iter([
                Sql::Text(row_id),
                Sql::Text(py_str(field(&row, "workspace_id"))),
                Sql::Text(py_str(field(&row, "profile_id"))),
                Sql::Text(py_str(field(&row, "wallet_id"))),
                Sql::Integer(AUTHORITY_VERSION),
                Sql::Text(strings_json(
                    get(entry, "observer_ids").unwrap_or(&Value::List(vec![])),
                )?),
                Sql::Text(strings_json(
                    get(entry, "observer_kinds").unwrap_or(&Value::List(vec![])),
                )?),
                Sql::Text(target.chain.into()),
                Sql::Text(target.network.into()),
                Sql::Text(target.application_revision.into()),
                Sql::Text(graph_hash(field(&row, "raw_json"))?),
                Sql::Text(crate::observer_state::quantity_hash(&row)?),
                Sql::Text(fee_attribution(field(&row, "raw_json")).into()),
                Sql::Text(timestamp.clone()),
                Sql::Text(timestamp.clone()),
            ]),
        )
        .map_err(database_error)?;
    }
    Ok(keys.len())
}

/// The accounting-relevant closed facts `_observed_accounting_inputs` compares.
#[derive(Debug)]
pub struct AccountingInputs {
    pub states: BTreeMap<Physical, bool>,
    pub bindings: Vec<(Value, Vec<Value>)>,
}

impl AccountingInputs {
    /// Python's `(states, bindings)` dict-tuple equality.
    pub fn same(&self, other: &Self) -> bool {
        self.states == other.states
            && self.bindings.len() == other.bindings.len()
            && self.bindings.iter().all(|(id, values)| {
                other.bindings.iter().any(|(other_id, other_values)| {
                    py_eq(id, other_id)
                        && values.len() == other_values.len()
                        && values.iter().zip(other_values).all(|(a, b)| py_eq(a, b))
                })
            })
    }
}

const BINDING_FIELDS: [&str; 8] = [
    "observation_authority_version",
    "observation_graph_hash",
    "observation_quantity_hash",
    "observation_observer_ids",
    "observation_observer_kinds",
    "observation_fee_attribution",
    "observation_chain",
    "observation_network",
];

/// `sync._observed_accounting_inputs(conn, profile_id, entries)`.
pub fn accounting_inputs(
    conn: &Connection,
    profile_id: &str,
    entries: &[Value],
) -> Result<AccountingInputs> {
    let txids: BTreeSet<String> = entries
        .iter()
        .filter_map(|entry| canonical_txid(field(entry, "external_id")))
        .collect();
    let txids: Vec<String> = txids.into_iter().collect();
    let mut states = BTreeMap::new();
    let mut bindings: Vec<(Value, Vec<Value>)> = Vec::new();
    for batch in txids.chunks(400) {
        let placeholders = vec!["?"; batch.len()].join(",");
        let found = rows(
            conn,
            &format!(
                "SELECT t.*, w.kind AS wallet_kind, w.config_json AS wallet_config_json,
                observation.authority_version AS observation_authority_version,
                observation.graph_hash AS observation_graph_hash,
                observation.quantity_hash AS observation_quantity_hash,
                observation.observed_at AS observation_observed_at,
                observation.observer_ids_json AS observation_observer_ids,
                observation.observer_kinds_json AS observation_observer_kinds,
                observation.fee_attribution AS observation_fee_attribution,
                observation.chain AS observation_chain,
                observation.network AS observation_network
            FROM wallets w
            CROSS JOIN transactions t INDEXED BY idx_transactions_wallet_external_ci_match
                ON t.wallet_id = w.id
            LEFT JOIN chain_observation_provenance observation ON observation.transaction_id = t.id
            WHERE w.profile_id = ? AND t.profile_id = ? AND t.excluded = 0
                AND t.external_id IS NOT NULL AND LOWER(t.external_id) IN ({placeholders})"
            ),
            params_from_iter(
                [profile_id.to_owned(), profile_id.to_owned()]
                    .into_iter()
                    .chain(batch.iter().cloned()),
            ),
        )?;
        states.extend(confirmation_states(&found)?);
        for row in &found {
            let id = field(row, "id").clone();
            let values: Vec<Value> = BINDING_FIELDS
                .iter()
                .map(|name| field(row, name).clone())
                .collect();
            if let Some(slot) = bindings.iter_mut().find(|(other, _)| py_eq(other, &id)) {
                slot.1 = values;
            } else {
                bindings.push((id, values));
            }
        }
    }
    Ok(AccountingInputs { states, bindings })
}

/// Canonical journal freshness, including transaction count and calculation semantics.
pub fn journal_current(conn: &Connection, profile_id: &str) -> Result<bool> {
    Ok(first(
        conn,
        &format!("SELECT 1 FROM profiles p WHERE id = ? AND last_processed_at IS NOT NULL
            AND journal_input_version = last_processed_input_version AND last_processed_tx_count=(SELECT COUNT(*) FROM transactions t WHERE t.profile_id=p.id AND t.excluded=0) AND {}",kassiber_store::semantics::current_sql("p")),
        [profile_id],
    )?
    .is_some())
}

#[cfg(test)]
mod tests;

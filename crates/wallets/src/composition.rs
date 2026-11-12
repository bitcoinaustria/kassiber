//! Compose prepared observer projections without any I/O. The coordinator must
//! persist the entire result under its wallet savepoint, after scope fencing.
use crate::bitcoin_records::{entries, from_esplora, text};
use kassiber_envelope::AppError;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, AppError>;
type Key = (String, String, String);
/// JSON-only facts for one structural observer identity. An identity and its
/// source/scope must already be bound by the preparation coordinator.
#[derive(Default)]
pub struct ObserverFacts {
    pub observer_id: String,
    pub observer_kind: String,
    pub records: Vec<Value>,
    pub outputs: Vec<Value>,
    pub retracted_external_ids: Vec<String>,
    pub checkpoint: Value,
    pub reused_records: u64,
}
pub struct Composed {
    pub records: Vec<Value>,
    pub metadata: Value,
    /// Private structural mappings; never a public or AI projection. The
    /// atomic coordinator supplies the application revision when persisting.
    pub provenance_entries: Vec<Value>,
}
fn conflict(kind: &str, txid: Option<&str>, message: &str) -> AppError {
    let mut details = json!({"conflict_kind":kind});
    if let Some(txid) = txid {
        details["txid"] = json!(txid);
    }
    let mut error = AppError::new("observer_projection_conflict", message)
        .with_details(kassiber_pytext::loads(&details.to_string()).expect("JSON details"));
    if kind == "mixed_observer_routes" {
        error = error.with_hint("Select exactly one observer route before contacting the backend.");
    }
    error
}
fn txid(record: &Value) -> String {
    let value = if crate::descriptors::truthy(&record["txid"]) {
        &record["txid"]
    } else {
        &record["external_id"]
    };
    if value.is_null() {
        String::new()
    } else {
        crate::descriptors::text(value)
    }
}
fn key(record: &Value) -> Key {
    (
        txid(record),
        text(&record["asset"]).into(),
        text(&record["direction"]).into(),
    )
}
fn provenance_key(record: &Value) -> Key {
    let (txid, asset, direction) = key(record);
    (
        kassiber_pytext::strip(&txid).to_lowercase(),
        kassiber_pytext::strip(&asset).to_uppercase(),
        kassiber_pytext::strip(&direction).to_lowercase(),
    )
}
fn canonical_or_literal(value: &str) -> String {
    let value = kassiber_pytext::strip(value);
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        value.to_lowercase()
    } else {
        value.into()
    }
}
fn insert<K: PartialEq, V>(rows: &mut Vec<(K, V)>, key: K, value: V, replace: bool) {
    if let Some((_, previous)) = rows.iter_mut().find(|(k, _)| *k == key) {
        if replace {
            *previous = value;
        }
    } else {
        rows.push((key, value));
    }
}

/// Match Python's `apply_fetch_observer_updates` composition. Passing no
/// observers preserves the adapter route. Observer facts and a compatibility
/// projection cannot be combined; select exactly one route before acquisition.
pub fn compose(
    observers: &[ObserverFacts],
    adapter_records: &[Value],
    adapter_metadata: &Value,
    backend_name: &str,
) -> Result<Composed> {
    if observers.is_empty() {
        return Ok(Composed {
            records: adapter_records.to_vec(),
            metadata: adapter_metadata.clone(),
            provenance_entries: vec![],
        });
    }
    if !adapter_records.is_empty() || !adapter_metadata["utxos"].is_null() {
        return Err(conflict(
            "mixed_observer_routes",
            None,
            "A wallet refresh cannot apply dependency and compatibility projections together",
        ));
    }
    let mut outputs = vec![];
    let mut retracted = vec![];
    let mut checkpoints = BTreeMap::new();
    let mut highest = BTreeMap::<String, i64>::new();
    let mut records = vec![];
    let mut groups: Vec<(String, Vec<Value>)> = vec![];
    let mut reused = 0_u64;
    for observer in observers {
        reused = reused.checked_add(observer.reused_records).ok_or_else(|| {
            AppError::new("observer_state_invalid", "Invalid reused record count")
        })?;
        for output in &observer.outputs {
            let point = (
                text(&output["txid"]).to_owned(),
                output["vout"].as_u64().unwrap_or(0),
            );
            insert(&mut outputs, point, output.clone(), true);
        }
        retracted.extend(observer.retracted_external_ids.iter().cloned());
        checkpoints.insert(observer.observer_id.clone(), observer.checkpoint.clone());
        for (branch, value) in observer.checkpoint["highest_used"]
            .as_object()
            .into_iter()
            .flatten()
        {
            if let Some(value) = crate::scan::integer(value).and_then(|n| i64::try_from(n).ok()) {
                highest
                    .entry(branch.clone())
                    .and_modify(|v| *v = (*v).max(value))
                    .or_insert(value.max(-1));
            }
        }
        for record in &observer.records {
            let id = txid(record);
            let raw = record["raw_json"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok());
            if !id.is_empty()
                && record["asset"] == "BTC"
                && raw.as_ref().is_some_and(|r| r["observer"] == "bdk")
            {
                if let Some((_, group)) = groups.iter_mut().find(|(key, _)| key == &id) {
                    group.push(raw.unwrap());
                } else {
                    groups.push((id, vec![raw.unwrap()]));
                }
            } else {
                insert(&mut records, key(record), record.clone(), false);
            }
        }
    }
    for (id, group) in groups {
        let scripts: BTreeSet<_> = group
            .iter()
            .flat_map(|r| entries(r, "observer_owned_scripts"))
            .map(crate::descriptors::text)
            .filter(|s| !s.is_empty())
            .collect();
        // max() in Python keeps the first candidate on a tie.
        let richest = group
            .iter()
            .enumerate()
            .max_by_key(|(i, raw)| {
                (
                    entries(raw, "vin")
                        .iter()
                        .filter(|v| crate::descriptors::truthy(&v["prevout"]))
                        .count(),
                    std::cmp::Reverse(*i),
                )
            })
            .map(|(_, v)| v)
            .expect("nonempty BDK group");
        let mut raw = richest.clone();
        let mut vin = entries(&raw, "vin").to_vec();
        let input_key = |v: &Value| {
            (
                text(&v["txid"]).to_lowercase(),
                v["vout"].as_u64().unwrap_or(0),
            )
        };
        for candidate in &group {
            for input in entries(candidate, "vin")
                .iter()
                .filter(|v| v.is_object() && crate::descriptors::truthy(&v["prevout"]))
            {
                let target = vin
                    .iter_mut()
                    .rev()
                    .find(|v| !text(&v["txid"]).is_empty() && input_key(v) == input_key(input))
                    .ok_or_else(|| {
                        conflict(
                            "bdk_inconsistent_inputs",
                            Some(&id),
                            "BDK observers returned inconsistent inputs for one transaction",
                        )
                    })?;
                if !target["prevout"].is_null() && target["prevout"] != input["prevout"] {
                    return Err(conflict(
                        "bdk_conflicting_prevouts",
                        Some(&id),
                        "BDK observers returned conflicting prevouts for one transaction",
                    ));
                }
                target["prevout"] = input["prevout"].clone();
            }
        }
        raw["vin"] = json!(vin);
        raw.as_object_mut()
            .expect("BDK graph object")
            .remove("observer_owned_scripts");
        if let Some(record) = from_esplora(
            &raw,
            &scripts,
            if backend_name.is_empty() {
                "BDK"
            } else {
                backend_name
            },
        )? {
            insert(
                &mut records,
                (
                    id,
                    text(&record["asset"]).into(),
                    text(&record["direction"]).into(),
                ),
                record,
                true,
            );
        }
    }
    let records: Vec<_> = records.into_iter().map(|(_, v)| v).collect();
    let live: BTreeSet<_> = records
        .iter()
        .map(|r| canonical_or_literal(&txid(r)))
        .collect();
    let mut absent = vec![];
    for value in retracted {
        if !live.contains(&canonical_or_literal(&value))
            && !value.is_empty()
            && !absent.contains(&value)
        {
            absent.push(value);
        }
    }
    let mut metadata = if adapter_metadata.is_object() {
        adapter_metadata.clone()
    } else {
        json!({})
    };
    // Full snapshot, including empty: the last spent output must clear inventory.
    metadata["utxos"] = json!(outputs.into_iter().map(|(_, v)| v).collect::<Vec<_>>());
    if reused > 0 {
        metadata["observer_records_reused"] = json!(reused);
    }
    if !absent.is_empty() {
        metadata["observer_retracted_external_ids"] = json!(absent);
    }
    if !checkpoints.is_empty() {
        let mut checkpoint = if metadata["freshness_checkpoint"].is_object() {
            metadata["freshness_checkpoint"].clone()
        } else {
            json!({})
        };
        checkpoint["observer_instances"] = json!(checkpoints);
        if !highest.is_empty() {
            checkpoint["highest_used"] = json!(highest);
        }
        metadata["freshness_checkpoint"] = checkpoint;
    }
    let provenance_entries = provenance(observers, &records);
    Ok(Composed {
        records,
        metadata,
        provenance_entries,
    })
}

#[derive(Default, Clone)]
struct Sources {
    ids: BTreeSet<String>,
    kinds: BTreeSet<String>,
}
fn provenance(observers: &[ObserverFacts], records: &[Value]) -> Vec<Value> {
    let mut sources = BTreeMap::<Key, Sources>::new();
    let mut by_txid = BTreeMap::<String, Sources>::new();
    for observer in observers {
        for record in &observer.records {
            let key = provenance_key(record);
            if key.0.is_empty() {
                continue;
            }
            let aggregate = by_txid.entry(key.0.clone()).or_default();
            aggregate.ids.insert(observer.observer_id.clone());
            aggregate.kinds.insert(observer.observer_kind.clone());
            let source = sources.entry(key).or_default();
            source.ids.insert(observer.observer_id.clone());
            source.kinds.insert(observer.observer_kind.clone());
        }
    }
    records.iter().map(provenance_key).filter_map(|key| {
        sources.get(&key).or_else(|| by_txid.get(&key.0)).filter(|_| !key.0.is_empty()).map(|s| (key,s))
    }).collect::<BTreeMap<_,_>>().into_iter().map(|((id,asset,direction),s)| json!({"external_id":id,"asset":asset,"direction":direction,"observer_ids":s.ids,"observer_kinds":s.kinds})).collect()
}

#[cfg(test)]
mod tests;

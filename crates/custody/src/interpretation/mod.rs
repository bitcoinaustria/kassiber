//! Incremental interpreter port. The product operation deliberately does not
//! publish this stage until every interpreter and their composition are ported.
//! The example harness accepts only the ordinary raw snapshot, never Python
//! claims, pairs, authority decisions, or materialized components.
mod channels;
mod claims;
mod consolidation;
mod fanout;
mod holds;
mod htlc;
mod matcher;
mod native;
mod ownership;
mod pairs;
mod pipeline;
mod privacy;
mod recorded;
mod safety;
mod samourai;
mod text;
use crate::{
    codec::{text, truth},
    components, evidence,
    snapshot::Snapshot,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn python_value_string(value: &kassiber_pyjson::Value) -> String {
    text::string(value)
}

/// Python's int() family, before any bounded arithmetic. Raw row and compiler
/// facts may contain strings or containers even though canonical quantities do not.
pub(super) fn integer(value: &Value) -> Result<Value, String> {
    crate::codec::integer(value).map_err(|_| {
        if value.is_null() || value.is_array() || value.is_object() {
            "TypeError"
        } else {
            "ValueError"
        }
        .to_owned()
    })
}

pub(super) fn integer_or_zero(value: &Value) -> Result<Value, String> {
    if truth(value) {
        integer(value)
    } else {
        Ok(json!(0))
    }
}

pub(super) fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

pub(super) fn anchor(row: &Value) -> String {
    if truth(&row["journal_transaction_id"]) {
        text(&row["journal_transaction_id"])
    } else {
        text(&row["id"])
    }
}

/// Keep the canonical event alias iteration: an alias may resolve to a keeper
/// whose transaction id differs from the original row's id.
fn observations(canonical: &Value) -> BTreeMap<String, Value> {
    let by_hash: BTreeMap<_, _> = array(&canonical["events"])
        .iter()
        .flat_map(|event| array(&event["legs"]))
        .map(|item| (text(&item["quantity_hash"]), item))
        .collect();
    let mut result = BTreeMap::new();
    for event in array(&canonical["events"]) {
        for alias in array(&event["observation_aliases"]) {
            result.insert(text(&alias[0]), by_hash[&text(&alias[1])].clone());
        }
    }
    result
}

pub(super) struct Context {
    pub observations: BTreeMap<String, Value>,
    pub canonical: Value,
    pub excluded: BTreeSet<String>,
}
impl Context {
    fn load(snapshot: &Snapshot) -> Result<Self, String> {
        let components = components::compute(snapshot).map_err(|e| e.code.to_owned())?;
        let evidence = evidence::compute(snapshot)?;
        let excluded = array(&components["components"])
            .iter()
            .flat_map(|item| array(&item["legs"]))
            .filter_map(|leg| {
                let value = if truth(&leg["anchor_transaction_id"]) {
                    &leg["anchor_transaction_id"]
                } else {
                    &leg["transaction_id"]
                };
                truth(value).then(|| text(value))
            })
            .collect();
        Ok(Self {
            observations: observations(&evidence["canonical_input"]),
            canonical: evidence["canonical_input"].clone(),
            excluded,
        })
    }
}

/// Test harness API, independent of `custody` and its implemented-stage list.
pub fn partial(snapshot: &Snapshot) -> Result<Value, String> {
    let context = Context::load(snapshot)?;
    Ok(
        json!({"native": native::compile(snapshot, &context), "channels": channels::compile(snapshot), "graph": crate::graph::prerequisites(snapshot).unwrap_or_else(|error|json!({"error":error})), "safety": safety::compile(snapshot, &context)?, "pipeline": pipeline::compile(snapshot, &context)?, "consolidation": consolidation::compile(snapshot, &context)?, "ownership": ownership::compile(snapshot, &context)?, "fanout": fanout::compile(snapshot, &context)?, "recorded": recorded::compile(snapshot, &context)?, "samourai": samourai::compile(snapshot, &context)?, "shared": {
            "observations_by_transaction": context.observations,
            "excluded_transaction_ids": context.excluded,
            "row_anchor_ids": snapshot.transactions.iter().map(|row| anchor(&json!(row))).collect::<Vec<_>>()
        }}),
    )
}

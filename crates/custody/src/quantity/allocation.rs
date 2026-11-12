//! Ordered N:M allocation with exact half-open coordinates and residuals.
use super::{exact, number};
use crate::codec;
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Debug, PartialEq)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
    pub details: Value,
}
fn error(code: &'static str, message: impl Into<String>, details: Value) -> Error {
    Error {
        code,
        message: message.into(),
        details,
    }
}
// JSON arrays, strings and object keys are the iterable raw bucket shapes.
fn sequence(v: &Value) -> Option<Vec<Value>> {
    match v {
        Value::Array(v) => Some(v.clone()),
        Value::String(v) => Some(v.chars().map(|c| json!(c.to_string())).collect()),
        Value::Object(v) => Some(v.keys().map(|s| json!(s)).collect()),
        _ => None,
    }
}
fn buckets(raw: &Value, kind: &str) -> Result<Vec<(String, BigInt)>, Error> {
    let raw = sequence(raw).ok_or_else(|| {
        error(
            "custody_allocation_invalid_buckets",
            format!("{kind} allocation buckets must be a sequence"),
            json!({"kind":kind}),
        )
    })?;
    let mut result = Vec::new();
    for (ordinal, bucket) in raw.iter().enumerate() {
        let pair = sequence(bucket).filter(|v| v.len() == 2).ok_or_else(|| {
            error(
                "custody_allocation_invalid_bucket",
                format!("each {kind} allocation bucket must contain id and amount"),
                json!({"kind":kind,"ordinal":ordinal}),
            )
        })?;
        let id = if codec::truth(&pair[0]) {
            crate::gaps::raw_string(&pair[0])
        } else {
            String::new()
        };
        let n = exact(&pair[1]).ok().filter(|n| *n >= BigInt::from(0));
        if id.is_empty() || n.is_none() {
            return Err(error(
                "custody_allocation_invalid_bucket",
                format!("{kind} allocation bucket ids and amounts must be exact"),
                json!({"kind":kind,"ordinal":ordinal,"bucket_id":id,"amount_msat":pair[1]}),
            ));
        }
        result.push((id, n.unwrap()));
    }
    if result
        .iter()
        .map(|(id, _)| id)
        .collect::<BTreeSet<_>>()
        .len()
        != result.len()
    {
        return Err(error(
            "custody_allocation_duplicate_bucket",
            format!("{kind} allocation bucket ids must be unique"),
            json!({"kind":kind}),
        ));
    }
    Ok(result)
}

/// Allocation is all-or-nothing. Validate both boundary sequences, then the
/// requested amount, then both capacities, matching Python's admission order.
pub fn allocate_msat_fifo(
    sources: &Value,
    sinks: &Value,
    amount_msat: Option<&Value>,
) -> Result<Value, Error> {
    let sources = buckets(sources, "source")?;
    let sinks = buckets(sinks, "sink")?;
    let source_total: BigInt = sources.iter().map(|(_, n)| n).sum();
    let sink_total: BigInt = sinks.iter().map(|(_, n)| n).sum();
    let requested = match amount_msat.filter(|v| !v.is_null()) {
        None => sink_total.clone(),
        Some(value) => exact(value)
            .ok()
            .filter(|n| *n >= BigInt::from(0))
            .ok_or_else(|| {
                error(
                    "custody_allocation_invalid_amount",
                    "allocation amount must be an exact non-negative integer",
                    json!({"amount_msat":value}),
                )
            })?,
    };
    if requested > source_total || requested > sink_total {
        return Err(error(
            "custody_allocation_insufficient_capacity",
            "allocation boundaries cannot satisfy the requested quantity",
            json!({"amount_msat":number(&requested),"source_capacity_msat":number(&source_total),"sink_capacity_msat":number(&sink_total)}),
        ));
    }
    let mut source_used = vec![BigInt::from(0); sources.len()];
    let mut sink_used = vec![BigInt::from(0); sinks.len()];
    let (mut source, mut sink) = (0, 0);
    let mut remaining = requested.clone();
    let mut cells = Vec::new();
    while remaining > BigInt::from(0) {
        while source_used[source] == sources[source].1 {
            source += 1;
        }
        while sink_used[sink] == sinks[sink].1 {
            sink += 1;
        }
        let amount = remaining
            .clone()
            .min(&sources[source].1 - &source_used[source])
            .min(&sinks[sink].1 - &sink_used[sink]);
        cells.push(json!({"$type":"DeterministicAllocationCell","source_id":sources[source].0,"sink_id":sinks[sink].0,"amount_msat":number(&amount),"source_start_msat":number(&source_used[source]),"sink_start_msat":number(&sink_used[sink])}));
        source_used[source] += &amount;
        sink_used[sink] += &amount;
        remaining -= amount;
    }
    let residuals = |buckets: &[(String, BigInt)], used: &[BigInt]| {
        buckets
            .iter()
            .zip(used)
            .map(|((id, n), used)| json!([id, number(&(n - used))]))
            .collect::<Vec<_>>()
    };
    Ok(
        json!({"$type":"DeterministicAllocationResult","cells":cells,"allocated_msat":number(&requested),"source_remaining":residuals(&sources,&source_used),"sink_remaining":residuals(&sinks,&sink_used)}),
    )
}

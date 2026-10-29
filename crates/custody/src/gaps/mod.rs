//! Bounded, advisory missing-wallet search. No storage or accounting authority.
mod candidate;
mod normalize;
pub mod pagination;
mod search;

use crate::{codec, snapshot::Snapshot};
use kassiber_pyjson::{BigInt, Value as Py};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub use candidate::{authored_claim_fingerprint, candidate_fingerprint, snapshot_gap, Candidate};
pub use search::{search, Limits, SearchResult};

#[derive(Clone, Debug)]
struct Leg {
    id: String,
    profile_id: String,
    wallet_id: String,
    wallet_label: String,
    when: i64,
    direction: String,
    asset: String,
    chain: String,
    network: String,
    principal: BigInt,
    fee: BigInt,
    debit: BigInt,
    signals: BTreeSet<String>,
    disqualifiers: BTreeSet<String>,
}
type Group = Vec<usize>;

fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn zero() -> BigInt {
    BigInt::from(0)
}
fn number(n: &BigInt) -> Value {
    serde_json::from_str(&n.to_string()).expect("exact integer")
}
fn ids(group: &[usize], legs: &[Leg]) -> Vec<String> {
    let mut ids: Vec<_> = group.iter().map(|&i| legs[i].id.clone()).collect();
    ids.sort();
    ids
}
fn total(group: &[usize], legs: &[Leg]) -> BigInt {
    group.iter().map(|&i| &legs[i].principal).sum()
}
fn distance(a: &BigInt, b: &BigInt) -> BigInt {
    if a >= b {
        a - b
    } else {
        b - a
    }
}
fn seconds(delta: i64) -> i64 {
    // int(timedelta.total_seconds()) truncates a correctly rounded rational.
    // Casting the microsecond numerator to f64 first double-rounds dates more
    // than 285 years apart. Only rounding up to the next integral second can
    // change int(); compare its exact half-ULP boundary in integer arithmetic.
    let magnitude = delta.unsigned_abs();
    let whole = magnitude / 1_000_000;
    let next = whole + 1;
    let exponent = next.ilog2();
    let shift = 53 - exponent + u32::from(next.is_power_of_two());
    let distance = u128::from(1_000_000 - magnitude % 1_000_000);
    let rounded_up = (distance << shift) <= 1_000_000;
    let result = (whole + u64::from(rounded_up)) as i64;
    if delta < 0 {
        -result
    } else {
        result
    }
}

/// Recompute the production stage from the SQL-selected raw worklist. The two
/// ignored populations come from Rust's preceding stages, never Python verdicts.
pub fn stage(
    snapshot: &Snapshot,
    components: &Value,
    evidence: &Value,
    interpretation: &Py,
) -> Result<Value, String> {
    let (ignored, accounting) =
        crate::interpretation::gap_ignored_ids(snapshot, components, evidence, interpretation)?;
    let rows: Vec<_> = snapshot
        .gap_worklist
        .rows
        .iter()
        .map(|r| json!(r))
        .collect();
    compute(
        &rows,
        snapshot.gap_worklist.large_book,
        &ignored,
        &accounting,
        250,
    )
}

/// Python compute_gap_search_result, including its normalization-before-ignore
/// admission order and the accounting population's separate blockers.
pub fn compute(
    rows: &[Value],
    large_book: bool,
    ignored: &BTreeSet<String>,
    accounting_ignored: &BTreeSet<String>,
    limit: usize,
) -> Result<Value, String> {
    for row in rows {
        normalize::leg(row, &BTreeSet::new())?;
    }
    let limits = Limits {
        max_input_rows: if large_book { 1 } else { 50_000 },
        max_candidates: limit,
        ..Limits::default()
    };
    let bounded = |mut r: SearchResult| {
        if large_book && r.search_complete {
            r.accounting_candidates.clear();
            r.search_complete = false;
            r.limit_kind = Some("boundary_worklist");
            r.promotion_eligible_count = 0;
            r.message = Some("custody-gap search used a bounded large-book worklist".into());
        }
        r
    };
    let mut result = bounded(search(rows, ignored, &limits)?);
    if accounting_ignored != ignored {
        let accounting = bounded(search(rows, accounting_ignored, &limits)?);
        result.accounting_candidates = accounting.accounting_candidates;
        result.promotion_eligible_count = accounting.promotion_eligible_count;
        result.blocking_source_ids = accounting.blocking_source_ids;
    }
    Ok(result.value())
}

/// Preserve Python stringification for raw containers as well as scalar facts.
fn string(v: &Value) -> String {
    crate::interpretation::python_value_string(&codec::from_json(v).expect("JSON value"))
}
pub(crate) fn raw_string(v: &Value) -> String {
    string(v)
}
fn text(v: &Value) -> String {
    if codec::truth(v) {
        string(v)
    } else {
        String::new()
    }
}

/// Latest raw siblings are retained by the loader. A conflicting revision or a
/// reopened bridge cannot suppress a newly computed accounting candidate.
pub fn dismissed_fingerprints(rows: &[Value]) -> Value {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for row in rows {
        groups.entry(string(&row["gap_id"])).or_default().push(row);
    }
    let mut result = serde_json::Map::new();
    for (id, rows) in groups {
        let signature = |r: &Value| {
            json!([
                r["candidate_fingerprint"],
                r["action"],
                if codec::truth(&r["event_kind"]) {
                    r["event_kind"].clone()
                } else {
                    json!("review_decision")
                },
                r["component_id"]
            ])
        };
        let last = rows.last().expect("nonempty revision siblings");
        if rows.iter().any(|row| signature(row) != signature(last)) {
            continue;
        }
        if last["action"] == "dismissed" && last["event_kind"] != "bridge_reopened" {
            result.insert(id, json!(string(&last["candidate_fingerprint"])));
        }
    }
    Value::Object(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vectors() -> &'static Vec<Value> {
        static V: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
        V.get_or_init(|| {
            serde_json::from_str(include_str!(
                "../../../../tests/fixtures/custody_gap_stages.json"
            ))
            .unwrap()
        })
    }
    #[test]
    fn gaps_operation_vectors() {
        for case in vectors() {
            let response: Value =
                serde_json::from_str(&crate::custody(&case["input"].to_string())).unwrap();
            if let Some(error) = case["error"].as_str() {
                assert_eq!(response["error"]["code"], error, "{}", case["case"]);
                assert!(response.get("stages").is_none());
            } else {
                assert_eq!(
                    response["stages"]["gaps"], case["expected"],
                    "{}",
                    case["case"]
                );
                assert_eq!(
                    response["skipped_stages"],
                    json!(["quantity", "finalized_projection"])
                );
            }
        }
    }
    #[test]
    fn gaps_ignored_ids_vectors() {
        for case in vectors().iter().filter(|c| c.get("error").is_none()) {
            let snapshot: Snapshot = serde_json::from_value(case["input"].clone()).unwrap();
            let components = crate::components::compute(&snapshot).unwrap();
            let evidence = crate::evidence::compute(&snapshot).unwrap();
            let migration = crate::interpretation::migration_quarantines(&snapshot).unwrap();
            let interpretation =
                crate::interpretation::stage(&snapshot, &components, &evidence, migration).unwrap();
            let (ignored, accounting) = crate::interpretation::gap_ignored_ids(
                &snapshot,
                &components,
                &evidence,
                &interpretation,
            )
            .unwrap();
            assert_eq!(json!(ignored), case["ignored"], "{}", case["case"]);
            assert_eq!(
                json!(accounting),
                case["accounting_ignored"],
                "{}",
                case["case"]
            );
        }
    }
}

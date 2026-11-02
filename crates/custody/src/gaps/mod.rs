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
pub(crate) use normalize::iso as iso_datetime;
pub use search::{search, Limits, SearchResult};

/// Locate an admission failure so service adapters can preserve Python's
/// exception message as well as the pure search oracle's exception class.
pub fn normalization_failure(rows: &[Value]) -> Option<(&Value, String)> {
    rows.iter().find_map(|row| {
        normalize::leg(row, &BTreeSet::new())
            .err()
            .map(|code| (row, code))
    })
}

/// Raw timestamp prefixes used by Python's saved review snapshots.
pub fn year_prefix(text: &str) -> Option<String> {
    let prefix: String = text.chars().take(4).collect();
    (!prefix.is_empty() && prefix.chars().all(crate::python_text::isdigit)).then_some(prefix)
}

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
    /// Dense rank of `id` among the searched legs: sorted ranks order and
    /// compare exactly as the sorted id strings do.
    rank: u32,
    /// `principal` when the search runs on `i128` amounts.
    small: i128,
}

/// An exact search amount. The search uses `i128` only when every principal
/// fits `i64` and there are fewer than 2^20 legs, so every sum and scaled
/// bound stays below 2^103; otherwise it uses `BigInt`. Both truncate
/// division toward zero, and principals are positive.
trait Amount: Clone + Ord {
    const SMALL: bool;
    fn of(leg: &Leg) -> Self;
    fn zero() -> Self;
    fn plus(&self, other: &Self) -> Self;
    fn minus(&self, other: &Self) -> Self;
    /// `self * ppm / 1_000_000`
    fn scaled(&self, ppm: u32) -> Self;
}
impl Amount for BigInt {
    const SMALL: bool = false;
    fn of(leg: &Leg) -> Self {
        leg.principal.clone()
    }
    fn zero() -> Self {
        zero()
    }
    fn plus(&self, other: &Self) -> Self {
        self + other
    }
    fn minus(&self, other: &Self) -> Self {
        self - other
    }
    fn scaled(&self, ppm: u32) -> Self {
        self * ppm / 1_000_000
    }
}
impl Amount for i128 {
    const SMALL: bool = true;
    fn of(leg: &Leg) -> Self {
        leg.small
    }
    fn zero() -> Self {
        0
    }
    fn plus(&self, other: &Self) -> Self {
        self + other
    }
    fn minus(&self, other: &Self) -> Self {
        self - other
    }
    fn scaled(&self, ppm: u32) -> Self {
        self * i128::from(ppm) / 1_000_000
    }
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
/// The group's identity key: `ids(group)` as ranks, with the same order.
fn key(group: &[usize], legs: &[Leg]) -> Vec<u32> {
    let mut ranks: Vec<u32> = group.iter().map(|&i| legs[i].rank).collect();
    ranks.sort_unstable();
    ranks
}
fn total(group: &[usize], legs: &[Leg]) -> BigInt {
    group.iter().map(|&i| &legs[i].principal).sum()
}
fn sum<N: Amount>(group: &[usize], legs: &[Leg]) -> N {
    group
        .iter()
        .fold(N::zero(), |total, &i| total.plus(&N::of(&legs[i])))
}
fn distance<N: Amount>(a: &N, b: &N) -> N {
    if a >= b {
        a.minus(b)
    } else {
        b.minus(a)
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
    evidence: &crate::evidence::Evidence,
    interpretation: &Py,
) -> Result<Value, String> {
    let span = crate::timing::span("gaps.ignored");
    let (ignored, accounting) =
        crate::interpretation::gap_ignored_ids(snapshot, components, evidence, interpretation)?;
    drop(span);
    compute_rows(
        &snapshot.gap_worklist.rows,
        |row| normalize::leg(row, &BTreeSet::new()),
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
    compute_rows(
        rows,
        |row| normalize::leg(row, &BTreeSet::new()),
        large_book,
        ignored,
        accounting_ignored,
        limit,
    )
}

fn compute_rows<T: Sync>(
    rows: &[T],
    normalize: impl Fn(&T) -> Result<Option<Leg>, String> + Sync,
    large_book: bool,
    ignored: &BTreeSet<String>,
    accounting_ignored: &BTreeSet<String>,
    limit: usize,
) -> Result<Value, String> {
    let span = crate::timing::span("gaps.admission");
    // Admission normalizes every row; the first refusal in row order wins.
    let mut legs = Vec::new();
    for leg in crate::par::map(rows, normalize) {
        if let Some(leg) = crate::par::unwrap(leg)? {
            legs.push(leg);
        }
    }
    drop(span);
    let _span = crate::timing::span("gaps.search");
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
    let advisory = if accounting_ignored == ignored {
        std::mem::take(&mut legs)
            .into_iter()
            .filter(|leg| !ignored.contains(&leg.id))
            .collect()
    } else {
        legs.iter()
            .filter(|leg| !ignored.contains(&leg.id))
            .cloned()
            .collect()
    };
    let mut result = bounded(search::normalized(advisory, &limits)?);
    if accounting_ignored != ignored {
        legs.retain(|leg| !accounting_ignored.contains(&leg.id));
        let accounting = bounded(search::normalized(legs, &limits)?);
        result.accounting_candidates = accounting.accounting_candidates;
        result.promotion_eligible_count = accounting.promotion_eligible_count;
        result.blocking_source_ids = accounting.blocking_source_ids;
    }
    Ok(result.into_value())
}

/// Preserve Python stringification for raw containers as well as scalar facts.
fn string(v: &Value) -> String {
    // str() of a JSON string is the string itself.
    if let Value::String(text) = v {
        return text.clone();
    }
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

/// Bounds used by Python's SQL worklist loader above the 50,000-row threshold.
/// Admission uses the same normalizer as search, before any claimed-id filter.
pub fn worklist_amount_ranges(rows: &[Value]) -> Result<Vec<(String, BigInt, BigInt)>, String> {
    let mut ranges = std::collections::BTreeMap::<String, (BigInt, BigInt)>::new();
    for row in rows {
        let Some(leg) = normalize::leg(row, &BTreeSet::new())? else {
            continue;
        };
        let minimum_total: BigInt = &leg.principal * 800_000_u32 / 1_000_000_u32;
        let minimum = ((minimum_total + 255_u32) / 256_u32).max(BigInt::from(1));
        let maximum: BigInt = &leg.principal + &leg.principal * 250_000_u32 / 1_000_000_u32;
        ranges
            .entry(leg.asset)
            .and_modify(|(low, high)| {
                *low = low.clone().min(minimum.clone());
                *high = high.clone().max(maximum.clone());
            })
            .or_insert((minimum, maximum));
    }
    Ok(ranges
        .into_iter()
        .map(|(asset, (low, high))| (asset, low, high))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gap_compute_retains_limit_and_admission_precedence() {
        assert_eq!(
            compute(&[], false, &BTreeSet::new(), &BTreeSet::new(), 0).unwrap_err(),
            "ValueError"
        );
        let rows = [
            json!({"id":"row","profile_id":"p","wallet_id":"w","asset":"BTC","direction":"outbound","amount":1,"occurred_at":"0001-01-01T00:00:00+01:00"}),
        ];
        let ignored = BTreeSet::from(["row".to_owned()]);
        assert_eq!(
            compute(&rows, false, &ignored, &ignored, 0).unwrap_err(),
            "OverflowError"
        );
    }
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
            let response: Value = serde_json::from_str(&crate::custody_with_finalized(
                &case["input"].to_string(),
                false,
            ))
            .unwrap();
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
                    json!(["finalized_projection", "fusion"])
                );
            }
        }
    }
    #[test]
    fn gaps_ignored_ids_vectors() {
        for case in vectors().iter().filter(|c| c.get("error").is_none()) {
            let snapshot: Snapshot = serde_json::from_value(case["input"].clone()).unwrap();
            let components = crate::components::compute(&snapshot).unwrap();
            let evidence = crate::evidence::stage(&snapshot).unwrap();
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

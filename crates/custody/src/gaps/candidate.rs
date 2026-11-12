use super::{
    array, distance, ids, key, normalize::iso, number, seconds, sum, total, zero, Amount, Group,
    Leg,
};
use crate::codec;
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct Candidate {
    pub value: Value,
    pub score: i64,
    pub coverage: BigInt,
    pub source_total: BigInt,
    pub elapsed: i64,
    pub id: String,
    pub source_ids: Vec<String>,
    pub return_ids: Vec<String>,
    pub reasons: Vec<String>,
    pub eligible: bool,
}

pub(super) fn build(sources: &Group, returns: &Group, legs: &[Leg]) -> Candidate {
    let source_total = total(sources, legs);
    let return_total = total(returns, legs);
    let source_fee: BigInt = sources.iter().map(|&i| &legs[i].fee).sum();
    let source_debit: BigInt = sources.iter().map(|&i| &legs[i].debit).sum();
    let retained = source_total.clone().min(return_total.clone());
    let residual = &source_total - &retained;
    let excess = &return_total - &retained;
    let coverage = &retained * 1_000_000 / &source_total;
    let started = sources.iter().map(|&i| legs[i].when).min().unwrap();
    let ended = returns.iter().map(|&i| legs[i].when).max().unwrap();
    let elapsed = seconds(ended - started).max(0);
    let unique = |group: &Group, label: bool| -> BTreeSet<String> {
        group
            .iter()
            .map(|&i| {
                if label {
                    &legs[i].wallet_label
                } else {
                    &legs[i].wallet_id
                }
            })
            .cloned()
            .collect()
    };
    let source_wallets = unique(sources, false);
    let destination_wallets = unique(returns, false);
    let different = source_wallets != destination_wallets;
    let mut reasons = vec![if coverage >= BigInt::from(950_000) {
        "amount_coverage_high"
    } else {
        "amount_coverage_partial"
    }
    .to_owned()];
    if elapsed >= 90 * 86_400 {
        reasons.push("long_horizon".into());
    }
    if sources.len() > 1 {
        reasons.push("split_source".into());
    }
    if returns.len() > 1 {
        reasons.push("split_return".into());
    }
    if different {
        reasons.push("wallet_transition".into());
    }
    if residual != zero() {
        reasons.push("unresolved_residual".into());
    }
    if excess != zero() {
        reasons.push("return_exceeds_source".into());
    }
    let signals: BTreeSet<_> = sources
        .iter()
        .chain(returns)
        .flat_map(|&i| legs[i].signals.iter().cloned())
        .collect();
    let disqualifiers: BTreeSet<_> = sources
        .iter()
        .chain(returns)
        .flat_map(|&i| legs[i].disqualifiers.iter().cloned())
        .collect();
    reasons.extend(signals);
    reasons.extend(disqualifiers);
    let signal_count: usize = sources
        .iter()
        .chain(returns)
        .map(|&i| legs[i].signals.len())
        .sum();
    let score = score(
        &coverage,
        &excess,
        &source_total,
        elapsed,
        different,
        signal_count,
    );
    let source_ids = ids(sources, legs);
    let return_ids = ids(returns, legs);
    let first = &legs[sources[0]];
    let mut identity = vec![first.profile_id.clone(), first.asset.clone()];
    identity.extend(source_ids.iter().cloned());
    identity.push("->".into());
    identity.extend(return_ids.iter().cloned());
    let id = format!("cg_{}", &codec::sha256(&identity.join("\x1f"))[..24]);
    let value = json!({
        "$type":"CustodyGapCandidate", "gap_id":id, "profile_id":first.profile_id,
        "asset":first.asset, "protocol_chain":first.chain, "network":first.network,
        "source_ids":source_ids,"return_ids":return_ids,"source_wallet_ids":source_wallets,
        "destination_wallet_ids":destination_wallets,"source_wallet_labels":unique(sources,true),
        "destination_wallet_labels":unique(returns,true),"source_total_msat":number(&source_total),
        "source_fee_msat":number(&source_fee),"source_debit_msat":number(&source_debit),
        "return_total_msat":number(&return_total),"retained_msat":number(&retained),
        "residual_msat":number(&residual),"excess_msat":number(&excess),"coverage_ppm":number(&coverage),
        "started_at":iso(started),"ended_at":iso(ended),"elapsed_seconds":elapsed,"score":score,
        "confidence":if score>=850 {"strong"} else if score>=750 {"moderate"} else {"weak"},
        "reason_codes":reasons,"promotion_eligible":false,"competitor_score_margin":null,
        "conflict_set_id":"","conflict_size":1
    });
    Candidate {
        value,
        score,
        coverage,
        source_total,
        elapsed,
        id,
        source_ids,
        return_ids,
        reasons,
        eligible: false,
    }
}

fn score(
    coverage: &BigInt,
    excess: &BigInt,
    source: &BigInt,
    elapsed: i64,
    different: bool,
    signals: usize,
) -> i64 {
    let amount: BigInt = coverage * 700 / 1_000_000;
    let days = elapsed / 86_400;
    let bits = i64::from((days + 1).ilog2() + 1);
    let time = 25.max(200 - 10 * bits);
    let wallet = if different { 50 } else { 0 };
    let evidence = 80.min(signals as i64 * 40);
    let excess_ppm: BigInt = excess * 1_000_000 / source;
    let penalty = BigInt::from(150).min(excess_ppm * 150 / 250_000);
    let score: BigInt = amount + time + wallet + evidence - penalty;
    score
        .max(zero())
        .min(BigInt::from(1000))
        .to_string()
        .parse()
        .expect("bounded score")
}

pub(super) fn stamp(
    mut candidates: Vec<Candidate>,
    limited: &BTreeSet<String>,
    margin: i64,
) -> Vec<Candidate> {
    // Union by index with the smallest gap id as representative, as Python.
    let mut parent: Vec<usize> = (0..candidates.len()).collect();
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    let mut by_leg = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        for id in candidate.source_ids.iter().chain(&candidate.return_ids) {
            if let Some(&other) = by_leg.get(id) {
                let a = root(&mut parent, index);
                let b = root(&mut parent, other);
                if candidates[a].id <= candidates[b].id {
                    parent[b] = a;
                } else {
                    parent[a] = b;
                }
            } else {
                by_leg.insert(id.clone(), index);
            }
        }
    }
    let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..candidates.len() {
        clusters
            .entry(root(&mut parent, index))
            .or_default()
            .push(index);
    }
    for (root, members) in clusters {
        let cluster_id = candidates[root].id.clone();
        let mut scores: Vec<_> = members
            .iter()
            .map(|&i| (candidates[i].score, candidates[i].id.clone()))
            .collect();
        scores.sort_by(|a, b| b.cmp(a));
        for &index in &members {
            let candidate = &mut candidates[index];
            let competitor = scores
                .iter()
                .find(|(_, id)| *id != candidate.id)
                .map(|(score, _)| candidate.score - score);
            let structured = candidate
                .reasons
                .iter()
                .any(|r| r.starts_with("structured_") && r != "structured_external_origin");
            let disqualified = candidate
                .reasons
                .iter()
                .any(|r| r == "structured_external_origin");
            candidate.eligible =
                structured && !disqualified && competitor.is_none_or(|v| v >= margin);
            if limited.contains(&candidate.id) {
                candidate.reasons.push("search_capacity_incomplete".into());
            }
            candidate.reasons.push(
                if candidate.eligible {
                    "promotion_eligible_structured_signal"
                } else if disqualified {
                    "promotion_ineligible_external_origin"
                } else if !structured {
                    "search_hint_only"
                } else {
                    "competitor_margin_insufficient"
                }
                .into(),
            );
            if limited.contains(&candidate.id) && candidate.eligible {
                candidate.eligible = false;
                candidate
                    .reasons
                    .retain(|r| r != "promotion_eligible_structured_signal");
                candidate
                    .reasons
                    .push("capacity_source_suspense_required".into());
            }
            candidate.value["conflict_set_id"] = json!(cluster_id);
            candidate.value["conflict_size"] = json!(members.len());
            candidate.value["competitor_score_margin"] = json!(competitor);
            candidate.value["promotion_eligible"] = json!(candidate.eligible);
            candidate.value["reason_codes"] = json!(candidate.reasons);
        }
    }
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.coverage.cmp(&a.coverage))
            .then(b.source_total.cmp(&a.source_total))
            .then(a.elapsed.cmp(&b.elapsed))
            .then(a.id.cmp(&b.id))
    });
    candidates
}

fn fingerprint(candidate: &Value, authored: bool) -> Result<String, String> {
    let mut payload = serde_json::Map::new();
    payload.insert("schema_version".into(), json!(if authored { 1 } else { 2 }));
    for key in [
        "gap_id",
        "profile_id",
        "asset",
        "protocol_chain",
        "network",
        "source_ids",
        "return_ids",
        "source_wallet_ids",
        "destination_wallet_ids",
        "source_total_msat",
        "source_fee_msat",
        "source_debit_msat",
        "return_total_msat",
        "retained_msat",
        "residual_msat",
        "excess_msat",
        "started_at",
        "ended_at",
    ] {
        payload.insert(key.into(), candidate[key].clone());
    }
    if !authored {
        for key in [
            "promotion_eligible",
            "competitor_score_margin",
            "conflict_set_id",
            "conflict_size",
            "reason_codes",
        ] {
            payload.insert(key.into(), candidate[key].clone());
        }
    }
    codec::hash(&Value::Object(payload), true).map(|v| v.0)
}
pub fn candidate_fingerprint(candidate: &Value) -> Result<String, String> {
    fingerprint(candidate, false)
}
pub fn authored_claim_fingerprint(candidate: &Value) -> Result<String, String> {
    fingerprint(candidate, true)
}

/// Privacy-safe candidate row. Persisted readiness remains the quantity stage's
/// responsibility; this helper never declares a reviewed transfer or basis.
pub fn snapshot_gap(candidate: &Value, rows: &[Value]) -> Result<Value, String> {
    use super::normalize;
    let mut affected = 0;
    let mut years = BTreeSet::new();
    let ended =
        crate::components::time::instant(&candidate["ended_at"]).map_err(|e| e.code.to_owned())?;
    for row in rows {
        if let Some(leg) = normalize::leg(row, &BTreeSet::new())? {
            if leg.direction == "outbound"
                && array(&candidate["destination_wallet_ids"]).contains(&json!(leg.wallet_id))
                && ended.is_some_and(|ended| leg.when > ended)
            {
                affected += 1;
                years.insert(normalize::iso(leg.when)[..4].parse::<u32>().unwrap());
            }
        }
    }
    let mut result = json!({"gap_id":candidate["gap_id"],"status":if candidate["conflict_size"].as_u64().unwrap_or(1)>1 {"conflicting"} else {"needs_review"},
        "source_wallet_label":array(&candidate["source_wallet_labels"]).iter().map(|v|v.as_str().unwrap()).collect::<Vec<_>>().join(" + "),
        "downstream":{"affected_disposals":affected,"affected_years":years}});
    for key in [
        "asset",
        "destination_wallet_labels",
        "source_total_msat",
        "source_fee_msat",
        "source_debit_msat",
        "return_total_msat",
        "retained_msat",
        "residual_msat",
        "excess_msat",
        "started_at",
        "ended_at",
        "confidence",
        "reason_codes",
        "promotion_eligible",
        "competitor_score_margin",
    ] {
        result[key] = candidate[key].clone();
    }
    Ok(result)
}

pub(super) fn group_key<N: Amount>(
    group: &Group,
    legs: &[Leg],
    target: &N,
) -> (N, usize, Vec<u32>) {
    (
        distance(target, &sum(group, legs)),
        group.len(),
        key(group, legs),
    )
}

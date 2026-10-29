use super::{
    candidate::{self, Candidate},
    distance, ids, normalize, seconds, total, zero, Group, Leg,
};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub min_coverage_ppm: u32,
    pub max_excess_ppm: u32,
    pub max_input_rows: usize,
    pub max_source_legs: usize,
    pub max_return_legs: usize,
    pub max_aggregate_return_legs: usize,
    pub max_source_groups: usize,
    pub max_return_pool: usize,
    pub beam_width: usize,
    pub max_return_groups_per_source: usize,
    pub max_candidates: usize,
    pub return_era_gap_seconds: i64,
    pub promotion_score_margin: i64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            min_coverage_ppm: 800_000,
            max_excess_ppm: 250_000,
            max_input_rows: 50_000,
            max_source_legs: 3,
            max_return_legs: 16,
            max_aggregate_return_legs: 256,
            max_source_groups: 256,
            max_return_pool: 512,
            beam_width: 48,
            max_return_groups_per_source: 24,
            max_candidates: 250,
            return_era_gap_seconds: 180 * 86_400,
            promotion_score_margin: 75,
        }
    }
}
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub candidates: Vec<Candidate>,
    pub accounting_candidates: Vec<Candidate>,
    pub search_complete: bool,
    pub limit_kind: Option<&'static str>,
    pub candidate_count: usize,
    pub promotion_eligible_count: usize,
    pub blocking_source_ids: BTreeSet<String>,
    pub message: Option<String>,
}
impl SearchResult {
    pub fn value(&self) -> Value {
        json!({"$type":"CustodyGapSearchResult","candidates":self.candidates.iter().map(|v|&v.value).collect::<Vec<_>>(),
            "accounting_candidates":self.accounting_candidates.iter().map(|v|&v.value).collect::<Vec<_>>(),
            "search_complete":self.search_complete,"limit_kind":self.limit_kind,"candidate_count":self.candidate_count,
            "promotion_eligible_count":self.promotion_eligible_count,"blocking_source_ids":self.blocking_source_ids,"message":self.message})
    }
}

fn chronological(group: &mut [usize], legs: &[Leg]) {
    group.sort_by(|&a, &b| (legs[a].when, &legs[a].id).cmp(&(legs[b].when, &legs[b].id)));
}
fn bounds(target: &BigInt, limits: &Limits) -> (BigInt, BigInt) {
    (
        target * limits.min_coverage_ppm / 1_000_000,
        target + target * limits.max_excess_ppm / 1_000_000,
    )
}
fn block(group: &[usize], legs: &[Leg], blocked: &mut BTreeSet<String>) {
    blocked.extend(
        group
            .iter()
            .filter(|&&i| !legs[i].signals.is_empty() && legs[i].disqualifiers.is_empty())
            .map(|&i| legs[i].id.clone()),
    );
}

pub fn search(
    rows: &[Value],
    ignored: &BTreeSet<String>,
    limits: &Limits,
) -> Result<SearchResult, String> {
    if !(1..=1_000_000).contains(&limits.min_coverage_ppm)
        || limits.return_era_gap_seconds < 1
        || limits.promotion_score_margin < 0
        || [
            limits.max_input_rows,
            limits.max_source_legs,
            limits.max_return_legs,
            limits.max_aggregate_return_legs,
            limits.max_source_groups,
            limits.max_return_pool,
            limits.beam_width,
            limits.max_return_groups_per_source,
            limits.max_candidates,
        ]
        .contains(&0)
    {
        return Err("ValueError".into());
    }
    let mut legs = Vec::new();
    for row in rows {
        if let Some(leg) = normalize::leg(row, ignored)? {
            legs.push(leg);
        }
    }
    let worklist_limited = legs.len() > limits.max_input_rows;
    let mut limited_ids = BTreeSet::new();
    let mut capacity = worklist_limited;
    let mut blocked = BTreeSet::new();
    let mut scopes = BTreeMap::new();
    for (i, leg) in legs.iter().enumerate() {
        scopes
            .entry((&leg.profile_id, &leg.asset, &leg.chain, &leg.network))
            .or_insert_with(Vec::new)
            .push(i);
    }
    let mut generated = BTreeMap::new();
    for mut scoped in scopes.into_values() {
        chronological(&mut scoped, &legs);
        let mut sources: Vec<_> = scoped
            .iter()
            .copied()
            .filter(|&i| legs[i].direction == "outbound")
            .collect();
        let returns: Vec<_> = scoped
            .iter()
            .copied()
            .filter(|&i| legs[i].direction == "inbound")
            .collect();
        if sources.is_empty() || returns.is_empty() {
            continue;
        }
        // Sum only nonzero terms; a public large max_source_legs must not make
        // capacity admission loop beyond the actual source population.
        let source_group_count: usize = (1..=limits.max_source_legs.min(sources.len()))
            .map(|size| sources.len() - size + 1)
            .sum();
        let source_limited = source_group_count > limits.max_source_groups;
        if worklist_limited || source_limited {
            let structured: Vec<_> = sources
                .iter()
                .copied()
                .filter(|&i| !legs[i].signals.is_empty())
                .collect();
            block(&structured, &legs, &mut blocked);
            let mut selected: BTreeSet<_> = structured
                .into_iter()
                .take(limits.max_source_groups)
                .collect();
            let budget = limits.max_source_groups - selected.len();
            let ordinary: Vec<_> = sources
                .iter()
                .copied()
                .filter(|&i| legs[i].signals.is_empty())
                .collect();
            selected.extend(ordinary.iter().rev().take(budget).copied());
            sources.retain(|i| selected.contains(i));
            capacity = true;
            if sources.is_empty() {
                continue;
            }
        }
        let mut by_amount = returns.clone();
        if worklist_limited {
            by_amount.sort_by(|&a, &b| {
                (&legs[a].principal, legs[a].when, &legs[a].id).cmp(&(
                    &legs[b].principal,
                    legs[b].when,
                    &legs[b].id,
                ))
            });
        }
        let eras = if worklist_limited {
            return_eras(&returns, &legs, limits.return_era_gap_seconds)
        } else {
            Vec::new()
        };
        let max_source_legs = if worklist_limited || source_limited {
            1
        } else {
            limits.max_source_legs
        };
        'sources: for end in 0..sources.len() {
            for size in 1..=max_source_legs.min(end + 1) {
                let source_group = sources[end + 1 - size..=end].to_vec();
                let boundary = source_group.iter().map(|&i| legs[i].when).max().unwrap();
                let target = total(&source_group, &legs);
                // CPython uses bisect_right((boundary, "\uffff")), including
                // a same-time astral id even though ordinary same-time ids skip.
                let first = returns.partition_point(|&i| {
                    (legs[i].when, legs[i].id.as_str()) <= (boundary, "\u{ffff}")
                });
                if first >= returns.len() {
                    continue;
                }
                let eligible = &returns[first..];
                let pool_limited = worklist_limited || eligible.len() > limits.max_return_pool;
                let pool = if worklist_limited {
                    indexed_pool(&by_amount, &legs, boundary, &target, limits.max_return_pool)
                } else {
                    if pool_limited {
                        capacity = true;
                        block(&source_group, &legs, &mut blocked);
                    }
                    bounded_pool(eligible, &legs, &target, limits.max_return_pool)
                };
                let mut groups = return_groups(&pool, &legs, &target, limits);
                let (aggregates, aggregate_limited) = if worklist_limited {
                    matching_eras(&eras, &legs, boundary, &target, limits)
                } else {
                    wallet_eras(eligible, &legs, &target, limits)
                };
                if aggregate_limited {
                    capacity = true;
                    block(&source_group, &legs, &mut blocked);
                }
                groups.extend(aggregates);
                let deduped: BTreeMap<_, _> =
                    groups.into_iter().map(|g| (ids(&g, &legs), g)).collect();
                for group in deduped.into_values() {
                    let candidate = candidate::build(&source_group, &group, &legs);
                    if candidate.score < 650 {
                        continue;
                    }
                    if worklist_limited || source_limited || pool_limited || aggregate_limited {
                        limited_ids.insert(candidate.id.clone());
                    }
                    generated.insert(candidate.id.clone(), candidate);
                }
                if source_limited && generated.len() > limits.max_candidates {
                    break 'sources;
                }
            }
        }
    }
    let mut candidates = candidate::stamp(
        generated.into_values().collect(),
        &limited_ids,
        limits.promotion_score_margin,
    );
    let count = candidates.len();
    let accounting: Vec<_> = candidates.iter().filter(|c| c.eligible).cloned().collect();
    let promotion_count = accounting.len();
    let (complete, kind, message) = if count > limits.max_candidates {
        candidates.truncate(limits.max_candidates);
        (false,Some("candidate_population"),Some(format!("custody-gap search generated {count} candidates, including {promotion_count} promotion-eligible candidates; configured maximum is {}",limits.max_candidates)))
    } else if capacity || !limited_ids.is_empty() {
        (false,Some("boundary_worklist"),Some("custody-gap search used a bounded source/return worklist; structured boundaries remain reviewable but discovery is incomplete".into()))
    } else {
        (true, None, None)
    };
    Ok(SearchResult {
        candidates,
        accounting_candidates: accounting,
        search_complete: complete,
        limit_kind: kind,
        candidate_count: count,
        promotion_eligible_count: promotion_count,
        blocking_source_ids: blocked,
        message,
    })
}

fn bounded_pool(rows: &[usize], legs: &[Leg], target: &BigInt, maximum: usize) -> Group {
    let mut ordered = rows.to_vec();
    chronological(&mut ordered, legs);
    if ordered.len() <= maximum {
        return ordered;
    }
    ordered.sort_by_key(|&i| {
        (
            distance(target, &legs[i].principal),
            legs[i].when,
            legs[i].id.clone(),
        )
    });
    ordered.truncate(maximum);
    chronological(&mut ordered, legs);
    ordered
}
fn indexed_pool(
    rows: &[usize],
    legs: &[Leg],
    boundary: i64,
    target: &BigInt,
    maximum: usize,
) -> Group {
    let pivot = rows.partition_point(|&i| legs[i].principal < *target);
    let mut left = pivot;
    let mut right = pivot;
    let mut result = Vec::new();
    let mut inspected = 0;
    while result.len() < maximum && inspected < maximum.saturating_mul(8) {
        if left == 0 && right >= rows.len() {
            break;
        }
        let take_right = right < rows.len()
            && (left == 0
                || distance(target, &legs[rows[right]].principal)
                    < distance(target, &legs[rows[left - 1]].principal));
        let leg = if take_right {
            let i = rows[right];
            right += 1;
            i
        } else {
            left -= 1;
            rows[left]
        };
        inspected += 1;
        if legs[leg].when > boundary {
            result.push(leg);
        }
    }
    chronological(&mut result, legs);
    result
}

struct Era {
    group: Group,
    total: BigInt,
}
fn eras_by_wallet(rows: &[usize], legs: &[Leg], gap: i64) -> Vec<Group> {
    let mut wallets = BTreeMap::new();
    for &i in rows {
        wallets
            .entry(&legs[i].wallet_id)
            .or_insert_with(Vec::new)
            .push(i);
    }
    let mut result = Vec::new();
    for mut ordered in wallets.into_values() {
        chronological(&mut ordered, legs);
        let mut current: Group = Vec::new();
        for i in ordered {
            if current
                .last()
                .is_some_and(|&last| seconds(legs[i].when - legs[last].when) > gap)
            {
                result.push(std::mem::take(&mut current));
            }
            current.push(i);
        }
        if !current.is_empty() {
            result.push(current);
        }
    }
    result
}
fn return_eras(rows: &[usize], legs: &[Leg], gap: i64) -> Vec<Era> {
    let mut eras: Vec<_> = eras_by_wallet(rows, legs, gap)
        .into_iter()
        .map(|group| Era {
            total: total(&group, legs),
            group,
        })
        .collect();
    eras.sort_by_key(|era| {
        (
            era.total.clone(),
            era.group.iter().map(|&i| legs[i].when).max().unwrap(),
            ids(&era.group, legs),
        )
    });
    eras
}
fn matching_eras(
    eras: &[Era],
    legs: &[Leg],
    boundary: i64,
    target: &BigInt,
    limits: &Limits,
) -> (Vec<Group>, bool) {
    let (minimum, maximum) = bounds(target, limits);
    let start = eras.partition_point(|era| era.total < minimum);
    let end = eras.partition_point(|era| era.total <= maximum);
    let mut groups = Vec::new();
    // Python's hard inspection bound is DEFAULT_MAX_RETURN_POOL, independently
    // of the configurable beam pool maximum.
    for era in eras[start..end].iter().take(512) {
        if legs[era.group[0]].when <= boundary {
            continue;
        }
        if era.group.len() > limits.max_aggregate_return_legs {
            return (Vec::new(), true);
        }
        groups.push(era.group.clone());
    }
    rank_groups(
        &mut groups,
        legs,
        target,
        limits.max_return_groups_per_source,
    );
    (groups, false)
}
fn wallet_eras(
    rows: &[usize],
    legs: &[Leg],
    target: &BigInt,
    limits: &Limits,
) -> (Vec<Group>, bool) {
    let (minimum, maximum) = bounds(target, limits);
    let mut groups = Vec::new();
    for group in eras_by_wallet(rows, legs, limits.return_era_gap_seconds) {
        let amount = total(&group, legs);
        if amount < minimum || amount > maximum {
            continue;
        }
        if group.len() > limits.max_aggregate_return_legs {
            return (Vec::new(), true);
        }
        groups.push(group);
    }
    rank_groups(
        &mut groups,
        legs,
        target,
        limits.max_return_groups_per_source,
    );
    (groups, false)
}
fn rank_groups(groups: &mut Vec<Group>, legs: &[Leg], target: &BigInt, maximum: usize) {
    groups.sort_by_key(|g| candidate::group_key(g, legs, target));
    groups.truncate(maximum);
}
fn return_groups(rows: &[usize], legs: &[Leg], target: &BigInt, limits: &Limits) -> Vec<Group> {
    let (minimum, maximum) = bounds(target, limits);
    let mut ordered = rows.to_vec();
    chronological(&mut ordered, legs);
    let mut states: Vec<(Group, BigInt)> = vec![(Vec::new(), zero())];
    let mut qualifying = BTreeMap::new();
    for row in ordered {
        let mut additions = Vec::new();
        for (group, total) in &states {
            if group.len() >= limits.max_return_legs {
                continue;
            }
            let next_total = total + &legs[row].principal;
            if next_total > maximum {
                continue;
            }
            let mut next = group.clone();
            next.push(row);
            if next_total >= minimum {
                qualifying.insert(ids(&next, legs), next.clone());
            }
            additions.push((next, next_total));
        }
        states.extend(additions);
        let mut deduped = BTreeMap::new();
        for (group, total) in states {
            deduped.insert(ids(&group, legs), (group, total));
        }
        states = deduped.into_values().collect();
        states.sort_by_key(|(group, total)| {
            (
                usize::from(!group.is_empty()),
                distance(target, total),
                group.len(),
                ids(group, legs),
            )
        });
        states.truncate(limits.beam_width);
    }
    let mut groups: Vec<_> = qualifying.into_values().collect();
    rank_groups(
        &mut groups,
        legs,
        target,
        limits.max_return_groups_per_source,
    );
    groups
}

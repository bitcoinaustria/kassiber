use super::{
    candidate::{self, Candidate},
    distance, key, normalize, seconds, sum, Amount, Group, Leg,
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
    /// `value`, moving the candidates' objects.
    pub fn into_value(self) -> Value {
        let mut result = serde_json::Map::new();
        result.insert("$type".into(), json!("CustodyGapSearchResult"));
        result.insert(
            "candidates".into(),
            Value::Array(self.candidates.into_iter().map(|v| v.value).collect()),
        );
        result.insert(
            "accounting_candidates".into(),
            Value::Array(
                self.accounting_candidates
                    .into_iter()
                    .map(|v| v.value)
                    .collect(),
            ),
        );
        result.insert("search_complete".into(), json!(self.search_complete));
        result.insert("limit_kind".into(), json!(self.limit_kind));
        result.insert("candidate_count".into(), json!(self.candidate_count));
        result.insert(
            "promotion_eligible_count".into(),
            json!(self.promotion_eligible_count),
        );
        result.insert(
            "blocking_source_ids".into(),
            json!(self.blocking_source_ids),
        );
        result.insert("message".into(), json!(self.message));
        Value::Object(result)
    }
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
fn bounds<N: Amount>(target: &N, limits: &Limits) -> (N, N) {
    (
        target.scaled(limits.min_coverage_ppm),
        target.plus(&target.scaled(limits.max_excess_ppm)),
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

/// Admitted legs with dense id ranks, in row order.
fn ranked(rows: &[Value], ignored: &BTreeSet<String>) -> Result<Vec<Leg>, String> {
    let mut legs = Vec::new();
    for row in rows {
        if let Some(leg) = normalize::leg(row, ignored)? {
            legs.push(leg);
        }
    }
    rank(&mut legs);
    Ok(legs)
}

fn rank(legs: &mut [Leg]) {
    let mut sorted_ids: Vec<&str> = legs.iter().map(|leg: &Leg| leg.id.as_str()).collect();
    sorted_ids.sort_unstable();
    sorted_ids.dedup();
    let ranks: Vec<u32> = legs
        .iter()
        .map(|leg| {
            sorted_ids
                .binary_search(&leg.id.as_str())
                .expect("ranked id") as u32
        })
        .collect();
    for (leg, rank) in legs.iter_mut().zip(ranks) {
        leg.rank = rank;
    }
}

pub fn search(
    rows: &[Value],
    ignored: &BTreeSet<String>,
    limits: &Limits,
) -> Result<SearchResult, String> {
    validate_limits(limits)?;
    search_ranked(ranked(rows, ignored)?, limits)
}

fn validate_limits(limits: &Limits) -> Result<(), String> {
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
    Ok(())
}

pub(super) fn normalized(mut legs: Vec<Leg>, limits: &Limits) -> Result<SearchResult, String> {
    validate_limits(limits)?;
    rank(&mut legs);
    search_ranked(legs, limits)
}

fn search_ranked(mut legs: Vec<Leg>, limits: &Limits) -> Result<SearchResult, String> {
    let bound = BigInt::from(i64::MAX);
    if legs.len() < 1 << 20 && legs.iter().all(|leg| leg.principal <= bound) {
        for leg in &mut legs {
            leg.small = i128::try_from(&leg.principal).expect("principal within i64");
        }
        search_legs::<i128>(legs, limits)
    } else {
        search_legs::<BigInt>(legs, limits)
    }
}

/// The search over normalized, ranked legs, on exact amounts of type `N`.
fn search_legs<N: Amount>(legs: Vec<Leg>, limits: &Limits) -> Result<SearchResult, String> {
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
                let target: N = sum(&source_group, &legs);
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
                    groups.into_iter().map(|g| (key(&g, &legs), g)).collect();
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

fn bounded_pool<N: Amount>(rows: &[usize], legs: &[Leg], target: &N, maximum: usize) -> Group {
    let mut ordered = rows.to_vec();
    chronological(&mut ordered, legs);
    if ordered.len() <= maximum {
        return ordered;
    }
    ordered.sort_by_cached_key(|&i| {
        (
            distance(target, &N::of(&legs[i])),
            legs[i].when,
            legs[i].rank,
        )
    });
    ordered.truncate(maximum);
    chronological(&mut ordered, legs);
    ordered
}
fn indexed_pool<N: Amount>(
    rows: &[usize],
    legs: &[Leg],
    boundary: i64,
    target: &N,
    maximum: usize,
) -> Group {
    let pivot = rows.partition_point(|&i| N::of(&legs[i]) < *target);
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
                || distance(target, &N::of(&legs[rows[right]]))
                    < distance(target, &N::of(&legs[rows[left - 1]])));
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

struct Era<N> {
    group: Group,
    total: N,
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
fn return_eras<N: Amount>(rows: &[usize], legs: &[Leg], gap: i64) -> Vec<Era<N>> {
    let mut eras: Vec<_> = eras_by_wallet(rows, legs, gap)
        .into_iter()
        .map(|group| Era {
            total: sum::<N>(&group, legs),
            group,
        })
        .collect();
    eras.sort_by_cached_key(|era| {
        (
            era.total.clone(),
            era.group.iter().map(|&i| legs[i].when).max().unwrap(),
            key(&era.group, legs),
        )
    });
    eras
}
fn matching_eras<N: Amount>(
    eras: &[Era<N>],
    legs: &[Leg],
    boundary: i64,
    target: &N,
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
fn wallet_eras<N: Amount>(
    rows: &[usize],
    legs: &[Leg],
    target: &N,
    limits: &Limits,
) -> (Vec<Group>, bool) {
    let (minimum, maximum) = bounds(target, limits);
    let mut groups = Vec::new();
    for group in eras_by_wallet(rows, legs, limits.return_era_gap_seconds) {
        let amount: N = sum(&group, legs);
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
fn rank_groups<N: Amount>(groups: &mut Vec<Group>, legs: &[Leg], target: &N, maximum: usize) {
    groups.sort_by_cached_key(|g| candidate::group_key(g, legs, target));
    groups.truncate(maximum);
}
fn beam_priority(group: &[usize]) -> usize {
    usize::from(!group.is_empty())
}
fn return_groups<N: Amount>(
    rows: &[usize],
    legs: &[Leg],
    target: &N,
    limits: &Limits,
) -> Vec<Group> {
    let (minimum, maximum) = bounds(target, limits);
    let mut ordered = rows.to_vec();
    chronological(&mut ordered, legs);
    // Each state carries its identity key, so keys are computed once.
    let mut states: Vec<(Group, N, Vec<u32>)> = vec![(Vec::new(), N::zero(), Vec::new())];
    let mut qualifying = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for row in ordered {
        let previous_states = states.len();
        let rank = legs[row].rank;
        let amount = N::of(&legs[row]);
        for index in 0..previous_states {
            let (group, total, parent_key) = &states[index];
            if group.len() >= limits.max_return_legs {
                continue;
            }
            let next_total = total.plus(&amount);
            if next_total > maximum {
                continue;
            }
            let mut next = group.clone();
            next.push(row);
            let mut next_key = Vec::with_capacity(parent_key.len() + 1);
            next_key.extend_from_slice(parent_key);
            let at = next_key.partition_point(|old| *old <= rank);
            next_key.insert(at, rank);
            if next_total >= minimum {
                qualifying.insert(next_key.clone(), (next.clone(), next_total.clone()));
            }
            states.push((next, next_total, next_key));
        }
        if !seen.insert(legs[row].rank) {
            let mut deduped = BTreeMap::new();
            for (group, total, key) in states {
                deduped.insert(key, (group, total));
            }
            states = deduped
                .into_iter()
                .map(|(key, (group, total))| (group, total, key))
                .collect();
        }
        if N::SMALL {
            states.sort_by(|(a, total_a, key_a), (b, total_b, key_b)| {
                (beam_priority(a), distance(target, total_a), a.len(), key_a).cmp(&(
                    beam_priority(b),
                    distance(target, total_b),
                    b.len(),
                    key_b,
                ))
            });
        } else {
            states.sort_by_cached_key(|(group, total, key)| {
                (
                    beam_priority(group),
                    distance(target, total),
                    group.len(),
                    key.clone(),
                )
            });
        }
        states.truncate(limits.beam_width);
    }
    // A qualifying group's key and total are its `group_key` parts, and keys
    // are unique; select the best groups, then order them.
    let mut ranked: Vec<_> = qualifying
        .into_iter()
        .map(|(key, (group, total))| ((distance(target, &total), group.len(), key), group))
        .collect();
    let maximum = limits.max_return_groups_per_source;
    if ranked.len() > maximum && maximum > 0 {
        ranked.select_nth_unstable_by(maximum - 1, |a, b| a.0.cmp(&b.0));
    }
    ranked.truncate(maximum);
    ranked.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    ranked.into_iter().map(|(_, group)| group).collect()
}

#[cfg(test)]
mod amount_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gap_beam_unique_and_duplicate_id_python_hashes() {
        for (duplicate, expected) in [
            (
                false,
                "4d8f5ed323911b4550c336a17d498f01e3269d78ead7ef3de5e516ddc18e7508",
            ),
            (
                true,
                "6a759a7d7907b648b27449a17f31a40056baaa471e4541f4026b1222fc11285d",
            ),
        ] {
            let mut rows = vec![
                json!({"id":"out","profile_id":"p","wallet_id":"old","wallet_label":"Old","wallet_kind":"descriptor","direction":"outbound","asset":"BTC","occurred_at":"2020-01-01T00:00:00Z","amount":1_000_000,"fee":0,"raw_json":"{}","privacy_boundary":"coinjoin"}),
            ];
            for (i, amount) in [300_000, 350_000, 400_000, 450_000, 500_000, 550_000]
                .into_iter()
                .enumerate()
            {
                rows.push(json!({"id":format!("return-{}", if duplicate { i % 3 } else { i }),"profile_id":"p","wallet_id":"new","wallet_label":"New","wallet_kind":"descriptor","direction":"inbound","asset":"BTC","occurred_at":"2021-01-01T00:00:00Z","amount":amount,"fee":0,"raw_json":"{}"}));
            }
            let result = search(
                &rows,
                &BTreeSet::new(),
                &Limits {
                    beam_width: 3,
                    ..Limits::default()
                },
            )
            .unwrap();
            assert_eq!(result.clone().into_value(), result.value());
            let result = result.value();
            assert_eq!(crate::codec::sha256(&result.to_string()), expected);
        }

        let rows: Vec<_> = [
            1_400_000, 700_000, 1_300_000, 1_500_000, 700_000, 100_000, 500_000, 900_000,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, amount)| {
            json!({"id":format!("return-{}",i%3),"profile_id":"p","wallet_id":"w","wallet_kind":"descriptor","asset":"BTC","direction":"inbound","occurred_at":"2021-01-01T00:00:00Z","amount":amount,"fee":0,"raw_json":"{}"})
        })
        .collect();
        let mut legs = ranked(&rows, &BTreeSet::new()).unwrap();
        for leg in &mut legs {
            leg.small = leg.principal.to_string().parse().unwrap();
        }
        let indexes: Vec<_> = (0..legs.len()).collect();
        let limits = Limits {
            min_coverage_ppm: 900_000,
            max_excess_ppm: 100_000,
            max_return_legs: 4,
            beam_width: 5,
            max_return_groups_per_source: 20,
            ..Limits::default()
        };
        for groups in [
            return_groups(&indexes, &legs, &2_000_000_i128, &limits),
            return_groups(&indexes, &legs, &BigInt::from(2_000_000), &limits),
        ] {
            let principals: Vec<Vec<_>> = groups
                .iter()
                .map(|group| group.iter().map(|index| legs[*index].small).collect())
                .collect();
            assert_eq!(
                principals,
                vec![
                    vec![1_500_000, 500_000],
                    vec![1_500_000, 500_000, 100_000],
                    vec![500_000, 700_000, 900_000],
                    vec![500_000, 700_000, 900_000, 100_000],
                ]
            );
        }
    }

    fn cases() -> Vec<Value> {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_gap_cases.json"
        ))
        .unwrap();
        vectors["search"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case.get("error").is_none())
            .cloned()
            .collect()
    }
    fn operands(case: &Value) -> (Vec<Value>, BTreeSet<String>, Limits) {
        (
            case["rows"].as_array().unwrap().clone(),
            serde_json::from_value(case["ignored"].clone()).unwrap(),
            serde_json::from_value(case["limits"].clone()).unwrap(),
        )
    }
    /// Both amount types over the same legs; `None` when a principal exceeds i64.
    fn both(rows: &[Value], ignored: &BTreeSet<String>, limits: &Limits) -> Option<(Value, Value)> {
        let legs = ranked(rows, ignored).unwrap();
        let wide = search_legs::<BigInt>(legs.clone(), limits).unwrap().value();
        let mut narrow = legs;
        for leg in &mut narrow {
            leg.small = i64::try_from(&leg.principal).ok()?.into();
        }
        Some((wide, search_legs::<i128>(narrow, limits).unwrap().value()))
    }

    #[test]
    fn gap_amount_paths_agree_on_fixture_books() {
        let mut compared = 0;
        for case in cases() {
            let (rows, ignored, limits) = operands(&case);
            if let Some((wide, narrow)) = both(&rows, &ignored, &limits) {
                assert_eq!(wide, narrow, "{}", case["case"]);
                compared += 1;
            }
        }
        assert!(compared > 100, "{compared}");
    }

    #[test]
    fn gap_amount_paths_agree_at_the_i64_boundary() {
        let mut active = 0;
        for case in cases().iter().filter(|case| {
            !case["expected"]["candidates"]
                .as_array()
                .unwrap()
                .is_empty()
        }) {
            let (rows, ignored, limits) = operands(case);
            // Principals at and just below i64::MAX: sums and scaled bounds
            // exceed i64, and only i128 holds them exactly.
            let mut top = rows.clone();
            for (i, row) in top.iter_mut().enumerate() {
                row["amount"] = json!(i64::MAX - i as i64);
                row["fee"] = json!(0);
            }
            let (wide, narrow) = both(&top, &ignored, &limits).expect("i64 principals");
            assert_eq!(wide, narrow, "{}", case["case"]);
            active += usize::from(!wide["candidates"].as_array().unwrap().is_empty());
            // One principal past i64::MAX keeps the whole search on BigInt.
            let mut past = top;
            past[0]["amount"] = json!(u64::MAX);
            let legs = ranked(&past, &ignored).unwrap();
            assert!(
                both(&past, &ignored, &limits).is_none()
                    || legs.iter().all(|l| l.principal <= BigInt::from(i64::MAX))
            );
            assert_eq!(
                search(&past, &ignored, &limits).unwrap().value(),
                search_legs::<BigInt>(legs, &limits).unwrap().value(),
                "{}",
                case["case"]
            );
        }
        assert!(active > 20, "{active}");
    }
}

//! Conditional financial-flow partitions over captured transaction facts.
//! Counts are exact integers; value exchangeability is not ownership evidence.
use super::{field, fields, list, Text};
use kassiber_envelope::{dict, get, s, set, truthy};
use kassiber_pyjson::{BigInt, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

pub const MODEL: &str = "independent-flow-partitions-no-intrafees-v1";
const MAX_IO: usize = 128;
const MAX_PATTERNS: usize = 65_536;
const MAX_MEMO_CELLS: usize = 1_000_000;

fn integer(v: &Value) -> Option<BigInt> {
    let raw = match v {
        Value::Int(n) => n.to_string(),
        Value::Str(v) => v.as_str()?.to_owned(),
        _ => return None,
    };
    let n: BigInt = raw.parse().ok()?;
    (n >= BigInt::from(0) && raw == n.to_string()).then_some(n)
}
fn limit(v: &Value, default: usize, maximum: usize) -> usize {
    integer(v)
        .map(|n| n.min(BigInt::from(maximum)).to_string().parse().unwrap())
        .unwrap_or(default)
        .max(1)
}
fn num(n: usize) -> Value {
    Value::Int(BigInt::from(n))
}
fn strings(v: &[&str]) -> Value {
    Value::List(v.iter().map(|v| s(*v)).collect())
}

pub fn normalize_scenario(v: &Value) -> Result<Value, &'static str> {
    if v == &Value::Null {
        return Ok(dict([
            ("kind", s("independent")),
            ("protocol", s("generic")),
        ]));
    }
    if !matches!(v, Value::Dict(_))
        || fields(v).any(|(k, _)| {
            ![
                "kind",
                "protocol",
                "max_received_fee_msat",
                "max_paid_fee_msat",
            ]
            .contains(&k)
        })
    {
        return Err("invalid_scenario");
    }
    let kind = field(v, "kind").as_str().unwrap_or("");
    let protocol = get(v, "protocol")
        .unwrap_or(&s("generic"))
        .as_str()
        .unwrap_or("")
        .to_owned();
    if !["independent", "coinjoin_intrafees"].contains(&kind)
        || !["generic", "whirlpool", "joinmarket", "wabisabi"].contains(&protocol.as_str())
    {
        return Err("invalid_scenario");
    }
    let mut result = dict([("kind", s(kind)), ("protocol", s(protocol))]);
    for k in ["max_paid_fee_msat", "max_received_fee_msat"] {
        if kind == "independent" {
            if get(v, k).is_some() {
                return Err("independent_scenario_has_intrafees");
            }
        } else {
            let n = integer(field(v, k)).ok_or("invalid_intrafees")?;
            if &n % 1000 != BigInt::from(0) || n > BigInt::from(2_100_000_000_000_000_000u64) {
                return Err("invalid_intrafees");
            }
            set(&mut result, k, s(n.to_string()));
        }
    }
    Ok(result)
}

/// In-process callbacks carry no identifiers and are never serialized or stored.
#[derive(Default)]
pub struct Hooks<'a> {
    pub progress: Option<&'a mut dyn FnMut(Value)>,
    pub cancelled: Option<&'a mut dyn FnMut() -> bool>,
    #[cfg(any(test, feature = "test-harness"))]
    pub elapsed_ms: Option<&'a mut dyn FnMut() -> f64>,
    #[cfg(any(test, feature = "test-harness"))]
    pub max_memo_cells: Option<usize>,
}
#[derive(Debug)]
struct Stopped(&'static str, &'static str);
type Bounded<T> = Result<T, Stopped>;
struct Budget<'a> {
    hooks: Hooks<'a>,
    start: Instant,
    states: usize,
    cache_hits: usize,
    lower_bound: BigInt,
    phase: &'static str,
    max_states: usize,
    duration: usize,
}
impl Budget<'_> {
    fn tick(&mut self, work: usize, phase: Option<&'static str>) -> Bounded<()> {
        let before = self.states;
        self.states += work;
        if let Some(p) = phase {
            self.phase = p;
        }
        if self.states > self.max_states {
            return Err(Stopped("model_bounded", "state_budget"));
        }
        if phase.is_none() && before / 64 == self.states / 64 {
            return Ok(());
        }
        let elapsed = self.start.elapsed().as_secs_f64() * 1000.0;
        #[cfg(any(test, feature = "test-harness"))]
        let elapsed = self.hooks.elapsed_ms.as_mut().map_or(elapsed, |f| f());
        if self.hooks.cancelled.as_mut().is_some_and(|f| f()) {
            return Err(Stopped("cancelled", "cancelled"));
        }
        if elapsed >= self.duration as f64 {
            return Err(Stopped("timeout", "time_budget"));
        }
        if phase.is_some() || before / 1024 != self.states / 1024 {
            if let Some(f) = self.hooks.progress.as_mut() {
                f(dict([
                    ("phase", s(self.phase)),
                    ("states_explored", num(self.states)),
                    ("elapsed_ms", Value::Int(BigInt::from(elapsed as u64))),
                    ("cache_hits", num(self.cache_hits)),
                    (
                        "interpretation_count_lower_bound",
                        s(self.lower_bound.to_string()),
                    ),
                ]));
            }
        }
        Ok(())
    }
}

type Group = (Vec<usize>, Vec<usize>);
#[derive(Clone)]
struct Solution {
    count: BigInt,
    histogram: Vec<BigInt>,
    pairs: Vec<BigInt>,
    examples: Vec<Vec<Group>>,
}
struct Product {
    current: Vec<usize>,
    minimum: Vec<usize>,
    maximum: Vec<usize>,
    done: bool,
}
impl Product {
    fn new(maximum: &[usize], pivot: Option<usize>) -> Self {
        let mut minimum = vec![0; maximum.len()];
        if let Some(p) = pivot {
            minimum[p] = 1;
        }
        Self {
            current: minimum.clone(),
            minimum,
            maximum: maximum.to_vec(),
            done: false,
        }
    }
}
impl Iterator for Product {
    type Item = Vec<usize>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let result = self.current.clone();
        self.done = true;
        for i in (0..self.current.len()).rev() {
            if self.current[i] < self.maximum[i] {
                self.current[i] += 1;
                self.done = false;
                break;
            }
            self.current[i] = self.minimum[i];
        }
        Some(result)
    }
}
fn sum(values: &[BigInt], counts: &[usize]) -> BigInt {
    values.iter().zip(counts).map(|(v, n)| v * *n).sum()
}
fn subtract(a: &[usize], b: &[usize]) -> Vec<usize> {
    a.iter().zip(b).map(|(a, b)| a - b).collect()
}
fn comb(n: usize, k: usize) -> BigInt {
    (0..k.min(n - k)).fold(BigInt::from(1), |v, i| v * (n - i) / (i + 1))
}
fn ways(all: &[usize], group: &[usize], pivot: Option<usize>) -> BigInt {
    all.iter()
        .zip(group)
        .enumerate()
        .map(|(i, (a, b))| {
            let p = usize::from(pivot == Some(i));
            comb(a - p, b - p)
        })
        .product()
}
#[derive(Clone)]
struct Pattern {
    value: BigInt,
    counts: Vec<usize>,
    ways: BigInt,
}
type Key = (Vec<usize>, Vec<usize>, BigInt);
struct Solver<'a, 'b> {
    iv: Vec<BigInt>,
    ov: Vec<BigInt>,
    initial_i: Vec<usize>,
    initial_o: Vec<usize>,
    received: BigInt,
    paid: BigInt,
    budget: &'a mut Budget<'b>,
    memo: BTreeMap<Key, Solution>,
    output_cache: BTreeMap<Vec<usize>, Vec<Pattern>>,
    cells: usize,
}
impl Solver<'_, '_> {
    fn charge(&mut self, n: usize) -> Bounded<()> {
        self.cells += n;
        let cap = MAX_MEMO_CELLS;
        #[cfg(any(test, feature = "test-harness"))]
        let cap = self.budget.hooks.max_memo_cells.unwrap_or(cap);
        if self.cells > cap {
            return Err(Stopped("model_bounded", "memo_budget"));
        }
        Ok(())
    }
    fn output_patterns(&mut self, outs: &[usize]) -> Bounded<Vec<Pattern>> {
        if let Some(v) = self.output_cache.get(outs) {
            return Ok(v.clone());
        }
        let mut patterns = vec![];
        for counts in Product::new(outs, None) {
            self.budget.tick(self.ov.len(), None)?;
            if counts.iter().any(|n| *n > 0) {
                self.charge(self.ov.len() + 3)?;
                patterns.push(Pattern {
                    value: sum(&self.ov, &counts),
                    ways: ways(outs, &counts, None),
                    counts,
                });
            }
        }
        patterns.sort_by(|a, b| a.value.cmp(&b.value).then(a.counts.cmp(&b.counts)));
        self.output_cache.insert(outs.to_vec(), patterns.clone());
        Ok(patterns)
    }
    fn visit(&mut self, ins: &[usize], outs: &[usize], mining: BigInt) -> Bounded<Solution> {
        self.budget.tick(1, None)?;
        let ni = self.iv.len();
        let no = self.ov.len();
        let zero = BigInt::from(0);
        if ins.iter().all(|n| *n == 0) || outs.iter().all(|n| *n == 0) {
            let feasible = ins.iter().chain(outs).all(|n| *n == 0);
            return Ok(Solution {
                count: BigInt::from(u8::from(feasible)),
                histogram: if feasible {
                    vec![BigInt::from(1)]
                } else {
                    vec![]
                },
                pairs: vec![zero; ni * no],
                examples: if feasible { vec![vec![]] } else { vec![] },
            });
        }
        let key = (ins.to_vec(), outs.to_vec(), mining.clone());
        if let Some(v) = self.memo.get(&key) {
            self.budget.cache_hits += 1;
            return Ok(v.clone());
        }
        let mut histogram =
            vec![zero.clone(); ins.iter().sum::<usize>().min(outs.iter().sum()) + 1];
        self.charge(ni * no + histogram.len() + 1)?;
        let mut pairs = vec![zero.clone(); ni * no];
        let patterns = self.output_patterns(outs)?;
        let pivot = ins.iter().position(|n| *n > 0).unwrap();
        let mut examples = vec![];
        let mut total = zero.clone();
        for ig in Product::new(ins, Some(pivot)) {
            self.budget.tick(ni, None)?;
            let isum = sum(&self.iv, &ig);
            let low_value = &isum - &mining - &self.paid;
            let high_value = &isum + &self.received;
            let low = patterns.partition_point(|p| p.value < low_value);
            let high = patterns.partition_point(|p| p.value <= high_value);
            if low == high {
                continue;
            }
            let remaining_i = subtract(ins, &ig);
            let ways_i = ways(ins, &ig, Some(pivot));
            for p in &patterns[low..high] {
                self.budget.tick(no, None)?;
                let required = (&isum - &p.value - &self.paid).max(zero.clone());
                let child =
                    self.visit(&remaining_i, &subtract(outs, &p.counts), &mining - required)?;
                if child.count == zero {
                    continue;
                }
                let weight = &ways_i * &p.ways;
                total += &weight * &child.count;
                if ins == self.initial_i && outs == self.initial_o {
                    self.budget.lower_bound = self.budget.lower_bound.clone().max(total.clone());
                }
                for (g, n) in child.histogram.iter().enumerate() {
                    histogram[g + 1] += &weight * n;
                }
                self.budget.tick(ni * no, None)?;
                for (a, i) in ig.iter().enumerate() {
                    for (b, o) in p.counts.iter().enumerate() {
                        let pos = a * no + b;
                        pairs[pos] += &weight * (&child.pairs[pos] + &child.count * *i * *o);
                    }
                }
                for tail in child.examples.iter().take(3 - examples.len()) {
                    let mut example = vec![(ig.clone(), p.counts.clone())];
                    example.extend(tail.clone());
                    examples.push(example);
                }
            }
        }
        let result = Solution {
            count: total,
            histogram,
            pairs,
            examples,
        };
        self.memo.insert(key, result.clone());
        Ok(result)
    }
}

fn classes(values: &[BigInt]) -> (Vec<BigInt>, Vec<usize>) {
    let mut counts = BTreeMap::<BigInt, usize>::new();
    for v in values {
        *counts.entry(v.clone()).or_default() += 1;
    }
    counts.into_iter().unzip()
}
fn solve(
    solver: &mut Solver<'_, '_>,
    inv: &[BigInt],
    outv: &[BigInt],
    fee: BigInt,
) -> Bounded<(Solution, Vec<BigInt>, Value)> {
    let ni = solver.iv.len();
    let no = solver.ov.len();
    let mut metrics = dict([
        ("memo_entries", num(0)),
        ("value_classes_inputs", num(ni)),
        ("value_classes_outputs", num(no)),
    ]);
    if inv.len() == 1 || outv.len() == 1 {
        let pairs = solver
            .initial_i
            .iter()
            .flat_map(|a| solver.initial_o.iter().map(move |b| BigInt::from(a * b)))
            .collect();
        let result = Solution {
            count: BigInt::from(1),
            histogram: vec![BigInt::from(0), BigInt::from(1)],
            pairs,
            examples: vec![vec![(solver.initial_i.clone(), solver.initial_o.clone())]],
        };
        solver.budget.tick(ni * no, Some("projection"))?;
        set(&mut metrics, "exact_reduction", s("single_flow_group"));
        return Ok((result, vec![BigInt::from(1); ni * no], metrics));
    }
    for counts in [&solver.initial_i, &solver.initial_o] {
        if counts.iter().fold(1usize, |a, c| a.saturating_mul(c + 1)) > MAX_PATTERNS {
            return Err(Stopped("model_bounded", "value_pattern_budget"));
        }
    }
    solver.output_patterns(&solver.initial_o.clone())?;
    solver.budget.tick(1, Some("enumeration"))?;
    let result = solver.visit(&solver.initial_i.clone(), &solver.initial_o.clone(), fee)?;
    solver.budget.lower_bound = result.count.clone();
    solver.budget.tick(1, Some("projection"))?;
    let mut counts = vec![];
    for a in 0..ni {
        for b in 0..no {
            solver.budget.tick(1, None)?;
            let denominator = solver.initial_i[a] * solver.initial_o[b];
            assert_eq!(
                &result.pairs[a * no + b] % denominator,
                BigInt::from(0),
                "non_integral_exchangeable_link_count"
            );
            counts.push(&result.pairs[a * no + b] / denominator);
        }
    }
    set(&mut metrics, "memo_entries", num(solver.memo.len()));
    Ok((result, counts, metrics))
}

// CPython converts integers with round-to-even before calling the platform log2.
// Large integers use a rounded 53-bit mantissa and a separate binary exponent.
fn log2(n: &BigInt) -> f64 {
    let projected = n.to_string().parse::<f64>().unwrap();
    if projected.is_finite() {
        return projected.log2();
    }
    let shift = n.bits() as usize - 53;
    let mut top = n >> shift;
    let remainder = n - (&top << shift);
    let half = BigInt::from(1) << (shift - 1);
    if remainder > half || (remainder == half && &top % 2 != BigInt::from(0)) {
        top += 1;
    }
    let mantissa = top.to_string().parse::<f64>().unwrap() / 2f64.powi(53);
    mantissa.log2() + n.bits() as f64
}

pub fn analyze_transaction_entropy(
    facts: &Value,
    chain: &str,
    max_states: &Value,
    max_duration_ms: &Value,
    scenario: &Value,
    hooks: Hooks<'_>,
) -> Value {
    let mut base = dict([
        ("model", s(MODEL)),
        ("rule_version", s("local-entropy-v2")),
        ("status", s("unsupported")),
        ("reason", Value::Null),
        ("interpretation_count", Value::Null),
        ("interpretation_count_lower_bound", s("0")),
        ("entropy_bits", Value::Null),
        ("link_counts", Value::List(vec![])),
        ("deterministic_links", Value::List(vec![])),
        ("states_explored", num(0)),
        (
            "assumptions",
            strings(&[
                "complete_bitcoin_values",
                "independent_financial_flow_groups",
                "nonnegative_mining_fee_per_group",
                "no_intergroup_payments_or_intrafees",
                "every_group_has_inputs_and_outputs",
                "no_ownership_constraints",
                "uniform_interpretations_for_log2_only",
            ]),
        ),
        (
            "limitations",
            strings(&[
                "not_wallet_ownership_probability",
                "not_satoshi_flow",
                "not_composable_across_transactions",
                "unmarked_joint_payments_may_violate_model",
                "protocol_name_is_context_not_detection",
            ]),
        ),
    ]);
    let refuse = |mut base: Value, reason: &str| {
        set(&mut base, "reason", s(reason));
        base
    };
    let normalized = match normalize_scenario(scenario) {
        Ok(v) => v,
        Err(e) => return refuse(base, e),
    };
    let intrafees = field(&normalized, "kind") == &s("coinjoin_intrafees");
    set(&mut base, "scenario", normalized.clone());
    if intrafees {
        set(
            &mut base,
            "model",
            s("financial-flow-partitions-bounded-intrafees-v1"),
        );
        set(
            &mut base,
            "assumptions",
            strings(&[
                "complete_bitcoin_values",
                "bounded_net_participant_fees",
                "globally_conserved_nonnegative_mining_fees",
                "partitions_counted_once_regardless_of_fee_allocations",
                "every_group_has_inputs_and_outputs",
                "no_ownership_constraints",
                "uniform_interpretations_for_log2_only",
            ]),
        );
        let mut limitations = list(field(&base, "limitations")).to_vec();
        limitations.extend(
            list(&strings(&[
                "not_full_protocol_validation",
                "multiple_fee_payers_allowed",
                "not_payment_in_coinjoin_model",
                "fee_only_participants_not_separately_modeled",
            ]))
            .to_vec(),
        );
        set(&mut base, "limitations", Value::List(limitations));
    }
    if chain != "bitcoin" {
        return refuse(base, "bitcoin_only");
    }
    let collaboration = field(facts, "collaboration");
    if truthy(collaboration)
        && (!matches!(collaboration, Value::Dict(_))
            || field(collaboration, "kind") != &s("coinjoin"))
    {
        return refuse(base, "joint_payment_or_unknown_collaboration");
    }
    if field(facts, "complete") != &Value::Bool(true) {
        return refuse(base, "incomplete_transaction");
    }
    let (inputs, outputs) = (field(facts, "inputs"), field(facts, "outputs"));
    if !matches!(inputs, Value::List(_))
        || !matches!(outputs, Value::List(_))
        || list(inputs).is_empty()
        || list(outputs).is_empty()
    {
        return refuse(base, "missing_inputs_or_outputs");
    }
    let (inputs, outputs) = (list(inputs), list(outputs));
    if inputs.len() > MAX_IO || outputs.len() > MAX_IO {
        set(&mut base, "status", s("model_bounded"));
        set(
            &mut base,
            "limits",
            dict([("max_inputs", num(MAX_IO)), ("max_outputs", num(MAX_IO))]),
        );
        return refuse(base, "input_output_limit");
    }
    if inputs
        .iter()
        .chain(outputs)
        .any(|r| !matches!(r, Value::Dict(_)))
    {
        return refuse(base, "invalid_input_output");
    }
    fn ids(rows: &[Value]) -> Option<Vec<&str>> {
        rows.iter()
            .map(|r| field(r, "output_id").as_str().filter(|v| !v.is_empty()))
            .collect::<Option<Vec<_>>>()
    }
    let (Some(in_ids), Some(out_ids)) = (ids(inputs), ids(outputs)) else {
        return refuse(base, "invalid_or_duplicate_output_identity");
    };
    let unique: BTreeSet<_> = in_ids.iter().chain(&out_ids).collect();
    if unique.len() != in_ids.len() + out_ids.len() {
        return refuse(base, "invalid_or_duplicate_output_identity");
    }
    let amounts = |rows: &[Value]| {
        rows.iter()
            .map(|r| integer(field(r, "amount_msat")))
            .collect::<Option<Vec<_>>>()
    };
    let (Some(inv), Some(outv)) = (amounts(inputs), amounts(outputs)) else {
        return refuse(base, "unknown_amount");
    };
    if inv.iter().chain(&outv).any(|v| v % 1000 != BigInt::from(0)) {
        return refuse(base, "non_integral_bitcoin_satoshi");
    }
    if inv.iter().chain(&outv).any(|v| *v == BigInt::from(0)) {
        return refuse(base, "zero_value_output_model_unsupported");
    }
    let fee = inv.iter().sum::<BigInt>() - outv.iter().sum::<BigInt>();
    if fee < BigInt::from(0)
        || (field(facts, "fee_msat") != &Value::Null
            && integer(field(facts, "fee_msat")) != Some(fee.clone()))
    {
        return refuse(base, "inconsistent_amounts_or_fee");
    }
    let states = limit(max_states, 200_000, 2_000_000);
    let duration = limit(max_duration_ms, 1000, 30_000);
    set(
        &mut base,
        "limits",
        dict([
            ("max_states", num(states)),
            ("max_duration_ms", num(duration)),
            ("max_inputs", num(MAX_IO)),
            ("max_outputs", num(MAX_IO)),
            ("max_value_patterns", num(MAX_PATTERNS)),
            ("max_memo_cells", num(MAX_MEMO_CELLS)),
        ]),
    );
    #[cfg(any(test, feature = "test-harness"))]
    if let Some(cap) = hooks.max_memo_cells {
        let mut limits = field(&base, "limits").clone();
        set(&mut limits, "max_memo_cells", num(cap));
        set(&mut base, "limits", limits);
    }
    let mut budget = Budget {
        hooks,
        start: Instant::now(),
        states: 0,
        cache_hits: 0,
        lower_bound: BigInt::from(1),
        phase: "preparation",
        max_states: states,
        duration,
    };
    let inv: Vec<_> = inv.iter().map(|v| v / 1000).collect();
    let outv: Vec<_> = outv.iter().map(|v| v / 1000).collect();
    let (iv, initial_i) = classes(&inv);
    let (ov, initial_o) = classes(&outv);
    let mut solver = Solver {
        iv,
        ov,
        initial_i,
        initial_o,
        received: if intrafees {
            integer(field(&normalized, "max_received_fee_msat")).unwrap() / 1000
        } else {
            BigInt::from(0)
        },
        paid: if intrafees {
            integer(field(&normalized, "max_paid_fee_msat")).unwrap() / 1000
        } else {
            BigInt::from(0)
        },
        budget: &mut budget,
        memo: BTreeMap::new(),
        output_cache: BTreeMap::new(),
        cells: 0,
    };
    let compute = |solver: &mut Solver<'_, '_>| -> Bounded<Value> {
        solver.budget.tick(1, Some("preparation"))?;
        let (result, counts, metrics) = solve(solver, &inv, &outv, &fee / 1000)?;
        let mut matrix = vec![];
        for (i, input) in in_ids.iter().enumerate() {
            for (j, output) in out_ids.iter().enumerate() {
                solver.budget.tick(1, None)?;
                let a = solver.iv.binary_search(&inv[i]).unwrap();
                let b = solver.ov.binary_search(&outv[j]).unwrap();
                matrix.push(dict([
                    ("input_id", s(*input)),
                    ("output_id", s(*output)),
                    (
                        "interpretation_count",
                        s(counts[a * solver.ov.len() + b].to_string()),
                    ),
                ]));
            }
        }
        let mut examples = vec![];
        for example in &result.examples {
            let mut available_i: Vec<_> = solver
                .iv
                .iter()
                .map(|v| {
                    in_ids
                        .iter()
                        .zip(&inv)
                        .filter(|(_, a)| *a == v)
                        .map(|(id, _)| *id)
                        .collect::<Vec<_>>()
                })
                .collect();
            let mut available_o: Vec<_> = solver
                .ov
                .iter()
                .map(|v| {
                    out_ids
                        .iter()
                        .zip(&outv)
                        .filter(|(_, a)| *a == v)
                        .map(|(id, _)| *id)
                        .collect::<Vec<_>>()
                })
                .collect();
            let mut groups = vec![];
            for (ig, og) in example {
                solver.budget.tick(1, None)?;
                let collect = |available: &mut [Vec<&str>], counts: &[usize]| {
                    Value::List(
                        available
                            .iter_mut()
                            .zip(counts)
                            .flat_map(|(a, n)| a.drain(..*n).map(s))
                            .collect(),
                    )
                };
                groups.push(dict([
                    ("input_ids", collect(&mut available_i, ig)),
                    ("output_ids", collect(&mut available_o, og)),
                    (
                        if intrafees {
                            "net_difference_msat"
                        } else {
                            "fee_msat"
                        },
                        s(((sum(&solver.iv, ig) - sum(&solver.ov, og)) * 1000u32).to_string()),
                    ),
                ]));
            }
            examples.push(Value::List(groups));
        }
        solver.budget.tick(1, Some("complete"))?;
        let deterministic = matrix
            .iter()
            .filter(|v| field(v, "interpretation_count") == &s(result.count.to_string()))
            .map(|v| {
                let mut v = v.clone();
                set(&mut v, "conditional_on_model", Value::Bool(true));
                v
            })
            .collect();
        let mut computation = dict([
            ("algorithm", s("value_class_partition_dp")),
            ("phase", s("complete")),
            ("cache_hits", num(solver.budget.cache_hits)),
        ]);
        for (k, v) in fields(&metrics) {
            set(&mut computation, k, v.clone());
        }
        Ok(dict([
            ("status", s("exact")),
            ("interpretation_count", s(result.count.to_string())),
            (
                "interpretation_count_lower_bound",
                s(result.count.to_string()),
            ),
            ("entropy_bits", Value::Float(log2(&result.count))),
            ("link_counts", Value::List(matrix)),
            ("deterministic_links", Value::List(deterministic)),
            ("states_explored", num(solver.budget.states)),
            ("fee_msat", s(fee.to_string())),
            (
                "participant_group_counts",
                dict(
                    result
                        .histogram
                        .iter()
                        .enumerate()
                        .filter(|(_, v)| **v != BigInt::from(0))
                        .map(|(k, v)| (k.to_string(), s(v.to_string()))),
                ),
            ),
            ("examples", Value::List(examples.clone())),
            (
                "examples_truncated",
                Value::Bool(result.count > BigInt::from(examples.len())),
            ),
            ("computation", computation),
        ]))
    };
    match compute(&mut solver) {
        Ok(v) => {
            for (k, v) in fields(&v) {
                set(&mut base, k, v.clone());
            }
        }
        Err(Stopped(status, reason)) => {
            set(&mut base, "status", s(status));
            set(&mut base, "reason", s(reason));
            set(&mut base, "states_explored", num(budget.states));
            set(
                &mut base,
                "interpretation_count_lower_bound",
                s(budget.lower_bound.to_string()),
            );
            set(
                &mut base,
                "computation",
                dict([
                    ("algorithm", s("value_class_partition_dp")),
                    ("phase", s(budget.phase)),
                    ("cache_hits", num(budget.cache_hits)),
                ]),
            );
        }
    }
    base
}

#[cfg(any(test, feature = "test-harness"))]
pub mod fixture;
#[cfg(test)]
mod tests;

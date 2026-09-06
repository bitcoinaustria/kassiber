"""Bounded exact financial-flow partitions, independently implemented in Python.

Model: https://gist.github.com/LaurentMT/e758767ca4038ac40aaf
Participant fees: https://github.com/JoinMarket-Org/joinmarket-clientserver/blob/master/docs/USAGE.md
Variable amounts: https://github.com/WalletWasabi/WabiSabi/blob/master/protocol.md

Equal amounts are exchangeable *for this model*, not evidence of equal owners.
We memoize remaining value multiplicities and weight each transition by the
number of labeled subsets it represents. A distinguished remaining input makes
unlabeled groups unique. Aggregate pair incidence divided by class cardinality
recovers each labeled input/output link count. No independent-tier assumption,
greedy change ownership or floating point arithmetic enters the enumeration.
"""
from __future__ import annotations

from bisect import bisect_left, bisect_right
from collections import Counter
from dataclasses import dataclass
from itertools import product
import math
import time
from typing import Any, Callable, Mapping

from ...errors import AppError

ENTROPY_MODEL = "independent-flow-partitions-no-intrafees-v1"
RULE_VERSION = "local-entropy-v2"
MAX_IO = 128
MAX_PATTERNS = 65_536
MAX_MEMO_CELLS = 1_000_000
MAX_STATES = 2_000_000
MAX_DURATION_MS = 30_000
_PROTOCOLS = {"generic", "whirlpool", "joinmarket", "wabisabi"}


def _integer(value: Any) -> int | None:
    if isinstance(value, bool):
        return None
    try:
        number = int(value)
    except (TypeError, ValueError, OverflowError):
        return None
    return number if number >= 0 and str(number) == str(value) else None


def _limit(value: Any, default: int, maximum: int) -> int:
    number = _integer(value)
    return min(max(number if number is not None else default, 1), maximum)


def normalize_entropy_scenario(scenario: Any = None) -> dict[str, str]:
    """Validate an explicit conditional model; protocol names never infer fees.

    In the intrafee model each group may receive/pay at most the stated amount.
    Mining fee allocations must be nonnegative and sum to the transaction fee.
    We count partitions with at least one feasible allocation, not allocations
    themselves. Multiple payers are allowed; this is not a full protocol proof.
    """
    if scenario is None:
        return {"kind": "independent", "protocol": "generic"}
    if not isinstance(scenario, Mapping) or set(scenario) - {"kind", "protocol", "max_received_fee_msat", "max_paid_fee_msat"}:
        raise ValueError("invalid_scenario")
    kind, protocol = scenario.get("kind"), scenario.get("protocol", "generic")
    if not isinstance(kind, str) or not isinstance(protocol, str) or kind not in {"independent", "coinjoin_intrafees"} or protocol not in _PROTOCOLS:
        raise ValueError("invalid_scenario")
    result = {"kind": kind, "protocol": protocol}
    keys = {"max_received_fee_msat", "max_paid_fee_msat"}
    if kind == "independent":
        if keys & set(scenario):
            raise ValueError("independent_scenario_has_intrafees")
    else:
        for key in sorted(keys):
            value = _integer(scenario.get(key))
            if value is None or value % 1000 or value > 21_000_000 * 100_000_000 * 1000:
                raise ValueError("invalid_intrafees")
            result[key] = str(value)
    return result


def normalize_scenario(scenario: Any = None) -> dict[str, str]:
    """Application validation seam for synchronous requests and job submission."""
    try:
        return normalize_entropy_scenario(scenario)
    except ValueError as error:
        raise AppError("Invalid entropy scenario", code="validation", details={"reason": str(error)}, retryable=False) from error


class _Stopped(Exception):
    def __init__(self, status: str, reason: str):
        self.status, self.reason = status, reason


class _Budget:
    def __init__(self, states: int, duration: int, progress: Callable | None, cancelled: Callable | None):
        self.max_states, self.duration = states, duration
        self.progress, self.cancelled = progress, cancelled
        self.start = time.monotonic()
        self.states = self.cache_hits = 0
        self.lower_bound = 1  # The all-in-one partition is feasible after validation.
        self.phase = "preparation"

    def tick(self, work: int = 1, *, phase: str | None = None, force: bool = False) -> None:
        before = self.states
        self.states += work
        if phase is not None:
            self.phase, force = phase, True
        if self.states > self.max_states:
            raise _Stopped("model_bounded", "state_budget")
        if not force and before // 64 == self.states // 64:
            return
        elapsed = time.monotonic() - self.start
        if self.cancelled and self.cancelled():
            raise _Stopped("cancelled", "cancelled")
        if elapsed * 1000 >= self.duration:
            raise _Stopped("timeout", "time_budget")
        if self.progress and (force or before // 1024 != self.states // 1024):
            self.progress({"phase": self.phase, "states_explored": self.states,
                           "elapsed_ms": int(elapsed * 1000), "cache_hits": self.cache_hits,
                           "interpretation_count_lower_bound": str(self.lower_bound)})


@dataclass(frozen=True)
class _Result:
    count: int
    histogram: tuple[int, ...]
    pairs: tuple[int, ...]
    examples: tuple[tuple, ...] = ()


def _solve(inv: list[int], outv: list[int], fee: int, received: int, paid: int, budget: _Budget):
    iv, ov = tuple(sorted(set(inv))), tuple(sorted(set(outv)))
    ic, oc = Counter(inv), Counter(outv)
    initial_i, initial_o = tuple(ic[v] for v in iv), tuple(oc[v] for v in ov)
    ni, no = len(iv), len(ov)
    # With one input or one output there can only be one nonempty flow group.
    # This covers large payments/batches and consolidations without subset work.
    if len(inv) == 1 or len(outv) == 1:
        pairs = tuple(a * b for a in initial_i for b in initial_o)
        result = _Result(1, (0, 1), pairs, (((initial_i, initial_o),),))
        budget.tick(ni * no, phase="projection")
        return result, {(a, b): 1 for a in iv for b in ov}, iv, ov, {
            "memo_entries": 0, "value_classes_inputs": ni, "value_classes_outputs": no,
            "exact_reduction": "single_flow_group"}
    for counts in (initial_i, initial_o):
        if math.prod(c + 1 for c in counts) > MAX_PATTERNS:
            raise _Stopped("model_bounded", "value_pattern_budget")
    memo: dict[tuple, _Result] = {}
    output_cache: dict[tuple, tuple] = {}
    cells = 0
    empty = _Result(1, (1,), (0,) * (ni * no), ((),))
    impossible = _Result(0, (), (0,) * (ni * no))

    def output_patterns(outs: tuple[int, ...]):
        nonlocal cells
        if outs in output_cache:
            return output_cache[outs]
        patterns = []
        for counts in product(*(range(c + 1) for c in outs)):
            budget.tick(no)
            if any(counts):
                cells += no + 3
                if cells > MAX_MEMO_CELLS:
                    raise _Stopped("model_bounded", "memo_budget")
                patterns.append((sum(v * c for v, c in zip(ov, counts)), counts,
                                 math.prod(math.comb(a, b) for a, b in zip(outs, counts))))
        patterns.sort()
        output_cache[outs] = ([value for value, _, _ in patterns], patterns)
        return output_cache[outs]

    output_patterns(initial_o)

    def visit(ins: tuple[int, ...], outs: tuple[int, ...], mining_budget: int) -> _Result:
        nonlocal cells
        budget.tick()
        if not any(ins):
            return impossible if any(outs) else empty
        if not any(outs):
            return impossible
        key = (ins, outs, mining_budget)
        if key in memo:
            budget.cache_hits += 1
            return memo[key]
        # Charge before descending: active ancestors count toward the memory cap.
        hist = [0] * (min(sum(ins), sum(outs)) + 1)
        cells += ni * no + len(hist) + 1
        if cells > MAX_MEMO_CELLS:
            raise _Stopped("model_bounded", "memo_budget")
        pair_counts = [0] * (ni * no)
        values, patterns = output_patterns(outs)
        examples = []
        total = 0
        pivot = next(i for i, c in enumerate(ins) if c)
        ranges = [range(1 if a == pivot else 0, count + 1) for a, count in enumerate(ins)]
        for ig in product(*ranges):
            budget.tick(ni)
            isum = sum(v * c for v, c in zip(iv, ig))
            low = bisect_left(values, isum - mining_budget - paid)
            high = bisect_right(values, isum + received)
            if low == high:
                continue
            remaining_i = tuple(a - b for a, b in zip(ins, ig))
            ways_i = math.prod(math.comb(a - (j == pivot), b - (j == pivot)) for j, (a, b) in enumerate(zip(ins, ig)))
            for index in range(low, high):
                budget.tick(no)
                osum, og, ways_o = patterns[index]
                difference = isum - osum
                # m_g >= max(0, input_g-output_g-max_paid). The sum of these
                # minima must fit the mining fee. Upper bounds m_g <= d_g+recv
                # have sum F+groups*recv >= F, so this is also sufficient.
                required_mining = max(0, difference - paid)
                remaining_o = tuple(a - b for a, b in zip(outs, og))
                child = visit(remaining_i, remaining_o, mining_budget - required_mining)
                if not child.count:
                    continue
                ways = ways_i * ways_o
                total += ways * child.count
                if ins == initial_i and outs == initial_o:
                    budget.lower_bound = max(budget.lower_bound, total)
                for groups, number in enumerate(child.histogram):
                    hist[groups + 1] += ways * number
                budget.tick(ni * no)
                for a in range(ni):
                    for b in range(no):
                        pos = a * no + b
                        pair_counts[pos] += ways * (child.pairs[pos] + child.count * ig[a] * og[b])
                if len(examples) < 3:
                    examples.extend(((ig, og), *tail) for tail in child.examples[:3 - len(examples)])
        result = _Result(total, tuple(hist), tuple(pair_counts), tuple(examples))
        memo[key] = result
        return result

    budget.tick(phase="enumeration")
    result = visit(initial_i, initial_o, fee)
    budget.lower_bound = result.count
    budget.tick(phase="projection")
    # Exchangeability recovers *labeled* pair counts, not uniform-owner priors.
    matrix = {}
    for a, value_i in enumerate(iv):
        for b, value_o in enumerate(ov):
            budget.tick()
            pair, remainder = divmod(result.pairs[a * no + b], initial_i[a] * initial_o[b])
            if remainder:
                raise AssertionError("non_integral_exchangeable_link_count")
            matrix[(value_i, value_o)] = pair
    return result, matrix, iv, ov, {"memo_entries": len(memo), "value_classes_inputs": ni, "value_classes_outputs": no}


def analyze_transaction_entropy(
    transaction_facts: Mapping[str, Any], *, chain: str = "bitcoin",
    max_states: int = 200_000, max_duration_ms: int = 1000,
    scenario: Mapping[str, Any] | None = None,
    progress: Callable[[dict[str, Any]], None] | None = None,
    cancelled: Callable[[], bool] | None = None,
) -> dict[str, Any]:
    """Return exact conditional counts or only a proved lower bound if stopped.

    Callbacks are in-process hooks, never serialized or stored. No callback
    exposes transaction identifiers. A protocol name is context, not detection.
    """
    base: dict[str, Any] = {
        "model": ENTROPY_MODEL, "rule_version": RULE_VERSION, "status": "unsupported", "reason": None,
        "interpretation_count": None, "interpretation_count_lower_bound": "0", "entropy_bits": None,
        "link_counts": [], "deterministic_links": [], "states_explored": 0,
        "assumptions": ["complete_bitcoin_values", "independent_financial_flow_groups",
                        "nonnegative_mining_fee_per_group", "no_intergroup_payments_or_intrafees",
                        "every_group_has_inputs_and_outputs", "no_ownership_constraints", "uniform_interpretations_for_log2_only"],
        "limitations": ["not_wallet_ownership_probability", "not_satoshi_flow", "not_composable_across_transactions",
                        "unmarked_joint_payments_may_violate_model", "protocol_name_is_context_not_detection"],
    }
    try:
        normalized = normalize_entropy_scenario(scenario)
    except ValueError as error:
        return dict(base, reason=str(error))
    base["scenario"] = normalized
    intrafees = normalized["kind"] == "coinjoin_intrafees"
    if intrafees:
        base["model"] = "financial-flow-partitions-bounded-intrafees-v1"
        base["assumptions"] = ["complete_bitcoin_values", "bounded_net_participant_fees",
                               "globally_conserved_nonnegative_mining_fees", "partitions_counted_once_regardless_of_fee_allocations",
                               "every_group_has_inputs_and_outputs", "no_ownership_constraints", "uniform_interpretations_for_log2_only"]
        base["limitations"].extend(["not_full_protocol_validation", "multiple_fee_payers_allowed", "not_payment_in_coinjoin_model",
                                    "fee_only_participants_not_separately_modeled"])
    if chain != "bitcoin":
        return dict(base, reason="bitcoin_only")
    collaboration = transaction_facts.get("collaboration") or {}
    if collaboration and (not isinstance(collaboration, Mapping) or collaboration.get("kind") != "coinjoin"):
        return dict(base, reason="joint_payment_or_unknown_collaboration")
    if transaction_facts.get("complete") is not True:
        return dict(base, reason="incomplete_transaction")
    inputs, outputs = transaction_facts.get("inputs", ()), transaction_facts.get("outputs", ())
    if not isinstance(inputs, (list, tuple)) or not isinstance(outputs, (list, tuple)) or not inputs or not outputs:
        return dict(base, reason="missing_inputs_or_outputs")
    if len(inputs) > MAX_IO or len(outputs) > MAX_IO:
        return dict(base, status="model_bounded", reason="input_output_limit", limits={"max_inputs": MAX_IO, "max_outputs": MAX_IO})
    if any(not isinstance(row, Mapping) for row in (*inputs, *outputs)):
        return dict(base, reason="invalid_input_output")
    in_ids, out_ids = [row.get("output_id") for row in inputs], [row.get("output_id") for row in outputs]
    if any(not isinstance(ident, str) or not ident for ident in (*in_ids, *out_ids)) or len(set(in_ids)) != len(in_ids) or len(set(out_ids)) != len(out_ids) or set(in_ids) & set(out_ids):
        return dict(base, reason="invalid_or_duplicate_output_identity")
    inv, outv = [_integer(row.get("amount_msat")) for row in inputs], [_integer(row.get("amount_msat")) for row in outputs]
    if any(value is None for value in (*inv, *outv)):
        return dict(base, reason="unknown_amount")
    if any(value % 1000 for value in (*inv, *outv)):
        return dict(base, reason="non_integral_bitcoin_satoshi")
    if any(value == 0 for value in (*inv, *outv)):
        return dict(base, reason="zero_value_output_model_unsupported")
    fee = sum(inv) - sum(outv)
    if fee < 0 or transaction_facts.get("fee_msat") is not None and _integer(transaction_facts["fee_msat"]) != fee:
        return dict(base, reason="inconsistent_amounts_or_fee")
    states, duration = _limit(max_states, 200_000, MAX_STATES), _limit(max_duration_ms, 1000, MAX_DURATION_MS)
    base["limits"] = {"max_states": states, "max_duration_ms": duration, "max_inputs": MAX_IO,
                      "max_outputs": MAX_IO, "max_value_patterns": MAX_PATTERNS, "max_memo_cells": MAX_MEMO_CELLS}
    budget = _Budget(states, duration, progress, cancelled)
    received, paid = (int(normalized[key]) // 1000 for key in ("max_received_fee_msat", "max_paid_fee_msat")) if intrafees else (0, 0)
    inv, outv, fee_sats = [v // 1000 for v in inv], [v // 1000 for v in outv], fee // 1000
    try:
        budget.tick(phase="preparation")
        result, counts, iv, ov, metrics = _solve(inv, outv, fee_sats, received, paid, budget)
        matrix = []
        for i, input_id in enumerate(in_ids):
            for j, output_id in enumerate(out_ids):
                budget.tick()
                matrix.append({"input_id": input_id, "output_id": output_id, "interpretation_count": str(counts[(inv[i], outv[j])])})
        examples = []
        for example in result.examples:
            available_i = {v: [in_ids[i] for i, amount in enumerate(inv) if v == amount] for v in iv}
            available_o = {v: [out_ids[i] for i, amount in enumerate(outv) if v == amount] for v in ov}
            groups = []
            for ig, og in example:
                budget.tick()
                ii, oo = [], []
                for values, available, counts_group, destination in ((iv, available_i, ig, ii), (ov, available_o, og, oo)):
                    for value, number in zip(values, counts_group):
                        destination.extend(available[value][:number])
                        del available[value][:number]
                delta = sum(v * c for v, c in zip(iv, ig)) - sum(v * c for v, c in zip(ov, og))
                groups.append({"input_ids": ii, "output_ids": oo, "net_difference_msat" if intrafees else "fee_msat": str(delta * 1000)})
            examples.append(groups)
        budget.tick(phase="complete")
    except _Stopped as stop:
        return dict(base, status=stop.status, reason=stop.reason, states_explored=budget.states,
                    interpretation_count_lower_bound=str(budget.lower_bound),
                    computation={"algorithm": "value_class_partition_dp", "phase": budget.phase, "cache_hits": budget.cache_hits})
    return dict(base, status="exact", interpretation_count=str(result.count), interpretation_count_lower_bound=str(result.count),
                entropy_bits=math.log2(result.count), link_counts=matrix,
                deterministic_links=[dict(row, conditional_on_model=True) for row in matrix if int(row["interpretation_count"]) == result.count],
                states_explored=budget.states, fee_msat=str(fee),
                participant_group_counts={str(k): str(v) for k, v in enumerate(result.histogram) if v},
                examples=examples, examples_truncated=result.count > len(examples),
                computation={"algorithm": "value_class_partition_dp", "phase": "complete", "cache_hits": budget.cache_hits, **metrics})

"""Independent whole-partition/fee-allocation and combinatorial oracles.

Published 5x7 vector: transaction 7d588d52d1cece7a18d663c977d6143016b5b326404bbf286bc024d5d54fcecb.
Cross-reference (no source code copied):
https://github.com/Copexit/am-i-exposed/blob/3dd81a0dcf9fb4fedd6db6871e5e74315a50531f/boltzmann-rs/tests/known_txs.rs#L118-L166
"""
from collections import Counter
from itertools import product
import math
from unittest.mock import patch

import pytest

from kassiber.core.chain_analysis.entropy import analyze_transaction_entropy, normalize_scenario
from kassiber.errors import AppError


def facts(inputs, outputs, **kwargs):
    return {"inputs": [{"output_id": f"i{i}", "amount_msat": str(v * 1000)} for i, v in enumerate(inputs)],
            "outputs": [{"output_id": f"o{i}", "amount_msat": str(v * 1000)} for i, v in enumerate(outputs)],
            "fee_msat": str((sum(inputs) - sum(outputs)) * 1000), "complete": True, **kwargs}


def scenario(received, paid, protocol="generic"):
    return {"kind": "coinjoin_intrafees", "protocol": protocol,
            "max_received_fee_msat": str(received * 1000), "max_paid_fee_msat": str(paid * 1000)}


def fee_oracle(inputs, outputs, received, paid):
    """Combined-set partitions, then brute force actual mining fee allocations.

    Neither class multiplicities, subset masks, nor the solver's min-fee
    inequality appears here. A partition counts once even if many fee
    allocations work. Deliberately expensive, limited to small independent tests.
    """
    n, m = len(inputs), len(outputs)
    count, histogram = 0, Counter()
    links = [[0] * m for _ in range(n)]

    def partitions(prefix=()):
        if len(prefix) == n + m:
            yield prefix
        else:
            for label in range(max(prefix, default=-1) + 2):
                yield from partitions((*prefix, label))

    fee = sum(inputs) - sum(outputs)
    for word in partitions():
        blocks = []
        for k in range(max(word) + 1):
            ii = [i for i in range(n) if word[i] == k]
            oo = [j for j in range(m) if word[n + j] == k]
            if not ii or not oo:
                break
            blocks.append(sum(inputs[i] for i in ii) - sum(outputs[j] for j in oo))
        else:
            for mining in product(range(fee + 1), repeat=len(blocks)):
                if sum(mining) == fee and all(-received <= d - f <= paid for d, f in zip(blocks, mining)):
                    count += 1
                    histogram[len(blocks)] += 1
                    for i in range(n):
                        for j in range(m):
                            links[i][j] += word[i] == word[n + j]
                    break
    return count, [str(v) for row in links for v in row], {str(k): str(v) for k, v in histogram.items()}


def test_bounded_participant_fees_match_independent_fee_allocation_oracle():
    for amounts in product((1, 2, 3), repeat=4):
        inputs, outputs = amounts[:2], amounts[2:]
        if not 0 <= sum(inputs) - sum(outputs) <= 2:
            continue
        for received, paid in [(0, 0), (1, 1), (2, 1), (1, 0)]:
            expected, links, histogram = fee_oracle(inputs, outputs, received, paid)
            actual = analyze_transaction_entropy(facts(inputs, outputs), scenario=scenario(received, paid))
            assert actual["status"] == "exact", (amounts, received, paid)
            assert actual["interpretation_count"] == str(expected), (amounts, received, paid)
            assert [r["interpretation_count"] for r in actual["link_counts"]] == links
            assert actual["participant_group_counts"] == histogram


@pytest.mark.parametrize("inputs,outputs,received,paid", [
    ([3, 3, 1], [1, 2, 3], 2, 0), ([4, 2, 2], [3, 3], 2, 1),
    ([5, 5], [3, 3, 3], 1, 2), ([3, 2, 1], [2, 2, 2], 1, 2),
])
def test_grouped_inputs_outputs_and_global_fee_conservation(inputs, outputs, received, paid):
    expected, links, histogram = fee_oracle(inputs, outputs, received, paid)
    actual = analyze_transaction_entropy(facts(inputs, outputs), scenario=scenario(received, paid))
    assert actual["interpretation_count"] == str(expected)
    assert [r["interpretation_count"] for r in actual["link_counts"]] == links
    assert actual["participant_group_counts"] == histogram
    # Fee windows alone permit independent groups (3->1),(3->2),(1->3)
    # in the first case, although paid=0 cannot fund a receiving participant.
    if paid == 0:
        independent = analyze_transaction_entropy(facts(inputs, outputs))
        assert actual["interpretation_count"] == independent["interpretation_count"]


def test_published_coinjoin_reference_vector_with_explicit_fee_scenario():
    inputs = [260_994_463, 98_615_817, 84_911_243, 20_112_774, 79_168_410]
    outputs = [14_868_890, 84_077_613, 84_077_613, 15_369_204, 177_252_160, 84_077_613, 84_077_613]
    f = facts(inputs, outputs, collaboration={"kind": "coinjoin"})
    baseline = analyze_transaction_entropy(f)
    expanded = analyze_transaction_entropy(f, scenario=scenario(420_388, 1_261_164, "joinmarket"))
    assert baseline["interpretation_count"] == "1"
    assert expanded["interpretation_count"] == "95"
    # Reference matrix is output-major; the engine exposes input-major pairs.
    expected = [[95, 9, 25, 11, 11], [35, 38, 46, 33, 33], [35, 38, 46, 33, 33],
                [35, 38, 46, 33, 33], [35, 38, 46, 33, 33], [9, 27, 43, 73, 73], [11, 73, 21, 27, 27]]
    # Restore original output order (the reference sorts values descending).
    sorted_i = sorted(range(len(inputs)), key=lambda i: -inputs[i])
    sorted_o = sorted(range(len(outputs)), key=lambda i: -outputs[i])
    matrix = {(r["input_id"], r["output_id"]): int(r["interpretation_count"]) for r in expanded["link_counts"]}
    for j, output_pos in enumerate(sorted_o):
        for i, input_pos in enumerate(sorted_i):
            assert matrix[(f"i{input_pos}", f"o{output_pos}")] == expected[j][i]
    assert "globally_conserved_nonnegative_mining_fees" in expanded["assumptions"]
    assert "no_intergroup_payments_or_intrafees" not in expanded["assumptions"]
    assert expanded["scenario"]["protocol"] == "joinmarket"
    assert "not_full_protocol_validation" in expanded["limitations"]


def symmetric_oracle(n):
    """Sum over integer partitions of n, independently of production's DP.

    With m_k groups of size k, labeled partitions have weight
    (n!)² / product((k!)^(2*m_k) * m_k!). Pair incidence is sum(m_k*k²).
    """
    def shapes(remaining, minimum=1):
        if not remaining:
            yield ()
        for k in range(minimum, remaining + 1):
            for tail in shapes(remaining - k, k):
                yield (k, *tail)

    count = pairs = 0
    hist = Counter()
    for shape in shapes(n):
        denominator = math.prod(math.factorial(k) ** (2 * number) * math.factorial(number) for k, number in Counter(shape).items())
        weight = math.factorial(n) ** 2 // denominator
        count += weight
        pairs += weight * sum(k * k for k in shape)
        hist[len(shape)] += weight
    return str(count), str(pairs // (n * n)), {str(k): str(v) for k, v in hist.items()}


@pytest.mark.parametrize("n", [2, 4, 9, 16, 32])
def test_exact_large_equal_value_counts_match_integer_partition_oracle(n):
    expected, pair, histogram = symmetric_oracle(n)
    actual = analyze_transaction_entropy(facts([100_100] * n, [100_000] * n))
    assert actual["status"] == "exact"
    assert actual["interpretation_count"] == expected
    assert {r["interpretation_count"] for r in actual["link_counts"]} == {pair}
    assert actual["participant_group_counts"] == histogram
    assert len(actual["link_counts"]) == n * n
    assert not actual["deterministic_links"]


def test_one_hundred_inputs_and_outputs_fit_default_budget_without_pseudo_counts():
    result = analyze_transaction_entropy(facts([100_100] * 100, [100_000] * 100))
    assert result["status"] == "exact"
    assert result["states_explored"] < 50_000
    assert int(result["interpretation_count"]) > 2 ** 200
    assert len(result["link_counts"]) == 10_000
    assert result["computation"]["memo_entries"] == 100


def test_combined_small_inputs_can_fund_larger_output_under_wabisabi_context():
    # A reduced independent counterexample to the upstream tier-eligibility
    # rule. Full routing-shape example is separately bounded, never faked exact.
    inputs, outputs = [100, 60, 60], [100, 50, 50]
    expected, links, _ = fee_oracle(inputs, outputs, 0, 0)
    result = analyze_transaction_entropy(facts(inputs, outputs), scenario={"kind": "independent", "protocol": "wabisabi"})
    assert result["interpretation_count"] == str(expected)
    assert [r["interpretation_count"] for r in result["link_counts"]] == links
    assert not any(r["input_id"] == "i0" and r["output_id"] == "o0" for r in result["deterministic_links"])
    assert int(next(r["interpretation_count"] for r in result["link_counts"] if r["input_id"] == "i1" and r["output_id"] == "o0")) > 0


def test_full_tier_detection_counterexample_remains_ambiguous_in_exact_solver():
    inputs = [100, 60, 60, 20, 20, 20, 10, 10, 10, 10, 10]
    outputs = [100, 50, 50, 20, 20, 20, 10, 10, 10, 10, 10]
    result = analyze_transaction_entropy(facts(inputs, outputs), max_states=2_000_000, max_duration_ms=5000,
                                         scenario={"kind": "independent", "protocol": "wabisabi"})
    assert result["status"] == "exact"
    assert not any(r["input_id"] == "i0" and r["output_id"] == "o0" for r in result["deterministic_links"])
    assert int(next(r["interpretation_count"] for r in result["link_counts"] if r["input_id"] == "i1" and r["output_id"] == "o0")) > 0


@pytest.mark.parametrize("inputs,outputs", [(list(range(1, 129)), [8200]), ([9000], list(range(1, 129)))])
def test_large_one_sided_batches_are_exact_without_subset_enumeration(inputs, outputs):
    result = analyze_transaction_entropy(facts(inputs, outputs))
    assert result["status"] == "exact" and result["interpretation_count"] == "1"
    assert result["computation"]["exact_reduction"] == "single_flow_group"
    assert len(result["deterministic_links"]) == 128
    assert all(r["conditional_on_model"] for r in result["deterministic_links"])


def test_intrafees_permutation_preserves_labeled_link_counts_and_examples_conserve():
    inputs, outputs = [9, 6, 4], [8, 5, 5]
    f = facts(inputs, outputs)
    options = scenario(2, 3)
    result = analyze_transaction_entropy(f, scenario=options)
    reordered = analyze_transaction_entropy({**f, "inputs": list(reversed(f["inputs"])), "outputs": list(reversed(f["outputs"]))}, scenario=options)
    as_map = lambda r: {(x["input_id"], x["output_id"]): x["interpretation_count"] for x in r["link_counts"]}
    assert as_map(result) == as_map(reordered)
    assert result["interpretation_count"] == reordered["interpretation_count"]
    for example in result["examples"]:
        assert sum(int(group["net_difference_msat"]) for group in example) == int(f["fee_msat"])
        assert sorted(x for group in example for x in group["input_ids"]) == ["i0", "i1", "i2"]
        assert sorted(x for group in example for x in group["output_ids"]) == ["o0", "o1", "o2"]


@pytest.mark.parametrize("bad", [{}, {"kind": []}, {"kind": "independent", "protocol": {}},
    {"kind": "independent", "max_paid_fee_msat": "0"}, {"kind": "coinjoin_intrafees"},
    scenario(-1, 0), {**scenario(1, 1), "max_paid_fee_msat": "0.1"}, {**scenario(1, 1), "max_paid_fee_msat": "1"},
    {**scenario(1, 1), "protocol": "unknown"}, {**scenario(1, 1), "extra": "x"},
])
def test_invalid_scenarios_fail_before_computing(bad):
    with pytest.raises(AppError, match="Invalid entropy scenario"):
        normalize_scenario(bad)
    result = analyze_transaction_entropy(facts([2], [1]), scenario=bad)
    assert result["status"] == "unsupported" and result["states_explored"] == 0


def test_protocol_context_does_not_override_payjoin_or_incomplete_facts():
    for kind in ("payjoin", "payment_in_coinjoin", "unknown"):
        result = analyze_transaction_entropy(facts([7, 5], [6, 6], collaboration={"kind": kind}), scenario=scenario(2, 2, "joinmarket"))
        assert result["reason"] == "joint_payment_or_unknown_collaboration"
        assert not result["deterministic_links"]


def test_preparation_progress_cancellation_and_limits_withhold_partial_certainty():
    updates = []
    f = facts(list(range(1, 15)), list(range(1, 15)))
    cancelled = lambda: bool(updates and updates[-1]["states_explored"] >= 1024)
    result = analyze_transaction_entropy(f, progress=updates.append, cancelled=cancelled)
    assert result["status"] == "cancelled"
    assert result["computation"]["phase"] == "preparation"
    assert result["interpretation_count"] is None and not result["link_counts"]
    assert not result["deterministic_links"] and result["entropy_bits"] is None
    assert all(set(row) == {"phase", "states_explored", "elapsed_ms", "cache_hits", "interpretation_count_lower_bound"} for row in updates)
    assert updates[0]["phase"] == "preparation"
    with patch("kassiber.core.chain_analysis.entropy.time.monotonic", side_effect=[0, 0, 2]):
        timeout = analyze_transaction_entropy(f)
    assert timeout["status"] == "timeout" and timeout["computation"]["phase"] == "preparation"
    bounded = analyze_transaction_entropy(f, max_states=100)
    assert bounded["reason"] == "state_budget"
    assert bounded["computation"]["phase"] == "preparation"


def test_progress_and_cancellation_during_enumeration_and_memory_bound():
    updates = []
    result = analyze_transaction_entropy(facts([100_100] * 100, [100_000] * 100), progress=updates.append,
                                         cancelled=lambda: bool(updates and updates[-1]["phase"] == "enumeration"))
    assert result["status"] == "cancelled"
    assert result["computation"]["phase"] == "enumeration"
    assert result["interpretation_count"] is None and result["link_counts"] == []
    with patch("kassiber.core.chain_analysis.entropy.MAX_MEMO_CELLS", 10):
        bounded = analyze_transaction_entropy(facts([1, 2, 3], [1, 2, 3]))
    assert bounded["reason"] == "memo_budget" and bounded["link_counts"] == []
    patterns = analyze_transaction_entropy(facts(list(range(1, 18)), list(range(1, 18))))
    assert patterns["reason"] == "value_pattern_budget"

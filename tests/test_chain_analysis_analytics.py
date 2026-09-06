"""Independent amount-partition oracles and adversarial local graph fixtures."""
from copy import deepcopy
from itertools import product
import math
from types import MappingProxyType
from unittest.mock import patch

import pytest

from kassiber.core.chain_analysis.analytics import analyze_index, analyze_transaction_entropy


def entropy_facts(inputs, outputs, **kwargs):
    return {"inputs": [{"output_id": f"i{i}", "amount_msat": str(value * 1000)} for i, value in enumerate(inputs)],
            "outputs": [{"output_id": f"o{i}", "amount_msat": str(value * 1000)} for i, value in enumerate(outputs)],
            "complete": True, "fee_msat": str((sum(inputs) - sum(outputs)) * 1000), "collaboration": None, **kwargs}


def independent_oracle(inputs, outputs):
    """Enumerate set partitions of the combined I+O positions, not paired masks.

    A restricted-growth word names blocks in order of first occurrence. Each
    combined partition is generated exactly once, then validated as a whole.
    This deliberately avoids production's recursive fee pruning/subset pairing.
    """
    n, m = len(inputs), len(outputs)
    count, matrix = 0, [[0] * m for _ in inputs]

    def words(prefix):
        if len(prefix) == n + m:
            yield prefix
        else:
            for group in range(max(prefix, default=-1) + 2):
                yield from words([*prefix, group])

    for word in words([]):
        valid = True
        for block in set(word):
            ii = [i for i in range(n) if word[i] == block]
            oo = [j for j in range(m) if word[n + j] == block]
            if not ii or not oo or sum(inputs[i] for i in ii) < sum(outputs[j] for j in oo):
                valid = False
                break
        if valid:
            count += 1
            for i in range(n):
                for j in range(m):
                    matrix[i][j] += word[i] == word[n + j]
    return count, [str(value) for row in matrix for value in row]


@pytest.mark.parametrize("inputs,outputs,expected", [([10], [6, 3], 1), ([10, 20], [9, 19], 2), ([10, 10], [9, 9], 3), ([10, 10, 10], [10, 10, 10], 16)])
def test_published_model_small_interpretations(inputs, outputs, expected):
    result = analyze_transaction_entropy(entropy_facts(inputs, outputs))
    count, links = independent_oracle(inputs, outputs)
    assert result["status"] == "exact"
    assert count == expected == int(result["interpretation_count"])
    assert result["entropy_bits"] == math.log2(expected)
    assert [row["interpretation_count"] for row in result["link_counts"]] == links
    assert all(row["conditional_on_model"] for row in result["deterministic_links"])


def test_all_small_unequal_amount_cases_match_independent_oracle():
    for values in product((1, 2, 3), repeat=4):
        ins, outs = values[:2], values[2:]
        if sum(ins) < sum(outs):
            continue
        expected, matrix = independent_oracle(ins, outs)
        result = analyze_transaction_entropy(entropy_facts(ins, outs))
        assert result["status"] == "exact", values
        assert int(result["interpretation_count"]) == expected, values
        assert [row["interpretation_count"] for row in result["link_counts"]] == matrix, values
    # Different input/output cardinalities exercise grouped outputs and fees.
    for ins, outs in [([6, 4, 3], [7, 5]), ([11, 8], [5, 5, 7]), ([8, 8], [4, 4, 4, 3])]:
        expected, matrix = independent_oracle(ins, outs)
        result = analyze_transaction_entropy(entropy_facts(ins, outs))
        assert int(result["interpretation_count"]) == expected
        assert [row["interpretation_count"] for row in result["link_counts"]] == matrix


def test_entropy_is_permutation_invariant_and_fee_model_changes_count():
    a = analyze_transaction_entropy(entropy_facts([10, 20], [9, 19]))
    b = analyze_transaction_entropy(entropy_facts([20, 10], [19, 9]))
    assert a["interpretation_count"] == b["interpretation_count"] == "2"
    assert {tuple(sorted(row["fee_msat"] for row in example)) for example in a["examples"]} == {("1000", "1000"), ("2000",)}
    # Same two outputs, different input balances: equal-output count is not the algorithm.
    assert analyze_transaction_entropy(entropy_facts([10, 10], [9, 9]))["interpretation_count"] == "3"
    assert analyze_transaction_entropy(entropy_facts([1, 19], [9, 9]))["interpretation_count"] == "1"


@pytest.mark.parametrize("kind", ["payjoin", "collaborative", "payment_in_coinjoin", "unknown"])
def test_joint_payment_and_unknown_collaboration_do_not_publish_false_certainty(kind):
    facts = entropy_facts([7, 5], [6, 6], collaboration={"kind": kind})
    result = analyze_transaction_entropy(facts)
    assert result["status"] == "unsupported"
    assert result["interpretation_count"] is None
    assert not result["deterministic_links"]
    # The same shape has one interpretation only under the independent-flow assumption.
    facts["collaboration"] = None
    assert analyze_transaction_entropy(facts)["interpretation_count"] == "1"


def test_coinjoin_supported_conditionally_but_not_a_privacy_probability():
    facts = entropy_facts([10, 10], [9, 9], collaboration={"kind": "coinjoin"})
    result = analyze_transaction_entropy(facts)
    assert result["interpretation_count"] == "3"
    assert "no_intergroup_payments_or_intrafees" in result["assumptions"]
    assert "not_wallet_ownership_probability" in result["limitations"]


def test_entropy_fail_closed_for_unknown_nonbitcoin_conflicting_and_oversized():
    facts = entropy_facts([10, 10], [9, 9])
    assert analyze_transaction_entropy(facts, chain="liquid")["reason"] == "bitcoin_only"
    assert analyze_transaction_entropy(dict(facts, complete=False))["reason"] == "incomplete_transaction"
    assert analyze_transaction_entropy(dict(facts, fee_msat="0"))["reason"] == "inconsistent_amounts_or_fee"
    bad = deepcopy(facts)
    bad["inputs"][0]["amount_msat"] = None
    assert analyze_transaction_entropy(bad)["reason"] == "unknown_amount"
    bad["inputs"][0]["amount_msat"] = "10001"
    assert analyze_transaction_entropy(bad)["reason"] == "non_integral_bitcoin_satoshi"
    bad["inputs"][0]["amount_msat"] = "0"
    assert analyze_transaction_entropy(bad)["reason"] == "zero_value_output_model_unsupported"
    bad = deepcopy(facts)
    bad["inputs"][1]["output_id"] = "i0"
    assert analyze_transaction_entropy(bad)["reason"] == "invalid_or_duplicate_output_identity"
    large = analyze_transaction_entropy(entropy_facts([10] * 129, [9] * 129))
    assert large["status"] == "model_bounded"
    assert large["interpretation_count"] is None


def test_state_and_time_limits_never_publish_partial_deterministic_links():
    facts = entropy_facts([10] * 4, [9] * 4)
    result = analyze_transaction_entropy(facts, max_states=20)
    assert result["status"] == "model_bounded"
    assert int(result["interpretation_count_lower_bound"]) > 0
    assert result["entropy_bits"] is None and result["interpretation_count"] is None
    assert not result["link_counts"] and not result["deterministic_links"]
    with patch("kassiber.core.chain_analysis.entropy.time.monotonic", side_effect=[0, 2]):
        result = analyze_transaction_entropy(facts)
    assert result["status"] == "timeout"
    assert not result["deterministic_links"]


class Graph:
    def __init__(self):
        self.index = {"nodes": {}, "edges": {}, "subjects": {}, "transaction_facts": {}, "output_facts": {}, "labels": []}

    def output(self, name, amount=10, *, script=None, wallet=None, change=False, chain="bitcoin", network="main", status="observed"):
        if chain == "liquid" and network == "main":
            network = "liquidv1"
        ident = f"{chain}:{network}:out:{name}"
        self.index["nodes"][ident] = {"id": ident, "kind": "output", "chain": chain, "network": network, "status": status, "wallet_ids": [wallet] if wallet else [], "amount_msat": str(amount * 1000) if amount is not None else None, "evidence": [{"source": "wallet_inventory", "reference": name}]}
        self.index["output_facts"][ident] = {"script": script, "ownership_known": bool(wallet), "branch_role": "change" if change else "receive"}
        return ident

    def tx(self, name, ins, outs, *, collaboration=None, chain="bitcoin", network="main", status="observed"):
        if chain == "liquid" and network == "main":
            network = "liquidv1"
        ident = f"{chain}:{network}:tx:{name}"
        self.index["nodes"][ident] = {"id": ident, "kind": "transaction", "chain": chain, "network": network, "status": status, "evidence": [{"source": "stored_transaction", "reference": name}]}
        self.index["subjects"][name] = [ident]
        self.index["transaction_facts"][ident] = {"inputs": [{"output_id": key, "amount_msat": self.index["nodes"][key]["amount_msat"]} for key in ins], "outputs": [{"output_id": key, "amount_msat": self.index["nodes"][key]["amount_msat"]} for key in outs], "complete": True, "fee_msat": None, "collaboration": {"kind": collaboration} if collaboration else None}
        for inp in ins:
            self.edge(inp, ident, "spends")
        for out in outs:
            self.edge(ident, out, "creates")
        return ident

    def edge(self, source, target, kind):
        ident = f"{kind}:{source}:{target}"
        self.index["edges"][ident] = {"id": ident, "source": source, "target": target, "kind": kind, "status": "observed", "evidence": [{"source": "fixture", "reference": ident}]}
        return ident

    def label(self, ident, subject, **kwargs):
        node = self.index["nodes"][subject]
        self.index["labels"].append({"id": ident, "revision": 1, "subject": subject, "node_ids": [subject], "chain": node["chain"], "network": node["network"], "label": "Local Exchange", "source": "my_export", "category": "exchange", "confidence": "user_confirmed", "cluster_defining": False, **kwargs})

    def analyze(self, **query):
        return analyze_index(self.index, query, set(self.index["nodes"]))


def test_hypotheses_opt_in_reversible_and_collaboration_excluded():
    graph = Graph()
    a, b, out = graph.output("a"), graph.output("b"), graph.output("out")
    tx = graph.tx("tx", [a, b], [out])
    assert not graph.analyze()["clusters"]
    result = graph.analyze(include_hypotheses=True)
    assert result["clusters"][0]["node_ids"] == sorted([a, b])
    assert result["hypothesis_edges"][0]["reversible"]
    for kind in ("coinjoin", "payjoin", "unknown"):
        graph.index["transaction_facts"][tx]["collaboration"] = {"kind": kind}
        result = graph.analyze(include_hypotheses=True)
        assert not result["clusters"]
        assert any(row["code"] == "collaborative_boundary" for row in result["findings"])
    assert len(graph.index["edges"]) == 3  # hypotheses never persist into facts


def test_script_reuse_is_domain_scoped_and_does_not_expose_script():
    graph = Graph()
    a = graph.output("a", script="0014abcd")
    b = graph.output("b", script="0014abcd")
    graph.output("foreign", script="0014abcd", network="regtest")
    graph.output("liquid", script="0014abcd", chain="liquid")
    graph.output("op1", script="6a00")
    graph.output("op2", script="6a00")
    result = graph.analyze(include_hypotheses=True)
    assert result["clusters"][0]["node_ids"] == [a, b]
    assert len(result["clusters"]) == 1
    assert "0014abcd" not in str(result)


def test_private_change_requires_observer_and_shared_wallet_evidence():
    graph = Graph()
    a = graph.output("a", wallet="wallet")
    change = graph.output("change", wallet="wallet", change=True)
    pay = graph.output("payment")
    tx = graph.tx("tx", [a], [change, pay])
    assert not graph.analyze(observer="public", include_hypotheses=True)["clusters"]
    result = graph.analyze(observer="owner", include_hypotheses=True)
    assert result["hypothesis_edges"][0]["rule"] == "private_wallet_change"
    assert result["hypothesis_edges"][0]["observer"] == "owner"
    graph.index["transaction_facts"][tx]["collaboration"] = {"kind": "payjoin"}
    assert not graph.analyze(observer="owner", include_hypotheses=True)["clusters"]


def test_public_change_hypothesis_states_alternative_and_collaboration_guard():
    graph = Graph()
    inp = graph.output("in", script="0014a")
    change = graph.output("change", script="0014a")
    other = graph.output("other", script="0014b")
    tx = graph.tx("tx", [inp], [change, other])
    result = graph.analyze(include_hypotheses=True)
    rule = next(row for row in result["hypothesis_edges"] if row["rule"] == "change_script_return")
    assert "payment_to_same_script_remains_alternative" in rule["premises"]
    graph.index["transaction_facts"][tx]["collaboration"] = {"kind": "coinjoin"}
    assert all(row["rule"] != "change_script_return" for row in graph.analyze(include_hypotheses=True)["hypothesis_edges"])


def branching_graph(collaboration=None):
    graph = Graph()
    root_input = graph.output("root_input", 100)
    a, b = graph.output("a", 40), graph.output("b", 40)
    root = graph.tx("root", [root_input], [a, b], collaboration=collaboration)
    c, d = graph.output("c", 30), graph.output("d", 30)
    left, right = graph.tx("left", [a], [c]), graph.tx("right", [b], [d])
    e = graph.output("e", 50)
    join = graph.tx("join", [c, d], [e])
    return graph, root, left, right, join, root_input, e


def test_multi_transaction_fan_and_reconvergence_patterns_have_exact_paths():
    graph, root, left, right, join, _, _ = branching_graph()
    result = graph.analyze(depth=6)
    codes = {row["code"] for row in result["patterns"]}
    assert {"fan_in", "fan_out", "reconvergence"} <= codes
    pattern = next(row for row in result["patterns"] if row["code"] == "reconvergence" and root in row["node_ids"])
    assert {left, right, join} <= set(pattern["node_ids"])
    assert len(pattern["edge_ids"]) == 8
    assert all(ident in graph.index["edges"] for ident in pattern["edge_ids"])
    assert not result["clusters"]


def test_postmix_reconvergence_preserves_physical_facts_without_mix_clustering():
    graph, root, _, _, _, _, _ = branching_graph("coinjoin")
    result = graph.analyze(depth=6, include_hypotheses=True)
    assert any(row["code"] == "postmix_reconvergence" and root in row["node_ids"] for row in result["patterns"])
    assert all(edge["rule"] != "common_input_control" or not set(edge["evidence"][0].values()) & {"root"} for edge in result["hypothesis_edges"])


def test_stale_or_conflicting_branch_cannot_prove_reconvergence():
    graph, _, left, _, _, _, _ = branching_graph()
    for status in ("stale", "conflicting"):
        graph.index["nodes"][left]["status"] = status
        assert all(row["code"] != "reconvergence" for row in graph.analyze(depth=6)["patterns"])


def test_selected_edge_frontier_does_not_restore_pruned_connectivity():
    graph, _, _, _, _, _, _ = branching_graph()
    result = graph.analyze(depth=6, _selected_edge_ids=[])
    assert not result["patterns"]


def test_peel_candidate_needs_three_amount_backed_ordinary_steps():
    graph = Graph()
    previous = graph.output("start", 100)
    txids = []
    for i in range(4):
        remainder = graph.output(f"change{i}", 90 - i * 10)
        payment = graph.output(f"pay{i}", 9)
        txids.append(graph.tx(f"peel{i}", [previous], [remainder, payment]))
        previous = remainder
    result = graph.analyze(depth=6, include_hypotheses=True)
    candidate = next(row for row in result["patterns"] if row["code"] == "peel_chain_candidate")
    assert len(candidate["node_ids"]) >= 3
    assert candidate["evidence_level"] == "heuristic"
    graph.index["transaction_facts"][txids[1]]["collaboration"] = {"kind": "payjoin"}
    assert all(row["code"] != "peel_chain_candidate" for row in graph.analyze(depth=6, include_hypotheses=True)["patterns"])


def test_label_exposure_direct_and_indirect_is_claim_plus_path_not_taint():
    graph, root, _, _, join, _, _ = branching_graph()
    graph.label("label1", root, category="mixer")
    direct = graph.analyze(observer="owner", subject="root")
    assert direct["exposure"][0]["kind"] == "direct_label_match"
    indirect = graph.analyze(observer="owner", subject="join", depth=6)
    assert indirect["exposure"][0]["kind"] == "indirect_topological_contact"
    assert indirect["exposure"][0]["node_ids"][0] == root
    assert indirect["exposure"][0]["node_ids"][-1] == join
    assert indirect["exposure"][0]["claim"]["category"] == "mixer"
    assert not indirect["exposure"][0]["taint_inference"]
    assert not graph.analyze(observer="public", subject="join")["exposure"]


def test_label_cluster_is_explicit_reversible_and_network_scoped():
    graph = Graph()
    a, b = graph.output("a"), graph.output("b")
    foreign = graph.output("foreign", network="regtest")
    for ident, subject in (("1", a), ("2", b), ("3", foreign)):
        graph.label(ident, subject)
    assert not graph.analyze(observer="owner", include_hypotheses=True)["clusters"]
    for row in graph.index["labels"]:
        row["cluster_defining"] = True
    assert graph.analyze(observer="owner", include_hypotheses=True)["clusters"][0]["node_ids"] == [a, b]
    assert not graph.analyze(observer="public", include_hypotheses=True)["clusters"]
    graph.index["labels"][1]["deleted"] = True
    assert not graph.analyze(observer="owner", include_hypotheses=True)["clusters"]


def test_cross_rail_exposure_uses_typed_custody_evidence_only_for_owner():
    graph = Graph()
    bitcoin = graph.tx("btc", [graph.output("a")], [graph.output("b")])
    liquid = graph.tx("lbtc", [graph.output("c", chain="liquid")], [graph.output("d", chain="liquid")], chain="liquid")
    relation = graph.edge(bitcoin, liquid, "custody")
    graph.label("exchange", bitcoin)
    result = graph.analyze(observer="owner", subject="lbtc")
    assert result["exposure"][0]["edge_ids"] == [relation]
    assert result["exposure"][0]["path_kinds"] == ["custody"]
    assert not graph.analyze(observer="owner", subject="lbtc", include_relations=False)["exposure"]
    assert not graph.analyze(observer="public", subject="lbtc")["exposure"]


def test_pure_index_analysis_accepts_immutable_nested_mappings():
    graph = Graph()
    graph.tx("tx", [graph.output("a")], [graph.output("b", 9)])
    before = deepcopy(graph.index)

    def freeze(value):
        if isinstance(value, dict):
            return MappingProxyType({k: freeze(v) for k, v in value.items()})
        if isinstance(value, list):
            return tuple(freeze(v) for v in value)
        return value

    result = analyze_index(freeze(graph.index), {"subject": "tx", "action": "entropy"}, set(graph.index["nodes"]))
    assert result["entropy"][0]["status"] == "exact"
    assert graph.index == before


def test_peel_hypothesis_is_not_enabled_by_default():
    graph = Graph()
    previous = graph.output("start", 100)
    for i in range(4):
        remainder = graph.output(f"change{i}", 90 - i * 10)
        payment = graph.output(f"pay{i}", 9)
        graph.tx(f"peel{i}", [previous], [remainder, payment])
        previous = remainder
    assert all(row["code"] != "peel_chain_candidate" for row in graph.analyze()["patterns"])


def test_hypotheses_never_reintroduce_pruned_spend_edges():
    graph = Graph()
    a, b, out = graph.output("a"), graph.output("b"), graph.output("out")
    graph.tx("tx", [a, b], [out])
    assert not graph.analyze(include_hypotheses=True, _selected_edge_ids=[])["clusters"]


def test_unknown_or_noncanonical_domain_never_clusters_labels_or_scripts():
    graph = Graph()
    for network in ("unknown", "mainnet", ""):
        graph.output(f"a-{network}", network=network, script="0014aa")
        graph.output(f"b-{network}", network=network, script="0014aa")
    assert not graph.analyze(include_hypotheses=True)["clusters"]

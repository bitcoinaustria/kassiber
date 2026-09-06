"""Pure, reversible analytics over a local observation index.

No output grants ownership, accounting, or taint authority. Entropy implements
paired partitions of independent financial flows (LaurentMT's Boltzmann model),
with no intrafees or joint payments, not probabilities of wallet ownership.
https://gist.github.com/LaurentMT/e758767ca4038ac40aaf
"""
from __future__ import annotations

from collections import defaultdict, deque
import hashlib
import json
import math
import time
from typing import Any, Mapping, Sequence

from ...wallet_descriptors import normalize_network

RULE_VERSION = "local-analytics-v1"
ENTROPY_MODEL = "independent-flow-partitions-no-intrafees-v1"
_BAD_STATUS = {"stale", "conflicting", "retracted"}


def _id(kind: str, *parts: Any) -> str:
    return f"{kind}:" + hashlib.sha256(json.dumps(parts, sort_keys=True, default=str).encode()).hexdigest()[:24]


def _get(index: Any, name: str, default: Any = None) -> Any:
    return index.get(name, default) if isinstance(index, Mapping) else getattr(index, name, default)


def _amount(value: Any) -> int | None:
    if isinstance(value, bool):
        return None
    try:
        result = int(value)
    except (TypeError, ValueError, OverflowError):
        return None
    return result if str(result) == str(value) and result >= 0 else None


def _bounded_int(value: Any, default: int, maximum: int) -> int:
    return min(max(_amount(value) if _amount(value) is not None else default, 1), maximum)


def _domain(node: Mapping) -> tuple[str, str] | None:
    chain, network = node.get("chain"), node.get("network")
    if chain == "lightning":
        return (chain, network) if network in {"main", "test", "signet", "regtest"} else None
    if chain not in {"bitcoin", "liquid"} or not isinstance(network, str):
        return None
    try:
        canonical = normalize_network(chain, network)
    except ValueError:
        return None
    return (chain, network) if canonical == network else None


def _plain(value: Any) -> Any:
    if isinstance(value, Mapping):
        return {key: _plain(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_plain(item) for item in value]
    return value


class _Limit(Exception):
    def __init__(self, status: str):
        self.status = status


def analyze_transaction_entropy(
    transaction_facts: Mapping[str, Any], *, chain: str = "bitcoin",
    max_states: int = 200_000, max_duration_ms: int = 1000,
) -> dict[str, Any]:
    """Enumerate each compatible paired partition exactly once.

    Every group has at least one input and output. Its input sum covers its
    output sum; the difference is that group's nonnegative mining fee. All
    inputs/outputs occur once, groups are unlabeled, and intergroup payments are
    excluded. Enumerating groups by the lowest unused input removes participant
    permutations. Link counts describe membership in financial-flow groups,
    never ownership or a unique path taken by individual satoshis.

    Partial searches publish only a lower bound; they never publish an entropy,
    a normalized link matrix or deterministic links. Limits include preparation.
    """
    base: dict[str, Any] = {
        "model": ENTROPY_MODEL, "rule_version": RULE_VERSION,
        "status": "unsupported", "reason": None, "interpretation_count": None,
        "interpretation_count_lower_bound": "0", "entropy_bits": None,
        "link_counts": [], "deterministic_links": [], "states_explored": 0,
        "assumptions": ["complete_bitcoin_values", "independent_financial_flow_groups",
                        "nonnegative_mining_fee_per_group", "no_intergroup_payments_or_intrafees",
                        "no_ownership_constraints", "uniform_interpretations_for_log2_only"],
        "limitations": ["not_wallet_ownership_probability", "not_satoshi_flow",
                        "not_composable_across_transactions", "unmarked_joint_payments_may_violate_model"],
    }
    if chain != "bitcoin":
        return dict(base, reason="bitcoin_only")
    collaboration = transaction_facts.get("collaboration") or {}
    if collaboration and collaboration.get("kind") != "coinjoin":
        return dict(base, reason="joint_payment_or_unknown_collaboration")
    if transaction_facts.get("complete") is not True:
        return dict(base, reason="incomplete_transaction")
    inputs, outputs = transaction_facts.get("inputs", ()), transaction_facts.get("outputs", ())
    if not isinstance(inputs, (list, tuple)) or not isinstance(outputs, (list, tuple)) or not inputs or not outputs:
        return dict(base, reason="missing_inputs_or_outputs")
    if len(inputs) > 8 or len(outputs) > 8:
        return dict(base, status="model_bounded", reason="input_output_limit", limits={"max_inputs": 8, "max_outputs": 8})
    if any(not isinstance(row, Mapping) for row in (*inputs, *outputs)):
        return dict(base, reason="invalid_input_output")
    in_ids, out_ids = [row.get("output_id") for row in inputs], [row.get("output_id") for row in outputs]
    if any(not isinstance(ident, str) or not ident for ident in (*in_ids, *out_ids)) or len(set(in_ids)) != len(in_ids) or len(set(out_ids)) != len(out_ids) or set(in_ids) & set(out_ids):
        return dict(base, reason="invalid_or_duplicate_output_identity")
    inv, outv = [_amount(row.get("amount_msat")) for row in inputs], [_amount(row.get("amount_msat")) for row in outputs]
    if any(value is None for value in (*inv, *outv)):
        return dict(base, reason="unknown_amount")
    if any(value % 1000 for value in (*inv, *outv)):
        return dict(base, reason="non_integral_bitcoin_satoshi")
    # Zero-value outputs (e.g. OP_RETURN) need an explicit attribution model.
    if any(value == 0 for value in (*inv, *outv)):
        return dict(base, reason="zero_value_output_model_unsupported")
    fee = sum(inv) - sum(outv)
    if fee < 0 or transaction_facts.get("fee_msat") is not None and _amount(transaction_facts["fee_msat"]) != fee:
        return dict(base, reason="inconsistent_amounts_or_fee")
    state_limit = _bounded_int(max_states, 200_000, 2_000_000)
    duration = _bounded_int(max_duration_ms, 1000, 5000)
    deadline = time.monotonic() + duration / 1000
    states = 0
    count = 0
    links = [[0] * len(outputs) for _ in inputs]
    group_counts: dict[int, int] = defaultdict(int)
    examples: list[list[dict]] = []

    def tick() -> None:
        nonlocal states
        states += 1
        if states > state_limit:
            raise _Limit("model_bounded")
        if time.monotonic() >= deadline:
            raise _Limit("timeout")

    def sums(values: Sequence[int]) -> list[int]:
        result = [0] * (1 << len(values))
        for mask in range(1, len(result)):
            tick()
            bit = mask & -mask
            result[mask] = result[mask ^ bit] + values[bit.bit_length() - 1]
        return result

    def visit(imask: int, omask: int, groups: list[tuple[int, int]]) -> None:
        nonlocal count
        tick()
        if not imask:
            if omask:
                return
            count += 1
            group_counts[len(groups)] += 1
            for igroup, ogroup in groups:
                for i in range(len(inputs)):
                    if igroup & (1 << i):
                        for j in range(len(outputs)):
                            if ogroup & (1 << j):
                                links[i][j] += 1
            if len(examples) < 3:
                examples.append([{"input_ids": [in_ids[i] for i in range(len(inputs)) if ig & (1 << i)],
                                  "output_ids": [out_ids[j] for j in range(len(outputs)) if og & (1 << j)],
                                  "fee_msat": str(isums[ig] - osums[og])} for ig, og in groups])
            return
        if not omask:
            return
        first = imask & -imask
        igroup = imask
        while igroup:
            if igroup & first:
                ogroup = omask
                while ogroup:
                    tick()
                    if 0 <= isums[igroup] - osums[ogroup] <= fee:
                        visit(imask ^ igroup, omask ^ ogroup, [*groups, (igroup, ogroup)])
                    ogroup = (ogroup - 1) & omask
            igroup = (igroup - 1) & imask

    try:
        isums, osums = sums(inv), sums(outv)
        visit((1 << len(inputs)) - 1, (1 << len(outputs)) - 1, [])
    except _Limit as limit:
        return dict(base, status=limit.status, reason="time_budget" if limit.status == "timeout" else "state_budget",
                    states_explored=states, interpretation_count_lower_bound=str(count),
                    limits={"max_states": state_limit, "max_duration_ms": duration})
    matrix = [{"input_id": in_ids[i], "output_id": out_ids[j], "interpretation_count": str(links[i][j])}
              for i in range(len(inputs)) for j in range(len(outputs))]
    return dict(base, status="exact", reason=None, interpretation_count=str(count),
                interpretation_count_lower_bound=str(count), entropy_bits=math.log2(count) if count else None,
                link_counts=matrix, deterministic_links=[dict(row, conditional_on_model=True) for row in matrix if count and int(row["interpretation_count"]) == count],
                states_explored=states, fee_msat=str(fee),
                participant_group_counts={str(k): str(v) for k, v in sorted(group_counts.items())},
                examples=examples, examples_truncated=count > len(examples))


def _finding(code: str, ids: Sequence[str], *, evidence: Sequence[Mapping] = (),
             detail: str, observer: str, level: str = "heuristic", edge_ids: Sequence[str] = ()) -> dict:
    return {"id": _id(code, sorted(set(ids))), "code": code, "rule_version": RULE_VERSION,
            "title": code.replace("_", " ").capitalize(), "detail": detail,
            "severity": "info", "evidence_level": level, "observer": observer,
            "node_ids": sorted(set(ids)), "edge_ids": sorted(set(edge_ids)),
            "evidence": _plain(evidence), "accounting_authority": False}


def analyze_index(index: Any, query: Mapping[str, Any], selected_node_ids: set[str]) -> dict[str, Any]:
    """Analyze only the supplied query subgraph; never mutate or rescan sources."""
    all_nodes = _get(index, "nodes", {})
    selected = set(selected_node_ids) & set(all_nodes)
    nodes = {ident: all_nodes[ident] for ident in sorted(selected)}
    edges = {ident: row for ident, row in _get(index, "edges", {}).items()
             if row.get("source") in selected and row.get("target") in selected
             and ("_selected_edge_ids" not in query or ident in query["_selected_edge_ids"])}
    txfacts = _get(index, "transaction_facts", {})
    outfacts = _get(index, "output_facts", {})
    observer = query.get("observer", "public")
    if observer not in {"public", "owner", "disclosed"}:
        observer = "public"
    allow_hypotheses = query.get("include_hypotheses") is True
    result: dict[str, Any] = {"rule_version": RULE_VERSION, "findings": [], "clusters": [],
                             "patterns": [], "exposure": [], "entropy": [], "hypothesis_edges": [],
                             "coverage": {"selected_node_count": len(nodes), "hypotheses_enabled": allow_hypotheses,
                                          "observer": observer, "topology_complete": False,
                                          "scope": "selected_local_observations", "stopped_reasons": []}}
    # Stale/conflicting observations cannot establish derived connectivity.
    safe = {ident for ident, node in nodes.items() if node.get("status") not in _BAD_STATUS and _domain(node)}
    physical = {ident: edge for ident, edge in edges.items()
                if edge.get("kind") in {"creates", "spends"} and edge.get("status") not in _BAD_STATUS
                and edge["source"] in safe and edge["target"] in safe
                and _domain(nodes[edge["source"]]) == _domain(nodes[edge["target"]])}
    adjacency: dict[str, list[tuple[str, str]]] = defaultdict(list)
    reverse: dict[str, list[tuple[str, str]]] = defaultdict(list)
    for ident, edge in sorted(physical.items()):
        adjacency[edge["source"]].append((edge["target"], ident))
        reverse[edge["target"]].append((edge["source"], ident))
    txids = sorted(ident for ident in safe if nodes[ident].get("kind") == "transaction" and ident in txfacts)
    labels = _get(index, "labels", ())
    if isinstance(labels, Mapping):
        labels = list(labels.values())
    visible_labels = []
    if observer in {"owner", "disclosed"}:
        for row in labels:
            if not isinstance(row, Mapping) or row.get("deleted"):
                continue
            matched = row.get("node_ids") or _get(index, "subjects", {}).get(row.get("subject"), ())
            matched = sorted(ident for ident in matched if ident in safe and _domain(nodes[ident]) == (row.get("chain"), row.get("network")))
            if matched:
                visible_labels.append((row, matched))
                if len(visible_labels) >= 2000:
                    result["coverage"]["stopped_reasons"].append("label_analysis_budget")
                    break
    hypothesis_edges: list[dict] = []

    def hypothesis(code: str, a: str, b: str, premises: Sequence[str], refs: Sequence[Mapping], scope: str = "public") -> None:
        if not allow_hypotheses or a == b or a not in safe or b not in safe or _domain(nodes[a]) != _domain(nodes[b]):
            return
        if len(hypothesis_edges) >= 500:
            if "hypothesis_budget" not in result["coverage"]["stopped_reasons"]:
                result["coverage"]["stopped_reasons"].append("hypothesis_budget")
            return
        a, b = sorted((a, b))
        hypothesis_edges.append({"id": _id(code, a, b), "source": a, "target": b,
                                 "kind": "hypothesis", "rule": code, "rule_version": RULE_VERSION,
                                 "label": code.replace("_", " "), "status": "hypothesis", "evidence_level": "heuristic",
                                 "observer": scope, "premises": list(premises), "evidence": _plain(refs),
                                 "reversible": True, "accounting_authority": False})

    scripts: dict[tuple, list[str]] = defaultdict(list)
    for ident in sorted(safe):
        script = outfacts.get(ident, {}).get("script")
        if isinstance(script, str) and script and not script.startswith("6a"):
            scripts[(*_domain(nodes[ident]), script)].append(ident)
    for members in scripts.values():
        if len(members) > 1:
            result["findings"].append(_finding("script_reuse", members, detail="The same locking script occurs on distinct locally observed outputs. Common control remains a hypothesis.", observer="public", level="observed", evidence=[ref for member in members for ref in nodes[member].get("evidence", ())]))
            for member in members[1:]:
                hypothesis("reused_script_control", members[0], member, ["same_locking_script", "script_control_is_not_entity_identity"], nodes[member].get("evidence", ()))
    for txid in txids:
        fact = txfacts[txid]
        observed_inputs = {node for node, _ in reverse.get(txid, ())}
        observed_outputs = {node for node, _ in adjacency.get(txid, ())}
        ins = [row["output_id"] for row in fact.get("inputs", ()) if row.get("output_id") in observed_inputs]
        outs = [row["output_id"] for row in fact.get("outputs", ()) if row.get("output_id") in observed_outputs]
        collab = fact.get("collaboration")
        if collab:
            result["findings"].append(_finding("collaborative_boundary", [txid], detail="Collaboration evidence suppresses common-input and change ownership assumptions; physical spends remain observable.", observer=observer, level=collab.get("evidence_level", "heuristic"), evidence=[collab]))
            continue
        if nodes[txid].get("chain") != "bitcoin":
            continue
        for other in ins[1:]:
            hypothesis("common_input_control", ins[0], other, ["observed_co_spend", "assumes_no_undetected_collaboration"], nodes[txid].get("evidence", ()))
        if fact.get("complete") and len(outs) == 2 and len(fact.get("outputs", ())) == 2 and ins:
            input_scripts = {outfacts.get(ident, {}).get("script") for ident in ins} - {None, ""}
            candidates = [ident for ident in outs if outfacts.get(ident, {}).get("script") in input_scripts]
            if len(candidates) == 1:
                hypothesis("change_script_return", ins[0], candidates[0], ["two_output_transaction", "one_output_reuses_input_script", "assumes_no_undetected_collaboration", "payment_to_same_script_remains_alternative"], nodes[txid].get("evidence", ()))
        if observer in {"owner", "disclosed"}:
            for output in outs:
                if outfacts.get(output, {}).get("branch_role") != "change":
                    continue
                owners = set(nodes[output].get("wallet_ids", ()))
                for inp in ins:
                    if owners & set(nodes[inp].get("wallet_ids", ())):
                        hypothesis("private_wallet_change", inp, output, ["local_wallet_branch_evidence", "not_public_observer_knowledge", "assumes_no_undetected_collaboration"], nodes[output].get("evidence", ()), observer)
    label_groups: dict[tuple, list[tuple[Mapping, str]]] = defaultdict(list)
    for label, members in visible_labels:
        if label.get("cluster_defining") is True or label.get("cluster_defining") == 1:
            key = (label.get("chain"), label.get("network"), label.get("source"), label.get("label"))
            label_groups[key].extend((label, ident) for ident in members)
    for members in label_groups.values():
        for label, ident in members[1:]:
            hypothesis("authored_label_cluster", members[0][1], ident, ["explicit_cluster_defining_label", "attribution_claim_not_ownership_proof"], [{"source": "local_label", "reference": label.get("id"), "revision": label.get("revision"), "confidence": label.get("confidence")}], observer)
    hypothesis_edges = list({row["id"]: row for row in hypothesis_edges}.values())
    result["hypothesis_edges"] = sorted(hypothesis_edges, key=lambda row: row["id"])
    _clusters(result, hypothesis_edges, nodes, observer)
    _patterns(result, nodes, txfacts, txids, adjacency, reverse, physical, query)
    exposure_forward = {key: list(value) for key, value in adjacency.items()}
    exposure_reverse = {key: list(value) for key, value in reverse.items()}
    if observer != "public" and query.get("include_relations", True):
        for ident, edge in sorted(edges.items()):
            if edge.get("kind") == "custody" and edge.get("status") not in _BAD_STATUS and edge["source"] in safe and edge["target"] in safe:
                exposure_forward.setdefault(edge["source"], []).append((edge["target"], ident))
                exposure_reverse.setdefault(edge["target"], []).append((edge["source"], ident))
    _exposure(result, index, query, nodes, visible_labels, exposure_forward, exposure_reverse, edges, observer)
    if query.get("mode") == "entropy" or query.get("action") == "entropy" or query.get("entropy") is True:
        roots = _roots(index, query, selected)
        candidates = [txid for txid in txids if txid in roots]
        if not candidates:
            result["entropy"].append({"status": "unsupported", "reason": "select_transaction_subject", "model": ENTROPY_MODEL})
        for txid in candidates[:1]:
            result["entropy"].append(dict(analyze_transaction_entropy(txfacts[txid], chain=nodes[txid].get("chain"), max_states=query.get("entropy_max_states", 200_000), max_duration_ms=query.get("entropy_max_duration_ms", 1000)), transaction_node_id=txid))
        if len(candidates) > 1:
            result["coverage"]["stopped_reasons"].append("entropy_single_transaction_limit")
    result["coverage"].update(transaction_count=len(txids), amount_complete_transaction_count=sum(bool(txfacts[ident].get("complete")) and all(_amount(row.get("amount_msat")) is not None for row in (*txfacts[ident].get("inputs", ()), *txfacts[ident].get("outputs", ()))) for ident in txids), hypothesis_count=len(hypothesis_edges), pattern_count=len(result["patterns"]))
    return result


def _clusters(result: dict, edges: Sequence[Mapping], nodes: Mapping, observer: str) -> None:
    parents: dict[str, str] = {}

    def root(node: str) -> str:
        parents.setdefault(node, node)
        while parents[node] != node:
            parents[node] = parents[parents[node]]
            node = parents[node]
        return node

    for edge in edges:
        a, b = root(edge["source"]), root(edge["target"])
        parents[max(a, b)] = min(a, b)
    components: dict[str, list[str]] = defaultdict(list)
    for member in parents:
        components[root(member)].append(member)
    for members in components.values():
        refs = [edge for edge in edges if edge["source"] in members]
        result["clusters"].append({"id": _id("cluster", sorted(members), observer), "node_ids": sorted(members),
                                   "edge_ids": sorted(edge["id"] for edge in refs), "observer": observer,
                                   "kind": "possible_common_control", "evidence_level": "heuristic",
                                   "rules": sorted({edge["rule"] for edge in refs}), "rule_version": RULE_VERSION,
                                   "chain": nodes[members[0]].get("chain"), "network": nodes[members[0]].get("network"),
                                   "reversible": True, "accounting_authority": False})
    result["clusters"].sort(key=lambda row: row["id"])


def _roots(index: Any, query: Mapping, selected: set[str]) -> set[str]:
    given = query.get("_subject_node_ids")
    if given:
        return set(given) & selected
    subjects = query.get("subjects") or [query.get("subject")]
    if isinstance(subjects, str):
        subjects = [subjects]
    resolved = set()
    for subject in subjects:
        if isinstance(subject, str):
            if subject in selected:
                resolved.add(subject)
            resolved.update(_get(index, "subjects", {}).get(subject, ()))
    return resolved & selected if resolved else selected


def _patterns(result: dict, nodes: Mapping, facts: Mapping, txids: list[str], adjacency: Mapping,
              reverse: Mapping, physical: Mapping, query: Mapping) -> None:
    max_steps = _bounded_int(query.get("pattern_depth", query.get("depth", 6)), 6, 12)
    remaining = 20_000
    patterns: dict[str, dict] = {}

    def add(code: str, ids: Sequence[str], edges: Sequence[str], detail: str, level: str = "observed") -> None:
        if len(patterns) >= 250:
            if "pattern_result_budget" not in result["coverage"]["stopped_reasons"]:
                result["coverage"]["stopped_reasons"].append("pattern_result_budget")
            return
        row = _finding(code, ids, detail=detail, observer="public", level=level, edge_ids=edges, evidence=[ref for ident in sorted(set(edges)) for ref in physical.get(ident, {}).get("evidence", ())])
        row["scope"] = "selected_local_topology"
        patterns[row["id"]] = row

    # Topology facts do not imply any particular input/output financial flow.
    for txid in txids:
        incoming = [(parent, eid1, output, eid2) for output, eid2 in reverse.get(txid, ()) for parent, eid1 in reverse.get(output, ())]
        outgoing = [(child, eid2, output, eid1) for output, eid1 in adjacency.get(txid, ()) for child, eid2 in adjacency.get(output, ())]
        if len({row[0] for row in incoming}) > 1:
            add("fan_in", [txid, *[row[0] for row in incoming]], [edge for row in incoming for edge in (row[1], row[3])], "Inputs from multiple observed parent transactions are spent together. This is topology, not common ownership.")
        if len({row[0] for row in outgoing}) > 1:
            add("fan_out", [txid, *[row[0] for row in outgoing]], [edge for row in outgoing for edge in (row[1], row[3])], "Different outputs have locally observed spends in different later transactions.")
    for origin in txids[:100]:
        reached: dict[str, dict[str, tuple[list[str], list[str]]]] = defaultdict(dict)
        for first_output, first_edge in adjacency.get(origin, ()):
            queue = deque([(first_output, [origin, first_output], [first_edge], 0)])
            seen = {origin, first_output}
            while queue and remaining:
                current, path, evidence, hops = queue.popleft()
                remaining -= 1
                if nodes[current].get("kind") == "transaction":
                    reached[current].setdefault(first_output, (path, evidence))
                if hops >= max_steps * 2:
                    if adjacency.get(current) and "pattern_depth_limit" not in result["coverage"]["stopped_reasons"]:
                        result["coverage"]["stopped_reasons"].append("pattern_depth_limit")
                    continue
                for nxt, edge in adjacency.get(current, ()):
                    if nxt not in seen:
                        seen.add(nxt)
                        queue.append((nxt, [*path, nxt], [*evidence, edge], hops + 1))
        for target, branches in reached.items():
            if len(branches) > 1:
                paths = list(branches.values())[:2]
                ids, eids = [node for path, _ in paths for node in path], [edge for _, proof in paths for edge in proof]
                code = "postmix_reconvergence" if (facts[origin].get("collaboration") or {}).get("kind") == "coinjoin" else "reconvergence"
                add(code, ids, eids, "Distinct output branches reach the same later transaction through observed spends. No unique value allocation or common owner is inferred.")
                if code == "postmix_reconvergence":
                    pattern = patterns.get(_id(code, sorted(set(ids))))
                    if pattern is not None:
                        pattern["boundary_evidence"] = _plain(facts[origin]["collaboration"])
                        pattern["conditional_on_boundary_interpretation"] = True
        if not remaining:
            break
    if len(txids) > 100 or not remaining:
        result["coverage"]["stopped_reasons"].append("pattern_search_budget")

    # A peel candidate follows the strictly larger output through >=3 ordinary
    # 1-in/2-out transactions. Every amount and link is a listed premise.
    next_peel: dict[str, tuple[str, list[str]]] = {}
    for txid in txids:
        fact = facts[txid]
        ins, outs = fact.get("inputs", ()), fact.get("outputs", ())
        if nodes[txid].get("chain") != "bitcoin" or fact.get("collaboration") or not fact.get("complete") or len(ins) != 1 or len(outs) != 2:
            continue
        amounts = [_amount(row.get("amount_msat")) for row in outs]
        input_amount = _amount(ins[0].get("amount_msat"))
        if input_amount is None or any(value is None for value in amounts) or sum(amounts) > input_amount or amounts[0] == amounts[1]:
            continue
        output = outs[0 if amounts[0] > amounts[1] else 1]["output_id"]
        spenders = adjacency.get(output, ())
        if len(spenders) == 1:
            child, spend_edge = spenders[0]
            creates = [eid for dest, eid in adjacency.get(txid, ()) if dest == output]
            if creates:
                next_peel[txid] = (child, [creates[0], spend_edge])
    for txid in sorted(next_peel) if query.get("include_hypotheses") is True else ():
        if txid in {row[0] for row in next_peel.values()}:
            continue
        path, proof = [txid], []
        while path[-1] in next_peel and len(path) < 12:
            child, edges = next_peel[path[-1]]
            if child in path or child not in next_peel:
                break
            path.append(child)
            proof.extend(edges)
        if len(path) >= 3:
            add("peel_chain_candidate", path, proof, "At least three non-marked 1-input/2-output transactions continue through the larger output. Repeated payments and ownership remain hypotheses; undetected collaboration can invalidate them.", "heuristic")
    result["patterns"] = sorted(patterns.values(), key=lambda row: row["id"])


def _exposure(result: dict, index: Any, query: Mapping, nodes: Mapping, labels: Sequence,
              adjacency: Mapping, reverse: Mapping, edges: Mapping, observer: str) -> None:
    if not labels:
        return
    roots = _roots(index, query, set(nodes))
    max_depth = _bounded_int(query.get("depth", 6), 6, 12) * 2
    remaining = 20_000
    stopped = False
    for label, targets in labels:
        claim = {key: label.get(key) for key in ("id", "revision", "label", "category", "source", "confidence")}
        for target in targets:
            if len(result["exposure"]) >= 250 or not remaining:
                stopped = True
                break
            if target in roots:
                result["exposure"].append({"id": _id("exposure", claim["id"], target), "kind": "direct_label_match", "claim": claim, "node_ids": [target], "edge_ids": [], "observer": observer, "authority": "sourced_claim", "taint_inference": False})
                continue
            for direction, graph in (("descendant", adjacency), ("ancestor", reverse)):
                queue = deque([(target, [target], [])])
                seen = {target}
                found = None
                while queue and remaining:
                    current, path, proof = queue.popleft()
                    remaining -= 1
                    if current in roots:
                        found = (path, proof)
                        break
                    if len(proof) >= max_depth:
                        if graph.get(current) and "exposure_depth_limit" not in result["coverage"]["stopped_reasons"]:
                            result["coverage"]["stopped_reasons"].append("exposure_depth_limit")
                        continue
                    for nxt, edge in graph.get(current, ()):
                        if nxt not in seen:
                            seen.add(nxt)
                            queue.append((nxt, [*path, nxt], [*proof, edge]))
                if found:
                    path, proof = found
                    result["exposure"].append({"id": _id("exposure", claim["id"], path, direction), "kind": "indirect_topological_contact", "direction_from_label": direction, "claim": claim, "node_ids": path, "edge_ids": proof, "observer": observer, "authority": "sourced_claim_with_evidence_path", "path_kinds": [edges[ident]["kind"] for ident in proof], "taint_inference": False, "limitations": ["adjacency_does_not_prove_payment_or_ownership", "selected_local_coverage_only"]})
            if len(result["exposure"]) >= 250 or not remaining:
                stopped = True
                break
        if stopped:
            break
    if stopped:
        result["coverage"]["stopped_reasons"].append("exposure_search_budget")
    result["exposure"].sort(key=lambda row: row["id"])

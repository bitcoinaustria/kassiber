"""Pure, reversible analytics over a local observation index.

No output grants ownership, accounting, or taint authority. Entropy implements
paired partitions of independent financial flows (LaurentMT's Boltzmann model),
with explicit fee scenarios, not probabilities of wallet ownership.
https://gist.github.com/LaurentMT/e758767ca4038ac40aaf
"""
from __future__ import annotations

from collections import defaultdict, deque
import hashlib
import json
from typing import Any, Mapping, Sequence

from ...wallet_descriptors import normalize_network
from .entropy import ENTROPY_MODEL, analyze_transaction_entropy
from .components import Components

RULE_VERSION = "local-analytics-v1"
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
    selected_edges = frozenset(query["_selected_edge_ids"]) if "_selected_edge_ids" in query else None
    edges = {ident: row for ident, row in _get(index, "edges", {}).items()
             if row.get("source") in selected and row.get("target") in selected
             and (selected_edges is None or ident in selected_edges)}
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
    if observer in {"owner", "disclosed", "public"}:
        for row in labels:
            if not isinstance(row, Mapping) or row.get("deleted"):
                continue
            if observer == "public" and not (row.get("dataset_id") and row.get("visibility") == "public"):
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
        if len(ins) > 1:
            result["findings"].append(_finding("observed_co_spend", [txid, *ins], detail="These observed outputs are spent in one transaction. Common ownership is a separate hypothesis and can fail under collaboration.", observer="public", level="observed", evidence=nodes[txid].get("evidence", ()), edge_ids=[edge for output, edge in reverse.get(txid, ()) if output in ins]))
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
    result["coverage"].update(transaction_count=len(txids), amount_complete_transaction_count=sum(bool(txfacts[ident].get("complete")) and all(_amount(row.get("amount_msat")) is not None for row in (*txfacts[ident].get("inputs", ()), *txfacts[ident].get("outputs", ()))) for ident in txids), hypothesis_count=len(hypothesis_edges), pattern_count=len(result["patterns"]), label_count=len(visible_labels))
    return result


def _clusters(result: dict, edges: Sequence[Mapping], nodes: Mapping, observer: str) -> None:
    components = Components({edge[side] for edge in edges for side in ("source", "target")})
    for edge in edges:
        components.union(edge["source"], edge["target"])
    edges_by_root: dict[str, list[Mapping]] = defaultdict(list)
    for edge in edges:
        edges_by_root[components.find(edge["source"])].append(edge)
    for members in components.groups():
        refs = edges_by_root[components.find(members[0])]
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
        claim = {key: label.get(key) for key in ("id", "revision", "label", "category", "source", "confidence", "dataset_id", "dataset_version", "content_sha256", "license", "attribution_method", "source_record", "valid_from", "valid_until", "visibility") if key in label}
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

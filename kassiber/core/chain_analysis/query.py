"""Bounded indexed graph queries. A returned path is connectivity, not taint."""
from __future__ import annotations

from collections import defaultdict, deque
import re
from typing import Any, Mapping

from ...errors import AppError
from ...time_utils import parse_iso_datetime_or_none
from ..custody_evidence import resolve_protocol_scope
from .index import AnalysisIndex, integer, observer_index, thaw


def _invalid(message: str, **details):
    raise AppError(message, code="validation", details=details or None, retryable=False)


def normalize_query(args: Mapping[str, Any] | None) -> dict[str, Any]:
    if args is not None and not isinstance(args, Mapping):
        _invalid("Chain analysis arguments must be an object")
    args = dict(args or {})
    allowed = {"mode", "subject", "target", "chain", "network", "direction", "depth", "node_limit", "edge_limit", "include_relations", "include_hypotheses", "observer", "min_amount_msat", "start", "end"}
    if set(args) - allowed:
        _invalid("Unsupported chain analysis arguments", fields=sorted(set(args) - allowed))
    query = {"mode": "overview", "direction": "both", "depth": 4, "node_limit": 400, "edge_limit": 1200, "include_relations": True, "include_hypotheses": False, "observer": "owner", **args}
    for field, choices in (("mode", {"overview", "trace", "path"}), ("direction", {"backward", "forward", "both"}), ("observer", {"public", "owner", "disclosed"}), ("chain", {"bitcoin", "liquid"}), ("network", {"main", "test", "signet", "regtest"})):
        if field in query and (not isinstance(query[field], str) or query[field] not in choices):
            _invalid(f"Invalid {field}", choices=sorted(choices))
    for field, low, high in (("depth", 1, 50), ("node_limit", 25, 2000), ("edge_limit", 50, 6000)):
        if type(query[field]) is not int or not low <= query[field] <= high:
            _invalid(f"{field} must be an integer between {low} and {high}")
    for field in ("include_relations", "include_hypotheses"):
        if type(query[field]) is not bool:
            _invalid(f"{field} must be a boolean")
    for field in ("subject", "target"):
        if field in query:
            if not isinstance(query[field], str) or not query[field].strip() or len(query[field]) > 1024:
                _invalid(f"{field} must be a nonempty identifier up to 1024 characters")
            query[field] = query[field].strip()
    if query["mode"] in {"trace", "path"} and not query.get("subject"):
        _invalid("A subject is required for trace/path mode")
    if query["mode"] == "path" and not query.get("target"):
        _invalid("A target is required for path mode")
    if "min_amount_msat" in query:
        if not isinstance(query["min_amount_msat"], str) or integer(query["min_amount_msat"]) is None:
            _invalid("min_amount_msat must be an exact nonnegative decimal integer string")
    parsed = {}
    for field in ("start", "end"):
        if field in query:
            value = query[field]
            if not isinstance(value, str) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})", value):
                _invalid(f"{field} must be an RFC3339 timestamp")
            when = parse_iso_datetime_or_none(value)
            if when is None or when.tzinfo is None:
                _invalid(f"{field} must include a valid timezone")
            parsed[field] = when
    if "start" in parsed and "end" in parsed and parsed["start"] > parsed["end"]:
        _invalid("start must not be later than end")
    return query


def resolve_subject(index: AnalysisIndex, subject: str, query: Mapping[str, Any]) -> tuple[str, ...]:
    candidates = index.subjects.get(subject, ())
    if not candidates and subject.lower() != subject:
        candidates = index.subjects.get(subject.lower(), ())
    candidates = tuple(node_id for node_id in candidates if _domain(index.nodes[node_id], query))
    if not candidates:
        raise AppError("No locally observed graph subject matches this identifier", code="not_found", details={"subject": subject, "local_only": True}, retryable=False)
    domains = {(index.nodes[node_id]["chain"], index.nodes[node_id]["network"]) for node_id in candidates}
    # Wallets and authored relation records intentionally span rails. A bare
    # physical identifier must be disambiguated instead of bridging networks.
    physical = len(subject.split(":")) <= 2 and (len(subject.split(":")[0]) == 64 or any(index.nodes[node_id].get("address") == subject for node_id in candidates))
    if physical and len(domains) > 1:
        raise AppError("Subject is observed in multiple chain/network domains", code="subject_ambiguous", details={"domains": [{"chain": chain, "network": network} for chain, network in sorted(domains)]}, retryable=False)
    return candidates


def _domain(node: Mapping[str, Any], query: Mapping[str, Any]) -> bool:
    if "chain" in query and node.get("chain") != query["chain"]:
        return False
    if "network" in query:
        try:
            wanted = resolve_protocol_scope({"chain": node.get("chain"), "network": query["network"]})
        except (ValueError, AppError):
            return False
        return node.get("network") == wanted.network
    return True


class _Traversal:
    def __init__(self, index: AnalysisIndex, query: Mapping[str, Any]):
        self.index, self.query = index, query
        self.nodes: set[str] = set()
        self.edges: set[str] = set()
        self.frontier: dict[tuple[str, str], dict] = {}
        self.inspections = 0
        self.budget = False
        self.filtered = defaultdict(int)
        self.frontier_omitted = 0

    def stop(self, node_id: str, reason: str, **details):
        if (node_id, reason) not in self.frontier and len(self.frontier) >= self.query["node_limit"]:
            self.frontier_omitted += 1
            return
        self.frontier[(node_id, reason)] = {"node_id": node_id, "reason": reason, **details}

    def admit(self, node_id: str) -> bool:
        if node_id in self.nodes:
            return True
        if len(self.nodes) >= self.query["node_limit"]:
            self.budget = True
            self.stop(node_id, "node_limit")
            return False
        self.nodes.add(node_id)
        return True

    def neighbors(self, node_id: str, blocked: frozenset[str] = frozenset()):
        node = self.index.nodes[node_id]
        if node["status"] in {"conflicting", "stale"}:
            self.stop(node_id, node["status"])
            return
        if node["status"] == "missing":
            self.stop(node_id, "missing_observation")
        if node["kind"] == "transaction" and not self.index.transaction_facts.get(node_id, {}).get("complete"):
            self.stop(node_id, "incomplete_transaction")
        if node["kind"] == "output" and self.query["direction"] in {"forward", "both"} and not any(self.index.edges[edge_id]["kind"] == "spends" for edge_id in self.index.outgoing.get(node_id, ())):
            self.stop(node_id, "unobserved_successor")
        edge_ids = set()
        if self.query["direction"] in {"forward", "both"}:
            edge_ids.update(self.index.outgoing.get(node_id, ()))
        if self.query["direction"] in {"backward", "both"}:
            edge_ids.update(self.index.incoming.get(node_id, ()))
        for edge_id in sorted(edge_ids):
            if edge_id in blocked:
                continue
            if self.inspections >= self.query["edge_limit"]:
                self.budget = True
                self.stop(node_id, "edge_limit")
                return
            self.inspections += 1
            edge = self.index.edges[edge_id]
            other = edge["target"] if edge["source"] == node_id else edge["source"]
            if edge["kind"] == "custody" and (not self.query["include_relations"] or self.query["observer"] == "public"):
                self.filtered["relations_disabled"] += 1
                continue
            if edge.get("status") in {"stale", "conflicting"}:
                self.stop(node_id, f"{edge['status']}_relation" if edge["kind"] == "custody" else edge["status"], edge_id=edge_id)
                continue
            if not _domain(self.index.nodes[other], self.query):
                self.filtered["domain"] += 1
                self.stop(node_id, "domain_filter", edge_id=edge_id)
                continue
            amount = edge.get("amount_msat")
            if self.query.get("min_amount_msat") is not None and amount is not None and int(amount) < int(self.query["min_amount_msat"]):
                self.filtered["amount"] += 1
                self.stop(node_id, "amount_filter", edge_id=edge_id)
                continue
            occurred = self.index.nodes[other].get("occurred_at")
            when = parse_iso_datetime_or_none(occurred) if occurred else None
            if when is not None and ((self.query.get("start") and when < parse_iso_datetime_or_none(self.query["start"])) or (self.query.get("end") and when > parse_iso_datetime_or_none(self.query["end"]))):
                self.filtered["time"] += 1
                self.stop(node_id, "time_filter", edge_id=edge_id)
                continue
            if not self.admit(other):
                continue
            self.edges.add(edge_id)
            yield other, edge_id

    def walk(self, seeds: tuple[str, ...]):
        queue = deque()
        distance = {}
        for seed in seeds:
            if self.admit(seed):
                queue.append(seed)
                distance[seed] = 0
            if self.budget:
                break
        while queue:
            node_id = queue.popleft()
            depth = distance[node_id]
            if depth >= self.query["depth"]:
                self.stop(node_id, "depth_limit", depth=depth)
                continue
            for other, _edge_id in self.neighbors(node_id):
                if other not in distance:
                    distance[other] = depth + 1
                    queue.append(other)

    def path(self, seeds: tuple[str, ...], targets: set[str], blocked: frozenset[str] = frozenset()) -> dict | None:
        queue = deque()
        prior: dict[str, tuple[str, str] | None] = {}
        depth = {}
        for seed in seeds:
            if self.admit(seed):
                prior[seed], depth[seed] = None, 0
                queue.append(seed)
        while queue:
            node_id = queue.popleft()
            status = self.index.nodes[node_id]["status"]
            if status in {"stale", "conflicting"}:
                self.stop(node_id, status)
                continue
            if node_id in targets:
                nodes, edges = [node_id], []
                while prior[node_id] is not None:
                    parent, edge_id = prior[node_id]
                    edges.append(edge_id)
                    nodes.append(parent)
                    node_id = parent
                edges.reverse(); nodes.reverse()
                kind = "relation" if any(self.index.edges[item]["kind"] == "custody" for item in edges) else "physical"
                return {"kind": kind, "node_ids": nodes, "edge_ids": edges}
            if depth[node_id] >= self.query["depth"]:
                self.stop(node_id, "depth_limit", depth=depth[node_id])
                continue
            for other, edge_id in self.neighbors(node_id, blocked):
                if other not in prior:
                    prior[other], depth[other] = (node_id, edge_id), depth[node_id] + 1
                    queue.append(other)
        return None


def query_index(index: AnalysisIndex, args: Mapping[str, Any] | None) -> dict[str, Any]:
    """Query an already built immutable index; performs no SQL or network IO."""
    query = normalize_query(args)
    index = observer_index(index, query["observer"])
    if query.get("subject"):
        seeds = resolve_subject(index, query["subject"], query)
    else:
        seeds = index.seed_nodes(query) if hasattr(index, "seed_nodes") else tuple(node_id for node_id in index.profile_seeds if _domain(index.nodes[node_id], query))
    traversal = _Traversal(index, query)
    paths = []
    if query["mode"] == "path":
        targets = set(resolve_subject(index, query["target"], query))
        primary = traversal.path(seeds, targets)
        if primary:
            paths.append(primary)
            # Alternative routes are independently searched with one primary
            # edge removed, sharing the same total work budget.
            seen = {tuple(primary["edge_ids"])}
            for edge_id in primary["edge_ids"]:
                if traversal.budget or len(paths) >= 3:
                    break
                alternative = traversal.path(seeds, targets, frozenset({edge_id}))
                if alternative and tuple(alternative["edge_ids"]) not in seen:
                    paths.append(alternative)
                    seen.add(tuple(alternative["edge_ids"]))
            paths.sort(key=lambda path: (len(path["edge_ids"]), path["edge_ids"]))
    else:
        traversal.walk(seeds)
    nodes = [thaw(index.nodes[node_id]) for node_id in sorted(traversal.nodes)]
    edges = [thaw(index.edges[edge_id]) for edge_id in sorted(traversal.edges)]
    # Inspectable stale/conflicting relations are returned between selected
    # endpoints, but they never participate in reachability or paths.
    edge_ids = {edge["id"] for edge in edges}
    if query["include_relations"] and query["observer"] != "public":
        for node_id in sorted(traversal.nodes):
            for edge_id in index.outgoing.get(node_id, ()):
                edge = index.edges[edge_id]
                if edge["kind"] == "custody" and edge["target"] in traversal.nodes and edge_id not in edge_ids and len(edges) < query["edge_limit"]:
                    edges.append(thaw(edge)); edge_ids.add(edge_id)
    candidates = index.findings_for_nodes(traversal.nodes) if hasattr(index, "findings_for_nodes") else index.findings
    findings = [thaw(item) for item in candidates if not item["node_ids"] or set(item["node_ids"]) & traversal.nodes]
    coverage = thaw(index.coverage)
    coverage["observer_knowledge"] = {"public": "public_chain_facts", "owner": "local_owner_evidence", "disclosed": "assumes_all_local_owner_evidence_disclosed"}[query["observer"]]
    frontier = sorted(traversal.frontier.values(), key=lambda item: (item["node_id"], item["reason"]))
    missing = any(node["status"] == "missing" for node in nodes) or any(item["reason"] in {"missing_observation", "incomplete_transaction", "unobserved_successor"} for item in frontier)
    stale = any(node["status"] == "stale" for node in nodes) or any("stale" in item["reason"] for item in frontier)
    coverage.update(complete=not frontier and not missing and not stale and not coverage["missing_tables"] and not coverage["invalid_observations"], budget_exhausted=traversal.budget, missing=missing, stale=stale, frontier_omitted_count=traversal.frontier_omitted, pruning=[{"filter": key, "count": value} for key, value in sorted(traversal.filtered.items())], depth_unit="graph_edges", scope="locally_observed_subgraph", unobserved_successors="An output without a local spend observation is not proven unspent.", unknown_filter_values="Unknown amounts and times remain visible; filters prune known values only.", path_semantics="Connectivity only. No input-to-output allocation or common ownership is implied.", alternative_paths="Up to three distinct bounded alternatives; absence is not proof that no other path exists.")
    result = {"schema_version": 1, "snapshot_id": index.snapshot_id, "query": query, "summary": {"node_count": len(nodes), "edge_count": len(edges), "transaction_count": sum(node["kind"] == "transaction" for node in nodes), "output_count": sum(node["kind"] == "output" for node in nodes), "record_count": sum(node["kind"] == "record" for node in nodes), "path_count": len(paths), "visited_node_count": len(traversal.nodes), "inspected_edge_count": traversal.inspections}, "nodes": nodes, "edges": edges, "findings": findings, "clusters": [], "paths": paths, "frontier": frontier, "coverage": coverage, "capabilities": {"local_graph": True, "backward_forward_trace": True, "bounded_alternative_paths": True, "cross_rail_custody_relations": True, "network_acquisition": False, "taint_attribution": False, "global_chain_completeness": False}}
    return result

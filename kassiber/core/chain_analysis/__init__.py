"""One local evidence graph shared by the CLI, desktop and consented agent tools."""
from __future__ import annotations

import sqlite3
from typing import Any, Mapping

from ...errors import AppError
from .index import AnalysisIndex, build_index, observer_index, thaw
from .query import normalize_query, query_index, resolve_subject


def run_analysis(conn: sqlite3.Connection, profile_id: str, args: Mapping[str, Any] | None = None) -> dict[str, Any]:
    query = normalize_query(args)
    index = observer_index(build_index(conn, profile_id), query["observer"])
    result = query_index(index, query)
    from .analytics import analyze_index
    analytics_query = dict(query)
    analytics_query["_subject_node_ids"] = resolve_subject(index, query["subject"], query) if query.get("subject") else index.profile_seeds
    analytics_query["_selected_edge_ids"] = [edge["id"] for edge in result["edges"]]
    analytics = analyze_index(index, analytics_query, {node["id"] for node in result["nodes"]})
    result["findings"].extend(thaw(analytics.get("findings", [])))
    result["clusters"] = thaw(analytics.get("clusters", []))
    result["coverage"]["analytics"] = thaw(analytics.get("coverage", {}))
    result["capabilities"]["hypothesis_scope"] = "overlay_only; hypotheses never establish physical or custody reachability"
    for key in ("patterns", "exposure", "entropy"):
        if key in analytics:
            result[key] = thaw(analytics[key])
    if query["include_hypotheses"]:
        selected = {node["id"] for node in result["nodes"]}
        for edge in analytics.get("hypothesis_edges", ()):
            if len(result["edges"]) >= query["edge_limit"]:
                result["coverage"]["budget_exhausted"] = True
                break
            if edge["source"] in selected and edge["target"] in selected:
                result["edges"].append(thaw(edge))
        result["summary"]["edge_count"] = len(result["edges"])
    return result


def run_entropy(conn: sqlite3.Connection, profile_id: str, args: Mapping[str, Any]) -> dict[str, Any]:
    if not isinstance(args, Mapping) or set(args) - {"subject", "chain", "network", "max_states"}:
        raise AppError("Unsupported entropy arguments", code="validation", retryable=False)
    query = normalize_query({"mode": "trace", **{key: value for key, value in args.items() if key != "max_states"}})
    states = args.get("max_states", 200000)
    if type(states) is not int or not 1 <= states <= 200000:
        raise AppError("max_states must be between 1 and 200000", code="validation", retryable=False)
    index = build_index(conn, profile_id)
    try:
        subjects = resolve_subject(index, query["subject"], query)
    except AppError as error:
        if error.code != "subject_ambiguous":
            raise
        return {"schema_version": 1, "snapshot_id": index.snapshot_id, "subject": query["subject"], "status": "unavailable", "reason": "ambiguous_domain"}
    if len(subjects) != 1 or index.nodes[subjects[0]]["kind"] != "transaction":
        return {"schema_version": 1, "snapshot_id": index.snapshot_id, "subject": query["subject"], "status": "unavailable", "reason": "single_physical_transaction_required"}
    node_id = subjects[0]
    from .analytics import analyze_transaction_entropy
    result = analyze_transaction_entropy(index.transaction_facts.get(node_id, {}), chain=index.nodes[node_id]["chain"], max_states=states)
    return {"schema_version": 1, "snapshot_id": index.snapshot_id, "subject": node_id, **thaw(result)}


__all__ = ["AnalysisIndex", "build_index", "normalize_query", "query_index", "run_analysis", "run_entropy"]

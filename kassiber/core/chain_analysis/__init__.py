"""One local evidence graph shared by the CLI, desktop and consented agent tools."""
from __future__ import annotations

import sqlite3
from typing import Any, Mapping

from ...errors import AppError
from .index import AnalysisIndex, build_index, observer_index, thaw
from .query import normalize_query, query_index, resolve_subject


def run_analysis(conn: sqlite3.Connection, profile_id: str, args: Mapping[str, Any] | None = None) -> dict[str, Any]:
    query = normalize_query(args)
    return analyze_snapshot(build_index(conn, profile_id), query)


def analyze_snapshot(index: AnalysisIndex, args: Mapping[str, Any] | None = None) -> dict[str, Any]:
    """Compose one observer's bounded findings from an immutable local index.

    Pure: no database reads, source refreshes, or network requests. Both the
    workbench and Privacy Mirror use this same interpretation of the evidence.
    """
    query = normalize_query(args)
    index = observer_index(index, query["observer"])
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
    from .features import evaluate_features
    from .index import digest
    result["transaction_features"] = []
    for node in result["nodes"]:
        if node.get("status") in {"missing", "stale", "conflicting", "retracted"}:
            continue
        fact = index.transaction_facts.get(node["id"], {})
        snapshot = fact.get("features")
        if not snapshot:
            continue
        result["transaction_features"].append({"subject": node["id"], "features": thaw(snapshot)})
        for finding in evaluate_features(snapshot, collaboration=fact.get("collaboration")):
            finding = thaw(finding)
            result["findings"].append({
                **finding, "id": digest([node["id"], finding["code"], finding["rule_version"]]),
                "title": finding["code"].replace("_", " ").capitalize(), "detail": finding["message"],
                "node_ids": [node["id"]], "edge_ids": [], "accounting_authority": False,
                "observer": query["observer"], "evidence_level": finding["authority"],
            })
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


def prepare_entropy(conn: sqlite3.Connection, profile_id: str, args: Mapping[str, Any]):
    """Freeze observer-visible inputs before computation leaves the DB thread."""
    if not isinstance(args, Mapping) or set(args) - {"subject", "chain", "network", "observer", "max_states", "max_duration_ms", "scenario"}:
        raise AppError("Unsupported entropy arguments", code="validation", retryable=False)
    query = normalize_query({"mode": "trace", **{key: value for key, value in args.items() if key in {"subject", "chain", "network", "observer"}}})
    states = args.get("max_states", 200000)
    duration = args.get("max_duration_ms", 1000)
    if type(states) is not int or not 1 <= states <= 2000000:
        raise AppError("max_states must be between 1 and 2000000", code="validation", retryable=False)
    if type(duration) is not int or not 1 <= duration <= 30000:
        raise AppError("max_duration_ms must be between 1 and 30000", code="validation", retryable=False)
    from .entropy import normalize_scenario
    scenario = normalize_scenario(args.get("scenario"))
    index = observer_index(build_index(conn, profile_id), query["observer"])
    context = {"schema_version": 1, "snapshot_id": index.snapshot_id, "subject": query.get("subject"), "observer": query["observer"]}
    try:
        subjects = resolve_subject(index, query["subject"], query)
    except AppError as error:
        if error.code != "subject_ambiguous":
            raise
        return context, None, {"status": "unavailable", "reason": "ambiguous_domain"}
    if len(subjects) != 1 or index.nodes[subjects[0]]["kind"] != "transaction":
        return context, None, {"status": "unavailable", "reason": "single_physical_transaction_required"}
    node_id = subjects[0]
    context["subject"] = node_id
    options = {"chain": index.nodes[node_id]["chain"], "max_states": states, "max_duration_ms": duration}
    options["scenario"] = scenario
    return context, thaw(index.transaction_facts.get(node_id, {})), options


def run_entropy(conn: sqlite3.Connection, profile_id: str, args: Mapping[str, Any]) -> dict[str, Any]:
    context, facts, options = prepare_entropy(conn, profile_id, args)
    if facts is None:
        return {**context, **options}
    from .analytics import analyze_transaction_entropy
    return {**context, **thaw(analyze_transaction_entropy(facts, **options))}


def start_entropy(conn: sqlite3.Connection, profile_id: str, args: Mapping[str, Any]) -> dict[str, Any]:
    from ..chain_analysis_runtime import JOBS, scope_key
    from .analytics import analyze_transaction_entropy
    context, facts, options = prepare_entropy(conn, profile_id, args)
    def compute(progress, cancelled):
        result = options if facts is None else analyze_transaction_entropy(facts, **options, progress=progress, cancelled=cancelled)
        return {**context, **thaw(result)}
    return JOBS.start(scope_key(conn, profile_id), {**dict(args), "snapshot_id": context["snapshot_id"]}, compute)


__all__ = ["AnalysisIndex", "build_index", "normalize_query", "query_index", "analyze_snapshot", "run_analysis", "run_entropy"]

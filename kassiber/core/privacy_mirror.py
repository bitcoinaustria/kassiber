"""Personal relevance over the shared, observer-visible investigation snapshot.

No graph construction, ownership heuristic, parser, score or network transport
lives here. Findings retain their canonical evidence and rule identifiers; this
projection only chooses what needs attention and how to investigate it.
"""
from __future__ import annotations

from copy import deepcopy
import time
from typing import Any, Mapping

from .chain_analysis import analyze_snapshot
from .chain_analysis.projection import read_index
from .chain_analysis.entropy import analyze_transaction_entropy
from .chain_analysis.index import AnalysisIndex, digest, observer_index, thaw


QUERY = {
    "mode": "overview", "observer": "public", "direction": "both",
    "depth": 12, "node_limit": 2000, "edge_limit": 6000,
    "include_relations": False, "include_hypotheses": True,
}
MAX_FINDINGS = 100
MAX_ENTROPY_TRANSACTIONS = 12
ENTROPY_DURATION_MS = 25
ENTROPY_TOTAL_MS = 300
ENTROPY_MAX_STATES = 5000
_UNSAFE = {"missing", "stale", "conflicting"}
_LINK_RULES = {"common_input_control", "change_script_return", "reused_script_control"}
ASSUMPTIONS = [
    "public_chain_observer_with_locally_available_history",
    "private_ownership_selects_relevance_not_observer_knowledge",
    "common_control_is_conditional_not_identity_proof",
    "collaboration_blocks_ownership_assumptions_not_physical_spends",
    "topological_contact_is_not_payment_allocation_or_taint",
    "local_coverage_is_not_whole_chain_completeness",
    "entropy_counts_conditional_models_not_ownership_probabilities",
]


def _investigation(index: AnalysisIndex, subjects=()) -> dict:
    subject = next((value for value in subjects if value in index.nodes), None)
    query = dict(QUERY)
    if subject is not None:
        node = index.nodes[subject]
        network = {"liquidv1": "main", "liquidtestnet": "test", "elementsregtest": "regtest"}.get(node["network"], node["network"])
        query.update(mode="trace", subject=subject, chain=node["chain"], network=network)
    return {"query": query, "snapshot_id": index.snapshot_id}


def _owned_outputs(index: AnalysisIndex) -> set[str]:
    # This is only a relevance overlay. Ambiguous private ownership cannot
    # identify which wallet should act, and never adds public cluster edges.
    if hasattr(index, "owned_output_ids"):
        return index.owned_output_ids()
    return {
        ident for ident, fact in index.output_facts.items()
        if fact.get("ownership_known") and not fact.get("ownership_ambiguous")
        and len(index.nodes[ident].get("wallet_ids", ())) == 1
        and index.nodes[ident].get("status") not in _UNSAFE
    }


def _relevance(index: AnalysisIndex, owned: set[str], subjects, *, edge_ids=None) -> tuple[str, int]:
    related = set(subjects)
    if edge_ids is not None:
        for ident in edge_ids:
            edge = index.edges.get(ident, {})
            related.update(value for key in ("source", "target") if (value := edge.get(key)))
    spends, receipts = set(), set()
    for ident in subjects:
        fact = index.transaction_facts.get(ident, {})
        # A path concerns its evidenced branches, not all other participants'
        # inputs or sibling outputs at every transaction along the route.
        spends.update(row["output_id"] for row in fact.get("inputs", ()) if row["output_id"] in owned and (edge_ids is None or row["output_id"] in related))
        receipts.update(row["output_id"] for row in fact.get("outputs", ()) if row["output_id"] in owned and (edge_ids is None or row["output_id"] in related))
    affected = (related & owned) | spends | receipts
    if spends:
        return "own_spend", len(affected)
    if related & owned:
        return "owned_output", len(affected)
    if receipts:
        return "received_context", len(affected)
    return "nearby_context", 0


def _findings(owner: AnalysisIndex, public: AnalysisIndex, analysis: Mapping) -> list[dict]:
    owned = _owned_outputs(owner)
    result = {}

    def add(row, category, *, code=None, subjects=None, assumptions=(), title=None, detail=None):
        code = code or row.get("code") or row.get("kind")
        subjects = list(dict.fromkeys(subjects if subjects is not None else row.get("node_ids", ())))
        subjects = [ident for ident in subjects if ident in public.nodes]
        # Stale/conflicting/missing observations belong in coverage, not current
        # findings. Keep this projection fail-closed even for a partial result.
        if not subjects or any(public.nodes[ident].get("status") in _UNSAFE for ident in subjects):
            return
        relevance, count = _relevance(owner, owned, subjects, edge_ids=row.get("edge_ids", ()) if category == "pattern" or row.get("kind") == "indirect_topological_contact" else None)
        actionable = relevance in {"own_spend", "owned_output"} and (
            code in _LINK_RULES | {"script_reuse", "postmix_reconvergence"}
        )
        identity = row.get("id") or digest([code, sorted(subjects)])
        result[identity] = {
            "id": identity, "code": code, "category": category,
            "severity": "warning" if actionable else "info",
            "authority": row.get("authority") or row.get("evidence_level") or "observed",
            "relevance": relevance, "title": title or row.get("title") or str(code).replace("_", " ").capitalize(),
            "detail": detail or row.get("detail") or row.get("message") or "",
            "assumptions": list(dict.fromkeys([*assumptions, *row.get("assumptions", ()), *row.get("premises", ())])),
            "limitations": list(row.get("limitations", ())),
            "affected_output_count": count,
            "source_ids": list(row.get("source_ids", [identity])),
            "investigation": _investigation(public, subjects),
        }
        if row.get("claim"):
            result[identity]["claim"] = thaw(row["claim"])
        if code == "script_reuse":
            # Equal scripts can occur on physically disconnected transactions.
            # Seed the shared script alias so the workbench sees every member;
            # tracing just the first output would lose the reported evidence.
            script = public.output_facts.get(subjects[0], {}).get("script")
            if script:
                result[identity]["investigation"]["query"] = (
                    {**result[identity]["investigation"]["query"], "subject": f"script:{script}"}
                    if len(script) + len("script:") <= 1024 else dict(QUERY)
                )

    for row in analysis.get("findings", ()):
        code = row.get("code")
        if code in {"script_reuse", "collaborative_boundary"}:
            add(row, "linkage")
        elif row.get("rule_version"):
            add(row, "structure")

    # A common-input transaction may yield several edges. One finding carries
    # all canonical edge IDs instead of producing a card for each input pair.
    groups = {}
    for edge in analysis.get("edges", ()):
        if edge.get("kind") != "hypothesis" or edge.get("rule") not in _LINK_RULES:
            continue
        if edge["rule"] == "reused_script_control":
            continue  # the observed script_reuse finding already explains it
        anchors = [ref.get("reference") for ref in edge.get("evidence", ()) if ref.get("reference") in public.transaction_facts]
        anchor = anchors[0] if anchors else edge["source"]
        key = (edge["rule"], anchor)
        group = groups.setdefault(key, {"node_ids": [anchor], "source_ids": [], "premises": [], "evidence_level": "hypothesis"})
        group["node_ids"].extend([edge["source"], edge["target"]])
        group["source_ids"].append(edge["id"])
        group["premises"].extend(edge.get("premises", ()))
    for (code, anchor), row in groups.items():
        row["id"] = digest(["privacy_mirror", code, anchor, sorted(row["source_ids"])])
        add(row, "linkage", code=code, detail={
            "common_input_control": "Observed inputs are spent together. Common control is conditional on this being an ordinary, non-collaborative transaction.",
            "change_script_return": "One output returns to an input script. Change is a hypothesis; a payment to the same script remains an alternative.",
        }[code])
    for row in analysis.get("patterns", ()):
        add(row, "pattern", assumptions=("connectivity_does_not_allocate_value",))
    for row in analysis.get("exposure", ()):
        add(row, "attribution", title="Public attribution claim", detail=(
            "A locally imported public dataset names this subject. The claim is not verified ownership."
            if row["kind"] == "direct_label_match" else
            "Observed transaction links reach a subject named by a locally imported public dataset. This is not evidence of payment, ownership or taint."
        ), assumptions=("source_claim_requires_independent_review",))
    return sorted(result.values(), key=lambda row: (row["severity"] != "warning", row["relevance"] in {"received_context", "nearby_context"}, row["code"], row["id"]))


def _entropy(owner: AnalysisIndex, public: AnalysisIndex, selected: set[str]) -> dict:
    owned = _owned_outputs(owner)
    candidates = [ident for ident in selected
                  if public.nodes[ident].get("status") not in _UNSAFE
                  and public.transaction_facts.get(ident, {}).get("inputs")]
    candidates.sort(key=lambda ident: (
        _relevance(owner, owned, [ident])[0] != "own_spend",
        len(public.transaction_facts[ident].get("inputs", ())) < 2, ident,
    ))
    results = []
    deadline = time.monotonic() + ENTROPY_TOTAL_MS / 1000
    for ident in candidates[:MAX_ENTROPY_TRANSACTIONS]:
        remaining_ms = int((deadline - time.monotonic()) * 1000)
        if remaining_ms <= 0:
            break
        value = analyze_transaction_entropy(
            public.transaction_facts[ident], chain=public.nodes[ident]["chain"],
            max_states=ENTROPY_MAX_STATES, max_duration_ms=min(ENTROPY_DURATION_MS, remaining_ms),
        )
        keys = ("status", "reason", "model", "rule_version", "interpretation_count", "interpretation_count_lower_bound", "entropy_bits", "conditional_on_model", "assumptions", "limitations", "limits")
        results.append({"subject": ident, "conditional_on_model": True, **{key: thaw(value[key]) for key in keys if key in value},
                        "investigation": _investigation(public, [ident])})
    return {"evaluated": len(results), "eligible": len(candidates), "omitted": len(candidates) - len(results),
            "results": results, "limits": {"transactions": MAX_ENTROPY_TRANSACTIONS, "total_duration_ms": ENTROPY_TOTAL_MS, "states_per_transaction": ENTROPY_MAX_STATES}}


def _coverage(public: AnalysisIndex, analysis: Mapping, entropy: Mapping, finding_count: int) -> dict:
    nodes = analysis["nodes"]
    selected = {row["id"] for row in nodes if row["kind"] == "transaction"}
    available = public.available_transaction_ids() if hasattr(public, "available_transaction_ids") else {ident for ident, node in public.nodes.items() if node["kind"] == "transaction" and node["status"] != "missing"}
    safe = {ident for ident in selected if public.nodes[ident]["status"] not in _UNSAFE}
    coverage = analysis["coverage"]
    analytics = coverage.get("analytics", {})
    datasets = coverage.get("datasets", {})
    reasons = set(analytics.get("stopped_reasons", ())) - {"entropy_single_transaction_limit"}
    reasons.update(row["reason"] for row in analysis.get("frontier", ()))
    if finding_count > MAX_FINDINGS:
        reasons.add("finding_display_limit")
    if entropy["omitted"]:
        reasons.add("entropy_transaction_budget")
    if len(selected & available) < len(available):
        reasons.add("local_transactions_outside_selected_subgraph")
    if datasets.get("truncated") or datasets.get("subjects_truncated"):
        reasons.add("dataset_match_budget")
    if coverage.get("invalid_observations"):
        reasons.add("invalid_observations")
    if coverage.get("missing_tables"):
        reasons.add("missing_local_tables")
    for result in entropy["results"]:
        if result["status"] != "exact":
            reasons.add("entropy_" + str(result.get("reason") or result["status"]))
    missing = sum(row["status"] == "missing" for row in nodes)
    stale = sum(row["status"] == "stale" for row in nodes)
    conflicting = sum(row["status"] == "conflicting" for row in nodes)
    truncated = bool(coverage.get("budget_exhausted") or finding_count > MAX_FINDINGS or entropy["omitted"] or len(selected & available) < len(available)
                     or any("budget" in reason or "limit" in reason for reason in reasons))
    partial = bool(reasons or missing or stale or conflicting or not coverage.get("complete"))
    feature_count = sum(bool(public.transaction_facts.get(ident, {}).get("features")) for ident in safe)
    complete_amounts = int(analytics.get("amount_complete_transaction_count") or 0)
    public_claims = len(public.labels)
    examined_claims = int(analytics.get("label_count") or 0)
    checks = [
        {"code": "physical_topology", "status": "partial" if partial else "complete", "evaluated": len(safe), "eligible": len(available)},
        {"code": "ownership_hypotheses", "status": "bounded" if truncated else "partial" if partial else "complete", "evaluated": len(safe), "eligible": len(available)},
        {"code": "structural_features", "status": "partial" if feature_count < len(selected) else "complete", "evaluated": feature_count, "eligible": len(selected)},
        {"code": "multi_hop_patterns", "status": "bounded" if truncated else "partial" if partial else "complete", "evaluated": len(safe), "eligible": len(available)},
        {"code": "public_attribution", "status": "unavailable" if not public_claims else "bounded" if truncated else "partial" if partial or examined_claims < public_claims else "complete", "evaluated": examined_claims, "eligible": public_claims,
         **({"reason": "no_local_public_claims"} if not public_claims else {})},
        {"code": "conditional_entropy", "status": "not_applicable" if not entropy["eligible"] else "bounded" if entropy["omitted"] else "partial" if any(row["status"] != "exact" for row in entropy["results"]) else "complete",
         "evaluated": entropy["evaluated"], "eligible": entropy["eligible"]},
    ]
    if not safe:
        for check in checks:
            if check["code"] != "conditional_entropy":
                check.update(status="unavailable", reason="no_analyzable_transactions")
    return {"status": "unavailable" if not safe else "partial" if partial else "complete",
            "examined_transactions": len(safe), "available_transactions": len(available),
            "amount_complete_transactions": complete_amounts, "missing_nodes": missing, "stale_nodes": stale,
            "conflicting_nodes": conflicting, "truncated": truncated, "stopped_reasons": sorted(reasons), "checks": checks}


def build_privacy_mirror(conn, profile_id: str, *, redacted: bool = True) -> dict[str, Any]:
    """One read snapshot, bounded analysis, then an explicit audience projection."""
    with read_index(conn, profile_id) as owner:
        return _mirror_snapshot(conn, profile_id, owner, redacted=redacted)


def _mirror_snapshot(conn, profile_id, owner, *, redacted):
    public = observer_index(owner, "public")
    analysis = analyze_snapshot(owner, QUERY)
    findings = _findings(owner, public, analysis)
    entropy = _entropy(owner, public, {node["id"] for node in analysis["nodes"]})
    coverage = _coverage(public, analysis, entropy, len(findings))
    owned = _owned_outputs(owner)
    domains = {(public.nodes[ident]["chain"], public.nodes[ident]["network"]) for ident in owned if ident in public.nodes}
    attention = sum(row["severity"] == "warning" for row in findings)
    payload = {
        "payload_schema_version": 2, "observer": "public", "local_only": True,
        "read_only": True, "advisory_only": True,
        "redaction": "desktop_local" if not redacted else "ai_export_safe",
        "investigation": _investigation(public),
        "summary": {
            "status": "unavailable" if not owned or coverage["status"] == "unavailable" else "findings" if attention else "no_observed_exposure",
            "finding_count": len(findings), "attention_count": attention,
            "owned_output_count": len(owned), "analyzed_transaction_count": coverage["examined_transactions"],
            "local_transaction_count": coverage["available_transactions"], "domain_count": len(domains),
        },
        "findings": findings[:MAX_FINDINGS], "coverage": coverage, "entropy": entropy,
        "assumptions": list(ASSUMPTIONS),
    }
    return project_privacy_mirror(conn, profile_id, payload) if redacted else payload


def project_privacy_mirror(conn, profile_id: str, payload: Mapping) -> dict:
    """Keep explanatory codes/counts; project every graph handoff before AI/export."""
    from .chain_analysis_ai import project_ai_result
    result = deepcopy(dict(payload))
    result["redaction"] = "ai_export_safe"

    def project_handoff(value):
        return project_ai_result(conn, profile_id, value)

    result["investigation"] = project_handoff(result["investigation"])
    for finding in result["findings"]:
        identifiers = project_handoff({"id": finding["id"], "edge_ids": finding.pop("source_ids", [])})
        finding["id"] = identifiers["id"]
        finding["source_ids"] = identifiers["edge_ids"]
        finding["investigation"] = project_handoff(finding["investigation"])
        if "claim" in finding:
            finding["claim"] = project_handoff({"claim": finding["claim"]}).get("claim", {})
    for row in result["entropy"]["results"]:
        row["subject"] = project_handoff({"subject": row["subject"]})["subject"]
        row["investigation"] = project_handoff(row["investigation"])
    return result

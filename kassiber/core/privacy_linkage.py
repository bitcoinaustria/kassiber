"""Private inventory and PSBT views over the shared local evidence engine.

Chain parsing, public-control hypotheses, structural features, and component
composition live in chain_analysis. This module projects those results onto
locally owned outputs, adds explicitly hypothetical counterparty/source-funds
anchors, and compares read-only PSBT proposals against that observer baseline.
Private wallet branch evidence never creates a public ownership edge.

Reduced payloads omit addresses, scripts, descriptors, config, raw transactions,
and derivation material. Nothing here refreshes observations or contacts a node.
"""

from __future__ import annotations

import sqlite3
from collections import defaultdict, deque
from dataclasses import dataclass, field, replace
from typing import Any, Iterable, Mapping, Sequence

from ..wallet_descriptors import normalize_asset_code
from .chain_analysis import AnalysisIndex, analyze_snapshot, build_index
from .chain_analysis.index import thaw
from .chain_analysis.components import Components
from .chain_analysis.features import evaluate_features, extract_transaction_features
from .custody_evidence import resolve_protocol_scope
from ..errors import AppError

EVIDENCE_EXACT = "exact"
EVIDENCE_DERIVED = "derived"
EVIDENCE_UNKNOWN = "unknown"
CHANGE_EVIDENCE_GROUND_TRUTH = "ground_truth"
CHANGE_EVIDENCE_IMPORTED = "imported"
CHANGE_EVIDENCE_HEURISTIC = "heuristic"
CHANGE_EVIDENCE_UNAVAILABLE = "unavailable"
ATTRIBUTION_EMITTED_BY_YOU = "emitted_by_you"
ATTRIBUTION_OBSERVED_FROM_COUNTERPARTY = "observed_from_counterparty"
ADVERSARY_PASSIVE_CHAIN = "passive_chain_watcher"
ADVERSARY_KYC_SOURCE_FUNDS = "reviewed_kyc_source_funds_anchor_watcher"
ADVERSARY_KNOWN_COUNTERPARTY = "known_counterparty_hypothetical"
LOCAL_SUPPORT_SUPPORTED = "supported_by_local_ground_truth"
LOCAL_SUPPORT_NOT_SUPPORTED = "not_supported_by_local_ground_truth_under_this_model"
SOURCE_PROXIMITY_KNOWN = "known_source_proximity"
SOURCE_PROXIMITY_UNKNOWN = "unknown_provenance"
SOURCE_PROXIMITY_PARTIAL = "partial_source_proximity"
PRIVACY_LINKAGE_SCHEMA_VERSION = 1
PSBT_PRIVACY_SCHEMA_VERSION = 1


def _normalize_txid(value: Any) -> str:
    return str(value or "").strip().lower()


@dataclass(frozen=True)
class OwnedOutputNode:
    """One locally known owned Bitcoin output.

    ``address_key`` and ``script_key`` are in-memory-only comparison keys. They
    intentionally do not appear in the redacted payload.
    """

    node_id: str
    txid: str
    vout: int
    wallet_id: str
    amount_msat: int
    asset: str
    spent_by: str | None = None
    network: str = "main"
    branch_role: str = "unknown"
    branch_evidence_level: str = EVIDENCE_UNKNOWN
    change_evidence: str = CHANGE_EVIDENCE_UNAVAILABLE
    branch_source: str = "branch_metadata_unavailable"
    has_address: bool = False
    has_script: bool = False
    address_key: str | None = field(default=None, repr=False, compare=False)
    script_key: str | None = field(default=None, repr=False, compare=False)

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "node_id": self.node_id,
            "txid": self.txid,
            "vout": self.vout,
            "wallet_id": self.wallet_id,
            "asset": self.asset,
            "amount_msat": self.amount_msat,
            "spent_by_present": bool(self.spent_by),
            "branch_role": self.branch_role,
            "branch_evidence_level": self.branch_evidence_level,
            "change_evidence": self.change_evidence,
            "branch_source": self.branch_source,
            "address_present": self.has_address,
            "script_present": self.has_script,
        }


@dataclass(frozen=True)
class PrivacyLinkageEdge:
    edge_id: str
    kind: str
    heuristic: str
    from_node_id: str
    to_node_id: str
    evidence_level: str
    source: str
    txid: str | None
    amount_msat: int
    new_linkage: bool
    merged_cluster_count: int
    evidence: Mapping[str, Any]
    observer_linkage: bool = True

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "observer_linkage": self.observer_linkage,
            "edge_id": self.edge_id,
            "kind": self.kind,
            "heuristic": self.heuristic,
            "from_node_id": self.from_node_id,
            "to_node_id": self.to_node_id,
            "evidence_level": self.evidence_level,
            "source": self.source,
            "txid": self.txid,
            "amount_msat": self.amount_msat,
            "new_linkage": self.new_linkage,
            "merged_cluster_count": self.merged_cluster_count,
            "evidence": dict(self.evidence),
        }


@dataclass(frozen=True)
class PrivacyTransactionTell:
    tell_id: str
    txid: str
    kind: str
    attribution: str
    evidence_level: str
    source: str
    penalizes_wallet: bool
    evidence: Mapping[str, Any]

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "tell_id": self.tell_id,
            "txid": self.txid,
            "kind": self.kind,
            "attribution": self.attribution,
            "evidence_level": self.evidence_level,
            "source": self.source,
            "penalizes_wallet": self.penalizes_wallet,
            "evidence": dict(self.evidence),
        }


@dataclass(frozen=True)
class AdversaryInferenceAnchor:
    anchor_id: str
    kind: str
    evidence_level: str
    source: str
    matched_node_ids: tuple[str, ...]
    support_status: str

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "anchor_id": self.anchor_id,
            "kind": self.kind,
            "evidence_level": self.evidence_level,
            "source": self.source,
            "matched_node_ids": list(self.matched_node_ids),
            "matched_node_count": len(self.matched_node_ids),
            "support_status": self.support_status,
        }


@dataclass(frozen=True)
class AdversaryInferenceCluster:
    cluster_id: str
    node_ids: tuple[str, ...]
    wallet_ids: tuple[str, ...]
    edge_ids: tuple[str, ...]
    anchor_ids: tuple[str, ...]
    anchor_kinds: tuple[str, ...]
    evidence_level: str
    support_status: str
    model_basis: str

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "cluster_id": self.cluster_id,
            "node_ids": list(self.node_ids),
            "wallet_ids": list(self.wallet_ids),
            "wallet_count": len(self.wallet_ids),
            "edge_ids": list(self.edge_ids),
            "anchor_ids": list(self.anchor_ids),
            "anchor_kinds": list(self.anchor_kinds),
            "evidence_level": self.evidence_level,
            "support_status": self.support_status,
            "model_basis": self.model_basis,
        }


@dataclass(frozen=True)
class AdversaryInferenceView:
    tier: str
    label: str
    evidence_level: str
    model_assumptions: tuple[Mapping[str, Any], ...]
    summary: Mapping[str, Any]
    clusters: tuple[AdversaryInferenceCluster, ...]
    unsupported_anchors: tuple[AdversaryInferenceAnchor, ...]

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "tier": self.tier,
            "label": self.label,
            "evidence_level": self.evidence_level,
            "model_assumptions": [
                dict(assumption) for assumption in self.model_assumptions
            ],
            "summary": dict(self.summary),
            "clusters": [cluster.to_redacted_payload() for cluster in self.clusters],
            "unsupported_anchors": [
                anchor.to_redacted_payload() for anchor in self.unsupported_anchors
            ],
        }


@dataclass(frozen=True)
class SourceProximityFact:
    coin_id: str
    provenance_status: str
    evidence_level: str
    source_types: tuple[str, ...]
    nearest_hop_count: int | None
    supported_value_msat: int
    unknown_value_msat: int
    coverage_ratio_ppm: int
    support_status: str
    evidence: Mapping[str, Any]

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "coin_id": self.coin_id,
            "provenance_status": self.provenance_status,
            "evidence_level": self.evidence_level,
            "source_types": list(self.source_types),
            "nearest_hop_count": self.nearest_hop_count,
            "supported_value_msat": self.supported_value_msat,
            "unknown_value_msat": self.unknown_value_msat,
            "coverage_ratio_ppm": self.coverage_ratio_ppm,
            "support_status": self.support_status,
            "evidence": dict(self.evidence),
        }


@dataclass(frozen=True)
class PassiveObserverEntity:
    entity_id: str
    node_ids: tuple[str, ...]
    edge_ids: tuple[str, ...]
    heuristics: tuple[str, ...]
    evidence_level: str
    linkage_score: int
    consequence_msat: int

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "entity_id": self.entity_id,
            "node_ids": list(self.node_ids),
            "edge_ids": list(self.edge_ids),
            "heuristics": list(self.heuristics),
            "evidence_level": self.evidence_level,
            "linkage_score": self.linkage_score,
            "consequence_msat": self.consequence_msat,
        }


@dataclass(frozen=True)
class PrivacyLinkageFinding:
    finding_id: str
    kind: str
    severity: str
    title: str
    detail: str
    evidence_level: str
    linkage_score: int
    consequence_msat: int
    edge_ids: tuple[str, ...]
    evidence: Mapping[str, Any]

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "id": self.finding_id,
            "kind": self.kind,
            "severity": self.severity,
            "title": self.title,
            "detail": self.detail,
            "evidence_level": self.evidence_level,
            "linkage_score": self.linkage_score,
            "consequence_msat": self.consequence_msat,
            "edge_ids": list(self.edge_ids),
            "evidence": dict(self.evidence),
        }


@dataclass(frozen=True)
class PrivacyLinkageGraph:
    nodes: Mapping[str, OwnedOutputNode]
    edges: tuple[PrivacyLinkageEdge, ...]
    transaction_tells: tuple[PrivacyTransactionTell, ...]
    adversary_views: tuple[AdversaryInferenceView, ...]
    source_proximity: tuple[SourceProximityFact, ...]
    observer_entities: tuple[PassiveObserverEntity, ...]
    findings: tuple[PrivacyLinkageFinding, ...]
    limitations: tuple[Mapping[str, Any], ...]
    analysed_transaction_count: int = 0
    scored_transaction_count: int = 0
    transaction_coverage_unknown_count: int = 0

    @property
    def linkage_score(self) -> int:
        return sum(edge.merged_cluster_count for edge in self.edges)

    @property
    def consequence_msat(self) -> int:
        return sum(edge.amount_msat for edge in self.edges if edge.new_linkage)

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "payload_schema_version": PRIVACY_LINKAGE_SCHEMA_VERSION,
            "redaction": "ai_export_safe",
            "local_only": True,
            "read_only": True,
            "advisory_only": True,
            "asset_scope": "bitcoin_utxo_only",
            "summary": {
                "node_count": len(self.nodes),
                "edge_count": len(self.edges),
                "observer_linkage_edge_count": sum(edge.observer_linkage for edge in self.edges),
                "local_fact_edge_count": sum(not edge.observer_linkage for edge in self.edges),
                "new_linkage_edge_count": sum(1 for edge in self.edges if edge.new_linkage),
                "linkage_score": self.linkage_score,
                "consequence_msat": self.consequence_msat,
                "observer_entity_count": len(self.observer_entities),
                "transaction_tell_count": len(self.transaction_tells),
                "analysed_transaction_count": self.analysed_transaction_count,
                "scored_transaction_count": self.scored_transaction_count,
                "transaction_coverage_unknown_count": self.transaction_coverage_unknown_count,
                "adversary_view_count": len(self.adversary_views),
                "source_proximity_coin_count": len(self.source_proximity),
                "source_proximity_known_coin_count": sum(
                    1
                    for fact in self.source_proximity
                    if fact.provenance_status != SOURCE_PROXIMITY_UNKNOWN
                ),
                "source_proximity_unknown_coin_count": sum(
                    1
                    for fact in self.source_proximity
                    if fact.provenance_status == SOURCE_PROXIMITY_UNKNOWN
                ),
                "source_proximity_supported_value_msat": sum(
                    fact.supported_value_msat for fact in self.source_proximity
                ),
                "source_proximity_unknown_value_msat": sum(
                    fact.unknown_value_msat for fact in self.source_proximity
                ),
                "counterparty_observation_count": sum(
                    1
                    for tell in self.transaction_tells
                    if tell.attribution == ATTRIBUTION_OBSERVED_FROM_COUNTERPARTY
                ),
                "wallet_grade_penalty_count": sum(
                    1 for tell in self.transaction_tells if tell.penalizes_wallet
                ),
                "finding_count": len(self.findings),
                "limitation_count": len(self.limitations),
            },
            "nodes": [
                self.nodes[node_id].to_redacted_payload()
                for node_id in sorted(self.nodes)
            ],
            "edges": [edge.to_redacted_payload() for edge in self.edges],
            "observer_entities": [
                entity.to_redacted_payload() for entity in self.observer_entities
            ],
            "transaction_tells": [
                tell.to_redacted_payload() for tell in self.transaction_tells
            ],
            "adversary_views": [
                view.to_redacted_payload() for view in self.adversary_views
            ],
            "source_proximity": [
                fact.to_redacted_payload() for fact in self.source_proximity
            ],
            "findings": [finding.to_redacted_payload() for finding in self.findings],
            "limitations": [dict(limitation) for limitation in self.limitations],
        }


@dataclass(frozen=True)
class PsbtPrivacyFinding:
    finding_id: str
    kind: str
    severity: str
    title: str
    detail: str
    evidence_level: str
    evidence: Mapping[str, Any]

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "id": self.finding_id,
            "kind": self.kind,
            "severity": self.severity,
            "title": self.title,
            "detail": self.detail,
            "evidence_level": self.evidence_level,
            "evidence": dict(self.evidence),
        }


@dataclass(frozen=True)
class PsbtAdversaryDelta:
    tier: str
    label: str
    evidence_level: str
    source: str
    cluster_merge_delta: int
    owned_input_cluster_count: int
    newly_exposed_component_count: int
    evidence: Mapping[str, Any]

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "tier": self.tier,
            "label": self.label,
            "evidence_level": self.evidence_level,
            "source": self.source,
            "cluster_merge_delta": self.cluster_merge_delta,
            "owned_input_cluster_count": self.owned_input_cluster_count,
            "newly_exposed_component_count": self.newly_exposed_component_count,
            "evidence": dict(self.evidence),
        }


@dataclass(frozen=True)
class PsbtWhatIfScenario:
    scenario: str
    evidence_level: str
    support_status: str
    cluster_merge_delta: int
    evidence: Mapping[str, Any]

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "scenario": self.scenario,
            "evidence_level": self.evidence_level,
            "support_status": self.support_status,
            "cluster_merge_delta": self.cluster_merge_delta,
            "evidence": dict(self.evidence),
        }


@dataclass(frozen=True)
class PsbtTransactionTell:
    tell_id: str
    kind: str
    attribution: str
    evidence_level: str
    source: str
    penalizes_wallet: bool
    evidence: Mapping[str, Any]

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "tell_id": self.tell_id,
            "kind": self.kind,
            "attribution": self.attribution,
            "evidence_level": self.evidence_level,
            "source": self.source,
            "penalizes_wallet": self.penalizes_wallet,
            "evidence": dict(self.evidence),
        }


@dataclass(frozen=True)
class PsbtPrivacyAnalysis:
    summary: Mapping[str, Any]
    cluster_merge: Mapping[str, Any]
    change_evidence: Mapping[str, Any]
    unknowns: Mapping[str, Any]
    transaction_tells: tuple[PsbtTransactionTell, ...]
    adversary_deltas: tuple[PsbtAdversaryDelta, ...]
    what_if: tuple[PsbtWhatIfScenario, ...]
    findings: tuple[PsbtPrivacyFinding, ...]
    limitations: tuple[Mapping[str, Any], ...]

    def to_redacted_payload(self) -> dict[str, Any]:
        return {
            "payload_schema_version": PSBT_PRIVACY_SCHEMA_VERSION,
            "redaction": "ai_export_safe",
            "local_only": True,
            "read_only": True,
            "advisory_only": True,
            "signing_supported": False,
            "broadcast_supported": False,
            "psbt_exposure": "reduced_findings_only",
            "summary": dict(self.summary),
            "cluster_merge": dict(self.cluster_merge),
            "change_evidence": dict(self.change_evidence),
            "unknowns": dict(self.unknowns),
            "transaction_tells": [
                tell.to_redacted_payload() for tell in self.transaction_tells
            ],
            "adversary_deltas": [
                delta.to_redacted_payload() for delta in self.adversary_deltas
            ],
            "what_if": [scenario.to_redacted_payload() for scenario in self.what_if],
            "findings": [finding.to_redacted_payload() for finding in self.findings],
            "limitations": [dict(limitation) for limitation in self.limitations],
        }


@dataclass(frozen=True)
class _DecodedPsbtInput:
    prev_txid: str
    vout: int
    sequence: int


@dataclass(frozen=True)
class _DecodedPsbtOutput:
    value_msat: int
    script_key: str
    is_op_return: bool


@dataclass(frozen=True)
class _DecodedPsbt:
    version: int
    locktime: int
    inputs: tuple[_DecodedPsbtInput, ...]
    outputs: tuple[_DecodedPsbtOutput, ...]
    unsigned_tx_clean: bool
    signature_material_present: bool
    features: Mapping[str, Any]


def _decode_psbt(psbt_text: str) -> _DecodedPsbt:
    # Keep inventory/privacy policy here; framing, v2 reconstruction and UTXO
    # validation belong to the same read-only decoder used by chain analysis.
    from .chain_analysis.psbt import decode_psbt_structure

    try:
        decoded = decode_psbt_structure(psbt_text)
    except AppError as exc:
        raise ValueError("PSBT structure or supplied prevout evidence is invalid") from exc
    return _DecodedPsbt(
        version=decoded["version"], locktime=decoded["locktime"],
        inputs=tuple(_DecodedPsbtInput(**row) for row in decoded["inputs"]),
        outputs=tuple(_DecodedPsbtOutput(**row) for row in decoded["outputs"]),
        unsigned_tx_clean=decoded["unsigned_tx_clean"],
        signature_material_present=decoded["signature_material_present"],
        features=extract_transaction_features({
            "version": decoded["version"], "locktime": decoded["locktime"],
            "vin": [{"txid": row["prev_txid"], "vout": row["vout"], "sequence": row["sequence"]} for row in decoded["inputs"]],
            "vout": [{"value": row["value_msat"] // 1000, "scriptpubkey": row["script_key"]} for row in decoded["outputs"]],
        }, source="psbt_proposal"),
    )


def build_privacy_linkage_graph(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    index: AnalysisIndex | None = None,
    analysis: Mapping[str, Any] | None = None,
) -> PrivacyLinkageGraph:
    """Read shared analysis and private authored scenarios in one snapshot.

    Callers supplying an existing index keep the surrounding read snapshot
    open while composing other projections, as the Privacy Mirror report does.
    """
    conn.execute("SAVEPOINT privacy_linkage_read")
    try:
        return _project_privacy_linkage_graph(conn, profile_id, index=index, analysis=analysis)
    finally:
        conn.execute("RELEASE SAVEPOINT privacy_linkage_read")


def _project_privacy_linkage_graph(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    index: AnalysisIndex | None,
    analysis: Mapping[str, Any] | None,
) -> PrivacyLinkageGraph:
    """Project the shared public analysis onto private owned-output relevance.

    This compatibility payload serves PSBT deltas and authored source-funds
    scenarios. Physical parsing, ownership hypotheses and structural findings
    are computed exclusively by the same engine as the investigation workbench.
    """
    index = index if index is not None else build_index(conn, profile_id)
    limitations: list[dict[str, Any]] = [
        {"code": "local_only_no_probe", "message": "No network request was made; analysis uses the immutable local observation snapshot.", "evidence_level": EVIDENCE_EXACT},
        {"code": "advisory_no_coin_selection", "message": "This graph is advisory and does not recommend which coins to spend.", "evidence_level": EVIDENCE_EXACT},
    ]
    nodes, canonical = _owned_nodes_from_index(index, limitations)
    if not nodes:
        if not any(item["code"] == "mixed_bitcoin_networks_require_selection" for item in limitations):
            limitations.append({"code": "no_owned_bitcoin_outputs", "message": "No unambiguous owned Bitcoin outputs are available.", "evidence_level": EVIDENCE_UNKNOWN})
        views = _build_adversary_views(conn, profile_id, {}, (), (), {})
        return PrivacyLinkageGraph({}, (), (), views, (), (), (), tuple(limitations))
    network = next(iter(nodes.values())).network
    if analysis is None:
        analysis = analyze_snapshot(index, {"observer": "public", "chain": "bitcoin", "network": network,
                                           "include_hypotheses": True, "include_relations": False,
                                           "node_limit": 2000, "edge_limit": 6000, "depth": 12})
    if analysis.get("snapshot_id") != index.snapshot_id or analysis.get("query", {}).get("observer") != "public":
        raise ValueError("Privacy linkage requires public analysis of the same immutable snapshot")
    selected = {row["id"] for row in analysis.get("nodes", ())}
    facts = _spend_facts_from_index(index, canonical, network)
    unavailable_roles = {node_id for fact in facts.values() if fact.input_node_ids for node_id in fact.output_node_ids
                         if nodes[node_id].branch_role == "unknown"}
    if unavailable_roles:
        limitations.append({"code": "change_role_unavailable", "message": "Private inventory branch evidence is unavailable for some outputs; no change role is invented.",
                            "evidence_level": EVIDENCE_UNKNOWN, "evidence": {"owned_output_count": len(unavailable_roles)}})
    observed = {index.nodes[ident]["txid"] for ident, fact in index.transaction_facts.items()
                if fact.get("transaction_ids") and index.nodes[ident].get("chain") == "bitcoin" and index.nodes[ident].get("network") == network}
    analysed = {index.nodes[ident]["txid"] for ident, fact in index.transaction_facts.items()
                if ident in selected and fact.get("complete") and fact.get("transaction_ids")
                and index.nodes[ident].get("chain") == "bitcoin" and index.nodes[ident].get("network") == network}
    if observed - analysed:
        limitations.append({"code": "transaction_analysis_coverage_incomplete", "message": "Some observations lack a complete selected Bitcoin graph; they do not count as analysed transactions.", "evidence_level": EVIDENCE_UNKNOWN, "evidence": {"transaction_count": len(observed - analysed)}})
    coverage = analysis.get("coverage", {})
    stopped = coverage.get("analytics", {}).get("stopped_reasons", ())
    if coverage.get("budget_exhausted") or stopped:
        limitations.append({"code": "shared_analysis_bounded", "message": "The shared graph analysis reached a work or result bound. Absence of another link is not proven.", "evidence_level": EVIDENCE_UNKNOWN,
                            "evidence": {"stopped_reasons": list(stopped), "budget_exhausted": bool(coverage.get("budget_exhausted"))}})
    collaboration_count = sum(fact.collaboration is not None for fact in facts.values())
    if collaboration_count:
        limitations.append({"code": "collaborative_ownership_uncertain", "message": "Collaborative co-spends remain physical facts; common-control assumptions do not cross them.", "evidence_level": EVIDENCE_UNKNOWN, "evidence": {"transaction_count": collaboration_count}})
    components = Components(nodes)
    edges: list[PrivacyLinkageEdge] = []
    by_node: dict[str, set[str]] = defaultdict(set)
    heuristics: dict[str, set[str]] = defaultdict(set)
    scores: dict[str, int] = {}

    def project_edge(row: Mapping[str, Any], *, kind: str, left: str, right: str,
                     observer_linkage: bool = True, txid: str | None = None) -> None:
        a, b = canonical[left], canonical[right]
        if a == b:
            return
        a, b = sorted((a, b))
        merged = components.union(a, b) if observer_linkage else False
        ident = str(row["id"]) + ":" + a + ":" + b
        edge = PrivacyLinkageEdge(ident, kind, str(row.get("rule") or row.get("code") or kind), a, b,
                                  EVIDENCE_DERIVED if observer_linkage else EVIDENCE_EXACT,
                                  "shared_chain_analysis", txid, min(nodes[a].amount_msat, nodes[b].amount_msat) if merged else 0,
                                  merged, int(merged), {"shared_evidence_id": row["id"], "rule_version": row.get("rule_version"),
                                  "premises": list(row.get("premises", ())), "collaboration": facts.get(txid).collaboration if txid in facts else None}, observer_linkage)
        edges.append(edge)
        scores[ident] = int(merged)
        if observer_linkage:
            for member in (a, b):
                by_node[member].add(ident)
                heuristics[member].add(edge.heuristic)

    kinds = {"reused_script_control": "address_reuse", "common_input_control": "common_input", "change_script_return": "change_output"}
    # Reuse precedes co-spend in this legacy delta view so a previously linked
    # pair is not described as a newly exposed pair when subsequently co-spent.
    hypothesis_edges = [row for row in analysis.get("edges", ()) if row.get("kind") == "hypothesis"]
    for row in sorted(hypothesis_edges, key=lambda row: (0 if row.get("rule") == "reused_script_control" else 1, row["id"])):
        if row["source"] in canonical and row["target"] in canonical:
            txid = None
            for ref in row.get("evidence", ()):
                subject = str(ref.get("reference") or "")
                if subject in index.nodes and index.nodes[subject].get("kind") == "transaction":
                    txid = index.nodes[subject]["txid"]
                    break
            project_edge(row, kind=kinds.get(row.get("rule"), "shared_cluster"), left=row["source"], right=row["target"], txid=txid)
    # Canonical clusters can connect two owned outputs through other observed
    # outputs. Preserve that transitive projection without re-running a rule.
    for cluster in analysis.get("clusters", ()):
        owned = [ident for ident in cluster["node_ids"] if ident in canonical]
        for other in owned[1:]:
            if components.find(canonical[owned[0]]) != components.find(canonical[other]):
                project_edge(cluster, kind="shared_cluster", left=owned[0], right=other)
    for finding in analysis.get("findings", ()):
        if finding.get("code") != "observed_co_spend":
            continue
        transaction_ids = [ident for ident in finding["node_ids"] if index.nodes[ident].get("kind") == "transaction"]
        txid = index.nodes[transaction_ids[0]]["txid"] if transaction_ids else None
        if txid not in facts or not facts[txid].collaboration:
            continue
        owned = [ident for ident in finding["node_ids"] if ident in canonical]
        for other in owned[1:]:
            project_edge(finding, kind="common_input", left=owned[0], right=other, observer_linkage=False, txid=txid)
    tells = _transaction_tells_from_analysis(index, analysis, facts, network)
    entities = _build_observer_entities(nodes, edges, components, by_node, heuristics, scores)
    source_proximity = _build_source_proximity(conn, profile_id, nodes, limitations)
    views = _build_adversary_views(conn, profile_id, nodes, edges, entities, facts)
    return PrivacyLinkageGraph(nodes, tuple(edges), tuple(tells), tuple(views), tuple(source_proximity), tuple(entities),
                               tuple(_build_findings(edges)), tuple(limitations), len(analysed),
                               len(analysed | {tell.txid for tell in tells if tell.penalizes_wallet}), len(observed - analysed))


def analyze_psbt_privacy(
    conn: sqlite3.Connection,
    profile_id: str,
    psbt_text: str,
) -> PsbtPrivacyAnalysis:
    """Decode a PSBT locally and score its redacted pre-broadcast privacy deltas."""

    decoded = _decode_psbt(psbt_text)
    limitations: list[dict[str, Any]] = [
        {
            "code": "local_only_no_probe",
            "message": "No network request was made; PSBT analysis used local inventory and the unsigned transaction only.",
            "evidence_level": EVIDENCE_EXACT,
        },
        {
            "code": "redacted_psbt_payload",
            "message": "Raw PSBT bytes, addresses, scripts, descriptors, xpubs, backend configuration, raw_json, branch/index values, and derivation paths are omitted.",
            "evidence_level": EVIDENCE_EXACT,
        },
        {
            "code": "advisory_no_input_proposal",
            "message": "This analysis is read-only and does not propose transaction inputs or outputs.",
            "evidence_level": EVIDENCE_EXACT,
        },
    ]
    graph = build_privacy_linkage_graph(conn, profile_id)
    baseline_bounded = any(item.get("code") == "shared_analysis_bounded" and (
        item.get("evidence", {}).get("budget_exhausted")
        or "hypothesis_budget" in item.get("evidence", {}).get("stopped_reasons", ())
    ) for item in graph.limitations)
    if baseline_bounded:
        limitations.append({"code": "historical_linkage_bounded", "message": "The historical observer model reached an analysis bound; additional existing links can change the proposed merge delta.", "evidence_level": EVIDENCE_UNKNOWN})
    outpoint_to_node = {
        (node.txid, node.vout): node for node in graph.nodes.values()
    }
    known_input_nodes: list[OwnedOutputNode] = []
    unknown_input_count = 0
    for psbt_input in decoded.inputs:
        node = outpoint_to_node.get((psbt_input.prev_txid, psbt_input.vout))
        if node is None:
            unknown_input_count += 1
            continue
        known_input_nodes.append(node)
    known_input_node_ids = tuple(sorted({node.node_id for node in known_input_nodes}))
    known_wallet_ids = tuple(sorted({node.wallet_id for node in known_input_nodes}))
    components, node_to_component = _build_inference_components(
        graph.nodes,
        graph.edges,
    )
    input_component_ids = tuple(
        sorted(
            {
                node_to_component[node.node_id].component_id
                for node in known_input_nodes
                if node.node_id in node_to_component
            }
        )
    )
    cluster_merge_delta = max(0, len(input_component_ids) - 1)
    cluster_evidence_level = (
        EVIDENCE_UNKNOWN if unknown_input_count or baseline_bounded else EVIDENCE_DERIVED
    )

    script_to_nodes: dict[str, list[OwnedOutputNode]] = defaultdict(list)
    for node in graph.nodes.values():
        if node.script_key:
            script_to_nodes[node.script_key].append(node)
    owned_output_count = 0
    receive_reuse_output_count = 0
    change_like_output_count = 0
    unknown_output_count = 0
    receive_reuse_component_ids: set[str] = set()
    change_items: list[dict[str, Any]] = []
    change_item_levels: list[str] = []
    for output in decoded.outputs:
        matches = script_to_nodes.get(output.script_key, [])
        if not matches:
            unknown_output_count += 1
            continue
        owned_output_count += 1
        roles = {node.branch_role for node in matches}
        levels = [node.branch_evidence_level for node in matches]
        item_level = _combine_evidence_levels(levels)
        change_item_levels.append(item_level)
        if "receive" in roles:
            receive_reuse_output_count += 1
            for node in matches:
                component = node_to_component.get(node.node_id)
                if component is not None:
                    receive_reuse_component_ids.add(component.component_id)
            change_items.append(
                {
                    "kind": "existing_receive_output_reuse",
                    "evidence_level": item_level,
                    "matched_local_output_count": len(matches),
                    "change_evidence": "receive_branch_match",
                }
            )
        elif "change" in roles:
            change_like_output_count += 1
            change_items.append(
                {
                    "kind": "existing_change_output_match",
                    "evidence_level": item_level,
                    "matched_local_output_count": len(matches),
                    "change_evidence": _combine_change_evidence(matches),
                }
            )
        else:
            change_items.append(
                {
                    "kind": "owned_output_role_unknown",
                    "evidence_level": EVIDENCE_UNKNOWN,
                    "matched_local_output_count": len(matches),
                    "change_evidence": CHANGE_EVIDENCE_UNAVAILABLE,
                }
            )
            change_item_levels.append(EVIDENCE_UNKNOWN)
    receive_reuse_delta = len(
        receive_reuse_component_ids.difference(input_component_ids)
    )
    change_evidence_level = (
        _combine_evidence_levels(change_item_levels)
        if change_item_levels
        else EVIDENCE_UNKNOWN
    )

    if unknown_input_count:
        limitations.append(
            {
                "code": "psbt_inputs_not_in_local_inventory",
                "message": "Some PSBT inputs do not match local watch-only Bitcoin UTXO inventory, so ownership and linkage coverage is incomplete.",
                "evidence_level": EVIDENCE_UNKNOWN,
                "evidence": {"input_count": unknown_input_count},
            }
        )
    if decoded.signature_material_present:
        limitations.append(
            {
                "code": "psbt_signature_material_present",
                "message": "The PSBT input maps contain signing or finalization material; analysis still reads only transaction graph fields.",
                "evidence_level": EVIDENCE_EXACT,
            }
        )
    if not decoded.unsigned_tx_clean:
        limitations.append(
            {
                "code": "unsigned_transaction_not_clean",
                "message": "The PSBT unsigned transaction carried script or witness bytes; graph analysis continued from decoded vin/vout only.",
                "evidence_level": EVIDENCE_UNKNOWN,
            }
        )

    transaction_tells = tuple(
        _build_psbt_transaction_tells(
            decoded,
            known_input_nodes,
            unknown_input_count,
        )
    )
    findings = tuple(
        _build_psbt_findings(
            cluster_merge_delta=cluster_merge_delta,
            cluster_evidence_level=cluster_evidence_level,
            unknown_input_count=unknown_input_count,
            receive_reuse_output_count=receive_reuse_output_count,
            receive_reuse_delta=receive_reuse_delta,
            signature_material_present=decoded.signature_material_present,
            unsigned_tx_clean=decoded.unsigned_tx_clean,
        )
    )
    adversary_deltas = tuple(
        _build_psbt_adversary_deltas(
            graph,
            input_component_ids,
            cluster_merge_delta,
            unknown_input_count,
        )
    )
    what_if = tuple(
        _build_psbt_what_if(
            input_component_ids=input_component_ids,
            cluster_merge_delta=cluster_merge_delta,
            cluster_evidence_level=cluster_evidence_level,
            receive_reuse_output_count=receive_reuse_output_count,
            receive_reuse_delta=receive_reuse_delta,
        )
    )
    blast_radius_score = cluster_merge_delta + receive_reuse_delta
    cluster_merge = {
        "kind": "psbt_common_input_overlay",
        "evidence_level": cluster_evidence_level,
        "owned_input_count": len(known_input_nodes),
        "owned_input_node_count": len(known_input_node_ids),
        "owned_input_cluster_count": len(input_component_ids),
        "owned_input_wallet_count": len(known_wallet_ids),
        "cluster_merge_delta": cluster_merge_delta,
        "affected_cluster_ids": list(input_component_ids),
        "affected_node_ids": list(known_input_node_ids),
        "unknown_input_count": unknown_input_count,
    }
    change_evidence = {
        "evidence_level": change_evidence_level,
        "owned_output_match_count": owned_output_count,
        "receive_reuse_output_count": receive_reuse_output_count,
        "change_like_output_count": change_like_output_count,
        "unknown_output_count": unknown_output_count,
        "receive_reuse_cluster_delta": receive_reuse_delta,
        "items": change_items,
    }
    unknowns = {
        "evidence_level": EVIDENCE_UNKNOWN if unknown_input_count or baseline_bounded else EVIDENCE_EXACT,
        "input_count": unknown_input_count,
        "output_without_local_match_count": unknown_output_count,
        "coverage_complete": unknown_input_count == 0 and not baseline_bounded,
    }
    summary = {
        "evidence_level": (
            EVIDENCE_UNKNOWN if unknown_input_count or baseline_bounded else EVIDENCE_DERIVED
        ),
        "decode_status": "decoded",
        "input_count": len(decoded.inputs),
        "output_count": len(decoded.outputs),
        "known_owned_input_count": len(known_input_nodes),
        "unknown_input_count": unknown_input_count,
        "owned_input_cluster_count": len(input_component_ids),
        "cluster_merge_delta": cluster_merge_delta,
        "receive_reuse_output_count": receive_reuse_output_count,
        "receive_reuse_cluster_delta": receive_reuse_delta,
        "blast_radius_score": blast_radius_score,
        "transaction_tell_count": len(transaction_tells),
        "adversary_delta_count": len(adversary_deltas),
        "finding_count": len(findings),
        "limitation_count": len(limitations),
        "signature_material_present": decoded.signature_material_present,
    }
    return PsbtPrivacyAnalysis(
        summary=summary,
        cluster_merge=cluster_merge,
        change_evidence=change_evidence,
        unknowns=unknowns,
        transaction_tells=transaction_tells,
        adversary_deltas=adversary_deltas,
        what_if=what_if,
        findings=findings,
        limitations=tuple(limitations),
    )


def _combine_change_evidence(nodes: Sequence[OwnedOutputNode]) -> str:
    values = {node.change_evidence for node in nodes}
    if CHANGE_EVIDENCE_GROUND_TRUTH in values:
        return CHANGE_EVIDENCE_GROUND_TRUTH
    if CHANGE_EVIDENCE_IMPORTED in values:
        return CHANGE_EVIDENCE_IMPORTED
    if CHANGE_EVIDENCE_HEURISTIC in values:
        return CHANGE_EVIDENCE_HEURISTIC
    return CHANGE_EVIDENCE_UNAVAILABLE


def _build_psbt_transaction_tells(
    decoded: _DecodedPsbt,
    known_input_nodes: Sequence[OwnedOutputNode],
    unknown_input_count: int,
) -> list[PsbtTransactionTell]:
    tells: list[PsbtTransactionTell] = []
    attribution = ATTRIBUTION_EMITTED_BY_YOU if known_input_nodes else EVIDENCE_UNKNOWN

    def add(
        kind: str,
        source: str,
        evidence_level: str,
        penalizes_wallet: bool,
        evidence: Mapping[str, Any],
    ) -> None:
        tells.append(
            PsbtTransactionTell(
                tell_id=f"{kind}:psbt:{len(tells)}",
                kind=kind,
                attribution=attribution,
                evidence_level=evidence_level,
                source=source,
                penalizes_wallet=penalizes_wallet,
                evidence=dict(evidence),
            )
        )

    if len(decoded.inputs) > 1:
        add(
            "sender_common_input",
            "decoded_psbt_unsigned_tx",
            EVIDENCE_UNKNOWN if unknown_input_count else EVIDENCE_DERIVED,
            bool(known_input_nodes),
            {
                "input_count": len(decoded.inputs),
                "known_owned_input_count": len(known_input_nodes),
                "unknown_input_count": unknown_input_count,
            },
        )
    structural = {row["code"] for row in evaluate_features(decoded.features)}
    if "explicit_rbf_signal" in structural:
        add(
            "sender_rbf",
            "decoded_psbt_unsigned_tx",
            EVIDENCE_EXACT,
            bool(known_input_nodes),
            {"rbf_signaled": True},
        )
    if "op_return_output" in structural:
        add(
            "op_return_output",
            "decoded_psbt_unsigned_tx",
            EVIDENCE_EXACT,
            bool(known_input_nodes),
            {"op_return_present": True},
        )
    input_value_msat = sum(node.amount_msat for node in known_input_nodes)
    output_value_msat = sum(output.value_msat for output in decoded.outputs)
    fee_known = (
        bool(decoded.inputs)
        and unknown_input_count == 0
        and len(known_input_nodes) == len(decoded.inputs)
        and input_value_msat >= output_value_msat
    )
    add(
        "fee_observation",
        "decoded_psbt_amounts",
        EVIDENCE_EXACT if fee_known else EVIDENCE_UNKNOWN,
        False,
        {
            "fee_known": fee_known,
            "fee_value_redacted": True,
            "unknown_input_count": unknown_input_count,
        },
    )
    return tells


def _build_psbt_findings(
    *,
    cluster_merge_delta: int,
    cluster_evidence_level: str,
    unknown_input_count: int,
    receive_reuse_output_count: int,
    receive_reuse_delta: int,
    signature_material_present: bool,
    unsigned_tx_clean: bool,
) -> list[PsbtPrivacyFinding]:
    findings: list[PsbtPrivacyFinding] = []

    def add(
        *,
        finding_id: str,
        kind: str,
        severity: str,
        title: str,
        detail: str,
        evidence_level: str,
        evidence: Mapping[str, Any],
    ) -> None:
        findings.append(
            PsbtPrivacyFinding(
                finding_id=finding_id,
                kind=kind,
                severity=severity,
                title=title,
                detail=detail,
                evidence_level=evidence_level,
                evidence=dict(evidence),
            )
        )

    if cluster_merge_delta:
        add(
            finding_id="psbt_cluster_merge",
            kind="cluster_merge",
            severity="warning",
            title="PSBT may link local clusters under a common-input assumption",
            detail=(
                f"Assuming an ordinary non-collaborative spend, the decoded transaction "
                f"would create {cluster_merge_delta} common-input cluster merge(s) among locally known inputs."
            ),
            evidence_level=cluster_evidence_level,
            evidence={"cluster_merge_delta": cluster_merge_delta},
        )
    if unknown_input_count:
        add(
            finding_id="psbt_unknown_inputs",
            kind="unknown_inputs",
            severity="warning",
            title="PSBT has inputs outside local inventory",
            detail=(
                f"{unknown_input_count} input(s) do not match local watch-only "
                "Bitcoin UTXO inventory, so the graph delta is incomplete."
            ),
            evidence_level=EVIDENCE_UNKNOWN,
            evidence={"unknown_input_count": unknown_input_count},
        )
    if receive_reuse_output_count:
        add(
            finding_id="psbt_existing_receive_reuse",
            kind="receive_reuse",
            severity="warning" if receive_reuse_delta else "info",
            title="PSBT output matches an existing local receive script",
            detail=(
                f"{receive_reuse_output_count} output(s) match existing local "
                "receive evidence; the reduced payload omits script data."
            ),
            evidence_level=EVIDENCE_EXACT,
            evidence={
                "receive_reuse_output_count": receive_reuse_output_count,
                "receive_reuse_cluster_delta": receive_reuse_delta,
            },
        )
    if signature_material_present:
        add(
            finding_id="psbt_signature_material_present",
            kind="signature_material_present",
            severity="info",
            title="PSBT input maps contain signing material",
            detail=(
                "The parser detected input-map signing or finalization material; "
                "the privacy analysis used only graph fields."
            ),
            evidence_level=EVIDENCE_EXACT,
            evidence={"signature_material_present": True},
        )
    if not unsigned_tx_clean:
        add(
            finding_id="psbt_unsigned_tx_not_clean",
            kind="unsigned_tx_not_clean",
            severity="warning",
            title="Unsigned transaction contains script or witness bytes",
            detail=(
                "The decoded unsigned transaction was not empty-script clean; "
                "the graph delta may be incomplete."
            ),
            evidence_level=EVIDENCE_UNKNOWN,
            evidence={"unsigned_tx_clean": False},
        )
    return findings


def _build_psbt_adversary_deltas(
    graph: PrivacyLinkageGraph,
    input_component_ids: Sequence[str],
    cluster_merge_delta: int,
    unknown_input_count: int,
) -> list[PsbtAdversaryDelta]:
    input_components = set(input_component_ids)
    evidence_level = EVIDENCE_UNKNOWN if unknown_input_count else EVIDENCE_DERIVED
    deltas: list[PsbtAdversaryDelta] = []
    for view in graph.adversary_views:
        exposed_components = {cluster.cluster_id for cluster in view.clusters}
        if view.tier == ADVERSARY_PASSIVE_CHAIN:
            newly_exposed = len(input_components.difference(exposed_components))
            visible_cluster_delta = cluster_merge_delta
        elif input_components.intersection(exposed_components):
            newly_exposed = len(input_components.difference(exposed_components))
            visible_cluster_delta = cluster_merge_delta
        else:
            newly_exposed = 0
            visible_cluster_delta = 0
        deltas.append(
            PsbtAdversaryDelta(
                tier=view.tier,
                label=view.label,
                evidence_level=evidence_level,
                source="psbt_common_input_overlay",
                cluster_merge_delta=visible_cluster_delta,
                owned_input_cluster_count=len(input_components),
                newly_exposed_component_count=newly_exposed,
                evidence={
                    "current_exposed_cluster_count": view.summary.get(
                        "exposed_cluster_count", 0
                    ),
                    "unknown_input_count": unknown_input_count,
                    "model_assumption_count": len(view.model_assumptions),
                },
            )
        )
    return deltas


def _build_psbt_what_if(
    *,
    input_component_ids: Sequence[str],
    cluster_merge_delta: int,
    cluster_evidence_level: str,
    receive_reuse_output_count: int,
    receive_reuse_delta: int,
) -> list[PsbtWhatIfScenario]:
    return [
        PsbtWhatIfScenario(
            scenario="fresh_receive_output",
            evidence_level=EVIDENCE_DERIVED,
            support_status=LOCAL_SUPPORT_SUPPORTED,
            cluster_merge_delta=0,
            evidence={
                "modeled_output_reuse": False,
                "assumption": "A fresh receive output is modeled as absent from the current local graph.",
            },
        ),
        PsbtWhatIfScenario(
            scenario="existing_receive_reuse",
            evidence_level=(
                EVIDENCE_EXACT if receive_reuse_output_count else EVIDENCE_UNKNOWN
            ),
            support_status=(
                LOCAL_SUPPORT_SUPPORTED
                if receive_reuse_output_count
                else LOCAL_SUPPORT_NOT_SUPPORTED
            ),
            cluster_merge_delta=receive_reuse_delta,
            evidence={
                "matched_existing_receive_output_count": receive_reuse_output_count,
                "receive_reuse_cluster_delta": receive_reuse_delta,
            },
        ),
        PsbtWhatIfScenario(
            scenario="hypothetical_input_consolidation",
            evidence_level=cluster_evidence_level,
            support_status=(
                LOCAL_SUPPORT_SUPPORTED
                if len(input_component_ids) > 1
                else LOCAL_SUPPORT_NOT_SUPPORTED
            ),
            cluster_merge_delta=cluster_merge_delta,
            evidence={
                "owned_input_cluster_count": len(input_component_ids),
                "cluster_merge_delta": cluster_merge_delta,
            },
        ),
    ]


def _owned_nodes_from_index(index: AnalysisIndex, limitations: list[dict[str, Any]]) -> tuple[dict[str, OwnedOutputNode], dict[str, str]]:
    inventory = {ident: node for ident, node in index.nodes.items() if node.get("kind") == "output" and index.output_facts.get(ident, {}).get("inventory_observation_count")}
    networks = {node["network"] for node in inventory.values() if node.get("chain") == "bitcoin" and node.get("asset") == "BTC"}
    if len(networks) > 1:
        limitations.append({"code": "mixed_bitcoin_networks_require_selection", "message": "The PSBT/privacy inventory projection requires one Bitcoin network; the chain workbench retains all separate domains.", "evidence_level": EVIDENCE_EXACT, "evidence": {"network_count": len(networks)}})
        return {}, {}
    nodes, canonical = {}, {}
    ambiguous = ignored = 0
    for ident, row in inventory.items():
        if row.get("chain") != "bitcoin" or row.get("asset") != "BTC":
            ignored += 1
            continue
        facts = index.output_facts[ident]
        if not facts.get("ownership_known") or facts.get("ownership_ambiguous") or len(row.get("wallet_ids", ())) != 1 or row.get("status") in {"conflicting", "stale"} or row.get("amount_msat") is None:
            ambiguous += 1
            continue
        txid, vout = row["outpoint"].rsplit(":", 1)
        spends = [index.edges[eid]["target"] for eid in index.outgoing.get(ident, ()) if index.edges[eid]["kind"] == "spends" and index.edges[eid].get("status") not in {"conflicting", "stale"}]
        spent_by = index.nodes[spends[0]].get("txid") if len(spends) == 1 else None
        node_id = row["outpoint"]
        nodes[node_id] = OwnedOutputNode(node_id, txid, int(vout), row["wallet_ids"][0], int(row["amount_msat"]), "BTC", spent_by,
                                        row["network"], facts.get("branch_role") or "unknown", facts.get("branch_evidence_level") or EVIDENCE_UNKNOWN,
                                        facts.get("change_evidence") or CHANGE_EVIDENCE_UNAVAILABLE, facts.get("branch_source") or "branch_metadata_unavailable",
                                        bool(row.get("address")), bool(facts.get("script")), row.get("address"), facts.get("script"))
        canonical[ident] = node_id
    if ignored:
        limitations.append({"code": "non_bitcoin_utxos_ignored", "message": "The PSBT inventory projection covers Bitcoin outputs only.", "evidence_level": EVIDENCE_EXACT, "evidence": {"ignored_count": ignored}})
    if ambiguous:
        limitations.append({"code": "ambiguous_owned_outpoints_ignored", "message": "Ambiguous or conflicting owned outputs cannot establish a private wallet match.", "evidence_level": EVIDENCE_UNKNOWN, "evidence": {"ignored_count": ambiguous}})
    return nodes, canonical


@dataclass
class _SpendFact:
    txid: str
    input_node_ids: set[str] = field(default_factory=set)
    output_node_ids: set[str] = field(default_factory=set)
    collaboration: dict[str, Any] | None = None
    network: str | None = None


def _spend_facts_from_index(index: AnalysisIndex, canonical: Mapping[str, str], network: str) -> dict[str, _SpendFact]:
    facts = {}
    for ident, row in index.nodes.items():
        if row.get("kind") != "transaction" or row.get("chain") != "bitcoin" or row.get("network") != network:
            continue
        fact = index.transaction_facts.get(ident, {})
        facts[row["txid"]] = _SpendFact(row["txid"],
            {canonical[index.edges[eid]["source"]] for eid in index.incoming.get(ident, ()) if index.edges[eid]["kind"] == "spends" and index.edges[eid]["source"] in canonical},
            {canonical[index.edges[eid]["target"]] for eid in index.outgoing.get(ident, ()) if index.edges[eid]["kind"] == "creates" and index.edges[eid]["target"] in canonical},
            thaw(fact.get("collaboration")), network)
    return facts


def _table_columns(conn: sqlite3.Connection, table: str) -> set[str]:
    try:
        rows = conn.execute(f"PRAGMA table_info({table})").fetchall()
    except sqlite3.DatabaseError:
        return set()
    columns: set[str] = set()
    for row in rows:
        try:
            columns.add(str(row["name"]))
        except (KeyError, TypeError, IndexError):
            columns.add(str(row[1]))
    return columns


def _privacy_transaction_rows(
    conn: sqlite3.Connection, profile_id: str, network: str | None,
    *, include_excluded: bool = True,
) -> list[dict[str, Any]]:
    """Bitcoin/network evidence; accounting exclusion is not a chain tombstone."""
    columns = _table_columns(conn, "transactions")
    if "profile_id" not in columns:
        return []
    wallet_columns = _table_columns(conn, "wallets")
    joined = "wallet_id" in columns and {"id", "kind", "config_json"}.issubset(wallet_columns)
    wallet_select = (
        "w.kind AS wallet_kind, w.config_json AS wallet_config_json"
        if joined else "NULL AS wallet_kind, NULL AS wallet_config_json"
    )
    wallet_join = "LEFT JOIN wallets w ON w.id = t.wallet_id" if joined else ""
    excluded = "AND t.excluded = 0" if not include_excluded and "excluded" in columns else ""
    rows = conn.execute(
        f"SELECT t.*, {wallet_select} FROM transactions t {wallet_join} "
        f"WHERE t.profile_id = ? {excluded} ORDER BY t.external_id ASC",
        (profile_id,),
    ).fetchall()
    selected = []
    for stored in rows:
        row = {key: None for key in ("id", "external_id", "direction", "fee", "raw_json", "privacy_boundary", "review_status", "counterparty")}
        row.update(dict(stored))
        try:
            scope = resolve_protocol_scope(row)
        except (ValueError, AppError):
            continue
        if scope.protocol_chain != "bitcoin" or (network is not None and scope.network != network):
            continue
        if normalize_asset_code(row.get("asset") or "BTC") != "BTC":
            continue
        selected.append(row)
    return selected


def _transaction_tells_from_analysis(index: AnalysisIndex, analysis: Mapping[str, Any], facts: Mapping[str, _SpendFact], network: str) -> list[PrivacyTransactionTell]:
    tells = []
    kinds = {"explicit_rbf_signal": "sender_rbf", "op_return_output": "op_return_output", "rounded_fee_rate": "fee_fingerprint", "observed_co_spend": "sender_common_input"}
    for finding in analysis.get("findings", ()):
        kind = kinds.get(finding.get("code"))
        if not kind:
            continue
        for ident in finding.get("node_ids", ()):
            node = index.nodes.get(ident, {})
            if node.get("kind") != "transaction" or node.get("chain") != "bitcoin" or node.get("network") != network:
                continue
            fact = index.transaction_facts.get(ident, {})
            if not fact.get("transaction_ids"):
                continue
            if kind == "sender_common_input" and fact.get("collaboration"):
                continue
            txid = node["txid"]
            directions = set(fact.get("directions", ()))
            incoming = bool(directions & {"inbound", "receive", "received", "deposit", "income", "buy"})
            outgoing = bool(directions & {"outbound", "send", "sent", "withdrawal", "sell"})
            emitted = outgoing if incoming != outgoing else bool(facts.get(txid) and facts[txid].input_node_ids)
            attribution = ATTRIBUTION_EMITTED_BY_YOU if emitted else ATTRIBUTION_OBSERVED_FROM_COUNTERPARTY
            evidence = {"shared_finding_id": finding["id"], "attribution_source": "local_owner_observation", "attribution_evidence_level": EVIDENCE_DERIVED}
            if kind == "fee_fingerprint":
                rate = next((row["value"] for row in fact.get("features", {}).get("features", ()) if row["code"] == "fee_rate"), {})
                if rate.get("vsize"):
                    evidence.update(sat_vb=(rate["fee_sats"] + rate["vsize"] // 2) // rate["vsize"], pattern="rounded_fee_rate")
            tells.append(PrivacyTransactionTell(str(finding["id"]), txid, kind, attribution, EVIDENCE_DERIVED, "shared_chain_analysis", emitted, evidence))
    return tells


@dataclass(frozen=True)
class _SourceReach:
    anchor_key: str
    source_type: str
    txid: str
    hop_count: int
    supported_value_msat: int | None
    evidence_level: str


@dataclass(frozen=True)
class _ReviewedFundingEdge:
    from_txid: str
    to_txid: str
    supported_value_msat: int | None
    evidence_level: str


def _int_or_none(value: Any) -> int | None:
    if value in (None, ""):
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _nonnegative_int_or_none(value: Any) -> int | None:
    parsed = _int_or_none(value)
    if parsed is None:
        return None
    return max(0, parsed)


def _source_proximity_unknown(
    node: OwnedOutputNode,
    *,
    reason: str,
) -> SourceProximityFact:
    return SourceProximityFact(
        coin_id=node.node_id,
        provenance_status=SOURCE_PROXIMITY_UNKNOWN,
        evidence_level=EVIDENCE_UNKNOWN,
        source_types=(),
        nearest_hop_count=None,
        supported_value_msat=0,
        unknown_value_msat=node.amount_msat,
        coverage_ratio_ppm=0,
        support_status=LOCAL_SUPPORT_NOT_SUPPORTED,
        evidence={
            "reason": reason,
            "coverage_gap": True,
            "model_scope": "reviewed_local_source_funds_only",
        },
    )


def _source_link_supported_amount(row: Mapping[str, Any]) -> int | None:
    for key in ("allocation_amount", "source_amount", "to_amount"):
        amount = _nonnegative_int_or_none(row[key])
        if amount is not None:
            return amount
    return None


def _tx_link_supported_amount(row: Mapping[str, Any]) -> int | None:
    for key in ("allocation_amount", "to_amount"):
        amount = _nonnegative_int_or_none(row[key])
        if amount is not None:
            return amount
    return None


def _source_proximity_tables_available(conn: sqlite3.Connection) -> tuple[bool, str | None]:
    source_columns = _table_columns(conn, "source_funds_sources")
    link_columns = _table_columns(conn, "source_funds_links")
    transaction_columns = _table_columns(conn, "transactions")
    required_sources = {"id", "profile_id", "source_type", "asset", "amount", "review_state"}
    required_links = {
        "id",
        "profile_id",
        "from_source_id",
        "from_transaction_id",
        "to_transaction_id",
        "state",
        "confidence",
        "allocation_amount",
        "asset",
    }
    required_transactions = {"id", "profile_id", "external_id", "amount", "asset"}
    if not required_sources.issubset(source_columns):
        return False, "source_funds_sources_unavailable"
    if not required_links.issubset(link_columns):
        return False, "source_funds_links_unavailable"
    if not required_transactions.issubset(transaction_columns):
        return False, "source_funds_transaction_amounts_unavailable"
    return True, None


def _load_source_reaches_and_edges(
    conn: sqlite3.Connection,
    profile_id: str,
    allowed_transaction_ids: set[str],
) -> tuple[list[_SourceReach], dict[str, list[_ReviewedFundingEdge]], str | None]:
    available, reason = _source_proximity_tables_available(conn)
    if not available:
        return [], {}, reason
    rows = conn.execute(
        """
        SELECT l.id AS link_id,
               l.from_source_id,
               l.from_transaction_id,
               l.to_transaction_id,
               l.confidence,
               l.allocation_amount,
               l.asset AS link_asset,
               s.source_type,
               s.asset AS source_asset,
               s.amount AS source_amount,
               to_tx.external_id AS to_external_id,
               to_tx.amount AS to_amount,
               to_tx.asset AS to_asset,
               from_tx.external_id AS from_external_id
        FROM source_funds_links l
        LEFT JOIN source_funds_sources s
          ON s.profile_id = l.profile_id AND s.id = l.from_source_id
        LEFT JOIN transactions to_tx
          ON to_tx.profile_id = l.profile_id AND to_tx.id = l.to_transaction_id
        LEFT JOIN transactions from_tx
          ON from_tx.profile_id = l.profile_id AND from_tx.id = l.from_transaction_id
        WHERE l.profile_id = ?
          AND l.state = 'reviewed'
          AND (s.review_state IS NULL OR s.review_state = 'reviewed')
        ORDER BY l.id ASC
        """,
        (profile_id,),
    ).fetchall()
    initial_reaches: list[_SourceReach] = []
    source_by_anchor: dict[str, str] = {}
    source_allocated: dict[str, int] = defaultdict(int)
    source_capacity: dict[str, int] = {}
    outgoing: dict[str, list[_ReviewedFundingEdge]] = defaultdict(list)
    for row in rows:
        if row["to_transaction_id"] not in allowed_transaction_ids:
            continue
        if row["from_transaction_id"] and row["from_transaction_id"] not in allowed_transaction_ids:
            continue
        link_asset = normalize_asset_code(row["link_asset"])
        to_asset = normalize_asset_code(row["to_asset"])
        if link_asset != "BTC" or to_asset != "BTC":
            continue
        evidence_level = _confidence_to_evidence_level(row["confidence"])
        to_txid = _normalize_txid(row["to_external_id"])
        if not to_txid:
            continue
        if row["from_source_id"]:
            source_type = str(row["source_type"] or "unknown").strip().lower() or "unknown"
            source_asset = normalize_asset_code(row["source_asset"])
            if source_asset != "BTC":
                continue
            source_id = str(row["from_source_id"])
            anchor_key = f"source:{row['link_id']}"
            source_by_anchor[anchor_key] = source_id
            source_allocated[source_id] += _source_link_supported_amount(row) or 0
            capacity = _nonnegative_int_or_none(row["source_amount"])
            if capacity is not None:
                source_capacity[source_id] = capacity
            initial_reaches.append(
                _SourceReach(
                    anchor_key=f"source:{row['link_id']}",
                    source_type=source_type,
                    txid=to_txid,
                    hop_count=0,
                    supported_value_msat=_source_link_supported_amount(row),
                    evidence_level=evidence_level,
                )
            )
        elif row["from_transaction_id"]:
            from_txid = _normalize_txid(row["from_external_id"])
            if not from_txid:
                continue
            outgoing[from_txid].append(
                _ReviewedFundingEdge(
                    from_txid=from_txid,
                    to_txid=to_txid,
                    supported_value_msat=_tx_link_supported_amount(row),
                    evidence_level=evidence_level,
                )
            )
    overcommitted = {
        source_id for source_id, capacity in source_capacity.items()
        if source_allocated[source_id] > capacity
    }
    initial_reaches = [
        replace(reach, supported_value_msat=None)
        if source_by_anchor[reach.anchor_key] in overcommitted else reach
        for reach in initial_reaches
    ]
    return initial_reaches, outgoing, None


def _propagate_source_reaches(
    initial_reaches: Sequence[_SourceReach],
    outgoing: Mapping[str, Sequence[_ReviewedFundingEdge]],
) -> dict[str, list[_SourceReach]]:
    by_tx: dict[str, list[_SourceReach]] = defaultdict(list)
    best: dict[tuple[str, str], _SourceReach] = {}
    queue: deque[_SourceReach] = deque()
    for reach in initial_reaches:
        key = (reach.txid, reach.anchor_key)
        current = best.get(key)
        if current is not None and current.supported_value_msat is not None:
            if (
                reach.hop_count > current.hop_count
                or (reach.supported_value_msat or 0) <= current.supported_value_msat
            ):
                continue
        best[key] = reach
        queue.append(reach)
    while queue:
        reach = queue.popleft()
        branches = outgoing.get(reach.txid, ())
        overcommitted = (
            len(branches) > 1
            and reach.supported_value_msat is not None
            and sum(edge.supported_value_msat or 0 for edge in branches) > reach.supported_value_msat
        )
        # A reviewed transaction-level fork cannot promise the same source
        # capacity to multiple children. Keep it unknown; do not choose a FIFO
        # allocation. Sequential propagation remains bounded one edge at a time.
        for edge in branches:
            if overcommitted or edge.supported_value_msat is None or reach.supported_value_msat is None:
                supported_value = None
            else:
                supported_value = min(reach.supported_value_msat, edge.supported_value_msat)
            propagated = _SourceReach(
                anchor_key=reach.anchor_key,
                source_type=reach.source_type,
                txid=edge.to_txid,
                hop_count=reach.hop_count + 1,
                supported_value_msat=supported_value,
                evidence_level=_combine_evidence_levels(
                    (reach.evidence_level, edge.evidence_level)
                ),
            )
            key = (propagated.txid, propagated.anchor_key)
            current = best.get(key)
            if current is not None:
                current_value = -1 if current.supported_value_msat is None else current.supported_value_msat
                propagated_value = -1 if supported_value is None else supported_value
                if (
                    current.hop_count < propagated.hop_count
                    or (
                        current.hop_count == propagated.hop_count
                        and current_value >= propagated_value
                    )
                ):
                    continue
            best[key] = propagated
            queue.append(propagated)
    for reach in best.values():
        by_tx[reach.txid].append(reach)
    return by_tx


def _source_proximity_for_node(
    node: OwnedOutputNode,
    reaches: Sequence[_SourceReach],
) -> SourceProximityFact:
    if not reaches:
        return _source_proximity_unknown(node, reason="no_reviewed_source_path")
    if all(reach.supported_value_msat is None for reach in reaches):
        return _source_proximity_unknown(node, reason="unallocated_reviewed_source_path")
    nearest_hop = min(reach.hop_count for reach in reaches)
    nearest_reaches = [reach for reach in reaches if reach.hop_count == nearest_hop]
    supported_value = min(
        node.amount_msat,
        sum(reach.supported_value_msat or 0 for reach in nearest_reaches),
    )
    unknown_value = max(0, node.amount_msat - supported_value)
    coverage_ratio_ppm = (
        int((supported_value * 1_000_000) // node.amount_msat)
        if node.amount_msat > 0
        else 0
    )
    if supported_value <= 0:
        status = SOURCE_PROXIMITY_UNKNOWN
        support_status = LOCAL_SUPPORT_NOT_SUPPORTED
    elif unknown_value:
        status = SOURCE_PROXIMITY_PARTIAL
        support_status = LOCAL_SUPPORT_SUPPORTED
    else:
        status = SOURCE_PROXIMITY_KNOWN
        support_status = LOCAL_SUPPORT_SUPPORTED
    evidence_levels = [reach.evidence_level for reach in nearest_reaches]
    if unknown_value:
        evidence_levels.append(EVIDENCE_UNKNOWN)
    evidence_level = _combine_evidence_levels(evidence_levels)
    return SourceProximityFact(
        coin_id=node.node_id,
        provenance_status=status,
        evidence_level=evidence_level,
        source_types=tuple(sorted({reach.source_type for reach in nearest_reaches})),
        nearest_hop_count=nearest_hop,
        supported_value_msat=supported_value,
        unknown_value_msat=unknown_value,
        coverage_ratio_ppm=coverage_ratio_ppm,
        support_status=support_status,
        evidence={
            "reviewed_path_count": len(nearest_reaches),
            "coverage_gap": bool(unknown_value),
            "model_scope": "reviewed_local_source_funds_only",
            "value_basis": "reviewed_allocations_or_source_amounts",
        },
    )


def _build_source_proximity(
    conn: sqlite3.Connection,
    profile_id: str,
    nodes: Mapping[str, OwnedOutputNode],
    limitations: list[dict[str, Any]],
) -> tuple[SourceProximityFact, ...]:
    limitations.append(
        {
            "code": "source_funds_not_global_provenance",
            "message": (
                "Source proximity uses reviewed local source-funds anchors only; "
                "unreviewed or missing provenance remains unknown."
            ),
            "evidence_level": EVIDENCE_EXACT,
        }
    )
    network = next(iter(nodes.values())).network if nodes else None
    allowed_ids = {row["id"] for row in _privacy_transaction_rows(conn, profile_id, network, include_excluded=False)}
    initial_reaches, outgoing, unavailable_reason = _load_source_reaches_and_edges(
        conn, profile_id, allowed_ids,
    )
    if unavailable_reason:
        limitations.append(
            {
                "code": unavailable_reason,
                "message": "Reviewed source-funds proximity could not be computed from the local schema.",
                "evidence_level": EVIDENCE_UNKNOWN,
            }
        )
        return tuple(
            _source_proximity_unknown(node, reason=unavailable_reason)
            for node in sorted(nodes.values(), key=lambda item: item.node_id)
        )
    reaches_by_tx = _propagate_source_reaches(initial_reaches, outgoing)
    owned_by_tx: dict[str, list[OwnedOutputNode]] = defaultdict(list)
    for node in nodes.values():
        owned_by_tx[node.txid].append(node)
    facts_list: list[SourceProximityFact] = []
    for txid, siblings in sorted(owned_by_tx.items()):
        reaches = reaches_by_tx.get(txid, ())
        nearest = min((reach.hop_count for reach in reaches), default=None)
        budget = sum(reach.supported_value_msat or 0 for reach in reaches if reach.hop_count == nearest)
        ambiguous = len(siblings) > 1 and 0 < budget < sum(node.amount_msat for node in siblings)
        for node in sorted(siblings, key=lambda item: item.node_id):
            facts_list.append(
                _source_proximity_unknown(node, reason="ambiguous_output_allocation")
                if ambiguous else _source_proximity_for_node(node, reaches)
            )
    facts = tuple(facts_list)
    unknown_count = sum(
        1 for fact in facts if fact.provenance_status == SOURCE_PROXIMITY_UNKNOWN
    )
    if unknown_count:
        limitations.append(
            {
                "code": "source_proximity_coverage_gaps",
                "message": "Some owned coins do not have reviewed local source-funds proximity.",
                "evidence_level": EVIDENCE_UNKNOWN,
                "evidence": {
                    "unknown_coin_count": unknown_count,
                    "coin_count": len(facts),
                },
            }
        )
    return facts


@dataclass(frozen=True)
class _InferenceComponent:
    component_id: str
    node_ids: tuple[str, ...]
    wallet_ids: tuple[str, ...]
    edge_ids: tuple[str, ...]
    evidence_level: str


@dataclass(frozen=True)
class _AnchorCandidate:
    anchor_id: str
    kind: str
    txid: str | None
    evidence_level: str
    source: str


def _combine_evidence_levels(levels: Iterable[str]) -> str:
    values = list(levels)
    if not values:
        return EVIDENCE_EXACT
    if any(value == EVIDENCE_UNKNOWN for value in values):
        return EVIDENCE_UNKNOWN
    if any(value == EVIDENCE_DERIVED for value in values):
        return EVIDENCE_DERIVED
    return EVIDENCE_EXACT


def _confidence_to_evidence_level(value: Any) -> str:
    confidence = str(value or "").strip().lower()
    if confidence == "exact":
        return EVIDENCE_EXACT
    if confidence in {"strong", "weak"}:
        return EVIDENCE_DERIVED
    return EVIDENCE_UNKNOWN


def _build_inference_components(
    nodes: Mapping[str, OwnedOutputNode],
    edges: Sequence[PrivacyLinkageEdge],
) -> tuple[tuple[_InferenceComponent, ...], dict[str, _InferenceComponent]]:
    edges = tuple(edge for edge in edges if edge.observer_linkage)
    uf = Components(nodes)
    for edge in edges:
        uf.union(edge.from_node_id, edge.to_node_id)
    edge_by_id = {edge.edge_id: edge for edge in edges}
    edge_ids_by_root: dict[str, set[str]] = defaultdict(set)
    for edge in edges:
        root = uf.find(edge.from_node_id)
        edge_ids_by_root[root].add(edge.edge_id)
    components: list[_InferenceComponent] = []
    node_to_component: dict[str, _InferenceComponent] = {}
    for index, node_ids in enumerate(
        uf.groups(include_singletons=True),
        start=1,
    ):
        root = uf.find(node_ids[0])
        edge_ids = tuple(sorted(edge_ids_by_root.get(root, ())))
        component = _InferenceComponent(
            component_id=f"inference_cluster:{index}",
            node_ids=tuple(node_ids),
            wallet_ids=tuple(sorted({nodes[node_id].wallet_id for node_id in node_ids})),
            edge_ids=edge_ids,
            evidence_level=_combine_evidence_levels(
                edge_by_id[edge_id].evidence_level for edge_id in edge_ids
            ),
        )
        components.append(component)
        for node_id in node_ids:
            node_to_component[node_id] = component
    return tuple(components), node_to_component


def _txid_to_node_ids(
    nodes: Mapping[str, OwnedOutputNode],
    tx_facts: Mapping[str, _SpendFact],
) -> dict[str, set[str]]:
    txids: dict[str, set[str]] = defaultdict(set)
    for node in nodes.values():
        txids[node.txid].add(node.node_id)
    for txid, fact in tx_facts.items():
        txids[txid].update(fact.input_node_ids)
        txids[txid].update(fact.output_node_ids)
    return txids


def _load_source_funds_anchor_candidates(
    conn: sqlite3.Connection,
    profile_id: str,
    allowed_transaction_ids: set[str],
) -> tuple[tuple[_AnchorCandidate, ...], str | None]:
    link_columns = _table_columns(conn, "source_funds_links")
    transaction_columns = _table_columns(conn, "transactions")
    required_link_columns = {
        "id",
        "profile_id",
        "state",
        "from_source_id",
        "from_transaction_id",
        "to_transaction_id",
        "confidence",
    }
    if not required_link_columns.issubset(link_columns) or not {
        "id",
        "profile_id",
        "external_id",
    }.issubset(transaction_columns):
        return (), "source_funds_anchor_tables_unavailable"
    rows = conn.execute(
        """
        SELECT l.from_source_id, l.from_transaction_id, l.to_transaction_id,
               l.confidence,
               to_tx.external_id AS to_external_id,
               from_tx.external_id AS from_external_id
        FROM source_funds_links l
        LEFT JOIN transactions to_tx
          ON to_tx.profile_id = l.profile_id AND to_tx.id = l.to_transaction_id
        LEFT JOIN transactions from_tx
          ON from_tx.profile_id = l.profile_id AND from_tx.id = l.from_transaction_id
        WHERE l.profile_id = ?
          AND l.state = 'reviewed'
        ORDER BY l.id ASC
        """,
        (profile_id,),
    ).fetchall()
    candidates: list[_AnchorCandidate] = []

    def add(kind: str, txid: Any, evidence_level: str, source: str) -> None:
        candidates.append(
            _AnchorCandidate(
                anchor_id=f"{ADVERSARY_KYC_SOURCE_FUNDS}:{kind}:{len(candidates) + 1}",
                kind=kind,
                txid=_normalize_txid(txid) or None,
                evidence_level=evidence_level,
                source=source,
            )
        )

    for row in rows:
        if row["to_transaction_id"] not in allowed_transaction_ids and row["to_external_id"] is not None:
            continue
        if row["from_transaction_id"] and row["from_transaction_id"] not in allowed_transaction_ids and row["from_external_id"] is not None:
            continue
        evidence_level = _confidence_to_evidence_level(row["confidence"])
        if row["from_source_id"]:
            add(
                "reviewed_source_anchor",
                row["to_external_id"],
                evidence_level,
                "reviewed_source_funds_anchor",
            )
        if row["from_transaction_id"]:
            add(
                "reviewed_source_funds_parent",
                row["from_external_id"],
                evidence_level,
                "reviewed_source_funds_link",
            )
            add(
                "reviewed_source_funds_child",
                row["to_external_id"],
                evidence_level,
                "reviewed_source_funds_link",
            )
    return tuple(candidates), None


def _load_counterparty_anchor_candidates(
    conn: sqlite3.Connection,
    profile_id: str,
    allowed_transaction_ids: set[str],
) -> tuple[tuple[_AnchorCandidate, ...], str | None]:
    columns = _table_columns(conn, "transactions")
    if not {"profile_id", "external_id", "counterparty"}.issubset(columns):
        return (), "counterparty_annotations_unavailable"
    rows = conn.execute(
        """
        SELECT id, external_id
        FROM transactions
        WHERE profile_id = ?
          AND external_id IS NOT NULL
          AND trim(external_id) != ''
          AND counterparty IS NOT NULL
          AND trim(counterparty) != ''
        ORDER BY external_id ASC
        """,
        (profile_id,),
    ).fetchall()
    candidates = [
        _AnchorCandidate(
            anchor_id=f"{ADVERSARY_KNOWN_COUNTERPARTY}:transaction:{index}",
            kind="known_counterparty_transaction",
            txid=_normalize_txid(row["external_id"]) or None,
            evidence_level=EVIDENCE_EXACT,
            source="local_counterparty_annotation",
        )
        for index, row in enumerate(rows, start=1)
        if row["id"] in allowed_transaction_ids
    ]
    return tuple(candidates), None


def _materialize_anchors(
    candidates: Sequence[_AnchorCandidate],
    txid_nodes: Mapping[str, set[str]],
) -> tuple[tuple[AdversaryInferenceAnchor, ...], tuple[AdversaryInferenceAnchor, ...]]:
    matched: list[AdversaryInferenceAnchor] = []
    unsupported: list[AdversaryInferenceAnchor] = []
    for candidate in candidates:
        node_ids = tuple(sorted(txid_nodes.get(candidate.txid or "", ())))
        anchor = AdversaryInferenceAnchor(
            anchor_id=candidate.anchor_id,
            kind=candidate.kind,
            evidence_level=candidate.evidence_level,
            source=candidate.source,
            matched_node_ids=node_ids,
            support_status=(
                LOCAL_SUPPORT_SUPPORTED
                if node_ids
                else LOCAL_SUPPORT_NOT_SUPPORTED
            ),
        )
        if node_ids:
            matched.append(anchor)
        else:
            unsupported.append(anchor)
    return tuple(matched), tuple(unsupported)


def _model_assumption(code: str, statement: str, evidence_level: str) -> dict[str, Any]:
    return {
        "code": code,
        "statement": statement,
        "evidence_level": evidence_level,
    }


def _unknown_coverage(
    *,
    total_nodes: int,
    total_wallets: int,
    exposed_node_ids: set[str],
    exposed_wallet_ids: set[str],
    unsupported_anchor_count: int,
    reason: str | None,
) -> dict[str, Any]:
    node_count = max(0, total_nodes - len(exposed_node_ids))
    wallet_count = max(0, total_wallets - len(exposed_wallet_ids))
    if reason:
        status = reason
    elif node_count or wallet_count or unsupported_anchor_count:
        status = "partial_model_coverage"
    else:
        status = "complete_for_local_graph"
    return {
        "status": status,
        "node_count": node_count,
        "wallet_count": wallet_count,
        "anchor_count_without_local_graph": unsupported_anchor_count,
        "evidence_level": EVIDENCE_UNKNOWN if reason or unsupported_anchor_count else EVIDENCE_EXACT,
    }


def _cluster_from_component(
    component: _InferenceComponent,
    *,
    anchors: Sequence[AdversaryInferenceAnchor],
    model_basis: str,
) -> AdversaryInferenceCluster:
    return AdversaryInferenceCluster(
        cluster_id=component.component_id,
        node_ids=component.node_ids,
        wallet_ids=component.wallet_ids,
        edge_ids=component.edge_ids,
        anchor_ids=tuple(anchor.anchor_id for anchor in anchors),
        anchor_kinds=tuple(sorted({anchor.kind for anchor in anchors})),
        evidence_level=_combine_evidence_levels(
            [component.evidence_level, *(anchor.evidence_level for anchor in anchors)]
        ),
        support_status=LOCAL_SUPPORT_SUPPORTED,
        model_basis=model_basis,
    )


def _passive_chain_view(
    components: Sequence[_InferenceComponent],
    observer_entities: Sequence[PassiveObserverEntity],
    *,
    total_node_count: int,
    total_wallet_count: int,
) -> AdversaryInferenceView:
    visible_components = [component for component in components if component.edge_ids]
    clusters = tuple(
        _cluster_from_component(
            component,
            anchors=(),
            model_basis="bitcoin_graph_facts_only",
        )
        for component in visible_components
    )
    exposed_node_ids = {node_id for cluster in clusters for node_id in cluster.node_ids}
    exposed_wallet_ids = {
        wallet_id for cluster in clusters for wallet_id in cluster.wallet_ids
    }
    unknown = _unknown_coverage(
        total_nodes=total_node_count,
        total_wallets=total_wallet_count,
        exposed_node_ids=exposed_node_ids,
        exposed_wallet_ids=exposed_wallet_ids,
        unsupported_anchor_count=0,
        reason=None if clusters else "no_local_linkage_edges",
    )
    summary = {
        "observer_entity_count": len(observer_entities),
        "wallet_count": len(exposed_wallet_ids),
        "exposed_cluster_count": len(clusters),
        "unknown_coverage": unknown,
        "evidence_level": _combine_evidence_levels(cluster.evidence_level for cluster in clusters),
    }
    return AdversaryInferenceView(
        tier=ADVERSARY_PASSIVE_CHAIN,
        label="Passive chain watcher",
        evidence_level=summary["evidence_level"],
        model_assumptions=(
            _model_assumption(
                "bitcoin_graph_facts_only",
                "Model assumes common-input ownership only where no collaborative boundary is known. Private wallet change metadata does not establish public observer linkage.",
                EVIDENCE_DERIVED,
            ),
            _model_assumption(
                "no_reputation_lists",
                "Model does not use hosted entity, reputation, or address-owner lists.",
                EVIDENCE_EXACT,
            ),
            _model_assumption(
                "local_inventory_scope",
                "Model coverage is limited to locally stored Bitcoin watch-only outputs and transaction facts.",
                EVIDENCE_EXACT,
            ),
        ),
        summary=summary,
        clusters=clusters,
        unsupported_anchors=(),
    )


def _anchored_adversary_view(
    *,
    tier: str,
    label: str,
    model_basis: str,
    assumptions: Sequence[Mapping[str, Any]],
    candidates: Sequence[_AnchorCandidate],
    candidate_unavailable_reason: str | None,
    components: Sequence[_InferenceComponent],
    node_to_component: Mapping[str, _InferenceComponent],
    txid_nodes: Mapping[str, set[str]],
    total_node_count: int,
    total_wallet_count: int,
) -> AdversaryInferenceView:
    matched_anchors, unsupported_anchors = _materialize_anchors(candidates, txid_nodes)
    anchors_by_component: dict[str, list[AdversaryInferenceAnchor]] = defaultdict(list)
    for anchor in matched_anchors:
        component_ids = {
            node_to_component[node_id].component_id
            for node_id in anchor.matched_node_ids
            if node_id in node_to_component
        }
        for component_id in component_ids:
            anchors_by_component[component_id].append(anchor)
    component_by_id = {component.component_id: component for component in components}
    clusters = tuple(
        _cluster_from_component(
            component_by_id[component_id],
            anchors=tuple(sorted(anchors, key=lambda item: item.anchor_id)),
            model_basis=model_basis,
        )
        for component_id, anchors in sorted(anchors_by_component.items())
    )
    exposed_node_ids = {node_id for cluster in clusters for node_id in cluster.node_ids}
    exposed_wallet_ids = {
        wallet_id for cluster in clusters for wallet_id in cluster.wallet_ids
    }
    no_anchor_reason = None
    if candidate_unavailable_reason:
        no_anchor_reason = candidate_unavailable_reason
    elif not candidates:
        no_anchor_reason = "no_model_anchors"
    unknown = _unknown_coverage(
        total_nodes=total_node_count,
        total_wallets=total_wallet_count,
        exposed_node_ids=exposed_node_ids,
        exposed_wallet_ids=exposed_wallet_ids,
        unsupported_anchor_count=len(unsupported_anchors),
        reason=no_anchor_reason,
    )
    summary = {
        "observer_entity_count": sum(1 for cluster in clusters if cluster.edge_ids),
        "wallet_count": len(exposed_wallet_ids),
        "exposed_cluster_count": len(clusters),
        "anchor_count": len(candidates),
        "unsupported_anchor_count": len(unsupported_anchors),
        "unknown_coverage": unknown,
        "evidence_level": _combine_evidence_levels(
            [
                *(cluster.evidence_level for cluster in clusters),
                *(anchor.evidence_level for anchor in unsupported_anchors),
                unknown["evidence_level"],
            ]
        ),
    }
    return AdversaryInferenceView(
        tier=tier,
        label=label,
        evidence_level=summary["evidence_level"],
        model_assumptions=tuple(dict(assumption) for assumption in assumptions),
        summary=summary,
        clusters=clusters,
        unsupported_anchors=unsupported_anchors,
    )


def _build_adversary_views(
    conn: sqlite3.Connection,
    profile_id: str,
    nodes: Mapping[str, OwnedOutputNode],
    edges: Sequence[PrivacyLinkageEdge],
    observer_entities: Sequence[PassiveObserverEntity],
    tx_facts: Mapping[str, _SpendFact],
) -> tuple[AdversaryInferenceView, ...]:
    components, node_to_component = _build_inference_components(nodes, edges)
    txid_nodes = _txid_to_node_ids(nodes, tx_facts)
    total_wallet_ids = {node.wallet_id for node in nodes.values()}
    network = next(iter(nodes.values())).network if nodes else None
    allowed_ids = {row["id"] for row in _privacy_transaction_rows(conn, profile_id, network)}
    reviewed_ids = {row["id"] for row in _privacy_transaction_rows(conn, profile_id, network, include_excluded=False)}
    source_candidates, source_reason = _load_source_funds_anchor_candidates(
        conn, profile_id, reviewed_ids,
    )
    counterparty_candidates, counterparty_reason = _load_counterparty_anchor_candidates(
        conn, profile_id, allowed_ids,
    )
    return (
        _passive_chain_view(
            components,
            observer_entities,
            total_node_count=len(nodes),
            total_wallet_count=len(total_wallet_ids),
        ),
        _anchored_adversary_view(
            tier=ADVERSARY_KYC_SOURCE_FUNDS,
            label="Reviewed KYC/source-funds anchor watcher",
            model_basis="reviewed_local_anchor_plus_bitcoin_graph_facts",
            assumptions=(
                _model_assumption(
                    "reviewed_anchor_starting_knowledge",
                    "Model assumes this observer starts only from locally reviewed source-funds anchors and then applies local Bitcoin graph facts.",
                    EVIDENCE_DERIVED,
                ),
                _model_assumption(
                    "not_global_kyc_knowledge",
                    "Model does not claim what every exchange, custodian, or reviewer knows.",
                    EVIDENCE_EXACT,
                ),
            ),
            candidates=source_candidates,
            candidate_unavailable_reason=source_reason,
            components=components,
            node_to_component=node_to_component,
            txid_nodes=txid_nodes,
            total_node_count=len(nodes),
            total_wallet_count=len(total_wallet_ids),
        ),
        _anchored_adversary_view(
            tier=ADVERSARY_KNOWN_COUNTERPARTY,
            label="Known-counterparty hypothetical",
            model_basis="known_counterparty_anchor_plus_bitcoin_graph_facts",
            assumptions=(
                _model_assumption(
                    "counterparty_starting_knowledge",
                    "Model assumes a specific counterparty knows only transactions locally annotated with a counterparty value, then applies local Bitcoin graph facts.",
                    EVIDENCE_DERIVED,
                ),
                _model_assumption(
                    "hypothetical_not_identity_claim",
                    "Model reports a bounded hypothetical, not an absolute identity or reputation claim.",
                    EVIDENCE_EXACT,
                ),
            ),
            candidates=counterparty_candidates,
            candidate_unavailable_reason=counterparty_reason,
            components=components,
            node_to_component=node_to_component,
            txid_nodes=txid_nodes,
            total_node_count=len(nodes),
            total_wallet_count=len(total_wallet_ids),
        ),
    )


def _build_observer_entities(
    nodes: Mapping[str, OwnedOutputNode],
    edges: Sequence[PrivacyLinkageEdge],
    uf: Components,
    edge_ids_by_node: Mapping[str, set[str]],
    heuristics_by_node: Mapping[str, set[str]],
    score_by_edge: Mapping[str, int],
) -> list[PassiveObserverEntity]:
    entities: list[PassiveObserverEntity] = []
    for index, node_ids in enumerate(
        uf.groups(),
        start=1,
    ):
        edge_ids = sorted(set().union(*(edge_ids_by_node[node_id] for node_id in node_ids)))
        heuristics = sorted(set().union(*(heuristics_by_node[node_id] for node_id in node_ids)))
        score = sum(score_by_edge.get(edge_id, 0) for edge_id in edge_ids)
        consequence = sum(
            edge.amount_msat
            for edge in edges
            if edge.edge_id in edge_ids and edge.new_linkage
        )
        entities.append(
            PassiveObserverEntity(
                entity_id=f"observer_entity:{index}",
                node_ids=tuple(node_ids),
                edge_ids=tuple(edge_ids),
                heuristics=tuple(heuristics),
                evidence_level=EVIDENCE_DERIVED,
                linkage_score=score,
                consequence_msat=consequence,
            )
        )
    return entities


def _build_findings(edges: Sequence[PrivacyLinkageEdge]) -> list[PrivacyLinkageFinding]:
    by_kind: dict[str, list[PrivacyLinkageEdge]] = defaultdict(list)
    for edge in edges:
        if edge.new_linkage:
            by_kind[edge.kind].append(edge)
    findings: list[PrivacyLinkageFinding] = []
    for kind, group in sorted(by_kind.items()):
        linkage_score = sum(edge.merged_cluster_count for edge in group)
        consequence_msat = sum(edge.amount_msat for edge in group)
        evidence_level = (
            EVIDENCE_DERIVED
            if any(edge.evidence_level == EVIDENCE_DERIVED for edge in group)
            else EVIDENCE_EXACT
        )
        findings.append(
            PrivacyLinkageFinding(
                finding_id=f"{kind}_linkage",
                kind=kind,
                severity=_severity(linkage_score, consequence_msat),
                title=_finding_title(kind),
                detail=_finding_detail(kind, linkage_score),
                evidence_level=evidence_level,
                linkage_score=linkage_score,
                consequence_msat=consequence_msat,
                edge_ids=tuple(edge.edge_id for edge in group),
                evidence={
                    "new_cluster_merges": linkage_score,
                    "edge_count": len(group),
                    "consequence_msat": consequence_msat,
                },
            )
        )
    return findings


def _severity(linkage_score: int, consequence_msat: int) -> str:
    if linkage_score >= 3:
        return "alert" if consequence_msat >= 100_000_000_000 else "warning"
    if linkage_score >= 1:
        return "warning" if consequence_msat >= 100_000_000_000 else "info"
    return "info"


def _finding_title(kind: str) -> str:
    return {
        "address_reuse": "Address reuse links owned outputs",
        "common_input": "Common-input ownership heuristic links clusters",
        "change_output": "Change-output evidence links input and output clusters",
    }.get(kind, "Owned output linkage found")


def _finding_detail(kind: str, linkage_score: int) -> str:
    if kind == "common_input":
        return (
            f"Common-input evidence created {linkage_score} new passive-observer "
            "cluster merge(s); already-linked inputs do not add score."
        )
    if kind == "change_output":
        return (
            f"Change-output evidence created {linkage_score} new passive-observer "
            "cluster merge(s) from local branch and transaction structure."
        )
    if kind == "address_reuse":
        return (
            f"Address reuse created {linkage_score} new passive-observer "
            "cluster merge(s)."
        )
    return f"Local Bitcoin UTXO linkage created {linkage_score} new cluster merge(s)."

"""Local observation graph. Acquisition and accounting interpretation live elsewhere.

Every source is read once under the caller's SQLite snapshot. Immutable adjacency
indexes support repeated queries without rescanning the book. A transaction edge
proves connectivity, never which input funded which output or common ownership.
"""
from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass, replace
import hashlib
import json
import sqlite3
from types import MappingProxyType
from typing import Any, Mapping

from ...errors import AppError
from ..custody_evidence import resolve_protocol_scope
from ..onchain import (
    input_outpoint, normalized_script_hex,
    output_address, output_script, output_value_sats, stored_tx_mapping,
)
from ..privacy_hygiene import collaborative_transaction_evidence


def digest(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), default=str).encode()).hexdigest()


def canonical_txid(value: Any) -> str | None:
    value = str(value or "").strip().lower()
    return value if len(value) == 64 and all(c in "0123456789abcdef" for c in value) else None


def integer(value: Any) -> int | None:
    if isinstance(value, bool):
        return None
    try:
        result = int(value)
    except (TypeError, ValueError, OverflowError):
        return None
    return result if str(value).strip() == str(result) and result >= 0 else None


def tx_node_id(chain: str, network: str, txid: str) -> str:
    return f"{chain}:{network}:tx:{txid}"


def output_node_id(chain: str, network: str, txid: str, vout: int) -> str:
    return f"{chain}:{network}:out:{txid}:{vout}"


def evidence(source: str, reference: str, level: str = "observed") -> dict[str, str]:
    return {"source": source, "reference": reference, "level": level}


@dataclass(frozen=True)
class AnalysisIndex:
    snapshot_id: str
    nodes: Mapping[str, Mapping[str, Any]]
    edges: Mapping[str, Mapping[str, Any]]
    outgoing: Mapping[str, tuple[str, ...]]
    incoming: Mapping[str, tuple[str, ...]]
    subjects: Mapping[str, tuple[str, ...]]
    transaction_facts: Mapping[str, Mapping[str, Any]]
    output_facts: Mapping[str, Mapping[str, Any]]
    findings: tuple[Mapping[str, Any], ...]
    coverage: Mapping[str, Any]
    labels: tuple[Mapping[str, Any], ...]
    profile_seeds: tuple[str, ...]


def _collaboration(a: Mapping | None, b: Mapping | None) -> Mapping | None:
    """Stronger exclusions dominate shape heuristics independent of row order."""
    values = [value for value in (a, b) if value]
    if not values:
        return None
    kind_rank = {"coinjoin": 1, "collaborative": 2, "payment_in_coinjoin": 3, "payjoin": 3, "unknown": 4}
    evidence_rank = {"heuristic": 1, "imported": 2, "reviewed": 3}
    return max(values, key=lambda value: (kind_rank.get(value.get("kind"), 4), evidence_rank.get(value.get("evidence_level"), 0), json.dumps(dict(value), sort_keys=True)))


def _freeze(value: Any) -> Any:
    if isinstance(value, dict):
        return MappingProxyType({key: _freeze(item) for key, item in value.items()})
    if isinstance(value, (tuple, list)):
        return tuple(_freeze(item) for item in value)
    return value


def thaw(value: Any) -> Any:
    if isinstance(value, Mapping):
        return {key: thaw(item) for key, item in value.items()}
    if isinstance(value, (tuple, list)):
        return [thaw(item) for item in value]
    return value


def observer_index(index: AnalysisIndex, observer: str) -> AnalysisIndex:
    """Apply knowledge boundaries before filters, traversal and analytics."""
    if observer != "public" or index.coverage.get("observer_knowledge") == "public_chain_facts":
        return index
    nodes = {}
    output_facts = {}

    def refs(values, subject):
        # The observation source class is inspectable, private DB record IDs,
        # connection names and acquisition timestamps are not public knowledge.
        return [{"source": value.get("source"), "reference": subject, "level": value.get("level")}
                for value in values]

    for ident, node in index.nodes.items():
        if node["kind"] not in {"transaction", "output"} or node.get("chain") not in {"bitcoin", "liquid"}:
            continue
        public = thaw(node)
        public["wallet_ids"] = []
        public.pop("transaction_id", None)
        public.pop("occurred_at", None)
        public["evidence"] = refs(node.get("evidence", ()), ident)
        private = index.output_facts.get(ident, {})
        if node["kind"] == "output":
            output_facts[ident] = {"script": private.get("script"), "branch_role": None, "ownership_known": False}
            if node["chain"] == "liquid":
                public["amount_msat"] = private.get("public_amount_msat")
                public["asset"] = private.get("public_asset")
                # A confidential address contains recipient information absent
                # from the locking script; it is never observer-public here.
                public.pop("address", None)
                public["amount_visibility"] = "public_explicit" if public["amount_msat"] is not None else "unknown_for_observer"
        nodes[ident] = public
    edges = {}
    outgoing, incoming = defaultdict(list), defaultdict(list)
    for ident, edge in index.edges.items():
        if edge["kind"] not in {"creates", "spends"} or edge["source"] not in nodes or edge["target"] not in nodes:
            continue
        public = thaw(edge)
        output = nodes[edge["target"] if edge["kind"] == "creates" else edge["source"]]
        public["amount_msat"], public["asset"] = output.get("amount_msat"), output.get("asset")
        public["evidence"] = refs(edge.get("evidence", ()), output["id"])
        edges[ident] = public
        outgoing[edge["source"]].append(ident)
        incoming[edge["target"]].append(ident)
    facts = {}
    for ident, fact in index.transaction_facts.items():
        if ident not in nodes:
            continue
        public = thaw(fact)
        for side in ("inputs", "outputs"):
            for item in public[side]:
                item["amount_msat"] = nodes.get(item["output_id"], {}).get("amount_msat")
        if nodes[ident]["chain"] == "liquid":
            public["fee_msat"] = None
        marker = public.get("collaboration")
        if marker and marker.get("source") not in {"equal_output_coinjoin", "large_equal_output_coinjoin"}:
            public["collaboration"] = {"kind": "unknown", "source": "observer_exclusion", "evidence_level": "unavailable"}
        facts[ident] = public
    subjects = defaultdict(set)
    for ident, node in nodes.items():
        for alias in (ident, node.get("txid") if node["kind"] == "transaction" else None, node.get("outpoint"), node.get("address")):
            if alias:
                subjects[alias].add(ident)
        script = output_facts.get(ident, {}).get("script")
        if script:
            subjects[f"script:{script}"].add(ident)
    findings = []
    for finding in index.findings:
        ids = [ident for ident in finding["node_ids"] if ident in nodes]
        if not ids:
            continue
        public = thaw(finding)
        public["node_ids"] = ids
        public["edge_ids"] = [ident for ident in public.get("edge_ids", ()) if ident in edges]
        public["evidence"] = refs(finding.get("evidence", ()), ids[0])
        findings.append(public)
    coverage = thaw(index.coverage)
    coverage.update(observer_knowledge="public_chain_facts", hidden_private_node_count=len(index.nodes) - len(nodes),
                    hidden_private_relation_count=sum(edge["kind"] == "custody" for edge in index.edges.values()),
                    public_liquid_value_policy="explicit_public_values_only", private_labels_withheld=True)
    return replace(index, nodes=_freeze(nodes), edges=_freeze(edges),
                   outgoing=_freeze({key: tuple(value) for key, value in outgoing.items()}),
                   incoming=_freeze({key: tuple(value) for key, value in incoming.items()}),
                   subjects=_freeze({key: tuple(sorted(value)) for key, value in subjects.items()}),
                   transaction_facts=_freeze(facts), output_facts=_freeze(output_facts), findings=_freeze(findings),
                   coverage=_freeze(coverage), labels=(), profile_seeds=tuple(ident for ident in index.profile_seeds if ident in nodes))


class _Builder:
    def __init__(self, profile_id: str):
        self.profile_id = profile_id
        self.nodes: dict[str, dict] = {}
        self.edges: dict[str, dict] = {}
        self.subjects: dict[str, set[str]] = defaultdict(set)
        self.tx_facts: dict[str, dict] = {}
        self.output_facts: dict[str, dict] = {}
        self.findings: list[dict] = []
        self.coverage: dict[str, Any] = {"local_only": True, "egress": "none", "missing_tables": [], "invalid_observations": 0, "cache_rejected": 0, "source_rows": {}, "custody_fresh": False}
        self.field_ranks: dict[tuple[str, str], int] = {}
        self.shapes: dict[str, set[str]] = defaultdict(set)
        self.complete_members: dict[str, tuple[set[str], set[str]]] = {}
        self.records: dict[str, str] = {}
        self.labels: list[dict] = []
        self.profile_seeds: set[str] = set()

    def finding(self, code: str, ids: list[str], detail: str, *, severity: str = "warning", refs: list[dict] | None = None):
        self.findings.append({"id": digest([code, sorted(ids), detail]), "code": code, "severity": severity, "title": code.replace("_", " ").capitalize(), "detail": detail, "node_ids": sorted(ids), "edge_ids": [], "evidence": refs or []})

    def alias(self, alias: Any, node_id: str):
        if alias not in (None, ""):
            self.subjects[str(alias)].add(node_id)

    def node(self, node_id: str, chain: str | None, network: str | None, kind: str, *, txid: str | None = None) -> dict:
        if node_id not in self.nodes:
            self.nodes[node_id] = {"id": node_id, "kind": kind, "chain": chain, "network": network, "label": (txid[:12] if txid else kind), "wallet_ids": [], "amount_msat": None, "asset": "BTC" if chain == "bitcoin" else None, "status": "missing", "evidence": []}
            if txid:
                self.nodes[node_id]["txid"] = txid
            self.alias(node_id, node_id)
        return self.nodes[node_id]

    def merge(self, node: dict, updates: Mapping[str, Any], rank: int, ref: dict):
        if ref not in node["evidence"]:
            node["evidence"].append(ref)
        for field, value in updates.items():
            if value is None:
                continue
            key = (node["id"], field)
            previous_rank = self.field_ranks.get(key, -1)
            previous = node.get(field)
            if rank == previous_rank and previous is not None and previous != value and field in {"amount_msat", "asset", "address"}:
                node["status"] = "conflicting"
                self.finding("conflicting_output_observations", [node["id"]], f"Retained local observations disagree about {field}; traversal stops at this output.", refs=[ref])
                # Do not arbitrarily select one value as fact.
                node[field] = None
                self.field_ranks[key] = 100
            elif rank > previous_rank or previous is None and previous_rank < 100:
                node[field] = value
                self.field_ranks[key] = rank

    def own(self, node: dict, wallet_id: str | None):
        if wallet_id and wallet_id not in node["wallet_ids"]:
            node["wallet_ids"].append(wallet_id)
            self.alias(wallet_id, node["id"])
            self.alias(f"wallet:{wallet_id}", node["id"])

    def edge(self, source: str, target: str, kind: str, ref: dict, *, suffix: str = "", level: str = "observed", amount: str | None = None, asset: str | None = None, status: str = "observed"):
        edge_id = f"{kind}:{digest([source, target, suffix])[:32]}"
        if edge_id not in self.edges:
            self.edges[edge_id] = {"id": edge_id, "source": source, "target": target, "kind": kind, "evidence_level": level, "amount_msat": amount, "asset": asset, "label": kind, "status": status, "evidence": []}
        edge = self.edges[edge_id]
        if ref not in edge["evidence"]:
            edge["evidence"].append(ref)
        return edge

    def output(self, chain: str, network: str, txid: str, vout: int, entry: Mapping[str, Any], ref: dict, rank: int) -> dict:
        parent = self.node(tx_node_id(chain, network, txid), chain, network, "transaction", txid=txid)
        self.alias(txid, parent["id"])
        node = self.node(output_node_id(chain, network, txid, vout), chain, network, "output", txid=txid)
        outpoint = f"{txid}:{vout}"
        node.update(outpoint=outpoint, label=f"{txid[:10]}:{vout}")
        value = output_value_sats(entry)
        amount = str(value * 1000) if value is not None and value >= 0 else None
        raw_asset = entry.get("asset_id") or entry.get("asset")
        asset = "BTC" if chain == "bitcoin" else (str(raw_asset) if raw_asset else None)
        self.merge(node, {"amount_msat": amount, "asset": asset, "address": output_address(entry)}, rank, ref)
        if node["status"] == "missing" and entry:
            node["status"] = "reference" if rank == 1 else "observed"
        script = normalized_script_hex(output_script(entry))
        facts = self.output_facts.setdefault(node["id"], {"script": None, "branch_role": None, "ownership_known": False})
        if script:
            script_rank = self.field_ranks.get((node["id"], "script"), -1)
            if script_rank == rank and facts.get("script") not in (None, script):
                node["status"] = "conflicting"
                facts["script"] = None
                self.field_ranks[(node["id"], "script")] = 100
                self.finding("conflicting_output_script", [node["id"]], "Retained observations disagree about the locking script; reuse and control hypotheses are unavailable.", refs=[ref])
            elif rank > script_rank:
                facts["script"] = script
                self.field_ranks[(node["id"], "script")] = rank
            self.alias(f"script:{script}", node["id"])
        # LWK marks locally unblinded values as owned. Explicit fee/external
        # values and native public value fields may be used by a public observer;
        # inventory and commitment-backed unblinding never add public values.
        public_value = chain == "bitcoin" or (rank < 3 and entry.get("role") != "owned"
            and not entry.get("valuecommitment") and not entry.get("value_commitment")
            and (entry.get("role") in {"fee", "external"} or "value" in entry))
        public_asset = chain == "bitcoin" or (public_value and not entry.get("assetcommitment") and not entry.get("asset_commitment"))
        for field, observed in (("public_amount_msat", amount if public_value else None), ("public_asset", asset if public_asset else None)):
            if observed is not None and not facts.get(field + "_conflict"):
                if facts.get(field) not in (None, observed):
                    facts[field] = None
                    facts[field + "_conflict"] = True
                else:
                    facts[field] = observed
        self.alias(outpoint, node["id"])
        self.alias(node.get("address"), node["id"])
        self.edge(parent["id"], node["id"], "creates", ref)
        return node

    def transaction(self, row: dict, *, cached: bool = False):
        try:
            scope = resolve_protocol_scope(row)
        except (ValueError, AppError):
            self.coverage["invalid_observations"] += 1
            return
        chain, network = scope.protocol_chain, scope.network
        raw = stored_tx_mapping(row.get("raw_json"), allow_nested=True) or {}
        external = canonical_txid(row.get("external_id"))
        raw_id = canonical_txid(raw.get("txid"))
        if external and raw_id and external != raw_id:
            self.coverage["invalid_observations"] += 1
            return
        txid = raw_id or external
        is_chain = chain in {"bitcoin", "liquid"} and txid is not None
        node_id = tx_node_id(chain, network, txid) if is_chain else f"{chain}:{network}:record:{row['id']}"
        node = self.node(node_id, chain, network, "transaction" if is_chain else "record", txid=txid if is_chain else None)
        ref = evidence("reference_cache" if cached else "stored_transaction", str(row["id"]), "reference" if cached else "observed")
        rank = 1 if cached else 2
        if node["status"] == "missing" or node["status"] == "reference" and not cached:
            node["status"] = "reference" if cached else "observed"
        if not cached:
            node.setdefault("transaction_id", row["id"])
            self.records[row["id"]] = node_id
            self.alias(row["id"], node_id)
            self.alias(f"record:{row['id']}", node_id)
            self.own(node, row.get("wallet_id"))
            self.profile_seeds.add(node_id)
        self.merge(node, {"occurred_at": row.get("occurred_at")}, rank, ref)
        if not is_chain:
            value = integer(row.get("amount"))
            self.merge(node, {"amount_msat": str(value) if value is not None else None, "asset": row.get("asset")}, rank, ref)
            return
        self.alias(txid, node_id)
        confirmations = raw.get("confirmations")
        if isinstance(confirmations, int) and confirmations < 0 or raw.get("removed") is True:
            node["status"] = "stale"
            self.finding("retracted_transaction", [node_id], "Stored transaction is removed or conflicted; it cannot prove current reachability.")
        fact = self.tx_facts.setdefault(node_id, {"inputs": [], "outputs": [], "fee_msat": None, "complete": False, "collaboration": None})
        fact["collaboration"] = _collaboration(fact["collaboration"], collaborative_transaction_evidence(row, stored_tx_mapping(row.get("raw_json")) or {}))
        vin = raw.get("vin") if isinstance(raw.get("vin"), list) else []
        vout = raw.get("vout") if isinstance(raw.get("vout"), list) else []
        input_ids, output_ids = [], []
        valid_inputs = bool(vin)
        coinbase = False
        for item in vin:
            if not isinstance(item, Mapping):
                valid_inputs = False
                continue
            point = input_outpoint(item)
            if item.get("is_pegin") is True or item.get("is_peg_in") is True:
                self.finding("native_pegin_boundary", [node_id], "A Liquid peg-in input crosses a protocol boundary; no same-chain parent or ownership link is inferred from its reference.", severity="info", refs=[ref])
                continue
            if point is None:
                if item.get("is_coinbase") is True or isinstance(item.get("coinbase"), str) and item["coinbase"]:
                    coinbase = True
                else:
                    valid_inputs = False
                continue
            prevout = item.get("prevout") if isinstance(item.get("prevout"), Mapping) else {}
            output = self.output(chain, network, *point, prevout, ref, rank)
            input_ids.append(output["id"])
            self.edge(output["id"], node_id, "spends", ref)
        valid_outputs = bool(vout)
        for position, item in enumerate(vout):
            if not isinstance(item, Mapping):
                valid_outputs = False
                continue
            number = integer(item.get("n", position))
            if number is None:
                valid_outputs = False
                continue
            if output_value_sats(item) is None and normalized_script_hex(output_script(item)) is None and not item.get("valuecommitment"):
                valid_outputs = False
            output = self.output(chain, network, txid, number, item, ref, rank)
            output_ids.append(output["id"])
        complete = valid_inputs and valid_outputs and len(set(input_ids)) == len(input_ids) and len(set(output_ids)) == len(output_ids)
        if complete:
            # Partial observations may enrich one shape; incompatible complete
            # shapes are never silently combined into a synthetic transaction.
            self.shapes[node_id].add(digest([input_ids, output_ids, coinbase]))
            self.complete_members.setdefault(node_id, (set(input_ids), set(output_ids)))
            if len(self.shapes[node_id]) > 1:
                node["status"] = "conflicting"
                self.finding("conflicting_transaction_shape", [node_id], "Complete local transaction shapes disagree; physical traversal stops here.")
        fact["complete"] |= complete
        fact["inputs"] = sorted(set(item["output_id"] for item in fact["inputs"]) | set(input_ids))
        fact["outputs"] = sorted(set(item["output_id"] for item in fact["outputs"]) | set(output_ids))
        fact["inputs"] = [{"output_id": item, "amount_msat": None} for item in fact["inputs"]]
        fact["outputs"] = [{"output_id": item, "amount_msat": None} for item in fact["outputs"]]
        known_shape = self.complete_members.get(node_id)
        if known_shape is not None and (
            {item["output_id"] for item in fact["inputs"]} - known_shape[0]
            or {item["output_id"] for item in fact["outputs"]} - known_shape[1]
        ):
            node["status"] = "conflicting"
            self.finding("partial_observation_conflicts_with_complete_shape", [node_id], "A partial observation names inputs or outputs absent from a complete observation. They cannot form one synthetic transaction.", refs=[ref])

    def inventory(self, row: dict):
        try:
            scope = resolve_protocol_scope(row)
        except (ValueError, AppError):
            self.coverage["invalid_observations"] += 1
            return
        txid, vout = canonical_txid(row.get("txid")), integer(row.get("vout"))
        if scope.protocol_chain not in {"bitcoin", "liquid"} or txid is None or vout is None:
            self.coverage["invalid_observations"] += 1
            return
        ref = evidence("wallet_inventory", str(row["id"]), "wallet_observed")
        amount = integer(row.get("amount"))
        entry = {"scriptpubkey": row.get("script_pubkey"), "address": row.get("address"), "asset": row.get("asset")}
        node = self.output(scope.protocol_chain, scope.network, txid, vout, entry, ref, 3)
        self.merge(node, {"amount_msat": str(amount) if amount is not None else None}, 3, ref)
        self.own(node, row.get("wallet_id"))
        self.profile_seeds.add(node["id"])
        self.output_facts[node["id"]].update(branch_role=row.get("branch_label"), ownership_known=True)
        spent = canonical_txid(row.get("spent_by"))
        if spent:
            target = self.node(tx_node_id(scope.protocol_chain, scope.network, spent), scope.protocol_chain, scope.network, "transaction", txid=spent)
            self.alias(spent, target["id"])
            self.edge(node["id"], target["id"], "spends", ref, level="wallet_observed")

    def relations(self, decisions: list[dict], economics: list[dict], fresh: bool):
        self.coverage["custody_fresh"] = fresh
        for table, rows, id_key in (("journal_custody_decisions", decisions, "decision_id"), ("journal_custody_economic_relations", economics, "relation_id")):
            for row in rows:
                source = self.records.get(row.get("source_transaction_id"))
                target = self.records.get(row.get("target_transaction_id"))
                reference = str(row[id_key])
                level = "native_verified" if row.get("state") == "internal_verified" else "reviewed"
                ref = evidence(table, reference, level)
                if source is None:
                    self.finding("missing_custody_anchor", [], "A retained custody relation is missing its source observation.", refs=[ref])
                    continue
                if target is None:
                    record_id = f"record:custody:{reference}"
                    target_node = self.node(record_id, None, None, "record")
                    target_node.update(label=row.get("relation_kind") or "Missing custody endpoint", status="missing", evidence=[ref])
                    target = record_id
                if table == "journal_custody_decisions":
                    start, end = integer(row.get("source_start_msat")), integer(row.get("source_end_msat"))
                    amount = str(end - start) if start is not None and end is not None and end >= start else None
                else:
                    value = integer(row.get("source_amount_msat"))
                    amount = str(value) if value is not None else None
                edge = self.edge(source, target, "custody", ref, suffix=reference, level=level, amount=amount, asset=row.get("source_asset"), status="observed" if fresh else "stale")
                edge.update(label=row.get("relation_kind") or row.get("state") or "custody", relation_id=reference, basis_state=row.get("basis_state"), target_asset=row.get("target_asset"))
                if table == "journal_custody_economic_relations":
                    value = integer(row.get("target_amount_msat"))
                    edge["target_amount_msat"] = str(value) if value is not None else None
                self.alias(reference, source)
                self.alias(reference, target)
                if row.get("component_id"):
                    self.alias(row["component_id"], source)
                    self.alias(row["component_id"], target)

    def finish(self) -> AnalysisIndex:
        outgoing: dict[str, list[str]] = defaultdict(list)
        incoming: dict[str, list[str]] = defaultdict(list)
        spends: dict[str, list[dict]] = defaultdict(list)
        for edge in self.edges.values():
            outgoing[edge["source"]].append(edge["id"])
            incoming[edge["target"]].append(edge["id"])
            if edge["kind"] in {"creates", "spends"}:
                output = self.nodes[edge["target"] if edge["kind"] == "creates" else edge["source"]]
                edge["amount_msat"], edge["asset"] = output["amount_msat"], output["asset"]
            if edge["kind"] == "spends":
                spends[edge["source"]].append(edge)
        for output_id, edges in spends.items():
            if len({edge["target"] for edge in edges}) > 1:
                self.nodes[output_id]["status"] = "conflicting"
                for edge in edges:
                    edge["status"] = "conflicting"
                self.finding("competing_spends", [output_id, *sorted({edge["target"] for edge in edges})], "Multiple retained transactions spend the same outpoint. No current winner is inferred.")
        for node_id, fact in self.tx_facts.items():
            for side in ("inputs", "outputs"):
                for item in fact[side]:
                    item["amount_msat"] = self.nodes[item["output_id"]]["amount_msat"]
            amounts = [item["amount_msat"] for side in ("inputs", "outputs") for item in fact[side]]
            if fact["complete"] and fact["inputs"] and all(value is not None for value in amounts):
                value = sum(int(item["amount_msat"]) for item in fact["inputs"]) - sum(int(item["amount_msat"]) for item in fact["outputs"])
                if value >= 0 and self.nodes[node_id]["chain"] == "bitcoin":
                    fact["fee_msat"] = str(value)
            if self.nodes[node_id]["status"] in {"conflicting", "stale"}:
                fact["complete"] = False
        for node in self.nodes.values():
            node["wallet_ids"].sort()
            node["evidence"].sort(key=lambda ref: (ref["source"], ref["reference"]))
        self.coverage.update(node_count=len(self.nodes), edge_count=len(self.edges), missing_node_count=sum(node["status"] == "missing" for node in self.nodes.values()), conflicting_node_count=sum(node["status"] == "conflicting" for node in self.nodes.values()), complete_transaction_count=sum(fact["complete"] for fact in self.tx_facts.values()), reference_node_count=sum(node["status"] == "reference" for node in self.nodes.values()))
        for label in self.labels:
            label["node_ids"] = sorted(node_id for node_id in self.subjects.get(label.get("subject"), ()) if self.nodes[node_id]["chain"] == label.get("chain") and self.nodes[node_id]["network"] == label.get("network"))
        snapshot_id = digest([self.profile_id, sorted(self.nodes.items()), sorted(self.edges.items()), self.coverage, sorted(self.tx_facts.items()), sorted(self.output_facts.items()), self.labels])
        return AnalysisIndex(snapshot_id, _freeze(self.nodes), _freeze(self.edges), _freeze({key: tuple(sorted(values)) for key, values in outgoing.items()}), _freeze({key: tuple(sorted(values)) for key, values in incoming.items()}), _freeze({key: tuple(sorted(values)) for key, values in self.subjects.items()}), _freeze(self.tx_facts), _freeze(self.output_facts), _freeze(self.findings), _freeze(self.coverage), _freeze(self.labels), tuple(sorted(self.profile_seeds)))


def build_index(conn: sqlite3.Connection, profile_id: str) -> AnalysisIndex:
    """Read one consistent local snapshot; never refresh, mutate, or egress."""
    builder = _Builder(profile_id)
    conn.execute("SAVEPOINT chain_analysis_read")
    try:
        tables = {row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        def read(table: str, *, scoped: bool = True) -> list[dict]:
            if table not in tables:
                builder.coverage["missing_tables"].append(table)
                return []
            rows = conn.execute(f"SELECT * FROM {table}" + (" WHERE profile_id = ?" if scoped else ""), (profile_id,) if scoped else ()).fetchall()
            names = [column[0] for column in conn.execute(f"SELECT * FROM {table} LIMIT 0").description]
            result = [dict(zip(names, row)) for row in rows]
            builder.coverage["source_rows"][table] = len(result)
            return result
        wallets = {row["id"]: row for row in read("wallets")}
        transactions = read("transactions")
        for stored in transactions:
            row = dict(stored)
            wallet = wallets.get(row.get("wallet_id"), {})
            row.update(wallet_kind=wallet.get("kind"), wallet_config_json=wallet.get("config_json"))
            builder.transaction(row)
        for row in read("transaction_graph_cache", scoped=False):
            payload = stored_tx_mapping(row.get("payload_json"))
            txid = canonical_txid(row.get("txid"))
            try:
                scope = resolve_protocol_scope({"chain": row.get("chain"), "network": row.get("network")})
            except (ValueError, AppError):
                scope = None
            if row.get("schema_version") != 1 or txid is None or payload is None or scope is None or scope.protocol_chain not in {"bitcoin", "liquid"} or not isinstance(payload.get("vin"), list) or not isinstance(payload.get("vout"), list) or payload.get("txid") is not None and canonical_txid(payload["txid"]) != txid or payload.get("chain") not in (None, scope.protocol_chain) or payload.get("network") not in (None, scope.network):
                builder.coverage["cache_rejected"] += 1
                continue
            builder.transaction({"id": f"cache:{scope.protocol_chain}:{scope.network}:{txid}", "chain": scope.protocol_chain, "network": scope.network, "external_id": txid, "raw_json": payload}, cached=True)
        for row in read("wallet_utxos"):
            builder.inventory(row)
        # Explicitly acquired reference observations are profile-scoped. Their
        # acquisition timestamp/status remain reference provenance, not current
        # wallet ownership or an automatic custody decision.
        if "chain_analysis_observations" in tables:
            for row in read("chain_analysis_observations"):
                payload = stored_tx_mapping(row.get("payload_json"))
                txid = canonical_txid(row.get("txid"))
                if payload is None or txid is None:
                    builder.coverage["invalid_observations"] += 1
                    continue
                try:
                    scope = resolve_protocol_scope(row)
                    payload_scope = resolve_protocol_scope({"chain": scope.protocol_chain, "network": scope.network, "raw_json": payload})
                except (ValueError, AppError):
                    builder.coverage["invalid_observations"] += 1
                    continue
                if payload_scope != scope or canonical_txid(payload.get("txid")) not in (None, txid) or scope.protocol_chain not in {"bitcoin", "liquid"}:
                    builder.coverage["invalid_observations"] += 1
                    continue
                node_id = tx_node_id(scope.protocol_chain, scope.network, txid)
                builder.transaction({"id": f"acquired:{scope.protocol_chain}:{scope.network}:{txid}", "chain": scope.protocol_chain, "network": scope.network, "external_id": txid, "raw_json": payload}, cached=True)
                if node_id in builder.nodes:
                    builder.profile_seeds.add(node_id)
                    # Closed source metadata only; never arbitrary status blobs.
                    status = stored_tx_mapping(row.get("status_json")) or {}
                    node = builder.nodes[node_id]
                    node["evidence"].append({"source": "local_acquisition", "reference": node_id, "level": "reference", "observed_at": row.get("observed_at"), "source_name": row.get("source_name"), "confirmed": status.get("confirmed") if isinstance(status.get("confirmed"), bool) else None, "block_height": integer(status.get("block_height")), "status_commitment": digest(status)})
                    if status.get("removed") is True or status.get("conflicted") is True:
                        node["status"] = "stale"
        if "chain_analysis_labels" in tables:
            for row in read("chain_analysis_labels"):
                if row.get("deleted"):
                    continue
                try:
                    scope = resolve_protocol_scope(row)
                except (ValueError, AppError):
                    builder.coverage["invalid_observations"] += 1
                    continue
                builder.labels.append({**row, "chain": scope.protocol_chain, "network": scope.network})
            builder.labels.sort(key=lambda row: str(row.get("id")))
        profiles = []
        if "profiles" in tables:
            cursor = conn.execute("SELECT * FROM profiles WHERE id = ?", (profile_id,))
            names = [item[0] for item in cursor.description]
            profiles = [dict(zip(names, row)) for row in cursor]
        profile = profiles[0] if profiles else {}
        fresh = bool(profile.get("last_processed_at")) and profile.get("journal_input_version") == profile.get("last_processed_input_version") and profile.get("last_processed_tx_count") == sum(not row.get("excluded") for row in transactions)
        builder.relations(read("journal_custody_decisions"), read("journal_custody_economic_relations"), fresh)
        if not fresh:
            builder.finding("custody_projection_stale", [], "Canonical custody relations require a current journal rebuild before they can be traversed.", severity="info")
        result = builder.finish()
    finally:
        conn.execute("RELEASE SAVEPOINT chain_analysis_read")
    return result

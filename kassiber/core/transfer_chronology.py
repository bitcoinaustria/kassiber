"""Order same-time MOVE allocations using observed chain dependencies first."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Sequence

from ..transfers import canonical_txid, onchain_transfer_scope
from .chain_observer.provenance import row_has_current_authoritative_observation
from .onchain import stored_tx_mapping


@dataclass(frozen=True)
class ChainOrderEvidence:
    scope: tuple[str, str, str, str]
    parents: frozenset[tuple[str, str, str, str]]


def chain_order_evidence(row: Mapping[str, Any]) -> ChainOrderEvidence | None:
    """Capture proof before custody slicing changes the observed quantity/id.

    Imported graph-shaped JSON alone is insufficient. The closed observer
    provenance must still match the original graph and quantity exactly.
    """
    if not row_has_current_authoritative_observation(row):
        return None
    scope = onchain_transfer_scope(row)
    if scope is None:
        return None
    raw = stored_tx_mapping(row["raw_json"]) or {}
    graphs = [raw, *(raw[key] for key in ("tx", "ownership_graph")
                     if isinstance(raw.get(key), Mapping))]
    parents = set()
    for graph in graphs:
        inputs = graph.get("vin")
        if not isinstance(inputs, list):
            continue
        for entry in inputs:
            if not isinstance(entry, Mapping):
                continue
            txid = canonical_txid(entry.get("txid"))
            vout = entry.get("vout")
            if txid and type(vout) is int and vout >= 0:
                parents.add((scope[0], scope[1], txid, scope[3]))
    return ChainOrderEvidence(scope, frozenset(parents))


def order_same_time_transfers(
    transfer_items: Sequence[tuple[int, tuple[str, str]]],
    transfers_by_id: Mapping[str, Any],
) -> list[tuple[str, str]]:
    """Keep stable order for ties; wallet hints cannot contradict chain facts."""
    ids = [ident for _, (_, ident) in transfer_items]
    original_order = {ident: index for index, (_, ident) in transfer_items}
    outgoing: dict[str, set[str]] = {ident: set() for ident in ids}
    evidence = {
        ident: transfers_by_id[ident].out_row.get("custody_chain_order")
        for ident in ids
    }
    for source_id in ids:
        source = evidence[source_id]
        if not isinstance(source, ChainOrderEvidence):
            continue
        for target_id in ids:
            target = evidence[target_id]
            if (source_id != target_id and isinstance(target, ChainOrderEvidence)
                    and source.scope in target.parents):
                outgoing[source_id].add(target_id)
    has_chain_dependencies = any(outgoing.values())

    # Every wallet hint below may revisit the same native path. Cache the
    # immutable hard graph once; its already ordered pairs need no soft edge.
    hard_descendants: dict[str, set[str]] = {}
    if has_chain_dependencies:
        for ident in ids:
            pending = list(outgoing[ident])
            descendants: set[str] = set()
            while pending:
                target = pending.pop()
                if target not in descendants:
                    descendants.add(target)
                    pending.extend(outgoing[target])
            hard_descendants[ident] = descendants

    def reaches(start: str, end: str) -> bool:
        pending = [start]
        visited = set()
        while pending:
            current = pending.pop()
            if current == end:
                return True
            if current not in visited:
                visited.add(current)
                pending.extend(outgoing[current])
        return False

    # Preserve the existing graphless wallet-dependency behavior, but only add
    # hints that do not reverse an already proven chain (A -> B -> A is not a
    # transaction cycle). Same-event allocations have no temporal relationship.
    for source_id in ids:
        source = transfers_by_id[source_id]
        for target_id in ids:
            target = transfers_by_id[target_id]
            source_proof, target_proof = evidence[source_id], evidence[target_id]
            if (isinstance(source_proof, ChainOrderEvidence)
                    and isinstance(target_proof, ChainOrderEvidence)
                    and source_proof.scope == target_proof.scope):
                continue
            if source_id == target_id or source.to_wallet_id != target.from_wallet_id:
                continue
            if has_chain_dependencies:
                if (target_id in hard_descendants[source_id]
                        or source_id in hard_descendants[target_id]):
                    continue
                if reaches(target_id, source_id):
                    continue
            outgoing[source_id].add(target_id)

    incoming = dict.fromkeys(ids, 0)
    for targets in outgoing.values():
        for target in targets:
            incoming[target] += 1
    ready = sorted((ident for ident in ids if incoming[ident] == 0), key=original_order.get)
    ordered = []
    while ready:
        ident = ready.pop(0)
        ordered.append(("transfer", ident))
        for target in sorted(outgoing[ident], key=original_order.get):
            incoming[target] -= 1
            if incoming[target] == 0:
                ready.append(target)
        ready.sort(key=original_order.get)
    if len(ordered) != len(ids):
        return [item for _, item in sorted(transfer_items)]
    return ordered

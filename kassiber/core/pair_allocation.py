"""Deterministic ordering + fee allocation for multi-leg transfer components.

Every consumer that walks a manual multi-pair component — tax booking,
per-country regime inference, any future country module — MUST use the
same leg order and the same allocator. Source-level fees preserve recorded
evidence; only an unexplained residual is routed by the deterministic flow.
Within one source's fan-out, the fee is then split in canonical leg order.
This module is the single country-agnostic source of truth for both.
"""

from __future__ import annotations

from collections import deque
from typing import Any, Mapping, Sequence


def _field(row: Mapping[str, Any], key: str) -> Any:
    if hasattr(row, "get"):
        return row.get(key)
    return row[key] if key in row.keys() else None


def ordered_pair_component(
    component: Sequence[Mapping[str, Mapping[str, Any]]],
) -> list[Mapping[str, Mapping[str, Any]]]:
    """Canonical chronological leg order for a multi-pair component.

    Sorted by both legs' timestamps, then row ids — never by pair-record
    id, whose creation order carries no economic meaning.
    """
    return sorted(
        component,
        key=lambda pair: (
            str(_field(pair["out"], "occurred_at") or ""),
            str(_field(pair["in"], "occurred_at") or ""),
            str(pair["out"]["id"]),
            str(pair["in"]["id"]),
        ),
    )


#: Below one satoshi, a receipt "excess" can only be representation noise:
#: LND's REST fallback stores sat-truncated values (up to 999 msat under the
#: true amount) while a CLN partner leg is msat-exact.
SUB_SAT_MSAT = 1000


def outbound_transfer_evidence_msat(
    row: Mapping[str, Any],
) -> tuple[int, int, bool]:
    """Return ``(sent, explicit_fee, has_implicit_fee)`` for an outbound row.

    Most backends report principal in ``amount`` and the miner/routing fee in
    ``fee``. Net-delta imports instead set ``amount_includes_fee``; for those,
    ``amount`` is already the total sent and any gap to the receipt is implicit.
    Keeping this interpretation here prevents booking and country-specific lot
    tracking from drifting on mixed-source components.
    """

    amount_msat = int(_field(row, "amount") or 0)
    fee_msat = int(_field(row, "fee") or 0)
    has_implicit_fee = bool(_field(row, "amount_includes_fee"))
    if has_implicit_fee:
        return amount_msat, 0, True
    return amount_msat + fee_msat, max(0, fee_msat), False


def clamped_receipt_msat(sent_msat: int, received_msat: int) -> int:
    """Clamp a sub-sat receipt excess down to the sent total.

    Booking and regime inference MUST apply this identically wherever they
    accept a transfer pair: clamping in one model but not the other either
    books a MOVE that inference never saw (regimes/pools desync) or vice
    versa. Anything >= 1 sat of excess is returned unchanged so real
    mismatches still fail the caller's sent < received guard.
    """
    if 0 < received_msat - sent_msat < SUB_SAT_MSAT:
        return sent_msat
    return received_msat


def clamped_component_receipts_msat(
    sent_msat: int, received_amounts_msat: Sequence[int]
) -> list[int]:
    """Clamp aggregate sub-sat receipt excess across a multi-leg component."""
    adjusted = [max(0, int(amount)) for amount in received_amounts_msat]
    clamped_total = clamped_receipt_msat(int(sent_msat), sum(adjusted))
    excess = sum(adjusted) - clamped_total
    if excess <= 0:
        return adjusted
    for index in range(len(adjusted) - 1, -1, -1):
        if excess <= 0:
            break
        reduction = min(adjusted[index], excess)
        adjusted[index] -= reduction
        excess -= reduction
    return adjusted


def allocate_fee_msat(total_fee_msat: int, bases: Sequence[int]) -> list[int]:
    """Allocate an aggregate multi-link fee greedily across legs.

    Each leg absorbs up to its own base; any residual lands on the last
    leg. Negative bases never produce a negative allocation.
    """
    remaining = max(0, int(total_fee_msat))
    allocated: list[int] = []
    for base in bases:
        if remaining <= 0:
            allocated.append(0)
            continue
        portion = min(max(0, int(base)), remaining)
        allocated.append(portion)
        remaining -= portion
    if remaining and allocated:
        allocated[-1] += remaining
    return allocated


def allocate_bipartite_flow_msat(
    supplies: Mapping[str, int],
    demands: Mapping[str, int],
    edges: Sequence[tuple[str, str]],
) -> list[tuple[str, str, int]] | None:
    """Find a deterministic exact flow over reviewed source/destination edges.

    A custody component is a small bipartite transportation graph. A greedy
    edge walk is not correct here: it can consume a constrained destination
    with a flexible source and report an otherwise feasible component as
    unbalanced. This is an Edmonds-Karp max-flow with stable insertion-order
    traversal, so it can reroute earlier allocations through residual edges and
    still produces reproducible audit output.

    ``None`` means the reviewed edge set cannot satisfy every positive supply
    and demand exactly. Unknown endpoints are ignored; duplicate edges are
    coalesced. Zero-valued legs are valid but do not appear in the result.
    """

    normalized_supplies = {str(key): int(value) for key, value in supplies.items()}
    normalized_demands = {str(key): int(value) for key, value in demands.items()}
    if any(value < 0 for value in normalized_supplies.values()) or any(
        value < 0 for value in normalized_demands.values()
    ):
        return None
    total_supply = sum(normalized_supplies.values())
    if total_supply != sum(normalized_demands.values()):
        return None
    if total_supply == 0:
        return []

    source = ("root", "source")
    sink = ("root", "sink")
    capacity: dict[tuple[tuple[str, str], tuple[str, str]], int] = {}
    residual: dict[tuple[tuple[str, str], tuple[str, str]], int] = {}
    adjacency: dict[tuple[str, str], list[tuple[str, str]]] = {}

    def add_edge(left: tuple[str, str], right: tuple[str, str], limit: int) -> None:
        adjacency.setdefault(left, [])
        adjacency.setdefault(right, [])
        if right not in adjacency[left]:
            adjacency[left].append(right)
        if left not in adjacency[right]:
            adjacency[right].append(left)
        capacity[(left, right)] = capacity.get((left, right), 0) + max(0, limit)
        residual[(left, right)] = residual.get((left, right), 0) + max(0, limit)
        residual.setdefault((right, left), 0)

    for out_id, amount in normalized_supplies.items():
        add_edge(source, ("out", out_id), amount)
    for in_id, amount in normalized_demands.items():
        add_edge(("in", in_id), sink, amount)

    reviewed_edges: list[tuple[str, str]] = []
    seen_edges: set[tuple[str, str]] = set()
    for raw_out_id, raw_in_id in edges:
        edge = (str(raw_out_id), str(raw_in_id))
        if (
            edge in seen_edges
            or edge[0] not in normalized_supplies
            or edge[1] not in normalized_demands
        ):
            continue
        seen_edges.add(edge)
        reviewed_edges.append(edge)
        add_edge(("out", edge[0]), ("in", edge[1]), total_supply)

    max_flow = 0
    while max_flow < total_supply:
        parent: dict[tuple[str, str], tuple[str, str] | None] = {source: None}
        queue = deque([source])
        while queue and sink not in parent:
            node = queue.popleft()
            for neighbor in adjacency.get(node, ()):
                if neighbor in parent or residual.get((node, neighbor), 0) <= 0:
                    continue
                parent[neighbor] = node
                queue.append(neighbor)
                if neighbor == sink:
                    break
        if sink not in parent:
            break

        increment = total_supply - max_flow
        node = sink
        while parent[node] is not None:
            previous = parent[node]
            increment = min(increment, residual[(previous, node)])
            node = previous
        node = sink
        while parent[node] is not None:
            previous = parent[node]
            residual[(previous, node)] -= increment
            residual[(node, previous)] = residual.get((node, previous), 0) + increment
            node = previous
        max_flow += increment

    if max_flow != total_supply:
        return None

    result: list[tuple[str, str, int]] = []
    for out_id, in_id in reviewed_edges:
        left = ("out", out_id)
        right = ("in", in_id)
        amount = capacity[(left, right)] - residual[(left, right)]
        if amount > 0:
            result.append((out_id, in_id, amount))
    return result


def allocate_transfer_component_flow_msat(
    source_sent_msat: Mapping[str, int],
    source_explicit_fee_msat: Mapping[str, int],
    destination_amount_msat: Mapping[str, int],
    edges: Sequence[tuple[str, str]],
    *,
    implicit_fee_source_ids: Sequence[str] = (),
) -> tuple[list[tuple[str, str, int]], dict[str, int]] | None:
    """Allocate a reviewed multi-source transfer without moving recorded fees.

    Each source's explicit fee is a lower bound: it stays attached to the
    outbound row that reported it. Only the aggregate gap left after those
    explicit fees is unexplained and therefore eligible for deterministic
    allocation. Rows whose amount explicitly includes an unknown fee are tried
    first for that residual; the max-flow residual graph can still reroute the
    fee when an allowed custody edge makes that necessary.

    This is ownership/accounting plumbing, not tax policy. Country-specific
    consumers reuse the returned principal flows so their downstream lot state
    cannot disagree with the generic transfer journal.

    Returns ``(principal_edge_flows, total_fee_by_source)``. ``None`` means the
    reviewed graph, amounts, or explicit fee evidence cannot conserve exactly.
    """

    sent = {
        str(source_id): int(amount)
        for source_id, amount in source_sent_msat.items()
    }
    destinations = {
        str(destination_id): int(amount)
        for destination_id, amount in destination_amount_msat.items()
    }
    if any(amount < 0 for amount in sent.values()) or any(
        amount < 0 for amount in destinations.values()
    ):
        return None

    normalized_explicit = {
        str(source_id): int(fee or 0)
        for source_id, fee in source_explicit_fee_msat.items()
    }
    explicit = {
        source_id: normalized_explicit.get(source_id, 0) for source_id in sent
    }
    if any(fee < 0 or fee > sent[source_id] for source_id, fee in explicit.items()):
        return None
    if any(source_id not in sent for source_id in normalized_explicit):
        return None

    aggregate_fee = sum(sent.values()) - sum(destinations.values())
    explicit_total = sum(explicit.values())
    unexplained_fee = aggregate_fee - explicit_total
    if aggregate_fee < 0 or unexplained_fee < 0:
        return None

    remaining_supply = {
        source_id: sent[source_id] - explicit[source_id] for source_id in sent
    }
    implicit_ids = {str(source_id) for source_id in implicit_fee_source_ids}
    source_order = [source_id for source_id in sent if source_id in implicit_ids]
    source_order.extend(
        source_id for source_id in sent if source_id not in implicit_ids
    )
    ordered_supply = {
        source_id: remaining_supply[source_id] for source_id in source_order
    }

    fee_sink = "__kassiber_unexplained_transfer_fee__"
    while fee_sink in destinations:
        fee_sink += "_"
    augmented_demands = {fee_sink: unexplained_fee, **destinations}
    # Fee edges come first so genuinely implicit-fee rows absorb the residual
    # before principal allocation. Edmonds-Karp can undo that choice through
    # reverse edges if a constrained reviewed destination needs the source.
    augmented_edges = [
        *((source_id, fee_sink) for source_id in source_order),
        *(
            (str(source_id), str(destination_id))
            for source_id, destination_id in edges
        ),
    ]
    allocated = allocate_bipartite_flow_msat(
        ordered_supply,
        augmented_demands,
        augmented_edges,
    )
    if allocated is None:
        return None

    total_fee_by_source = dict(explicit)
    principal_flows: list[tuple[str, str, int]] = []
    for source_id, destination_id, amount in allocated:
        if destination_id == fee_sink:
            total_fee_by_source[source_id] += amount
        else:
            principal_flows.append((source_id, destination_id, amount))

    return principal_flows, total_fee_by_source


def pair_record_id(pair) -> str | None:
    """Stable review-record id of a pair (manual/bulk/rule pairing), or None."""
    raw = pair.get("pair_id") if hasattr(pair, "get") else None
    if raw in (None, ""):
        raw = pair.get("id") if hasattr(pair, "get") else None
    if raw in (None, ""):
        return None
    return str(raw)


def pair_group_id(pair) -> str | None:
    """Derived-transfer group id carried by engine-derived pairs, or None."""
    raw = pair.get("group_id") if hasattr(pair, "get") else None
    if raw in (None, ""):
        return None
    return str(raw)


def is_component_member(pair) -> bool:
    """Whether a pair may FORM a shared multi-pair component.

    Only reviewed pair records that are not engine-derived group legs merge
    into components; auto-detected and derived pairs stay singletons. Booking
    and every regime-inference consumer must use the same membership, or the
    two models allocate the shared fee over different pair sets.
    """
    return pair_record_id(pair) is not None and pair_group_id(pair) is None


def connected_pair_components(items, leg_ids, membership=None):
    """Group pair-like items into connected components over shared leg ids.

    ``leg_ids`` maps an item to its ``(out_id, in_id)`` (return None to drop
    the item). Items rejected by ``membership`` never join a shared
    component; they trail the member components as singletons. This is the
    single component builder for booking, regime inference, and
    source-of-funds allocation — one membership, one traversal, no drift.
    """
    member_items: list[tuple] = []
    singletons: list = []
    for item in items:
        ids = leg_ids(item)
        if ids is None:
            continue
        if membership is not None and not membership(item):
            singletons.append(item)
            continue
        member_items.append((item, (str(ids[0]), str(ids[1]))))

    row_to_indexes: dict[str, list[int]] = {}
    for index, (_item, ids) in enumerate(member_items):
        for row_id in ids:
            row_to_indexes.setdefault(row_id, []).append(index)

    components: list[list] = []
    seen: set[int] = set()
    for start in range(len(member_items)):
        if start in seen:
            continue
        stack = [start]
        seen.add(start)
        indexes: list[int] = []
        while stack:
            index = stack.pop()
            indexes.append(index)
            for row_id in member_items[index][1]:
                for linked in row_to_indexes.get(row_id, ()):
                    if linked not in seen:
                        seen.add(linked)
                        stack.append(linked)
        components.append([member_items[i][0] for i in sorted(indexes)])
    components.extend([item] for item in singletons)
    return components

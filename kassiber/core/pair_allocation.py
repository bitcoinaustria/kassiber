"""Deterministic ordering + fee allocation for multi-leg transfer components.

Every consumer that walks a manual multi-pair component — tax booking,
per-country regime inference, any future country module — MUST use the
same leg order and the same allocator: the allocator is greedy (the fee
lands on the first legs), so two consumers walking different orders book
the fee on different legs and silently drift apart. This module is the
single country-agnostic source of truth for both.
"""

from __future__ import annotations

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

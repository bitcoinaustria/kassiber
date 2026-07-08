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

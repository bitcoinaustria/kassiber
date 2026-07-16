"""Pure allocation contract shared by custody quantity and tax projection."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping


OWNED_SINK_ROLES = frozenset({"destination", "retained"})
ATTRIBUTED_SINK_ROLES = frozenset({"fee", "external"})
SUSPENSE_SINK_ROLES = frozenset({"suspense"})


class CustodyAllocationError(Exception):
    """A component cannot be projected under the shared allocation contract."""

    def __init__(self, code: str, message: str, **details: Any):
        super().__init__(message)
        self.code = code
        self.message = message
        self.details = details


@dataclass(frozen=True)
class DeterministicAllocationCell:
    """One exact FIFO source-to-sink allocation with half-open offsets."""

    source_id: str
    sink_id: str
    amount_msat: int
    source_start_msat: int
    sink_start_msat: int

    @property
    def source_end_msat(self) -> int:
        return self.source_start_msat + self.amount_msat

    @property
    def sink_end_msat(self) -> int:
        return self.sink_start_msat + self.amount_msat


@dataclass(frozen=True)
class DeterministicAllocationResult:
    """A complete allocation plus ordered residual capacities."""

    cells: tuple[DeterministicAllocationCell, ...]
    allocated_msat: int
    source_remaining: tuple[tuple[str, int], ...]
    sink_remaining: tuple[tuple[str, int], ...]


def _allocation_buckets(
    raw: Any,
    *,
    kind: str,
) -> tuple[tuple[str, int], ...]:
    try:
        buckets = tuple(raw)
    except TypeError as exc:
        raise CustodyAllocationError(
            "custody_allocation_invalid_buckets",
            f"{kind} allocation buckets must be a sequence",
            kind=kind,
        ) from exc
    normalized: list[tuple[str, int]] = []
    for ordinal, bucket in enumerate(buckets):
        try:
            bucket_id, amount_msat = bucket
        except (TypeError, ValueError) as exc:
            raise CustodyAllocationError(
                "custody_allocation_invalid_bucket",
                f"each {kind} allocation bucket must contain id and amount",
                kind=kind,
                ordinal=ordinal,
            ) from exc
        bucket_id = str(bucket_id or "")
        if not bucket_id or type(amount_msat) is not int or amount_msat < 0:
            raise CustodyAllocationError(
                "custody_allocation_invalid_bucket",
                f"{kind} allocation bucket ids and amounts must be exact",
                kind=kind,
                ordinal=ordinal,
                bucket_id=bucket_id,
                amount_msat=amount_msat,
            )
        normalized.append((bucket_id, amount_msat))
    ids = [bucket_id for bucket_id, _amount in normalized]
    if len(ids) != len(set(ids)):
        raise CustodyAllocationError(
            "custody_allocation_duplicate_bucket",
            f"{kind} allocation bucket ids must be unique",
            kind=kind,
        )
    return tuple(normalized)


def allocate_msat_fifo(
    sources: Any,
    sinks: Any,
    *,
    amount_msat: int | None = None,
) -> DeterministicAllocationResult:
    """Allocate an exact msat quantity across ordered N:M boundaries.

    Input order is the only priority rule. The returned offsets are stable
    accounting coordinates, not claims about physical sat ordering. The
    operation is all-or-nothing: insufficient source or sink capacity raises
    before a result can escape.
    """

    source_buckets = _allocation_buckets(sources, kind="source")
    sink_buckets = _allocation_buckets(sinks, kind="sink")
    requested = (
        sum(amount for _bucket_id, amount in sink_buckets)
        if amount_msat is None
        else amount_msat
    )
    if type(requested) is not int or requested < 0:
        raise CustodyAllocationError(
            "custody_allocation_invalid_amount",
            "allocation amount must be an exact non-negative integer",
            amount_msat=requested,
        )
    source_total = sum(amount for _bucket_id, amount in source_buckets)
    sink_total = sum(amount for _bucket_id, amount in sink_buckets)
    if requested > source_total or requested > sink_total:
        raise CustodyAllocationError(
            "custody_allocation_insufficient_capacity",
            "allocation boundaries cannot satisfy the requested quantity",
            amount_msat=requested,
            source_capacity_msat=source_total,
            sink_capacity_msat=sink_total,
        )

    source_used = [0] * len(source_buckets)
    sink_used = [0] * len(sink_buckets)
    source_index = sink_index = 0
    remaining = requested
    cells: list[DeterministicAllocationCell] = []
    while remaining:
        while source_used[source_index] == source_buckets[source_index][1]:
            source_index += 1
        while sink_used[sink_index] == sink_buckets[sink_index][1]:
            sink_index += 1
        source_id, source_capacity = source_buckets[source_index]
        sink_id, sink_capacity = sink_buckets[sink_index]
        amount = min(
            remaining,
            source_capacity - source_used[source_index],
            sink_capacity - sink_used[sink_index],
        )
        cells.append(
            DeterministicAllocationCell(
                source_id=source_id,
                sink_id=sink_id,
                amount_msat=amount,
                source_start_msat=source_used[source_index],
                sink_start_msat=sink_used[sink_index],
            )
        )
        source_used[source_index] += amount
        sink_used[sink_index] += amount
        remaining -= amount

    return DeterministicAllocationResult(
        cells=tuple(cells),
        allocated_msat=requested,
        source_remaining=tuple(
            (bucket_id, capacity - source_used[index])
            for index, (bucket_id, capacity) in enumerate(source_buckets)
        ),
        sink_remaining=tuple(
            (bucket_id, capacity - sink_used[index])
            for index, (bucket_id, capacity) in enumerate(sink_buckets)
        ),
    )


def infer_component_allocations(
    component: Mapping[str, Any],
) -> list[dict[str, Any]]:
    """Return explicit allocations or the deterministic 1:N/N:1 form."""

    explicit = [dict(item) for item in component.get("allocations", ())]
    if explicit:
        return sorted(
            explicit,
            key=lambda item: (
                int(item.get("ordinal") or 0),
                str(item.get("id") or ""),
            ),
        )

    legs = [
        leg
        for leg in component.get("legs", ())
        if int(leg.get("amount_msat") or 0) > 0
    ]
    sources = [leg for leg in legs if leg.get("role") == "source"]
    sinks = [leg for leg in legs if leg.get("role") != "source"]
    quantity_mode = (
        str(component.get("conservation_mode") or "quantity") == "quantity"
    )
    if not quantity_mode and (len(sources) != 1 or len(sinks) != 1):
        raise CustodyAllocationError(
            "custody_component_allocation_required",
            "multi-leg conversion components require explicit source-to-sink allocations",
        )
    if len(sources) == 1:
        source = sources[0]
        return [
            {
                "id": f"inferred:{index}",
                "ordinal": index,
                "source_leg_id": source["id"],
                "sink_leg_id": sink["id"],
                # A reviewed conversion may have unlike quantities only in
                # the unambiguous 1:1 shape checked above.
                "source_amount_msat": (
                    int(sink["amount_msat"])
                    if quantity_mode
                    else int(source["amount_msat"])
                ),
                "sink_amount_msat": int(sink["amount_msat"]),
            }
            for index, sink in enumerate(sinks)
        ]
    if len(sinks) == 1 and sinks[0].get("role") in OWNED_SINK_ROLES:
        sink = sinks[0]
        return [
            {
                "id": f"inferred:{index}",
                "ordinal": index,
                "source_leg_id": source["id"],
                "sink_leg_id": sink["id"],
                "source_amount_msat": int(source["amount_msat"]),
                "sink_amount_msat": int(source["amount_msat"]),
            }
            for index, source in enumerate(sources)
        ]
    raise CustodyAllocationError(
        "custody_component_allocation_required",
        "N:M custody components require explicit source-to-sink allocations",
    )


__all__ = [
    "ATTRIBUTED_SINK_ROLES",
    "CustodyAllocationError",
    "DeterministicAllocationCell",
    "DeterministicAllocationResult",
    "OWNED_SINK_ROLES",
    "SUSPENSE_SINK_ROLES",
    "allocate_msat_fifo",
    "infer_component_allocations",
]

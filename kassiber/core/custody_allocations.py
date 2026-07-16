"""Pure allocation contract shared by custody quantity and tax projection."""

from __future__ import annotations

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
    "OWNED_SINK_ROLES",
    "SUSPENSE_SINK_ROLES",
    "infer_component_allocations",
]

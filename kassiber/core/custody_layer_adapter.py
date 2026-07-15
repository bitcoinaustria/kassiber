"""Tax-neutral input contract for future Bitcoin custody layers.

Ark, Bark, and other layer-specific adapters are intentionally out of scope.
This small seam pins what any such adapter must provide before an interpreter
can make custody claims: native identity and relations, exact quantity and
fees, custody/finality/exit state, and bounded evidence provenance.
"""

from __future__ import annotations

from dataclasses import dataclass
import json
import re
from typing import Any, Mapping, Protocol, Sequence

from .custody_evidence import CanonicalQuantityInput, build_canonical_quantity_input


_TOKEN = re.compile(r"[a-z0-9][a-z0-9_.:-]{0,127}")
_DIRECTIONS = frozenset({"inbound", "outbound"})
_CUSTODY_STATES = frozenset({"owned", "external", "unknown"})
_FINALITY_STATES = frozenset({"pending", "final", "reorged", "unknown"})
_EXIT_STATES = frozenset(
    {"none", "cooperative", "unilateral", "expired", "settled", "unknown"}
)


def _token(value: Any, field: str) -> str:
    text = str(value or "").strip().lower()
    if not _TOKEN.fullmatch(text):
        raise ValueError(f"{field} must be a bounded native token")
    return text


def _native_refs(values: Sequence[str], field: str) -> tuple[str, ...]:
    return tuple(sorted({_token(value, field) for value in values}))


@dataclass(frozen=True)
class CustodyLayerEvent:
    """One wallet-local aggregate emitted by a Bitcoin-layer adapter."""

    layer: str
    network: str
    native_namespace: str
    native_event_id: str
    wallet_id: str
    direction: str
    asset: str
    exposure: str
    amount_msat: int
    fee_msat: int
    occurred_at: str
    custody_state: str
    finality_state: str
    exit_state: str
    parent_event_ids: tuple[str, ...] = ()
    spent_event_ids: tuple[str, ...] = ()
    evidence_provenance: tuple[tuple[str, str], ...] = ()

    def __post_init__(self) -> None:
        for field in (
            "layer",
            "network",
            "native_namespace",
            "native_event_id",
            "wallet_id",
            "exposure",
        ):
            _token(getattr(self, field), field)
        if self.direction not in _DIRECTIONS:
            raise ValueError("direction must be inbound or outbound")
        if not self.asset or self.asset != self.asset.upper():
            raise ValueError("asset must be a non-empty uppercase code")
        if (
            type(self.amount_msat) is not int
            or type(self.fee_msat) is not int
            or self.amount_msat < 0
            or self.fee_msat < 0
        ):
            raise ValueError("layer quantity and fee must be non-negative integers")
        if not self.occurred_at:
            raise ValueError("layer events require occurred_at")
        if self.custody_state not in _CUSTODY_STATES:
            raise ValueError("unsupported custody_state")
        if self.finality_state not in _FINALITY_STATES:
            raise ValueError("unsupported finality_state")
        if self.exit_state not in _EXIT_STATES:
            raise ValueError("unsupported exit_state")
        _native_refs(self.parent_event_ids, "parent_event_id")
        _native_refs(self.spent_event_ids, "spent_event_id")
        if len(self.evidence_provenance) > 32:
            raise ValueError("evidence provenance is bounded to 32 fields")
        for key, value in self.evidence_provenance:
            _token(key, "evidence_provenance.key")
            if not isinstance(value, str) or len(value) > 512:
                raise ValueError("evidence provenance values must be bounded strings")

    def to_quantity_row(self) -> dict[str, Any]:
        """Return the existing canonical quantity boundary, with rich evidence.

        A future interpreter consumes the layer metadata from the immutable
        evidence snapshot and emits normal ``QuantityClaim`` values. No tax
        engine type or country policy crosses this boundary.
        """

        native_id = _token(self.native_event_id, "native_event_id")
        namespace = _token(self.native_namespace, "native_namespace")
        wallet_id = _token(self.wallet_id, "wallet_id")
        evidence = {
            "schema_version": 1,
            "layer": _token(self.layer, "layer"),
            "exposure": _token(self.exposure, "exposure"),
            "custody_state": self.custody_state,
            "finality_state": self.finality_state,
            "exit_state": self.exit_state,
            "parent_event_ids": list(
                _native_refs(self.parent_event_ids, "parent_event_id")
            ),
            "spent_event_ids": list(
                _native_refs(self.spent_event_ids, "spent_event_id")
            ),
            "provenance": dict(sorted(self.evidence_provenance)),
        }
        return {
            "id": f"layer:{namespace}:{native_id}:{wallet_id}:{self.direction}",
            "native_event_id": native_id,
            "native_namespace": namespace,
            # Future Bitcoin layers inherit Bitcoin's chain/network quantity
            # domain. Their distinct rail identity remains explicit evidence
            # and must be opted into by the layer interpreter.
            "chain": "bitcoin",
            "network": _token(self.network, "network"),
            "wallet_id": wallet_id,
            "direction": self.direction,
            "asset": self.asset,
            "amount": self.amount_msat,
            "fee": self.fee_msat,
            "occurred_at": self.occurred_at,
            "raw_json": json.dumps(
                {"_kassiber_custody_layer": evidence},
                sort_keys=True,
                separators=(",", ":"),
            ),
        }


class CustodyLayerAdapter(Protocol):
    """Read-only adapter surface; transport and authentication stay outside."""

    def custody_events(self) -> Sequence[CustodyLayerEvent]:
        raise NotImplementedError


def build_layer_quantity_input(
    adapter: CustodyLayerAdapter,
) -> CanonicalQuantityInput:
    events = tuple(adapter.custody_events())
    rows = [event.to_quantity_row() for event in events]
    return build_canonical_quantity_input(rows)


def layer_evidence_from_observation(payload_json: str) -> Mapping[str, Any]:
    """Read the typed layer envelope from an immutable evidence snapshot."""

    payload = json.loads(payload_json)
    raw = payload.get("raw_json", {}) if isinstance(payload, Mapping) else {}
    evidence = raw.get("_kassiber_custody_layer", {}) if isinstance(raw, Mapping) else {}
    return evidence if isinstance(evidence, Mapping) else {}


__all__ = [
    "CustodyLayerAdapter",
    "CustodyLayerEvent",
    "build_layer_quantity_input",
    "layer_evidence_from_observation",
]

"""Compact storage for the tax calculation retained on journal entries.

A realized entry keeps the engine's calculation (``ledger_response``'s
``calculation``) so ``reports explain`` can show it later. Stored as sorted
JSON objects, the calculations were about 72% of ``journal_entries`` bytes:
every fragment repeated its event, every source repeated twelve pricing keys,
and every msat amount was stored twice.

``calculation_compact`` stores the same value as one positional JSON array::

    [2, engine, method, sources, fragments]

``sources`` holds each distinct event or lot once, in order of first use
(event before lot, fragment by fragment)::

    [transaction_id, engine_event_id, occurred_at, asset, spot_price_exact,
     fiat_fee_exact, crypto_fee_msat, <the PRICING_KEYS values in order>]

and each fragment refers to its sources by index (or ``null``)::

    [quantity_msat, cost_basis_exact, proceeds_exact, gain_loss_exact,
     unit_basis_override_exact, basis_authority, acquisition_basis_exact,
     acquisition_quantity_msat, event_index, lot_index]

The ``*_msat_exact`` strings are restored as ``str`` of their integers. The
encoding is used only when that restores exactly what ``json.loads`` returns
for the sorted object, including key order and value types. Any other
calculation keeps the legacy ``calculation_json`` text, byte for byte. A row
has at most one of the two columns set.
"""

from __future__ import annotations

import json
from typing import Any, Mapping, Optional

COMPACT_VERSION = 2

_TOP_KEYS = frozenset(("engine", "fragments", "method", "schema_version"))
_FRAGMENT_KEYS = frozenset(
    (
        "acquisition_basis_exact",
        "acquisition_quantity_msat",
        "acquisition_quantity_msat_exact",
        "basis_authority",
        "cost_basis_exact",
        "event",
        "gain_loss_exact",
        "lot",
        "proceeds_exact",
        "quantity_msat",
        "quantity_msat_exact",
        "unit_basis_override_exact",
    )
)
_SOURCE_KEYS = frozenset(
    (
        "asset",
        "crypto_fee_msat",
        "crypto_fee_msat_exact",
        "engine_event_id",
        "fiat_fee_exact",
        "occurred_at",
        "pricing",
        "spot_price_exact",
        "transaction_id",
    )
)
# Sorted, as ``json.dumps(sort_keys=True)`` wrote them and ``json.loads``
# returns them.
PRICING_KEYS = (
    "fiat_currency",
    "fiat_price_source",
    "fiat_rate_exact",
    "fiat_value_exact",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
    "pricing_pair",
    "pricing_provider",
    "pricing_quality",
    "pricing_source_kind",
    "pricing_timestamp",
)
_PRICING_KEYS = frozenset(PRICING_KEYS)
_SOURCE_WIDTH = 7 + len(PRICING_KEYS)


class _NotCompact(Exception):
    pass


def _text(value: Any) -> Any:
    if value is None or type(value) is str:
        return value
    raise _NotCompact


def _integer(value: Any) -> int:
    if type(value) is int:
        return value
    raise _NotCompact


def _source(source: Any) -> Optional[tuple[Any, ...]]:
    if source is None:
        return None
    if type(source) is not dict or source.keys() != _SOURCE_KEYS:
        raise _NotCompact
    pricing = source["pricing"]
    if type(pricing) is not dict or pricing.keys() != _PRICING_KEYS:
        raise _NotCompact
    fee = _integer(source["crypto_fee_msat"])
    if source["crypto_fee_msat_exact"] != str(fee):
        raise _NotCompact
    return (
        _text(source["transaction_id"]),
        _text(source["engine_event_id"]),
        _text(source["occurred_at"]),
        _text(source["asset"]),
        _text(source["spot_price_exact"]),
        _text(source["fiat_fee_exact"]),
        fee,
        *(_text(pricing[key]) for key in PRICING_KEYS),
    )


def _compact(calculation: Any) -> str:
    if type(calculation) is not dict or calculation.keys() != _TOP_KEYS:
        raise _NotCompact
    if calculation["schema_version"] != 1 or type(calculation["schema_version"]) is not int:
        raise _NotCompact
    engine = calculation["engine"]
    method = calculation["method"]
    fragments = calculation["fragments"]
    if type(engine) is not str or type(method) is not str or type(fragments) is not list:
        raise _NotCompact
    sources: list[tuple[Any, ...]] = []
    positions: dict[tuple[Any, ...], int] = {}

    def reference(source: Any) -> Optional[int]:
        encoded = _source(source)
        if encoded is None:
            return None
        position = positions.get(encoded)
        if position is None:
            position = positions[encoded] = len(sources)
            sources.append(encoded)
        return position

    compact_fragments = []
    for fragment in fragments:
        if type(fragment) is not dict or fragment.keys() != _FRAGMENT_KEYS:
            raise _NotCompact
        quantity = _integer(fragment["quantity_msat"])
        if fragment["quantity_msat_exact"] != str(quantity):
            raise _NotCompact
        acquired = fragment["acquisition_quantity_msat"]
        if acquired is None:
            if fragment["acquisition_quantity_msat_exact"] is not None:
                raise _NotCompact
        elif fragment["acquisition_quantity_msat_exact"] != str(_integer(acquired)):
            raise _NotCompact
        compact_fragments.append(
            (
                quantity,
                _text(fragment["cost_basis_exact"]),
                _text(fragment["proceeds_exact"]),
                _text(fragment["gain_loss_exact"]),
                _text(fragment["unit_basis_override_exact"]),
                _text(fragment["basis_authority"]),
                _text(fragment["acquisition_basis_exact"]),
                acquired,
                reference(fragment["event"]),
                reference(fragment["lot"]),
            )
        )
    return json.dumps(
        [COMPACT_VERSION, engine, method, sources, compact_fragments],
        separators=(",", ":"),
    )


def encode_calculation(calculation: Any) -> tuple[Optional[str], Optional[str]]:
    """``(calculation_json, calculation_compact)`` for one entry's calculation."""

    if not calculation:
        return None, None
    try:
        return None, _compact(calculation)
    except _NotCompact:
        return json.dumps(calculation, sort_keys=True), None


def _decode_source(sources: list[Any], index: Any) -> Optional[dict[str, Any]]:
    if index is None:
        return None
    if type(index) is not int or not 0 <= index < len(sources):
        raise ValueError("calculation source index")
    source = sources[index]
    if type(source) is not list or len(source) != _SOURCE_WIDTH:
        raise ValueError("calculation source")
    (transaction_id, engine_event_id, occurred_at, asset, spot_price, fiat_fee, fee) = source[:7]
    if type(fee) is not int:
        raise ValueError("calculation source fee")
    return {
        "asset": asset,
        "crypto_fee_msat": fee,
        "crypto_fee_msat_exact": str(fee),
        "engine_event_id": engine_event_id,
        "fiat_fee_exact": fiat_fee,
        "occurred_at": occurred_at,
        "pricing": dict(zip(PRICING_KEYS, source[7:])),
        "spot_price_exact": spot_price,
        "transaction_id": transaction_id,
    }


def decode_compact(text: str) -> dict[str, Any]:
    """The calculation ``calculation_compact`` stores; ``ValueError`` if malformed."""

    value = json.loads(text)
    if type(value) is not list or len(value) != 5 or value[0] != COMPACT_VERSION or type(value[0]) is not int:
        raise ValueError("calculation encoding")
    _, engine, method, sources, fragments = value
    if type(sources) is not list or type(fragments) is not list:
        raise ValueError("calculation encoding")
    decoded = []
    for fragment in fragments:
        if type(fragment) is not list or len(fragment) != 10:
            raise ValueError("calculation fragment")
        (quantity, cost_basis, proceeds, gain_loss, override, authority, basis, acquired, event, lot) = fragment
        if type(quantity) is not int or (acquired is not None and type(acquired) is not int):
            raise ValueError("calculation fragment quantity")
        decoded.append(
            {
                "acquisition_basis_exact": basis,
                "acquisition_quantity_msat": acquired,
                "acquisition_quantity_msat_exact": None if acquired is None else str(acquired),
                "basis_authority": authority,
                "cost_basis_exact": cost_basis,
                "event": _decode_source(sources, event),
                "gain_loss_exact": gain_loss,
                "lot": _decode_source(sources, lot),
                "proceeds_exact": proceeds,
                "quantity_msat": quantity,
                "quantity_msat_exact": str(quantity),
                "unit_basis_override_exact": override,
            }
        )
    return {"engine": engine, "fragments": decoded, "method": method, "schema_version": 1}


def legacy_text(compact: str) -> str:
    """The ``calculation_json`` text a compact value was encoded from.

    Malformed compact text is returned unchanged, as a corrupt legacy column
    would be.
    """

    try:
        return json.dumps(decode_compact(compact), sort_keys=True)
    except (ValueError, TypeError):
        return compact


def stored_calculation(row: Mapping[str, Any]) -> Any:
    """A stored entry's calculation, or ``None`` when it retains none.

    The compact column wins when both are set. Malformed compact text raises
    ``ValueError``, as malformed legacy text does in ``json.loads``.
    """

    compact = row["calculation_compact"] if "calculation_compact" in row.keys() else None
    if compact:
        return decode_compact(compact)
    legacy = row["calculation_json"]
    if not legacy:
        return None
    return json.loads(legacy)


__all__ = [
    "COMPACT_VERSION",
    "PRICING_KEYS",
    "decode_compact",
    "encode_calculation",
    "legacy_text",
    "stored_calculation",
]

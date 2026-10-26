"""A tagged JSON codec for the tax-ledger goldens (plan 20 phase 4).

The projection-level goldens (``tax-engine/core/tests/data/ledger_fixtures.json``)
store each case's engine inputs and outcomes as JSON. Plain JSON loses what
the comparison checks: a ``Decimal``'s exponent, a tuple against a list, a
dict key that is a tuple, and the ``ChainOrderEvidence`` a projection row
carries. Those values are tagged as single-key objects whose key starts with
``$``; everything else is plain JSON, with dict key order and float ``repr``
preserved. A dict whose keys are not all plain strings is stored as a list of
pairs, so no stored key can be mistaken for a tag.

``encode_inputs`` and ``decode_inputs`` also keep one fact of a finalized
projection that values alone do not carry: a move pair's ``out`` and ``in``
are the projection's own row objects, not copies.
"""

from __future__ import annotations

import json
import math
from decimal import Decimal
from typing import Any, Mapping

from kassiber.core.custody_tax_projection import FinalizedTaxProjection
from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.transfer_chronology import ChainOrderEvidence

_DECIMAL = "$decimal"
_FLOAT = "$float"
_TUPLE = "$tuple"
_SET = "$set"
_FROZENSET = "$frozenset"
_DICT = "$dict"
_CHAIN_ORDER = "$chain_order"
_ROW = "$row"
_TAGS = frozenset({_DECIMAL, _FLOAT, _TUPLE, _SET, _FROZENSET, _DICT, _CHAIN_ORDER, _ROW})


def _sorted_members(values: Any) -> list[Any]:
    # A set has no order of its own; sort the encoded members so the stored
    # document does not depend on the process's string hashing.
    encoded = [encode(value) for value in values]
    return sorted(encoded, key=lambda item: json.dumps(item, sort_keys=True, separators=(",", ":")))


def encode(value: Any) -> Any:
    """``value`` as a JSON-ready document; ``decode`` gives it back."""
    if value is None or isinstance(value, (bool, int, str)):
        return value
    if isinstance(value, float):
        # JSON numbers keep a finite float exactly (json writes its repr).
        return value if math.isfinite(value) else {_FLOAT: repr(value)}
    if isinstance(value, Decimal):
        if type(value) is not Decimal:
            raise TypeError(f"cannot store a {type(value).__name__}; the goldens keep plain Decimals only")
        return {_DECIMAL: str(value)}
    if isinstance(value, ChainOrderEvidence):
        return {_CHAIN_ORDER: {"scope": list(value.scope), "parents": [list(item) for item in sorted(value.parents)]}}
    if isinstance(value, tuple):
        return {_TUPLE: [encode(item) for item in value]}
    if isinstance(value, list):
        return [encode(item) for item in value]
    if isinstance(value, frozenset):
        return {_FROZENSET: _sorted_members(value)}
    if isinstance(value, set):
        return {_SET: _sorted_members(value)}
    if isinstance(value, dict):
        if all(isinstance(key, str) and not key.startswith("$") for key in value):
            return {key: encode(item) for key, item in value.items()}
        return {_DICT: [[encode(key), encode(item)] for key, item in value.items()]}
    raise TypeError(f"cannot store a {type(value).__name__} in the tax-ledger goldens")


def decode(document: Any) -> Any:
    """The value ``encode`` stored."""
    if isinstance(document, list):
        return [decode(item) for item in document]
    if not isinstance(document, dict):
        return document
    if len(document) == 1:
        ((key, payload),) = document.items()
        if key in _TAGS:
            return _decode_tagged(key, payload)
    return {key: decode(item) for key, item in document.items()}


def _decode_tagged(tag: str, payload: Any) -> Any:
    if tag == _DECIMAL:
        return Decimal(payload)
    if tag == _FLOAT:
        return float(payload)
    if tag == _TUPLE:
        return tuple(decode(item) for item in payload)
    if tag == _FROZENSET:
        return frozenset(decode(item) for item in payload)
    if tag == _SET:
        return {decode(item) for item in payload}
    if tag == _DICT:
        return {decode(key): decode(item) for key, item in payload}
    if tag == _CHAIN_ORDER:
        return ChainOrderEvidence(tuple(payload["scope"]), frozenset(tuple(item) for item in payload["parents"]))
    raise ValueError(f"{tag} is only valid inside encoded engine inputs")


_PROJECTION_FIELDS = ("rows", "intra_pairs", "cross_asset_pairs", "quarantines", "selected_move_ids")


def encode_inputs(profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> dict[str, Any]:
    """One engine call's arguments: the profile and the ledger inputs.

    Only the ``TaxEngineLedgerInputs`` fields are kept; a test subclass's
    source material is not engine input. A pair leg that is one of the
    projection's rows is stored as ``{"$row": index}``.
    """
    projection = inputs.finalized_tax_projection
    row_index = {id(row): index for index, row in enumerate(projection.rows)}

    def leg(row: Any) -> Any:
        index = row_index.get(id(row))
        return {_ROW: index} if index is not None else encode(row)

    if type(projection.intra_pairs) is not tuple:
        raise TypeError(f"cannot store move pairs held in a {type(projection.intra_pairs).__name__}")
    pairs = []
    for pair in projection.intra_pairs:
        if not isinstance(pair, dict):
            raise TypeError(f"cannot store a {type(pair).__name__} move pair")
        pairs.append({key: leg(value) if key in ("out", "in") else encode(value) for key, value in pair.items()})
    return {
        "profile": encode(dict(profile)),
        "projection": {
            "rows": encode(projection.rows),
            "intra_pairs": {_TUPLE: pairs},
            "cross_asset_pairs": encode(projection.cross_asset_pairs),
            "quarantines": encode(projection.quarantines),
            "selected_move_ids": encode(projection.selected_move_ids),
        },
        "wallet_refs_by_id": encode(inputs.wallet_refs_by_id),
        "direct_payout_records": encode(inputs.direct_payout_records),
    }


def decode_inputs(document: Mapping[str, Any]) -> tuple[dict[str, Any], TaxEngineLedgerInputs]:
    """The profile and ledger inputs ``encode_inputs`` stored."""
    stored = document["projection"]
    rows = decode(stored["rows"])

    def leg(value: Any) -> Any:
        if isinstance(value, dict) and set(value) == {_ROW}:
            return rows[value[_ROW]]
        return decode(value)

    pairs = tuple(
        {key: leg(value) if key in ("out", "in") else decode(value) for key, value in pair.items()}
        for pair in stored["intra_pairs"][_TUPLE]
    )
    projection = FinalizedTaxProjection(
        rows=rows,
        intra_pairs=pairs,
        cross_asset_pairs=decode(stored["cross_asset_pairs"]),
        quarantines=decode(stored["quarantines"]),
        selected_move_ids=decode(stored["selected_move_ids"]),
    )
    inputs = TaxEngineLedgerInputs(
        finalized_tax_projection=projection,
        wallet_refs_by_id=decode(document["wallet_refs_by_id"]),
        direct_payout_records=decode(document["direct_payout_records"]),
    )
    return decode(document["profile"]), inputs


__all__ = ["decode", "decode_inputs", "encode", "encode_inputs"]

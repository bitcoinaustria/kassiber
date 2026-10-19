"""Kassiber's tax engine class: the finalized projection in, the journal ledger out.

``KassiberTaxEngine`` is the engine ``custody_journal.py`` runs (plan 20,
phase 4). For every non-empty book it builds the schema-2 request
(``ledger_request.build_request``), which runs the configuration checks and
refuses shapes a finalized projection cannot have, runs the Rust engine's
``ledger`` operation (``kassiber_tax.ledger``) on it, and renders the
response into the journal ledger (``ledger_response.read_ledger``). The
tests compare production with the frozen phase-3 adapter running on RP2
(``tests/tax_engine_compare.py``).
"""

from __future__ import annotations

from typing import Any, Mapping

import kassiber_tax

from ...tax_policy import require_tax_processing_supported
from .base import TaxEngineLedgerInputs, TaxEngineLedgerResult
from .ledger_request import build_request, dumps
from .ledger_response import empty_result, loads, raise_for_error, read_ledger

GENERIC_BITCOIN_RAIL_QUARANTINE_REASON = "bitcoin_rail_carry_basis_unresolved"


def _finalized_projection(inputs: TaxEngineLedgerInputs) -> Any:
    from ..custody_tax_projection import FinalizedTaxProjection

    projection = inputs.finalized_tax_projection
    if not isinstance(projection, FinalizedTaxProjection):
        # The phase-3 adapter's message, which the comparison checks.
        raise TypeError(
            "GenericRP2TaxEngine requires a FinalizedTaxProjection; raw rows are not a tax-engine input"
        )
    return projection


class KassiberTaxEngine:
    """The tax engine for every supported profile, over finalized projections."""

    def __init__(self, profile: Mapping[str, Any]):
        require_tax_processing_supported(profile)
        self.profile = profile

    def build_ledger_state(self, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        """The journal ledger of one finalized custody projection."""
        projection = _finalized_projection(inputs)
        if not projection.rows:
            # No configuration, method, or currency check runs on an empty
            # book, and nothing reaches an engine.
            return empty_result(inputs)
        request = build_request(self.profile, inputs)
        response = loads(kassiber_tax.ledger(dumps(request)))
        return read_ledger(self.profile, inputs, response)


def manifest() -> dict[str, Any]:
    """The engine's names and tables (the ``manifest`` operation)."""
    response = loads(kassiber_tax.ledger(dumps({"schema_version": 2, "operation": "manifest"})))
    raise_for_error(response)
    return {key: value for key, value in response.items() if key not in ("schema_version", "ok")}


__all__ = [
    "GENERIC_BITCOIN_RAIL_QUARANTINE_REASON",
    "KassiberTaxEngine",
    "manifest",
]

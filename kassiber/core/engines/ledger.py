"""Kassiber's tax engine class: the finalized projection in, the journal ledger out.

``KassiberTaxEngine`` is the engine ``custody_journal.py`` runs (plan 20,
phase 4). For every non-empty book it builds the schema-2 request
(``ledger_request.build_request``), which runs the configuration checks and
refuses shapes a finalized projection cannot have. It then routes the book:
to the Rust engine (``kassiber_tax.ledger``, read by ``ledger_response``)
when the engine implements every capability the book uses
(``ROUTED_CAPABILITIES``), and otherwise to the phase-3 adapter
(``engines/rp2.py``), unchanged. Capabilities move to the engine one at a
time while the tests compare production with the frozen oracle
(``tests/tax_engine_compare.py``); so far ``generic`` and ``at`` are routed,
so a generic book without the Bitcoin rail carry and an Austrian book
without a carrying-value cross-asset pair run on the engine.

A book's capabilities:

- ``generic``: a generic book;
- ``generic_carry``: a generic book with a carrying-value Bitcoin rail pair
  whose legs are projection rows (the carry loop);
- ``at``: an Austrian book;
- ``at_swap``: an Austrian book with a carrying-value cross-asset pair whose
  legs are projection rows (swap-link selection).
"""

from __future__ import annotations

from typing import Any, Mapping

import kassiber_tax

from ...tax_policy import normalize_tax_country, profile_value, require_tax_processing_supported
from .base import TaxEngineLedgerInputs, TaxEngineLedgerResult
from .ledger_request import build_request, dumps
from .ledger_response import empty_result, loads, raise_for_error, read_ledger

GENERIC_BITCOIN_RAIL_QUARANTINE_REASON = "bitcoin_rail_carry_basis_unresolved"

CAPABILITIES = ("generic", "generic_carry", "at", "at_swap")
# The capabilities the Rust engine implements; a book goes to the engine only
# when all of its capabilities are here.
ROUTED_CAPABILITIES: frozenset[str] = frozenset({"generic", "at"})

V2 = "v2"
LEGACY = "legacy"
# A call that returns or raises before routing: a projection without rows,
# raw rows, or a book the request builder refuses.
NOT_ROUTED = "none"
UNCLASSIFIED = "unclassified"


def capabilities(request: Mapping[str, Any]) -> frozenset[str]:
    """The capabilities a book's request uses."""
    row_ids = {row["id"] for row in request["rows"]}
    carried = [
        pair
        for pair in request["cross_asset_pairs"]
        if pair["policy"] == "carrying-value" and pair["out_id"] in row_ids and pair["in_id"] in row_ids
    ]
    if request["policy"]["country"]["kind"] == "at":
        return frozenset({"at", "at_swap"} if carried else {"at"})
    if any(pair["bitcoin_rail"] for pair in carried):
        return frozenset({"generic", "generic_carry"})
    return frozenset({"generic"})


def primary_capability(book_capabilities: frozenset[str]) -> str:
    """The capability a call counts under: the most specific one the book uses."""
    for capability in ("at_swap", "at", "generic_carry", "generic"):
        if capability in book_capabilities:
            return capability
    return UNCLASSIFIED


def route(book_capabilities: frozenset[str]) -> str:
    """``v2`` when the engine implements every capability, else ``legacy``."""
    return V2 if book_capabilities <= ROUTED_CAPABILITIES else LEGACY


def _finalized_projection(inputs: TaxEngineLedgerInputs) -> Any:
    from ..custody_tax_projection import FinalizedTaxProjection

    projection = inputs.finalized_tax_projection
    if not isinstance(projection, FinalizedTaxProjection):
        # The phase-3 adapter's message, which the comparison checks.
        raise TypeError(
            "GenericRP2TaxEngine requires a FinalizedTaxProjection; raw rows are not a tax-engine input"
        )
    return projection


def classify(profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> tuple[str, str]:
    """The capability a ``build_ledger_state`` call counts under, and its route.

    A call that does not reach routing counts as ``none``: under its
    country's base capability for a projection without rows, and as
    ``unclassified`` for raw rows or a book the builder refuses.
    """
    try:
        projection = _finalized_projection(inputs)
    except TypeError:
        return UNCLASSIFIED, NOT_ROUTED
    if not projection.rows:
        try:
            country = normalize_tax_country(profile_value(profile, "tax_country"))
        except ValueError:
            return UNCLASSIFIED, NOT_ROUTED
        return ("at" if country == "at" else "generic"), NOT_ROUTED
    try:
        request = build_request(profile, inputs)
    except Exception:  # noqa: BLE001 - the call raises the same before routing
        return UNCLASSIFIED, NOT_ROUTED
    book_capabilities = capabilities(request)
    return primary_capability(book_capabilities), route(book_capabilities)


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
        if route(capabilities(request)) == LEGACY:
            return self._legacy(inputs)
        return self._v2(request, inputs)

    def _legacy(self, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        from .rp2 import GenericRP2TaxEngine

        return GenericRP2TaxEngine(self.profile).build_ledger_state(inputs)

    def _v2(self, request: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult:
        response = loads(kassiber_tax.ledger(dumps(request)))
        return read_ledger(self.profile, inputs, response)


def manifest() -> dict[str, Any]:
    """The engine's names and tables (the ``manifest`` operation)."""
    response = loads(kassiber_tax.ledger(dumps({"schema_version": 2, "operation": "manifest"})))
    raise_for_error(response)
    return {key: value for key, value in response.items() if key not in ("schema_version", "ok")}


__all__ = [
    "CAPABILITIES",
    "GENERIC_BITCOIN_RAIL_QUARANTINE_REASON",
    "KassiberTaxEngine",
    "LEGACY",
    "NOT_ROUTED",
    "ROUTED_CAPABILITIES",
    "UNCLASSIFIED",
    "V2",
    "capabilities",
    "classify",
    "manifest",
    "primary_capability",
    "route",
]

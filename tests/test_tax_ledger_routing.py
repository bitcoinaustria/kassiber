"""Capability routing of the production tax engine (plan 20 phase 4, design §5.3).

``KassiberTaxEngine`` builds the engine request for every non-empty book and
sends the book to the Rust engine only when the engine implements every
capability the book uses (``ledger.ROUTED_CAPABILITIES``); every other book
runs on the phase-3 adapter. Here:

- the classifier gives every golden case and seeded random book the
  capabilities the adapter's own conditions imply, read from the inputs
  (an Austrian book, a generic one, a generic carrying-value Bitcoin rail
  pair, an Austrian carrying-value cross-asset pair, each with both legs
  projected), and the golden corpus uses every capability; a carrying pair
  with either leg outside the projection adds none;
- each case routes to v2 exactly when its capabilities are routed: with
  nothing routed (today) every book runs on the adapter and gives the
  stored ``expected``; with the case's capabilities routed it reaches the
  engine, which does not implement the ledger operation yet;
- calls that end before routing count as such.
"""

from __future__ import annotations

import json
from functools import lru_cache
from pathlib import Path
from typing import Any
from unittest.mock import patch

import pytest

from kassiber.core.engines import ledger
from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines.errors import EngineContractError
from kassiber.transfers import is_bitcoin_rail_pair
from tests import tax_engine_compare as compare
from tests import tax_ledger_codec as codec
from tests.tax_ledger_generator import WALLET_REFS, direct, profile, projected, random_inputs

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tax-engine" / "core" / "tests" / "data" / "ledger_fixtures.json"
RANDOM_SEEDS = 50


@lru_cache(maxsize=1)
def _document() -> dict[str, Any]:
    return json.loads(FIXTURES.read_text(encoding="utf-8"))


def _routed_cases() -> list[dict[str, Any]]:
    return [case for case in _document()["cases"] if case["request"] is not None]


def _expected_capabilities(book_profile: Any, inputs: TaxEngineLedgerInputs) -> frozenset[str]:
    """The capabilities by the adapter's conditions (``engines/rp2.py``).

    The generic carry runs for a carrying-value pair of Bitcoin rail assets
    whose legs are both projected rows; Austrian link selection considers a
    carrying-value pair whose legs are both projected rows.
    """
    projection = inputs.finalized_tax_projection
    row_ids = {str(row["id"]) for row in projection.rows}
    carried = [
        pair
        for pair in projection.cross_asset_pairs
        if pair.get("policy") == "carrying-value" and str(pair.get("out_id")) in row_ids and str(pair.get("in_id")) in row_ids
    ]
    if str(book_profile.get("tax_country") or "").strip().lower() == "at":
        return frozenset({"at", "at_swap"} if carried else {"at"})
    if any(is_bitcoin_rail_pair(pair.get("out_asset"), pair.get("in_asset")) for pair in carried):
        return frozenset({"generic", "generic_carry"})
    return frozenset({"generic"})


def test_the_classifier_follows_the_adapters_conditions() -> None:
    used = set()
    for case in _routed_cases():
        book_profile, inputs = codec.decode_inputs(case["inputs"])
        capabilities = ledger.capabilities(case["request"])
        assert capabilities == _expected_capabilities(book_profile, inputs), case["name"]
        used |= capabilities
    assert used == set(ledger.CAPABILITIES)


@pytest.mark.parametrize("country", ["generic", "at"])
def test_random_books_classify_by_the_same_conditions(country: str) -> None:
    from kassiber.core.engines.ledger_request import build_request

    for seed in range(RANDOM_SEEDS):
        book = random_inputs(seed, country)
        assert ledger.capabilities(build_request(book.profile, book.inputs)) == _expected_capabilities(book.profile, book.inputs), seed


@pytest.mark.parametrize("country", ["generic", "at"])
def test_a_carrying_pair_counts_only_with_both_legs_projected(country: str) -> None:
    from kassiber.core.engines.ledger_request import build_request

    peg_out = projected("peg-out", "C", "outbound", 60_000_000_000, "2024-05-01T00:00:00Z", rate="60000")
    peg_in = projected("peg-in", "L", "inbound", 60_000_000_000, "2024-05-01T01:00:00Z", rate="60000", asset="LBTC")
    swap = {
        "pair_id": "rail-1",
        "out_id": peg_out["id"],
        "in_id": peg_in["id"],
        "out_transaction_id": "peg-out",
        "in_transaction_id": "peg-in",
        "out_asset": "BTC",
        "in_asset": "LBTC",
        "policy": "carrying-value",
    }
    book_profile = profile(country, "fifo" if country == "at" else "FIFO")
    carried = {"at": frozenset({"at", "at_swap"}), "generic": frozenset({"generic", "generic_carry"})}[country]
    base = frozenset({"at"}) if country == "at" else frozenset({"generic"})
    for rows, expected in (([peg_out, peg_in], carried), ([peg_out], base), ([peg_in], base)):
        inputs = direct(rows, cross_asset_pairs=[swap])
        assert _expected_capabilities(book_profile, inputs) == expected, len(rows)
        assert ledger.capabilities(build_request(book_profile, inputs)) == expected, [item["journal_transaction_id"] for item in rows]


def test_the_primary_capability_is_the_most_specific() -> None:
    assert ledger.primary_capability(frozenset({"generic"})) == "generic"
    assert ledger.primary_capability(frozenset({"generic", "generic_carry"})) == "generic_carry"
    assert ledger.primary_capability(frozenset({"at"})) == "at"
    assert ledger.primary_capability(frozenset({"at", "at_swap"})) == "at_swap"
    assert ledger.primary_capability(frozenset()) == ledger.UNCLASSIFIED


def test_nothing_is_routed_yet() -> None:
    assert ledger.ROUTED_CAPABILITIES == frozenset()
    assert compare.ROUTED_CAPABILITIES is ledger.ROUTED_CAPABILITIES


@pytest.mark.parametrize("name", [case["name"] for case in _routed_cases()])
def test_each_case_routes_by_its_capabilities(name: str) -> None:
    case = next(case for case in _routed_cases() if case["name"] == name)
    book_profile, inputs = codec.decode_inputs(case["inputs"])
    capabilities = ledger.capabilities(case["request"])
    primary = ledger.primary_capability(capabilities)
    # Nothing routed: the adapter runs and gives the stored outcome.
    assert ledger.classify(book_profile, inputs) == (primary, ledger.LEGACY)
    actual = compare.outcome_document(compare.run_production(book_profile, inputs))
    assert compare.document_differences(codec.decode(case["expected"]), actual) == []
    # Everything the book uses routed: the book reaches the engine. A forced
    # route is not production's behavior, so it runs on the unpatched class,
    # outside a shadow session's comparison.
    with patch.object(ledger, "ROUTED_CAPABILITIES", capabilities):
        assert ledger.classify(book_profile, inputs) == (primary, ledger.V2)
        _result, error = compare.run_production(book_profile, inputs)
        assert isinstance(error, EngineContractError)
        assert str(error) == "Unsupported: the ledger operation is not implemented yet"
    # One capability missing: the adapter again.
    for missing in capabilities:
        with patch.object(ledger, "ROUTED_CAPABILITIES", capabilities - {missing}):
            assert ledger.classify(book_profile, inputs) == (primary, ledger.LEGACY), missing


def test_calls_that_end_before_routing() -> None:
    book_profile = profile("generic", "FIFO")
    empty = direct([])
    assert ledger.classify(book_profile, empty) == ("generic", ledger.NOT_ROUTED)
    assert ledger.classify(profile("at", "fifo"), empty) == ("at", ledger.NOT_ROUTED)
    assert ledger.classify(profile("xx", "fifo"), empty) == (ledger.UNCLASSIFIED, ledger.NOT_ROUTED)
    raw = TaxEngineLedgerInputs(finalized_tax_projection=[], wallet_refs_by_id=WALLET_REFS)  # type: ignore[arg-type]
    assert ledger.classify(book_profile, raw) == (ledger.UNCLASSIFIED, ledger.NOT_ROUTED)
    refused = projected("buy", "C", "inbound", 1_000, "2024-01-01T00:00:00Z")
    refused["fee"] = -1
    assert ledger.classify(book_profile, direct([refused])) == (ledger.UNCLASSIFIED, ledger.NOT_ROUTED)
    labelled = {key: dict(value) for key, value in WALLET_REFS.items()}
    labelled["C"]["label"] = "Cold, Savings"
    buy = projected("buy", "C", "inbound", 1_000, "2024-01-01T00:00:00Z")
    assert ledger.classify(book_profile, direct([buy], wallet_refs=labelled)) == (ledger.UNCLASSIFIED, ledger.NOT_ROUTED)

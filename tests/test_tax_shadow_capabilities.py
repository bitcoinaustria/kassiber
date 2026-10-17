"""The capabilities the tax-shadow summary counts calls by (plan 20 phase 4, design §5.3).

Every non-empty book runs on the Rust engine. A shadow session counts each
production call under the most specific engine capability its book
exercises (``tests/tax_engine_compare.py``: ``classify``), so a lane whose
books never reach a capability shows it. Here:

- the classifier gives every golden case and seeded random book the
  capabilities the oracle adapter's own conditions imply, read from the
  inputs (an Austrian book, a generic one, a generic carrying-value Bitcoin
  rail pair, an Austrian carrying-value cross-asset pair, each with both
  legs projected), and the golden corpus uses every capability; a carrying
  pair with either leg outside the projection adds none;
- every golden case with a request reaches the engine under its primary
  capability;
- calls that end before the engine count as such.
"""

from __future__ import annotations

import json
from functools import lru_cache
from pathlib import Path
from typing import Any

import pytest

from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines.ledger_request import build_request
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


def _requested_cases() -> list[dict[str, Any]]:
    return [case for case in _document()["cases"] if case["request"] is not None]


def _expected_capabilities(book_profile: Any, inputs: TaxEngineLedgerInputs) -> frozenset[str]:
    """The capabilities by the oracle adapter's conditions (``tests/tax_oracle/adapter.py``).

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
    for case in _requested_cases():
        book_profile, inputs = codec.decode_inputs(case["inputs"])
        capabilities = compare.capabilities(case["request"])
        assert capabilities == _expected_capabilities(book_profile, inputs), case["name"]
        used |= capabilities
    assert used == set(compare.CAPABILITIES)


@pytest.mark.parametrize("country", ["generic", "at"])
def test_random_books_classify_by_the_same_conditions(country: str) -> None:
    for seed in range(RANDOM_SEEDS):
        book = random_inputs(seed, country)
        assert compare.capabilities(build_request(book.profile, book.inputs)) == _expected_capabilities(book.profile, book.inputs), seed


@pytest.mark.parametrize("country", ["generic", "at"])
def test_a_carrying_pair_counts_only_with_both_legs_projected(country: str) -> None:
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
        assert compare.capabilities(build_request(book_profile, inputs)) == expected, [item["journal_transaction_id"] for item in rows]


def test_the_primary_capability_is_the_most_specific() -> None:
    assert compare.primary_capability(frozenset({"generic"})) == "generic"
    assert compare.primary_capability(frozenset({"generic", "generic_carry"})) == "generic_carry"
    assert compare.primary_capability(frozenset({"at"})) == "at"
    assert compare.primary_capability(frozenset({"at", "at_swap"})) == "at_swap"
    assert compare.primary_capability(frozenset()) == compare.UNCLASSIFIED


def test_every_requested_case_reaches_the_engine_under_its_capability() -> None:
    reached = set()
    for case in _requested_cases():
        book_profile, inputs = codec.decode_inputs(case["inputs"])
        primary = compare.primary_capability(compare.capabilities(case["request"]))
        assert compare.classify(book_profile, inputs) == (primary, compare.V2), case["name"]
        reached.add(primary)
    # Austrian books with swap links or without, generic ones with the rail
    # carry or without.
    assert reached == set(compare.CAPABILITIES)


def test_calls_that_end_before_the_engine() -> None:
    book_profile = profile("generic", "FIFO")
    empty = direct([])
    assert compare.classify(book_profile, empty) == ("generic", compare.NOT_ROUTED)
    assert compare.classify(profile("at", "fifo"), empty) == ("at", compare.NOT_ROUTED)
    assert compare.classify(profile("xx", "fifo"), empty) == (compare.UNCLASSIFIED, compare.NOT_ROUTED)
    raw = TaxEngineLedgerInputs(finalized_tax_projection=[], wallet_refs_by_id=WALLET_REFS)  # type: ignore[arg-type]
    assert compare.classify(book_profile, raw) == (compare.UNCLASSIFIED, compare.NOT_ROUTED)
    refused = projected("buy", "C", "inbound", 1_000, "2024-01-01T00:00:00Z")
    refused["fee"] = -1
    assert compare.classify(book_profile, direct([refused])) == (compare.UNCLASSIFIED, compare.NOT_ROUTED)
    labelled = {key: dict(value) for key, value in WALLET_REFS.items()}
    labelled["C"]["label"] = "Cold, Savings"
    buy = projected("buy", "C", "inbound", 1_000, "2024-01-01T00:00:00Z")
    assert compare.classify(book_profile, direct([buy], wallet_refs=labelled)) == (compare.UNCLASSIFIED, compare.NOT_ROUTED)

"""The engine's normalized stage against the oracle's (plan 20 phase 4, design §3.10).

The engine's ``prepare`` operation stops after the final prepare pass and
returns, per asset, what the adapter's normalization produced: the events
and transfers the walk books, their order, normalize's quarantines, and the
time from which the asset's lot state is tainted. Here each of those is
compared, field by field and exactly (types, decimal exponents, key and list
order), with the frozen oracle's ``normalize_tax_asset_inputs`` as the
adapter's ``_prepare_assets`` calls it, on the legacy-native backend:

- every generic golden case without the generic rail carry;
- the first 200 seeded generic books of ``tests/tax_ledger_generator.py``
  without a carrying-value cross-asset pair;
- shapes neither reaches: stored rates and values that are not finite (each
  read as CPython's ``Decimal`` reads it, so a NaN raises where it is
  compared), a product that overflows, a subnormal price, missing wallet
  labels, an empty timestamp, two blocked groups whose legs share
  transactions, a group blocked for two reasons, a failing ungrouped
  move.

The engine names rows by id; the fields the reader takes from the rows and
pairs (times, wallet ids, external ids, pairing sources) are filled in from
the projection here, and its quarantines are rendered by the production
reader (``ledger_response.render_quarantine``), so the ``detail_kind``
enrichment is compared too. A failure is compared by its signature, raised
through the production stage table (``errors.ledger_error``).

A last check runs the whole oracle on the legacy-native backend and records
each normalization it performs, so the mirror of the adapter's call used
here cannot drift from the adapter's own. Austrian books and the generic
rail carry are not ported yet; the engine answers ``Unsupported`` for them
(``tax-engine/core/tests/ledger_fixtures.rs``).
"""

from __future__ import annotations

import dataclasses
import json
from collections import defaultdict
from decimal import Decimal
from functools import lru_cache
from pathlib import Path
from typing import Any, Mapping, Optional
from unittest.mock import patch

import kassiber_tax
import pytest

from kassiber.core import pricing
from kassiber.core.engines import ledger, ledger_request, ledger_response
from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines.errors import ledger_error
from tests import tax_engine_compare as compare
from tests import tax_ledger_codec as codec
from tests.tax_ledger_generator import WALLET_REFS, direct, move, profile, projected, random_inputs
from tests.tax_oracle import adapter as oracle_adapter

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tax-engine" / "core" / "tests" / "data" / "ledger_fixtures.json"
RANDOM_BOOKS = 200
CARRYING_VALUE = "carrying-value"
GENERIC = frozenset({"generic"})

Document = dict[str, Any]


@lru_cache(maxsize=1)
def _document() -> dict[str, Any]:
    return json.loads(FIXTURES.read_text(encoding="utf-8"))


def _generic_case_names() -> list[str]:
    """The golden cases whose books are generic and run no rail carry."""
    return [
        case["name"]
        for case in _document()["cases"]
        if case["request"] is not None and ledger.capabilities(case["request"]) == GENERIC
    ]


def _golden(name: str) -> tuple[Mapping[str, Any], TaxEngineLedgerInputs]:
    case = next(case for case in _document()["cases"] if case["name"] == name)
    return codec.decode_inputs(case["inputs"])


@lru_cache(maxsize=1)
def _random_seeds() -> tuple[int, ...]:
    """The first ``RANDOM_BOOKS`` seeds whose generic book has no carrying-value cross-asset pair."""
    seeds: list[int] = []
    seed = 0
    while len(seeds) < RANDOM_BOOKS:
        book = random_inputs(seed, "generic")
        pairs = book.inputs.finalized_tax_projection.cross_asset_pairs
        if not any(pair.get("policy") == CARRYING_VALUE for pair in pairs):
            seeds.append(seed)
        seed += 1
    return tuple(seeds)


# ---------------------------------------------------------------------------
# The oracle's normalized stage


def _final_pass(
    book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs
) -> tuple[dict[str, list[Mapping[str, Any]]], dict[str, list[Mapping[str, Any]]], set[str]]:
    """The arguments of the adapter's final prepare pass for a generic book
    without the rail carry: rows and moves grouped as
    ``_build_finalized_ledger_state`` groups them
    (``tests/tax_oracle/adapter.py:2209-2244``), after the carry, which
    returns the rows as they are and blocks none.
    """
    adapter = oracle_adapter
    projection = inputs.finalized_tax_projection
    pairs_by_asset: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    for pair in projection.intra_pairs:
        pairs_by_asset[str(adapter._row_get(pair["out"], "asset"))].append(pair)
    generic_rail = adapter._apply_generic_bitcoin_rail_carry_values(
        book_profile,
        list(projection.rows),
        projection.cross_asset_pairs,
        pairs_by_asset,
        None,
        inputs.wallet_refs_by_id,
        excluded_row_ids=set(),
        loan_leg_by_transaction_id={},
    )
    assert generic_rail.blocked_row_ids == set() and generic_rail.quarantines == []
    assert len(generic_rail.rows) == len(projection.rows)
    assert all(left is right for left, right in zip(generic_rail.rows, projection.rows))
    rows_by_asset: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    for row in generic_rail.rows:
        rows_by_asset[str(adapter._row_get(row, "asset"))].append(row)
    return rows_by_asset, pairs_by_asset, generic_rail.blocked_row_ids


def _oracle_prepare_assets(
    book_profile: Mapping[str, Any],
    inputs: TaxEngineLedgerInputs,
    final_pass: Optional[tuple[Any, Any, set[str]]] = None,
) -> list[Any]:
    """The adapter's ``_prepare_assets`` over the final pass, as the adapter
    calls it, with the walk stubbed out: one ``NormalizedTaxAssetInputs``
    per asset."""
    rows_by_asset, pairs_by_asset, excluded = final_pass or _final_pass(book_profile, inputs)
    with patch.object(oracle_adapter, "_prepare_rp2_asset_input", lambda *args: None):
        prepared = oracle_adapter._prepare_assets(
            book_profile,
            rows_by_asset,
            inputs.wallet_refs_by_id,
            pairs_by_asset,
            None,
            excluded_row_ids=excluded,
            loan_leg_by_transaction_id={},
        )
    return [normalized for normalized, _ in prepared]


def _fields(item: Any) -> Document:
    return {field.name: getattr(item, field.name) for field in dataclasses.fields(item)}


def _row_ref(position: Mapping[int, int], row: Mapping[str, Any]) -> tuple[str, Any]:
    # Which projection row an event or move leg is: the row object itself,
    # not a copy.
    index = position.get(id(row))
    return ("row", index) if index is not None else ("not a projection row", row.get("id"))


def _oracle_asset(projection: Any, normalized: Any) -> Document:
    position = {id(row): index for index, row in enumerate(projection.rows)}
    events = []
    for event in normalized.events:
        document = _fields(event)
        document["raw_row"] = _row_ref(position, event.raw_row)
        events.append(document)
    transfers = []
    for transfer in normalized.transfers:
        document = _fields(transfer)
        document["out_row"] = _row_ref(position, transfer.out_row)
        document["in_row"] = _row_ref(position, transfer.in_row)
        transfers.append(document)
    return {
        "asset": normalized.asset,
        "events": events,
        "transfers": transfers,
        "ordered_items": list(normalized.ordered_items),
        "quarantines": list(normalized.quarantines),
        "earliest_lot_contamination_at": normalized.earliest_lot_contamination_at,
    }


def _oracle_document(book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> Document:
    projection = inputs.finalized_tax_projection
    # Outside the captured outcome, so a book that runs the carry fails here.
    final_pass = _final_pass(book_profile, inputs)
    normalized, error = compare.run_on_oracle_backend(
        compare.LEGACY_NATIVE_BACKEND, lambda: _oracle_prepare_assets(book_profile, inputs, final_pass)
    )
    if error is not None:
        return {"error": compare.error_signature(error)}
    return {"assets": [_oracle_asset(projection, item) for item in normalized]}


# ---------------------------------------------------------------------------
# The engine's normalized stage


def _decimal(text: Optional[str]) -> Optional[Decimal]:
    if text is None:
        return None
    # A price or value the row stored as ``Infinity`` stays one.
    return Decimal(text) if text == "Infinity" else ledger_request.decode_decimal(text)


def _engine_asset(book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs, asset: Mapping[str, Any]) -> Document:
    projection = inputs.finalized_tax_projection
    rows_by_id = {str(row["id"]): row for row in projection.rows}
    position = {str(row["id"]): index for index, row in enumerate(projection.rows)}
    wallets = inputs.wallet_refs_by_id
    normalized = asset["normalized"]
    events = []
    for event in normalized["events"]:
        row = rows_by_id[event["row"]]
        events.append(
            {
                "transaction_id": row["id"],
                "asset": asset["asset"],
                "occurred_at": row["occurred_at"],
                "wallet_id": wallets[row["wallet_id"]]["id"],
                "wallet_label": event["wallet_label"],
                "direction": row["direction"],
                "amount": _decimal(event["amount"]),
                "fee": _decimal(event["fee"]),
                "spot_price": _decimal(event["spot_price"]),
                "fiat_value": _decimal(event["fiat_value"]),
                "description": event["description"],
                "raw_row": ("row", position[event["row"]]),
                "at_regime": event["at_regime"],
                "at_regime_basis": event["at_regime_basis"],
                "cost_basis_pool_id": event["cost_basis_pool_id"],
                "at_swap_link": event["at_swap_link"],
                # The adapter passes no loan legs (B map §1.8).
                "loan_leg_role": None,
            }
        )
    transfers = []
    for transfer in normalized["transfers"]:
        pair = projection.intra_pairs[transfer["pair"]]
        out_row, in_row = pair["out"], pair["in"]
        # The engine names the pair and its legs consistently.
        assert (str(out_row["id"]), str(in_row["id"])) == (transfer["out"], transfer["in"]), transfer
        transfers.append(
            {
                "asset": asset["asset"],
                "occurred_at": out_row["occurred_at"],
                "out_transaction_id": out_row["id"],
                "in_transaction_id": in_row["id"],
                "from_wallet_id": wallets[out_row["wallet_id"]]["id"],
                "from_wallet_label": transfer["from_wallet_label"],
                "to_wallet_id": wallets[in_row["wallet_id"]]["id"],
                "to_wallet_label": transfer["to_wallet_label"],
                "sent": _decimal(transfer["sent"]),
                "received": _decimal(transfer["received"]),
                "fee": _decimal(transfer["fee"]),
                "spot_price": _decimal(transfer["spot_price"]),
                "description": transfer["description"],
                "external_id": out_row["external_id"],
                "out_row": ("row", position[transfer["out"]]),
                "in_row": ("row", position[transfer["in"]]),
                "from_cost_basis_pool_id": transfer["from_cost_basis_pool_id"],
                "to_cost_basis_pool_id": transfer["to_cost_basis_pool_id"],
                "at_regime": transfer["at_regime"],
                "regime_flows": transfer["regime_flows"],
                # No live branch sets a transfer id or blocks rows outside
                # the pair (B map §3.5).
                "transfer_id": None,
                "group_id": transfer["group_id"],
                "group_block_rows": (),
                "pairing_source": pair.get("source"),
            }
        )
    return {
        "asset": asset["asset"],
        "events": events,
        "transfers": transfers,
        "ordered_items": [tuple(item) for item in normalized["ordered_items"]],
        "quarantines": [
            ledger_response.render_quarantine(book_profile, rows_by_id, item) for item in normalized["quarantines"]
        ],
        "earliest_lot_contamination_at": normalized["earliest_lot_contamination_at"],
    }


def _engine_response(book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> dict[str, Any]:
    request = ledger_request.build_request(book_profile, inputs, operation="prepare")
    assert ledger.capabilities(request) == GENERIC
    return ledger_response.loads(kassiber_tax.ledger(ledger_request.dumps(request)))


def _engine_document(book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> Document:
    response = _engine_response(book_profile, inputs)
    if response["ok"] is not True:
        return {"error": compare.error_signature(ledger_error(response["error"]))}
    assert response["engine_version"] == kassiber_tax.engine_version()
    return {"assets": [_engine_asset(book_profile, inputs, asset) for asset in response["assets"]]}


def _assert_same_normalized_stage(book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> Document:
    expected = _oracle_document(book_profile, inputs)
    actual = _engine_document(book_profile, inputs)
    assert compare.value_differences(expected, actual, "normalized") == []
    return expected


# ---------------------------------------------------------------------------
# The comparisons


def test_the_corpus_reaches_the_normalize_branches() -> None:
    """The golden and random books compared below reach booked moves,
    tainted lot states, and every quarantine normalize raises on a generic
    book but its group blocking: the golden bundle fails later, in the
    walk's group gate, so the edge books block groups here."""
    reasons: dict[str, set[str]] = {"golden": set(), "random": set()}
    transfers = contaminated = 0
    books = [("golden", *_golden(name)) for name in _generic_case_names()]
    for seed in _random_seeds():
        book = random_inputs(seed, "generic")
        books.append(("random", book.profile, book.inputs))
    for source, book_profile, inputs in books:
        document = _oracle_document(book_profile, inputs)
        assert "assets" in document, (source, document)
        for asset in document["assets"]:
            reasons[source] |= {item["reason"] for item in asset["quarantines"]}
            transfers += len(asset["transfers"])
            contaminated += asset["earliest_lot_contamination_at"] is not None
    assert len(_generic_case_names()) >= 35
    assert reasons["golden"] | reasons["random"] == {
        "missing_spot_price",
        "pricing_review_required",
        "privacy_hop_unresolved",
        "conflicting_spend",
        "pending_onchain_confirmation",
    }, reasons
    assert transfers > 0 and contaminated > 0


@pytest.mark.parametrize("name", _generic_case_names())
def test_the_golden_books_normalize_as_the_oracle(name: str) -> None:
    _assert_same_normalized_stage(*_golden(name))


def test_the_random_books_normalize_as_the_oracle() -> None:
    differing: list[str] = []
    failed = 0
    for seed in _random_seeds():
        book = random_inputs(seed, "generic")
        expected = _oracle_document(book.profile, book.inputs)
        failed += "error" in expected
        differences = compare.value_differences(expected, _engine_document(book.profile, book.inputs), "normalized")
        if differences:
            differing.append(f"seed {seed}: {differences[:3]}")
    assert differing == []
    assert failed == 0


EDGE_BOOKS = {
    # CPython raises InvalidOperation where it compares a NaN with zero.
    "rate-nan": lambda: [projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="NaN")],
    "rate-signaling-nan": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate=None, fiat_rate_exact="sNaN")
    ],
    "value-nan": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate=None, fiat_value="NaN")
    ],
    # An event reads its stored value for its fiat value even when its rate
    # prices it.
    "rate-positive-value-nan": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="40000", fiat_value="NaN")
    ],
    # Infinity is a price; the value is read after the product.
    "rate-infinity": lambda: [projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="Infinity")],
    "rate-infinity-value": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="Infinity", fiat_value="5")
    ],
    "rate-infinity-value-nan": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="Infinity", fiat_value="NaN")
    ],
    "rate-negative-infinity-value": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="-Infinity", fiat_value="1000")
    ],
    "value-infinity": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate=None, fiat_value="Infinity")
    ],
    # A fee-only spend prices its fee, and 0 * Infinity is invalid.
    "fee-only-rate-infinity": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z"),
        projected("fee", "A", "outbound", 0, "2024-01-02T00:00:00Z", fee=1_000_000, rate="Infinity"),
    ],
    "fee-only-value": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z"),
        projected("fee", "A", "outbound", 0, "2024-01-02T00:00:00Z", fee=1_000_000, rate=None, fiat_value="3"),
    ],
    "product-overflows": lambda: [
        projected("buy", "A", "inbound", 200_000_000_000, "2024-01-01T00:00:00Z", rate="9E+999999")
    ],
    "price-subnormal": lambda: [
        projected("buy", "A", "inbound", 300_000_000_000, "2024-01-01T00:00:00Z", rate=None, fiat_value="1E-999999")
    ],
    "rate-zero-value-negative": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="0", fiat_value="-5")
    ],
}


def _without_text(row: dict[str, Any]) -> dict[str, Any]:
    # ``row()`` defaults an empty kind, so clear it afterwards.
    return {**row, "note": None, "description": None, "kind": ""}


def _in_wallet(row: dict[str, Any], wallet_id: str) -> dict[str, Any]:
    return {**row, "wallet_id": wallet_id}


def _edge_moves() -> dict[str, tuple[list[dict[str, Any]], list[dict[str, Any]], Mapping[str, Any]]]:
    """Books with moves, for a profile that requires coarse prices to be
    reviewed: each name says the shape."""

    def buy(tx: str, wallet: str, at: str = "2024-01-01T00:00:00Z") -> dict[str, Any]:
        return projected(tx, wallet, "inbound", 50_000_000_000, at)

    def legs(tx: str, source: str, target: str, at: str, *, amount: int = 10_000_000_000, fee: int = 1_000_000, anchor: Optional[str] = None, **fields: Any):
        anchor = anchor or tx
        out = projected(f"{tx}-out", source, "outbound", amount, at, fee=fee, anchor=anchor, side="out", **fields)
        inbound = projected(f"{tx}-in", target, "inbound", amount, at, anchor=anchor, side="in", **fields)
        return out, inbound

    books: dict[str, tuple[list[dict[str, Any]], list[dict[str, Any]], Mapping[str, Any]]] = {}

    # The out leg's NaN rate raises before the in leg is read.
    out, inbound = legs("m", "A", "B", "2024-02-01T00:00:00Z", rate="NaN")
    books["move-rate-nan"] = ([buy("buy", "A"), out, inbound], [move(out, inbound)], WALLET_REFS)

    # A move reads a stored value only for a leg whose rate does not price
    # it, so a NaN value behind a positive rate is never compared.
    out, inbound = legs("m", "A", "B", "2024-02-01T00:00:00Z", fiat_value="NaN")
    books["move-rate-positive-value-nan"] = ([buy("buy", "A"), out, inbound], [move(out, inbound)], WALLET_REFS)

    # An out leg at -Infinity has no price, so the in leg's prices the move.
    out = projected("m-out", "A", "outbound", 10_000_000_000, "2024-02-01T00:00:00Z", fee=1_000_000, anchor="m", side="out", rate="-Infinity")
    inbound = projected("m-in", "B", "inbound", 10_000_000_000, "2024-02-01T00:00:00Z", anchor="m", side="in", rate="41000")
    books["move-priced-by-in-leg"] = ([buy("buy", "A"), out, inbound], [move(out, inbound)], WALLET_REFS)

    # An unpriced, fee-bearing move without a group: the move and its
    # partner leg are quarantined, without a group.
    out, inbound = legs("m", "A", "B", "2024-02-01T00:00:00Z", rate=None)
    sell = projected("sell", "B", "outbound", 1_000_000_000, "2024-03-01T00:00:00Z")
    books["ungrouped-unpriced-move"] = ([buy("buy", "A"), out, inbound, sell], [move(out, inbound, group_id=None)], WALLET_REFS)

    # Two blocked groups. The second group's priced move leaves from a slice
    # of the transaction the first group's block already flagged, so only
    # its destination's transaction is flagged for the second group.
    g1_fail_out, g1_fail_in = legs("g1-fail", "A", "B", "2024-02-01T00:00:00Z", rate=None)
    g1_pass_out, g1_pass_in = legs("g1-pass", "A", "C", "2024-02-01T00:00:00Z")
    g2_fail_out, g2_fail_in = legs("g2-fail", "B", "C", "2024-02-02T00:00:00Z", rate=None)
    g2_out = projected("g2-out", "A", "outbound", 20_000_000_000, "2024-02-02T00:00:00Z", anchor="g1-pass", side="out2")
    g2_in = projected("g2-in", "C", "inbound", 20_000_000_000, "2024-02-02T00:00:00Z", anchor="g2-new", side="in2")
    books["two-blocked-groups-sharing-transactions"] = (
        [buy("buy", "A"), buy("buy-b", "B"), g1_fail_out, g1_fail_in, g1_pass_out, g1_pass_in, g2_fail_out, g2_fail_in, g2_out, g2_in],
        [
            move(g1_fail_out, g1_fail_in, group_id="g1"),
            move(g1_pass_out, g1_pass_in, group_id="g1"),
            move(g2_fail_out, g2_fail_in, group_id="g2"),
            move(g2_out, g2_in, group_id="g2"),
        ],
        WALLET_REFS,
    )

    # A group whose first failing move needs coarse review and whose second
    # is unpriced: the group keeps the first reason, so its priced sibling
    # is flagged as blocked by the review.
    coarse = {"pricing_quality": pricing.QUALITY_COARSE_FALLBACK, "pricing_granularity": "daily", "pricing_provider": "kraken"}
    coarse_out, coarse_in = legs("coarse", "A", "B", "2024-02-01T00:00:00Z", **coarse)
    unpriced_out, unpriced_in = legs("unpriced", "A", "B", "2024-02-02T00:00:00Z", rate=None)
    sibling_out, sibling_in = legs("sibling", "A", "C", "2024-02-03T00:00:00Z")
    books["blocked-group-keeps-its-first-reason"] = (
        [buy("buy", "A"), coarse_out, coarse_in, unpriced_out, unpriced_in, sibling_out, sibling_in],
        [
            move(coarse_out, coarse_in, group_id="g"),
            move(unpriced_out, unpriced_in, group_id="g"),
            move(sibling_out, sibling_in, group_id="g"),
        ],
        WALLET_REFS,
    )

    # A wallet without a label: details carry ``None``, a move without text
    # is ``Transfer None -> Hot``, and a group-block flag names the wallet id.
    unlabelled = {**WALLET_REFS, "U": {**WALLET_REFS["A"], "id": "U", "label": None}}
    named_out, named_in = legs("u", "A", "B", "2024-02-01T00:00:00Z")
    unpriced_out, unpriced_in = legs("v", "A", "B", "2024-02-02T00:00:00Z", rate=None)
    sibling_out, sibling_in = legs("w", "A", "B", "2024-02-02T00:00:00Z", fee=0)
    named_out = _in_wallet(_without_text(named_out), "U")
    unpriced_out = _in_wallet(unpriced_out, "U")
    sibling_out = _in_wallet(sibling_out, "U")
    books["unlabelled-wallet"] = (
        [
            _in_wallet(buy("buy", "A"), "U"),
            named_out,
            named_in,
            unpriced_out,
            unpriced_in,
            sibling_out,
            sibling_in,
            _in_wallet(projected("unpriced", "A", "inbound", 1_000, "2024-03-01T00:00:00Z", rate=None), "U"),
        ],
        [move(named_out, named_in), move(unpriced_out, unpriced_in, group_id="vw"), move(sibling_out, sibling_in, group_id="vw")],
        unlabelled,
    )

    # A quarantined row without a time does not taint the lot state; the
    # next quarantined one does.
    books["empty-timestamp"] = (
        [
            projected("untimed", "A", "inbound", 1_000, "", rate=None),
            buy("buy", "A"),
            projected("unpriced", "A", "inbound", 1_000, "2024-02-01T00:00:00Z", rate=None),
        ],
        [],
        WALLET_REFS,
    )

    # Text: an event without note, description, or kind is named by its row
    # id; an empty note gives way to the description; a move takes its out
    # leg's note.
    out, inbound = legs("t", "A", "B", "2024-02-01T00:00:00Z", note="cold storage top-up")
    books["text-fallbacks"] = (
        [
            _without_text(buy("bare", "A")),
            out,
            inbound,
            projected("described", "B", "outbound", 1_000_000_000, "2024-03-01T00:00:00Z", note="", description="paid rent"),
        ],
        [move(out, inbound)],
        WALLET_REFS,
    )
    return books


EDGE_MOVES = sorted(_edge_moves())


@pytest.mark.parametrize("name", sorted(EDGE_BOOKS))
def test_edge_rows_normalize_as_the_oracle(name: str) -> None:
    _assert_same_normalized_stage(profile("generic", "FIFO"), direct(EDGE_BOOKS[name]()))


@pytest.mark.parametrize("name", EDGE_MOVES)
def test_edge_moves_normalize_as_the_oracle(name: str) -> None:
    rows, pairs, wallets = _edge_moves()[name]
    _assert_same_normalized_stage(profile("generic", "FIFO", require_coarse_review=True), direct(rows, pairs, wallet_refs=wallets))


def _quarantines(asset: Document, reason: str) -> list[tuple[str, dict[str, Any]]]:
    return [(item["transaction_id"], json.loads(item["detail_json"])) for item in asset["quarantines"] if item["reason"] == reason]


def test_the_edge_books_reach_their_shapes() -> None:
    """Each edge book shows on the oracle what its name says."""
    generic = profile("generic", "FIFO")
    rows = {name: _oracle_document(generic, direct(build())) for name, build in EDGE_BOOKS.items()}
    invalid = ("InvalidOperation", "[<class 'decimal.InvalidOperation'>]")
    assert {name for name, document in rows.items() if document.get("error") == invalid} == {
        "rate-nan",
        "rate-signaling-nan",
        "value-nan",
        "rate-positive-value-nan",
        "rate-infinity-value-nan",
        "fee-only-rate-infinity",
    }
    assert rows["product-overflows"] == {"error": ("Overflow", "[<class 'decimal.Overflow'>]")}

    def event(name: str, index: int = 0) -> Document:
        return rows[name]["assets"][0]["events"][index]

    infinity = Decimal("Infinity")
    assert (event("rate-infinity")["spot_price"], event("rate-infinity")["fiat_value"]) == (infinity, infinity)
    assert (event("rate-infinity-value")["spot_price"], event("rate-infinity-value")["fiat_value"]) == (infinity, Decimal("5"))
    assert (event("value-infinity")["spot_price"], event("value-infinity")["fiat_value"]) == (infinity, infinity)
    assert event("rate-negative-infinity-value")["spot_price"] == Decimal("1000")
    assert (event("fee-only-value", 1)["spot_price"], event("fee-only-value", 1)["fiat_value"]) == (Decimal("3E+5"), Decimal("3"))
    assert event("price-subnormal")["spot_price"].as_tuple().exponent == -1000030
    assert [item["reason"] for item in rows["rate-zero-value-negative"]["assets"][0]["quarantines"]] == ["missing_spot_price"]

    review = profile("generic", "FIFO", require_coarse_review=True)
    moves = {name: _oracle_document(review, direct(book_rows, pairs, wallet_refs=wallets)) for name, (book_rows, pairs, wallets) in _edge_moves().items()}
    assert moves["move-rate-nan"] == {"error": invalid}
    assert moves["move-priced-by-in-leg"]["assets"][0]["transfers"][0]["spot_price"] == Decimal("41000")
    assert moves["move-rate-positive-value-nan"]["assets"][0]["transfers"][0]["spot_price"] == Decimal("40000")

    ungrouped = moves["ungrouped-unpriced-move"]["assets"][0]
    assert [(tx, "transfer_group_id" in detail) for tx, detail in _quarantines(ungrouped, "missing_spot_price")] == [("m", False), ("m", False)]
    assert _quarantines(ungrouped, "derived_transfer_group_blocked") == []

    blocked = moves["two-blocked-groups-sharing-transactions"]["assets"][0]
    assert blocked["transfers"] == []
    assert [(tx, detail["transfer_group_id"], detail["blocked_by_reason"]) for tx, detail in _quarantines(blocked, "derived_transfer_group_blocked")] == [
        ("g1-pass", "g1", "missing_spot_price"),
        ("g2-new", "g2", "missing_spot_price"),
    ]

    first_reason = moves["blocked-group-keeps-its-first-reason"]["assets"][0]
    assert first_reason["transfers"] == []
    assert [(item["transaction_id"], item["reason"]) for item in first_reason["quarantines"]] == [
        ("coarse", "pricing_review_required"),
        ("coarse", "pricing_review_required"),
        ("unpriced", "missing_spot_price"),
        ("unpriced", "missing_spot_price"),
        ("sibling", "derived_transfer_group_blocked"),
    ]
    assert [(tx, detail["blocked_by_reason"]) for tx, detail in _quarantines(first_reason, "derived_transfer_group_blocked")] == [
        ("sibling", "pricing_review_required"),
    ]

    unlabelled = moves["unlabelled-wallet"]["assets"][0]
    assert [transfer["description"] for transfer in unlabelled["transfers"]] == ["Transfer None -> Hot"]
    assert [(tx, detail["from_wallet"]) for tx, detail in _quarantines(unlabelled, "missing_spot_price")[:2]] == [("v", None), ("v", None)]
    assert [(tx, detail["wallet"]) for tx, detail in _quarantines(unlabelled, "derived_transfer_group_blocked")] == [("w", "U")]
    assert _quarantines(unlabelled, "missing_spot_price")[2] == (
        "unpriced",
        {"asset": "BTC", "direction": "inbound", "required_for": "acquisition", "wallet": None},
    )

    untimed = moves["empty-timestamp"]["assets"][0]
    assert [tx for tx, _ in _quarantines(untimed, "missing_spot_price")] == ["untimed", "unpriced"]
    assert untimed["earliest_lot_contamination_at"] == "2024-02-01T00:00:00Z"

    text = moves["text-fallbacks"]["assets"][0]
    assert [item["description"] for item in text["events"]] == [text["events"][0]["transaction_id"], "paid rent"]
    assert [transfer["description"] for transfer in text["transfers"]] == ["cold storage top-up"]


def test_the_mirror_is_the_adapters_own_call() -> None:
    """The whole oracle, on the legacy-native backend, normalizes each asset
    exactly as ``_oracle_prepare_assets`` does, as far as it gets."""
    recorded: list[Any] = []
    normalize = oracle_adapter.normalize_tax_asset_inputs

    def recording(*args: Any, **kwargs: Any) -> Any:
        result = normalize(*args, **kwargs)
        recorded.append(result)
        return result

    books = [_golden(name) for name in _generic_case_names()]
    for seed in _random_seeds():
        book = random_inputs(seed, "generic")
        books.append((book.profile, book.inputs))
    complete = 0
    for book_profile, inputs in books:
        recorded.clear()
        with patch.object(oracle_adapter, "normalize_tax_asset_inputs", recording):
            _, error = compare.run_oracle(book_profile, inputs, backend=compare.LEGACY_NATIVE_BACKEND)
        mirror = _oracle_prepare_assets(book_profile, inputs)
        projection = inputs.finalized_tax_projection
        # One prepare pass: one normalization per asset, until the walk or
        # anything later fails.
        assert len(recorded) <= len(mirror)
        if error is None:
            assert len(recorded) == len(mirror)
            complete += 1
        assert compare.value_differences(
            [_oracle_asset(projection, item) for item in mirror[: len(recorded)]],
            [_oracle_asset(projection, item) for item in recorded],
        ) == []
    assert complete >= 200

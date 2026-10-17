"""The engine's prepare stages against the oracle's (plan 20 phase 4, design §3.10).

The engine's ``prepare`` operation stops after the final prepare pass and
returns, per asset, the two stages the adapter's ``_prepare_assets`` builds:

- the normalized stage: the events and transfers the walk books, their
  order, normalize's quarantines, and the time from which the asset's lot
  state is tainted;
- the prepared stage: the lot engine's entries (v1 entry documents with
  their notes and ``str(transaction)`` text, ``None`` for an asset without
  an inbound entry), the gates' quarantines, and the transfer audit, with
  the transfer legs' description;

the generic Bitcoin rail carry that runs before the first pass: the rows
it prices, the rows its blocked pairs leave out of every later pass, their
quarantines, and the passes it ran (each prepares and computes the whole
book); and the Austrian swap-link selection over the first pass, which
decides whether a second pass runs: the rows it links, the rows its blocked
pairs leave out of the second pass, and their quarantines.

Here each of those is compared, field by field and exactly (types, decimal
exponents, key and list order), with the frozen oracle's intermediates as
its own ``_apply_generic_bitcoin_rail_carry_values``, ``_prepare_assets``,
and ``_select_at_cross_asset_swap_links`` build them on the legacy-native
backend, whose entries are v1 documents already:

- every golden case: a generic one's rail carry, and an Austrian one's
  regime tracker, the regimes, elections, and flows it gives events and
  moves, the overrides, and, with a carrying-value cross-asset pair, the
  selection and the second pass;
- the first 200 seeded generic books and the first 200 seeded Austrian
  books of ``tests/tax_ledger_generator.py`` without a carrying-value
  cross-asset pair, the first 200 seeded Austrian books with one, and the
  first 100 seeded generic books with the rail carry;
- shapes neither reaches: stored rates and values that are not finite (each
  read as CPython's ``Decimal`` reads it, so a NaN raises where it is
  compared and an entry's infinite value where its constructor compares
  it), a product that overflows, a subnormal price, missing wallet labels,
  an empty timestamp, two blocked groups whose legs share transactions, a
  group blocked for two reasons, a failing ungrouped move, a group a spend
  drains between its moves, one instant spelled two ways, a holder label
  with spaces around it; on Austrian books, times the tracker reads
  otherwise than RP2 (without an offset, a date, a lowercase ``z``) or
  cannot read, a time without a value, the tracker's order at one instant,
  moves out of an empty source and with a held leg, a move whose fee
  spans both regimes, an override of a move's regime, an election an
  override drops, and wallets whose ids the tracker merges; and swap pairs
  whose out leg is a move's, whose leg is not a row, without an id (also
  over a direct-payout row), whose quarantined transaction only the pair
  names, whose leg's transaction has two reasons, with an unconfirmed leg,
  sharing a row, and one linked on a first-pass regime the second pass
  changes; and rail pairs whose source has no basis and no reason, whose
  source has no amount, and whose coarse in leg the carry prices.

A failed pass is compared by the stages it reached and its signature,
raised through the production stage table (``errors.ledger_error``); the
same pass on RP2, the oracle production must equal, fails exactly so. Two
books where the legacy-native backend checks an infinite value before RP2's
constructor would take RP2's failure and the legacy-native stages.

The engine names rows by id; the fields the reader takes from the rows and
pairs (times, wallet ids, external ids, pairing sources) are filled in from
the projection here, and its quarantines and audit rows are rendered by the
production reader (``ledger_response.render_quarantine``,
``render_audit``), so the ``detail_kind`` enrichment and the audit's key
order are compared too.

A last check runs the whole oracle on the legacy-native backend and records
each normalization and walk it performs, in the carry's passes and both
later passes, so the mirror of the adapter's calls used here cannot drift
from the adapter's own.
"""

from __future__ import annotations

import dataclasses
import json
from collections import defaultdict
from contextlib import contextmanager
from decimal import Decimal
from functools import lru_cache
from pathlib import Path
from typing import Any, Iterator, Mapping, Optional
from unittest.mock import patch

import kassiber_tax
import pytest

from kassiber.core import pricing
from kassiber.core.engines import ledger_request, ledger_response
from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines.errors import ledger_error
from tests import tax_engine_compare as compare
from tests import tax_ledger_codec as codec
from tests.tax_ledger_generator import (
    WALLET_REFS,
    chain_order,
    chain_txid,
    direct,
    graph_json,
    move,
    profile,
    projected,
    random_inputs,
)
from tests.tax_oracle import adapter as oracle_adapter

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tax-engine" / "core" / "tests" / "data" / "ledger_fixtures.json"
RANDOM_BOOKS = 200
RANDOM_CARRY_BOOKS = 100
CARRYING_VALUE = "carrying-value"
GENERIC = frozenset({"generic"})
GENERIC_CARRY = frozenset({"generic", "generic_carry"})
AUSTRIAN = frozenset({"at"})
AUSTRIAN_SWAPS = frozenset({"at", "at_swap"})

Document = dict[str, Any]


@lru_cache(maxsize=1)
def _document() -> dict[str, Any]:
    return json.loads(FIXTURES.read_text(encoding="utf-8"))


def _case_names(capabilities: frozenset[str]) -> list[str]:
    """The golden cases whose books use exactly ``capabilities``."""
    return [
        case["name"]
        for case in _document()["cases"]
        if case["request"] is not None and compare.capabilities(case["request"]) == capabilities
    ]


def _generic_case_names() -> list[str]:
    """The golden cases whose books are generic and run no rail carry."""
    return _case_names(GENERIC)


def _austrian_case_names() -> list[str]:
    """The golden cases whose books are Austrian without a carrying-value pair."""
    return _case_names(AUSTRIAN)


def _swap_case_names() -> list[str]:
    """The golden cases whose books are Austrian with a carrying-value pair."""
    return _case_names(AUSTRIAN_SWAPS)


def _carry_case_names() -> list[str]:
    """The golden cases whose books run the generic rail carry."""
    return _case_names(GENERIC_CARRY)


def _golden(name: str) -> tuple[Mapping[str, Any], TaxEngineLedgerInputs]:
    case = next(case for case in _document()["cases"] if case["name"] == name)
    return codec.decode_inputs(case["inputs"])


@lru_cache(maxsize=2)
def _random_seeds(country: str = "generic") -> tuple[int, ...]:
    """The first ``RANDOM_BOOKS`` seeds whose book of ``country`` has no carrying-value cross-asset pair."""
    seeds: list[int] = []
    seed = 0
    while len(seeds) < RANDOM_BOOKS:
        book = random_inputs(seed, country)
        pairs = book.inputs.finalized_tax_projection.cross_asset_pairs
        if not any(pair.get("policy") == CARRYING_VALUE for pair in pairs):
            seeds.append(seed)
        seed += 1
    return tuple(seeds)


@lru_cache(maxsize=1)
def _random_swap_seeds() -> tuple[int, ...]:
    """The first ``RANDOM_BOOKS`` seeds whose Austrian book selects swap links:
    one with a carrying-value cross-asset pair whose legs are rows."""
    seeds: list[int] = []
    seed = 0
    while len(seeds) < RANDOM_BOOKS:
        book = random_inputs(seed, "at")
        request = ledger_request.build_request(book.profile, book.inputs)
        if compare.capabilities(request) == AUSTRIAN_SWAPS:
            seeds.append(seed)
        seed += 1
    return tuple(seeds)


@lru_cache(maxsize=1)
def _random_carry_seeds() -> tuple[int, ...]:
    """The first ``RANDOM_CARRY_BOOKS`` seeds whose generic book runs the rail carry."""
    seeds: list[int] = []
    seed = 0
    while len(seeds) < RANDOM_CARRY_BOOKS:
        book = random_inputs(seed, "generic")
        request = ledger_request.build_request(book.profile, book.inputs)
        if compare.capabilities(request) == GENERIC_CARRY:
            seeds.append(seed)
        seed += 1
    return tuple(seeds)


# ---------------------------------------------------------------------------
# The oracle's normalized stage


@dataclasses.dataclass
class _OracleRun:
    """What the adapter's passes over one book left: each pass's
    ``[normalized, prepared]`` per asset (the carry's passes first), the
    carry's result and how many passes it ran (``None`` when it raised),
    the swap-link selection (``None`` when the first pass after the carry
    raised), and every row the carry rewrote, kept so each stays the object
    it was."""

    passes: list[list[list[Any]]] = dataclasses.field(default_factory=list)
    carry: Optional[tuple[Any, int]] = None
    selection: Optional[tuple[dict[str, str], set[str], list[dict[str, Any]]]] = None
    rewrites: list[Mapping[str, Any]] = dataclasses.field(default_factory=list)

    def positions(self, projection: Any) -> dict[int, int]:
        """Each projection row, and each rewrite of one, by object identity."""
        index = {str(row["id"]): position for position, row in enumerate(projection.rows)}
        positions = {id(row): position for position, row in enumerate(projection.rows)}
        positions.update({id(row): index[str(row["id"])] for row in self.rewrites})
        return positions


@contextmanager
def _configuration(book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> Iterator[Any]:
    """The adapter's configuration, opened as its ``build_ledger_state``
    opens it (``tests/tax_oracle/adapter.py:2333-2342``), on the backend in
    force."""
    projection = inputs.finalized_tax_projection
    wallet_labels = [
        str(item["label"]) for item in inputs.wallet_refs_by_id.values() if item.get("label") not in (None, "")
    ]
    assets = {str(oracle_adapter._row_get(row, "asset")) for row in projection.rows}
    with oracle_adapter._rp2_configuration(book_profile, wallet_labels, assets) as configuration:
        yield configuration


def _oracle_passes(book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs, run: _OracleRun) -> None:
    """The adapter's prepare passes, as ``_build_finalized_ledger_state``
    runs them (``tests/tax_oracle/adapter.py:2209-2266``): the generic rail
    carry (``_apply_generic_bitcoin_rail_carry_values``, which returns an
    Austrian book's rows as they are), whose passes prepare and compute the
    whole book; ``_prepare_assets`` over the rows the carry leaves, without
    the rows it withholds; the swap-link selection over that pass (nothing on
    a generic book); and, when the selection links or blocks a pair, the
    second pass, with the links and also without the blocked rows. Each pass
    appends a list to ``run.passes``, and each asset's ``[normalized,
    prepared]`` to that list as it goes (``prepared`` stays ``None`` when the
    walk raises)."""
    adapter = oracle_adapter
    projection = inputs.finalized_tax_projection
    walk = adapter._prepare_rp2_asset_input
    prepare = adapter._prepare_assets
    rewrite = adapter._with_carried_fiat_value

    def recording(walk_profile: Mapping[str, Any], normalized: Any, configuration: Any) -> Any:
        run.passes[-1].append([normalized, None])
        prepared = walk(walk_profile, normalized, configuration)
        run.passes[-1][-1][1] = prepared
        return prepared

    def prepare_pass(*args: Any, **kwargs: Any) -> Any:
        run.passes.append([])
        return prepare(*args, **kwargs)

    def rewriting(row: Mapping[str, Any], carried_basis: Decimal) -> dict[str, Any]:
        run.rewrites.append(rewrite(row, carried_basis))
        return run.rewrites[-1]

    pairs_by_asset: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    for pair in projection.intra_pairs:
        pairs_by_asset[str(adapter._row_get(pair["out"], "asset"))].append(pair)
    with _configuration(book_profile, inputs) as configuration, patch.multiple(
        adapter,
        _prepare_rp2_asset_input=recording,
        _prepare_assets=prepare_pass,
        _with_carried_fiat_value=rewriting,
    ):
        generic_rail = adapter._apply_generic_bitcoin_rail_carry_values(
            book_profile,
            list(projection.rows),
            projection.cross_asset_pairs,
            pairs_by_asset,
            configuration,
            inputs.wallet_refs_by_id,
            excluded_row_ids=set(),
            loan_leg_by_transaction_id={},
        )
        run.carry = (generic_rail, len(run.passes))
        rows_by_asset: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
        for row in generic_rail.rows:
            rows_by_asset[str(adapter._row_get(row, "asset"))].append(row)
        arguments = (book_profile, rows_by_asset, inputs.wallet_refs_by_id, pairs_by_asset, configuration)
        prepared_by_asset = adapter._prepare_assets(
            *arguments, excluded_row_ids=generic_rail.blocked_row_ids, loan_leg_by_transaction_id={}
        )
        links, blocked, quarantines = adapter._select_at_cross_asset_swap_links(
            book_profile, projection.cross_asset_pairs, generic_rail.rows, prepared_by_asset
        )
        run.selection = (links, blocked, quarantines)
        if links or blocked:
            adapter._prepare_assets(
                *arguments,
                at_swap_link_by_row_id=links,
                excluded_row_ids=set(generic_rail.blocked_row_ids) | blocked,
                loan_leg_by_transaction_id={},
            )


def _fields(item: Any) -> Document:
    return {field.name: getattr(item, field.name) for field in dataclasses.fields(item)}


def _row_ref(position: Mapping[int, int], row: Mapping[str, Any]) -> tuple[str, Any]:
    # Which projection row an event or move leg is: the row object itself
    # (or the carry's rewrite of it), not a copy.
    index = position.get(id(row))
    return ("row", index) if index is not None else ("not a projection row", row.get("id"))


def _oracle_asset(position: Mapping[int, int], normalized: Any) -> Document:
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


def _oracle_prepared(normalized: Any, prepared: Any) -> Document:
    """The prepared stage: the entries as v1 documents, the gates'
    quarantines (the adapter's list starts with normalize's), and the
    transfer audit with its legs' description."""
    gate_quarantines = prepared.quarantines[len(normalized.quarantines) :]
    assert prepared.quarantines[: len(normalized.quarantines)] == list(normalized.quarantines)
    input_data = prepared.input_data
    return {
        "entries": None if input_data is None else [entry.request_entry for entry in input_data.entries()],
        "quarantines": gate_quarantines,
        "intra_audit": prepared.intra_audit,
        "descriptions": [
            oracle_adapter._compose_transfer_journal_description(audit) for audit in prepared.intra_audit
        ],
    }


def _oracle_selection(
    selection: Optional[tuple[dict[str, str], set[str], list[dict[str, Any]]]],
) -> Optional[Document]:
    """The swap-link selection: each linked row and its link in the order
    the pairs linked them, the blocked ids sorted, and the pairs'
    quarantines; ``None`` when the first pass failed before it."""
    if selection is None:
        return None
    links, blocked, quarantines = selection
    return {"links": list(links.items()), "blocked": sorted(blocked), "quarantines": list(quarantines)}


def _oracle_carry(projection: Any, carry: Optional[tuple[Any, int]]) -> Optional[Document]:
    """The rail carry: each row it rewrote, in row order, with its carried
    value and rate; the withheld ids sorted; the blocked pairs' quarantines;
    and its passes. ``None`` when a pass of it raised."""
    if carry is None:
        return None
    generic_rail, passes = carry
    return {
        "carried": [
            (str(row["id"]), row["fiat_value_exact"], row["fiat_rate_exact"])
            for row, original in zip(generic_rail.rows, projection.rows)
            if row is not original
        ],
        "blocked": sorted(generic_rail.blocked_row_ids),
        "quarantines": list(generic_rail.quarantines),
        "passes": passes,
    }


def _oracle_document(book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> Document:
    projection = inputs.finalized_tax_projection
    run = _OracleRun()
    _, error = compare.run_on_oracle_backend(
        compare.LEGACY_NATIVE_BACKEND, lambda: _oracle_passes(book_profile, inputs, run)
    )
    final_pass = run.passes[-1] if run.passes else []
    position = run.positions(projection)
    return {
        "assets": [
            {
                "normalized": _oracle_asset(position, normalized),
                "prepared": None if prepared is None else _oracle_prepared(normalized, prepared),
            }
            for normalized, prepared in final_pass
        ],
        "swap_links": _oracle_selection(run.selection),
        "rail_carry": _oracle_carry(projection, run.carry),
        "error": compare.error_signature(error),
    }


def _rp2_error(book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> Optional[tuple[Any, ...]]:
    """How the same passes fail on RP2, the oracle production must equal."""
    _, error = compare.run_on_oracle_backend(
        compare.ORACLE_BACKEND, lambda: _oracle_passes(book_profile, inputs, _OracleRun())
    )
    return compare.error_signature(error)


# ---------------------------------------------------------------------------
# The engine's stages


def _decimal(text: Optional[str]) -> Optional[Decimal]:
    if text is None:
        return None
    # A price or value the row stored as ``Infinity`` stays one.
    return Decimal(text) if text == "Infinity" else ledger_request.decode_decimal(text)


def _engine_asset(
    book_profile: Mapping[str, Any],
    inputs: TaxEngineLedgerInputs,
    asset: Mapping[str, Any],
    rows_by_id: Mapping[str, Mapping[str, Any]],
) -> Document:
    projection = inputs.finalized_tax_projection
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


def _engine_prepared(
    book_profile: Mapping[str, Any],
    inputs: TaxEngineLedgerInputs,
    prepared: Mapping[str, Any],
    rows_by_id: Mapping[str, Mapping[str, Any]],
) -> Document:
    return {
        "entries": prepared["entries"],
        "quarantines": [
            ledger_response.render_quarantine(book_profile, rows_by_id, item) for item in prepared["quarantines"]
        ],
        "intra_audit": [ledger_response.render_audit(inputs, item) for item in prepared["intra_audit"]],
        "descriptions": [item["description"] for item in prepared["intra_audit"]],
    }


def _engine_selection(
    book_profile: Mapping[str, Any],
    swap_links: Optional[Mapping[str, Any]],
    rows_by_id: Mapping[str, Mapping[str, Any]],
) -> Optional[Document]:
    if swap_links is None:
        return None
    return {
        "links": [tuple(item) for item in swap_links["links"]],
        "blocked": list(swap_links["blocked"]),
        "quarantines": [
            ledger_response.render_quarantine(book_profile, rows_by_id, item) for item in swap_links["quarantines"]
        ],
    }


def _engine_carry(
    book_profile: Mapping[str, Any],
    rail_carry: Optional[Mapping[str, Any]],
    rows_by_id: Mapping[str, Mapping[str, Any]],
) -> Optional[Document]:
    if rail_carry is None:
        return None
    return {
        "carried": [(item["row"], item["value"], item["rate"]) for item in rail_carry["carried"]],
        "blocked": list(rail_carry["blocked"]),
        "quarantines": [
            ledger_response.render_quarantine(book_profile, rows_by_id, item) for item in rail_carry["quarantines"]
        ],
        "passes": rail_carry["passes"],
    }


def _engine_response(book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> dict[str, Any]:
    request = ledger_request.build_request(book_profile, inputs, operation="prepare")
    assert compare.capabilities(request) in (GENERIC, GENERIC_CARRY, AUSTRIAN, AUSTRIAN_SWAPS)
    return ledger_response.loads(kassiber_tax.ledger(ledger_request.dumps(request)))


def _engine_document(book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> Document:
    response = _engine_response(book_profile, inputs)
    if response["ok"] is True:
        assert response["engine_version"] == kassiber_tax.engine_version()
        error = None
    else:
        error = compare.error_signature(ledger_error(response["error"]))
    rail_carry = response["rail_carry"]
    # The rows as the final pass read them, after the carry, as the reader
    # renders them.
    rows_by_id = ledger_response.rows_after_carry(
        inputs.finalized_tax_projection.rows, rail_carry["carried"] if rail_carry else []
    )
    return {
        "assets": [
            {
                "normalized": _engine_asset(book_profile, inputs, asset, rows_by_id),
                "prepared": None
                if asset["prepared"] is None
                else _engine_prepared(book_profile, inputs, asset["prepared"], rows_by_id),
            }
            for asset in response["assets"]
        ],
        "swap_links": _engine_selection(book_profile, response["swap_links"], rows_by_id),
        "rail_carry": _engine_carry(book_profile, rail_carry, rows_by_id),
        "error": error,
    }


def _assert_same_stages(book_profile: Mapping[str, Any], inputs: TaxEngineLedgerInputs) -> Document:
    expected = _oracle_document(book_profile, inputs)
    actual = _engine_document(book_profile, inputs)
    assert compare.value_differences(expected, actual, "prepare") == []
    assert _rp2_error(book_profile, inputs) == expected["error"]
    return expected


# ---------------------------------------------------------------------------
# The comparisons


def _gate_quarantines(document: Document) -> list[tuple[str, str, dict[str, Any]]]:
    return [
        (item["transaction_id"], item["reason"], json.loads(item["detail_json"]))
        for asset in document["assets"]
        if asset["prepared"] is not None
        for item in asset["prepared"]["quarantines"]
    ]


def test_the_corpus_reaches_the_normalize_and_walk_branches() -> None:
    """The golden and random books compared below reach booked moves,
    tainted lot states, every quarantine normalize raises on a generic book
    (its group blocking through the goldens ``generic-bundle-unpriced-move``
    and ``generic-bundle-blocked-twice``), and every gate of a generic
    book's walk. ``missing_cost_basis`` is not among them: on a
    generic book the priced amount is the sum of the accounts, so the
    account gate fails first (only an Austrian swap-out may overdraw;
    ``ledger::walk`` tests it)."""
    reasons: dict[str, set[str]] = {"golden": set(), "random": set()}
    gates: dict[str, set[str]] = {"golden": set(), "random": set()}
    transfers = contaminated = audit_rows = without_inbound = 0
    books = [("golden", *_golden(name)) for name in _generic_case_names()]
    for seed in _random_seeds():
        book = random_inputs(seed, "generic")
        books.append(("random", book.profile, book.inputs))
    for source, book_profile, inputs in books:
        document = _oracle_document(book_profile, inputs)
        assert document["error"] is None, (source, document["error"])
        for asset in document["assets"]:
            normalized, prepared = asset["normalized"], asset["prepared"]
            reasons[source] |= {item["reason"] for item in normalized["quarantines"]}
            transfers += len(normalized["transfers"])
            contaminated += normalized["earliest_lot_contamination_at"] is not None
            gates[source] |= {item["reason"] for item in prepared["quarantines"]}
            audit_rows += len(prepared["intra_audit"])
            without_inbound += prepared["entries"] is None
    assert len(_generic_case_names()) >= 35
    assert reasons["golden"] | reasons["random"] == {
        "missing_spot_price",
        "pricing_review_required",
        "privacy_hop_unresolved",
        "conflicting_spend",
        "pending_onchain_confirmation",
        "derived_transfer_group_blocked",
    }, reasons
    assert gates["golden"] | gates["random"] == {
        "insufficient_lots",
        "basis_provenance_incomplete",
        "derived_transfer_group_blocked",
        "unclassified_income_kind",
        "non_sale_disposal_kind",
    }, gates
    assert transfers > 0 and contaminated > 0 and audit_rows > 0 and without_inbound > 0


def test_the_ungrouped_gate_and_coarse_move_goldens_are_compared() -> None:
    """The goldens design §8 names for this comparison are among the
    compared books, with their shapes: an ungrouped move that fails the gate
    is quarantined on both legs when they are two transactions and once
    when they share one; a coarse fee-bearing move is reviewed on its priced
    leg with its partner, and neither leg books."""
    names = set(_generic_case_names())
    ungrouped = {
        "generic-ungrouped-pass",
        "generic-ungrouped-fail-distinct-anchors",
        "generic-ungrouped-fail-shared-anchor",
    }
    coarse = {"generic-coarse-move-out-priced", "generic-coarse-move-in-priced"}
    assert ungrouped | coarse <= names

    def gates(name: str) -> list[tuple[str, str, dict[str, Any]]]:
        return [item for item in _gate_quarantines(_oracle_document(*_golden(name))) if item[1] == "insufficient_lots"]

    assert gates("generic-ungrouped-pass") == []
    distinct = gates("generic-ungrouped-fail-distinct-anchors")
    assert len(distinct) == 2 and distinct[1][2]["paired_leg"] == "inbound"
    assert distinct[1][2]["blocked_by_transaction_id"] == distinct[0][0] != distinct[1][0]
    shared = gates("generic-ungrouped-fail-shared-anchor")
    assert len(shared) == 1 and "paired_leg" not in shared[0][2]
    for name in sorted(coarse):
        document = _oracle_document(*_golden(name))
        normalized = document["assets"][0]["normalized"]
        details = [json.loads(item["detail_json"]) for item in normalized["quarantines"]]
        assert [item["reason"] for item in normalized["quarantines"]][:2] == ["pricing_review_required"] * 2, name
        assert "pricing_quality" in details[0] and details[1]["paired_leg"] is True, name


def test_every_golden_book_is_compared() -> None:
    names = _generic_case_names() + _carry_case_names() + _austrian_case_names() + _swap_case_names()
    requested = [case["name"] for case in _document()["cases"] if case["request"] is not None]
    assert sorted(names) == sorted(requested)


@pytest.mark.parametrize(
    "name", _generic_case_names() + _carry_case_names() + _austrian_case_names() + _swap_case_names()
)
def test_the_golden_books_prepare_as_the_oracle(name: str) -> None:
    _assert_same_stages(*_golden(name))


@pytest.mark.parametrize("country", ["generic", "at"])
def test_the_random_books_prepare_as_the_oracle(country: str) -> None:
    differing: list[str] = []
    failed = 0
    for seed in _random_seeds(country):
        book = random_inputs(seed, country)
        expected = _oracle_document(book.profile, book.inputs)
        failed += expected["error"] is not None
        differences = compare.value_differences(expected, _engine_document(book.profile, book.inputs), "prepare")
        if differences:
            differing.append(f"seed {seed}: {differences[:3]}")
        if _rp2_error(book.profile, book.inputs) != expected["error"]:
            differing.append(f"seed {seed}: RP2 fails otherwise")
    assert differing == []
    assert failed == 0


def test_the_random_swap_books_prepare_as_the_oracle() -> None:
    """The first 200 seeded Austrian books with swap links: both passes'
    selection and the final pass. Some fail in a pass where the oracle does
    (a marker a row's text repeats, read by an entry's constructor); the
    rest fail later, if at all, in the lot engine (``test_tax_ledger_goldens``
    compares those outcomes)."""
    differing: list[str] = []
    second_passes = 0
    for seed in _random_swap_seeds():
        book = random_inputs(seed, "at")
        expected = _oracle_document(book.profile, book.inputs)
        differences = compare.value_differences(expected, _engine_document(book.profile, book.inputs), "prepare")
        if differences:
            differing.append(f"seed {seed}: {differences[:3]}")
        if _rp2_error(book.profile, book.inputs) != expected["error"]:
            differing.append(f"seed {seed}: RP2 fails otherwise")
        selection = expected["swap_links"]
        second_passes += selection is not None and bool(selection["links"] or selection["blocked"])
    assert differing == []
    assert second_passes > RANDOM_BOOKS // 2, second_passes


def test_the_random_carry_books_prepare_as_the_oracle() -> None:
    """The first 100 seeded generic books with the rail carry: the carry,
    with every pass it ran, and the final pass. None fails a pass; some
    block a pair, and some carry a hop's basis on to another hop."""
    differing: list[str] = []
    blocking = carrying = repeated = 0
    for seed in _random_carry_seeds():
        book = random_inputs(seed, "generic")
        expected = _oracle_document(book.profile, book.inputs)
        assert expected["error"] is None, (seed, expected["error"])
        differences = compare.value_differences(expected, _engine_document(book.profile, book.inputs), "prepare")
        if differences:
            differing.append(f"seed {seed}: {differences[:3]}")
        carry = expected["rail_carry"]
        blocking += bool(carry["blocked"])
        carrying += bool(carry["carried"])
        repeated += carry["passes"] > 2
    assert differing == []
    assert blocking > RANDOM_CARRY_BOOKS // 10 and carrying > RANDOM_CARRY_BOOKS // 4, (blocking, carrying)
    assert repeated > 0


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
    # The walk buckets by the text of a time, so the sale, spelled with an
    # offset, sorts before the purchase at the same instant.
    "same-instant-two-spellings": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z"),
        projected("sell", "A", "outbound", 10_000_000_000, "2024-01-01T00:00:00+00:00"),
    ],
    # A sale priced at Infinity: without a fee, `fee * spot_price` raises
    # before the constructor; with one, the constructor's price check does.
    "sell-rate-infinity": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z"),
        projected("sell", "A", "outbound", 10_000_000_000, "2024-02-01T00:00:00Z", rate="Infinity"),
    ],
    "sell-rate-infinity-fee": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z"),
        projected("sell", "A", "outbound", 10_000_000_000, "2024-02-01T00:00:00Z", fee=1_000, rate="Infinity"),
    ],
    # A finite price with an infinite value fails at the proceeds check.
    "sell-value-infinity": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z"),
        projected("sell", "A", "outbound", 10_000_000_000, "2024-02-01T00:00:00Z", fiat_value="Infinity"),
    ],
    # The fee's value overflows while the sale's own value does not.
    "fee-times-price-overflows": lambda: [
        projected("buy", "A", "inbound", 400_000_000_000, "2024-01-01T00:00:00Z"),
        projected("sell", "A", "outbound", 1, "2024-02-01T00:00:00Z", fee=200_000_000_000, rate="9E+999999"),
    ],
    # The constructor parses the time: text it cannot read, and a time
    # without an offset.
    "unparsed-timestamp": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z"),
        projected("late", "A", "inbound", 1_000, "not a time"),
    ],
    "naive-timestamp": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z"),
        projected("late", "A", "inbound", 1_000, "2024-03-01T00:00:00"),
    ],
    # An asset without an inbound entry hands the lot engine nothing.
    "asset-without-inbound": lambda: [
        projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z"),
        projected("lbtc-sell", "L", "outbound", 1_000, "2024-02-01T00:00:00Z", asset="LBTC"),
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
    # A move priced at Infinity fails at the constructor's price check.
    out, inbound = legs("m", "A", "B", "2024-02-01T00:00:00Z", rate="Infinity")
    books["move-rate-infinity"] = ([buy("buy", "A"), out, inbound], [move(out, inbound)], WALLET_REFS)

    # A group that passes its check at its first move, then a sale drains
    # the source: the first move stays booked, and the move that no longer
    # fits and the group's later one are quarantined.
    one_out, one_in = legs("g-one", "A", "B", "2024-02-01T00:00:00Z", amount=15_000_000_000)
    spend = projected("spend", "A", "outbound", 30_000_000_000, "2024-02-02T00:00:00Z")
    two_out = projected("g-two-out", "A", "outbound", 15_000_000_000, "2024-02-03T00:00:00Z", fee=1_000_000, anchor="g-two", side="out")
    two_in = projected("g-two-in", "C", "inbound", 15_000_000_000, "2024-02-03T00:00:00Z", anchor="g-two-receipt", side="in")
    three_out, three_in = legs("g-three", "B", "C", "2024-02-04T00:00:00Z", amount=1_000_000_000)
    books["group-drained-between-moves"] = (
        [buy("buy", "A"), one_out, one_in, spend, two_out, two_in, three_out, three_in],
        [move(one_out, one_in, group_id="g"), move(two_out, two_in, group_id="g"), move(three_out, three_in, group_id="g")],
        WALLET_REFS,
    )

    # A group whose check fails at its second move, which leaves from a
    # slice of the first move's transaction: each transaction is named once.
    one_out, one_in = legs("p-one", "A", "B", "2024-02-01T00:00:00Z", amount=15_000_000_000)
    two_out = projected("p-two-out", "A", "outbound", 45_000_000_000, "2024-02-02T00:00:00Z", fee=1_000_000, anchor="p-one", side="out2")
    two_in = projected("p-two-in", "C", "inbound", 45_000_000_000, "2024-02-02T00:00:00Z", anchor="p-two", side="in2")
    books["group-check-fails-at-a-later-move"] = (
        [buy("buy", "A"), one_out, one_in, two_out, two_in],
        [move(one_out, one_in, group_id="g"), move(two_out, two_in, group_id="g")],
        WALLET_REFS,
    )

    # Two same-time moves whose wallets hint each other's order (a cycle):
    # the chain evidence says the second spends the first, which also funds
    # the second's source.
    late_out, late_in = legs("late", "B", "A", "2024-02-01T00:00:00Z", amount=5_000_000_000, chain_order=chain_order("late", "early"))
    early_out, early_in = legs("early", "A", "B", "2024-02-01T00:00:00Z", chain_order=chain_order("early"))
    books["chain-evidence-against-wallet-hints"] = (
        [buy("buy", "A"), late_out, late_in, early_out, early_in],
        [move(late_out, late_in), move(early_out, early_in)],
        WALLET_REFS,
    )

    # Three same-time moves listed against a wallet hint: the second funds
    # the first's source, so it books first; the first, ready only then,
    # still goes before the third, which has been ready from the start.
    first_out, first_in = legs("first", "B", "C", "2024-02-01T00:00:00Z", amount=5_000_000_000)
    second_out, second_in = legs("second", "A", "B", "2024-02-01T00:00:00Z")
    third_out, third_in = legs("third", "A", "C", "2024-02-01T00:00:00Z")
    books["a-move-made-ready-precedes-one-waiting"] = (
        [buy("buy", "A"), first_out, first_in, second_out, second_in, third_out, third_in],
        [move(first_out, first_in), move(second_out, second_in), move(third_out, third_in)],
        WALLET_REFS,
    )

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
INVALID = ("InvalidOperation", "[<class 'decimal.InvalidOperation'>]")


@pytest.mark.parametrize("name", sorted(EDGE_BOOKS))
def test_edge_rows_prepare_as_the_oracle(name: str) -> None:
    _assert_same_stages(profile("generic", "FIFO"), direct(EDGE_BOOKS[name]()))


@pytest.mark.parametrize("name", EDGE_MOVES)
def test_edge_moves_prepare_as_the_oracle(name: str) -> None:
    rows, pairs, wallets = _edge_moves()[name]
    _assert_same_stages(profile("generic", "FIFO", require_coarse_review=True), direct(rows, pairs, wallet_refs=wallets))


def test_a_holder_label_with_spaces_fails_as_the_oracle() -> None:
    """The configuration's holder is the stripped profile label, while
    every entry carries the label as it is, so the first entry's holder is
    not known (a listed strip artifact, design §6)."""
    book_profile = profile("generic", "FIFO", label=" Default ")
    inputs = direct([projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z")])
    document = _assert_same_stages(book_profile, inputs)
    assert document["error"] == ("EngineValueError", "Parameter 'holder' value is not known:  Default ")
    assert [asset["prepared"] for asset in document["assets"]] == [None]


def test_rp2s_constructor_order_decides_where_an_infinite_value_fails() -> None:
    """Two books whose entry has an infinite value and fails an earlier
    constructor check too. RP2's constructor reaches the other check first:
    a wallet without a label is not a string, a time it cannot read does
    not parse. The legacy-native backend checks every value before it
    constructs, so it fails on the value; the engine fails as RP2 does, and
    its stages are the legacy-native ones."""
    generic = profile("generic", "FIFO")
    unlabelled = {**WALLET_REFS, "U": {**WALLET_REFS["A"], "id": "U", "label": None}}
    books = {
        "value-infinity-in-an-unlabelled-wallet": (
            direct(
                [_in_wallet(projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", fiat_value="Infinity"), "U")],
                wallet_refs=unlabelled,
            ),
            ("EngineTypeError", "Parameter 'exchange' has non-string value None"),
        ),
        "rate-infinity-at-an-unparsed-time": (
            direct([projected("buy", "A", "inbound", 100_000_000_000, "not a time", rate="Infinity")]),
            ("EngineValueError", "Error parsing parameter 'timestamp': Unknown string format: not a time"),
        ),
    }
    for name, (inputs, expected) in books.items():
        legacy = _oracle_document(generic, inputs)
        engine = _engine_document(generic, inputs)
        assert _rp2_error(generic, inputs) == engine["error"] == expected, name
        assert legacy["error"] == INVALID, name
        assert compare.value_differences({**legacy, "error": expected}, engine, "prepare") == [], name


def _quarantines(stage: Document, reason: str) -> list[tuple[str, dict[str, Any]]]:
    return [(item["transaction_id"], json.loads(item["detail_json"])) for item in stage["quarantines"] if item["reason"] == reason]


def test_the_edge_books_reach_their_shapes() -> None:
    """Each edge book shows on the oracle what its name says."""
    generic = profile("generic", "FIFO")
    rows = {name: _oracle_document(generic, direct(build())) for name, build in EDGE_BOOKS.items()}

    def normalize_raises(document: Document) -> bool:
        return document["error"] is not None and document["assets"] == []

    def walk_raises(document: Document) -> bool:
        return document["error"] is not None and [asset["prepared"] for asset in document["assets"]] == [None]

    assert {name for name, document in rows.items() if document["error"] == INVALID and normalize_raises(document)} == {
        "rate-nan",
        "rate-signaling-nan",
        "value-nan",
        "rate-positive-value-nan",
        "rate-infinity-value-nan",
        "fee-only-rate-infinity",
    }
    assert {name for name, document in rows.items() if document["error"] == INVALID and walk_raises(document)} == {
        "rate-infinity",
        "rate-infinity-value",
        "value-infinity",
        "sell-rate-infinity",
        "sell-rate-infinity-fee",
        "sell-value-infinity",
    }
    overflow = ("Overflow", "[<class 'decimal.Overflow'>]")
    assert rows["product-overflows"]["error"] == overflow and normalize_raises(rows["product-overflows"])
    assert rows["fee-times-price-overflows"]["error"] == overflow and walk_raises(rows["fee-times-price-overflows"])
    walk_errors = {
        "price-subnormal": "BTC InTransaction (2024-01-01 00:00:00+00:00, id 1): parameter 'spot_price' cannot be 0",
        "unparsed-timestamp": "Error parsing parameter 'timestamp': Unknown string format: not a time",
        "naive-timestamp": "Parameter 'timestamp' value has no timezone info: 2024-03-01T00:00:00",
    }
    for name, message in walk_errors.items():
        assert rows[name]["error"] == ("EngineValueError", message) and walk_raises(rows[name]), name

    def event(name: str, index: int = 0) -> Document:
        return rows[name]["assets"][0]["normalized"]["events"][index]

    infinity = Decimal("Infinity")
    assert (event("rate-infinity")["spot_price"], event("rate-infinity")["fiat_value"]) == (infinity, infinity)
    assert (event("rate-infinity-value")["spot_price"], event("rate-infinity-value")["fiat_value"]) == (infinity, Decimal("5"))
    assert (event("value-infinity")["spot_price"], event("value-infinity")["fiat_value"]) == (infinity, infinity)
    assert (event("sell-rate-infinity", 1)["spot_price"], event("sell-rate-infinity", 1)["fee"]) == (infinity, Decimal("0"))
    assert event("sell-rate-infinity-fee", 1)["spot_price"] == infinity
    assert (event("sell-value-infinity", 1)["spot_price"], event("sell-value-infinity", 1)["fiat_value"]) == (Decimal("40000"), infinity)
    assert event("rate-negative-infinity-value")["spot_price"] == Decimal("1000")
    assert (event("fee-only-value", 1)["spot_price"], event("fee-only-value", 1)["fiat_value"]) == (Decimal("3E+5"), Decimal("3"))
    assert event("price-subnormal")["spot_price"].as_tuple().exponent == -1000030
    assert [item["reason"] for item in rows["rate-zero-value-negative"]["assets"][0]["normalized"]["quarantines"]] == ["missing_spot_price"]

    fee_only = rows["fee-only-value"]["assets"][0]["prepared"]["entries"]
    assert [(entry["kind"], entry["transaction_type"], entry["fiat_out_no_fee"] if entry["kind"] == "out" else None) for entry in fee_only] == [
        ("in", "BUY", None),
        ("out", "FEE", None),
    ]
    spellings = rows["same-instant-two-spellings"]["assets"][0]["prepared"]
    assert _quarantines(spellings, "insufficient_lots") == [
        ("sell", {"asset": "BTC", "available": 0.0, "required": 0.1, "wallet": "Cold"})
    ]
    without_inbound = rows["asset-without-inbound"]["assets"]
    assert [asset["normalized"]["asset"] for asset in without_inbound] == ["BTC", "LBTC"]
    assert without_inbound[1]["prepared"]["entries"] is None
    assert [tx for tx, _ in _quarantines(without_inbound[1]["prepared"], "insufficient_lots")] == ["lbtc-sell"]

    review = profile("generic", "FIFO", require_coarse_review=True)
    moves = {name: _oracle_document(review, direct(book_rows, pairs, wallet_refs=wallets)) for name, (book_rows, pairs, wallets) in _edge_moves().items()}
    assert moves["move-rate-nan"]["error"] == INVALID and normalize_raises(moves["move-rate-nan"])
    assert moves["move-rate-infinity"]["error"] == INVALID and walk_raises(moves["move-rate-infinity"])
    assert moves["move-rate-infinity"]["assets"][0]["normalized"]["transfers"][0]["spot_price"] == infinity

    def normalized(name: str) -> Document:
        return moves[name]["assets"][0]["normalized"]

    def prepared(name: str) -> Document:
        return moves[name]["assets"][0]["prepared"]

    assert normalized("move-priced-by-in-leg")["transfers"][0]["spot_price"] == Decimal("41000")
    assert normalized("move-rate-positive-value-nan")["transfers"][0]["spot_price"] == Decimal("40000")

    ungrouped = normalized("ungrouped-unpriced-move")
    assert [(tx, "transfer_group_id" in detail) for tx, detail in _quarantines(ungrouped, "missing_spot_price")] == [("m", False), ("m", False)]
    assert _quarantines(ungrouped, "derived_transfer_group_blocked") == []

    blocked = normalized("two-blocked-groups-sharing-transactions")
    assert blocked["transfers"] == []
    assert [(tx, detail["transfer_group_id"], detail["blocked_by_reason"]) for tx, detail in _quarantines(blocked, "derived_transfer_group_blocked")] == [
        ("g1-pass", "g1", "missing_spot_price"),
        ("g2-new", "g2", "missing_spot_price"),
    ]

    first_reason = normalized("blocked-group-keeps-its-first-reason")
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

    # The walk's group gate, checked again at a later move after a spend.
    drained = prepared("group-drained-between-moves")
    assert [(item["transaction_id"], item["reason"], json.loads(item["detail_json"]).get("paired_leg")) for item in drained["quarantines"]] == [
        ("g-two", "insufficient_lots", None),
        ("g-two-receipt", "derived_transfer_group_blocked", True),
        ("g-three", "derived_transfer_group_blocked", False),
    ]
    assert json.loads(drained["quarantines"][0]["detail_json"])["transfer_group_id"] == "g"
    assert [audit["out_anchor_transaction_id"] for audit in drained["intra_audit"]] == ["g-one"]
    assert [entry["kind"] for entry in drained["entries"]] == ["in", "out", "intra"]
    # The group's check at its first move, failing at the second; each
    # transaction is named once.
    later = prepared("group-check-fails-at-a-later-move")
    assert [(item["transaction_id"], item["reason"]) for item in later["quarantines"]] == [
        ("p-one", "insufficient_lots"),
        ("p-two", "derived_transfer_group_blocked"),
    ]
    assert later["intra_audit"] == []

    # Chain evidence orders the moves against their wallets' hints, so the
    # move listed second books first and funds the other.
    chained = prepared("chain-evidence-against-wallet-hints")
    assert chained["quarantines"] == []
    assert [audit["out_anchor_transaction_id"] for audit in chained["intra_audit"]] == ["early", "late"]
    # A move that becomes ready joins the waiting ones in normalize's order.
    ready = moves["a-move-made-ready-precedes-one-waiting"]["assets"][0]
    assert [(transfer["from_wallet_label"], transfer["to_wallet_label"]) for transfer in ready["normalized"]["transfers"]] == [
        ("Hot", "Exchange"),
        ("Cold", "Hot"),
        ("Cold", "Exchange"),
    ]
    assert ready["prepared"]["quarantines"] == []
    assert [audit["out_anchor_transaction_id"] for audit in ready["prepared"]["intra_audit"]] == ["second", "first", "third"]

    assert moves["unlabelled-wallet"]["error"] == ("EngineTypeError", "Parameter 'exchange' has non-string value None")
    assert walk_raises(moves["unlabelled-wallet"])
    unlabelled = normalized("unlabelled-wallet")
    assert [transfer["description"] for transfer in unlabelled["transfers"]] == ["Transfer None -> Hot"]
    assert [(tx, detail["from_wallet"]) for tx, detail in _quarantines(unlabelled, "missing_spot_price")[:2]] == [("v", None), ("v", None)]
    assert [(tx, detail["wallet"]) for tx, detail in _quarantines(unlabelled, "derived_transfer_group_blocked")] == [("w", "U")]
    assert _quarantines(unlabelled, "missing_spot_price")[2] == (
        "unpriced",
        {"asset": "BTC", "direction": "inbound", "required_for": "acquisition", "wallet": None},
    )

    untimed = normalized("empty-timestamp")
    assert [tx for tx, _ in _quarantines(untimed, "missing_spot_price")] == ["untimed", "unpriced"]
    assert untimed["earliest_lot_contamination_at"] == "2024-02-01T00:00:00Z"

    text = normalized("text-fallbacks")
    assert [item["description"] for item in text["events"]] == [text["events"][0]["transaction_id"], "paid rent"]
    assert [transfer["description"] for transfer in text["transfers"]] == ["cold storage top-up"]
    # The descriptions are the entries' notes; a move's legs are described
    # by their wallets.
    entries = prepared("text-fallbacks")["entries"]
    assert [entry["notes"] for entry in entries] == [text["events"][0]["transaction_id"], "paid rent", "cold storage top-up"]
    assert prepared("text-fallbacks")["descriptions"] == ["Transfer Cold -> Hot"]


# ---------------------------------------------------------------------------
# Austrian edge books


ALT_DAY = "2020-06-01T00:00:00Z"


def _austrian_edges() -> dict[str, tuple[list[dict[str, Any]], list[dict[str, Any]], Mapping[str, Any]]]:
    """Austrian books for the tracker's shapes the corpus does not reach:
    each name says the shape (checked on the oracle below)."""

    def buy(tx: str, wallet: str, at: str, amount: int = 50_000_000_000, **fields: Any) -> dict[str, Any]:
        return projected(tx, wallet, "inbound", amount, at, kind="buy", **fields)

    def sell(tx: str, wallet: str, at: str, amount: int = 10_000_000_000, **fields: Any) -> dict[str, Any]:
        return projected(tx, wallet, "outbound", amount, at, kind="sell", **fields)

    def legs(tx: str, source: str, target: str, at: str, *, amount: int = 10_000_000_000, fee: int = 0, out: Optional[dict[str, Any]] = None, inbound: Optional[dict[str, Any]] = None):
        out_row = projected(f"{tx}-out", source, "outbound", amount, at, fee=fee, anchor=tx, side="out", kind="transfer", **(out or {}))
        in_row = projected(f"{tx}-in", target, "inbound", amount, at, anchor=tx, side="in", kind="transfer", **(inbound or {}))
        return out_row, in_row

    books: dict[str, tuple[list[dict[str, Any]], list[dict[str, Any]], Mapping[str, Any]]] = {}

    # The tracker reads a time without an offset as UTC, where RP2 cannot
    # read it: the unpriced purchase is quarantined, but its Neu still
    # counts, so the wallet's later sale is an election.
    books["naive-time-counts-as-utc"] = (
        [buy("alt", "A", ALT_DAY), buy("naive", "A", "2021-02-28T23:30:00", rate=None), sell("sale", "A", "2022-06-01T00:00:00Z")],
        [],
        WALLET_REFS,
    )
    # A date is midnight UTC for the tracker; the walk's constructor refuses it.
    books["date-only-time"] = (
        [buy("alt", "A", ALT_DAY), buy("dated", "A", "2021-03-01"), sell("sale", "A", "2022-06-01T00:00:00Z")],
        [],
        WALLET_REFS,
    )
    # Times the tracker cannot read fail with Python's ValueError before
    # anything is normalized: text, a lowercase ``z`` RP2 would read, and an
    # empty time on a row normalize would quarantine.
    books["unreadable-time"] = ([buy("alt", "A", ALT_DAY), buy("bad", "A", "not a time")], [], WALLET_REFS)
    books["lowercase-z-time"] = ([buy("alt", "A", ALT_DAY), sell("z", "A", "2022-06-01T00:00:00z")], [], WALLET_REFS)
    books["empty-time"] = ([buy("alt", "A", ALT_DAY), buy("untimed", "A", "", rate=None)], [], WALLET_REFS)
    # One instant in the tracker's order: the purchase counts before the
    # sale, so the sale elects Neu; the instant spelled ``+00:00`` sorts
    # before ``Z``, so that sale sees only Alt.
    books["one-instant-in-the-trackers-order"] = (
        [
            buy("alt", "A", ALT_DAY),
            sell("same-time-sale", "A", "2022-06-01T00:00:00Z"),
            buy("same-time-buy", "A", "2022-06-01T00:00:00Z"),
            sell("offset-sale", "A", "2022-06-01T00:00:00+00:00"),
        ],
        [],
        WALLET_REFS,
    )
    # A move whose in leg is unconfirmed is no move: its out leg is a
    # standalone sale, for the tracker and for normalize.
    pending = chain_txid("pending-receipt")
    out_row, in_row = legs(
        "held", "A", "B", "2022-06-01T00:00:00Z",
        inbound={"external_id": pending, "raw_json": graph_json(pending, confirmed=False)},
    )
    books["move-with-a-pending-leg"] = (
        [buy("alt", "A", ALT_DAY), buy("neu", "A", "2022-01-01T00:00:00Z"), out_row, in_row],
        [move(out_row, in_row)],
        WALLET_REFS,
    )
    # A move whose fee spans the last Neu and Alt: the fee's first slice is
    # Neu, so the move's regime is Neu, and only Alt reaches the destination.
    out_row, in_row = legs("span", "A", "B", "2022-06-01T00:00:00Z", amount=50_000_000_000, fee=20_000_000)
    books["move-fee-spans-regimes"] = (
        [buy("alt", "A", ALT_DAY, amount=100_000_000_000), buy("neu", "A", "2022-01-01T00:00:00Z", amount=10_000_000), out_row, in_row],
        [move(out_row, in_row)],
        WALLET_REFS,
    )
    # Overrides: an inbound's override is its event's regime but moves no
    # availability, so its wallet's later sale is Alt; a sale's override
    # drops its election; a move's override replaces the tracker's regime
    # and keeps its flows.
    out_row, in_row = legs("said-alt", "A", "B", "2022-06-03T00:00:00Z", out={"at_regime_override": "alt"})
    books["overrides-and-the-tracker"] = (
        [
            buy("alt", "A", ALT_DAY),
            buy("alt-said-neu", "C", ALT_DAY, at_regime_override="neu"),
            buy("neu", "A", "2022-01-01T00:00:00Z"),
            sell("c-sale", "C", "2022-06-01T00:00:00Z"),
            sell("elected-said-neu", "A", "2022-06-02T00:00:00Z", at_regime_override="neu"),
            out_row,
            in_row,
        ],
        [move(out_row, in_row)],
        WALLET_REFS,
    )
    # A wallet with an empty id counts as ``default`` for the tracker, so it
    # shares the Alt of the wallet whose id is ``default``.
    merged = {
        **WALLET_REFS,
        "": {**WALLET_REFS["A"], "id": "", "label": "Blank"},
        "default": {**WALLET_REFS["A"], "id": "default", "label": "Fallback"},
    }
    books["wallet-ids-the-tracker-merges"] = (
        [_in_wallet(buy("alt", "A", ALT_DAY), "default"), _in_wallet(sell("blank-sale", "A", "2022-06-01T00:00:00Z"), "")],
        [],
        merged,
    )
    return books


AUSTRIAN_EDGES = sorted(_austrian_edges())


@pytest.mark.parametrize("name", AUSTRIAN_EDGES)
def test_austrian_edge_books_prepare_as_the_oracle(name: str) -> None:
    rows, pairs, wallets = _austrian_edges()[name]
    _assert_same_stages(profile("at", "moving_average_at"), direct(rows, pairs, wallet_refs=wallets))


def test_the_austrian_edge_books_reach_their_shapes() -> None:
    """Each Austrian edge book shows on the oracle what its name says."""
    books = _austrian_edges()
    documents = {
        name: _oracle_document(profile("at", "moving_average_at"), direct(rows, pairs, wallet_refs=wallets))
        for name, (rows, pairs, wallets) in books.items()
    }

    def anchors(name: str) -> dict[str, str]:
        return {str(row["id"]): str(row["journal_transaction_id"]) for row in books[name][0]}

    def events(name: str) -> dict[str, tuple[Any, Any]]:
        by_id = anchors(name)
        return {
            by_id[str(event["transaction_id"])]: (event["at_regime"], event["at_regime_basis"])
            for asset in documents[name]["assets"]
            for event in asset["normalized"]["events"]
        }

    def transfers(name: str) -> list[tuple[Any, Any]]:
        return [
            (transfer["at_regime"], transfer["regime_flows"])
            for asset in documents[name]["assets"]
            for transfer in asset["normalized"]["transfers"]
        ]

    naive = documents["naive-time-counts-as-utc"]
    assert naive["error"] is None
    assert [item["transaction_id"] for item in naive["assets"][0]["normalized"]["quarantines"]] == ["naive"]
    assert events("naive-time-counts-as-utc")["sale"] == ("neu", "wahlrecht")

    dated = documents["date-only-time"]
    assert dated["error"] == ("EngineValueError", "Parameter 'timestamp' value has no timezone info: 2021-03-01")
    assert dated["assets"][0]["prepared"] is None
    assert events("date-only-time") == {"alt": ("alt", None), "dated": ("neu", None), "sale": ("neu", "wahlrecht")}

    for name, text in (("unreadable-time", "not a time"), ("lowercase-z-time", "2022-06-01T00:00:00z"), ("empty-time", "")):
        assert documents[name]["error"] == ("ValueError", f"Invalid isoformat string: {text!r}"), name
        assert documents[name]["assets"] == [], name

    assert events("one-instant-in-the-trackers-order") == {
        "alt": ("alt", None),
        "same-time-sale": ("neu", "wahlrecht"),
        "same-time-buy": ("neu", None),
        "offset-sale": ("alt", None),
    }

    held = documents["move-with-a-pending-leg"]["assets"][0]["normalized"]
    assert [(item["transaction_id"], item["reason"]) for item in held["quarantines"]] == [("held", "pending_onchain_confirmation")]
    assert held["transfers"] == []
    assert events("move-with-a-pending-leg")["held"] == ("neu", "wahlrecht")

    assert transfers("move-fee-spans-regimes") == [
        ("neu", {"out": {"alt": 50_010_000_000, "neu": 10_000_000}, "in": {"alt": 50_000_000_000, "neu": 0}})
    ]

    overrides = events("overrides-and-the-tracker")
    assert overrides["alt-said-neu"] == ("neu", None)
    assert overrides["c-sale"] == ("alt", None)
    assert overrides["elected-said-neu"] == ("neu", None)
    [(regime, flows)] = transfers("overrides-and-the-tracker")
    assert regime == "alt" and flows["out"]["neu"] > 0

    assert events("wallet-ids-the-tracker-merges")["blank-sale"] == ("alt", None)


# ---------------------------------------------------------------------------
# Austrian swap-link selection


NEU_DAY = "2024-02-01T00:00:00Z"
SwapBook = tuple[list[dict[str, Any]], list[dict[str, Any]], list[dict[str, Any]]]


def _cross(out_row: Mapping[str, Any], in_row: Mapping[str, Any], pair_id: Optional[str] = "swap", **fields: Any) -> dict[str, Any]:
    """A reviewed carrying-value cross-asset pair, as the projection lists one."""
    return {
        "pair_id": pair_id,
        "component_id": None,
        "out_id": out_row["id"],
        "in_id": in_row["id"],
        "out_transaction_id": out_row["journal_transaction_id"],
        "in_transaction_id": in_row["journal_transaction_id"],
        "out_asset": out_row["asset"],
        "in_asset": in_row["asset"],
        "policy": CARRYING_VALUE,
        "kind": "manual",
        **fields,
    }


def _swap_edges() -> dict[str, SwapBook]:
    """Austrian books for selection shapes the swap goldens and seeded
    books do not reach: each name says the shape (checked on the oracle
    below). Rows are Neu unless their time is ``ALT_DAY``."""

    def buy(tx: str, wallet: str, at: str = "2024-01-01T00:00:00Z", amount: int = 100_000_000_000, **fields: Any) -> dict[str, Any]:
        return projected(tx, wallet, "inbound", amount, at, kind="buy", **fields)

    def swap(tx: str, wallet: str = "A", at: str = NEU_DAY, amount: int = 10_000_000_000, **fields: Any) -> tuple[dict[str, Any], dict[str, Any]]:
        out_row = projected(f"{tx}-out", wallet, "outbound", amount, at, kind="sell", rate="60000", side="out", **fields)
        in_row = projected(f"{tx}-in", wallet, "inbound", 600_000_000_000_000, at, kind="buy", rate="1", asset="USDT", side="in")
        return out_row, in_row

    books: dict[str, SwapBook] = {}

    # The out leg is also a move's out leg, so normalize makes no event of
    # it and no quarantine names it: the pair is blocked as
    # ``swap_leg_unavailable``, and the second pass books the move at its in
    # leg.
    move_out = projected("move-out", "A", "outbound", 10_000_000_000, NEU_DAY, anchor="move", side="out", kind="transfer")
    move_in = projected("move-in", "B", "inbound", 10_000_000_000, NEU_DAY, anchor="move", side="in", kind="transfer")
    usdt_in = projected("usdt-in", "B", "inbound", 600_000_000_000_000, NEU_DAY, kind="buy", rate="1", asset="USDT")
    books["swap-leg-is-a-move-leg"] = (
        [buy("buy", "A"), move_out, move_in, usdt_in],
        [move(move_out, move_in, group_id=None)],
        [_cross(move_out, usdt_in)],
    )
    # A pair with a leg outside the projection is skipped: its rows book
    # as a sale and a purchase.
    out_row, in_row = swap("swap")
    books["pair-leg-outside-the-projection"] = (
        [buy("buy", "A"), out_row, in_row],
        [],
        [_cross(out_row, in_row, in_id="custody-tax:not-projected")],
    )
    # A pair without an id links its rows by ``<out>-><in>``.
    out_row, in_row = swap("swap")
    books["pair-without-an-id"] = ([buy("buy", "A"), out_row, in_row], [], [_cross(out_row, in_row, pair_id=None)])
    # A leg's reason is found by the row, its transaction, then the pair's
    # transaction for the side: here only the pair names a transaction the
    # first pass quarantined (an unpriced purchase after the swap).
    out_row, in_row = swap("swap")
    books["reason-by-the-pairs-transaction"] = (
        [buy("buy", "A"), out_row, in_row, buy("other", "A", at="2024-03-01T00:00:00Z", rate=None)],
        [],
        [_cross(out_row, in_row, out_transaction_id="other")],
    )
    # A transaction's first reason wins: an unpriced slice of the swap-out's
    # transaction (normalize's ``missing_spot_price``) taints the lot state,
    # so the first pass's gate then refuses the swap-out itself
    # (``basis_provenance_incomplete``).
    out_row, in_row = swap("swap")
    slice_row = projected("swap-slice", "A", "outbound", 1_000_000_000, NEU_DAY, kind="sell", rate=None, anchor="swap-out", side="slice")
    books["first-reason-of-a-transaction"] = ([buy("buy", "A"), out_row, slice_row, in_row], [], [_cross(out_row, in_row)])
    # An unconfirmed in leg is held: no event, its hold's reason.
    pending = chain_txid("pending-swap-in")
    usdt_out = projected("usdt-out", "A", "outbound", 300_000_000_000_000, NEU_DAY, kind="sell", rate="1", asset="USDT")
    btc_in = projected(
        "btc-in", "A", "inbound", 5_000_000_000, NEU_DAY, kind="buy", rate="60000",
        external_id=pending, raw_json=graph_json(pending, confirmed=False),
    )
    books["held-in-leg"] = (
        [buy("usdt-buy", "A", amount=300_000_000_000_000, rate="1", asset="USDT"), usdt_out, btc_in],
        [],
        [_cross(usdt_out, btc_in)],
    )
    # One row in two pairs: the later pair's link replaces the earlier's on
    # the shared row, which keeps its place; the earlier in leg keeps its
    # own link (the lot engine then finds it unpaired).
    out_row, first_in = swap("swap")
    second_in = projected("second-in", "A", "inbound", 300_000_000_000_000, NEU_DAY, kind="buy", rate="1", asset="USDT")
    books["one-row-in-two-pairs"] = (
        [buy("buy", "A"), out_row, first_in, second_in],
        [],
        [_cross(out_row, first_in, pair_id="first"), _cross(out_row, second_in, pair_id="second")],
    )
    # The link is chosen on the first pass's regime, Neu while a swap-in
    # funds the wallet with Neu; the second pass leaves that swap-in out
    # (its pair is blocked: nothing to swap from), so the linked sale is
    # Alt there and stays linked.
    usdt_short = projected("usdt-short", "A", "outbound", 300_000_000_000_000, "2024-01-01T00:00:00Z", kind="sell", rate="1", asset="USDT")
    neu_in = projected("neu-in", "A", "inbound", 50_000_000_000, "2024-01-01T00:00:00Z", kind="buy", rate="40000")
    out_row, in_row = swap("swap", at="2024-06-01T00:00:00Z", amount=30_000_000_000)
    books["a-link-keeps-its-first-pass-regime"] = (
        [buy("alt", "A", at=ALT_DAY), usdt_short, neu_in, out_row, in_row],
        [],
        [_cross(usdt_short, neu_in, pair_id="blocked"), _cross(out_row, in_row, pair_id="kept")],
    )
    # A pair without an id whose out row is a direct-payout row: the
    # fallback id starts like a direct payout's, so only the out leg is
    # quarantined, and nothing more is withheld (by the pair's own id).
    payout_out = {**projected("payout-out", "A", "outbound", 200_000_000_000, NEU_DAY, kind="sell", rate="60000"), "id": "direct-payout:edge:source"}
    payout_in = projected("payout-in", "A", "inbound", 600_000_000_000_000, NEU_DAY, kind="buy", rate="1", asset="USDT")
    books["fallback-id-of-a-direct-payout-row"] = (
        [buy("buy", "A"), payout_out, payout_in],
        [],
        [_cross(payout_out, payout_in, pair_id=None)],
    )
    return books


SWAP_EDGES = sorted(_swap_edges())


def _swap_edge(name: str) -> tuple[Mapping[str, Any], TaxEngineLedgerInputs]:
    rows, pairs, cross = _swap_edges()[name]
    return profile("at", "moving_average_at"), direct(rows, pairs, cross_asset_pairs=cross)


@pytest.mark.parametrize("name", SWAP_EDGES)
def test_swap_edge_books_prepare_as_the_oracle(name: str) -> None:
    _assert_same_stages(*_swap_edge(name))


def _pair_fates(inputs: TaxEngineLedgerInputs, selection: Document) -> dict[str, str]:
    """Each carrying-value pair's fate in a selection, by its link id:
    ``linked``, ``blocked: <reason code>``, ``skipped`` (an Alt swap), or
    ``outside`` (a leg is not a row)."""
    projection = inputs.finalized_tax_projection
    row_ids = {str(row["id"]) for row in projection.rows}
    linked = {link for _row, link in selection["links"]}
    blocked = {
        json.loads(item["detail_json"])["at_swap_link"]: json.loads(item["detail_json"])["reason_code"]
        for item in selection["quarantines"]
    }
    fates: dict[str, str] = {}
    for pair in projection.cross_asset_pairs:
        if pair.get("policy") != CARRYING_VALUE:
            continue
        link = str(pair.get("pair_id") or f"{pair['out_id']}->{pair['in_id']}")
        if str(pair["out_id"]) not in row_ids or str(pair["in_id"]) not in row_ids:
            fates[link] = "outside"
        elif link in blocked:
            fates[link] = f"blocked: {blocked[link]}"
        else:
            fates[link] = "linked" if link in linked else "skipped"
    return fates


def test_the_swap_edge_books_reach_their_shapes() -> None:
    """Each swap edge book shows on the oracle what its name says."""
    documents = {name: (_swap_edge(name)[1], _oracle_document(*_swap_edge(name))) for name in SWAP_EDGES}

    def fates(name: str) -> dict[str, str]:
        inputs, document = documents[name]
        return _pair_fates(inputs, document["swap_links"])

    def events(name: str) -> dict[str, tuple[Any, Any]]:
        inputs, document = documents[name]
        anchors = {str(row["id"]): str(row["journal_transaction_id"]) for row in inputs.finalized_tax_projection.rows}
        return {
            anchors[str(event["transaction_id"])]: (event["at_regime"], event["at_swap_link"])
            for asset in document["assets"]
            for event in asset["normalized"]["events"]
        }

    for name in SWAP_EDGES:
        assert documents[name][1]["error"] is None, name
    assert fates("swap-leg-is-a-move-leg") == {"swap": "blocked: swap_leg_unavailable"}
    [move_stage] = [asset for asset in documents["swap-leg-is-a-move-leg"][1]["assets"] if asset["normalized"]["transfers"]]
    assert len(move_stage["prepared"]["intra_audit"]) == 1
    assert fates("pair-leg-outside-the-projection") == {"swap": "outside"}
    assert documents["pair-leg-outside-the-projection"][1]["swap_links"] == {"links": [], "blocked": [], "quarantines": []}
    [link] = fates("pair-without-an-id")
    assert link.endswith(":out->" + link.split("->")[1]) and fates("pair-without-an-id") == {link: "linked"}
    assert fates("reason-by-the-pairs-transaction") == {"swap": "blocked: missing_spot_price"}
    assert fates("first-reason-of-a-transaction") == {"swap": "blocked: missing_spot_price"}
    run = _OracleRun()
    compare.run_on_oracle_backend(
        compare.LEGACY_NATIVE_BACKEND,
        lambda: _oracle_passes(*_swap_edge("first-reason-of-a-transaction"), run),
    )
    [first_btc] = [prepared for normalized, prepared in run.passes[0] if normalized.asset == "BTC"]
    assert [(item["transaction_id"], item["reason"]) for item in first_btc.quarantines] == [
        ("swap-out", "missing_spot_price"),
        ("swap-out", "basis_provenance_incomplete"),
    ]
    assert fates("held-in-leg") == {"swap": "blocked: pending_onchain_confirmation"}
    two = documents["one-row-in-two-pairs"][1]["swap_links"]
    assert [link for _row, link in two["links"]] == ["second", "first", "second"]
    assert fates("a-link-keeps-its-first-pass-regime") == {"blocked": "blocked: insufficient_lots", "kept": "linked"}
    assert events("a-link-keeps-its-first-pass-regime")["swap-out"] == ("alt", "kept")
    payout = documents["fallback-id-of-a-direct-payout-row"][1]["swap_links"]
    assert [json.loads(item["detail_json"]).get("out_amount") for item in payout["quarantines"]] == [None]
    assert payout["blocked"] == sorted(["direct-payout:edge:source", _swap_edges()["fallback-id-of-a-direct-payout-row"][0][2]["id"]])


def test_the_swap_books_reach_the_selection_branches() -> None:
    """The swap goldens and seeded books compared above link Neu swaps,
    skip Alt ones, block pairs for a normalize reason and for a gate
    reason, block a direct payout (withholding its target-asset disposal),
    and run a second pass whose gates differ from the first's (the pass-B
    overdraw reaches ``missing_cost_basis``)."""
    fates: set[str] = set()
    withheld_payouts = 0
    books = [_golden(name) for name in _swap_case_names()]
    books += [(book.profile, book.inputs) for book in (random_inputs(seed, "at") for seed in _random_swap_seeds())]
    for book_profile, inputs in books:
        document = _oracle_document(book_profile, inputs)
        if document["swap_links"] is None:
            continue
        fates |= set(_pair_fates(inputs, document["swap_links"]).values())
        withheld_payouts += any(item.endswith(":out") for item in document["swap_links"]["blocked"])
    assert {"linked", "skipped", "blocked: missing_spot_price", "blocked: insufficient_lots"} <= fates, fates
    assert withheld_payouts >= 2
    overdraw = _oracle_document(*_golden("at-pass-b-overdraw-sale"))
    assert "missing_cost_basis" in {item[1] for item in _gate_quarantines(overdraw)}


def _carry_edges() -> dict[str, tuple[Mapping[str, Any], TaxEngineLedgerInputs]]:
    """Generic rail-carry books the goldens and seeds may not reach: a pair
    whose out row is a purchase (no fragment basis and no quarantine, and an
    unpriced in leg that does not explain it), a fee-only source (no amount
    to price it by), and a coarse in leg the carry prices exactly under the
    review opt-in."""

    def row(name: str, wallet: str, direction: str, at: str, amount: int, **fields: Any) -> dict[str, Any]:
        item = projected(name, wallet, direction, amount, at, kind="buy" if direction == "inbound" else "sell", **fields)
        item["id"] = f"custody-tax:{name}"
        return item

    def rail(out_row: Mapping[str, Any], in_row: Mapping[str, Any], pair_id: Optional[str]) -> dict[str, Any]:
        return _cross(out_row, in_row, pair_id)

    btc = 100_000_000_000
    books: dict[str, tuple[Mapping[str, Any], TaxEngineLedgerInputs]] = {}
    buy = row("buy", "C", "inbound", "2024-01-01T00:00:00Z", btc)
    peg_in = row("peg-in", "L", "inbound", "2024-05-02T00:00:00Z", 50_000_000_000, rate=None, asset="LBTC")
    sell = row("sell", "C", "outbound", "2024-07-01T00:00:00Z", 10_000_000_000, rate="70000")
    books["source-without-basis"] = (
        profile("generic", "FIFO"),
        direct([buy, peg_in, sell], cross_asset_pairs=[rail(buy, peg_in, None)]),
    )
    buy = row("buy", "C", "inbound", "2024-01-01T00:00:00Z", btc)
    fee = row("peg-out", "C", "outbound", "2024-05-01T00:00:00Z", 0, fee=1_000_000, rate="60000")
    peg_in = row("peg-in", "L", "inbound", "2024-05-02T00:00:00Z", 500_000, rate="60000", asset="LBTC")
    books["fee-only-source"] = (
        profile("generic", "FIFO"),
        direct([buy, fee, peg_in], cross_asset_pairs=[rail(fee, peg_in, "rail-1")]),
    )
    buy = row("buy", "C", "inbound", "2024-01-01T00:00:00Z", btc)
    peg_out = row("peg-out", "C", "outbound", "2024-05-01T00:00:00Z", 60_000_000_000, rate="60000")
    peg_in = row(
        "peg-in",
        "L",
        "inbound",
        "2024-05-02T00:00:00Z",
        59_900_000_000,
        rate="60000",
        asset="LBTC",
        pricing_quality=pricing.QUALITY_COARSE_FALLBACK,
    )
    books["coarse-in-leg-reviewed"] = (
        profile("generic", "FIFO", require_coarse_review=True),
        direct([buy, peg_out, peg_in], cross_asset_pairs=[rail(peg_out, peg_in, "rail-1")]),
    )
    return books


CARRY_EDGES = sorted(_carry_edges())


@pytest.mark.parametrize("name", CARRY_EDGES)
def test_carry_edge_books_prepare_as_the_oracle(name: str) -> None:
    _assert_same_stages(*_carry_edges()[name])


def test_the_carry_edge_books_reach_their_shapes() -> None:
    documents = {name: _oracle_document(*book) for name, book in _carry_edges().items()}
    unavailable = documents["source-without-basis"]["rail_carry"]
    assert unavailable["carried"] == [] and unavailable["passes"] == 2
    assert [
        (item["transaction_id"], json.loads(item["detail_json"])["reason_code"]) for item in unavailable["quarantines"]
    ] == [("buy", "source_basis_unavailable"), ("peg-in", "source_basis_unavailable")]
    fee_only = documents["fee-only-source"]["rail_carry"]
    assert fee_only["carried"] == [
        ("custody-tax:peg-out", "0.4", None),
        ("custody-tax:peg-in", "0.4", "80000"),
    ]
    reviewed = documents["coarse-in-leg-reviewed"]
    assert [item[0] for item in reviewed["rail_carry"]["carried"]] == ["custody-tax:peg-out", "custody-tax:peg-in"]
    # Reviewed on the carry's first pass, booked at the carried basis on the
    # final one.
    run = _OracleRun()
    compare.run_on_oracle_backend(
        compare.LEGACY_NATIVE_BACKEND, lambda: _oracle_passes(*_carry_edges()["coarse-in-leg-reviewed"], run)
    )
    first_liquid = [normalized for normalized, _prepared in run.passes[0] if normalized.asset == "LBTC"]
    assert [item["reason"] for item in first_liquid[0].quarantines] == ["pricing_review_required"]
    final_liquid = [asset for asset in reviewed["assets"] if asset["normalized"]["asset"] == "LBTC"]
    assert final_liquid[0]["normalized"]["quarantines"] == []
    assert [event["fiat_value"] for event in final_liquid[0]["normalized"]["events"]] == [Decimal("24000")]


def test_the_mirror_is_the_adapters_own_call() -> None:
    """The whole oracle, on the legacy-native backend, normalizes and walks
    each asset exactly as ``_oracle_passes`` does, in the rail carry's
    passes and the later ones, as far as it gets."""
    recorded: list[list[Any]] = []
    walk = oracle_adapter._prepare_rp2_asset_input

    def recording(walk_profile: Mapping[str, Any], normalized: Any, configuration: Any) -> Any:
        recorded.append([normalized, None])
        prepared = walk(walk_profile, normalized, configuration)
        recorded[-1][1] = prepared
        return prepared

    names = _generic_case_names() + _carry_case_names() + _austrian_case_names() + _swap_case_names()
    books = [_golden(name) for name in names]
    for country in ("generic", "at"):
        for seed in _random_seeds(country):
            book = random_inputs(seed, country)
            books.append((book.profile, book.inputs))
    for seed in _random_swap_seeds():
        book = random_inputs(seed, "at")
        books.append((book.profile, book.inputs))
    for seed in _random_carry_seeds():
        book = random_inputs(seed, "generic")
        books.append((book.profile, book.inputs))
    complete = second_passes = carry_passes = 0
    for book_profile, inputs in books:
        recorded.clear()
        with patch.object(oracle_adapter, "_prepare_rp2_asset_input", recording):
            _, error = compare.run_oracle(book_profile, inputs, backend=compare.LEGACY_NATIVE_BACKEND)
        run = _OracleRun()
        compare.run_on_oracle_backend(compare.LEGACY_NATIVE_BACKEND, lambda: _oracle_passes(book_profile, inputs, run))
        mirror = [stage for one_pass in run.passes for stage in one_pass]
        second_passes += run.selection is not None and bool(run.selection[0] or run.selection[1])
        carry_passes += run.carry[1] if run.carry is not None else 0
        projection = inputs.finalized_tax_projection
        # Rows only: a rewrite the carry made in either run is not one.
        position = {id(row): index for index, row in enumerate(projection.rows)}

        def stages(items: list[list[Any]]) -> list[Document]:
            return [
                {
                    "normalized": _oracle_asset(position, normalized),
                    "prepared": None if prepared is None else _oracle_prepared(normalized, prepared),
                }
                for normalized, prepared in items
            ]

        # Each prepare pass normalizes and walks each asset once, until
        # anything fails: the rail carry's passes first, then the final
        # pass, and an Austrian book whose selection links or blocks a pair
        # runs a second pass.
        assert len(recorded) <= len(mirror)
        if error is None:
            assert len(recorded) == len(mirror)
            complete += 1
        assert compare.value_differences(stages(mirror[: len(recorded)]), stages(recorded)) == []
    assert complete >= 600
    assert second_passes > RANDOM_BOOKS // 2
    assert carry_passes > 2 * RANDOM_CARRY_BOOKS

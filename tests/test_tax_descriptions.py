"""Journal descriptions and entry types of the tax ledger.

An event's description is its persisted journal text: Austrian books prefix
it with the regime markers in a fixed order (``at_regime``,
``at_regime_basis``, ``at_pool``, ``at_swap_link``), and ``exit_tax`` and
the UI read them back. The end-to-end cases run ``build_ledger_state``.
``MarkerGrammarTest`` pins the grammar on the frozen oracle's composer until
the engine's ``describe.rs`` takes these cases over (plan 20, phase 4).
"""

import unittest
from dataclasses import dataclass
from decimal import Decimal
from typing import Optional

import pytest

from kassiber.core.engines import GenericRP2TaxEngine
from kassiber.errors import AppError
from tests.tax_ledger_generator import finalized, pair, profile, row
from tests.tax_oracle.adapter import _compose_event_notes

MOVING_AVERAGE_AT = profile("at", "moving_average_at")


def _ledger(book_profile, rows, pairs=()):
    return GenericRP2TaxEngine(book_profile).build_ledger_state(
        finalized(book_profile, rows, pairs)
    )


def _descriptions(state):
    return {
        (entry["transaction_id"], entry["entry_type"]): entry["description"]
        for entry in state.entries
    }


@dataclass
class _EventStub:
    description: str = ""
    at_regime: Optional[str] = None
    at_regime_basis: Optional[str] = None
    cost_basis_pool_id: Optional[str] = None
    at_swap_link: Optional[str] = None


class MarkerGrammarTest(unittest.TestCase):
    def test_single_regime_marker(self):
        event = _EventStub(at_regime="neu", description="Bought BTC")
        self.assertEqual(_compose_event_notes(event), "at_regime=neu Bought BTC")

    def test_multi_marker(self):
        event = _EventStub(
            at_regime="neu",
            cost_basis_pool_id="wallet-1",
            at_swap_link="swap-42",
            description="Swapped LBTC for BTC",
        )
        self.assertEqual(
            _compose_event_notes(event),
            "at_regime=neu at_pool=wallet-1 at_swap_link=swap-42 Swapped LBTC for BTC",
        )

    def test_regime_basis_follows_the_regime(self):
        event = _EventStub(
            at_regime="neu",
            at_regime_basis="wahlrecht",
            cost_basis_pool_id="global",
            description="Sell",
        )
        self.assertEqual(
            _compose_event_notes(event),
            "at_regime=neu at_regime_basis=wahlrecht at_pool=default Sell",
        )

    def test_description_only(self):
        event = _EventStub(description="Regular buy")
        self.assertEqual(_compose_event_notes(event), "Regular buy")

    def test_empty_description_with_markers(self):
        event = _EventStub(
            at_regime="alt", cost_basis_pool_id="wallet-2", description=""
        )
        self.assertEqual(_compose_event_notes(event), "at_regime=alt at_pool=wallet-2")

    def test_no_markers_no_description(self):
        self.assertEqual(_compose_event_notes(_EventStub()), "")

    def test_empty_swap_link_is_not_emitted(self):
        # A bare `at_swap_link=` token is never emitted.
        event = _EventStub(at_swap_link="", description="Buy")
        self.assertEqual(_compose_event_notes(event), "Buy")


class DescriptionTest(unittest.TestCase):
    def test_austrian_markers_compose_in_a_fixed_order(self):
        # A Neu swap out of a wallet that also holds Alt coins: the disposal
        # carries every marker, an election, the pool, and the swap link.
        state = _ledger(
            MOVING_AVERAGE_AT,
            [
                row("buy-alt", "C", "inbound", 20_000_000_000, "2020-06-01T00:00:00Z", rate="8000", kind="buy"),
                row("buy-neu", "C", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="40000", kind="buy"),
                row("swap-out", "C", "outbound", 50_000_000_000, "2024-06-01T00:00:00Z", fee=1_000_000, rate="60000", kind="sell"),
                row("swap-in", "C", "inbound", 30_000_000_000_000, "2024-06-01T00:00:00Z", rate="1", kind="buy", asset="USDT"),
            ],
            [pair("swap-1", "swap-out", "swap-in")],
        )

        self.assertEqual(
            _descriptions(state),
            {
                ("buy-alt", "acquisition"): "at_regime=alt at_pool=default buy-alt via Exchange",
                ("buy-neu", "acquisition"): "at_regime=neu at_pool=default buy-neu via Exchange",
                ("swap-out", "disposal"): (
                    "at_regime=neu at_regime_basis=wahlrecht at_pool=default "
                    "at_swap_link=swap-1 swap-out via Exchange"
                ),
                ("swap-in", "acquisition"): (
                    "at_regime=neu at_pool=default at_swap_link=swap-1 swap-in via Exchange"
                ),
            },
        )

    def test_austrian_move_fee_carries_its_regime_then_the_pool(self):
        state = _ledger(
            MOVING_AVERAGE_AT,
            [
                row("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="40000", kind="buy"),
                row("move-out", "A", "outbound", 50_000_000_000, "2024-06-01T00:00:00Z", fee=1_000_000, rate="60000"),
                row("move-in", "B", "inbound", 50_000_000_000, "2024-06-01T00:10:00Z", rate="60000"),
            ],
            [pair("pair-1", "move-out", "move-in")],
        )

        descriptions = _descriptions(state)
        self.assertEqual(
            descriptions[("move-out", "transfer_fee")],
            "at_regime=neu at_pool=default move-out via Cold",
        )
        leg = "at_regime=neu at_alt_out=0 at_alt_in=0 at_pool=default Transfer Cold -> Hot"
        self.assertEqual(descriptions[("move-out", "transfer_out")], leg)
        self.assertEqual(descriptions[("move-in", "transfer_in")], leg)

    def test_generic_descriptions_carry_no_markers(self):
        book = profile("generic", "FIFO")
        state = _ledger(
            book,
            [
                row("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00Z", rate="40000", kind="buy"),
                row("move-out", "A", "outbound", 50_000_000_000, "2024-06-01T00:00:00Z", fee=1_000_000, rate="60000"),
                row("move-in", "B", "inbound", 50_000_000_000, "2024-06-01T00:10:00Z", rate="60000"),
                row("sell", "B", "outbound", 1_000_000_000, "2024-07-01T00:00:00Z", rate="70000", kind="sell"),
                row("mined", "A", "inbound", 1_000_000_000, "2024-07-02T00:00:00Z", rate="70000", kind="mining"),
            ],
            [pair("pair-1", "move-out", "move-in")],
        )

        self.assertEqual(
            _descriptions(state),
            {
                ("buy", "acquisition"): "buy via Cold",
                ("mined", "acquisition"): "mined via Cold",
                ("mined", "income"): "mined via Cold",
                ("move-out", "transfer_fee"): "move-out via Cold",
                ("move-out", "transfer_out"): "Transfer Cold -> Hot",
                ("move-in", "transfer_in"): "Transfer Cold -> Hot",
                ("sell", "disposal"): "sell via Hot",
            },
        )

    def test_wallet_cost_basis_pools_fail_closed(self):
        # Per-wallet pools have no reviewed cross-pool contract, so a profile
        # asking for them stops the build instead of booking global pools.
        for country, method in (("at", "moving_average_at"), ("generic", "FIFO")):
            book = profile(country, method, cost_basis_pool_scope="wallet")
            with self.subTest(country=country):
                with self.assertRaises(AppError) as raised:
                    _ledger(
                        book,
                        [row("buy", "A", "inbound", 100_000_000, "2025-06-01T00:00:00Z", kind="buy")],
                    )
                self.assertEqual(raised.exception.code, "validation")
                self.assertEqual(
                    str(raised.exception), "Unsupported cost-basis pool scope 'wallet'"
                )

    def test_inbound_kinds_book_by_their_type(self):
        # Earn kinds book an acquisition and an income; Austria splits income
        # into general and capital yield. Wages and an explicit plain-deposit
        # override book a plain acquisition; a blank override still overrides,
        # an empty one does not.
        cases = (
            ("buy", None, None),
            ("wages", None, None),
            ("income", None, "income_general"),
            ("mining-reward", "", "income_general"),
            ("deposit", " Lending Interest ", "income_capital_yield"),
            ("staking", None, "income_capital_yield"),
            ("income", "wages", None),
            ("income", "deposit", None),
            ("income", " ", None),
        )
        rows = [
            row(
                f"in-{index}", "A", "inbound", 1_000_000_000,
                f"2025-01-{index + 1:02d}T00:00:00Z", rate="50000",
                kind=kind, kind_override=override,
            )
            for index, (kind, override, _) in enumerate(cases)
        ]
        state = _ledger(MOVING_AVERAGE_AT, rows)

        self.assertEqual(state.quarantines, [])
        for index, (kind, override, category) in enumerate(cases):
            with self.subTest(kind=kind, override=override):
                entries = [
                    entry for entry in state.entries if entry["transaction_id"] == f"in-{index}"
                ]
                expected = ["acquisition"] if category is None else ["acquisition", "income"]
                self.assertEqual(sorted(entry["entry_type"] for entry in entries), expected)
                incomes = [entry for entry in entries if entry["entry_type"] == "income"]
                self.assertEqual([entry["at_category"] for entry in incomes], [category] if category else [])


@pytest.mark.parametrize(
    "book",
    [profile("generic", "MOVING_AVERAGE"), MOVING_AVERAGE_AT],
    ids=["generic", "at"],
)
def test_pool_holdings_use_report_basis_without_rewriting_acquisition_journal(book):
    state = _ledger(
        book,
        [
            row("buy-1", "A", "inbound", 100_000_000_000, "2023-01-01T00:00:00Z", rate="100", kind="buy"),
            row("buy-2", "A", "inbound", 100_000_000_000, "2023-02-01T00:00:00Z", rate="300", kind="buy"),
            row("sell", "A", "outbound", 50_000_000_000, "2023-03-01T00:00:00Z", rate="400", kind="sell"),
        ],
    )

    assert sorted(
        entry["fiat_value"] for entry in state.entries if entry["entry_type"] == "acquisition"
    ) == [Decimal("100"), Decimal("300")]
    assert [
        entry["cost_basis"] for entry in state.entries if entry["entry_type"] == "disposal"
    ] == [Decimal("100")]
    for holdings in (state.account_holdings, state.wallet_holdings):
        assert list(holdings.values()) == [
            {"quantity": Decimal("1.5"), "cost_basis": Decimal("300")}
        ]


if __name__ == "__main__":
    unittest.main()

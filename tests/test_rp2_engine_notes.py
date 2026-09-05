import unittest
from collections import defaultdict
from dataclasses import dataclass
from datetime import date
from decimal import Decimal
from typing import Optional

import pytest

from kassiber.core.engines.rp2 import (
    _compose_event_notes,
    _compose_transfer_notes,
    _rp2_in_transaction_type,
    _accumulate_asset_holdings,
    _append_rp2_journal_entries,
    _build_rp2_accounting_engine,
    _rp2_configuration,
)
from kassiber.errors import AppError


@dataclass
class _EventStub:
    description: str = ""
    at_regime: Optional[str] = None
    cost_basis_pool_id: Optional[str] = None
    at_swap_link: Optional[str] = None


@dataclass
class _TransferStub:
    description: str = ""
    from_cost_basis_pool_id: Optional[str] = None
    to_cost_basis_pool_id: Optional[str] = None


@dataclass
class _KindEventStub:
    raw_row: dict[str, object]


class ComposeEventNotesTest(unittest.TestCase):
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
        # Empty swap-link id would be rejected by rp2 (RP2ValueError); the adapter
        # must never emit a bare `at_swap_link=` token.
        event = _EventStub(at_swap_link="", description="Buy")
        self.assertEqual(_compose_event_notes(event), "Buy")

    def test_transfer_only_pool_marker(self):
        transfer = _TransferStub(
            from_cost_basis_pool_id="wallet-3",
            to_cost_basis_pool_id="wallet-3",
            description="Wallet move",
        )
        self.assertEqual(
            _compose_transfer_notes(transfer), "at_pool=wallet-3 Wallet move"
        )

    def test_transfer_description_only(self):
        transfer = _TransferStub(description="Wallet move")
        self.assertEqual(_compose_transfer_notes(transfer), "Wallet move")

    def test_generic_adapter_does_not_emit_austrian_pool_marker(self):
        event = _EventStub(cost_basis_pool_id="global", description="Buy")
        self.assertEqual(
            _compose_event_notes(event, include_austrian_markers=False), "Buy"
        )

    def test_austrian_cross_pool_transfer_fails_closed_without_wire_contract(self):
        transfer = _TransferStub(
            from_cost_basis_pool_id="wallet-1",
            to_cost_basis_pool_id="wallet-2",
            description="Wallet move",
        )
        with self.assertRaises(AppError) as raised:
            _compose_transfer_notes(transfer)
        self.assertEqual(raised.exception.code, "unsupported")

    def test_wages_provenance_books_as_plain_acquisition(self):
        self.assertEqual(
            _rp2_in_transaction_type(_KindEventStub(raw_row={"kind": "wages"})),
            "BUY",
        )


def _pool_result(country, buys, sales, from_date=date.min, to_date=date.max):
    from rp2.configuration import Configuration
    from rp2.in_transaction import InTransaction
    from rp2.input_data import InputData
    from rp2.out_transaction import OutTransaction
    from rp2.rp2_decimal import RP2Decimal
    from rp2.tax_engine import compute_tax
    from rp2.transaction_set import TransactionSet

    profile = {
        "id": "profile",
        "workspace_id": "workspace",
        "label": "Holder",
        "tax_country": country,
        "fiat_currency": "EUR",
        "gains_algorithm": "moving_average_at" if country == "at" else "moving_average",
        "tax_long_term_days": 365,
    }
    with _rp2_configuration(profile, ["Wallet"], ["BTC"]) as original:
        configuration = Configuration(
            original.configuration_path, original.country, from_date, to_date
        )
        in_set = TransactionSet(configuration, "IN", "BTC")
        out_set = TransactionSet(configuration, "OUT", "BTC")
        for row, (timestamp, amount, price) in enumerate(buys, 1):
            in_set.add_entry(
                InTransaction(
                    configuration,
                    timestamp,
                    "BTC",
                    "Wallet",
                    "Holder",
                    "BUY",
                    RP2Decimal(price),
                    RP2Decimal(amount),
                    fiat_fee=RP2Decimal("0"),
                    row=row,
                )
            )
        for row, (timestamp, amount, price) in enumerate(sales, len(buys) + 1):
            out_set.add_entry(
                OutTransaction(
                    configuration,
                    timestamp,
                    "BTC",
                    "Wallet",
                    "Holder",
                    "SELL",
                    RP2Decimal(price),
                    RP2Decimal(amount),
                    RP2Decimal("0"),
                    row=row,
                )
            )
        inputs = InputData(
            "BTC",
            in_set,
            out_set,
            TransactionSet(configuration, "INTRA", "BTC"),
            from_date=from_date,
            to_date=to_date,
        )
        computed = compute_tax(
            configuration, _build_rp2_accounting_engine(profile), inputs
        )
    wallet = {
        "id": "wallet",
        "label": "Wallet",
        "wallet_account_id": "account",
        "account_code": "treasury",
        "account_label": "Treasury",
    }
    entries = []
    _append_rp2_journal_entries(entries, computed, {"Wallet": wallet}, profile, {}, [])
    holdings = defaultdict(
        lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")}
    )
    wallet_holdings = defaultdict(
        lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")}
    )
    _accumulate_asset_holdings(
        holdings, wallet_holdings, computed, computed.balance_set, {"Wallet": wallet}
    )
    return entries, list(holdings.values())


@pytest.mark.parametrize("country", ["generic", "at"])
def test_pool_holdings_use_report_basis_without_rewriting_acquisition_journal(country):
    entries, holdings = _pool_result(
        country,
        [
            ("2023-01-01T00:00:00Z", "1", "100"),
            ("2023-02-01T00:00:00Z", "1", "300"),
        ],
        [("2023-03-01T00:00:00Z", "1", "400")],
    )
    assert sorted(
        row["fiat_value"] for row in entries if row["entry_type"] == "acquisition"
    ) == [Decimal("100"), Decimal("300")]
    assert sum(
        row["cost_basis"] for row in entries if row["entry_type"] == "disposal"
    ) == Decimal("200")
    assert holdings == [{"quantity": Decimal("1"), "cost_basis": Decimal("200")}]


@pytest.mark.parametrize("country", ["generic", "at"])
@pytest.mark.parametrize("from_date", [date.min, date(2023, 2, 15)])
def test_pool_holdings_cutoff_excludes_future_buys_and_includes_prewindow_sales(
    country, from_date
):
    _, holdings = _pool_result(
        country,
        [
            ("2023-01-01T00:00:00Z", "1", "100"),
            ("2023-03-01T00:00:00Z", "1", "300"),
        ],
        [("2023-02-01T00:00:00Z", "0.5", "400")],
        from_date,
        date(2023, 2, 28),
    )
    assert holdings == [{"quantity": Decimal("0.5"), "cost_basis": Decimal("50")}]


if __name__ == "__main__":
    unittest.main()

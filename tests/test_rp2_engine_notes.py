import unittest
from dataclasses import dataclass
from typing import Optional

from kassiber.core.engines.rp2 import (
    _compose_event_notes,
    _compose_transfer_notes,
    _rp2_in_transaction_type,
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
        self.assertEqual(_compose_transfer_notes(transfer), "at_pool=wallet-3 Wallet move")

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


if __name__ == "__main__":
    unittest.main()

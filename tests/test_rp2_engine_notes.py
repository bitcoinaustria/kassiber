import unittest
from dataclasses import dataclass, field
from decimal import Decimal
from typing import Optional

from kassiber.core.engines.rp2 import _compose_event_notes, _compose_transfer_notes


@dataclass
class _EventStub:
    description: str = ""
    at_regime: Optional[str] = None
    at_pool: Optional[str] = None
    at_swap_link: Optional[str] = None
    carried_basis_fiat: Optional[Decimal] = None


@dataclass
class _TransferStub:
    description: str = ""
    at_pool: Optional[str] = None


class ComposeEventNotesTest(unittest.TestCase):
    def test_single_regime_marker(self):
        event = _EventStub(at_regime="neu", description="Bought BTC")
        self.assertEqual(_compose_event_notes(event), "at_regime=neu Bought BTC")

    def test_multi_marker(self):
        event = _EventStub(
            at_regime="neu",
            at_pool="wallet-1",
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
        event = _EventStub(at_regime="alt", at_pool="wallet-2", description="")
        self.assertEqual(_compose_event_notes(event), "at_regime=alt at_pool=wallet-2")

    def test_no_markers_no_description(self):
        self.assertEqual(_compose_event_notes(_EventStub()), "")

    def test_empty_swap_link_is_not_emitted(self):
        # Empty swap-link id would be rejected by rp2 (RP2ValueError); the adapter
        # must never emit a bare `at_swap_link=` token.
        event = _EventStub(at_swap_link="", description="Buy")
        self.assertEqual(_compose_event_notes(event), "Buy")

    def test_transfer_only_pool_marker(self):
        transfer = _TransferStub(at_pool="wallet-3", description="Wallet move")
        self.assertEqual(_compose_transfer_notes(transfer), "at_pool=wallet-3 Wallet move")

    def test_transfer_description_only(self):
        transfer = _TransferStub(description="Wallet move")
        self.assertEqual(_compose_transfer_notes(transfer), "Wallet move")


class CarriedBasisOverrideTest(unittest.TestCase):
    def test_in_transaction_uses_carried_basis_when_present(self):
        # Verify via the call-site code: when event.carried_basis_fiat is set,
        # fiat_in_with_fee uses that value instead of fiat_value.
        from kassiber.core.tax_events import NormalizedTaxEvent

        event = NormalizedTaxEvent(
            transaction_id="tx-1",
            asset="BTC",
            occurred_at="2026-01-01T00:00:00Z",
            wallet_id="w1",
            wallet_label="W1",
            direction="inbound",
            amount=Decimal("1"),
            fee=Decimal("0"),
            spot_price=Decimal("60000"),
            fiat_value=Decimal("60000"),
            description="Swap in",
            raw_row={},
            at_regime="neu",
            at_pool="w1",
            at_swap_link="swap-1",
            carried_basis_fiat=Decimal("42000"),
        )
        self.assertEqual(event.carried_basis_fiat, Decimal("42000"))
        self.assertEqual(event.fiat_value, Decimal("60000"))

    def test_carried_basis_fallback_to_fiat_value(self):
        from kassiber.core.tax_events import NormalizedTaxEvent

        event = NormalizedTaxEvent(
            transaction_id="tx-2",
            asset="BTC",
            occurred_at="2026-01-01T00:00:00Z",
            wallet_id="w1",
            wallet_label="W1",
            direction="inbound",
            amount=Decimal("1"),
            fee=Decimal("0"),
            spot_price=Decimal("60000"),
            fiat_value=Decimal("60000"),
            description="Buy",
            raw_row={},
        )
        self.assertIsNone(event.carried_basis_fiat)


if __name__ == "__main__":
    unittest.main()

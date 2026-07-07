"""Lightning channel-lifecycle netting.

A channel funding tx moves the operator's own BTC into a 2-of-2 they co-control
(not a disposal); a close returns it (not an acquisition). When a separately
synced on-chain wallet records those txs, the tax engine must recognize them via
the derived channel roles and suppress them as non-events — the same machinery
loan collateral lock/release uses.
"""

from __future__ import annotations

import unittest
from decimal import Decimal

from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines.rp2 import GenericRP2TaxEngine
from kassiber.core.lightning.channel_lifecycle import channel_role_map
from kassiber.core.loans import CHANNEL_CLOSE, CHANNEL_OPEN

FUNDING_TXID = "aa" * 32
CLOSING_TXID = "bb" * 32
ONE_BTC = 100_000_000_000  # msat
FEE_MSAT = 100_000_000  # 0.001 BTC


class ChannelRoleMapTest(unittest.TestCase):
    def test_funding_outbound_maps_to_channel_open(self) -> None:
        channels = [{"funding_txid": FUNDING_TXID, "closing_txid": CLOSING_TXID}]
        txs = [
            {"id": "open", "external_id": FUNDING_TXID, "direction": "outbound"},
            {"id": "close", "external_id": CLOSING_TXID, "direction": "inbound"},
            {"id": "other", "external_id": "cc" * 32, "direction": "outbound"},
        ]
        roles = channel_role_map(channels, txs)
        self.assertEqual(roles, {"open": CHANNEL_OPEN, "close": CHANNEL_CLOSE})

    def test_funding_outpoint_form_and_case_folding(self) -> None:
        channels = [{"funding_outpoint": f"{FUNDING_TXID}:1"}]
        txs = [{"id": "open", "external_id": FUNDING_TXID.upper(), "direction": "outbound"}]
        self.assertEqual(channel_role_map(channels, txs), {"open": CHANNEL_OPEN})

    def test_direction_guard(self) -> None:
        # A change/receive leg that shares the funding txid but is inbound must
        # NOT be labeled a channel open.
        channels = [{"funding_txid": FUNDING_TXID}]
        txs = [{"id": "change", "external_id": FUNDING_TXID, "direction": "inbound"}]
        self.assertEqual(channel_role_map(channels, txs), {})

    def test_no_channels_is_empty(self) -> None:
        txs = [{"id": "x", "external_id": FUNDING_TXID, "direction": "outbound"}]
        self.assertEqual(channel_role_map([], txs), {})


def _profile():
    return {
        "id": "p1",
        "workspace_id": "w1",
        "label": "BA",
        "tax_country": "at",
        "gains_algorithm": "moving_average_at",
    }


def _wallet_refs():
    return {
        "onchain": {
            "id": "onchain",
            "label": "onchain",
            "wallet_account_id": "acct-1",
            "account_code": "A",
            "account_label": "Account A",
        },
    }


def _row(tx_id, direction, amount_msat, occurred_at, *, external_id=None, fee=0):
    return {
        "id": tx_id,
        "wallet_id": "onchain",
        "wallet_label": "onchain",
        "asset": "BTC",
        "direction": direction,
        "amount": amount_msat,
        "fee": fee,
        "fiat_rate": 50_000,
        "fiat_value": None,
        "kind": "deposit" if direction == "inbound" else "withdrawal",
        "description": tx_id,
        "note": None,
        "external_id": external_id or tx_id,
        "occurred_at": occurred_at,
    }


def _run(rows, channel_roles):
    return GenericRP2TaxEngine(_profile()).build_ledger_state(
        TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=_wallet_refs(),
            manual_pair_records=[],
            channel_roles=channel_roles,
        )
    )


def _btc_quantity(result):
    return sum(
        totals["quantity"]
        for key, totals in result.account_holdings.items()
        if key[3] == "BTC"
    )


def _has_disposal(result):
    return any(
        Decimal(str(row.get("quantity", 0) or 0)) != 0 for row in result.tax_summary
    )


class ChannelLifecycleEngineTest(unittest.TestCase):
    def test_channel_open_is_suppressed_not_a_disposal(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
        ]
        # Baseline: with no channel role, the funding outbound books as a disposal.
        baseline = _run(rows, {})
        self.assertEqual(_btc_quantity(baseline), Decimal("0"))
        self.assertTrue(_has_disposal(baseline))

        # Recognized as a channel open: suppressed — the coin stays owned.
        roles = channel_role_map([{"funding_txid": FUNDING_TXID}], rows)
        tagged = _run(rows, roles)
        self.assertEqual(_btc_quantity(tagged), Decimal("1"))
        self.assertFalse(_has_disposal(tagged))

    def test_channel_open_books_miner_fee_without_disposing_principal(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC + FEE_MSAT, "2025-05-01T00:00:00Z"),
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
                fee=FEE_MSAT,
            ),
        ]
        roles = channel_role_map([{"funding_txid": FUNDING_TXID}], rows)
        result = _run(rows, roles)

        # The channel capacity stays owned, but the L1 miner fee left the pool.
        self.assertEqual(_btc_quantity(result), Decimal("1"))
        self.assertFalse(_has_disposal(result))
        fee_entries = [row for row in result.entries if row["entry_type"] == "fee"]
        self.assertEqual(len(fee_entries), 1)
        self.assertEqual(Decimal(str(fee_entries[0]["quantity"])), Decimal("-0.001"))

    def test_open_then_close_round_trip_is_net_zero(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("close", "inbound", ONE_BTC, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
        ]
        roles = channel_role_map(
            [{"funding_txid": FUNDING_TXID, "closing_txid": CLOSING_TXID}], rows
        )
        result = _run(rows, roles)
        # Both suppressed: exactly the original 1 BTC, no disposal, no second
        # acquisition at market price.
        self.assertEqual(_btc_quantity(result), Decimal("1"))
        self.assertFalse(_has_disposal(result))


if __name__ == "__main__":  # pragma: no cover
    unittest.main()

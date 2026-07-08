"""Lightning channel-lifecycle netting.

A channel funding tx moves the operator's own BTC into a 2-of-2 they co-control
(not a disposal); a close returns it (not an acquisition). When a separately
synced on-chain wallet records those txs, the tax engine must recognize them via
the derived channel roles and suppress them as non-events — the same machinery
loan collateral lock/release uses.
"""

from __future__ import annotations

import json
import unittest
from decimal import Decimal

from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines.rp2 import GenericRP2TaxEngine
from kassiber.core.lightning.channel_lifecycle import channel_role_map, channel_transfer_pairs
from kassiber.core.loans import (
    CHANNEL_CLOSE,
    CHANNEL_CLOSE_MISMATCH,
    CHANNEL_OPEN,
    CHANNEL_OPEN_MISMATCH,
)

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

    def test_funding_with_external_payment_flags_mismatch(self) -> None:
        # The recorded outflow (channel + external payment) clearly exceeds the
        # funded balance: suppressing the whole row would untax the payment.
        channels = [
            {"funding_txid": FUNDING_TXID, "funding_amount_msat": 100_000_000_000}
        ]
        txs = [
            {
                "id": "open",
                "external_id": FUNDING_TXID,
                "direction": "outbound",
                "amount": 130_000_000_000,  # 0.3 BTC beyond the channel
                "fee": 500_000,
            }
        ]
        self.assertEqual(
            channel_role_map(channels, txs), {"open": CHANNEL_OPEN_MISMATCH}
        )

    def test_batched_open_sums_funded_amounts_per_txid(self) -> None:
        # multifundchannel: one funding tx opens N channels — the recorded
        # outflow equals the SUM of the per-channel funded amounts, so
        # first-wins capture would false-positive the mismatch guard.
        channels = [
            {"funding_txid": FUNDING_TXID, "funding_amount_msat": 60_000_000_000},
            {"funding_txid": FUNDING_TXID, "funding_amount_msat": 40_000_000_000},
        ]
        txs = [
            {
                "id": "open",
                "external_id": FUNDING_TXID,
                "direction": "outbound",
                "amount": 100_000_000_000,
                "fee": 500_000,
            }
        ]
        self.assertEqual(channel_role_map(channels, txs), {"open": CHANNEL_OPEN})

    def test_funding_amount_within_tolerance_still_opens(self) -> None:
        channels = [
            {"funding_txid": FUNDING_TXID, "funding_amount_msat": 100_000_000_000}
        ]
        txs = [
            {
                "id": "open",
                "external_id": FUNDING_TXID,
                "direction": "outbound",
                "amount": 100_000_000_000,
                "fee": 500_000,
            }
        ]
        self.assertEqual(channel_role_map(channels, txs), {"open": CHANNEL_OPEN})


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
        "node": {
            "id": "node",
            "label": "node",
            "wallet_account_id": "acct-node",
            "account_code": "LN",
            "account_label": "Lightning",
        },
    }


def _row(
    tx_id,
    direction,
    amount_msat,
    occurred_at,
    *,
    external_id=None,
    fee=0,
    wallet_id="onchain",
):
    wallet_ref = _wallet_refs()[wallet_id]
    return {
        "id": tx_id,
        "wallet_id": wallet_id,
        "wallet_label": wallet_ref["label"],
        "wallet_account_id": wallet_ref["wallet_account_id"],
        "account_code": wallet_ref["account_code"],
        "account_label": wallet_ref["account_label"],
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


def _run(rows, channel_roles, channel_pairs=()):
    return GenericRP2TaxEngine(_profile()).build_ledger_state(
        TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=_wallet_refs(),
            manual_pair_records=[],
            channel_roles=channel_roles,
            channel_transfer_pairs=channel_pairs,
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

    def test_austrian_channel_open_fee_uses_alt_lot_when_only_alt_is_available(
        self,
    ) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC + FEE_MSAT, "2021-02-01T00:00:00Z"),
            _row(
                "fund",
                "outbound",
                ONE_BTC,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
                fee=FEE_MSAT,
            ),
            _row(
                "sell",
                "outbound",
                ONE_BTC // 2,
                "2025-06-02T00:00:00Z",
                external_id="cc" * 32,
            ),
        ]
        roles = channel_role_map([{"funding_txid": FUNDING_TXID}], rows)
        result = _run(rows, roles)

        self.assertEqual(result.quarantines, [])
        self.assertEqual(_btc_quantity(result), Decimal("0.5"))
        fee_entries = [row for row in result.entries if row["entry_type"] == "fee"]
        self.assertEqual(len(fee_entries), 1)
        self.assertEqual(Decimal(str(fee_entries[0]["quantity"])), Decimal("-0.001"))

    def test_channel_open_pair_credits_node_wallet_capacity(self) -> None:
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
            _row(
                "node-pay",
                "outbound",
                ONE_BTC // 2,
                "2025-06-02T00:00:00Z",
                wallet_id="node",
            ),
        ]
        channel_rows = [{"funding_txid": FUNDING_TXID, "wallet_id": "node"}]
        result = _run(
            rows,
            channel_role_map(channel_rows, rows),
            channel_transfer_pairs(channel_rows, rows, _wallet_refs()),
        )

        self.assertEqual(result.quarantines, [])
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities.get("onchain", Decimal("0")), Decimal("0"))
        self.assertEqual(wallet_quantities["node"], Decimal("0.5"))
        transfer_audit = [
            row
            for row in result.intra_audit
            if row.get("pairing_source") == "channel_lifecycle"
        ]
        self.assertEqual(len(transfer_audit), 1)
        self.assertEqual(transfer_audit[0]["from_wallet_id"], "onchain")
        self.assertEqual(transfer_audit[0]["to_wallet_id"], "node")

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

    def test_channel_pairs_move_capacity_back_on_close(self) -> None:
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("close", "inbound", ONE_BTC, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
            }
        ]
        result = _run(
            rows,
            channel_role_map(channel_rows, rows),
            channel_transfer_pairs(channel_rows, rows, _wallet_refs()),
        )

        self.assertEqual(result.quarantines, [])
        self.assertFalse(_has_disposal(result))
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities["onchain"], Decimal("1"))
        self.assertEqual(wallet_quantities.get("node", Decimal("0")), Decimal("0"))


    def test_force_close_sweep_round_trip_is_net_zero(self) -> None:
        # A force-close pays the wallet via a separate timelocked SWEEP tx: its
        # own txid never equals the recorded closing txid, but its inputs spend
        # the commitment tx. Without the vin match the open stays suppressed
        # while the sweep books a fresh market-priced acquisition — channel
        # capacity double-counted plus a phantom basis reset.
        sweep_txid = "dd" * 32
        sweep_raw = json.dumps(
            {"txid": sweep_txid, "vin": [{"txid": CLOSING_TXID, "vout": 0}]}
        )
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("sweep", "inbound", ONE_BTC, "2025-08-01T00:00:00Z", external_id=sweep_txid),
        ]
        rows[2]["raw_json"] = sweep_raw
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
            }
        ]
        roles = channel_role_map(channel_rows, rows)
        self.assertEqual(roles["sweep"], CHANNEL_CLOSE)
        result = _run(
            rows,
            roles,
            channel_transfer_pairs(channel_rows, rows, _wallet_refs()),
        )

        self.assertEqual(result.quarantines, [])
        self.assertFalse(_has_disposal(result))
        self.assertEqual(_btc_quantity(result), Decimal("1"))
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities["onchain"], Decimal("1"))
        self.assertEqual(wallet_quantities.get("node", Decimal("0")), Decimal("0"))


    def test_close_fee_gap_books_as_move_fee_disposal(self) -> None:
        # The settled channel balance at close (bkpr debit) exceeds the
        # on-chain receipt by the commitment fee. Without booking that gap the
        # node wallet keeps a phantom residual forever and the fee is never
        # taxed.
        received = ONE_BTC - 100_000_000  # 0.999 BTC
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("close", "inbound", received, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
            }
        ]
        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())
        close_pair = next(p for p in pairs if p["kind"] == CHANNEL_CLOSE)
        self.assertEqual(int(close_pair["out"]["fee"]), 100_000_000)

        result = _run(rows, channel_role_map(channel_rows, rows), pairs)
        self.assertEqual(result.quarantines, [])
        # The close fee books as a real fee disposal on the MOVE.
        fee_entries = [
            entry for entry in result.entries if entry["entry_type"] == "transfer_fee"
        ]
        self.assertEqual(len(fee_entries), 1)
        self.assertEqual(Decimal(str(fee_entries[0]["quantity"])), Decimal("-0.001"))
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities["onchain"], Decimal("0.999"))
        # No phantom residual stranded in the node wallet.
        self.assertEqual(wallet_quantities.get("node", Decimal("0")), Decimal("0"))


    def test_channel_close_carries_alt_availability_back_to_onchain_wallet(self) -> None:
        received = ONE_BTC - FEE_MSAT
        rows = [
            _row("buy", "inbound", ONE_BTC, "2021-02-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("close", "inbound", received, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
            _row("sell", "outbound", received // 2, "2025-08-01T00:00:00Z"),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
            }
        ]
        result = _run(
            rows,
            channel_role_map(channel_rows, rows),
            channel_transfer_pairs(channel_rows, rows, _wallet_refs()),
        )

        self.assertEqual(result.quarantines, [])
        close_fee = [
            entry
            for entry in result.entries
            if entry["transaction_id"] == "close"
            and entry["entry_type"] == "transfer_fee"
        ]
        self.assertEqual(len(close_fee), 1)
        self.assertIn("at_regime=alt", close_fee[0]["description"])
        sale = next(
            entry for entry in result.entries
            if entry["transaction_id"] == "sell"
            and entry["entry_type"] == "disposal"
        )
        self.assertEqual(sale["at_category"], "alt_taxfree")


    def test_funding_with_external_payment_quarantines_for_split(self) -> None:
        # 1.0 BTC funded into the channel, but the funding tx spent 1.3 BTC —
        # 0.3 went to an external recipient. Suppression would untax the
        # payment; standalone booking would dispose the owned capacity. The
        # engine must quarantine the row for an explicit split review.
        rows = [
            _row("buy", "inbound", 2 * ONE_BTC, "2025-05-01T00:00:00Z"),
            _row(
                "fund",
                "outbound",
                ONE_BTC + 30_000_000_000,
                "2025-06-01T00:00:00Z",
                external_id=FUNDING_TXID,
            ),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "wallet_id": "node",
                "funding_amount_msat": ONE_BTC,
            }
        ]
        result = _run(
            rows,
            channel_role_map(channel_rows, rows),
            channel_transfer_pairs(channel_rows, rows, _wallet_refs()),
        )
        reasons = [q["reason"] for q in result.quarantines]
        self.assertEqual(reasons, ["channel_open_unresolved"])
        # Neither a disposal of the whole outflow nor a capacity MOVE books.
        entry_types = [entry["entry_type"] for entry in result.entries]
        self.assertNotIn("transfer_out", entry_types)
        self.assertFalse(_has_disposal(result))


    def test_implausible_close_gap_quarantines_instead_of_booking_fee(self) -> None:
        # The synthesized close pair clones the receipt row, so the generic
        # transfer-fee implausibility guard (out.amount - in.amount == 0) can
        # never fire for it — a mis-captured close balance (unsynced sweep,
        # HTLC value lost to the peer) would book an UNBOUNDED silent fee.
        # The lifecycle ceiling must quarantine instead.
        received = ONE_BTC - 10_000_000_000  # 0.9 BTC: 10% gap, 10x tolerance
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("close", "inbound", received, "2025-07-01T00:00:00Z", external_id=CLOSING_TXID),
        ]
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
            }
        ]
        roles = channel_role_map(channel_rows, rows)
        self.assertEqual(roles["close"], CHANNEL_CLOSE_MISMATCH)
        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())
        self.assertFalse(any(p["kind"] == CHANNEL_CLOSE for p in pairs))

        result = _run(rows, roles, pairs)
        reasons = [q["reason"] for q in result.quarantines]
        self.assertIn("channel_close_unresolved", reasons)
        # No silent 0.1 BTC "fee" disposal books.
        fee_entries = [
            entry for entry in result.entries if entry["entry_type"] == "transfer_fee"
        ]
        self.assertEqual(fee_entries, [])


class MultiSweepCloseTest(unittest.TestCase):
    def test_multi_sweep_close_books_one_fee_for_the_group(self) -> None:
        # A force close pays back in several legs (to_local sweep + HTLC
        # sweep). The single close fee is balance - SUM(legs); per-leg
        # evaluation would book each other leg's amount as a "fee" once each
        # and double-debit the node wallet.
        sweep_one = json.dumps(
            {"txid": "dd" * 32, "vin": [{"txid": CLOSING_TXID, "vout": 0}]}
        )
        sweep_two = json.dumps(
            {"txid": "ee" * 32, "vin": [{"txid": CLOSING_TXID, "vout": 1}]}
        )
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            _row("sweep-1", "inbound", 60_000_000_000, "2025-08-01T00:00:00Z", external_id="dd" * 32),
            _row("sweep-2", "inbound", 39_900_000_000, "2025-08-02T00:00:00Z", external_id="ee" * 32),
        ]
        rows[2]["raw_json"] = sweep_one
        rows[3]["raw_json"] = sweep_two
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
            }
        ]
        roles = channel_role_map(channel_rows, rows)
        self.assertEqual(roles["sweep-1"], CHANNEL_CLOSE)
        self.assertEqual(roles["sweep-2"], CHANNEL_CLOSE)
        pairs = channel_transfer_pairs(channel_rows, rows, _wallet_refs())
        close_fees = sorted(
            int(p["out"]["fee"]) for p in pairs if p["kind"] == CHANNEL_CLOSE
        )
        # One group fee (0.001 BTC), on one leg only.
        self.assertEqual(close_fees, [0, 100_000_000])

        result = _run(rows, roles, pairs)
        self.assertEqual(result.quarantines, [])
        wallet_quantities = {
            key[1]: totals["quantity"] for key, totals in result.wallet_holdings.items()
        }
        self.assertEqual(wallet_quantities["onchain"], Decimal("0.999"))
        self.assertEqual(wallet_quantities.get("node", Decimal("0")), Decimal("0"))

    def test_vin_match_after_full_recovery_is_not_a_close_leg(self) -> None:
        # The peer's to_remote output pays them directly from the commitment
        # tx; if they later pay US spending that output, our inbound must not
        # be reclassified as a close leg once the close is fully accounted —
        # that would suppress taxable income.
        payout = json.dumps({"txid": "dd" * 32, "vin": [{"txid": CLOSING_TXID, "vout": 0}]})
        peer_payment = json.dumps(
            {"txid": "ee" * 32, "vin": [{"txid": CLOSING_TXID, "vout": 1}]}
        )
        rows = [
            _row("buy", "inbound", ONE_BTC, "2025-05-01T00:00:00Z"),
            _row("fund", "outbound", ONE_BTC, "2025-06-01T00:00:00Z", external_id=FUNDING_TXID),
            # Our full settled balance came back in the first sweep.
            _row("sweep", "inbound", ONE_BTC, "2025-08-01T00:00:00Z", external_id="dd" * 32),
            # Later inbound funded by the peer's swept commitment output.
            _row("peer-pay", "inbound", 30_000_000_000, "2025-09-01T00:00:00Z", external_id="ee" * 32),
        ]
        rows[2]["raw_json"] = payout
        rows[3]["raw_json"] = peer_payment
        channel_rows = [
            {
                "funding_txid": FUNDING_TXID,
                "closing_txid": CLOSING_TXID,
                "wallet_id": "node",
                "close_balance_msat": ONE_BTC,
            }
        ]
        roles = channel_role_map(channel_rows, rows)
        self.assertEqual(roles.get("sweep"), CHANNEL_CLOSE)
        self.assertNotIn("peer-pay", roles)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()

import json
import unittest

from kassiber.core.tax_events import normalize_tax_asset_inputs


def _row(
    tx_id,
    wallet_id,
    direction,
    amount,
    *,
    occurred_at="2026-01-01T00:00:00Z",
    fee=0,
    fiat_rate=None,
    fiat_value=None,
    external_id=None,
):
    return {
        "id": tx_id,
        "wallet_id": wallet_id,
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee,
        "fiat_rate": fiat_rate,
        "fiat_value": fiat_value,
        "kind": "deposit" if direction == "inbound" else "withdrawal",
        "description": tx_id,
        "note": None,
        "external_id": external_id or tx_id,
    }


class NormalizeTaxAssetInputsTest(unittest.TestCase):
    def setUp(self):
        self.profile = {"id": "profile-1", "workspace_id": "workspace-1"}
        self.wallet_refs_by_id = {
            "wallet-a": {"id": "wallet-a", "label": "Wallet A"},
            "wallet-b": {"id": "wallet-b", "label": "Wallet B"},
        }

    def test_happy_path_normalizes_priced_inbound_event(self):
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [
                _row(
                    "tx-1",
                    "wallet-a",
                    "inbound",
                    100_000_000_000,
                    fiat_value=60_000,
                )
            ],
            self.wallet_refs_by_id,
            [],
        )
        self.assertEqual(inputs.asset, "BTC")
        self.assertEqual(inputs.ordered_items, [("event", "tx-1")])
        self.assertEqual(inputs.quarantines, [])
        self.assertFalse(hasattr(inputs, "row_by_id"))
        self.assertEqual(len(inputs.events), 1)
        event = inputs.events[0]
        self.assertEqual(event.wallet_label, "Wallet A")
        self.assertEqual(float(event.amount), 1.0)
        self.assertEqual(float(event.spot_price), 60000.0)
        self.assertEqual(float(event.fiat_value), 60000.0)

    def test_same_asset_transfer_normalizes_without_row_lookup(self):
        out_row = _row(
            "tx-out",
            "wallet-a",
            "outbound",
            50_000_000_000,
            fee=100_000_000,
            fiat_rate=65_000,
            external_id="pair-0",
        )
        in_row = _row(
            "tx-in",
            "wallet-b",
            "inbound",
            50_000_000_000,
            external_id="pair-0",
        )
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [out_row, in_row],
            self.wallet_refs_by_id,
            [{"out": out_row, "in": in_row}],
        )
        self.assertFalse(hasattr(inputs, "row_by_id"))
        self.assertEqual(inputs.events, [])
        self.assertEqual(inputs.quarantines, [])
        self.assertEqual(inputs.ordered_items, [("transfer", "tx-out")])
        self.assertEqual(len(inputs.transfers), 1)
        transfer = inputs.transfers[0]
        self.assertEqual(float(transfer.sent), 0.501)
        self.assertEqual(float(transfer.received), 0.5)
        self.assertEqual(float(transfer.fee), 0.001)
        self.assertEqual(float(transfer.spot_price), 65000.0)

    def test_transfer_mismatch_quarantines_without_normalized_transfer(self):
        out_row = _row(
            "tx-out",
            "wallet-a",
            "outbound",
            50_000_000_000,
            external_id="pair-1",
        )
        in_row = _row(
            "tx-in",
            "wallet-b",
            "inbound",
            60_000_000_000,
            external_id="pair-1",
        )
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [out_row, in_row],
            self.wallet_refs_by_id,
            [{"out": out_row, "in": in_row}],
        )
        self.assertEqual(inputs.transfers, [])
        self.assertEqual(len(inputs.quarantines), 1)
        self.assertEqual(inputs.quarantines[0]["reason"], "transfer_mismatch")
        detail = json.loads(inputs.quarantines[0]["detail_json"])
        self.assertEqual(detail["from_wallet"], "Wallet A")
        self.assertEqual(detail["to_wallet"], "Wallet B")

    def test_transfer_fee_without_spot_price_quarantines(self):
        out_row = _row(
            "tx-out",
            "wallet-a",
            "outbound",
            50_000_000_000,
            fee=100_000_000,
            external_id="pair-2",
        )
        in_row = _row(
            "tx-in",
            "wallet-b",
            "inbound",
            50_000_000_000,
            external_id="pair-2",
        )
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [out_row, in_row],
            self.wallet_refs_by_id,
            [{"out": out_row, "in": in_row}],
        )
        self.assertEqual(inputs.transfers, [])
        self.assertEqual(len(inputs.quarantines), 1)
        self.assertEqual(inputs.quarantines[0]["reason"], "missing_spot_price")
        detail = json.loads(inputs.quarantines[0]["detail_json"])
        self.assertEqual(detail["required_for"], "transfer_fee")

    def test_unsupported_direction_quarantines(self):
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [_row("tx-odd", "wallet-a", "sideways", 100_000_000)],
            self.wallet_refs_by_id,
            [],
        )
        self.assertEqual(inputs.events, [])
        self.assertEqual(len(inputs.quarantines), 1)
        self.assertEqual(inputs.quarantines[0]["reason"], "unsupported_tax_direction")
        detail = json.loads(inputs.quarantines[0]["detail_json"])
        self.assertEqual(detail["direction"], "sideways")


if __name__ == "__main__":
    unittest.main()

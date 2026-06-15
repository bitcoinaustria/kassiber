import json
import unittest

from kassiber.core.engines.rp2 import _apply_cross_asset_splits
from kassiber.core.tax_events import (
    build_tax_quarantine,
    dedupe_quarantines,
    normalize_tax_asset_inputs,
)


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
    raw_json=None,
    privacy_boundary=None,
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
        "privacy_boundary": privacy_boundary,
        "raw_json": raw_json or "{}",
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

    def test_negative_fiat_value_falls_back_to_spot_derived_value(self):
        # A malformed negative fiat_value is truthy, so the old `or` fallback let
        # it through to RP2 and crashed the whole report. It must clamp to the
        # spot-derived value (amount * spot_price) instead.
        row = _row(
            "tx-neg", "wallet-a", "inbound", 100_000_000_000,
            fiat_rate=60_000, fiat_value=-50,
        )
        inputs = normalize_tax_asset_inputs(
            self.profile, "BTC", [row], self.wallet_refs_by_id, [],
        )
        self.assertEqual(inputs.quarantines, [])
        self.assertEqual(len(inputs.events), 1)
        self.assertEqual(float(inputs.events[0].fiat_value), 60000.0)

    def test_implausible_transfer_fee_quarantines(self):
        # Reproduces the id=47 split-peg case: a single outbound (0.04702253)
        # fans out to an owned wallet (0.02750000) AND a Liquid peg, so the
        # 1-out/1-in pairing absorbs the ~0.0195 peg as an implied "fee". That
        # must be quarantined for review, never booked as a transfer fee.
        out_row = _row(
            "tx-out",
            "wallet-a",
            "outbound",
            4_702_253_000,
            fiat_rate=63_255,
            external_id="pair-peg",
        )
        in_row = _row(
            "tx-in",
            "wallet-b",
            "inbound",
            2_750_000_000,
            external_id="pair-peg",
        )
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [out_row, in_row],
            self.wallet_refs_by_id,
            [{"out": out_row, "in": in_row}],
        )
        self.assertEqual(inputs.transfers, [])
        self.assertEqual(inputs.events, [])
        self.assertEqual(len(inputs.quarantines), 1)
        self.assertEqual(
            inputs.quarantines[0]["reason"], "transfer_fee_implausible"
        )
        detail = json.loads(inputs.quarantines[0]["detail_json"])
        self.assertEqual(detail["from_wallet"], "Wallet A")
        self.assertEqual(detail["to_wallet"], "Wallet B")
        self.assertAlmostEqual(detail["implied_fee"], 0.01952253, places=8)
        self.assertGreater(detail["implied_fee"], detail["fee_ceiling"])
        self.assertEqual(detail["required_for"], "transfer_fee_review")

    def test_high_recorded_fee_self_transfer_not_implausible(self):
        # 0.001 BTC moved with a high 0.00005 BTC RECORDED network fee. The full
        # implied fee exceeds the max(1%, 2500 sats) band, but it's entirely the
        # recorded miner fee (out.amount == in.amount, nothing unrecognized left
        # the source), so it must still pair, not quarantine as implausible.
        out_row = _row(
            "tx-out", "wallet-a", "outbound", 100_000_000,
            fee=5_000_000, fiat_rate=65_000, external_id="high-fee",
        )
        in_row = _row(
            "tx-in", "wallet-b", "inbound", 100_000_000, external_id="high-fee",
        )
        inputs = normalize_tax_asset_inputs(
            self.profile, "BTC", [out_row, in_row], self.wallet_refs_by_id,
            [{"out": out_row, "in": in_row}],
        )
        self.assertEqual(inputs.quarantines, [])
        self.assertEqual(len(inputs.transfers), 1)
        self.assertAlmostEqual(float(inputs.transfers[0].fee), 0.00005, places=8)

    def test_transfer_fee_just_under_ceiling_still_pairs(self):
        # A 0.0005 BTC implied fee on a 0.1 BTC transfer is under the
        # max(1%, 2500 sats) = 0.001 BTC ceiling, so it still normalizes as a
        # transfer (guards against over-quarantining genuine network fees).
        out_row = _row(
            "tx-out",
            "wallet-a",
            "outbound",
            10_000_000_000,
            fiat_rate=65_000,
            external_id="pair-ok",
        )
        in_row = _row(
            "tx-in",
            "wallet-b",
            "inbound",
            9_950_000_000,
            external_id="pair-ok",
        )
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [out_row, in_row],
            self.wallet_refs_by_id,
            [{"out": out_row, "in": in_row}],
        )
        self.assertEqual(inputs.quarantines, [])
        self.assertEqual(len(inputs.transfers), 1)
        self.assertAlmostEqual(float(inputs.transfers[0].fee), 0.0005, places=8)

    def test_owned_fanout_quarantines_all_legs(self):
        # One tx fans out from wallet-a to BOTH wallet-b and wallet-c.
        # detect_intra_transfers skips it (not 1-out/1-in); booking each leg
        # standalone would destroy basis, so every leg is quarantined.
        refs = {
            "wallet-a": {"id": "wallet-a", "label": "Wallet A"},
            "wallet-b": {"id": "wallet-b", "label": "Wallet B"},
            "wallet-c": {"id": "wallet-c", "label": "Wallet C"},
        }
        out_row = _row("fan-out", "wallet-a", "outbound", 50_000_000_000,
                       fiat_rate=60_000, external_id="fanout-1")
        in_b = _row("fan-in-b", "wallet-b", "inbound", 30_000_000_000,
                    fiat_rate=60_000, external_id="fanout-1")
        in_c = _row("fan-in-c", "wallet-c", "inbound", 20_000_000_000,
                    fiat_rate=60_000, external_id="fanout-1")
        inputs = normalize_tax_asset_inputs(
            self.profile, "BTC", [out_row, in_b, in_c], refs, [],
        )
        self.assertEqual(inputs.transfers, [])
        self.assertEqual(inputs.events, [])
        self.assertEqual(len(inputs.quarantines), 3)
        self.assertTrue(
            all(q["reason"] == "owned_fanout_unresolved" for q in inputs.quarantines)
        )

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
        # The fee can't be priced, so the whole transfer is quarantined (not
        # emitted as a partial zero-fee MOVE that would leave the un-moved fee
        # quantity double-spendable in the source). Resolved by pricing the fee.
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

    def test_privacy_hop_evidence_quarantines_without_provenance(self):
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [
                _row(
                    "tx-coinjoin",
                    "wallet-a",
                    "outbound",
                    100_000_000,
                    fiat_rate=60_000,
                    privacy_boundary="coinjoin",
                    raw_json=json.dumps(
                        {
                            "source": "wasabi_gethistory",
                            "islikelycoinjoin": True,
                        }
                    ),
                )
            ],
            self.wallet_refs_by_id,
            [],
        )

        self.assertEqual(inputs.events, [])
        self.assertEqual(inputs.transfers, [])
        self.assertEqual(len(inputs.quarantines), 1)
        self.assertEqual(inputs.quarantines[0]["reason"], "privacy_hop_unresolved")
        detail = json.loads(inputs.quarantines[0]["detail_json"])
        self.assertEqual(detail["privacy_hop"], "coinjoin")
        self.assertEqual(detail["privacy_boundary"], "coinjoin")
        self.assertEqual(detail["required_for"], "explicit_user_owned_provenance")

    def test_privacy_hop_evidence_blocks_transfer_pair_inference(self):
        out_row = _row(
            "tx-out",
            "wallet-a",
            "outbound",
            50_000_000_000,
            fee=100_000_000,
            fiat_rate=65_000,
            external_id="pair-privacy",
            privacy_boundary="coinjoin",
        )
        in_row = _row(
            "tx-in",
            "wallet-b",
            "inbound",
            50_000_000_000,
            external_id="pair-privacy",
        )
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [out_row, in_row],
            self.wallet_refs_by_id,
            [{"out": out_row, "in": in_row}],
        )

        self.assertEqual(inputs.events, [])
        self.assertEqual(inputs.transfers, [])
        self.assertEqual(len(inputs.quarantines), 1)
        self.assertEqual(inputs.quarantines[0]["reason"], "privacy_hop_unresolved")
        detail = json.loads(inputs.quarantines[0]["detail_json"])
        self.assertEqual(detail["privacy_boundary"], "coinjoin")
        self.assertEqual(detail["direction"], "transfer")


class BuildTaxQuarantineTest(unittest.TestCase):
    profile = {"id": "p", "workspace_id": "w"}

    def test_uses_real_id_for_synthetic_rows(self):
        # A synthetic engine-only row (e.g. a direct-payout or cross-split leg)
        # must quarantine against its real tx so the journal_quarantines FK to
        # transactions(id) holds — otherwise the whole `journals process` aborts.
        row = {"id": "direct-payout:abc:out", "journal_transaction_id": "real-tx"}
        q = build_tax_quarantine(self.profile, row, "reason", {})
        self.assertEqual(q["transaction_id"], "real-tx")

    def test_uses_own_id_for_real_rows(self):
        q = build_tax_quarantine(self.profile, {"id": "real-tx"}, "reason", {})
        self.assertEqual(q["transaction_id"], "real-tx")


class CrossAssetSplitTest(unittest.TestCase):
    def test_value_only_pricing_materializes_unit_rate(self):
        # A row priced by fiat_value alone (no fiat_rate) must keep a usable price
        # on both split legs (a derived per-unit rate), not become unpriced.
        out_row = {
            "id": "btc-out", "asset": "BTC", "direction": "outbound",
            "amount": 50_000_000_000, "fee": 0,
            "fiat_rate": None, "fiat_rate_exact": None,
            "fiat_value": 3000.0, "fiat_value_exact": "3000",
        }
        in_row = {"id": "lbtc-in", "asset": "LBTC", "direction": "inbound", "amount": 19_800_000_000}
        record = {
            "id": "pair-1", "out_transaction_id": "btc-out",
            "in_transaction_id": "lbtc-in", "out_amount": 20_000_000_000,
        }
        rows, _records, out_map = _apply_cross_asset_splits([out_row, in_row], [record])
        by_id = {r["id"]: r for r in rows}
        # 0.5 BTC priced at 3000 EUR => 6000 EUR/BTC unit rate on both legs.
        self.assertEqual(by_id["btc-out"]["fiat_rate"], "6000")
        self.assertIsNone(by_id["btc-out"]["fiat_value"])
        synthetic = next(r for r in rows if str(r["id"]).startswith("cross-split:"))
        self.assertEqual(synthetic["fiat_rate"], "6000")
        self.assertEqual(synthetic["amount"], 20_000_000_000)
        self.assertEqual(by_id["btc-out"]["amount"], 30_000_000_000)
        self.assertEqual(out_map[synthetic["id"]], "btc-out")


class DedupeQuarantinesTest(unittest.TestCase):
    profile = {"id": "p", "workspace_id": "w"}

    def _q(self, tx_id, reason, detail):
        # build through the real builder so detail_json serialization matches prod
        return build_tax_quarantine(self.profile, {"id": tx_id}, reason, detail)

    def test_distinct_reasons_for_same_tx_merge_into_one_row(self):
        # Two engine legs (e.g. a direct-payout synthetic leg AND another drop)
        # that map back to the same real tx would otherwise collide on
        # journal_quarantines' PRIMARY KEY and abort the whole run. They must
        # collapse to ONE row, with the later reason preserved under detail.
        out = dedupe_quarantines(
            [
                self._q("real-tx", "missing_cost_basis", {"required": 1.0}),
                self._q("real-tx", "basis_provenance_incomplete", {"since": "x"}),
            ]
        )
        self.assertEqual(len(out), 1)
        self.assertEqual(out[0]["transaction_id"], "real-tx")
        self.assertEqual(out[0]["reason"], "missing_cost_basis")
        detail = json.loads(out[0]["detail_json"])
        self.assertEqual(detail["required"], 1.0)
        self.assertEqual(len(detail["additional_reasons"]), 1)
        self.assertEqual(
            detail["additional_reasons"][0]["reason"], "basis_provenance_incomplete"
        )
        self.assertEqual(detail["additional_reasons"][0]["detail"]["since"], "x")

    def test_exact_duplicate_for_same_tx_is_dropped_silently(self):
        out = dedupe_quarantines(
            [
                self._q("real-tx", "missing_cost_basis", {"required": 1.0}),
                self._q("real-tx", "missing_cost_basis", {"required": 1.0}),
            ]
        )
        self.assertEqual(len(out), 1)
        self.assertNotIn("additional_reasons", json.loads(out[0]["detail_json"]))

    def test_distinct_transactions_preserved_in_order(self):
        out = dedupe_quarantines(
            [
                self._q("tx-b", "r1", {}),
                self._q("tx-a", "r2", {}),
            ]
        )
        self.assertEqual([q["transaction_id"] for q in out], ["tx-b", "tx-a"])


if __name__ == "__main__":
    unittest.main()

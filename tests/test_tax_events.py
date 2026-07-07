import json
import unittest

from kassiber.msat import msat_to_btc
from kassiber.core.engines import TaxEngineLedgerInputs, build_tax_engine
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
    amount_includes_fee=False,
    payment_hash=None,
    kind=None,
):
    return {
        "id": tx_id,
        "wallet_id": wallet_id,
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "fee": fee,
        "amount_includes_fee": amount_includes_fee,
        "fiat_rate": fiat_rate,
        "fiat_value": fiat_value,
        "kind": kind or ("deposit" if direction == "inbound" else "withdrawal"),
        "description": tx_id,
        "note": None,
        "external_id": external_id or tx_id,
        "payment_hash": payment_hash,
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

    def test_manual_one_to_many_pairs_group_and_allocate_fee_once(self):
        out_row = _row(
            "premix-out",
            "wallet-a",
            "outbound",
            100_000_000_000,
            fee=1_000_000,
            fiat_rate=65_000,
            external_id="premix-out",
        )
        in_one = _row(
            "postmix-in-1",
            "wallet-b",
            "inbound",
            60_000_000_000,
            external_id="postmix-1",
        )
        in_two = _row(
            "postmix-in-2",
            "wallet-b",
            "inbound",
            39_999_000_000,
            external_id="postmix-2",
        )
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [out_row, in_one, in_two],
            self.wallet_refs_by_id,
            [
                {
                    "out": out_row,
                    "in": in_one,
                    "pair_id": "pair-1",
                    "source": "manual",
                },
                {
                    "out": out_row,
                    "in": in_two,
                    "pair_id": "pair-2",
                    "source": "manual",
                },
            ],
        )
        self.assertEqual(inputs.events, [])
        self.assertEqual(inputs.quarantines, [])
        self.assertEqual(len(inputs.transfers), 2)
        self.assertEqual(
            [item[0] for item in inputs.ordered_items],
            ["transfer", "transfer"],
        )
        self.assertEqual(
            sum(t.sent for t in inputs.transfers),
            msat_to_btc(100_001_000_000),
        )
        self.assertEqual(
            sum(t.received for t in inputs.transfers),
            msat_to_btc(99_999_000_000),
        )
        self.assertEqual(sum(t.fee for t in inputs.transfers), msat_to_btc(2_000_000))
        self.assertTrue(all(t.group_id for t in inputs.transfers))
        self.assertTrue(all(t.transfer_id for t in inputs.transfers))

    def test_reviewed_whirlpool_one_to_many_resolves_privacy_boundary(self):
        out_row = _row(
            "premix-out",
            "wallet-a",
            "outbound",
            100_000_000_000,
            fee=1_000_000,
            fiat_rate=65_000,
            external_id="premix-out",
            privacy_boundary="coinjoin",
        )
        in_one = _row(
            "postmix-in-1",
            "wallet-b",
            "inbound",
            60_000_000_000,
            external_id="postmix-1",
        )
        in_two = _row(
            "toxic-change-in",
            "wallet-b",
            "inbound",
            39_999_000_000,
            external_id="toxic-change",
        )
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [out_row, in_one, in_two],
            self.wallet_refs_by_id,
            [
                {
                    "out": out_row,
                    "in": in_one,
                    "pair_id": "pair-1",
                    "kind": "whirlpool",
                    "source": "manual",
                },
                {
                    "out": out_row,
                    "in": in_two,
                    "pair_id": "pair-2",
                    "kind": "whirlpool",
                    "source": "manual",
                },
            ],
        )

        self.assertEqual(inputs.events, [])
        self.assertEqual(inputs.quarantines, [])
        self.assertEqual(len(inputs.transfers), 2)
        self.assertEqual({t.pairing_source for t in inputs.transfers}, {"manual"})

    def test_manual_multi_pair_implausible_fee_quarantines_entire_group(self):
        out_row = _row(
            "premix-out",
            "wallet-a",
            "outbound",
            100_000_000_000,
            fiat_rate=65_000,
            external_id="premix-out",
        )
        in_one = _row(
            "postmix-in-1",
            "wallet-b",
            "inbound",
            40_000_000_000,
            external_id="postmix-1",
        )
        in_two = _row(
            "postmix-in-2",
            "wallet-b",
            "inbound",
            40_000_000_000,
            external_id="postmix-2",
        )
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [out_row, in_one, in_two],
            self.wallet_refs_by_id,
            [
                {
                    "out": out_row,
                    "in": in_one,
                    "pair_id": "pair-1",
                    "source": "manual",
                },
                {
                    "out": out_row,
                    "in": in_two,
                    "pair_id": "pair-2",
                    "source": "manual",
                },
            ],
        )

        self.assertEqual(inputs.events, [])
        self.assertEqual(inputs.transfers, [])
        self.assertEqual(len(inputs.quarantines), 3)
        self.assertTrue(
            all(q["reason"] == "transfer_fee_implausible" for q in inputs.quarantines)
        )
        detail = json.loads(inputs.quarantines[0]["detail_json"])
        self.assertAlmostEqual(detail["implied_fee"], 0.2, places=8)
        self.assertGreater(detail["implied_fee"], detail["fee_ceiling"])

    def test_manual_many_to_one_pairs_group_and_allocate_destination_once(self):
        out_one = _row(
            "premix-out-1",
            "wallet-a",
            "outbound",
            40_000_000_000,
            fee=500_000,
            fiat_rate=65_000,
            external_id="premix-out-1",
        )
        out_two = _row(
            "premix-out-2",
            "wallet-a",
            "outbound",
            60_000_000_000,
            fee=500_000,
            fiat_rate=65_000,
            external_id="premix-out-2",
        )
        in_row = _row(
            "postmix-in",
            "wallet-b",
            "inbound",
            99_999_000_000,
            external_id="postmix-in",
        )
        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [out_one, out_two, in_row],
            self.wallet_refs_by_id,
            [
                {
                    "out": out_one,
                    "in": in_row,
                    "pair_id": "pair-1",
                    "source": "manual",
                },
                {
                    "out": out_two,
                    "in": in_row,
                    "pair_id": "pair-2",
                    "source": "manual",
                },
            ],
        )
        self.assertEqual(inputs.events, [])
        self.assertEqual(inputs.quarantines, [])
        self.assertEqual(len(inputs.transfers), 2)
        self.assertEqual(
            sum(t.sent for t in inputs.transfers),
            msat_to_btc(100_001_000_000),
        )
        self.assertEqual(
            sum(t.received for t in inputs.transfers),
            msat_to_btc(99_999_000_000),
        )
        self.assertEqual(sum(t.fee for t in inputs.transfers), msat_to_btc(2_000_000))
        self.assertEqual({t.in_transaction_id for t in inputs.transfers}, {"postmix-in"})

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
        # BOTH legs of the unbooked pair are quarantined — the recorded inbound
        # must never be silently dropped (it would later trip insufficient_lots).
        self.assertEqual(len(inputs.quarantines), 2)
        self.assertTrue(
            all(q["reason"] == "transfer_fee_implausible" for q in inputs.quarantines)
        )
        primary = next(
            q for q in inputs.quarantines
            if not json.loads(q["detail_json"]).get("paired_leg")
        )
        partner = next(
            q for q in inputs.quarantines
            if json.loads(q["detail_json"]).get("paired_leg")
        )
        self.assertEqual(primary["transaction_id"], out_row["id"])
        self.assertEqual(partner["transaction_id"], in_row["id"])
        detail = json.loads(primary["detail_json"])
        self.assertEqual(detail["from_wallet"], "Wallet A")
        self.assertEqual(detail["to_wallet"], "Wallet B")
        self.assertAlmostEqual(detail["implied_fee"], 0.01952253, places=8)
        self.assertGreater(detail["implied_fee"], detail["fee_ceiling"])
        self.assertEqual(detail["required_for"], "transfer_fee_review")

    def test_btcpay_fee_inclusive_self_transfer_books_with_correct_fee(self):
        # BTCPay reports the net wallet delta with the miner fee folded into
        # `amount` (fee column 0, amount_includes_fee=1). For a 0.001 BTC move
        # synced as a BTCPay outbound (0.00103 net) paired with an esplora-style
        # inbound (0.001 recipient), the 0.00003 gap IS the miner fee — it must
        # book as a transfer, NOT quarantine transfer_fee_implausible the way the
        # same gap would for a node-backed (recipient-only) outbound.
        out_row = _row(
            "btcpay-out", "wallet-a", "outbound", 103_000_000,
            fiat_rate=65_000, external_id="btcpay-move", amount_includes_fee=True,
        )
        in_row = _row(
            "node-in", "wallet-b", "inbound", 100_000_000, external_id="btcpay-move",
        )
        inputs = normalize_tax_asset_inputs(
            self.profile, "BTC", [out_row, in_row], self.wallet_refs_by_id,
            [{"out": out_row, "in": in_row}],
        )
        self.assertEqual(inputs.quarantines, [])
        self.assertEqual(len(inputs.transfers), 1)
        self.assertAlmostEqual(float(inputs.transfers[0].fee), 0.00003, places=8)

        # Control: the identical amounts WITHOUT the fee-inclusive flag (a
        # node-backed recipient-only outbound) stay quarantined, so the
        # split-peg/unrecognized-outflow guard is not weakened in general.
        out_node = _row(
            "node-out", "wallet-a", "outbound", 103_000_000,
            fiat_rate=65_000, external_id="node-move",
        )
        in_node = _row(
            "node-in-2", "wallet-b", "inbound", 100_000_000, external_id="node-move",
        )
        control = normalize_tax_asset_inputs(
            self.profile, "BTC", [out_node, in_node], self.wallet_refs_by_id,
            [{"out": out_node, "in": in_node}],
        )
        self.assertEqual(control.transfers, [])
        # Both legs quarantined (the in leg is no longer silently dropped).
        self.assertEqual(len(control.quarantines), 2)
        self.assertTrue(
            all(q["reason"] == "transfer_fee_implausible" for q in control.quarantines)
        )

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

    def test_zero_value_inbound_does_not_block_self_transfer(self):
        # A stray 0-value inbound row sharing the txid of a real cold->hot
        # self-transfer must not inflate the inbound count: detect_intra_transfers
        # still pairs the real legs (rather than skipping a non-1-out/1-in group),
        # and _owned_fanout_row_ids does not flip the group into a spurious
        # owned_fanout_unresolved quarantine.
        from kassiber.transfers import detect_intra_transfers

        refs = {
            "wallet-a": {"id": "wallet-a", "label": "Wallet A"},
            "wallet-b": {"id": "wallet-b", "label": "Wallet B"},
            "wallet-c": {"id": "wallet-c", "label": "Wallet C"},
        }
        out_row = _row("move-out", "wallet-a", "outbound", 50_000_000_000,
                       fee=10_000_000, fiat_rate=60_000, external_id="move-1")
        in_row = _row("move-in", "wallet-b", "inbound", 50_000_000_000,
                      fiat_rate=60_000, external_id="move-1")
        zero_in = _row("zero-in", "wallet-c", "inbound", 0,
                       fiat_rate=60_000, external_id="move-1")
        rows = [out_row, in_row, zero_in]

        pairs, matched = detect_intra_transfers(rows)
        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["out"]["id"], "move-out")
        self.assertEqual(pairs[0]["in"]["id"], "move-in")
        self.assertEqual(matched, {"move-out", "move-in"})

        inputs = normalize_tax_asset_inputs(self.profile, "BTC", rows, refs, pairs)
        self.assertEqual(len(inputs.transfers), 1)
        self.assertFalse(
            any(q["reason"] == "owned_fanout_unresolved" for q in inputs.quarantines)
        )

    def test_detect_intra_transfers_pairs_lightning_by_payment_hash(self):
        # An own-node LN payment (LND pays a CLN invoice) shares a payment_hash
        # across two owned wallets but has distinct external_ids, so the txid
        # grouping never pairs it. The payment_hash pass must recognize it as a
        # self-transfer so the inbound is not booked as phantom income.
        from kassiber.transfers import detect_intra_transfers

        payment_hash = "ab" * 32
        out_row = _row(
            "lnd:pay:x", "wallet-lnd", "outbound", 1_000_000_000,
            fee=2_000_000, external_id="lnd:pay:x", payment_hash=payment_hash,
            kind="lnd_pay",
        )
        in_row = _row(
            "cln:income:y", "wallet-cln", "inbound", 1_000_000_000,
            external_id="cln:income:y", payment_hash=payment_hash,
            kind="cln_invoice",
        )
        pairs, matched = detect_intra_transfers([out_row, in_row])
        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["out"]["id"], "lnd:pay:x")
        self.assertEqual(pairs[0]["in"]["id"], "cln:income:y")
        self.assertEqual(matched, {"lnd:pay:x", "cln:income:y"})

    def test_lightning_payment_hash_no_owned_receiver_stays_unpaired(self):
        # A payment to an EXTERNAL node has only an outbound leg; no inbound row
        # shares the hash, so it must NOT pair and stays a real disposal.
        from kassiber.transfers import detect_intra_transfers

        out_row = _row(
            "cln:pay:ext", "wallet-cln", "outbound", 500_000_000,
            fee=1_000_000, external_id="cln:pay:ext", payment_hash="cd" * 32,
            kind="cln_pay",
        )
        pairs, matched = detect_intra_transfers([out_row])
        self.assertEqual(pairs, [])
        self.assertEqual(matched, set())

    def test_lightning_same_wallet_payment_hash_pairs_as_internal_move(self):
        # A circular self-payment through the same owned node has distinct
        # external ids but the same payment hash. Pair it as an internal move so
        # the legs do not become a taxable disposal plus a fresh acquisition.
        from kassiber.transfers import detect_intra_transfers

        payment_hash = "ef" * 32
        out_row = _row(
            "p:out", "wallet-x", "outbound", 100_000_000,
            fee=1_000_000, fiat_rate=60_000,
            external_id="p:out", payment_hash=payment_hash, kind="cln_pay",
        )
        in_row = _row(
            "p:in", "wallet-x", "inbound", 100_000_000,
            external_id="p:in", payment_hash=payment_hash, kind="cln_invoice",
        )
        pairs, matched = detect_intra_transfers([out_row, in_row])
        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["out"]["id"], "p:out")
        self.assertEqual(pairs[0]["in"]["id"], "p:in")
        self.assertEqual(matched, {"p:out", "p:in"})

        inputs = normalize_tax_asset_inputs(
            self.profile,
            "BTC",
            [out_row, in_row],
            {"wallet-x": {"id": "wallet-x", "label": "Node"}},
            pairs,
        )
        self.assertEqual(inputs.events, [])
        self.assertEqual(len(inputs.transfers), 1)
        self.assertEqual(float(inputs.transfers[0].fee), 0.00001)
        self.assertEqual(inputs.quarantines, [])

    def test_detect_intra_transfers_folds_mixed_case_txid(self):
        # A txid recorded uppercase in one wallet and lowercase in another is the
        # same on-chain transaction; the grouping must fold case so the pair is
        # detected rather than split into a phantom disposal + acquisition.
        from kassiber.transfers import detect_intra_transfers

        txid = "cd" * 32
        out_row = _row("o", "wallet-a", "outbound", 50_000_000_000,
                       external_id=txid.upper())
        in_row = _row("i", "wallet-b", "inbound", 50_000_000_000,
                      external_id=txid.lower())
        pairs, matched = detect_intra_transfers([out_row, in_row])
        self.assertEqual(len(pairs), 1)
        self.assertEqual(matched, {"o", "i"})

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
        # Both legs flagged: the recorded inbound is not silently dropped.
        self.assertEqual(len(inputs.quarantines), 2)
        self.assertTrue(
            all(q["reason"] == "transfer_mismatch" for q in inputs.quarantines)
        )
        self.assertEqual(
            {q["transaction_id"] for q in inputs.quarantines}, {"tx-out", "tx-in"}
        )
        primary = next(
            q for q in inputs.quarantines
            if not json.loads(q["detail_json"]).get("paired_leg")
        )
        detail = json.loads(primary["detail_json"])
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
        # Both legs flagged: the recorded inbound is not silently dropped.
        self.assertEqual(len(inputs.quarantines), 2)
        self.assertTrue(
            all(q["reason"] == "missing_spot_price" for q in inputs.quarantines)
        )
        primary = next(
            q for q in inputs.quarantines
            if not json.loads(q["detail_json"]).get("paired_leg")
        )
        detail = json.loads(primary["detail_json"])
        self.assertEqual(detail["required_for"], "transfer_fee")

    def test_derived_transfer_group_blocks_siblings_when_one_leg_needs_review(self):
        refs = {
            "wallet-a": {"id": "wallet-a", "label": "Wallet A"},
            "wallet-b": {"id": "wallet-b", "label": "Wallet B"},
            "wallet-c": {"id": "wallet-c", "label": "Wallet C"},
        }
        profile = {**self.profile, "require_coarse_review": True}
        out_a = _row(
            "tx-out-a",
            "wallet-a",
            "outbound",
            50_000_000_000,
            fee=100_000_000,
            fiat_rate=65_000,
            external_id="grouped",
        )
        out_a["pricing_quality"] = "coarse_fallback"
        out_c = _row(
            "tx-out-c",
            "wallet-c",
            "outbound",
            30_000_000_000,
            fiat_rate=65_000,
            external_id="grouped",
        )
        in_b_from_a = _row(
            "tx-in-b-a",
            "wallet-b",
            "inbound",
            50_000_000_000,
            external_id="grouped",
        )
        in_b_from_c = _row(
            "tx-in-b-c",
            "wallet-b",
            "inbound",
            30_000_000_000,
            external_id="grouped",
        )
        replaced_real_receipt = _row(
            "tx-in-b-real",
            "wallet-b",
            "inbound",
            80_000_000_000,
            external_id="grouped",
        )
        pairs = [
            {
                "out": out_a,
                "in": in_b_from_a,
                "source": "multi_source_consolidation",
                "group_id": "grouped-transfer",
                "group_block_rows": (replaced_real_receipt,),
            },
            {
                "out": out_c,
                "in": in_b_from_c,
                "source": "multi_source_consolidation",
                "group_id": "grouped-transfer",
                "group_block_rows": (replaced_real_receipt,),
            },
        ]

        inputs = normalize_tax_asset_inputs(
            profile,
            "BTC",
            [out_a, out_c, in_b_from_a, in_b_from_c],
            refs,
            pairs,
        )

        self.assertEqual(inputs.transfers, [])
        self.assertEqual(inputs.ordered_items, [])
        reasons_by_id = {q["transaction_id"]: q["reason"] for q in inputs.quarantines}
        self.assertEqual(reasons_by_id["tx-out-a"], "pricing_review_required")
        self.assertEqual(reasons_by_id["tx-in-b-a"], "pricing_review_required")
        self.assertEqual(reasons_by_id["tx-out-c"], "derived_transfer_group_blocked")
        self.assertEqual(reasons_by_id["tx-in-b-c"], "derived_transfer_group_blocked")
        self.assertEqual(reasons_by_id["tx-in-b-real"], "derived_transfer_group_blocked")
        blocked_detail = json.loads(
            next(
                q["detail_json"]
                for q in inputs.quarantines
                if q["transaction_id"] == "tx-out-c"
            )
        )
        self.assertEqual(blocked_detail["transfer_group_id"], "grouped-transfer")
        self.assertEqual(blocked_detail["blocked_by_reason"], "pricing_review_required")

    def test_blocked_synthetic_group_contaminates_by_journal_transaction_id(self):
        refs = {
            "wallet-a": {"id": "wallet-a", "label": "Wallet A"},
            "wallet-b": {"id": "wallet-b", "label": "Wallet B"},
        }
        profile = {**self.profile, "require_coarse_review": True}
        out_row = _row(
            "multi-consol:tx:out:wallet-a",
            "wallet-a",
            "outbound",
            50_000_000_000,
            fee=100_000_000,
            fiat_rate=65_000,
            external_id="multi-consol:tx:out:wallet-a",
        )
        out_row["journal_transaction_id"] = "real-out-a"
        out_row["pricing_quality"] = "coarse_fallback"
        in_row = _row(
            "multi-consol:tx:in:wallet-a",
            "wallet-b",
            "inbound",
            50_000_000_000,
            external_id="multi-consol:tx:in:wallet-a",
        )
        in_row["journal_transaction_id"] = "real-in-b"

        inputs = normalize_tax_asset_inputs(
            profile,
            "BTC",
            [out_row, in_row],
            refs,
            [{"out": out_row, "in": in_row, "group_id": "grouped-transfer"}],
        )

        self.assertEqual(inputs.transfers, [])
        self.assertEqual(inputs.earliest_lot_contamination_at, out_row["occurred_at"])

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

    def test_reviewed_whirlpool_pair_resolves_privacy_boundary(self):
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
            [{"out": out_row, "in": in_row, "kind": "whirlpool"}],
        )

        self.assertEqual(inputs.events, [])
        self.assertEqual(inputs.quarantines, [])
        self.assertEqual(len(inputs.transfers), 1)
        self.assertEqual(inputs.transfers[0].out_transaction_id, "tx-out")
        self.assertEqual(inputs.transfers[0].in_transaction_id, "tx-in")


class LightningPaymentHashEngineTest(unittest.TestCase):
    def test_same_wallet_payment_hash_books_fee_not_sell_buy(self):
        profile = {
            "id": "profile-1",
            "workspace_id": "workspace-1",
            "label": "Default",
            "fiat_currency": "USD",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }
        wallet_refs = {
            "wallet-x": {
                "id": "wallet-x",
                "label": "Node",
                "wallet_account_id": "acct-1",
                "account_code": "treasury",
                "account_label": "Treasury",
            }
        }
        payment_hash = "ef" * 32

        def engine_row(tx_id, direction, amount, external_id, *, fee=0, payment_hash=None):
            return {
                "id": tx_id,
                "workspace_id": "workspace-1",
                "profile_id": "profile-1",
                "wallet_id": "wallet-x",
                "wallet_label": "Node",
                "wallet_account_id": "acct-1",
                "account_code": "treasury",
                "account_label": "Treasury",
                "external_id": external_id,
                "occurred_at": "2026-01-01T00:00:00Z",
                "created_at": "2026-01-01T00:00:00Z",
                "direction": direction,
                "asset": "BTC",
                "amount": amount,
                "fee": fee,
                "fiat_currency": "USD",
                "fiat_rate": 40_000.0,
                "fiat_rate_exact": "40000",
                "fiat_value": None,
                "kind": "ln_pay" if direction == "outbound" else "ln_invoice",
                "description": tx_id,
                "note": None,
                "raw_json": "{}",
                "excluded": 0,
                "payment_hash": payment_hash,
            }

        state = build_tax_engine(profile).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=[
                    engine_row("acq", "inbound", 100_000_000_000, "acq"),
                    engine_row(
                        "ln-pay",
                        "outbound",
                        5_000_000_000,
                        "ln-pay-external",
                        fee=1_000_000,
                        payment_hash=payment_hash,
                    ),
                    engine_row(
                        "ln-invoice",
                        "inbound",
                        5_000_000_000,
                        "ln-invoice-external",
                        payment_hash=payment_hash,
                    ),
                ],
                wallet_refs_by_id=wallet_refs,
                manual_pair_records=[],
            )
        )

        self.assertEqual(state.quarantines, [])
        entry_types = [entry["entry_type"] for entry in state.entries]
        self.assertEqual(entry_types.count("acquisition"), 1)
        self.assertIn("transfer_fee", entry_types)
        self.assertIn("transfer_out", entry_types)
        self.assertIn("transfer_in", entry_types)
        self.assertNotIn("disposal", entry_types)
        self.assertEqual(len(state.intra_audit), 1)
        self.assertEqual(state.intra_audit[0]["from_wallet_label"], "Node")
        self.assertEqual(state.intra_audit[0]["to_wallet_label"], "Node")
        self.assertAlmostEqual(state.intra_audit[0]["crypto_fee"], 0.00001)
        holdings = {
            label: float(totals["quantity"])
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(holdings["Node"], 0.99999)


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


class ClampedZeroSelfSendTest(unittest.TestCase):
    profile = {"id": "profile-1", "workspace_id": "workspace-1"}
    refs = {
        "wallet-a": {"id": "wallet-a", "label": "Wallet A"},
        "wallet-b": {"id": "wallet-b", "label": "Wallet B"},
        "wallet-c": {"id": "wallet-c", "label": "Wallet C"},
        "wallet-d": {"id": "wallet-d", "label": "Wallet D"},
    }

    def test_clamped_zero_guard_fires_even_when_group_has_a_booked_pair(self):
        # Codex review: a txid carrying BOTH a normal self-transfer pair AND a
        # clamped amount=0 outbound with a cross-wallet inbound. The pair must book
        # as a MOVE, and the clamped source + its cross-wallet receipt must STILL be
        # quarantined (the earlier "any leg paired -> skip group" path used to skip
        # the zero-out guard entirely, leaving the receipt a phantom acquisition).
        a_out = _row("a-out", "wallet-a", "outbound", 50_000_000_000, external_id="mixed")
        b_in = _row("b-in", "wallet-b", "inbound", 50_000_000_000, external_id="mixed")
        c_out = _row("c-zero", "wallet-c", "outbound", 0, external_id="mixed")
        d_in = _row("d-in", "wallet-d", "inbound", 30_000_000_000,
                    external_id="mixed", fiat_value=18_000)
        inputs = normalize_tax_asset_inputs(
            self.profile, "BTC", [a_out, b_in, c_out, d_in], self.refs,
            [{"out": a_out, "in": b_in}],
        )
        # The pair booked as a MOVE.
        self.assertEqual(len(inputs.transfers), 1)
        # The clamped source + its cross-wallet receipt are quarantined.
        quar_ids = {q["transaction_id"] for q in inputs.quarantines
                    if q["reason"] == "owned_fanout_unresolved"}
        self.assertEqual(quar_ids, {"c-zero", "d-in"})
        # d-in is NOT booked as a standalone acquisition.
        self.assertNotIn("d-in", [e.transaction_id for e in inputs.events])

    def test_clamped_zero_outbound_with_cross_wallet_inbound_quarantines(self):
        # #9: a coinjoin/payjoin self-send where wallet A's net outflow fell below
        # the miner fee gets its outbound amount clamped to 0; wallet B receives a
        # positive inbound under the same txid. Every positive-amount filter skips
        # A, so without the guard B books a phantom standalone acquisition. The
        # group must instead be quarantined for review.
        a_out = _row("a-cj-out", "wallet-a", "outbound", 0, external_id="cj-tx")
        b_in = _row("b-cj-in", "wallet-b", "inbound", 50_000_000_000,
                    external_id="cj-tx", fiat_value=30_000)
        inputs = normalize_tax_asset_inputs(
            self.profile, "BTC", [a_out, b_in], self.refs, [],
        )
        self.assertTrue(
            any(q["reason"] == "owned_fanout_unresolved" for q in inputs.quarantines)
        )
        # B's inbound is NOT booked as a standalone acquisition.
        self.assertNotIn("b-cj-in", [e.transaction_id for e in inputs.events])

    def test_single_wallet_fee_consolidation_not_quarantined(self):
        # A clamped amount=0 outbound with NO cross-wallet inbound (an ordinary
        # within-wallet fee/consolidation) must NOT be quarantined by the guard.
        a_out = _row("a-fee", "wallet-a", "outbound", 0, fee=2000, external_id="fee-tx")
        inputs = normalize_tax_asset_inputs(
            self.profile, "BTC", [a_out], self.refs, [],
        )
        self.assertEqual(
            [q for q in inputs.quarantines if q["reason"] == "owned_fanout_unresolved"],
            [],
        )


class ConflictPairInteractionTest(unittest.TestCase):
    profile = {"id": "profile-1", "workspace_id": "workspace-1"}
    refs = {
        "wallet-a": {"id": "wallet-a", "label": "Wallet A"},
        "wallet-b": {"id": "wallet-b", "label": "Wallet B"},
    }

    def test_conflict_loser_in_manual_pair_does_not_book_transfer(self):
        # Codex review: a same-asset manual pair whose OUT leg is a shared-prevout
        # conflict loser must NOT book a transfer using the quarantined loser, even
        # though the partner has a different txid (apply_manual_pairs allows that).
        # The caller passes the conflict set (computed over the full asset rows);
        # the loser is quarantined and the pair is suppressed.
        out = _row("loser-out", "wallet-a", "outbound", 50_000_000_000, external_id="loser-tx")
        partner = _row("partner-in", "wallet-b", "inbound", 50_000_000_000,
                       external_id="other-tx", fiat_value=30_000)
        inputs = normalize_tax_asset_inputs(
            self.profile, "BTC", [out, partner], self.refs,
            [{"out": out, "in": partner}],
            conflict_row_ids={"loser-out"},
        )
        self.assertEqual(list(inputs.transfers), [])
        self.assertTrue(
            any(
                q["reason"] == "conflicting_spend" and q["transaction_id"] == "loser-out"
                for q in inputs.quarantines
            )
        )

    def test_passed_conflict_set_overrides_local_detection(self):
        # The conflict_row_ids the caller passes (full-asset-row detection, stable
        # across the two-pass Austrian prep) is honored verbatim — not recomputed
        # from the possibly-reduced rows handed to this call.
        a = _row("a", "wallet-a", "outbound", 10_000_000_000, external_id="tx-a", fiat_value=6000)
        inputs = normalize_tax_asset_inputs(
            self.profile, "BTC", [a], self.refs, [], conflict_row_ids={"a"},
        )
        self.assertTrue(any(q["reason"] == "conflicting_spend" for q in inputs.quarantines))
        self.assertEqual(list(inputs.events), [])  # the loser is not booked


class AustrianSelfTransferRegimeTest(unittest.TestCase):
    AT_PROFILE = {"id": "p", "workspace_id": "ws", "tax_country": "at"}
    REFS = {
        "wallet-a": {"id": "wallet-a", "label": "Wallet A"},
        "wallet-b": {"id": "wallet-b", "label": "Wallet B"},
    }

    def _move_fee_regime(self, neu_acq_id):
        # Alt lot (2020) + Neu acq (2025-02-01) sharing occurred_at with a
        # self-transfer move (2025-02-01) that has a fee. The Neu acq id flips
        # whether it sorts before/after the move on the raw DB key.
        alt = _row("alt", "wallet-a", "inbound", 30_000_000_000,
                   occurred_at="2020-06-01T00:00:00Z", fiat_rate=10_000)
        neu = _row(neu_acq_id, "wallet-a", "inbound", 40_000_000_000,
                   occurred_at="2025-02-01T00:00:00Z", fiat_rate=60_000)
        out_row = _row("zzz-move-out", "wallet-a", "outbound", 50_000_000_000,
                       occurred_at="2025-02-01T00:00:00Z", fee=100_000_000,
                       fiat_rate=60_000, external_id="mv")
        in_row = _row("mv-in", "wallet-b", "inbound", 50_000_000_000,
                      occurred_at="2025-02-01T00:00:00Z", external_id="mv")
        inputs = normalize_tax_asset_inputs(
            self.AT_PROFILE, "BTC", [alt, neu, out_row, in_row], self.REFS,
            [{"out": out_row, "in": in_row}],
        )
        self.assertEqual(len(inputs.transfers), 1)
        return inputs.transfers[0].at_regime

    def test_self_transfer_fee_regime_is_order_independent(self):
        # #4: economically identical books must not differ by the Neu acq's id.
        # The move post-dates the cutoff and Neu inventory exists, so the fee is
        # unambiguously neu — deterministically, not an id artifact.
        self.assertEqual(self._move_fee_regime("aaa-neu"), "neu")
        self.assertEqual(self._move_fee_regime("zzz-neu"), "neu")

    def test_self_transfer_fee_honors_regime_override(self):
        alt = _row("alt", "wallet-a", "inbound", 30_000_000_000,
                   occurred_at="2020-06-01T00:00:00Z", fiat_rate=10_000)
        neu = _row("neu", "wallet-a", "inbound", 40_000_000_000,
                   occurred_at="2025-02-01T00:00:00Z", fiat_rate=60_000)
        out_row = _row("move-out", "wallet-a", "outbound", 50_000_000_000,
                       occurred_at="2025-02-01T00:00:00Z", fee=100_000_000,
                       fiat_rate=60_000, external_id="mv")
        out_row["at_regime_override"] = "alt"
        in_row = _row("mv-in", "wallet-b", "inbound", 50_000_000_000,
                      occurred_at="2025-02-01T00:00:00Z", external_id="mv")
        inputs = normalize_tax_asset_inputs(
            self.AT_PROFILE, "BTC", [alt, neu, out_row, in_row], self.REFS,
            [{"out": out_row, "in": in_row}],
        )

        self.assertEqual(len(inputs.transfers), 1)
        self.assertEqual(inputs.transfers[0].at_regime, "alt")

    def test_conflict_loser_pair_excluded_from_regime_inference(self):
        # Codex review: a conflict-loser leg manually paired to an inbound with
        # another txid must be dropped from regime inference too (not just from
        # booking) — otherwise infer_outbound_regimes treats the partner inbound as
        # a transfer leg and skips its Alt/Neu availability, while the booking-time
        # filter books it standalone, so a later disposal from that wallet is
        # mis-tagged. Here the Neu partner inbound must count, tagging the later
        # sell as neu.
        partner = _row("partner-in", "wallet-b", "inbound", 50_000_000_000,
                       occurred_at="2024-06-01T00:00:00Z", fiat_rate=60000)
        loser = _row("loser-out", "wallet-a", "outbound", 50_000_000_000,
                     occurred_at="2025-01-01T00:00:00Z", fiat_rate=60000, external_id="loser-tx")
        sell = _row("sell-b", "wallet-b", "outbound", 30_000_000_000,
                    occurred_at="2025-06-01T00:00:00Z", fiat_rate=60000, fiat_value=18000)
        inputs = normalize_tax_asset_inputs(
            self.AT_PROFILE, "BTC", [partner, loser, sell], self.REFS,
            [{"out": loser, "in": partner}], conflict_row_ids={"loser-out"},
        )
        by_id = {e.transaction_id: e for e in inputs.events}
        self.assertEqual(by_id["sell-b"].at_regime, "neu")
        self.assertTrue(
            any(q["reason"] == "conflicting_spend" for q in inputs.quarantines)
        )

    def test_samourai_internal_transfer_fee_carries_regime(self):
        # #5: a Whirlpool tx0 (samourai child rows) under AT with mixed Alt/Neu
        # must stamp at_regime on its MOVE fee disposal, or rp2 aborts the whole
        # asset on an ambiguous disposal.
        def _cfg(section):
            return json.dumps({"samourai": {"role": "child", "group_id": "wp", "section": section}})
        alt = _row("alt", "wallet-a", "inbound", 30_000_000_000,
                   occurred_at="2020-06-01T00:00:00Z", fiat_rate=10_000)
        neu = _row("neu", "wallet-a", "inbound", 40_000_000_000,
                   occurred_at="2024-06-01T00:00:00Z", fiat_rate=60_000)
        out_row = _row("wp-out", "wallet-a", "outbound", 50_000_000_000,
                       occurred_at="2025-02-01T00:00:00Z", fee=100_000_000,
                       fiat_rate=60_000, external_id="wptx")
        out_row["config_json"] = _cfg("deposit")
        in_row = _row("wp-in", "wallet-b", "inbound", 49_900_000_000,
                      occurred_at="2025-02-01T00:00:00Z", external_id="wptx")
        in_row["config_json"] = _cfg("premix")
        inputs = normalize_tax_asset_inputs(
            self.AT_PROFILE, "BTC", [alt, neu, out_row, in_row], self.REFS, [],
        )
        self.assertEqual(len(inputs.transfers), 1)
        self.assertIn(inputs.transfers[0].at_regime, ("alt", "neu"))

    def test_samourai_internal_transfer_carries_alt_availability_to_destination(self):
        def _cfg(section):
            return json.dumps({"samourai": {"role": "child", "group_id": "wp", "section": section}})

        alt = _row("alt", "wallet-a", "inbound", 60_000_000_000,
                   occurred_at="2020-06-01T00:00:00Z", fiat_rate=10_000)
        out_row = _row("wp-out", "wallet-a", "outbound", 50_000_000_000,
                       occurred_at="2025-02-01T00:00:00Z", fee=100_000_000,
                       fiat_rate=60_000, external_id="wptx")
        out_row["config_json"] = _cfg("deposit")
        in_row = _row("wp-in", "wallet-b", "inbound", 49_900_000_000,
                      occurred_at="2025-02-01T00:00:00Z", external_id="wptx")
        in_row["config_json"] = _cfg("premix")
        sell = _row("sell", "wallet-b", "outbound", 10_000_000_000,
                    occurred_at="2025-03-01T00:00:00Z", fiat_rate=60_000,
                    fiat_value=6_000)

        inputs = normalize_tax_asset_inputs(
            self.AT_PROFILE, "BTC", [alt, out_row, in_row, sell], self.REFS, [],
        )

        by_id = {event.transaction_id: event for event in inputs.events}
        self.assertEqual(by_id["sell"].at_regime, "alt")


if __name__ == "__main__":
    unittest.main()

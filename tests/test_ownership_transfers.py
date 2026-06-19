"""Pure-function tests for the address-ownership self-transfer deriver.

The deriver reads the on-chain transaction graph (stored esplora ``vin``/
``vout``) and a profile-wide :class:`OwnedIndex` to prove that an outbound's
output paid an address owned by another of the user's wallets. These tests
feed synthetic rows + a hand-built index so they pin the algorithm without
SQLite or a real descriptor scan.

Amount convention mirrors production: row ``amount``/``fee`` are in msat,
esplora ``vout[].value`` is in sats (msat = sats * 1000).
"""

import json
import unittest

from kassiber.core.ownership import OwnedIndex, OwnedMatch
from kassiber.core.ownership_transfers import derive_ownership_transfers


# Arbitrary distinct scriptPubKey hex per wallet — values are opaque to the
# deriver, which only joins them through the index.
SCRIPT = {
    "A": "0014" + "aa" * 20,
    "B": "0014" + "bb" * 20,
    "C": "0014" + "cc" * 20,
    "EXT": "0014" + "ee" * 20,  # external recipient, never in the index
}
SATS = 1000  # msat per sat


def _match(wallet_id, label):
    return OwnedMatch(
        wallet_id=wallet_id,
        wallet_label=label,
        account="",
        chain="bitcoin",
        network="mainnet",
        branch_label="",
        address_index=None,
        derivation_path=None,
        source="derived",
    )


def _index(owned_scripts):
    """``owned_scripts``: {script_hex: (wallet_id, label)}."""
    index = OwnedIndex()
    for script, (wallet_id, label) in owned_scripts.items():
        index.add_script(script, _match(wallet_id, label))
    return index


def _refs(*wallet_ids):
    return {
        wid: {
            "id": wid,
            "label": f"Wallet {wid}",
            "wallet_account_id": f"acct-{wid}",
            "account_code": "treasury",
            "account_label": "Treasury",
        }
        for wid in wallet_ids
    }


def _outbound(*, row_id, wallet_id, amount_sats, fee_sats, txid, input_scripts, outputs):
    """``outputs``: list of (script_hex, value_sats)."""
    vin = [
        {"txid": f"prev-{i}", "vout": i, "prevout": {"scriptpubkey": script}}
        for i, script in enumerate(input_scripts)
    ]
    vout = [
        {"n": n, "scriptpubkey": script, "value": value}
        for n, (script, value) in enumerate(outputs)
    ]
    return {
        "id": row_id,
        "wallet_id": wallet_id,
        "wallet_label": f"Wallet {wallet_id}",
        "direction": "outbound",
        "asset": "BTC",
        "amount": amount_sats * SATS,
        "fee": fee_sats * SATS,
        "external_id": txid,
        "occurred_at": "2026-03-14T17:30:00Z",
        "created_at": "2026-03-14T17:30:00Z",
        "fiat_rate": 40000.0,
        "fiat_rate_exact": "40000",
        "fiat_value": None,
        "raw_json": json.dumps({"txid": txid, "vin": vin, "vout": vout}),
    }


def _inbound(*, row_id, wallet_id, amount_sats, txid, occurred_at="2026-03-14T17:31:00Z"):
    return {
        "id": row_id,
        "wallet_id": wallet_id,
        "wallet_label": f"Wallet {wallet_id}",
        "direction": "inbound",
        "asset": "BTC",
        "amount": amount_sats * SATS,
        "fee": 0,
        "external_id": txid,
        "occurred_at": occurred_at,
        "created_at": occurred_at,
        "raw_json": "{}",
    }


class OwnershipDeriverTests(unittest.TestCase):
    def _run(self, rows, owned_scripts, refs, already=None):
        return derive_ownership_transfers(
            rows,
            index=_index(owned_scripts),
            wallet_refs_by_id=refs,
            already_paired_ids=already or set(),
        )

    def test_one_to_one_mismatched_txid_pairs_existing_row(self):
        # A -> B, both rows recorded but with different external_ids (CSV import).
        # detect_intra_transfers misses this; the deriver proves it by ownership
        # and pairs the existing inbound row (matched on exact value).
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        b_in = _inbound(row_id="b-in", wallet_id="B", amount_sats=50_000_000,
                        txid="provider-xyz")
        result = self._run([out, b_in], {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(len(result.derived_pairs), 1)
        pair = result.derived_pairs[0]
        self.assertEqual(pair["in"]["id"], "b-in")  # real row reused, not synthesized
        self.assertEqual(pair["out"]["amount"], pair["in"]["amount"])
        self.assertEqual(result.dropped_out_ids, {"a-out"})
        self.assertEqual(result.out_row_overrides, {})

    def test_one_to_one_sync_gap_synthesizes_inbound(self):
        # Destination B recorded NO row (never synced). The deriver synthesizes
        # the inbound leg and resolves its wallet ref from wallet_refs_by_id.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        result = self._run([out], {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(len(result.derived_pairs), 1)
        synth_in = result.derived_pairs[0]["in"]
        self.assertTrue(str(synth_in["id"]).startswith("owned-derive:"))
        self.assertEqual(synth_in["wallet_id"], "B")
        self.assertEqual(synth_in["direction"], "inbound")
        self.assertEqual(synth_in["wallet_label"], "Wallet B")
        self.assertIn(synth_in, result.synthetic_rows)
        self.assertEqual(result.dropped_out_ids, {"a-out"})

    def test_sync_gap_without_ref_declines(self):
        # No ref for the destination wallet -> cannot book the MOVE target; the
        # whole tx is left to existing handling rather than guessed.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        result = self._run([out], {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("A"))
        self.assertEqual(result.derived_pairs, [])
        self.assertEqual(result.dropped_out_ids, set())

    def test_fanout_one_to_two_emits_balanced_pairs(self):
        # One spend to two owned wallets (1->N). detect_intra skips it and the
        # pipeline quarantines; the deriver emits one pair per leg, each with
        # out.amount == in.amount (so the implausible-fee guard never trips),
        # and puts the whole network fee on the first leg only.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=80_000_000, fee_sats=2000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000), (SCRIPT["C"], 30_000_000)],
        )
        b_in = _inbound(row_id="b-in", wallet_id="B", amount_sats=50_000_000, txid="real-txid")
        c_in = _inbound(row_id="c-in", wallet_id="C", amount_sats=30_000_000, txid="real-txid")
        result = self._run(
            [out, b_in, c_in],
            {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B"), SCRIPT["C"]: ("C", "C")},
            _refs("B", "C"),
        )
        self.assertEqual(len(result.derived_pairs), 2)
        for pair in result.derived_pairs:
            self.assertEqual(pair["out"]["amount"], pair["in"]["amount"])
        fees = sorted(pair["out"]["fee"] for pair in result.derived_pairs)
        self.assertEqual(fees, [0, 2000 * SATS])  # fee on exactly one leg
        self.assertEqual({p["in"]["id"] for p in result.derived_pairs}, {"b-in", "c-in"})
        self.assertEqual(result.dropped_out_ids, {"a-out"})

    def test_multiple_outputs_to_same_wallet_aggregate_to_one_leg(self):
        # A wallet that receives two outputs in one tx records a single inbound
        # row of their combined value, so the deriver must aggregate per dest.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 20_000_000), (SCRIPT["B"], 30_000_000)],
        )
        b_in = _inbound(row_id="b-in", wallet_id="B", amount_sats=50_000_000, txid="real-txid")
        result = self._run([out, b_in], {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(len(result.derived_pairs), 1)
        self.assertEqual(result.derived_pairs[0]["in"]["id"], "b-in")
        self.assertEqual(result.derived_pairs[0]["out"]["amount"], 50_000_000 * SATS)

    def test_consolidation_multi_source_declined(self):
        # Two owned wallets fund one tx (N->1). Per-wallet sync double-counts the
        # fee, so the amounts are unreliable; the deriver declines and leaves the
        # transaction to the existing fan-out quarantine.
        a_out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"], SCRIPT["B"]],
            outputs=[(SCRIPT["C"], 79_000_000)],
        )
        result = self._run(
            [a_out],
            {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B"), SCRIPT["C"]: ("C", "C")},
            _refs("A", "B", "C"),
        )
        self.assertEqual(result.derived_pairs, [])

    def test_change_and_external_only_not_derived(self):
        # Change back to self + a payment to an external recipient, no owned
        # destination -> ordinary outbound payment, left on the disposal path.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=40_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["A"], 10_000_000), (SCRIPT["EXT"], 40_000_000)],
        )
        result = self._run([out], {SCRIPT["A"]: ("A", "A")}, _refs("A"))
        self.assertEqual(result.derived_pairs, [])
        self.assertEqual(result.out_row_overrides, {})
        self.assertEqual(result.dropped_out_ids, set())

    def test_mixed_owned_leg_and_external_overrides_residual(self):
        # One owned leg (B) + a real external payment. The owned portion becomes
        # a MOVE; the residual stays as a disposal of the source row.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=70_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000), (SCRIPT["EXT"], 20_000_000)],
        )
        result = self._run([out], {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(len(result.derived_pairs), 1)
        self.assertEqual(result.derived_pairs[0]["out"]["amount"], 50_000_000 * SATS)
        self.assertNotIn("a-out", result.dropped_out_ids)
        self.assertIn("a-out", result.out_row_overrides)
        self.assertEqual(result.out_row_overrides["a-out"]["amount"], 20_000_000 * SATS)
        # The whole miner fee rides the MOVE leg; the residual disposal must NOT
        # carry it again, else the fee leaves the source pool twice.
        self.assertEqual(result.derived_pairs[0]["out"]["fee"], 1000 * SATS)
        self.assertEqual(result.out_row_overrides["a-out"]["fee"], 0)
        # Conservation: total booked outflow == source amount + source fee, once.
        total_out = (
            sum(p["out"]["amount"] + p["out"]["fee"] for p in result.derived_pairs)
            + result.out_row_overrides["a-out"]["amount"]
            + result.out_row_overrides["a-out"]["fee"]
        )
        self.assertEqual(total_out, (70_000_000 + 1000) * SATS)

    def test_unrelated_equal_value_deposit_not_cannibalized(self):
        # B has the genuine self-transfer leg (CSV provider id) AND an unrelated
        # deposit of the same value carrying a DIFFERENT real on-chain txid. The
        # deriver must reuse the genuine row, never cannibalize the unrelated one
        # (which would destroy that acquisition's basis).
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        legit = _inbound(row_id="b-legit", wallet_id="B", amount_sats=50_000_000,
                         txid="provider-xyz")
        unrelated = _inbound(row_id="b-unrelated", wallet_id="B", amount_sats=50_000_000,
                             txid="a" * 64)  # a different real on-chain txid
        result = self._run([out, legit, unrelated],
                           {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(len(result.derived_pairs), 1)
        self.assertEqual(result.derived_pairs[0]["in"]["id"], "b-legit")

    def test_ambiguous_equal_value_candidates_synthesize(self):
        # Two same-value B inbounds, both non-txid ids in window -> ambiguous, so
        # the deriver synthesizes rather than guessing which one to consume.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        c1 = _inbound(row_id="b-1", wallet_id="B", amount_sats=50_000_000, txid="prov-1")
        c2 = _inbound(row_id="b-2", wallet_id="B", amount_sats=50_000_000, txid="prov-2")
        result = self._run([out, c1, c2],
                           {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(len(result.derived_pairs), 1)
        self.assertTrue(str(result.derived_pairs[0]["in"]["id"]).startswith("owned-derive:"))

    def test_inbound_asset_mismatch_not_reused(self):
        # A same-integer-amount inbound of a DIFFERENT asset must never be reused
        # for a BTC leg, even if it shares the txid.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        lbtc = _inbound(row_id="b-lbtc", wallet_id="B", amount_sats=50_000_000, txid="real-txid")
        lbtc["asset"] = "LBTC"
        result = self._run([out, lbtc],
                           {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(len(result.derived_pairs), 1)
        self.assertTrue(str(result.derived_pairs[0]["in"]["id"]).startswith("owned-derive:"))

    def test_no_transaction_json_skipped(self):
        # CSV import / Liquid: raw_json has no vin/vout -> nothing to read.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        out["raw_json"] = "{}"
        result = self._run([out], {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(result.derived_pairs, [])

    def test_cross_asset_peg_to_unowned_federation_not_derived(self):
        # BTC peg-in: the output pays a Liquid federation address we do not own,
        # so it is never an owned leg. Pegs stay on the heuristic + review path.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["EXT"], 50_000_000)],  # federation script, unowned
        )
        result = self._run([out], {SCRIPT["A"]: ("A", "A")}, _refs("A"))
        self.assertEqual(result.derived_pairs, [])

    def test_unresolvable_input_declined(self):
        # We watch only the recipient: the spend's inputs are not ours, so we
        # cannot prove this is our outbound. Declined.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["EXT"]],  # foreign input
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        result = self._run([out], {SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(result.derived_pairs, [])

    def test_already_paired_source_skipped(self):
        # A same-txid auto pair or a manual pair already covers this out row.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        b_in = _inbound(row_id="b-in", wallet_id="B", amount_sats=50_000_000, txid="real-txid")
        result = self._run(
            [out, b_in],
            {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")},
            _refs("B"),
            already={"a-out"},
        )
        self.assertEqual(result.derived_pairs, [])

    def test_none_index_no_ops(self):
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        result = derive_ownership_transfers(
            [out], index=None, wallet_refs_by_id=_refs("B"), already_paired_ids=set()
        )
        self.assertEqual(result.derived_pairs, [])


if __name__ == "__main__":
    unittest.main()

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
from kassiber.core.ownership_transfers import (
    derive_ownership_transfers,
    derive_recorded_fanout_transfers,
)


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

    def test_one_to_one_non_txid_candidate_blocks_for_review(self):
        # A -> B, both rows recorded but with different external_ids (CSV import).
        # Without shared txid evidence, an exact provider id could be either the
        # real leg or an unrelated same-amount receipt. Block the source for
        # review instead of cannibalizing or duplicating the inbound.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        b_in = _inbound(row_id="b-in", wallet_id="B", amount_sats=50_000_000,
                        txid="provider-xyz")
        result = self._run([out, b_in], {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(result.derived_pairs, [])
        self.assertEqual(
            [item["reason"] for item in result.blocked_sources],
            ["ownership_transfer_destination_ambiguous"],
        )

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
        self.assertEqual(
            [item["reason"] for item in result.blocked_sources],
            ["ownership_transfer_destination_missing_ref"],
        )

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
        self.assertEqual(
            [item["reason"] for item in result.blocked_sources],
            ["ownership_transfer_source_ambiguous"],
        )

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
        # B has a possible self-transfer leg (CSV provider id) AND an unrelated
        # same-value deposit carrying a DIFFERENT real on-chain txid. The deriver
        # must not consume the provider-id row without explicit txid evidence.
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
        self.assertEqual(result.derived_pairs, [])
        self.assertEqual(
            [item["reason"] for item in result.blocked_sources],
            ["ownership_transfer_destination_ambiguous"],
        )

    def test_ambiguous_equal_value_candidates_declined(self):
        # Two same-value B inbounds, both non-txid ids in window -> ambiguous.
        # The deriver must DECLINE the whole tx: synthesizing would fabricate a
        # duplicate transfer_in on top of the genuine recorded leg (holdings
        # inflation); reusing would cannibalize an unrelated deposit. Leave the
        # source on its existing path.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        c1 = _inbound(row_id="b-1", wallet_id="B", amount_sats=50_000_000, txid="prov-1")
        c2 = _inbound(row_id="b-2", wallet_id="B", amount_sats=50_000_000, txid="prov-2")
        result = self._run([out, c1, c2],
                           {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(result.derived_pairs, [])
        self.assertEqual(result.synthetic_rows, [])
        self.assertEqual(result.dropped_out_ids, set())
        self.assertEqual(result.out_row_overrides, {})
        self.assertEqual(
            [item["reason"] for item in result.blocked_sources],
            ["ownership_transfer_destination_ambiguous"],
        )

    def test_duplicate_same_txid_candidates_declined(self):
        # Even shared txid evidence is ambiguous if the destination has duplicate
        # exact rows for the same spend.
        txid = "a" * 64
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid=txid, input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        c1 = _inbound(row_id="b-1", wallet_id="B", amount_sats=50_000_000, txid=txid)
        c2 = _inbound(row_id="b-2", wallet_id="B", amount_sats=50_000_000, txid=txid)
        result = self._run([out, c1, c2],
                           {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(result.derived_pairs, [])
        self.assertEqual(
            [item["reason"] for item in result.blocked_sources],
            ["ownership_transfer_destination_ambiguous"],
        )

    def test_near_value_candidate_blocks_synthesize(self):
        # B recorded the genuine leg via CSV but the amount is off by a sat
        # (rounding / fee-on-receive), so there is no exact match. Synthesizing
        # would duplicate that near row -> decline instead.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        off = _inbound(row_id="b-off", wallet_id="B", amount_sats=49_999_999, txid="prov-genuine")
        result = self._run([out, off],
                           {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(result.derived_pairs, [])
        self.assertEqual(
            [item["reason"] for item in result.blocked_sources],
            ["ownership_transfer_destination_ambiguous"],
        )

    def test_different_txid_deposit_does_not_block_synthesize(self):
        # B has a same-value deposit from a DIFFERENT real on-chain tx; the leg
        # from THIS tx was never recorded (sync gap). That separate receipt is
        # provably not this leg, so synthesize the leg and keep the deposit.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        other = _inbound(row_id="b-other", wallet_id="B", amount_sats=50_000_000, txid="b" * 64)
        result = self._run([out, other],
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

    def test_invalid_output_index_falls_back_to_position(self):
        # Imported raw JSON with a malformed vout.n should not crash journal
        # processing; output order is enough to mint stable synthetic ids.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        payload = json.loads(out["raw_json"])
        payload["vout"][0]["n"] = "bad"
        out["raw_json"] = json.dumps(payload)
        result = self._run([out], {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(len(result.derived_pairs), 1)
        self.assertIn("owned-derive:real-txid:out:0", result.derived_pairs[0]["out"]["id"])

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

    def test_input_shared_script_with_source_still_single_source(self):
        # An input whose script is owned by the source AND another wallet (shared
        # descriptor / reused address) is still the source's spend. Resolution is
        # set-based, so it derives regardless of which owner the index lists first.
        shared_input = "0014" + "77" * 20
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[shared_input],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        index = OwnedIndex()
        index.add_script(shared_input, _match("B", "B"))  # non-source listed FIRST
        index.add_script(shared_input, _match("A", "A"))  # source second
        index.add_script(SCRIPT["B"], _match("B", "B"))
        result = derive_ownership_transfers(
            [out], index=index, wallet_refs_by_id=_refs("B"), already_paired_ids=set()
        )
        self.assertEqual(len(result.derived_pairs), 1)

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
        self.assertEqual(
            [item["reason"] for item in result.blocked_sources],
            ["ownership_transfer_source_ambiguous"],
        )

    def test_output_owned_by_source_and_other_is_change_not_leg(self):
        # A script owned by BOTH the source wallet and another wallet is change
        # back to self (matches the sync amount model), never a transfer leg.
        shared = "0014" + "55" * 20
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(shared, 50_000_000)],
        )
        index = OwnedIndex()
        index.add_script(SCRIPT["A"], _match("A", "A"))
        index.add_script(shared, _match("A", "A"))
        index.add_script(shared, _match("B", "B"))
        result = derive_ownership_transfers(
            [out], index=index, wallet_refs_by_id=_refs("A", "B"), already_paired_ids=set()
        )
        self.assertEqual(result.derived_pairs, [])

    def test_output_owned_by_two_non_source_wallets_declines(self):
        # A script owned by two different non-source wallets can't be routed; the
        # whole tx is declined rather than guessing a destination.
        shared = "0014" + "66" * 20
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(shared, 50_000_000)],
        )
        index = OwnedIndex()
        index.add_script(SCRIPT["A"], _match("A", "A"))
        index.add_script(shared, _match("B", "B"))
        index.add_script(shared, _match("C", "C"))
        result = derive_ownership_transfers(
            [out], index=index, wallet_refs_by_id=_refs("B", "C"), already_paired_ids=set()
        )
        self.assertEqual(result.derived_pairs, [])
        self.assertEqual(
            [item["reason"] for item in result.blocked_sources],
            ["ownership_transfer_ambiguous_output"],
        )

    def test_synthetic_prefix_inbound_not_reused(self):
        # A synthetic inbound minted by another stage (direct-payout target leg)
        # must not be consumed as a MOVE destination — synthesize a fresh leg.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        synth_in = _inbound(row_id="direct-payout:P:in", wallet_id="B",
                            amount_sats=50_000_000, txid="real-txid")
        result = self._run([out, synth_in],
                           {SCRIPT["A"]: ("A", "A"), SCRIPT["B"]: ("B", "B")}, _refs("B"))
        self.assertEqual(len(result.derived_pairs), 1)
        self.assertTrue(str(result.derived_pairs[0]["in"]["id"]).startswith("owned-derive:"))

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

    def test_output_match_on_other_network_is_not_owned_leg(self):
        # The destination scriptpubkey hex is owned only by a wallet on a
        # DIFFERENT network (mainnet/testnet siblings share the same 0014... hex).
        # A real mainnet BTC payment must not be re-routed into the testnet wallet
        # as a phantom MOVE — it stays on the disposal path.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        index = OwnedIndex()
        index.add_script(SCRIPT["A"], _match("A", "A"))  # source: bitcoin/mainnet
        index.add_script(
            SCRIPT["B"],
            OwnedMatch("T", "Testnet", "", "bitcoin", "testnet", "", None, None, "derived"),
        )
        result = derive_ownership_transfers(
            [out], index=index, wallet_refs_by_id=_refs("T"), already_paired_ids=set()
        )
        self.assertEqual(result.derived_pairs, [])
        self.assertEqual(result.blocked_sources, [])

    def test_output_match_picks_source_network_when_script_collides(self):
        # The destination script is owned by BOTH a mainnet wallet B and a testnet
        # collision wallet T. The leg must route to B (the source's network), not
        # be declined as multi-owner-ambiguous.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        index = OwnedIndex()
        index.add_script(SCRIPT["A"], _match("A", "A"))
        index.add_script(SCRIPT["B"], _match("B", "B"))  # bitcoin/mainnet
        index.add_script(
            SCRIPT["B"],
            OwnedMatch("T", "Testnet", "", "bitcoin", "testnet", "", None, None, "derived"),
        )
        result = derive_ownership_transfers(
            [out], index=index, wallet_refs_by_id=_refs("B"), already_paired_ids=set()
        )
        self.assertEqual(len(result.derived_pairs), 1)
        self.assertEqual(result.derived_pairs[0]["in"]["wallet_id"], "B")

    def test_equivalent_network_spellings_still_derive(self):
        # The index seeds (chain, network) from paths with inconsistent spelling
        # (descriptor normalizes; address-list / inventory store raw config / DB
        # values). A legit mainnet A->B move where the two wallets were seeded
        # with different-but-equivalent spellings (bitcoin/main vs btc/mainnet,
        # and an empty network defaulting to main) must still derive — the filter
        # normalizes before comparing.
        out = _outbound(
            row_id="a-out", wallet_id="A", amount_sats=50_000_000, fee_sats=1000,
            txid="real-txid", input_scripts=[SCRIPT["A"]],
            outputs=[(SCRIPT["B"], 50_000_000)],
        )
        for src, dst in (
            (("bitcoin", "main"), ("btc", "mainnet")),
            (("btc", "mainnet"), ("bitcoin", "main")),
            (("bitcoin", ""), ("bitcoin", "main")),
        ):
            with self.subTest(src=src, dst=dst):
                index = OwnedIndex()
                index.add_script(
                    SCRIPT["A"],
                    OwnedMatch("A", "A", "", src[0], src[1], "", None, None, "derived"),
                )
                index.add_script(
                    SCRIPT["B"],
                    OwnedMatch("B", "B", "", dst[0], dst[1], "", None, None, "derived"),
                )
                result = derive_ownership_transfers(
                    [out], index=index, wallet_refs_by_id=_refs("B"),
                    already_paired_ids=set(),
                )
                self.assertEqual(len(result.derived_pairs), 1)
                self.assertEqual(result.derived_pairs[0]["in"]["wallet_id"], "B")


def _plain_row(*, row_id, wallet_id, direction, amount_sats, txid, asset="LBTC", fee_sats=0):
    """A recorded row with NO on-chain graph (Liquid / graphless CSV shape)."""
    return {
        "id": row_id,
        "wallet_id": wallet_id,
        "wallet_label": f"Wallet {wallet_id}",
        "direction": direction,
        "asset": asset,
        "amount": amount_sats * SATS,
        "fee": fee_sats * SATS,
        "external_id": txid,
        "occurred_at": "2026-03-14T17:30:00Z",
        "created_at": "2026-03-14T17:30:00Z",
        "fiat_rate": 40000.0,
        "fiat_rate_exact": "40000",
        "fiat_value": None,
        "raw_json": "{}",
    }


class RecordedFanoutDeriverTests(unittest.TestCase):
    """The graphless 1->N decomposer (Liquid / CSV) working from rows alone."""

    def test_recorded_fanout_decomposes_into_legs(self):
        # A spends 0.8 LBTC fanning to B (0.5) and C (0.3), all recorded under
        # one txid. detect_intra skips the 1-out/2-in shape; the decomposer pairs
        # each recorded inbound, whole fee on the first leg, drops the source.
        rows = [
            _plain_row(row_id="a-out", wallet_id="A", direction="outbound",
                       amount_sats=80_000_000, fee_sats=2000, txid="lq"),
            _plain_row(row_id="b-in", wallet_id="B", direction="inbound",
                       amount_sats=50_000_000, txid="lq"),
            _plain_row(row_id="c-in", wallet_id="C", direction="inbound",
                       amount_sats=30_000_000, txid="lq"),
        ]
        result = derive_recorded_fanout_transfers(rows, already_paired_ids=set())
        self.assertEqual(len(result.derived_pairs), 2)
        self.assertEqual(result.dropped_out_ids, {"a-out"})
        self.assertEqual({p["in"]["id"] for p in result.derived_pairs}, {"b-in", "c-in"})
        for pair in result.derived_pairs:
            self.assertEqual(pair["source"], "recorded_fanout")
            self.assertEqual(pair["out"]["amount"], pair["in"]["amount"])
        fees = sorted(p["out"]["fee"] for p in result.derived_pairs)
        self.assertEqual(fees, [0, 2000 * SATS])  # whole fee on exactly one leg

    def test_shortfall_not_decomposed(self):
        # A destination wasn't synced -> recorded inbounds don't sum to the
        # outbound -> the split would be wrong, so decline (leave to quarantine).
        rows = [
            _plain_row(row_id="a-out", wallet_id="A", direction="outbound",
                       amount_sats=80_000_000, txid="lq"),
            _plain_row(row_id="b-in", wallet_id="B", direction="inbound",
                       amount_sats=50_000_000, txid="lq"),
        ]
        result = derive_recorded_fanout_transfers(rows, already_paired_ids=set())
        self.assertEqual(result.derived_pairs, [])

    def test_consolidation_not_decomposed(self):
        # Two outbounds under one txid (N->1 consolidation): per-wallet fee is
        # double-counted, amounts unreliable -> decline.
        rows = [
            _plain_row(row_id="a-out", wallet_id="A", direction="outbound",
                       amount_sats=50_000_000, txid="cons"),
            _plain_row(row_id="b-out", wallet_id="B", direction="outbound",
                       amount_sats=30_000_000, txid="cons"),
            _plain_row(row_id="c-in", wallet_id="C", direction="inbound",
                       amount_sats=80_000_000, txid="cons"),
        ]
        result = derive_recorded_fanout_transfers(rows, already_paired_ids=set())
        self.assertEqual(result.derived_pairs, [])

    def test_one_to_one_left_to_detect_intra(self):
        # A clean 1-out/1-in is detect_intra_transfers' job; the decomposer only
        # handles >=2 destinations, so it declines here.
        rows = [
            _plain_row(row_id="a-out", wallet_id="A", direction="outbound",
                       amount_sats=50_000_000, txid="lq"),
            _plain_row(row_id="b-in", wallet_id="B", direction="inbound",
                       amount_sats=50_000_000, txid="lq"),
        ]
        result = derive_recorded_fanout_transfers(rows, already_paired_ids=set())
        self.assertEqual(result.derived_pairs, [])

    def test_already_paired_source_skipped(self):
        rows = [
            _plain_row(row_id="a-out", wallet_id="A", direction="outbound",
                       amount_sats=80_000_000, txid="lq"),
            _plain_row(row_id="b-in", wallet_id="B", direction="inbound",
                       amount_sats=50_000_000, txid="lq"),
            _plain_row(row_id="c-in", wallet_id="C", direction="inbound",
                       amount_sats=30_000_000, txid="lq"),
        ]
        result = derive_recorded_fanout_transfers(rows, already_paired_ids={"a-out"})
        self.assertEqual(result.derived_pairs, [])

    def test_multi_source_not_decomposed_when_one_source_already_paired(self):
        # Two wallets fund the spend (A + B both outbound under one txid). Even
        # when one source is already paired elsewhere, the group is still a
        # multi-source consolidation whose per-wallet amounts are unreliable, so
        # the surviving source must NOT be split. The consolidation guard counts
        # ALL positive outbounds, not just the unpaired ones.
        rows = [
            _plain_row(row_id="a-out", wallet_id="A", direction="outbound",
                       amount_sats=50_000_000, txid="T"),
            _plain_row(row_id="b-out", wallet_id="B", direction="outbound",
                       amount_sats=80_000_000, txid="T"),
            _plain_row(row_id="c-in", wallet_id="C", direction="inbound",
                       amount_sats=50_000_000, txid="T"),
            _plain_row(row_id="d-in", wallet_id="D", direction="inbound",
                       amount_sats=30_000_000, txid="T"),
        ]
        result = derive_recorded_fanout_transfers(rows, already_paired_ids={"a-out"})
        self.assertEqual(result.derived_pairs, [])
        self.assertEqual(result.dropped_out_ids, set())

    def test_same_wallet_double_inbound_declined(self):
        # Two inbounds from the SAME destination wallet under one txid is an odd
        # shape (sync records one combined inbound per wallet) -> decline.
        rows = [
            _plain_row(row_id="a-out", wallet_id="A", direction="outbound",
                       amount_sats=80_000_000, txid="lq"),
            _plain_row(row_id="b-in-1", wallet_id="B", direction="inbound",
                       amount_sats=50_000_000, txid="lq"),
            _plain_row(row_id="b-in-2", wallet_id="B", direction="inbound",
                       amount_sats=30_000_000, txid="lq"),
        ]
        result = derive_recorded_fanout_transfers(rows, already_paired_ids=set())
        self.assertEqual(result.derived_pairs, [])


if __name__ == "__main__":
    unittest.main()

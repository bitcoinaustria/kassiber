"""Integration tests: the ownership deriver through the real RP2 engine.

Two layers:

* engine-level — drive ``GenericRP2TaxEngine.build_ledger_state`` directly with
  hand-built esplora rows + a hand-built ``OwnedIndex`` to prove a 1->N fan-out
  becomes carrying MOVEs (``transfer_in``/``transfer_out``) instead of the
  ``owned_fanout_unresolved`` quarantine it gets without the deriver.
* handler-level — exercise ``handlers.build_ledger_state`` end-to-end against a
  temp SQLite DB so the new index-build + all-wallet-refs wiring is covered, and
  confirm derived pairs are never persisted to ``transaction_pairs``.
"""

import json
import tempfile
import unittest
from pathlib import Path

from kassiber.cli import handlers
from kassiber.core.engines import TaxEngineLedgerInputs, build_tax_engine
from kassiber.core.ownership import OwnedIndex, OwnedMatch
from kassiber.core.sync_backends import address_to_scriptpubkey
from kassiber.db import open_db


NOW = "2026-01-01T00:00:00Z"
BTC = 100_000_000_000  # 1 BTC in msat
SATS = 1000  # msat per sat

PROFILE = {
    "id": "profile-1",
    "workspace_id": "ws-1",
    "label": "Default",
    "fiat_currency": "USD",
    "tax_country": "generic",
    "tax_long_term_days": 365,
    "gains_algorithm": "FIFO",
}

SCRIPT_A = "0014" + "a1" * 20
SCRIPT_B = "0014" + "b2" * 20
SCRIPT_C = "0014" + "c3" * 20

WALLET_REFS = {
    wid: {
        "id": wid,
        "label": label,
        "wallet_account_id": "acct-1",
        "account_code": "treasury",
        "account_label": "Treasury",
    }
    for wid, label in (("A", "Cold"), ("B", "Hot"), ("C", "Savings"))
}


def _match(wallet_id, label):
    return OwnedMatch(wallet_id, label, "", "bitcoin", "main", "", None, None, "derived")


def _fanout_index():
    index = OwnedIndex()
    index.add_script(SCRIPT_A, _match("A", "Cold"))
    index.add_script(SCRIPT_B, _match("B", "Hot"))
    index.add_script(SCRIPT_C, _match("C", "Savings"))
    return index


def _row(wallet_id, direction, amount, *, external_id, raw_json="{}", fee=0, asset="BTC"):
    ref = WALLET_REFS[wallet_id]
    return {
        "id": f"{wallet_id}-{direction}-{external_id}",
        "workspace_id": "ws-1",
        "profile_id": "profile-1",
        "wallet_id": wallet_id,
        "wallet_label": ref["label"],
        "wallet_account_id": ref["wallet_account_id"],
        "account_code": ref["account_code"],
        "account_label": ref["account_label"],
        "external_id": external_id,
        "occurred_at": NOW,
        "created_at": NOW,
        "direction": direction,
        "asset": asset,
        "amount": amount,
        "fee": fee,
        "fiat_currency": "USD",
        "fiat_rate": 40000.0,
        "fiat_rate_exact": "40000",
        "fiat_value": None,
        "kind": "withdrawal" if direction == "outbound" else "deposit",
        "description": f"{wallet_id} {direction}",
        "note": None,
        "raw_json": raw_json,
        "excluded": 0,
    }


def _esplora_fanout_json():
    return json.dumps(
        {
            "txid": "tx0",
            "vin": [{"txid": "prevtx", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}}],
            "vout": [
                {"n": 0, "scriptpubkey": SCRIPT_B, "value": 50_000_000},  # 0.5 BTC
                {"n": 1, "scriptpubkey": SCRIPT_C, "value": 30_000_000},  # 0.3 BTC
            ],
        }
    )


def _fanout_rows():
    # A buys 1 BTC, then one tx fans 0.5 -> B and 0.3 -> C (both record inbounds
    # under the same txid "tx0"). detect_intra_transfers skips the 1-out/2-in
    # shape; without the deriver the journal pipeline quarantines it.
    return [
        _row("A", "inbound", BTC, external_id="acq-1"),
        _row("A", "outbound", 80 * BTC // 100, external_id="tx0",
             raw_json=_esplora_fanout_json(), fee=2_000_000),
        _row("B", "inbound", 50 * BTC // 100, external_id="tx0"),
        _row("C", "inbound", 30 * BTC // 100, external_id="tx0"),
    ]


class OwnershipDeriverEngineTest(unittest.TestCase):
    def _run(self, owned_index):
        return build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=_fanout_rows(),
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=owned_index,
            )
        )

    def test_recorded_fanout_decomposed_without_ownership_index(self):
        # All legs of this fan-out were synced (recorded under one txid across
        # wallets), so the row-based recorded-fanout decomposer proves the
        # self-transfer and books MOVEs even with NO ownership index — the index
        # (graph read) is only needed for sync-gap / mismatched-txid cases.
        state = self._run(owned_index=None)
        reasons = {q["reason"] for q in state.quarantines}
        self.assertNotIn("owned_fanout_unresolved", reasons)
        entry_types = sorted(entry["entry_type"] for entry in state.entries)
        self.assertEqual(entry_types.count("transfer_out"), 2)
        self.assertEqual(entry_types.count("transfer_in"), 2)

    def test_fanout_becomes_moves_with_deriver(self):
        state = self._run(owned_index=_fanout_index())
        reasons = {q["reason"] for q in state.quarantines}
        self.assertNotIn("owned_fanout_unresolved", reasons)

        entry_types = sorted(entry["entry_type"] for entry in state.entries)
        self.assertEqual(entry_types.count("transfer_out"), 2)
        self.assertEqual(entry_types.count("transfer_in"), 2)
        self.assertIn("acquisition", entry_types)

        holdings = {
            label: float(totals["quantity"])
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        # Basis carried across: 0.5 BTC now sits in Hot, 0.3 in Savings, the
        # remainder (minus the network fee) stays in Cold. No disposal/gain.
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.5, places=6)
        self.assertAlmostEqual(holdings.get("Savings", 0.0), 0.3, places=6)

    def test_duplicate_outbound_group_quarantines_instead_of_deriving(self):
        # A stale duplicate source-overlap row can pass the source fallback via
        # txid_wallets. The deriver must decline the whole multi-outbound group
        # so the fanout quarantine blocks every leg instead of synthesizing a MOVE
        # and leaving a sibling disposal or duplicate synthetic id behind.
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        index.add_script(SCRIPT_C, _match("C", "Savings"))
        index.note_txid("prevtx", "B", "Hot")
        dup_json = json.dumps(
            {
                "txid": "dup-tx",
                "vin": [{"txid": "prevtx", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}}],
                "vout": [{"n": 0, "scriptpubkey": SCRIPT_C, "value": 85_000_000}],
            }
        )
        rows = [
            _row("A", "inbound", BTC, external_id="acqA"),
            _row("B", "inbound", BTC, external_id="acqB"),
            _row("A", "outbound", 85 * BTC // 100, external_id="dup-tx", raw_json=dup_json),
            _row("B", "outbound", 85 * BTC // 100, external_id="dup-tx", raw_json=dup_json),
            _row("C", "inbound", 85 * BTC // 100, external_id="dup-tx"),
        ]

        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=index,
            )
        )

        self.assertEqual(
            sorted(q["reason"] for q in state.quarantines),
            ["owned_fanout_unresolved"] * 3,
        )
        self.assertFalse(
            any(audit.get("pairing_source") == "ownership_derived" for audit in state.intra_audit)
        )
        entry_types = [entry["entry_type"] for entry in state.entries]
        self.assertNotIn("disposal", entry_types)
        self.assertNotIn("transfer_out", entry_types)
        self.assertNotIn("transfer_in", entry_types)

    def test_derived_move_provenance_is_surfaced(self):
        # The non-taxable treatment must be auditable: every leg the deriver
        # proved from the on-chain graph is tagged "ownership_derived" in the
        # intra-transfer audit, and records the basis in its entry description so
        # the report / transaction view shows WHY it is a MOVE.
        state = self._run(owned_index=_fanout_index())
        derived = [
            audit
            for audit in state.intra_audit
            if audit.get("pairing_source") == "ownership_derived"
        ]
        self.assertEqual(len(derived), 2)  # both fan-out legs
        descriptions = [
            entry.get("description", "")
            for entry in state.entries
            if entry["entry_type"] in ("transfer_in", "transfer_out")
        ]
        self.assertEqual(len(descriptions), 4)  # 2 fan-out legs x (out + in)
        self.assertTrue(
            all("proven by address ownership" in d for d in descriptions),
            descriptions,
        )

    def test_row_matched_move_has_no_ownership_provenance(self):
        # A plain same-txid transfer (paired by detect_intra_transfers, not
        # graph-derived) must NOT carry the ownership-provenance note — the
        # marker is specific to derived MOVEs.
        rows = [
            _row("A", "inbound", BTC, external_id="acq"),
            _row("A", "outbound", BTC, external_id="move-tx"),
            _row("B", "inbound", BTC, external_id="move-tx"),
        ]
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=None,
            )
        )
        transfers = [
            entry
            for entry in state.entries
            if entry["entry_type"] in ("transfer_in", "transfer_out")
        ]
        self.assertEqual(len(transfers), 2)
        self.assertFalse(
            any("proven by address ownership" in e.get("description", "") for e in transfers),
            [e.get("description") for e in transfers],
        )


SCRIPT_EXT = "0014" + "ee" * 20  # external recipient, never owned


class OwnershipDeriverMixedSpendTest(unittest.TestCase):
    """Residual-SELL path: one spend pays an owned wallet AND an external party.

    Exercises the engine branch where the source is overridden to a residual
    disposal. Locks in the fee fix: the miner fee must leave the source pool
    exactly once (on the MOVE leg), not twice. On the buggy code the doubled
    fee makes required > available and trips a false insufficient_lots /
    missing_cost_basis quarantine on a transaction that balances on-chain.
    """

    def _rows(self):
        mixed_json = json.dumps(
            {
                "txid": "mixed-tx",
                "vin": [{"txid": "prevtx", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}}],
                "vout": [
                    {"n": 0, "scriptpubkey": SCRIPT_B, "value": 50_000_000},  # owned 0.5
                    {"n": 1, "scriptpubkey": SCRIPT_EXT, "value": 20_000_000},  # external 0.2
                ],
            }
        )
        return [
            # A acquires exactly what it then spends: 0.7 outputs + 0.0001 fee.
            _row("A", "inbound", 70_010_000_000, external_id="acq-1"),
            _row("A", "outbound", 70_000_000_000, external_id="mixed-tx",
                 raw_json=mixed_json, fee=10_000_000),
        ]

    def test_mixed_spend_books_move_and_residual_without_phantom_fee(self):
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=self._rows(),
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=index,
            )
        )
        reasons = {q["reason"] for q in state.quarantines}
        self.assertNotIn("insufficient_lots", reasons)
        self.assertNotIn("missing_cost_basis", reasons)
        self.assertNotIn("transfer_fee_implausible", reasons)

        entry_types = [e["entry_type"] for e in state.entries]
        self.assertIn("transfer_in", entry_types)
        self.assertIn("transfer_out", entry_types)

        holdings = {
            label: float(totals["quantity"])
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.5, places=6)
        # Source fully spent (0.5 moved + 0.2 sold + 0.0001 fee == 0.7001 acquired).
        self.assertAlmostEqual(holdings.get("Cold", 0.0), 0.0, places=6)

    def test_direct_payout_remainder_can_still_be_derived(self):
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        out = self._rows()[1]
        direct_payouts = [
            {
                "id": "direct-payout-1",
                "out_transaction_id": out["id"],
                "kind": "direct-swap-payout",
                "policy": "taxable",
                "payout_asset": "BTC",
                "payout_amount": 20_000_000_000,
                "payout_occurred_at": "2026-01-01T00:01:00Z",
                "payout_fiat_value": 8000,
                "payout_external_id": "provider-payout",
                "counterparty": "external-recipient",
                "notes": "direct payout",
                "swap_fee_msat": 0,
                "swap_fee_kind": "combined",
                "created_at": "2026-01-01T00:01:00Z",
                "out_amount": 20_000_000_000,
            }
        ]
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=self._rows(),
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                direct_payout_records=direct_payouts,
                owned_index=index,
            )
        )

        self.assertEqual(state.quarantines, [])
        entry_types = [entry["entry_type"] for entry in state.entries]
        self.assertIn("transfer_in", entry_types)
        self.assertIn("transfer_out", entry_types)
        disposals = [entry for entry in state.entries if entry["entry_type"] == "disposal"]
        self.assertEqual(len(disposals), 1)
        self.assertAlmostEqual(float(disposals[0]["quantity"]), -0.2, places=6)
        holdings = {
            label: float(totals["quantity"])
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.5, places=6)
        self.assertAlmostEqual(holdings.get("Cold", 0.0), 0.0, places=6)

    def test_whole_row_payout_not_hijacked_by_same_txid_inbound(self):
        # #2: a reviewed WHOLE-row taxable direct payout whose out tx shares a
        # txid with another owned wallet's recorded inbound (a batched tx) must
        # book its declared disposal — detect_intra_transfers must NOT pair the
        # payout's proceeds row with the sibling inbound into a non-taxable MOVE
        # (which would silently drop the 20000 proceeds).
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        rows = [
            _row("A", "inbound", BTC, external_id="acqA"),
            _row("A", "outbound", 50 * BTC // 100, external_id="payout-tx"),
            _row("B", "inbound", 50 * BTC // 100, external_id="payout-tx"),
        ]
        direct_payouts = [
            {
                "id": "direct-payout-hijack",
                "out_transaction_id": "A-outbound-payout-tx",
                "kind": "direct-swap-payout",
                "policy": "taxable",
                "payout_asset": "BTC",
                "payout_amount": 50 * BTC // 100,
                "payout_occurred_at": NOW,
                "payout_fiat_value": 20000,
                "payout_external_id": "provider-payout",
                "counterparty": "external-recipient",
                "notes": "direct payout",
                "swap_fee_msat": 0,
                "swap_fee_kind": "combined",
                "created_at": NOW,
                "out_amount": 50 * BTC // 100,  # whole row
            }
        ]
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                direct_payout_records=direct_payouts,
                owned_index=index,
            )
        )
        entry_types = [e["entry_type"] for e in state.entries]
        # The payout disposal is booked, not hijacked into a MOVE.
        self.assertNotIn("transfer_out", entry_types)
        self.assertFalse(
            any(
                e["entry_type"] == "acquisition" and e["wallet_id"] == "B"
                for e in state.entries
            )
        )
        disposals = [e for e in state.entries if e["entry_type"] == "disposal"]
        self.assertEqual(len(disposals), 1)
        self.assertAlmostEqual(float(disposals[0]["quantity"]), -0.5, places=6)
        self.assertAlmostEqual(float(disposals[0]["proceeds"]), 20000, places=2)
        # The suppressed sibling receipt is a real synced row contradicting the
        # whole-row review — it must surface for review, not vanish silently.
        conflicts = [
            q
            for q in state.quarantines
            if q["reason"] == "direct_payout_conflicting_receipt"
        ]
        self.assertEqual(len(conflicts), 1)
        self.assertEqual(conflicts[0]["transaction_id"], "B-inbound-payout-tx")

    def test_invalid_payout_does_not_prune_self_transfer_pair(self):
        # Codex review: a direct payout whose out_amount EXCEEDS the source amount
        # is rejected (direct_payout_out_amount_invalid, no proceeds row). It must
        # NOT be treated as a claimed payout — pruning the same-txid self-transfer
        # pair for a rejected payout drops the transfer with no disposal to replace
        # it, leaving the destination a phantom acquisition. The pair is preserved.
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        rows = [
            _row("A", "inbound", BTC, external_id="acqA"),
            _row("A", "outbound", 50 * BTC // 100, external_id="inv-tx"),
            _row("B", "inbound", 50 * BTC // 100, external_id="inv-tx"),
        ]
        direct_payouts = [
            {
                "id": "payout-invalid",
                "out_transaction_id": "A-outbound-inv-tx",
                "kind": "direct-swap-payout",
                "policy": "taxable",
                "payout_asset": "BTC",
                "payout_amount": 60 * BTC // 100,
                "payout_occurred_at": NOW,
                "payout_fiat_value": 24000,
                "payout_external_id": "provider-payout",
                "counterparty": "external-recipient",
                "notes": "direct payout",
                "swap_fee_msat": 0,
                "swap_fee_kind": "combined",
                "created_at": NOW,
                "out_amount": 60 * BTC // 100,  # > source amount -> invalid/blocked
            }
        ]
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                direct_payout_records=direct_payouts,
                owned_index=index,
            )
        )
        reasons = [q["reason"] for q in state.quarantines]
        self.assertIn("direct_payout_out_amount_invalid", reasons)
        entry_types = [e["entry_type"] for e in state.entries]
        # The self-transfer pair survived (booked as a MOVE); Hot is NOT a phantom
        # standalone acquisition.
        self.assertIn("transfer_in", entry_types)
        self.assertFalse(
            any(e["entry_type"] == "acquisition" and e["wallet_id"] == "B" for e in state.entries)
        )

    def test_whole_row_payout_with_readable_graph_not_restored_as_move(self):
        # Codex review #1: the payout out row has a READABLE graph that also pays
        # an owned sibling wallet + an external residual, so
        # graph_partial_payment_out_ids WITHHOLDS its auto-pair before the payout
        # prune runs. The payout-claimed id must also be dropped from the withheld
        # set, or the restore-withheld path re-adds it and books the reviewed
        # payout as a non-taxable MOVE (dropping the declared proceeds).
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        spend = json.dumps(
            {
                "txid": "pp2",
                "vin": [{"txid": "pv", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}}],
                "vout": [
                    {"n": 0, "scriptpubkey": SCRIPT_B, "value": 30_000_000},  # owned sibling
                    {"n": 1, "scriptpubkey": SCRIPT_EXT, "value": 20_000_000},  # external
                ],
            }
        )
        rows = [
            _row("A", "inbound", BTC, external_id="acqA"),
            _row("A", "outbound", 50 * BTC // 100, external_id="pp2", raw_json=spend),
            _row("B", "inbound", 30 * BTC // 100, external_id="pp2"),
        ]
        direct_payouts = [
            {
                "id": "direct-payout-graph",
                "out_transaction_id": "A-outbound-pp2",
                "kind": "direct-swap-payout",
                "policy": "taxable",
                "payout_asset": "BTC",
                "payout_amount": 50 * BTC // 100,
                "payout_occurred_at": NOW,
                "payout_fiat_value": 20000,
                "payout_external_id": "provider-payout",
                "counterparty": "external-recipient",
                "notes": "direct payout",
                "swap_fee_msat": 0,
                "swap_fee_kind": "combined",
                "created_at": NOW,
                "out_amount": 50 * BTC // 100,  # whole row
            }
        ]
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                direct_payout_records=direct_payouts,
                owned_index=index,
            )
        )
        entry_types = [e["entry_type"] for e in state.entries]
        self.assertNotIn("transfer_out", entry_types)  # not hijacked into a MOVE
        self.assertFalse(
            any(
                e["entry_type"] == "acquisition" and e["wallet_id"] == "B"
                for e in state.entries
            )
        )
        disposals = [e for e in state.entries if e["entry_type"] == "disposal"]
        self.assertTrue(disposals)
        self.assertAlmostEqual(
            float(disposals[0]["proceeds"]), 20000, places=2
        )


class OwnershipDeriverAmbiguityTest(unittest.TestCase):
    """Ambiguous destination must not inflate holdings.

    When the destination has two equal-value inbounds (the genuine self-transfer
    leg recorded by a CSV import + an unrelated deposit of the same amount), the
    deriver must decline rather than fabricate a duplicate transfer_in. On the
    buggy code this booked the leg twice (Hot = 1.5 instead of 1.0) — silent
    holdings inflation and understated future gains.
    """

    def test_ambiguous_destination_does_not_inflate_holdings(self):
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        spend = json.dumps(
            {
                "txid": "real-T",
                "vin": [{"txid": "pv", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}}],
                "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 50_000_000}],
            }
        )
        rows = [
            _row("A", "inbound", 70_000_000_000, external_id="acq"),
            _row("A", "outbound", 50_000_000_000, external_id="real-T", raw_json=spend),
            _row("B", "inbound", 50_000_000_000, external_id="prov-genuine"),
            _row("B", "inbound", 50_000_000_000, external_id="prov-other"),
        ]
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=index,
            )
        )
        self.assertEqual(
            [q["reason"] for q in state.quarantines],
            ["ownership_transfer_destination_ambiguous"],
        )
        entry_types = [entry["entry_type"] for entry in state.entries]
        # The ambiguous source is NOT dropped (that would lose the disposal that
        # offsets B's recorded receipts and inflate the total). It stays on its
        # conservative disposal path — correct holdings — plus the review flag,
        # and no fabricated transfer_in.
        self.assertIn("disposal", entry_types)
        self.assertNotIn("transfer_in", entry_types)
        holdings = {
            label: round(float(totals["quantity"]), 5)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        # B keeps exactly its two recorded 0.5 receipts = 1.0, never 1.5.
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 1.0, places=6)
        # Source A (0.7 acquired) is debited by the 0.5 disposal -> 0.2, never
        # left at 0.7 (the silent source-side inflation of block-and-remove).
        self.assertAlmostEqual(holdings.get("Cold", 0.0), 0.2, places=6)

    def test_duplicate_same_txid_destination_quarantines_source(self):
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        txid = "a" * 64
        spend = json.dumps(
            {
                "txid": txid,
                "vin": [{"txid": "pv", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}}],
                "vout": [{"n": 0, "scriptpubkey": SCRIPT_B, "value": 50_000_000}],
            }
        )
        rows = [
            _row("A", "inbound", BTC, external_id="acq"),
            _row("A", "outbound", 50_000_000_000, external_id=txid, raw_json=spend),
            _row("B", "inbound", 50_000_000_000, external_id=txid),
            _row("B", "inbound", 50_000_000_000, external_id=txid),
        ]
        rows[-2]["id"] = "b-in-1"
        rows[-1]["id"] = "b-in-2"
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=index,
            )
        )
        # The destination's two recorded inbounds share the spend's txid with the
        # source, so the whole 1-out/2-in group is a recorded fan-out: the
        # existing owned-fanout guard holds it back and quarantines every leg.
        # The deriver leaves the source in place (no second quarantine — it would
        # only be collapsed by dedupe_quarantines) and never books a transfer_in.
        self.assertEqual(
            sorted(q["reason"] for q in state.quarantines),
            ["owned_fanout_unresolved"] * 3,
        )
        self.assertNotIn("transfer_in", [entry["entry_type"] for entry in state.entries])
        holdings = {
            label: round(float(totals["quantity"]), 5)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        # The held-back group books nothing into B — no inflation.
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.0, places=6)

    def test_multi_source_sync_gap_quarantines_source(self):
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        index.add_script(SCRIPT_C, _match("C", "Savings"))
        spend = json.dumps(
            {
                "txid": "multi-source",
                "vin": [
                    {"txid": "prev-a", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}},
                    {"txid": "prev-b", "vout": 1, "prevout": {"scriptpubkey": SCRIPT_B}},
                ],
                "vout": [{"n": 0, "scriptpubkey": SCRIPT_C, "value": 80_000_000}],
            }
        )
        rows = [
            _row("A", "inbound", BTC, external_id="acq"),
            _row("A", "outbound", 80_000_000_000, external_id="multi-source", raw_json=spend),
        ]
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=index,
            )
        )
        self.assertEqual(
            [q["reason"] for q in state.quarantines],
            ["ownership_transfer_source_ambiguous"],
        )
        # The unsplittable source is flagged for review but NOT dropped from
        # booking: it posts its normal disposal (matching the deriver-off
        # baseline), so the source wallet is debited and holdings are not
        # inflated. Dropping it instead would leave the spent coins in the source.
        entry_types = [entry["entry_type"] for entry in state.entries]
        self.assertIn("disposal", entry_types)
        holdings = {
            label: round(float(totals["quantity"]), 5)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(holdings.get("Cold", 0.0), 0.2, places=6)

    def test_consolidation_with_recorded_destination_books_moves(self):
        # A+B -> C, all owned, C's inbound recorded under the spend's txid. Each
        # contributing wallet syncs the spend independently and stamps the whole
        # fee onto its own row, so the per-wallet amounts cannot just be summed.
        # The multi-source consolidation deriver reads the single fee once and
        # the destination total from the graph, books one carrying MOVE per
        # contributor, and drops C's recorded receipt (replaced by the synthetic
        # in-legs). No quarantine, no disposal, and the profile total stays 0.8.
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        index.add_script(SCRIPT_C, _match("C", "Savings"))
        consol = json.dumps(
            {
                "txid": "consol",
                "vin": [
                    {"txid": "pa", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}},
                    {"txid": "pb", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_B}},
                ],
                "vout": [{"n": 0, "scriptpubkey": SCRIPT_C, "value": 80_000_000}],
            }
        )
        rows = [
            _row("A", "inbound", 50_000_000_000, external_id="acqA"),
            _row("B", "inbound", 30_000_000_000, external_id="acqB"),
            _row("A", "outbound", 50_000_000_000, external_id="consol", raw_json=consol),
            _row("B", "outbound", 30_000_000_000, external_id="consol", raw_json=consol),
            _row("C", "inbound", 80_000_000_000, external_id="consol"),
        ]
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=index,
            )
        )
        holdings = {
            label: round(float(totals["quantity"]), 5)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(sum(holdings.values()), 0.8, places=6)
        self.assertAlmostEqual(holdings.get("Cold", 0.0), 0.0, places=6)
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.0, places=6)
        self.assertAlmostEqual(holdings.get("Savings", 0.0), 0.8, places=6)
        entry_types = [entry["entry_type"] for entry in state.entries]
        self.assertNotIn("disposal", entry_types)
        self.assertIn("transfer_out", entry_types)
        self.assertIn("transfer_in", entry_types)
        transfer_in_transaction_ids = {
            entry["transaction_id"]
            for entry in state.entries
            if entry["entry_type"] == "transfer_in"
        }
        self.assertEqual(transfer_in_transaction_ids, {"C-inbound-consol"})
        self.assertEqual(state.quarantines, [])

    def test_fanout_amount_mismatch_with_recorded_inbounds_does_not_inflate(self):
        # 1->N spend whose parsed owned outputs (0.5 + 0.3) exceed the recorded
        # outbound amount (0.4) -> the deriver declines (amount_mismatch). Both
        # destination inbounds are recorded under the spend's txid, so the
        # owned-fanout guard holds the whole group. Block-and-remove dropped the
        # source and let the two inbound legs book as fresh acquisitions,
        # inflating the total to 1.8; it must stay 1.0.
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        index.add_script(SCRIPT_C, _match("C", "Savings"))
        fan = json.dumps(
            {
                "txid": "fan",
                "vin": [{"txid": "pa", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}}],
                "vout": [
                    {"n": 0, "scriptpubkey": SCRIPT_B, "value": 50_000_000},
                    {"n": 1, "scriptpubkey": SCRIPT_C, "value": 30_000_000},
                ],
            }
        )
        rows = [
            _row("A", "inbound", BTC, external_id="acqA"),
            _row("A", "outbound", 40_000_000_000, external_id="fan", raw_json=fan),
            _row("B", "inbound", 50_000_000_000, external_id="fan"),
            _row("C", "inbound", 30_000_000_000, external_id="fan"),
        ]
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=index,
            )
        )
        holdings = {
            label: round(float(totals["quantity"]), 5)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(sum(holdings.values()), 1.0, places=6)
        entry_types = [entry["entry_type"] for entry in state.entries]
        self.assertNotIn("transfer_in", entry_types)
        self.assertNotIn("disposal", entry_types)
        self.assertEqual(
            sorted(q["reason"] for q in state.quarantines),
            ["owned_fanout_unresolved"] * 3,
        )

    def test_blocked_source_with_different_txid_destination_matches_baseline(self):
        # The destinations of a blocked spend were recorded under their OWN
        # external_id (CSV import / separate sync), so they do NOT share the
        # source's (external_id, asset) group and the owned-fanout guard does not
        # fire. The blocked source must still post its disposal (matching the
        # deriver-off baseline) — dropping it would leave the spent coins in the
        # source while the destinations stay booked, inflating the profile total.
        consol = json.dumps(
            {
                "txid": "consol",
                "vin": [
                    {"txid": "pa", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}},
                    {"txid": "pb", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_B}},
                ],
                "vout": [{"n": 0, "scriptpubkey": SCRIPT_C, "value": 80_000_000}],
            }
        )
        rows = [
            _row("A", "inbound", 50_000_000_000, external_id="acqA"),
            _row("B", "inbound", 30_000_000_000, external_id="acqB"),
            _row("A", "outbound", 50_000_000_000, external_id="consol", raw_json=consol),
            _row("B", "outbound", 30_000_000_000, external_id="consol", raw_json=consol),
            # C's receipt recorded under its OWN provider id, not the spend txid.
            _row("C", "inbound", 80_000_000_000, external_id="exchange-deposit-77"),
        ]

        def _total(owned_index):
            state = build_tax_engine(PROFILE).build_ledger_state(
                TaxEngineLedgerInputs(
                    rows=rows,
                    wallet_refs_by_id=WALLET_REFS,
                    manual_pair_records=[],
                    owned_index=owned_index,
                )
            )
            return sum(
                float(totals["quantity"])
                for _, totals in state.wallet_holdings.items()
            ), [q["reason"] for q in state.quarantines]

        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        index.add_script(SCRIPT_C, _match("C", "Savings"))
        on_total, on_reasons = _total(index)
        off_total, _ = _total(None)
        # No inflation: deriver ON never books more coins than the baseline.
        self.assertAlmostEqual(on_total, off_total, places=6)
        # Both source spends are flagged for review.
        self.assertEqual(
            sorted(on_reasons), ["ownership_transfer_source_ambiguous"] * 2
        )

    def test_off_group_fanout_destination_does_not_restore_partial_pair(self):
        # Codex sidecar review: graph proves A paid B AND C, but only B shares
        # A's external_id and C was imported under a provider id. The A->B pair is
        # withheld so the deriver can decompose 1->N; when C's off-group inbound
        # makes that derivation ambiguous, restoring only A->B would quarantine
        # A/B as an implausible-fee transfer and still book C as an acquisition,
        # inflating holdings to 1.3 BTC. Leave the source on the conservative
        # disposal path, book the recorded receipts, and surface the review flag.
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        index.add_script(SCRIPT_C, _match("C", "Savings"))
        fan = json.dumps(
            {
                "txid": "fanout-tx",
                "vin": [{"txid": "pa", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}}],
                "vout": [
                    {"n": 0, "scriptpubkey": SCRIPT_B, "value": 50_000_000},
                    {"n": 1, "scriptpubkey": SCRIPT_C, "value": 30_000_000},
                ],
            }
        )
        rows = [
            _row("A", "inbound", BTC, external_id="acqA"),
            _row(
                "A",
                "outbound",
                80_000_000_000,
                external_id="fanout-tx",
                raw_json=fan,
            ),
            _row("B", "inbound", 50_000_000_000, external_id="fanout-tx"),
            _row("C", "inbound", 30_000_000_000, external_id="exchange-deposit-77"),
        ]
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=index,
            )
        )
        reasons = [q["reason"] for q in state.quarantines]
        self.assertIn("ownership_transfer_destination_ambiguous", reasons)
        self.assertNotIn("transfer_fee_implausible", reasons)
        self.assertNotIn(
            "transfer_in", [entry["entry_type"] for entry in state.entries]
        )
        holdings = {
            label: round(float(totals["quantity"]), 5)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(sum(holdings.values()), 1.0, places=6)
        self.assertAlmostEqual(holdings.get("Cold", 0.0), 0.2, places=6)
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.5, places=6)
        self.assertAlmostEqual(holdings.get("Savings", 0.0), 0.3, places=6)


class OwnershipDeriverHandlerTest(unittest.TestCase):
    def _seed(self, conn):
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-1", "Main", NOW),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            ("profile-1", "ws-1", "Default", "USD", "generic", 365, "FIFO", NOW),
        )
        conn.execute(
            """
            INSERT INTO accounts(
                id, workspace_id, profile_id, code, label, account_type, asset, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            ("acct-1", "ws-1", "profile-1", "treasury", "Treasury", "asset", "BTC", NOW),
        )
        for wid, label in (("wallet-a", "Cold"), ("wallet-b", "Hot")):
            conn.execute(
                """
                INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at)
                VALUES(?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (wid, "ws-1", "profile-1", "acct-1", label, "custom", "{}", NOW),
            )

    def _utxo(self, conn, wallet_id, address, txid, vout):
        conn.execute(
            """
            INSERT INTO wallet_utxos(
                id, workspace_id, profile_id, wallet_id, chain, network, asset,
                amount, txid, vout, outpoint, confirmation_status, address,
                branch_label, branch_index, address_index, first_seen_at, last_seen_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                f"utxo-{wallet_id}-{txid}-{vout}", "ws-1", "profile-1", wallet_id,
                "bitcoin", "main", "BTC", 50_000_000, txid, vout, f"{txid}:{vout}",
                "confirmed", address, "receive", 0, 0, NOW, NOW,
            ),
        )

    def _tx(self, conn, *, tx_id, wallet_id, direction, amount, external_id, raw_json, fee=0):
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id, "ws-1", "profile-1", wallet_id, external_id, f"fp-{tx_id}",
                NOW, direction, "BTC", amount, fee, "USD", 40000.0, None,
                "withdrawal" if direction == "outbound" else "deposit", raw_json, NOW,
            ),
        )

    def test_handler_derives_sync_gap_move_and_does_not_persist_pairs(self):
        addr_a = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        addr_b = "bc1q0xcqpzrky6eff2g52qdye53xkk9jxkvrh6yhyw"
        script_a = address_to_scriptpubkey(addr_a).hex()
        script_b = address_to_scriptpubkey(addr_b).hex()
        with tempfile.TemporaryDirectory(prefix="kassiber-owned-derive-") as tmp:
            conn = open_db(Path(tmp) / "data")
            self._seed(conn)
            # Cold owns the input it spends; Hot's address is known (light scan)
            # but Hot recorded NO inbound row — the sync-gap case.
            self._utxo(conn, "wallet-a", addr_a, "prevtx", 0)
            self._utxo(conn, "wallet-b", addr_b, "scan-only", 0)
            self._tx(
                conn, tx_id="acq", wallet_id="wallet-a", direction="inbound",
                amount=BTC, external_id="acq", raw_json="{}",
            )
            self._tx(
                conn, tx_id="cold-out", wallet_id="wallet-a", direction="outbound",
                amount=50 * BTC // 100, external_id="spend-tx", fee=1_000_000,
                raw_json=json.dumps(
                    {
                        "txid": "spend-tx",
                        "vin": [{"txid": "prevtx", "vout": 0,
                                 "prevout": {"scriptpubkey": script_a}}],
                        "vout": [{"n": 0, "scriptpubkey": script_b, "value": 50_000_000}],
                    }
                ),
            )
            conn.commit()

            profile = conn.execute(
                "SELECT * FROM profiles WHERE id = 'profile-1'"
            ).fetchone()
            state = handlers.build_ledger_state(conn, profile)

            reasons = {q["reason"] for q in state["quarantines"]}
            self.assertNotIn("owned_fanout_unresolved", reasons)
            entry_types = [e["entry_type"] for e in state["entries"]]
            self.assertIn("transfer_out", entry_types)
            self.assertIn("transfer_in", entry_types)
            holdings = {
                label: float(totals["quantity"])
                for (_, label, _, _), totals in state["wallet_holdings"].items()
            }
            # The MOVE landed 0.5 BTC of carried basis in the rowless Hot wallet.
            self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.5, places=6)

            # Derived pairs are recomputed each run — never written to the table.
            self.assertEqual(
                conn.execute("SELECT COUNT(*) FROM transaction_pairs").fetchone()[0], 0
            )

    def test_process_journals_persists_derived_move(self):
        # process_journals (the real `journals process` command) INSERTs journal
        # entries; journal_entries.transaction_id has an FK into transactions, so
        # the synthetic owned-derive: leg ids must be mapped to the real source
        # tx. Without that mapping this raises a FOREIGN KEY IntegrityError.
        addr_a = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        addr_b = "bc1q0xcqpzrky6eff2g52qdye53xkk9jxkvrh6yhyw"
        script_a = address_to_scriptpubkey(addr_a).hex()
        script_b = address_to_scriptpubkey(addr_b).hex()
        with tempfile.TemporaryDirectory(prefix="kassiber-owned-derive-persist-") as tmp:
            conn = open_db(Path(tmp) / "data")
            self._seed(conn)
            self._utxo(conn, "wallet-a", addr_a, "prevtx", 0)
            self._utxo(conn, "wallet-b", addr_b, "scan-only", 0)
            self._tx(
                conn, tx_id="acq", wallet_id="wallet-a", direction="inbound",
                amount=BTC, external_id="acq", raw_json="{}",
            )
            self._tx(
                conn, tx_id="cold-out", wallet_id="wallet-a", direction="outbound",
                amount=50 * BTC // 100, external_id="spend-tx", fee=1_000_000,
                raw_json=json.dumps(
                    {
                        "txid": "spend-tx",
                        "vin": [{"txid": "prevtx", "vout": 0,
                                 "prevout": {"scriptpubkey": script_a}}],
                        "vout": [{"n": 0, "scriptpubkey": script_b, "value": 50_000_000}],
                    }
                ),
            )
            conn.commit()

            # Must not raise (FK violation on the synthetic leg ids).
            handlers.process_journals(conn, "Main", "Default")

            rows = conn.execute(
                "SELECT entry_type, transaction_id FROM journal_entries"
            ).fetchall()
            types = sorted(r["entry_type"] for r in rows)
            self.assertIn("transfer_out", types)
            self.assertIn("transfer_in", types)
            # Every persisted entry references a real transaction row (FK holds).
            real_ids = {
                r["id"] for r in conn.execute("SELECT id FROM transactions").fetchall()
            }
            for r in rows:
                self.assertIn(r["transaction_id"], real_ids)
            audit = handlers.inspect_transfer_audit(conn, "Main", "Default")
            derived = [
                row
                for row in audit["same_asset_transfers"]
                if row.get("pairing_source") == "ownership_derived"
            ]
            self.assertEqual(len(derived), 1)
            self.assertEqual(derived[0]["from_wallet"], "Cold")
            self.assertEqual(derived[0]["to_wallet"], "Hot")


class DuplicateWalletLabelGuardTest(unittest.TestCase):
    def test_warns_on_shared_label(self):
        refs = {
            "w1": {"id": "w1", "label": "Hot"},
            "w2": {"id": "w2", "label": "Hot"},
            "w3": {"id": "w3", "label": "Cold"},
        }
        warnings = handlers._duplicate_label_warnings(refs)
        self.assertEqual(len(warnings), 1)
        warning = warnings[0]
        self.assertEqual(warning["code"], "duplicate_wallet_label")
        self.assertEqual(warning["label"], "Hot")
        self.assertEqual(warning["wallet_ids"], ["w1", "w2"])

    def test_unique_labels_produce_no_warning(self):
        refs = {
            "w1": {"id": "w1", "label": "Hot"},
            "w2": {"id": "w2", "label": "Cold"},
        }
        self.assertEqual(handlers._duplicate_label_warnings(refs), [])


class RecordedFanoutEngineTest(unittest.TestCase):
    """Liquid (no on-chain graph) 1->N self-transfer through the real engine."""

    def _liquid_fanout_rows(self, fee=0):
        # A (Cold) fans 0.8 LBTC to B (Hot) 0.5 and C (Savings) 0.3, all recorded
        # under one txid. No vin/vout (Liquid amounts are confidential), so the
        # address-ownership deriver can't read it — the recorded-fanout
        # decomposer pairs it from the rows.
        return [
            _row("A", "inbound", 80 * BTC // 100 + fee, external_id="acqA", asset="LBTC"),
            _row("A", "outbound", 80 * BTC // 100, external_id="lq", fee=fee, asset="LBTC"),
            _row("B", "inbound", 50 * BTC // 100, external_id="lq", asset="LBTC"),
            _row("C", "inbound", 30 * BTC // 100, external_id="lq", asset="LBTC"),
        ]

    def test_liquid_fanout_books_moves_without_index(self):
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=self._liquid_fanout_rows(fee=2_000_000),
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=None,  # no graph, no index — rows alone
            )
        )
        self.assertNotIn(
            "owned_fanout_unresolved", {q["reason"] for q in state.quarantines}
        )
        entry_types = sorted(e["entry_type"] for e in state.entries)
        self.assertEqual(entry_types.count("transfer_out"), 2)
        self.assertEqual(entry_types.count("transfer_in"), 2)
        holdings = {
            label: round(float(totals["quantity"]), 5)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        # Basis carried: 0.5 -> Hot, 0.3 -> Savings, source drained. No disposal.
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.5, places=6)
        self.assertAlmostEqual(holdings.get("Savings", 0.0), 0.3, places=6)
        self.assertNotIn("disposal", entry_types)

    def test_liquid_fanout_with_unsynced_destination_quarantines(self):
        # C never synced -> the recorded inbounds don't conserve, so the
        # decomposer declines and the spend stays on its review path (not booked
        # as a partial/incorrect split).
        rows = self._liquid_fanout_rows(fee=0)[:-1]  # drop C's inbound
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=None,
            )
        )
        entry_types = [e["entry_type"] for e in state.entries]
        self.assertNotIn("transfer_in", entry_types)
        self.assertTrue(state.quarantines)  # flagged for review, nothing mis-booked


class MultiSourceConsolidationEngineTest(unittest.TestCase):
    """Through the real engine: a cross-wallet consolidation books carrying MOVEs."""

    def _run(self, rows):
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        index.add_script(SCRIPT_C, _match("C", "Savings"))
        return build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=index,
            )
        )

    def test_consolidation_with_fee_books_fee_once(self):
        # in_A=0.5, in_B=0.3, whole-tx fee=0.001 stamped on BOTH rows. The
        # recorded amounts are net of the fee (0.499 / 0.299) and the graph
        # destination is 0.799. If the fee were double-counted the pool would
        # end at 0.798; booked once it is 0.799.
        consol = json.dumps(
            {
                "txid": "consol-fee",
                "vin": [
                    {"txid": "pa", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}},
                    {"txid": "pb", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_B}},
                ],
                "vout": [{"n": 0, "scriptpubkey": SCRIPT_C, "value": 79_900_000}],
            }
        )
        rows = [
            _row("A", "inbound", 50_000_000_000, external_id="acqA"),
            _row("B", "inbound", 30_000_000_000, external_id="acqB"),
            _row("A", "outbound", 49_900_000_000, external_id="consol-fee",
                 raw_json=consol, fee=100_000_000),
            _row("B", "outbound", 29_900_000_000, external_id="consol-fee",
                 raw_json=consol, fee=100_000_000),
            _row("C", "inbound", 79_900_000_000, external_id="consol-fee"),
        ]
        state = self._run(rows)
        self.assertEqual(state.quarantines, [])
        entry_types = [e["entry_type"] for e in state.entries]
        self.assertIn("transfer_out", entry_types)
        self.assertIn("transfer_in", entry_types)
        self.assertNotIn("disposal", entry_types)
        # Exactly one fee leg disposes the miner fee.
        self.assertEqual(entry_types.count("transfer_fee"), 1)
        holdings = {
            label: round(float(totals["quantity"]), 6)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(holdings.get("Cold", 0.0), 0.0, places=6)
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.0, places=6)
        self.assertAlmostEqual(holdings.get("Savings", 0.0), 0.799, places=6)
        self.assertAlmostEqual(sum(holdings.values()), 0.799, places=6)

    def test_consolidation_with_sync_gapped_destination_books_moves(self):
        # Same consolidation but the destination never synced an inbound. The
        # graph still proves C owns the output, so the legs are synthesized.
        consol = json.dumps(
            {
                "txid": "consol-gap",
                "vin": [
                    {"txid": "pa", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}},
                    {"txid": "pb", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_B}},
                ],
                "vout": [{"n": 0, "scriptpubkey": SCRIPT_C, "value": 80_000_000}],
            }
        )
        rows = [
            _row("A", "inbound", 50_000_000_000, external_id="acqA"),
            _row("B", "inbound", 30_000_000_000, external_id="acqB"),
            _row("A", "outbound", 50_000_000_000, external_id="consol-gap", raw_json=consol),
            _row("B", "outbound", 30_000_000_000, external_id="consol-gap", raw_json=consol),
        ]
        state = self._run(rows)
        self.assertEqual(state.quarantines, [])
        entry_types = [e["entry_type"] for e in state.entries]
        self.assertIn("transfer_in", entry_types)
        self.assertNotIn("disposal", entry_types)
        holdings = {
            label: round(float(totals["quantity"]), 6)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(holdings.get("Savings", 0.0), 0.8, places=6)
        self.assertAlmostEqual(sum(holdings.values()), 0.8, places=6)

    def test_off_group_nonexact_receipt_does_not_double_count(self):
        # A+B -> C consolidation (graph dest 0.8) where C recorded its receipt
        # off-group at a slightly different amount (0.79999). The deriver must
        # decline (the receipt is near the spend time), so C is credited once via
        # its real receipt — NOT 0.8 (synthetic legs) PLUS 0.79999 (~1.6 total).
        consol = json.dumps(
            {
                "txid": "f4consol",
                "vin": [{"txid": "pa", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}},
                        {"txid": "pb", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_B}}],
                "vout": [{"n": 0, "scriptpubkey": SCRIPT_C, "value": 80_000_000}],
            }
        )
        rows = [
            _row("A", "inbound", 50_000_000_000, external_id="acqA"),
            _row("B", "inbound", 30_000_000_000, external_id="acqB"),
            _row("A", "outbound", 50_000_000_000, external_id="f4consol", raw_json=consol),
            _row("B", "outbound", 30_000_000_000, external_id="f4consol", raw_json=consol),
            _row("C", "inbound", 79_999_000_000, external_id="exchange-deposit-9"),
        ]
        state = self._run(rows)
        holdings = {
            label: round(float(totals["quantity"]), 6)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(holdings.get("Savings", 0.0), 0.79999, places=6)
        self.assertAlmostEqual(sum(holdings.values()), 0.79999, places=6)

    def test_grouped_consolidation_gate_quarantines_atomically(self):
        # A+B -> C is derived as a grouped consolidation. If B lacks enough lots,
        # the gate must not book A's sibling MOVE and drop C's recorded receipt;
        # the entire derived group is deferred for review.
        consol = json.dumps(
            {
                "txid": "consol-partial-lots",
                "vin": [
                    {"txid": "pa", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}},
                    {"txid": "pb", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_B}},
                ],
                "vout": [{"n": 0, "scriptpubkey": SCRIPT_C, "value": 80_000_000}],
            }
        )
        rows = [
            _row("A", "inbound", 50_000_000_000, external_id="acqA"),
            _row("B", "inbound", 10_000_000_000, external_id="acqB"),
            _row(
                "A",
                "outbound",
                50_000_000_000,
                external_id="consol-partial-lots",
                raw_json=consol,
            ),
            _row(
                "B",
                "outbound",
                30_000_000_000,
                external_id="consol-partial-lots",
                raw_json=consol,
            ),
            _row("C", "inbound", 80_000_000_000, external_id="consol-partial-lots"),
        ]

        state = self._run(rows)

        entry_types = [entry["entry_type"] for entry in state.entries]
        self.assertNotIn("transfer_out", entry_types)
        self.assertNotIn("transfer_in", entry_types)
        reasons = {q["reason"] for q in state.quarantines}
        self.assertIn("insufficient_lots", reasons)
        self.assertIn("derived_transfer_group_blocked", reasons)
        reasons_by_id = {q["transaction_id"]: q["reason"] for q in state.quarantines}
        self.assertEqual(
            reasons_by_id["C-inbound-consol-partial-lots"],
            "derived_transfer_group_blocked",
        )
        holdings = {
            label: round(float(totals["quantity"]), 6)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(holdings.get("Savings", 0.0), 0.0, places=6)


class SameTimestampTransferOrderingEngineTest(unittest.TestCase):
    def test_same_timestamp_transfer_chain_books_funding_move_first(self):
        # Input order intentionally places Hot's spend before the Cold->Hot
        # funding MOVE. Old same-timestamp tiebreaking followed that stream order
        # and quarantined the Hot spend as insufficient_lots. The gate now orders
        # same-time transfers by wallet dependency.
        rows = [
            _row("A", "inbound", BTC, external_id="acq"),
            _row("B", "outbound", 60_000_000_000, external_id="hot-to-savings"),
            _row("A", "outbound", 60_000_000_000, external_id="cold-to-hot"),
            _row("B", "inbound", 60_000_000_000, external_id="cold-to-hot"),
            _row("C", "inbound", 60_000_000_000, external_id="hot-to-savings"),
        ]

        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=None,
            )
        )

        self.assertEqual(state.quarantines, [])
        entry_types = [entry["entry_type"] for entry in state.entries]
        self.assertEqual(entry_types.count("transfer_out"), 2)
        self.assertEqual(entry_types.count("transfer_in"), 2)
        holdings = {
            label: round(float(totals["quantity"]), 6)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(holdings.get("Cold", 0.0), 0.4, places=6)
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.0, places=6)
        self.assertAlmostEqual(holdings.get("Savings", 0.0), 0.6, places=6)


class PartialPaymentWithholdingEngineTest(unittest.TestCase):
    """Fix: a same-txid 1-out/1-in pair that ALSO pays a (small) external party.

    detect_intra_transfers would pair the owned leg and silently fold the
    external payment into the implied MOVE fee (sub-ceiling, so not even
    quarantined). Withholding the pair lets the graph deriver book the owned
    MOVE and keep the external residual as a real taxable disposal.
    """

    def test_external_residual_is_taxed_not_absorbed_as_fee(self):
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        index.add_script(SCRIPT_B, _match("B", "Hot"))
        # A spends 0.5 to B (own) + 0.002 to an external recipient + 0.0001 fee.
        # The external leg (0.002) is under the swap-fee ceiling for a 0.502
        # outbound, so without the withhold it is absorbed as a MOVE fee.
        spend = json.dumps(
            {
                "txid": "partial-pp",
                "vin": [{"txid": "prevtx", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}}],
                "vout": [
                    {"n": 0, "scriptpubkey": SCRIPT_B, "value": 50_000_000},  # owned 0.5
                    {"n": 1, "scriptpubkey": SCRIPT_EXT, "value": 200_000},  # external 0.002
                ],
            }
        )
        rows = [
            _row("A", "inbound", 50_210_000_000, external_id="acq"),
            _row("A", "outbound", 50_200_000_000, external_id="partial-pp",
                 raw_json=spend, fee=10_000_000),
            # B recorded its receipt under the SAME txid, so detect_intra pairs it.
            _row("B", "inbound", 50_000_000_000, external_id="partial-pp"),
        ]
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(
                rows=rows,
                wallet_refs_by_id=WALLET_REFS,
                manual_pair_records=[],
                owned_index=index,
            )
        )
        self.assertEqual(state.quarantines, [])
        entry_types = [e["entry_type"] for e in state.entries]
        self.assertIn("transfer_out", entry_types)
        self.assertIn("transfer_in", entry_types)
        # The external payment is now a real disposal, not folded into the fee.
        self.assertIn("disposal", entry_types)
        disposed = sum(
            abs(float(e["quantity"]))
            for e in state.entries
            if e["entry_type"] == "disposal"
        )
        self.assertAlmostEqual(disposed, 0.002, places=6)
        holdings = {
            label: round(float(totals["quantity"]), 6)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.5, places=6)
        # 0.5021 acquired - 0.5 moved - 0.002 sold - 0.0001 fee == 0.
        self.assertAlmostEqual(holdings.get("Cold", 0.0), 0.0, places=6)

    def test_withhold_rolls_back_when_owned_output_is_ambiguous(self):
        # The owned output is paid to a script owned by TWO of the user's wallets
        # (shared descriptor / reused address), so the ownership deriver cannot
        # route the leg and DECLINES. The withhold must roll the pair back to its
        # original self-transfer instead of orphaning it into a full disposal +
        # phantom acquisition (which carried no quarantine). Falling back here
        # absorbs the small external leg as fee (the documented sub-ceiling P2),
        # but never destroys basis or invents a disposal.
        index = OwnedIndex()
        index.add_script(SCRIPT_A, _match("A", "Cold"))
        shared = "0014" + "dd" * 20
        index.add_script(shared, _match("B", "Hot"))
        index.add_script(shared, _match("C", "Savings"))  # ambiguous: two owners
        spend = json.dumps(
            {
                "txid": "ambig-pp",
                "vin": [{"txid": "pv", "vout": 0, "prevout": {"scriptpubkey": SCRIPT_A}}],
                "vout": [
                    {"n": 0, "scriptpubkey": shared, "value": 50_000_000},  # owned, ambiguous
                    {"n": 1, "scriptpubkey": SCRIPT_EXT, "value": 200_000},  # external 0.002
                ],
            }
        )
        rows = [
            _row("A", "inbound", 50_210_000_000, external_id="acq"),
            _row("A", "outbound", 50_200_000_000, external_id="ambig-pp",
                 raw_json=spend, fee=10_000_000),
            _row("B", "inbound", 50_000_000_000, external_id="ambig-pp"),
        ]
        state = build_tax_engine(PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(rows=rows, wallet_refs_by_id=WALLET_REFS,
                                  manual_pair_records=[], owned_index=index)
        )
        entry_types = [e["entry_type"] for e in state.entries]
        # The self-transfer MOVE is booked (rolled back), NOT a phantom acquisition.
        self.assertIn("transfer_in", entry_types)
        self.assertIn("transfer_out", entry_types)
        # No phantom acquisition at the destination: only A's real funding acq.
        self.assertEqual(entry_types.count("acquisition"), 1)
        holdings = {
            label: round(float(totals["quantity"]), 6)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        # Hot got 0.5 of CARRIED basis (a MOVE), not a fresh 0.5 acquisition; the
        # source is not over-disposed.
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.5, places=6)
        self.assertAlmostEqual(holdings.get("Cold", 0.0), 0.0, places=6)
        # No silent phantom: total holdings == acquired - fee-absorbed outflow.
        self.assertLessEqual(sum(holdings.values()), 0.5021)


class AustrianSelfTransferEngineTest(unittest.TestCase):
    """AT MOVE-fee disposal must carry a regime or rp2 aborts the whole asset."""

    AT_PROFILE = {
        "id": "profile-1", "workspace_id": "ws-1", "label": "AT",
        "fiat_currency": "EUR", "tax_country": "at", "tax_long_term_days": 365,
        "gains_algorithm": "moving_average_at",
    }

    def _at_row(self, wid, direction, amount_msat, occurred_at, ext, fee=0, rate=40000.0):
        ref = WALLET_REFS[wid]
        return {
            "id": f"{wid}-{direction}-{ext}", "workspace_id": "ws-1", "profile_id": "profile-1",
            "wallet_id": wid, "wallet_label": ref["label"],
            "wallet_account_id": ref["wallet_account_id"], "account_code": ref["account_code"],
            "account_label": ref["account_label"], "external_id": ext,
            "occurred_at": occurred_at, "created_at": occurred_at, "direction": direction,
            "asset": "BTC", "amount": amount_msat, "fee": fee, "fiat_currency": "EUR",
            "fiat_rate": rate, "fiat_rate_exact": str(int(rate)), "fiat_value": None,
            "kind": "withdrawal" if direction == "outbound" else "deposit",
            "description": f"{wid} {direction}", "note": None, "raw_json": "{}", "excluded": 0,
        }

    def test_mixed_alt_neu_self_transfer_fee_does_not_abort_report(self):
        # A long-term Austrian holder with one Altvermoegen lot (pre-2021-03-01)
        # and one Neuvermoegen lot (post) does an ordinary self-transfer with a
        # miner fee. The fee is a taxable disposal; without a regime tag rp2's
        # moving-average raises "Ambiguous Austrian disposal" and the WHOLE BTC
        # report aborts. It must book cleanly instead.
        rows = [
            self._at_row("A", "inbound", 30_000_000_000, "2020-06-01T00:00:00Z", "altacq", rate=10000.0),
            self._at_row("A", "inbound", 40_000_000_000, "2024-06-01T00:00:00Z", "neuacq", rate=60000.0),
            self._at_row("A", "outbound", 50_000_000_000, "2025-02-01T00:00:00Z", "selfmove",
                         fee=100_000_000, rate=60000.0),
            self._at_row("B", "inbound", 50_000_000_000, "2025-02-01T00:00:00Z", "selfmove", rate=60000.0),
        ]
        # Must not raise AppError("Ambiguous Austrian disposal").
        state = build_tax_engine(self.AT_PROFILE).build_ledger_state(
            TaxEngineLedgerInputs(rows=rows, wallet_refs_by_id=WALLET_REFS,
                                  manual_pair_records=[], owned_index=None)
        )
        entry_types = [e["entry_type"] for e in state.entries]
        self.assertIn("transfer_out", entry_types)
        self.assertIn("transfer_in", entry_types)
        self.assertIn("transfer_fee", entry_types)
        holdings = {
            label: round(float(totals["quantity"]), 6)
            for (_, label, _, _), totals in state.wallet_holdings.items()
        }
        # 0.7 held, 0.5 moved to Hot, 0.001 fee left the pool.
        self.assertAlmostEqual(holdings.get("Cold", 0.0), 0.199, places=6)
        self.assertAlmostEqual(holdings.get("Hot", 0.0), 0.5, places=6)


if __name__ == "__main__":
    unittest.main()

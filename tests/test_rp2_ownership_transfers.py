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


def _row(wallet_id, direction, amount, *, external_id, raw_json="{}", fee=0):
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
        "asset": "BTC",
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

    def test_fanout_quarantined_without_deriver(self):
        state = self._run(owned_index=None)
        reasons = {q["reason"] for q in state.quarantines}
        self.assertIn("owned_fanout_unresolved", reasons)

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

    def test_consolidation_with_recorded_destination_does_not_inflate(self):
        # A+B -> C, all owned, C's inbound recorded under the spend's txid. The
        # per-wallet amounts are unreliable (the fee is double-counted across the
        # contributing wallets) so the deriver declines (source_ambiguous). The
        # whole same-txid group is a recorded consolidation, so the existing
        # owned-fanout guard holds back EVERY leg. The old block-and-remove path
        # dropped only the source outbounds, leaving C's acquisition booked and
        # doubling the profile total to 1.6 — it must stay 0.8.
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
        self.assertAlmostEqual(holdings.get("Savings", 0.0), 0.0, places=6)
        entry_types = [entry["entry_type"] for entry in state.entries]
        self.assertNotIn("disposal", entry_types)
        self.assertNotIn("transfer_in", entry_types)
        self.assertEqual(
            sorted(q["reason"] for q in state.quarantines),
            ["owned_fanout_unresolved"] * 3,
        )

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


if __name__ == "__main__":
    unittest.main()

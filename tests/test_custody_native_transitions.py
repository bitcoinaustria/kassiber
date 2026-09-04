"""Hard HTLC facts reach custody and tax without authored pair records."""

import json
import unittest
import tempfile
from pathlib import Path
from unittest.mock import patch

from kassiber.core.engines import GenericRP2TaxEngine
from kassiber.core.custody_evidence import build_canonical_quantity_input, enriched_quantity_rows
from kassiber.core.custody_interpreters import (
    _observations_by_transaction, _pair_claims, compile_custody_interpreters,
)
from tests.custody_tax_helpers import authoritative_chain_observation, finalized_tax_inputs
from tests.custody_tax_helpers import persist_authoritative_chain_observation
from kassiber.core import custody_journal
from kassiber.db import open_db
from tests import test_lwk_observer


PROFILE = {
    "id": "profile", "workspace_id": "workspace", "label": "Default",
    "fiat_currency": "USD", "tax_country": "generic", "gains_algorithm": "FIFO",
}


def native_transition_rows(role="claim"):
    source, target, _ = test_lwk_observer.LwkDescriptorContractTest()._htlc_matcher_rows(role)
    source["fee"] = 10_000
    source.setdefault("external_id", "native-ln-payment")
    refs = {}
    rows = []
    for row in (source, target):
        wallet = row["wallet_id"]
        refs[wallet] = {
            "id": wallet, "label": wallet, "wallet_account_id": "account",
            "account_code": "treasury", "account_label": "Treasury",
        }
        row.update(
            workspace_id="workspace", profile_id="profile", wallet_label=wallet,
            wallet_account_id="account", account_code="treasury", account_label="Treasury",
            fiat_currency="USD", fiat_rate=40_000, fiat_rate_exact="40000", fiat_value=None,
            created_at=row["occurred_at"],
            note=None, description="Native HTLC transfer",
        )
        if row["wallet_kind"] != "lnd":
            row = authoritative_chain_observation(row, observer_kind="lwk", fee_attribution="exact")
            row["observation_observer_kinds_json"] = '["lwk"]'
        rows.append(row)
    acquisition = {
        **rows[0], "id": "acquisition", "external_id": "initial-acquisition",
        "direction": "inbound", "kind": "deposit", "payment_hash": None,
        "payment_hash_source": None, "amount": 100_010_000, "fee": 0,
        "occurred_at": "2020-01-01T00:00:00Z", "created_at": "2020-01-01T00:00:00Z",
        "raw_json": "{}",
        "fiat_rate": 20_000, "fiat_rate_exact": "20000",
    }
    for key in list(acquisition):
        if key.startswith("observation_"):
            acquisition.pop(key)
    return [acquisition, *rows], refs


class NativeTransitionEngineTest(unittest.TestCase):
    def test_exact_claim_and_refund_book_without_authored_review(self):
        for role in ("claim", "refund"):
            with self.subTest(role=role):
                rows, refs = native_transition_rows(role)
                inputs = finalized_tax_inputs(
                    PROFILE, rows=rows, wallet_refs_by_id=refs,
                )
                state = GenericRP2TaxEngine(PROFILE).build_ledger_state(inputs)
                self.assertEqual(state.quarantines, [])
                types = [entry["entry_type"] for entry in state.entries]
                if role == "refund":
                    self.assertIn("transfer_in", types)
                    self.assertIn("transfer_out", types)
                    self.assertNotIn("disposal", types)
                    fees = [e for e in state.entries if e["entry_type"] == "transfer_fee"]
                    self.assertEqual(len(fees), 1)
                    self.assertAlmostEqual(float(fees[0]["quantity"]), -0.0000051)
                else:
                    # Generic BTC/LBTC carrying is represented by neutral
                    # disposal/acquisition entries with the source basis.
                    cross = inputs.finalized_tax_projection.cross_asset_pairs
                    self.assertEqual(len(cross), 1)
                    self.assertEqual(cross[0]["policy"], "carrying-value")
                    disposal = next(e for e in state.entries if e["entry_type"] == "disposal")
                    self.assertEqual(disposal["gain_loss"], 0)
                    receipt = next(e for e in state.entries if e["transaction_id"] == "claim")
                    self.assertAlmostEqual(float(receipt["fiat_value"]), 20.002)
                self.assertAlmostEqual(sum(float(v["quantity"]) for v in state.wallet_holdings.values()), 0.000995)

    def test_same_hard_evidence_respects_disabled_cross_rail_carry_policy(self):
        rows, refs = native_transition_rows()
        profile = {**PROFILE, "bitcoin_rail_carrying_value": False}
        inputs = finalized_tax_inputs(profile, rows=rows, wallet_refs_by_id=refs)
        cross = inputs.finalized_tax_projection.cross_asset_pairs
        self.assertEqual(len(cross), 1)
        self.assertEqual(cross[0]["policy"], "taxable")
        state = GenericRP2TaxEngine(profile).build_ledger_state(inputs)
        self.assertEqual(state.quarantines, [])
        disposal = next(e for e in state.entries if e["entry_type"] == "disposal")
        self.assertAlmostEqual(float(disposal["gain_loss"]), 19.798)
        self.assertAlmostEqual(sum(float(v["quantity"]) for v in state.wallet_holdings.values()), 0.000995)

    def test_fee_inclusive_source_cannot_create_partial_move_and_new_acquisition(self):
        rows, refs = native_transition_rows()
        rows[1].update(amount_includes_fee=1, fee=1_000_000)
        state = GenericRP2TaxEngine(PROFILE).build_ledger_state(finalized_tax_inputs(
            PROFILE, rows=rows, wallet_refs_by_id=refs,
        ))
        held = {
            item["transaction_id"] for item in state.quarantines
            if item["reason"] == "native_transition_amount_mismatch"
        }
        self.assertEqual(held, {"send", "claim"})
        self.assertFalse(any(e["transaction_id"] in held for e in state.entries))

    def test_shortfall_across_dates_holds_both_anchors_instead_of_wrong_fee_year(self):
        for target_time in ("2026-01-02T12:00:00Z", "unknown"):
            with self.subTest(target_time=target_time):
                rows, refs = native_transition_rows("refund")
                rows[1]["occurred_at"] = "2025-12-31T12:00:00Z"
                rows[2]["occurred_at"] = target_time
                state = GenericRP2TaxEngine(PROFILE).build_ledger_state(finalized_tax_inputs(
                    PROFILE, rows=rows, wallet_refs_by_id=refs,
                ))
                holds = {
                    item["transaction_id"] for item in state.quarantines
                    if item["reason"] == "native_transition_fee_timing_unresolved"
                }
                self.assertEqual(holds, {"send", "claim"})
                self.assertFalse(any(e["transaction_id"] in holds for e in state.entries))

    def test_distinct_native_routes_keep_distinct_projection_ids(self):
        rows, refs = native_transition_rows()
        rows[0]["amount"] *= 2
        source = {**rows[1], "id": "send-second", "external_id": "native-ln-payment-second", "payment_hash": "12" * 32}
        target = {**rows[2], "id": "claim-second", "external_id": "88" * 32, "payment_hash": "12" * 32}
        raw = json.loads(target["raw_json"])
        raw["txid"] = target["external_id"]
        raw["vin"][0]["txid"] = "77" * 32
        raw["htlc_spend"].update(payment_hash=target["payment_hash"], funding_txid="77" * 32)
        target["raw_json"] = json.dumps(raw)
        target = authoritative_chain_observation(target, observer_kind="lwk")
        rows.extend((source, target))
        for country in ("generic", "at"):
            with self.subTest(country=country):
                profile = {**PROFILE, "tax_country": country}
                inputs = finalized_tax_inputs(profile, rows=rows, wallet_refs_by_id=refs)
                pairs = inputs.finalized_tax_projection.cross_asset_pairs
                self.assertEqual(len(pairs), 2)
                self.assertEqual(len({p["pair_id"] for p in pairs}), 2)
                state = GenericRP2TaxEngine(profile).build_ledger_state(inputs)
                self.assertEqual(state.quarantines, [])
                self.assertAlmostEqual(sum(float(v["quantity"]) for v in state.wallet_holdings.values()), 0.00199)


class NativeTransitionAuthorityTest(unittest.TestCase):
    def compile(self, rows, refs, **kwargs):
        canonical = build_canonical_quantity_input(enriched_quantity_rows(rows))
        return compile_custody_interpreters(
            rows, canonical, profile=PROFILE, wallet_refs_by_id=refs, **kwargs,
        )

    def assert_no_native_claims(self, compiled):
        self.assertFalse(any(c.reason == "native_htlc_transition" for c in compiled.claims))

    def test_untrusted_stale_and_partial_proofs_never_auto_resolve(self):
        for scenario in ("missing_authority", "stale_quantity", "stale_graph", "wrong_observer", "partial_refund"):
            with self.subTest(scenario=scenario):
                rows, refs = native_transition_rows("refund" if scenario == "partial_refund" else "claim")
                if scenario == "missing_authority":
                    rows[-1].pop("observation_authority_version")
                elif scenario == "stale_quantity":
                    rows[-1]["amount"] -= 1
                elif scenario == "stale_graph":
                    raw = json.loads(rows[-1]["raw_json"])
                    raw["changed"] = True
                    rows[-1]["raw_json"] = json.dumps(raw)
                elif scenario == "wrong_observer":
                    rows[-1]["observation_observer_kinds_json"] = '["bitcoinrpc"]'
                else:
                    raw = json.loads(rows[1]["raw_json"])
                    raw["vout"][0]["value_sats"] = 50_000
                    rows[1]["raw_json"] = json.dumps(raw)
                    rows[1] = authoritative_chain_observation(rows[1], observer_kind="lwk")
                self.assert_no_native_claims(self.compile(rows, refs))

    def test_dismissals_and_occupied_anchors_are_respected(self):
        for occupancy in (
            {"swap_dismissals": [{"out_transaction_id": "send", "in_transaction_id": "claim"}]},
            {"component_transaction_ids": ("send",)},
            {"loan_legs": [{"transaction_id": "send"}]},
            {"channel_roles": {"send": "channel_open"}},
        ):
            with self.subTest(occupancy=occupancy):
                rows, refs = native_transition_rows()
                self.assert_no_native_claims(self.compile(rows, refs, **occupancy))

    def test_hidden_duplicate_keeps_original_hash_and_refund_conflicts(self):
        for role in ("claim", "refund"):
            for suppress in ("component", "dismissal"):
                with self.subTest(role=role, suppress=suppress):
                    rows, refs = native_transition_rows(role)
                    rows.append({**rows[-1], "id": "competing-receipt"})
                    kwargs = (
                        {"component_transaction_ids": ("competing-receipt",)}
                        if suppress == "component"
                        else {"swap_dismissals": [{"out_transaction_id": "send", "in_transaction_id": "competing-receipt"}]}
                    )
                    self.assert_no_native_claims(self.compile(rows, refs, **kwargs))

    def test_expired_dismissal_allows_current_native_proof(self):
        rows, refs = native_transition_rows()
        compiled = self.compile(rows, refs, swap_dismissals=[{
            "out_transaction_id": "send", "in_transaction_id": "claim", "expires_at": "2000-01-01T00:00:00Z",
        }])
        self.assertTrue(any(c.reason == "native_htlc_transition" for c in compiled.claims))

    def test_raw_only_refund_attestation_reaches_native_interpreter(self):
        rows, refs = native_transition_rows("refund")
        for key in ("swap_refund_funding_txid", "swap_refund_funding_vout", "swap_refund_funding_outpoint"):
            rows[-1].pop(key, None)
        compiled = self.compile(rows, refs)
        self.assertTrue(any(c.reason == "native_htlc_transition" for c in compiled.claims))

    def test_source_label_cannot_forge_a_native_claim(self):
        rows, refs = native_transition_rows()
        canonical = build_canonical_quantity_input(enriched_quantity_rows(rows))
        observations = _observations_by_transaction(canonical)
        claims, _, _ = _pair_claims(
            [{"out": rows[1], "in": rows[2], "source": "native_htlc_transition", "kind": "reverse-submarine-swap", "policy": "carrying-value"}],
            observations, excluded_transaction_ids=set(), wallet_refs_by_id=refs,
            physical_scopes_by_anchor={},
        )
        self.assertEqual(claims, ())

    def test_ordinary_books_do_not_run_swap_matching(self):
        rows, refs = native_transition_rows()
        with patch("kassiber.core.custody_native_transitions.suggest_swap_candidates", side_effect=AssertionError("unexpected matcher")):
            self.compile(rows[:1], refs)


class NativeTransitionDatabaseTest(unittest.TestCase):
    def test_journal_uses_persisted_authority_without_writing_review_records(self):
        for role in ("claim", "refund"):
            with self.subTest(role=role), tempfile.TemporaryDirectory() as tmp:
                conn = open_db(Path(tmp) / "data")
                try:
                    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('workspace','W','2020')")
                    conn.execute(
                        "INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,gains_algorithm,created_at) "
                        "VALUES('profile','workspace','P','USD','generic','FIFO','2020')"
                    )
                    rows, refs = native_transition_rows(role)
                    for wallet_id, ref in refs.items():
                        kind = next(row["wallet_kind"] for row in rows if row["wallet_id"] == wallet_id)
                        config = {"chain": "lightning", "network": "regtest"} if kind == "lnd" else {"chain": "liquid", "network": "elementsregtest"}
                        conn.execute(
                            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'workspace','profile',?,?,?,'2020')",
                            (wallet_id, ref["label"], kind, json.dumps(config)),
                        )
                    columns = {row["name"] for row in conn.execute("PRAGMA table_info(transactions)")}
                    for original in rows:
                        row = {key: value for key, value in original.items() if key in columns}
                        row["fingerprint"] = "fp-" + row["id"]
                        if not isinstance(row["raw_json"], str):
                            row["raw_json"] = json.dumps(row["raw_json"])
                        names = tuple(row)
                        conn.execute(
                            f"INSERT INTO transactions ({','.join(names)}) VALUES ({','.join('?' for _ in names)})",
                            tuple(row[name] for name in names),
                        )
                        if original["id"] != "acquisition" and original["wallet_kind"] != "lnd":
                            persist_authoritative_chain_observation(conn, row["id"], observer_kind="lwk")
                    profile = conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
                    state = custody_journal.build_ledger_state(conn, profile)
                    self.assertEqual(state["quarantines"], [])
                    self.assertTrue(any(
                        decision.reason == "native_htlc_transition"
                        for decision in state["custody_quantity"].projection.decisions
                    ))
                    self.assertEqual(conn.execute("SELECT COUNT(*) FROM transaction_pairs").fetchone()[0], 0)
                    self.assertEqual(conn.execute("SELECT COUNT(*) FROM custody_components").fetchone()[0], 0)
                    conn.execute(
                        "INSERT INTO transaction_pair_dismissals(id,workspace_id,profile_id,out_transaction_id,in_transaction_id,created_at) "
                        "VALUES('dismissal','workspace','profile','send','claim','2026-01-01T00:00:00Z')"
                    )
                    dismissed = custody_journal.build_ledger_state(conn, profile)
                    self.assertFalse(any(
                        decision.reason == "native_htlc_transition"
                        for decision in dismissed["custody_quantity"].projection.decisions
                    ))
                finally:
                    conn.close()

"""A txid shared by several rows never resolves by guessing.

CoinJoins and batched deposits import one row per wallet and direction under
one txid. References fail closed with every candidate, and a qualified
``<txid>:<wallet>:<in|out>`` reference selects exactly one row.
"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from kassiber.cli.handlers import resolve_transaction
from kassiber.core.transaction_lookup import find_transaction
from kassiber.core.ui_snapshot import build_transactions_resolve_snapshot
from kassiber.daemon import _tool_result_from_app_error
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError

NOW = "2026-01-01T00:00:00Z"
TXID = "a" * 64


class TransactionLookupTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="kassiber-tx-lookup-")
        self.addCleanup(tmp.cleanup)
        self.conn = open_db(str(Path(tmp.name) / "data"))
        self.addCleanup(self.conn.close)
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Main', ?)",
            (NOW,),
        )
        self.conn.execute(
            "INSERT INTO profiles(id, workspace_id, label, created_at) "
            "VALUES('profile', 'ws', 'Book', ?)",
            (NOW,),
        )
        for wallet_id, label in (
            ("wallet-a", "Mixer Out"),
            ("wallet-b", "Cold B"),
            ("wallet-c", "Cold C"),
        ):
            self.conn.execute(
                "INSERT INTO wallets(id, workspace_id, profile_id, label, kind, "
                "config_json, created_at) VALUES(?, 'ws', 'profile', ?, 'custom', '{}', ?)",
                (wallet_id, label, NOW),
            )
        for tx_id, wallet_id, external_id, direction, amount, minute in (
            ("tx-a-out", "wallet-a", TXID, "outbound", 300_000, 1),
            ("tx-b-in", "wallet-b", TXID, "inbound", 100_000, 2),
            ("tx-c-in", "wallet-c", TXID, "inbound", 199_000, 3),
            ("tx-colon", "wallet-a", "kraken:L123", "inbound", 5_000, 4),
        ):
            self.conn.execute(
                "INSERT INTO transactions(id, workspace_id, profile_id, wallet_id, "
                "fingerprint, external_id, occurred_at, direction, asset, amount, fee, "
                "created_at) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, ?, 'BTC', ?, 0, ?)",
                (
                    tx_id,
                    wallet_id,
                    f"fp-{tx_id}",
                    external_id,
                    f"2026-01-01T00:0{minute}:00Z",
                    direction,
                    amount,
                    NOW,
                ),
            )
        set_setting(self.conn, "context_workspace", "ws")
        set_setting(self.conn, "context_profile", "profile")
        self.conn.commit()

    def _resolved(self, reference, **kwargs):
        return find_transaction(self.conn, "profile", reference, **kwargs)["id"]

    def test_exact_ids_and_unique_external_ids_are_unchanged(self):
        self.assertEqual(self._resolved("tx-b-in"), "tx-b-in")
        self.assertEqual(self._resolved("kraken:L123"), "tx-colon")
        self.assertIsNone(find_transaction(self.conn, "profile", TXID[:12]))
        self.assertIsNone(find_transaction(self.conn, "profile", "missing"))

    def test_shared_txid_fails_closed_with_every_candidate(self):
        with self.assertRaises(AppError) as caught:
            resolve_transaction(self.conn, "profile", TXID)
        error = caught.exception
        self.assertEqual(error.code, "ambiguous_reference")
        self.assertEqual(error.details["candidate_count"], 3)
        candidates = {item["transaction_id"]: item for item in error.details["candidates"]}
        self.assertEqual(set(candidates), {"tx-a-out", "tx-b-in", "tx-c-in"})
        self.assertEqual(candidates["tx-b-in"]["wallet"], "Cold B")
        self.assertEqual(candidates["tx-b-in"]["direction"], "inbound")
        self.assertEqual(candidates["tx-b-in"]["amount_msat"], 100_000)
        remediation = error.details["remediation"]
        self.assertEqual(remediation["action"], "retry_with_full_transaction_id")
        self.assertEqual(
            set(remediation["target"]["transaction_ids"]), set(candidates)
        )

        # The AI sees the candidates and the remediation, not just the code.
        result = _tool_result_from_app_error(error)
        self.assertEqual(result["reason"], "ambiguous_reference")
        self.assertEqual(result["details"]["candidate_count"], 3)
        self.assertIn("remediation", result)
        self.assertNotIn("remediation", result["details"])

    def test_direction_filter_narrows_but_still_fails_closed(self):
        self.assertEqual(
            resolve_transaction(self.conn, "profile", TXID, direction="outbound")["id"],
            "tx-a-out",
        )
        with self.assertRaises(AppError) as caught:
            resolve_transaction(self.conn, "profile", TXID, direction="inbound")
        self.assertEqual(caught.exception.details["candidate_count"], 2)

    def test_qualified_references_select_exactly_one_row(self):
        self.assertEqual(self._resolved(f"{TXID}:out"), "tx-a-out")
        self.assertEqual(self._resolved(f"{TXID}:cold b"), "tx-b-in")
        self.assertEqual(self._resolved(f"{TXID}:wallet-c:in"), "tx-c-in")
        self.assertEqual(self._resolved(f"{TXID}:Cold C:inbound"), "tx-c-in")
        # A resolved qualified row has exactly the plain transaction columns.
        row = find_transaction(self.conn, "profile", f"{TXID}:out")
        self.assertNotIn("lookup_wallet_label", row.keys())

        with self.assertRaises(AppError) as caught:
            find_transaction(self.conn, "profile", f"{TXID}:in")
        self.assertEqual(caught.exception.code, "ambiguous_reference")
        self.assertEqual(caught.exception.details["candidate_count"], 2)

        with self.assertRaises(AppError) as caught:
            find_transaction(self.conn, "profile", f"{TXID}:Cold B:out")
        self.assertEqual(caught.exception.code, "reference_qualifier_unmatched")
        self.assertEqual(caught.exception.details["candidate_count"], 3)

    def test_resolve_snapshot_discloses_rows_sharing_the_txid(self):
        payload = build_transactions_resolve_snapshot(self.conn, {"query": TXID})
        self.assertEqual(payload["transaction"]["id"], "tx-c-in")
        self.assertEqual(payload["sharedExternalIdCount"], 3)
        self.assertEqual(
            {item["transaction_id"] for item in payload["sharedExternalIdMatches"]},
            {"tx-a-out", "tx-b-in"},
        )

        qualified = build_transactions_resolve_snapshot(
            self.conn, {"query": f"{TXID}:Cold B"}
        )
        self.assertEqual(qualified["transaction"]["id"], "tx-b-in")

        single = build_transactions_resolve_snapshot(self.conn, {"query": "tx-colon"})
        self.assertEqual(single["sharedExternalIdCount"], 1)
        self.assertEqual(single["sharedExternalIdMatches"], [])


if __name__ == "__main__":
    unittest.main()

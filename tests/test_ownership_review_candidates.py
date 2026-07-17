import json
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.cli import handlers
from kassiber.core import custody_journal
from kassiber.core.ownership import OwnedIndex, OwnedMatch
from kassiber.core.ui_snapshot import _ownership_review_candidate_blocker
from kassiber.db import open_db


NOW = "2026-07-10T10:00:00Z"
SCRIPT_A = "0014" + "aa" * 20
SCRIPT_B = "0014" + "bb" * 20
GRAPH_TXID = "12" * 32
PARENT_TXID = "34" * 32


def _match(wallet_id, label):
    return OwnedMatch(
        wallet_id=wallet_id,
        wallet_label=label,
        account="",
        chain="bitcoin",
        network="main",
        branch_label="receive",
        address_index=0,
        derivation_path=None,
        source="derived",
        wallet_kind="descriptor",
    )


class OwnershipReviewCandidateTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-ownership-review-")
        self.addCleanup(self.tmp.cleanup)
        self.db_path = Path(self.tmp.name) / "data"
        self.conn = open_db(self.db_path)
        self.sqlite_path = Path(
            self.conn.execute("PRAGMA database_list").fetchone()["file"]
        )
        self.addCleanup(self.conn.close)
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws", "Main", NOW),
        )
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            ("profile", "ws", "Book", "EUR", "at", 365, "FIFO", NOW),
        )
        self.conn.execute(
            """
            INSERT INTO accounts(
                id, workspace_id, profile_id, code, label, account_type, asset, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            ("account", "ws", "profile", "treasury", "Treasury", "asset", "BTC", NOW),
        )
        for wallet_id, label in (("cold", "Cold"), ("hot", "Hot")):
            self.conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, account_id, label, kind,
                    config_json, created_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    wallet_id,
                    "ws",
                    "profile",
                    "account",
                    label,
                    "descriptor",
                    "{}",
                    NOW,
                ),
            )
        graph = json.dumps(
            {
                "txid": GRAPH_TXID,
                "vin": [
                    {
                        "txid": PARENT_TXID,
                        "vout": 0,
                        "prevout": {"scriptpubkey": SCRIPT_A},
                    }
                ],
                "vout": [
                    {"n": 0, "scriptpubkey": SCRIPT_B, "value": 500_000}
                ],
            }
        )
        self._insert_tx(
            "out",
            "cold",
            GRAPH_TXID,
            "outbound",
            500_000_000,
            graph,
        )
        self._insert_tx(
            "in",
            "hot",
            "provider-settlement",
            "inbound",
            500_000_000,
            "{}",
        )
        self.conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            (
                "out",
                "ws",
                "profile",
                "ownership_transfer_destination_ambiguous",
                "{}",
                NOW,
            ),
        )
        self.conn.commit()
        self.index = OwnedIndex()
        self.index.add_script(SCRIPT_A, _match("cold", "Cold"))
        self.index.add_script(SCRIPT_B, _match("hot", "Hot"))

    def _insert_tx(self, tx_id, wallet_id, external_id, direction, amount, raw_json):
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, kind, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                "ws",
                "profile",
                wallet_id,
                external_id,
                f"fp-{tx_id}",
                NOW,
                direction,
                "BTC",
                amount,
                0,
                "transfer",
                raw_json,
                NOW,
            ),
        )

    def _suggest(self):
        with patch.object(
            handlers.core_ownership,
            "build_owned_index",
            return_value=(self.index, []),
        ):
            return handlers.suggest_transfer_candidates(
                self.conn, "Main", "Book", candidate_type="transfer"
            )

    def test_blocked_graph_proof_surfaces_without_script_material(self):
        payload = self._suggest()
        self.assertEqual(payload["counts"]["ownership"], 1)
        candidate = payload["candidates"][0]
        self.assertEqual(candidate["method"], "ownership_graph")
        self.assertEqual((candidate["out_id"], candidate["in_id"]), ("out", "in"))
        self.assertEqual(candidate["default_policy"], "carrying-value")
        serialized = json.dumps(candidate)
        self.assertNotIn(SCRIPT_A, serialized)
        self.assertNotIn(SCRIPT_B, serialized)
        self.assertNotIn("derivation", serialized)

    def test_existing_pair_store_accepts_candidate_and_clears_it(self):
        candidate = self._suggest()["candidates"][0]
        handlers.create_transaction_pair(
            self.conn,
            "Main",
            "Book",
            candidate["out_id"],
            candidate["in_id"],
            kind="manual",
            policy="carrying-value",
            confidence_at_pair="exact",
        )
        self.assertEqual(self._suggest()["counts"]["ownership"], 0)

    def test_report_blocker_routes_ownership_review_to_transfer_suggestions(self):
        self._insert_tx(
            "mismatch-out",
            "cold",
            "mismatch-tx",
            "outbound",
            100_000_000,
            "{}",
        )
        self.conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            (
                "mismatch-out",
                "ws",
                "profile",
                "ownership_transfer_amount_mismatch",
                "{}",
                NOW,
            ),
        )
        rows = handlers._load_matcher_rows(self.conn, "profile")
        quarantines = [
            {"transaction_id": row["transaction_id"], "reason": row["reason"]}
            for row in self.conn.execute(
                "SELECT transaction_id, reason FROM journal_quarantines "
                "WHERE profile_id = ?",
                ("profile",),
            ).fetchall()
        ]
        counts = custody_journal.ownership_review_counts(
            rows,
            self.index,
            quarantines,
            [],
        )
        self.assertEqual(
            counts,
            {
                "total": 1,
                "by_reason": {"ownership_transfer_destination_ambiguous": 1},
            },
        )
        self.conn.execute(
            "UPDATE profiles SET ownership_review_counts_json = ? WHERE id = ?",
            (json.dumps(counts), "profile"),
        )
        self.conn.commit()
        with patch(
            "kassiber.core.ui_snapshot.core_ownership.build_owned_index",
            side_effect=AssertionError("report blockers must not rebuild ownership"),
        ):
            ownership = _ownership_review_candidate_blocker(self.conn, "profile")
        self.assertIsNotNone(ownership)
        self.assertEqual(ownership["daemon_kind"], "ui.transfers.suggest")
        self.assertEqual(ownership["daemon_args"]["method"], "ownership_graph")
        self.assertEqual(ownership["counts"]["total"], 1)

    def test_journal_processing_persists_ownership_review_count_cache(self):
        state = {
            "entries": [],
            "quarantines": [],
            "cross_asset_pairs": [],
            "direct_swap_payouts": [],
            "tax_summary": [],
            "account_holdings": {},
            "wallet_holdings": {},
            "ownership_review_counts": {
                "total": 1,
                "by_reason": {"owned_fanout_unresolved": 1},
            },
            "warnings": [],
        }
        with patch.object(custody_journal, "build_ledger_state", return_value=state):
            handlers.process_journals(self.conn, "Main", "Book")
        cached = self.conn.execute(
            "SELECT ownership_review_counts_json FROM profiles WHERE id = ?",
            ("profile",),
        ).fetchone()["ownership_review_counts_json"]
        self.assertEqual(json.loads(cached), state["ownership_review_counts"])
        handlers.invalidate_journals(self.conn, "profile")
        invalidated = self.conn.execute(
            "SELECT ownership_review_counts_json FROM profiles WHERE id = ?",
            ("profile",),
        ).fetchone()["ownership_review_counts_json"]
        self.assertIsNone(invalidated)

    def test_cache_migration_marks_legacy_ownership_quarantine_stale(self):
        self.conn.execute(
            "UPDATE profiles SET last_processed_at = ?, last_processed_tx_count = 2 "
            "WHERE id = ?",
            (NOW, "profile"),
        )
        self.conn.commit()
        self.conn.close()
        legacy = sqlite3.connect(self.sqlite_path)
        legacy.execute("ALTER TABLE profiles DROP COLUMN ownership_review_counts_json")
        legacy.commit()
        legacy.close()

        upgraded = open_db(self.db_path)
        self.addCleanup(upgraded.close)
        profile = upgraded.execute(
            "SELECT last_processed_at, last_processed_tx_count, "
            "ownership_review_counts_json FROM profiles WHERE id = ?",
            ("profile",),
        ).fetchone()
        self.assertIsNone(profile["last_processed_at"])
        self.assertEqual(profile["last_processed_tx_count"], 0)
        self.assertIsNone(profile["ownership_review_counts_json"])


if __name__ == "__main__":
    unittest.main()

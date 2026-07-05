import tempfile
import unittest
from pathlib import Path

from kassiber.core.repo import resolve_profile, resolve_wallet, resolve_workspace
from kassiber.db import open_db
from kassiber.errors import AppError


NOW = "2026-07-01T12:00:00Z"


class RepositoryResolutionTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-repo-resolve-")
        self.conn = open_db(Path(self._tmp.name) / "data")

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def _insert_workspace(self, id_, label):
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            (id_, label, NOW),
        )

    def _insert_profile(self, id_, workspace_id, label):
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            )
            VALUES(?, ?, ?, 'EUR', 'generic', 365, 'FIFO', ?)
            """,
            (id_, workspace_id, label, NOW),
        )

    def _insert_wallet(self, id_, workspace_id, profile_id, label):
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind,
                config_json, created_at
            )
            VALUES(?, ?, ?, NULL, ?, 'descriptor', '{}', ?)
            """,
            (id_, workspace_id, profile_id, label, NOW),
        )

    def test_workspace_label_ambiguity_does_not_pick_first_row(self):
        self._insert_workspace("ws-a", "Main")
        self._insert_workspace("ws-b", "main")

        with self.assertRaises(AppError) as ctx:
            resolve_workspace(self.conn, "MAIN")

        self.assertEqual(ctx.exception.code, "validation")
        self.assertEqual(
            [row["id"] for row in ctx.exception.details["matches"]],
            ["ws-a", "ws-b"],
        )

    def test_profile_label_ambiguity_does_not_pick_first_row(self):
        self._insert_workspace("ws", "Main")
        self._insert_profile("pf-a", "ws", "Default")
        self._insert_profile("pf-b", "ws", "default")

        with self.assertRaises(AppError) as ctx:
            resolve_profile(self.conn, "ws", "DEFAULT")

        self.assertEqual(ctx.exception.code, "validation")
        self.assertEqual(
            [row["id"] for row in ctx.exception.details["matches"]],
            ["pf-a", "pf-b"],
        )

    def test_wallet_label_ambiguity_does_not_pick_first_row(self):
        self._insert_workspace("ws", "Main")
        self._insert_profile("pf", "ws", "Default")
        self._insert_wallet("wal-a", "ws", "pf", "Treasury")
        self._insert_wallet("wal-b", "ws", "pf", "treasury")

        with self.assertRaises(AppError) as ctx:
            resolve_wallet(self.conn, "pf", "TREASURY")

        self.assertEqual(ctx.exception.code, "validation")
        self.assertEqual(
            [row["id"] for row in ctx.exception.details["matches"]],
            ["wal-a", "wal-b"],
        )

    def test_id_lookup_wins_over_case_insensitive_label_match(self):
        self._insert_workspace("ws", "Main")
        self._insert_profile("pf", "ws", "Default")
        self._insert_wallet("treasury", "ws", "pf", "Cold")
        self._insert_wallet("wal-b", "ws", "pf", "Treasury")

        wallet = resolve_wallet(self.conn, "pf", "treasury")

        self.assertEqual(wallet["id"], "treasury")

"""Reset book data, then Sync, re-imports every wallet's history.

The reset used to keep the wallets' sync checkpoints, so the next sync was
incremental and re-imported almost nothing, and sync-all skipped retired
wallets even with ``--force-full``.
"""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber import daemon_freshness
from kassiber.cli import handlers as cli_handlers
from kassiber.core import freshness, maintenance
from kassiber.db import open_db

CREATED_AT = "2026-06-04T00:00:00Z"
SYNCED_AT = "2026-06-05T00:00:00Z"
CHECKPOINT = {"electrum_statuses": {"script": "history-hash"}}


class ResetResyncTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="kassiber-reset-resync-")
        self.addCleanup(temp.cleanup)
        self.data_root = Path(temp.name) / "data"
        self.conn = open_db(self.data_root)
        self.addCleanup(self.conn.close)
        conn = self.conn
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'Main', ?)",
            (CREATED_AT,),
        )
        conn.execute(
            """
            INSERT INTO profiles(id, workspace_id, label, fiat_currency, created_at)
            VALUES('profile', 'ws', 'Book', 'EUR', ?)
            """,
            (CREATED_AT,),
        )
        conn.execute(
            "INSERT INTO settings(key, value) VALUES('context_workspace', 'ws'), ('context_profile', 'profile')"
        )
        for wallet_id, label, config in (
            ("active", "Active", {"addresses": ["bc1qactive"], "last_synced_at": SYNCED_AT}),
            (
                "retired",
                "Retired",
                {"addresses": ["bc1qretired"], "deprecated": True, "last_synced_at": SYNCED_AT},
            ),
        ):
            conn.execute(
                """
                INSERT INTO wallets(
                    id, workspace_id, profile_id, account_id, label, kind,
                    config_json, created_at
                ) VALUES(?, 'ws', 'profile', NULL, ?, 'address', ?, ?)
                """,
                (wallet_id, label, json.dumps(config, sort_keys=True), CREATED_AT),
            )
            conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id,
                    fingerprint, occurred_at, direction, asset, amount, fee,
                    raw_json, created_at
                ) VALUES(?, 'ws', 'profile', ?, ?, ?, ?, 'inbound', 'BTC', 1000, 0, '{}', ?)
                """,
                (f"tx-{wallet_id}", wallet_id, f"{wallet_id}-txid", f"fp-{wallet_id}", CREATED_AT, CREATED_AT),
            )
        freshness.upsert_source_state(
            conn,
            profile_id="profile",
            source_key=freshness.source_key(freshness.SOURCE_ONCHAIN, "active"),
            source_type=freshness.SOURCE_ONCHAIN,
            source_label="Active",
            status=freshness.STATUS_FRESH,
            paused=True,
            checkpoint=CHECKPOINT,
        )
        conn.commit()

    def _cli_sync_all(self, *, force_full=False):
        with patch(
            "kassiber.cli.handlers._prefetch_chain_wallets",
            return_value={},
        ) as prefetch, patch(
            "kassiber.cli.handlers._apply_wallet_sync_atomically",
            side_effect=lambda conn, runtime, profile, wallet, *args, **kwargs: [
                {"wallet": wallet["label"], "status": "synced"}
            ],
        ):
            results = cli_handlers.sync_wallet(
                self.conn, {}, "ws", "profile", sync_all=True, force_full=force_full
            )
        return (
            [result["wallet"] for result in results],
            prefetch.call_args.kwargs["freshness_checkpoints"],
        )

    def _daemon_sync_all(self, *, force_full=False):
        specs = daemon_freshness._freshness_wallet_source_specs(
            self.conn,
            "profile",
            include_rates=False,
            include_journals=False,
            force_full=force_full,
        )
        return [spec["payload"]["wallet_label"] for spec in specs]

    def test_ongoing_sync_all_skips_retired_wallets_unless_forced(self):
        self.assertEqual(self._cli_sync_all(), (["Active"], {"active": CHECKPOINT}))
        self.assertEqual(self._daemon_sync_all(), ["Active"])
        wallets, checkpoints = self._cli_sync_all(force_full=True)
        self.assertEqual(wallets, ["Active", "Retired"])
        self.assertEqual(checkpoints, {})
        self.assertEqual(self._daemon_sync_all(force_full=True), ["Active", "Retired"])

    def test_sync_after_reset_reimports_every_wallet_in_full(self):
        summary = maintenance.reset_current_profile_data(self.conn, str(self.data_root))
        self.assertEqual(summary["removed"]["sync_checkpoints"], 1)
        self.assertEqual(summary["removed"]["wallet_sync_times"], 2)

        state = freshness.get_source_state(
            self.conn, "profile", freshness.source_key(freshness.SOURCE_ONCHAIN, "active")
        )
        self.assertEqual(state["checkpoint"], {})
        self.assertEqual(state["stale_reason"], "book_data_reset")
        self.assertTrue(state["paused"])

        wallets, checkpoints = self._cli_sync_all()
        self.assertEqual(wallets, ["Active", "Retired"])
        self.assertEqual(checkpoints, {"active": {}})
        self.assertEqual(self._daemon_sync_all(), ["Active", "Retired"])

        # Once a retired wallet's history is back, it is skipped again.
        for wallet_id in ("active", "retired"):
            config = json.loads(
                self.conn.execute(
                    "SELECT config_json FROM wallets WHERE id = ?", (wallet_id,)
                ).fetchone()[0]
            )
            self.assertNotIn("last_synced_at", config)
            cli_handlers._mark_wallet_synced(
                self.conn,
                self.conn.execute("SELECT * FROM wallets WHERE id = ?", (wallet_id,)).fetchone(),
            )
        self.assertEqual(self._cli_sync_all()[0], ["Active"])
        self.assertEqual(self._daemon_sync_all(), ["Active"])


if __name__ == "__main__":
    unittest.main()

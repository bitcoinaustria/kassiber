"""Per-wallet completeness class and scan bounds.

The class states which kind of source a wallet's history came from; the bounds
state the observed edges of that history. Neither may leak source content
(descriptors, xpubs, addresses, paths), and both must be shared by the CLI and
the daemon wallet list.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest

from kassiber.core import wallet_completeness as wc
from kassiber.time_utils import UNKNOWN_OCCURRED_AT


REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ADDRESS = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"


def _row(occurred_at, direction, amount, asset="BTC", fee=0):
    return {
        "id": f"{occurred_at}-{direction}",
        "external_id": None,
        "occurred_at": occurred_at,
        "confirmed_at": occurred_at,
        "raw_json": None,
        "direction": direction,
        "asset": asset,
        "amount": amount,
        "fee": fee,
        "created_at": occurred_at,
    }


class CompletenessClassTests(unittest.TestCase):
    def test_classes_follow_the_source_shape(self):
        cases = [
            ("descriptor", {"descriptor": "wpkh(xpub/0/*)"}, "descriptor-derived"),
            ("xpub", {"xpub": "xpub", "script_types": ["p2wpkh"]}, "descriptor-derived"),
            ("samourai", {}, "descriptor-derived"),
            ("descriptor", {"source_file": "/x.csv", "source_format": "csv"}, "file-import"),
            ("address", {"addresses": [ADDRESS]}, "address-list"),
            ("coreln", {"backend": "node"}, "lightning-node"),
            ("lnd", {"backend": "node"}, "lightning-node"),
            ("lnd", {"source_file": "/x.csv"}, "file-import"),
            ("kraken", {"backend": "kraken"}, "provider-api"),
            ("kraken", {"source_file": "/x.csv"}, "exchange-export"),
            ("strike", {}, "exchange-export"),
            ("phoenix", {"source_format": "phoenix_csv"}, "file-import"),
            ("custom", {}, "manual"),
            ("custom", {"source_format": "generic_ledger"}, "manual"),
            ("custom", {"source_format": "csv", "source_file": "/x.csv"}, "file-import"),
            ("untracked", {}, "untracked"),
            ("address", {"sync_source": "btcpay", "backend": "b"}, "provider-api"),
        ]
        for kind, config, expected in cases:
            with self.subTest(kind=kind, config=config):
                self.assertEqual(wc.completeness_class(kind, config), expected)
                self.assertIn(expected, wc.COMPLETENESS_CLASSES)


class ScanBoundsTests(unittest.TestCase):
    def test_address_list_is_incomplete_by_construction_and_never_synced(self):
        bounds = wc.scan_bounds("address", {"addresses": [ADDRESS]}, [])
        self.assertEqual(
            bounds["warnings"],
            ["incomplete_by_construction", "never_synced", "no_transactions"],
        )
        self.assertEqual(bounds["history_before_first_transaction"], "unknown")

    def test_synced_descriptor_wallet_is_scanned_unless_a_start_is_declared(self):
        config = {"descriptor": "wpkh(x)", "last_synced_at": "2025-01-01T00:00:00Z"}
        rows = [_row("2024-01-01T00:00:00Z", "inbound", 1000)]
        bounds = wc.scan_bounds("descriptor", config, rows)
        self.assertEqual(bounds["history_before_first_transaction"], "scanned")
        self.assertEqual(bounds["warnings"], [])
        self.assertEqual(bounds["first_transaction_at"], "2024-01-01T00:00:00Z")
        self.assertEqual(bounds["last_synced_at"], "2025-01-01T00:00:00Z")

        declared = wc.scan_bounds(
            "descriptor", {**config, "birthday": "2023-06-01T00:00:00Z"}, rows
        )
        self.assertEqual(declared["history_before_first_transaction"], "declared_start")
        self.assertEqual(declared["declared_scan_start_at"], "2023-06-01T00:00:00Z")

    def test_spending_more_than_was_received_flags_missing_early_history(self):
        rows = [
            _row("2024-01-01T00:00:00Z", "outbound", 5000),
            _row("2024-02-01T00:00:00Z", "inbound", 9000),
        ]
        bounds = wc.scan_bounds("custom", {}, rows)
        self.assertIn("history_starts_after_first_funding", bounds["warnings"])
        self.assertEqual(bounds["deficit_assets"], ["BTC"])

    def test_undated_rows_do_not_move_the_bounds(self):
        rows = [
            _row(UNKNOWN_OCCURRED_AT, "inbound", 1000),
            _row("2024-02-01T00:00:00Z", "inbound", 9000),
        ]
        bounds = wc.scan_bounds("custom", {}, rows)
        self.assertEqual(bounds["first_transaction_at"], "2024-02-01T00:00:00Z")
        self.assertEqual(bounds["undated_transactions"], 1)
        self.assertIn("undated_transactions", bounds["warnings"])


class SharedListSurfaceTests(unittest.TestCase):
    """CLI `wallets list` and daemon `ui.wallets.list` carry the same fields."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.data_root = os.path.join(self._tmp.name, "data")
        self.run_cli("init")
        self.run_cli("workspaces", "create", "W")
        self.run_cli(
            "profiles", "create", "P", "--workspace", "W",
            "--fiat-currency", "EUR", "--tax-country", "at",
        )
        self.run_cli(
            "wallets", "create", "--workspace", "W", "--profile", "P",
            "--label", "Addr", "--kind", "address", "--address", ADDRESS,
        )

    def run_cli(self, *args):
        completed = subprocess.run(
            [sys.executable, "-m", "kassiber", "--data-root", self.data_root,
             "--machine", *args],
            capture_output=True,
            text=True,
            cwd=REPO_ROOT,
        )
        return json.loads(completed.stdout) if completed.stdout.strip() else {}

    def test_cli_and_daemon_agree_and_leak_no_source_content(self):
        cli_rows = self.run_cli("wallets", "list", "--workspace", "W", "--profile", "P")["data"]
        self.assertEqual(len(cli_rows), 1)
        self.assertEqual(cli_rows[0]["completeness_class"], "address-list")

        from kassiber.core.ui_snapshot import build_wallets_list_snapshot
        from kassiber.db import open_db

        conn = open_db(self.data_root)
        try:
            snapshot = build_wallets_list_snapshot(conn, {})
        finally:
            conn.close()
        wallet = snapshot["wallets"][0]
        self.assertEqual(wallet["completeness_class"], cli_rows[0]["completeness_class"])
        self.assertEqual(wallet["scan_bounds"], cli_rows[0]["scan_bounds"])
        self.assertNotIn(ADDRESS, json.dumps(wallet))


if __name__ == "__main__":
    unittest.main()

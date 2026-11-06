"""Transfer dismissals through the Rust CLI and daemon (main sync S6).

A dismissal marks the book's journals stale, the vetoes can be listed and
undone (`transfers dismissals list|delete`, `ui.transfers.dismissals.*`),
and suggestions report what the matcher excluded. Each case runs the Python
reference and the Rust binary on copies of one book and compares the bytes
and all 113 tables, plaintext and SQLCipher.
"""
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import unittest

from kassiber.secrets.sqlcipher import open_encrypted

from tests.rust_desktop_parity import RUST_BIN
from tests.rust_port.sync_parity import SyncParity

ROOT = Path(__file__).resolve().parent.parent
VECTORS = json.loads((ROOT / "crates/transfers/tests/dismissals.json").read_text())
SHARED = "a" * 64


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN names the Rust binary")
class TransferDismissalParity(SyncParity, unittest.TestCase):
    SQL = VECTORS["sql"]
    CONTEXT = ("w", "p")

    def journal_input_version(self):
        path = self.root / "kassiber.sqlite3"
        conn = open_encrypted(path, self.passphrase) if self.passphrase else sqlite3.connect(path)
        try:
            return conn.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0]
        finally:
            conn.close()

    def test_cli_dismiss_list_delete_and_suggest_match_python(self):
        envelopes = self.cli([
            ["transfers", "dismissals", "list"],
            ["transfers", "dismissals", "list", "--include-expired"],
            ["transfers", "dismissals", "list", "--limit", "1"],
            ["transfers", "dismissals", "list", "--limit", "0"],
            ["transfers", "dismissals", "list", "--limit", "501"],
            ["transfers", "dismissals", "list", "--profile", "missing"],
            ["transfers", "suggest"],
            ["transfers", "dismiss", "--tx-out", f"{SHARED}:out", "--tx-in", "xi",
             "--reason", "not one movement"],
            ["transfers", "dismiss", "--tx-out", "xo", "--tx-in", f"{SHARED}:cold b",
             "--expires-in-days", "0"],
            ["transfers", "dismiss", "--tx-out", SHARED, "--tx-in", "xi"],
            ["transfers", "dismissals", "delete", "--id", "d-past"],
            ["transfers", "dismissals", "delete", "--id", "d-past"],
            ["transfers", "dismissals", "delete", "--id", "d-bad", "--tx-out", "o2"],
            ["transfers", "dismissals", "delete"],
            ["transfers", "dismissals", "delete", "--tx-out", f"{SHARED}:out", "--tx-in", f"{SHARED}:Cold B"],
            ["transfers", "dismissals", "delete", "--tx-out", SHARED, "--tx-in", "i1"],
            ["transfers", "dismissals", "list", "--include-expired"],
            ["transfers", "suggest"],
        ])
        listed = envelopes[0]["data"]
        self.assertEqual(listed["counts"], {"active": 5, "expired": 2, "permanent": 3})
        self.assertEqual(len(envelopes[1]["data"]["dismissals"]), 7)
        self.assertTrue(envelopes[2]["data"]["has_more"])
        self.assertEqual(envelopes[3]["error"]["code"], "validation")
        self.assertIn("exclusions", envelopes[6]["data"])
        self.assertEqual(envelopes[9]["error"]["code"], "ambiguous_reference")
        self.assertEqual(envelopes[10]["data"]["deleted"], "d-past")
        self.assertEqual(envelopes[14]["data"]["deleted"], "d-perm")
        # Three successful writes (two dismissals, two deletes) each made the
        # journals stale; refusals did not.
        self.assertEqual(self.journal_input_version(), 4 + 4)

    def test_daemon_kinds_match_python(self):
        listed = self.masked_read(
            [{}, {"include_expired": True}, {"include_expired": "yes"},
             {"include_expired": None}, {"limit": 1}, {"limit": True}, {"limit": None},
             {"limit": 500, "include_expired": False}, {"profile": "missing"}],
            kind="ui.transfers.dismissals.list",
        )
        self.assertEqual(listed[0]["data"]["counts"]["active"], 5)
        self.assertEqual(listed[2]["error"]["message"],
                         "ui.transfers.dismissals.list include_expired must be a boolean")
        suggest = self.compare([{}, {"candidate_type": "transfer"}], kind="ui.transfers.suggest")
        self.assertIn("exclusions", suggest[0]["data"])
        # The review context refreshes stale journals first, in both runtimes.
        context = self.masked([{}], kind="ui.transfers.review_context")
        self.assertIn("exclusions", context[0]["data"])
        self.masked(
            [{"out_id": f"{SHARED}:out", "in_id": "xi"},
             {"tx_out": "xo", "tx_in": f"{SHARED}:Cold B", "expires_in_days": 0},
             {"tx_out": SHARED, "tx_in": "xi"}],
            kind="ui.transfers.dismiss",
        )
        deleted = self.masked(
            [{"dismissal_id": "d-past"}, {"dismissal_id": "d-past"},
             {"dismissal_id": "d-bad", "out_id": "o2"}, {"dismissal_id": 7},
             {"out_id": f"{SHARED}:out", "tx_in": f"{SHARED}:Cold B"},
             {"tx_out": "", "out_id": "o2", "in_id": "i2"}, {}],
            kind="ui.transfers.dismissals.delete",
        )
        self.assertEqual(deleted[0]["data"]["deleted"], "d-past")
        self.assertEqual(deleted[1]["error"]["code"], "not_found")
        self.assertEqual(deleted[4]["data"]["deleted"], "d-perm")
        self.assertEqual(deleted[5]["data"]["deleted"], "d-empty")

    def test_hybrid_serves_the_dismissal_leaves_in_rust(self):
        log = self.base / "hybrid.jsonl"
        env = {**self.env, "KASSIBER_HYBRID_RUST_BIN": RUST_BIN, "KASSIBER_HYBRID_LOG": str(log)}
        args = ["--machine", "--data-root", str(self.root)]
        if self.passphrase:
            return  # the hybrid wrapper's passphrase routing is covered elsewhere
        for argv in (["transfers", "dismissals", "list"],
                     ["transfers", "dismissals", "delete", "--id", "d-past"]):
            result = subprocess.run(
                [sys.executable, str(ROOT / "tests/rust_port/hybrid_cli.py"), *args, *argv],
                env=env, cwd=ROOT, capture_output=True, timeout=120,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        served = [json.loads(line) for line in log.read_text().splitlines()]
        self.assertEqual([row["served_by"] for row in served], ["rust", "rust"], served)


class EncryptedTransferDismissalParity(TransferDismissalParity):
    PASSPHRASE = "sync-s6-passphrase"


if __name__ == "__main__":
    unittest.main()

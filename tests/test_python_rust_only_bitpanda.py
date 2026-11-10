"""Python refuses the Bitpanda exchange sync only the Rust backend serves.

Python has no Bitpanda importer (docs/reference/bitpanda.md#python). Its CLI
must refuse the command, the backend kind and the wallet kind cleanly and
without writing, and a book that already holds a Rust-created Bitpanda
backend and wallet must still list and sync in Python without contacting
anything.
"""

import hashlib
import json
import os
import sqlite3
import tempfile
import unittest
from pathlib import Path

from tests import cli_runner


class PythonRefusesBitpandaTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-python-bitpanda-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"
        # Children must not resolve the owner's real app-data root, and the
        # test socket guard fails any egress.
        self.env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
        self.env["KASSIBER_OFFLINE_PREFERENCE_FILE"] = str(Path(self.tmp.name) / "offline-mode.json")
        self.env["KASSIBER_TEST_NO_EGRESS"] = "1"
        self.cli("init")
        self.cli("workspaces", "create", "Main")
        self.cli("profiles", "create", "Book", "--fiat-currency", "EUR")

    def cli(self, *args, check=True):
        result = cli_runner.run(
            ["--data-root", str(self.data_root), "--machine", *args],
            reference=True,
            capture_output=True,
            text=True,
            env=self.env,
            check=False,
        )
        if check:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        return result

    def database(self) -> Path:
        return self.data_root / "kassiber.sqlite3"

    def digest(self) -> str:
        with sqlite3.connect(self.database()) as conn:
            rows = [
                (table, sorted(map(repr, conn.execute(f'SELECT * FROM "{table}"'))))
                for (table,) in conn.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            ]
        return hashlib.sha256(repr(rows).encode()).hexdigest()

    def test_the_command_backend_kind_and_wallet_kind_are_refused_without_writing(self):
        before = self.digest()
        sync = self.cli("wallets", "sync-bitpanda", "--backend", "bitpanda-main", check=False)
        self.assertEqual(sync.returncode, 2)
        envelope = json.loads(sync.stdout)
        self.assertEqual(envelope["error"]["code"], "usage_error")
        self.assertIn("invalid choice: 'sync-bitpanda'", envelope["error"]["message"])

        backend = self.cli("backends", "create", "bitpanda-main", "--kind", "bitpanda",
                           "--url", "https://api.public.bitpanda.com", check=False)
        self.assertEqual(backend.returncode, 2)
        self.assertIn("invalid choice: 'bitpanda'", json.loads(backend.stdout)["error"]["message"])

        wallet = self.cli("wallets", "create", "--label", "Bitpanda", "--kind", "bitpanda", check=False)
        self.assertNotEqual(wallet.returncode, 0)
        error = json.loads(wallet.stdout)["error"]
        self.assertIn("Unsupported wallet kind 'bitpanda'", error["message"])
        self.assertEqual(self.digest(), before)

    def test_a_rust_created_bitpanda_wallet_lists_and_syncs_as_sourceless(self):
        # What `wallets sync-bitpanda` leaves in a book: a bitpanda backend
        # and a provider wallet pointing at it.
        with sqlite3.connect(self.database()) as conn:
            workspace, profile = conn.execute("SELECT workspace_id, id FROM profiles").fetchone()
            conn.execute(
                "INSERT INTO backends(name,kind,chain,network,url,token,config_json,created_at,updated_at) "
                "VALUES('bitpanda-main','bitpanda',NULL,NULL,'https://api.public.bitpanda.com','synthetic-key','{}',"
                "'2026-11-08T00:00:00Z','2026-11-08T00:00:00Z')"
            )
            conn.execute(
                "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                "VALUES('w-bitpanda',?,?,'Bitpanda','bitpanda','{\"backend\": \"bitpanda-main\"}','2026-11-08T00:00:00Z')",
                (workspace, profile),
            )
        listed = json.loads(self.cli("wallets", "list").stdout)
        self.assertEqual([w["kind"] for w in listed["data"]], ["bitpanda"])
        synced = json.loads(self.cli("wallets", "sync", "--wallet", "Bitpanda").stdout)
        outcomes = json.dumps(synced["data"])
        self.assertIn("skipped", outcomes)
        self.assertNotIn("synthetic-key", outcomes)
        with sqlite3.connect(self.database()) as conn:
            self.assertEqual(conn.execute("SELECT count(*) FROM transactions").fetchone(), (0,))


if __name__ == "__main__":
    unittest.main()

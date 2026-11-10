"""`wallets sync-bitpanda` through the Rust binary, end to end.

Runs `KASSIBER_RUST_BIN` (a `--features test-harness` build of
crates/kassiber) against the loopback Bitpanda fake on a disposable book:
saving the backend and reading the book send nothing, the explicit sync
reads and imports, offline mode stops it before any request, and the key
appears in no output and no table but `backends`.
"""

import json
import os
import sqlite3
import subprocess
import tempfile
import unittest
from pathlib import Path

from tests.rust_port.exchange_api import KEY, Bitpanda, serve

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
if os.environ.get("CI") and not RUST_BIN:
    raise RuntimeError("CI must set KASSIBER_RUST_BIN; the Bitpanda CLI test cannot skip")


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to a `--features test-harness` build of crates/kassiber")
class RustBitpandaCliTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-bitpanda-cli-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"
        # Children must not resolve the owner's real app-data root.
        self.env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
        self.env["KASSIBER_OFFLINE_PREFERENCE_FILE"] = str(Path(self.tmp.name) / "offline-mode.json")
        self.cli("init")
        self.cli("workspaces", "create", "Main")
        self.cli("profiles", "create", "Book", "--fiat-currency", "EUR")

    def cli(self, *args, secret=None, check=True):
        result = subprocess.run(
            [RUST_BIN, "--data-root", str(self.data_root), "--machine", *args],
            input=None if secret is None else f"{secret}\n",
            capture_output=True, text=True, env=self.env, timeout=120, check=False,
        )
        if check:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn(KEY, result.stdout + result.stderr)
        return result

    def book_text(self):
        conn = sqlite3.connect(f"file:{self.data_root / 'kassiber.sqlite3'}?mode=ro", uri=True)
        try:
            cells = []
            for (table,) in conn.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"):
                if table == "backends":
                    continue
                for row in conn.execute(f'SELECT * FROM "{table}"'):
                    cells.extend(str(value) for value in row if isinstance(value, str))
            return "\n".join(cells)
        finally:
            conn.close()

    def save_backend(self, url):
        self.cli("backends", "create", "bitpanda-main", "--kind", "bitpanda", "--url", url,
                 "--token-stdin", secret=KEY)

    def test_saved_backend_is_configuration_and_the_sync_imports(self):
        exchange = Bitpanda()
        with serve(exchange) as backend:
            self.save_backend(backend["url"])
            self.cli("wallets", "list")
            self.cli("status")
            self.assertEqual(exchange.requests, [])
            result = self.cli("wallets", "sync-bitpanda", "--backend", "bitpanda-main")
        envelope = json.loads(result.stdout)
        self.assertEqual(envelope["kind"], "wallets.sync-bitpanda")
        self.assertEqual((envelope["data"]["imported"], envelope["data"]["wallet"]), (5, "Bitpanda"))
        self.assertTrue(all(r["method"] == "GET" for r in exchange.requests))
        listed = json.loads(self.cli("wallets", "list").stdout)["data"]
        self.assertEqual([(w["label"], w["kind"]) for w in listed], [("Bitpanda", "bitpanda")])
        self.assertNotIn(KEY, self.book_text())

    def test_offline_mode_stops_the_sync_before_any_request(self):
        exchange = Bitpanda()
        with serve(exchange) as backend:
            self.save_backend(backend["url"])
            self.cli("offline", "on")
            result = self.cli("wallets", "sync-bitpanda", "--backend", "bitpanda-main", check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout)["error"]["code"], "network_egress_disabled")
        self.assertEqual(exchange.requests, [])

    def test_an_echoed_key_is_withheld_from_output_and_book(self):
        exchange = Bitpanda()
        exchange.faults[1] = (401, {}, json.dumps({"error": {"code": f"invalid key {KEY}"}}).encode())
        with serve(exchange) as backend:
            self.save_backend(backend["url"])
            result = self.cli("wallets", "sync-bitpanda", "--backend", "bitpanda-main", check=False)
        error = json.loads(result.stdout)["error"]
        self.assertEqual(error["code"], "auth_error")
        self.assertEqual(error["details"]["bitpanda_error"], "invalid key <redacted>")
        self.assertNotIn(KEY, self.book_text())


if __name__ == "__main__":
    unittest.main()

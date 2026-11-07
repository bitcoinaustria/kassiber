"""The product, run as a CLI against loopback servers that echo credentials.

`tests/test_credential_echo.py` checks each error path in process. These
tests run the same failures through the command line (`tests/cli_runner.py`,
so `KASSIBER_TEST_CLI` points them at another executable such as the Rust
binary): a loopback server repeats every credential the request carried, and
neither the command's output nor any table of the book may show it. Only the
tables that hold the credentials by design are skipped.
"""

import os
import sqlite3
import stat
import sys
import tempfile
import unittest
from pathlib import Path

from tests import cli_runner
from tests.test_credential_echo import (
    AI_KEY,
    BTCPAY_KEY,
    ESPLORA_TOKEN,
    EXCHANGE_KEY,
    EXCHANGE_SECRET,
    MACAROON,
    RPC_PASSWORD,
    RPC_USER,
    RUNE,
    CredentialEchoAssertions,
    EchoServer,
    json_echo,
    text_echo,
)

# Tables that store a credential on purpose (the backend and provider rows).
CREDENTIAL_TABLES = frozenset({"backends", "ai_providers", "ai_provider_secret_refs"})
ADDRESS = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"


class CliCredentialEchoTest(CredentialEchoAssertions, unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-credential-echo-cli-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"
        # Children must not resolve the owner's real app-data root.
        self.env = {key: value for key, value in os.environ.items() if key != "XDG_DATA_HOME"}
        self.env.setdefault("KASSIBER_OFFLINE_PREFERENCE_FILE", str(Path(self.tmp.name) / "offline-mode.json"))
        self.cli("init")
        self.cli("workspaces", "create", "Main")
        self.cli("profiles", "create", "Book", "--fiat-currency", "EUR")

    def cli(self, *args, secret=None, check=True):
        result = cli_runner.run(
            ["--data-root", str(self.data_root), "--machine", *args],
            capture_output=True,
            text=True,
            input=None if secret is None else f"{secret}\n",
            secret_input=secret is not None,
            env=self.env,
            check=False,
        )
        if check:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def output(self, *args):
        result = self.cli(*args, check=False)
        return result.stdout + result.stderr

    def book(self):
        cells = []
        conn = sqlite3.connect(f"file:{self.data_root / 'kassiber.sqlite3'}?mode=ro", uri=True)
        try:
            tables = [row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")]
            for table in tables:
                if table in CREDENTIAL_TABLES:
                    continue
                for row in conn.execute(f'SELECT * FROM "{table}"'):
                    cells.extend(f"{table}: {value}" for value in row if isinstance(value, str))
        finally:
            conn.close()
        return "\n".join(cells)

    def assertNoEcho(self, output, credentials):
        self.assertWithheld({"output": output, "book": self.book()}, credentials)

    def test_btcpay_store_sync(self):
        with EchoServer(json_echo(500)) as server:
            self.cli("backends", "create", "shop", "--kind", "btcpay", "--url", server.url, "--token-stdin", secret=BTCPAY_KEY)
            output = self.output("btcpay", "provenance", "sync", "--backend", "shop", "--store-id", "S1")
        self.assertIn("BTCPay returned HTTP 500", output)
        self.assertIn("btcpay_store_sync_states: protocol_error", self.book())
        self.assertNoEcho(output, (BTCPAY_KEY,))

    def test_lnd_wallet_sync(self):
        with EchoServer(text_echo(500)) as server:
            self.cli("backends", "create", "node", "--kind", "lnd", "--url", server.url, "--token-stdin", secret=MACAROON)
            self.cli("wallets", "create", "--label", "Node", "--kind", "lnd", "--backend", "node")
            output = self.output("wallets", "sync", "--wallet", "Node")
        self.assertIn("LND returned HTTP 500", output)
        self.assertNoEcho(output, (MACAROON,))

    def test_esplora_wallet_sync(self):
        with EchoServer(json_echo(401)) as server:
            self.cli("backends", "create", "explorer", "--kind", "esplora", "--url", f"{server.url}/api", "--token-stdin", secret=ESPLORA_TOKEN)
            self.cli("wallets", "create", "--label", "Cold", "--kind", "address", "--address", ADDRESS, "--backend", "explorer")
            output = self.output("wallets", "sync", "--wallet", "Cold")
        self.assertIn("HTTP 401", output)
        self.assertNoEcho(output, (ESPLORA_TOKEN,))

    def test_bitcoin_core_wallet_sync(self):
        with EchoServer(text_echo(401)) as server:
            self.cli(
                "backends", "create", "core", "--kind", "bitcoinrpc", "--url", server.url,
                "--username", RPC_USER, "--password-stdin", secret=RPC_PASSWORD,
            )
            self.cli("wallets", "create", "--label", "Cold", "--kind", "address", "--address", ADDRESS, "--backend", "core")
            output = self.output("wallets", "sync", "--wallet", "Cold")
        self.assertIn("HTTP 401", output)
        self.assertNoEcho(output, (RPC_PASSWORD, f"{RPC_USER}:{RPC_PASSWORD}"))

    def test_kraken_sync(self):
        with EchoServer(text_echo(403)) as server:
            self.cli(
                "backends", "create", "kraken", "--kind", "kraken", "--url", server.url,
                "--auth-header", EXCHANGE_SECRET, "--token-stdin", secret=EXCHANGE_KEY,
            )
            output = self.output("wallets", "sync-kraken", "--backend", "kraken")
        self.assertIn("HTTP 403", output)
        self.assertNoEcho(output, (EXCHANGE_KEY, EXCHANGE_SECRET))

    def test_ai_provider_test(self):
        with EchoServer(json_echo(401)) as server:
            self.cli(
                "ai", "providers", "create", "remote", "--base-url", f"{server.url}/v1",
                "--api-key-stdin", "--default-model", "m", "--kind", "remote", secret=AI_KEY,
            )
            self.cli("ai", "providers", "update", "remote", "--acknowledge")
            output = self.output("ai", "providers", "test", "remote")
        self.assertIn("ai_auth_failed", output)
        self.assertNoEcho(output, (AI_KEY,))

    def test_core_lightning_wallet_sync(self):
        script = Path(self.tmp.name) / "lightning-cli"
        script.write_text(
            f"#!{sys.executable}\nimport sys\n"
            "sys.stderr.write('commando failed: ' + ' '.join(sys.argv[1:]))\nsys.exit(1)\n",
            encoding="utf-8",
        )
        script.chmod(stat.S_IRWXU)
        self.cli(
            "backends", "create", "cln", "--kind", "coreln", "--url", "cln://commando",
            "--lightning-cli", str(script), "--commando-peer-id", "02" + "11" * 32, "--token-stdin", secret=RUNE,
        )
        self.cli("wallets", "create", "--label", "Node", "--kind", "coreln", "--backend", "cln")
        output = self.output("wallets", "sync", "--wallet", "Node")
        self.assertIn("commando failed", output)
        self.assertNoEcho(output, (RUNE,))


if __name__ == "__main__":
    unittest.main()

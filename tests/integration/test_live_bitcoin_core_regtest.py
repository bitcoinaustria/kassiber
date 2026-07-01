from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from urllib import request

from tests.integration.env import skip_unless_integration


ROOT = Path(__file__).resolve().parents[2]


def _rpc(url: str, username: str, password: str, method: str, params=None, wallet=None):
    endpoint = url.rstrip("/")
    if wallet:
        endpoint = f"{endpoint}/wallet/{wallet}"
    payload = json.dumps(
        {
            "jsonrpc": "1.0",
            "id": "kassiber-live-regtest",
            "method": method,
            "params": [] if params is None else params,
        }
    ).encode("utf-8")
    req = request.Request(endpoint, data=payload, headers={"Content-Type": "application/json"})
    import base64

    token = base64.b64encode(f"{username}:{password}".encode("utf-8")).decode("ascii")
    req.add_header("Authorization", f"Basic {token}")
    with request.urlopen(req, timeout=30) as response:
        decoded = json.loads(response.read().decode("utf-8"))
    if decoded.get("error"):
        raise AssertionError(f"RPC {method} failed: {decoded['error']}")
    return decoded.get("result")


def _run(data_root: Path, *args: str):
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            "--machine",
            *args,
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(
            f"kassiber {' '.join(args)} failed\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    return json.loads(result.stdout)


@skip_unless_integration
class LiveBitcoinCoreRegtestTest(unittest.TestCase):
    def test_core_rpc_watch_only_sync_journal_and_export(self):
        url = os.environ.get("KASSIBER_REGTEST_CORE_URL", "http://127.0.0.1:18443")
        username = os.environ.get("KASSIBER_REGTEST_RPC_USER", "kassiber")
        password = os.environ.get("KASSIBER_REGTEST_RPC_PASSWORD", "kassiber")

        chain = _rpc(url, username, password, "getblockchaininfo")
        self.assertEqual(chain["chain"], "regtest")

        source_wallet = f"kassiber-src-{os.getpid()}"
        try:
            _rpc(url, username, password, "createwallet", [source_wallet, False, False, "", False, True, True])
        except AssertionError as exc:
            if "already exists" not in str(exc):
                raise
        address = _rpc(url, username, password, "getnewaddress", ["kassiber receive", "bech32"], wallet=source_wallet)

        with tempfile.TemporaryDirectory() as tmp:
            data_root = Path(tmp) / "data"
            xlsx_file = Path(tmp) / "core-regtest.xlsx"

            _run(data_root, "init")
            _run(data_root, "workspaces", "create", "Regtest")
            _run(
                data_root,
                "profiles",
                "create",
                "Replay",
                "--workspace",
                "Regtest",
                "--fiat-currency",
                "EUR",
                "--tax-country",
                "generic",
                "--gains-algorithm",
                "FIFO",
            )
            _run(
                data_root,
                "backends",
                "create",
                "core-regtest",
                "--kind",
                "bitcoinrpc",
                "--url",
                url,
                "--chain",
                "bitcoin",
                "--network",
                "regtest",
                "--username",
                username,
                "--password",
                password,
                "--wallet-prefix",
                f"kassiber-test-{os.getpid()}",
                "--timeout",
                "30",
            )
            _run(
                data_root,
                "wallets",
                "create",
                "--label",
                "Core regtest",
                "--kind",
                "address",
                "--backend",
                "core-regtest",
                "--chain",
                "bitcoin",
                "--network",
                "regtest",
                "--address",
                address,
            )
            first_sync = _run(data_root, "wallets", "sync", "--wallet", "Core regtest")
            first_sync_data = first_sync["data"][0] if isinstance(first_sync["data"], list) else first_sync["data"]
            self.assertEqual(first_sync_data["backend_kind"], "bitcoinrpc")

            send_address = _rpc(
                url,
                username,
                password,
                "getnewaddress",
                ["external", "bech32"],
                wallet=source_wallet,
            )
            _rpc(url, username, password, "generatetoaddress", [101, address])
            _rpc(url, username, password, "sendtoaddress", [send_address, 0.01], wallet=source_wallet)
            _rpc(url, username, password, "generatetoaddress", [1, address])

            sync = _run(data_root, "wallets", "sync", "--wallet", "Core regtest")
            sync_data = sync["data"][0] if isinstance(sync["data"], list) else sync["data"]
            self.assertEqual(sync_data["backend_kind"], "bitcoinrpc")
            self.assertGreaterEqual(sync_data["records_fetched"], 1)

            transactions = _run(data_root, "transactions", "list", "--limit", "100")
            self.assertGreaterEqual(len(transactions["data"]), 1)
            for row in transactions["data"]:
                _run(data_root, "rates", "set", "BTC-EUR", row["occurred_at"], "30000")
            journal = _run(data_root, "journals", "process")
            self.assertGreaterEqual(journal["data"]["entries_created"], 1)
            summary = _run(data_root, "reports", "summary")
            self.assertGreaterEqual(summary["data"]["metrics"]["active_transactions"], 1)
            export = _run(data_root, "reports", "export-xlsx", "--file", str(xlsx_file))
            self.assertEqual(export["data"]["file"], str(xlsx_file))
            self.assertTrue(xlsx_file.exists())


if __name__ == "__main__":
    unittest.main()

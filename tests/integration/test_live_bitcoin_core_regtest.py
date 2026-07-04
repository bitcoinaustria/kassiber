from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
import uuid
from pathlib import Path
from urllib import request

from kassiber.core.sync_backends import sanitize_wallet_segment

from tests.integration.env import skip_unless_integration


ROOT = Path(__file__).resolve().parents[2]
RPC_TIMEOUT = float(os.environ.get("KASSIBER_REGTEST_RPC_TIMEOUT", "300"))


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
    with request.urlopen(req, timeout=RPC_TIMEOUT) as response:
        decoded = json.loads(response.read().decode("utf-8"))
    if decoded.get("error"):
        raise AssertionError(f"RPC {method} failed: {decoded['error']}")
    return decoded.get("result")


def _run(data_root: Path, *args: str, pass_fds: tuple[int, ...] = ()):
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
        pass_fds=pass_fds,
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

        run_id = uuid.uuid4().hex[:12]
        source_wallet = f"kassiber-src-{run_id}"
        wallet_prefix = f"kassiber-test-{run_id}"
        created_core_wallets = [source_wallet]

        try:
            _rpc(url, username, password, "createwallet", [source_wallet, False, False, "", False, True, True])
            address = _rpc(
                url,
                username,
                password,
                "getnewaddress",
                ["kassiber receive", "bech32"],
                wallet=source_wallet,
            )

            with tempfile.TemporaryDirectory() as tmp:
                # resolve() so path assertions survive macOS /var -> /private/var symlinks
                data_root = Path(tmp).resolve() / "data"
                xlsx_file = Path(tmp).resolve() / "core-regtest.xlsx"

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
                with tempfile.TemporaryFile("w+") as username_fd, tempfile.TemporaryFile("w+") as password_fd:
                    username_fd.write(username)
                    username_fd.flush()
                    username_fd.seek(0)
                    password_fd.write(password)
                    password_fd.flush()
                    password_fd.seek(0)
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
                        "--username-fd",
                        str(username_fd.fileno()),
                        "--password-fd",
                        str(password_fd.fileno()),
                        "--wallet-prefix",
                        wallet_prefix,
                        "--timeout",
                        "30",
                        pass_fds=(username_fd.fileno(), password_fd.fileno()),
                    )
                wallet_create = _run(
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
                wallet_data = wallet_create["data"]
                created_core_wallets.append(
                    f"{sanitize_wallet_segment(wallet_prefix)}-{sanitize_wallet_segment(wallet_data['id'])}"
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
                # Mining to the watched address exercises coinbase handling:
                # only matured "generate" rows may import, immature ones must not.
                _rpc(url, username, password, "generatetoaddress", [101, address])
                _rpc(url, username, password, "sendtoaddress", [send_address, 0.01], wallet=source_wallet)
                _rpc(url, username, password, "sendtoaddress", [address, 0.015], wallet=source_wallet)
                _rpc(url, username, password, "generatetoaddress", [1, address])

                sync = _run(data_root, "wallets", "sync", "--wallet", "Core regtest")
                sync_data = sync["data"][0] if isinstance(sync["data"], list) else sync["data"]
                self.assertEqual(sync_data["backend_kind"], "bitcoinrpc")
                self.assertGreaterEqual(sync_data["records_fetched"], 1)

                transactions = _run(data_root, "transactions", "list", "--limit", "100")
                self.assertGreaterEqual(len(transactions["data"]), 2)
                directions = {row["direction"] for row in transactions["data"]}
                # mature coinbase + explicit receive inbound, watched spend outbound
                self.assertEqual(directions, {"inbound", "outbound"})
                for row in transactions["data"]:
                    _run(data_root, "rates", "set", "BTC-EUR", row["occurred_at"], "30000")
                journal = _run(data_root, "journals", "process")
                self.assertGreaterEqual(journal["data"]["entries_created"], 1)
                summary = _run(data_root, "reports", "summary")
                self.assertGreaterEqual(summary["data"]["metrics"]["active_transactions"], 1)
                export = _run(data_root, "reports", "export-xlsx", "--file", str(xlsx_file))
                self.assertEqual(export["data"]["file"], str(xlsx_file))
                self.assertTrue(xlsx_file.exists())
        finally:
            # Best-effort teardown: a failed unload must not fail the test (the
            # wallet may never have been created if setup aborted early), but we
            # still surface it so a leaked regtest wallet is diagnosable.
            for wallet_name in reversed(created_core_wallets):
                try:
                    _rpc(url, username, password, "unloadwallet", [wallet_name])
                except AssertionError as exc:
                    print(f"cleanup: could not unload {wallet_name}: {exc}", file=sys.stderr)


if __name__ == "__main__":
    unittest.main()

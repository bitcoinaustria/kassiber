"""Silent Payments through the Rust binary against the lane's Frigate node.

`integration-harness.sh silent-payments` starts the regtest stack with the
Frigate Electrum server (Silent Payments index) and waits for it. This smoke
drives only the `kassiber` binary and the regtest services: a Frigate backend
marked for Silent Payments, a watch-only silent-payment wallet, and a sync
whose scan result names a real Core transaction, so the imported transaction
and UTXO are checked against Bitcoin Core's own view of it. The scan result
uses the demo's scan-file route (`regtest_demo._write_silent_payment_scan_files`)
because Core cannot pay a BIP352 address itself.

Product refusals are checked through the CLI too: spending material in the
descriptor, a world-readable scan file, and an ordinary Electrum backend.
"""
from __future__ import annotations

import base64
import json
import os
import socket
import tempfile
import unittest
import uuid
from datetime import datetime, timezone
from decimal import Decimal
from pathlib import Path
from urllib import request

from tests.integration import cli
from tests.integration.book import descriptor_fingerprint, open_book
from tests.integration.env import skip_unless_integration

ROOT = Path(__file__).resolve().parents[2]
SCENARIO = ROOT / "dev" / "regtest" / "scenarios" / "full_accounting.json"
MSAT_PER_SAT = 1000


def _scenario_descriptor() -> str:
    scenario = json.loads(SCENARIO.read_text(encoding="utf-8"))
    return next(
        str(wallet["sp_descriptor"])
        for wallet in scenario["wallets"]
        if wallet.get("kind") == "silent-payment"
    )


def _rpc(method: str, params: list | None = None, *, wallet: str | None = None):
    url = os.environ.get("KASSIBER_REGTEST_CORE_URL", "http://127.0.0.1:18443").rstrip("/")
    if wallet:
        url += f"/wallet/{wallet}"
    user = os.environ.get("KASSIBER_REGTEST_RPC_USER", "kassiber")
    password = os.environ.get("KASSIBER_REGTEST_RPC_PASSWORD", "kassiber")
    payload = json.dumps({"jsonrpc": "1.0", "id": "kassiber-sp", "method": method, "params": params or []})
    req = request.Request(url, data=payload.encode("utf-8"), headers={"Content-Type": "application/json"})
    token = base64.b64encode(f"{user}:{password}".encode("utf-8")).decode("ascii")
    req.add_header("Authorization", f"Basic {token}")
    with request.urlopen(req, timeout=120) as response:
        body = json.loads(response.read().decode("utf-8"))
    if body.get("error"):
        raise AssertionError(f"RPC {method} failed: {body['error']}")
    return body.get("result")


def _frigate_features() -> dict:
    """`server.features` from Frigate, which wants `server.version` first."""
    port = int(os.environ.get("KASSIBER_REGTEST_FRIGATE_PORT", "18548"))
    with socket.create_connection(("127.0.0.1", port), timeout=10) as sock:
        stream = sock.makefile("rwb")
        results = []
        for ident, method, params in (
            ("version", "server.version", ["Kassiber silent payments smoke", "1.6"]),
            ("features", "server.features", []),
        ):
            stream.write(json.dumps({"jsonrpc": "2.0", "id": ident, "method": method, "params": params}).encode() + b"\n")
            stream.flush()
            response = json.loads(stream.readline().decode("utf-8"))
            if response.get("error"):
                raise AssertionError(f"Frigate {method} failed: {response['error']}")
            results.append(response.get("result"))
    return results[1]


def _frigate_url() -> str:
    return f"tcp://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_FRIGATE_PORT', '18548')}"


def _fulcrum_url() -> str:
    return f"tcp://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT', '18543')}"


@skip_unless_integration
class LiveSilentPaymentsTest(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        self.assertEqual(_rpc("getblockchaininfo")["chain"], "regtest")
        directory = tempfile.TemporaryDirectory(prefix="kassiber-live-sp-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name).resolve()
        self.root = self.base / "data"
        self.faucet = f"kassiber-sp-faucet-{uuid.uuid4().hex[:12]}"
        _rpc("createwallet", [self.faucet])
        self.addCleanup(self._unload)
        self.mining = _rpc("getnewaddress", ["mining", "bech32"], wallet=self.faucet)
        _rpc("generatetoaddress", [101, self.mining])

    def _unload(self):
        try:
            _rpc("unloadwallet", [self.faucet])
        except Exception:  # noqa: BLE001 - teardown must not mask the result
            pass

    def run_cli(self, *args: str, ok: bool = True) -> dict:
        result = cli.run(
            ["--data-root", str(self.root), "--machine", *args],
            capture_output=True, text=True, check=False, timeout=600,
        )
        try:
            payload = json.loads(result.stdout)
        except json.JSONDecodeError as exc:
            raise AssertionError(f"kassiber {' '.join(args)}: no envelope\n{result.stdout}\n{result.stderr}") from exc
        if ok:
            self.assertEqual(result.returncode, 0, f"kassiber {' '.join(args)}: {result.stdout}{result.stderr}")
        else:
            self.assertNotEqual(result.returncode, 0, f"kassiber {' '.join(args)} unexpectedly succeeded")
        return payload

    def write_scan(self, descriptor: str, start_height: int) -> tuple[Path, dict]:
        """A complete scan result naming one real, confirmed Core output."""
        receive = _rpc("getnewaddress", ["sp detected receive", "bech32m"], wallet=self.faucet)
        txid = _rpc("sendtoaddress", [receive, "0.01234567"], wallet=self.faucet)
        block_hash = _rpc("generatetoaddress", [1, self.mining])[0]
        tx = _rpc("getrawtransaction", [txid, True])
        block = _rpc("getblock", [block_hash])
        self.assertEqual(tx["blockhash"], block_hash)
        output = next(row for row in tx["vout"] if row["scriptPubKey"].get("address") == receive)
        # The faucet must not spend the output the scan claims for the wallet.
        _rpc("lockunspent", [False, [{"txid": txid, "vout": int(output["n"])}]], wallet=self.faucet)
        sats = int(Decimal(str(output["value"])) * 100_000_000)
        row = {
            "txid": txid,
            "vout": int(output["n"]),
            "amount_sats": sats,
            "script_pubkey": output["scriptPubKey"]["hex"],
            "silent_payment": True,
            "block_height": int(block["height"]),
            "block_time": _iso(int(block["time"])),
            "confirmations": int(_rpc("getrawtransaction", [txid, True]).get("confirmations") or 1),
            "raw": {"source": "regtest-frigate-silent-payments"},
        }
        payload = {
            "schema_version": 1,
            "complete": True,
            "descriptor_fingerprint": descriptor_fingerprint(descriptor),
            "range": {"from_height": start_height, "to_height": int(block["height"])},
            "transactions": [{
                "txid": txid,
                "block_height": row["block_height"],
                "block_time": row["block_time"],
                "confirmations": row["confirmations"],
                "outputs": [row],
            }],
            "utxos": [row],
        }
        path = self.base / "scan.json"
        path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        os.chmod(path, 0o600)
        return path, row

    def test_frigate_backend_wallet_sync_matches_core(self):
        features = _frigate_features()
        self.assertIn(0, list(features.get("silent_payments") or []), features)

        descriptor = _scenario_descriptor()
        scan_path, truth = self.write_scan(descriptor, 0)
        descriptor_path = self.base / "sp-descriptor.txt"
        descriptor_path.write_text(descriptor + "\n", encoding="utf-8")
        os.chmod(descriptor_path, 0o600)

        self.run_cli("init")
        self.run_cli("workspaces", "create", "Regtest")
        self.run_cli("profiles", "create", "Silent", "--workspace", "Regtest", "--fiat-currency", "EUR",
                     "--tax-country", "generic", "--gains-algorithm", "FIFO")
        scope = ("--workspace", "Regtest", "--profile", "Silent")
        self.run_cli("backends", "create", "frigate-sp", "--kind", "electrum", "--url", _frigate_url(),
                     "--chain", "bitcoin", "--network", "regtest", "--timeout", "30",
                     "--silent-payments", "--silent-payment-scan-file", str(scan_path))
        self.run_cli("backends", "create", "fulcrum-plain", "--kind", "electrum", "--url", _fulcrum_url(),
                     "--chain", "bitcoin", "--network", "regtest", "--timeout", "30")

        backends = {row["name"]: row for row in self.run_cli("backends", "list")["data"]}
        self.assertIs(backends["frigate-sp"].get("silent_payments"), True, backends["frigate-sp"])
        self.assertNotIn("silent_payment_scan_file", backends["frigate-sp"])
        self.assertNotIn(str(scan_path), json.dumps(backends))
        self.assertFalse(backends["fulcrum-plain"].get("silent_payments"), backends["fulcrum-plain"])

        # Spending material never becomes a watch-only wallet.
        self.assert_spending_refused(scope)

        self.run_cli("wallets", "create", *scope, "--label", "SP watch", "--kind", "silent-payment",
                     "--backend", "frigate-sp", "--chain", "bitcoin", "--network", "regtest",
                     "--sp-descriptor-file", str(descriptor_path), "--sp-scan-start-height", "0")
        wallet_json = json.dumps(self.run_cli("wallets", "list", *scope)["data"])
        self.assertNotIn(descriptor, wallet_json)

        first = self.run_cli("wallets", "sync", *scope, "--wallet", "SP watch")["data"]
        first = first[0] if isinstance(first, list) else first
        self.assertEqual(first.get("imported"), 1, first)
        repeat = self.run_cli("wallets", "sync", *scope, "--wallet", "SP watch")["data"]
        repeat = repeat[0] if isinstance(repeat, list) else repeat
        self.assertEqual(repeat.get("imported"), 0, repeat)

        conn = open_book(self.root, readonly=True)
        try:
            transactions = [dict(row) for row in conn.execute(
                "SELECT external_id, direction, asset, amount, confirmed_at FROM transactions")]
            utxos = [dict(row) for row in conn.execute(
                "SELECT txid, vout, asset, amount, block_height, spent_at FROM wallet_utxos")]
            stored = json.dumps([dict(row) for row in conn.execute("SELECT config_json FROM wallets")])
        finally:
            conn.close()
        self.assertEqual(len(transactions), 1, transactions)
        self.assertEqual(transactions[0]["external_id"], truth["txid"])
        self.assertEqual(transactions[0]["direction"], "inbound")
        self.assertEqual(transactions[0]["asset"], "BTC")
        self.assertEqual(transactions[0]["amount"], truth["amount_sats"] * MSAT_PER_SAT)
        self.assertIsNotNone(transactions[0]["confirmed_at"])
        self.assertEqual(utxos, [{
            "txid": truth["txid"], "vout": truth["vout"], "asset": "BTC",
            "amount": truth["amount_sats"] * MSAT_PER_SAT, "block_height": truth["block_height"],
            "spent_at": None,
        }])
        self.assertNotIn(str(scan_path), stored)

        # An ordinary Electrum server cannot scan for Silent Payments.
        self.run_cli("wallets", "update", *scope, "--wallet", "SP watch", "--backend", "fulcrum-plain")
        refused = self.run_cli("wallets", "sync", *scope, "--wallet", "SP watch", ok=False)
        self.assertEqual(self.error_code(refused), "silent_payment_backend_unsupported", refused)

        # A scan result other accounts can read is refused, not imported.
        self.run_cli("wallets", "update", *scope, "--wallet", "SP watch", "--backend", "frigate-sp")
        os.chmod(scan_path, 0o644)
        exposed = self.run_cli("wallets", "sync", *scope, "--wallet", "SP watch", ok=False)
        self.assertEqual(exposed.get("kind"), "error", exposed)
        os.chmod(scan_path, 0o600)

    def assert_spending_refused(self, scope):
        result = cli.run(
            ["--data-root", str(self.root), "--machine", "wallets", "create", *scope, "--label", "Spend",
             "--kind", "silent-payment", "--backend", "frigate-sp", "--chain", "bitcoin",
             "--network", "regtest", "--sp-descriptor-stdin", "--sp-scan-start-height", "0"],
            input="sp(tspspend1q" + "q" * 40 + ")\n", capture_output=True, text=True, check=False, timeout=120,
        )
        self.assertNotEqual(result.returncode, 0, result.stdout)
        payload = json.loads(result.stdout)
        self.assertEqual(self.error_code(payload), "wallet_spending_private_material", payload)
        self.assertNotIn("tspspend1", result.stdout + result.stderr)

    @staticmethod
    def error_code(payload: dict) -> str | None:
        error = payload.get("error")
        return error.get("code") if isinstance(error, dict) else None


def _iso(timestamp: int) -> str:
    return datetime.fromtimestamp(timestamp, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


if __name__ == "__main__":
    unittest.main()

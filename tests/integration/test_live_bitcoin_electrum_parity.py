from __future__ import annotations

import base64
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import uuid
from decimal import Decimal
from pathlib import Path
from urllib import parse, request

from kassiber.db import open_db

from tests.integration.env import skip_unless_integration


ROOT = Path(__file__).resolve().parents[2]
SAT = Decimal("0.00000001")
APP_NAME = "kassiber"
RPC_TIMEOUT = float(os.environ.get("KASSIBER_REGTEST_RPC_TIMEOUT", "300"))


def _sanitize_wallet_segment(value: str) -> str:
    text = str(value).strip().lower()
    cleaned = []
    for char in text:
        if char.isalnum() or char in {"-", "_", "."}:
            cleaned.append(char)
        else:
            cleaned.append("-")
    sanitized = "".join(cleaned).strip("-")
    return sanitized or APP_NAME


def _rpc(url: str, username: str, password: str, method: str, params=None, wallet=None):
    endpoint = url.rstrip("/")
    if wallet:
        endpoint = f"{endpoint}/wallet/{wallet}"
    payload = json.dumps(
        {
            "jsonrpc": "1.0",
            "id": "kassiber-live-electrum-parity",
            "method": method,
            "params": [] if params is None else params,
        }
    ).encode("utf-8")
    req = request.Request(endpoint, data=payload, headers={"Content-Type": "application/json"})
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


def _electrum_url() -> str:
    return (
        os.environ.get("KASSIBER_REGTEST_ELECTRUM_URL")
        or f"tcp://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT', '18543')}"
    )


def _electrum_endpoint(url: str) -> tuple[str, int]:
    parsed = parse.urlsplit(url if "://" in url else f"tcp://{url}")
    if parsed.scheme != "tcp":
        raise AssertionError(f"Live Fulcrum parity test expects a tcp:// Electrum URL, got {url!r}")
    if not parsed.hostname or not parsed.port:
        raise AssertionError(f"Invalid Electrum URL: {url!r}")
    return parsed.hostname, parsed.port


def _electrum_call(url: str, method: str, params=None):
    host, port = _electrum_endpoint(url)
    payload = {
        "jsonrpc": "2.0",
        "id": f"kassiber-{method}",
        "method": method,
        "params": [] if params is None else list(params),
    }
    with socket.create_connection((host, port), timeout=5) as sock:
        sock.sendall(json.dumps(payload).encode("utf-8") + b"\n")
        raw = b""
        while not raw.endswith(b"\n"):
            chunk = sock.recv(65536)
            if not chunk:
                raise AssertionError("Electrum server closed the connection")
            raw += chunk
    response = json.loads(raw.decode("utf-8"))
    if response.get("error"):
        raise AssertionError(f"Electrum {method} failed: {response['error']}")
    return response.get("result")


def _wait_for_electrum(url: str, *, min_height: int, txids: list[str] | None = None) -> None:
    deadline = time.monotonic() + 180
    last_error: Exception | None = None
    txids = txids or []
    while time.monotonic() < deadline:
        try:
            _electrum_call(url, "server.version", ["Kassiber parity test", "1.4"])
            header = _electrum_call(url, "blockchain.headers.subscribe")
            height = int((header or {}).get("height") or 0)
            if height >= min_height:
                for txid in txids:
                    _electrum_call(url, "blockchain.transaction.get", [txid])
                return
        except Exception as exc:  # Fulcrum may still be booting or indexing.
            last_error = exc
        time.sleep(2)
    raise AssertionError(
        f"Timed out waiting for Electrum/Fulcrum at {url} to reach height {min_height}"
        + (f": {last_error}" if last_error else "")
    )


def _create_backend_and_wallet(
    data_root: Path,
    *,
    backend_kind: str,
    backend_url: str,
    label: str,
    addresses: list[str],
    rpc_username: str | None = None,
    rpc_password: str | None = None,
    wallet_prefix: str | None = None,
) -> dict:
    _run(data_root, "init")
    _run(data_root, "workspaces", "create", "Parity")
    _run(
        data_root,
        "profiles",
        "create",
        "Default",
        "--workspace",
        "Parity",
        "--fiat-currency",
        "EUR",
        "--tax-country",
        "generic",
        "--gains-algorithm",
        "FIFO",
    )
    backend_name = f"{backend_kind}-regtest"
    backend_args = [
        "backends",
        "create",
        backend_name,
        "--kind",
        backend_kind,
        "--url",
        backend_url,
        "--chain",
        "bitcoin",
        "--network",
        "regtest",
        "--timeout",
        "30",
        "--batch-size",
        "25",
    ]
    pass_fds: tuple[int, ...] = ()
    if backend_kind == "bitcoinrpc":
        if rpc_username is None or rpc_password is None:
            raise AssertionError("bitcoinrpc parity backend requires RPC credentials")
        username_fd = tempfile.TemporaryFile("w+")
        password_fd = tempfile.TemporaryFile("w+")
        with username_fd, password_fd:
            username_fd.write(rpc_username)
            username_fd.flush()
            username_fd.seek(0)
            password_fd.write(rpc_password)
            password_fd.flush()
            password_fd.seek(0)
            backend_args.extend(
                [
                    "--username-fd",
                    str(username_fd.fileno()),
                    "--password-fd",
                    str(password_fd.fileno()),
                    "--wallet-prefix",
                    wallet_prefix or "kassiber-parity",
                ]
            )
            pass_fds = (username_fd.fileno(), password_fd.fileno())
            _run(data_root, *backend_args, pass_fds=pass_fds)
    else:
        _run(data_root, *backend_args)

    address_args: list[str] = []
    for address in addresses:
        address_args.extend(["--address", address])
    wallet_create = _run(
        data_root,
        "wallets",
        "create",
        "--workspace",
        "Parity",
        "--profile",
        "Default",
        "--label",
        label,
        "--kind",
        "address",
        "--backend",
        backend_name,
        "--chain",
        "bitcoin",
        "--network",
        "regtest",
        *address_args,
    )
    return wallet_create["data"]


def _sync(data_root: Path, wallet_label: str) -> dict:
    payload = _run(data_root, "wallets", "sync", "--wallet", wallet_label)
    data = payload["data"]
    if isinstance(data, list):
        if len(data) != 1:
            raise AssertionError(f"Expected one wallet sync result, got {data}")
        return data[0]
    return data


def _transaction_projection(data_root: Path) -> list[dict]:
    conn = open_db(data_root)
    try:
        rows = conn.execute(
            """
            SELECT external_id, occurred_at, confirmed_at, direction, asset,
                   amount, fee, amount_includes_fee, kind, raw_json
            FROM transactions
            ORDER BY external_id, direction, asset
            """
        ).fetchall()
    finally:
        conn.close()
    projected = []
    for row in rows:
        raw = json.loads(row["raw_json"] or "{}")
        projected.append(
            {
                "external_id": row["external_id"],
                "occurred_at": row["occurred_at"],
                "confirmed_at": row["confirmed_at"],
                "direction": row["direction"],
                "asset": row["asset"],
                "amount": row["amount"],
                "fee": row["fee"],
                "amount_includes_fee": row["amount_includes_fee"],
                "kind": row["kind"],
                "raw_payload_present": bool(raw),
            }
        )
    return projected


def _utxo_projection(data_root: Path) -> list[dict]:
    conn = open_db(data_root)
    try:
        rows = conn.execute(
            """
            SELECT txid, vout, asset, amount, confirmation_status,
                   block_height, spent_at IS NOT NULL AS spent
            FROM wallet_utxos
            ORDER BY txid, vout
            """
        ).fetchall()
    finally:
        conn.close()
    return [dict(row) for row in rows]


def _assert_books_match(testcase: unittest.TestCase, core_root: Path, electrum_root: Path) -> None:
    testcase.assertEqual(_transaction_projection(core_root), _transaction_projection(electrum_root))
    testcase.assertEqual(_utxo_projection(core_root), _utxo_projection(electrum_root))


def _send_all_from_wallet(
    url: str,
    username: str,
    password: str,
    *,
    wallet: str,
    destination: str,
    fee_btc: Decimal = Decimal("0.00001000"),
) -> str:
    utxos = _rpc(url, username, password, "listunspent", [1, 9999999, []], wallet=wallet)
    if not utxos:
        raise AssertionError(f"Wallet {wallet} has no confirmed UTXOs to spend")
    total = sum(Decimal(str(row["amount"])) for row in utxos)
    amount = (total - fee_btc).quantize(SAT)
    if amount <= 0:
        raise AssertionError(f"Wallet {wallet} balance {total} is too small for fee {fee_btc}")
    inputs = [{"txid": row["txid"], "vout": row["vout"]} for row in utxos]
    raw = _rpc(
        url,
        username,
        password,
        "createrawtransaction",
        [inputs, {destination: float(amount)}],
    )
    signed = _rpc(url, username, password, "signrawtransactionwithwallet", [raw], wallet=wallet)
    if not signed.get("complete"):
        raise AssertionError(f"Wallet {wallet} did not fully sign spend-all transaction: {signed}")
    return _rpc(url, username, password, "sendrawtransaction", [signed["hex"]])


@skip_unless_integration
class LiveBitcoinElectrumParityTest(unittest.TestCase):
    def test_fulcrum_electrum_matches_core_rpc_for_address_wallet(self):
        core_url = os.environ.get("KASSIBER_REGTEST_CORE_URL", "http://127.0.0.1:18443")
        username = os.environ.get("KASSIBER_REGTEST_RPC_USER", "kassiber")
        password = os.environ.get("KASSIBER_REGTEST_RPC_PASSWORD", "kassiber")
        electrum_url = _electrum_url()

        chain = _rpc(core_url, username, password, "getblockchaininfo")
        self.assertEqual(chain["chain"], "regtest")

        run_id = uuid.uuid4().hex[:12]
        faucet_wallet = f"kassiber-faucet-{run_id}"
        owner_wallet = f"kassiber-owner-{run_id}"
        created_wallets = [faucet_wallet, owner_wallet]

        try:
            _rpc(core_url, username, password, "createwallet", [faucet_wallet, False, False, "", False, True, True])
            _rpc(core_url, username, password, "createwallet", [owner_wallet, False, False, "", False, True, True])
            mining_address = _rpc(
                core_url,
                username,
                password,
                "getnewaddress",
                ["mining", "bech32"],
                wallet=faucet_wallet,
            )
            watched_addresses = [
                _rpc(
                    core_url,
                    username,
                    password,
                    "getnewaddress",
                    [f"watched {index}", "bech32"],
                    wallet=owner_wallet,
                )
                for index in range(2)
            ]
            external_address = _rpc(
                core_url,
                username,
                password,
                "getnewaddress",
                ["external", "bech32"],
                wallet=faucet_wallet,
            )

            _rpc(core_url, username, password, "generatetoaddress", [101, mining_address])
            funding_txid = _rpc(
                core_url,
                username,
                password,
                "sendmany",
                ["", {watched_addresses[0]: 1.25, watched_addresses[1]: 0.75}],
                wallet=faucet_wallet,
            )
            _rpc(core_url, username, password, "generatetoaddress", [1, mining_address])
            chain = _rpc(core_url, username, password, "getblockchaininfo")
            _wait_for_electrum(electrum_url, min_height=int(chain["blocks"]), txids=[funding_txid])

            with tempfile.TemporaryDirectory(prefix="kassiber-electrum-parity-") as tmp:
                base = Path(tmp).resolve()
                core_root = base / "core"
                electrum_root = base / "electrum"
                wallet_prefix = f"kassiber-parity-{_sanitize_wallet_segment(run_id)}"
                core_wallet = _create_backend_and_wallet(
                    core_root,
                    backend_kind="bitcoinrpc",
                    backend_url=core_url,
                    label="Core parity",
                    addresses=watched_addresses,
                    rpc_username=username,
                    rpc_password=password,
                    wallet_prefix=wallet_prefix,
                )
                created_wallets.append(
                    f"{_sanitize_wallet_segment(wallet_prefix)}-{_sanitize_wallet_segment(core_wallet['id'])}"
                )
                _create_backend_and_wallet(
                    electrum_root,
                    backend_kind="electrum",
                    backend_url=electrum_url,
                    label="Electrum parity",
                    addresses=watched_addresses,
                )

                core_first = _sync(core_root, "Core parity")
                electrum_first = _sync(electrum_root, "Electrum parity")
                self.assertEqual(core_first["backend_kind"], "bitcoinrpc")
                self.assertEqual(electrum_first["backend_kind"], "electrum")
                self.assertEqual(core_first["imported"], 1)
                self.assertEqual(electrum_first["imported"], 1)
                _assert_books_match(self, core_root, electrum_root)

                spend_txid = _send_all_from_wallet(
                    core_url,
                    username,
                    password,
                    wallet=owner_wallet,
                    destination=external_address,
                )
                _rpc(core_url, username, password, "generatetoaddress", [1, mining_address])
                chain = _rpc(core_url, username, password, "getblockchaininfo")
                _wait_for_electrum(electrum_url, min_height=int(chain["blocks"]), txids=[spend_txid])
                core_second = _sync(core_root, "Core parity")
                electrum_second = _sync(electrum_root, "Electrum parity")
                self.assertEqual(core_second["imported"], 1)
                self.assertEqual(electrum_second["imported"], 1)
                _assert_books_match(self, core_root, electrum_root)

                later_txid = _rpc(
                    core_url,
                    username,
                    password,
                    "sendtoaddress",
                    [watched_addresses[0], 0.33333333],
                    wallet=faucet_wallet,
                )
                _rpc(core_url, username, password, "generatetoaddress", [1, mining_address])
                chain = _rpc(core_url, username, password, "getblockchaininfo")
                _wait_for_electrum(electrum_url, min_height=int(chain["blocks"]), txids=[later_txid])
                core_third = _sync(core_root, "Core parity")
                electrum_third = _sync(electrum_root, "Electrum parity")
                self.assertEqual(core_third["imported"], 1)
                self.assertEqual(electrum_third["imported"], 1)
                _assert_books_match(self, core_root, electrum_root)

                core_noop = _sync(core_root, "Core parity")
                electrum_noop = _sync(electrum_root, "Electrum parity")
                self.assertEqual(core_noop["imported"], 0)
                self.assertEqual(electrum_noop["imported"], 0)
                _assert_books_match(self, core_root, electrum_root)
        finally:
            for wallet_name in reversed(created_wallets):
                try:
                    _rpc(core_url, username, password, "unloadwallet", [wallet_name])
                except AssertionError as exc:
                    print(f"cleanup: could not unload {wallet_name}: {exc}", file=sys.stderr)


if __name__ == "__main__":
    unittest.main()

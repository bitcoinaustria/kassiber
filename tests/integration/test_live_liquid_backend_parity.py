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
from kassiber.msat import msat_to_btc

from tests.integration.env import skip_unless_integration


ROOT = Path(__file__).resolve().parents[2]
SAT = Decimal("0.00000001")


def _rpc(url: str, username: str, password: str, method: str, params=None, wallet=None):
    endpoint = url.rstrip("/")
    if wallet:
        endpoint = f"{endpoint}/wallet/{wallet}"
    payload = json.dumps(
        {
            "jsonrpc": "1.0",
            "id": "kassiber-live-liquid-parity",
            "method": method,
            "params": [] if params is None else params,
        }
    ).encode("utf-8")
    req = request.Request(endpoint, data=payload, headers={"Content-Type": "application/json"})
    token = base64.b64encode(f"{username}:{password}".encode("utf-8")).decode("ascii")
    req.add_header("Authorization", f"Basic {token}")
    with request.urlopen(req, timeout=120) as response:
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


def _elements_url() -> str:
    return os.environ.get("KASSIBER_REGTEST_ELEMENTS_URL") or (
        f"http://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_ELEMENTS_RPC_PORT', '18547')}"
    )


def _electrum_url() -> str:
    return (
        os.environ.get("KASSIBER_REGTEST_LIQUID_ELECTRUM_URL")
        or f"tcp://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT', '18545')}"
    )


def _mempool_url() -> str:
    return (
        os.environ.get("KASSIBER_REGTEST_LIQUID_MEMPOOL_URL")
        or f"http://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT', '18546')}/api"
    )


def _electrum_endpoint(url: str) -> tuple[str, int]:
    parsed = parse.urlsplit(url if "://" in url else f"tcp://{url}")
    if parsed.scheme != "tcp":
        raise AssertionError(f"Live Liquid parity test expects a tcp:// Electrum URL, got {url!r}")
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
                raise AssertionError("Liquid Electrum server closed the connection")
            raw += chunk
    response = json.loads(raw.decode("utf-8"))
    if response.get("error"):
        raise AssertionError(f"Liquid Electrum {method} failed: {response['error']}")
    return response.get("result")


def _http_get_text(url: str) -> str:
    with request.urlopen(url, timeout=10) as response:
        return response.read().decode("utf-8")


def _wait_for_liquid_indexes(
    *,
    electrum_url: str,
    mempool_url: str,
    min_height: int,
    txids: list[str],
) -> None:
    deadline = time.monotonic() + 180
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            _electrum_call(electrum_url, "server.version", ["Kassiber Liquid parity test", "1.4"])
            header = _electrum_call(electrum_url, "blockchain.headers.subscribe")
            electrum_height = int((header or {}).get("height") or 0)
            mempool_height = int(_http_get_text(f"{mempool_url.rstrip('/')}/blocks/tip/height").strip())
            if electrum_height >= min_height and mempool_height >= min_height:
                for txid in txids:
                    _electrum_call(electrum_url, "blockchain.transaction.get", [txid])
                    hex_payload = _http_get_text(f"{mempool_url.rstrip('/')}/tx/{txid}/hex").strip()
                    if not hex_payload:
                        raise AssertionError(f"Liquid mempool endpoint returned empty hex for {txid}")
                return
        except Exception as exc:
            last_error = exc
        time.sleep(2)
    raise AssertionError(
        f"Timed out waiting for Liquid indexes at {electrum_url} and {mempool_url}"
        + (f": {last_error}" if last_error else "")
    )


def _ensure_wallet(url: str, username: str, password: str, wallet_name: str) -> None:
    loaded = set(_rpc(url, username, password, "listwallets") or [])
    if wallet_name in loaded:
        return
    try:
        _rpc(url, username, password, "loadwallet", [wallet_name, True])
        return
    except AssertionError:
        pass
    try:
        _rpc(url, username, password, "createwallet", [wallet_name, False, False, "", False, True, True])
    except AssertionError:
        _rpc(url, username, password, "createwallet", [wallet_name])


def _unload_wallet(url: str, username: str, password: str, wallet_name: str) -> None:
    try:
        _rpc(url, username, password, "unloadwallet", [wallet_name])
    except AssertionError as exc:
        print(f"cleanup: could not unload {wallet_name}: {exc}", file=sys.stderr)


def _unconfidential_address(
    url: str,
    username: str,
    password: str,
    wallet_name: str,
    address: str,
) -> str:
    try:
        info = _rpc(url, username, password, "getaddressinfo", [address], wallet=wallet_name)
    except AssertionError:
        return address
    return str(info.get("unconfidential") or address)


def _descriptor_without_checksum(value: str) -> str:
    return str(value or "").split("#", 1)[0]


def _active_descriptor(descriptors: list[dict], *, internal: bool) -> str:
    candidates = [
        row
        for row in descriptors
        if bool(row.get("active")) and bool(row.get("internal")) is internal and row.get("desc")
    ]
    if not candidates:
        candidates = [
            row
            for row in descriptors
            if bool(row.get("internal")) is internal and row.get("desc")
        ]
    if not candidates:
        label = "change" if internal else "receive"
        raise AssertionError(f"Elements wallet did not expose an active {label} descriptor")
    return _descriptor_without_checksum(str(candidates[0]["desc"]))


def _blinded_liquid_descriptor(master_blinding_key: str, descriptor: str) -> str:
    descriptor = _descriptor_without_checksum(descriptor)
    if descriptor.startswith(("ct(", "blinded(")):
        return descriptor
    return f"ct(slip77({master_blinding_key}),{descriptor})"


def _write_liquid_descriptor_files(
    base_dir: Path,
    *,
    url: str,
    username: str,
    password: str,
    wallet_name: str,
) -> tuple[Path, Path]:
    descriptor_dir = base_dir / "descriptors"
    descriptor_dir.mkdir(parents=True, exist_ok=True)
    try:
        payload = _rpc(url, username, password, "listdescriptors", [False], wallet=wallet_name)
    except AssertionError:
        payload = _rpc(url, username, password, "listdescriptors", [], wallet=wallet_name)
    descriptors = payload.get("descriptors") if isinstance(payload, dict) else payload
    if not isinstance(descriptors, list):
        raise AssertionError(f"Elements wallet {wallet_name} returned no descriptors")
    master_blinding_key = str(_rpc(url, username, password, "dumpmasterblindingkey", [], wallet=wallet_name))
    receive = _blinded_liquid_descriptor(master_blinding_key, _active_descriptor(descriptors, internal=False))
    change = _blinded_liquid_descriptor(master_blinding_key, _active_descriptor(descriptors, internal=True))
    receive_path = descriptor_dir / "receive.txt"
    change_path = descriptor_dir / "change.txt"
    receive_path.write_text(receive + "\n", encoding="utf-8")
    change_path.write_text(change + "\n", encoding="utf-8")
    os.chmod(receive_path, 0o600)
    os.chmod(change_path, 0o600)
    return receive_path, change_path


def _elements_policy_asset_id(url: str, username: str, password: str) -> str:
    try:
        labels = _rpc(url, username, password, "dumpassetlabels")
    except AssertionError:
        return ""
    if isinstance(labels, dict):
        return str(labels.get("bitcoin") or labels.get("LBTC") or labels.get("lbtc") or "")
    return ""


def _create_backend_and_wallet(
    data_root: Path,
    *,
    backend_kind: str,
    backend_url: str,
    label: str,
    receive_descriptor_file: Path,
    change_descriptor_file: Path,
    policy_asset_id: str,
) -> None:
    _run(data_root, "init")
    _run(data_root, "workspaces", "create", "Liquid Parity")
    _run(
        data_root,
        "profiles",
        "create",
        "Default",
        "--workspace",
        "Liquid Parity",
        "--fiat-currency",
        "EUR",
        "--tax-country",
        "generic",
        "--gains-algorithm",
        "FIFO",
    )
    backend_name = f"{backend_kind}-regtest"
    _run(
        data_root,
        "backends",
        "create",
        backend_name,
        "--kind",
        backend_kind,
        "--url",
        backend_url,
        "--chain",
        "liquid",
        "--network",
        "elementsregtest",
        "--timeout",
        "30",
        "--batch-size",
        "25",
    )
    wallet_args = [
        "wallets",
        "create",
        "--workspace",
        "Liquid Parity",
        "--profile",
        "Default",
        "--label",
        label,
        "--kind",
        "descriptor",
        "--backend",
        backend_name,
        "--chain",
        "liquid",
        "--network",
        "elementsregtest",
        "--descriptor-file",
        str(receive_descriptor_file),
        "--change-descriptor-file",
        str(change_descriptor_file),
        "--gap-limit",
        "20",
    ]
    if policy_asset_id:
        wallet_args.extend(["--policy-asset", policy_asset_id])
    _run(data_root, *wallet_args)


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
                "amount": str(msat_to_btc(row["amount"]).quantize(SAT)),
                "fee": str(msat_to_btc(row["fee"]).quantize(SAT)),
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


def _assert_books_match(testcase: unittest.TestCase, electrum_root: Path, esplora_root: Path) -> None:
    testcase.assertEqual(_transaction_projection(electrum_root), _transaction_projection(esplora_root))
    testcase.assertEqual(_utxo_projection(electrum_root), _utxo_projection(esplora_root))


def _row_by_txid(data_root: Path) -> dict[str, dict]:
    return {row["external_id"]: row for row in _transaction_projection(data_root)}


def _assert_liquid_shape(
    testcase: unittest.TestCase,
    data_root: Path,
    txids: dict[str, str],
    *,
    pending_confirmed: bool,
) -> None:
    rows = _row_by_txid(data_root)
    expected = {
        "receive_a": ("inbound", "0.01000000", "0.00000000", "deposit"),
        "receive_b": ("inbound", "0.00400000", "0.00000000", "deposit"),
        "spend": ("outbound", "0.00250000", None, "withdrawal"),
        "self_fee": ("outbound", "0.00000000", None, "fee"),
        "pending": ("inbound", "0.00077777", "0.00000000", "deposit"),
    }
    for key, (direction, amount, fee, kind) in expected.items():
        row = rows.get(txids[key])
        testcase.assertIsNotNone(row, f"missing Liquid row for {key}={txids[key]}")
        if row is None:
            continue
        testcase.assertEqual(row["direction"], direction)
        testcase.assertEqual(row["asset"], "LBTC")
        testcase.assertEqual(row["amount"], amount)
        testcase.assertEqual(row["kind"], kind)
        if fee is None:
            testcase.assertGreater(Decimal(row["fee"]), Decimal("0"), row)
        else:
            testcase.assertEqual(row["fee"], fee)
        if key == "pending" and not pending_confirmed:
            testcase.assertFalse(row["confirmed_at"], row)
        else:
            testcase.assertTrue(row["confirmed_at"], row)


def _assert_elements_wallet_txids(
    testcase: unittest.TestCase,
    *,
    url: str,
    username: str,
    password: str,
    wallet_name: str,
    txids: dict[str, str],
    pending_confirmed: bool,
) -> None:
    for key, txid in txids.items():
        tx = _rpc(url, username, password, "gettransaction", [txid], wallet=wallet_name)
        testcase.assertEqual(tx.get("txid"), txid)
        testcase.assertTrue(tx.get("hex"), f"Elements wallet did not return raw tx hex for {key}={txid}")
        confirmations = int(tx.get("confirmations") or 0)
        if key == "pending" and not pending_confirmed:
            testcase.assertEqual(confirmations, 0, tx)
        else:
            testcase.assertGreaterEqual(confirmations, 1, tx)


@skip_unless_integration
class LiveLiquidBackendParityTest(unittest.TestCase):
    def test_liquid_electrum_and_esplora_match_for_descriptor_wallet(self):
        elements_url = _elements_url()
        electrum_url = _electrum_url()
        mempool_url = _mempool_url()
        username = os.environ.get("KASSIBER_REGTEST_RPC_USER", "kassiber")
        password = os.environ.get("KASSIBER_REGTEST_RPC_PASSWORD", "kassiber")

        chain = _rpc(elements_url, username, password, "getblockchaininfo")
        self.assertEqual(chain["chain"], "elementsregtest")

        run_id = uuid.uuid4().hex[:12]
        faucet_wallet = f"kassiber-liquid-faucet-{run_id}"
        owner_wallet = f"kassiber-liquid-owner-{run_id}"
        created_wallets = [faucet_wallet, owner_wallet]

        try:
            _ensure_wallet(elements_url, username, password, faucet_wallet)
            _ensure_wallet(elements_url, username, password, owner_wallet)
            mining_confidential = _rpc(
                elements_url,
                username,
                password,
                "getnewaddress",
                ["liquid mining"],
                wallet=faucet_wallet,
            )
            mining_address = _unconfidential_address(
                elements_url,
                username,
                password,
                faucet_wallet,
                mining_confidential,
            )
            watched_addresses = [
                _rpc(
                    elements_url,
                    username,
                    password,
                    "getnewaddress",
                    [f"watched {index}"],
                    wallet=owner_wallet,
                )
                for index in range(3)
            ]
            external_address = _rpc(
                elements_url,
                username,
                password,
                "getnewaddress",
                ["external"],
                wallet=faucet_wallet,
            )

            _rpc(elements_url, username, password, "generatetoaddress", [101, mining_address])
            txids = {
                "receive_a": _rpc(
                    elements_url,
                    username,
                    password,
                    "sendtoaddress",
                    [watched_addresses[0], 0.01000000],
                    wallet=faucet_wallet,
                )
            }
            _rpc(elements_url, username, password, "generatetoaddress", [1, mining_address])
            txids["receive_b"] = _rpc(
                elements_url,
                username,
                password,
                "sendtoaddress",
                [watched_addresses[1], 0.00400000],
                wallet=faucet_wallet,
            )
            _rpc(elements_url, username, password, "generatetoaddress", [1, mining_address])
            txids["spend"] = _rpc(
                elements_url,
                username,
                password,
                "sendtoaddress",
                [external_address, 0.00250000],
                wallet=owner_wallet,
            )
            _rpc(elements_url, username, password, "generatetoaddress", [1, mining_address])
            txids["self_fee"] = _rpc(
                elements_url,
                username,
                password,
                "sendtoaddress",
                [watched_addresses[2], 0.00100000],
                wallet=owner_wallet,
            )
            _rpc(elements_url, username, password, "generatetoaddress", [1, mining_address])
            chain = _rpc(elements_url, username, password, "getblockchaininfo")
            _wait_for_liquid_indexes(
                electrum_url=electrum_url,
                mempool_url=mempool_url,
                min_height=int(chain["blocks"]),
                txids=list(txids.values()),
            )
            _assert_elements_wallet_txids(
                self,
                url=elements_url,
                username=username,
                password=password,
                wallet_name=owner_wallet,
                txids=txids,
                pending_confirmed=True,
            )

            with tempfile.TemporaryDirectory(prefix="kassiber-liquid-parity-") as tmp:
                base = Path(tmp).resolve()
                receive_file, change_file = _write_liquid_descriptor_files(
                    base,
                    url=elements_url,
                    username=username,
                    password=password,
                    wallet_name=owner_wallet,
                )
                policy_asset_id = _elements_policy_asset_id(elements_url, username, password)
                electrum_root = base / "electrum"
                esplora_root = base / "esplora"
                _create_backend_and_wallet(
                    electrum_root,
                    backend_kind="electrum",
                    backend_url=electrum_url,
                    label="Liquid Electrum parity",
                    receive_descriptor_file=receive_file,
                    change_descriptor_file=change_file,
                    policy_asset_id=policy_asset_id,
                )
                _create_backend_and_wallet(
                    esplora_root,
                    backend_kind="liquid-esplora",
                    backend_url=mempool_url,
                    label="Liquid Esplora parity",
                    receive_descriptor_file=receive_file,
                    change_descriptor_file=change_file,
                    policy_asset_id=policy_asset_id,
                )

                electrum_first = _sync(electrum_root, "Liquid Electrum parity")
                esplora_first = _sync(esplora_root, "Liquid Esplora parity")
                self.assertEqual(electrum_first["backend_kind"], "electrum")
                self.assertEqual(esplora_first["backend_kind"], "esplora")
                self.assertEqual(electrum_first["imported"], 4)
                self.assertEqual(esplora_first["imported"], 4)
                _assert_books_match(self, electrum_root, esplora_root)

                txids["pending"] = _rpc(
                    elements_url,
                    username,
                    password,
                    "sendtoaddress",
                    [watched_addresses[0], 0.00077777],
                    wallet=faucet_wallet,
                )
                chain = _rpc(elements_url, username, password, "getblockchaininfo")
                _wait_for_liquid_indexes(
                    electrum_url=electrum_url,
                    mempool_url=mempool_url,
                    min_height=int(chain["blocks"]),
                    txids=[txids["pending"]],
                )
                _assert_elements_wallet_txids(
                    self,
                    url=elements_url,
                    username=username,
                    password=password,
                    wallet_name=owner_wallet,
                    txids=txids,
                    pending_confirmed=False,
                )
                electrum_pending = _sync(electrum_root, "Liquid Electrum parity")
                esplora_pending = _sync(esplora_root, "Liquid Esplora parity")
                self.assertEqual(electrum_pending["imported"], 1)
                self.assertEqual(esplora_pending["imported"], 1)
                _assert_books_match(self, electrum_root, esplora_root)
                _assert_liquid_shape(self, electrum_root, txids, pending_confirmed=False)

                _rpc(elements_url, username, password, "generatetoaddress", [1, mining_address])
                chain = _rpc(elements_url, username, password, "getblockchaininfo")
                _wait_for_liquid_indexes(
                    electrum_url=electrum_url,
                    mempool_url=mempool_url,
                    min_height=int(chain["blocks"]),
                    txids=[txids["pending"]],
                )
                _assert_elements_wallet_txids(
                    self,
                    url=elements_url,
                    username=username,
                    password=password,
                    wallet_name=owner_wallet,
                    txids=txids,
                    pending_confirmed=True,
                )
                _sync(electrum_root, "Liquid Electrum parity")
                _sync(esplora_root, "Liquid Esplora parity")
                _assert_books_match(self, electrum_root, esplora_root)
                _assert_liquid_shape(self, electrum_root, txids, pending_confirmed=True)

                electrum_noop = _sync(electrum_root, "Liquid Electrum parity")
                esplora_noop = _sync(esplora_root, "Liquid Esplora parity")
                self.assertEqual(electrum_noop["imported"], 0)
                self.assertEqual(esplora_noop["imported"], 0)
                _assert_books_match(self, electrum_root, esplora_root)
        finally:
            for wallet_name in reversed(created_wallets):
                _unload_wallet(elements_url, username, password, wallet_name)


if __name__ == "__main__":
    unittest.main()

"""Opt-in live wallet-sync tests.

These tests exercise real wallet/backend flows and are skipped by default.
Set ``KASSIBER_LIVE_SYNC_TESTS=1`` to run them. The default path uses only
local Docker/localhost services and generated regtest wallets.
"""

from __future__ import annotations

import base64
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_BITCOIND_IMAGE = "bitcoin/bitcoin:28.1"


def _truthy(value):
    return str(value or "").strip().lower() in {"1", "true", "yes", "on"}


LIVE_SYNC_TESTS = _truthy(os.environ.get("KASSIBER_LIVE_SYNC_TESTS"))
REQUIRE_BITCOIN_REGTEST = _truthy(os.environ.get("KASSIBER_REQUIRE_BITCOIN_REGTEST"))
REQUIRE_LIQUID_LIVE = _truthy(os.environ.get("KASSIBER_REQUIRE_LIQUID_LIVE"))


class BitcoinRpcError(AssertionError):
    pass


class BitcoinRpc:
    def __init__(self, url, username, password):
        self.url = url.rstrip("/")
        self.username = username
        self.password = password

    def _url(self, wallet=None):
        if not wallet:
            return self.url
        return f"{self.url}/wallet/{urlparse.quote(wallet, safe='')}"

    def call(self, method, params=None, wallet=None):
        token = f"{self.username}:{self.password}".encode("utf-8")
        payload = json.dumps(
            {
                "jsonrpc": "1.0",
                "id": f"kassiber-live-{method}",
                "method": method,
                "params": [] if params is None else params,
            }
        ).encode("utf-8")
        req = urlrequest.Request(
            self._url(wallet),
            data=payload,
            headers={
                "Authorization": f"Basic {base64.b64encode(token).decode('ascii')}",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            with urlrequest.urlopen(req, timeout=10) as response:
                message = json.loads(response.read().decode("utf-8"))
        except (urlerror.URLError, TimeoutError) as exc:
            raise BitcoinRpcError(str(exc)) from exc
        if message.get("error"):
            error = message["error"]
            raise BitcoinRpcError(
                f"{method} failed ({error.get('code', 'unknown')}): {error.get('message', error)}"
            )
        return message.get("result")


def _run_json(data_root, *args):
    cmd = [
        sys.executable,
        "-m",
        "kassiber",
        "--data-root",
        str(data_root),
        "--machine",
        *args,
    ]
    result = subprocess.run(
        cmd,
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    stdout = result.stdout.strip()
    if not stdout:
        raise AssertionError(
            f"CLI produced no stdout for {args!r}; returncode={result.returncode}; stderr={result.stderr!r}"
        )
    try:
        payload = json.loads(stdout)
    except json.JSONDecodeError as exc:
        raise AssertionError(f"CLI stdout was not JSON for {args!r}: {stdout[:500]}") from exc
    return payload, result


def _assert_ok(testcase, payload, result, kind):
    testcase.assertEqual(result.returncode, 0, msg=f"payload={payload!r}; stderr={result.stderr!r}")
    testcase.assertEqual(payload.get("schema_version"), 1)
    testcase.assertEqual(payload.get("kind"), kind)


def _docker(*args, check=True):
    result = subprocess.run(
        ["docker", *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if check and result.returncode != 0:
        raise AssertionError(
            f"docker {' '.join(args)} failed with {result.returncode}\n"
            f"stdout={result.stdout!r}\nstderr={result.stderr!r}"
        )
    return result


def _docker_daemon_available():
    if not shutil.which("docker"):
        return False, "docker CLI is not installed"
    result = _docker("info", check=False)
    if result.returncode != 0:
        return False, result.stderr.strip() or result.stdout.strip() or "docker daemon is not reachable"
    return True, ""


def _docker_image_available(image):
    result = _docker("image", "inspect", image, check=False)
    return result.returncode == 0


def _mapped_port(container_name, container_port):
    deadline = time.time() + 20
    last_output = ""
    while time.time() < deadline:
        result = _docker("port", container_name, f"{container_port}/tcp", check=False)
        last_output = (result.stdout or result.stderr).strip()
        if result.returncode == 0 and result.stdout.strip():
            endpoint = result.stdout.strip().splitlines()[0]
            return int(endpoint.rsplit(":", 1)[1])
        time.sleep(0.2)
    raise AssertionError(f"Docker did not publish {container_port}/tcp for {container_name}: {last_output}")


def _wait_for_bitcoin_rpc(rpc):
    deadline = time.time() + 60
    last_error = None
    while time.time() < deadline:
        try:
            rpc.call("getblockchaininfo")
            return
        except BitcoinRpcError as exc:
            last_error = exc
            time.sleep(0.5)
    raise AssertionError(f"Bitcoin Core RPC did not become ready: {last_error}") from last_error


def _url_is_loopback(value):
    parsed = urlparse.urlsplit(value if "://" in value else f"tcp://{value}")
    host = (parsed.hostname or "").lower()
    return host in {"localhost", "127.0.0.1", "::1"}


@unittest.skipUnless(
    LIVE_SYNC_TESTS,
    "set KASSIBER_LIVE_SYNC_TESTS=1 to run local live sync integration tests",
)
class BitcoinCoreRegtestLiveSyncTest(unittest.TestCase):
    def setUp(self):
        available, reason = _docker_daemon_available()
        if not available:
            if REQUIRE_BITCOIN_REGTEST:
                self.fail(reason)
            self.skipTest(reason)

        self.image = os.environ.get("KASSIBER_BITCOIND_IMAGE", DEFAULT_BITCOIND_IMAGE)
        if not _docker_image_available(self.image) and not _truthy(os.environ.get("KASSIBER_LIVE_SYNC_PULL")):
            message = (
                f"Docker image {self.image!r} is not present locally; "
                "pre-pull it or set KASSIBER_LIVE_SYNC_PULL=1 to allow Docker to pull"
            )
            if REQUIRE_BITCOIN_REGTEST:
                self.fail(message)
            self.skipTest(message)

        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-live-sync-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"
        self.container_name = f"kassiber-bitcoin-regtest-{os.getpid()}-{int(time.time() * 1000)}"
        self.addCleanup(self._cleanup_container)

        _docker(
            "run",
            "--rm",
            "-d",
            "--name",
            self.container_name,
            "-p",
            "127.0.0.1::18443",
            self.image,
            "-regtest=1",
            "-server=1",
            "-txindex=1",
            "-fallbackfee=0.0001",
            "-rpcuser=kassiber",
            "-rpcpassword=kassiber",
            "-rpcbind=0.0.0.0",
            "-rpcallowip=0.0.0.0/0",
            "-printtoconsole=1",
        )
        rpc_port = _mapped_port(self.container_name, 18443)
        self.rpc = BitcoinRpc(f"http://127.0.0.1:{rpc_port}", "kassiber", "kassiber")
        _wait_for_bitcoin_rpc(self.rpc)

    def _cleanup_container(self):
        if getattr(self, "container_name", None):
            _docker("rm", "-f", self.container_name, check=False)

    def _fund_real_wallet_address(self):
        wallet_name = "miner"
        self.rpc.call("createwallet", [wallet_name])
        mining_address = self.rpc.call("getnewaddress", ["mining", "bech32"], wallet=wallet_name)
        self.rpc.call("generatetoaddress", [101, mining_address])
        watched_address = self.rpc.call("getnewaddress", ["kassiber-watch", "bech32"], wallet=wallet_name)
        txid = self.rpc.call("sendtoaddress", [watched_address, 0.25], wallet=wallet_name)
        self.rpc.call("generatetoaddress", [1, mining_address])
        return watched_address, txid

    def test_bitcoin_core_regtest_address_wallet_syncs_real_receive(self):
        watched_address, txid = self._fund_real_wallet_address()

        payload, result = _run_json(self.data_root, "init")
        _assert_ok(self, payload, result, "init")
        payload, result = _run_json(self.data_root, "workspaces", "create", "Main")
        _assert_ok(self, payload, result, "workspaces.create")
        payload, result = _run_json(
            self.data_root,
            "profiles",
            "create",
            "--workspace",
            "Main",
            "Default",
        )
        _assert_ok(self, payload, result, "profiles.create")

        payload, result = _run_json(
            self.data_root,
            "backends",
            "create",
            "regtest-core",
            "--kind",
            "bitcoinrpc",
            "--url",
            self.rpc.url,
            "--chain",
            "bitcoin",
            "--network",
            "regtest",
            "--username",
            "kassiber",
            "--password",
            "kassiber",
            "--wallet-prefix",
            "live-regtest",
        )
        _assert_ok(self, payload, result, "backends.create")

        payload, result = _run_json(
            self.data_root,
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "OnchainRegtest",
            "--kind",
            "address",
            "--backend",
            "regtest-core",
            "--chain",
            "bitcoin",
            "--network",
            "regtest",
            "--address",
            watched_address,
        )
        _assert_ok(self, payload, result, "wallets.create")

        payload, result = _run_json(
            self.data_root,
            "wallets",
            "sync",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "OnchainRegtest",
        )
        _assert_ok(self, payload, result, "wallets.sync")
        self.assertEqual(len(payload["data"]), 1)
        first_sync = payload["data"][0]
        self.assertEqual(first_sync["status"], "synced")
        self.assertEqual(first_sync["backend_kind"], "bitcoinrpc")
        self.assertEqual(first_sync["chain"], "bitcoin")
        self.assertEqual(first_sync["network"], "regtest")
        self.assertEqual(first_sync["sync_mode"], "addresses")
        self.assertEqual(first_sync["imported"], 1)
        self.assertEqual(first_sync["skipped"], 0)
        self.assertEqual(first_sync["imported_addresses"], 1)

        payload, result = _run_json(
            self.data_root,
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "OnchainRegtest",
        )
        _assert_ok(self, payload, result, "transactions.list")
        self.assertEqual(len(payload["data"]), 1)
        tx = payload["data"][0]
        self.assertEqual(tx["external_id"], txid)
        self.assertEqual(tx["direction"], "inbound")
        self.assertEqual(tx["asset"], "BTC")
        self.assertAlmostEqual(tx["amount"], 0.25, places=8)
        self.assertEqual(tx["fee"], 0.0)
        self.assertTrue(tx["confirmed_at"])

        payload, result = _run_json(
            self.data_root,
            "wallets",
            "sync",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "OnchainRegtest",
        )
        _assert_ok(self, payload, result, "wallets.sync")
        second_sync = payload["data"][0]
        self.assertEqual(second_sync["imported"], 0)
        self.assertEqual(second_sync["skipped"], 1)
        self.assertEqual(second_sync["imported_addresses"], 0)


@unittest.skipUnless(
    LIVE_SYNC_TESTS,
    "set KASSIBER_LIVE_SYNC_TESTS=1 to run local live sync integration tests",
)
class LiquidLocalBackendLiveSyncTest(unittest.TestCase):
    """Exercise a developer-provided local Liquid Esplora/Electrum backend.

    Kassiber's Liquid sync path currently requires Esplora/Electrum, so this
    test is parameterized instead of trying to own a full Liquid indexer stack.
    """

    def setUp(self):
        self.backend_url = os.environ.get("KASSIBER_LIVE_LIQUID_BACKEND_URL")
        self.descriptor_file = os.environ.get("KASSIBER_LIVE_LIQUID_DESCRIPTOR_FILE")
        self.change_descriptor_file = os.environ.get("KASSIBER_LIVE_LIQUID_CHANGE_DESCRIPTOR_FILE")
        missing = [
            name
            for name, value in (
                ("KASSIBER_LIVE_LIQUID_BACKEND_URL", self.backend_url),
                ("KASSIBER_LIVE_LIQUID_DESCRIPTOR_FILE", self.descriptor_file),
                ("KASSIBER_LIVE_LIQUID_CHANGE_DESCRIPTOR_FILE", self.change_descriptor_file),
            )
            if not value
        ]
        if missing:
            if REQUIRE_LIQUID_LIVE:
                self.fail(f"local Liquid backend test needs {', '.join(missing)}")
            self.skipTest(f"local Liquid backend test needs {', '.join(missing)}")
        if not _url_is_loopback(self.backend_url):
            if REQUIRE_LIQUID_LIVE:
                self.fail("local Liquid live sync tests only allow loopback backend URLs")
            self.skipTest("local Liquid live sync tests only allow loopback backend URLs")
        for path in (self.descriptor_file, self.change_descriptor_file):
            if not Path(path).expanduser().is_file():
                if REQUIRE_LIQUID_LIVE:
                    self.fail(f"Liquid descriptor file does not exist: {path}")
                self.skipTest(f"Liquid descriptor file does not exist: {path}")

        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-live-liquid-")
        self.addCleanup(self.tmp.cleanup)
        self.data_root = Path(self.tmp.name) / "data"

    def test_local_liquid_descriptor_wallet_syncs_records(self):
        backend_kind = os.environ.get("KASSIBER_LIVE_LIQUID_BACKEND_KIND", "esplora").strip().lower()
        network = os.environ.get("KASSIBER_LIVE_LIQUID_NETWORK", "elementsregtest").strip()
        expect_records = not _truthy(os.environ.get("KASSIBER_LIVE_LIQUID_ALLOW_EMPTY"))

        payload, result = _run_json(self.data_root, "init")
        _assert_ok(self, payload, result, "init")
        payload, result = _run_json(self.data_root, "workspaces", "create", "Main")
        _assert_ok(self, payload, result, "workspaces.create")
        payload, result = _run_json(
            self.data_root,
            "profiles",
            "create",
            "--workspace",
            "Main",
            "Default",
        )
        _assert_ok(self, payload, result, "profiles.create")

        backend_args = [
            "backends",
            "create",
            "liquid-local",
            "--kind",
            backend_kind,
            "--url",
            self.backend_url,
            "--chain",
            "liquid",
            "--network",
            network,
            "--timeout",
            os.environ.get("KASSIBER_LIVE_LIQUID_TIMEOUT", "30"),
        ]
        if backend_kind == "electrum":
            backend_args.extend(["--batch-size", os.environ.get("KASSIBER_LIVE_LIQUID_BATCH_SIZE", "10")])
            if _truthy(os.environ.get("KASSIBER_LIVE_LIQUID_INSECURE")):
                backend_args.extend(["--insecure", "1"])
        payload, result = _run_json(self.data_root, *backend_args)
        _assert_ok(self, payload, result, "backends.create")

        payload, result = _run_json(
            self.data_root,
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "LiquidLocal",
            "--kind",
            "descriptor",
            "--backend",
            "liquid-local",
            "--chain",
            "liquid",
            "--network",
            network,
            "--descriptor-file",
            self.descriptor_file,
            "--change-descriptor-file",
            self.change_descriptor_file,
            "--gap-limit",
            os.environ.get("KASSIBER_LIVE_LIQUID_GAP_LIMIT", "5"),
        )
        _assert_ok(self, payload, result, "wallets.create")

        payload, result = _run_json(
            self.data_root,
            "wallets",
            "sync",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "LiquidLocal",
        )
        _assert_ok(self, payload, result, "wallets.sync")
        self.assertEqual(len(payload["data"]), 1)
        sync = payload["data"][0]
        self.assertEqual(sync["status"], "synced")
        self.assertEqual(sync["chain"], "liquid")
        self.assertEqual(sync["network"], network)
        self.assertEqual(sync["sync_mode"], "descriptor")
        self.assertGreater(sync["target_count"], 0)
        if expect_records:
            self.assertGreater(sync["imported"] + sync["skipped"], 0)


if __name__ == "__main__":
    unittest.main()

import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from kassiber import daemon
from kassiber.errors import AppError


class DaemonBitcoinRpcProbeTest(unittest.TestCase):
    def test_bitcoinrpc_probe_success(self):
        ctx = SimpleNamespace(runtime_config={})
        calls = []

        def fake_call(backend, method, params=None, wallet_name=None, timeout=None):
            del wallet_name, timeout
            calls.append((backend, method, params))
            if method == "getblockchaininfo":
                return {
                    "chain": "main",
                    "blocks": 850_000,
                    "headers": 850_000,
                    "pruned": False,
                    "initialblockdownload": False,
                }
            if method == "getnetworkinfo":
                return {"version": 270000}
            raise AssertionError(f"Unexpected RPC call: {method}")

        with patch("kassiber.daemon.bitcoinrpc_call", side_effect=fake_call):
            payload = daemon._test_bitcoinrpc_backend_payload(
                ctx,
                {
                    "url": "http://127.0.0.1:8332",
                    "config": {"username": "rpcuser", "password": "rpcpass"},
                },
            )

        self.assertTrue(payload["reachable"])
        self.assertEqual(payload["chain"], "main")
        self.assertEqual(payload["blocks"], 850_000)
        self.assertEqual(payload["version"], 270000)
        self.assertEqual(calls[0][0]["username"], "rpcuser")

    def test_bitcoinrpc_probe_unreachable_returns_payload(self):
        ctx = SimpleNamespace(runtime_config={})

        with patch(
            "kassiber.daemon.bitcoinrpc_call",
            side_effect=AppError("connection refused", code="bitcoinrpc_unreachable", retryable=True),
        ):
            payload = daemon._test_bitcoinrpc_backend_payload(
                ctx,
                {
                    "url": "http://127.0.0.1:8332",
                    "config": {"username": "rpcuser", "password": "rpcpass"},
                },
            )

        self.assertFalse(payload["reachable"])
        self.assertEqual(payload["error"]["code"], "bitcoinrpc_unreachable")
        self.assertTrue(payload["error"]["retryable"])

    def test_bitcoinrpc_probe_rejects_pruned_below_birthday(self):
        ctx = SimpleNamespace(runtime_config={})

        def fake_call(backend, method, params=None, wallet_name=None, timeout=None):
            del backend, wallet_name, timeout
            if method == "getblockchaininfo":
                return {
                    "chain": "main",
                    "blocks": 20,
                    "headers": 20,
                    "pruned": True,
                    "pruneheight": 8,
                    "initialblockdownload": False,
                }
            if method == "getnetworkinfo":
                return {"version": 270000}
            if method == "getblockhash":
                return f"block-{params[0]}"
            if method == "getblockheader":
                return {"time": int(str(params[0]).split("-")[-1]) * 10}
            raise AssertionError(f"Unexpected RPC call: {method}")

        with patch("kassiber.daemon.bitcoinrpc_call", side_effect=fake_call):
            with self.assertRaises(AppError) as raised:
                daemon._test_bitcoinrpc_backend_payload(
                    ctx,
                    {
                        "url": "http://127.0.0.1:8332",
                        "config": {"username": "rpcuser", "password": "rpcpass"},
                        "birthday": "1970-01-01T00:00:50Z",
                    },
                )

        self.assertEqual(raised.exception.code, "bitcoinrpc_pruned_below_birthday")
        self.assertEqual(raised.exception.details["birthday_height"], 5)
        self.assertEqual(raised.exception.details["pruneheight"], 8)

    def test_detect_core_returns_cookiefile_candidate_without_cookie_contents(self):
        with tempfile.TemporaryDirectory() as tmp:
            bitcoin_dir = Path(tmp) / ".bitcoin"
            bitcoin_dir.mkdir()
            (bitcoin_dir / ".cookie").write_text("__cookie__:secret", encoding="utf-8")

            with patch.object(daemon.Path, "home", return_value=Path(tmp)), patch(
                "kassiber.daemon._bitcoinrpc_probe_payload",
                return_value={
                    "reachable": True,
                    "chain": "main",
                    "network": "main",
                    "blocks": 850_000,
                    "headers": 850_000,
                    "pruned": False,
                    "ibd": False,
                },
            ) as probe:
                payload = daemon._detect_core_payload({})

        self.assertEqual(len(payload["candidates"]), 1)
        candidate = payload["candidates"][0]
        self.assertEqual(candidate["url"], "http://127.0.0.1:8332")
        self.assertEqual(candidate["auth_source"], "cookiefile")
        self.assertTrue(candidate["cookiefile"].endswith(".bitcoin/.cookie"))
        self.assertNotIn("secret", str(candidate))
        probe.assert_called_once()

    def test_detect_core_returns_empty_when_no_default_cookie_exists(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(
            daemon.Path,
            "home",
            return_value=Path(tmp),
        ), patch("kassiber.daemon._bitcoinrpc_probe_payload") as probe:
            payload = daemon._detect_core_payload({})

        self.assertEqual(payload, {"candidates": []})
        probe.assert_not_called()


if __name__ == "__main__":
    unittest.main()

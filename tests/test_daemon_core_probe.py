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
                return {"version": 270000, "connections": 8}
            if method == "listwallets":
                return ["kassiber-wallet-1"]
            if method == "getbestblockhash":
                return "best-block"
            if method == "getblockfilter":
                self.assertEqual(params, ["best-block"])
                return {"filter": "00", "header": "11"}
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
        self.assertEqual(payload["status"], "synchronized")
        self.assertEqual(payload["peers"], 8)
        self.assertTrue(payload["wallet_rpc"]["available"])
        self.assertTrue(payload["block_filters"]["available"])
        self.assertEqual(payload["warnings"], [])
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
        self.assertEqual(payload["status"], "unresponsive")
        self.assertEqual(payload["warnings"], ["unresponsive"])

    def test_bitcoinrpc_probe_rejects_unsafe_inline_cookiefile(self):
        ctx = SimpleNamespace(runtime_config={})

        with patch("kassiber.daemon.bitcoinrpc_call") as call:
            with self.assertRaises(AppError) as raised:
                daemon._test_bitcoinrpc_backend_payload(
                    ctx,
                    {
                        "url": "https://attacker.example",
                        "config": {"cookiefile": "/etc/passwd"},
                    },
                )

        self.assertEqual(raised.exception.code, "validation")
        self.assertIn("loopback", str(raised.exception))
        call.assert_not_called()

    def test_bitcoinrpc_probe_rejects_unsafe_saved_cookiefile_backend(self):
        ctx = SimpleNamespace(
            runtime_config={
                "env_file": "/tmp/backends.env",
                "default_backend": "core",
                "backends": {
                    "core": {
                        "name": "core",
                        "kind": "bitcoinrpc",
                        "url": "http://192.168.1.10:8332",
                        "cookiefile": "/home/tg/.bitcoin/.cookie",
                    }
                },
            }
        )

        with patch("kassiber.daemon.bitcoinrpc_call") as call:
            with self.assertRaises(AppError) as raised:
                daemon._test_bitcoinrpc_backend_payload(ctx, {"backend": "core"})

        self.assertEqual(raised.exception.code, "validation")
        self.assertIn("loopback", str(raised.exception))
        call.assert_not_called()

    def test_bitcoinrpc_probe_rejects_unsafe_detected_credential_ref(self):
        ctx = SimpleNamespace(runtime_config={})
        with patch(
            "kassiber.daemon._core_backend_from_credential_ref",
            return_value={
                "name": "local-core-main",
                "kind": "bitcoinrpc",
                "network": "main",
                "url": "http://192.168.1.10:8332",
                "cookiefile": "/home/tg/.bitcoin/.cookie",
            },
        ), patch("kassiber.daemon.bitcoinrpc_call") as call:
            with self.assertRaises(AppError) as raised:
                daemon._test_bitcoinrpc_backend_payload(
                    ctx,
                    {"credential_ref": "local-core:test"},
                )

        self.assertEqual(raised.exception.code, "validation")
        self.assertIn("loopback", str(raised.exception))
        call.assert_not_called()

    def test_desktop_cookiefile_validation_allows_only_local_default_cookie(self):
        with tempfile.TemporaryDirectory() as tmp:
            bitcoin_dir = Path(tmp) / ".bitcoin"
            bitcoin_dir.mkdir()
            cookiefile = bitcoin_dir / ".cookie"
            cookiefile.write_text("__cookie__:secret", encoding="utf-8")

            with patch.object(daemon.Path, "home", return_value=Path(tmp)):
                daemon._validate_desktop_bitcoinrpc_cookiefile(
                    "bitcoinrpc",
                    "http://127.0.0.1:8332",
                    {"cookiefile": str(cookiefile)},
                )
                with self.assertRaises(AppError) as raised:
                    daemon._validate_desktop_bitcoinrpc_cookiefile(
                        "bitcoinrpc",
                        "https://attacker.example",
                        {"cookiefile": str(cookiefile)},
                    )

        self.assertEqual(raised.exception.code, "validation")
        self.assertIn("loopback", str(raised.exception))

    def test_detect_core_skips_non_loopback_bitcoin_conf_rpcconnect(self):
        with tempfile.TemporaryDirectory() as tmp:
            bitcoin_dir = Path(tmp) / ".bitcoin"
            bitcoin_dir.mkdir()
            (bitcoin_dir / "bitcoin.conf").write_text(
                "\n".join(
                    [
                        "rpcconnect=192.168.1.10",
                        "rpcuser=alice",
                        "rpcpassword=correct horse battery staple",
                    ]
                ),
                encoding="utf-8",
            )

            with patch.object(daemon.Path, "home", return_value=Path(tmp)), patch(
                "kassiber.daemon._bitcoinrpc_probe_payload",
            ) as probe:
                payload = daemon._detect_core_payload({})

        self.assertEqual(payload, {"candidates": []})
        probe.assert_not_called()

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
                return {"version": 270000, "connections": 8}
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
                    "peers": 8,
                    "status": "synchronized",
                    "pruned": False,
                    "ibd": False,
                    "wallet_rpc": {"available": True, "loaded_wallet_count": 0},
                    "block_filters": {"available": True},
                    "warnings": [],
                },
            ) as probe:
                payload = daemon._detect_core_payload({})

        self.assertEqual(len(payload["candidates"]), 1)
        candidate = payload["candidates"][0]
        self.assertEqual(candidate["url"], "http://127.0.0.1:8332")
        self.assertEqual(candidate["auth_source"], "cookiefile")
        self.assertTrue(candidate["cookiefile"].endswith(".bitcoin/.cookie"))
        self.assertTrue(candidate["credential_ref"].startswith("local-core:"))
        self.assertNotIn("secret", str(candidate))
        probe.assert_called_once()

    def test_bitcoinrpc_probe_uses_detected_cookie_credential_ref(self):
        ctx = SimpleNamespace(runtime_config={})
        with tempfile.TemporaryDirectory() as tmp:
            bitcoin_dir = Path(tmp) / ".bitcoin"
            bitcoin_dir.mkdir()
            (bitcoin_dir / ".cookie").write_text("__cookie__:secret", encoding="utf-8")
            ref = daemon._core_candidate_credential_ref(
                daemon._core_local_probe_candidates(bitcoin_dir)[0]
            )
            seen_backend = {}

            def fake_call(backend, method, params=None, wallet_name=None, timeout=None):
                del params, wallet_name, timeout
                seen_backend.update(backend)
                if method == "getblockchaininfo":
                    return {
                        "chain": "main",
                        "blocks": 1,
                        "headers": 1,
                        "pruned": False,
                        "initialblockdownload": False,
                    }
                if method == "getnetworkinfo":
                    return {"version": 270000, "connections": 1}
                if method == "listwallets":
                    return []
                if method == "getbestblockhash":
                    return "best-block"
                if method == "getblockfilter":
                    return {"filter": "00"}
                raise AssertionError(f"Unexpected RPC call: {method}")

            with patch.object(daemon.Path, "home", return_value=Path(tmp)), patch(
                "kassiber.daemon.bitcoinrpc_call",
                side_effect=fake_call,
            ):
                payload = daemon._test_bitcoinrpc_backend_payload(
                    ctx,
                    {"credential_ref": ref, "timeout": 10},
                )

        self.assertTrue(payload["reachable"])
        self.assertTrue(seen_backend["cookiefile"].endswith(".bitcoin/.cookie"))

    def test_detect_core_reads_bitcoin_conf_rpc_settings(self):
        with tempfile.TemporaryDirectory() as tmp:
            bitcoin_dir = Path(tmp) / ".bitcoin"
            bitcoin_dir.mkdir()
            (bitcoin_dir / "bitcoin.conf").write_text(
                "\n".join(
                    [
                        "rpcbind=127.0.0.1",
                        "rpcuser=alice",
                        "rpcpassword=correct horse battery staple",
                        "[signet]",
                        "rpcport=38332",
                        "rpccookiefile=signet/.cookie",
                    ]
                ),
                encoding="utf-8",
            )
            signet_dir = bitcoin_dir / "signet"
            signet_dir.mkdir()
            (signet_dir / ".cookie").write_text("__cookie__:secret", encoding="utf-8")
            probed_backends = []

            def fake_probe(backend):
                probed_backends.append(dict(backend))
                return {
                    "reachable": True,
                    "chain": backend["network"],
                    "network": backend["network"],
                    "blocks": 10,
                    "headers": 10,
                    "peers": 2,
                    "status": "synchronized",
                    "pruned": False,
                    "ibd": False,
                    "wallet_rpc": {"available": True, "loaded_wallet_count": 0},
                    "block_filters": {"available": True},
                    "warnings": [],
                }

            with patch.object(daemon.Path, "home", return_value=Path(tmp)), patch(
                "kassiber.daemon._bitcoinrpc_probe_payload",
                side_effect=fake_probe,
            ):
                payload = daemon._detect_core_payload({})

        basic = next(
            candidate
            for candidate in payload["candidates"]
            if candidate["auth_source"] == "basic"
        )
        self.assertEqual(basic["url"], "http://127.0.0.1:8332")
        self.assertEqual(basic["credential_source"], "bitcoin.conf")
        self.assertTrue(basic["credential_ref"].startswith("local-core:"))
        self.assertNotIn("username", basic)
        self.assertNotIn("password", basic)
        signet = next(
            candidate
            for candidate in payload["candidates"]
            if candidate["network"] == "signet"
            and candidate["auth_source"] == "cookiefile"
        )
        self.assertEqual(signet["url"], "http://127.0.0.1:38332")
        self.assertEqual(signet["auth_source"], "cookiefile")
        self.assertTrue(signet["cookiefile"].endswith(".bitcoin/signet/.cookie"))
        self.assertTrue(signet["credential_ref"].startswith("local-core:"))
        self.assertNotIn("secret", str(payload))
        self.assertNotIn("correct horse battery staple", str(payload))
        self.assertTrue(
            any(backend["username"] == "alice" for backend in probed_backends)
        )

    def test_bitcoinrpc_probe_uses_detected_basic_credential_ref(self):
        ctx = SimpleNamespace(runtime_config={})
        with tempfile.TemporaryDirectory() as tmp:
            bitcoin_dir = Path(tmp) / ".bitcoin"
            bitcoin_dir.mkdir()
            (bitcoin_dir / "bitcoin.conf").write_text(
                "rpcuser=alice\nrpcpassword=correct horse battery staple\n",
                encoding="utf-8",
            )
            ref = daemon._core_candidate_credential_ref(
                next(
                    candidate
                    for candidate in daemon._core_local_probe_candidates(bitcoin_dir)
                    if candidate["auth_source"] == "basic"
                )
            )
            seen_backend = {}

            def fake_call(backend, method, params=None, wallet_name=None, timeout=None):
                del params, wallet_name, timeout
                seen_backend.update(backend)
                if method == "getblockchaininfo":
                    return {
                        "chain": "main",
                        "blocks": 1,
                        "headers": 1,
                        "pruned": False,
                        "initialblockdownload": False,
                    }
                if method == "getnetworkinfo":
                    return {"version": 270000, "connections": 1}
                if method == "listwallets":
                    return []
                if method == "getbestblockhash":
                    return "best-block"
                if method == "getblockfilter":
                    return {"filter": "00"}
                raise AssertionError(f"Unexpected RPC call: {method}")

            with patch.object(daemon.Path, "home", return_value=Path(tmp)), patch(
                "kassiber.daemon.bitcoinrpc_call",
                side_effect=fake_call,
            ):
                payload = daemon._test_bitcoinrpc_backend_payload(
                    ctx,
                    {"credential_ref": ref, "timeout": 10},
                )

        self.assertTrue(payload["reachable"])
        self.assertEqual(seen_backend["username"], "alice")
        self.assertEqual(seen_backend["password"], "correct horse battery staple")

    def test_backend_create_uses_detected_basic_credential_ref(self):
        ctx = SimpleNamespace(conn=object(), runtime_config={})
        detected = {
            "name": "local-core-main",
            "kind": "bitcoinrpc",
            "chain": "bitcoin",
            "network": "main",
            "url": "http://127.0.0.1:8332",
            "auth_source": "basic",
            "credential_source": "bitcoin.conf",
            "username": "alice",
            "password": "correct horse battery staple",
        }
        created_payload = {"name": "core"}

        with patch(
            "kassiber.daemon._core_backend_from_credential_ref",
            return_value=detected,
        ), patch("kassiber.daemon.merge_db_backends"), patch(
            "kassiber.daemon.core_accounts.create_backend",
            return_value=created_payload,
        ) as create_backend:
            payload = daemon._create_backend_payload(
                ctx,
                {
                    "name": "core",
                    "kind": "bitcoinrpc",
                    "url": "http://attacker.example:8332",
                    "credential_ref": "local-core:test",
                    "config": {"display_name": "Local Core"},
                },
            )

        self.assertEqual(payload, created_payload)
        create_backend.assert_called_once()
        _conn, name, kind, url = create_backend.call_args.args[:4]
        self.assertEqual(name, "core")
        self.assertEqual(kind, "bitcoinrpc")
        self.assertEqual(url, "http://127.0.0.1:8332")
        config = create_backend.call_args.kwargs["config"]
        self.assertEqual(config["display_name"], "Local Core")
        self.assertEqual(config["username"], "alice")
        self.assertEqual(config["password"], "correct horse battery staple")

    def test_bitcoin_conf_rpc_url_accepts_host_port(self):
        self.assertEqual(
            daemon._core_rpc_url_from_settings(
                "main",
                {"rpcconnect": "127.0.0.1:28332"},
            ),
            "http://127.0.0.1:28332",
        )
        self.assertEqual(
            daemon._core_rpc_url_from_settings(
                "signet",
                {"rpcbind": "[::1]:38333"},
            ),
            "http://[::1]:38333",
        )

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

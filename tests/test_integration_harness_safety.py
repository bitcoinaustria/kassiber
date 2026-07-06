from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def _sourceable_harness_prefix(tmp_path: Path) -> Path:
    text = (ROOT / "scripts" / "integration-harness.sh").read_text(encoding="utf-8")
    prefix = text.split('\ncase "$MODE" in\n', 1)[0]
    path = tmp_path / "integration-harness-prefix.sh"
    path.write_text(prefix, encoding="utf-8")
    return path


def _run_harness_snippet(tmp_path: Path, snippet: str, env_updates: dict[str, str]):
    prefix = _sourceable_harness_prefix(tmp_path)
    env = os.environ.copy()
    env.update(env_updates)
    env.setdefault("VIRTUAL_ENV", "test")
    return subprocess.run(
        ["bash", "-c", f'source "{prefix}"; {snippet}'],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
        timeout=10,
    )


def _demo_manifest_env(tmp_path: Path, demo_home: Path) -> dict[str, str]:
    home = tmp_path / "home"
    home.mkdir(exist_ok=True)
    return {
        "HOME": str(home),
        "KASSIBER_REGTEST_DEMO_HOME": str(demo_home),
        "KASSIBER_REGTEST_CORE_URL": "http://127.0.0.1:18443",
        "KASSIBER_REGTEST_ELEMENTS_RPC_PORT": "18884",
        "KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT": "50001",
        "KASSIBER_REGTEST_FRIGATE_PORT": "18548",
        "KASSIBER_REGTEST_BITCOIN_MEMPOOL_PORT": "8080",
        "KASSIBER_REGTEST_LIQUID_ELECTRUM_PORT": "60001",
        "KASSIBER_REGTEST_LIQUID_MEMPOOL_PORT": "8081",
        "KASSIBER_REGTEST_RPC_USER": "demo-user",
        "KASSIBER_REGTEST_RPC_PASSWORD": "demo-pass",
    }


class IntegrationHarnessSafetyTest(unittest.TestCase):
    def test_demo_home_validation_rejects_dangerous_rebuild_and_purge_paths(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-demo-safety-") as tmp:
            tmp_path = Path(tmp)
            home = tmp_path / "home"
            home.mkdir()
            unsafe_paths = [
                home,
                Path("/tmp"),
                tmp_path / "not-a-dedicated-demo-dir",
            ]
            for unsafe in unsafe_paths:
                result = _run_harness_snippet(
                    tmp_path,
                    "demo_assert_safe_home rebuild",
                    {"HOME": str(home), "KASSIBER_REGTEST_DEMO_HOME": str(unsafe)},
                )
                self.assertNotEqual(result.returncode, 0, unsafe)
                self.assertIn("Refusing", result.stderr)

            purge_home = tmp_path / "kassiber-regtest-demo"
            result = _run_harness_snippet(
                tmp_path,
                "demo_assert_safe_home purge",
                {"HOME": str(home), "KASSIBER_REGTEST_DEMO_HOME": str(purge_home)},
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("missing Kassiber regtest demo manifest", result.stderr)

    def test_demo_home_validation_allows_purge_with_valid_manifest_marker(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-demo-safety-") as tmp:
            tmp_path = Path(tmp)
            demo_home = tmp_path / "custom-book"
            demo_home.mkdir()
            (demo_home / "demo-manifest.json").write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "scenario_id": "full-accounting-v1",
                        "data_root": str(demo_home / "data"),
                        "export_dir": str(demo_home / "exports"),
                    }
                ),
                encoding="utf-8",
            )

            result = _run_harness_snippet(
                tmp_path,
                "demo_assert_safe_home purge",
                _demo_manifest_env(tmp_path, demo_home),
            )

            self.assertEqual(result.returncode, 0, result.stderr)

    def test_demo_write_manifest_replaces_preexisting_file_with_private_mode(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-demo-manifest-") as tmp:
            tmp_path = Path(tmp)
            demo_home = tmp_path / "kassiber-regtest-demo"
            env = _demo_manifest_env(tmp_path, demo_home)
            result = _run_harness_snippet(
                tmp_path,
                (
                    'mkdir -p "$DEMO_HOME"; '
                    'printf stale > "$DEMO_MANIFEST"; '
                    'chmod 0644 "$DEMO_MANIFEST"; '
                    'demo_write_manifest abc123'
                ),
                env,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            manifest = demo_home / "demo-manifest.json"
            self.assertEqual(manifest.stat().st_mode & 0o777, 0o600)
            payload = json.loads(manifest.read_text(encoding="utf-8"))
            self.assertEqual(payload["scenario_checksum"], "abc123")

    def test_demo_write_manifest_includes_btcpay_seed_metadata_when_enabled(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-demo-manifest-") as tmp:
            tmp_path = Path(tmp)
            demo_home = tmp_path / "kassiber-regtest-demo"
            demo_home.mkdir()
            (demo_home / "btcpay-seed.json").write_text(
                json.dumps(
                    {
                        "api_key": "test-token",
                        "payment_method_id": "BTC-CHAIN",
                        "store_id": "store123",
                        "user": "merchant.regtest@example.invalid",
                    }
                ),
                encoding="utf-8",
            )
            env = _demo_manifest_env(tmp_path, demo_home)
            env.update(
                {
                    "KASSIBER_REGTEST_DEMO_BTCPAY_ENABLED": "1",
                    "KASSIBER_REGTEST_BTCPAY_PORT": "18549",
                    "KASSIBER_REGTEST_BTCPAY_NBXPLORER_PORT": "18550",
                }
            )
            result = _run_harness_snippet(tmp_path, "demo_write_manifest abc123", env)

            self.assertEqual(result.returncode, 0, result.stderr)
            payload = json.loads((demo_home / "demo-manifest.json").read_text(encoding="utf-8"))
            self.assertTrue(payload["btcpay_enabled"])
            self.assertEqual(payload["btcpay_url"], "http://127.0.0.1:18549")
            self.assertEqual(payload["btcpay_backend"], "btcpay-regtest")
            self.assertEqual(payload["btcpay_wallet"], "BTCPay Regtest Store")
            self.assertEqual(payload["btcpay_store_id"], "store123")
            self.assertEqual(payload["btcpay_payment_method_id"], "BTC-CHAIN")
            self.assertEqual(payload["btcpay_user"], "merchant.regtest@example.invalid")
            self.assertEqual(payload["btcpay_api_key"], "test-token")

    def test_demo_btcpay_is_default_on_with_opt_out(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-demo-btcpay-") as tmp:
            tmp_path = Path(tmp)
            default_result = _run_harness_snippet(
                tmp_path,
                'demo_configure_btcpay; printf "%s:%s:%s\\n" "${KASSIBER_REGTEST_USE_BTCPAY_COMPOSE:-}" "${KASSIBER_REGTEST_DEMO_BTCPAY_ENABLED:-}" "${KASSIBER_REGTEST_BTCPAY_PORT:-}"',
                {"KASSIBER_REGTEST_RPC_PORT": "18443"},
            )
            self.assertEqual(default_result.returncode, 0, default_result.stderr)
            self.assertEqual(default_result.stdout.strip(), "1:1:18549")

            disabled_result = _run_harness_snippet(
                tmp_path,
                'demo_configure_btcpay; printf "%s:%s\\n" "${KASSIBER_REGTEST_USE_BTCPAY_COMPOSE:-}" "${KASSIBER_REGTEST_DEMO_BTCPAY_ENABLED:-}"',
                {
                    "KASSIBER_REGTEST_DEMO_BTCPAY": "0",
                    "KASSIBER_REGTEST_RPC_PORT": "18443",
                },
            )
            self.assertEqual(disabled_result.returncode, 0, disabled_result.stderr)
            self.assertEqual(disabled_result.stdout.strip(), ":0")


if __name__ == "__main__":
    unittest.main()

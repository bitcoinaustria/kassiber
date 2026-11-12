"""The chain-observer lane's own wiring and its truth manifest (no Docker)."""
from __future__ import annotations

import unittest
from pathlib import Path

from tests.integration.chain_observer_oracle import BITCOIN_TRANSITIONS, LIQUID_TRANSITIONS, TruthManifest

ROOT = Path(__file__).resolve().parents[2]


class ChainObserverOracleTest(unittest.TestCase):
    def capture_all(self, manifest: TruthManifest, names, tip_hash: str) -> None:
        for name in names:
            manifest.capture(
                name,
                tip={"height": 1, "hash": tip_hash},
                transactions=(),
                utxos=(),
                ownership=(),
                highest_used={"receive": 0, "change": 0},
            )

    def test_truth_manifest_is_bounded_and_ordered(self):
        bitcoin = TruthManifest("bitcoin", "regtest", "unit")
        self.capture_all(bitcoin, BITCOIN_TRANSITIONS, "11" * 32)
        bitcoin.validate()

        liquid = TruthManifest("liquid", "elementsregtest", "unit")
        self.capture_all(liquid, LIQUID_TRANSITIONS, "22" * 32)
        liquid.validate()

        broken = TruthManifest("bitcoin", "regtest", "unit")
        self.capture_all(broken, ("noop",), "33" * 32)
        with self.assertRaisesRegex(AssertionError, "transition order"):
            broken.validate()

    def test_lane_is_shared_chain_selectable_and_python_product_free(self):
        harness = (ROOT / "scripts" / "integration-harness.sh").read_text(encoding="utf-8")
        self.assertIn("KASSIBER_CHAIN_OBSERVER_CHAIN", harness)
        self.assertIn("run_with_bitcoin_core run_chain_observer_oracle", harness)
        for module in ("test_live_chain_observer_oracle", "test_live_bdk_observer", "test_live_lwk_observer"):
            self.assertIn(f"py_unittest tests.integration.{module}", harness)
        self.assertNotIn("compose.chain-observers", harness)
        self.assertNotIn("bdkpython", harness)
        self.assertNotIn("uv run", harness)

        oracle = (ROOT / "tests" / "integration" / "chain_observer_oracle.py").read_text(encoding="utf-8")
        self.assertIn("deadline = time.monotonic() + 180", oracle)
        self.assertIn("transport tips have not caught up", oracle)

    def test_ci_job_is_path_aware_and_runs_the_rust_binary(self):
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
        job = workflow[workflow.index("  chain-observers:"):workflow.index("\n  required:")]
        self.assertIn("name: Chain observers (Linux Docker)", job)
        self.assertIn("Detect observer-sensitive changes", job)
        for path in ("crates/chain", "crates/wallets", "crates/store", "dev/regtest", "tests/integration"):
            self.assertIn(path, job)
        self.assertIn("cargo build --locked --release -p kassiber", job)
        self.assertIn("./scripts/integration-harness.sh chain-observers", job)
        self.assertIn("./scripts/integration-harness.sh chain-analysis", job)
        for python_setup in ("setup-uv", "uv sync", "uv run", "setup-python"):
            self.assertNotIn(python_setup, job)
        self.assertIn("- chain-observers", workflow[workflow.index("\n  required:"):])


if __name__ == "__main__":
    unittest.main()

"""The shadow runtime on the fast exchange lifecycle (`runtime_shadow.py`)."""
from __future__ import annotations

import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from tests.integration import regtest_demo
from tests.integration.regtest_exchange_cases import EXPECTED, native_replay_lifecycle
from tests.integration.runtime_parity import RUST_BIN, child_env

ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN names the Rust binary to compare")
class RuntimeShadowTest(unittest.TestCase):
    def test_every_step_runs_on_both_runtimes_and_matches(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp).resolve()
            root = base / "work"
            root.mkdir()
            log = base / "shadow" / "steps.jsonl"
            env = child_env(base / "home", python_child=True)
            env.update(
                KASSIBER_RUST_BIN=RUST_BIN,
                KASSIBER_RUNTIME_SHADOW_DIR=str(base / "shadow"),
                KASSIBER_RUNTIME_SHADOW_LOG=str(log),
                KASSIBER_TEST_CLI=json.dumps([sys.executable, str(ROOT / "tests/integration/runtime_shadow.py")]),
            )
            (base / "shadow").mkdir()
            with patch.dict(os.environ, env, clear=True):
                result = native_replay_lifecycle(root, regtest_demo.run_cli)
            steps = [json.loads(line) for line in log.read_text().splitlines()]
        # The lane kept Python's answers.
        self.assertEqual(result["actual"]["combined_quantity_msat"], EXPECTED["combined_quantity_msat"])
        self.assertGreater(len(steps), 15)
        for step in steps:
            self.assertEqual((step["python_rc"], step["rust_rc"]), (0, 0), step)
            self.assertIn(step["envelope"], {"same", "identity-ordered"}, step)
            self.assertIn(step["book"], {"same", "identity-ordered"}, step)
        self.assertEqual(steps[0]["argv"][-1], "init")


if __name__ == "__main__":
    unittest.main()

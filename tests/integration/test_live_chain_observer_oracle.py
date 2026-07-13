from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path

from tests.integration.chain_observer_oracle import run
from tests.integration.env import skip_unless_integration


@skip_unless_integration
class LiveChainObserverOracleTest(unittest.TestCase):
    def test_independent_core_and_elements_truth_manifests(self):
        selected = os.environ.get("KASSIBER_CHAIN_OBSERVER_CHAIN", "all")
        with tempfile.TemporaryDirectory(prefix="kassiber-observer-oracle-test-") as tmp:
            result = run(chain=selected, output_dir=Path(tmp))
            manifests = result["data"]["manifests"]
            expected = {"bitcoin", "liquid"} if selected == "all" else {selected}
            self.assertEqual(set(manifests), expected)
            for chain, entry in manifests.items():
                path = Path(entry["path"])
                self.assertTrue(path.is_file())
                self.assertEqual(entry["manifest"]["chain"], chain)
                self.assertGreaterEqual(len(entry["manifest"]["transitions"]), 12)
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)


if __name__ == "__main__":
    unittest.main()

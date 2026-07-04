from __future__ import annotations

import os
import unittest

from . import lightning_business_regtest


@unittest.skipUnless(
    os.environ.get("KASSIBER_LIGHTNING_BUSINESS") == "1",
    "set KASSIBER_LIGHTNING_BUSINESS=1 to run the live CLN business regtest lane",
)
class LiveLightningBusinessRegtest(unittest.TestCase):
    def test_merchant_only_lightning_business_lane(self) -> None:
        summary = lightning_business_regtest.run()
        self.assertEqual(summary["snapshot"]["alias"], "kassiber-merchant")
        self.assertGreaterEqual(summary["snapshot"]["channels"], 2)
        self.assertGreaterEqual(summary["snapshot"]["forwards"], 1)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()

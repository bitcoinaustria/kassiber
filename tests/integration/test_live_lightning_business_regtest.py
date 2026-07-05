from __future__ import annotations

import os
import unittest

from . import lightning_business_regtest


@unittest.skipUnless(
    os.environ.get("KASSIBER_LIGHTNING_BUSINESS") == "1",
    "set KASSIBER_LIGHTNING_BUSINESS=1 to run the live Lightning business regtest lane",
)
class LiveLightningBusinessRegtest(unittest.TestCase):
    def test_cln_merchant_with_lnd_backup_channel(self) -> None:
        summary = lightning_business_regtest.run()
        self.assertEqual(summary["snapshot"]["alias"], "kassiber-merchant")
        self.assertGreaterEqual(summary["snapshot"]["channels"], 3)
        self.assertGreaterEqual(summary["snapshot"]["forwards"], 1)
        self.assertEqual(summary["lnd_snapshot"]["alias"], "kassiber-lnd-backup")
        self.assertGreaterEqual(summary["lnd_snapshot"]["channels"], 1)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()

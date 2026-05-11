from __future__ import annotations

import inspect
import unittest

from kassiber.cli.main import build_parser
from kassiber.core import reports as core_reports


class ReportContractDriftTests(unittest.TestCase):
    def test_balance_history_default_interval_is_shared(self):
        signature = inspect.signature(core_reports.report_balance_history)
        self.assertEqual(
            signature.parameters["interval"].default,
            core_reports.DEFAULT_BALANCE_HISTORY_INTERVAL,
        )

        args = build_parser().parse_args(["reports", "balance-history"])
        self.assertEqual(args.interval, core_reports.DEFAULT_BALANCE_HISTORY_INTERVAL)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()

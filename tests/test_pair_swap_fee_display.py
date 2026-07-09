"""Persisted NULL swap_fee_msat must not invent out-in as a fee."""

from __future__ import annotations

import unittest

from kassiber.core.reports import _pair_swap_fee_msat


class PairSwapFeeDisplayTests(unittest.TestCase):
    def test_null_swap_fee_is_zero_not_amount_delta(self):
        self.assertEqual(
            _pair_swap_fee_msat(
                {
                    "swap_fee_msat": None,
                    "out_amount": 501_000_000_000,
                    "in_amount": 500_000_000_000,
                }
            ),
            0,
        )

    def test_persisted_swap_fee_is_returned(self):
        self.assertEqual(
            _pair_swap_fee_msat(
                {
                    "swap_fee_msat": 5_000_000,
                    "out_amount": 100_000_000_000,
                    "in_amount": 99_000_000_000,
                }
            ),
            5_000_000,
        )

    def test_zero_swap_fee_is_returned(self):
        self.assertEqual(
            _pair_swap_fee_msat(
                {"swap_fee_msat": 0, "out_amount": 1, "in_amount": 1}
            ),
            0,
        )


if __name__ == "__main__":
    unittest.main()

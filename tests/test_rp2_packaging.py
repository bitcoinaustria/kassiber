"""The adapter's configuration labels must survive RP2's INI syntax.

RP2 reads wallet, profile, and asset labels from an INI file, so a label with
an INI delimiter fails closed with a validation error naming the field. The
RP2 import checks (no file logger, a read-only cwd) live with the oracle in
``tests/tax_oracle/test_oracle_contract.py``.
"""

from __future__ import annotations

import unittest

from kassiber.errors import AppError


class Rp2ConfigurationLabelTest(unittest.TestCase):
    def test_rp2_configuration_rejects_delimited_tokens(self) -> None:
        from kassiber.core.engines.rp2 import _rp2_configuration

        cases = [
            ("wallet_label", {"label": "Default"}, ["Cold, Savings"], ["BTC"]),
            ("profile_label", {"label": "Alice\nBob"}, ["Cold"], ["BTC"]),
            ("asset", {"label": "Default"}, ["Cold"], ["BTC=USD"]),
        ]
        for field, profile, wallet_labels, assets in cases:
            with self.subTest(field=field):
                with self.assertRaises(AppError) as raised:
                    with _rp2_configuration(profile, wallet_labels, assets):
                        pass

                self.assertEqual(raised.exception.code, "validation")
                self.assertEqual(raised.exception.details["field"], field)


if __name__ == "__main__":
    unittest.main()

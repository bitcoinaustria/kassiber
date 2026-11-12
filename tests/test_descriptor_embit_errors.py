"""embit's own descriptor errors never escape a user-facing path.

embit raises `DescriptorError` (an `EmbitError`, not a `ValueError`) for
descriptors it parses but rejects, such as a Liquid view key whose wildcard
or branches do not mirror the spending descriptor. Each path answers the
structured validation error the Rust backend sends, without the parser's
text, which can quote key material.
"""
from __future__ import annotations

import unittest

from kassiber.core import samourai
from kassiber.core.wallets import _wallet_descriptor_state
from kassiber.errors import AppError

from .test_daemon_descriptor_preview import _mismatched_view_keys


class EmbitDescriptorErrorTests(unittest.TestCase):
    def assert_invalid_descriptor(self, call, secrets):
        with self.assertRaises(AppError) as ctx:
            call()
        error = ctx.exception
        self.assertEqual((error.code, str(error), error.hint, error.details),
                         ("validation", "Invalid wallet descriptor", None, None))
        for secret in secrets:
            self.assertNotIn(secret, str(error))

    def test_wallet_listing_refuses_a_stored_mismatched_view_key(self):
        # `list_wallets` / `get_wallet` describe each stored descriptor.
        for name, (descriptor, secrets) in _mismatched_view_keys().items():
            with self.subTest(name):
                config = {"descriptor": descriptor, "chain": "liquid"}
                self.assert_invalid_descriptor(lambda: _wallet_descriptor_state(config), secrets)

    def test_samourai_descriptor_validation_refuses_embit_errors(self):
        for name, (descriptor, secrets) in _mismatched_view_keys().items():
            with self.subTest(name):
                config = {"descriptor": descriptor, "chain": "liquid"}
                self.assert_invalid_descriptor(lambda: samourai._validate_descriptor_config(config), secrets)

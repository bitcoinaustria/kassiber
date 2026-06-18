"""Profile tax-policy display must reflect the STORED accounting method.

Regression guard for the divergence where an Austrian book left on FIFO was
displayed as "ATM" (moving-average) because the label/method helpers substituted
the AT policy default instead of the profile's actual stored gains_algorithm —
masking a tax-affecting misconfiguration the engine would still compute on.
"""

import tempfile
import unittest
from pathlib import Path

from kassiber.core.accounts import _normalized_profile_algorithm
from kassiber.core.ui_snapshot import _profile_policy_method, _tax_policy_label
from kassiber.db import ensure_schema_compat, open_db
from kassiber.tax_policy import build_tax_policy


class ProfilePolicyLabelTest(unittest.TestCase):
    def _profile(self, **overrides):
        base = {
            "tax_country": "at",
            "gains_algorithm": "MOVING_AVERAGE_AT",
            "fiat_currency": "EUR",
            "tax_long_term_days": 365,
        }
        base.update(overrides)
        return base

    def test_austrian_book_on_fifo_is_not_mislabeled_atm(self):
        fifo = self._profile(gains_algorithm="FIFO")
        self.assertEqual(_tax_policy_label(fifo), "Austria - FIFO - EUR")
        self.assertEqual(_profile_policy_method(fifo), "fifo")

    def test_austrian_book_on_moving_average_shows_atm(self):
        atm = self._profile(gains_algorithm="MOVING_AVERAGE_AT")
        self.assertEqual(_tax_policy_label(atm), "Austria - ATM - EUR")
        self.assertEqual(_profile_policy_method(atm), "moving_average_at")

    def test_generic_book_label_unchanged(self):
        generic = self._profile(
            tax_country="generic",
            gains_algorithm="HIFO",
            fiat_currency="CHF",
            tax_long_term_days=730,
        )
        self.assertEqual(
            _tax_policy_label(generic),
            "Generic - HIFO - CHF - 730 day long-term",
        )
        self.assertEqual(_profile_policy_method(generic), "hifo")


class AustrianMethodEnforcementTest(unittest.TestCase):
    """An Austrian book must always resolve to the moving-average method; FIFO
    (or any other) is coerced, so a book can never silently inherit/keep FIFO
    for Austria the way this one did."""

    def _policy(self, country):
        return build_tax_policy(
            {
                "tax_country": country,
                "fiat_currency": "EUR",
                "tax_long_term_days": 365,
            }
        )

    def test_austrian_book_is_coerced_to_moving_average(self):
        at = self._policy("at")
        self.assertEqual(_normalized_profile_algorithm("FIFO", at), "MOVING_AVERAGE_AT")
        self.assertEqual(_normalized_profile_algorithm("HIFO", at), "MOVING_AVERAGE_AT")
        self.assertEqual(_normalized_profile_algorithm(None, at), "MOVING_AVERAGE_AT")
        self.assertEqual(
            _normalized_profile_algorithm("moving_average_at", at),
            "MOVING_AVERAGE_AT",
        )

    def test_generic_book_keeps_the_chosen_method(self):
        generic = self._policy("generic")
        self.assertEqual(_normalized_profile_algorithm("FIFO", generic), "FIFO")
        self.assertEqual(_normalized_profile_algorithm("HIFO", generic), "HIFO")


class AustrianMethodOnOpenBackfillTest(unittest.TestCase):
    """Books that predate the enforcement get healed on open: an Austrian book
    stuck on FIFO is coerced to moving_average_at and its journals invalidated."""

    def _open(self):
        root = Path(tempfile.mkdtemp())
        return open_db(str(root / "kb.sqlite3"))

    def test_open_corrects_austrian_fifo_and_invalidates_journals(self):
        conn = self._open()
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) "
            "VALUES('w', 'Main', '2026-01-01T00:00:00Z')"
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, journal_input_version,
                last_processed_input_version, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES('p', 'w', 'Default', 'EUR', 'at', 365, 'FIFO', 0, 0,
                     '2026-01-02T00:00:00Z', 5, '2026-01-01T00:00:00Z')
            """
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES('g', 'w', 'Generic', 'EUR', 'generic', 365, 'FIFO',
                     '2026-01-01T00:00:00Z')
            """
        )
        conn.commit()

        ensure_schema_compat(conn)  # the on-open backfill (idempotent)
        conn.commit()

        at = conn.execute(
            "SELECT gains_algorithm, journal_input_version, last_processed_at, "
            "last_processed_tx_count FROM profiles WHERE id='p'"
        ).fetchone()
        self.assertEqual(at["gains_algorithm"], "MOVING_AVERAGE_AT")
        self.assertEqual(at["journal_input_version"], 1)  # bumped → reprocess
        self.assertIsNone(at["last_processed_at"])
        self.assertEqual(at["last_processed_tx_count"], 0)

        # A generic book on FIFO is a legitimate choice and stays untouched.
        gen = conn.execute(
            "SELECT gains_algorithm FROM profiles WHERE id='g'"
        ).fetchone()
        self.assertEqual(gen["gains_algorithm"], "FIFO")
        conn.close()


if __name__ == "__main__":
    unittest.main()

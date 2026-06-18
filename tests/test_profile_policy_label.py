"""Profile tax-policy display must reflect the STORED accounting method.

Regression guard for the divergence where an Austrian book left on FIFO was
displayed as "ATM" (moving-average) because the label/method helpers substituted
the AT policy default instead of the profile's actual stored gains_algorithm —
masking a tax-affecting misconfiguration the engine would still compute on.
"""

import unittest

from kassiber.core.accounts import _normalized_profile_algorithm
from kassiber.core.ui_snapshot import _profile_policy_method, _tax_policy_label
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


class UpdateProfileMethodPreservationTest(unittest.TestCase):
    """A non-method profile update (coarse-review toggle, label edit) must NOT
    silently re-coerce a legacy Austrian-on-FIFO book to moving-average. That
    incidental coercion is exactly the silent tax-method mutation the
    explicit-surface revert (3896bdd3) removed; only an explicit method/country
    change may re-enforce the Austrian method.
    """

    def _legacy_at_fifo_book(self):
        import tempfile
        from pathlib import Path

        from kassiber.core import accounts as core_accounts
        from kassiber.db import open_db

        tmp = tempfile.TemporaryDirectory(prefix="kassiber-updateprofile-")
        self.addCleanup(tmp.cleanup)
        conn = open_db(Path(tmp.name) / "data")
        self.addCleanup(conn.close)
        core_accounts.create_workspace(conn, "MB")
        # create_profile coerces AT -> moving-average, so reproduce the legacy
        # state (FIFO inherited before enforcement existed) with a direct write.
        row = core_accounts.create_profile(
            conn, "MB", "Dep", "EUR", "MOVING_AVERAGE_AT", "at", 365
        )
        profile_id = row["id"]
        conn.execute(
            "UPDATE profiles SET gains_algorithm='FIFO' WHERE id=?", (profile_id,)
        )
        conn.commit()
        return conn, core_accounts, profile_id

    def _row(self, conn, profile_id):
        row = conn.execute(
            "SELECT gains_algorithm, journal_input_version FROM profiles WHERE id=?",
            (profile_id,),
        ).fetchone()
        return row["gains_algorithm"], int(row["journal_input_version"] or 0)

    def test_coarse_review_toggle_preserves_legacy_fifo_method(self):
        conn, core_accounts, profile_id = self._legacy_at_fifo_book()
        core_accounts.update_profile(
            conn, "MB", profile_id, {"require_coarse_review": True}
        )
        algo, _ = self._row(conn, profile_id)
        self.assertEqual(algo, "FIFO")

    def test_label_change_preserves_method_and_does_not_reprocess(self):
        conn, core_accounts, profile_id = self._legacy_at_fifo_book()
        _, before = self._row(conn, profile_id)
        core_accounts.update_profile(conn, "MB", profile_id, {"label": "Dep renamed"})
        algo, after = self._row(conn, profile_id)
        self.assertEqual(algo, "FIFO")
        self.assertEqual(after, before)

    def test_explicit_method_change_still_enforces_austrian_method(self):
        conn, core_accounts, profile_id = self._legacy_at_fifo_book()
        _, before = self._row(conn, profile_id)
        # The dialog sends gains_algorithm explicitly; even a non-AT request is
        # coerced to moving-average for an Austrian book, and journals recompute.
        core_accounts.update_profile(
            conn, "MB", profile_id, {"gains_algorithm": "FIFO"}
        )
        algo, after = self._row(conn, profile_id)
        self.assertEqual(algo, "MOVING_AVERAGE_AT")
        self.assertGreater(after, before)


if __name__ == "__main__":
    unittest.main()

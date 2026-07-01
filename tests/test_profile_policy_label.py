"""Profile tax-policy display must reflect the STORED accounting method.

Regression guard for the divergence where an Austrian book left on FIFO was
displayed as "ATM" (moving-average) because the label/method helpers substituted
the AT policy default instead of the profile's actual stored gains_algorithm —
masking a tax-affecting misconfiguration the engine would still compute on.
"""

import unittest

from kassiber.core.accounts import _normalized_profile_algorithm
from kassiber.core.ui_snapshot import _profile_policy_method, _tax_policy_label
from kassiber.errors import AppError
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
    """An Austrian book defaults to the moving-average method (gleitender
    Durchschnittspreis) but may use any RP2 method. An explicit, caller-supplied
    method is preserved verbatim — no coercion — and only an unspecified method
    falls back to the Austrian default."""

    def _policy(self, country):
        return build_tax_policy(
            {
                "tax_country": country,
                "fiat_currency": "EUR",
                "tax_long_term_days": 365,
            }
        )

    def test_austrian_book_preserves_explicit_method_and_defaults_to_moving_average(self):
        at = self._policy("at")
        # Explicit methods are kept — Austrian books accept the generic methods
        # too, not only moving-average.
        self.assertEqual(_normalized_profile_algorithm("FIFO", at), "FIFO")
        self.assertEqual(_normalized_profile_algorithm("HIFO", at), "HIFO")
        self.assertEqual(
            _normalized_profile_algorithm("moving_average_at", at),
            "MOVING_AVERAGE_AT",
        )
        # No explicit method falls back to the Austrian default.
        self.assertEqual(_normalized_profile_algorithm(None, at), "MOVING_AVERAGE_AT")

    def test_generic_book_keeps_the_chosen_method(self):
        generic = self._policy("generic")
        self.assertEqual(_normalized_profile_algorithm("FIFO", generic), "FIFO")
        self.assertEqual(_normalized_profile_algorithm("HIFO", generic), "HIFO")


class UpdateProfileMethodPreservationTest(unittest.TestCase):
    """A non-method profile update (coarse-review toggle, label edit) must NOT
    silently mutate an Austrian-on-FIFO book's method. That incidental coercion
    is exactly the silent tax-method mutation the explicit-surface revert
    (3896bdd3) removed; only an explicit method/country change alters the stored
    method.
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
        # Austrian books accept any method now, so create the AT-on-FIFO book
        # directly — no coercion to work around.
        row = core_accounts.create_profile(
            conn, "MB", "Dep", "EUR", "FIFO", "at", 365
        )
        profile_id = row["id"]
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

    def test_explicit_method_change_applies_requested_method(self):
        conn, core_accounts, profile_id = self._legacy_at_fifo_book()
        _, before = self._row(conn, profile_id)
        # The dialog sends gains_algorithm explicitly; Austrian books accept any
        # RP2 method now, so the requested method is applied (not coerced) and
        # journals recompute.
        core_accounts.update_profile(
            conn, "MB", profile_id, {"gains_algorithm": "LIFO"}
        )
        algo, after = self._row(conn, profile_id)
        self.assertEqual(algo, "LIFO")
        self.assertGreater(after, before)


class ProfileRegionSwitchTest(unittest.TestCase):
    """The book-settings dialog can switch a book's region (tax_country).
    update_profile must apply the new region, validate the method for that
    region, and reprocess journals. The dialog always pairs the switch with a
    region-valid method; a generic book still rejects the Austrian method, but an
    Austrian book now accepts any RP2 method (the sent method is applied, not
    coerced to moving-average).
    """

    def _book(self, gains_algorithm, tax_country):
        import tempfile
        from pathlib import Path

        from kassiber.core import accounts as core_accounts
        from kassiber.db import open_db

        tmp = tempfile.TemporaryDirectory(prefix="kassiber-region-")
        self.addCleanup(tmp.cleanup)
        conn = open_db(Path(tmp.name) / "data")
        self.addCleanup(conn.close)
        core_accounts.create_workspace(conn, "MB")
        row = core_accounts.create_profile(
            conn, "MB", "Dep", "EUR", gains_algorithm, tax_country, 365
        )
        return conn, core_accounts, row["id"]

    def _row(self, conn, profile_id):
        row = conn.execute(
            "SELECT tax_country, gains_algorithm, journal_input_version "
            "FROM profiles WHERE id=?",
            (profile_id,),
        ).fetchone()
        return (
            row["tax_country"],
            row["gains_algorithm"],
            int(row["journal_input_version"] or 0),
        )

    def test_switch_generic_to_austria_applies_method_and_reprocesses(self):
        conn, core_accounts, profile_id = self._book("FIFO", "generic")
        _, _, before = self._row(conn, profile_id)
        core_accounts.update_profile(
            conn,
            "MB",
            profile_id,
            {"tax_country": "at", "gains_algorithm": "MOVING_AVERAGE_AT"},
        )
        country, algo, after = self._row(conn, profile_id)
        self.assertEqual(country, "at")
        self.assertEqual(algo, "MOVING_AVERAGE_AT")
        self.assertGreater(after, before)

    def test_switch_to_austria_keeps_an_explicit_non_austrian_method(self):
        conn, core_accounts, profile_id = self._book("FIFO", "generic")
        core_accounts.update_profile(
            conn,
            "MB",
            profile_id,
            {"tax_country": "at", "gains_algorithm": "FIFO"},
        )
        country, algo, _ = self._row(conn, profile_id)
        # Austrian books accept any RP2 method now, so an explicit FIFO survives
        # the region switch instead of being coerced to moving-average.
        self.assertEqual((country, algo), ("at", "FIFO"))

    def test_switch_austria_to_generic_with_region_valid_method(self):
        conn, core_accounts, profile_id = self._book("MOVING_AVERAGE_AT", "at")
        core_accounts.update_profile(
            conn,
            "MB",
            profile_id,
            {"tax_country": "generic", "gains_algorithm": "FIFO"},
        )
        country, algo, _ = self._row(conn, profile_id)
        self.assertEqual((country, algo), ("generic", "FIFO"))

    def test_switch_to_generic_keeping_austrian_method_is_rejected(self):
        # Documents why the dialog must send a region-valid method with a switch:
        # the Austrian method is not valid for a generic book.
        conn, core_accounts, profile_id = self._book("MOVING_AVERAGE_AT", "at")
        with self.assertRaises(AppError):
            core_accounts.update_profile(
                conn,
                "MB",
                profile_id,
                {"tax_country": "generic", "gains_algorithm": "MOVING_AVERAGE_AT"},
            )


class DaemonUpdateProfilePayloadRegionTest(unittest.TestCase):
    """The ui.profiles.update daemon payload must forward tax_country, not just
    the method, so the GUI can switch a book's region.
    """

    def _book(self, gains_algorithm, tax_country):
        import tempfile
        from pathlib import Path

        from kassiber.core import accounts as core_accounts
        from kassiber.db import open_db

        tmp = tempfile.TemporaryDirectory(prefix="kassiber-daemon-region-")
        self.addCleanup(tmp.cleanup)
        conn = open_db(Path(tmp.name) / "data")
        self.addCleanup(conn.close)
        core_accounts.create_workspace(conn, "MB")
        row = core_accounts.create_profile(
            conn, "MB", "Dep", "EUR", gains_algorithm, tax_country, 365
        )
        return conn, row["id"]

    def _stored(self, conn, profile_id):
        row = conn.execute(
            "SELECT tax_country, gains_algorithm FROM profiles WHERE id=?",
            (profile_id,),
        ).fetchone()
        return row["tax_country"], row["gains_algorithm"]

    def test_payload_applies_region_switch(self):
        from kassiber.daemon import _update_profile_payload

        conn, profile_id = self._book("FIFO", "generic")
        _update_profile_payload(
            conn,
            {
                "profile_id": profile_id,
                "gains_algorithm": "MOVING_AVERAGE_AT",
                "tax_country": "at",
            },
        )
        self.assertEqual(self._stored(conn, profile_id), ("at", "MOVING_AVERAGE_AT"))

    def test_payload_without_region_only_changes_method(self):
        from kassiber.daemon import _update_profile_payload

        conn, profile_id = self._book("FIFO", "generic")
        _update_profile_payload(
            conn,
            {"profile_id": profile_id, "gains_algorithm": "HIFO"},
        )
        self.assertEqual(self._stored(conn, profile_id), ("generic", "HIFO"))

    def test_payload_rejects_blank_region(self):
        from kassiber.daemon import _update_profile_payload

        conn, profile_id = self._book("FIFO", "generic")
        with self.assertRaises(AppError):
            _update_profile_payload(
                conn,
                {
                    "profile_id": profile_id,
                    "gains_algorithm": "FIFO",
                    "tax_country": "   ",
                },
            )


class DaemonCreateProfilePayloadRegionTest(unittest.TestCase):
    """The ui.profiles.create daemon payload can pick a region + method
    explicitly when not copying from a source book. Copying inherits the
    source's settings verbatim; explicit picks are ignored in that case.
    """

    def _conn(self):
        import tempfile
        from pathlib import Path

        from kassiber.core import accounts as core_accounts
        from kassiber.db import open_db

        tmp = tempfile.TemporaryDirectory(prefix="kassiber-create-region-")
        self.addCleanup(tmp.cleanup)
        conn = open_db(Path(tmp.name) / "data")
        self.addCleanup(conn.close)
        core_accounts.create_workspace(conn, "MB")
        seed = core_accounts.create_profile(
            conn, "MB", "Seed", "EUR", "FIFO", "generic", 365
        )
        return conn, core_accounts, seed["workspace_id"]

    def test_explicit_region_and_method(self):
        from kassiber.daemon import _create_profile_payload

        conn, _core, ws = self._conn()
        res = _create_profile_payload(
            conn,
            {
                "workspace_id": ws,
                "label": "AT book",
                "tax_country": "at",
                "gains_algorithm": "MOVING_AVERAGE_AT",
            },
        )
        self.assertEqual(res["defaults"]["tax_country"], "at")
        self.assertEqual(res["defaults"]["gains_algorithm"], "MOVING_AVERAGE_AT")

    def test_explicit_generic_method(self):
        from kassiber.daemon import _create_profile_payload

        conn, _core, ws = self._conn()
        res = _create_profile_payload(
            conn,
            {
                "workspace_id": ws,
                "label": "HIFO book",
                "tax_country": "generic",
                "gains_algorithm": "HIFO",
            },
        )
        self.assertEqual(res["defaults"]["tax_country"], "generic")
        self.assertEqual(res["defaults"]["gains_algorithm"], "HIFO")

    def test_explicit_austria_keeps_non_austrian_method(self):
        from kassiber.daemon import _create_profile_payload

        conn, _core, ws = self._conn()
        res = _create_profile_payload(
            conn,
            {
                "workspace_id": ws,
                "label": "AT with FIFO",
                "tax_country": "at",
                "gains_algorithm": "FIFO",
            },
        )
        # Austrian books accept any RP2 method now — the explicit FIFO is kept.
        self.assertEqual(res["defaults"]["tax_country"], "at")
        self.assertEqual(res["defaults"]["gains_algorithm"], "FIFO")

    def test_explicit_generic_after_at_context_resets_holding_period(self):
        # The default-derived region would be AT (the last created book becomes
        # the context default); an explicit generic pick must use the generic
        # holding period, not inherit the Austrian 0-day period.
        from kassiber.daemon import _create_profile_payload

        conn, core_accounts, ws = self._conn()
        core_accounts.create_profile(
            conn, ws, "ATctx", "EUR", "MOVING_AVERAGE_AT", "at", 0
        )
        res = _create_profile_payload(
            conn,
            {
                "workspace_id": ws,
                "label": "Generic again",
                "tax_country": "generic",
                "gains_algorithm": "FIFO",
            },
        )
        self.assertEqual(res["defaults"]["tax_country"], "generic")
        self.assertEqual(int(res["defaults"]["tax_long_term_days"]), 365)

    def test_source_profile_inherits_and_ignores_explicit_region(self):
        from kassiber.daemon import _create_profile_payload

        conn, core_accounts, ws = self._conn()
        source = core_accounts.create_profile(
            conn, ws, "ATsrc", "EUR", "MOVING_AVERAGE_AT", "at", 0
        )
        res = _create_profile_payload(
            conn,
            {
                "workspace_id": ws,
                "label": "Copy",
                "source_profile_id": source["id"],
                # These must be ignored — copying inherits the source's region.
                "tax_country": "generic",
                "gains_algorithm": "FIFO",
            },
        )
        self.assertEqual(res["defaults"]["tax_country"], "at")
        self.assertEqual(res["defaults"]["gains_algorithm"], "MOVING_AVERAGE_AT")


if __name__ == "__main__":
    unittest.main()

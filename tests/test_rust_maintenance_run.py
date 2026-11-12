import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_report_blockers import BARRIERS, CURRENT
from tests.test_rust_report_capital_gains import BOOK, generated, journal_shape
from tests.test_rust_transaction_dashboard import ACTIVITY


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustMaintenanceRunTest(DesktopParity, unittest.TestCase):
    KIND = "ui.maintenance.run"

    def test_refusals_leave_every_table_unchanged(self):
        self.change(ACTIVITY + CURRENT + "UPDATE profiles SET journal_input_version=3 WHERE id='p';")
        self.compare([[], "x", 3, {"z": 1, "a": 2}, {"sync": "never", "x": 1}, {"sync": "bad"},
                      {"sync": None}, {"sync": 1}, {"sync": True}, {"sync": " never"}, {"sync": "NEVER"}])

    def test_unhashable_sync_is_the_typed_validation_refusal(self):
        # tests/rust_port/repro_maintenance_unhashable_sync.py: Python checks
        # the type before set membership, so a list or object is refused like
        # any other bad mode, with no traceback.
        self.change(ACTIVITY + CURRENT + "UPDATE profiles SET journal_input_version=3 WHERE id='p';")
        responses = self.compare([{"sync": []}, {"sync": {"a": 1}}, {"sync": ["never"]}])
        self.assertEqual([(r["error"]["code"], r["error"]["details"]) for r in responses],
                         [("validation", {"sync": []}), ("validation", {"sync": {"a": 1}}),
                          ("validation", {"sync": ["never"]})])

    def test_current_book_without_sync_matches_wire_bytes(self):
        self.change(BARRIERS)
        self.compare([..., None, {}, {"sync": "never"}, {"sync": "if_enabled"}])
        self.change("UPDATE profiles SET require_coarse_review=1 WHERE id='p';"
                    "INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p',"
                    "'{\"report_read_sync\": false, \"background_enabled\": true}');")
        self.compare([{"sync": "if_enabled"}, {"sync": "never"}])
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare([{}, {"sync": "always"}, {"sync": "never"}])

    def test_mixed_network_book_refuses_journal_processing(self):
        self.change(ACTIVITY + CURRENT + "UPDATE profiles SET journal_input_version=3 WHERE id='p';")
        responses = self.compare([{"sync": "never"}], mutates=True, mask=generated, tables=journal_shape)
        self.assertEqual(responses[0]["error"]["code"], "book_network_review_required")

    def test_journals_of_other_semantics_are_rebuilt(self):
        # Counts and input versions match; only the calculation semantics the
        # journals were built with is not the required one, which Python's
        # projection_freshness calls stale. The rebuild meets the mixed book.
        self.change(ACTIVITY + CURRENT + "UPDATE profiles SET journal_semantics_version=1,"
                    "required_journal_semantics_version=1 WHERE id='p';")
        responses = self.compare([{"sync": "never"}], mutates=True, mask=generated, tables=journal_shape)
        self.assertEqual(responses[0]["error"]["code"], "book_network_review_required")

    def test_sqlcipher_maintenance_run_matches_wire_bytes(self):
        self.encrypt("synthetic maintenance key")
        self.change(BARRIERS)
        self.compare([{}, {"sync": "never"}])


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustMaintenanceStaleBookTest(DesktopParity, unittest.TestCase):
    KIND = "ui.maintenance.run"
    SEED = None

    def seed(self):
        self.change(BOOK + "UPDATE profiles SET journal_input_version=8 WHERE id='p';")

    def test_stale_journals_are_processed_and_reported(self):
        responses = self.compare([{"sync": "never"}, {}], mutates=True, mask=generated, tables=journal_shape)
        self.assertIn("journals", responses[0]["data"]["maintenance"])
        self.assertNotIn("journals", responses[1]["data"]["maintenance"])

    def test_sqlcipher_stale_book_matches_wire_bytes(self):
        self.encrypt("synthetic stale maintenance key")
        self.compare([{"sync": "if_enabled"}], mutates=True, mask=generated, tables=journal_shape)

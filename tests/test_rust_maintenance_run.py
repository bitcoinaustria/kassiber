import json
import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity, run_candidate
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

    def test_unhashable_sync_answers_python_envelope_without_traceback(self):
        # tests/rust_port/repro_maintenance_unhashable_sync.py: Python lets the
        # TypeError escape with a traceback; Rust keeps its envelope, minus debug.
        requests = self.requests([{"sync": []}, {"sync": {"a": 1}}])
        wire = "".join(json.dumps(request) + "\n" for request in [*requests, dict(kind="daemon.shutdown")])
        result = run_candidate(["--machine", "--data-root", str(self.root), "daemon"],
                               env=self.env, input=wire.encode())
        self.assertEqual(result.stderr, b"")
        errors = [json.loads(line)["error"] for line in result.stdout.splitlines()[1:3]]
        self.assertEqual([(e["code"], e["message"], e["debug"]) for e in errors],
                         [("internal_error", "unhashable type: 'list'", ""),
                          ("internal_error", "unhashable type: 'dict'", "")])

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

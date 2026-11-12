import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_evidence_summary import EVIDENCE

LOANS = """
INSERT INTO loan_legs(id,workspace_id,profile_id,transaction_id,loan_id,role,note,created_at) VALUES
 ('loan-1','w','p','h','loan','collateral','/home/user/loan.pdf','2025'),
 ('loan-2','w','p','i','loan','returned',NULL,'2025'),
 ('loan-3','w','p','k','terms','collateral','https://lender.invalid/terms','2025');
INSERT INTO transaction_edit_events(id,workspace_id,profile_id,transaction_id,source,changed_at,journal_input_version_after)
 VALUES('e9','w','p','j','gui','2025-07-01T00:00:00Z',9);
"""


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustTransactionReviewContextTest(DesktopParity, unittest.TestCase):
    KIND = "ui.transactions.review_context"

    def setUp(self):
        super().setUp()
        attachments = self.base / "state" / "attachments" / "p"
        attachments.mkdir(parents=True)
        (attachments / "present.pdf").write_bytes(b"%PDF")

    def test_refusals_match_wire_bytes(self):
        self.change(EVIDENCE)
        self.compare([..., None, [], "x", {}, {"transaction": " "}, {"transaction": 3},
                      {"transaction": "h", "z": 1, "a": 2}, {"transaction": "absent"},
                      {"transaction": "h", "history_limit": 0}, {"transaction": "h", "history_limit": "x"},
                      {"transaction": "h", "history_limit": None},
                      {"transaction": "h", "include_graph": 1},
                      {"transaction": "h", "include_privacy": None}])

    def test_review_bundles_match_wire_bytes(self):
        self.change(EVIDENCE + LOANS)
        responses = self.compare([{"transaction": ref} for ref in ("h", " i ", "j", "k", "l", "m", "a", "c")]
                                 + [{"transaction": "h", "history_limit": 1, "include_graph": False,
                                     "include_privacy": False, "include_evidence": False},
                                    {"transaction": "i", "history_limit": "99"},
                                    {"transaction": "j", "history_limit": 2.7}])
        self.assertEqual(responses[0]["data"]["safety"]["network_contacted"], False)

    def test_stale_book_is_read_without_maintenance(self):
        self.change(EVIDENCE + "UPDATE profiles SET journal_input_version=9 WHERE id='p';")
        self.compare([{"transaction": "h"}])

    def test_missing_context_matches_wire_bytes(self):
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare([{"transaction": "a"}])

    def test_sqlcipher_review_context_matches_wire_bytes(self):
        self.encrypt("synthetic review context key")
        self.change(EVIDENCE + LOANS)
        self.compare([{"transaction": "h"}, {"transaction": "j"}])

import unittest

from kassiber.db import open_db
from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_report_capital_gains import BOOK

LABELS = """
INSERT INTO bip329_labels(id,workspace_id,profile_id,record_type,ref,label,origin,spendable,data_json,created_at)
VALUES('l1','ws','p','tx','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','Payroll',NULL,NULL,'{}','2025'),
      ('l2','ws','p','xpub','xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz','Cold xpub',NULL,NULL,'{}','2025'),
      ('l3','ws','p','pubkey','02abc','Short key',NULL,NULL,'{}','2025');
"""


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustExportPreviewTest(DesktopParity, unittest.TestCase):
    KIND = "ui.exports.preview"
    SEED = None

    def seed(self):
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript(BOOK)

    def test_refusals_match_wire_bytes(self):
        self.compare([..., None, [], "x", {"z": 1, "y": 2}, {"export": 3}, {"export": "x", "format": 4},
                      {}, {"export": ""}, {"export": "bogus"}, {"export": "year_handoff"},
                      {"export": "year_handoff", "year": True}, {"export": "year_handoff", "year": "2025"},
                      {"export": "transactions", "format": "pdf"}, {"export": "transactions", "wallet": 3},
                      {"export": "transactions", "wallets": ["a", 1]},
                      {"export": "transactions", "wallets": [], "wallet": "x"},
                      {"export": "transactions", "range": 4}, {"export": "transactions", "range": " "},
                      {"export": "transactions", "range": "bogus"}, {"export": "bogus", "range": "this_year"},
                      {"export": "transactions", "wallet": "absent"},
                      {"export": "transactions", "wallets": []}])

    def test_transaction_platform_handoff_and_label_previews_match_wire_bytes(self):
        args = [{"export": "transactions"}, {"export": "transactions", "format": "csv"},
                {"export": "transactions", "format": ""},
                {"export": "transactions", "range": "this_year"},
                {"export": "transactions", "from": "2024-01-01", "to": "2024-12-31", "timezone": "Europe/Vienna"},
                {"export": "transactions", "range": "last_year", "format": "csv"},
                {"export": "transactions", "format": "cointracking"},
                {"export": "transactions", "format": "blockpit", "range": "last_year"},
                {"export": "year_handoff", "year": 2024}, {"export": "year_handoff", "year": 2025},
                {"export": "year_handoff", "year": 1999},
                {"export": "labels"}, {"export": "labels", "format": "jsonl"}]
        responses = self.compare(args)
        served = {index for index, response in enumerate(responses) if response["kind"] == self.KIND}
        self.assertTrue({0, 1, 8, 11} <= served, [r.get("error") for r in responses])
        self.assertTrue(responses[0]["data"]["sheets"][0]["rows"])
        self.change(LABELS)
        self.compare([{"export": "labels"}])
        self.change("UPDATE profiles SET journal_input_version=8 WHERE id='p';")
        self.compare(args)
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare([{"export": "transactions"}, {"export": "labels"}, {"export": "year_handoff", "year": 2024},
                      {"export": "transactions", "range": "this_year"}])

    def test_sqlcipher_preview_matches_wire_bytes(self):
        self.encrypt("synthetic preview key")
        self.compare([{"export": "transactions"}, {"export": "year_handoff", "year": 2024}, {"export": "labels"}])

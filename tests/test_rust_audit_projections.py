import re
import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity

CLOCK = re.compile(rb'"generated_at":"[^"]*"')


def clock(line):
    return CLOCK.sub(b'"generated_at":"<now>"', line)


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustAuditProjectionsTest(DesktopParity, unittest.TestCase):
    KIND = "ui.audit.changes_since_last_answer"

    def test_changes_since_last_answer_match_wire_bytes(self):
        self.change("""
            INSERT INTO rates_cache(pair,timestamp,rate,source,fetched_at)
            VALUES('BTC-EUR','2025-01-01T00:00:00Z',1,'manual','2025-02-01T00:00:00.5Z');
            INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,description,created_at)
            VALUES('j','w','p','b','ln','2025','transfer_in','BTC',1,'x','2025-02-02T00:00:00Z');
        """)
        args = [..., None, {}, [], "x", {"z": 1, "since": "2025"}, {"since": 3}, {"since": " "},
                {"since": "bad"}, {"since": "2025-02-01"}, {"since": " 2025-02-01T00:00:00+01:00 "},
                {"since": "2025-02-01T00:00:00.000001Z"}, {"since": "2030-01-01T00:00:00Z"},
                {"since": "2000-01-01"}, {"since": None}]
        self.compare(args, mask=clock)
        self.change("UPDATE workspaces SET label='' WHERE id='w'; DELETE FROM rates_cache;")
        self.compare([{}, {"since": "2000-01-01"}], mask=clock)
        self.change("DELETE FROM settings WHERE key='context_workspace';")
        self.compare([{}, {"since": "2000-01-01"}, {"since": "bad"}], mask=clock)

    def test_worklist_keeps_python_direct_dispatch_refusal(self):
        self.compare([..., None, {}, {"limit": 3}, [], {"categories": ["x"]}],
                     kind="ui.review.worklist")
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare([..., {}], kind="ui.review.worklist")

    def test_sqlcipher_audit_reads_match_wire_bytes(self):
        self.encrypt("synthetic audit key")
        self.compare([{}, {"since": "2000-01-01"}], mask=clock)
        self.compare([{}], kind="ui.review.worklist")

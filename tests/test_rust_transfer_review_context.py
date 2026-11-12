import unittest

from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_report_capital_gains import generated, journal_shape
from tests.test_rust_transaction_dashboard import ACTIVITY

REVIEW = ACTIVITY + """
UPDATE transactions SET description='Boltz swap ' || printf('%0300d', 1), note='api_key=synthetic-private-token aqua'
 WHERE id='h';
UPDATE transactions SET counterparty='Phoenix wallet', payment_hash='feed', payment_hash_source='lnd' WHERE id='i';
INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('t-ln','w','p','ln','Lightning top-up','2025');
INSERT INTO transaction_tags(transaction_id,tag_id) VALUES('i','t-ln');
INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,description,created_at)
VALUES('je-h','w','p','h','cold','2025-03-05T10:00:00Z','disposal','BTC',-5000000,'withdrawal secret=xyz','2025'),
      ('je-i','w','p','i','ln','2025-03-05T10:30:00Z','acquisition','BTC',4999000,'deposit','2025');
INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,detail_json,created_at)
VALUES('j','w','p','missing_price','{"asset":"BTC"}','2025'),('k','w','p','other','not json','2025');
INSERT INTO swap_matching_rules(id,workspace_id,profile_id,name,predicate_json,kind,policy,enabled,created_at,updated_at)
VALUES('r1','w','p','Boltz','{"method":"heuristic"}','submarine-swap','carrying-value',1,'2025-01-01','2025-01-01'),
      ('r2','w','p','Old','{}','manual','taxable',0,'2024-01-01','2024-01-01');
INSERT INTO saved_views(id,workspace_id,profile_id,surface,name,filter_json,created_at,updated_at)
VALUES('v1','w','p','swap_candidates','Swaps','{}','2025','2025'),('v2','w','p','transfer_candidates','Moves','{}','2025','2025');
UPDATE profiles SET last_processed_at='2025',last_processed_tx_count=13,journal_input_version=2,last_processed_input_version=2 WHERE id='p';
UPDATE profiles SET journal_semantics_version=10 WHERE id='p';
"""


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustTransferReviewContextTest(DesktopParity, unittest.TestCase):
    KIND = "ui.transfers.review_context"

    def test_refusals_match_wire_bytes(self):
        self.change(REVIEW)
        self.compare([[], "x", {"z": 1, "a": 2}, {"limit": "x"}, {"limit": 0}, {"limit": -3},
                      {"limit": None}, {"profile": "absent"}, {"workspace": "nope"}])

    def test_candidates_summaries_journals_and_actions_match_wire_bytes(self):
        self.change(REVIEW)
        responses = self.compare([..., {}, {"limit": 1}, {"limit": "2"}, {"limit": 2.9}, {"limit": True},
                                  {"limit": 500}, {"candidate_type": "transfer"},
                                  {"candidate_type": "swap"}, {"confidence": "strong"},
                                  {"method": "heuristic", "fee_pct_max": 0.5, "fee_sats_min": 1},
                                  {"time_window_seconds": 60, "fee_pct_max": 0.0001},
                                  {"asset_pair": "BTC-LBTC"}, {"route_pair": "onchain-lightning"},
                                  {"workspace": "Main", "profile": "Book"}, {"profile": "foreign"}])
        self.assertTrue(responses[1]["data"]["review_items"])

    def test_stale_journals_are_rebuilt_before_the_read(self):
        self.change(REVIEW + "UPDATE profiles SET journal_input_version=3 WHERE id='p';")
        self.compare([{}], mutates=True, mask=generated, tables=journal_shape)

    def test_sqlcipher_review_context_matches_wire_bytes(self):
        self.encrypt("synthetic review key")
        self.change(REVIEW)
        self.compare([{}, {"limit": 1, "candidate_type": "transfer"}])

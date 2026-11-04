import json
import unittest

from kassiber.core.source_funds import _snapshot_hash
from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_report_blockers import BARRIERS

CASE = {"case": {"snapshot_hash": "inner"}, "reveal_mode": "summary",
        "target": {"transaction_id": "k", "label": "Peg target"},
        "graph": {"nodes": [{"node_type": "transaction", "transaction_id": "j"},
                            {"node_type": "source", "transaction_id": "x"}, "bad",
                            {"node_type": "transaction", "transaction_id": "h"}]},
        "explain_gates": {"exportable": 1}, "note": "Grüße"}
RECEIPT = {"status": "verified", "base_input_version": 1, "result_input_version": 2,
           "transaction_ids": ["h", "i"], "note": "€ ünïcode"}
# Direct evidence, source-of-funds links, journal rows, review history, filed
# reports, AI assistance and review receipts across the barrier book.
EVIDENCE = BARRIERS + """
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,original_filename,stored_relpath,source_url,media_type,size_bytes,sha256,copied_from_attachment_id,created_at) VALUES
 ('at-file','w','p','h','file','Board approval minutes','minutes.pdf','p/present.pdf',NULL,'application/pdf',4,'ab',NULL,'2025-01-01'),
 ('at-missing','w','p','h','file',NULL,'gone.pdf','p/missing.pdf',NULL,NULL,NULL,NULL,'at-file','2025-01-02'),
 ('at-escape','w','p','i','file','','  ','../outside.pdf',NULL,'',7,'',NULL,'2025-01-03'),
 ('at-absolute','w','p','i','file','abs',NULL,'/etc/passwd',NULL,NULL,NULL,NULL,NULL,'2025-01-04'),
 ('at-url','w','p','i','url',NULL,NULL,NULL,'https://bob:pw@Docs.Example.COM:8443/doc?id=1#x','text/html',NULL,NULL,NULL,'2025-01-05'),
 ('at-query','w','p','j','url','Invoice',NULL,NULL,'https://shop.example/inv?Api%5FKey=s&x=+y','',NULL,NULL,NULL,'2025-01-06'),
 ('at-frag','w','p','j','url',NULL,NULL,NULL,'http://h.example/a#token',NULL,NULL,NULL,NULL,'2025-01-07'),
 ('at-plain','w','p','k','url',NULL,NULL,NULL,'mailto:x@example.com',NULL,NULL,NULL,NULL,'2025-01-08'),
 ('at-src','w','p',NULL,'file','Bank statement','stmt.pdf','p/present.pdf',NULL,NULL,4,NULL,NULL,'2025-01-09'),
 ('at-link','w','p',NULL,'url','Payslip',NULL,NULL,'https://hr.example/p?sig=1',NULL,NULL,NULL,NULL,'2025-01-10');
INSERT INTO source_funds_sources(id,workspace_id,profile_id,source_type,label,asset,amount,fiat_currency,fiat_value,acquired_at,description,review_state,created_at,updated_at) VALUES
 ('src','w','p','salary','Salary','BTC',123456789,'EUR',12.5,'2024-01-01',NULL,'reviewed','2025','2025');
INSERT INTO source_funds_source_attachments(source_id,attachment_id,created_at) VALUES('src','at-src','2025');
INSERT INTO source_funds_links(id,workspace_id,profile_id,from_source_id,from_transaction_id,to_transaction_id,link_type,state,confidence,method,asset,allocation_amount,from_asset,from_allocation_amount,allocation_policy,explanation,uses_chain_observation,chain_data_confirmed,created_at,updated_at) VALUES
 ('l1','w','p','src',NULL,'h','root','reviewed','high','manual','BTC',5000000,NULL,7,'fifo','Salary paid',1,1,'2025','2025'),
 ('l2','w','p',NULL,'l','h','parent','suggested','low','auto','BTC',3,'LBTC',4,'fifo','hidden',0,1,'2025','2025'),
 ('l3','w','p',NULL,'m','i','parent','reviewed','high','manual','BTC',1,'',NULL,'pro_rata',NULL,0,0,'2025','2025'),
 ('l4','w','p',NULL,'l','j','parent','rejected','low','auto','BTC',1,NULL,NULL,'fifo',NULL,0,0,'2025','2025');
INSERT INTO source_funds_link_attachments(link_id,attachment_id,created_at) VALUES('l3','at-link','2025');
INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,fiat_value,gain_loss,description,created_at) VALUES
 ('je-h','w','p','h','cold','2025-03-05T10:00:00Z','disposal','BTC',-5000000,-2.5,0.25,'withdrawal','2025'),
 ('je-i','w','p','i','ln','2025-03-05T10:30:00Z','acquisition','BTC',4999000,2.5,NULL,NULL,'2025');
INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,detail_json,created_at) VALUES('j','w','p','missing_price','{"asset":"BTC"}','2025');
UPDATE transactions SET pricing_source_kind='manual', note='management resolution' WHERE id='h';
UPDATE transactions SET fiat_price_source='kraken', excluded=1 WHERE id='l';
INSERT INTO transaction_tags(transaction_id,tag_id) VALUES('i','z'),('i','a');
INSERT INTO custody_gap_reviews(id,workspace_id,profile_id,gap_id,revision,candidate_fingerprint,action,event_kind,component_id,authored_source,reason,snapshot_json,created_at) VALUES
 ('r1','w','p','g1',1,'fp','resolved','bridge_created','cc','gui','bridged','{"retained_msat": 5, "residual_msat": "7", "residual_classification": {"classification": "fee"}}','2025-02-01'),
 ('r2','w','p','g2',2,'fp','dismissed','review_decision',NULL,'user',NULL,'{"residual_classification": "plain"}','2025-02-02'),
 ('r3','w','p','g3',1,'fp','dismissed','bridge_reopened','cc','ai_tool','r','{}','2025-02-03');
INSERT INTO custody_gap_review_relation_sets(id,review_id,workspace_id,profile_id,expected_source_count,expected_return_count,created_at) VALUES
 ('rs1','r1','w','p',1,1,'2025'),('rs2','r2','w','p',1,1,'2025');
INSERT INTO custody_gap_review_transactions(id,review_id,workspace_id,profile_id,role,transaction_id,created_at) VALUES
 ('x1','r1','w','p','source','h','2025'),('x2','r1','w','p','return','i','2025'),('x3','r2','w','p','source','j','2025'),
 ('x4','r2','w','p','return','k','2025');
INSERT INTO filed_report_snapshots(id,workspace_id,profile_id,report_kind,report_state,period_start_year,period_end_year,content_sha256,classification_summary_json,gain_summary_json,report_scope_json,authored_source,notes,created_at) VALUES
 ('fs1','w','p','capital_gains','filed',2025,2025,'{HASH}','{"a": 1}','[1]','bad','user','note','2025-03-01'),
 ('fs2','w','p','summary','saved',2024,2025,'{HASH}','{}','{"g": 2}','{}','gui',NULL,'2025-03-02');
INSERT INTO custody_filed_report_impacts(id,workspace_id,profile_id,filed_report_snapshot_id,component_id,review_id,gap_id,affected_period_start_year,affected_period_end_year,before_classification_summary_json,after_classification_summary_json,before_gain_summary_json,after_gain_summary_json,amendment_warning,created_at) VALUES
 ('im1','w','p','fs1','cc','r1','g1',2025,2025,'{"b": 1}','{}','{}','{"status": "pending"}','Amend','2025-03-03');
INSERT INTO custody_filed_report_impact_resolutions(id,workspace_id,profile_id,impact_id,rebuilt_at,after_classification_summary_json,after_gain_summary_json,classification_changed,gain_changed,amendment_status,created_at) VALUES
 ('ir1','w','p','im1','2025-03-04','{"c": 2}','{}',1,0,'review_required','2025-03-04');
INSERT INTO custody_ai_assistance_audits(id,workspace_id,profile_id,tool_name,daemon_kind,call_id,provider_kind,model,gap_id,candidate_fingerprint,facts_sha256,user_edited,consent_decision,consent_requested_at,consent_decided_at,execution_status,execution_code,result_sha256,review_id,component_id,created_at) VALUES
 ('ai1','w','p','custody.review','ui.custody.review.apply','c1','local','m','g1','fp','{HASH}',1,'allow_once','2025','2025','executed',NULL,'{HASH}','r1','cc','2025-04-01'),
 ('ai2','w','p','custody.review','ui.custody.review.plan','c2','local','m',NULL,NULL,'{HASH}',0,'deny','2025','2025','denied','denied',NULL,NULL,'other','2025-04-02');
INSERT INTO review_workflow_receipts(id,workspace_id,profile_id,idempotency_key,artifact_digest,receipt_json,created_at) VALUES
 ('rc1','w','p','k1','{HASH}','{RECEIPT}','2025-05-01'),
 ('rc2','w','p','k2','{HASH}','{"status": "verified", "base_input_version": 0, "result_input_version": 1, "transaction_ids": ["k"]}','2025-05-02');
INSERT INTO custody_authored_evidence_snapshots(workspace_id,profile_id,subject_kind,subject_id,detail_hash,quantity_hash,payload_json,created_at) VALUES
 ('w','p','custody_component','cc','d1','q1','{}','2025'),('w','p','custody_claim','claim','d2','q2','{}','2025');
INSERT INTO source_funds_cases(id,workspace_id,profile_id,target_transaction_id,target_amount,asset,reveal_mode,status,snapshot_hash,snapshot_json,created_at,updated_at) VALUES
 ('case','w','p','k',1,'LBTC','summary','saved','{CASE_HASH}','{CASE}','2025','2025'),
 ('tampered','w','p','k',1,'LBTC','summary','saved','0','{CASE}','2025','2025');
""".replace("{HASH}", "a" * 64).replace("{RECEIPT}", json.dumps(RECEIPT)).replace(
    "{CASE_HASH}", _snapshot_hash(CASE)).replace("{CASE}", json.dumps(CASE))


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustEvidenceSummaryTest(DesktopParity, unittest.TestCase):
    KIND = "ui.audit.evidence.summary"

    def setUp(self):
        super().setUp()
        attachments = self.base / "state" / "attachments" / "p"
        attachments.mkdir(parents=True)
        (attachments / "present.pdf").write_bytes(b"%PDF")

    def test_refusals_match_wire_bytes(self):
        self.compare([[], "x", {"transaction": 3}, {"transaction": "h", "transactions": ["i"]},
                      {"transactions": []}, {"transactions": "h"}, {"transactions": ["h", " "]},
                      {"transactions": ["h", 1]}, {"transaction": "h", "source_funds_case": "case"},
                      {"source_funds_case": 1}, {"include_journal_state": 1},
                      {"include_copied_attachments": None}, {"include_url_references": "no"},
                      {"include_edit_history": 0}, {"transaction": "absent"},
                      {"source_funds_case": "absent"}])
        self.change(EVIDENCE)
        self.compare([{"source_funds_case": "tampered"}, {"transaction": "nope"}])
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare([..., {}, {"transaction": "h"}])

    def test_whole_book_and_option_variants_match_wire_bytes(self):
        self.change(EVIDENCE)
        responses = self.compare([..., None, {}, {"unknown": True},
                                  {"include_journal_state": False}, {"include_review_state": False},
                                  {"include_edit_history": True},
                                  {"include_journal_state": False, "include_review_state": False,
                                   "include_edit_history": True}])
        summary = responses[1]["data"]["summary"]
        self.assertIn("filed_report_snapshot_count", summary)
        self.assertTrue(responses[1]["data"]["transactions"])

    def test_bounded_scopes_match_wire_bytes(self):
        self.change(EVIDENCE)
        self.compare([{"transaction": " h "}, {"transaction": "i"}, {"transactions": ["i", "h", "h"]},
                      {"transactions": ["j"]}, {"transactions": ["k", " l "]},
                      {"source_funds_case": "case"}, {"source_funds_case": " case ",
                                                      "include_edit_history": True},
                      {"transaction": "h", "include_review_state": False}])
        self.change("UPDATE profiles SET last_processed_input_version=0,journal_input_version=0 WHERE id='p';"
                    "DELETE FROM journal_quantity_issues WHERE issue_id <> 'q1';")
        self.compare([{}, {"transaction": "h"}])

    def test_sqlcipher_evidence_summary_matches_wire_bytes(self):
        self.encrypt("synthetic evidence key")
        self.change(EVIDENCE)
        self.compare([{}, {"transactions": ["h", "i"]}])

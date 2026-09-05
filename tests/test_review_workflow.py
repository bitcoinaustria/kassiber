"""Portable mixed reviews use the real custody engine and atomic domain writes."""
import copy
import json
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

from kassiber.cli.handlers import _metadata_hooks
from kassiber.core import custody_journal, review_workflow as review
from kassiber.db import SCHEMA, open_db
from kassiber.errors import AppError
from kassiber.secrets import sqlcipher


class ReviewWorkflowTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.conn = open_db(self.tmp.name)
        self.addCleanup(self.conn.close)
        self.seed(self.conn)
        self.hooks = review.ReviewHooks(metadata=_metadata_hooks())
        self.profile = self.conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()

    @staticmethod
    def seed(conn):
        conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','W','2020')")
        conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','w','P','2020')")
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wallet','w','p','Wallet','manual','2020')")
        for tx in ('a','b'):
            conn.execute(
                "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fee,kind,raw_json,created_at) "
                "VALUES(?,'w','p','wallet',?,'2020-01-01T00:00:00Z','inbound','BTC',100000000000,0,'buy','{}','2020')", (tx, tx),
            )
        conn.commit()

    def operations(self):
        return [
            {'type':'price_override','transaction_id':'a','fiat_rate':'20000','reason':'Invoice reviewed'},
            {'type':'exclude','transaction_id':'b','reason':'Duplicate source reviewed'},
        ]

    def plan(self):
        return review.plan_review(self.conn, self.profile, operations=self.operations(),
                                  expected_input_version=0, hooks=self.hooks)

    def test_mixed_review_is_read_only_then_verified_once_with_history(self):
        before = self.conn.total_changes
        artifact = self.plan()
        self.assertEqual(self.conn.total_changes, before)
        self.assertEqual(self.conn.execute('SELECT count(*) FROM transaction_edit_events').fetchone()[0], 0)
        self.assertEqual(artifact['before']['quarantine_count'], 2)
        self.assertEqual(artifact['after']['quarantine_count'], 0)
        self.assertTrue(artifact['after']['report_ready'])
        self.assertEqual(review.validate_review(self.conn,self.profile,artifact=artifact,hooks=self.hooks),artifact)
        receipt = review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key='review-1',hooks=self.hooks,authored_source='ai_tool')
        self.assertEqual(receipt['status'],'verified')
        self.assertEqual(receipt['result_input_version'],2)
        self.assertEqual(self.conn.execute('SELECT count(*) FROM transaction_edit_events').fetchone()[0],2)
        holding=self.conn.execute('SELECT quantity,cost_basis FROM journal_wallet_holdings').fetchone()
        self.assertEqual(holding['quantity'],100000000000)
        self.assertEqual(holding['cost_basis'],20000)
        self.assertEqual(review.get_receipt(self.conn,self.profile,receipt_id=receipt['id']),receipt)
        self.assertEqual(review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key='review-1',hooks=self.hooks),receipt)
        self.assertEqual(self.conn.execute('SELECT count(*) FROM transaction_edit_events').fetchone()[0],2)

    def test_stale_tampered_and_idempotency_conflict_fail_closed(self):
        artifact = self.plan()
        tampered = copy.deepcopy(artifact)
        tampered['operations'][0]['fiat_rate']='1'
        with self.assertRaises(AppError) as error:
            review.apply_review(self.conn,self.profile,artifact=tampered,idempotency_key='x',hooks=self.hooks)
        self.assertEqual(error.exception.code,'review_artifact_invalid')
        self.conn.execute('UPDATE profiles SET journal_input_version=1')
        self.conn.commit()
        with self.assertRaises(AppError) as error:
            review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key='x',hooks=self.hooks)
        self.assertEqual(error.exception.code,'custody_review_plan_stale')
        self.conn.execute('UPDATE profiles SET journal_input_version=0')
        self.conn.commit()
        other=review.plan_review(self.conn,self.profile,operations=[self.operations()[0]],expected_input_version=0,hooks=self.hooks)
        review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key='x',hooks=self.hooks)
        with self.assertRaises(AppError) as error:
            review.apply_review(self.conn,self.profile,artifact=other,idempotency_key='x',hooks=self.hooks)
        self.assertEqual(error.exception.code,'review_idempotency_conflict')

    def test_apply_failure_rolls_back_all_domain_changes_and_projection(self):
        artifact=self.plan()
        with patch.object(custody_journal,'store_ledger_state',side_effect=RuntimeError('injected storage failure')):
            with self.assertRaisesRegex(RuntimeError,'storage failure'):
                review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key='x',hooks=self.hooks)
        self.assertEqual(self.conn.execute('SELECT count(*) FROM transaction_edit_events').fetchone()[0],0)
        self.assertEqual(self.conn.execute('SELECT count(*) FROM review_workflow_receipts').fetchone()[0],0)
        self.assertEqual(self.conn.execute('SELECT sum(excluded) FROM transactions').fetchone()[0],0)
        self.assertEqual(self.conn.execute('SELECT journal_input_version FROM profiles').fetchone()[0],0)

    def test_inspect_current_unprocessed_cases_and_version_bound_pagination(self):
        self.assertEqual(self.conn.execute('SELECT count(*) FROM journal_quarantines').fetchone()[0],0)
        page=review.inspect_cases(self.conn,self.profile,limit=1)
        self.assertEqual(page['cases'][0]['transaction_id'],'a')
        second=review.inspect_cases(self.conn,self.profile,limit=1,cursor=page['next_cursor'])
        self.assertEqual(second['cases'][0]['transaction_id'],'b')
        self.assertIsNone(second['next_cursor'])
        self.conn.execute('UPDATE profiles SET journal_input_version=1')
        self.conn.commit()
        with self.assertRaises(AppError) as error:
            review.inspect_cases(self.conn,self.profile,cursor=page['next_cursor'])
        self.assertEqual(error.exception.code,'review_cursor_stale')

    def test_same_binding_sqlcipher_memory_preview_and_apply(self):
        if not sqlcipher.sqlcipher_available():
            self.skipTest('SQLCipher unavailable')
        encrypted=sqlcipher.open_encrypted(self.tmp.name+'/encrypted.sqlite','review-test-password')
        self.addCleanup(encrypted.close)
        encrypted.row_factory=sqlcipher.get_row_class()
        encrypted.executescript(SCHEMA)
        self.seed(encrypted)
        profile=encrypted.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
        artifact=review.plan_review(encrypted,profile,operations=self.operations(),expected_input_version=0,hooks=self.hooks)
        self.assertEqual(encrypted.execute('SELECT count(*) FROM transaction_edit_events').fetchone()[0],0)
        receipt=review.apply_review(encrypted,profile,artifact=artifact,idempotency_key='cipher',hooks=self.hooks)
        self.assertTrue(receipt['verification']['report_ready'])
        with open(self.tmp.name+'/encrypted.sqlite','rb') as handle:
            self.assertNotEqual(handle.read(16),b'SQLite format 3\x00')

    def test_plan_rejects_active_write_transaction_without_committing_it(self):
        self.conn.execute("UPDATE transactions SET note='uncommitted' WHERE id='a'")
        with self.assertRaises(AppError) as error:
            self.plan()
        self.assertEqual(error.exception.code,'review_transaction_active')
        self.assertTrue(self.conn.in_transaction)
        self.conn.rollback()

    def test_active_read_transaction_supports_read_only_preview(self):
        self.conn.execute('BEGIN')
        self.conn.execute('SELECT count(*) FROM transactions').fetchone()
        artifact=self.plan()
        self.assertEqual(artifact['after']['quarantine_count'],0)
        self.assertTrue(self.conn.in_transaction)
        self.conn.rollback()

    def test_receipt_is_discoverable_after_restart(self):
        receipt=review.apply_review(self.conn,self.profile,artifact=self.plan(),idempotency_key='resume',hooks=self.hooks)
        page=review.inspect_cases(self.conn,self.profile)
        self.assertEqual(page['cases'],[])
        self.assertEqual(page['recent_receipts'][0]['id'],receipt['id'])
        self.assertNotIn('proposed_operations',page['recent_receipts'][0])

    def test_second_metadata_failure_preserves_caller_transaction(self):
        artifact=self.plan()
        self.conn.execute("UPDATE transactions SET note='caller-owned' WHERE id='a'")
        original=review.metadata.transaction_history.append_event
        calls=0
        def fail_second(*args,**kwargs):
            nonlocal calls
            calls+=1
            if calls==2:
                raise RuntimeError('metadata failure')
            return original(*args,**kwargs)
        with patch.object(review.metadata.transaction_history,'append_event',side_effect=fail_second):
            with self.assertRaisesRegex(RuntimeError,'metadata failure'):
                review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key='fail',hooks=self.hooks,commit=False)
        self.assertTrue(self.conn.in_transaction)
        self.assertEqual(self.conn.execute("SELECT note FROM transactions WHERE id='a'").fetchone()[0],'caller-owned')
        self.assertEqual(self.conn.execute('SELECT count(*) FROM transaction_edit_events').fetchone()[0],0)
        self.conn.rollback()

    def test_component_and_price_apply_together_with_real_carried_basis(self):
        self.conn.execute("UPDATE transactions SET direction='outbound',kind='send',occurred_at='2021-01-01T00:00:00Z',fiat_rate=30000,fiat_rate_exact='30000',raw_json=? WHERE id='b'",(json.dumps({'privacy_boundary':'coinjoin'}),))
        self.conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('target','w','p','Target','manual','2020')")
        self.conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fee,kind,fiat_rate,fiat_rate_exact,raw_json,created_at) VALUES('c','w','p','target','c','2022-01-01T00:00:00Z','inbound','BTC',100000000000,0,'receive',40000,'40000',?,'2020')",(json.dumps({'privacy_boundary':'coinjoin'}),))
        self.conn.commit()
        component={'type':'custody_component','request':{'action':'create','activate':True,'components':[{
            'component_type':'manual_bridge','evidence_kind':'manual_claim','evidence_grade':'reviewed',
            'change_reason':'Both custody endpoints reviewed',
            'legs':[{'role':'source','transaction':'b','amount_msat':100000000000},
                    {'role':'destination','transaction':'c','amount_msat':100000000000}],
        }]}}
        artifact=review.plan_review(self.conn,self.profile,operations=[self.operations()[0],component],expected_input_version=0,hooks=self.hooks)
        self.assertTrue(artifact['after']['report_ready'])
        receipt=review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key='component',hooks=self.hooks,authored_source='ai_tool')
        holding=self.conn.execute("SELECT quantity,cost_basis FROM journal_wallet_holdings WHERE wallet_id='target'").fetchone()
        self.assertEqual(holding['quantity'],100000000000)
        self.assertEqual(holding['cost_basis'],20000)
        self.assertTrue(receipt['verification']['report_ready'])
        self.assertEqual(self.conn.execute("SELECT count(*) FROM custody_components WHERE state='active'").fetchone()[0],1)

    def test_audit_receipt_scope_is_hash_only_and_explicit(self):
        receipt=review.apply_review(self.conn,self.profile,artifact=self.plan(),idempotency_key='audit',hooks=self.hooks)
        matching=review.audit_receipt_summary(self.conn,'p',transaction_ids=['a'])
        self.assertEqual(matching['count'],1)
        self.assertEqual(matching['records'][0]['id'],receipt['id'])
        self.assertNotIn('Invoice reviewed',json.dumps(matching))
        self.assertNotIn('wallet_holdings',json.dumps(matching))
        self.assertEqual(review.audit_receipt_summary(self.conn,'p',transaction_ids=['unrelated'])['count'],0)
        self.assertEqual(review.audit_receipt_summary(self.conn,'p',transaction_ids=[])['count'],0)

    def test_commit_failure_preserves_original_error_and_rolls_back(self):
        class CommitFailure:
            def __init__(self,conn):
                self.conn=conn
            def __getattr__(self,name):
                return getattr(self.conn,name)
            def commit(self):
                raise sqlite3.OperationalError('injected commit failure')
        artifact=self.plan()
        with self.assertRaisesRegex(sqlite3.OperationalError,'injected commit failure'):
            review.apply_review(CommitFailure(self.conn),self.profile,artifact=artifact,idempotency_key='commit-fail',hooks=self.hooks)
        self.assertFalse(self.conn.in_transaction)
        self.assertEqual(self.conn.execute('SELECT count(*) FROM review_workflow_receipts').fetchone()[0],0)
        self.assertEqual(self.conn.execute('SELECT count(*) FROM transaction_edit_events').fetchone()[0],0)
        self.assertEqual(self.conn.execute('SELECT sum(excluded) FROM transactions').fetchone()[0],0)

    def test_profile_reset_deletes_receipts(self):
        from kassiber.core import maintenance
        review.apply_review(self.conn,self.profile,artifact=self.plan(),idempotency_key='reset',hooks=self.hooks)
        self.conn.execute("INSERT INTO settings(key,value) VALUES('context_workspace','w')")
        self.conn.execute("INSERT INTO settings(key,value) VALUES('context_profile','p')")
        self.conn.commit()
        result=maintenance.reset_current_profile_data(self.conn,self.tmp.name)
        self.assertEqual(self.conn.execute('SELECT count(*) FROM review_workflow_receipts').fetchone()[0],0)
        self.assertEqual(result['removed']['review_workflow_receipts'],1)


    def test_price_exponent_amplification_is_rejected_before_formatting(self):
        for price in ("1e999999999", "1e-999999999", "9" * 513):
            with self.subTest(price=price[:30]):
                operation = {**self.operations()[0], "fiat_rate": price}
                with patch.object(review, "format", create=True,
                                  side_effect=AssertionError("must reject before expansion")) as formatter:
                    with self.assertRaises(AppError) as error:
                        review._operations([operation])
                    self.assertEqual(error.exception.code, "validation")
                    formatter.assert_not_called()
        ordinary = {**self.operations()[0], "fiat_rate": "1e-8"}
        normalized = review._operations([ordinary])
        self.assertEqual(normalized[0]["fiat_rate"], "0.00000001")
        self.assertEqual(review._operations(normalized), normalized)

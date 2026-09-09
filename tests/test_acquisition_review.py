"""Acquisition declarations use the real canonical preview and journal engine."""
import copy
import unittest
from decimal import Decimal
from unittest.mock import patch

from kassiber.core import custody_journal, metadata, review_workflow as review
from kassiber.db import open_db
from kassiber.errors import AppError
from tests import test_review_workflow as fixtures


class AcquisitionReviewTest(unittest.TestCase):
    setUp = fixtures.ReviewWorkflowTest.setUp
    seed = staticmethod(fixtures.ReviewWorkflowTest.seed)
    def priced(self, country="generic"):
        self.conn.execute("UPDATE profiles SET tax_country=?,fiat_currency='EUR'", (country,))
        self.conn.execute("UPDATE transactions SET occurred_at='2024-01-01T00:00:00Z',fiat_rate=20000,fiat_value=20000")
        self.conn.commit()
        self.profile = self.conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()

    def acquisition_plan(self, kind="income", **extra):
        version = self.conn.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0]
        return review.plan_review(self.conn, self.profile,
            operations=[{"type":"kind_override", "transaction_id":"a", "kind":kind, "reason":"Source reviewed", **extra}],
            expected_input_version=version, hooks=self.hooks)

    def test_supported_inbound_preview_exact_effect_apply_and_history_undo(self):
        self.priced()
        artifact = self.acquisition_plan()
        self.assertEqual(artifact["before"]["quarantine_count"], 0)
        self.assertEqual(Decimal(artifact["before"]["accounting_totals"][0]["acquisition_basis"]), Decimal("40000"))
        self.assertEqual(Decimal(artifact["after"]["accounting_totals"][0]["income"]), 20000)
        self.assertEqual(artifact["before"]["accounting_totals"][0]["income"], "0")
        self.assertIsNone(self.conn.execute("SELECT kind_override FROM transactions WHERE id='a'").fetchone()[0])
        receipt = review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key="kind",hooks=self.hooks)
        self.assertEqual(receipt["status"],"verified")
        row = self.conn.execute("SELECT kind,kind_override,note FROM transactions WHERE id='a'").fetchone()
        self.assertEqual(tuple(row),("buy","income",None))
        self.assertEqual(review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key="kind",hooks=self.hooks),receipt)
        metadata.revert_transaction_edit(self.conn,'w','p','a',self.hooks.metadata,
                                        event_id=receipt["operations"][0]["result"]["history_event_id"])
        state = custody_journal.build_ledger_state(self.conn,self.profile)
        self.assertFalse(any(e["entry_type"] == "income" for e in state["entries"]))
        self.assertEqual(self.conn.execute("SELECT count(*) FROM transaction_edit_events").fetchone()[0],2)

    def test_clear_declaration_and_unsupported_valuation(self):
        self.priced()
        with self.assertRaises(AppError) as error:
            self.acquisition_plan(valuation_mode="zero_cost")
        self.assertEqual(error.exception.code,"acquisition_valuation_unsupported")
        metadata.update_transaction_metadata(self.conn,'w','p','a',self.hooks.metadata,kind='income',kind_set=True)
        artifact = self.acquisition_plan(None)
        self.assertEqual(artifact["after"]["accounting_totals"][0]["income"], "0")

    def test_book_identity_and_same_economics_policy_change_reject_apply(self):
        self.priced()
        artifact = self.acquisition_plan()
        other = open_db(self.tmp.name + '/other')
        self.addCleanup(other.close)
        self.seed(other)
        other.execute("UPDATE profiles SET fiat_currency='EUR'")
        other.execute("UPDATE transactions SET occurred_at='2024-01-01T00:00:00Z',fiat_rate=20000,fiat_value=20000")
        other.commit()
        with self.assertRaises(AppError) as error:
            review.apply_review(other,self.profile,artifact=artifact,idempotency_key="kind",hooks=self.hooks)
        self.assertEqual(error.exception.code,"review_plan_stale")
        self.conn.execute("UPDATE profiles SET gains_algorithm='LIFO'")
        self.conn.commit()
        with self.assertRaises(AppError) as error:
            review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key="kind",hooks=self.hooks)
        self.assertEqual(error.exception.code,"review_plan_stale")

    def test_acquisition_failure_rolls_back_kind_history_and_journal(self):
        self.priced()
        artifact = self.acquisition_plan()
        with patch.object(custody_journal,'store_ledger_state',side_effect=RuntimeError('failed')):
            with self.assertRaises(RuntimeError):
                review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key="kind",hooks=self.hooks)
        self.assertIsNone(self.conn.execute("SELECT kind_override FROM transactions WHERE id='a'").fetchone()[0])
        self.assertEqual(self.conn.execute("SELECT count(*) FROM transaction_edit_events").fetchone()[0],0)

    def test_austrian_airdrop_and_hardfork_gate_prevents_later_contaminated_gains(self):
        self.priced('at')
        self.conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fee,kind,raw_json,created_at,fiat_rate,fiat_value) VALUES('sale','w','p','wallet','sale','2024-02-01T00:00:00Z','outbound','BTC',50000000000,0,'sell','{}','2024',30000,15000)")
        self.conn.commit()
        for kind in ('airdrop','hardfork'):
            with self.subTest(kind=kind):
                artifact = self.acquisition_plan(kind)
                reasons = {q['transaction_id']:q['reason'] for q in artifact['after']['quarantines']}
                self.assertEqual(reasons['a'],'acquisition_valuation_unsupported')
                self.assertIn('sale',reasons)
                self.assertFalse(artifact['after']['report_ready'])
                self.assertEqual(artifact['after']['accounting_totals'][0]['gain_loss'],'0')
        # The gate also applies to the importer's original kind with no review artifact.
        self.conn.execute("UPDATE transactions SET kind='hardfork' WHERE id='a'")
        self.conn.commit()
        state = custody_journal.build_ledger_state(self.conn,self.profile)
        self.assertIn('acquisition_valuation_unsupported', [q['reason'] for q in state['quarantines']])

    def test_austrian_staking_and_wages_keep_supported_semantics(self):
        self.priced('at')
        for kind, income in [('staking',20000),('wages',0),('buy',0)]:
            with self.subTest(kind=kind):
                artifact = self.acquisition_plan(kind)
                self.assertTrue(artifact['after']['report_ready'])
                self.assertEqual(Decimal(artifact['after']['accounting_totals'][0]['income']),income)

    def test_forged_effects_and_outbound_declarations_rejected(self):
        self.priced()
        artifact = self.acquisition_plan()
        forged = copy.deepcopy(artifact)
        forged['after']['accounting_totals'][0]['income']='1'
        with self.assertRaises(AppError):
            review.apply_review(self.conn,self.profile,artifact=forged,idempotency_key='x',hooks=self.hooks)
        for kind in ('sell', [], {}, 1):
            with self.subTest(kind=kind), self.assertRaises(AppError):
                self.acquisition_plan(kind)

    def test_refresh_retains_authored_kind_and_raw_source(self):
        from kassiber.cli.handlers import _import_coordinator_hooks
        from kassiber.core.imports import import_records_into_wallet
        self.priced()
        wallet = self.conn.execute("SELECT * FROM wallets WHERE id='wallet'").fetchone()
        record = {"txid":"imported", "occurred_at":"2024-03-01T00:00:00Z", "direction":"inbound", "asset":"BTC", "amount":"0.1", "fee":"0", "fiat_rate":"20000", "kind":"buy", "raw_json":{"source":"original"}}
        hooks = _import_coordinator_hooks()
        import_records_into_wallet(self.conn,self.profile,wallet,[record],"generic_csv",hooks)
        tx = self.conn.execute("SELECT * FROM transactions WHERE external_id='imported'").fetchone()
        version = self.conn.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0]
        artifact = review.plan_review(self.conn,self.profile,operations=[{"type":"kind_override","transaction_id":tx['id'],"kind":"income","reason":"Reviewed"}],expected_input_version=version,hooks=self.hooks)
        review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key='import',hooks=self.hooks)
        import_records_into_wallet(self.conn,self.profile,wallet,[record],"generic_csv",hooks)
        refreshed = self.conn.execute("SELECT kind,kind_override,raw_json FROM transactions WHERE id=?",(tx['id'],)).fetchone()
        self.assertEqual(tuple(refreshed),('buy','income',tx['raw_json']))

    def test_semantic_upgrade_invalidates_existing_austrian_projection_once(self):
        from kassiber.core.acquisition_review import VALUATION_MIGRATION, migrate_valuation_support
        self.priced('at')
        self.conn.execute("UPDATE transactions SET kind='airdrop' WHERE id='a'")
        self.conn.execute("UPDATE profiles SET last_processed_at='2024-03-01T00:00:00Z'")
        self.conn.execute("DELETE FROM schema_migration_audits WHERE migration_name=?",(VALUATION_MIGRATION,))
        self.conn.commit()
        self.assertTrue(migrate_valuation_support(self.conn))
        row = self.conn.execute("SELECT journal_input_version,last_processed_at FROM profiles WHERE id='p'").fetchone()
        self.assertEqual(tuple(row),(1,None))
        self.assertFalse(migrate_valuation_support(self.conn))
        self.assertEqual(self.conn.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0],1)

    def test_changed_economic_input_rejects_stale_preview_even_without_version_bump(self):
        self.priced()
        artifact = self.acquisition_plan()
        self.conn.execute("UPDATE transactions SET fiat_rate=21000,fiat_value=21000 WHERE id='b'")
        self.conn.commit()
        with self.assertRaises(AppError) as error:
            review.apply_review(self.conn,self.profile,artifact=artifact,idempotency_key='economic',hooks=self.hooks)
        self.assertEqual(error.exception.code,'review_plan_stale')

    def test_local_financial_review_cannot_be_requested_through_ai(self):
        from kassiber.daemon import _validate_ai_review_operations
        with self.assertRaises(AppError) as error:
            _validate_ai_review_operations([{'type':'kind_override','transaction_id':'a','kind':'income','reason':'Review'}])
        self.assertEqual(error.exception.code,'interaction_required')

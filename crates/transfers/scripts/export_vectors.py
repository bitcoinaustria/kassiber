#!/usr/bin/env python3
"""Generate synthetic stored-service oracle rows with locked Python 3.11."""
import json
from pathlib import Path
import tempfile
import sys
import uuid
from contextlib import ExitStack
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.db import open_db
from kassiber.cli import handlers
from kassiber.core import custody_gap_reviews, custody_review_terms, review_workflow


def quote(v):
    if v is None:
        return 'NULL'
    if isinstance(v, str):
        return "'" + v.replace("'", "''") + "'"
    return str(v)


def main():
    with tempfile.TemporaryDirectory(prefix="kassiber-transfer-oracle-") as root:
        c = open_db(root)
        c.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025-01-01T00:00:00Z')")
        c.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','at','2025-01-01T00:00:00Z')")
        for wallet in ('source', 'target'):
            c.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES(?,'w','p',?,'custom','2025-01-01T00:00:00Z')", (wallet, wallet))
        for i, (wallet, direction, amount) in enumerate((('source','outbound',100000000001),('target','inbound',70000000001),('target','inbound',30000000000),('source','outbound',9007199254740993))):
            c.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,kind,created_at) VALUES(?,'w','p',?,?,?,'BTC',?,?,0,'2025-02-01T00:00:00Z','deposit','2025-01-01T00:00:00Z')", (f'tx{i}',wallet,f'fp{i}',f'external{i}',direction,amount))
        c.execute("INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p')")
        c.commit()
        handlers.create_transaction_pair(c,'w','p','tx0','tx1')
        handlers.create_transaction_pair(c,'w','p','tx0','tx2')
        handlers.create_direct_swap_payout(c,'w','p','tx3',payout_asset='LBTC',payout_amount='90071.99254740993',payout_fiat_value='123.456789',policy='carrying-value',notes='literal € 🚀')
        for name, predicate in [('z',{}),('a',{'out_asset':'BTC','max_fee_pct':'0.000_001','min_confidence':'exact'})]:
            with patch.object(handlers,'now_iso',return_value='2025-06-01T00:00:00Z'):
                handlers.create_transfer_rule(c,'w','p',name=name,predicate=predicate)
        c.execute("INSERT INTO swap_matching_rules(id,workspace_id,profile_id,predicate_json,kind,policy,enabled,created_at,updated_at) VALUES('invalid','w','p','{bad','bad','bad',1,'2025-06-01T00:00:00Z','2025-06-01T00:00:00Z')")
        for revision, action, event in [(1,'dismissed','review_decision'),(2,'resolved','bridge_reopened'),(3,'resolved','review_decision')]:
            c.execute("INSERT INTO custody_gap_reviews(id,workspace_id,profile_id,gap_id,candidate_fingerprint,revision,action,event_kind,authored_source,reason,snapshot_json,created_at) VALUES(?,'w','p','gap','fingerprint',?,?,?,'cli','reviewed',?,'2025-05-01T00:00:00Z')", (f'review{revision}',revision,action,event,json.dumps({'retained_msat':9007199254740993,'residual_msat':4,'residual_classification':{'classification':'retained'},'source_transaction_ids':['never-disclose']})))
        from kassiber.core.custody_gap_reviews import custody_gap_review_transaction_id
        for revision in (1,2,3):
            for role,txid in [('source','tx0'),('return','tx1')]:
                review_id=f'review{revision}'
                c.execute("INSERT INTO custody_gap_review_transactions(id,review_id,workspace_id,profile_id,role,transaction_id,created_at) VALUES(?,?,'w','p',?,?,'2025-05-01T00:00:00Z')",(custody_gap_review_transaction_id(review_id,role,txid),review_id,role,txid))
        from kassiber.db import backfill_custody_gap_review_relation_set
        for revision in (1,2,3):
            backfill_custody_gap_review_relation_set(c,review_id=f'review{revision}',workspace_id='w',profile_id='p',created_at='2025-05-01T00:00:00Z',expected_source_count=1,expected_return_count=1)
        receipt = {'id':'receipt','status':'verified','verification':{'report_ready':False},'unicode':'€ 🚀','result_input_version':7}
        c.execute("INSERT INTO review_workflow_receipts(id,workspace_id,profile_id,idempotency_key,artifact_digest,receipt_json,created_at) VALUES('receipt','w','p','key',?,?, '2025-01-01T00:00:00Z')", ('f'*64,json.dumps(receipt)))
        c.commit()
        # Snapshot authored rows, not the database implementation's schema text.
        tables = ('workspaces','profiles','wallets','transactions','custody_components','custody_component_legs','custody_component_allocations','custody_component_economic_terms','custody_component_transaction_memberships','custody_component_evidence_commitments','custody_gap_reviews','custody_gap_review_transactions','custody_gap_review_relation_sets','swap_matching_rules','review_workflow_receipts')
        statements=[]
        for table in tables:
            for row in c.execute(f'SELECT * FROM {table}'):
                statements.append(f'INSERT INTO {table} ({",".join(row.keys())}) VALUES({",".join(quote(v) for v in row)});')
        cases=[]
        for operation,args,run in [
            ('transfers.list',{},lambda:handlers.list_transaction_pairs(c,'w','p')),
            ('transfers.payouts.list',{},lambda:handlers.list_direct_swap_payouts(c,'w','p')),
            ('transfers.rules.list',{},lambda:handlers.list_transfer_rules(c,'w','p')),
            ('transfers.gaps.history',{'gap_id':'gap','limit':2},lambda:custody_gap_reviews.list_review_history(c,'p','gap',limit=2)),
            ('review.receipt',{'receipt_id':'receipt'},lambda:review_workflow.get_receipt(c,c.execute("SELECT * FROM profiles WHERE id='p'").fetchone(),receipt_id='receipt')),
        ]:
            cases.append({'operation':operation,'args':args,'expected':run()})
        output={'sql':statements,'reads':cases}
        path=ROOT/'crates/transfers/tests/python.json'
        path.write_text(json.dumps(output,ensure_ascii=True,indent=2)+'\n')
        c.close()

if __name__=='__main__':
    from kassiber.core import custody_authored_migration, custody_components
    import itertools
    counter=itertools.count(1)
    with ExitStack() as stack:
        stack.enter_context(patch.object(uuid,'uuid4',side_effect=lambda:uuid.UUID(int=next(counter))))
        for module in (handlers,custody_review_terms,custody_authored_migration):
            stack.enter_context(patch.object(module,'now_iso',return_value='2025-06-01T00:00:00Z'))
        stack.enter_context(patch.object(custody_components,'_now_iso',return_value='2025-06-01T00:00:00Z'))
        main()

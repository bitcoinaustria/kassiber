"""Freeze the unchanged Python batch validator, including authoritative lineage."""
import argparse
import json
import sqlite3
import tempfile
from copy import deepcopy
from unittest.mock import patch

from kassiber import daemon
from kassiber.core import source_funds
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION, open_db
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_cases import dump, STAMP
from crates.reports.scripts.gen_source_funds_links import BASE_SQL, ROOT

DESTINATION = ROOT / 'crates/reports/tests/data/source_funds_batch.json'
BASE_SQL += """
DELETE FROM journal_custody_decisions;
DELETE FROM journal_custody_economic_relations;
UPDATE source_funds_links SET state='rejected';
UPDATE source_funds_links SET state='suggested', method='custody_component',confidence='exact',
allocation_amount=100001,from_allocation_amount=100001,uses_chain_observation=0 WHERE id='l-z';
INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,target_start_msat,target_end_msat,source_wallet_id,target_wallet_id,source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,basis_state,reason,occurred_at,target_occurred_at,created_at) VALUES
(printf('%064d',1),'ws','p','in','out',printf('%064d',2),0,100001,printf('%064d',3),0,100001,'cold','cold','main','main','bitcoin','bitcoin','BTC','BTC','internal_reviewed','eligible','reviewed','2024-01-01T00:00:00Z','2024-12-31T23:30:00Z','fixed');
UPDATE profiles SET last_processed_at='current',last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id='p' AND excluded=0),last_processed_input_version=journal_input_version WHERE id='p';UPDATE profiles SET journal_semantics_version=""" + str(JOURNAL_CALCULATION_SEMANTICS_VERSION) + """ WHERE id='p';
"""

RESTAMP = " UPDATE profiles SET journal_semantics_version=" + str(JOURNAL_CALCULATION_SEMANTICS_VERSION) + " WHERE id='p';"


def outcome(conn, case):
    calls = []
    def now():
        calls.append('now')
        return STAMP
    with patch.object(source_funds, '_now', now):
        try:
            command, args = case['command'], case['args']
            if command.startswith('ui.'):
                data = daemon._ui_source_funds_payload_from_conn(conn, command, daemon._coerce_args_dict(None, args))
            else:
                if args.get('dry_run'):
                    conn.execute('SAVEPOINT dry_run')
                data = source_funds.bulk_review_suggestions(conn, args.get('workspace'), args.get('profile'), daemon._source_funds_hooks(), target_transaction_ref=args['target_transaction'], commit=not args.get('dry_run'))
                if args.get('dry_run'):
                    conn.execute('ROLLBACK TO dry_run')
                    conn.execute('RELEASE dry_run')
                    data['dry_run'] = True
            result = dict(ok=data)
            envelope = build_envelope(command, data)
        except Exception as error:
            conn.rollback()
            if isinstance(error, AppError):
                fields = dict(code=error.code, message=str(error), hint=error.hint, details=error.details, retryable=error.retryable)
            else:
                fields = dict(code='internal_error', message=str(error), hint=None, details=None, retryable=False)
            result = dict(error=fields)
            envelope = build_error_envelope(**fields)
    result['calls'] = calls
    result['cli'] = json.dumps(envelope, indent=2)+'\n'
    result['daemon'] = json.dumps(envelope, separators=(',', ':'))+'\n'
    return result


def structural_sql(template):
    from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
    quote = lambda v: template.execute('SELECT quote(?)', (v,)).fetchone()[0]
    script = '0014'+'aa'*20
    parent, spend = 'a1'*32, 'b2'*32
    result = ''
    for identity, txid, amount, vin in [('in',parent,200000,[]),('out',spend,199000,[dict(txid=parent,vout=0,prevout=dict(scriptpubkey=script,value=200))])]:
        row = dict(template.execute('SELECT * FROM transactions WHERE id=?',(identity,)).fetchone())
        raw = json.dumps(dict(txid=txid,chain='bitcoin',network='main',observer_owned_scripts=[script],vin=vin,vout=[dict(n=0,scriptpubkey=script,value=amount//1000)]),sort_keys=True)
        row.update(raw_json=raw, external_id=txid,external_id_kind='txid',amount=amount,fee=1000 if vin else 0,amount_includes_fee=0)
        fields = ['raw_json','external_id','external_id_kind','amount','fee','amount_includes_fee']
        result += 'UPDATE transactions SET '+','.join(f'{k}={quote(row[k])}' for k in fields)+f' WHERE id={quote(identity)};\n'
        provenance=dict(transaction_id=identity,workspace_id='ws',profile_id='p',wallet_id='cold',authority_version=1,observer_ids_json='[]',observer_kinds_json='[]',chain='bitcoin',network='main',application_revision='synthetic',graph_hash=canonical_graph_hash(raw),quantity_hash=canonical_observed_quantity_hash(row),fee_attribution='exact',observed_at=STAMP,updated_at=STAMP)
        result += 'INSERT INTO chain_observation_provenance('+','.join(provenance)+') VALUES('+','.join(map(quote,provenance.values()))+');\n'
    result += "UPDATE wallets SET config_json='{}' WHERE id='cold'; UPDATE source_funds_links SET method='utxo_spend',allocation_amount=199000,from_allocation_amount=200000 WHERE id='l-z';"
    return result


def cases(template):
    result = []
    def add(name, sql='', desktop=True, **args):
        result.append(dict(name=name,command='ui.source_funds.links.bulk_review' if desktop else 'source-funds.links.bulk-review',args=dict(target_transaction='out',**args),sql=sql))
    add('current-custody')
    add('cli-current', desktop=False)
    add('cli-dry-run', desktop=False,dry_run=True)
    for name, sql in {
        'stale-version':"UPDATE profiles SET last_processed_input_version=journal_input_version-1 WHERE id='p';",
        'stale-count':"UPDATE profiles SET last_processed_tx_count=0 WHERE id='p';",
        'not-processed':"UPDATE profiles SET last_processed_at=NULL WHERE id='p';",
        # Moving a processing marker also clears the semantics stamp; restamp
        # so the marker alone decides.
        'stale-version-stamped':"UPDATE profiles SET last_processed_input_version=journal_input_version-1 WHERE id='p';"
            + RESTAMP,
        'not-processed-stamped':"UPDATE profiles SET last_processed_at=NULL WHERE id='p';" + RESTAMP,
        'no-projection':"DELETE FROM journal_custody_decisions;",
        'wrong-direction':"UPDATE journal_custody_decisions SET target_transaction_id='earn';",
        'zero-projection':"PRAGMA ignore_check_constraints=ON; UPDATE journal_custody_decisions SET target_end_msat=0,source_end_msat=0; PRAGMA ignore_check_constraints=OFF;",
        'multiple-slices':"UPDATE journal_custody_decisions SET source_end_msat=50000,target_end_msat=50000; INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,target_start_msat,target_end_msat,source_wallet_id,target_wallet_id,source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,basis_state,reason,component_id,occurred_at,target_occurred_at,created_at) SELECT printf('%064d',9),workspace_id,profile_id,source_transaction_id,target_transaction_id,source_observation_hash,50000,100001,target_observation_hash,50000,100001,source_wallet_id,target_wallet_id,source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,basis_state,reason,component_id,occurred_at,target_occurred_at,created_at FROM journal_custody_decisions;",
        'excluded-parent':"UPDATE transactions SET excluded=1 WHERE id='in'; UPDATE profiles SET last_processed_tx_count=last_processed_tx_count-1 WHERE id='p';",
        'future-parent':"UPDATE transactions SET occurred_at='2027-01-01' WHERE id='in';",
        'self-loop':"UPDATE source_funds_links SET from_transaction_id='out' WHERE id='l-z';",
        'rejected-pair':"UPDATE source_funds_links SET from_source_id=NULL,from_transaction_id='in',to_transaction_id='out' WHERE id='l-old';",
        'empty-asset-normalization':"UPDATE transactions SET asset='' WHERE id IN ('in','out'); UPDATE source_funds_links SET asset='',from_asset=NULL WHERE id='l-z';",
        'malformed-amount':"UPDATE source_funds_links SET allocation_amount='bad' WHERE id='l-z';",
        'stale-malformed-amount':"UPDATE profiles SET last_processed_at=NULL WHERE id='p'; UPDATE source_funds_links SET allocation_amount='bad' WHERE id='l-z';",
        'mismatch-malformed-gross':"UPDATE source_funds_links SET allocation_amount=1,from_allocation_amount='bad' WHERE id='l-z';",
        'allocation-bound':"UPDATE transactions SET amount=1 WHERE id='out';",
        'gross-bound':"UPDATE transactions SET amount=1 WHERE id='in';",
        'cross-book':"UPDATE source_funds_links SET profile_id='other' WHERE id='l-z';",
        'cycle':"UPDATE source_funds_links SET from_source_id=NULL,from_transaction_id='out',to_transaction_id='in',state='suggested' WHERE id='l-a';",
        'projection-refusal':"CREATE TRIGGER refuse_projection AFTER UPDATE ON source_funds_links WHEN NEW.id='l-z' BEGIN UPDATE source_funds_links SET allocation_amount='bad' WHERE id='l-z'; END;",
    }.items():
        if name != 'projection-refusal':
            add(name,sql)
    for key, values in dict(method=['manual','provider_id','utxo_spend','custody_component'],confidence=['weak','unknown','strong','exact'],allocation_amount=[None,0,100000,100002],from_allocation_amount=[None,0,100001,100003],asset=['LBTC','BTC'],from_asset=['LBTC',None,'BTC'],uses_chain_observation=[0,1],state=['reviewed','rejected']).items():
        for i,v in enumerate(values):
            quoted=template.execute('SELECT quote(?)',(v,)).fetchone()[0]
            add(f'candidate-{key}-{i}',f"UPDATE source_funds_links SET {key}={quoted} WHERE id='l-z';")
    for link_type in source_funds.LINK_TYPES:
        add('custody-type-'+link_type,f"UPDATE journal_custody_decisions SET reason='{link_type}'; UPDATE source_funds_links SET link_type='{link_type}' WHERE id='l-z';")
    for source_rail,target_rail,link_type in [('bitcoin','liquid','peg_in'),('liquid','bitcoin','peg_out'),('bitcoin','lightning','lightning_funding'),('lightning','bitcoin','lightning_close')]:
        add('custody-rail-'+link_type,f"UPDATE journal_custody_decisions SET source_rail='{source_rail}',target_rail='{target_rail}'; UPDATE source_funds_links SET link_type='{link_type}' WHERE id='l-z';")
    add('custody-conversion',"DELETE FROM journal_custody_decisions; INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,source_transaction_id,target_transaction_id,source_asset,target_asset,source_amount_msat,target_amount_msat,basis_state,occurred_at,created_at) VALUES(printf('%064d',10),'ws','p','conversion','in','out','BTC','BTC',100001,100001,'eligible','fixed','fixed'); UPDATE source_funds_links SET link_type='swap' WHERE id='l-z';")
    proof=json.dumps(dict(component_id='component',allocations=[dict(destination_kind='wallet',source_transaction_id='in',target_transaction_id='out',amount_msat=100001,asset='BTC')]),sort_keys=True)
    proof_literal=template.execute('SELECT quote(?)',(proof,)).fetchone()[0]
    reconciliation="DELETE FROM journal_custody_decisions; INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,created_at) VALUES('component','lineage','ws','p',1,'manual_bridge','active','fixed'); UPDATE source_funds_links SET from_source_id=NULL,from_transaction_id='in',to_transaction_id='out',state='reviewed',method='custody_component',link_type='self_transfer',asset='BTC',from_asset='BTC',allocation_amount=100001,from_allocation_amount=100001 WHERE id='l-a'; INSERT INTO journal_custody_reconciliations(profile_id,component_id,input_version,component_revision,proof_json,created_at) SELECT 'p','component',journal_input_version,1,"+proof_literal+",'fixed' FROM profiles WHERE id='p';"
    add('reconciled-reviewed',reconciliation)
    add('reconciled-stale-revision',reconciliation+"UPDATE journal_custody_reconciliations SET component_revision=2;")
    add('reconciled-stale-input',reconciliation+"UPDATE journal_custody_reconciliations SET input_version=input_version-1;")
    add('reconciled-inactive',reconciliation.replace("'manual_bridge','active'","'manual_bridge','draft'"))
    add('reconciled-amount-drift',reconciliation+"UPDATE source_funds_links SET from_allocation_amount=1 WHERE id='l-a';")
    for i, ids in enumerate([None,[],['l-z'],['l-a'],[' l-z '],['missing'],['l-z','l-z'],['l-z']*501,'l-z',[42],[''],[None],{}]):
        add(f'selection-{i}',link_ids=ids)
    for i,target in enumerate(['in','missing','external-out','duplicate',' out ','',None,False,42]):
        for desktop in [False,True]:
            case=dict(name=f'target-{desktop}-{i}',command='ui.source_funds.links.bulk_review' if desktop else 'source-funds.links.bulk-review',args=dict(target_transaction=target),sql='')
            if not desktop and not isinstance(target,str):continue
            result.append(case)
    result.append(dict(name='target-alias',command='ui.source_funds.links.bulk_review',args=dict(target_transaction_ref=' out '),sql=''))
    for i,args in enumerate([None,[],False,42,'wrong']):
        result.append(dict(name=f'args-{i}',command='ui.source_funds.links.bulk_review',args=args,sql=''))
    structural = structural_sql(template)
    for name,sql in {
        'valid':'',
        'unattested':"DELETE FROM chain_observation_provenance;",
        'tampered':"UPDATE transactions SET raw_json='{}' WHERE id='in';",
        'amount-drift':"UPDATE transactions SET amount=amount+1 WHERE id='in';",
        'privacy':"UPDATE transactions SET privacy_boundary='coinjoin' WHERE id='in';",
        'samourai':"UPDATE wallets SET config_json='{\"samourai\":{\"role\":\"child\",\"group_id\":\"group\",\"section\":\"deposit\"}}' WHERE id='cold';",
        'wrong-wallet':"UPDATE transactions SET wallet_id='hot' WHERE id='in';",
        'excluded':"UPDATE transactions SET excluded=1 WHERE id='in';",
        'gross-drift':"UPDATE source_funds_links SET from_allocation_amount=200001 WHERE id='l-z';",
        'malformed-without-derivation':"DELETE FROM chain_observation_provenance; UPDATE source_funds_links SET allocation_amount='bad' WHERE id='l-z';",
        'non-exact':"UPDATE source_funds_links SET confidence='strong' WHERE id='l-z';",
        'inventory-veto':"INSERT INTO wallet_utxos(id,wallet_id,workspace_id,profile_id,chain,network,asset,txid,vout,amount,first_seen_at,last_seen_at,outpoint,confirmation_status) VALUES('u-veto','hot','ws','p','bitcoin','main','BTC','"+'a1'*32+"',0,200000,'fixed','fixed','synthetic:0','confirmed');",
    }.items():add('structural-'+name,structural+sql)
    columns=[r['name'] for r in template.execute('PRAGMA table_info(source_funds_links)')]
    second="INSERT INTO source_funds_links SELECT "+','.join("'l-second'" if c=='id' else "'zz'" if c=='created_at' else c for c in columns)+" FROM source_funds_links WHERE id='l-z';"
    ancestor="UPDATE source_funds_links SET from_source_id=NULL,from_transaction_id='out',to_transaction_id='no-tax',method='custody_component',state='suggested',confidence='exact',link_type='self_transfer',asset='BTC',from_asset='BTC',allocation_amount=100001,from_allocation_amount=100001 WHERE id='l-a'; INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,target_start_msat,target_end_msat,source_wallet_id,target_wallet_id,source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,basis_state,reason,occurred_at,target_occurred_at,created_at) VALUES(printf('%064d',9),'ws','p','out','no-tax',printf('%064d',2),0,100001,printf('%064d',3),0,100001,'cold','hot','main','main','bitcoin','bitcoin','BTC','BTC','internal_reviewed','eligible','reviewed','2024-12-31T23:30:00Z','2025-02-01T00:00:00Z','fixed');"
    for suffix,edit in [('suggested',''),('reviewed',"UPDATE source_funds_links SET state='reviewed' WHERE id='l-a';"),('rejected',"UPDATE source_funds_links SET state='rejected' WHERE id='l-a';"),('hint',"UPDATE source_funds_links SET method='provider_id' WHERE id='l-a';")]:
        add('upstream-'+suffix,ancestor+edit)
        result[-1]['args']['target_transaction']='no-tax'
    add('two-current',second)
    add('two-selected',second,link_ids=['l-second'])
    add('two-dry-run',second,desktop=False,dry_run=True)
    for refusal in ['ABORT','ROLLBACK']:
        add('partial-'+refusal,second+f"CREATE TRIGGER refuse_write BEFORE UPDATE ON source_funds_links WHEN OLD.id='l-second' BEGIN SELECT RAISE({refusal},'synthetic late refusal'); END;")
    for refusal in ['ABORT','ROLLBACK']:
        add('late-'+refusal,f"CREATE TRIGGER refuse_write BEFORE UPDATE ON source_funds_links BEGIN SELECT RAISE({refusal},'synthetic late refusal'); END;")
    add('commit-deferred',"CREATE TABLE commit_failure(id TEXT REFERENCES profiles(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER refuse_commit AFTER UPDATE ON source_funds_links BEGIN INSERT INTO commit_failure VALUES('missing'); END;")
    add('custody-swap-target-asset',"UPDATE journal_custody_decisions SET reason='swap'; UPDATE source_funds_links SET link_type='swap',asset='LBTC' WHERE id='l-z';")
    add('structural-gross-within-parent',structural+"UPDATE source_funds_links SET from_allocation_amount=199999 WHERE id='l-z';")
    return result


def structural_vectors():
    from tests.test_source_funds_assembly import (
        ParentSpendDerivationTests, ChangePassthroughTests, PassthroughWeightTests,
        WalletIsolationTests, OrderAndScaleTests, _index, _authoritative,
    )
    from kassiber.core.source_funds_assembly import derive_parent_spend_pairs
    vectors=[]
    def capture(name, rows, index=None, skip=lambda row:False):
        index=index or {}
        pairs=derive_parent_spend_pairs(rows,index,skip_row=skip)
        projected=deepcopy(rows)
        for original,row in zip(rows,projected,strict=True):
            if skip(original):row['privacy_boundary']='coinjoin'
        vectors.append(dict(name=name,rows=projected,inventory=[dict(key=list(k),info=v) for k,v in index.items()],pairs=[dict(from_transaction_id=p['from_row']['id'],to_transaction_id=p['to_row']['id'],allocation=p['allocation_msat'],gross=p['from_allocation_msat']) for p in pairs]))
    for cls in [ParentSpendDerivationTests,ChangePassthroughTests,PassthroughWeightTests,WalletIsolationTests]:
        fixture=cls();fixture.setUp()
        capture(cls.__name__,fixture.rows)
        capture(cls.__name__+'-reverse',list(reversed(fixture.rows)))
        capture(cls.__name__+'-privacy',fixture.rows,skip=lambda row:row['direction']=='outbound')
    fixture=ParentSpendDerivationTests();fixture.setUp()
    capture('corroborating-inventory',fixture.rows,_index(fixture.parents))
    capture('ambiguous-inventory',fixture.rows,{k:dict(v,ambiguous=True) for k,v in _index(fixture.parents).items()})
    capture('contradictory-inventory',fixture.rows,_index(fixture.parents,wallet='other'))
    for count in [12,1101]:
        chain=OrderAndScaleTests()._long_chain(count)
        capture(f'chain-{count}',chain)
        capture(f'chain-{count}-reverse',list(reversed(chain)))
    fixture=ChangePassthroughTests();fixture.setUp()
    raw=json.loads(fixture.hop['raw_json']);raw['vin'][0]['txid']=fixture.spend_txid
    fixture.hop['raw_json']=json.dumps(raw,sort_keys=True)
    rows=[_authoritative(row) if row['id']=='hop' else row for row in fixture.rows]
    capture('cycle',rows)
    fixture=PassthroughWeightTests();fixture.setUp()
    # Largest remainders must follow row ids even if chronology puts them in
    # the opposite order; a floating allocation cannot preserve this msat.
    for row in fixture.rows:
        if row['direction']=='outbound' and row['amount']:
            row['amount']+=1
    capture('weighted-msat-remainder',[_authoritative(row) for row in reversed(fixture.rows)])
    return vectors


def generate():
    scratch = ROOT/'target/r5-sof-mutations-4/scratch'
    scratch.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='batch-oracle-',dir=scratch) as directory:
        template=open_db(__import__('pathlib').Path(directory)/'data')
        template.executescript(BASE_SQL)
        template.commit()
        schema='\n'.join(template.iterdump())
        corpus=cases(template)
        from sqlcipher3 import dbapi2 as cipher
        for case in corpus:
            for encrypted in [False,True]:
                driver=cipher if encrypted else sqlite3
                conn=driver.connect(':memory:');conn.row_factory=driver.Row
                conn.executescript(schema);conn.execute('PRAGMA foreign_keys=ON')
                try:conn.executescript(case['sql'])
                except Exception as error:raise AssertionError(case['name']) from error
                before=dump(conn) if not case['name'].endswith('deferred') else None
                expected=outcome(conn,case)
                after=dump(conn) if before is not None else None
                if before is not None:
                    assert {k:v for k,v in before.items() if k!='source_funds_links'}=={k:v for k,v in after.items() if k!='source_funds_links'},case['name']
                    if 'error' in expected or (isinstance(case['args'],dict) and case['args'].get('dry_run')):
                        assert before==after,case['name']
                if not encrypted:
                    case['expected']=expected
                    case['tables']={'source_funds_links':[dict(row) for row in conn.execute('SELECT * FROM source_funds_links ORDER BY rowid')]}
                elif expected!=case['expected']:case['expected_cipher']=expected
                conn.close()
        template.close()
    return json.dumps(dict(base_sql=BASE_SQL,cases=corpus,structural_vectors=structural_vectors()),ensure_ascii=True,indent=2)+'\n'


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--write',action='store_true');args=parser.parse_args()
    generated=generate()
    if args.write:DESTINATION.write_text(generated)
    else:assert DESTINATION.read_text()==generated
    print(f"source-funds batch oracle: {len(json.loads(generated)['cases'])} cases")

if __name__=='__main__':main()

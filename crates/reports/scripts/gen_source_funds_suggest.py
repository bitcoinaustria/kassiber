"""Freeze suggest previews from Python with deterministic identities and rollback."""
import argparse
from itertools import count
import json
from pathlib import Path
import tempfile
from unittest.mock import patch
from uuid import UUID

from kassiber import daemon
from kassiber.core import source_funds as sof
from kassiber.db import open_db
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_projection import tables, wire
from crates.reports.scripts.gen_source_funds_review import BASE_SQL
from crates.reports.scripts.gen_source_funds_report import transaction, insert, link
from crates.reports.scripts.gen_source_funds_reads import quote

ROOT=Path(__file__).resolve().parents[3]
DESTINATION=ROOT/'crates/reports/tests/data/source_funds_suggest.json'
SCRATCH=ROOT/'target/r5-sof-reads-5/scratch'
STAMP='2026-01-02T03:04:05Z'
CLEAN="DELETE FROM source_funds_links;UPDATE transactions SET excluded=1 WHERE profile_id='p';"


def pair(key='trade_id',value='proof',*,a=100_000,b=100_000,source_asset='BTC',target_asset='BTC'):
    raw=wire({key:value})
    return CLEAN+transaction('suggest-out',a,direction='outbound',asset=source_asset,raw_json=raw)+transaction('suggest-in',b,wallet='hot',asset=target_asset,raw_json=raw)


def outcome(conn,args):
    ids=count(1)
    conn.execute('SAVEPOINT expected_preview')
    try:
        with patch.object(sof,'_now',return_value=STAMP),patch.object(sof.uuid,'uuid4',side_effect=lambda:UUID(f'00000000-0000-4000-8000-{next(ids):012d}')):
            data=sof.suggest_links(conn,args.get('workspace'),args.get('profile'),daemon._source_funds_hooks(),target_transaction_ref=args.get('target_transaction'),include_broad_hints=args.get('include_broad_hints',False),max_suggestions=args.get('max_suggestions',500),commit=False)
        data['dry_run']=True
        return dict(ok=data)
    except AppError as e:
        return dict(error=dict(code=e.code,message=str(e),hint=e.hint,details=e.details,retryable=e.retryable))
    except Exception as e:
        return dict(error=dict(code='internal_error',message=str(e),hint=None,details=None,retryable=False))
    finally:
        conn.execute('ROLLBACK TO expected_preview');conn.execute('RELEASE expected_preview')


def structural(rows,owned):
    sql=CLEAN
    for wallet in sorted({r['wallet_id'] for r in rows}):
        if wallet not in ['cold','hot','other']:
            sql+=insert('wallets',dict(id=wallet,workspace_id='ws',profile_id='p',label='Lineage '+wallet,kind='custom',config_json='{}',created_at='fixed'))
    for row in rows:
        fields={k:v for k,v in row.items() if not k.startswith('observation_') and k!='wallet_config_json'}
        fields.setdefault('external_id_kind','txid')
        fields.setdefault('fee',0)
        sql+=transaction(row['id'],row['amount'],**{k:v for k,v in fields.items() if k not in ['id','amount']})
        if row.get('observation_authority_version'):
            sql+=insert('chain_observation_provenance',dict(transaction_id=row['id'],workspace_id='ws',profile_id='p',wallet_id=row['wallet_id'],authority_version=row['observation_authority_version'],observer_ids_json='[]',observer_kinds_json='[]',chain='bitcoin',network='main',application_revision='synthetic',graph_hash=row['observation_graph_hash'],quantity_hash=row['observation_quantity_hash'],fee_attribution='exact',observed_at='fixed',updated_at='fixed'))
    for i,row in enumerate(owned):
        fields={k:v for k,v in row.items() if k not in ['ambiguous','asset_identity']}
        fields.update(id='owned-'+str(i),workspace_id='ws',profile_id='p',amount=1000,outpoint=str(row['txid'])+':'+str(row['vout']),confirmation_status='confirmed',first_seen_at='fixed',last_seen_at='fixed')
        sql+=insert('wallet_utxos',fields)
    return sql


def cases():
    result=[]
    def add(name,args=None,sql=None,seed=BASE_SQL):
        result.append(dict(name=name,command='source-funds.suggest',args_json=wire(dict(dry_run=True,**(args or {}))),sql=seed+(pair() if sql is None else sql)))
    add('provider-default')
    for key in sof.PROVIDER_EVIDENCE_KEYS:
        for broad in [False,True]:add('key-'+key+'-'+str(broad),dict(include_broad_hints=broad),pair(key))
    for value in [None,'',False,True,0,1,1.5,[],{},['a'],{'x':1},' Ω ','\ud800','\ud800\udfff']:
        add('provider-value-'+repr(value),sql=pair(value=value))
    for a,b in [(0,0),(100000,101000),(100000,101011),(100000,101012),(-100000,100000),(100000,-100000),(1000,2000),(1000,2001),(0.9,1.9),(1e20,1e20)]:
        add('amount-'+str((a,b)),sql=pair(a=a,b=b))
    for a,b in [('btc','BTC'),('L-BTC','LBTC'),('BTC','LBTC'),('ab'*32,'AB'*32)]:add('asset-'+a+'-'+b,sql=pair(source_asset=a,target_asset=b))
    for target in [None,'suggest-in','suggest-out','in','missing','duplicate','  suggest-in  ']:add('target-'+str(target),dict(target_transaction=target))
    for profile in ['Other','missing']:add('profile-'+profile,dict(profile=profile))
    for cap in [-1,0,1,500,501,10**40]:add('cap-'+str(cap),dict(max_suggestions=cap))
    for state in ['suggested','reviewed','rejected']:
        for method in ['provider_trade_id','manual']:
            for confidence in ['weak','strong','exact']:
                sql=pair()+link('existing','suggest-out','suggest-in',1,state=state,method=method,confidence=confidence,link_type='trade',explanation='old',uses_chain_observation=1,chain_data_confirmed=0)
                sql+="INSERT INTO source_funds_link_attachments(link_id,attachment_id,created_at) VALUES('existing','a-a','fixed');"
                add('existing-'+state+'-'+method+'-'+confidence,sql=sql)
    add('existing-multiple',sql=pair()+link('weak','suggest-out','suggest-in',1,state='suggested',confidence='weak')+link('unknown','suggest-out','suggest-in',1,state='suggested',confidence='unknown'))
    for name,change in {
        'excluded':"UPDATE transactions SET excluded=1 WHERE id='suggest-out';",
        'same-wallet':"UPDATE transactions SET wallet_id='cold' WHERE id='suggest-in';",
        'different-day':"UPDATE transactions SET occurred_at='2024-01-02T00:00:00Z' WHERE id='suggest-in';",
        'empty-time':"UPDATE transactions SET occurred_at='' WHERE id='suggest-out';",
        'invalid-raw':"UPDATE transactions SET raw_json='{broken' WHERE id='suggest-in';",
        'non-object-raw':"UPDATE transactions SET raw_json='[1,2]' WHERE id='suggest-in';",
        'zero-target':"UPDATE transactions SET amount=0 WHERE id='suggest-in';",
        'outside-profile':"UPDATE transactions SET profile_id='other' WHERE id='suggest-in';",
        'stale':"UPDATE profiles SET last_processed_at=NULL WHERE id='p';",
        'source-blocked':"INSERT INTO freshness_source_states(profile_id,source_type,source_key,source_label,status,blocking_reports,updated_at) VALUES('p','wallet','cold','Cold','error',1,'fixed');",
        'no-context':"DELETE FROM settings WHERE key='context_profile';",
    }.items():add(name,dict(include_broad_hints=True),pair()+change)
    add('broad-same-wallet',dict(include_broad_hints=True),pair(value=None)+"UPDATE transactions SET wallet_id='cold' WHERE id='suggest-in';")
    add('broad-different-day',dict(include_broad_hints=True),pair(value=None)+"UPDATE transactions SET occurred_at='2024-01-02T00:00:00Z' WHERE id='suggest-in';")
    for b in [101000,101001,-100000]:add('broad-threshold-'+str(b),dict(include_broad_hints=True),pair(value=None,a=100000,b=b))
    for n in [2,501]:
        sql=pair()+''.join(transaction('extra-'+str(i),100000,wallet='hot',raw_json=wire(dict(trade_id='proof'))) for i in range(n-1))
        add('cartesian-'+str(n),dict(max_suggestions=1000),sql)
        if n==2:add('cap-rollback',dict(max_suggestions=1),sql)
    sql=pair()+transaction('unrelated',100000,wallet='hot',raw_json=wire(dict(trade_id='proof')))
    add('provider-widens-scope',dict(target_transaction='suggest-in'),sql)
    add('target-frontier',dict(target_transaction='in'),pair()+link('scope','suggest-in','in',1))
    add('target-rejected-frontier',dict(target_transaction='in'),pair()+link('scope','suggest-in','in',1,state='rejected'))
    supersede=pair()+transaction('extra',100000,wallet='hot',raw_json=wire(dict(trade_id='proof')))
    supersede+="UPDATE transactions SET raw_json="+quote(wire(dict(trade_id='proof',order_id='unique')))+" WHERE id IN ('suggest-in','suggest-out');"
    supersede+=link('updated','suggest-out','suggest-in',1,state='suggested',method='provider_trade_id',confidence='weak',link_type='trade')
    supersede+="INSERT INTO source_funds_link_attachments(link_id,attachment_id,created_at) VALUES('updated','a-a','fixed');"
    add('superseded-updated-attachments',sql=supersede)
    reports=json.loads((ROOT/'crates/reports/tests/data/source_funds_report.json').read_text())
    for case in reports:
        if case['name'].startswith('custody-') and not case['name'].endswith('-text'):
            sql=case['sql']+"DELETE FROM source_funds_links;"
            add('projection-'+case['name'],dict(target_transaction='in'),sql='',seed=sql)
        if case['name'].startswith('reconciliation-'):
            add(case['name'],dict(target_transaction='in'),sql='',seed=case['sql'])
        if case['name'] in ['structural-current','structural-no-authority','structural-privacy','structural-contradicted-inventory']:
            add(case['name'],dict(target_transaction='spend'),sql="DELETE FROM source_funds_links;",seed=case['sql'])
    projection_seed=next(c['sql'] for c in reports if c['name']=='custody-current')
    add('projection-does-not-parse-observed-amount',dict(target_transaction='in'),sql='',seed=projection_seed+"DELETE FROM source_funds_links;UPDATE transactions SET amount='not-an-integer' WHERE id='out';")
    proof=next(c['sql'] for c in reports if c['name']=='reconciliation-current')
    projection=next(c['sql'] for c in reports if c['name']=='custody-current').rsplit("INSERT INTO journal_custody_decisions",1)[1]
    projection="INSERT INTO journal_custody_decisions"+projection.replace("'out'","'out2'").replace("'"+'d'*64+"'","'"+'e'*64+"'")
    add('reconciled-target-suppresses-native',dict(target_transaction='in'),sql='',seed=proof+transaction('out2',1000,direction='outbound')+projection+"UPDATE profiles SET last_processed_tx_count=5 WHERE id='p';")
    projection_seed=next(c['sql'] for c in reports if c['name']=='custody-current')
    add('projection-order',dict(target_transaction='in'),sql='',seed=projection_seed+"DELETE FROM source_funds_links;"+transaction('out2',1000,direction='outbound')+projection+"UPDATE profiles SET last_processed_tx_count=5 WHERE id='p';")
    lineage=json.loads((ROOT/'crates/reports/tests/data/source_funds_lineage.json').read_text())
    for case in lineage:
        if case['name'] in ['ChangePassthroughTestsFalse','PassthroughWeightTestsFalse','ChangePassthroughTestsprivacy','ChangePassthroughTestsinventory','wallet-memo-isolation','chain-3']:
            rows=case['rows'];target=next((r['id'] for r in rows if r['id']=='spend'),rows[-1]['id'])
            add('lineage-'+case['name'],dict(target_transaction=target),structural(rows,case['owned']))
    from tests.test_source_funds_assembly import _authoritative
    lineage_case=next(c for c in lineage if c['name']=='ChangePassthroughTestsFalse')
    rows=[]
    for row in lineage_case['rows']:
        row=dict(row)
        if row['direction']=='inbound':
            raw=json.loads(row['raw_json']);raw['trade_id']='ancestor-provider';row['raw_json']=wire(raw);row=_authoritative(row)
        rows.append(row)
    scoped=structural(rows,[])+transaction('provider-outsider',100000,direction='outbound',raw_json=wire(dict(trade_id='ancestor-provider')))
    add('structural-does-not-widen-provider-scope',dict(target_transaction=rows[-1]['id']),scoped)
    for source,target in [('deposit','premix'),('deposit','badbank'),('premix','postmix'),('postmix','postmix'),('deposit','postmix')]:
        sql=pair()+"UPDATE transactions SET external_id="+quote('ab'*32)+",external_id_kind='txid',raw_json="+quote(wire(dict(chain='bitcoin',network='main',trade_id='proof')))+" WHERE id IN ('suggest-out','suggest-in');"
        for wallet,section in [('cold',source),('hot',target)]:
            sql+='UPDATE wallets SET config_json='+quote(wire(dict(chain='bitcoin',network='main',samourai=dict(role='child',group_id='group',section=section))))+' WHERE id='+quote(wallet)+';'
        add('samourai-'+source+'-'+target,dict(target_transaction='suggest-in'),sql)
    sql=next(c['sql'] for c in result if c['name']=='samourai-deposit-premix')
    raw=wire(dict(chain='bitcoin',network='main'))
    sql+=transaction('surrogate-in',100000,wallet='other',external_id='ab'*32,external_id_kind='txid',raw_json=raw)
    sql+='UPDATE transactions SET raw_json='+quote(raw)+" WHERE id IN ('suggest-out','suggest-in');"
    sql+='UPDATE wallets SET config_json='+quote(wire(dict(chain='bitcoin',network='main',samourai=dict(role='child',group_id='group',section='\ud800'))))+" WHERE id='other';"
    add('samourai-unrecognized-surrogate-section',sql='',seed=sql)
    return result


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--write',action='store_true');args=parser.parse_args()
    SCRATCH.mkdir(parents=True,exist_ok=True);fixtures=cases()
    with tempfile.TemporaryDirectory(dir=SCRATCH) as temporary:
        conn=open_db(Path(temporary)/'data')
        for case in fixtures:
            conn.executescript('SAVEPOINT suggest_case;'+case['sql']);before=tables(conn)
            expected=outcome(conn,json.loads(case['args_json']))
            assert tables(conn)==before,case['name']
            if case['name']=='structural-does-not-widen-provider-scope':assert expected['ok']['inserted']==1,expected
            if case['name']=='projection-order':assert expected['ok']['inserted']==2,expected
            if case['name']=='samourai-unrecognized-surrogate-section':assert expected['ok']['inserted']==2,expected
            if case['name']=='superseded-updated-attachments':
                assert expected['ok']['inserted']==3,expected
                assert expected['ok']['links'][0]['attachments']==[],expected
            case['expected_json']=wire(expected)
            conn.execute('ROLLBACK TO suggest_case');conn.execute('RELEASE suggest_case')
        conn.close()
    generated=json.dumps(fixtures,ensure_ascii=True,indent=2)+'\n'
    if args.write:DESTINATION.write_text(generated)
    elif DESTINATION.read_text()!=generated:raise SystemExit('source-funds suggestion vectors differ')
    print(f'{len(fixtures)} suggestion cases verified; all 114 tables unchanged')


if __name__=='__main__':main()

import argparse
import json
from pathlib import Path
import tempfile

from kassiber import daemon
from kassiber.core import source_funds_coverage as coverage
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION, open_db
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_projection import tables, wire
from crates.reports.scripts.gen_source_funds_review import BASE_SQL
from crates.reports.scripts.gen_source_funds_report import transaction, link

ROOT=Path(__file__).resolve().parents[3]
DESTINATION=ROOT/'crates/reports/tests/data/source_funds_coverage.json'
SCRATCH=ROOT/'target/r5-sof-reads-5/scratch'


def outcome(conn,command,args):
    try:
        if command.startswith('ui.'):
            data=daemon._ui_source_funds_payload_from_conn(conn,command,daemon._coerce_args_dict(None,args))
        else:
            data=coverage.compute_coverage(conn,args.get('workspace'),args.get('profile'),daemon._source_funds_hooks(),max_depth=args.get('max_depth',8),max_transactions=args.get('max_transactions',2000))
            if args.get('format') in ['table','plain']:data='\n'.join(coverage.coverage_summary_text(data))
        return dict(ok=data)
    except AppError as e:
        return dict(error=dict(code=e.code,message=str(e),hint=e.hint,details=e.details,retryable=e.retryable))
    except Exception as e:
        return dict(error=dict(code='internal_error',message=str(e),hint=None,details=None,retryable=False))


def cases():
    result=[]
    def add(name,args=None,sql='',seed=BASE_SQL,command='ui.source_funds.coverage'):
        result.append(dict(name=name,command=command,args_json=wire(args or {}),sql=seed+sql))
    reports=json.loads((ROOT/'crates/reports/tests/data/source_funds_report.json').read_text())
    names=['ready','stale','blocked-source','no-context','unknown-source','missing-history','attestation','no-source-amount','source-overallocated','source-asset','source-future','invalid-time','missing-pricing','null-allocation','zero-from','null-from','heuristic','chain-unconfirmed','chain-confirmed','chain-method','stale-custody','stale-structural','coinjoin','payjoin','custody-current','custody-stale-version','custody-wrong-count','reconciliation-current','reconciliation-retired','structural-current','structural-no-authority','structural-privacy','structural-contradicted-inventory','branch-depth-8','cycle']
    names += ['live-standard-in','explorer-bitcoin-main-https://[broken/api','shared-unknown']
    for case in reports:
        if case['name'] in names:
            add('report-'+case['name'],seed=case['sql'])
    clean="DELETE FROM source_funds_links WHERE id!='l-a';UPDATE source_funds_links SET from_allocation_amount=allocation_amount;"
    for name,kind in [('fully-traced','fiat_purchase'),('attested','opening_balance_attestation'),('unattributed','unknown')]:
        add(name,sql=clean+"UPDATE source_funds_sources SET source_type='"+kind+"' WHERE id='s-a';")
    split=clean+"UPDATE source_funds_links SET allocation_amount=61728394500,from_allocation_amount=61728394500 WHERE id='l-a';UPDATE source_funds_sources SET amount=61728394501,acquired_at=NULL WHERE id='s-old';"+link('l-b','s-old','in',61728394501,source=True)
    for first,second in [('fiat_purchase','missing_history'),('fiat_purchase','unknown'),('opening_balance_attestation','unknown')]:
        add('mixed-'+first+'-'+second,sql=split+"UPDATE source_funds_sources SET source_type='"+first+"' WHERE id='s-a';UPDATE source_funds_sources SET source_type='"+second+"' WHERE id='s-old';")
    add('desktop-unknown-fields',dict(unknown=True,profile='missing',workspace='missing'))
    for profile in ['Other','missing']:
        add('cli-profile-'+profile,dict(profile=profile),command='source-funds.coverage')
    malformed="UPDATE profiles SET last_processed_at='fixed',last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id='p' AND excluded=0),last_processed_input_version=journal_input_version WHERE id='p';UPDATE profiles SET journal_semantics_version=" + str(JOURNAL_CALCULATION_SEMANTICS_VERSION) + " WHERE id='p';ALTER TABLE journal_custody_reconciliations RENAME COLUMN proof_json TO hidden_proof;"
    add('sqlite-error',sql=clean+malformed)
    add('unlinked-malformed-schema',sql=malformed+'DELETE FROM source_funds_links;')
    add('capped-malformed-schema',dict(max_transactions=0),sql=malformed,command='source-funds.coverage')
    for cap in [-1,0,1,2,3,2000,10**40]:
        add('cli-cap-'+str(cap),dict(max_transactions=cap),command='source-funds.coverage')
    for depth in [-1,0,1,2,8,64,99]:
        add('cli-depth-'+str(depth),dict(max_depth=depth),command='source-funds.coverage')
    for cap in [None,False,True,-1,0,1,2,2.5,'2',50_001,10**40,[],{}]:
        add('desktop-cap-'+repr(cap),dict(max_transactions=cap))
    for depth in [None,False,True,-1,0,1,2,2.5,'2',33,10**40,[],{}]:
        add('desktop-depth-'+repr(depth),dict(max_depth=depth))
    for name,sql in {
        'empty':"DELETE FROM source_funds_links;DELETE FROM source_funds_cases;DELETE FROM source_funds_source_attachments;DELETE FROM source_funds_link_attachments;DELETE FROM attachments;UPDATE transactions SET excluded=1;",
        'unlinked':"DELETE FROM source_funds_links;",
        'excluded-inbound':"UPDATE transactions SET excluded=1 WHERE id='earn';",
        'rejected-only':"UPDATE source_funds_links SET state='rejected';",
        'suggested':"UPDATE source_funds_links SET state='suggested';",
        'fractions':"UPDATE transactions SET amount=0.9 WHERE id='in';",
        'negative':"UPDATE transactions SET amount=-1000 WHERE id='in';",
        'unbounded-positive-total':"UPDATE transactions SET amount=9223372036854775807 WHERE profile_id='p' AND direction='inbound';",
        'unbounded-negative-total':"UPDATE transactions SET amount=-9223372036854775808 WHERE profile_id='p' AND direction='inbound';",
        'wallet-label-order':"UPDATE wallets SET label=CASE id WHEN 'cold' THEN 'z Ω' ELSE 'a Ω' END WHERE profile_id='p';",
        'wallet-label-case':"UPDATE wallets SET label=CASE id WHEN 'cold' THEN 'Same Ω' ELSE 'SAME Ω' END WHERE profile_id='p';UPDATE transactions SET asset='BTC' WHERE profile_id='p';",

        'multiple-assets':transaction('lbtc',54321,asset='LBTC')+transaction('otherasset',87654,asset='L-BTC')+transaction('small',1),
        'unattributed-dominates':"UPDATE source_funds_sources SET source_type='unknown' WHERE id='s-a';",
    }.items():add(name,sql=sql)
    for args in [None,[],False]:
        result.append(dict(name='raw-'+repr(args),command='ui.source_funds.coverage',args_json=wire(args),sql=BASE_SQL))
    population='DELETE FROM source_funds_links;'+''.join(transaction(f'population-{i:04}',i+1) for i in range(2001))
    add('default-production-cap',sql=population)
    add('complete-population',dict(max_transactions=5000),sql=population)
    for cap in [0,1,2]:
        add('text-cap-'+str(cap),dict(max_transactions=cap,format='plain'),command='source-funds.coverage')
    return result


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--write',action='store_true');args=parser.parse_args()
    SCRATCH.mkdir(parents=True,exist_ok=True);fixtures=cases()
    with tempfile.TemporaryDirectory(dir=SCRATCH) as temporary:
        conn=open_db(Path(temporary)/'data')
        for case in fixtures:
            conn.executescript('SAVEPOINT coverage_case;'+case['sql'])
            before=tables(conn)
            expected=outcome(conn,case['command'],json.loads(case['args_json']))
            assert tables(conn)==before,case['name']
            case['expected_json']=wire(expected)
            if case['name'] in ['fully-traced','attested','unattributed']:
                bucket=case['name'].replace('-','_')
                assert expected['ok']['totals']['buckets'][bucket]['tx_count']==1,(case['name'],expected)
            if case['name'].startswith('mixed-'):
                bucket='unattributed' if case['name'].endswith('unknown') else 'attested'
                assert expected['ok']['totals']['buckets'][bucket]['tx_count']==1,(case['name'],expected)
            if case['name']=='sqlite-error':assert expected['ok']['totals']['buckets']['in_review']['tx_count']>0
            if case['name']=='report-explorer-bitcoin-main-https://[broken/api':assert expected['error']['message']=='Invalid IPv6 URL'
            conn.execute('ROLLBACK TO coverage_case');conn.execute('RELEASE coverage_case')
        conn.close()
    generated=json.dumps(fixtures,ensure_ascii=True,indent=2)+'\n'
    if args.write:DESTINATION.write_text(generated)
    elif DESTINATION.read_text()!=generated:raise SystemExit('source-funds coverage vectors differ')
    print(f'{len(fixtures)} coverage cases verified; all 114 tables unchanged')


if __name__=='__main__':main()

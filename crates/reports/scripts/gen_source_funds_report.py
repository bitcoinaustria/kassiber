import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from kassiber import daemon
from kassiber.core import source_funds as sof
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION, open_db
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_reads import BASE, quote
from crates.reports.scripts.gen_source_funds_projection import tables, wire

ROOT = Path(__file__).resolve().parents[3]
DESTINATION = ROOT/'crates/reports/tests/data/source_funds_report.json'
SCRATCH = ROOT/'target/r5-sof-reads-4/scratch'


def outcome(conn, command, args):
    try:
        if command.startswith('ui.'):
            result = daemon._ui_source_funds_payload_from_conn(conn, command, daemon._coerce_args_dict(None, args))
        else:
            result = sof.build_report(conn, args.get('workspace'), args.get('profile'), daemon._source_funds_hooks(),
                target_transaction_ref=args.get('target_transaction'), target_amount=args.get('target_amount'),
                report_purpose=args.get('purpose') or 'existing_transaction',
                planned_destination=args.get('planned_destination'), planned_note=args.get('planned_note'),
                reveal_mode=args.get('reveal_mode'), max_depth=args.get('max_depth', 8), recipient_ref=args.get('recipient'),
                include_diagrams=True, report_options=dict(diagram_detail=args.get('diagram_detail'), amount_precision=args.get('amount_precision'),
                    mask_recipient=args.get('mask_recipient'), omit_sections=args.get('omit_section'),
                    reveal_overrides=dict(item.split('=', 1) for item in args.get('reveal_override', []) if '=' in item)))
            if args.get('format') in ('table', 'plain'):
                result = '\n'.join(sof.build_report_lines(result, daemon._source_funds_hooks()))
        return dict(ok=result)
    except AppError as error:
        return dict(error=dict(code=error.code, message=str(error), hint=error.hint, details=error.details, retryable=error.retryable))
    except Exception as error:
        return dict(error=dict(code='internal_error', message=str(error), hint=None, details=None, retryable=False))


def transaction(id, amount, day=1, wallet='cold', **fields):
    defaults = dict(id=id, workspace_id='ws', profile_id='p', wallet_id=wallet, fingerprint=id,
                  occurred_at=f'2024-01-{day:02}T00:00:00Z', direction='inbound', asset='BTC',
                  amount=amount, fee=0, fiat_value=1, created_at='fixed')
    defaults.update(fields)
    return insert('transactions', defaults)


def insert(table, fields):
    def literal(v):
        return 'NULL' if v is None else str(v) if isinstance(v, (int, float)) else quote(v)
    return f'INSERT INTO {table}({",".join(fields)}) VALUES({",".join(map(literal,fields.values()))});\n'


def link(id, parent, child, amount, from_amount=None, source=False, **extra):
    fields = dict(id=id, workspace_id='ws', profile_id='p', to_transaction_id=child,
                  link_type='manual_source' if source else 'self_transfer', state='reviewed', confidence='exact',
                  method='manual', asset='BTC', allocation_amount=amount, from_asset='BTC',
                  from_allocation_amount=amount if from_amount is None else from_amount,
                  allocation_policy='explicit', created_at='fixed', updated_at='fixed')
    fields['from_source_id' if source else 'from_transaction_id'] = parent
    fields.update(extra)
    return insert('source_funds_links', fields)


def cases():
    result = []
    base = (ROOT/'crates/reports/tests/data/book.sql').read_text()+BASE
    def add(name, args=None, sql='', command='ui.source_funds.preview', seed=base):
        result.append(dict(name=name, command=command, args_json=wire({'target_transaction':'in', **(args or {})}), sql=seed+sql))
    for mode in sof.REVEAL_MODES:
        for target in ['in', 'out', 'earn', 'no-tax', 'excluded']:
            add(f'live-{mode}-{target}', dict(target_transaction=target, reveal_mode=mode))
        for purpose in sof.REPORT_PURPOSES:
            for detail in ['summary', 'detailed']:
                add(f'options-{mode}-{purpose}-{detail}',dict(reveal_mode=mode, report_purpose=purpose,
                    planned_destination='  Exchange Ω  ', planned_note='  Planned note  ',
                    report_options=dict(diagram_detail=detail,amount_precision='sats',mask_recipient=True,
                        omit_sections=['flow_links'], reveal_overrides={'in':'hide'})))
        for recipient in ['r-a', 'alpha', 'r-i', 'missing']:
            add(f'recipient-{mode}-{recipient}',dict(recipient=recipient,reveal_mode=mode))
    for recipient in [None,'','r-a','ALPHA','alpha','r-i','missing','  ']:
        add('recipient-default-'+str(recipient),dict(recipient=recipient))
    for args in [dict(target_transaction=''),dict(target_transaction='  in  '),dict(target_transaction='duplicate'),dict(target_transaction='missing'),dict(target_transaction=1),dict(target_transaction=None),dict(reveal_mode='wrong'),dict(report_purpose='wrong'),dict(max_depth=-1),dict(max_depth=0),dict(max_depth=1),dict(max_depth=65),dict(max_depth=True),dict(max_depth=1.9),dict(max_depth='2'),dict(planned_destination=42,planned_note=['x']),dict(report_options=['invalid'])]:
        add('argument-'+repr(args),args)
    for quantity in ['0','-0.00000000001','0.00000000001','0.00000000123','0.1','1.23456789001','2','bad','NaN','Infinity','-Infinity','NaN_١٢','sNaN12','_In_f_','nan-invalid',False,1.5]:
        add('target-amount-'+str(quantity),dict(target_amount=quantity))
    add('partial-inexact-ratio',dict(target_amount='0.00000000001'),sql="DELETE FROM source_funds_links WHERE id='l-z';UPDATE transactions SET amount=3 WHERE id='in';UPDATE source_funds_sources SET amount=2 WHERE id='s-a';UPDATE source_funds_links SET allocation_amount=3,from_allocation_amount=2 WHERE id='l-a';")
    add('partial-text',dict(target_amount='0.1',format='table'),sql="DELETE FROM source_funds_links WHERE id='l-z';UPDATE source_funds_links SET from_allocation_amount=allocation_amount WHERE id='l-a';",command='reports.source-funds')
    for sql_name, sql in {
        'ready':"DELETE FROM source_funds_links WHERE id='l-z';UPDATE source_funds_links SET from_allocation_amount=allocation_amount WHERE id='l-a';",
        'stale':"UPDATE profiles SET journal_input_version=99 WHERE id='p';",
        'blocked-source':"INSERT INTO freshness_source_states(profile_id,source_type,source_key,source_label,status,blocking_reports,updated_at) VALUES('p','wallet','cold','Cold','error',1,'fixed');",
        'no-context':"DELETE FROM settings WHERE key='context_profile';",
        'unknown-source':"UPDATE source_funds_sources SET source_type='unknown' WHERE id='s-a';",
        'missing-history':"UPDATE source_funds_sources SET source_type='missing_history',amount=NULL WHERE id='s-a';",
        'attestation':"UPDATE source_funds_sources SET source_type='opening_balance_attestation',amount=NULL WHERE id='s-a';",
        'no-source-amount':"UPDATE source_funds_sources SET amount=NULL WHERE id='s-a';",
        'source-overallocated':"UPDATE source_funds_sources SET amount=1 WHERE id='s-a';",
        'source-asset':"UPDATE source_funds_sources SET asset='LBTC' WHERE id='s-a';",
        'source-future':"UPDATE source_funds_sources SET acquired_at='2025-01-01T00:00:00+02:00' WHERE id='s-a';",
        'invalid-time':"UPDATE source_funds_sources SET acquired_at='not a date' WHERE id='s-a';",
        'raw-privacy':"UPDATE transactions SET privacy_boundary='payment-in-coinjoin' WHERE id='in';",
        'missing-pricing':"UPDATE transactions SET fiat_value=NULL,fiat_rate=NULL WHERE id='in';",
        'null-allocation':"UPDATE source_funds_links SET allocation_amount=NULL WHERE id='l-a';",
        'zero-from':"UPDATE source_funds_links SET from_allocation_amount=0 WHERE id='l-a';",
        'null-from':"UPDATE source_funds_links SET from_allocation_amount=NULL WHERE id='l-a';",
        'heuristic':"UPDATE source_funds_links SET allocation_policy='heuristic' WHERE id='l-a';",
        'chain-unconfirmed':"UPDATE source_funds_links SET uses_chain_observation=1 WHERE id='l-a';",
        'chain-confirmed':"UPDATE source_funds_links SET uses_chain_observation=1,chain_data_confirmed=1 WHERE id='l-a';",
        'chain-method':"UPDATE source_funds_links SET method='chain_observation' WHERE id='l-a';",
        'stale-custody':"UPDATE source_funds_links SET method='custody_component' WHERE id='l-a';",
        'stale-structural':"UPDATE source_funds_links SET method='utxo_spend' WHERE id='l-a';",
        'coinjoin':"UPDATE source_funds_links SET link_type='coinjoin' WHERE id='l-a';",
        'payjoin':"UPDATE source_funds_links SET link_type='payjoin' WHERE id='l-a';",
        'negative-zero':"UPDATE transactions SET fiat_value=-0.0 WHERE id='in';",
    }.items():
        add(sql_name,sql=sql)
        add(sql_name+'-text',dict(format='table'),sql=sql,command='reports.source-funds')
    for chain, network in [('bitcoin','main'),('bitcoin','test'),('bitcoin','signet'),('bitcoin','regtest'),('liquid','liquidv1'),('liquid','liquidtestnet'),('bad','main')]:
        sql="UPDATE transactions SET external_id="+quote('ab'*32)+" WHERE id='in';UPDATE wallets SET config_json="+quote(wire(dict(chain=chain,network=network)))+" WHERE id='cold';"
        add(f'explorer-{chain}-{network}',sql=sql)
        for url in ['https://user:secret@Explorer.Example:8443/path/api/?query=private#fragment','http://[::1]:3000/api','ftp://example.com/api','https://example.com:invalid/api','https://example.com/aPi','https://[broken/api','https://[fe80::1%25EN0]:3000/api','https://example.com:+80/api']:
            backend=insert('backends',dict(name='custom',kind='esplora',chain=chain if chain!='bad' else 'bitcoin',network=network,url=url,config_json=wire(dict(infrastructure_owner='self',display_name='Local explorer')),created_at='fixed',updated_at='fixed'))
            add(f'explorer-{chain}-{network}-{url}',sql=sql+backend)
    for config in ['{broken','[1]','{"chain":"bitcoin","network":"wrong"}']:
        add('explorer-config-'+config,sql="UPDATE transactions SET external_id="+quote('ab'*32)+" WHERE id='in';UPDATE wallets SET config_json="+quote(config)+" WHERE id='cold';")
    add('explorer-invalid-config-custom',sql="UPDATE transactions SET external_id="+quote('ab'*32)+" WHERE id='in';UPDATE wallets SET config_json='{broken' WHERE id='cold';"+insert('backends',dict(name='custom',kind='esplora',chain='bitcoin',network='main',url='https://custom.invalid/api',created_at='fixed',updated_at='fixed')))
    graph="DELETE FROM source_funds_links;UPDATE transactions SET amount=1000,occurred_at='2024-01-06T00:00:00Z' WHERE id='in';"
    graph+=transaction('root',1000,2)+transaction('a',600,3)+transaction('b',400,3)+transaction('c',400,4)
    graph+=link('root-source','s-a','root',1000,source=True)+link('root-a','root','a',600)+link('root-b','root','b',400)+link('b-c','b','c',400)+link('a-target','a','in',600)+link('c-target','c','in',400)
    for depth in [1,2,3,4,8,64]:
        add(f'branch-depth-{depth}',dict(max_depth=depth),graph)
    for amount in ['0.000000005','0.00000001','0.00000002']:
        add('branch-partial-'+amount,dict(target_amount=amount),graph)
    add('branch-conflicting-assets',sql=graph+"UPDATE source_funds_links SET from_asset='LBTC' WHERE id='root-a';")
    add('cycle',sql=graph+link('cycle','in','root',1000))
    add('parent-future',sql=graph+"UPDATE transactions SET occurred_at='2025-01-01' WHERE id='a';")
    add('parent-self-asset',sql=graph+"UPDATE transactions SET asset='LBTC' WHERE id='a';")
    add('shared-unknown',sql=graph+"UPDATE source_funds_sources SET source_type='unknown' WHERE id='s-a';")
    for count in [2,8,24]:
        sql="DELETE FROM source_funds_links;UPDATE transactions SET amount="+str(count*1000)+" WHERE id='in';"
        for i in range(count):
            sql+=transaction(f'tied-{i}',1000,1,external_id='shared label')+link(f'tie-{i}',f'tied-{i}','in',1000)+link(f'tie-root-{i}','s-a',f'tied-{i}',1000,source=True)
        add(f'tied-{count}',sql=sql)
    for count in [5,18,66]:
        sql="DELETE FROM source_funds_links;UPDATE transactions SET amount=1000 WHERE id='in';"
        for i in range(count):
            sql+=transaction(f'hop-{i:02}',1000,1)+link(f'hop-edge-{i}',f'hop-{i:02}','in' if i==0 else f'hop-{i-1:02}',1000)
        sql+=link('last-source','s-a',f'hop-{count-1:02}',1000,source=True)
        for detail in ['summary','detailed']:
            add(f'depth-{count}-{detail}',dict(max_depth=64,report_options=dict(diagram_detail=detail)),sql)
            add(f'depth-{count}-{detail}-cli',dict(max_depth=64,diagram_detail=detail),sql,command='reports.source-funds')
    custody = "DELETE FROM journal_custody_decisions;DELETE FROM journal_custody_economic_relations;DELETE FROM source_funds_links;UPDATE transactions SET amount=1000 WHERE id IN ('in','out');"
    custody += link('custody', 'out', 'in', 1000, method='custody_component')
    custody += "UPDATE profiles SET last_processed_at='fixed',last_processed_tx_count=4,last_processed_input_version=journal_input_version WHERE id='p';UPDATE profiles SET journal_semantics_version=" + str(JOURNAL_CALCULATION_SEMANTICS_VERSION) + " WHERE id='p';"
    decision = dict(decision_id='d'*64,workspace_id='ws',profile_id='p',source_transaction_id='out',target_transaction_id='in',source_observation_hash='a'*64,target_observation_hash='b'*64,source_start_msat=0,source_end_msat=1000,target_start_msat=0,target_end_msat=1000,source_wallet_id='cold',target_wallet_id='cold',source_network='main',target_network='main',source_rail='bitcoin',target_rail='bitcoin',source_asset='BTC',target_asset='BTC',state='internal_reviewed',basis_state='eligible',reason='self_transfer',created_at='fixed')
    projection = custody + insert('journal_custody_decisions',decision)
    for name,change in {'current':'','stale-version':"UPDATE profiles SET journal_input_version=99 WHERE id='p';",'wrong-count':"UPDATE profiles SET last_processed_tx_count=0 WHERE id='p';",'not-processed':"UPDATE profiles SET last_processed_at=NULL WHERE id='p';",'wrong-allocation':"UPDATE source_funds_links SET allocation_amount=999 WHERE id='custody';",'wrong-from':"UPDATE source_funds_links SET from_allocation_amount=999 WHERE id='custody';",'excluded-parent':"UPDATE transactions SET excluded=1 WHERE id='out';",'wrong-asset':"UPDATE source_funds_links SET from_asset='LBTC' WHERE id='custody';",'wrong-type':"UPDATE source_funds_links SET link_type='swap' WHERE id='custody';"}.items():
        add('custody-'+name,sql=projection+change)
    split = dict(decision,decision_id='e'*64,source_start_msat=400,target_start_msat=400)
    add('custody-grouped',sql=custody+insert('journal_custody_decisions',dict(decision,source_end_msat=400,target_end_msat=400))+insert('journal_custody_decisions',split))
    component=insert('custody_components',dict(id='component',lineage_id='lineage',workspace_id='ws',profile_id='p',revision=2,component_type='manual_bridge',state='active',created_at='fixed'))
    proof=dict(component_id='component',allocations=[dict(destination_kind='wallet',source_transaction_id='out',target_transaction_id='in',amount_msat=1000,asset='BTC')])
    reconciliation=custody+component+insert('journal_custody_reconciliations',dict(profile_id='p',component_id='component',input_version=7,component_revision=2,proof_json=wire(proof),created_at='fixed'))
    for name,change in {'current':'','retired':"UPDATE custody_components SET state='superseded';",'revision':"UPDATE journal_custody_reconciliations SET component_revision=3;",'version':"UPDATE journal_custody_reconciliations SET input_version=99;",'non-wallet':"UPDATE journal_custody_reconciliations SET proof_json="+quote(wire(dict(proof,allocations=[dict(proof['allocations'][0],destination_kind='external')])) )+';', 'wrong-asset':"UPDATE source_funds_links SET asset='LBTC' WHERE id='custody';",'wrong-amount':"UPDATE source_funds_links SET from_allocation_amount=999 WHERE id='custody';"}.items():
        add('reconciliation-'+name,sql=reconciliation+change)
    add('privacy-fee-reference-bug',sql="DELETE FROM source_funds_links WHERE id='l-z';UPDATE transactions SET privacy_boundary='coinjoin' WHERE id='in';UPDATE source_funds_sources SET amount=123456789124 WHERE id='s-a';UPDATE source_funds_links SET link_type='coinjoin',from_allocation_amount=123456789124 WHERE id='l-a';")
    add('svg-xml-escaping',sql="UPDATE transactions SET external_id="+quote('Target <>&" Ω')+" WHERE id='in';UPDATE source_funds_sources SET label="+quote('Root <>&" Café Ω')+" WHERE id='s-a';")
    for source_rail,target_rail,kind in [('bitcoin','liquid','peg_in'),('liquid','bitcoin','peg_out'),('bitcoin','lightning','lightning_funding'),('lightning','bitcoin','lightning_close')]:
        add('custody-rail-'+kind,sql=custody+insert('journal_custody_decisions',dict(decision,source_rail=source_rail,target_rail=target_rail,reason='unrecognized'))+"UPDATE source_funds_links SET link_type="+quote(kind)+" WHERE id='custody';")
    conversion=insert('journal_custody_economic_relations',dict(relation_id='f'*64,workspace_id='ws',profile_id='p',relation_kind='conversion',source_transaction_id='out',target_transaction_id='in',source_asset='BTC',target_asset='LBTC',source_amount_msat=1200,target_amount_msat=1000,basis_state='eligible',created_at='fixed'))
    add('custody-conversion',sql=custody+conversion+"UPDATE transactions SET asset='LBTC' WHERE id='in';UPDATE source_funds_links SET link_type='swap',asset='LBTC',from_allocation_amount=1200 WHERE id='custody';")
    cap="DELETE FROM source_funds_links;UPDATE transactions SET amount=5001 WHERE id='in';"
    cap+=''.join(link(f'cap-{i:05}','s-a','in',1,source=True) for i in range(5001))
    add('report-edge-cap',sql=cap)
    cap="DELETE FROM source_funds_links;UPDATE transactions SET amount=5000 WHERE id='in';"
    for i in range(4999):
        source_id=f'cap-source-{i:05}'
        cap+=insert('source_funds_sources',dict(id=source_id,workspace_id='ws',profile_id='p',source_type='income',label=source_id,asset='BTC',amount=2,created_at='fixed',updated_at='fixed'))
        cap+=link(f'node-cap-{i:05}',source_id,'in',1,source=True)
    cap+=link('node-cap-last','cap-source-00000','in',1,source=True)
    add('report-node-cap-with-retained-duplicate',sql=cap)
    structural = structural_fixture()
    for name,change in {'current':'','no-authority':'DELETE FROM chain_observation_provenance;', 'parent-excluded':"UPDATE transactions SET excluded=1 WHERE id='p0';",'privacy':"UPDATE transactions SET privacy_boundary='coinjoin' WHERE id='p0';",'contradicted-inventory':"UPDATE wallet_utxos SET asset='LBTC' WHERE txid="+quote('a1'*32)+';', 'wrong-confidence':"UPDATE source_funds_links SET confidence='likely';", 'wrong-gross':"UPDATE source_funds_links SET from_allocation_amount=from_allocation_amount+1;",'wallet-samourai':"UPDATE wallets SET config_json="+quote(wire(dict(chain='bitcoin',network='main',samourai=dict(role='child',group_id='g',section='postmix'))))+';'}.items():
        add('structural-'+name,dict(target_transaction='spend'),sql=change,seed=structural)
    return result


def structural_fixture():
    from unittest.mock import patch
    from uuid import UUID
    from tests.test_source_funds_chain_assembly import consolidation_book
    import itertools
    SCRATCH.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(dir=SCRATCH) as temporary:
        fixture=consolidation_book.__wrapped__(Path(temporary))
        conn=next(fixture)
        counter=itertools.count(1)
        with patch('uuid.uuid4',side_effect=lambda:UUID(int=next(counter))),patch.object(sof,'now_iso',return_value='2026-01-02T03:04:05Z'):
            sof.assemble_history(conn,None,None,daemon._source_funds_hooks(),target_transaction_ref='spend')
        script=''
        for table in ['workspaces','profiles','wallets','transactions','chain_observation_provenance','wallet_utxos','source_funds_links']:
            for row in conn.execute('SELECT * FROM '+table):
                fields=dict(row)
                for key in ['created_at','updated_at','observed_at','processed_at']:
                    if key in fields and fields[key]: fields[key]='fixed'
                script+=insert(table,fields)
        script+="INSERT OR REPLACE INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','profile');"
        fixture.close()
        return script


def evaluate(cases):
    SCRATCH.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(dir=SCRATCH) as temporary:
        conn=open_db(Path(temporary)/'data')
        result=[]
        for case in cases:
            conn.executescript('SAVEPOINT report_case;'+case['sql'])
            before=tables(conn)
            expected=outcome(conn,case['command'],json.loads(case['args_json']))
            assert tables(conn)==before,case['name']
            if case['name']=='partial-inexact-ratio':
                assert any(item['code']=='ambiguous_allocation' for item in expected['ok']['findings'])
                assert expected['ok']['graph']['edges']==[]
            if case['name']=='report-node-cap-with-retained-duplicate':
                codes=[item['code'] for item in expected['ok']['findings']]
                assert codes.index('path_truncated')<codes.index('ambiguous_allocation')
                assert len(expected['ok']['graph']['edges'])==4999
            if case['name']=='privacy-fee-reference-bug':
                assert expected['ok']['allocations']['route_difference_explanation']==dict(kind='network_fees',fee_msat=123,fee=1.23e-9,hop_count=1)
            if case['name'] in ['custody-current','custody-grouped','reconciliation-current','structural-current']:
                report=expected['ok']
                codes={f['code'] for f in report['findings']}
                assert not {'stale_custody_component_lineage','stale_structural_lineage'} & codes,(case['name'],report['findings'])
                assert report['graph']['edges'],case['name']
            result.append(wire(expected))
            conn.execute('ROLLBACK TO report_case');conn.execute('RELEASE report_case')
        conn.close()
    return result


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--write',action='store_true');parser.add_argument('--child',action='store_true');args=parser.parse_args()
    fixtures=cases()
    if args.child:
        print(wire(evaluate(fixtures)));return
    variants=[]
    for seed in ['0','1','2']:
        result=subprocess.run([sys.executable,'-m','crates.reports.scripts.gen_source_funds_report','--child'],cwd=ROOT,env={**os.environ,'PYTHONHASHSEED':seed},capture_output=True,text=True,timeout=180)
        assert result.returncode==0,result.stderr
        assert not result.stderr,result.stderr
        variants.append(json.loads(result.stdout))
    for index,case in enumerate(fixtures):
        case['expected_variants']=list(dict.fromkeys(variant[index] for variant in variants))
    generated=json.dumps(fixtures,ensure_ascii=True,indent=2)+'\n'
    if args.write:DESTINATION.write_text(generated)
    elif DESTINATION.read_text()!=generated:raise SystemExit('source-funds report vectors differ')
    print(f'{len(fixtures)} complete report cases verified across seeds 0, 1 and 2; all 114 tables unchanged')


if __name__=='__main__':main()

import argparse
import json
from pathlib import Path
import sqlite3
import tempfile
from unittest.mock import patch

from kassiber import daemon
from kassiber.core import source_funds
from kassiber.db import open_db
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_cases import dump, STAMP
from crates.reports.scripts.gen_source_funds_reads import BASE, ROOT

DESTINATION = ROOT / 'crates/reports/tests/data/source_funds_links.json'
BASE_SQL = (ROOT / 'crates/reports/tests/data/book.sql').read_text() + BASE + """
UPDATE source_funds_links SET from_transaction_id='in',to_transaction_id='out',
link_type='self_transfer',asset='BTC',from_asset='BTC',allocation_amount=100001,
from_allocation_amount=100002 WHERE id='l-z';
"""
WRITTEN = ['source_funds_links', 'source_funds_link_attachments']
DEFAULT = dict(to_transaction='out', from_transaction='in', from_source=None,
               link_type='self_transfer', state='reviewed', confidence='strong',
               method='manual', asset=None, allocation_amount='0.00000100001',
               from_asset=None, from_allocation_amount='0.00000100002',
               allocation_policy='explicit', explanation='Provenance Ω, never ownership',
               uses_chain_observation=False, chain_data_confirmed=False)


def outcome(conn, case):
    calls = []
    def identity():
        calls.append('uuid')
        return case.get('id', 'new-link')
    def now():
        calls.append('now')
        return STAMP
    with patch.object(source_funds.uuid, 'uuid4', identity), patch.object(source_funds, '_now', now):
        try:
            command, args = case['command'], case['args']
            if command.startswith('ui.'):
                data = daemon._ui_source_funds_payload_from_conn(conn, command, daemon._coerce_args_dict(None, args))
            else:
                fields = dict(args)
                workspace, profile = fields.pop('workspace', None), fields.pop('profile', None)
                action = command.rsplit('.', 1)[1]
                if action == 'create':
                    for key in ['to_transaction', 'from_transaction', 'from_source']:
                        fields[key + '_ref'] = fields.pop(key, None)
                    fields['attachment_ids'] = fields.pop('attachments', [])
                    data = source_funds.create_link(conn, workspace, profile, daemon._source_funds_hooks(), **fields)
                elif action == 'review':
                    fields['link_ref'] = fields.pop('link')
                    data = source_funds.update_link_review(conn, workspace, profile, daemon._source_funds_hooks(), **fields)
                else:
                    data = source_funds.attach_link_evidence(conn, workspace, profile, daemon._source_funds_hooks(), link_ref=args['link'], attachment_id=args['attachment'])
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


def cases():
    result = []
    def add(name, action='create', desktop=True, sql='', identity='new-link', **fields):
        args = dict(DEFAULT) if action == 'create' else dict(link='l-z')
        if action == 'attach':
            args['attachment_id' if desktop else 'attachment'] = 'a-a'
        args.update(fields)
        result.append(dict(name=name, command=f'ui.source_funds.links.{action}' if desktop else f'source-funds.links.{action}', args=args, sql=sql, id=identity))
    add('desktop-create')
    add('cli-create', desktop=False, attachments=['a-a','a-z','a-a','a-orphan'])
    add('create-source', from_transaction=None, from_source='Purchase Ω', link_type='manual_source')
    add('create-attachments', attachment_ids=['a-a','a-z','a-a','a-orphan'])
    add('create-singular', attachment_id='a-a')
    add('create-explicit-empty', attachment_ids=[], attachment_id='missing')
    for key, values in dict(
        link_type=list(source_funds.LINK_TYPES)+[' Self-Transfer ', 'bad','',None,False,42],
        state=list(source_funds.LINK_STATES)+[' Accept ','accepted','reject','bad',' ', '',None,42],
        confidence=list(source_funds.CONFIDENCE_LEVELS)+[' EXACT ','bad',' ', '',None,42],
        allocation_policy=list(source_funds.ALLOCATION_POLICIES)+[' EXPLICIT ','bad',' ', '',None,42],
        to_transaction=['external-in','duplicate','missing','',None,42,' out '],
        from_transaction=['duplicate','missing','out','earn',None,''],
        from_source=['Unknown','Purchase Ω','missing','s-other','s-a'],
        asset=[' l-btc ', '', None,42,'custom Ω'],
        from_asset=['LBTC','',None,42],
        method=['',None,False,42,[],{}],
        explanation=['',None,False,42,[],{}],
        allocation_amount=[None,'','0','-0','0.00000000001','0.000000000005','-0.0000000000049','-0.000000000005','-1','0.00123456789','0.00123456790','92233720.36854775808','1e100','bad','NaN','sNaN','Infinity',True,False,{},[]],
        from_allocation_amount=[None,'','0','-1','1.23456789001','1.23456789002','1e100','bad'],
        uses_chain_observation=[True,None,1,'true',False],
        chain_data_confirmed=[True,None,0,'false',False],
        attachment_ids=[None,[],['a-a','missing'],['a-other'],[None],[False],42,'a-a'],
    ).items():
        for index,item in enumerate(values):
            add(f'create-{key}-{index}', **{key:item})
    for state in source_funds.LINK_STATES:
        add('invalid-create-'+state, state=state, to_transaction='in',from_transaction='out')
    add('create-no-endpoint',from_transaction=None)
    add('create-other-source',from_transaction=None,from_source='s-other')
    add('create-source-after-child',from_transaction=None,from_source='s-a',sql="UPDATE source_funds_sources SET acquired_at='2026-01-01' WHERE id='s-a';")
    add('create-source-unknown-date',from_transaction=None,from_source='s-a',sql="UPDATE source_funds_sources SET acquired_at='wrong' WHERE id='s-a';")
    add('create-timezone-order',sql="UPDATE transactions SET occurred_at='2025-01-01T01:00:00+02:00' WHERE id='in';")
    add('create-no-context',sql="DELETE FROM settings WHERE key='context_profile';")
    add('create-desktop-ignores-scope',workspace='missing',profile='Other')
    add('create-cli-other-scope',desktop=False,profile='Other')
    add('create-id-collision',identity='l-a')
    add('create-deleted-before-return',sql="CREATE TRIGGER delete_link AFTER INSERT ON source_funds_links BEGIN DELETE FROM source_funds_links WHERE id=NEW.id; END;")
    add('create-duplicate',sql="UPDATE source_funds_links SET method='manual' WHERE id='l-z';",attachment_ids=['missing'])
    add('create-duplicate-invalid',sql="UPDATE source_funds_links SET method='manual' WHERE id='l-z';",allocation_amount='2')
    add('create-ambiguous-source',from_transaction=None,from_source='Purchase Ω',sql="UPDATE source_funds_sources SET label='PURCHASE Ω' WHERE id='s-z';")
    add('review-accept','review',state='accepted')
    add('review-cli','review',desktop=False,state='reviewed')
    add('review-no-updates','review')
    for key,values in dict(
        state=['reviewed','suggested','rejected',' Accept ','reject','',None,'bad',42],
        link_type=['swap',' Self-Transfer ','bad','',None,42],
        confidence=['exact',' STRONG ','bad','',None,42],
        allocation_policy=['explicit',' HEURISTIC ','bad','',None,42],
        allocation_amount=[None,'','0','0.00000100001','0.00123456790','-1','1e100','bad'],
        from_allocation_amount=[None,'','0','1.23456789002','-1','bad'],
        explanation=[None,'','Keep Ω',42,[],{}],
        uses_chain_observation=[None,True,False,42,'true'],
        chain_data_confirmed=[None,True,False,0,'false'],
        expected_allocation_amount_msat=[None,'','100001',100001,100000,'100001.0',100001.9,'١٠٠٠٠١','1_00_001',[],{},False,'bad',float('inf'),float('-inf'),float('nan'),1e100],
        expected_from_allocation_amount_msat=[None,'100002',100001,'bad',True],
    ).items():
        for index,item in enumerate(values):
            add(f'review-{key}-{index}','review',**{key:item})
    add('review-stale-both','review',state='reviewed',expected_allocation_amount_msat=100001,expected_from_allocation_amount_msat=100001)
    add('review-reject-allocation','review',state='rejected',allocation_amount='0')
    add('review-reject-source-allocation','review',state='reject',from_allocation_amount='0')
    add('review-reject-stale-order','review',state='reject',allocation_amount='0',expected_allocation_amount_msat=0)
    add('review-validation-order','review',confidence='bad',allocation_amount='bad',allocation_policy='bad')
    add('review-amount-policy-order','review',allocation_amount='bad',allocation_policy='bad')
    add('review-source-future','review',link='l-a',state='reviewed',sql="UPDATE source_funds_sources SET acquired_at='2026-01-01' WHERE id='s-a';")
    add('review-self-loop','review',state='reviewed',sql="UPDATE source_funds_links SET from_transaction_id='out' WHERE id='l-z';")
    add('review-future-parent','review',state='reviewed',sql="UPDATE transactions SET occurred_at='2026-01-01' WHERE id='in';")
    add('review-parent-bound','review',state='reviewed',sql="UPDATE source_funds_links SET from_allocation_amount=123456789002 WHERE id='l-z';")
    add('review-parent-fallback-bound','review',state='reviewed',sql="UPDATE transactions SET amount=1 WHERE id='in'; UPDATE source_funds_links SET from_allocation_amount=NULL WHERE id='l-z';")
    add('review-self-asset','review',state='reviewed',sql="UPDATE transactions SET asset='LBTC' WHERE id='in';")
    add('review-link-asset','review',state='reviewed',sql="UPDATE source_funds_links SET asset='LBTC' WHERE id='l-z';")
    add('review-from-asset','review',state='reviewed',sql="UPDATE source_funds_links SET from_asset='LBTC' WHERE id='l-z';")
    add('review-from-asset-fallback','review',state='reviewed',sql="UPDATE source_funds_links SET from_asset=NULL WHERE id='l-z';")
    add('review-suggested-allocation-bound','review',allocation_amount='2')
    add('review-rejected-state-preserved','review',link='l-old',explanation='Exact edit')
    for key in ['allocation_amount','from_allocation_amount']:
        add('review-preserve-'+key,'review',state='reviewed',sql=f"UPDATE source_funds_links SET {key}=1 WHERE id='l-z';")
    for action in ['review','attach']:
        add('desktop-'+action,action,**({'state':'reviewed'} if action=='review' else {}))
        add('cli-'+action,action,desktop=False,**({'state':'reviewed'} if action=='review' else {}))
        for index,ref in enumerate(['missing','l-other',' l-z ','',None,False,42]):
            add(f'{action}-ref-{index}',action,link=ref)
    for index,item in enumerate(['a-other','missing',' a-a ','a-orphan',None,False,42]):
        add(f'attach-evidence-{index}','attach',attachment_id=item)
    add('attach-repeat','attach',link='l-a')
    for action,table,verb in [('create','source_funds_links','INSERT'),('create','source_funds_link_attachments','INSERT'),('review','source_funds_links','UPDATE'),('attach','source_funds_link_attachments','INSERT')]:
        for refusal in ['ABORT','ROLLBACK']:
            add(f'{action}-{table}-{refusal}',action,sql=f"CREATE TRIGGER refuse_write BEFORE {verb} ON {table} BEGIN SELECT RAISE({refusal},'synthetic late refusal'); END;",**(dict(attachment_ids=['a-a']) if action=='create' else dict(state='reviewed') if action=='review' else {}))
        add(f'{action}-{table}-deferred',action,sql=f"CREATE TABLE commit_failure(id TEXT REFERENCES profiles(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER refuse_commit AFTER {verb} ON {table} BEGIN INSERT INTO commit_failure VALUES('missing'); END;",**(dict(attachment_ids=['a-a']) if action=='create' else dict(state='reviewed') if action=='review' else {}))
    for action in ['create','review','attach']:
        for index,args in enumerate([None,[],False,42,'wrong']):
            result.append(dict(name=f'{action}-args-{index}',command=f'ui.source_funds.links.{action}',args=args,sql='',id='new-link'))
    return result


def generate():
    scratch = ROOT / 'target/r5-sof-mutations-3/scratch'
    scratch.mkdir(parents=True,exist_ok=True)
    result = cases()
    with tempfile.TemporaryDirectory(prefix='link-oracle-',dir=scratch) as directory:
        template = open_db(Path(directory)/'data')
        template.executescript(BASE_SQL)
        template.commit()
        schema = '\n'.join(template.iterdump())
        for case in result:
            conn = sqlite3.connect(':memory:')
            conn.row_factory = sqlite3.Row
            template.backup(conn)
            conn.execute('PRAGMA foreign_keys=ON')
            conn.executescript(case['sql'])
            before = dump(conn) if 'CREATE TABLE' not in case['sql'] else None
            case['expected'] = outcome(conn,case)
            after = dump(conn) if before is not None else None
            if before is not None:
                assert {k:v for k,v in before.items() if k not in WRITTEN} == {k:v for k,v in after.items() if k not in WRITTEN},case['name']
                if 'error' in case['expected']:
                    assert before == after,case['name']
            case['tables'] = {table:[dict(row) for row in conn.execute(f'SELECT * FROM {table} ORDER BY rowid')] for table in WRITTEN}
            conn.close()
        template.close()
        from sqlcipher3 import dbapi2 as cipher
        for case in result:
            conn = cipher.connect(':memory:')
            conn.row_factory = cipher.Row
            conn.executescript(schema)
            conn.execute('PRAGMA foreign_keys=ON')
            conn.executescript(case['sql'])
            expected = outcome(conn,case)
            if expected != case['expected']:
                case['expected_cipher'] = expected
            conn.close()
    return json.dumps(dict(base_sql=BASE_SQL,cases=result),ensure_ascii=True,indent=2)+'\n'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--write',action='store_true')
    args = parser.parse_args()
    generated = generate()
    if args.write:
        DESTINATION.write_text(generated)
    else:
        assert DESTINATION.read_text() == generated
    print(f"source-funds link oracle: {len(json.loads(generated)['cases'])} cases")


if __name__ == '__main__':
    main()

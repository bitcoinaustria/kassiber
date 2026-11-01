from __future__ import annotations

import copy
import json
from pathlib import Path
import sys
from types import SimpleNamespace
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_index import database
from generate_analysis_durable import capture, canonical
from kassiber.core import chain_analysis_api as api, chain_analysis_cases as cases
from kassiber.core import chain_analysis_datasets as datasets
from kassiber.core.chain_analysis import projection_store, run_analysis
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError

DEST = ROOT / 'crates/wallets/tests/data/analysis_ops.json'
NOW = '2026-01-01T00:00:00Z'
FIRST = '00000000-0000-4000-8000-000000000001'
SECOND = '00000000-0000-4000-8000-000000000002'


class Sources:
    def __init__(self):
        self.counter = 0

    def id(self):
        self.counter += 1
        return f'00000000-0000-4000-8000-{self.counter:012x}'

    def uuid(self):
        ident = self.id()
        return SimpleNamespace(hex=ident.replace('-', ''))

    def label_uuid(self):
        source = self
        class Identity:
            hex = '0123456789abcdef0123456789abcdef'
            def __str__(self):
                return source.id()
        return Identity()


def evaluate(case, conn):
    source = Sources()
    result = []
    for action in case['actions']:
        operation = action['operation']
        if operation == 'sql':
            conn.executescript(action['sql'])
            value = None
        elif operation == 'begin':
            conn.execute('BEGIN')
            value = None
        elif operation == 'rollback':
            conn.rollback()
            value = None
        else:
            stamp = action.get('as_of', NOW)
            with patch.object(cases, 'now_iso', return_value=stamp), patch.object(datasets, 'now_iso', return_value=stamp), \
                 patch.object(projection_store, 'now_iso', return_value=stamp), \
                 patch.object(cases, 'uuid', SimpleNamespace(uuid4=source.label_uuid)), \
                 patch.object(projection_store, 'uuid', SimpleNamespace(uuid4=source.uuid)):
                try:
                    if case.get('caller_owned'):
                        args = action.get('args', {})
                        functions = {'labels.upsert': cases.upsert_label, 'labels.import': cases.import_labels, 'labels.delete': cases.delete_label, 'cases.save': cases.save_case}
                        if operation in functions:
                            payload = functions[operation](conn, case.get('profile', 'p'), args)
                        elif operation == 'cases.delete':
                            payload = cases.delete_case(conn, case.get('profile', 'p'), args['id'])
                        elif operation == 'datasets.revoke':
                            payload = datasets.revoke_dataset(conn, case.get('profile', 'p'), args['id'], args['expected_revision'])
                        else:
                            payload = datasets.discard_dataset(conn, case.get('profile', 'p'), args['id'])
                    else:
                        payload = api.dispatch(conn, 'ui.chain_analysis.' + operation, action.get('args'), profile=case.get('profile', 'p'))
                    value = build_envelope('ui.chain_analysis.' + operation, payload)
                except AppError as e:
                    value = build_error_envelope(e.code, str(e), e.details, e.hint, e.retryable)
                except (RuntimeError, OverflowError, UnicodeEncodeError, ValueError) as e:
                    value = build_error_envelope('internal_error', str(e))
        state = capture(conn)
        assert state['table_count'] == 112
        result.append({'result': value, 'state': state, 'in_transaction': conn.in_transaction})
    return result


def label(**kwargs):
    return {'subject': 'a' * 64, 'chain': 'bitcoin', 'network': 'main', 'label': 'Counterparty €', 'category': 'exchange', 'source': 'Reviewed statement', 'confidence': 'user_confirmed', **kwargs}


def dataset_sql(status='active', claims=0):
    sql = "INSERT INTO chain_analysis_datasets(id,profile_id,dataset_key,version,chain,network,visibility,status,manifest_json,created_at,updated_at) VALUES('pack','p','example','1','bitcoin','main','public','" + status + "','{\"valid_from\":null,\"valid_until\":null,\"source\":\"fixture\",\"attribution_method\":\"declared\",\"license\":\"test\"}','fixed','fixed');"
    if claims:
        sql += "WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<" + str(claims) + ") INSERT INTO chain_analysis_dataset_claims(dataset_id,record_number,subject,label,category,source_record_json) SELECT 'pack',n,'tx:" + 'a'*64 + "','Entity','other','{}' FROM numbers;"
    return sql


def vectors():
    corpus = []
    def add(group, name, actions, **kwargs):
        case = {'group': group, 'name': name, 'actions': copy.deepcopy(actions), **kwargs}
        conn = database(case.get('sql', ''))
        try:
            case['expected'] = evaluate(case, conn)
        finally:
            conn.close()
        corpus.append(case)
    def action(op, args=None, **kwargs):
        return {'operation': op, 'args': args, **kwargs}
    lifecycle = [action('labels.upsert', label()), action('labels.upsert', label(id=FIRST, expected_revision=1, label='Corrected', cluster_defining=True)),
        action('labels.upsert', label(id=FIRST, expected_revision=1)), action('labels.delete', {'id': FIRST, 'expected_revision': True}),
        action('labels.delete', {'id': FIRST, 'expected_revision': 2}), action('labels.delete', {'id': FIRST, 'expected_revision': 3}),
        action('labels.upsert', label(id=FIRST, expected_revision=3))]
    add('labels', 'revisioned-lifecycle', lifecycle)
    for field, values in {
        'chain': [None, [], '', 'liquid', 'unsupported', ' bitcoin '],
        'network': [None, {}, '', 'signet', 'test', 'regtest', 'main '],
        'subject': ['', None, 1, 'a'*63, 'b'*64+':00', 'b'*64+':4294967296', 'b'*64+':9999999999', 'b'*64+':10000000000', 'tx:'+'A'*64,
            'out:'+'A'*64+':0', 'bitcoin:main:tx:'+'A'*64, 'bitcoin:regtest:tx:'+'a'*64, 'wpkh(xprv-secret)', 'a'*64+'\n', '1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa'],
        'category': ['service', 'mixer', 'self', 'merchant', 'other', 'unknown', None],
        'confidence': ['imported', 'unverified', '', None, 'unsupported'],
        'cluster_defining': [True, False, 1, 0, None, 'false', []],
        'label': ['  Revised \u2003', 'x'*256, 'x'*257, '', '  ', None, 'x\n', '\u001c x'],
        'source': ['x'*512, 'x'*513],
    }.items():
        for i, value in enumerate(values):
            add('labels', f'{field}-{i}', [action('labels.upsert', label(**{field: value}))])
    for i, args in enumerate([None, [], {}, {'unknown': True}, label(id='absent', expected_revision=1), label(expected_revision=1), label(id='', expected_revision=1), label(expected_scope={'workspace_id':'ws','profile_id':'other'}), label(expected_scope={'workspace_id':'ws','profile_id':'p'})]):
        add('labels', f'arguments-{i}', [action('labels.upsert', args)])
    for chain, network, subject in [('bitcoin','test','1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa'), ('liquid','signet','a'*64), ('liquid','main','liquid:liquidv1:tx:'+'A'*64), ('liquid','regtest','liquid:elementsregtest:out:'+'b'*64+':0')]:
        add('labels', f'domain-{chain}-{network}', [action('labels.upsert',label(chain=chain,network=network,subject=subject))])
    from embit import base58, script, ec
    from embit.liquid.addresses import address
    from embit.liquid.networks import NETWORKS
    for network, name, prefix in [('main','liquidv1',57),('test','liquidtestnet',36),('regtest','elementsregtest',235)]:
        for i, subject in enumerate([
            address(script.Script(bytes.fromhex('0014'+'11'*20)), network=NETWORKS[name]),
            address(script.Script(bytes.fromhex('a914'+'11'*20+'87')), network=NETWORKS[name]),
            address(script.Script(bytes.fromhex('a914'+'11'*20+'87')), blinding_key=ec.PrivateKey(b'\x01'*32).get_public_key(), network=NETWORKS[name]),
            base58.encode_check(bytes([prefix])+b'\x11'*20),
            base58.encode_check(bytes([NETWORKS[name]['p2sh'][0]])+b'\x11'*19),
            base58.encode_check(b'\xc4'+b'\x11'*20),
        ]):
            add('labels',f'liquid-address-{network}-{i}',[action('labels.upsert',label(chain='liquid',network=network,subject=subject))])
    for i, items in enumerate([[], {}, None, [label(), label(chain='unsupported')], [label(), {'id': FIRST}], [label(), {'expected_revision': None}], [label(), False], [label(), label(subject='b'*64)]]):
        add('imports', f'atomic-import-{i}', [action('labels.import', {'items': items})])
    add('imports', 'import-limit', [action('labels.import', {'items': [None]*5001})])
    add('rollback', 'caller-import-rollback', [{'operation': 'begin'}, action('labels.import', {'items':[label(),label(subject='b'*64)]}), {'operation':'rollback'}], caller_owned=True)
    add('rollback', 'caller-failed-import', [{'operation':'begin'}, action('labels.upsert',label()), action('labels.import',{'items':[label(),label(chain='bad')]}), {'operation':'rollback'}], caller_owned=True)
    add('labels', 'foreign-profile', [action('labels.delete', {'id':FIRST,'expected_revision':1})], profile='other', sql="INSERT INTO chain_analysis_labels VALUES('"+FIRST+"','p','bitcoin','main','"+'a'*64+"','Claim','other','local','unverified',0,1,0,'fixed','fixed');")
    for field in ['label','source','subject','category','confidence']:
        for value in ['\ud800','x\ud800\ud801y','  \ud800  ']:
            add('labels',f'surrogate-{field}-{len(corpus)}',[action('labels.upsert',label(**{field:value}))])
    for operation in ['labels.upsert','labels.delete']:
        args=label(id=FIRST,expected_revision=9223372036854775807) if operation.endswith('upsert') else {'id':FIRST,'expected_revision':9223372036854775807}
        add('labels','revision-overflow-'+operation,[action(operation,args)],sql="INSERT INTO chain_analysis_labels VALUES('"+FIRST+"','p','bitcoin','main','"+'a'*64+"','Claim','other','local','unverified',0,9223372036854775807,0,'fixed','fixed');")
    for status in ['active','superseded','revoked','staging','failed']:
        for revision in [1, True, 0, None]:
            add('datasets', f'revoke-{status}-{revision}', [action('datasets.revoke', {'id':'pack','expected_revision':revision})], sql=dataset_sql(status,3))
        add('datasets', f'discard-{status}', [action('datasets.discard', {'id':'pack'})], sql=dataset_sql(status,2001 if status=='failed' else 3))
    add('rollback', 'dataset-caller-rollback', [{'operation':'begin'}, action('datasets.revoke',{'id':'pack','expected_revision':1}), {'operation':'rollback'}], caller_owned=True, sql=dataset_sql())
    add('rollback', 'dataset-busy', [{'operation':'begin'}, action('datasets.discard',{'id':'pack'}), {'operation':'rollback'}], caller_owned=True, sql=dataset_sql('staging'))
    for operation in ['datasets.revoke','datasets.discard','cases.delete']:
        add('datasets' if operation.startswith('datasets.') else 'cases', operation+'-missing', [action(operation, {'id':'absent', **({'expected_revision':1} if operation.endswith('revoke') else {})})])
    for observer in ['owner','public','disclosed']:
        conn = database()
        try:
            with patch.object(projection_store,'now_iso',return_value=NOW), patch.object(projection_store,'uuid',SimpleNamespace(uuid4=lambda:SimpleNamespace(hex=FIRST.replace('-','')))):
                snapshot = run_analysis(conn,'p',{'observer':observer})['snapshot_id']
        finally:
            conn.close()
        saved = {'title':' Investigation € ', 'query':{'observer':observer}, 'expected_snapshot_id':snapshot}
        add('cases', 'save-delete-'+observer, [action('cases.save',saved), action('cases.delete',{'id':SECOND}),action('cases.delete',{'id':SECOND})])
        add('rollback', 'case-caller-rollback-'+observer, [{'operation':'begin'},action('cases.save',saved),{'operation':'rollback'}], caller_owned=True)
    for args in [{'title':'Stale','query':{},'expected_snapshot_id':'stale'}, {'title':'','query':{},'expected_snapshot_id':'stale'}, {'title':'Invalid','query':{'unknown':True},'expected_snapshot_id':'stale'}]:
        add('cases','invalid-'+str(len(corpus)), [action('cases.save',args)])
    add('cases', 'corrupt-case-delete', [action('cases.delete',{'id':'corrupt'})], sql="INSERT INTO chain_analysis_cases VALUES('corrupt','p','Corrupt','{}','{\"query\":{},\"snapshot_id\":\"snapshot\"}','snapshot','wrong','fixed');")
    for case in corpus:
        if case['name'].startswith('save-delete-'):
            assert case['expected'][0]['result']['kind']!='error',case['name']
    return corpus


if __name__ == '__main__':
    DEST.write_text(json.dumps(vectors(), ensure_ascii=True, separators=(',', ':'))+'\n')

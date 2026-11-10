"""Unchanged-Python oracle for recurring acquisition grants; no source is contacted."""
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
from generate_analysis_durable import capture
from generate_analysis_index import SEED_TABLE_COUNT
from generate_analysis_ops import NOW, Sources
from generate_analysis_watches import BIND, database
from kassiber import egress_policy
from kassiber.core import chain_analysis_api as api, chain_analysis_backfill as backfill
from kassiber.core.chain_analysis import projection_store
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError

DEST = ROOT / 'crates/wallets/tests/data/analysis_grants.json'
EPOCH = 1767225600
TXID = 'a1' * 32


def backend(name='core', kind='bitcoinrpc', chain='bitcoin', network='main', url='http://127.0.0.1:8332/wallet', config='{}', timeout='NULL', proxy='NULL'):
    return ("INSERT INTO backends(name,kind,chain,network,url,timeout,tor_proxy,config_json,created_at,updated_at) "
            f"VALUES('{name}','{kind}',{chain if chain == 'NULL' else repr(chain)},{network if network == 'NULL' else repr(network)},'{url}',{timeout},{proxy},'{config}','fixed','fixed');")


BOOK = BIND + backend()


def evaluate(case, conn, resolve=None):
    source = Sources()
    results = []
    for action in case['actions']:
        if resolve is not None and callable(action.get('args')):
            action['args'] = action['args'](results)
        operation = action['operation']
        if operation == 'sql':
            conn.executescript(action['sql'])
            value = None
        elif operation in {'begin', 'rollback'}:
            conn.execute('BEGIN') if operation == 'begin' else conn.rollback()
            value = None
        else:
            epoch = action.get('epoch', EPOCH)
            with patch.object(backfill, 'now_iso', return_value=NOW), patch.object(projection_store, 'now_iso', return_value=NOW), \
                 patch.object(backfill, 'uuid', SimpleNamespace(uuid4=source.uuid)), \
                 patch.object(projection_store, 'uuid', SimpleNamespace(uuid4=source.uuid)), \
                 patch.object(backfill, 'time', SimpleNamespace(time=lambda: epoch)), \
                 patch.object(egress_policy, 'offline_mode_enabled', return_value=bool(case.get('offline'))):
                try:
                    profile = case.get('profile', 'p')
                    if case.get('caller_owned'):
                        payload = backfill.dispatch(conn, profile, operation, copy.deepcopy(action.get('args') or {}))
                    else:
                        payload = api.dispatch(conn, 'ui.chain_analysis.' + operation, copy.deepcopy(action.get('args')), profile=profile)
                    value = build_envelope('ui.chain_analysis.' + operation, payload)
                except AppError as e:
                    value = build_error_envelope(e.code, str(e), e.details, e.hint, e.retryable)
                except (RuntimeError, OverflowError, UnicodeEncodeError, ValueError, TypeError) as e:
                    value = build_error_envelope('internal_error', str(e))
        state = capture(conn)
        assert state['table_count'] == SEED_TABLE_COUNT
        results.append({'result': value, 'state': state, 'in_transaction': conn.in_transaction})
    return results


def data(results, index):
    value = results[index]['result']
    assert value['kind'] != 'error', value
    return value['data']


def vectors():
    corpus = []

    def add(name, actions, **kwargs):
        case = {'group': 'grants', 'name': name, 'encrypted': True, 'sql': BOOK, 'actions': actions, **kwargs}
        conn = database(case['sql'], case['encrypted'])
        try:
            case['expected'] = evaluate(case, conn, resolve=True)
        finally:
            conn.close()
        corpus.append(case)

    def act(op, args=None, **kwargs):
        return {'operation': 'sources.' + op, 'args': args, **kwargs}

    subject = {'backend': 'Core', 'network': 'main', 'mode': 'subject', 'subject': TXID, 'max_requests': 10, 'max_bytes': 8 * 1024 * 1024}
    blocks = {'backend': 'core', 'network': 'main', 'mode': 'blocks', 'start_height': 100, 'end_height': 120, 'max_requests': 100, 'max_bytes': 10 ** 9,
              'interval_seconds': 60, 'blocks_per_run': 2, 'duration_days': 2}
    plans = [None, [], {}, subject, blocks, {**blocks, 'chain_instance_id': None}, {**subject, 'chain': 'liquid'}, {**subject, 'network': 'mainnet'},
             {**subject, 'network': []}, {**subject, 'mode': {}}, {**subject, 'mode': 'bad'}, {**subject, 'backend': ' '}, {**subject, 'backend': 7}, {**subject, 'unknown': 1}]
    for name, low, high in (('interval_seconds', 30, 86400), ('duration_days', 1, 365), ('max_requests', 4, 100000000), ('max_bytes', 8 * 1024 * 1024, 10 ** 13), ('blocks_per_run', 1, 16)):
        plans += [{**subject, name: low - 1}, {**subject, name: high + 1}, {**subject, name: True}, {**subject, name: str(low)}, {**subject, name: high}]
    plans += [{**subject, 'subject': 'A1' * 32}, {**subject, 'subject': None}, {**subject, 'start_height': 1}, {**subject, 'end_height': 1},
              {**blocks, 'subject': TXID}, {**blocks, 'start_height': -1}, {**blocks, 'start_height': True}, {**blocks, 'start_height': 1.0},
              {**blocks, 'end_height': 99}, {**blocks, 'end_height': 'x'}, {**blocks, 'end_height': 100}, {**subject, 'backend': 'absent'},
              {**subject, 'network': 'test'}, {**subject, 'chain_instance_id': 'lab'}, {**subject, 'backend': 'esplora'}, {**subject, 'backend': 'testnet'},
              {**subject, 'backend': 'instance'}, {**subject, 'backend': 'proxied'}, {**subject, 'backend': 'badport'}, {**subject, 'backend': 'badtimeout'},
              {**subject, 'backend': 'v6'}, {**subject, 'backend': 'nochain'}]
    extra = (backend('esplora', 'esplora', url='https://blockstream.info/api') + backend('testnet', network='test') + backend('instance', config='{"chain_instance_id":"lab"}')
             + backend('proxied', url=' HTTP://User:Pw@Node.Example:08332/path?x=1#y', config='{"certificate":"/ca.pem","insecure":true,"timeout":" 12 "}', proxy="'socks5h://127.0.0.1:9050'")
             + backend('badport', url='http://node:99999') + backend('badtimeout', timeout="'soon'") + backend('v6', url='http://[::1]:8332') + backend('nochain', chain='NULL'))
    add('plan-arguments', [act('plan', args) for args in plans], sql=BOOK + extra)
    add('plan-unbound', [act('plan', subject)], sql=backend())
    add('plaintext-authorize', [act('plan', subject), act('authorize', lambda r: {'plan': data(r, 0)}), act('list', {})], encrypted=False)
    lifecycle = [act('plan', subject), act('authorize', {}), act('authorize', {'plan': []}), act('authorize', lambda r: {'plan': {**data(r, 0), 'plan_id': 'stale'}}),
                 act('authorize', lambda r: {'plan': {k: v for k, v in data(r, 0).items() if k != 'spec'}}),
                 act('authorize', lambda r: {'plan': data(r, 0)}), act('plan', blocks), act('authorize', lambda r: {'plan': data(r, 6)}, epoch=EPOCH + 5),
                 act('list', {}), act('list', {'limit': 1}), act('list', lambda r: {'after_id': data(r, 5)['id']}), act('list', {'limit': 0}), act('list', {'after_id': 'x' * 65}),
                 act('list', {'after_id': 1}), act('list', {'cursor': 'x'}),
                 act('run', lambda r: {'id': data(r, 5)['id']}, epoch=EPOCH + 60), act('run', {'id': 'absent'}), act('run', {}),
                 act('revoke', lambda r: {'id': data(r, 5)['id'], 'expected_revision': 2}), act('revoke', lambda r: {'id': data(r, 5)['id'], 'expected_revision': True}),
                 act('revoke', lambda r: {'id': data(r, 5)['id'], 'expected_revision': 1}), act('revoke', lambda r: {'id': ' ' + data(r, 5)['id'], 'expected_revision': 2}),
                 act('run', lambda r: {'id': data(r, 5)['id']}),
                 act('run', lambda r: {'id': data(r, 7)['id']}, epoch=EPOCH + 5 + 2 * 86400),
                 act('run', lambda r: {'id': data(r, 7)['id']}, epoch=EPOCH + 4 + 2 * 86400),
                 {'operation': 'sql', 'sql': "UPDATE backends SET updated_at='changed';"}, act('run', lambda r: {'id': data(r, 7)['id']}),
                 {'operation': 'sql', 'sql': "UPDATE backends SET updated_at='fixed';UPDATE chain_analysis_acquisition_grants SET binding_json='{}';"},
                 act('run', lambda r: {'id': data(r, 7)['id']}),
                 {'operation': 'sql', 'sql': "UPDATE settings SET value='" + 'f' * 32 + "' WHERE key='database_instance_id';"}, act('run', lambda r: {'id': data(r, 7)['id']}),
                 act('list', {})]
    add('grant-lifecycle', lifecycle)
    add('offline-run', [act('plan', subject), act('authorize', lambda r: {'plan': data(r, 0)}), act('run', lambda r: {'id': data(r, 1)['id']}), act('list', {})], offline=True)
    add('removed-source', [act('plan', subject), act('authorize', lambda r: {'plan': data(r, 0)}), {'operation': 'sql', 'sql': "DELETE FROM backends;"},
                           act('run', lambda r: {'id': data(r, 1)['id']}), act('plan', subject)])
    add('foreign-profile', [act('list', {}), act('revoke', {'id': 'g', 'expected_revision': 1}), act('run', {'id': 'g'})], profile='other', sql=BOOK +
        "INSERT INTO chain_analysis_acquisition_grants(id,profile_id,spec_json,binding_json,source_fingerprint,database_identity,status,next_run_at,expires_at,created_at) VALUES('g','p','{}','{}','f','d','active',0,0,'fixed');")
    add('caller-rollback', [{'operation': 'begin'}, act('plan', subject), act('authorize', lambda r: {'plan': data(r, 1)}),
                            act('run', lambda r: {'id': data(r, 2)['id']}), {'operation': 'rollback'}, act('list', {})], caller_owned=True)
    return corpus


if __name__ == '__main__':
    DEST.write_text(json.dumps(vectors(), ensure_ascii=True, separators=(',', ':')) + '\n')

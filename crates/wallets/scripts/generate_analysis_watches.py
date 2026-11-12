"""Unchanged-Python oracle for local evidence watches over keyed SQLCipher books."""
from __future__ import annotations

import copy
import json
from pathlib import Path
import sys
from types import SimpleNamespace
from unittest.mock import patch

from sqlcipher3 import dbapi2 as sqlcipher

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_index import BOOK, SEED, SEED_TABLE_COUNT
from generate_analysis_durable import capture
from generate_analysis_ops import NOW, Sources, label
from kassiber.core import chain_analysis_api as api, chain_analysis_cases as cases
from kassiber.core import chain_analysis_watches as watches, filed_report_chain
from kassiber.core.chain_analysis import projection_store
from kassiber.envelope import build_envelope, build_error_envelope
from kassiber.errors import AppError

DEST = ROOT / 'crates/wallets/tests/data/analysis_watches.json'
KEY = 'analysis-watch-fixture'
A, B, C, PREV = 'a1' * 32, 'b2' * 32, 'c4' * 32, 'c3' * 32
BLOCK, TIP = 'bb' * 32, 'cc' * 32
BIND = ("INSERT INTO book_network_bindings(profile_id,environment_id,environment,chain_instance_id,domains_json,acknowledgements_json,inventory_digest,created_at) VALUES('p','env','main',NULL,"
        "'[{\"chain\":\"bitcoin\",\"network\":\"main\",\"domain_id\":\"bound-main\",\"chain_instance_id\":null},{\"chain\":\"liquid\",\"network\":\"liquidv1\",\"domain_id\":\"bound-liquid\",\"chain_instance_id\":null}]','[]','d','fixed');")


def tx(txid, inputs, outputs):
    return json.dumps({'txid': txid, 'vin': [{'txid': t, 'vout': n} for t, n in inputs],
                       'vout': [{'n': n, 'value': 5000 - 100 * n, 'scriptpubkey': '0014' + f'{n + 1:02x}' * 20} for n in range(outputs)]})


def observation(txid, inputs, outputs, confirmations=0):
    status = json.dumps({'confirmed': confirmations > 0, 'confirmations': confirmations})
    return ("INSERT INTO chain_analysis_observations(profile_id,chain,network,txid,payload_json,status_json,source_name,observed_at) "
            f"VALUES('p','bitcoin','main','{txid}','{tx(txid, inputs, outputs)}','{status}','local','fixed');")


GRAPH = BIND + observation(A, [(PREV, 0)], 2, 3) + observation(B, [(A, 0)], 1)
SPEND = observation(C, [(A, 1)], 1)
CONFIRMED = BIND + (
    "INSERT INTO chain_analysis_acquisition_grants(id,profile_id,spec_json,binding_json,source_fingerprint,database_identity,status,next_run_at,expires_at,verified_tip_height,verified_tip_hash,verified_tip_at,created_at) "
    f"VALUES('g','p','{{}}','{{}}','f','0123456789abcdef0123456789abcdef','active',0,0,101,'{TIP}','2026-01-01T00:00:00Z','fixed');"
    f"INSERT INTO chain_analysis_reference_assertions VALUES('g','p','bound-main','bitcoin','main','{BLOCK}:0','{A}','{tx(A, [(PREV, 0)], 1)}',"
    f"'{json.dumps({'confirmed': True, 'block_hash': BLOCK, 'block_height': 100})}',1,'2026-01-01T00:00:00Z');")
REPORT = ("INSERT INTO filed_report_snapshots(id,workspace_id,profile_id,report_kind,report_state,period_start_year,period_end_year,content_sha256,gain_summary_json,report_scope_json,created_at) "
          "VALUES('snap','ws','p','austrian','filed',2025,2025,'" + 'e' * 64 + "','{\"gain\":\"1\"}','broken','fixed');"
          "INSERT INTO filed_report_chain_dependencies VALUES('snap','p','in','cold','bitcoin','main','" + A + "',"
          "'{\"status\":\"observed\",\"confirmed\":true,\"block_hash\":\"" + BLOCK + "\",\"domain_id\":\"bound-main\"}');")


def database(sql='', encrypted=True):
    conn = (sqlcipher if encrypted else __import__('sqlite3')).connect(':memory:')
    if encrypted:
        conn.execute(f"PRAGMA key='{KEY}'")
    conn.row_factory = (sqlcipher if encrypted else __import__('sqlite3')).Row
    conn.execute('PRAGMA foreign_keys=ON')
    conn.executescript(SEED.read_text())
    conn.executescript(BOOK.read_text())
    conn.executescript(sql)
    return conn


def evaluate(case, conn, resolve=None):
    source = Sources()
    results = []
    for index, action in enumerate(case['actions']):
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
            stamp = action.get('as_of', NOW)
            with patch.object(watches, 'now_iso', return_value=stamp), patch.object(filed_report_chain, 'now_iso', return_value=stamp), \
                 patch.object(cases, 'now_iso', return_value=stamp), patch.object(projection_store, 'now_iso', return_value=stamp), \
                 patch.object(watches, 'uuid', SimpleNamespace(uuid4=source.label_uuid)), \
                 patch.object(filed_report_chain, 'uuid', SimpleNamespace(uuid4=source.label_uuid)), \
                 patch.object(cases, 'uuid', SimpleNamespace(uuid4=source.label_uuid)), \
                 patch.object(projection_store, 'uuid', SimpleNamespace(uuid4=source.uuid)):
                try:
                    profile = case.get('profile', 'p')
                    if case.get('caller_owned'):
                        payload = watches.dispatch(conn, profile, operation.removeprefix('watches.'), copy.deepcopy(action.get('args') or {}))
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
        case = {'group': 'watches', 'name': name, 'encrypted': True, 'actions': actions, **kwargs}
        conn = database(case.get('sql', ''), case.get('encrypted', True))
        try:
            case['expected'] = evaluate(case, conn, resolve=True)
        finally:
            conn.close()
        corpus.append(case)

    def act(op, args=None, **kwargs):
        return {'operation': 'watches.' + op if '.' not in op else op, 'args': args, **kwargs}

    spent = {'rule': 'output_spent', 'query': {'subject': f'{A}:1'}}
    plan = lambda i: lambda r: {'plan': data(r, i)}
    add('plaintext-refusals', [act('preview', spent), act('evaluate', {}), act('list', {}), act('inbox', {}), act('create', {'plan': {'definition': {'rule': 'output_spent'}}})],
        sql=GRAPH, encrypted=False)
    invalid = [None, [], {}, {'rule': 'bad'}, {'rule': 1}, {'rule': 'output_spent'}, {'rule': 'output_spent', 'query': {'subject': A}, 'case_id': 'x'},
               {'rule': 'findings_changed', 'query': {'unknown': True}}, {'rule': 'findings_changed', 'query': []}, {'rule': 'output_spent', 'query': {'mode': 'overview'}},
               {'rule': 'connection_supported', 'query': {'mode': 'trace', 'subject': A}}, {'rule': 'findings_changed', 'query': {'mode': 'overview'}, 'threshold': 1},
               *({'rule': 'confirmations', 'query': {'subject': A}, 'threshold': t} for t in (0, True, 10001, '2', 1.0)),
               {'rule': 'output_spent', 'query': {'subject': f'bitcoin:main:out:{A}:1', 'chain': 'liquid'}},
               {'rule': 'output_spent', 'query': {'subject': f'bitcoin:test:out:{A}:1', 'network': 'main'}},
               {'rule': 'output_spent', 'query': {'subject': f'bitcoin:bogus:out:{A}:1', 'network': 'main'}},
               {'rule': 'output_spent', 'query': {'subject': f'bitcoin:mainnet:out:{A}:1'}},
               {'rule': 'output_spent', 'query': {'subject': f'liquid:liquidv1:out:{A}:1'}},
               {'rule': 'output_spent', 'case_id': 'absent'}, {'rule': 'output_spent', 'case_id': 7},
               {'rule': 'output_spent', 'query': {'subject': 'f' * 64}}, {'rule': 'findings_changed', 'query': {'chain': 'bitcoin'}},
               {'rule': 'findings_changed', 'query': {'chain': 'liquid', 'network': 'signet'}},
               {'rule': 'output_spent', 'query': {'subject': f'tx:{A}'}}, {'rule': 'confirmations', 'query': {'subject': A, 'mode': 'path', 'target': B}},
               {'rule': 'findings_changed', 'query': {'chain': 'bitcoin', 'mode': 'overview', 'expected_scope': 1}},
               {'rule': 'output_spent', 'query': {'subject': f'{A}:1'}, 'expected_scope': {'workspace_id': 'ws', 'profile_id': 'other'}}]
    add('preview-arguments', [act('preview', args) for args in invalid], sql=GRAPH)
    add('preview-unbound', [act('preview', {'rule': 'findings_changed', 'query': {'mode': 'overview'}}), act('preview', {'rule': 'findings_changed', 'query': {'chain': 'bitcoin'}}),
                            act('preview', {'rule': 'findings_changed', 'query': {'chain': 'bitcoin', 'network': 'main'}}), act('preview', spent)],
        sql=observation(A, [(PREV, 0)], 2))
    lifecycle = [act('preview', spent), act('create', plan(0)), act('create', plan(0)), act('list', {}), act('evaluate', {}),
                 {'operation': 'sql', 'sql': SPEND}, act('evaluate', {}, as_of='2026-01-02T00:00:00Z'), act('evaluate', {}), act('inbox', {}), act('inbox', {'limit': 1}),
                 act('inbox', lambda r: {'before': data(r, 8)['items'][0]['cursor']}), act('inbox', {'limit': 0}), act('inbox', {'limit': True}), act('inbox', {'before': 0}),
                 act('inbox', {'before': 2 ** 63}), act('inbox', {'unknown': 1}),
                 act('acknowledge', lambda r: {'id': data(r, 8)['items'][0]['id']}, as_of='2026-01-03T00:00:00Z'),
                 act('acknowledge', lambda r: {'id': ' ' + data(r, 8)['items'][0]['id'] + ' '}), act('acknowledge', {'id': 'absent'}), act('acknowledge', {'id': ''}),
                 act('inbox', {}), act('configure', lambda r: {'id': data(r, 1)['id'], 'expected_revision': 1, 'enabled': 'false'}),
                 act('configure', lambda r: {'id': data(r, 1)['id'], 'expected_revision': 2, 'enabled': False}),
                 act('configure', lambda r: {'id': data(r, 1)['id'], 'expected_revision': True, 'enabled': False}),
                 act('configure', lambda r: {'id': data(r, 1)['id'], 'expected_revision': 1, 'enabled': False}),
                 act('evaluate', {}), act('list', {}), act('preview', spent), act("create", plan(27)),
                 act('configure', lambda r: {'id': ' ' + data(r, 1)['id'], 'expected_revision': 2, 'enabled': True}),
                 act('configure', lambda r: {'id': data(r, 1)['id'], 'expected_revision': 2, 'enabled': True}),
                 act('delete', lambda r: {'id': data(r, 1)['id'], 'expected_revision': 2}),
                 act('delete', lambda r: {'id': data(r, 1)['id'], 'expected_revision': 3}), act('delete', lambda r: {'id': data(r, 1)['id'], 'expected_revision': 4}),
                 act('inbox', {}), act('list', {}), act('list', {'limit': 1})]
    add('output-spent-lifecycle', lifecycle, sql=GRAPH)
    definition = lambda r: {'definition': data(r, 0)['definition'], 'expected_plan_id': data(r, 0)['plan_id']}
    add('create-by-definition', [act('preview', {**spent, 'query': {'subject': f'{A}:0'}}), act('create', {}), act('create', {'plan': None}), act('create', {'plan': {'definition': []}}),
                                 act('create', lambda r: {**definition(r), 'expected_plan_id': 'stale'}),
                                 act('create', lambda r: {**definition(r), 'definition': {**data(r, 0)['definition'], 'rule_version': 2}}),
                                 act('create', lambda r: {**definition(r), 'definition': {**data(r, 0)['definition'], 'rule_version': True}}),
                                 act('create', lambda r: {**definition(r), 'definition': {**data(r, 0)['definition'], 'extra': 1}}),
                                 act('create', lambda r: {**definition(r), 'definition': {**data(r, 0)['definition'], 'query': {'subject': f'{A}:0'}}}),
                                 act('create', lambda r: {**definition(r), 'definition': {'rule': 'output_spent', 'query': {'subject': f'{A}:0'}}}),
                                 act('create', lambda r: {'plan': {**data(r, 0), 'local_only': False}}),
                                 act('create', lambda r: {**definition(r), 'plan': data(r, 0)}),
                                 act('create', definition), act('list', {})], sql=GRAPH)
    confirmed = {'rule': 'confirmations', 'query': {'subject': A, 'chain': 'bitcoin', 'network': 'main'}, 'threshold': 3}
    tip = lambda height, code='NULL': {'operation': 'sql', 'sql': f"UPDATE chain_analysis_acquisition_grants SET verified_tip_height={height},last_code={code};"}
    add('confirmations-report-impact', [act('preview', confirmed), act('create', plan(0)), tip(102), act('evaluate', {}), tip(101), act('evaluate', {}),
                                        tip(103), act('evaluate', {}), tip(103, "'reorg_reconciling'"), act('evaluate', {}), act('evaluate', {}), act('inbox', {}),
                                        {'operation': 'sql', 'sql': "INSERT INTO filed_report_chain_resolutions SELECT id,'2026-02-01T00:00:00Z','{\"gain\":\"2\"}','fixed' FROM filed_report_chain_impacts;"},
                                        act('inbox', {}), act('list', {})], sql=CONFIRMED + REPORT)
    add('confirmations-default-threshold', [act('preview', {'rule': 'confirmations', 'query': {'subject': f'bitcoin:main:tx:{A}'}}), act('create', plan(0)), tip(99), act('evaluate', {}), act('inbox', {})],
        sql=CONFIRMED)
    add('findings-and-attribution', [act('preview', {'rule': 'findings_changed', 'query': {'mode': 'overview'}}), act('create', plan(0)),
                                     act('preview', {'rule': 'attribution_changed', 'query': {'subject': A, 'mode': 'trace'}}), act('create', plan(2)),
                                     act('preview', {'rule': 'connection_supported', 'query': {'mode': 'path', 'subject': A, 'target': C}}), act('create', plan(4)),
                                     act('labels.upsert', label(subject=A)), {'operation': 'sql', 'sql': SPEND}, act('evaluate', {}), act('inbox', {}), act('list', {})], sql=GRAPH)
    spent0 = {'rule': 'output_spent', 'query': {'subject': f'{A}:0'}}
    add('domain-changed', [act('preview', spent0), act('create', plan(0)), {'operation': 'sql', 'sql': "DELETE FROM book_network_bindings;"},
                           act('evaluate', {}), act('evaluate', {}), act('inbox', {}), act('list', {})], sql=GRAPH)
    add('book-identity-changed', [act('preview', spent0), act('create', plan(0)), {'operation': 'sql', 'sql': "UPDATE chain_analysis_watches SET book_id='" + '0' * 32 + "';"},
                                  act('evaluate', {}), act('inbox', {}), {'operation': 'sql', 'sql': "UPDATE settings SET value='broken' WHERE key='database_instance_id';"},
                                  act('evaluate', {}), act('preview', spent), act('list', {})], sql=GRAPH)
    filler = ''.join(f"INSERT INTO chain_analysis_watches(id,profile_id,book_id,domain_json,definition_json,baseline_json,checkpoint,checked_at,created_at) VALUES('w{n:03d}','p','b','{{}}','{{\"n\":{n}}}','{{}}','','fixed','fixed');" for n in range(250))
    add('watch-limit', [act('preview', spent), act('create', plan(0))], sql=GRAPH + filler)
    add('paused-duplicate', [act('preview', spent), act('create', plan(0)), act('configure', lambda r: {'id': data(r, 1)['id'], 'expected_revision': 1, 'enabled': False}),
                             act('preview', spent), act('create', plan(3))], sql=GRAPH)
    add('case-based', [act('preview', {'rule': 'findings_changed', 'query': {'mode': 'overview'}}),
                       act('cases.save', lambda r: {'title': 'Saved', 'query': {}, 'expected_snapshot_id': data(r, 0)['revision']['snapshot_id']}),
                       act('preview', lambda r: {'rule': 'findings_changed', 'case_id': ' ' + data(r, 1)['id'] + ' '}),
                       act('preview', lambda r: {'rule': 'findings_changed', 'case_id': data(r, 1)['id']}), act('create', plan(3)),
                       act('preview', lambda r: {'rule': 'output_spent', 'case_id': data(r, 1)['id']}), act('list', {})], sql=GRAPH)
    add('foreign-profile', [act('list', {}), act('inbox', {}), act('acknowledge', {'id': 'event'}), act('configure', {'id': 'w', 'expected_revision': 1, 'enabled': False}),
                            act('delete', {'id': 'w', 'expected_revision': 1}), act('evaluate', {})], profile='other',
        sql=GRAPH + "INSERT INTO chain_analysis_watches(id,profile_id,book_id,domain_json,definition_json,baseline_json,checkpoint,checked_at,created_at) VALUES('w','p','b','{}','{}','{}','','fixed','fixed');"
            "INSERT INTO chain_analysis_watch_inbox(id,watch_id,profile_id,sequence,code,observation_json,created_at) VALUES('event','w','p',1,'spend_observed','{}','fixed');")
    add('corrupt-stored-watch', [act('list', {}), act('evaluate', {}), act('inbox', {})], sql=GRAPH +
        "INSERT INTO chain_analysis_watches(id,profile_id,book_id,domain_json,definition_json,baseline_json,checkpoint,checked_at,created_at) VALUES('w','p','b','{}','{\"rule\":\"output_spent\",\"rule_version\":2,\"query\":{}}','broken','','fixed','fixed');")
    add('stale-rule-version', [act('evaluate', {}), act('inbox', {}), act('list', {})], sql=GRAPH +
        "INSERT INTO chain_analysis_watches(id,profile_id,book_id,domain_json,definition_json,baseline_json,checkpoint,sequence,checked_at,created_at) VALUES('w','p','b','{}','{\"rule\":\"output_spent\",\"rule_version\":2,\"query\":{}}','{\"status\":\"observed\",\"values\":[\"x\"]}','',4,'fixed','fixed');")
    add('caller-rollback', [{'operation': 'begin'}, act('preview', spent), act('create', plan(1)), act('evaluate', {}), {'operation': 'rollback'}, act('list', {})],
        sql=GRAPH, caller_owned=True)
    return corpus


if __name__ == '__main__':
    DEST.write_text(json.dumps(vectors(), ensure_ascii=True, separators=(',', ':')) + '\n')

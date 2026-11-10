"""Dataset job starters on the shared analysis runtime: native receipts, progress
and tables equal unchanged Python's `job_starter` over the same staged file."""
import copy
import json
import os
from pathlib import Path
import sqlite3
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from tests.rust_sync_parity import fixed_elapsed_ms
from tests.rust_sync_env import child_env
from tests import test_rust_analysis_watches as WATCHES
from tests import test_rust_analysis_dataset_import as IMPORT

OPS = WATCHES.OPS
REFERENCE = IMPORT.REFERENCE


def book(path):
    conn = sqlite3.connect(path)
    conn.execute('PRAGMA foreign_keys=ON')
    conn.executescript(OPS.SEED.read_text())
    conn.executescript(OPS.BOOK.read_text())
    conn.close()


def redact(value):
    # The frozen corpus keeps a placeholder; the value is checked first.
    value = fixed_elapsed_ms(copy.deepcopy(value))
    for key in ('job_id', 'elapsed_ms', 'source_token'):
        if key in value:
            value[key] = '<redacted>'
    return value


def python_steps(database, source, steps):
    from kassiber import daemon_chain_analysis as dca
    from kassiber.core import chain_analysis_api as api, chain_analysis_runtime as runtime
    from kassiber.envelope import build_envelope, build_error_envelope
    from kassiber.errors import AppError
    jobs, sources = runtime.AnalysisJobs(), runtime.AnalysisSources()
    counter = REFERENCE.Sources()

    def open_worker(*args, expected_database_identity=None, **kwargs):
        worker = sqlite3.connect(database, check_same_thread=False)
        worker.row_factory = sqlite3.Row
        worker.execute('PRAGMA foreign_keys=ON')
        return worker

    conn = sqlite3.connect(database)
    conn.row_factory = sqlite3.Row
    conn.execute('PRAGMA foreign_keys=ON')
    outputs, token = [], None
    with patch.object(dca, 'JOBS', jobs), patch.object(dca, 'SOURCES', sources), patch.object(runtime, 'SOURCES', sources), \
         patch.object(runtime, 'JOBS', jobs), patch.object(dca, 'open_db', open_worker), \
         patch.object(REFERENCE.datasets, 'now_iso', return_value=REFERENCE.NOW), \
         patch.object(REFERENCE.datasets, 'uuid', SimpleNamespace(uuid4=counter.label_uuid)):
        starter = dca.job_starter('unused-data-root')
        scope = runtime.scope_key(conn, 'p')
        for step in steps:
            operation, args = step['operation'], copy.deepcopy(step.get('args'))
            if isinstance(args, dict) and args.get('source_token') == '$token':
                args['source_token'] = token
            try:
                if operation == 'stage':
                    payload = sources.stage(scope, str(source), 'dataset')
                    token = payload['source_token']
                    value = build_envelope('internal.chain_analysis.stage', redact(payload))
                else:
                    payload = api.dispatch(conn, 'ui.chain_analysis.' + operation, args, profile='p', job_starter=starter)
                    if operation.endswith('.start'):
                        if step.get('cancel'):
                            jobs.get(scope, payload['job_id'], cancel=True)
                        while payload['status'] == 'running':
                            payload = jobs.get(scope, payload['job_id'])
                    value = build_envelope('ui.chain_analysis.' + operation, redact(payload))
            except AppError as e:
                value = build_error_envelope(e.code, str(e), e.details, e.hint, e.retryable)
            outputs.append({'result': value, 'state': OPS.REFERENCE.capture(conn)})
    conn.close()
    return outputs


many = 'subject,label\n' + ''.join(f'{n:064x},Entity {n}\n' for n in range(2001))
SCENARIOS = {
    'import-and-discard': (REFERENCE.manifest(), [
        {'operation': 'stage'},
        {'operation': 'datasets.import.start', 'args': {'source_token': '$token', 'manifest': REFERENCE.manifest(), 'expected_sha256': 'f' * 64}, 'consumes_id': True},
        {'operation': 'datasets.discard.start', 'args': {'id': '00000000-0000-4000-8000-000000000001'}},
        {'operation': 'datasets.import.start', 'args': {'source_token': '$token', 'manifest': REFERENCE.manifest(version='2'), 'expected_sha256': None}, 'consumes_id': True},
        {'operation': 'datasets.discard.start', 'args': {'id': '00000000-0000-4000-8000-000000000002'}},
        {'operation': 'datasets.import', 'args': {'source_token': '$token', 'manifest': REFERENCE.manifest(version='3'), 'expected_sha256': None}},
        {'operation': 'datasets.import', 'args': {'source_token': '$token', 'manifest': REFERENCE.manifest(version='3', expected_active_id='00000000-0000-4000-8000-000000000002'),
                                                   'expected_sha256': None}},
        {'operation': 'datasets.import', 'args': {'source_token': 'unknown', 'manifest': REFERENCE.manifest(version='4'), 'expected_sha256': None}},
        {'operation': 'datasets.import.start', 'args': {'source_token': 'unknown', 'manifest': REFERENCE.manifest(version='4'), 'expected_sha256': None}},
        {'operation': 'datasets.import.start', 'args': {'source_token': '$token', 'manifest': REFERENCE.manifest(version='4')}},
        {'operation': 'datasets.import.start', 'args': {'source_token': '$token', 'manifest': {'name': 'x'}, 'expected_sha256': None}},
        {'operation': 'datasets.discard.start', 'args': {'id': 'absent'}},
        {'operation': 'datasets.discard.start', 'args': {}},
        {'operation': 'datasets.discard.start', 'args': {'id': '00000000-0000-4000-8000-000000000003'}},
        {'operation': 'datasets.import.start', 'args': {'source_token': '$token', 'manifest': REFERENCE.manifest(version='5'), 'expected_sha256': None,
                                                         'expected_scope': {'workspace_id': 'ws', 'profile_id': 'other'}}},
    ]),
}


def native(request, env):
    return WATCHES.native(request, env)


@pytest.mark.parametrize('name', sorted(SCENARIOS))
def test_dataset_jobs_match_python_receipts_progress_and_tables(tmp_path, name):
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    _, steps = SCENARIOS[name]
    source = tmp_path/'pack.csv'
    source.write_text(many)
    for side in ('native', 'reference'):
        book(tmp_path/f'{side}.db')
    expected = python_steps(tmp_path/'reference.db', source, steps)
    actual = native({'jobs': {'database': str(tmp_path/'native.db'), 'file': str(source), 'steps': steps}}, env)
    assert actual == expected


def test_dataset_job_cancellation_leaves_no_active_pack(tmp_path):
    env = child_env(tmp_path)
    env['KASSIBER_TEST_NO_EGRESS'] = 'strict'
    source = tmp_path/'pack.csv'
    source.write_text(many * 1)
    book(tmp_path/'native.db')
    steps = [{'operation': 'stage'},
             {'operation': 'datasets.import.start', 'args': {'source_token': '$token', 'manifest': REFERENCE.manifest(), 'expected_sha256': None}, 'cancel': True}]
    actual = native({'jobs': {'database': str(tmp_path/'native.db'), 'file': str(source), 'steps': steps}}, env)
    receipt = actual[1]['result']['data']
    assert receipt['status'] == 'cancelled' and receipt['cancel_requested'] is True
    assert receipt['result'] in ({'status': 'cancelled'}, {'status': 'cancelled', 'reason': 'cancelled'})
    conn = sqlite3.connect(tmp_path/'native.db')
    assert conn.execute("SELECT COUNT(*) FROM chain_analysis_datasets WHERE status='active'").fetchone()[0] == 0
    conn.close()


def test_dataset_job_corpus_frozen_python_receipts():
    import importlib.util
    spec = importlib.util.spec_from_file_location('analysis_dataset_job_vectors', Path(__file__).resolve().parents[1]/'crates/wallets/scripts/generate_analysis_dataset_jobs.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    assert OPS.canonical(module.vectors()) == OPS.canonical(json.loads(module.DEST.read_text()))


@pytest.mark.parametrize('offline', [False, True])
def test_dataset_job_daemon_refusals_match_python(tmp_path, offline):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    env = OPS.product_env(tmp_path)
    env['KASSIBER_ANALYSIS_REFERENCE_NO_WATCH_WORKER'] = env['KASSIBER_RUST_TEST_NO_WATCH_WORKER'] = '1'
    if offline: env['KASSIBER_NO_EGRESS'] = '1'
    else: env.pop('KASSIBER_NO_EGRESS', None)
    roots = WATCHES.books(tmp_path, REFERENCE_PACKS)
    requests = [('daemon.unlock', {'auth_response': {'passphrase_secret': WATCHES.KEY}}),
                ('ui.chain_analysis.datasets.import', {'source_token': 'unknown', 'manifest': REFERENCE.manifest(), 'expected_sha256': None}),
                ('ui.chain_analysis.datasets.import', {'manifest': REFERENCE.manifest()}),
                ('ui.chain_analysis.datasets.import.start', {'source_token': 'unknown', 'manifest': REFERENCE.manifest(), 'expected_sha256': None}),
                ('ui.chain_analysis.datasets.import.start', {'source_token': 'unknown', 'manifest': {'name': 'x'}, 'expected_sha256': None}),
                ('ui.chain_analysis.datasets.discard.start', {'id': 'absent'}),
                ('ui.chain_analysis.datasets.discard.start', {'id': 'active-pack'}),
                ('ui.chain_analysis.datasets.discard.start', []),
                ('ui.chain_analysis.datasets.import', {'expected_scope': {'workspace_id': 'ws', 'profile_id': 'other'}})]
    wire = b''.join(OPS.canonical({'request_id': str(i), 'kind': kind, 'args': args}) + b'\n' for i, (kind, args) in enumerate(requests))
    wire += OPS.canonical({'request_id': 'stop', 'kind': 'daemon.shutdown'}) + b'\n'
    actual = WATCHES.run(roots[0], ['daemon'], env, input=wire, passphrase=False)
    expected = WATCHES.run(roots[1], ['daemon'], env, reference=True, input=wire, passphrase=False)
    assert (actual.returncode, actual.stderr) == (0, b'')
    lines = [result.stdout.replace(str(root).encode(), b'ROOT').splitlines()[1:] for result, root in [(actual, roots[0]), (expected, roots[1])]]
    assert lines[0] == lines[1]
    assert all(json.loads(line)['kind'] == 'error' for line in lines[0][1:len(requests)])
    assert WATCHES.snapshot(roots[0]) == WATCHES.snapshot(roots[1])
    ready = json.loads(actual.stdout.splitlines()[0])['data']['supported_kinds']
    assert {'ui.chain_analysis.datasets.import', 'ui.chain_analysis.datasets.import.start', 'ui.chain_analysis.datasets.discard.start'} <= set(ready)
    evidence = Path(os.environ.get('KASSIBER_ANALYSIS_OPS_EVIDENCE', tmp_path))
    evidence.mkdir(parents=True, exist_ok=True)
    counts = {}
    for kind, _ in requests[1:]: counts[kind] = counts.get(kind, 0) + 1
    (evidence/f'datasets-daemon-{offline}.json').write_text(json.dumps({'served_by': counts, 'python_fallbacks': 0}, sort_keys=True))


REFERENCE_PACKS = ("INSERT INTO chain_analysis_datasets(id,profile_id,dataset_key,version,chain,network,visibility,status,manifest_json,created_at,updated_at) "
                   "VALUES('active-pack','p','pack','1','bitcoin','main','public','active',"
                   "'{\"valid_from\":null,\"valid_until\":null,\"source\":\"fixture\",\"attribution_method\":\"declared\",\"license\":\"test\"}','fixed','fixed');")

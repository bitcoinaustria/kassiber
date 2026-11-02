"""The daemon's background local-evidence worker: native and unchanged Python
emit the same wake-up events, contact the authorized source with the same
requests and save the same 112 tables after one tick."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

import pytest

from tests import test_rust_analysis_watches as WATCHES
from tests import test_rust_analysis_grants as GRANTS
from tests.rust_port import acquisition_fake_backends as FAKE

OPS = WATCHES.OPS
ROOT = Path(__file__).resolve().parents[1]


def session(root, env, *, reference, until=None, settle=1.0, limit=90.0):
    """Unlock, wait until the worker's first pass is observable, then shut down.

    Waiting on the observable outcome (not a fixed sleep) keeps slow SQLCipher
    opens on a loaded machine from racing the shutdown.
    """
    import threading
    prefix = [sys.executable, str(OPS.REFERENCE_MAIN)] if reference else [os.environ['KASSIBER_RUST_BIN']]
    child = subprocess.Popen([*prefix, '--machine', '--data-root', str(root), 'daemon'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, env=env, cwd=ROOT)
    lines = []
    reader = threading.Thread(target=lambda: lines.extend(iter(child.stdout.readline, b'')), daemon=True)
    reader.start()
    child.stdin.write(OPS.canonical({'request_id': 'unlock', 'kind': 'daemon.unlock', 'args': {'auth_response': {'passphrase_secret': WATCHES.KEY}}}) + b'\n')
    child.stdin.flush()
    deadline = time.monotonic() + limit
    while time.monotonic() < deadline and not (len(lines) >= 2 and (until is None or until(lines))):
        time.sleep(0.1)
    time.sleep(settle)
    child.stdin.write(OPS.canonical({'request_id': 'stop', 'kind': 'daemon.shutdown'}) + b'\n')
    child.stdin.close()
    child.wait(timeout=120)
    reader.join(timeout=30)
    err = child.stderr.read()
    assert child.returncode == 0, err
    return [line.rstrip(b'\n').replace(str(root).encode(), b'ROOT') for line in lines[1:]]


def quiet(recorder, seconds=1.5):
    """True once the fake backend saw requests and then none for `seconds`."""
    state = {'count': 0, 'since': time.monotonic()}
    def check(_lines):
        count = len(recorder.log)
        if count != state['count']:
            state.update(count=count, since=time.monotonic())
        return count > 0 and time.monotonic() - state['since'] >= seconds
    return check


def environment(tmp_path):
    env = GRANTS.grant_env(tmp_path, False)
    env.pop('KASSIBER_ANALYSIS_REFERENCE_NO_WATCH_WORKER')
    env.pop('KASSIBER_RUST_TEST_NO_WATCH_WORKER')
    env['KASSIBER_TEST_NO_EGRESS'] = '1'
    return env


def test_watch_worker_emits_one_event_and_matches_python_tables(tmp_path):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    env = environment(tmp_path)
    roots = WATCHES.books(tmp_path)
    plan = json.loads(WATCHES.run(roots[1], ['chain-analysis', 'watches', 'preview', '--document', json.dumps(WATCHES.SPENT)], env, reference=True).stdout)['data']
    for root, reference in ((roots[0], False), (roots[1], True)):
        created = WATCHES.run(root, ['chain-analysis', 'watches', 'create', '--plan', json.dumps(plan)], env, reference=reference)
        assert created.returncode == 0, created.stdout
    WATCHES.mutate(roots, WATCHES.REFERENCE.SPEND)
    assert WATCHES.snapshot(roots[0]) == WATCHES.snapshot(roots[1])
    event = lambda lines: any(b'watches.changed' in line for line in lines)
    actual = session(roots[0], env, reference=False, until=event)
    expected = session(roots[1], env, reference=True, until=event)
    assert actual == expected
    kinds = [json.loads(line)['kind'] for line in actual]
    assert kinds == ['daemon.unlock', 'ui.chain_analysis.watches.changed', 'daemon.shutdown']
    assert json.loads(actual[1]) == {'kind': 'ui.chain_analysis.watches.changed', 'schema_version': 1, 'data': {'local_only': True}, 'event': True}
    assert WATCHES.snapshot(roots[0]) == WATCHES.snapshot(roots[1])
    # A delivered event is not repeated on the next unlock.
    assert [json.loads(line)['kind'] for line in session(roots[0], env, reference=False, settle=3.0)] == ['daemon.unlock', 'daemon.shutdown']


@pytest.mark.parametrize('mode', ['subject', 'blocks'])
def test_source_runner_matches_python_requests_and_tables(tmp_path, mode):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    transactions = FAKE.build_chain()
    chain = FAKE.Chain(transactions, blocks=FAKE.build_blocks(transactions, [[0, 1], [2], [3]]))
    recorder = FAKE.Recorder()
    server = FAKE.http_server(chain, recorder)
    try:
        port = server.server_address[1]
        env = environment(tmp_path)
        backend = (f"INSERT INTO backends(name,kind,chain,network,url,config_json,created_at,updated_at) VALUES('core','bitcoinrpc','bitcoin','main',"
                   f"'http://127.0.0.1:{port}/','{{\"username\":\"u\",\"password\":\"p\"}}','fixed','fixed');")
        roots = WATCHES.books(tmp_path, backend)
        # Only this test's grant is due; the shared book's sample grant is not.
        WATCHES.mutate(roots, "DELETE FROM chain_analysis_reference_assertions; DELETE FROM chain_analysis_acquisition_grants;")
        spec = ({'backend': 'core', 'network': 'main', 'mode': 'subject', 'subject': chain.order[1], 'max_requests': 10, 'max_bytes': 10 ** 9}
                if mode == 'subject' else
                {'backend': 'core', 'network': 'main', 'mode': 'blocks', 'start_height': 1, 'blocks_per_run': 2, 'max_requests': 100, 'max_bytes': 10 ** 9})
        plan = json.loads(WATCHES.run(roots[1], ['chain-analysis', 'sources', 'plan', '--document', json.dumps(spec)], env, reference=True).stdout)['data']
        for root, reference in ((roots[0], False), (roots[1], True)):
            granted = WATCHES.run(root, ['chain-analysis', 'sources', 'authorize', '--plan', json.dumps(plan)], env, reference=reference)
            assert granted.returncode == 0, granted.stdout
        assert WATCHES.snapshot(roots[0]) == WATCHES.snapshot(roots[1])
        recorder.take()
        actual = session(roots[0], env, reference=False, until=quiet(recorder))
        native_log = recorder.take()
        expected = session(roots[1], env, reference=True, until=quiet(recorder))
        reference_log = recorder.take()
        assert actual == expected
        assert native_log == reference_log and native_log
        assert WATCHES.snapshot(roots[0]) == WATCHES.snapshot(roots[1])
        listed = json.loads(WATCHES.run(roots[0], ['chain-analysis', 'sources', 'list'], env).stdout)['data']['items'][0]
        assert listed['last_code'] == ('observed' if mode == 'subject' else 'backfilling'), listed
    finally:
        server.shutdown()
        server.server_close()


def test_source_runner_reorg_and_resume_match_python(tmp_path):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    transactions = FAKE.build_chain()
    chain = FAKE.Chain(transactions, blocks=FAKE.build_blocks(transactions, [[0, 1], [2], [3]]))
    recorder = FAKE.Recorder()
    server = FAKE.http_server(chain, recorder)
    try:
        port = server.server_address[1]
        env = environment(tmp_path)
        backend = (f"INSERT INTO backends(name,kind,chain,network,url,config_json,created_at,updated_at) VALUES('core','bitcoinrpc','bitcoin','main',"
                   f"'http://127.0.0.1:{port}/','{{\"username\":\"u\",\"password\":\"p\"}}','fixed','fixed');")
        roots = WATCHES.books(tmp_path, backend)
        WATCHES.mutate(roots, "DELETE FROM chain_analysis_reference_assertions; DELETE FROM chain_analysis_acquisition_grants;")
        spec = {'backend': 'core', 'network': 'main', 'mode': 'blocks', 'start_height': 1, 'blocks_per_run': 2, 'max_requests': 1000, 'max_bytes': 10 ** 10}
        plan = json.loads(WATCHES.run(roots[1], ['chain-analysis', 'sources', 'plan', '--document', json.dumps(spec)], env, reference=True).stdout)['data']
        for root, reference in ((roots[0], False), (roots[1], True)):
            assert WATCHES.run(root, ['chain-analysis', 'sources', 'authorize', '--plan', json.dumps(plan)], env, reference=reference).returncode == 0
        codes = []
        for step in range(4):
            if step == 1:
                # Replace heights 2 and 3: the saved cursor's block is no longer canonical.
                chain.blocks = FAKE.build_blocks(transactions, [[0, 1], [3], [2]])
            env['KASSIBER_RUST_TEST_ANALYSIS_EPOCH'] = str(GRANTS.REFERENCE.EPOCH + 10 * step)
            recorder.take()
            actual = session(roots[0], env, reference=False, until=quiet(recorder))
            native_log = recorder.take()
            expected = session(roots[1], env, reference=True, until=quiet(recorder))
            assert (actual, native_log) == (expected, recorder.take()), step
            assert WATCHES.snapshot(roots[0]) == WATCHES.snapshot(roots[1]), step
            codes.append(json.loads(WATCHES.run(roots[0], ['chain-analysis', 'sources', 'list'], env).stdout)['data']['items'][0]['last_code'])
        assert codes[0] == 'backfilling' and 'reorg_reconciling' in codes, codes
    finally:
        server.shutdown()
        server.server_close()

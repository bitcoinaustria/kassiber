"""Acquisition apply against loopback fakes: native and Python send the same
requests, return the same bytes and save the same 112 tables; nothing is sent
before the reviewed plan matches or while egress is disabled."""
import json
import os
from pathlib import Path

import pytest

from tests import test_rust_analysis_watches as WATCHES
from tests import test_rust_analysis_grants as GRANTS
from tests.rust_port import acquisition_fake_backends as FAKE

OPS = WATCHES.OPS


@pytest.fixture
def fakes():
    chain = FAKE.Chain(FAKE.build_chain())
    recorder = FAKE.Recorder()
    servers = [FAKE.http_server(chain, recorder), FAKE.electrum_server(chain, recorder)]
    try:
        yield chain, recorder, servers
    finally:
        for server in servers:
            server.shutdown()
            server.server_close()


def backends(servers):
    http, electrum = (server.server_address[1] for server in servers)
    rows = [
        ("esplora", "esplora", f"http://127.0.0.1:{http}/api", '{"token":"synthetic-token"}'),
        ("core", "bitcoinrpc", f"http://127.0.0.1:{http}/", '{"username":"u","password":"p"}'),
        ("electrum", "electrum", f"tcp://127.0.0.1:{electrum}", '{}'),
        ("redirect", "esplora", f"http://127.0.0.1:{http}/redirect", '{}'),
        ("nocreds", "bitcoinrpc", f"http://127.0.0.1:{http}/", '{}'),
    ]
    return ''.join(f"INSERT INTO backends(name,kind,chain,network,url,config_json,created_at,updated_at) VALUES('{n}','{k}','bitcoin','main','{u}','{c}','fixed','fixed');" for n, k, u, c in rows)


def environment(tmp_path, *, offline=False, no_egress=False):
    env = GRANTS.grant_env(tmp_path, offline)
    # Loopback fakes only; any other destination still trips the guard.
    env['KASSIBER_TEST_NO_EGRESS'] = '1'
    if no_egress: env['KASSIBER_NO_EGRESS'] = '1'
    else: env.pop('KASSIBER_NO_EGRESS', None)
    return env


def plan(roots, env, chain, args):
    txid = chain.order[1]
    argv = ['chain-analysis', 'acquire', 'plan', args.pop('subject', txid), '--chain', 'bitcoin', '--network', 'main', *args.pop('argv')]
    result = WATCHES.run(roots[1], argv, env, reference=True)
    assert result.returncode == 0, result.stdout
    return json.loads(result.stdout)['data']


def compare(roots, env, recorder, argv=None, *, wire=None):
    recorder.take()
    if wire is None:
        actual = WATCHES.run(roots[0], argv, env)
        native_log = recorder.take()
        expected = WATCHES.run(roots[1], argv, env, reference=True)
        reference_log = recorder.take()
        assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr), argv
        output = json.loads(actual.stdout)
    else:
        actual = WATCHES.run(roots[0], ['daemon'], env, input=wire, passphrase=False)
        native_log = recorder.take()
        expected = WATCHES.run(roots[1], ['daemon'], env, reference=True, input=wire, passphrase=False)
        reference_log = recorder.take()
        assert (actual.returncode, actual.stderr) == (0, b'')
        lines = [result.stdout.replace(str(root).encode(), b'ROOT').splitlines()[1:] for result, root in [(actual, roots[0]), (expected, roots[1])]]
        assert lines[0] == lines[1]
        output = [json.loads(line) for line in lines[0]]
    assert native_log == reference_log
    assert WATCHES.snapshot(roots[0]) == WATCHES.snapshot(roots[1])
    return output, native_log


CASES = {
    'esplora-both': {'argv': ['--backend', 'esplora', '--depth', '3']},
    'esplora-backward-depth': {'argv': ['--backend', 'esplora', '--direction', 'backward', '--depth', '1']},
    'esplora-transaction-limit': {'argv': ['--backend', 'esplora', '--max-transactions', '2']},
    'core-both': {'argv': ['--backend', 'core', '--depth', '3']},
    'core-forward': {'argv': ['--backend', 'core', '--direction', 'forward']},
    'electrum-both': {'argv': ['--backend', 'electrum', '--depth', '2']},
    'unknown-subject': {'argv': ['--backend', 'esplora'], 'subject': 'ab' * 32},
    'unknown-core-subject': {'argv': ['--backend', 'core'], 'subject': 'ab' * 32},
    'redirect-refused': {'argv': ['--backend', 'redirect']},
    'missing-credentials': {'argv': ['--backend', 'nocreds']},
}


@pytest.mark.parametrize('name', sorted(CASES))
def test_acquisition_apply_cli_matches_python_requests_bytes_and_tables(tmp_path, fakes, name):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    chain, recorder, servers = fakes
    env = environment(tmp_path)
    roots = WATCHES.books(tmp_path, backends(servers))
    reviewed = plan(roots, env, chain, dict(CASES[name], argv=list(CASES[name]['argv'])))
    output, log = compare(roots, env, recorder, ['chain-analysis', 'acquire', 'apply', '--plan', json.dumps(reviewed)])
    if name not in {'redirect-refused', 'missing-credentials'}:
        assert output['kind'] == 'chain-analysis.acquire.apply', output
    assert bool(log) == (name != 'missing-credentials'), log
    evidence = Path(os.environ.get('KASSIBER_ANALYSIS_OPS_EVIDENCE', tmp_path))
    evidence.mkdir(parents=True, exist_ok=True)
    (evidence/f'acquire-apply-{name}.json').write_text(json.dumps({'served_by': {'chain-analysis.acquire.apply': 1}, 'python_fallbacks': 0, 'requests': len(log)}, sort_keys=True))


@pytest.mark.parametrize('mode', ['offline', 'no-egress', 'stale', 'genesis', 'spender-index'])
def test_acquisition_apply_refusals_send_nothing_or_match(tmp_path, fakes, mode):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    chain, recorder, servers = fakes
    env = environment(tmp_path)
    roots = WATCHES.books(tmp_path, backends(servers))
    reviewed = plan(roots, env, chain, {'argv': ['--backend', 'core' if mode in {'genesis', 'spender-index'} else 'esplora']})
    if mode == 'stale': reviewed = {**reviewed, 'snapshot_id': 'stale'}
    if mode == 'genesis': chain.genesis = '11' * 32
    if mode == 'spender-index': chain.spender_index = False
    if mode == 'offline': (tmp_path/'offline.json').write_text(json.dumps({'schema_version': 1, 'enabled': True}))
    if mode == 'no-egress': env['KASSIBER_NO_EGRESS'] = '1'
    output, log = compare(roots, env, recorder, ['chain-analysis', 'acquire', 'apply', '--plan', json.dumps(reviewed)])
    if mode in {'offline', 'no-egress', 'stale'}:
        assert output['kind'] == 'error' and log == []
    elif mode == 'genesis':
        assert output['error']['code'] == 'backend_network_mismatch'
    else:
        assert {'txid': chain.order[1], 'reason': 'historical_spender_index_unavailable'} in output['data']['frontier']


def test_acquisition_apply_daemon_matches_python(tmp_path, fakes):
    if not os.environ.get('KASSIBER_RUST_BIN'): pytest.skip('requires kassiber test-harness build')
    chain, recorder, servers = fakes
    env = environment(tmp_path)
    roots = WATCHES.books(tmp_path, backends(servers))
    reviewed = plan(roots, env, chain, {'argv': ['--backend', 'esplora', '--depth', '2']})
    requests = [('daemon.unlock', {'auth_response': {'passphrase_secret': WATCHES.KEY}}),
                ('ui.chain_analysis.acquire.apply', {'plan': {**reviewed, 'plan_id': 'stale'}}),
                ('ui.chain_analysis.acquire.apply', {'plan': reviewed}),
                ('ui.chain_analysis.acquire.apply', {'plan': reviewed})]
    wire = b''.join(OPS.canonical({'request_id': str(i), 'kind': kind, 'args': args}) + b'\n' for i, (kind, args) in enumerate(requests))
    wire += OPS.canonical({'request_id': 'stop', 'kind': 'daemon.shutdown'}) + b'\n'
    output, log = compare(roots, env, recorder, wire=wire)
    assert [r['kind'] for r in output[:4]] == ['daemon.unlock', 'error', 'ui.chain_analysis.acquire.apply', 'error']
    assert log


def test_acquisition_python_reference_truncated_electrum_abort_is_recorded():
    from tests.rust_port.repro_acquisition_electrum_truncated import reproduce
    assert reproduce() == {'caught_by_frontier': False, 'type': 'RuntimeError'}

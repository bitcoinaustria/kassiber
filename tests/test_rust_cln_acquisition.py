from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import threading

import pytest

from tests.test_rust_lightning_acquisition import environment

ROOT = Path(__file__).resolve().parents[1]
BIN = os.environ.get('KASSIBER_LIGHTNING_FIXTURE_BIN')


@contextmanager
def node(data, *, failure=None, before_reply=None):
    hits = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            args = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            method = args[-1]
            hits.append(args)
            if before_reply is not None:
                before_reply(len(hits) - 1)
            body = json.dumps(data[method]).encode()
            self.send_response(500 if failure == method else 200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=lambda: server.serve_forever(poll_interval=0.01))
    thread.start()
    try:
        yield f'http://127.0.0.1:{server.server_port}', hits
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def fake_cli(root):
    path = root / 'fake-lightning-cli'
    path.write_text(f'#!{sys.executable}\n' + '''import json,sys
from urllib.request import Request,urlopen
args = sys.argv[1:]
url = next(a.split('=',1)[1] for a in args if a.startswith('--rpc-file='))
try:
    with urlopen(Request(url, data=json.dumps(args).encode()), timeout=2) as response:
        sys.stdout.write(response.read().decode())
except Exception:
    print('DISCARDED_CLN_STDERR_SECRET', file=sys.stderr)
    raise SystemExit(7)
''')
    path.chmod(0o700)
    return path


PYTHON_READ = '''
from contextlib import nullcontext
from dataclasses import asdict
from datetime import datetime, timezone
import json,sys
from unittest.mock import patch
from kassiber.core.lightning import cln
from kassiber.core.lightning.cln import CoreLightningAdapter
from kassiber.errors import AppError
from kassiber.time_utils import timestamp_to_iso
data = json.load(sys.stdin)
# Reference vectors end the routing window at a frozen instant.
clock = (patch.object(cln, 'now_iso', lambda: datetime.fromtimestamp(data['now'], timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'))
         if 'now' in data else nullcontext())
try:
    with clock:
        snapshot = CoreLightningAdapter().fetch_node_snapshot({}, data['backend'], window_days=data['window'])
    print(json.dumps({'channels': [{'id': c.id, 'peer_alias': c.peer_alias, 'capacity_sat': c.capacity_sat, 'earned_routing_sat': c.earned_routing_sat} for c in snapshot.channels], 'routing': asdict(snapshot.routing) if snapshot.routing else None}))
except AppError as error:
    print(json.dumps({'error': error.code}))
'''


@pytest.mark.skipif(not BIN or os.name != 'posix', reason='requires native acquisition fixture and executable fake CLI')
def test_cln_report_real_process_transport_matches_python_without_fallback(tmp_path):
    cases = json.loads((ROOT / 'crates/chain/fixtures/python_cln.json').read_text())
    selected = [c for c in cases if 'expected' in c and not c['failures']][::19]
    assert len(selected) >= 12
    cli = fake_cli(tmp_path)
    for case in selected:
        outputs, requests = [], []
        for native in [False, True]:
            with node(case['data']) as (url, hits):
                data = {'backend': {'kind': 'coreln', 'name': 'fake', 'url': 'cln://local',
                                    'lightning_cli': str(cli), 'rpc_file': url,
                                    'lightning_dir': 'disposable lightning directory', 'network': 'regtest',
                                    'timeout': 2},
                        'window': case['window'], 'now': case['now'], 'cln_fixture': str(cli)}
                result = subprocess.run([BIN] if native else [sys.executable, '-c', PYTHON_READ],
                                        cwd=ROOT, env=environment(tmp_path), input=json.dumps(data),
                                        text=True, capture_output=True, check=True, timeout=20)
                output = json.loads(result.stdout)
                if native:
                    assert output['attempts'] == output['records'] == 9
                    output = output['response']['result']
                outputs.append(output)
                requests.append([[arg for arg in args if not arg.startswith('--rpc-file=')] for args in hits])
        assert outputs == [case['expected'], case['expected']], case['name']
        assert requests[0] == requests[1], case['name']
        assert [args[-1] for args in requests[0]] == [call['method'] for call in case['calls']]


@pytest.mark.skipif(not BIN or os.name != 'posix', reason='requires native acquisition fixture and executable fake CLI')
def test_cln_commando_configurations_are_refused_like_python_without_launch(tmp_path):
    # lightning-cli takes a commando rune only in argv, so both backends
    # refuse a commando configuration before the fake CLI can run.
    cli = fake_cli(tmp_path)
    for fields in [
        {'url': 'cln://commando', 'commando_peer_id': 'private-peer', 'token': 'private-rune'},
        {'url': 'cln://local', 'token': 'private-rune'},
        {'url': 'cln://local', 'commando_peer_id': 'private-peer'},
        {'url': 'CLN://Commando'},
    ]:
        with node({'getinfo': {}}) as (url, hits):
            data = {'backend': {'kind': 'coreln', 'name': 'fake', 'rpc_file': url, 'lightning_cli': str(cli), **fields},
                    'cln_fixture': str(cli), 'window': 30}
            outputs = []
            for native in [False, True]:
                result = subprocess.run([BIN] if native else [sys.executable, '-c', PYTHON_READ], cwd=ROOT,
                                        env=environment(tmp_path), input=json.dumps(data), text=True,
                                        capture_output=True, check=True, timeout=20)
                assert 'private-rune' not in result.stdout + result.stderr
                output = json.loads(result.stdout)
                if native:
                    assert output['attempts'] == output['records'] == 0
                    output = {'error': output['response']['error']['code']}
                outputs.append(output)
            assert outputs == [{'error': 'validation'}, {'error': 'validation'}], fields
            assert hits == []


@pytest.mark.skipif(not BIN or os.name != 'posix', reason='requires native acquisition fixture and executable fake CLI')
def test_cln_report_process_policy_refusals_make_no_hits(tmp_path):
    cli = fake_cli(tmp_path)
    with node({'getinfo': {}}) as (url, hits):
        for fields, extra, expected in [
            ({'consent': False}, {}, 'interaction_required'),
            ({'offline': True}, {}, 'network_egress_disabled'),
            ({}, {'KASSIBER_NO_EGRESS': '1'}, 'network_egress_disabled'),
        ]:
            data = {'backend': {'kind': 'coreln', 'name': 'fake', 'rpc_file': url, 'lightning_cli': str(cli)},
                    'cln_fixture': str(cli), **fields}
            result = subprocess.run([BIN], cwd=ROOT, env={**environment(tmp_path), **extra}, input=json.dumps(data),
                                    text=True, capture_output=True, check=True, timeout=10)
            output = json.loads(result.stdout)
            assert output['response']['error']['code'] == expected
            assert output['attempts'] == output['records'] == 0
        assert hits == []


@pytest.mark.skipif(not BIN or os.name != 'posix', reason='requires native acquisition fixture and executable fake CLI')
def test_cln_report_transport_failure_optional_resources_and_secret_free_errors(tmp_path):
    case = json.loads((ROOT / 'crates/chain/fixtures/python_cln.json').read_text())[0]
    cli = fake_cli(tmp_path)
    for failure in ['getinfo', 'listfunds', 'listpeerchannels', 'listforwards', 'listpays', 'bkpr-listincome']:
        outputs, counts = [], []
        for native in [False, True]:
            with node(case['data'], failure=failure) as (url, hits):
                data = {'backend': {'kind': 'coreln', 'name': 'fake', 'rpc_file': url, 'lightning_cli': str(cli)},
                        'cln_fixture': str(cli), 'window': 30}
                result = subprocess.run([BIN] if native else [sys.executable, '-c', PYTHON_READ], cwd=ROOT,
                                        env=environment(tmp_path), input=json.dumps(data), text=True,
                                        capture_output=True, check=True, timeout=20)
                output = json.loads(result.stdout)
                if native:
                    assert 'DISCARDED_CLN_STDERR_SECRET' not in result.stdout
                    output = output['response']
                    output = {'error': output['error']['code']} if 'error' in output else output['result']
                outputs.append(output)
                counts.append(len(hits))
        assert outputs[0] == outputs[1], failure
        assert counts == ([1, 1] if failure == 'getinfo' else [9, 9])


def test_cln_report_acquisition_vectors_match_unchanged_python():
    subprocess.run([sys.executable, str(ROOT / 'crates/chain/scripts/gen_cln_vectors.py'), '--check'],
                   cwd=ROOT, check=True, capture_output=True, text=True, timeout=180)


def test_cln_reference_error_disclosure_and_round_8_fixes_remain_reproducible():
    result = subprocess.run([sys.executable, str(ROOT / 'crates/chain/scripts/repro_python_cln.py')],
                            cwd=ROOT, check=True, capture_output=True, text=True, timeout=30)
    assert json.loads(result.stdout) == {
        'stderr_discloses_response': True,
        'command_discloses_rpc_path': True,
        'amount_error_discloses_value': True,
        'nested_output_zero_is_kept': True,
        'window_bounds_routing': True,
    }

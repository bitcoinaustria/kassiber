"""Byte-level process witnesses for the shared Rust pairing command routes."""
import datetime
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from kassiber.db import open_db
from tests import cli_runner
from tests.test_rust_shell_parity import _environment

ROOT = Path(__file__).resolve().parents[1]
RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')


@unittest.skipUnless(RUST_BIN, 'set KASSIBER_RUST_BIN to the harness build')
class RustPairingCommandsTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix='kassiber-pairing-commands-')
        self.addCleanup(temp.cleanup)
        self.base = Path(temp.name)
        self.env = _environment(self.base / 'config')
        self.env['HOME'] = os.environ['HOME']
        self.env['KASSIBER_TEST_NO_EGRESS'] = '1'
        owners = self.base / 'owners'
        owners.mkdir(mode=0o700)
        self.env['KASSIBER_RUST_TEST_OWNER_ROOT'] = str(owners)
        original = cli_runner.prefix
        script = (
            "import sys; from pathlib import Path; "
            "from kassiber.operator import project; "
            "root=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:root; "
            "from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))"
        )
        def prefix(*, reference=False, python_options=()):
            return (sys.executable,'-c',script,str(owners)) if reference else original(reference=reference,python_options=python_options)
        patch = mock.patch.object(cli_runner,'prefix',side_effect=prefix)
        patch.start()
        self.addCleanup(patch.stop)
        self.started = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(seconds=30)
        self.fixture = json.loads((ROOT / 'crates/transfers/tests/pairing_services.json').read_text())
        self.log = self.base / 'served.jsonl'
        self.env.update(KASSIBER_HYBRID_RUST_BIN=RUST_BIN, KASSIBER_HYBRID_LOG=str(self.log), KASSIBER_HYBRID_RUST_DAEMON='1')

    def book(self, name, shape):
        root = self.base / name
        scenario = next(s for s in self.fixture['scenarios'] if s['name'] == '1-' + shape)
        c = open_db(root)
        c.execute('PRAGMA foreign_keys=OFF')
        tables = [r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")]
        for table in tables:
            c.execute('DELETE FROM "'+table+'"')
        for query in scenario['sql']:
            c.execute(query)
        c.execute("INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p')")
        c.commit()
        c.close()
        return root

    def dump(self, root):
        c = open_db(root)
        try:
            tables = [r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
            return {t: sorted((dict(r) for r in c.execute('SELECT * FROM "'+t+'"')), key=lambda r: json.dumps(r,sort_keys=True)) for t in tables}
        finally:
            c.close()

    def cli(self, root, argv, rust):
        options = dict(capture_output=True, env=self.env, cwd=ROOT, timeout=120)
        args = ['--machine','--data-root',str(root),*argv]
        return subprocess.run([sys.executable,str(ROOT/'tests/rust_port/hybrid_cli.py'),*args],**options) if rust else cli_runner.run(args,reference=True,**options)

    def normalized_result(self, output):
        # Each applied pair introduces a UUID and a component UUID derived from
        # it. Fixed-source native oracles independently pin all their DB bytes.
        data = json.loads(output)
        for i, pair in enumerate((data.get('data') or {}).get('applied', [])):
            for field in ('id','component_id'):
                if pair.get(field):
                    output = output.replace(pair[field].encode(), f'<{i}-{field}>'.encode())
        def clock(match):
            stamp = datetime.datetime.fromisoformat(match.group().decode().replace('Z','+00:00'))
            return b'<time>' if self.started <= stamp <= datetime.datetime.now(datetime.timezone.utc)+datetime.timedelta(seconds=30) else match.group()
        return re.sub(rb'20[0-9]{2}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?Z',clock,output)

    def test_pairing_cli_bytes_dry_run_rows_hybrid_and_python_projections(self):
        from kassiber.daemon import _ui_exact_integer_payload, _ui_swap_matching_payload_from_conn
        shapes = ('exact','ambiguous','tolerance','multi-wallet','rules','dismissal','unpaired','ownership','booked','guards-after-rule','fee-variants')
        # Actual generated shape labels are asserted so the witness cannot go vacuous.
        available = {s['name'][2:] for s in self.fixture['scenarios'] if s['name'].startswith('1-')}
        shapes = tuple(shape for shape in shapes if shape in available)
        self.assertEqual(len(shapes),11)
        for shape in shapes:
            roots = [self.book(f'cli-{shape}-{r}',shape) for r in (False,True)]
            for argv in [
                ['transfers','suggest'],
                ['transfers','suggest','--confidence','strong','--candidate-type','transfer'],
                ['transfers','suggest','--asset-pair','invalid'],
                ['transfers','bulk-pair','--dry-run'],
                ['transfers','rules','apply','--dry-run'],
                ['transfers','rules','apply'],
                ['transfers','bulk-pair'],
                ['transfers','suggest'],
            ]:
                results=[]
                for rust,root in zip((False,True),roots):
                    before=self.dump(root)
                    result=self.cli(root,argv,rust)
                    self.assertEqual(result.returncode,1 if 'invalid' in argv else 0,(shape,argv,result.stdout,result.stderr))
                    results.append((result.returncode,self.normalized_result(result.stdout),result.stderr))
                    if '--dry-run' in argv or argv[1]=='suggest':
                        self.assertEqual(self.dump(root),before,(shape,argv,rust))
                    if rust and result.returncode==0:
                        c=open_db(root)
                        # The unchanged AI/desktop adapters consume the authored
                        # Rust rows; their stable reads match the Rust transport.
                        try:
                            for kind,key in [('ui.transfers.list','pairs'),('ui.transfers.rules.list','rules')]:
                                expected=_ui_swap_matching_payload_from_conn(c,kind,dict(workspace='w',profile='p'))[key]
                                actual=self.cli(root,kind[3:].split('.'),True)
                                self.assertEqual(_ui_exact_integer_payload(json.loads(actual.stdout)['data']),expected)
                            context=_ui_swap_matching_payload_from_conn(c,'ui.transfers.review_context',dict(workspace='w',profile='p'))
                            self.assertIn('review_items',context)
                        finally:c.close()
                self.assertEqual(results[0],results[1],(shape,argv))
        observed=[json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertTrue(all(r['served_by']=='rust' for r in observed),observed)
        self.assertTrue({'transfers.suggest','transfers.bulk-pair','transfers.rules.apply'} <= {r['command'] for r in observed})

    def test_pairing_daemon_bytes_defaults_coercion_and_authority(self):
        for shape in ('multi-wallet','rules','ownership'):
            requests=[]
            for kind in ('ui.transfers.suggest','ui.transfers.rules.apply','ui.transfers.bulk_pair'):
                requests += [dict(kind=kind,args=args) for args in (
                    dict(time_window_seconds='bad',profile='missing'),
                    dict(fee_pct_max='bad'),
                    dict(fee_sats_min=[]),
                    dict(candidate_type='bad'),
                    dict(asset_pair='bad'),
                    dict(asset_pair=['BTC','LBTC']),
                    dict(route_pair=1),
                    dict(time_window_seconds=10**400),
                    dict(time_window_seconds=0,fee_pct_max=0,fee_sats_min=0,confidence='invalid'),
                    dict(time_window_seconds=' ８６_４００ ',fee_pct_max='0.01',fee_sats_min=2500.9,method='none'),
                    dict(authored_source='untrusted',dry_run=True),
                )]
            requests.append(dict(kind='daemon.shutdown'))
            results=[]
            for rust in (False,True):
                root=self.book(f'daemon-{shape}-{rust}',shape)
                args=['--data-root',str(root),'daemon']
                payload=''.join(json.dumps(dict(r,request_id=str(i)))+'\n' for i,r in enumerate(requests)).encode()
                options=dict(input=payload,capture_output=True,env=self.env,cwd=ROOT,timeout=120)
                result=subprocess.run([sys.executable,str(ROOT/'tests/rust_port/hybrid_cli.py'),*args],**options) if rust else cli_runner.run(args,reference=True,**options)
                self.assertEqual(result.returncode,0,result.stderr)
                lines=[]
                for line in result.stdout.splitlines(keepends=True)[1:]:
                    parsed=json.loads(line)
                    if (parsed.get('error') or {}).get('code')=='internal_error':
                        parsed['error']['debug']=None
                        line=(json.dumps(parsed,ensure_ascii=False,separators=(',',':'))+'\n').encode()
                    lines.append(self.normalized_result(line))
                results.append((lines,result.stderr))
            for i,(expected,actual) in enumerate(zip(results[0][0],results[1][0])):
                self.assertEqual(expected,actual,(shape,i,requests[i]))
            self.assertEqual(len(results[0][0]),len(results[1][0]))
        observed=[json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(observed,[dict(command='daemon',served_by='rust')]*3)

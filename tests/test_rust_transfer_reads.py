"""Exact CLI and desktop read projections over independently hydrated books."""
import json
import os
from pathlib import Path
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
class RustTransferReadsTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix='kassiber-transfer-reads-')
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
            return (sys.executable, '-c', script, str(owners)) if reference else original(reference=reference, python_options=python_options)
        patch = mock.patch.object(cli_runner, 'prefix', side_effect=prefix)
        patch.start()
        self.addCleanup(patch.stop)
        self.log = self.base / 'served.jsonl'
        self.env.update(KASSIBER_HYBRID_RUST_BIN=RUST_BIN, KASSIBER_HYBRID_LOG=str(self.log), KASSIBER_HYBRID_RUST_DAEMON='1')

    def book(self, name, scenario):
        root = self.base / name
        c = open_db(root)
        c.execute('PRAGMA foreign_keys=OFF')
        tables = [r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")]
        for table in tables:
            c.execute('DELETE FROM "'+table+'"')
        for query in scenario['sql']:
            c.execute(query)
        c.execute("INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p')")
        c.commit()
        c.execute('PRAGMA foreign_keys=ON')
        self.assertEqual(c.execute('PRAGMA foreign_key_check').fetchall(), [])
        c.close()
        return root

    def dump(self, root):
        c = open_db(root)
        try:
            tables = [r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
            return {t: sorted((dict(r) for r in c.execute('SELECT * FROM "'+t+'"')), key=lambda r: json.dumps(r, sort_keys=True)) for t in tables}
        finally:
            c.close()

    def run_cli(self, root, argv, rust):
        args = ['--machine', '--data-root', str(root), *argv]
        options = dict(capture_output=True, env=self.env, cwd=ROOT, timeout=120)
        return subprocess.run([sys.executable, str(ROOT/'tests/rust_port/hybrid_cli.py'), *args], **options) if rust else cli_runner.run(args, reference=True, **options)

    def run_daemon(self, root, requests, rust):
        args = ['--data-root', str(root), 'daemon']
        payload = ''.join(json.dumps(dict(r, request_id=str(i)))+'\n' for i, r in enumerate([*requests, dict(kind='daemon.shutdown')])).encode()
        options = dict(input=payload, capture_output=True, env=self.env, cwd=ROOT, timeout=120)
        result = subprocess.run([sys.executable, str(ROOT/'tests/rust_port/hybrid_cli.py'), *args], **options) if rust else cli_runner.run(args, reference=True, **options)
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = result.stdout.splitlines(keepends=True)[1:]
        self.assertEqual(len(lines), len(requests)+1)
        # Only Python attaches traceback debug to untyped exceptions, as in
        # the established shell contract. All other wire bytes stay exact.
        for i, line in enumerate(lines):
            parsed = json.loads(line)
            if (parsed.get('error') or {}).get('code') == 'internal_error':
                parsed['error']['debug'] = None
                lines[i] = (json.dumps(parsed, ensure_ascii=False, separators=(',', ':'))+'\n').encode()
        return lines

    def component_scenarios(self):
        fixture = json.loads((ROOT/'crates/transfers/tests/component_reads.json').read_text())
        selected = []
        for name in ('locations', 'revise', 'conflict', 'native', 'conversion', 'competing-active'):
            matches = [s for s in fixture['scenarios'] if s['name'].startswith(name+'-')]
            self.assertTrue(matches, name)
            selected.append(matches[-1])
        return selected

    def test_component_cli_bytes_private_views_filters_and_unchanged_rows(self):
        for scenario in self.component_scenarios():
            roots = [self.book(f'cli-{scenario["name"]}-{rust}', scenario) for rust in (False, True)]
            argv = [
                ['list'], ['list', '--limit', '1'], ['list', '--effective-only'],
                ['list', '--include-local-evidence'], ['list', '--transaction', 'd-ext'],
                ['list', '--transaction', 'j-ext'], ['list', '--limit', '0'],
            ]
            ids = sorted({case['args']['component_id'] for case in scenario['cases'] if case['operation'].endswith('show') and 'result' in case})
            self.assertTrue(ids, scenario['name'])
            argv += [['show', '--component-id', id, *local] for id in ids for local in ([], ['--include-local-evidence'])]
            argv += [['show', '--component-id', 'absent'], ['show', '--component-id', ids[0], '--profile', 'other']]
            for args in argv:
                results = []
                for rust, root in zip((False, True), roots):
                    before = self.dump(root)
                    result = self.run_cli(root, ['transfers', 'components', *args], rust)
                    if args == ['list']:
                        self.assertEqual(result.returncode, 0, result.stderr)
                    results.append((result.returncode, result.stdout, result.stderr))
                    self.assertEqual(self.dump(root), before)
                self.assertEqual(results[0], results[1], (scenario['name'], args))
        observed = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertTrue(all(r['served_by'] == 'rust' for r in observed), observed)
        self.assertEqual({r['command'] for r in observed}, {'transfers.components.list', 'transfers.components.show'})

    def test_component_daemon_bytes_paging_privacy_and_validation_order(self):
        for scenario in self.component_scenarios():
            ids = sorted({case['args']['component_id'] for case in scenario['cases'] if case['operation'].endswith('show') and 'result' in case})
            requests = [dict(kind='ui.transfers.components.list', args=args) for args in (
                {}, dict(limit=1), dict(effective_only=True, limit=1), dict(include_local_evidence=True),
                dict(limit=True, profile='missing'), dict(limit=1001), dict(effective_only=1),
                dict(transaction='absent', effective_only=1), dict(transaction='j-ext', effective_only=1),
                dict(profile='missing', effective_only=1), dict(state=' ACTIVE '), dict(component_type='bad'),
            )]
            requests += [dict(kind='ui.transfers.components.get', args=args) for args in (
                dict(component_id=ids[0]), dict(component_id=' '+ids[0]+' ', include_local_evidence=True),
                dict(component_id='absent'), dict(component_id=True), dict(component_id=' ', profile='missing'),
                dict(component_id='\ud800'), dict(component_id=ids[0]+'\ud800'),
            )]
            results = []
            for rust in (False, True):
                root = self.book(f'daemon-{scenario["name"]}-{rust}', scenario)
                before = self.dump(root)
                results.append(self.run_daemon(root, requests, rust))
                self.assertEqual(self.dump(root), before)
                for index in (0, 1, 2, 3, 12, 13):
                    self.assertNotIn('error', json.loads(results[-1][index]), results[-1][index])
            self.assertEqual(results[0], results[1], scenario['name'])
        observed = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(observed, [dict(command='daemon', served_by='rust')]*6)

    def gap_scenarios(self):
        fixture=json.loads((ROOT/'crates/transfers/tests/gap_reads.json').read_text())
        names=('lifecycle--1-base','lifecycle--1-issues','lifecycle-1-base','lifecycle-3-base','lifecycle-1-review-conflict','lifecycle--1-competing','split-large--1-base','evidence-changed-3-base')
        return [next(s for s in fixture['scenarios'] if s['name']==name) for name in names]

    def test_gap_cli_bytes_pages_history_refusals_and_unchanged_rows(self):
        for scenario in self.gap_scenarios():
            roots=[self.book(f'gap-cli-{scenario["name"]}-{rust}',scenario) for rust in (False,True)]
            gap=next(c['args']['gap_id'] for c in scenario['cases'] if c['operation'].endswith('review') and c['args']['gap_id'] not in ('','absent'))
            commands=[['list'],['list','--limit','1'],['list','--cursor','0'],['list','--limit','0'],['list','--cursor','bad'],['review','--gap-id',gap],['review','--gap-id','absent'],['review','--gap-id','']]
            commands += [['list','--limit','1','--cursor',c['args']['cursor']] for c in scenario['cases'] if isinstance(c['args'].get('cursor'),str) and c['args']['cursor'].startswith('cgr3.')]
            for args in commands:
                results=[]
                for rust,root in zip((False,True),roots):
                    before=self.dump(root)
                    result=self.run_cli(root,['transfers','gaps',*args],rust)
                    if args==['list']:
                        self.assertEqual(result.returncode,0,(scenario['name'],result.stdout,result.stderr))
                    results.append((result.returncode,result.stdout,result.stderr))
                    self.assertEqual(self.dump(root),before)
                self.assertEqual(results[0],results[1],(scenario['name'],args))
        observed=[json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertTrue(all(r['served_by']=='rust' for r in observed),observed)
        self.assertEqual({r['command'] for r in observed},{'transfers.gaps.list','transfers.gaps.review'})

    def test_gap_daemon_bytes_projection_pages_and_validation_order(self):
        for scenario in self.gap_scenarios():
            gap=next(c['args']['gap_id'] for c in scenario['cases'] if c['operation'].endswith('review') and c['args']['gap_id'] not in ('','absent'))
            requests=[dict(kind='ui.custody.gaps.list',args=args) for args in (
                {},dict(limit=1),dict(limit=201),dict(limit=True),dict(cursor=0),dict(include_local_evidence=True),dict(limit=0,profile='missing'),dict(cursor='bad',profile='missing'),
            )]
            requests += [dict(kind='ui.custody.gaps.review_context',args=args) for args in (
                dict(gap_id=gap),dict(gap_id=' '+gap+' '),dict(gap_id='absent'),dict(gap_id=True),dict(gap_id=''),dict(gap_id=gap,limit=1),dict(gap_id=None,profile='missing'),
                dict(gap_id='\ud800'),dict(gap_id=gap+'\ud800'),
            )]
            requests.append(dict(kind='ui.custody.gaps.list',args=dict(cursor='0\ud800')))
            results=[]
            for rust in (False,True):
                root=self.book(f'gap-daemon-{scenario["name"]}-{rust}',scenario)
                before=self.dump(root)
                results.append(self.run_daemon(root,requests,rust))
                self.assertEqual(self.dump(root),before)
                for index in (0,1,8,9,10):
                    self.assertNotIn('error',json.loads(results[-1][index]),results[-1][index])
            self.assertEqual(results[0],results[1],scenario['name'])
        observed=[json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(observed,[dict(command='daemon',served_by='rust')]*8)

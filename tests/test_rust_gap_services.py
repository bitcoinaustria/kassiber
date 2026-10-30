"""Real CLI/daemon gap-authoring parity on independent synthetic book copies."""
import json
import datetime
import re
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from kassiber.db import open_db, custody_gap_review_relation_set_id, custody_gap_review_transaction_id
from tests import cli_runner
from tests.test_rust_shell_parity import _environment

ROOT = Path(__file__).resolve().parents[1]
RUST_BIN = os.environ.get('KASSIBER_RUST_BIN')


@unittest.skipUnless(RUST_BIN, 'set KASSIBER_RUST_BIN to the harness build')
class RustGapServicesTest(unittest.TestCase):
    def setUp(self):
        self.started = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(seconds=60)
        temp = tempfile.TemporaryDirectory(prefix='kassiber-gap-process-')
        self.addCleanup(temp.cleanup)
        self.base = Path(temp.name)
        self.env = _environment(self.base / 'config')
        self.env['HOME'] = os.environ['HOME']
        self.env['KASSIBER_TEST_NO_EGRESS'] = '1'
        self.fixture = json.loads((ROOT / 'crates/transfers/tests/gaps.json').read_text())
        self.scenario = self.fixture['scenarios'][0]

    def book(self, name, before=0):
        root = self.base / name
        c = open_db(root)
        for q in self.scenario['sql']:
            c.execute(q)
        c.commit()
        if before:
            # Restore declared full-row fixture state, including immutable history.
            # FK checks are disabled only while hydrating this disposable fixture.
            c.execute('PRAGMA foreign_keys=OFF')
            state = self.scenario['cases'][before-1]['rows']
            for table in reversed(self.fixture['tables']):
                c.execute(f'DELETE FROM "{table}"')
            for table in self.fixture['tables']:
                for row in self.fixture['states'][state[table]]:
                    fields = ','.join('"'+k+'"' for k in row)
                    marks = ','.join('?' for _ in row)
                    c.execute(f'INSERT INTO "{table}"({fields}) VALUES({marks})', tuple(row.values()))
            # Writes during fixture hydration bump these observation/profile markers.
            for table, keys in (('profiles', ('id',)), ('chain_index_clock', ('id',))):
                for row in self.fixture['states'][state[table]]:
                    fields = [k for k in row if k not in keys]
                    c.execute(f'UPDATE "{table}" SET '+','.join(f'"{k}"=?' for k in fields)+' WHERE id=?', tuple(row[k] for k in fields)+(row['id'],))
            c.execute('DELETE FROM chain_index_dirty')
            for row in self.fixture['states'][state['chain_index_dirty']]:
                c.execute('INSERT INTO chain_index_dirty(source_table,source_key,revision) VALUES(?,?,?)', tuple(row[k] for k in ('source_table','source_key','revision')))
            c.commit()
            c.execute('PRAGMA foreign_keys=ON')
            self.assertEqual(c.execute('PRAGMA foreign_key_check').fetchall(), [])
        c.execute("INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p')")
        c.commit()
        c.close()
        return root

    def state(self, root, review_id=None):
        c = open_db(root)
        try:
            result = {t: sorted((dict(r) for r in c.execute(f'SELECT * FROM "{t}"')), key=lambda r: json.dumps(r,sort_keys=True)) for t in self.fixture['tables']}
        finally:
            c.close()
        normalized = json.loads(self.normalize(json.dumps(result), review_id))
        return {table: sorted(rows, key=lambda row: json.dumps(row, sort_keys=True))
                for table, rows in normalized.items()}

    def normalize(self, text, review_id=None):
        # Declared identity mapping: the one newly generated review UUID and
        # its relation/impact ids. Native fixed-source tests pin their exact bytes.
        if review_id:
            identities = [(review_id, '<review>'), (custody_gap_review_relation_set_id(review_id), '<relation-set>')]
            for role, tx in (('source','out'),('return','in')):
                identities.append((custody_gap_review_transaction_id(review_id,role,tx), '<'+role+'-relation>'))
            for identity, token in identities:
                text = text.replace(identity, token)
        def clock(match):
            value = datetime.datetime.fromisoformat(match.group().replace('Z', '+00:00'))
            return '<time>' if self.started <= value <= datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(seconds=60) else match.group()
        return re.sub(r'20[0-9]{2}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?Z', clock, text)

    def test_gap_cli_bytes_rows_and_hybrid(self):
        log = self.base / 'served.jsonl'
        env = {**self.env, 'KASSIBER_HYBRID_RUST_BIN':RUST_BIN, 'KASSIBER_HYBRID_LOG':str(log)}
        for index, case in enumerate(self.scenario['cases']):
            operation = case['operation'].rsplit('.',1)[1]
            args = case['args']
            argv = ['transfers','gaps',operation,'--action',args['action'],'--gap-id',args['gap_id']]
            for key in ('classification','reason','expected_input_version'):
                if key in args:
                    argv += ['--'+key.replace('_','-'),str(args[key])]
            observed = []
            for rust in (False,True):
                root = self.book(f'cli-{index}-{rust}',before=index)
                command = ['--machine','--data-root',str(root),*argv]
                options = dict(capture_output=True,env=env,cwd=ROOT,timeout=120)
                result = (subprocess.run([sys.executable,str(ROOT/'tests/rust_port/hybrid_cli.py'),*command],**options) if rust else cli_runner.run(command,reference=True,**options))
                data = json.loads(result.stdout).get('data') or {}
                review_id = data.get('review_id')
                observed.append((result.returncode,self.normalize(result.stdout.decode(),review_id),result.stderr,self.state(root,review_id)))
            self.assertEqual(observed[0],observed[1],(index,case['args']))
        rows = [json.loads(line) for line in log.read_text().splitlines()]
        self.assertEqual(len(rows),len(self.scenario['cases']))
        self.assertTrue(all(r['served_by']=='rust' for r in rows),rows)
        self.assertEqual({r['command'] for r in rows},{'transfers.gaps.plan','transfers.gaps.apply'})

    def test_gap_daemon_bytes_rows_and_validation(self):
        log = self.base / 'daemon-served.jsonl'
        env = {**self.env, 'KASSIBER_HYBRID_RUST_BIN':RUST_BIN, 'KASSIBER_HYBRID_LOG':str(log), 'KASSIBER_HYBRID_RUST_DAEMON':'1'}
        # Each action starts with identical fixed identities, so correction
        # commitments remain byte-comparable despite the new random review UUID.
        for index,case in enumerate(self.scenario['cases']):
            applying = case['operation'].endswith('apply')
            kind = 'ui.custody.review.apply' if applying else 'ui.custody.review.plan'
            args = {k:v for k,v in case['args'].items() if k!='authored_source'}
            requests = [
                {'kind':kind,'args':{**args,'authored_source':'ai_tool'}},
                {'kind':kind,'args':{**args,'reason':42}},
                {'kind':kind,'args':{**args,'action':'unknown'}},
                {'kind':kind,'args':{**args,'gap_id':'  '}},
                {'kind':kind,'args':{**args,'reason':'x'*501}},
                {'kind':kind,'args':{**args,'action':'classify_residual','classification':42}},
                *([{'kind':kind,'args':{**args,'expected_input_version':v}}
                   for v in (True,-1,'0',None)] if applying else []),
                {'kind':kind,'args':args},
                {'kind':'daemon.shutdown'},
            ]
            observed = []
            for rust in (False,True):
                root = self.book(f'daemon-{index}-{rust}',before=index)
                payload = ''.join(json.dumps({**r,'request_id':str(i)})+'\n' for i,r in enumerate(requests)).encode()
                command = ['--data-root',str(root),'daemon']
                options = dict(input=payload,capture_output=True,env=env,cwd=ROOT,timeout=120)
                result = subprocess.run([sys.executable,str(ROOT/'tests/rust_port/hybrid_cli.py'),*command],**options) if rust else cli_runner.run(command,reference=True,**options)
                self.assertEqual(result.returncode,0,result.stderr)
                lines = result.stdout.splitlines(keepends=True)[1:]
                self.assertEqual(len(lines),len(requests))
                data = json.loads(lines[-2]).get('data') or {}
                review_id = data.get('review_id')
                observed.append((self.normalize(b''.join(lines).decode(),review_id),result.stderr,self.state(root,review_id)))
            self.assertEqual(observed[0],observed[1],(index,args))
        rows = [json.loads(line) for line in log.read_text().splitlines()]
        self.assertEqual(len(rows),len(self.scenario['cases']))
        self.assertTrue(all(r=={'command':'daemon','served_by':'rust'} for r in rows),rows)

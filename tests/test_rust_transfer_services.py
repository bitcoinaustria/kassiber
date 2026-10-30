"""Synthetic process oracles for the stored transfer/review slice."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from kassiber.db import open_db
from tests import cli_runner
from tests.test_rust_shell_parity import _environment

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")

@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustTransferServicesTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="kassiber-r5-transfers-")
        self.addCleanup(temp.cleanup)
        self.base = Path(temp.name)
        self.root = self.base / "data"
        self.env = _environment(self.base / "home")
        self.env["HOME"] = os.environ["HOME"]
        self.env["KASSIBER_TEST_NO_EGRESS"] = "1"
        fixture = json.loads((ROOT / "crates/transfers/tests/python.json").read_text())
        c = open_db(self.root)
        for statement in fixture["sql"]:
            c.execute(statement)
        c.execute("INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p')")
        c.commit()
        c.close()

    def cli(self, *args, rust=False):
        options = dict(capture_output=True, env=self.env, cwd=ROOT, timeout=120)
        argv = ["--machine", "--data-root", str(self.root), *args]
        return (subprocess.run([RUST_BIN, *argv], **options) if rust else cli_runner.run(argv, reference=True, **options))

    def test_stored_reads_and_errors_compare_as_raw_bytes(self):
        for args in [
            ("transfers", "list"), ("transfers", "payouts", "list"),
            ("transfers", "rules", "list"),
            ("transfers", "gaps", "history", "--gap-id", "gap", "--limit", "2"),
            ("transfers", "gaps", "history", "--gap-id", "gap", "--limit", "0"),
            ("review", "receipt", "--receipt-id", "receipt"),
            ("review", "receipt", "--idempotency-key", "key"),
            ("review", "receipt", "--receipt-id", "missing"),
        ]:
            expected = self.cli(*args)
            actual = self.cli(*args, rust=True)
            self.assertEqual((actual.returncode, actual.stdout, actual.stderr), (expected.returncode, expected.stdout, expected.stderr), args)

    def test_native_daemon_stored_projections_match_python(self):
        requests = [
            {"kind":"ui.transfers.list"}, {"kind":"ui.transfers.payouts.list"},
            {"kind":"ui.transfers.rules.list"},
            {"kind":"ui.custody.gaps.history","args":{"gap_id":" gap ","limit":2}},
            {"kind":"ui.custody.gaps.history","args":{"gap_id":"gap","limit":201}},
            {"kind":"ui.custody.gaps.history","args":{"gap_id":"gap","cursor":"bad"}},
            {"kind":"ui.review.receipt","args":{"receipt_id":"receipt"}},
            {"kind":"ui.review.receipt","args":{"unexpected":1}},
            {"kind":"daemon.shutdown"},
        ]
        for i, request in enumerate(requests):
            request["request_id"] = str(i)
        payload = ''.join(json.dumps(r)+'\n' for r in requests).encode()
        args = ["--data-root", str(self.root), "daemon"]
        records = []
        for rust in (False, True):
            options = dict(input=payload,capture_output=True,env=self.env,cwd=ROOT,timeout=120)
            result = (subprocess.run([RUST_BIN,*args], **options) if rust
                      else cli_runner.run(args, reference=True, **options))
            self.assertEqual(result.returncode,0,result.stderr)
            records.append(result.stdout.splitlines(keepends=True))
        self.assertEqual(records[0][1:],records[1][1:])

    def test_hybrid_routes_every_stored_command_to_rust(self):
        log = self.base / "hybrid.jsonl"
        env = {**self.env,"KASSIBER_HYBRID_RUST_BIN":RUST_BIN,"KASSIBER_HYBRID_LOG":str(log)}
        for args in [("transfers","list"),("transfers","payouts","list"),("transfers","rules","list"),("transfers","gaps","history","--gap-id","gap"),("review","receipt","--receipt-id","receipt")]:
            result = subprocess.run([sys.executable,str(ROOT/"tests/rust_port/hybrid_cli.py"),"--machine","--data-root",str(self.root),*args],env=env,cwd=ROOT,capture_output=True,timeout=120)
            self.assertEqual(result.returncode,0,result.stderr)
        observed = [json.loads(line) for line in log.read_text().splitlines()]
        self.assertEqual(len(observed),5)
        self.assertTrue(all(row["served_by"]=="rust" for row in observed),observed)

    def test_native_authoring_and_hybrid_witness_every_write(self):
        from kassiber.cli import handlers
        log = self.base / 'authoring-hybrid.jsonl'
        env = {**self.env,'KASSIBER_HYBRID_RUST_BIN':RUST_BIN,'KASSIBER_HYBRID_LOG':str(log)}
        def call(*args):
            result=subprocess.run([sys.executable,str(ROOT/'tests/rust_port/hybrid_cli.py'),'--machine','--data-root',str(self.root),*args],env=env,cwd=ROOT,capture_output=True,timeout=120)
            self.assertEqual(result.returncode,0,(result.stdout,result.stderr))
            return json.loads(result.stdout)['data']
        native=call('transfers','rules','create','--predicate','{"out_asset":"BTC","in_asset":"LBTC","max_fee_pct":"0.000_001"}','--name','€ 🚀')
        c=open_db(self.root)
        expected=handlers.create_transfer_rule(c,'w','p',predicate={'out_asset':'BTC','in_asset':'LBTC','max_fee_pct':'0.000_001'},name='€ 🚀')
        c.close()
        for key in ('id','created_at','updated_at'):
            native.pop(key,None);expected.pop(key,None)
        self.assertEqual(native,expected)
        call("transfers","rules","list")
        # Use an explicit fresh rule ID instead of assuming ordering of timestamps.
        rule=call('transfers','rules','create','--disabled')['id']
        call('transfers','rules','enable','--rule-id',rule)
        call('transfers','rules','disable','--rule-id',rule)
        call('transfers','rules','delete','--rule-id',rule)
        first=call('transfers','dismiss','--tx-out','tx0','--tx-in','tx1','--reason','reviewed','--expires-in-days','0')
        repeated=call('transfers','dismiss','--tx-out','external0','--tx-in','external1','--expires-in-days','0')
        self.assertEqual(first,repeated)
        self.assertIsNone(first['expires_at'])
        self.assertEqual(first['reason'],'reviewed')
        observed=[json.loads(line) for line in log.read_text().splitlines()]
        self.assertTrue(all(row['served_by']=='rust' for row in observed),observed)
        self.assertEqual({row['command'] for row in observed},{'transfers.rules.list','transfers.rules.create','transfers.rules.enable','transfers.rules.disable','transfers.rules.delete','transfers.dismiss'})

    def test_native_daemon_authoring_validation_and_aliases(self):
        requests=[
            {'kind':'ui.transfers.rules.create','args':{'predicate':[]}},
            {'kind':'ui.transfers.rules.create','args':{'predicate':{'out_asset':1}}},
            {'kind':'ui.transfers.rules.delete','args':{}},
            {'kind':'ui.transfers.rules.set_enabled','args':{'rule_id':'invalid','enabled':True}},
            {'kind':'ui.transfers.rules.set_enabled','args':{'rule_id':'invalid','enabled':False}},
            {'kind':'ui.transfers.dismiss','args':{'out_id':'tx1','in_id':'tx2','expires_in_days':0}},
            {'kind':'ui.transfers.dismiss','args':{'out_id':'absent','in_id':'tx2','expires_in_days':'bad'}},
            {'kind':'ui.transfers.dismiss','args':{'out_id':'absent','in_id':'tx2','expires_in_days':float('inf')}},
            {'kind':'ui.transfers.rules.create','args':{'predicate':{'out_asset':'BTC'},'enabled':False}},
            {'kind':'daemon.shutdown'},
        ]
        records=[]
        for rust in (False, True):
            payload=''.join(json.dumps({**r,'request_id':str(i)})+'\n' for i,r in enumerate(requests)).encode()
            args=['--data-root',str(self.root),'daemon']
            options=dict(input=payload,capture_output=True,env=self.env,cwd=ROOT,timeout=120)
            result=(subprocess.run([RUST_BIN,*args],**options) if rust
                    else cli_runner.run(args,reference=True,**options))
            self.assertEqual(result.returncode,0,result.stderr)
            records.append([json.loads(line) for line in result.stdout.splitlines()[1:]])
        for expected,actual in zip(*records):
            # Normalize production UUIDs and wall-clock timestamps; the native
            # row oracle independently pins all stored bytes with fixed sources.
            for key in ('id','created_at','updated_at','expires_at'):
                expected.get('data',{}).pop(key,None);actual.get('data',{}).pop(key,None)
            if (expected.get('error') or {}).get('code') == 'internal_error':
                # The shell contract omits interpreter-specific tracebacks.
                # Keep comparing all other error fields, including the message.
                self.assertIsNone(actual['error']['debug'])
                expected['error']['debug'] = None
            self.assertEqual(actual,expected)

    def _pair_book(self, name):
        root = self.base / name
        fixture = json.loads((ROOT / 'crates/transfers/tests/pairs.json').read_text())
        c = open_db(root)
        for statement in fixture['scenarios'][0]['sql']:
            c.execute(statement)
        c.execute("INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p')")
        c.commit()
        c.close()
        return root

    def test_hybrid_pair_update_unpair_are_served_by_rust(self):
        root = self._pair_book('hybrid-pairs')
        log = self.base / 'pair-hybrid.jsonl'
        env = {**self.env, 'KASSIBER_HYBRID_RUST_BIN':RUST_BIN, 'KASSIBER_HYBRID_LOG':str(log)}
        def call(*argv, ok=True):
            result = subprocess.run([sys.executable,str(ROOT/'tests/rust_port/hybrid_cli.py'),'--machine','--data-root',str(root),*argv], env=env,cwd=ROOT,capture_output=True,timeout=120)
            self.assertEqual(result.returncode,0 if ok else 1,(result.stdout,result.stderr))
            return json.loads(result.stdout)
        first=call('transfers','pair','--tx-out','d','--tx-in','e','--note','reviewed')['data']
        call('transfers','pair','--tx-out','d','--tx-in','e',ok=False)
        updated=call('transfers','update','--pair-id',first['id'],'--note','revision')['data']
        self.assertEqual(updated['id'],first['id'])
        self.assertNotEqual(updated['component_id'],first['component_id'])
        call('transfers','update','--pair-id',first['id'],'--policy','taxable',ok=False)
        call('transfers','unpair','--pair-id',first['id'])
        call('transfers','unpair','--pair-id',first['id'])
        rows=[json.loads(line) for line in log.read_text().splitlines()]
        self.assertEqual([r['command'] for r in rows],['transfers.pair']*2+['transfers.update']*2+['transfers.unpair']*2)
        self.assertTrue(all(r['served_by']=='rust' for r in rows),rows)

    def test_native_daemon_pair_revision_aliases_and_refusals_match_python(self):
        import re
        transcripts=[]
        for rust in (False,True):
            root=self._pair_book('daemon-pairs-'+str(rust))
            args=['--data-root',str(root),'daemon']
            options=dict(stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,env=self.env,cwd=ROOT)
            process=(subprocess.Popen([RUST_BIN,*args],**options) if rust else cli_runner.popen(args,reference=True,**options))
            self.addCleanup(lambda p=process: p.kill() if p.poll() is None else None)
            ready=json.loads(process.stdout.readline())
            self.assertEqual(ready['kind'],'daemon.ready')
            results=[]
            def call(kind,args):
                request=dict(kind=kind,args=args,request_id=str(len(results)))
                process.stdin.write(json.dumps(request)+'\n');process.stdin.flush()
                result=json.loads(process.stdout.readline());results.append(result)
                return result
            first=call('ui.transfers.pair',dict(out_id='d',in_id='e',note='reviewed',pair_source='bulk_selected'))['data']
            call('ui.transfers.pair',dict(tx_out='d',tx_in='e'))
            call('ui.transfers.update',dict(pair_id=first['id'],notes='revision'))
            call('ui.transfers.update',dict(pair_id=first['id'],notes=''))
            call('ui.transfers.update',dict(pair_id=first['id'],policy='taxable'))
            call('ui.transfers.update',{})
            call('ui.transfers.unpair',dict(pair_id=first['id']))
            call('ui.transfers.unpair',dict(pair_id=first['id']))
            call('ui.transfers.pair',dict(tx_out='d',tx_in='excluded'))
            call('ui.transfers.pair',dict(tx_out='d',tx_in='bad'))
            call('daemon.shutdown',{})
            process.stdin.close();process.wait(timeout=60)
            self.assertEqual(process.returncode,0,process.stderr.read())
            process.stdout.close();process.stderr.close()
            # Native fixed-source traces pin every stored byte. Process traces
            # normalize only generated UUIDs, wall-clock times and Python debug.
            replacements={}
            def normalize(value):
                if isinstance(value,dict):return {k:normalize(v) for k,v in sorted(value.items()) if k!='debug'}
                if isinstance(value,list):return [normalize(v) for v in value]
                if isinstance(value,str):
                    def identity(m):return replacements.setdefault(m[0],f'uuid-{len(replacements)}')
                    value=re.sub(r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}',identity,value)
                    return re.sub(r'202[0-9]-[0-9]{2}-[0-9]{2}T[0-9:]{8}Z','<time>',value)
                return value
            transcripts.append(normalize(results))
        self.assertEqual(transcripts[0],transcripts[1])

    def test_payout_delete_cli_daemon_and_hybrid_bytes(self):
        fixture = json.loads((ROOT / 'crates/transfers/tests/payout_delete.json').read_text())
        roots=[]
        for side in ('python','rust'):
            root=self.base/side
            c=open_db(root)
            for statement in fixture['scenarios'][0]['sql']: c.execute(statement)
            c.execute("INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p')")
            # Pin the supersession time at the storage boundary, including the
            # result of the first delete (then repeated deletes are idempotent).
            c.execute("UPDATE custody_components SET state='superseded',superseded_at='2025-06-01T00:00:00Z'")
            c.commit();c.close();roots.append(root)
        payout_id=fixture['scenarios'][0]['cases'][0]['args']['payout_id']
        log=self.base/'payout-hybrid.jsonl'
        env={**self.env,'KASSIBER_HYBRID_RUST_BIN':RUST_BIN,'KASSIBER_HYBRID_LOG':str(log)}
        for id in (payout_id,'missing',payout_id):
            argv=['--machine','--data-root',str(roots[0]),'transfers','payouts','delete','--payout-id',id]
            expected=cli_runner.run(argv,reference=True,capture_output=True,env=self.env,cwd=ROOT)
            argv[2]=str(roots[1])
            actual=subprocess.run([sys.executable,str(ROOT/'tests/rust_port/hybrid_cli.py'),*argv],capture_output=True,env=env,cwd=ROOT)
            self.assertEqual((actual.returncode,actual.stdout,actual.stderr),(expected.returncode,expected.stdout,expected.stderr))
        observed=[json.loads(line) for line in log.read_text().splitlines()]
        self.assertEqual(observed,[{'command':'transfers.payouts.delete','served_by':'rust'}]*3)
        requests=[{'request_id':str(i),'kind':'ui.transfers.payouts.delete','args':args} for i,args in enumerate(({'payout_id':payout_id},{'payout_id':'missing'},{},{'payout_id':0},{'payout_id':123},{'payout_id':payout_id,'authored_source':'untrusted'}))]
        requests.append({'request_id':'end','kind':'daemon.shutdown'})
        payload=''.join(json.dumps(r)+'\n' for r in requests).encode()
        records=[]
        for root,rust in zip(roots,(False,True)):
            argv=['--data-root',str(root),'daemon']
            opts=dict(input=payload,capture_output=True,env=self.env,cwd=ROOT,timeout=120)
            result=subprocess.run([RUST_BIN,*argv],**opts) if rust else cli_runner.run(argv,reference=True,**opts)
            self.assertEqual(result.returncode,0,result.stderr)
            records.append(result.stdout.splitlines(keepends=True)[1:])
        self.assertEqual(*records)

    def test_payout_create_cli_daemon_and_hybrid(self):
        root = self._pair_book('payout-create-hybrid')
        log = self.base / 'payout-create-served.jsonl'
        env = {**self.env, 'KASSIBER_HYBRID_RUST_BIN': RUST_BIN, 'KASSIBER_HYBRID_LOG': str(log)}
        for tx, extra, ok in [('d', ['--note',' € 🚀 '], True), ('d', [], False), ('missing', [], False), ('f', ['--payout-fiat-value','-1'], False)]:
            argv=['--machine','--data-root',str(root),'transfers','payouts','create','--tx-out',tx,'--payout-asset','ETH','--payout-amount','0.000000008',*extra]
            actual=subprocess.run([sys.executable,str(ROOT/'tests/rust_port/hybrid_cli.py'),*argv],capture_output=True,env=env,cwd=ROOT)
            self.assertEqual(actual.returncode,0 if ok else 1,(actual.stdout,actual.stderr))
        self.assertEqual([json.loads(line) for line in log.read_text().splitlines()],[{'command':'transfers.payouts.create','served_by':'rust'}]*4)
        for extra in (['--tx-out','absent'], ['--tx-out','f','--payout-fiat-value','-1'], ['--tx-out','f','--payout-occurred-at','bad']):
            argv=['--machine','--data-root',str(root),'transfers','payouts','create','--payout-asset','ETH','--payout-amount','0.000000008',*extra]
            opts=dict(capture_output=True,env=self.env,cwd=ROOT)
            expected=cli_runner.run(argv,reference=True,**opts)
            actual=subprocess.run([RUST_BIN,*argv],**opts)
            self.assertEqual((actual.returncode,actual.stdout,actual.stderr),(expected.returncode,expected.stdout,expected.stderr))
        records=[]
        for rust in (False,True):
            root=self._pair_book('payout-create-daemon-'+str(rust))
            requests=[{'kind':'ui.transfers.payouts.create','args':args} for args in (
                {'out_id':'d','payout_asset':'ETH','payout_amount':'0.000000008','note':' € 🚀 ','payout_fiat_value':'1.2345','authored_source':'untrusted'},
                {'tx_out':'absent','payout_asset':'ETH','payout_amount':'1'},
                {'tx_out':'f','payout_asset':'ETH','payout_amount':'0'},
                {'tx_out':'f','payout_asset':'ETH','payout_amount':'1','policy':'bad'},
            )]+[{'kind':'daemon.shutdown'}]
            payload=''.join(json.dumps({**r,'request_id':str(i)})+'\n' for i,r in enumerate(requests)).encode()
            opts=dict(input=payload,capture_output=True,env=self.env,cwd=ROOT,timeout=120)
            result=subprocess.run([RUST_BIN,'--data-root',str(root),'daemon'],**opts) if rust else cli_runner.run(['--data-root',str(root),'daemon'],reference=True,**opts)
            self.assertEqual(result.returncode,0,result.stderr)
            rows=[json.loads(line) for line in result.stdout.splitlines()[1:]]
            for row in rows:
                for key in ('id','component_id','created_at'):row.get('data',{}).pop(key,None)
            records.append(rows)
            self.assertEqual(rows[0].get("kind"), "ui.transfers.payouts.create", rows)
            c=open_db(root)
            self.assertEqual(c.execute("SELECT authored_source FROM custody_components WHERE state='active'").fetchone()[0],'gui',rows)
            c.close()
        self.assertEqual(*records)

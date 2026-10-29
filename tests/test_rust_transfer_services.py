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
        for command in ([sys.executable, "-m", "kassiber"], [RUST_BIN]):
            result = subprocess.run([*command,*args],input=payload,capture_output=True,env=self.env,cwd=ROOT,timeout=120)
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

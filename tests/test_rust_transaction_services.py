"""Boundary parity for the R5 local evidence and loan service slice."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from kassiber.db import open_db
from tests import cli_runner
from tests.test_rust_shell_parity import _environment

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the test-harness binary")
class RustTransactionServicesTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-r5-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.root = self.base / "data"
        self.env = _environment(self.base / "home")
        # The test candidate can route every setup call independently.
        self.ok(self.cli("init"))
        self.ok(self.cli("workspaces", "create", "Main"))
        self.ok(self.cli("profiles", "create", "Book", "--workspace", "Main"))
        self.ok(self.cli("wallets", "create", "--label", "Cold", "--workspace", "Main", "--profile", "Book", "--kind", "address", "--address", "bc1qsynthetic"))
        conn = open_db(str(self.root))
        workspace, profile, wallet = conn.execute("SELECT workspace_id,profile_id,id FROM wallets WHERE label='Cold'").fetchone()
        conn.execute("INSERT OR REPLACE INTO settings(key,value) VALUES('context_workspace',?),('context_profile',?)", (workspace,profile))
        for txid, direction in (("out", "outbound"), ("in", "inbound")):
            conn.execute("""INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,kind,created_at)
                VALUES(?,?,?,?,?,'BTC',?,100000001,0,'2025-01-01T00:00:00Z','deposit','2025-01-01T00:00:00Z')""", (txid,workspace,profile,wallet,txid,direction))
        conn.commit()
        conn.close()

    def cli(self, *args, reference=False, rust=False):
        argv = ["--machine", "--data-root", str(self.root), *args]
        options = dict(env=self.env, capture_output=True, cwd=ROOT, timeout=120)
        return (subprocess.run([RUST_BIN, *argv], **options) if rust
                else cli_runner.run(argv, reference=reference, **options))

    def ok(self, result):
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        return json.loads(result.stdout)["data"]

    def same_read(self, *args):
        reference = self.cli(*args, reference=True)
        rust = self.cli(*args, rust=True)
        self.assertEqual((rust.returncode,rust.stdout,rust.stderr), (reference.returncode,reference.stdout,reference.stderr))

    def test_loan_round_trip_and_byte_exact_reads_and_refusals(self):
        self.ok(self.cli("loans", "mark", "--txid", "out", "--as", "collateral", "--loan-id", "first"))
        self.ok(self.cli("loans", "mark", "--txid", "in", "--as", "returned", "--loan-id", "second"))
        linked = self.ok(self.cli("loans", "link", "--txid", "in", "--txid", "out"))
        self.assertEqual(linked["loan_id"], "second")
        self.same_read("loans", "list")
        self.same_read("loans", "mark", "--txid", "out", "--as", "returned")
        self.same_read("loans", "unmark", "--txid", "unknown")
        self.ok(self.cli("loans", "unmark", "--txid", "out"))
        self.same_read("loans", "list")

    def test_attachment_round_trip_and_byte_exact_reads(self):
        source = self.base / "receipt.txt"
        source.write_bytes(b"synthetic evidence\n")
        file = self.ok(self.cli("attachments", "add", "--transaction", "in", "--file", str(source)))
        url = self.ok(self.cli("attachments", "add", "--transaction", "in", "--url", "https://127.0.0.1:1/receipt.pdf"))
        self.ok(self.cli("attachments", "rename", url["id"], "--label", "Board decision"))
        self.ok(self.cli("attachments", "list"))
        self.same_read("attachments", "list")
        self.same_read("attachments", "verify")
        self.same_read("attachments", "rename", file["id"], "--label", "forbidden")
        self.ok(self.cli("attachments", "remove", file["id"]))
        self.same_read("attachments", "list")
        root = self.base / "attachments"
        (root / "orphan.bin").write_bytes(b"orphan")
        self.same_read("attachments", "gc", "--dry-run")
        self.ok(self.cli("attachments", "gc"))
        self.assertFalse((root / "orphan.bin").exists())

    def test_daemon_reads_and_validation_match_reference_bytes(self):
        self.ok(self.cli("loans", "mark", "--txid", "out", "--as", "principal-repaid"))
        url = self.ok(self.cli("attachments", "add", "--transaction", "in", "--url", "https://example.test/invoice.pdf"))
        requests = [
            {"kind":"ui.loans.list"},
            {"kind":"ui.loans.mark", "args":{"txid":"out","as":"returned"}},
            {"kind":"ui.loans.link", "args":{"txids":"bad"}},
            {"kind":"ui.loans.unmark", "args":{"txid":"absent"}},
            {"kind":"ui.loans.mark", "args":{"txid":3}},
            {"kind":"ui.attachments.list", "args":{"limit":201}},
            {"kind":"ui.attachments.list", "args":{"limit":10**100}},
            {"kind":"ui.attachments.list", "args":{"limit":str(10**100)}},
            {"kind":"ui.attachments.list", "args":{"limit":0}},
            {"kind":"ui.attachments.list", "args":{"unexpected":True}},
            {"kind":"ui.attachments.open", "args":{"attachment":url["id"]}},
            {"kind":"ui.attachments.rename", "args":{"attachment":url["id"],"label":" "}},
            {"kind":"ui.attachments.copy", "args":{"transaction":"in","source_transaction":"in","attachments":[url["id"]]}},
            {"kind":"daemon.shutdown"},
        ]
        wire = "".join(json.dumps({"request_id":str(i),**request})+"\n" for i,request in enumerate(requests)).encode()
        argv = ["--machine","--data-root",str(self.root),"daemon"]
        options = dict(env=self.env, input=wire, capture_output=True, cwd=ROOT,timeout=120)
        python = cli_runner.run(argv, reference=True, **options)
        rust = subprocess.run([RUST_BIN,*argv], **options)
        self.assertEqual((python.returncode,python.stderr),(0,b""))
        self.assertEqual((rust.returncode,rust.stderr),(0,b""))
        self.assertEqual(rust.stdout.splitlines()[1:],python.stdout.splitlines()[1:])

    def test_unported_families_fail_before_bootstrap(self):
        from tests.test_rust_hybrid_candidate import unported_read_witnesses

        witnesses = unported_read_witnesses(RUST_BIN, self.base)[:1]
        if not witnesses:
            self.skipTest("every database read is native in this build; "
                          "crates/kassiber/tests/binary.rs pins the gated leaves of product builds")
        for command in witnesses:
            absent = self.base / "uncreated-book"
            result = subprocess.run([RUST_BIN,"--machine","--data-root",str(absent),*command], env=self.env, capture_output=True, timeout=120)
            self.assertEqual(json.loads(result.stdout)["error"]["code"],"not_implemented")
            self.assertFalse(absent.exists())

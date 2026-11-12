"""Rust writes wait for another connection's writer instead of failing busy.

A write transaction begun deferred reads first and then upgrades its lock.
SQLite refuses that upgrade at once while another connection holds the writer,
without the busy handler, so the request fails as `database_busy`. Rust write
transactions now take the writer lock first (docs/reference/rust-write-transactions.md).
Each case here holds the writer from a second connection, runs one native
write through the Rust binary, and requires success after waiting, on a
plaintext and a SQLCipher book. The daemon cases matter most: a daemon opens
the book once, so its later writes meet the held writer inside their own
transaction (a CLI process already waits for it while opening the book). The
document import also has `tests/rust_port/repro_document_import_busy.py
--impl rust --hold-writer 2`.

`KASSIBER_WRITE_LOCK_COPIES=<n>` repeats each case n times (default 1).
"""
from contextlib import closing
import json
import os
import sqlite3
import subprocess
import threading
import time
import unittest

from kassiber.db import open_db
from kassiber.secrets.sqlcipher import open_encrypted
from tests import cli_runner
from tests.rust_desktop_parity import RUST_BIN, DesktopParity, run_candidate
from tests import test_rust_wallet_document_import as document_import
from tests.test_rust_wallet_document_import import IMPORT, PREVIEW, STAGE
from tests.test_rust_wallet_identify_onchain import Session

HOLD = 1.0
NOW = "2026-01-01T00:00:00Z"
COPIES = int(os.environ.get("KASSIBER_WRITE_LOCK_COPIES", "1"))
TRANSACTIONS = (
    ("out", "source-wallet", "outbound", 100_000, None, NOW),
    ("in", "sink-wallet", "inbound", 99_000, None, NOW),
    ("buy", "sink-wallet", "inbound", 5_000_000, None, "2024-02-01T00:00:00Z"),
    ("sell", "sink-wallet", "outbound", 1_000_000, 50_000, "2024-03-01T00:00:00Z"),
)


def hold_writer(test, seconds=HOLD):
    """Hold SQLite's writer lock from another connection, as a daemon worker does."""
    held, path = threading.Event(), test.root / "kassiber.sqlite3"

    def run():
        conn = open_encrypted(path, test.passphrase) if test.passphrase else sqlite3.connect(path)
        try:
            conn.execute("BEGIN IMMEDIATE")
            held.set()
            time.sleep(seconds)
            conn.commit()
        finally:
            conn.close()

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    test.addCleanup(thread.join, 60)
    test.assertTrue(held.wait(60))


def daemon(test, root):
    """A Rust daemon session on `root`, unlocked when the book is encrypted."""
    fds, argv = [], ["--machine", "--data-root", str(root)]
    if test.passphrase:
        read_end, write_end = os.pipe()
        os.write(write_end, test.passphrase.encode())
        os.close(write_end)
        fds.append(read_end)
        argv += ["--db-passphrase-fd", str(read_end)]
    process = subprocess.Popen([RUST_BIN, *argv, "daemon"], cwd=cli_runner.ROOT, env=test.env, pass_fds=fds,
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    for fd in fds:
        os.close(fd)
    test.addCleanup(lambda: process.poll() is None and process.kill())
    session = Session(process)

    def step(request_id, kind, args, *, held=False):
        if held:
            hold_writer(test)
        started = time.monotonic()
        session.send({"request_id": request_id, "kind": kind, "args": args})
        session.terminal(request_id)
        reply = json.loads(session.seen[-1])
        test.assertEqual(reply["kind"], kind, reply)
        if held:
            # It succeeded by waiting for the writer, not by racing ahead of it.
            test.assertGreaterEqual(time.monotonic() - started, HOLD * 0.5, kind)
        return reply["data"]

    if test.passphrase:
        session.send({"request_id": "unlock", "kind": "daemon.unlock",
                      "args": {"auth_response": {"passphrase_secret": test.passphrase}}})
        session.terminal("unlock")
        test.assertNotEqual(json.loads(session.seen[-1])["kind"], "error", session.seen[-1])
    return session, step


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN names the Rust binary")
class RustWritesWaitForTheWriter(DesktopParity, unittest.TestCase):
    SEED = None

    def seed(self):
        with closing(open_db(self.root, passphrase=self.passphrase)) as conn:
            conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Main',?)", (NOW,))
            conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('profile','ws','Book',?)", (NOW,))
            conn.execute("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) "
                         "VALUES('treasury','ws','profile','treasury','Treasury','asset',?)", (NOW,))
            for wallet_id, label in (("source-wallet", "Source"), ("sink-wallet", "Sink")):
                conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                             "VALUES(?,'ws','profile',?,'custom','{}',?)", (wallet_id, label, NOW))
            for tx_id, wallet_id, direction, amount, rate, occurred in TRANSACTIONS:
                conn.execute(
                    "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,"
                    "occurred_at,direction,asset,amount,fee,fiat_rate,created_at) "
                    "VALUES(?,'ws','profile',?,?,?,?,?,'BTC',?,0,?,?)",
                    (tx_id, wallet_id, f"fingerprint-{tx_id}", f"external-{tx_id}", occurred,
                     direction, amount, rate, occurred))
            conn.execute("INSERT INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','profile')")
            conn.commit()

    def rust(self, *argv, root=None):
        args = ["--machine", "--data-root", str(root or self.root)]
        if self.passphrase:
            args += ["--db-passphrase-fd", cli_runner.InputFd(self.passphrase.encode())]
        return run_candidate([*args, *argv], env=self.env, input=None)

    def ok(self, result):
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        envelope = json.loads(result.stdout)
        self.assertNotEqual(envelope["kind"], "error", envelope)
        return envelope["data"]

    def hold_writer(self, seconds=HOLD):
        hold_writer(self, seconds)

    def waits(self, *argv):
        """Run one write while another connection holds the writer; it must wait and succeed."""
        self.hold_writer()
        started = time.monotonic()
        data = self.ok(self.rust(*argv))
        self.assertGreaterEqual(time.monotonic() - started, HOLD * 0.5, argv)
        return data

    def input_version(self):
        with closing(open_db(self.root, passphrase=self.passphrase)) as conn:
            return int(conn.execute("SELECT journal_input_version FROM profiles").fetchone()[0] or 0)

    def check_writes(self, *, backup):
        self.waits("metadata", "notes", "set", "--transaction", "sell", "--note", "held writer")
        self.waits("metadata", "tags", "create", "--code", "held", "--label", "Held")
        self.waits("wallets", "create", "--label", "Held", "--kind", "custom")
        self.waits("rates", "set", "BTC-EUR", "2024-02-01T00:00:00Z", "40000")
        source = self.base / "import.json"
        source.write_text(json.dumps([
            {"external_id": "held-1", "occurred_at": "2024-01-01T00:00:00Z", "amount": "0.1", "asset": "BTC"},
        ]), encoding="utf-8")
        batch = self.waits("wallets", "import-json", "--wallet", "Source", "--file", str(source))["import_batch_id"]
        self.waits("imports", "rollback", "--batch", batch, "--confirm")
        self.waits("journals", "process")
        plan = self.base / "operations.json"
        plan.write_text(json.dumps([{"type": "exclude", "transaction_id": "sell", "reason": "Duplicate import"}]))
        artifact = self.base / "artifact.json"
        artifact.write_text(json.dumps(self.ok(self.rust(
            "review", "plan", "--operations-file", str(plan), "--expected-input-version", str(self.input_version())))))
        self.waits("review", "apply", "--artifact-file", str(artifact), "--idempotency-key", "held-writer")
        # Last: a new workspace becomes the context.
        self.waits("workspaces", "create", "Held workspace")
        if not backup:
            return  # Backups refuse a plaintext book.
        archive = self.base / "book.kassiber-backup"
        args = ["--backup-passphrase-fd", cli_runner.InputFd(b"synthetic backup passphrase")]
        self.waits("backup", "export", "--file", str(archive), *args)
        target = self.base / "restored" / "data"
        self.ok(self.rust("backup", "import", str(archive), "--install", "--target-data-root", str(target), *args,
                          root=target))

    def check_daemon_writes(self):
        session, step = daemon(self, self.root)
        step("note", "ui.transactions.metadata.update", {"transaction": "sell", "note": "held writer"}, held=True)
        step("wallet", "ui.wallets.create", {"label": "Held", "kind": "custom"}, held=True)
        step("process", "ui.journals.process", {}, held=True)
        artifact = step("plan", "ui.review.plan", {
            "operations": [{"type": "exclude", "transaction_id": "sell", "reason": "Duplicate import"}],
            "expected_input_version": self.input_version()})
        step("apply", "ui.review.apply", {"artifact": artifact, "idempotency_key": "held-writer"}, held=True)
        step("workspace", "ui.workspace.create", {"label": "Held desktop"}, held=True)
        _, stderr = session.finish()
        self.assertEqual(stderr, b"")

    def test_plaintext_daemon_writes_wait_for_a_held_writer(self):
        self.check_daemon_writes()

    def test_sqlcipher_daemon_writes_wait_for_a_held_writer(self):
        self.encrypt("synthetic write-lock key")
        self.check_daemon_writes()

    def test_plaintext_writes_wait_for_a_held_writer(self):
        self.check_writes(backup=False)

    def test_sqlcipher_writes_wait_for_a_held_writer(self):
        self.encrypt("synthetic write-lock key")
        self.check_writes(backup=True)


class RustDocumentImportWaitsForTheWriter(document_import.RustDocumentImportTest):
    """Stage, preview, then import while another connection holds the writer."""

    def check_import(self):
        session, step = daemon(self, self.root)
        token = step("stage", STAGE, {"source_file": str(self.source)})["document_token"]
        preview = step("preview", PREVIEW, {"document_token": token, "provider": "local"})
        ready = [row["id"] for row in preview["rows"] if row["status"] == "ready"]
        step("import", IMPORT, {"document_token": preview["document_token"], "wallet": "imports",
                                "selected_row_ids": ready}, held=True)
        # A reviewed import ends the daemon's session.
        session.process.stdin.close()
        session.process.wait(timeout=60)

    def plaintext_document_import_waits(self):
        self.check_import()

    def sqlcipher_document_import_waits(self):
        self.encrypt("synthetic document key")
        self.check_import()


# Only this module's own cases; the inherited parity cases run in their own module.
for _name in [name for name in dir(document_import.RustDocumentImportTest) if name.startswith("test_")]:
    setattr(RustDocumentImportWaitsForTheWriter, _name, None)
RustDocumentImportWaitsForTheWriter.test_plaintext_document_import_waits_for_a_held_writer = (
    RustDocumentImportWaitsForTheWriter.plaintext_document_import_waits)
RustDocumentImportWaitsForTheWriter.test_sqlcipher_document_import_waits_for_a_held_writer = (
    RustDocumentImportWaitsForTheWriter.sqlcipher_document_import_waits)

for _index in range(1, COPIES):
    for _case in (RustWritesWaitForTheWriter, RustDocumentImportWaitsForTheWriter):
        for _name in [name for name, value in vars(_case).items() if name.startswith("test_") and value
                      and not name[-3:].isdigit()]:
            setattr(_case, f"{_name}_{_index:03}", getattr(_case, _name))


if __name__ == "__main__":
    unittest.main()

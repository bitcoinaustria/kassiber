"""Check an encrypted book's document import waits for a concurrent writer.

    KASSIBER_RUST_BIN=target/debug/kassiber \
    uv run --locked python -m tests.rust_port.repro_document_import_busy \
        [--impl python|python-no-watch-worker|rust] [--copies N] [--workers N] \
        [--hold-writer SECONDS]

`RustDocumentImportTest.test_sqlcipher_import_matches_python` stages, previews
and imports right after `daemon.unlock`. Under load the Python reference's
import sometimes answered `database_busy` (SQLITE_BUSY, code 5) at once instead
of waiting: 2 of 78 and 24 of 96 copies at 16 workers on 2026-11-07.

Cause: the import opened its transaction with a bare SAVEPOINT, which SQLite
begins as a deferred transaction, and read before its first write. Unlock
starts the local-evidence watch worker (encrypted books only), whose `open_db`
takes the writer lock for the open-time schema work (it drops and recreates
triggers on every open). A connection that reads inside a transaction and then
asks for the writer lock while another connection holds it gets SQLITE_BUSY
without its busy handler, because waiting could deadlock, so `busy_timeout`
never applied. An encrypted book runs SQLCipher's SQLite 3.51.1 in
rollback-journal mode; WAL refuses the same upgrade. Before the fix,
`python-no-watch-worker` (the same daemon with the worker disabled) was never
busy in 96 copies while `python` was in 3, and `--hold-writer 2` (another
connection holds the writer lock while the import starts) failed every copy
of both `python` and `rust`. Python's write transactions now take the writer
lock first (`kassiber.db.begin_write_savepoint`, BEGIN IMMEDIATE) and wait.

Each copy runs one daemon of the chosen implementation on its own encrypted
book: unlock, stage, preview against a loopback OCR fixture, import. Exit 0
means every import succeeded (with `--hold-writer`, by waiting for the
writer).
"""
import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
MODULE = '''
import json
import os
import subprocess
import sys
import threading
import time

from kassiber.secrets.sqlcipher import open_encrypted
from tests import cli_runner
from tests.test_rust_wallet_document_import import IMPORT, PREVIEW, STAGE, RustDocumentImportTest as _Base
from tests.test_rust_wallet_identify_onchain import Session

IMPL = {impl!r}
HOLD_WRITER = {hold_writer!r}
NO_WATCH_WORKER = (
    "import sys; from kassiber import daemon; daemon._start_watch_worker = lambda *a, **k: None; "
    "from kassiber.cli.main import main; sys.argv[0] = 'kassiber'; sys.exit(main())"
)


class Repro(_Base):
    def import_once(self):
        self.encrypt("synthetic document key")
        read_end, write_end = os.pipe()
        os.write(write_end, self.passphrase.encode())
        os.close(write_end)
        argv = ["--machine", "--data-root", str(self.root), "--db-passphrase-fd", str(read_end), "daemon"]
        head = {{"rust": [os.environ["KASSIBER_RUST_BIN"]], "python": [sys.executable, "-m", "kassiber"],
                 "python-no-watch-worker": [sys.executable, "-c", NO_WATCH_WORKER]}}[IMPL]
        process = subprocess.Popen([*head, *argv], cwd=cli_runner.ROOT, env=self.env, pass_fds=[read_end],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        os.close(read_end)
        try:
            session = Session(process)

            def step(request_id, kind, args):
                session.send({{"request_id": request_id, "kind": kind, "args": args}})
                session.terminal(request_id)
                return json.loads(session.seen[-1])

            unlock = step("unlock", "daemon.unlock", {{"auth_response": {{"passphrase_secret": self.passphrase}}}})
            assert unlock["kind"] != "error", unlock
            token = step("stage", STAGE, {{"source_file": str(self.source)}})["data"]["document_token"]
            preview = step("preview", PREVIEW, {{"document_token": token, "provider": "local"}})
            assert "data" in preview, preview
            ready = [row["id"] for row in preview["data"]["rows"] if row["status"] == "ready"]
            if HOLD_WRITER:
                self.hold_writer(HOLD_WRITER)
            started = time.monotonic()
            reply = step("import", IMPORT, {{"document_token": preview["data"]["document_token"], "wallet": "imports",
                                             "selected_row_ids": ready}})
            with open(os.environ["REPRO_IMPORT_REPLIES"], "a", encoding="utf-8") as replies:
                replies.write(json.dumps(reply.get("error") or reply["kind"]) + "\\n")
            if HOLD_WRITER and reply["kind"] == IMPORT:
                # It succeeded by waiting for the writer, not by racing ahead of it.
                self.assertGreaterEqual(time.monotonic() - started, HOLD_WRITER * 0.5)
            self.assertEqual(reply["kind"], IMPORT, reply)
            # A reviewed import ends the daemon's session.
            process.stdin.close()
            process.wait(timeout=60)
        finally:
            if process.poll() is None:
                process.kill()

    def hold_writer(self, seconds):
        """Hold SQLite's writer lock from another connection, as a background writer does."""
        held, path = threading.Event(), self.root / "kassiber.sqlite3"

        def run():
            conn = open_encrypted(path, self.passphrase)
            try:
                conn.execute("BEGIN IMMEDIATE")
                held.set()
                time.sleep(seconds)
                conn.commit()
            finally:
                conn.close()

        thread = threading.Thread(target=run, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 60)
        assert held.wait(60)


for _index in range({copies}):
    setattr(Repro, f"test_copy{{_index:03}}", Repro.import_once)
'''


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--impl", choices=("python", "python-no-watch-worker", "rust"), default="python")
    parser.add_argument("--copies", type=int, default=96)
    parser.add_argument("--workers", type=int, default=16)
    parser.add_argument("--hold-writer", type=float, default=0.0, metavar="SECONDS",
                        help="hold the writer lock from another connection while the import starts")
    options = parser.parse_args()
    # The copies must sit under tests/ so the suite's conftest arms them.
    with tempfile.TemporaryDirectory(prefix="repro_docimport_", dir=ROOT / "tests") as directory:
        module = Path(directory) / "test_repro_document_import_busy.py"
        module.write_text(MODULE.format(impl=options.impl, copies=options.copies, hold_writer=options.hold_writer))
        replies_path = Path(directory) / "replies.jsonl"
        replies_path.touch()
        result = subprocess.run(
            [sys.executable, "-m", "pytest", str(module), "-k", "Repro and copy", "-n", str(options.workers),
             "--dist", "worksteal", "-q", "-p", "no:cacheprovider", "-rN", *([] if not os.environ.get("REPRO_DEBUG") else ["-x", "--tb=short"])],
            cwd=ROOT, capture_output=True, text=True, env={**os.environ, "REPRO_IMPORT_REPLIES": str(replies_path)})
        replies = replies_path.read_text().splitlines()
    busy = sum("database_busy" in reply for reply in replies)
    tail = result.stdout.strip().splitlines()[-1] if result.stdout.strip() else result.stderr[-2000:]
    if os.environ.get("REPRO_DEBUG"):
        print(result.stdout[-5000:])
    print(f"{options.impl}{' holding the writer' if options.hold_writer else ''}: {tail}; {len(replies)} import replies, {busy} database_busy")
    for reply in sorted(set(replies)):
        print(f"  {replies.count(reply)} x {reply}")
    return 0 if result.returncode == 0 and len(replies) == options.copies and not busy else 1


if __name__ == "__main__":
    raise SystemExit(main())

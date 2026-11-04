"""Opt-in on-chain identification: native streaming and cancellation compared
line for line with the Python daemon.

Each session drives the Python reference and then the Rust candidate against
the same loopback Esplora/Electrum fixtures, sending each request only after
the previous step's records arrived, so progress, cancellation and terminal
records are comparable byte for byte. Only the explicit request reaches the
fixtures; local identification never does.
"""
import json
import os
import queue
import socket
import subprocess
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from kassiber.db import open_db
from kassiber.egress_policy import set_offline_mode
from tests import cli_runner
from tests.rust_desktop_parity import DESKTOP_SQL, RUST_BIN, DesktopParity

LISTED = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
LISTED_SCRIPT = "0014e8df018c7e326cc253faac7e46cdc51e68542c42"
EXTERNAL = "0014" + "ee" * 20
KNOWN = {
    "a" * 64: {"vin": [{"txid": "9" * 64, "vout": 1, "prevout": {"scriptpubkey": EXTERNAL}}],
               "vout": [{"scriptpubkey": LISTED_SCRIPT, "value": 5}, {"scriptpubkey": "6a01ff", "value": 0}]},
    "b" * 64: {"vin": [{"txid": "c" * 64, "vout": 0, "prevout": {"scriptpubkey": LISTED_SCRIPT}}],
               "vout": [{"scriptpubkey": EXTERNAL, "value": 4}]},
    "d" * 64: {"vin": [{"txid": "e" * 64, "vout": 0}], "vout": [{"scriptpubkey": LISTED_SCRIPT}]},
}
# A raw transaction spending 99…99:1 to the listed script and an OP_RETURN.
RAW = ("0200000001" + "99" * 32 + "01000000" + "00" + "ffffffff" + "02"
       + "0500000000000000" + "16" + LISTED_SCRIPT + "0000000000000000" + "03" + "6a01ff" + "00000000")


class Esplora:
    """Serves `/tx/<txid>`; a held txid blocks until released."""

    def __init__(self):
        self.requests = []
        self.hold = None
        self.held = threading.Event()
        self.release = threading.Event()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                owner.requests.append(self.path)
                txid = self.path.rsplit("/", 1)[-1]
                if txid == owner.hold:
                    owner.held.set()
                    owner.release.wait(30)
                body = KNOWN.get(txid)
                self.send_response(200 if body else 404)
                payload = json.dumps(body).encode() if body else b"Transaction not found"
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def log_message(self, *args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"

    def reset(self, hold=None):
        self.requests.clear()
        self.hold = hold
        self.held.clear()
        self.release.clear()

    def close(self):
        self.release.set()
        self.server.shutdown()
        self.server.server_close()


class Electrum:
    """Newline JSON-RPC: version negotiation and raw transactions."""

    def __init__(self):
        self.calls = []
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(8)
        self.url = f"tcp://127.0.0.1:{self.listener.getsockname()[1]}"
        threading.Thread(target=self.accept, daemon=True).start()

    def accept(self):
        while True:
            try:
                connection, _ = self.listener.accept()
            except OSError:
                return
            threading.Thread(target=self.serve, args=(connection,), daemon=True).start()

    def serve(self, connection):
        with connection, connection.makefile("rwb") as stream:
            for line in stream:
                request = json.loads(line)
                self.calls.append(request["method"])
                if request["method"] == "server.version":
                    reply = {"result": ["Fixture 1.0", "1.4"]}
                elif request["params"] == ["a" * 64]:
                    reply = {"result": RAW}
                else:
                    reply = {"error": {"code": 2, "message": "missing transaction"}}
                stream.write(json.dumps({"jsonrpc": "2.0", "id": request["id"], **reply}).encode() + b"\n")
                stream.flush()

    def close(self):
        self.listener.close()


class Session:
    """One daemon process; records are read on a thread, steps wait for them."""

    def __init__(self, process):
        self.process = process
        self.lines = queue.Queue()
        self.seen = []
        self.kinds = {}
        threading.Thread(target=self.pump, daemon=True).start()
        self.next_line()  # daemon.ready

    def pump(self):
        for line in self.process.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def next_line(self):
        line = self.lines.get(timeout=120)
        assert line is not None, "daemon exited early"
        return line

    def send(self, request):
        self.kinds[json.dumps(request["request_id"])] = request["kind"]
        self.process.stdin.write(json.dumps(request).encode() + b"\n")
        self.process.stdin.flush()

    def until(self, predicate):
        while True:
            line = self.next_line()
            self.seen.append(line)
            if predicate(json.loads(line)):
                return

    def terminal(self, request_id):
        self.until(lambda r: r.get("request_id") == request_id and not r["kind"].endswith(".progress"))

    def finish(self):
        self.send({"request_id": "stop", "kind": "daemon.shutdown"})
        self.terminal("stop")
        self.process.stdin.close()
        self.process.wait(timeout=60)
        return self.seen, self.process.stderr.read()


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustIdentifyOnchainTest(DesktopParity, unittest.TestCase):
    KIND = "ui.wallets.identify_onchain"

    def setUp(self):
        self.esplora = Esplora()
        self.electrum = Electrum()
        self.addCleanup(self.esplora.close)
        self.addCleanup(self.electrum.close)
        super().setUp()
        # Loopback fixtures only: the product switch would refuse them.
        del self.env["KASSIBER_NO_EGRESS"]

    def seed(self):
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript(DESKTOP_SQL.read_text())
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                         "VALUES('list','w','p','Watch list','address',?,'2025')",
                         (json.dumps({"addresses": [LISTED]}),))
            conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,"
                         "occurred_at,direction,asset,amount,fee,amount_includes_fee,excluded,raw_json,created_at) "
                         "VALUES('local','w','p','list',?,'local-fp','2025','inbound','BTC',1,0,0,0,?,'2025')",
                         ("f" * 64, json.dumps({"txid": "f" * 64, "vin": [], "vout": [{"scriptpubkey": LISTED_SCRIPT}]})))
            conn.executescript(
                "INSERT INTO backends(name,kind,chain,network,url,created_at,updated_at) VALUES"
                f"('esp','esplora','bitcoin','main','{self.esplora.url}','2025','2025'),"
                f"('espnochain','mempool',NULL,NULL,'{self.esplora.url}','2025','2025'),"
                f"('elx','electrum','bitcoin','main','{self.electrum.url}','2025','2025'),"
                f"('elxnochain','electrum',NULL,NULL,'{self.electrum.url}','2025','2025'),"
                "('core','bitcoinrpc','bitcoin','main','http://127.0.0.1:1','2025','2025');")
        self.change("")

    def sessions(self, script, *, hold=None):
        """Run `script(session, fixture)` on Python, then on Rust; compare."""
        outputs = []
        for reference in (True, False):
            self.esplora.reset(hold)
            self.electrum.calls.clear()
            argv = ["--machine", "--data-root", str(self.root)]
            options = dict(env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            if reference:
                if self.passphrase:
                    argv += ["--db-passphrase-fd", cli_runner.InputFd(self.passphrase.encode())]
                process = cli_runner.popen([*argv, "daemon"], reference=True, **options)
            else:
                fds = []
                if self.passphrase:
                    read_end, write_end = os.pipe()
                    os.write(write_end, self.passphrase.encode())
                    os.close(write_end)
                    fds.append(read_end)
                    argv += ["--db-passphrase-fd", str(read_end)]
                process = subprocess.Popen([RUST_BIN, *argv, "daemon"], cwd=cli_runner.ROOT, pass_fds=fds, **options)
                for fd in fds:
                    os.close(fd)
            session = Session(process)
            if self.passphrase:
                session.send({"request_id": "unlock", "kind": "daemon.unlock",
                              "args": {"auth_response": {"passphrase_secret": self.passphrase}}})
                session.terminal("unlock")
            try:
                script(session)
                seen, stderr = session.finish()
            finally:
                if process.poll() is None:
                    process.kill()
            self.assertEqual(stderr, b"")
            outputs.append((seen, list(self.esplora.requests), list(self.electrum.calls)))
            kinds = session.kinds
        self.assertEqual(outputs[1], outputs[0])
        if path := os.environ.get("KASSIBER_DESKTOP_COUNTS"):
            with open(path, "a", encoding="utf-8") as handle:
                for line in outputs[1][0]:
                    record = json.loads(line)
                    if record["kind"].endswith(".progress") or record.get("request_id") == "stop":
                        continue
                    handle.write(json.dumps(dict(kind=kinds[json.dumps(record["request_id"])], served_by="rust",
                                                 ok=record["kind"] != "error")) + "\n")
        return [json.loads(line) for line in outputs[1][0]]

    def request(self, session, request_id, args, kind=None):
        session.send({"request_id": request_id, "kind": kind or self.KIND, "args": args})
        session.terminal(request_id)

    def test_local_answers_refusals_and_lookups_match_python(self):
        txids = ["a" * 64, "b" * 64, "d" * 64, "0" * 64, "f" * 64]

        def script(session):
            for index, args in enumerate([
                None, [], {"txids": 3}, {"backend": "nope", "txids": txids}, {"backend": "core", "txids": txids},
                {"backend": 7}, {"txids": ["f" * 64], "candidates": [LISTED, "bc1qnope"]},
                {"txids": ["f" * 64], "backend": "esp", "scan_to_index": 0},
                {"txids": txids, "candidates": [LISTED], "backend": "esp"},
                {"txids": txids, "backend": " ESPNOCHAIN "},
                {"txids": ["a" * 64, "0" * 64], "backend": "elx"},
                {"txids": ["a" * 64], "backend": "elxnochain"},
            ]):
                self.request(session, str(index), args)

        records = self.sessions(script)
        terminal = [r for r in records if r["kind"] != "ui.wallets.identify_onchain.progress"]
        self.assertEqual(terminal[8]["data"]["summary"]["owned"], 5, terminal[8])
        self.assertIn("cancelled", terminal[8]["data"])
        self.assertNotIn("cancelled", terminal[7]["data"])
        self.assertEqual(terminal[10]["data"]["results"][0]["classification"], "undetermined")
        self.assertEqual(terminal[11]["kind"], "error")

    def test_cancel_mid_request_and_before_registration_match_python(self):
        held = ["a" * 64, "b" * 64, "d" * 64]

        def script(session):
            session.send({"request_id": "v", "kind": self.KIND, "args": {"txids": held, "backend": "esp"}})
            session.until(lambda r: r.get("request_id") == "v")  # progress 0 of 3
            self.assertTrue(self.esplora.held.wait(30))
            self.request(session, "c", {"target_request_id": "v"}, kind="ui.wallets.identify_onchain.cancel")
            self.esplora.release.set()
            session.terminal("v")
            for index, args in enumerate([None, {}, {"target_request_id": ""}, {"target_request_id": 7},
                                          {"target_request_id": "v"}, {"target_request_id": "later"}]):
                self.request(session, f"x{index}", args, kind="ui.wallets.identify_onchain.cancel")
            session.send({"request_id": "later", "kind": self.KIND, "args": {"txids": held, "backend": "esp"}})
            session.terminal("later")
            session.send({"request_id": 7, "kind": self.KIND, "args": {"txids": ["b" * 64], "backend": "esp"}})
            session.terminal(7)

        records = self.sessions(script, hold="a" * 64)
        reports = [r for r in records if r["kind"] == self.KIND]
        self.assertEqual(reports[0]["data"]["cancelled"], True)
        self.assertEqual(reports[0]["data"]["warnings"][-1],
                         "Stopped after 1 of 3 on-chain lookups; the rest stay unknown.")
        self.assertEqual(reports[1]["data"]["warnings"][-1],
                         "Stopped after 0 of 3 on-chain lookups; the rest stay unknown.")

    def test_no_lookup_before_the_explicit_action_and_offline_refusal(self):
        def script(session):
            self.request(session, "local", {"txids": ["a" * 64, "b" * 64]}, kind="ui.wallets.identify")
            self.assertEqual(self.esplora.requests, [])
            self.assertEqual(self.electrum.calls, [])

        self.sessions(script)
        set_offline_mode(True, self.base / "preferences" / "offline.json")

        def offline(session):
            self.request(session, "esp", {"txids": ["a" * 64], "backend": "esp"})
            self.request(session, "elx", {"txids": ["a" * 64], "backend": "elx"})

        self.sessions(offline)
        self.assertEqual((self.esplora.requests, self.electrum.calls), ([], []))

    def test_sqlcipher_lookup_matches_python(self):
        self.encrypt("synthetic onchain key")
        records = self.sessions(lambda session: self.request(session, "v", {"txids": ["a" * 64], "backend": "esp"}))
        self.assertEqual(records[0]["kind"], "daemon.unlock")
        self.assertEqual(records[-2]["data"]["summary"]["owned"], 1)

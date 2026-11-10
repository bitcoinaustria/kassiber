"""`ui.transactions.graph` with an explicit `allowPublicLookup` against Python.

The lookup asks the user's backends on loopback fakes: an Esplora explorer,
an Electrum server serving real, txid-verified transactions
(docs/reference/rust-native-wallet-gaps.md, S9) and a Bitcoin Core node
answering `getrawtransaction` (rust-native-wallet-gaps.md, C12). Responses and every table
(the graph cache rows included) compare with the unchanged Python daemon.
"""
from contextlib import contextmanager
import json
import socket
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from embit.liquid.transaction import LTransaction, LTransactionInput, LTransactionOutput
from embit.script import Script
from embit.transaction import Transaction, TransactionInput, TransactionOutput

from tests.rust_desktop_parity import RUST_BIN, DesktopParity
from tests.test_rust_report_blockers import ADDR, SCRIPT
from tests.test_rust_transaction_graph import GRAPH, OTHER, PREV, TX1, TX2

PARENT = Transaction(vin=[TransactionInput(bytes.fromhex("aa" * 32), 0)],
                     vout=[TransactionOutput(70000, Script(bytes.fromhex(SCRIPT))),
                           TransactionOutput(1000, Script(bytes.fromhex(OTHER)))])
CHILD = Transaction(vin=[TransactionInput(PARENT.txid(), 0), TransactionInput(PARENT.txid(), 1)],
                    vout=[TransactionOutput(60000, Script(bytes.fromhex(OTHER)))])
POLICY = bytes.fromhex("6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d")[::-1]
LIQUID = LTransaction(vin=[LTransactionInput(bytes.fromhex("bb" * 32), 1)],
                      vout=[LTransactionOutput(POLICY, 1000, Script(bytes.fromhex(SCRIPT))),
                            LTransactionOutput(POLICY, 250, Script(b""))])
# Spends the same parent, so unverified bytes would still make a complete graph.
OTHER_CHILD = Transaction(vin=[TransactionInput(PARENT.txid(), 0)],
                          vout=[TransactionOutput(50000, Script(bytes.fromhex(OTHER)))])
CHILD_ID, PARENT_ID, LIQUID_ID = CHILD.txid().hex(), PARENT.txid().hex(), LIQUID.txid().hex()
# Two distinct parents, in input order: what one parent's failure does to
# the next shows in the graph cache.
SECOND_PARENT = Transaction(vin=[TransactionInput(bytes.fromhex("cc" * 32), 0)],
                            vout=[TransactionOutput(9000, Script(bytes.fromhex(OTHER)))])
TWO_PARENTS = Transaction(vin=[TransactionInput(PARENT.txid(), 0), TransactionInput(SECOND_PARENT.txid(), 0)],
                          vout=[TransactionOutput(75000, Script(bytes.fromhex(OTHER)))])
SECOND_PARENT_ID, TWO_PARENTS_ID = SECOND_PARENT.txid().hex(), TWO_PARENTS.txid().hex()
HEX = {CHILD_ID: CHILD.serialize().hex(), PARENT_ID: PARENT.serialize().hex(),
       LIQUID_ID: LIQUID.serialize().hex()}

ESPLORA_TX2 = {"txid": TX2, "version": 2, "locktime": 0, "size": 222, "vsize": 141, "weight": 561,
               "status": {"confirmed": True, "block_height": 800001},
               "vin": [{"txid": TX1, "vout": 1, "sequence": 1,
                        "prevout": {"scriptpubkey": SCRIPT, "value": 19000, "scriptpubkey_address": ADDR,
                                    "scriptpubkey_type": "v0_p2wpkh"}},
                       {"txid": PREV, "vout": 0, "prevout": {"scriptpubkey": OTHER, "value": 600}}],
               "vout": [{"scriptpubkey": SCRIPT, "value": 18000, "scriptpubkey_address": ADDR},
                        {"scriptpubkey": "6a0474657374", "value": 0, "scriptpubkey_type": "op_return"}]}
ROWS = """
INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,confirmed_at,created_at,kind,raw_json,privacy_boundary) VALUES
 ('e1','w','p','watch','e1','%(child)s','BTC','outbound',60000,0,'2025-05-07T00:00:00Z',NULL,'2025','withdrawal','{}',NULL),
 ('e2','w','p','liquid','e2','%(liquid)s','LBTC','inbound',1000,0,'2025-05-08T00:00:00Z',NULL,'2025','deposit','{}',NULL);
""" % {"child": CHILD_ID, "liquid": LIQUID_ID}


def backend(name, kind, url, chain="bitcoin", network="main", config="{}"):
    return ("INSERT INTO backends(name,kind,chain,network,url,timeout,config_json,created_at,updated_at) "
            f"VALUES('{name}','{kind}','{chain}','{network}','{url}',2,'{config}','2025','2025');\n")


@contextmanager
def esplora(state):
    hits = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            hits.append(self.path)
            status, payload = state.get(self.path, (404, {"error": "missing"}))
            body = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
            self.send_response(status)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=lambda: server.serve_forever(poll_interval=0.01))
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/api", hits
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


@contextmanager
def electrum(transactions):
    """Newline-delimited JSON-RPC: `server.version` and `blockchain.transaction.get`."""
    hits = []
    listener = socket.create_server(("127.0.0.1", 0))
    stop = threading.Event()

    def answer(request):
        method, params = request.get("method"), request.get("params") or []
        hits.append([method, *params])
        if method == "server.version":
            return {"jsonrpc": "2.0", "id": request.get("id"), "result": ["Fake 1.0", "1.4"]}
        if method == "blockchain.transaction.get" and params and params[0] in transactions:
            return {"jsonrpc": "2.0", "id": request.get("id"), "result": transactions[params[0]]}
        return {"jsonrpc": "2.0", "id": request.get("id"), "error": {"code": 2, "message": "missing"}}

    def serve(conn):
        with conn, conn.makefile("rwb") as stream:
            for line in stream:
                request = json.loads(line)
                reply = [answer(item) for item in request] if isinstance(request, list) else answer(request)
                stream.write(json.dumps(reply).encode() + b"\n")
                stream.flush()

    def accept():
        listener.settimeout(0.05)
        while not stop.is_set():
            try:
                conn, _ = listener.accept()
            except OSError:
                continue
            threading.Thread(target=serve, args=(conn,), daemon=True).start()

    thread = threading.Thread(target=accept)
    thread.start()
    try:
        yield f"tcp://127.0.0.1:{listener.getsockname()[1]}", hits
    finally:
        stop.set()
        thread.join()
        listener.close()


def core_verbose(tx, *, inline=None):
    """Core's `getrawtransaction <txid> true` for an embit transaction, BTC
    values spelled with eight decimals as Core writes them."""
    def vout(n, out):
        script = out.script_pubkey.data.hex()
        kind = "witness_v0_keyhash" if script.startswith("0014") else "nonstandard"
        return {"value": "@%.8f@" % (out.value / 1e8), "n": n,
                "scriptPubKey": {"asm": "", "hex": script, "type": kind}}
    vin = []
    for item in tx.vin:
        entry = {"txid": bytes(item.txid).hex(), "vout": item.vout, "scriptSig": {"asm": "", "hex": ""},
                 "sequence": item.sequence}
        if inline and bytes(item.txid).hex() in inline:
            entry["prevout"] = vout(item.vout, inline[bytes(item.txid).hex()].vout[item.vout])
        vin.append(entry)
    serialized = tx.serialize()
    return {"txid": tx.txid().hex(), "hash": tx.txid().hex(), "version": tx.version, "size": len(serialized),
            "vsize": len(serialized), "weight": 4 * len(serialized), "locktime": tx.locktime, "vin": vin,
            "vout": [vout(n, out) for n, out in enumerate(tx.vout)], "hex": serialized.hex()}


@contextmanager
def core(replies):
    """A Bitcoin Core JSON-RPC node: `getrawtransaction` answers from
    `replies` by txid (a dict result, a raw result, or an `(code, message)`
    RPC error); anything else is `-5`."""
    hits = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            params = request.get("params") or []
            hits.append([request.get("method"), *params[:1]])
            reply = replies.get(params[0] if params else None, (-5, "No such mempool or blockchain transaction"))
            if isinstance(reply, tuple):
                body = {"result": None, "error": {"code": reply[0], "message": reply[1]}, "id": request.get("id")}
            else:
                body = {"result": reply, "error": None, "id": request.get("id")}
            text = json.dumps(body)
            # Core writes amounts as numbers with eight decimals.
            text = text.replace('"@', "").replace('@"', "")
            data = text.encode()
            self.send_response(500 if isinstance(reply, tuple) else 200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=lambda: server.serve_forever(poll_interval=0.01))
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/", hits
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


CORE_AUTH = '{"username":"fixture","password":"secret"}'


def lookup_codes(response):
    return [w["code"] for w in response["data"]["warnings"] if "reference" in w["code"]]


def tables(snapshot):
    """The cache rows record when the lookup ran."""
    snapshot = dict(snapshot)
    snapshot["transaction_graph_cache"] = sorted(row[:5] for row in snapshot["transaction_graph_cache"])
    return snapshot


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustTransactionGraphPublicLookupTest(DesktopParity, unittest.TestCase):
    KIND = "ui.transactions.graph"

    def setUp(self):
        super().setUp()
        # The product switch would refuse every lookup; loopback fakes stand
        # in for the user's backends.
        self.env.pop("KASSIBER_NO_EGRESS", None)

    def lookup(self, args):
        return self.compare(args, mutates=True, tables=tables)

    def test_explorer_lookup_matches_python(self):
        state = {f"/api/tx/{TX2}": (200, ESPLORA_TX2)}
        with esplora(state) as (url, hits):
            self.change(GRAPH + backend("loop", "esplora", url)
                        + "INSERT INTO settings(key,value) VALUES('default_backend','loop');")
            first = self.lookup([{"transaction": "g2", "allowPublicLookup": True}])
            self.assertEqual(lookup_codes(first[0]), [])
            self.assertEqual(hits, [f"/api/tx/{TX2}"] * 2)
            # The stored cache answers the next read without a request.
            hits.clear()
            cached = self.lookup([{"transaction": "g2", "allowPublicLookup": True}, {"transaction": "g2"}])
            self.assertEqual(hits, [])
            self.assertEqual(lookup_codes(cached[0]), ["cached_reference_graph"])

    def test_explorer_failures_warn_like_python(self):
        incomplete = dict(ESPLORA_TX2, vin=[{"txid": TX1, "vout": 1}])
        for case, reply in [("missing", (404, {"error": "missing"})), ("invalid", (200, [1, 2])),
                            ("mismatch", (200, dict(ESPLORA_TX2, txid=TX1))),
                            ("no-legs", (200, {"txid": TX2})), ("incomplete", (200, incomplete)),
                            ("body", (200, b"not json"))]:
            with self.subTest(case), esplora({f"/api/tx/{TX2}": reply}) as (url, _):
                self.setUp()
                self.change(GRAPH + backend("loop", "esplora", url)
                            + "INSERT INTO settings(key,value) VALUES('default_backend','loop');")
                responses = self.lookup([{"transaction": "g2", "allowPublicLookup": True}])
                self.assertTrue(any(code.startswith("bitcoin_reference_lookup") for code in lookup_codes(responses[0])))

    def test_electrum_lookup_attaches_parents_like_python(self):
        with electrum(HEX) as (url, hits):
            self.change(GRAPH + ROWS + backend("loopelectrum", "electrum", url)
                        + "UPDATE wallets SET config_json='{\"backend\":\"loopelectrum\",\"chain\":\"bitcoin\",\"network\":\"main\"}' WHERE id='watch';")
            responses = self.lookup([{"transaction": "e1", "allowPublicLookup": True}])
            self.assertIn(["blockchain.transaction.get", PARENT_ID], hits)
            self.assertEqual(lookup_codes(responses[0]), [])

    def test_electrum_missing_parent_and_wrong_bytes_warn_like_python(self):
        for case, served in [("parent", {CHILD_ID: HEX[CHILD_ID]}),
                             ("bytes", {CHILD_ID: OTHER_CHILD.serialize().hex(), PARENT_ID: HEX[PARENT_ID]})]:
            with self.subTest(case), electrum(served) as (url, _):
                self.setUp()
                self.change(GRAPH + ROWS + backend("loopelectrum", "electrum", url)
                            + "INSERT INTO settings(key,value) VALUES('default_backend','loopelectrum');")
                self.lookup([{"transaction": "e1", "allowPublicLookup": True}])

    def test_liquid_electrum_lookup_matches_python(self):
        with electrum(HEX) as (url, _):
            self.change(GRAPH + ROWS + backend("liquidloop", "electrum", url, "liquid", "liquidtestnet")
                        + "INSERT INTO settings(key,value) VALUES('default_backend','liquidloop');")
            responses = self.lookup([{"transaction": "e2", "allowPublicLookup": True}])
            self.assertEqual(lookup_codes(responses[0]), [])

    def test_backend_choice_matches_python(self):
        """Shipped defaults are skipped unless chosen; the wallet's backend
        leads, then the user's others; offline mode fails every lookup."""
        state = {f"/api/tx/{TX2}": (200, ESPLORA_TX2)}
        with esplora(state) as (url, hits):
            for case in ["unconfigured", "shipped", "chosen-shipped", "wallet", "offline"]:
                with self.subTest(case):
                    self.setUp()
                    hits.clear()
                    sql = GRAPH
                    if case == "shipped":
                        sql += backend("mempool", "esplora", url)
                    elif case == "chosen-shipped":
                        sql += backend("mempool", "esplora", url, config='{"infrastructure_owner":"self"}')
                    elif case in {"wallet", "offline"}:
                        sql += backend("dead", "esplora", "http://127.0.0.1:9/api") + backend("mine", "esplora", url)
                        sql += "UPDATE wallets SET config_json='{\"backend\":\"mine\",\"addresses\":[\"%s\"]}' WHERE id='watch';" % ADDR
                    self.change(sql)
                    if case == "offline":
                        (self.base / "preferences").mkdir(exist_ok=True)
                        self.env["KASSIBER_OFFLINE_PREFERENCE_FILE"] = str(self.base / "preferences" / "offline.json")
                        (self.base / "preferences" / "offline.json").write_text('{"schema_version":1,"enabled":true}')
                    self.lookup([{"transaction": "g2", "allowPublicLookup": True}])
                    if case in {"unconfigured", "shipped", "offline"}:
                        self.assertEqual(hits, [])

    def test_core_lookup_attaches_parents_like_python(self):
        replies = {CHILD_ID: core_verbose(CHILD), PARENT_ID: core_verbose(PARENT)}
        with core(replies) as (url, hits):
            self.change(GRAPH + ROWS + backend("loopcore", "bitcoinrpc", url, config=CORE_AUTH)
                        + "UPDATE wallets SET config_json='{\"backend\":\"loopcore\",\"chain\":\"bitcoin\",\"network\":\"main\"}' WHERE id='watch';")
            responses = self.lookup([{"transaction": "e1", "allowPublicLookup": True}])
            self.assertIn(["getrawtransaction", PARENT_ID], hits)
            self.assertEqual(lookup_codes(responses[0]), [])
            # The parent and the graph were cached: no request the next time.
            hits.clear()
            cached = self.lookup([{"transaction": "e1", "allowPublicLookup": True}])
            self.assertEqual(hits, [])
            self.assertEqual(lookup_codes(cached[0]), ["cached_reference_graph"])

    def test_core_inline_prevouts_need_no_parent_like_python(self):
        replies = {CHILD_ID: core_verbose(CHILD, inline={PARENT_ID: PARENT})}
        with core(replies) as (url, hits):
            self.change(GRAPH + ROWS + backend("loopcore", "bitcoinrpc", url, config=CORE_AUTH)
                        + "INSERT INTO settings(key,value) VALUES('default_backend','loopcore');")
            responses = self.lookup([{"transaction": "e1", "allowPublicLookup": True}])
            self.assertEqual(hits, [["getrawtransaction", CHILD_ID]] * 2)
            self.assertEqual(lookup_codes(responses[0]), [])

    def test_core_failures_warn_like_python(self):
        """A missing parent (-5) leaves the graph incomplete; any other parent
        failure stops asking; a missing or malformed transaction fails the
        backend; each warns as Python does."""
        child = core_verbose(CHILD)
        complete = {CHILD_ID: child, PARENT_ID: core_verbose(PARENT)}
        for case, replies, config, code in [
            ("missing-parent", {CHILD_ID: child}, CORE_AUTH, "bitcoin_reference_lookup_incomplete"),
            ("parent-error", {CHILD_ID: child, PARENT_ID: (-32603, "internal")}, CORE_AUTH,
             "bitcoin_reference_lookup_incomplete"),
            ("missing", {}, CORE_AUTH, "bitcoin_reference_lookup_failed_bitcoinrpc"),
            ("malformed", {CHILD_ID: "not a transaction"}, CORE_AUTH,
             "bitcoin_reference_lookup_failed_bitcoinrpc"),
            ("no-auth", complete, "{}", "bitcoin_reference_lookup_failed_bitcoinrpc"),
        ]:
            with self.subTest(case), core(replies) as (url, _):
                self.setUp()
                self.change(GRAPH + ROWS + backend("loopcore", "bitcoinrpc", url, config=config)
                            + "INSERT INTO settings(key,value) VALUES('default_backend','loopcore');")
                responses = self.lookup([{"transaction": "e1", "allowPublicLookup": True}])
                self.assertIn(code, lookup_codes(responses[0]))

    def test_core_parent_failures_continue_or_stop_like_python(self):
        """The first parent missing (-5) still lets the second be fetched and
        cached; any other failure stops before it."""
        rows = ("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,"
                "amount,fee,occurred_at,confirmed_at,created_at,kind,raw_json,privacy_boundary) VALUES"
                f"('e3','w','p','watch','e3','{TWO_PARENTS_ID}','BTC','outbound',75000,0,'2025-05-09T00:00:00Z',"
                "NULL,'2025','withdrawal','{}',NULL);")
        for case, first in [("missing", (-5, "No such mempool or blockchain transaction")),
                            ("error", (-32603, "internal"))]:
            replies = {TWO_PARENTS_ID: core_verbose(TWO_PARENTS), PARENT_ID: first,
                       SECOND_PARENT_ID: core_verbose(SECOND_PARENT)}
            with self.subTest(case), core(replies) as (url, hits):
                self.setUp()
                self.change(GRAPH + rows + backend("loopcore", "bitcoinrpc", url, config=CORE_AUTH)
                            + "INSERT INTO settings(key,value) VALUES('default_backend','loopcore');")
                responses = self.lookup([{"transaction": "e3", "allowPublicLookup": True}])
                self.assertIn("bitcoin_reference_lookup_incomplete", lookup_codes(responses[0]))
                asked = ["getrawtransaction", SECOND_PARENT_ID] in hits
                self.assertEqual(asked, case == "missing")

"""A loopback Bitcoin Core JSON-RPC fake over `tests.rust_sync_chain.Chain`.

It keeps the watch-only wallets a sync creates, the descriptors and
addresses imported into each, and answers the wallet and chain RPCs
Kassiber's Core adapter reads from the chain model: transaction details,
verbose transactions (foreign parents answer Core's -5), unspent outputs and
block headers. Wallet semantics are simplified (a transaction spending a
wallet input lists its foreign outputs as sends and keeps wallet outputs as
change); both implementations read the same answers. No external network.
"""
from __future__ import annotations

from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading

from embit.descriptor import Descriptor
from embit.networks import NETWORKS
from embit.script import Script

COIN = 100_000_000


def _btc(sats: int) -> float:
    return sats / COIN


class Node:
    def __init__(self, chain, network="main"):
        self.chain = chain
        self.network = NETWORKS[network]
        self.wallets: dict[str, dict] = {}
        self.loaded: set[str] = set()
        self.calls: list[tuple[str, str, list]] = []
        self.pruned = False

    # -- wallet state ---------------------------------------------------------
    def watched(self, wallet):
        return self.wallets.get(wallet, {}).get("scripts", set())

    def address(self, script_hex):
        try:
            return Script(bytes.fromhex(script_hex)).address(self.network)
        except Exception:
            return None

    def import_descriptor(self, wallet, request):
        text = request["desc"].split("#", 1)[0]
        scripts = self.wallets[wallet]["scripts"]
        if text.startswith("addr("):
            address = text[5:-1]
            scripts.add(Script.from_address(address).data.hex())
            return {"success": True}
        descriptor = Descriptor.from_string(text)
        start, end = request.get("range", [0, 0])
        for index in range(start, end + 1):
            scripts.add(descriptor.derive(index).script_pubkey().data.hex())
        return {"success": True}

    # -- chain views ------------------------------------------------------------
    def tip_hash(self):
        return self.chain.block_hash(self.chain.tip)

    def confirmations(self, txid):
        height = self.chain.heights.get(txid)
        if height is None or height > self.chain.tip:
            return 0
        return self.chain.tip - height + 1

    def position(self, txid):
        height = self.chain.heights.get(txid)
        if height is None or height > self.chain.tip:
            return {"confirmations": 0, "time": self.chain.base_time}
        return {"confirmations": self.confirmations(txid), "blockhash": self.chain.block_hash(height),
                "blockheight": height, "blocktime": self.chain.block_time(height),
                "time": self.chain.block_time(height)}

    def prev(self, item):
        return self.chain.output(item.txid.hex(), item.vout)

    def owned_input(self, tx, scripts):
        return any((p := self.prev(i)) is not None and p.script_pubkey.data.hex() in scripts for i in tx.vin)

    def details(self, wallet, txid):
        scripts = self.watched(wallet)
        tx = self.chain.txs[txid]
        position = self.position(txid)
        spends = self.owned_input(tx, scripts)
        rows = []
        if spends:
            total_in = sum(p.value for i in tx.vin if (p := self.prev(i)) is not None)
            fee = total_in - sum(out.value for out in tx.vout)
            for index, out in enumerate(tx.vout):
                script = out.script_pubkey.data.hex()
                if script in scripts:
                    continue
                rows.append({"address": self.address(script), "category": "send", "amount": -_btc(out.value),
                             "vout": index, "fee": -_btc(fee), "abandoned": False, "txid": txid, **position})
        else:
            for index, out in enumerate(tx.vout):
                script = out.script_pubkey.data.hex()
                if script in scripts:
                    rows.append({"address": self.address(script), "category": "receive",
                                 "amount": _btc(out.value), "vout": index, "txid": txid, **position})
        return rows

    def related(self, wallet):
        scripts = self.watched(wallet)
        result = []
        for txid in self.chain.visible():
            tx = self.chain.txs[txid]
            if self.owned_input(tx, scripts) or any(o.script_pubkey.data.hex() in scripts for o in tx.vout):
                result.append(txid)
        return result

    def decoded(self, txid):
        tx = self.chain.txs[txid]
        vin = []
        for item in tx.vin:
            if item.txid == b"\x00" * 32:
                vin.append({"coinbase": item.script_sig.data.hex(), "sequence": item.sequence})
                continue
            entry = {"txid": item.txid.hex(), "vout": item.vout, "sequence": item.sequence}
            if item.witness.items:
                entry["txinwitness"] = [w.hex() for w in item.witness.items]
            vin.append(entry)
        vout = [{"n": index, "value": _btc(out.value), "scriptPubKey": {"hex": out.script_pubkey.data.hex()}}
                for index, out in enumerate(tx.vout)]
        return {"txid": txid, "version": tx.version, "locktime": tx.locktime, "vin": vin, "vout": vout}

    # -- RPC ------------------------------------------------------------------------
    def rpc(self, wallet, method, params):
        self.calls.append((wallet, method, params))
        if method == "listwallets":
            return sorted(self.loaded)
        if method == "loadwallet":
            if params[0] not in self.wallets:
                raise RpcError(-18, "Wallet file not found")
            self.loaded.add(params[0])
            return {"name": params[0]}
        if method == "createwallet":
            assert params[1] is True, params
            self.wallets[params[0]] = {"scripts": set()}
            self.loaded.add(params[0])
            return {"name": params[0]}
        if method == "getblockchaininfo":
            return {"chain": "main", "blocks": self.chain.tip, "pruned": self.pruned}
        if method == "getdescriptorinfo":
            return {"descriptor": f"{params[0]}#fixture0", "isrange": "*" in params[0]}
        if method == "getbestblockhash":
            return self.tip_hash()
        if method == "getblockheader":
            for height in range(self.chain.tip + 1):
                if self.chain.block_hash(height) == params[0]:
                    return {"hash": params[0], "height": height, "time": self.chain.block_time(height)}
            raise RpcError(-5, "Block not found")
        if method == "getblockhash":
            return self.chain.block_hash(params[0])
        if method == "getrawtransaction":
            # Any visible transaction, as a node with `txindex` answers it.
            txid = params[0]
            if txid not in self.chain.visible():
                raise RpcError(-5, "No such mempool or blockchain transaction")
            raw = self.chain.txs[txid].serialize()
            return {**self.decoded(txid), "hex": raw.hex(), "size": len(raw)}
        if wallet not in self.loaded:
            raise RpcError(-18, "Requested wallet does not exist or is not loaded")
        if method == "importdescriptors":
            assert not any("prv" in r["desc"] for r in params[0])
            return [self.import_descriptor(wallet, request) for request in params[0]]
        if method == "importmulti":
            for request in params[0]:
                address = request["scriptPubKey"]["address"]
                self.wallets[wallet]["scripts"].add(Script.from_address(address).data.hex())
            return [{"success": True} for _ in params[0]]
        if method == "getaddressinfo":
            script = Script.from_address(params[0]).data.hex()
            return {"address": params[0], "iswatchonly": script in self.watched(wallet), "ismine": False}
        if method == "listtransactions":
            rows = [row for txid in self.related(wallet) for row in self.details(wallet, txid)]
            skip, count = params[2], params[1]
            return rows[skip:skip + count]
        if method == "listsinceblock":
            since = None
            for height in range(self.chain.tip + 1):
                if self.chain.block_hash(height) == params[0]:
                    since = height
            if since is None:
                raise RpcError(-5, "Block not found")
            rows = [row for txid in self.related(wallet)
                    for row in self.details(wallet, txid)
                    if row["confirmations"] == 0 or row["blockheight"] > since]
            return {"transactions": rows, "removed": [], "lastblock": self.tip_hash()}
        if method == "gettransaction":
            txid = params[0]
            if txid not in self.related(wallet):
                raise RpcError(-5, "Invalid or non-wallet transaction id")
            result = {"txid": txid, **self.position(txid), "details": self.details(wallet, txid),
                      "hex": self.chain.txs[txid].serialize().hex()}
            if len(params) > 2 and params[2]:
                result["decoded"] = self.decoded(txid)
            return result
        if method == "listunspent":
            addresses = set(params[2])
            spent = {(i.txid.hex(), i.vout) for txid in self.chain.visible() for i in self.chain.txs[txid].vin}
            rows = []
            for txid in self.related(wallet):
                for index, out in enumerate(self.chain.txs[txid].vout):
                    script = out.script_pubkey.data.hex()
                    address = self.address(script)
                    if script in self.watched(wallet) and address in addresses and (txid, index) not in spent:
                        rows.append({"txid": txid, "vout": index, "address": address, "scriptPubKey": script,
                                     "amount": _btc(out.value), "confirmations": self.confirmations(txid),
                                     "spendable": False, "solvable": True, "safe": True})
            return rows
        raise AssertionError(method)


class RpcError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


@contextmanager
def serve(node):
    """Serve `node` over JSON-RPC 1.0 on loopback; yields the URL."""

    class Http(BaseHTTPRequestHandler):
        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            wallet = self.path.split("/wallet/", 1)[1] if "/wallet/" in self.path else ""
            with node.chain.lock:
                try:
                    reply = {"result": node.rpc(wallet, body["method"], body.get("params") or []),
                             "error": None, "id": body["id"]}
                    status = 200
                except RpcError as error:
                    reply = {"result": None, "error": {"code": error.code, "message": error.message},
                             "id": body["id"]}
                    status = 500
            data = json.dumps(reply).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *_):
            pass

    class Server(ThreadingHTTPServer):
        daemon_threads = True
        allow_reuse_address = True

    server = Server(("127.0.0.1", 0), Http)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_address[1]}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join()

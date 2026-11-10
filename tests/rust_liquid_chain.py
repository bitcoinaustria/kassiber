"""Loopback Liquid Electrum and Esplora fakes over the LWK vector phases.

Each phase (`crates/wallets/tests/vectors/lwk_chain.json`) lists the block
headers, every raw transaction (the wallet's and their foreign parents) and
the Electrum history of each owned script. Both fakes derive everything else
from those: statuses, unspent outputs, Esplora transaction JSON and script
statistics. No external network is involved.
"""
from __future__ import annotations

from contextlib import contextmanager
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import socketserver
import threading

from embit.liquid.transaction import LTransaction


def _scripthash(script: bytes) -> str:
    return hashlib.sha256(script).digest()[::-1].hex()


class View:
    """One phase as both protocols see it."""

    def __init__(self, phase):
        self.phase = phase
        self.heights = {}
        for entries in phase["history"].values():
            for entry in entries:
                self.heights[entry["tx_hash"]] = entry["height"]
        self.decoded = {txid: LTransaction.from_string(raw) for txid, raw in phase["transactions"].items()}

    def header(self, height):
        return self.phase["headers"][str(height)]

    def block_hash(self, height):
        return hashlib.sha256(bytes.fromhex(self.header(height))).hexdigest()

    def block_time(self, height):
        return int.from_bytes(bytes.fromhex(self.header(height))[68:72], "little")

    def status(self, txid):
        height = self.heights.get(txid, 0)
        if not height:
            return {"confirmed": False}
        return {"confirmed": True, "block_height": height, "block_hash": self.block_hash(height),
                "block_time": self.block_time(height)}

    def history(self, scripthash):
        return self.phase["history"].get(scripthash, [])

    def spent(self):
        return {(bytes(vin.txid).hex(), vin.vout)
                for txid in self.heights for vin in self.decoded[txid].vin}

    def unspent(self, scripthash):
        spent = self.spent()
        rows = []
        for entry in self.history(scripthash):
            txid = entry["tx_hash"]
            for index, out in enumerate(self.decoded[txid].vout):
                if _scripthash(out.script_pubkey.data) == scripthash and (txid, index) not in spent:
                    rows.append((txid, index, entry["height"]))
        return rows

    def esplora_tx(self, txid):
        tx = self.decoded[txid]
        vin = []
        for item in tx.vin:
            parent = bytes(item.txid).hex()
            entry = {"txid": parent, "vout": item.vout,
                     "witness": [bytes(w).hex() for w in item.witness.script_witness.items]}
            if parent in self.decoded and item.vout < len(self.decoded[parent].vout):
                entry["prevout"] = {"scriptpubkey": self.decoded[parent].vout[item.vout].script_pubkey.data.hex()}
            vin.append(entry)
        vout = [{"scriptpubkey": out.script_pubkey.data.hex()} for out in tx.vout]
        return {"txid": txid, "version": tx.version, "locktime": tx.locktime, "vin": vin, "vout": vout,
                "status": self.status(txid)}


@contextmanager
def electrum(state):
    """Serve `state["phase"]` over the Electrum protocol."""

    class Handler(socketserver.StreamRequestHandler):
        def handle(self):
            while line := self.rfile.readline():
                request = json.loads(line)
                batch = isinstance(request, list)
                replies = []
                for item in request if batch else [request]:
                    view = View(state["phase"])
                    method, params = item["method"], item.get("params", [])
                    if method == "server.version":
                        result = ["fixture", "1.4"]
                    elif method == "blockchain.headers.subscribe":
                        tip = view.phase["tip"]
                        result = {"height": tip, "hex": view.header(tip)}
                    elif method == "blockchain.block.header":
                        result = view.header(params[0])
                    elif method == "blockchain.scripthash.subscribe":
                        history = view.history(params[0])
                        result = hashlib.sha256(json.dumps(history).encode()).hexdigest() if history else None
                    elif method == "blockchain.scripthash.get_history":
                        result = view.history(params[0])
                    elif method == "blockchain.scripthash.listunspent":
                        result = [{"tx_hash": t, "tx_pos": n, "height": h} for t, n, h in view.unspent(params[0])]
                    elif method == "blockchain.transaction.get":
                        result = view.phase["transactions"][params[0]]
                    else:
                        raise AssertionError(method)
                    replies.append({"jsonrpc": "2.0", "id": item["id"], "result": result})
                self.wfile.write(json.dumps(replies if batch else replies[0]).encode() + b"\n")

    class Server(socketserver.ThreadingTCPServer):
        daemon_threads = True
        allow_reuse_address = True

    with Server(("127.0.0.1", 0), Handler) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield f"tcp://127.0.0.1:{server.server_address[1]}"
        finally:
            server.shutdown()
            thread.join()


@contextmanager
def esplora(state):
    """Serve `state["phase"]` over the Esplora REST subset Kassiber reads."""

    class Http(BaseHTTPRequestHandler):
        def do_GET(self):
            view = View(state["phase"])
            parts = self.path.strip("/").split("/")
            tip = view.phase["tip"]
            result = None
            if parts == ["blocks", "tip", "height"]:
                result = tip
            elif parts == ["blocks", "tip", "hash"]:
                result = view.block_hash(tip)
            elif parts[0] == "block-height" and len(parts) == 2:
                result = view.block_hash(int(parts[1]))
            elif parts[0] == "tx" and len(parts) == 3 and parts[2] == "hex":
                result = view.phase["transactions"].get(parts[1])
            elif parts[0] == "scripthash":
                history = view.history(parts[1])
                confirmed = sorted((e for e in history if e["height"]), key=lambda e: (-e["height"], e["tx_hash"]))
                mempool = [e for e in history if not e["height"]]
                if len(parts) == 2:
                    def side(count):
                        return {"funded_txo_count": 0, "funded_txo_sum": 0, "spent_txo_count": 0,
                                "spent_txo_sum": 0, "tx_count": count}
                    result = {"chain_stats": side(len(confirmed)), "mempool_stats": side(len(mempool))}
                elif parts[2:] == ["txs", "chain"]:
                    result = [view.esplora_tx(e["tx_hash"]) for e in confirmed[:25]]
                elif parts[2:4] == ["txs", "chain"] and len(parts) == 5:
                    ids = [e["tx_hash"] for e in confirmed]
                    start = ids.index(parts[4]) + 1 if parts[4] in ids else len(ids)
                    result = [view.esplora_tx(t) for t in ids[start:start + 25]]
                elif parts[2:] == ["txs", "mempool"]:
                    result = [view.esplora_tx(e["tx_hash"]) for e in mempool]
                elif parts[2:] == ["utxo"]:
                    result = [{"txid": t, "vout": n, "status": view.status(t)} for t, n, _ in view.unspent(parts[1])]
            if result is None:
                status, body = 404, b"not found"
            elif isinstance(result, (int, str)) and parts[0] in {"blocks", "block-height", "tx"}:
                status, body = 200, str(result).encode()
            else:
                status, body = 200, json.dumps(result).encode()
            self.send_response(status)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

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

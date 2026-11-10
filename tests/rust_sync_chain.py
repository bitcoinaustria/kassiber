"""Deterministic loopback Bitcoin chain for full wallet-sync parity tests.

One in-memory chain model answers both the Esplora REST subset and the
Electrum JSON-RPC subset that Kassiber's Python reference and Rust port use.
Each wallet or funding transaction is mined alone in its block, so a proof is
the transaction id itself. Rounds mutate the model between syncs (reorgs,
drops, replacements, mempool changes). No external network is involved.
"""
from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass, field
import errno
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import select
import socket
import socketserver
import ssl
import threading
import time

from embit.script import Script, Witness
from embit.transaction import Transaction, TransactionInput, TransactionOutput

# Public synthetic loopback TLS material (crates/net/tests/fixtures/README.md):
# the leaf names `localhost` and `hidden.onion`, signed by `ca.pem`.
TLS_FIXTURES = Path(__file__).resolve().parents[1] / "crates" / "net" / "tests" / "fixtures"
TEST_CA = TLS_FIXTURES / "ca.pem"

GENESIS = ("01000000" + "00" * 32 + "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a"
           + "29ab5f49ffff001d1dac2b7c")
FOREIGN = "0014" + "33" * 20


def _dsha(data: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def scripthash(script_hex: str) -> str:
    return hashlib.sha256(bytes.fromhex(script_hex)).digest()[::-1].hex()


@dataclass
class Chain:
    tip: int = 20
    next_parent: int = 1
    epoch: int = 0
    fork: int = 0
    base_time: int = 1_700_000_000
    txs: dict[str, Transaction] = field(default_factory=dict)
    heights: dict[str, int] = field(default_factory=dict)
    mempool: list[str] = field(default_factory=list)
    race_on_header: bool = False
    hits: list = field(default_factory=list)
    counter: int = 0
    lock: threading.RLock = field(default_factory=threading.RLock)

    # -- model -------------------------------------------------------------
    def _salt(self) -> bytes:
        self.counter += 1
        return self.counter.to_bytes(4, "little")

    def _next_height(self) -> int:
        height = self.next_parent
        self.next_parent += 1
        return height

    def fund(self, script_hex: str, value: int, *, height: int | None = None) -> tuple[str, int]:
        """A foreign transaction paying `value` sats to `script_hex`; returns its outpoint.

        Its own input spends a foreign coinbase, so every previous output a
        client may fetch exists on this chain.
        """
        grandparent = self.coinbase(FOREIGN, value + 1_000, height=self._next_height())
        parent = Transaction(vin=[TransactionInput(bytes.fromhex(grandparent), 0, sequence=0xFFFFFFFE,
                                                   witness=Witness([b"\xab" * 71, b"\x02" + b"\xcd" * 32]))],
                             vout=[TransactionOutput(value, Script(bytes.fromhex(script_hex)))])
        txid = self.add(parent, height=height if height is not None else self._next_height())
        return txid, 0

    def add(self, tx: Transaction, *, height: int | None = None) -> str:
        txid = tx.txid().hex()
        self.txs[txid] = tx
        if height is None:
            self.mempool.append(txid)
        else:
            self.heights[txid] = height
        return txid

    def spend(self, outpoints, outputs, *, height=None, sequence=0xFFFFFFFD, locktime=0) -> str:
        vin = [TransactionInput(bytes.fromhex(txid), vout, sequence=sequence,
                                witness=Witness([b"\xab" * 71, b"\x02" + b"\xcd" * 32]))
               for txid, vout in outpoints]
        vout = [TransactionOutput(value, Script(bytes.fromhex(script))) for script, value in outputs]
        return self.add(Transaction(vin=vin, vout=vout, locktime=locktime), height=height)

    def coinbase(self, script_hex: str, value: int, *, height: int) -> str:
        tx = Transaction(vin=[TransactionInput(b"\x00" * 32, 0xFFFFFFFF, Script(height.to_bytes(2, "little") + self._salt()),
                                               sequence=0xFFFFFFFF)],
                         vout=[TransactionOutput(value, Script(bytes.fromhex(script_hex)))])
        return self.add(tx, height=height)

    def confirm(self, txid: str, height: int) -> None:
        if txid in self.mempool:
            self.mempool.remove(txid)
        self.heights[txid] = height

    def unconfirm(self, txid: str) -> None:
        self.heights.pop(txid, None)
        if txid not in self.mempool:
            self.mempool.append(txid)

    def drop(self, txid: str) -> None:
        self.heights.pop(txid, None)
        if txid in self.mempool:
            self.mempool.remove(txid)

    def reorg(self, fork: int) -> None:
        """Replace every block at or above `fork` with a new branch."""
        self.epoch += 1
        self.fork = fork

    # -- headers -------------------------------------------------------------
    def block_txids(self, height: int) -> list[str]:
        return sorted(txid for txid, h in self.heights.items() if h == height)

    def block_time(self, height: int) -> int:
        bump = self.epoch * 100 if height >= self.fork and self.epoch else 0
        return self.base_time + height * 600 + bump

    def header(self, height: int) -> str:
        if height == 0:
            return GENESIS
        txids = self.block_txids(height)
        assert len(txids) <= 1, "one transaction per block keeps merkle proofs trivial"
        root = bytes.fromhex(txids[0])[::-1] if txids else b"\x00" * 32
        nonce = self.epoch if height >= self.fork else 0
        raw = (b"\x01\x00\x00\x00" + bytes.fromhex(self.block_hash(height - 1))[::-1] + root
               + self.block_time(height).to_bytes(4, "little") + b"\xff\xff\x7f\x20" + nonce.to_bytes(4, "little"))
        return raw.hex()

    def block_hash(self, height: int) -> str:
        return _dsha(bytes.fromhex(self.header(height)))[::-1].hex()

    # -- views ------------------------------------------------------------------
    def visible(self) -> list[str]:
        confirmed = sorted((h, txid) for txid, h in self.heights.items() if h <= self.tip)
        return [txid for _, txid in confirmed] + list(self.mempool)

    def output(self, txid: str, vout: int):
        tx = self.txs.get(txid)
        return tx.vout[vout] if tx is not None and vout < len(tx.vout) else None

    def status(self, txid: str) -> dict:
        height = self.heights.get(txid)
        if height is None or height > self.tip:
            return {"confirmed": False}
        return {"confirmed": True, "block_height": height, "block_hash": self.block_hash(height),
                "block_time": self.block_time(height)}

    def esplora_tx(self, txid: str) -> dict:
        tx = self.txs[txid]
        vin, total_in = [], 0
        for item in tx.vin:
            coinbase = item.txid == b"\x00" * 32
            entry = {"txid": item.txid.hex(), "vout": item.vout, "scriptsig": item.script_sig.data.hex(),
                     "witness": [w.hex() for w in item.witness.items], "is_coinbase": coinbase,
                     "sequence": item.sequence}
            prevout = None if coinbase else self.output(item.txid.hex(), item.vout)
            if prevout is not None:
                entry["prevout"] = {"scriptpubkey": prevout.script_pubkey.data.hex(), "value": prevout.value}
                total_in += prevout.value
            vin.append(entry)
        vout = [{"scriptpubkey": out.script_pubkey.data.hex(), "value": out.value} for out in tx.vout]
        fee = 0 if vin and vin[0]["is_coinbase"] else total_in - sum(out.value for out in tx.vout)
        return {"txid": txid, "version": tx.version, "locktime": tx.locktime, "vin": vin, "vout": vout,
                "size": len(tx.serialize()), "weight": 4 * len(tx.serialize()), "fee": fee,
                "status": self.status(txid)}

    def touching(self, script_hex: str) -> list[str]:
        result = []
        for txid in self.visible():
            tx = self.txs[txid]
            outputs = any(out.script_pubkey.data.hex() == script_hex for out in tx.vout)
            inputs = any((prev := self.output(item.txid.hex(), item.vout)) is not None
                         and prev.script_pubkey.data.hex() == script_hex for item in tx.vin)
            if outputs or inputs:
                result.append(txid)
        return result

    def unspent(self, script_hex: str) -> list[tuple[str, int, int]]:
        spent = {(item.txid.hex(), item.vout) for txid in self.visible() for item in self.txs[txid].vin}
        result = []
        for txid in self.visible():
            for index, out in enumerate(self.txs[txid].vout):
                if out.script_pubkey.data.hex() == script_hex and (txid, index) not in spent:
                    result.append((txid, index, out.value))
        return result

    def by_scripthash(self, scripts: dict[str, str], hashed: str) -> str | None:
        return scripts.get(hashed)

    def electrum_history(self, script_hex: str) -> list[dict]:
        rows = []
        for txid in self.touching(script_hex):
            status = self.status(txid)
            rows.append({"tx_hash": txid, "height": status["block_height"] if status["confirmed"] else 0})
        return rows

    def electrum_status(self, script_hex: str):
        rows = self.electrum_history(script_hex)
        if not rows:
            return None
        return hashlib.sha256("".join(f"{r['tx_hash']}:{r['height']}:" for r in rows).encode()).hexdigest()

    def stats(self, script_hex: str) -> dict:
        def side(txids):
            funded = spent = funded_sum = spent_sum = 0
            for txid in txids:
                tx = self.txs[txid]
                for out in tx.vout:
                    if out.script_pubkey.data.hex() == script_hex:
                        funded += 1
                        funded_sum += out.value
                for item in tx.vin:
                    prev = self.output(item.txid.hex(), item.vout)
                    if prev is not None and prev.script_pubkey.data.hex() == script_hex:
                        spent += 1
                        spent_sum += prev.value
            return {"funded_txo_count": funded, "funded_txo_sum": funded_sum, "spent_txo_count": spent,
                    "spent_txo_sum": spent_sum, "tx_count": len(txids)}

        txids = self.touching(script_hex)
        confirmed = [t for t in txids if self.status(t)["confirmed"]]
        return {"chain_stats": side(confirmed), "mempool_stats": side([t for t in txids if t not in confirmed])}


@contextmanager
def serve(chain: Chain, protocol: str, port: int = 0, *, tls: bool = False):
    """Serve `chain` over loopback; yields the backend URL.

    Oracle generators pass a fixed `port` because Esplora checkpoints commit
    to a digest of the endpoint. `tls` serves Electrum over TLS as
    `localhost`, with the test CA's leaf.
    """

    def script_index():
        scripts = {}
        for tx in chain.txs.values():
            for out in tx.vout:
                script = out.script_pubkey.data.hex()
                scripts[scripthash(script)] = script
        return scripts

    class Http(BaseHTTPRequestHandler):
        def do_GET(self):
            with chain.lock:
                chain.hits.append(self.path)
                parts = self.path.strip("/").split("/")
                if parts[0] == "api":
                    parts = parts[1:]
                status, result = 200, None
                if parts == ["blocks", "tip", "height"]:
                    result = chain.tip
                elif parts == ["blocks", "tip", "hash"]:
                    result = chain.block_hash(chain.tip)
                elif parts[0] == "block-height" and len(parts) == 2:
                    result = chain.block_hash(int(parts[1]))
                elif parts[0] == "tx" and len(parts) == 2 and parts[1] in chain.txs:
                    result = chain.esplora_tx(parts[1])
                elif parts[0] == "tx" and len(parts) == 3 and parts[2] == "hex" and parts[1] in chain.txs:
                    result = chain.txs[parts[1]].serialize().hex()
                elif parts[0] == "scripthash":
                    script = script_index().get(parts[1])
                    txids = chain.touching(script) if script else []
                    confirmed = [t for t in reversed(txids) if chain.status(t)["confirmed"]]
                    if len(parts) == 2:
                        result = chain.stats(script) if script else {
                            "chain_stats": {"funded_txo_count": 0, "funded_txo_sum": 0, "spent_txo_count": 0,
                                            "spent_txo_sum": 0, "tx_count": 0},
                            "mempool_stats": {"funded_txo_count": 0, "funded_txo_sum": 0, "spent_txo_count": 0,
                                              "spent_txo_sum": 0, "tx_count": 0}}
                    elif parts[2:] == ["txs", "chain"]:
                        result = [chain.esplora_tx(t) for t in confirmed[:25]]
                    elif parts[2:3] == ["txs"] and parts[3:4] == ["chain"] and len(parts) == 5:
                        index = confirmed.index(parts[4]) + 1 if parts[4] in confirmed else len(confirmed)
                        result = [chain.esplora_tx(t) for t in confirmed[index:index + 25]]
                    elif parts[2:] == ["txs", "mempool"]:
                        result = [chain.esplora_tx(t) for t in txids if not chain.status(t)["confirmed"]]
                    elif parts[2:] == ["utxo"]:
                        result = [{"txid": t, "vout": n, "value": v, "status": chain.status(t)}
                                  for t, n, v in (chain.unspent(script) if script else [])]
                if result is None:
                    status, body = 404, b"not found"
                elif isinstance(result, (int, str)) and parts[0] in {"blocks", "block-height", "tx"}:
                    body = str(result).encode()
                else:
                    body = json.dumps(result).encode()
            self.send_response(status)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    context = None
    if tls:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(TLS_FIXTURES / "server.pem", TLS_FIXTURES / "server-key.pem")

    class Electrum(socketserver.StreamRequestHandler):
        def setup(self):
            if context is not None:
                self.request = context.wrap_socket(self.request, server_side=True)
            super().setup()

        def handle(self):
            self.request.settimeout(10)
            while line := self.rfile.readline():
                payload = json.loads(line)
                batch = isinstance(payload, list)
                requests = payload if batch else [payload]
                responses = [self.answer(request) for request in requests]
                self.wfile.write(json.dumps(responses if batch else responses[0]).encode() + b"\n")

        def answer(self, request):
            with chain.lock:
                chain.hits.append(request)
                method, params = request["method"], request.get("params", [])
                scripts = script_index()
                if method == "server.version":
                    result = ["fixture", "1.6"]
                elif method == "server.ping":
                    result = None
                elif method == "blockchain.headers.subscribe":
                    result = {"height": chain.tip, "hex": chain.header(chain.tip)}
                elif method == "blockchain.block.header":
                    result = chain.header(params[0])
                    if chain.race_on_header:
                        chain.race_on_header = False
                        chain.reorg(max(1, chain.fork))
                elif method == "blockchain.block.headers":
                    start, count = params[:2]
                    end = min(chain.tip + 1, start + count)
                    headers = [chain.header(h) for h in range(start, end)]
                    result = {"count": end - start, "max": 2016, "headers": headers, "hex": "".join(headers)}
                elif method == "blockchain.transaction.get":
                    assert len(params) == 1 or params[1] is False, params
                    result = chain.txs[params[0]].serialize().hex()
                elif method == "blockchain.transaction.get_merkle":
                    status = chain.status(params[0])
                    result = {"pos": 0, "merkle": [], "block_height": status.get("block_height", params[1] if len(params) > 1 else 0)}
                elif method in {"blockchain.scripthash.subscribe", "blockchain.scripthash.get_history",
                                "blockchain.scripthash.listunspent"}:
                    script = scripts.get(params[0])
                    if method.endswith("subscribe"):
                        result = chain.electrum_status(script) if script else None
                    elif method.endswith("get_history"):
                        result = chain.electrum_history(script) if script else []
                    else:
                        result = [{"tx_hash": t, "tx_pos": n, "value": v,
                                   "height": chain.status(t).get("block_height", 0)}
                                  for t, n, v in (chain.unspent(script) if script else [])]
                else:
                    raise AssertionError(method)
                return {"jsonrpc": "2.0", "id": request.get("id"), "result": result}

    class Tcp(socketserver.ThreadingTCPServer):
        daemon_threads = True
        allow_reuse_address = True

    class Http11(ThreadingHTTPServer):
        daemon_threads = True
        allow_reuse_address = True

    # A fixed port is part of the generated vectors, so a concurrent session on
    # this machine that serves the same fixture holds it for a while: wait for
    # it instead of failing. An ephemeral port never collides.
    deadline = time.monotonic() + 600
    while True:
        try:
            server = (Http11 if protocol == "esplora" else Tcp)(
                ("127.0.0.1", port), Http if protocol == "esplora" else Electrum)
            break
        except OSError as error:
            if not port or error.errno != errno.EADDRINUSE or time.monotonic() >= deadline:
                raise
            time.sleep(0.2)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    scheme = "http" if protocol == "esplora" else "ssl" if tls else "tcp"
    host = "localhost" if tls else "127.0.0.1"
    try:
        yield f"{scheme}://{host}:{server.server_address[1]}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


@contextmanager
def socks(routes: dict[str, int]):
    """A loopback SOCKS5 proxy (no authentication) for proxied backends.

    `routes` maps each host a client may ask for to a loopback port, so a
    proxied name, `.onion` included, never reaches a resolver. Yields the
    proxy URL and the list of `(host, port)` pairs clients requested.
    """
    seen: list[tuple[str, int]] = []

    def exact(sock, size):
        data = b""
        while len(data) < size:
            chunk = sock.recv(size - len(data))
            if not chunk:
                raise ConnectionError("SOCKS client closed")
            data += chunk
        return data

    class Proxy(socketserver.BaseRequestHandler):
        def handle(self):
            client = self.request
            client.settimeout(10)
            version, count = exact(client, 2)
            if version != 5 or 0 not in exact(client, count):
                client.sendall(b"\x05\xff")
                return
            client.sendall(b"\x05\x00")
            version, command, _reserved, kind = exact(client, 4)
            if kind == 3:
                host = exact(client, exact(client, 1)[0]).decode("ascii")
            elif kind == 1:
                host = socket.inet_ntoa(exact(client, 4))
            else:
                host = socket.inet_ntop(socket.AF_INET6, exact(client, 16))
            port = int.from_bytes(exact(client, 2), "big")
            seen.append((host, port))
            if version != 5 or command != 1 or host not in routes:
                client.sendall(b"\x05\x04\x00\x01" + bytes(6))
                return
            upstream = socket.create_connection(("127.0.0.1", routes[host]), timeout=10)
            client.sendall(b"\x05\x00\x00\x01" + bytes(6))
            with upstream:
                while True:
                    ready, _, _ = select.select([client, upstream], [], [], 10)
                    if not ready:
                        return
                    for source in ready:
                        data = source.recv(65536)
                        if not data:
                            return
                        (upstream if source is client else client).sendall(data)

    class Tcp(socketserver.ThreadingTCPServer):
        daemon_threads = True
        allow_reuse_address = True

    server = Tcp(("127.0.0.1", 0), Proxy)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"socks5h://127.0.0.1:{server.server_address[1]}", seen
    finally:
        server.shutdown()
        server.server_close()
        thread.join()

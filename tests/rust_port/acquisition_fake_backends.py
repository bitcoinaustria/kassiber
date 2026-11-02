"""Deterministic loopback Esplora, Bitcoin Core and Electrum fakes for acquisition parity.

Only synthetic transactions are served. Each server records the exact request
sequence so native and Python runs can be compared request by request.
"""
from __future__ import annotations

from decimal import Decimal
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import socketserver
import threading

from embit.script import Script
from embit.transaction import Transaction, TransactionInput, TransactionOutput, Witness

GENESIS = '000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f'
BLOCK = 'bb' * 32


def _script(n):
    return bytes.fromhex('0014' + f'{n:02x}' * 20)


def build_chain():
    """Root <- ancestor; root -> child (output 0) -> grandchild; output 1 unspent."""
    ancestor = Transaction(version=2, vin=[TransactionInput(bytes.fromhex('00' * 32), 0xFFFFFFFF, Script(bytes.fromhex('03aabbcc')), 0xFFFFFFFF)],
                           vout=[TransactionOutput(50_000, Script(_script(1)))], locktime=0)
    root = Transaction(version=2, vin=[TransactionInput(ancestor.txid(), 0, Script(b''), 0xFFFFFFFD, Witness([b'\x30' * 71, b'\x02' * 33]))],
                       vout=[TransactionOutput(30_000, Script(_script(2))), TransactionOutput(19_000, Script(_script(3)))], locktime=100)
    child = Transaction(version=1, vin=[TransactionInput(root.txid(), 0, Script(b'\x00' * 23), 0xFFFFFFFE)],
                        vout=[TransactionOutput(29_000, Script(_script(4))), TransactionOutput(0, Script(bytes.fromhex('6a0474657374')))], locktime=0)
    grandchild = Transaction(version=2, vin=[TransactionInput(child.txid(), 0, Script(b''), 0xFFFFFFFF, Witness([b'\x01' * 64]))],
                             vout=[TransactionOutput(28_000, Script(bytes.fromhex('5120' + '05' * 32)))], locktime=0)
    return [ancestor, root, child, grandchild]


class Chain:
    def __init__(self, transactions, *, genesis=GENESIS, spender_index=True, broken=()):
        self.transactions = {tx.txid().hex(): tx for tx in transactions}
        self.order = [tx.txid().hex() for tx in transactions]
        self.genesis = genesis
        self.spender_index = spender_index
        self.broken = set(broken)

    def spender(self, txid, vout):
        for candidate in self.transactions.values():
            for index, row in enumerate(candidate.vin):
                if row.txid.hex() == txid and row.vout == vout:
                    return candidate.txid().hex(), index
        return None

    def prevout(self, row):
        source = self.transactions.get(row.txid.hex())
        return source.vout[row.vout] if source is not None and row.vout < len(source.vout) else None

    def esplora(self, txid):
        tx = self.transactions[txid]
        vin = []
        for row in tx.vin:
            coinbase = row.txid.hex() == '00' * 32
            entry = {'txid': row.txid.hex(), 'vout': row.vout, 'scriptsig': row.script_sig.data.hex(), 'witness': [w.hex() for w in row.witness.items],
                     'is_coinbase': coinbase, 'sequence': row.sequence}
            prev = None if coinbase else self.prevout(row)
            entry['prevout'] = None if prev is None else {'scriptpubkey': prev.script_pubkey.data.hex(), 'scriptpubkey_type': 'v0_p2wpkh', 'value': prev.value}
            vin.append(entry)
        vout = [{'scriptpubkey': out.script_pubkey.data.hex(), 'scriptpubkey_type': 'op_return' if out.script_pubkey.data[:1] == b'\x6a' else 'v0_p2wpkh', 'value': out.value}
                for out in tx.vout]
        index = self.order.index(txid)
        return {'txid': txid, 'version': tx.version, 'locktime': tx.locktime, 'vin': vin, 'vout': vout, 'size': len(tx.serialize()), 'weight': 4 * len(tx.serialize()),
                'fee': 1000, 'status': {'confirmed': index < 3, 'block_height': 100 + index if index < 3 else None, 'block_hash': BLOCK if index < 3 else None,
                                        'block_time': 1_700_000_000 + index if index < 3 else None}}

    def core(self, txid):
        tx = self.transactions[txid]
        vin = []
        for row in tx.vin:
            if row.txid.hex() == '00' * 32:
                vin.append({'coinbase': row.script_sig.data.hex(), 'sequence': row.sequence})
                continue
            entry = {'txid': row.txid.hex(), 'vout': row.vout, 'scriptSig': {'asm': '', 'hex': row.script_sig.data.hex()}, 'sequence': row.sequence}
            if row.witness.items:
                entry['txinwitness'] = [w.hex() for w in row.witness.items]
            prev = self.prevout(row)
            if prev is not None:
                entry['prevout'] = {'generated': False, 'height': 100, 'value': float(Decimal(prev.value) / Decimal(10 ** 8)),
                                    'scriptPubKey': {'asm': '', 'hex': prev.script_pubkey.data.hex(), 'type': 'witness_v0_keyhash'}}
            vin.append(entry)
        vout = [{'value': float(Decimal(out.value) / Decimal(10 ** 8)), 'n': n, 'scriptPubKey': {'asm': '', 'hex': out.script_pubkey.data.hex(),
                 'address': 'bc1qsynthetic' if n == 0 else None, 'type': 'witness_v0_keyhash'}} for n, out in enumerate(tx.vout)]
        index = self.order.index(txid)
        decoded = {'txid': txid, 'hash': txid, 'version': tx.version, 'size': len(tx.serialize()), 'vsize': len(tx.serialize()), 'weight': 4 * len(tx.serialize()),
                   'locktime': tx.locktime, 'vin': vin, 'vout': vout, 'hex': tx.serialize().hex()}
        if index < 3:
            decoded.update(blockhash=BLOCK, confirmations=3 - index, time=1_700_000_000)
        else:
            decoded['confirmations'] = 0
        return decoded


class Recorder:
    def __init__(self):
        self.log = []
        self.lock = threading.Lock()

    def record(self, entry):
        with self.lock:
            self.log.append(entry)

    def take(self):
        with self.lock:
            log, self.log = self.log, []
        return log


def http_server(chain, recorder, *, mode='esplora'):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'

        def log_message(self, *args):
            pass

        def reply(self, status, body, content='application/json'):
            data = body if isinstance(body, bytes) else body.encode()
            self.send_response(status)
            self.send_header('Content-Type', content)
            self.send_header('Content-Length', str(len(data)))
            self.send_header('Connection', 'close')
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            recorder.record({'method': 'GET', 'path': self.path, 'accept': self.headers.get('Accept'), 'authorization': self.headers.get('Authorization')})
            path = self.path
            if path == '/redirect/block-height/0':
                self.send_response(302); self.send_header('Location', '/block-height/0'); self.send_header('Content-Length', '0'); self.end_headers(); return
            if path.endswith('/block-height/0'):
                return self.reply(200, chain.genesis + '\n', 'text/plain')
            parts = path.strip('/').split('/')
            if len(parts) >= 2 and parts[-2] == 'tx' and parts[-1] in chain.transactions:
                txid = parts[-1]
                if txid in chain.broken:
                    return self.reply(200, '{"txid": "' + txid + '", "vin": [')
                return self.reply(200, json.dumps(chain.esplora(txid)))
            if len(parts) >= 3 and parts[-1] == 'outspends' and parts[-2] in chain.transactions:
                txid = parts[-2]
                rows = []
                for n in range(len(chain.transactions[txid].vout)):
                    spent = chain.spender(txid, n)
                    rows.append({'spent': spent is not None, **({'txid': spent[0], 'vin': spent[1], 'status': {'confirmed': True}} if spent else {})})
                return self.reply(200, json.dumps(rows))
            return self.reply(404, 'Transaction not found', 'text/plain')

        def do_POST(self):
            length = int(self.headers.get('Content-Length') or 0)
            body = json.loads(self.rfile.read(length))
            recorder.record({'method': 'POST', 'path': self.path, 'body': body, 'authorization': self.headers.get('Authorization')})
            ident, method, params = body.get('id'), body.get('method'), body.get('params')
            def result(value):
                return self.reply(200, json.dumps({'result': value, 'error': None, 'id': ident}))
            def error(code, message, status=500):
                return self.reply(status, json.dumps({'result': None, 'error': {'code': code, 'message': message}, 'id': ident}))
            if method == 'getblockhash':
                return result(chain.genesis)
            if method == 'getindexinfo':
                return result({'txospenderindex': {'synced': chain.spender_index, 'best_block_height': 103}} if chain.spender_index is not None else {})
            if method == 'getrawtransaction':
                txid = params[0]
                if txid not in chain.transactions:
                    return error(-5, 'No such mempool or blockchain transaction', 404)
                return result(chain.core(txid))
            if method == 'gettxspendingprevout':
                rows = []
                for ref in params[0]:
                    spent = chain.spender(ref['txid'], ref['vout'])
                    rows.append({'txid': ref['txid'], 'vout': ref['vout'], **({'spendingtxid': spent[0]} if spent else {})})
                return result(rows)
            return error(-32601, 'Method not found')

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def electrum_server(chain, recorder):
    class Handler(socketserver.StreamRequestHandler):
        def handle(self):
            for line in self.rfile:
                request = json.loads(line)
                recorder.record({'method': request.get('method'), 'params': request.get('params'), 'id': request.get('id')})
                method, params = request.get('method'), request.get('params') or []
                if method == 'server.version':
                    value = ['FakeElectrum 1.0', '1.4']
                elif method == 'server.features':
                    value = {'genesis_hash': chain.genesis, 'protocol_max': '1.4'}
                elif method == 'blockchain.transaction.get' and params and params[0] in chain.transactions:
                    if params[0] in chain.broken:
                        value = 'deadbeef'
                    else:
                        value = chain.transactions[params[0]].serialize().hex()
                else:
                    self.wfile.write((json.dumps({'jsonrpc': '2.0', 'id': request.get('id'), 'error': {'code': 2, 'message': 'missing'}}) + '\n').encode())
                    continue
                # A notification first: the client must skip unrelated messages.
                self.wfile.write(b'{"jsonrpc":"2.0","method":"blockchain.headers.subscribe","params":[]}\n')
                self.wfile.write((json.dumps({'jsonrpc': '2.0', 'id': request.get('id'), 'result': value}) + '\n').encode())

    class Server(socketserver.ThreadingTCPServer):
        allow_reuse_address = True
        daemon_threads = True

    server = Server(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server

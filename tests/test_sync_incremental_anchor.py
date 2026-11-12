"""Round-4 request-count repros, with real tip ancestry and reorg transitions."""
from collections import Counter
from contextlib import nullcontext
from dataclasses import replace
import hashlib
from types import SimpleNamespace

import pytest

from kassiber.core import sync_backends as sb
from kassiber.core.sync import WalletSyncState


@pytest.mark.parametrize('protocol,chain', [('esplora', 'bitcoin'), ('electrum', 'bitcoin'), ('electrum', 'liquid')])
def test_normal_tip_advance_reuses_history_but_reorg_refreshes_it(monkeypatch, protocol, chain):
    used, unused = 200, 40
    def script(i):
        return '0014' + hashlib.sha256(f'wallet-{i}'.encode()).hexdigest()[:40]
    def txid(i):
        return hashlib.sha256(f'tx-{i}'.encode()).hexdigest()
    def header(height):
        return (b'\x00' * 68 + (1700000000 + height).to_bytes(4, 'little') + b'\x00' * 8).hex()
    targets = [dict(script_pubkey=script(i), branch_index=0, address_index=i) for i in range(used + unused)]
    indices = {sb.scriptpubkey_scripthash(script(i)): i for i in range(used + unused)}
    tip = {'height': 900000, 'hash': '11' * 32, 'hex': header(900000)}
    block_hashes = {900000: tip['hash']}
    headers = {900000: tip['hex']}
    requests = Counter()
    def esplora_tx(i):
        return dict(txid=txid(i), fee=200,
            vin=[dict(txid=txid(1000+i), vout=0, prevout=dict(scriptpubkey='0014'+'ee'*20, value=100200))],
            vout=[dict(scriptpubkey=script(i), value=100000)],
            status=dict(confirmed=True, block_height=800000+i, block_hash='aa'*32, block_time=1700000000+i))
    def route(url, **kwargs):
        path = url.split('example.invalid/', 1)[1]
        if path == 'blocks/tip/hash':
            requests['tip_hash'] += 1
            return tip['hash']
        if path == 'blocks/tip/height':
            requests['tip_height'] += 1
            return str(tip['height'])
        if path.startswith('block-height/'):
            requests['block_hash'] += 1
            return block_hashes[int(path.split('/')[-1])]
        parts = path.split('/')
        assert parts[0] == 'scripthash', path
        i = indices[parts[1]]
        populated = i < used
        if len(parts) == 2:
            requests['stats'] += 1
            return dict(chain_stats=dict(tx_count=int(populated), funded_txo_count=int(populated),
                funded_txo_sum=100000*populated, spent_txo_count=0, spent_txo_sum=0), mempool_stats=dict(tx_count=0))
        if parts[2:] == ['txs', 'chain']:
            requests['history_chain'] += 1
            return [esplora_tx(i)] if populated else []
        if parts[2:] == ['txs', 'mempool']:
            requests['history_mempool'] += 1
            return []
        assert parts[2:] == ['utxo'], path
        requests['utxo'] += 1
        return []
    class Client:
        def call(self, method, params):
            requests[method] += 1
            if method == 'blockchain.headers.subscribe':
                return {'height': tip['height'], 'hex': tip['hex']}
            if method == 'blockchain.block.header':
                return headers.get(params[0], header(params[0]))
            if method == 'blockchain.scripthash.subscribe':
                i = indices[params[0]]
                return f'status-{i}' if i < used else None
            if method == 'blockchain.scripthash.get_history':
                i = indices[params[0]]
                return [dict(tx_hash=txid(i), height=800000+i)] if i < used else []
            if method == 'blockchain.scripthash.listunspent':
                return []
            assert method == 'blockchain.transaction.get', method
            return params[0]
        def batch_call(self, calls):
            return [self.call(method, params) for method, params in calls]
    tx_indices = {txid(i): i for i in range(used)}
    def decode(raw):
        i = tx_indices.get(raw)
        if i is None:
            return dict(txid=raw, vin=[], vout=[dict(script_hex='0014'+'ee'*20, value_sats=100200)])
        return dict(txid=raw, vin=[dict(txid=txid(1000+i), vout=0)],
                    vout=[dict(script_hex=script(i), value_sats=100000)])
    monkeypatch.setattr(sb, 'http_get_json', route)
    monkeypatch.setattr(sb, 'http_get_text', route)
    monkeypatch.setattr(sb, '_electrum_client_context', lambda _: nullcontext(Client()))
    monkeypatch.setattr(sb, 'decode_raw_transaction', decode)
    monkeypatch.setattr(sb, 'verify_electrum_transaction_id', lambda *args: None)
    monkeypatch.setattr(sb, 'decode_liquid_transaction', lambda raw: SimpleNamespace(
        vin=[SimpleNamespace(txid=txid(1000+tx_indices[raw]))] if raw in tx_indices else [], vout=[]))
    monkeypatch.setattr(sb, 'liquid_input_txid', lambda vin: vin.txid)
    monkeypatch.setattr(sb, 'record_components_from_liquid_tx', lambda *a, **k: [])
    state = WalletSyncState(chain=chain, network='main', descriptor_plan=None, policy_asset_id='',
        targets=targets, tracked_scripts={t['script_pubkey']: t for t in targets}, history_cache={})
    backend = dict(name='fixture', kind=protocol, url='https://example.invalid/')
    adapter = sb.compatibility_esplora_sync_adapter if protocol == 'esplora' else sb.compatibility_electrum_sync_adapter
    records, meta = adapter(backend, {}, state)
    counts = {'first': dict(requests)}
    state = replace(state, checkpoint=meta['freshness_checkpoint'],
        history_cache={r.get('external_id') or r['txid']: r['raw_json'] for r in records})
    requests.clear()
    _, meta = adapter(backend, {}, state)
    counts['same_tip'] = dict(requests)
    state = replace(state, checkpoint=meta['freshness_checkpoint'])
    tip.update(height=900001, hash='22'*32, hex=header(900001))
    block_hashes[900001] = tip['hash']
    headers[900001] = tip['hex']
    requests.clear()
    records, meta = adapter(backend, {}, state)
    counts['advance'] = dict(requests)
    print(protocol, chain, {key: {'total': sum(value.values()), 'requests': value} for key, value in counts.items()})
    assert sum(requests.values()) <= used + unused + 4
    assert not records
    assert meta['scripts_changed'] == 0
    assert not any('transaction.get' in name or 'history' in name for name in requests)
    # Even a taller tip can represent a reorg. Identical script statistics and
    # status hashes cannot hide a changed old tip block.
    state = replace(state, checkpoint=meta['freshness_checkpoint'])
    tip.update(height=900002, hash='33'*32, hex=header(900002))
    block_hashes[900002] = tip['hash']
    block_hashes[900001] = '44'*32
    headers[900001] = '44'*80
    requests.clear()
    _, meta = adapter(backend, {}, state)
    assert meta['scripts_changed'] >= used
    assert any('history' in name for name in requests)

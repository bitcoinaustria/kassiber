"""Compatibility cache ancestry matrix, including histories racing the tip read."""
import hashlib
from collections import Counter
from contextlib import nullcontext
from dataclasses import replace
from types import SimpleNamespace

import pytest

from kassiber.core import sync_backends as sb
from kassiber.core.sync import WalletSyncState

USED, UNUSED = 20, 10


def script(i):
    return '0014' + hashlib.sha256(f'w-{i}'.encode()).hexdigest()[:40]


def txid(i):
    return hashlib.sha256(f'tx-{i}'.encode()).hexdigest()


@pytest.fixture
def matrix(monkeypatch):
    class World:
        def __init__(self):
            self.chain = {h: ('a', h) for h in range(899980, 900001)}  # height -> (fork tag, height)
            self.tip = 900000
            self.special_height = None   # script 0 tx confirmed at this height (race)
            self.special = {}
            self.newtx = {}
            self.hook = None
            self.lie = {}
            self.req = Counter()
        def tag(self, h):
            return self.chain[h][0] if h in self.chain else 'a'
        def bhash(self, h):
            tag = self.tag(h); return hashlib.sha256(f'{tag}-{h}'.encode()).hexdigest()
        def bts(self, h):
            return 1700000000 + h + (0 if self.tag(h) == 'a' else 77)
        def bhex(self, h):
            return (bytes.fromhex(self.bhash(h)) + b'\x00' * 36 + self.bts(h).to_bytes(4, 'little') + b'\x00' * 8).hex()
        def extend(self, n, tag=None):
            for _ in range(n):
                self.tip += 1; self.chain[self.tip] = (tag or self.chain[self.tip - 1][0], self.tip)
        def reorg(self, depth, new_len, tag):
            base = self.tip - depth
            for h in list(self.chain):
                if h > base: del self.chain[h]
            self.tip = base
            self.extend(new_len, tag)
        def tx_height(self, i):
            return self.special.get(i, self.special_height if (i == 0 and self.special_height) else 800000 + i)

    W = World()
    indices = {sb.scriptpubkey_scripthash(script(i)): i for i in range(USED + UNUSED)}

    def esplora_tx(i, *, extra=False):
        h = W.newtx[i] if extra else W.tx_height(i)
        bh = W.bhash(h) if h in W.chain else 'aa' * 32
        bt = W.bts(h) if h in W.chain else 1700000000 + i
        return dict(txid=txid(5000 + i) if extra else txid(i), fee=200,
            vin=[dict(txid=txid(1000 + i), vout=0, prevout=dict(scriptpubkey='0014' + 'ee' * 20, value=100200))],
            vout=[dict(scriptpubkey=script(i), value=100000)],
            status=dict(confirmed=True, block_height=h, block_hash=bh, block_time=bt))

    def fire(i):
        if W.hook and W.hook[0] == i:
            action = W.hook[1]
            W.hook = None
            action(W)

    def route(url, **kw):
        path = url.split('example.invalid/', 1)[1]
        if path == 'blocks/tip/hash':
            W.req['tip_hash'] += 1; return W.lie.get('tip_hash', W.bhash(W.tip))
        if path == 'blocks/tip/height':
            W.req['tip_height'] += 1; return str(W.lie.get('tip_height', W.tip))
        if path.startswith('block-height/'):
            W.req['block_hash'] += 1
            h = int(path.split('/')[-1])
            if 'block_hash' in W.lie:
                v = W.lie['block_hash'](h)
                if isinstance(v, Exception): raise v
                return v
            if h not in W.chain: raise sb.AppError('not found', code='backend_error')
            return W.bhash(h)
        parts = path.split('/'); i = indices[parts[1]]; populated = i < USED
        fire(i)
        if len(parts) == 2:
            W.req['stats'] += 1
            populated = int(populated) + int(i in W.newtx)
            return dict(chain_stats=dict(tx_count=int(populated), funded_txo_count=int(populated), funded_txo_sum=100000 * populated,
                                         spent_txo_count=0, spent_txo_sum=0), mempool_stats=dict(tx_count=0))
        if parts[2:] == ['txs', 'chain']:
            W.req['history_chain'] += 1
            return ([esplora_tx(i, extra=True)] if i in W.newtx else []) + ([esplora_tx(i)] if populated else [])
        if parts[2:] == ['txs', 'mempool']:
            W.req['history_mempool'] += 1; return []
        W.req['utxo'] += 1; return []

    class Client:
        def call(self, method, params):
            W.req[method] += 1
            if method == 'blockchain.headers.subscribe':
                if 'subscribe' in W.lie: return W.lie['subscribe']()
                return {'height': W.tip, 'hex': W.bhex(W.tip)}
            if method == 'blockchain.block.header':
                if 'header' in W.lie:
                    v = W.lie['header'](params[0])
                    if isinstance(v, Exception): raise v
                    return v
                if params[0] > W.tip: raise sb.AppError('bad height', code='backend_error')
                return W.bhex(params[0])
            if method == 'blockchain.scripthash.subscribe':
                i = indices[params[0]]; fire(i)
                return f'status-{i}-{W.tx_height(i)}-{W.newtx.get(i)}' if i < USED else None
            if method == 'blockchain.scripthash.get_history':
                i = indices[params[0]]
                return ([dict(tx_hash=txid(i), height=W.tx_height(i))] if i < USED else []) + ([dict(tx_hash=txid(5000 + i), height=W.newtx[i])] if i in W.newtx else [])
            if method == 'blockchain.scripthash.listunspent': return []
            assert method == 'blockchain.transaction.get', method
            return params[0]
        def batch_call(self, calls): return [self.call(m, p) for m, p in calls]

    tx_indices = {txid(i): i for i in range(USED)}
    tx_indices.update({txid(5000 + i): i for i in range(USED)})
    def decode(raw):
        i = tx_indices.get(raw)
        if i is None: return dict(txid=raw, vin=[], vout=[dict(script_hex='0014' + 'ee' * 20, value_sats=100200)])
        return dict(txid=raw, vin=[dict(txid=txid(1000 + i), vout=0)], vout=[dict(script_hex=script(i), value_sats=100000)])

    for name, value in [
        ('http_get_json', route), ('http_get_text', route),
        ('_electrum_client_context', lambda _: nullcontext(Client())),
        ('decode_raw_transaction', decode),
        ('verify_electrum_transaction_id', lambda *args: None),
        ('decode_liquid_transaction', lambda raw: SimpleNamespace(
            vin=[SimpleNamespace(txid=txid(1000 + tx_indices[raw]))] if raw in tx_indices else [], vout=[])),
        ('liquid_input_txid', lambda vin: vin.txid),
        ('record_components_from_liquid_tx', lambda txid, occurred_at, *a, **k: [dict(
            txid=txid, occurred_at=occurred_at, confirmed_at=k['confirmed_at'], raw_json='{}')]),
    ]:
        monkeypatch.setattr(sb, name, value)

    targets = [dict(script_pubkey=script(i), branch_index=0, address_index=i) for i in range(USED + UNUSED)]
    def base_state(chain):
        return WalletSyncState(chain=chain, network='main', descriptor_plan=None, policy_asset_id='', targets=targets,
                               tracked_scripts={t['script_pubkey']: t for t in targets}, history_cache={})

    def tx0_view(records, meta, protocol):
        cp = meta['freshness_checkpoint']
        out = {}
        for r in records:
            if (r.get('txid') or '') == txid(0) or txid(0) in (r.get('external_id') or ''):
                raw = r.get('raw_json')
                out['rec_time'] = r.get('occurred_at') or r.get('confirmed_at')
                break
        if protocol == 'electrum':
            hs = cp.get('electrum_headers') or {}
            if W.special_height: out['cached_hdr_ts'] = hs.get(str(W.special_height))
        return out

    def run(protocol, chain, scenario):
        nonlocal W
        W = World()
        adapter = sb.compatibility_esplora_sync_adapter if protocol == 'esplora' else sb.compatibility_electrum_sync_adapter
        backend = dict(name='fixture', kind=protocol, url='https://example.invalid/')
        state = base_state(chain)
        pre = scenario.get('pre')
        if pre: pre(W)
        records, meta = adapter(backend, {}, state)
        cp = dict(meta['freshness_checkpoint'])
        initial = tx0_view(records, meta, protocol)
        if scenario.get('cp'): cp = scenario['cp'](cp)
        state = replace(state, checkpoint=cp, history_cache={r.get('external_id') or r['txid']: r['raw_json'] for r in records})
        W.lie.clear()
        scenario['change'](W)
        W.req.clear()
        records, meta = adapter(backend, {}, state)
        W.lie.clear()
        hist = sum(v for k, v in W.req.items() if 'history' in k)
        return dict(initial=initial, changed=meta.get('scripts_changed'), history_reqs=hist, total=sum(W.req.values()), reqs=dict(W.req), **tx0_view(records, meta, protocol))

    def run3(protocol, chain, *, first_sync=False, invalid_proof=None, lower_fork=False):
        nonlocal W
        W = World()
        adapter = sb.compatibility_esplora_sync_adapter if protocol == 'esplora' else sb.compatibility_electrum_sync_adapter
        backend = dict(name='fixture', kind=protocol, url='https://example.invalid/')
        state = base_state(chain)
        W.special = {0: 900000}

        def reorg(w):
            w.reorg(1, 2, 'b')
            w.newtx[1] = 900001

        steps = []
        for step in range(3):
            W.lie.clear()
            if step == (0 if first_sync else 1):
                if lower_fork:
                    # Both entries race above the initial tip. Only the lower
                    # entry belongs to an orphan; validating the maximum misses it.
                    W.extend(2)
                    W.special = {0: 900001, 1: 900002}
                    W.lie.update(tip_hash=W.bhash(900000), tip_height=900000,
                                 subscribe=lambda: {'height': 900000, 'hex': W.bhex(900000)})
                    real_hash = W.bhash
                    W.lie['block_hash'] = lambda h: 'ff' * 32 if h == 900001 else real_hash(h)
                    reads = 0
                    def raced_header(height):
                        nonlocal reads
                        header = W.bhex(height)
                        if height == 900001 and not reads:
                            reads += 1
                            W.reorg(2, 2, 'b')
                        return header
                    W.lie['header'] = raced_header
                else:
                    W.hook = (1, reorg)
                if invalid_proof:
                    value = sb.AppError('unavailable', code='backend_error') if invalid_proof == 'error' else 'zz'
                    def proof_only(normal):
                        initial_reads = 0
                        def read(height):
                            nonlocal initial_reads
                            if height == 900000:
                                initial_reads += 1
                                if initial_reads > int(first_sync):
                                    return value
                            return normal(height)
                        return read
                    W.lie['block_hash'] = proof_only(W.bhash)
                    W.lie['header'] = proof_only(W.bhex)
            W.req.clear()
            records, meta = adapter(backend, {}, state)
            cp = meta['freshness_checkpoint']
            cache = dict(state.history_cache)
            cache.update({r.get('external_id') or r['txid']: r['raw_json'] for r in records})
            state = replace(state, checkpoint=cp, history_cache=cache)
            steps.append(dict(checkpoint=cp, records=records, requests=dict(W.req),
                              history_reqs=sum(v for k, v in W.req.items() if 'history' in k)))
        return steps, W.bts(W.tx_height(0))

    def drop(key): return lambda cp: {k: v for k, v in cp.items() if k != key}
    def setkey(key, val): return lambda cp: {**cp, key: val}
    def lie(**kw):
        def f(w): w.lie.update(kw)
        return f
    def seq(*fs):
        def f(w):
            for g in fs: g(w)
        return f

    def race_pre(w):
        # Tip read returned 900000; block 900001 (fork a) arrived before history reads.
        w.lie.update(tip_hash=w.bhash(900000), tip_height=900000,
                     subscribe=lambda: {'height': 900000, 'hex': W.bhex(900000)})
        w.extend(1); w.special_height = 900001
    def hdr_lie(fn):
        return lambda h: fn(h) if h == 900000 else (W.bhex(h) if h <= W.tip else sb.AppError('bad', code='backend_error'))
    S = {
     'same_tip':              dict(change=lambda w: None, expect='reuse'),
     'extend1':               dict(change=lambda w: w.extend(1), expect='reuse'),
     'extend10_stale_anchor': dict(change=lambda w: w.extend(10), expect='reuse'),
     'same_height_new_hash':  dict(change=lambda w: w.reorg(1, 1, 'b'), expect='refresh'),
     'reorg_depth1_taller':   dict(change=lambda w: w.reorg(1, 2, 'b'), expect='refresh'),
     'reorg_depth2_taller':   dict(change=lambda w: w.reorg(2, 4, 'b'), expect='refresh'),
     'reorg_depth10_taller':  dict(change=lambda w: w.reorg(10, 12, 'b'), expect='refresh'),
     'reorg_shorter':         dict(change=lambda w: w.reorg(3, 2, 'b'), expect='refresh'),
     'legacy_no_height':      dict(change=lambda w: w.extend(1), cp=lambda cp: drop('esplora_tip_height')(drop('electrum_tip_height')(cp)), expect='refresh'),
     'height_bool':           dict(change=lambda w: w.extend(1), cp=lambda cp: setkey('esplora_tip_height', True)(setkey('electrum_tip_height', True)(cp)), expect='refresh'),
     'height_str':            dict(change=lambda w: w.extend(1), cp=lambda cp: setkey('esplora_tip_height', '900000')(setkey('electrum_tip_height', '900000')(cp)), expect='refresh'),
     'height_wrong_low':      dict(change=lambda w: w.reorg(1, 2, 'b'), cp=lambda cp: setkey('esplora_tip_height', 899990)(setkey('electrum_tip_height', 899990)(cp)), expect='refresh'),
     # backend lies / errors, during a reorg (must refresh)
     'reorg_blockhash_err':   dict(change=seq(lambda w: w.reorg(1, 2, 'b'), lie(block_hash=lambda h: sb.AppError('x', code='backend_error'), header=hdr_lie(lambda h: sb.AppError('x', code='backend_error')))), expect='refresh'),
     'reorg_blockhash_garbage': dict(change=seq(lambda w: w.reorg(1, 2, 'b'), lie(block_hash=lambda h: 'zz', header=hdr_lie(lambda h: 'zz'))), expect='refresh'),
     'reorg_tipheight_lie':   dict(change=seq(lambda w: w.reorg(1, 2, 'b'), lie(tip_height=900000)), expect='refresh'),
     'reorg_sub_height_lie':  dict(change=seq(lambda w: w.reorg(1, 2, 'b'), lie(subscribe=lambda: {'height': 900000, 'hex': W.bhex(W.tip)})), expect='refresh'),
     'reorg_sub_noheight':    dict(change=seq(lambda w: w.reorg(1, 2, 'b'), lie(subscribe=lambda: {'hex': W.bhex(W.tip)})), expect='refresh'),
     # previous sync raced a new block: history saw height 900001 (block a) after tip 900000 was read
     'race_then_reorg':       dict(pre=race_pre,
                                   change=lambda w: w.reorg(1, 2, 'b'), expect='refresh'),
     'race_then_extend':      dict(pre=race_pre,
                                   change=lambda w: w.extend(1), expect='reuse'),
    }

    return run, S, run3


SCENARIOS = [
    'same_tip', 'extend1', 'extend10_stale_anchor', 'same_height_new_hash',
    'reorg_depth1_taller', 'reorg_depth2_taller', 'reorg_depth10_taller', 'reorg_shorter',
    'legacy_no_height', 'height_bool', 'height_str', 'height_wrong_low',
    'reorg_blockhash_err', 'reorg_blockhash_garbage', 'reorg_tipheight_lie',
    'reorg_sub_height_lie', 'reorg_sub_noheight', 'race_then_reorg', 'race_then_extend',
]


@pytest.mark.parametrize('protocol,chain', [('esplora', 'bitcoin'), ('electrum', 'bitcoin'), ('electrum', 'liquid')])
@pytest.mark.parametrize('scenario', SCENARIOS)
def test_compatibility_history_anchor_matrix(matrix, protocol, chain, scenario):
    run, scenarios, _ = matrix
    config = scenarios[scenario]
    result = run(protocol, chain, config)
    refreshed = bool(result['history_reqs'] and result['changed'])
    assert refreshed == (config['expect'] == 'refresh'), result
    if scenario == 'race_then_reorg':
        if protocol == 'electrum':
            assert result['cached_hdr_ts'] == result['initial']['cached_hdr_ts'] + 77
        if chain == 'bitcoin':
            assert result['rec_time'] != result['initial']['rec_time']


@pytest.mark.parametrize('protocol,chain', [('esplora', 'bitcoin'), ('electrum', 'bitcoin'), ('electrum', 'liquid')])
@pytest.mark.parametrize('version', [None, 1])
def test_legacy_history_anchor_is_rebuilt(matrix, protocol, chain, version):
    run, scenarios, _ = matrix
    config = {**scenarios['same_tip'], 'cp': lambda cp: {
        **{key: value for key, value in cp.items() if key != 'compatibility_history_anchor_version'},
        **({'compatibility_history_anchor_version': version} if version is not None else {}),
    }}
    assert run(protocol, chain, config)['history_reqs'] > 0


@pytest.mark.parametrize('protocol,chain', [('esplora', 'bitcoin'), ('electrum', 'bitcoin'), ('electrum', 'liquid')])
@pytest.mark.parametrize('first_sync', [False, True], ids=['mid_sync_reorg_s2', 'mid_sync_reorg_s1'])
@pytest.mark.parametrize('invalid_proof', [None, 'error', 'garbage'])
def test_mid_sync_reorg_cannot_advance_anchor(matrix, protocol, chain, first_sync, invalid_proof):
    _, _, run3 = matrix
    steps, truth = run3(protocol, chain, first_sync=first_sync, invalid_proof=invalid_proof)
    raced = steps[0 if first_sync else 1]['checkpoint']
    assert raced[protocol + '_tip_height'] is None
    refreshed = steps[1 if first_sync else 2]
    assert refreshed['history_reqs'] > 0
    tx0 = next(r for r in refreshed['records'] if r.get('txid') == txid(0) or txid(0) in r.get('external_id', ''))
    assert tx0['occurred_at'] == sb.timestamp_to_iso(truth)
    if protocol == 'electrum':
        assert refreshed['checkpoint']['electrum_headers']['900000'] == truth


def test_esplora_checks_every_raced_block(matrix):
    _, _, run3 = matrix
    steps, _ = run3('esplora', 'bitcoin', first_sync=True, lower_fork=True)
    assert steps[0]['checkpoint']['esplora_tip_height'] is None
    assert steps[1]['history_reqs'] > 0


@pytest.mark.parametrize('chain', ['bitcoin', 'liquid'])
def test_electrum_checks_every_raced_header(matrix, chain):
    _, _, run3 = matrix
    steps, truth = run3('electrum', chain, first_sync=True, lower_fork=True)
    assert steps[0]['checkpoint']['electrum_tip_height'] is None
    assert steps[1]['history_reqs'] > 0
    assert steps[1]['checkpoint']['electrum_headers']['900001'] == truth


@pytest.mark.parametrize('chain', ['bitcoin', 'liquid'])
def test_electrum_version_two_anchor_is_rebuilt(matrix, chain):
    run, scenarios, _ = matrix
    config = {**scenarios['same_tip'], 'cp': lambda cp: {
        **cp, 'compatibility_history_anchor_version': 2,
    }}
    assert run('electrum', chain, config)['history_reqs'] > 0

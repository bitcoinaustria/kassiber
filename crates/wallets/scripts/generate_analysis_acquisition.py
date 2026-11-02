"""Unchanged-Python oracle for acquisition plans; planning never contacts a backend."""
from __future__ import annotations

import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_grants import backend, evaluate
from generate_analysis_watches import BIND, PREV, database

DEST = ROOT / 'crates/wallets/tests/data/analysis_acquisition.json'
A, B, C = 'a1' * 32, 'b2' * 32, 'c4' * 32
ADDRESS = 'bc1qqyqszqgpqyqszqgpqyqszqgpqyqszqgp8g62jc'


def tx(txid, inputs, outputs):
    vout = [{'n': n, 'value': 5000 - 100 * n, 'scriptpubkey': '0014' + '01' * 20, 'scriptpubkey_address': ADDRESS} if address else
            {'n': n, 'value': 5000 - 100 * n, 'scriptpubkey': '0014' + f'{n + 2:02x}' * 20} for n, address in enumerate(outputs)]
    return json.dumps({'txid': txid, 'vin': [{'txid': t, 'vout': n} for t, n in inputs], 'vout': vout})


def observation(txid, inputs, outputs):
    return ("INSERT INTO chain_analysis_observations(profile_id,chain,network,txid,payload_json,status_json,source_name,observed_at) "
            f"VALUES('p','bitcoin','main','{txid}','{tx(txid, inputs, outputs)}','{{}}','local','fixed');")


GRAPH = observation(A, [(PREV, 0)], [True, False]) + observation(B, [(A, 0)], [True])
BACKENDS = (backend() + backend('esplora', 'esplora', url='http://127.0.0.1:3002') + backend('electrum', 'electrum', url='tcp://127.0.0.1:50001')
            + backend('liquid', 'liquid-esplora', chain='liquid', network='liquidv1', url='http://127.0.0.1:3003')
            + backend('liquidcore', chain='liquid', network='liquidv1') + backend('lnd', 'lnd') + backend('alias', network='mainnet')
            + backend('bogus', network='bogus') + backend('testnet', network='test') + backend('instance', config='{"chain_instance_id":"lab"}')
            + backend('liquidtest', 'liquid-esplora', chain='liquid', network='liquidtestnet', url='http://127.0.0.1:3004'))
BOOK = BIND + BACKENDS + GRAPH


def vectors():
    corpus = []

    def add(name, actions, **kwargs):
        case = {'group': 'acquisition', 'name': name, 'encrypted': False, 'sql': BOOK, 'actions': actions, **kwargs}
        conn = database(case['sql'], case['encrypted'])
        try:
            case['expected'] = evaluate(case, conn, resolve=True)
        finally:
            conn.close()
        corpus.append(case)

    def act(args):
        return {'operation': 'acquire.plan', 'args': args}

    base = {'backend': 'Core', 'subject': A, 'chain': 'bitcoin', 'network': 'main', 'direction': 'both'}
    plans = [None, [], {}, base, {**base, 'depth': 10, 'max_transactions': 200}, {k: v for k, v in base.items() if k != 'direction'}, {**base, 'unknown': 1},
             {**base, 'chain': 'liquid', 'network': 'signet'}, {**base, 'chain': 'lightning'}, {**base, 'network': 'mainnet'}, {**base, 'network': 1},
             {**base, 'backend': ''}, {**base, 'subject': ' '}, {**base, 'subject': 'x' * 257}, {**base, 'direction': 'sideways'},
             *({**base, 'depth': v} for v in (0, 11, True, '3', 1.0)), *({**base, 'max_transactions': v} for v in (0, 201, False)),
             {**base, 'genesis_hash': 'ab' * 32}, {**base, 'genesis_hash': None}, {**base, 'genesis_hash': '000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f'},
             {**base, 'backend': 'liquidtest', 'chain': 'liquid', 'network': 'test'}, {**base, 'backend': 'liquidtest', 'chain': 'liquid', 'network': 'test', 'genesis_hash': 'AB' * 32},
             {**base, 'backend': 'liquidtest', 'chain': 'liquid', 'network': 'test', 'genesis_hash': 'ab' * 32},
             {**base, 'backend': 'absent'}, {**base, 'backend': 'lnd'}, {**base, 'backend': 'testnet'}, {**base, 'backend': 'alias'}, {**base, 'backend': 'bogus'},
             {**base, 'backend': 'liquidcore', 'chain': 'liquid'}, {**base, 'backend': 'instance'}, {**base, 'backend': 'esplora'}, {**base, 'backend': 'electrum'},
             {**base, 'backend': 'liquid', 'chain': 'liquid'},
             {**base, 'subject': 'TX:' + A.upper()}, {**base, 'subject': f'bitcoin:main:tx:{A}'}, {**base, 'subject': f'bitcoin:test:tx:{A}'}, {**base, 'subject': f'{A}:7'},
             {**base, 'subject': f'out:{A}:0'}, {**base, 'subject': ADDRESS}, {**base, 'subject': ADDRESS, 'max_transactions': 1}, {**base, 'subject': 'unknown-wallet'},
             {**base, 'subject': f'bitcoin:main:domain:bound-main:tx:{B}'}, {**base, 'subject': 'f' * 63}]
    add('plan-arguments', [act(args) for args in plans])
    add('plan-unbound', [act(base)], sql=BACKENDS + GRAPH)
    add('plan-book-instance', [act({**base, 'backend': 'instance'}), act(base)], sql=BOOK.replace("'main',NULL,", "'main','lab',", 1).replace('"chain_instance_id":null', '"chain_instance_id":"lab"'))
    return corpus


if __name__ == '__main__':
    DEST.write_text(json.dumps(vectors(), ensure_ascii=True, separators=(',', ':')) + '\n')

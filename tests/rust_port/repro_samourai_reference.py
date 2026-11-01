import json
from pathlib import Path
import sqlite3
import tempfile

from embit import bip32
from embit.networks import NETWORKS
from crates.wallets.scripts.generate_samourai_vectors import source
from kassiber.core import samourai
from tests.test_samourai_import import _seed_book


def main():
    descriptor = source(descriptor=True)
    del descriptor['change_descriptor']
    normalized = samourai.load_samourai_source_set_payload({'children': [descriptor]}, network='main')
    assert len(normalized) == 1
    assert 'change_descriptor' not in normalized[0]['config']
    print('single receive descriptors are accepted through synthesized branch validation')
    item = {**source(), 'label': 'Ignored custom label'}
    normalized = samourai.load_samourai_source_set_payload({'xpubs': [item]}, network='main')
    assert normalized[0]['label'] == 'Deposit Native SegWit'
    print('xpub source labels are ignored')
    item = source(descriptor=True)
    item['descriptor'] = item['descriptor'].replace('wpkh(', 'pkh(')
    item['change_descriptor'] = item['change_descriptor'].replace('wpkh(', 'pkh(')
    normalized = samourai.load_samourai_source_set_payload({'children': [item]}, network='main')
    assert normalized[0]['config']['samourai']['script_type'] == 'p2wpkh'
    assert normalized[0]['config']['descriptor'].startswith('pkh(')
    print('an explicit script declaration is not compared with the descriptor wrapper')
    reference = 'ab ' * 20 + 'abcd'
    normalized = samourai.load_samourai_source_set_payload({'xpubs': [{**source(), 'round_txid': reference}]}, network='main')
    assert normalized[0]['config']['samourai']['round_txid'] == reference
    assert len(reference) == 64 and len(bytes.fromhex(reference)) == 22
    print('round references can contain hex whitespace and decode to fewer than thirty-two bytes')
    item = source(network='test')
    key = bip32.HDKey.from_base58(item['xpub'])
    key.version = NETWORKS['main']['ypub']
    item['xpub'] = key.to_base58()
    normalized = samourai.load_samourai_source_set_payload({'xpubs': [item]}, network='test')
    assert normalized[0]['config']['network'] == 'test'
    assert item['xpub'] in normalized[0]['config']['descriptor']
    print('SLIP132 public versions bypass the reference xpub/tpub network-family check')
    with tempfile.TemporaryDirectory(prefix='samourai-reference-repro-') as root:
        path = Path(root) / 'source.json'
        path.write_text(json.dumps({'xpubs': [source()]}))
        with sqlite3.connect(':memory:') as conn:
            conn.row_factory = sqlite3.Row
            schema = Path(__file__).resolve().parents[2] / 'crates/store/fixtures/creation.json'
            for statement in json.loads(schema.read_text())['statements']:
                conn.execute(statement)
            _seed_book(conn)
            conn.execute("UPDATE profiles SET last_processed_at='2025-01-01',last_processed_tx_count=3,journal_input_version=7")
            conn.commit()
            result = samourai.import_samourai_wallet_group(conn, 'ws-1', 'profile-1',
                label='Group', source_set_file=str(path), gap_limit=10)
            assert result['group']['gap_limit'] == 10
            assert result['children'][0]['gap_limit'] == 100
            assert conn.execute('SELECT count(*) FROM import_batches').fetchone()[0] == 0
            profile = conn.execute('SELECT * FROM profiles').fetchone()
            assert profile['journal_input_version'] == 7 and profile['last_processed_tx_count'] == 3
            print('parent gap keeps the requested value while child gaps use the minimum')
            print('group creation records no import run and does not invalidate journal readiness')


if __name__ == '__main__':
    main()

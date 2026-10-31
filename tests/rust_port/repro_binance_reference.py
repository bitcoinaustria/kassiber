from pathlib import Path
import tempfile

from kassiber.importers import load_binance_supplemental_csv_records
from crates.wallets.scripts.generate_binance_vectors import inputs


def main():
    cases = inputs()
    with tempfile.TemporaryDirectory(prefix='binance-reference-repro-') as root:
        source = Path(root) / 'input.csv'
        def load(text):
            source.write_bytes(text.encode())
            return load_binance_supplemental_csv_records(source)
        ordinary = load(cases['autoinvest'])[0]
        conflicting = load(cases['mismatched-base'])[0]
        assert conflicting['asset'] == 'BTC'
        assert conflicting['amount'] == ordinary['amount']
        assert 'ETH' in conflicting['raw_json']
        repeated = load(cases['auto-duplicate'])
        assert repeated[0]['txid'] != repeated[1]['txid']
        assert repeated[0]['amount'] == repeated[1]['amount']
        dividend = load(cases['dividend'].replace('reward,', 'ab' * 32 + ','))[0]
        assert dividend['txid'] == 'binance:' + 'ab' * 32
        scaled = load(cases['autoinvest'].replace('0.002 BTC', '0.0020 BTC'))[0]
        assert scaled['amount'] == ordinary['amount']
        assert scaled['txid'] != ordinary['txid']
    print('Preserved Binance reference quirks: conflicting base symbol, row-index and decimal-scale identities')


if __name__ == '__main__':
    main()

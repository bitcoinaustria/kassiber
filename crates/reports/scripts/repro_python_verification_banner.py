import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

import openpyxl
import xlsxwriter
from kassiber.core import report_verify


def main():
    root = ROOT / 'target' / 'r5-reports13' / 'banner-repro'
    root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=root) as scratch:
        path = Path(scratch) / 'mismatch.xlsx'
        workbook = xlsxwriter.Workbook(path)
        report_verify.augment_workbook(workbook, gains_algorithm='FIFO', tax_country='generic',
            fiat_currency='EUR', wallet_scope_label=None, acquisitions=[], asset_rows=[],
            disposals=[dict(asset='BTC', quantity_msat=1000, quantity=1e-8, proceeds=12.34,
                            cost_basis=10.0, gain_loss=99.31, gain_loss_kassiber=99.31)])
        workbook.close()
        cached = openpyxl.load_workbook(path, data_only=True)
        observed = dict(disposal_check=cached['Disposals']['L3'].value,
                        workbook_banner=cached['Verify']['B2'].value)
        cached.close()
        assert observed == dict(disposal_check='DIFF', workbook_banner='ALL CHECKS OK')
        print(json.dumps(observed, sort_keys=True))


if __name__ == '__main__':
    main()

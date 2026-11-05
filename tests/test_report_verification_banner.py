"""The full-report banner agrees with its emitted cached checks before recalc."""
import openpyxl
import pytest
import xlsxwriter

from kassiber.core import report_verify


@pytest.mark.parametrize('mismatch', [None, 'disposal', 'holdings_qty_kassiber', 'cost_basis_kassiber',
    'avg_cost_kassiber', 'market_value_kassiber', 'unrealized_kassiber', 'realized_gain_kassiber'])
def test_cached_verification_banner_matches_all_check_columns(tmp_path, mismatch):
    path = tmp_path / 'report.xlsx'
    asset = dict(asset='BTC', quantity=1, cost_basis=10, market_value=12, unrealized_pnl=2, realized_gain=2.34,
        holdings_qty_kassiber=1, cost_basis_kassiber=10, avg_cost_kassiber=10, market_value_kassiber=12,
        unrealized_kassiber=2, realized_gain_kassiber=2.34)
    disposal = dict(asset='BTC', quantity_msat=1000, quantity=1e-8, proceeds=12.34, cost_basis=10,
                    gain_loss=2.34, gain_loss_kassiber=2.34)
    if mismatch == 'disposal':
        disposal.update(gain_loss=99.31, gain_loss_kassiber=99.31)
    elif mismatch:
        asset[mismatch] += 1
    with xlsxwriter.Workbook(path) as workbook:
        report_verify.augment_workbook(workbook, gains_algorithm='FIFO', tax_country='generic',
            fiat_currency='EUR', wallet_scope_label=None, acquisitions=[], asset_rows=[asset], disposals=[disposal])
    cached = openpyxl.load_workbook(path, data_only=True)
    try:
        checks = [cached['Disposals']['L3'].value] + [cached['Control'].cell(3, c).value for c in (7, 10, 13, 16, 19, 22)]
        assert any(str(check).startswith('DIFF') for check in checks) == bool(mismatch)
        expected = 'MISMATCH — see the highlighted check columns' if mismatch else 'ALL CHECKS OK'
        assert cached['Verify']['B2'].value == expected
    finally:
        cached.close()
    live = openpyxl.load_workbook(path, data_only=False)
    try:
        assert 'COUNTA' in live['Verify']['B2'].value
        assert 'COUNTIF' in live['Verify']['B2'].value
    finally:
        live.close()


def test_empty_workbook_cached_verification_passes(tmp_path):
    path = tmp_path / 'empty.xlsx'
    with xlsxwriter.Workbook(path) as workbook:
        report_verify.augment_workbook(workbook, gains_algorithm='FIFO', tax_country='generic',
            fiat_currency='EUR', wallet_scope_label=None, acquisitions=[], asset_rows=[], disposals=[])
    cached = openpyxl.load_workbook(path, data_only=True)
    try:
        assert cached['Verify']['B2'].value == 'ALL CHECKS OK'
    finally:
        cached.close()

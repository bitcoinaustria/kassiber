"""Direct payout terms apply only to their reviewed source allocation."""
import json
from tests import cli_runner
from decimal import Decimal

import pytest

from kassiber.core.custody_journal import CustodyJournalBuilder
from kassiber.daemon import _ui_swap_matching_payload_from_conn
from kassiber.db import open_db
from tests.test_custody_authored_migration import _scope, _tx, NOW


@pytest.mark.parametrize('surface', ['cli', 'daemon'])
@pytest.mark.parametrize('policy,asset', [('taxable', 'EUR'), ('carrying-value', 'LBTC')])
def test_partial_payout_terms_do_not_price_residual(tmp_path, surface, policy, asset):
    conn = open_db(tmp_path)
    try:
        _scope(conn)
        _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW, fee=1000)
        conn.commit()
        args = dict(payout_asset=asset, payout_amount=('1' if asset == 'EUR' else '0.0000004'), policy=policy,
                    payout_fiat_value='10', out_amount='0.0000004')
        if surface == 'cli':
            command = ['--data-root', str(tmp_path),
                       '--machine', 'transfers', 'payouts', 'create',
                       '--workspace', 'Books', '--profile', 'Book', '--tx-out', 'out']
            for key, value in args.items():
                command.extend(['--' + key.replace('_', '-'), value])
            result = cli_runner.run(command, capture_output=True, text=True)
            assert result.returncode == 0, result.stdout + result.stderr
        else:
            _ui_swap_matching_payload_from_conn(conn, 'ui.transfers.payouts.create',
                dict(workspace='Books', profile='Book', tx_out='out', **args))
        profile = conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
        decisions = CustodyJournalBuilder(conn, profile).build_custody_projection()
        projection = decisions.finalized_tax_projection
        residual = [r for r in projection.rows if r.get('custody_slice_start_msat') == 0
                    and r.get('custody_slice_end_msat') == 60000]
        assert len(residual) == 1
        assert residual[0].get('pricing_method') != 'direct_payout_review'
        assert residual[0].get('fiat_value_exact') is None
        if policy == 'taxable':
            reviewed = [r for r in projection.rows if r.get('pricing_method') == 'direct_payout_review']
            assert len(reviewed) == 1
            assert Decimal(reviewed[0]['fiat_value_exact']) == 10
        else:
            assert len(projection.cross_asset_pairs) == 1
            assert len([r for r in projection.rows if r['id'].startswith('direct-payout:')]) == 2
    finally:
        conn.close()


@pytest.mark.parametrize('value', ['3.333', '123456789012.1234567890123456789', '3.3330000000000002'])
def test_payout_preserves_authored_decimal(tmp_path, value):
    conn = open_db(tmp_path)
    try:
        _scope(conn)
        _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW)
        conn.commit()
        _ui_swap_matching_payload_from_conn(conn, 'ui.transfers.payouts.create',
            dict(workspace='Books', profile='Book', tx_out='out', payout_asset='EUR',
                 payout_amount='1', policy='taxable', payout_fiat_value=value))
        stored = conn.execute('SELECT payout_fiat_value_exact FROM custody_component_economic_terms').fetchone()[0]
        assert stored == value
        conn.close()
        conn = open_db(tmp_path)
        assert conn.execute('SELECT payout_fiat_value_exact FROM custody_component_economic_terms').fetchone()[0] == value
        projection = CustodyJournalBuilder(conn, conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()).build_custody_projection().finalized_tax_projection
        assert next(r['fiat_value_exact'] for r in projection.rows if r.get('pricing_method') == 'direct_payout_review') == value
    finally:
        conn.close()


@pytest.mark.parametrize('priced', [False, True])
@pytest.mark.parametrize('policy,asset', [('taxable', 'EUR'), ('carrying-value', 'LBTC')])
def test_partial_payout_ledger_journals_and_report(tmp_path, priced, policy, asset):
    from kassiber.cli.handlers import create_direct_swap_payout, process_journals

    conn = open_db(tmp_path)
    try:
        _scope(conn)
        _tx(conn, 'buy', 'btc', 'inbound', 'BTC', 200000, '2025-12-01T00:00:00Z')
        conn.execute("UPDATE transactions SET kind='buy', fiat_rate=1000000, fiat_rate_exact='1000000' WHERE id='buy'")
        _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW, fee=1000)
        if priced:
            conn.execute("UPDATE transactions SET fiat_rate=1000000, fiat_rate_exact='1000000' WHERE id='out'")
        conn.commit()
        create_direct_swap_payout(conn, 'Books', 'Book', 'out', payout_asset=asset,
            payout_amount='0.0000004', policy=policy, payout_fiat_value='10',
            out_amount='0.0000004')
        profile = conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
        ledger = CustodyJournalBuilder(conn, profile).build()
        disposals = [r for r in ledger['entries'] if r['entry_type'] == 'disposal']
        expected = (Decimal('10.6') if policy == 'taxable' else Decimal('11')) if priced else Decimal('0')
        assert sum(Decimal(str(r['proceeds'])) for r in disposals) == expected
        if priced and policy == 'carrying-value':
            # Generic rail carry has a basis-valued neutral source disposal.
            assert any(r['asset'] == 'BTC' and Decimal(str(r['proceeds'])) == Decimal('0.4')
                       and Decimal(str(r['gain_loss'])) == 0 for r in disposals)
        if not priced:
            assert any(q['transaction_id'] == 'out' and q['reason'] == 'missing_spot_price' for q in ledger['quarantines'])
        process_journals(conn, 'Books', 'Book')
        stored = conn.execute("SELECT proceeds_exact FROM journal_entries WHERE entry_type='disposal'").fetchall()
        assert sum(Decimal(r[0]) for r in stored) == expected
        if priced:
            # A previously current projection is replaced by an explicit process.
            conn.execute("UPDATE journal_entries SET proceeds=999, proceeds_exact='999' WHERE entry_type='disposal'")
            conn.commit()
            process_journals(conn, 'Books', 'Book')
            refreshed = conn.execute("SELECT proceeds_exact FROM journal_entries WHERE entry_type='disposal'").fetchall()
            assert sum(Decimal(r[0]) for r in refreshed) == expected
        # Reports read the stored projection, never the payout metadata as events.
        result = cli_runner.run(['--data-root', str(tmp_path),
                '--machine', 'reports', 'capital-gains', '--workspace', 'Books', '--profile', 'Book'],
            capture_output=True, text=True)
        payload = json.loads(result.stdout)
        assert result.returncode == 0, result.stdout + result.stderr
        assert sum(Decimal(str(r['proceeds'])) for r in payload['data']) == expected
    finally:
        conn.close()

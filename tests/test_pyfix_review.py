"""Regression coverage for the Python pricing and partial-payout review."""
import json
from dataclasses import replace
from decimal import Decimal
from types import SimpleNamespace

import pytest

from kassiber.cli.handlers import create_direct_swap_payout, process_journals
from kassiber.core import commercial, custody_tax_projection as ctp, metadata, review_workflow
from kassiber.core.custody_journal import CustodyJournalBuilder
from kassiber.core.custody_quantity import QuantitySlice
from kassiber.core.custody_review_terms import delete_payout_review, list_payout_reviews
from kassiber.core.imports import normalize_import_record
from kassiber.core.rates import set_manual_rate, upsert_rate
from kassiber.core.source_funds import create_source
from kassiber.db import open_db
from kassiber.errors import AppError
from tests import cli_runner
from tests.test_btcpay_commercial_provenance import _hooks
from tests.test_custody_authored_migration import _scope, _tx, NOW


@pytest.fixture
def book(tmp_path):
    conn = open_db(tmp_path)
    _scope(conn)
    _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW)
    conn.commit()
    yield conn
    conn.close()


BAD = ['NaN', 'sNaN', 'Infinity', '1e400', '1e999', '1e-20', '1e-300', '1e-999999999']


@pytest.mark.parametrize('value', BAD)
@pytest.mark.parametrize('path', ['metadata', 'review', 'rate', 'rate_cache', 'payout', 'import', 'document', 'source'])
def test_price_writes_reject_unsupported_values(book, value, path):
    with pytest.raises(AppError) as error:
        if path == 'metadata':
            metadata._clean_decimal_or_none(value, 'fiat_rate')
        elif path == 'review':
            review_workflow._operations([dict(type='price_override', transaction_id='out', reason='review', fiat_rate=value)])
        elif path == 'rate':
            set_manual_rate(book, 'BTC-EUR', NOW, value)
        elif path == 'rate_cache':
            upsert_rate(book, 'BTC-EUR', NOW, value, 'test')
        elif path == 'payout':
            create_direct_swap_payout(book, 'Books', 'Book', 'out', payout_asset='EUR', payout_amount='1', payout_fiat_value=value)
        elif path == 'import':
            normalize_import_record(dict(amount='1', occurred_at=NOW, fiat_rate=value))
        elif path == 'document':
            commercial.create_document(book, 'Books', 'Book', _hooks(), document_type='invoice', label='Invoice', fiat_value=value)
        else:
            create_source(book, 'Books', 'Book', SimpleNamespace(resolve_scope=_hooks().resolve_scope), source_type='fiat_purchase', label='Source', fiat_value=value)
    assert error.value.code == 'validation'


@pytest.mark.parametrize('value', BAD[:5])
@pytest.mark.parametrize('field', ['amount', 'rate', 'invoice_amount'])
def test_btcpay_rejects_nonfinite_fiat(book, value, field):
    ws, profile = _hooks().resolve_scope(book)
    invoice = dict(store_id='store', invoice_id='invoice', currency='EUR', amount='10', created_at=NOW,
                   payments=[dict(payment_id='payment', payment_method_id='BTC-CHAIN', amount='0.01',
                                  invoice_currency='EUR', invoice_amount='10', rate='1000', received_at=NOW)])
    if field == 'amount':
        invoice[field] = value
    else:
        invoice['payments'][0][field] = value
    with pytest.raises(AppError) as error:
        commercial.upsert_btcpay_provenance(book, ws, profile, backend_name='btcpay', invoices=[invoice])
    assert error.value.code == 'validation'


@pytest.mark.parametrize('amount,value', [('0.00000000001', '1e308'), ('100000000', '1e-13')])
def test_import_validates_value_derived_rate(amount, value):
    with pytest.raises(AppError) as error:
        normalize_import_record(dict(amount=amount, occurred_at=NOW, fiat_value=value))
    assert error.value.code == 'validation'


@pytest.mark.parametrize('value,stored', [('-0', '0'), ('0.0000001', '1E-7')])
def test_decimal_text_normalizes_exactly(book, value, stored):
    create_direct_swap_payout(book, 'Books', 'Book', 'out', payout_asset='EUR', payout_amount='1', payout_fiat_value=value)
    assert book.execute('SELECT payout_fiat_value_exact FROM custody_component_economic_terms').fetchone()[0] == stored


@pytest.mark.parametrize('surface', ['core', 'cli', 'daemon'])
def test_invalid_legacy_payout_list_and_delete(tmp_path, surface):
    conn = open_db(tmp_path)
    _scope(conn)
    _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW)
    conn.commit()
    payout = create_direct_swap_payout(conn, 'Books', 'Book', 'out', payout_asset='EUR', payout_amount='1', payout_fiat_value='10')
    payout_id = payout['id']
    conn.execute('DROP TRIGGER trg_custody_component_terms_immutable')
    conn.execute("UPDATE custody_component_economic_terms SET payout_fiat_value_exact='inf'")
    conn.commit()
    conn.close()
    conn = open_db(tmp_path)
    try:
        from kassiber.daemon import _ui_swap_matching_payload_from_conn
        scope = dict(workspace='Books', profile='Book')
        command = ['--machine', '--data-root', str(tmp_path),
                   'transfers', 'payouts']
        if surface == 'core':
            rows = list_payout_reviews(conn, 'profile')
        elif surface == 'daemon':
            rows = _ui_swap_matching_payload_from_conn(conn, 'ui.transfers.payouts.list', scope)['payouts']
        else:
            result = cli_runner.run(command + ['list', '--workspace', 'Books', '--profile', 'Book'], capture_output=True, text=True)
            assert result.returncode == 0, result.stdout + result.stderr
            rows = json.loads(result.stdout)['data']
        bad = next(row for row in rows if row['id'] == payout_id)
        assert bad['invalid'] and bad['payout_fiat_value'] is None
        if surface == 'core':
            deleted = delete_payout_review(conn, 'profile', payout_id)
        elif surface == 'daemon':
            deleted = _ui_swap_matching_payload_from_conn(conn, 'ui.transfers.payouts.delete', dict(**scope, payout_id=payout_id))
        else:
            result = cli_runner.run(command + ['delete', '--workspace', 'Books', '--profile', 'Book', '--payout-id', payout_id], capture_output=True, text=True)
            assert result.returncode == 0, result.stdout + result.stderr
            deleted = json.loads(result.stdout)['data']
        assert deleted['deleted_at']
        assert not list_payout_reviews(conn, 'profile')
    finally:
        conn.close()


@pytest.mark.parametrize('policy,asset', [('taxable', 'EUR'), ('carrying-value', 'LBTC')])
def test_split_payout_fails_closed(book, monkeypatch, policy, asset):
    create_direct_swap_payout(book, 'Books', 'Book', 'out', payout_asset=asset, payout_amount='0.0000004',
                             payout_fiat_value='10', out_amount='0.0000004', policy=policy)
    original = ctp.compile_finalized_tax_projection
    def split(profile, rows, state, **kwargs):
        decisions = []
        for decision in state.projection.decisions:
            if decision.reason == 'reviewed_direct_payout_source':
                source = decision.source
                middle = (source.start_msat + source.end_msat) // 2
                decisions.extend([replace(decision, source=QuantitySlice(source.observation_hash, source.start_msat, middle)),
                                  replace(decision, source=QuantitySlice(source.observation_hash, middle, source.end_msat))])
            else:
                decisions.append(decision)
        state = replace(state, projection=replace(state.projection, decisions=tuple(decisions)))
        if hasattr(state.tax_eligibility, 'eligible_decisions'):
            state = replace(state, tax_eligibility=replace(state.tax_eligibility, eligible_decisions=tuple(decisions)))
        return original(profile, rows, state, **kwargs)
    monkeypatch.setattr(ctp, 'compile_finalized_tax_projection', split)
    with pytest.raises(AppError, match='out: split payout'):
        CustodyJournalBuilder(book, book.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()).build_custody_projection()


def test_legacy_price_error_identifies_transaction(book):
    book.execute("UPDATE transactions SET fiat_rate='inf', fiat_rate_exact='Infinity' WHERE id='out'")
    book.commit()
    with pytest.raises(AppError, match='transaction out: fiat_rate'):
        process_journals(book, 'Books', 'Book')


@pytest.mark.parametrize('export_format', ['csv', 'xlsx'])
def test_export_survives_legacy_nonfinite_price(book, tmp_path, export_format):
    book.execute("UPDATE transactions SET fiat_rate='inf', fiat_rate_exact='Infinity' WHERE id='out'")
    book.commit()
    output = tmp_path / ('transactions.' + export_format)
    # XLSX remains a Python reference boundary; CSV is a registered Rust slice.
    result = cli_runner.run(['--machine', '--data-root', str(tmp_path),
                             'transactions', 'export', '--export-format', export_format, '--workspace', 'Books', '--profile', 'Book',
                             '--file', str(output)], reference=export_format != 'csv', capture_output=True, text=True)
    assert result.returncode == 0, result.stdout + result.stderr
    assert output.stat().st_size > 0
    if export_format == 'csv':
        assert 'inf' not in output.read_text().lower()


@pytest.mark.parametrize('value', ['1e400', '1e-20', '1e-999999999'])
def test_daemon_metadata_rejects_price_without_writes(book, value):
    from kassiber.daemon import _transaction_metadata_update_payload
    from kassiber.db import set_setting

    set_setting(book, 'context_workspace', 'ws')
    set_setting(book, 'context_profile', 'profile')
    book.commit()
    before = book.total_changes
    with pytest.raises(AppError) as error:
        _transaction_metadata_update_payload(book, dict(transaction='out', fiat_rate=value), default_source='user')
    assert error.value.code == 'validation'
    assert book.total_changes == before
